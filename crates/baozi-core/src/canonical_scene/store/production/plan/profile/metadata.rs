use super::*;

pub(super) fn validate_metadata<C, R>(
    metadata: MetadataAuthoringBatch<'_>,
    owners: ProfileOwnerCatalog<'_>,
    limits: SceneValidationLimits,
    storage: &mut ProfileValidationStorage<R>,
    metrics: &mut ProfileMetrics,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    let ProfileValidationStorage {
        metadata_owner_counts: owner_counts,
        metadata_owner_cursors: owner_cursors,
        metadata_last_inputs: last_inputs,
        metadata_plans: plans,
        ..
    } = storage;
    let owner_count = usize::try_from(owners.owner_count()).map_err(|_| {
        ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "metadata owner count",
        }
    })?;
    if owner_counts.capacity() < owner_count {
        return Err(ProfileAuthoringSemanticError::ScratchAllocation {
            resource: "profile metadata owner counts",
        }
        .into());
    }
    if owner_cursors.capacity() < owner_count {
        return Err(ProfileAuthoringSemanticError::ScratchAllocation {
            resource: "profile metadata owner cursors",
        }
        .into());
    }
    if last_inputs.capacity() < owner_count {
        return Err(ProfileAuthoringSemanticError::ScratchAllocation {
            resource: "profile metadata owner last inputs",
        }
        .into());
    }
    if plans.capacity() < metadata.entries().len() {
        return Err(ProfileAuthoringSemanticError::ScratchAllocation {
            resource: "profile metadata plans",
        }
        .into());
    }
    initialize_scratch(
        owner_counts,
        owner_count,
        0,
        "profile metadata owner counts",
        context,
    )?;
    initialize_scratch(
        owner_cursors,
        owner_count,
        0,
        "profile metadata owner cursors",
        context,
    )?;
    initialize_scratch(
        last_inputs,
        owner_count,
        u32::MAX,
        "profile metadata owner last inputs",
        context,
    )?;
    initialize_scratch(
        plans,
        metadata.entries().len(),
        ValidatedMetadataPlan {
            owner: CanonicalProfileOwner::Scene,
            input_index: usize::MAX,
        },
        "profile metadata plans",
        context,
    )?;

    for (input_index, entry) in metadata.entries().iter().copied().enumerate() {
        context.run_semantic_unit(SceneValidationWork::MetadataEntry, || {
            let (owner, owner_ordinal) = owners.normalize(entry.owner())?;
            let key_bytes = usize_to_u64(entry.key().as_str().len(), "metadata key bytes")?;
            if key_bytes > usize_to_u64(limits.max_metadata_key_bytes(), "metadata key limit")? {
                return Err(ProfileAuthoringSemanticError::LimitExceeded {
                    limit: ProfileAuthoringLimit::MetadataKeyBytes,
                    actual: key_bytes,
                    maximum: usize_to_u64(limits.max_metadata_key_bytes(), "metadata key limit")?,
                });
            }
            let input_index_u32 = u32::try_from(input_index).map_err(|_| {
                ProfileAuthoringSemanticError::CountDomainTooLarge {
                    collection: "metadata entries",
                }
            })?;
            let previous = *last_inputs.get(owner_ordinal).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "metadata owner last-input index",
                },
            )?;
            if previous != u32::MAX {
                let previous = usize::try_from(previous).map_err(|_| {
                    ProfileAuthoringSemanticError::AccountingOverflow {
                        dimension: "metadata prior input index",
                    }
                })?;
                let previous_key = metadata
                    .entries()
                    .get(previous)
                    .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                        dimension: "metadata prior input",
                    })?
                    .key();
                match previous_key.cmp(entry.key()) {
                    std::cmp::Ordering::Less => {}
                    std::cmp::Ordering::Equal => {
                        return Err(ProfileAuthoringSemanticError::DuplicateMetadataKey {
                            first: previous,
                            duplicate: input_index,
                        });
                    }
                    std::cmp::Ordering::Greater => {
                        return Err(ProfileAuthoringSemanticError::MetadataKeyOrder {
                            previous,
                            entry: input_index,
                            owner,
                        });
                    }
                }
            }
            *last_inputs.get_mut(owner_ordinal).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "metadata owner last-input index",
                },
            )? = input_index_u32;
            let owner_entry_count = owner_counts.get_mut(owner_ordinal).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "metadata owner count index",
                },
            )?;
            *owner_entry_count = owner_entry_count.checked_add(1).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "metadata owner entry count",
                },
            )?;
            let value = metadata_value_measure(input_index, entry.value(), limits)?;
            metrics.add_metadata_measure(key_bytes, value, limits.max_metadata_bytes())?;
            Ok(())
        })?;
        for byte in entry.key().as_str().as_bytes() {
            context.run_unit(SceneValidationWork::MetadataByte, || *byte)?;
            run_metadata_payload_cell(metrics, context, || Ok(*byte))?;
        }
        metrics.add_relation(context)?;

        visit_metadata_value_bytes(entry.value(), context)?;
        validate_metadata_value(input_index, entry.value(), metrics, context)?;
    }

    let mut next_owner_offset = 0_u32;
    for (&count, cursor) in owner_counts.iter().zip(owner_cursors.iter_mut()) {
        let following_owner_offset = next_owner_offset.checked_add(count).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata owner partition",
            },
        )?;
        context.run_unit(SceneValidationWork::Traversal, || {
            *cursor = next_owner_offset;
        })?;
        next_owner_offset = following_owner_offset;
    }
    if usize::try_from(next_owner_offset).ok() != Some(metadata.entries().len()) {
        return Err(ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "metadata owner partition total",
        }
        .into());
    }
    for (input_index, entry) in metadata.entries().iter().copied().enumerate() {
        let (owner, owner_ordinal) = owners.normalize(entry.owner())?;
        let cursor = owner_cursors.get_mut(owner_ordinal).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata owner cursor index",
            },
        )?;
        let output_index = usize::try_from(*cursor).map_err(|_| {
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata output index",
            }
        })?;
        let plan = plans.get_mut(output_index).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata output plan",
            },
        )?;
        let following_cursor =
            cursor
                .checked_add(1)
                .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "metadata owner cursor",
                })?;
        context.run_unit(SceneValidationWork::Traversal, || {
            *plan = ValidatedMetadataPlan { owner, input_index };
            *cursor = following_cursor;
        })?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
struct MetadataValueMeasure {
    bytes: u64,
    text_bytes: u64,
    payload_bytes: u64,
    bool_count: u64,
    i64_count: u64,
    u64_count: u64,
    f64_count: u64,
    f32_count: u64,
}

impl ProfileMetrics {
    fn add_metadata_measure(
        &mut self,
        key_bytes: u64,
        value: MetadataValueMeasure,
        maximum_metadata_bytes: u64,
    ) -> Result<(), ProfileAuthoringSemanticError> {
        let metadata_text_bytes = self
            .metadata_text_bytes
            .checked_add(key_bytes.checked_add(value.text_bytes).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "metadata text lane bytes",
                },
            )?)
            .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata text lane bytes",
            })?;
        let metadata_payload_bytes = self
            .metadata_payload_bytes
            .checked_add(value.payload_bytes)
            .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata byte lane bytes",
            })?;
        let metadata_bool_count = self
            .metadata_bool_count
            .checked_add(value.bool_count)
            .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata boolean lane values",
            })?;
        let metadata_i64_count = self.metadata_i64_count.checked_add(value.i64_count).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata signed lane values",
            },
        )?;
        let metadata_u64_count = self.metadata_u64_count.checked_add(value.u64_count).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata unsigned lane values",
            },
        )?;
        let metadata_f64_count = self.metadata_f64_count.checked_add(value.f64_count).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata f64 lane values",
            },
        )?;
        let metadata_f32_count = self.metadata_f32_count.checked_add(value.f32_count).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata f32 lane values",
            },
        )?;
        let entry_bytes = key_bytes.checked_add(value.bytes).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata entry bytes",
            },
        )?;
        let metadata_bytes = self.metadata_bytes.checked_add(entry_bytes).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata bytes",
            },
        )?;
        require_limit(
            ProfileAuthoringLimit::MetadataBytes,
            metadata_bytes,
            maximum_metadata_bytes,
        )?;

        self.metadata_text_bytes = metadata_text_bytes;
        self.metadata_payload_bytes = metadata_payload_bytes;
        self.metadata_bool_count = metadata_bool_count;
        self.metadata_i64_count = metadata_i64_count;
        self.metadata_u64_count = metadata_u64_count;
        self.metadata_f64_count = metadata_f64_count;
        self.metadata_f32_count = metadata_f32_count;
        self.metadata_bytes = metadata_bytes;
        Ok(())
    }
}

fn metadata_value_measure(
    _entry: usize,
    value: ExtensionValueRef<'_>,
    limits: SceneValidationLimits,
) -> Result<MetadataValueMeasure, ProfileAuthoringSemanticError> {
    let measure = match value {
        ExtensionValueRef::Bool(_) => MetadataValueMeasure {
            bytes: LOGICAL_U8_BYTES,
            bool_count: 1,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::I64(_) => MetadataValueMeasure {
            bytes: LOGICAL_U64_BYTES,
            i64_count: 1,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::U64(_) => MetadataValueMeasure {
            bytes: LOGICAL_U64_BYTES,
            u64_count: 1,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::F64(_) => MetadataValueMeasure {
            bytes: LOGICAL_U64_BYTES,
            f64_count: 1,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::String(value) => {
            let bytes = usize_to_u64(value.len(), "metadata string bytes")?;
            if value.len() > limits.max_metadata_string_bytes() {
                return Err(ProfileAuthoringSemanticError::LimitExceeded {
                    limit: ProfileAuthoringLimit::MetadataStringBytes,
                    actual: bytes,
                    maximum: usize_to_u64(
                        limits.max_metadata_string_bytes(),
                        "metadata string limit",
                    )?,
                });
            }
            MetadataValueMeasure {
                bytes,
                text_bytes: bytes,
                ..MetadataValueMeasure::default()
            }
        }
        ExtensionValueRef::Bytes {
            media_type,
            payload,
        } => {
            let media_type_bytes =
                usize_to_u64(media_type.as_str().len(), "metadata media type bytes")?;
            let payload_bytes = usize_to_u64(payload.len(), "metadata byte payload")?;
            let bytes = media_type_bytes.checked_add(payload_bytes).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "metadata byte value",
                },
            )?;
            MetadataValueMeasure {
                bytes,
                text_bytes: media_type_bytes,
                payload_bytes,
                ..MetadataValueMeasure::default()
            }
        }
        ExtensionValueRef::Vec2(_) => MetadataValueMeasure {
            bytes: 2 * LOGICAL_F32_BYTES,
            f32_count: 2,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::Vec3(_) => MetadataValueMeasure {
            bytes: 3 * LOGICAL_F32_BYTES,
            f32_count: 3,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::Vec4(_) | ExtensionValueRef::Color(_) => MetadataValueMeasure {
            bytes: 4 * LOGICAL_F32_BYTES,
            f32_count: 4,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::Mat4(_) => MetadataValueMeasure {
            bytes: 16 * LOGICAL_F32_BYTES,
            f32_count: 16,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::DenseArray(values) => dense_array_measure(values)?,
    };
    Ok(measure)
}

fn visit_metadata_value_bytes<C>(
    value: ExtensionValueRef<'_>,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    match value {
        ExtensionValueRef::Bool(value) => visit_metadata_bytes([u8::from(value)], context),
        ExtensionValueRef::I64(value) => visit_metadata_bytes(value.to_le_bytes(), context),
        ExtensionValueRef::U64(value) => visit_metadata_bytes(value.to_le_bytes(), context),
        ExtensionValueRef::F64(value) => visit_metadata_bytes(value.to_le_bytes(), context),
        ExtensionValueRef::String(value) => {
            visit_metadata_bytes(value.as_bytes().iter().copied(), context)
        }
        ExtensionValueRef::Bytes {
            media_type,
            payload,
        } => {
            visit_metadata_bytes(media_type.as_str().as_bytes().iter().copied(), context)?;
            visit_metadata_bytes(payload.iter().copied(), context)
        }
        ExtensionValueRef::Vec2(value) => visit_f32_metadata_bytes([value.x, value.y], context),
        ExtensionValueRef::Vec3(value) => {
            visit_f32_metadata_bytes([value.x, value.y, value.z], context)
        }
        ExtensionValueRef::Vec4(value) => {
            visit_f32_metadata_bytes([value.x, value.y, value.z, value.w], context)
        }
        ExtensionValueRef::Color(value) => {
            visit_f32_metadata_bytes([value.r, value.g, value.b, value.a], context)
        }
        ExtensionValueRef::Mat4(value) => {
            visit_f32_metadata_bytes(value.cols.into_iter().flatten(), context)
        }
        ExtensionValueRef::DenseArray(values) => visit_dense_metadata_value_bytes(values, context),
    }
}

fn visit_metadata_bytes<C>(
    bytes: impl IntoIterator<Item = u8>,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    for byte in bytes {
        context.run_unit(SceneValidationWork::MetadataByte, || byte)?;
    }
    Ok(())
}

fn visit_f32_metadata_bytes<C>(
    values: impl IntoIterator<Item = f32>,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    for value in values {
        visit_metadata_bytes(value.to_le_bytes(), context)?;
    }
    Ok(())
}

fn visit_dense_metadata_value_bytes<C>(
    values: ExtensionArrayRef<'_>,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    match values {
        ExtensionArrayRef::Bool(values) => {
            visit_metadata_bytes(values.iter().copied().map(u8::from), context)
        }
        ExtensionArrayRef::I64(values) => {
            for value in values {
                visit_metadata_bytes(value.to_le_bytes(), context)?;
            }
            Ok(())
        }
        ExtensionArrayRef::U64(values) => {
            for value in values {
                visit_metadata_bytes(value.to_le_bytes(), context)?;
            }
            Ok(())
        }
        ExtensionArrayRef::F64(values) => {
            for value in values {
                visit_metadata_bytes(value.to_le_bytes(), context)?;
            }
            Ok(())
        }
        ExtensionArrayRef::Vec2(values) => {
            for value in values {
                visit_f32_metadata_bytes([value.x, value.y], context)?;
            }
            Ok(())
        }
        ExtensionArrayRef::Vec3(values) => {
            for value in values {
                visit_f32_metadata_bytes([value.x, value.y, value.z], context)?;
            }
            Ok(())
        }
        ExtensionArrayRef::Vec4(values) => {
            for value in values {
                visit_f32_metadata_bytes([value.x, value.y, value.z, value.w], context)?;
            }
            Ok(())
        }
        ExtensionArrayRef::Color(values) => {
            for value in values {
                visit_f32_metadata_bytes([value.r, value.g, value.b, value.a], context)?;
            }
            Ok(())
        }
        ExtensionArrayRef::Mat4(values) => {
            for value in values {
                visit_f32_metadata_bytes(value.cols.into_iter().flatten(), context)?;
            }
            Ok(())
        }
    }
}

fn run_metadata_payload_cell<C, T>(
    metrics: &mut ProfileMetrics,
    context: &mut ProfileValidationContext<'_, C>,
    action: impl FnOnce() -> Result<T, ProfileAuthoringSemanticError>,
) -> ProfileValidationResult<T, C>
where
    C: CanonicalAuthoringControl,
{
    let maximum = context.limits.max_payload_cells();
    context.run_semantic_unit(SceneValidationWork::PayloadCell, || {
        let result = action()?;
        metrics.add_payload_cell(maximum)?;
        Ok(result)
    })
}

fn validate_metadata_value<C>(
    entry: usize,
    value: ExtensionValueRef<'_>,
    metrics: &mut ProfileMetrics,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    match value {
        ExtensionValueRef::Bool(value) => {
            run_metadata_payload_cell(metrics, context, || Ok(value))?;
            Ok(())
        }
        ExtensionValueRef::I64(value) => {
            run_metadata_payload_cell(metrics, context, || Ok(value))?;
            Ok(())
        }
        ExtensionValueRef::U64(value) => {
            run_metadata_payload_cell(metrics, context, || Ok(value))?;
            Ok(())
        }
        ExtensionValueRef::F64(value) => {
            validate_finite_metadata_cells(entry, [value.is_finite()], metrics, context)
        }
        ExtensionValueRef::String(value) => {
            for byte in value.as_bytes() {
                run_metadata_payload_cell(metrics, context, || Ok(*byte))?;
            }
            Ok(())
        }
        ExtensionValueRef::Bytes {
            media_type,
            payload,
        } => {
            for byte in media_type.as_str().as_bytes() {
                run_metadata_payload_cell(metrics, context, || Ok(*byte))?;
            }
            for byte in payload {
                run_metadata_payload_cell(metrics, context, || Ok(*byte))?;
            }
            Ok(())
        }
        ExtensionValueRef::Vec2(value) => validate_finite_metadata_cells(
            entry,
            [value.x.is_finite(), value.y.is_finite()],
            metrics,
            context,
        ),
        ExtensionValueRef::Vec3(value) => validate_finite_metadata_cells(
            entry,
            [
                value.x.is_finite(),
                value.y.is_finite(),
                value.z.is_finite(),
            ],
            metrics,
            context,
        ),
        ExtensionValueRef::Vec4(value) => validate_finite_metadata_cells(
            entry,
            [
                value.x.is_finite(),
                value.y.is_finite(),
                value.z.is_finite(),
                value.w.is_finite(),
            ],
            metrics,
            context,
        ),
        ExtensionValueRef::Color(value) => validate_finite_metadata_cells(
            entry,
            [
                value.r.is_finite(),
                value.g.is_finite(),
                value.b.is_finite(),
                value.a.is_finite(),
            ],
            metrics,
            context,
        ),
        ExtensionValueRef::Mat4(value) => validate_finite_metadata_cells(
            entry,
            value.cols.into_iter().flatten().map(f32::is_finite),
            metrics,
            context,
        ),
        ExtensionValueRef::DenseArray(values) => {
            validate_dense_metadata_value(entry, values, metrics, context)
        }
    }
}

fn validate_finite_metadata_cells<C>(
    entry: usize,
    cells: impl IntoIterator<Item = bool>,
    metrics: &mut ProfileMetrics,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    for finite in cells {
        run_metadata_payload_cell(metrics, context, || {
            if finite {
                Ok(())
            } else {
                Err(ProfileAuthoringSemanticError::MetadataNonFinite { entry })
            }
        })?;
    }
    Ok(())
}

fn validate_dense_metadata_value<C>(
    entry: usize,
    values: ExtensionArrayRef<'_>,
    metrics: &mut ProfileMetrics,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    match values {
        ExtensionArrayRef::Bool(values) => {
            for value in values {
                run_metadata_payload_cell(metrics, context, || Ok(*value))?;
            }
            Ok(())
        }
        ExtensionArrayRef::I64(values) => {
            for value in values {
                run_metadata_payload_cell(metrics, context, || Ok(*value))?;
            }
            Ok(())
        }
        ExtensionArrayRef::U64(values) => {
            for value in values {
                run_metadata_payload_cell(metrics, context, || Ok(*value))?;
            }
            Ok(())
        }
        ExtensionArrayRef::F64(values) => {
            for value in values {
                validate_finite_metadata_cells(entry, [value.is_finite()], metrics, context)?;
            }
            Ok(())
        }
        ExtensionArrayRef::Vec2(values) => {
            for value in values {
                validate_finite_metadata_cells(
                    entry,
                    [value.x.is_finite(), value.y.is_finite()],
                    metrics,
                    context,
                )?;
            }
            Ok(())
        }
        ExtensionArrayRef::Vec3(values) => {
            for value in values {
                validate_finite_metadata_cells(
                    entry,
                    [
                        value.x.is_finite(),
                        value.y.is_finite(),
                        value.z.is_finite(),
                    ],
                    metrics,
                    context,
                )?;
            }
            Ok(())
        }
        ExtensionArrayRef::Vec4(values) => {
            for value in values {
                validate_finite_metadata_cells(
                    entry,
                    [
                        value.x.is_finite(),
                        value.y.is_finite(),
                        value.z.is_finite(),
                        value.w.is_finite(),
                    ],
                    metrics,
                    context,
                )?;
            }
            Ok(())
        }
        ExtensionArrayRef::Color(values) => {
            for value in values {
                validate_finite_metadata_cells(
                    entry,
                    [
                        value.r.is_finite(),
                        value.g.is_finite(),
                        value.b.is_finite(),
                        value.a.is_finite(),
                    ],
                    metrics,
                    context,
                )?;
            }
            Ok(())
        }
        ExtensionArrayRef::Mat4(values) => {
            for value in values {
                validate_finite_metadata_cells(
                    entry,
                    value.cols.into_iter().flatten().map(f32::is_finite),
                    metrics,
                    context,
                )?;
            }
            Ok(())
        }
    }
}

fn dense_array_measure(
    values: ExtensionArrayRef<'_>,
) -> Result<MetadataValueMeasure, ProfileAuthoringSemanticError> {
    let (length, element_cells, element_bytes, lane) = match values {
        ExtensionArrayRef::Bool(values) => (values.len(), 1_u64, LOGICAL_U8_BYTES, 0_u8),
        ExtensionArrayRef::I64(values) => (values.len(), 1, LOGICAL_U64_BYTES, 1),
        ExtensionArrayRef::U64(values) => (values.len(), 1, LOGICAL_U64_BYTES, 2),
        ExtensionArrayRef::F64(values) => (values.len(), 1, LOGICAL_U64_BYTES, 3),
        ExtensionArrayRef::Vec2(values) => (values.len(), 2, 2 * LOGICAL_F32_BYTES, 4),
        ExtensionArrayRef::Vec3(values) => (values.len(), 3, 3 * LOGICAL_F32_BYTES, 4),
        ExtensionArrayRef::Vec4(values) => (values.len(), 4, 4 * LOGICAL_F32_BYTES, 4),
        ExtensionArrayRef::Color(values) => (values.len(), 4, 4 * LOGICAL_F32_BYTES, 4),
        ExtensionArrayRef::Mat4(values) => (values.len(), 16, 16 * LOGICAL_F32_BYTES, 4),
    };
    let elements = usize_to_u64(length, "metadata dense array elements")?;
    let cells = elements.checked_mul(element_cells).ok_or(
        ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "metadata dense array cells",
        },
    )?;
    let bytes = elements.checked_mul(element_bytes).ok_or(
        ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "metadata dense array bytes",
        },
    )?;
    let mut measure = MetadataValueMeasure {
        bytes,
        ..MetadataValueMeasure::default()
    };
    match lane {
        0 => measure.bool_count = cells,
        1 => measure.i64_count = cells,
        2 => measure.u64_count = cells,
        3 => measure.f64_count = cells,
        4 => measure.f32_count = cells,
        _ => {
            return Err(ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata dense-array lane",
            });
        }
    }
    Ok(measure)
}
