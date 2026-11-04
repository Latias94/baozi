use super::{
    BuiltCornerProjection, BuiltGeometryStreams, CornerChannelSelection, FaceVertex,
    IndexedCornerChannel, IndexedPosition, MeshWorkGroups, ObjFailure, ParsedObj, mtl,
    usize_amount,
};
use baozi_import::{
    CapacityPlan, ImportStorageClass, ImportWorkClass, MeteredVec, ParseChildren, ParseErrorView,
    ParsePieceError, ParseResult, ParseServices, ParseWorkLayout,
};

// Each row touches one source/corner and at most one XYZ/UVW value, below eight actions.
const ROWS_PER_POLL: usize = 32;

// Match a fresh MeteredVec reserve, including its minimum and replacement envelope.
fn envelope<T>(len: usize) -> Option<u64> {
    if len == 0 {
        return Some(0);
    }
    CapacityPlan::for_type::<T>(0, 0, len.max(4))
        .ok()?
        .envelope_delta_bytes()
        .ok()
}

pub(super) fn eligible(
    corners: usize,
    source_counts: [usize; 3],
    channels: CornerChannelSelection,
) -> bool {
    let required = [
        source_counts[0],
        if channels.texcoords {
            source_counts[1]
        } else {
            0
        },
        if channels.normals {
            source_counts[2]
        } else {
            0
        },
    ];
    if required.into_iter().any(|count| count > corners) {
        return false;
    }
    let fits = || {
        let references = corners.checked_mul(channels.retained_count())?;
        let previous = envelope::<IndexedPosition>(corners)?
            .checked_add(envelope::<IndexedCornerChannel>(references)?)?
            .checked_add(envelope::<bool>(corners)?)?;
        let dense = required.into_iter().try_fold(0_u64, |total, count| {
            total
                .checked_add(envelope::<Option<u32>>(count)?)?
                .checked_add(envelope::<usize>(count)?)
        })?;
        Some(dense <= previous)
    };
    fits().unwrap_or(false)
}

fn for_rows<'op, 'session>(
    children: &mut ParseChildren<'_, 'op, 'session>,
    group: &ParseWorkLayout<'op, 1>,
    len: usize,
    resource: &'static str,
    mut visit: impl FnMut(ParseErrorView<'session>, usize) -> Result<(), ParsePieceError<ObjFailure>>,
) -> ParseResult<()> {
    #[cfg(test)]
    super::dense_tests::before_rows(&mut children.services()?, resource)?;
    let mut start = 0;
    while start < len {
        let end = start.saturating_add(ROWS_PER_POLL).min(len);
        let amount = usize_amount(&children.services()?, end - start, resource)?;
        children.child(group, [amount], |work| -> ParseResult<()> {
            let errors = work.error_view();
            for cursor in start..end {
                work.run_returned_with_storage(|_storage| {
                    visit(errors, cursor)?;
                    #[cfg(test)]
                    super::dense_tests::row_completed(resource);
                    Ok(((), [1]))
                })?;
            }
            Ok(())
        })?;
        start = end;
    }
    Ok(())
}

struct SourceRemap<'session> {
    indices: MeteredVec<'session, Option<u32>>,
    sources: MeteredVec<'session, usize>,
}

impl<'session> SourceRemap<'session> {
    fn new<'op>(
        children: &mut ParseChildren<'_, 'op, 'session>,
        work: &ParseWorkLayout<'op, 1>,
        count: usize,
        resources: [&'static str; 3],
    ) -> ParseResult<Self> {
        let [indices_resource, sources_resource, init_resource] = resources;
        let exec = &mut children.services()?;
        let mut indices =
            exec.metered_vec(ImportStorageClass::FormatNormalization, indices_resource);
        let mut sources =
            exec.metered_vec(ImportStorageClass::FormatNormalization, sources_resource);
        indices.try_reserve(exec, count)?;
        sources.try_reserve(exec, count)?;
        for_rows(children, work, count, init_resource, |_errors, _| {
            indices
                .try_push_reserved(None)
                .map_err(ParsePieceError::Parse)
        })?;
        Ok(Self { indices, sources })
    }

    fn observe(
        &mut self,
        errors: ParseErrorView<'_>,
        source: usize,
        source_error: &'static str,
        index_resource: &'static str,
    ) -> Result<u32, ParsePieceError<ObjFailure>> {
        let slot = self
            .indices
            .as_mut_slice()
            .get_mut(source)
            .ok_or_else(|| ParsePieceError::Format(ObjFailure::internal(source_error)))?;
        if let Some(index) = *slot {
            return Ok(index);
        }
        let index = u32::try_from(self.sources.len())
            .map_err(|_| errors.allocation_failure(index_resource))?;
        self.sources.try_push_reserved(source)?;
        *slot = Some(index);
        Ok(index)
    }

    fn get(&self, source: usize) -> Result<u32, ParsePieceError<ObjFailure>> {
        self.indices
            .as_slice()
            .get(source)
            .copied()
            .flatten()
            .ok_or_else(|| {
                ParsePieceError::Format(ObjFailure::internal("obj-dense-source-remap-missing"))
            })
    }

    fn sort(&mut self, children: &mut ParseChildren<'_, '_, 'session>) -> ParseResult<()> {
        mtl::interruptible_sort_by(children, &mut self.sources, |work, left, right| {
            work.run_leaf(|| left.cmp(right))
        })
    }
}

fn projection<'session, T>(
    exec: &mut ParseServices<'_, '_, 'session>,
    values: usize,
    corners: usize,
    resources: [&'static str; 2],
) -> ParseResult<BuiltCornerProjection<'session, T>> {
    let mut source_values = exec.metered_vec(ImportStorageClass::FormatNormalization, resources[0]);
    let mut source_indices =
        exec.metered_vec(ImportStorageClass::FormatNormalization, resources[1]);
    source_values.try_reserve(exec, values)?;
    source_indices.try_reserve(exec, corners)?;
    Ok(BuiltCornerProjection {
        source_values,
        source_indices,
    })
}

pub(super) fn materialize<'op, 'session>(
    children: &mut ParseChildren<'_, 'op, 'session>,
    groups: &MeshWorkGroups<'op>,
    parsed: &ParsedObj<'_>,
    corners: &[FaceVertex],
    channels: CornerChannelSelection,
    attribute_stream_count: usize,
) -> ParseResult<BuiltGeometryStreams<'session>> {
    let work = children.bind_group([ImportWorkClass::WorkUnits])?;
    let mut position_remap = SourceRemap::new(
        children,
        &work,
        parsed.positions.len(),
        [
            "obj-dense-position-remap",
            "obj-unique-position-sources",
            "obj-dense-position-init-work",
        ],
    )?;
    let exec = &mut children.services()?;
    let mut indices = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-segment-indices",
    );
    indices.try_reserve(exec, corners.len())?;
    for_rows(
        children,
        &work,
        corners.len(),
        "obj-dense-position-discovery-work",
        |errors, cursor| {
            let index = position_remap.observe(
                errors,
                corners[cursor].position,
                "obj-position-source-index-out-of-range",
                "obj-vertex-index-space",
            )?;
            indices
                .try_push_reserved(index)
                .map_err(ParsePieceError::Parse)
        },
    )?;

    let mut texcoord_lanes = 2_u8;
    let mut texcoord_remap = if channels.texcoords {
        let mut remap = SourceRemap::new(
            children,
            &work,
            parsed.texcoords.len(),
            [
                "obj-dense-texcoord-remap",
                "obj-unique-texcoord-sources",
                "obj-dense-texcoord-init-work",
            ],
        )?;
        for_rows(
            children,
            &work,
            corners.len(),
            "obj-dense-texcoord-discovery-work",
            |errors, cursor| {
                let source = corners[cursor].texcoord.ok_or_else(|| {
                    ParsePieceError::Format(ObjFailure::internal(
                        "complete-texcoord-channel-has-missing-index",
                    ))
                })?;
                let value = parsed.texcoords.as_slice().get(source).ok_or_else(|| {
                    ParsePieceError::Format(ObjFailure::internal(
                        "obj-texcoord-source-index-out-of-range",
                    ))
                })?;
                remap.observe(
                    errors,
                    source,
                    "obj-texcoord-source-index-out-of-range",
                    "obj-texcoord-source-index-space",
                )?;
                texcoord_lanes = texcoord_lanes.max(value.lanes);
                Ok(())
            },
        )?;
        remap.sort(children)?;
        Some(remap)
    } else {
        None
    };
    let mut normal_remap = if channels.normals {
        let mut remap = SourceRemap::new(
            children,
            &work,
            parsed.normals.len(),
            [
                "obj-dense-normal-remap",
                "obj-unique-normal-sources",
                "obj-dense-normal-init-work",
            ],
        )?;
        for_rows(
            children,
            &work,
            corners.len(),
            "obj-dense-normal-discovery-work",
            |errors, cursor| {
                let source = corners[cursor].normal.ok_or_else(|| {
                    ParsePieceError::Format(ObjFailure::internal(
                        "complete-normal-channel-has-missing-index",
                    ))
                })?;
                remap.observe(
                    errors,
                    source,
                    "obj-normal-source-index-out-of-range",
                    "obj-normal-source-index-space",
                )?;
                Ok(())
            },
        )?;
        remap.sort(children)?;
        Some(remap)
    } else {
        None
    };

    let texcoords = if let Some(remap) = texcoord_remap.as_mut() {
        let exec = &mut children.services()?;
        let scalar_count = remap
            .sources
            .len()
            .checked_mul(usize::from(texcoord_lanes))
            .ok_or_else(|| exec.allocation_failure("obj-texcoord-source-scalar-count"))?;
        let mut built = projection(
            exec,
            scalar_count,
            corners.len(),
            [
                "obj-segment-texcoord-source-values",
                "obj-segment-texcoord-source-indices",
            ],
        )?;
        for_rows(
            children,
            &work,
            remap.sources.len(),
            "obj-dense-texcoord-projection-work",
            |errors, cursor| {
                let source = remap.sources.as_slice()[cursor];
                let value = parsed.texcoords.as_slice().get(source).ok_or_else(|| {
                    ParsePieceError::Format(ObjFailure::internal(
                        "obj-texcoord-source-index-out-of-range",
                    ))
                })?;
                let index = u32::try_from(cursor)
                    .map_err(|_| errors.allocation_failure("obj-texcoord-source-index-space"))?;
                built
                    .source_values
                    .try_extend_from_slice_reserved(&value.values[..usize::from(texcoord_lanes)])?;
                remap.indices.as_mut_slice()[source] = Some(index);
                Ok(())
            },
        )?;
        for_rows(
            children,
            &work,
            corners.len(),
            "obj-dense-texcoord-indices-work",
            |_errors, cursor| {
                let source = corners[cursor].texcoord.ok_or_else(|| {
                    ParsePieceError::Format(ObjFailure::internal(
                        "complete-texcoord-channel-has-missing-index",
                    ))
                })?;
                built
                    .source_indices
                    .try_push_reserved(remap.get(source)?)
                    .map_err(ParsePieceError::Parse)
            },
        )?;
        Some(built)
    } else {
        None
    };
    let normals = if let Some(remap) = normal_remap.as_mut() {
        let mut built = projection(
            &mut children.services()?,
            remap.sources.len(),
            corners.len(),
            [
                "obj-segment-normal-source-values",
                "obj-segment-normal-source-indices",
            ],
        )?;
        for_rows(
            children,
            &work,
            remap.sources.len(),
            "obj-dense-normal-projection-work",
            |errors, cursor| {
                let source = remap.sources.as_slice()[cursor];
                let value = parsed
                    .normals
                    .as_slice()
                    .get(source)
                    .copied()
                    .ok_or_else(|| {
                        ParsePieceError::Format(ObjFailure::internal(
                            "obj-normal-source-index-out-of-range",
                        ))
                    })?;
                let index = u32::try_from(cursor)
                    .map_err(|_| errors.allocation_failure("obj-normal-source-index-space"))?;
                built.source_values.try_push_reserved(value)?;
                remap.indices.as_mut_slice()[source] = Some(index);
                Ok(())
            },
        )?;
        for_rows(
            children,
            &work,
            corners.len(),
            "obj-dense-normal-indices-work",
            |_errors, cursor| {
                let source = corners[cursor].normal.ok_or_else(|| {
                    ParsePieceError::Format(ObjFailure::internal(
                        "complete-normal-channel-has-missing-index",
                    ))
                })?;
                built
                    .source_indices
                    .try_push_reserved(remap.get(source)?)
                    .map_err(ParsePieceError::Parse)
            },
        )?;
        Some(built)
    } else {
        None
    };

    let exec = &mut children.services()?;
    let scalar_count = position_remap
        .sources
        .len()
        .checked_mul(3)
        .ok_or_else(|| exec.allocation_failure("obj-position-scalar-count"))?;
    let mut positions = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-segment-position-scalars",
    );
    positions.try_reserve(exec, scalar_count)?;
    #[cfg(test)]
    super::dense_tests::before_rows(exec, "obj-dense-position-values-work")?;
    for sources in position_remap.sources.as_slice().chunks(ROWS_PER_POLL) {
        let amount = usize_amount(
            &children.services()?,
            sources.len(),
            "obj-geometry-materialization-work",
        )?;
        children.child(
            &groups.materialization,
            [amount, amount],
            |work| -> ParseResult<()> {
                for source in sources {
                    work.run_returned_with_storage::<_, ObjFailure>(|_storage| {
                        let position =
                            parsed.positions.as_slice().get(*source).ok_or_else(|| {
                                ParsePieceError::Format(ObjFailure::internal(
                                    "obj-position-source-index-out-of-range",
                                ))
                            })?;
                        positions.try_extend_from_slice_reserved(&[
                            position.x, position.y, position.z,
                        ])?;
                        #[cfg(test)]
                        super::dense_tests::row_completed("obj-dense-position-values-work");
                        Ok(((), [1, 1]))
                    })?;
                }
                Ok(())
            },
        )?;
    }
    let built = BuiltGeometryStreams {
        positions,
        texcoords,
        texcoord_lanes,
        normals,
        indices,
        attribute_stream_count,
    };
    #[cfg(test)]
    super::dense_tests::capture(
        position_remap.sources.as_slice(),
        texcoord_remap
            .as_ref()
            .map(|remap| remap.sources.as_slice()),
        normal_remap.as_ref().map(|remap| remap.sources.as_slice()),
        &built,
    );
    // These are per-segment lookup tables, never borrowed by canonical authoring.
    drop((position_remap, texcoord_remap, normal_remap));
    Ok(built)
}
