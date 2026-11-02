use crate::mtl::{
    self, MaterialLibrary, ParsedMaterial, ParsedMetadata, ParsedMetadataValue, ParsedTexture,
};
use crate::obj::{Face, FaceVertex, ParsedObj, StateNameId};
use crate::parser::usize_amount;
use crate::{FORMAT_ID, ObjFailure, diffuse_texture_role};
use baozi_import::authoring::{
    AlphaMode, AnimationAuthoringBatch, AppearanceAuthoringBatch, AttributeDomain,
    AttributeSemantic, AuthoredTransform, AuthoringRange, Color, ColorInterpretation,
    DeformationAuthoringBatch, ExtensionValueRef, FormatId, GeometryAttributeAuthoring,
    GeometryAuthoringBatch, GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, ImageAuthoring, ImageLocalId,
    LegacyMaterialFactors, LineageAuthoringBatch, LogicalMeshLocalId, LogicalMorphAuthoring,
    MAX_ENTITY_NAME_BYTES, MaterialAuthoring, MaterialBindingAuthoring, MaterialFactors,
    MaterialLocalId, MeshInstanceDeformationAuthoring, MetadataAuthoringBatch,
    MetadataAuthoringEntry, NamespacedName, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId,
    ObservedResourceAuthoring, OwnerProvenanceAuthoring, Presence, PrimitiveDeformationAuthoring,
    ProfileAuthoringBatch, ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring,
    ResourceIdentity, ResourceIdentityLocalId, ResourceSnapshotAuthoring, SamplerAuthoring,
    SamplerLocalId, SamplerState, SceneAuthoringBatch, SceneSpace, SessionOpaqueReason,
    ShadingModel as CanonicalShadingModel, SourceIdentityAuthoring, SourceIdentityLocalId,
    SourceObjectKind, SourceObjectLocator, SpatialAuthoringBatch, TextureAuthoring,
    TextureBlendOperation, TextureChannelSwizzle, TextureLocalId, TextureRole, TextureSourceKey,
    TextureStrength, TextureTransform, Vec3,
};
use baozi_import::{
    DependencySnapshot, DiagnosticCode, ImportStorageClass, ImportWorkClass, ImportWorkGroup,
    MeteredVec, ParseContext, ParseExecution, ParseResult, SourceScopeEntityKind,
};

const FALLBACK_RESOURCE_IDENTITY: &str = "session/obj-primary";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SegmentKey {
    object: Option<StateNameId>,
    group: Option<StateNameId>,
    material: Option<StateNameId>,
}

fn bounded_entity_name(name: &str) -> Option<&str> {
    (name.len() <= MAX_ENTITY_NAME_BYTES).then_some(name)
}

impl SegmentKey {
    fn from_face(face: &Face<'_>) -> Self {
        Self {
            object: face.object,
            group: face.group,
            material: face.material,
        }
    }

    fn display_name(self) -> Option<StateNameId> {
        self.group.or(self.object).or(self.material)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndexedPosition {
    source_index: usize,
    occurrence: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct CornerChannelState {
    has_source: bool,
    has_missing: bool,
}

impl CornerChannelState {
    fn observe(&mut self, source_is_present: bool) {
        self.has_source |= source_is_present;
        self.has_missing |= !source_is_present;
    }

    const fn is_partial(self) -> bool {
        self.has_source && self.has_missing
    }

    const fn is_complete(self) -> bool {
        self.has_source && !self.has_missing
    }
}

#[derive(Debug, Clone, Copy)]
struct CornerChannelSelection {
    texcoords: bool,
    normals: bool,
}

impl CornerChannelSelection {
    fn retained_count(self) -> usize {
        usize::from(self.texcoords) + usize::from(self.normals)
    }

    fn retained_amount(self, exec: &ParseExecution<'_, '_>) -> ParseResult<u64> {
        usize_amount(
            exec,
            self.retained_count(),
            "obj-canonical-attribute-stream-count",
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CornerChannel {
    Texcoord,
    Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct IndexedCornerChannel {
    channel: CornerChannel,
    source_index: usize,
    occurrence: usize,
}

struct BuiltCornerProjection<'session, T> {
    source_values: MeteredVec<'session, T>,
    source_indices: MeteredVec<'session, u32>,
}

impl<'session, T> BuiltCornerProjection<'session, T> {
    fn new(
        exec: &mut ParseExecution<'_, 'session>,
        corner_count: usize,
        source_value_count: usize,
        values_resource: &'static str,
        indices_resource: &'static str,
        indices_work_resource: &'static str,
    ) -> ParseResult<Self> {
        let mut source_values =
            exec.metered_vec(ImportStorageClass::FormatNormalization, values_resource);
        let mut source_indices =
            exec.metered_vec(ImportStorageClass::FormatNormalization, indices_resource);
        source_values.try_reserve(exec, source_value_count)?;
        source_indices.try_reserve(exec, corner_count)?;
        mtl::for_work_chunks(exec, corner_count, indices_work_resource, |exec, range| {
            source_indices.try_resize(exec, range.end, 0)
        })?;
        Ok(Self {
            source_values,
            source_indices,
        })
    }
}

struct SegmentBuilder<'session> {
    key: SegmentKey,
    corner_references: MeteredVec<'session, FaceVertex>,
    face_vertex_counts: MeteredVec<'session, u32>,
    smoothing_groups: Option<MeteredVec<'session, u32>>,
    attribute_stream_count: usize,
    texcoord_channel: CornerChannelState,
    normal_channel: CornerChannelState,
    has_polygon: bool,
}

struct MeshWorkGroups<'region> {
    streams: ImportWorkGroup<'region, 1>,
    face_cells: ImportWorkGroup<'region, 3>,
    corner_attributes: ImportWorkGroup<'region, 2>,
    materialization: ImportWorkGroup<'region, 2>,
    meshes: ImportWorkGroup<'region, 1>,
}

impl<'region> MeshWorkGroups<'region> {
    fn bind(exec: &mut ParseExecution<'region, '_>) -> ParseResult<Self> {
        Ok(Self {
            streams: exec.bind_work_group([ImportWorkClass::AttributeStreams])?,
            face_cells: exec.bind_work_group([
                ImportWorkClass::AttributeStreams,
                ImportWorkClass::AttributeCells,
                ImportWorkClass::TopologyCells,
            ])?,
            corner_attributes: exec.bind_work_group([
                ImportWorkClass::AttributeStreams,
                ImportWorkClass::AttributeCells,
            ])?,
            materialization: exec
                .bind_work_group([ImportWorkClass::WorkUnits, ImportWorkClass::Vertices])?,
            meshes: exec.bind_work_group([ImportWorkClass::Meshes])?,
        })
    }
}

fn state_name<'parsed>(
    exec: &mut ParseExecution<'_, '_>,
    parsed: &'parsed ParsedObj<'_>,
    id: StateNameId,
) -> ParseResult<&'parsed str> {
    parsed.state_name(id).ok_or_else(|| {
        exec.format_failure(ObjFailure::internal("segment-state-name-id-out-of-range"))
    })
}

fn record_partial_attribute_losses(
    exec: &mut ParseExecution<'_, '_>,
    segment_index: usize,
    partial_texcoords: bool,
    partial_normals: bool,
) -> ParseResult<()> {
    if !partial_texcoords && !partial_normals {
        return Ok(());
    }

    // Sparse canonical attributes supply defaults; they do not encode absent corner values.
    let segment_amount = usize_amount(exec, segment_index, "obj-partial-attribute-segment")?;
    let mut object_path = exec.metered_string(
        ImportStorageClass::FormatNormalization,
        "obj-partial-attribute-object-path",
    );
    for (partial, channel, code) in [
        (
            partial_texcoords,
            "texcoord",
            "obj.partial_texcoord_channel",
        ),
        (partial_normals, "normal", "obj.partial_normal_channel"),
    ] {
        if !partial {
            continue;
        }
        object_path.truncate(0);
        object_path.try_write_fmt(
            exec,
            format_args!("/segments/{segment_index}/attributes/{channel}"),
        )?;
        let code = DiagnosticCode::new(code).expect("static OBJ diagnostic code must be valid");
        exec.record_source_semantic_loss(
            code,
            SourceScopeEntityKind::Mesh,
            object_path.as_str(),
            |details| {
                details.unsigned("segment_index", segment_amount);
                details.string("channel", channel);
            },
            format_args!(
                "OBJ segment {segment_index} mixes corners with and without {channel} values; dropping this partial channel requires explicit source semantic loss authorization"
            ),
        )?;
    }
    Ok(())
}

fn checked_attribute_stream_count(
    exec: &ParseExecution<'_, '_>,
    current: usize,
    additional: usize,
) -> ParseResult<usize> {
    let count = current
        .checked_add(additional)
        .ok_or_else(|| exec.allocation_failure("obj-attribute-stream-count"))?;
    exec.enforce_resource_limit(
        "max_vertex_attribute_streams_per_mesh",
        usize_amount(exec, count, "obj-attribute-stream-count")?,
        usize_amount(
            exec,
            exec.limits().max_vertex_attribute_streams_per_mesh,
            "obj-attribute-stream-limit",
        )?,
    )?;
    Ok(count)
}

impl<'session> SegmentBuilder<'session> {
    fn new<'region>(
        exec: &mut ParseExecution<'region, 'session>,
        groups: &MeshWorkGroups<'region>,
        key: SegmentKey,
    ) -> ParseResult<Self> {
        let attribute_stream_count = checked_attribute_stream_count(exec, 0, 1)?;
        exec.with_work(&groups.streams, [1], |_exec, work| -> ParseResult<()> {
            work.complete_all()
        })?;
        Ok(Self {
            key,
            corner_references: exec.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-segment-corner-references",
            ),
            face_vertex_counts: exec.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-face-vertex-counts",
            ),
            smoothing_groups: None,
            attribute_stream_count,
            texcoord_channel: CornerChannelState::default(),
            normal_channel: CornerChannelState::default(),
            has_polygon: false,
        })
    }

    fn push_face<'region>(
        &mut self,
        exec: &mut ParseExecution<'region, 'session>,
        groups: &MeshWorkGroups<'region>,
        face: &Face<'_>,
    ) -> ParseResult<()> {
        let face_vertex_count = face.vertices.len();
        if face_vertex_count < 3 {
            return Err(
                exec.format_failure(ObjFailure::internal("normalized-face-has-too-few-vertices"))
            );
        }
        let face_count_cells = if self.has_polygon {
            1
        } else if face_vertex_count > 3 {
            self.face_vertex_counts
                .len()
                .checked_add(1)
                .ok_or_else(|| exec.allocation_failure("obj-canonical-topology-counter"))?
        } else {
            0
        };
        let canonical_topology_cells = face_vertex_count
            .checked_add(face_count_cells)
            .ok_or_else(|| exec.allocation_failure("obj-canonical-topology-counter"))?;
        let canonical_topology_cells = usize_amount(
            exec,
            canonical_topology_cells,
            "obj-canonical-topology-cells",
        )?;
        let face_count_cells =
            usize_amount(exec, face_count_cells, "obj-canonical-face-count-cells")?;
        let prior_face_count = self.face_vertex_counts.len();
        let smoothing_value = face.smoothing.canonical_value();
        let activates_smoothing = self.smoothing_groups.is_none() && smoothing_value.is_some();
        let additional_attribute_streams = usize::from(activates_smoothing);
        let attribute_stream_count = checked_attribute_stream_count(
            exec,
            self.attribute_stream_count,
            additional_attribute_streams,
        )?;
        let additional_attribute_streams = usize_amount(
            exec,
            additional_attribute_streams,
            "obj-canonical-attribute-stream-count",
        )?;
        let smoothing_cells = if self.smoothing_groups.is_some() {
            1
        } else if smoothing_value.is_some() {
            prior_face_count
                .checked_add(1)
                .ok_or_else(|| exec.allocation_failure("obj-smoothing-group-cell-count"))?
        } else {
            0
        };
        let smoothing_cells =
            usize_amount(exec, smoothing_cells, "obj-canonical-smoothing-group-cells")?;

        exec.with_work(
            &groups.face_cells,
            [
                additional_attribute_streams,
                smoothing_cells,
                canonical_topology_cells,
            ],
            |exec, work| -> ParseResult<()> {
                if face_vertex_count > 3 {
                    self.has_polygon = true;
                }
                let face_vertex_count = u32::try_from(face_vertex_count)
                    .map_err(|_| exec.allocation_failure("obj-face-vertex-count"))?;
                work.run([0, 0, face_count_cells], || {
                    self.face_vertex_counts.try_push(exec, face_vertex_count)
                })?;

                if let Some(smoothing_groups) = self.smoothing_groups.as_mut() {
                    let value = smoothing_value.unwrap_or(0);
                    work.run([0, 1, 0], || smoothing_groups.try_push(exec, value))?;
                } else if let Some(value) = smoothing_value {
                    let smoothing_count = prior_face_count
                        .checked_add(1)
                        .ok_or_else(|| exec.allocation_failure("obj-smoothing-group-cell-count"))?;
                    let mut smoothing_groups = exec.metered_vec(
                        ImportStorageClass::FormatNormalization,
                        "obj-face-smoothing-groups",
                    );
                    smoothing_groups.try_reserve(exec, smoothing_count)?;
                    let mut remaining = prior_face_count;
                    while remaining != 0 {
                        exec.poll()?;
                        let chunk_len = remaining.min(64);
                        let chunk_amount =
                            usize_amount(exec, chunk_len, "obj-smoothing-group-backfill-cells")?;
                        work.run([0, chunk_amount, 0], || {
                            for _ in 0..chunk_len {
                                smoothing_groups.try_push_reserved(0)?;
                            }
                            Ok(())
                        })?;
                        remaining -= chunk_len;
                    }
                    work.run([1, 1, 0], || smoothing_groups.try_push_reserved(value))?;
                    self.smoothing_groups = Some(smoothing_groups);
                }
                self.corner_references
                    .try_reserve(exec, face.vertices.len())?;

                for chunk in face.vertices.as_slice().chunks(64) {
                    exec.poll()?;
                    for vertex in chunk {
                        work.run([0, 0, 1], || {
                            self.corner_references.try_push_reserved(*vertex)?;
                            self.texcoord_channel.observe(vertex.texcoord.is_some());
                            self.normal_channel.observe(vertex.normal.is_some());
                            Ok(())
                        })?;
                    }
                }
                Ok(())
            },
        )?;
        self.attribute_stream_count = attribute_stream_count;
        Ok(())
    }

    fn materialize_geometry<'region>(
        &self,
        exec: &mut ParseExecution<'region, 'session>,
        groups: &MeshWorkGroups<'region>,
        parsed: &ParsedObj<'_>,
    ) -> ParseResult<BuiltGeometryStreams<'session>> {
        if self.corner_references.is_empty() {
            return Err(
                exec.format_failure(ObjFailure::internal("segment-has-no-vertex-references"))
            );
        }

        let channels = CornerChannelSelection {
            texcoords: self.texcoord_channel.is_complete(),
            normals: self.normal_channel.is_complete(),
        };
        let retained_amount = channels.retained_amount(exec)?;
        let total_attribute_stream_count = checked_attribute_stream_count(
            exec,
            self.attribute_stream_count,
            channels.retained_count(),
        )?;
        let corner_attribute_cell_count = self
            .corner_references
            .len()
            .checked_mul(channels.retained_count())
            .ok_or_else(|| exec.allocation_failure("obj-canonical-attribute-cells"))?;
        let corner_attribute_cell_amount = usize_amount(
            exec,
            corner_attribute_cell_count,
            "obj-canonical-attribute-cells",
        )?;
        exec.with_parent_work(
            &groups.corner_attributes,
            [retained_amount, corner_attribute_cell_amount],
            |exec, _parent| {
                self.materialize_admitted_geometry(
                    exec,
                    groups,
                    parsed,
                    channels,
                    total_attribute_stream_count,
                )
            },
        )
    }

    fn materialize_admitted_geometry<'region>(
        &self,
        exec: &mut ParseExecution<'region, 'session>,
        groups: &MeshWorkGroups<'region>,
        parsed: &ParsedObj<'_>,
        channels: CornerChannelSelection,
        total_attribute_stream_count: usize,
    ) -> ParseResult<BuiltGeometryStreams<'session>> {
        let corner_count = self.corner_references.len();
        let mut texcoord_lanes = 2_u8;
        let mut normals = if channels.normals {
            Some(BuiltCornerProjection::new(
                exec,
                corner_count,
                corner_count,
                "obj-segment-normal-source-values",
                "obj-segment-normal-source-indices",
                "obj-normal-source-index-init-work",
            )?)
        } else {
            None
        };
        let retained_channel_count = channels.retained_count();
        let mut sorted_positions = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-sorted-position-references",
        );
        sorted_positions.try_reserve(exec, corner_count)?;
        let channel_reference_count = corner_count
            .checked_mul(retained_channel_count)
            .ok_or_else(|| exec.allocation_failure("obj-corner-channel-reference-count"))?;
        let mut sorted_channels = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-sorted-corner-channel-references",
        );
        sorted_channels.try_reserve(exec, channel_reference_count)?;
        mtl::for_work_chunks(
            exec,
            corner_count,
            "obj-corner-reference-copy-work",
            |exec, range| {
                for occurrence in range {
                    let corner = self.corner_references.as_slice()[occurrence];
                    sorted_positions.try_push_reserved(IndexedPosition {
                        source_index: corner.position,
                        occurrence,
                    })?;
                    if channels.texcoords {
                        let source_index = corner.texcoord.ok_or_else(|| {
                            exec.format_failure(ObjFailure::internal(
                                "complete-texcoord-channel-has-missing-index",
                            ))
                        })?;
                        let source_value = parsed
                            .texcoords
                            .as_slice()
                            .get(source_index)
                            .ok_or_else(|| {
                                exec.format_failure(ObjFailure::internal(
                                    "obj-texcoord-source-index-out-of-range",
                                ))
                            })?;
                        texcoord_lanes = texcoord_lanes.max(source_value.lanes);
                        sorted_channels.try_push_reserved(IndexedCornerChannel {
                            channel: CornerChannel::Texcoord,
                            source_index,
                            occurrence,
                        })?;
                    }
                    if channels.normals {
                        let source_index = corner.normal.ok_or_else(|| {
                            exec.format_failure(ObjFailure::internal(
                                "complete-normal-channel-has-missing-index",
                            ))
                        })?;
                        sorted_channels.try_push_reserved(IndexedCornerChannel {
                            channel: CornerChannel::Normal,
                            source_index,
                            occurrence,
                        })?;
                    }
                }
                Ok(())
            },
        )?;
        let mut texcoords = if channels.texcoords {
            let scalar_count = corner_count
                .checked_mul(usize::from(texcoord_lanes))
                .ok_or_else(|| exec.allocation_failure("obj-texcoord-source-scalar-count"))?;
            Some(BuiltCornerProjection::new(
                exec,
                corner_count,
                scalar_count,
                "obj-segment-texcoord-source-values",
                "obj-segment-texcoord-source-indices",
                "obj-texcoord-source-index-init-work",
            )?)
        } else {
            None
        };
        mtl::interruptible_sort_by(exec, &mut sorted_positions, |_exec, _work, left, right| {
            Ok((left.source_index, left.occurrence).cmp(&(right.source_index, right.occurrence)))
        })?;
        mtl::interruptible_sort_by(exec, &mut sorted_channels, |_exec, _work, left, right| {
            Ok(left.cmp(right))
        })?;

        let mut first_position_occurrences = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-first-position-occurrences",
        );
        first_position_occurrences.try_reserve(exec, corner_count)?;
        mtl::for_work_chunks(
            exec,
            corner_count,
            "obj-first-position-occurrence-init-work",
            |exec, range| {
                first_position_occurrences.try_resize(exec, range.end, false)?;
                Ok(())
            },
        )?;

        let mut unique_position_count = 0usize;
        let mut previous_position = None;
        mtl::for_work_chunks(
            exec,
            sorted_positions.len(),
            "obj-position-grouping-work",
            |exec, range| {
                for cursor in range {
                    let indexed = sorted_positions.as_slice()[cursor];
                    if previous_position == Some(indexed.source_index) {
                        continue;
                    }
                    previous_position = Some(indexed.source_index);
                    first_position_occurrences.as_mut_slice()[indexed.occurrence] = true;
                    unique_position_count = unique_position_count
                        .checked_add(1)
                        .ok_or_else(|| exec.allocation_failure("obj-unique-position-count"))?;
                }
                Ok(())
            },
        )?;

        let mut positions = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-segment-position-scalars",
        );
        let mut indices = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-segment-indices",
        );
        let position_scalar_count = unique_position_count
            .checked_mul(3)
            .ok_or_else(|| exec.allocation_failure("obj-position-scalar-count"))?;
        positions.try_reserve(exec, position_scalar_count)?;
        indices.try_reserve(exec, corner_count)?;
        mtl::for_work_chunks(
            exec,
            corner_count,
            "obj-position-index-init-work",
            |exec, range| {
                indices.try_resize(exec, range.end, 0)?;
                Ok(())
            },
        )?;

        let mut previous_channel = None;
        let mut previous_source_index = None;
        let mut local_index = 0_u32;
        mtl::for_work_chunks(
            exec,
            sorted_channels.len(),
            "obj-corner-source-projection-work",
            |exec, range| {
                for cursor in range {
                    let reference = sorted_channels.as_slice()[cursor];
                    let is_new_source = previous_channel != Some(reference.channel)
                        || previous_source_index != Some(reference.source_index);
                    match reference.channel {
                        CornerChannel::Texcoord => {
                            let projection = texcoords.as_mut().ok_or_else(|| {
                                exec.format_failure(ObjFailure::internal(
                                    "texcoord-channel-projection-not-retained",
                                ))
                            })?;
                            if is_new_source {
                                let source_value = parsed
                                    .texcoords
                                    .as_slice()
                                    .get(reference.source_index)
                                    .copied()
                                    .ok_or_else(|| {
                                        exec.format_failure(ObjFailure::internal(
                                            "obj-texcoord-source-index-out-of-range",
                                        ))
                                    })?;
                                local_index = canonical_u32(
                                    exec,
                                    projection.source_values.len() / usize::from(texcoord_lanes),
                                    "obj-texcoord-source-index-space",
                                )?;
                                projection.source_values.try_extend_from_slice_reserved(
                                    &source_value.values[..usize::from(texcoord_lanes)],
                                )?;
                            }
                            let destination = projection
                                .source_indices
                                .as_mut_slice()
                                .get_mut(reference.occurrence)
                                .ok_or_else(|| {
                                    exec.format_failure(ObjFailure::internal(
                                        "obj-texcoord-corner-index-out-of-range",
                                    ))
                                })?;
                            *destination = local_index;
                        }
                        CornerChannel::Normal => {
                            let projection = normals.as_mut().ok_or_else(|| {
                                exec.format_failure(ObjFailure::internal(
                                    "normal-channel-projection-not-retained",
                                ))
                            })?;
                            if is_new_source {
                                let source_value = parsed
                                    .normals
                                    .as_slice()
                                    .get(reference.source_index)
                                    .copied()
                                    .ok_or_else(|| {
                                        exec.format_failure(ObjFailure::internal(
                                            "obj-normal-source-index-out-of-range",
                                        ))
                                    })?;
                                local_index = canonical_u32(
                                    exec,
                                    projection.source_values.len(),
                                    "obj-normal-source-index-space",
                                )?;
                                projection.source_values.try_push_reserved(source_value)?;
                            }
                            let destination = projection
                                .source_indices
                                .as_mut_slice()
                                .get_mut(reference.occurrence)
                                .ok_or_else(|| {
                                    exec.format_failure(ObjFailure::internal(
                                        "obj-normal-corner-index-out-of-range",
                                    ))
                                })?;
                            *destination = local_index;
                        }
                    }
                    previous_channel = Some(reference.channel);
                    previous_source_index = Some(reference.source_index);
                }
                Ok(())
            },
        )?;
        drop(sorted_channels);

        for (chunk_index, first_flags) in
            first_position_occurrences.as_slice().chunks(64).enumerate()
        {
            let start = chunk_index
                .checked_mul(64)
                .ok_or_else(|| exec.allocation_failure("obj-geometry-materialization-offset"))?;
            let end = start
                .checked_add(first_flags.len())
                .ok_or_else(|| exec.allocation_failure("obj-geometry-materialization-offset"))?;
            let chunk_positions = first_flags.iter().filter(|is_first| **is_first).count();
            exec.with_work(
                &groups.materialization,
                [
                    usize_amount(exec, first_flags.len(), "obj-geometry-materialization-work")?,
                    usize_amount(exec, chunk_positions, "obj-canonical-vertices")?,
                ],
                |exec, work| -> ParseResult<()> {
                    for occurrence in start..end {
                        let corner = self.corner_references.as_slice()[occurrence];
                        let is_first_position = first_position_occurrences.as_slice()[occurrence];
                        let position =
                            is_first_position.then(|| parsed.positions.as_slice()[corner.position]);
                        let vertex_amount = u64::from(is_first_position);
                        work.run([1, vertex_amount], || {
                            if let Some(position) = position {
                                let canonical_index = canonical_u32(
                                    exec,
                                    positions.len() / 3,
                                    "obj-vertex-index-space",
                                )?;
                                indices.as_mut_slice()[occurrence] = canonical_index;
                                positions.try_extend_from_slice_reserved(&[
                                    position.x, position.y, position.z,
                                ])?;
                            }
                            Ok(())
                        })?;
                    }
                    Ok(())
                },
            )?;
        }

        let mut current_position = None;
        let mut canonical_index = 0u32;
        mtl::for_work_chunks(
            exec,
            sorted_positions.len(),
            "obj-position-remap-work",
            |_exec, range| {
                for cursor in range {
                    let indexed = sorted_positions.as_slice()[cursor];
                    if current_position != Some(indexed.source_index) {
                        current_position = Some(indexed.source_index);
                        canonical_index = indices.as_slice()[indexed.occurrence];
                    } else {
                        indices.as_mut_slice()[indexed.occurrence] = canonical_index;
                    }
                }
                Ok(())
            },
        )?;

        drop(first_position_occurrences);
        drop(sorted_positions);
        Ok(BuiltGeometryStreams {
            positions,
            texcoords,
            texcoord_lanes,
            normals,
            indices,
            attribute_stream_count: total_attribute_stream_count,
        })
    }

    fn finish<'region>(
        self,
        exec: &mut ParseExecution<'region, 'session>,
        groups: &MeshWorkGroups<'region>,
        parsed: &ParsedObj<'_>,
        material: Option<MaterialLocalId>,
    ) -> ParseResult<BuiltSegment<'session>> {
        let streams = self.materialize_geometry(exec, groups, parsed)?;
        let SegmentBuilder {
            key,
            corner_references,
            face_vertex_counts,
            smoothing_groups,
            attribute_stream_count: _,
            texcoord_channel: _,
            normal_channel: _,
            has_polygon,
        } = self;
        drop(corner_references);
        let BuiltGeometryStreams {
            positions,
            texcoords,
            texcoord_lanes,
            normals,
            indices,
            attribute_stream_count,
        } = streams;
        let face_vertex_counts = if has_polygon {
            face_vertex_counts
        } else {
            drop(face_vertex_counts);
            exec.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-face-vertex-counts",
            )
        };
        Ok(BuiltSegment {
            key,
            positions,
            texcoords,
            texcoord_lanes,
            normals,
            indices,
            face_vertex_counts,
            smoothing_groups,
            attribute_stream_count,
            has_polygon,
            material,
        })
    }
}

struct BuiltGeometryStreams<'session> {
    positions: MeteredVec<'session, f32>,
    texcoords: Option<BuiltCornerProjection<'session, f32>>,
    texcoord_lanes: u8,
    normals: Option<BuiltCornerProjection<'session, [f32; 3]>>,
    indices: MeteredVec<'session, u32>,
    attribute_stream_count: usize,
}

struct BuiltSegment<'session> {
    key: SegmentKey,
    positions: MeteredVec<'session, f32>,
    texcoords: Option<BuiltCornerProjection<'session, f32>>,
    texcoord_lanes: u8,
    normals: Option<BuiltCornerProjection<'session, [f32; 3]>>,
    indices: MeteredVec<'session, u32>,
    face_vertex_counts: MeteredVec<'session, u32>,
    smoothing_groups: Option<MeteredVec<'session, u32>>,
    attribute_stream_count: usize,
    has_polygon: bool,
    material: Option<MaterialLocalId>,
}

pub(crate) fn scene_from_parsed<'session>(
    context: &mut ParseContext<'session>,
    parsed: ParsedObj<'session>,
    library: &MaterialLibrary<'session>,
) -> ParseResult<()> {
    let (materials, segments) = context.with_execution(|exec| {
        if parsed.faces.is_empty() {
            return Err(exec.format_failure(ObjFailure::empty_geometry()));
        }

        let material_projection = prepare_material_bindings(exec, library, &parsed)?;
        let groups = MeshWorkGroups::bind(exec)?;
        // The complete batch is borrowed authoring scratch. Core alone owns canonical-output
        // reservations when activation materializes and jointly publishes the retained Scene.
        let mut segments = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-segments",
        );
        let mut current: Option<SegmentBuilder<'session>> = None;

        for face in parsed.faces.as_slice() {
            let key = SegmentKey::from_face(face);
            let should_flush = current.as_ref().is_some_and(|segment| segment.key != key);
            if should_flush && let Some(segment) = current.take() {
                flush_segment(
                    exec,
                    &groups,
                    &mut segments,
                    &material_projection.bindings,
                    &parsed,
                    segment,
                )?;
            }
            if current.is_none() {
                current = Some(SegmentBuilder::new(exec, &groups, key)?);
            }
            let Some(segment) = current.as_mut() else {
                return Err(exec.format_failure(ObjFailure::internal(
                    "mesh-segment-missing-after-initialization",
                )));
            };
            segment.push_face(exec, &groups, face)?;
        }

        if let Some(segment) = current {
            flush_segment(
                exec,
                &groups,
                &mut segments,
                &material_projection.bindings,
                &parsed,
                segment,
            )?;
        }

        Ok::<_, baozi_import::ParseFailure>((material_projection.materials, segments))
    })?;
    submit_complete_batch(context, &parsed, &materials, &segments)
}

fn flush_segment<'region, 'session>(
    exec: &mut ParseExecution<'region, 'session>,
    groups: &MeshWorkGroups<'region>,
    segments: &mut MeteredVec<'session, BuiltSegment<'session>>,
    material_bindings: &MaterialBindings<'session, '_>,
    parsed: &ParsedObj<'_>,
    segment: SegmentBuilder<'session>,
) -> ParseResult<()> {
    record_partial_attribute_losses(
        exec,
        segments.len(),
        segment.texcoord_channel.is_partial(),
        segment.normal_channel.is_partial(),
    )?;
    let material = if let Some(id) = segment.key.material {
        let name = state_name(exec, parsed, id)?;
        Some(material_bindings.find(exec, name)?.ok_or_else(|| {
            exec.format_failure(ObjFailure::internal(
                "segment-material-missing-from-prepared-bindings",
            ))
        })?)
    } else {
        None
    };
    let built = segment.finish(exec, groups, parsed, material)?;
    exec.with_work(&groups.meshes, [1], |exec, work| {
        work.run_all(|| segments.try_push(exec, built))
    })
}

#[derive(Clone, Copy)]
struct MaterialBinding<'parsed> {
    name: &'parsed str,
    first_use: usize,
    id: Option<MaterialLocalId>,
}

struct MaterialBindings<'session, 'parsed> {
    values: MeteredVec<'session, MaterialBinding<'parsed>>,
}

impl MaterialBindings<'_, '_> {
    fn find(
        &self,
        exec: &mut ParseExecution<'_, '_>,
        name: &str,
    ) -> ParseResult<Option<MaterialLocalId>> {
        let index = mtl::interruptible_binary_search_by(
            exec,
            self.values.as_slice(),
            |exec, work, binding| work.compare_str(exec, binding.name, name),
        )?;
        Ok(index.and_then(|index| self.values.as_slice()[index].id))
    }
}

struct BuiltMaterial<'input, 'session> {
    name: &'input str,
    shading_model: CanonicalShadingModel,
    alpha_mode: AlphaMode,
    factors: MaterialFactors,
    metadata: Option<&'input ParsedMetadata<'session>>,
    diffuse_texture: Option<BuiltTexture<'session>>,
}

struct BuiltTexture<'session> {
    identity: ResourceIdentity,
    snapshot: DependencySnapshot<'session>,
    transform: TextureTransform,
    sampler: SamplerState,
}

struct MaterialProjection<'session, 'input> {
    bindings: MaterialBindings<'session, 'input>,
    materials: MeteredVec<'session, BuiltMaterial<'input, 'session>>,
}

fn prepare_material_bindings<'session, 'input>(
    exec: &mut ParseExecution<'_, 'session>,
    library: &'input MaterialLibrary<'session>,
    parsed: &'input ParsedObj<'session>,
) -> ParseResult<MaterialProjection<'session, 'input>> {
    let faces = parsed.faces.as_slice();
    let mut values = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-material-bindings",
    );
    mtl::for_work_chunks(
        exec,
        faces.len(),
        "obj-material-binding-scan-work",
        |exec, range| {
            for first_use in range {
                let Some(id) = faces[first_use].material else {
                    continue;
                };
                let name = state_name(exec, parsed, id)?;
                values.try_push(
                    exec,
                    MaterialBinding {
                        name,
                        first_use,
                        id: None,
                    },
                )?;
            }
            Ok(())
        },
    )?;

    mtl::interruptible_sort_by(exec, &mut values, |exec, work, left, right| {
        Ok(work
            .compare_str(exec, left.name, right.name)?
            .then_with(|| left.first_use.cmp(&right.first_use)))
    })?;
    mtl::interruptible_dedup_by(exec, &mut values, |exec, work, previous, current| {
        Ok(work.compare_str(exec, previous.name, current.name)? == std::cmp::Ordering::Equal)
    })?;
    mtl::interruptible_sort_by(exec, &mut values, |_exec, _work, left, right| {
        Ok(left.first_use.cmp(&right.first_use))
    })?;

    let mut materials = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "obj-complete-batch-materials",
    );
    materials.try_reserve(exec, values.len())?;
    let binding_work = exec.bind_work_group([ImportWorkClass::WorkUnits])?;
    let mut start = 0usize;
    while start < values.len() {
        let end = start.saturating_add(64).min(values.len());
        let mut built_ids: [Option<MaterialLocalId>; 64] = std::array::from_fn(|_| None);
        for (offset, index) in (start..end).enumerate() {
            let name = values.as_slice()[index].name;
            let material = build_material(exec, library, name)?;
            let id = canonical_u32(exec, materials.len(), "obj-material-local-id")?;
            materials.try_push_reserved(material)?;
            built_ids[offset] = Some(MaterialLocalId::new(id));
        }
        exec.with_work(
            &binding_work,
            [usize_amount(
                exec,
                end - start,
                "obj-material-binding-build-work",
            )?],
            |exec, work| -> ParseResult<()> {
                for (offset, index) in (start..end).enumerate() {
                    let id = built_ids[offset].take().ok_or_else(|| {
                        exec.format_failure(ObjFailure::internal(
                            "material-binding-build-result-missing",
                        ))
                    })?;
                    work.run([1], || {
                        values.as_mut_slice()[index].id = Some(id);
                        Ok(())
                    })?;
                }
                Ok(())
            },
        )?;
        start = end;
    }
    mtl::interruptible_sort_by(exec, &mut values, |exec, work, left, right| {
        work.compare_str(exec, left.name, right.name)
    })?;
    Ok(MaterialProjection {
        bindings: MaterialBindings { values },
        materials,
    })
}

fn build_material<'input, 'session>(
    exec: &mut ParseExecution<'_, 'session>,
    library: &'input MaterialLibrary<'session>,
    name: &'input str,
) -> ParseResult<BuiltMaterial<'input, 'session>> {
    if let Some(parsed) = library.find(exec, name)? {
        material_from_parsed(exec, library, parsed)
    } else {
        mtl::push_warning(
            exec,
            None,
            "obj.material_missing",
            format_args!("OBJ material '{name}' was referenced but not defined"),
        )?;
        material_from_factors(
            exec,
            name,
            CanonicalShadingModel::Phong,
            Vec3::ZERO,
            Color::WHITE,
            Vec3::ZERO,
            Color::linear_rgba(0.0, 0.0, 0.0, 1.0),
            0.0,
            None,
            None,
        )
    }
}

fn material_from_parsed<'input, 'session>(
    exec: &mut ParseExecution<'_, 'session>,
    library: &'input MaterialLibrary<'session>,
    parsed: &'input ParsedMaterial<'session>,
) -> ParseResult<BuiltMaterial<'input, 'session>> {
    let diffuse_texture = parsed
        .diffuse_texture
        .as_ref()
        .map(|texture| load_diffuse_texture(exec, library, texture))
        .transpose()?
        .flatten();

    material_from_factors(
        exec,
        parsed.name.as_str(),
        CanonicalShadingModel::Phong,
        parsed.ambient,
        parsed.base_color,
        parsed.specular,
        parsed.emissive,
        parsed_shininess(&parsed.metadata),
        Some(&parsed.metadata),
        diffuse_texture,
    )
}

fn load_diffuse_texture<'session>(
    exec: &mut ParseExecution<'_, 'session>,
    library: &MaterialLibrary<'session>,
    texture: &ParsedTexture<'_>,
) -> ParseResult<Option<BuiltTexture<'session>>> {
    let origin = library
        .origin(texture.origin)
        .ok_or_else(|| exec.format_failure(ObjFailure::internal("texture-origin-missing")))?;
    let outcome = exec.dependency(origin, &diffuse_texture_role(), texture.reference.as_str())?;
    let snapshot = match outcome.into_result() {
        Ok(snapshot) => snapshot,
        Err(evidence) => {
            mtl::push_warning(
                exec,
                None,
                "obj.mtl_texture_unavailable",
                format_args!(
                    "optional diffuse texture '{}' is unavailable; owner={:?}, cache={:?}",
                    evidence.normalized_path(),
                    evidence.owner(),
                    evidence.cache_disposition()
                ),
            )?;
            return Ok(None);
        }
    };
    let identity = ResourceIdentity::new(snapshot.identity().path().as_str())
        .map_err(|_| exec.format_failure(ObjFailure::internal("texture-identity-invalid")))?;
    Ok(Some(BuiltTexture {
        identity,
        snapshot,
        transform: texture.transform,
        sampler: texture.sampler,
    }))
}

#[allow(clippy::too_many_arguments)]
fn material_from_factors<'input, 'session>(
    exec: &mut ParseExecution<'_, '_>,
    name: &'input str,
    shading_model: CanonicalShadingModel,
    ambient: Vec3,
    base_color: Color,
    specular: Vec3,
    emissive: Color,
    shininess: f32,
    metadata: Option<&'input ParsedMetadata<'session>>,
    diffuse_texture: Option<BuiltTexture<'session>>,
) -> ParseResult<BuiltMaterial<'input, 'session>> {
    let legacy = LegacyMaterialFactors::try_new(
        ambient,
        Vec3::new(base_color.r, base_color.g, base_color.b),
        specular,
        Vec3::new(emissive.r, emissive.g, emissive.b),
        shininess,
        base_color.a,
    )
    .map_err(|_| exec.format_failure(ObjFailure::internal("invalid-material-factors")))?;
    let alpha_mode = if legacy.opacity() < 1.0 {
        AlphaMode::Blend
    } else {
        AlphaMode::Opaque
    };
    let emission = baozi_import::authoring::EmissionFactors::try_new(
        Vec3::new(emissive.r, emissive.g, emissive.b),
        None,
    )
    .map_err(|_| exec.format_failure(ObjFailure::internal("invalid-material-factors")))?;
    let factors = MaterialFactors::empty()
        .with_emission(emission)
        .with_legacy(Some(legacy));
    Ok(BuiltMaterial {
        name,
        shading_model,
        alpha_mode,
        factors,
        metadata,
        diffuse_texture,
    })
}

fn parsed_shininess(metadata: &ParsedMetadata<'_>) -> f32 {
    metadata
        .iter()
        .find_map(|(key, value)| match (key, value) {
            ("obj:ns", ParsedMetadataValue::F64(value)) => Some(*value as f32),
            _ => None,
        })
        .unwrap_or(0.0)
}

#[derive(Clone, Copy)]
enum BorrowedMetadataValue<'input> {
    I64(i64),
    F64(f64),
    String(&'input str),
}

impl<'input> BorrowedMetadataValue<'input> {
    const fn as_extension(self) -> ExtensionValueRef<'input> {
        match self {
            Self::I64(value) => ExtensionValueRef::I64(value),
            Self::F64(value) => ExtensionValueRef::F64(value),
            Self::String(value) => ExtensionValueRef::String(value),
        }
    }
}

struct PendingMetadata<'input> {
    owner: ProfileOwner,
    key: NamespacedName,
    value: BorrowedMetadataValue<'input>,
}

fn submit_complete_batch<'session>(
    context: &mut ParseContext<'session>,
    parsed: &ParsedObj<'session>,
    materials: &MeteredVec<'session, BuiltMaterial<'_, 'session>>,
    segments: &MeteredVec<'session, BuiltSegment<'session>>,
) -> ParseResult<()> {
    let segment_count = segments.len();
    let (
        roots,
        nodes,
        logical_meshes,
        primitives,
        attributes,
        mesh_instances,
        logical_morphs,
        primitive_deformations,
        mesh_instance_deformations,
    ) = context.with_execution(|exec| {
        let mut roots = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-roots",
        );
        let mut nodes = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-nodes",
        );
        let mut logical_meshes = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-logical-meshes",
        );
        let mut primitives = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-primitives",
        );
        let mut attributes = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-attributes",
        );
        let mut mesh_instances = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-mesh-instances",
        );
        let mut logical_morphs = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-logical-morphs",
        );
        let mut primitive_deformations = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-primitive-deformations",
        );
        let mut mesh_instance_deformations = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-instance-deformations",
        );

        roots.try_reserve(exec, segment_count)?;
        nodes.try_reserve(exec, segment_count)?;
        logical_meshes.try_reserve(exec, segment_count)?;
        primitives.try_reserve(exec, segment_count)?;
        mesh_instances.try_reserve(exec, segment_count)?;
        logical_morphs.try_reserve(exec, segment_count)?;
        primitive_deformations.try_reserve(exec, segment_count)?;
        mesh_instance_deformations.try_reserve(exec, segment_count)?;
        let attribute_count = segments
            .as_slice()
            .iter()
            .try_fold(0usize, |total, segment| {
                total
                    .checked_add(segment.attribute_stream_count)
                    .ok_or_else(|| exec.allocation_failure("obj-attribute-count"))
            })?;
        attributes.try_reserve(exec, attribute_count)?;

        for (index, segment) in segments.as_slice().iter().enumerate() {
            let local = canonical_u32(exec, index, "obj-node-local-id")?;
            let node_id = NodeLocalId::new(local);
            roots.try_push_reserved(node_id)?;
            let mut node = NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            );
            if let Some(name) = segment
                .key
                .display_name()
                .map(|id| state_name(exec, parsed, id))
                .transpose()?
                .and_then(bounded_entity_name)
            {
                node = node.with_name(name);
            }
            nodes.try_push_reserved(node)?;

            let attribute_start = attributes.len();
            let attribute_offset = canonical_u32(exec, attribute_start, "obj-attribute-offset")?;
            attributes.try_push_reserved(GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                segment.positions.as_slice(),
                Presence::Absent,
            ))?;
            if let Some(projection) = segment.normals.as_ref() {
                attributes.try_push_reserved(GeometryAttributeAuthoring::f32(
                    AttributeSemantic::Normal,
                    AttributeDomain::Corner,
                    3,
                    projection.source_values.as_slice().as_flattened(),
                    Presence::Present(projection.source_indices.as_slice()),
                ))?;
            }
            if let Some(projection) = segment.texcoords.as_ref() {
                attributes.try_push_reserved(GeometryAttributeAuthoring::f32(
                    AttributeSemantic::Texcoord(0),
                    AttributeDomain::Corner,
                    segment.texcoord_lanes,
                    projection.source_values.as_slice(),
                    Presence::Present(projection.source_indices.as_slice()),
                ))?;
            }
            if let Some(smoothing_groups) = segment.smoothing_groups.as_ref() {
                attributes.try_push_reserved(GeometryAttributeAuthoring::u32(
                    AttributeSemantic::SmoothingGroup,
                    AttributeDomain::Face,
                    1,
                    smoothing_groups.as_slice(),
                ))?;
            }
            let authored_attribute_count = attributes.len() - attribute_start;
            if authored_attribute_count != segment.attribute_stream_count {
                return Err(exec.format_failure(ObjFailure::internal(
                    "segment-attribute-stream-count-mismatch",
                )));
            }
            let attribute_range = AuthoringRange::new(
                attribute_offset,
                canonical_u32(exec, authored_attribute_count, "obj-attribute-range-count")?,
            );
            let topology = if segment.has_polygon {
                GeometryTopologyAuthoring::Polygons(segment.face_vertex_counts.as_slice())
            } else {
                GeometryTopologyAuthoring::TriangleList
            };
            primitives.try_push_reserved(GeometryPrimitiveAuthoring::new(
                topology,
                Presence::Present(segment.indices.as_slice()),
                segment.material.map_or(
                    baozi_import::authoring::MaterialAssignment::Unspecified,
                    baozi_import::authoring::MaterialAssignment::Authored,
                ),
                attribute_range,
            ))?;
            let mut logical_mesh = GeometryLogicalMeshAuthoring::new(AuthoringRange::new(local, 1));
            if let Some(name) = segment
                .key
                .display_name()
                .map(|id| state_name(exec, parsed, id))
                .transpose()?
                .and_then(bounded_entity_name)
            {
                logical_mesh = logical_mesh.with_name(name);
            }
            logical_meshes.try_push_reserved(logical_mesh)?;
            mesh_instances.try_push_reserved(GeometryMeshInstanceAuthoring::new(
                node_id,
                LogicalMeshLocalId::new(local),
            ))?;
            logical_morphs
                .try_push_reserved(LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[]))?;
            primitive_deformations
                .try_push_reserved(PrimitiveDeformationAuthoring::new(Presence::Absent, &[]))?;
            mesh_instance_deformations.try_push_reserved(MeshInstanceDeformationAuthoring::new(
                Presence::Absent,
                Presence::Absent,
            ))?;
        }

        Ok::<_, baozi_import::ParseFailure>((
            roots,
            nodes,
            logical_meshes,
            primitives,
            attributes,
            mesh_instances,
            logical_morphs,
            primitive_deformations,
            mesh_instance_deformations,
        ))
    })?;

    let texture_count = materials
        .as_slice()
        .iter()
        .filter(|material| material.diffuse_texture.is_some())
        .count();
    let source_key = context.with_execution(|exec| {
        TextureSourceKey::try_new("map_Kd")
            .map_err(|_| exec.format_failure(ObjFailure::internal("invalid-texture-source-key")))
    })?;
    let (images, samplers, textures, material_bindings, material_authoring) = context
        .with_execution(|exec| {
            let mut images = exec.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-complete-batch-images",
            );
            let mut samplers = exec.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-complete-batch-samplers",
            );
            let mut textures = exec.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-complete-batch-textures",
            );
            let mut material_bindings = exec.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-complete-batch-material-bindings",
            );
            let mut material_authoring = exec.metered_vec(
                ImportStorageClass::FormatNormalization,
                "obj-complete-batch-material-authoring",
            );
            images.try_reserve(exec, texture_count)?;
            samplers.try_reserve(exec, texture_count)?;
            textures.try_reserve(exec, texture_count)?;
            material_bindings.try_reserve(exec, texture_count)?;
            material_authoring.try_reserve(exec, materials.len())?;

            let texture_role = TextureRole::Diffuse;
            let channel_mapping = TextureChannelSwizzle::RGBA;
            let color_interpretation = ColorInterpretation::SrgbColor;
            let blend = TextureBlendOperation::Replace;

            for material in materials.as_slice() {
                let binding_offset =
                    canonical_u32(exec, material_bindings.len(), "obj-material-binding-offset")?;
                let binding_count = if let Some(texture) = &material.diffuse_texture {
                    let image_index = canonical_u32(exec, images.len(), "obj-image-local-id")?;
                    let sampler_index =
                        canonical_u32(exec, samplers.len(), "obj-sampler-local-id")?;
                    let texture_index =
                        canonical_u32(exec, textures.len(), "obj-texture-local-id")?;
                    let resource_index = image_index
                        .checked_add(1)
                        .ok_or_else(|| exec.allocation_failure("obj-texture-resource-local-id"))?;
                    images.try_push_reserved(ImageAuthoring::session_resource(
                        ResourceIdentityLocalId::new(resource_index),
                        Presence::Absent,
                    ))?;
                    samplers.try_push_reserved(SamplerAuthoring::new(texture.sampler))?;
                    textures.try_push_reserved(TextureAuthoring::new(
                        ImageLocalId::new(image_index),
                        SamplerLocalId::new(sampler_index),
                    ))?;
                    material_bindings.try_push_reserved(
                        MaterialBindingAuthoring::try_from_parts(
                            TextureLocalId::new(texture_index),
                            texture_role,
                            0,
                            channel_mapping,
                            color_interpretation,
                            texture.transform,
                            TextureStrength::ONE,
                            blend,
                            Some(&source_key),
                        )
                        .map_err(|_| {
                            exec.format_failure(ObjFailure::internal("invalid-texture-binding"))
                        })?,
                    )?;
                    1
                } else {
                    0
                };
                let authored = MaterialAuthoring::try_from_parts(
                    AuthoringRange::new(binding_offset, binding_count),
                    material.shading_model,
                    material.alpha_mode,
                    false,
                    material.factors,
                )
                .map_err(|_| {
                    exec.format_failure(ObjFailure::internal("invalid-material-authoring"))
                })?
                .with_name(material.name);
                material_authoring.try_push_reserved(authored)?;
            }

            Ok::<_, baozi_import::ParseFailure>((
                images,
                samplers,
                textures,
                material_bindings,
                material_authoring,
            ))
        })?;

    let pending_metadata = context.with_execution(|exec| {
        let mut pending_metadata = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-pending-metadata",
        );
        for (index, material) in materials.as_slice().iter().enumerate() {
            if let Some(metadata) = material.metadata {
                append_material_metadata(exec, &mut pending_metadata, index, metadata)?;
            }
        }
        for (index, segment) in segments.as_slice().iter().enumerate() {
            append_segment_metadata(exec, &mut pending_metadata, parsed, index, segment.key)?;
        }
        Ok::<_, baozi_import::ParseFailure>(pending_metadata)
    })?;
    let metadata_entries = context.with_execution(|exec| {
        let mut metadata_entries = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-metadata-authoring",
        );
        metadata_entries.try_reserve(exec, pending_metadata.len())?;
        for metadata in pending_metadata.as_slice() {
            metadata_entries.try_push_reserved(MetadataAuthoringEntry::new(
                metadata.owner,
                &metadata.key,
                metadata.value.as_extension(),
            ))?;
        }

        Ok::<_, baozi_import::ParseFailure>(metadata_entries)
    })?;

    let source_provenance = [ProvenanceLocalId::new(0)];
    let (resource_identity, resource_stability, format) = context.with_execution(|exec| {
        let (identity, stability) = profile_resource_identity(exec)?;
        let format = FormatId::new(FORMAT_ID)
            .map_err(|_| exec.allocation_failure("obj-profile-format-id"))?;
        Ok::<_, baozi_import::ParseFailure>((identity, stability, format))
    })?;
    let (resources, resource_snapshots, source_locators) = context.with_execution(|exec| {
        let resource_count = texture_count
            .checked_add(1)
            .ok_or_else(|| exec.allocation_failure("obj-profile-resource-count"))?;
        let mut resources = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-resources",
        );
        let mut resource_snapshots = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-resource-snapshots",
        );
        let mut source_locators = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-source-locators",
        );
        resources.try_reserve(exec, resource_count)?;
        resource_snapshots.try_reserve(exec, resource_count)?;
        source_locators.try_reserve(exec, resource_count)?;
        resources.try_push_reserved(ObservedResourceAuthoring::session_opaque(
            &resource_identity,
            resource_stability,
        ))?;
        resource_snapshots.try_push_reserved(ResourceSnapshotAuthoring::absent())?;
        source_locators
            .try_push_reserved(SourceObjectLocator::indexed(SourceObjectKind::Document, 0))?;
        for (image_index, texture) in materials
            .as_slice()
            .iter()
            .filter_map(|material| material.diffuse_texture.as_ref())
            .enumerate()
        {
            resources.try_push_reserved(ObservedResourceAuthoring::session_opaque(
                &texture.identity,
                SessionOpaqueReason::UnregisteredResolver,
            ))?;
            resource_snapshots.try_push_reserved(ResourceSnapshotAuthoring::snapshotted(
                texture.snapshot.bytes(),
            ))?;
            source_locators.try_push_reserved(SourceObjectLocator::indexed(
                SourceObjectKind::Image,
                u64::try_from(image_index)
                    .map_err(|_| exec.allocation_failure("obj-profile-image-locator"))?,
            ))?;
        }

        Ok::<_, baozi_import::ParseFailure>((resources, resource_snapshots, source_locators))
    })?;

    let (source_identities, provenance_nodes) = context.with_execution(|exec| {
        let resource_count = source_locators.len();
        let mut source_identities = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-source-identities",
        );
        let mut provenance_nodes = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-provenance-nodes",
        );
        source_identities.try_reserve(exec, resource_count)?;
        provenance_nodes.try_reserve(exec, resource_count)?;
        for (index, locator) in source_locators.as_slice().iter().enumerate() {
            let local = canonical_u32(exec, index, "obj-profile-source-identity")?;
            source_identities.try_push_reserved(SourceIdentityAuthoring::new(
                ResourceIdentityLocalId::new(local),
                &format,
                locator,
            ))?;
            provenance_nodes.try_push_reserved(ProvenanceNodeAuthoring::source(
                SourceIdentityLocalId::new(local),
            ))?;
        }

        Ok::<_, baozi_import::ParseFailure>((source_identities, provenance_nodes))
    })?;

    let appearance_provenance = context.with_execution(|exec| {
        let mut appearance_provenance = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-appearance-provenance",
        );
        appearance_provenance.try_reserve(exec, texture_count)?;
        for index in 0..texture_count {
            let resource = canonical_u32(
                exec,
                index
                    .checked_add(1)
                    .ok_or_else(|| exec.allocation_failure("obj-texture-provenance"))?,
                "obj-texture-provenance",
            )?;
            appearance_provenance.try_push_reserved(ProvenanceLocalId::new(resource))?;
        }
        Ok::<_, baozi_import::ParseFailure>(appearance_provenance)
    })?;

    let owner_provenance = context.with_execution(|exec| {
        let appearance_owner_count = texture_count
            .checked_mul(4)
            .ok_or_else(|| exec.allocation_failure("obj-profile-appearance-owner-count"))?;
        let owner_count = 2usize
            .checked_add(nodes.len())
            .and_then(|count| count.checked_add(appearance_owner_count))
            .and_then(|count| count.checked_add(material_authoring.len()))
            .and_then(|count| count.checked_add(logical_meshes.len()))
            .and_then(|count| count.checked_add(primitives.len()))
            .and_then(|count| count.checked_add(attributes.len()))
            .and_then(|count| count.checked_add(mesh_instances.len()))
            .ok_or_else(|| exec.allocation_failure("obj-profile-owner-count"))?;
        let mut owner_provenance = exec.metered_vec(
            ImportStorageClass::FormatNormalization,
            "obj-complete-batch-owner-provenance",
        );
        owner_provenance.try_reserve(exec, owner_count)?;
        owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
            ProfileOwner::Scene,
            &source_provenance,
        ))?;
        owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
            ProfileOwner::SyntheticRoot,
            &source_provenance,
        ))?;
        for index in 0..nodes.len() {
            owner_provenance.try_push_reserved(OwnerProvenanceAuthoring::new(
                ProfileOwner::Node(NodeLocalId::new(canonical_u32(
                    exec,
                    index,
                    "obj-profile-node-owner",
                )?)),
                &source_provenance,
            ))?;
        }
        for index in 0..texture_count {
            let local = canonical_u32(exec, index, "obj-profile-appearance-owner")?;
            let provenance = &appearance_provenance.as_slice()[index..=index];
            for owner in [
                ProfileOwner::Image(local),
                ProfileOwner::Sampler(local),
                ProfileOwner::Texture(local),
                ProfileOwner::TextureBinding(local),
            ] {
                owner_provenance
                    .try_push_reserved(OwnerProvenanceAuthoring::new(owner, provenance))?;
            }
        }
        push_indexed_owners(
            exec,
            &mut owner_provenance,
            material_authoring.len(),
            "obj-profile-material-owner",
            ProfileOwner::Material,
            &source_provenance,
        )?;
        push_indexed_owners(
            exec,
            &mut owner_provenance,
            logical_meshes.len(),
            "obj-profile-logical-mesh-owner",
            ProfileOwner::LogicalMesh,
            &source_provenance,
        )?;
        push_indexed_owners(
            exec,
            &mut owner_provenance,
            primitives.len(),
            "obj-profile-primitive-owner",
            ProfileOwner::Primitive,
            &source_provenance,
        )?;
        push_indexed_owners(
            exec,
            &mut owner_provenance,
            attributes.len(),
            "obj-profile-attribute-owner",
            ProfileOwner::Attribute,
            &source_provenance,
        )?;
        push_indexed_owners(
            exec,
            &mut owner_provenance,
            mesh_instances.len(),
            "obj-profile-mesh-instance-owner",
            ProfileOwner::MeshInstance,
            &source_provenance,
        )?;

        Ok::<_, baozi_import::ParseFailure>(owner_provenance)
    })?;

    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let node_batch = NodeAuthoringBatch::new(Presence::Present(roots.as_slice()), nodes.as_slice());
    let appearance = AppearanceAuthoringBatch::new(
        images.as_slice(),
        samplers.as_slice(),
        textures.as_slice(),
        material_bindings.as_slice(),
        material_authoring.as_slice(),
    );
    let geometry = GeometryAuthoringBatch::new(
        logical_meshes.as_slice(),
        primitives.as_slice(),
        attributes.as_slice(),
        mesh_instances.as_slice(),
    );
    let deformation = DeformationAuthoringBatch::new(
        &[],
        &[],
        logical_morphs.as_slice(),
        primitive_deformations.as_slice(),
        mesh_instance_deformations.as_slice(),
    );
    let animations = AnimationAuthoringBatch::empty();
    let profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(
            resources.as_slice(),
            source_identities.as_slice(),
            provenance_nodes.as_slice(),
            owner_provenance.as_slice(),
        ),
        MetadataAuthoringBatch::new(metadata_entries.as_slice()),
    )
    .with_resource_snapshots(resource_snapshots.as_slice());
    let input = SceneAuthoringBatch::new(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
        profile,
    );
    context.submit_complete(input)
}

fn append_material_metadata<'session, 'input>(
    exec: &mut ParseExecution<'_, 'session>,
    output: &mut MeteredVec<'session, PendingMetadata<'input>>,
    material: usize,
    metadata: &'input ParsedMetadata<'session>,
) -> ParseResult<()> {
    let owner = ProfileOwner::Material(canonical_u32(
        exec,
        material,
        "obj-metadata-material-owner",
    )?);
    for (key, value) in metadata.iter() {
        let value = match value {
            ParsedMetadataValue::I64(value) => BorrowedMetadataValue::I64(*value),
            ParsedMetadataValue::F64(value) => BorrowedMetadataValue::F64(*value),
            ParsedMetadataValue::String(value) => BorrowedMetadataValue::String(value.as_str()),
        };
        push_metadata(exec, output, owner, key, value)?;
    }
    Ok(())
}

fn append_segment_metadata<'session, 'input>(
    exec: &mut ParseExecution<'_, 'session>,
    output: &mut MeteredVec<'session, PendingMetadata<'input>>,
    parsed: &'input ParsedObj<'session>,
    mesh: usize,
    key: SegmentKey,
) -> ParseResult<()> {
    let owner = ProfileOwner::LogicalMesh(canonical_u32(
        exec,
        mesh,
        "obj-metadata-logical-mesh-owner",
    )?);
    for (name, id) in [
        ("obj:group", key.group),
        ("obj:material", key.material),
        ("obj:object", key.object),
    ] {
        if let Some(id) = id {
            let state_value = state_name(exec, parsed, id)?;
            push_metadata(
                exec,
                output,
                owner,
                name,
                BorrowedMetadataValue::String(state_value),
            )?;
        }
    }
    Ok(())
}

fn push_metadata<'session, 'input>(
    exec: &mut ParseExecution<'_, 'session>,
    output: &mut MeteredVec<'session, PendingMetadata<'input>>,
    owner: ProfileOwner,
    key: &str,
    value: BorrowedMetadataValue<'input>,
) -> ParseResult<()> {
    let key = NamespacedName::new(key)
        .map_err(|_| exec.format_failure(ObjFailure::internal("invalid-metadata-key")))?;
    output.try_push(exec, PendingMetadata { owner, key, value })
}

fn push_indexed_owners<'session, 'input>(
    exec: &mut ParseExecution<'_, 'session>,
    output: &mut MeteredVec<'session, OwnerProvenanceAuthoring<'input>>,
    count: usize,
    resource: &'static str,
    owner: impl Fn(u32) -> ProfileOwner,
    provenance: &'input [ProvenanceLocalId],
) -> ParseResult<()> {
    for index in 0..count {
        output.try_push_reserved(OwnerProvenanceAuthoring::new(
            owner(canonical_u32(exec, index, resource)?),
            provenance,
        ))?;
    }
    Ok(())
}

fn canonical_u32(
    exec: &ParseExecution<'_, '_>,
    value: usize,
    resource: &'static str,
) -> ParseResult<u32> {
    u32::try_from(value).map_err(|_| exec.allocation_failure(resource))
}

fn profile_resource_identity(
    exec: &ParseExecution<'_, '_>,
) -> ParseResult<(ResourceIdentity, SessionOpaqueReason)> {
    match ResourceIdentity::new(exec.source().as_str()) {
        Ok(identity) => Ok((identity, SessionOpaqueReason::CallerLabel)),
        Err(_) => ResourceIdentity::new(FALLBACK_RESOURCE_IDENTITY)
            .map(|identity| (identity, SessionOpaqueReason::SessionFallback))
            .map_err(|_| exec.allocation_failure("obj-profile-resource-identity")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_with_object(object: StateNameId) -> SegmentKey {
        SegmentKey {
            object: Some(object),
            group: None,
            material: None,
        }
    }

    #[test]
    fn segment_keys_compare_state_ids_in_constant_time() {
        assert_eq!(
            key_with_object(StateNameId::from_index(7)),
            key_with_object(StateNameId::from_index(7))
        );
        assert_ne!(
            key_with_object(StateNameId::from_index(7)),
            key_with_object(StateNameId::from_index(8))
        );
    }
}
