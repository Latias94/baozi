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
    C: SceneValidationControl,
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
        context.work(SceneValidationWork::MetadataEntry, 1)?;
        let (owner, owner_ordinal) = owners.normalize(entry.owner())?;
        let key_bytes = usize_to_u64(entry.key().as_str().len(), "metadata key bytes")?;
        enforce_limit::<C::Error>(
            ProfileAuthoringLimit::MetadataKeyBytes,
            key_bytes,
            usize_to_u64(limits.max_metadata_key_bytes(), "metadata key limit")?,
        )?;
        context.work(SceneValidationWork::MetadataByte, key_bytes)?;
        context.work(SceneValidationWork::PayloadCell, key_bytes)?;
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
                    }
                    .into());
                }
                std::cmp::Ordering::Greater => {
                    return Err(ProfileAuthoringSemanticError::MetadataKeyOrder {
                        previous,
                        entry: input_index,
                        owner,
                    }
                    .into());
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
        metrics.add_relation(context)?;

        let value = metadata_value_measure(input_index, entry.value(), limits)?;
        checked_add(
            &mut metrics.metadata_text_bytes,
            key_bytes.checked_add(value.text_bytes).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "metadata text lane bytes",
                },
            )?,
            "metadata text lane bytes",
        )?;
        checked_add(
            &mut metrics.metadata_payload_bytes,
            value.payload_bytes,
            "metadata byte lane bytes",
        )?;
        checked_add(
            &mut metrics.metadata_bool_count,
            value.bool_count,
            "metadata boolean lane values",
        )?;
        checked_add(
            &mut metrics.metadata_i64_count,
            value.i64_count,
            "metadata signed lane values",
        )?;
        checked_add(
            &mut metrics.metadata_u64_count,
            value.u64_count,
            "metadata unsigned lane values",
        )?;
        checked_add(
            &mut metrics.metadata_f64_count,
            value.f64_count,
            "metadata f64 lane values",
        )?;
        checked_add(
            &mut metrics.metadata_f32_count,
            value.f32_count,
            "metadata f32 lane values",
        )?;
        let entry_bytes = key_bytes.checked_add(value.bytes).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata entry bytes",
            },
        )?;
        checked_add(&mut metrics.metadata_bytes, entry_bytes, "metadata bytes")?;
        context.work(SceneValidationWork::MetadataByte, value.bytes)?;
        validate_metadata_value(input_index, entry.value(), context)?;
        metrics.add_accounted_payload(key_bytes.checked_add(value.cells).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata payload cells",
            },
        )?)?;
    }

    let mut next_owner_offset = 0_u32;
    for (&count, cursor) in owner_counts.iter().zip(owner_cursors.iter_mut()) {
        context.work(SceneValidationWork::Traversal, 1)?;
        *cursor = next_owner_offset;
        next_owner_offset = next_owner_offset.checked_add(count).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "metadata owner partition",
            },
        )?;
    }
    if usize::try_from(next_owner_offset).ok() != Some(metadata.entries().len()) {
        return Err(ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "metadata owner partition total",
        }
        .into());
    }
    for (input_index, entry) in metadata.entries().iter().copied().enumerate() {
        context.work(SceneValidationWork::Traversal, 1)?;
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
        *plan = ValidatedMetadataPlan { owner, input_index };
        *cursor =
            cursor
                .checked_add(1)
                .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "metadata owner cursor",
                })?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
struct MetadataValueMeasure {
    cells: u64,
    bytes: u64,
    text_bytes: u64,
    payload_bytes: u64,
    bool_count: u64,
    i64_count: u64,
    u64_count: u64,
    f64_count: u64,
    f32_count: u64,
}

fn metadata_value_measure(
    _entry: usize,
    value: ExtensionValueRef<'_>,
    limits: SceneValidationLimits,
) -> Result<MetadataValueMeasure, ProfileAuthoringSemanticError> {
    let measure = match value {
        ExtensionValueRef::Bool(_) => MetadataValueMeasure {
            cells: 1,
            bytes: LOGICAL_U8_BYTES,
            bool_count: 1,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::I64(_) => MetadataValueMeasure {
            cells: 1,
            bytes: LOGICAL_U64_BYTES,
            i64_count: 1,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::U64(_) => MetadataValueMeasure {
            cells: 1,
            bytes: LOGICAL_U64_BYTES,
            u64_count: 1,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::F64(_) => MetadataValueMeasure {
            cells: 1,
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
                cells: bytes,
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
                cells: bytes,
                bytes,
                text_bytes: media_type_bytes,
                payload_bytes,
                ..MetadataValueMeasure::default()
            }
        }
        ExtensionValueRef::Vec2(_) => MetadataValueMeasure {
            cells: 2,
            bytes: 2 * LOGICAL_F32_BYTES,
            f32_count: 2,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::Vec3(_) => MetadataValueMeasure {
            cells: 3,
            bytes: 3 * LOGICAL_F32_BYTES,
            f32_count: 3,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::Vec4(_) | ExtensionValueRef::Color(_) => MetadataValueMeasure {
            cells: 4,
            bytes: 4 * LOGICAL_F32_BYTES,
            f32_count: 4,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::Mat4(_) => MetadataValueMeasure {
            cells: 16,
            bytes: 16 * LOGICAL_F32_BYTES,
            f32_count: 16,
            ..MetadataValueMeasure::default()
        },
        ExtensionValueRef::DenseArray(values) => dense_array_measure(values)?,
    };
    Ok(measure)
}

fn validate_metadata_value<C>(
    entry: usize,
    value: ExtensionValueRef<'_>,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: SceneValidationControl,
{
    match value {
        ExtensionValueRef::Bool(_) | ExtensionValueRef::I64(_) | ExtensionValueRef::U64(_) => {
            context.work(SceneValidationWork::PayloadCell, 1)
        }
        ExtensionValueRef::F64(value) => {
            validate_finite_metadata_value(entry, value.is_finite(), 1, context)
        }
        ExtensionValueRef::String(value) => context.work(
            SceneValidationWork::PayloadCell,
            usize_to_u64(value.len(), "metadata string cells")?,
        ),
        ExtensionValueRef::Bytes {
            media_type,
            payload,
        } => context.work(
            SceneValidationWork::PayloadCell,
            checked_sum(
                [
                    usize_to_u64(media_type.as_str().len(), "metadata media type cells")?,
                    usize_to_u64(payload.len(), "metadata byte payload cells")?,
                ],
                "metadata byte value cells",
            )?,
        ),
        ExtensionValueRef::Vec2(value) => {
            validate_finite_metadata_value(entry, value.is_finite(), 2, context)
        }
        ExtensionValueRef::Vec3(value) => {
            validate_finite_metadata_value(entry, value.is_finite(), 3, context)
        }
        ExtensionValueRef::Vec4(value) => {
            validate_finite_metadata_value(entry, value.is_finite(), 4, context)
        }
        ExtensionValueRef::Color(value) => validate_finite_metadata_value(
            entry,
            value.r.is_finite()
                && value.g.is_finite()
                && value.b.is_finite()
                && value.a.is_finite(),
            4,
            context,
        ),
        ExtensionValueRef::Mat4(value) => {
            validate_finite_metadata_value(entry, value.is_finite(), 16, context)
        }
        ExtensionValueRef::DenseArray(values) => {
            validate_dense_metadata_value(entry, values, context)
        }
    }
}

fn validate_finite_metadata_value<C>(
    entry: usize,
    finite: bool,
    cells: u64,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: SceneValidationControl,
{
    context.work(SceneValidationWork::PayloadCell, cells)?;
    if finite {
        Ok(())
    } else {
        Err(ProfileAuthoringSemanticError::MetadataNonFinite { entry }.into())
    }
}

fn validate_dense_metadata_value<C>(
    entry: usize,
    values: ExtensionArrayRef<'_>,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: SceneValidationControl,
{
    match values {
        ExtensionArrayRef::Bool(values) => context.work(
            SceneValidationWork::PayloadCell,
            usize_to_u64(values.len(), "metadata boolean array cells")?,
        ),
        ExtensionArrayRef::I64(values) => context.work(
            SceneValidationWork::PayloadCell,
            usize_to_u64(values.len(), "metadata signed array cells")?,
        ),
        ExtensionArrayRef::U64(values) => context.work(
            SceneValidationWork::PayloadCell,
            usize_to_u64(values.len(), "metadata unsigned array cells")?,
        ),
        ExtensionArrayRef::F64(values) => {
            for value in values {
                validate_finite_metadata_value(entry, value.is_finite(), 1, context)?;
            }
            Ok(())
        }
        ExtensionArrayRef::Vec2(values) => {
            for value in values {
                validate_finite_metadata_value(entry, value.is_finite(), 2, context)?;
            }
            Ok(())
        }
        ExtensionArrayRef::Vec3(values) => {
            for value in values {
                validate_finite_metadata_value(entry, value.is_finite(), 3, context)?;
            }
            Ok(())
        }
        ExtensionArrayRef::Vec4(values) => {
            for value in values {
                validate_finite_metadata_value(entry, value.is_finite(), 4, context)?;
            }
            Ok(())
        }
        ExtensionArrayRef::Color(values) => {
            for value in values {
                validate_finite_metadata_value(
                    entry,
                    value.r.is_finite()
                        && value.g.is_finite()
                        && value.b.is_finite()
                        && value.a.is_finite(),
                    4,
                    context,
                )?;
            }
            Ok(())
        }
        ExtensionArrayRef::Mat4(values) => {
            for value in values {
                validate_finite_metadata_value(entry, value.is_finite(), 16, context)?;
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
        cells,
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
