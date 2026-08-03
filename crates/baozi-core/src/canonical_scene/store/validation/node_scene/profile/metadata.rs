use super::*;

pub(super) fn validate_metadata<R>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<ProfileMetadataDirectorySchema>,
    provenance_count: u32,
    owner_classes: ProfileOwnerClassCounts,
    expected: Option<&ValidatedProfileBatch<'_, R>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<MetadataValidatedRefs> {
    observe_work(1)?;
    let directory = session.canonical_records(directory_ref)?;
    if directory.rows() != 1 || directory.fields() != PROFILE_METADATA_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, profile_metadata_directory_field::SCHEMA_REVISION),
        PROFILE_METADATA_SCHEMA_REVISION,
    )?;
    let entry_count = directory.scalar::<u32>(0, profile_metadata_directory_field::ENTRY_COUNT)?;
    let (entries_ref, entries) = optional_records::<ProfileMetadataEntryTableSchema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::ENTRIES,
        entry_count,
        PROFILE_METADATA_ENTRY_FIELD_COUNT,
    )?;
    let text_count = directory.scalar::<u32>(0, profile_metadata_directory_field::TEXT_COUNT)?;
    let (text_ref, text) = optional_utf8::<ProfileMetadataTextSchema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::TEXTS,
        text_count,
    )?;
    let byte_count = directory.scalar::<u32>(0, profile_metadata_directory_field::BYTE_COUNT)?;
    let (bytes_ref, bytes) = optional_bytes::<ProfileMetadataBytesSchema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::BYTES,
        byte_count,
    )?;
    let bool_count = directory.scalar::<u32>(0, profile_metadata_directory_field::BOOL_COUNT)?;
    let (bools_ref, bools) = optional_sequence::<u8, ProfileMetadataBoolSchema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::BOOL_VALUES,
        bool_count,
    )?;
    let i64_count = directory.scalar::<u32>(0, profile_metadata_directory_field::I64_COUNT)?;
    let (i64s_ref, i64s) = optional_sequence::<i64, ProfileMetadataI64Schema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::I64_VALUES,
        i64_count,
    )?;
    let u64_count = directory.scalar::<u32>(0, profile_metadata_directory_field::U64_COUNT)?;
    let (u64s_ref, u64s) = optional_sequence::<u64, ProfileMetadataU64Schema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::U64_VALUES,
        u64_count,
    )?;
    let f64_count = directory.scalar::<u32>(0, profile_metadata_directory_field::F64_COUNT)?;
    let (f64s_ref, f64s) = optional_sequence::<f64, ProfileMetadataF64Schema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::F64_VALUES,
        f64_count,
    )?;
    let f32_count = directory.scalar::<u32>(0, profile_metadata_directory_field::F32_COUNT)?;
    let (f32s_ref, f32s) = optional_sequence::<f32, ProfileMetadataF32Schema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::F32_VALUES,
        f32_count,
    )?;
    if expected.is_some_and(|expected| {
        let shape = expected.retention_shape();
        entry_count != expected.metadata_entry_count()
            || text_count != shape.metadata_text_bytes()
            || byte_count != shape.metadata_payload_bytes()
            || bool_count != shape.metadata_bool_count()
            || i64_count != shape.metadata_i64_count()
            || u64_count != shape.metadata_u64_count()
            || f64_count != shape.metadata_f64_count()
            || f32_count != shape.metadata_f32_count()
    }) {
        return None;
    }

    let text_value = text.unwrap_or("");
    let byte_values = bytes.unwrap_or(&[]);
    let bool_values = bools.map(|value| value.as_slice()).unwrap_or(&[]);
    let i64_values = i64s.map(|value| value.as_slice()).unwrap_or(&[]);
    let u64_values = u64s.map(|value| value.as_slice()).unwrap_or(&[]);
    let f64_values = f64s.map(|value| value.as_slice()).unwrap_or(&[]);
    let f32_values = f32s.map(|value| value.as_slice()).unwrap_or(&[]);
    let entry_view = entries;
    let mut cursors = MetadataCursors::default();
    let mut previous_owner = None;
    let mut previous_key: Option<&str> = None;
    for row_index in 0..usize::try_from(entry_count).ok()? {
        observe_work(1)?;
        let row = entry_view?;
        let owner = decode_owner(
            row.scalar::<u8>(row_index, profile_metadata_entry_field::OWNER_KIND)?,
            row.scalar::<u32>(row_index, profile_metadata_entry_field::OWNER_INDEX)?,
        )?;
        if !owner_exists(owner_classes, owner) {
            return None;
        }
        let key = consume_text(
            text_value,
            &mut cursors.text,
            row.scalar::<u32>(row_index, profile_metadata_entry_field::KEY_OFFSET)?,
            row.scalar::<u32>(row_index, profile_metadata_entry_field::KEY_COUNT)?,
            true,
            expected
                .and_then(|expected| expected.metadata_entry(row_index))
                .map(|entry| entry.key().as_str()),
            observe_work,
        )?;
        if previous_owner.is_some_and(|previous| owner < previous)
            || previous_owner == Some(owner)
                && previous_key.is_some_and(|previous| previous.as_bytes() >= key.as_bytes())
        {
            return None;
        }
        previous_owner = Some(owner);
        previous_key = Some(key);
        let value_kind = row.scalar::<u8>(row_index, profile_metadata_entry_field::VALUE_KIND)?;
        let array_kind = row.scalar::<u8>(row_index, profile_metadata_entry_field::ARRAY_KIND)?;
        let logical_count = row.scalar::<u32>(
            row_index,
            profile_metadata_entry_field::LOGICAL_ELEMENT_COUNT,
        )?;
        let shape = metadata_shape(value_kind, array_kind, logical_count)?;
        let expected_entry = expected.and_then(|expected| expected.metadata_entry(row_index));
        if expected_entry.is_some_and(|entry| entry.owner() != owner) {
            return None;
        }
        let expected_shape = match expected_entry {
            Some(entry) => Some(expected_metadata_shape(entry.value())?),
            None => None,
        };
        if expected_shape.is_some_and(|expected| {
            expected.0 != value_kind || expected.1 != array_kind || expected.2 != logical_count
        }) {
            return None;
        }
        let auxiliary_count =
            row.scalar::<u32>(row_index, profile_metadata_entry_field::AUX_TEXT_COUNT)?;
        let expected_auxiliary = expected_entry.and_then(|entry| match entry.value() {
            ExtensionValueRef::String(value) => Some(value),
            ExtensionValueRef::Bytes { media_type, .. } => Some(media_type.as_str()),
            _ => None,
        });
        let auxiliary = consume_text(
            text_value,
            &mut cursors.text,
            row.scalar::<u32>(row_index, profile_metadata_entry_field::AUX_TEXT_OFFSET)?,
            auxiliary_count,
            shape.auxiliary_text && value_kind == PROFILE_METADATA_BYTES,
            expected_entry.and(expected_auxiliary),
            observe_work,
        )?;
        if shape.auxiliary_text
            != (value_kind == PROFILE_METADATA_STRING || value_kind == PROFILE_METADATA_BYTES)
            || !shape.auxiliary_text && auxiliary_count != 0
            || value_kind == PROFILE_METADATA_BYTES && auxiliary.is_empty()
        {
            return None;
        }
        let stored_byte_count =
            row.scalar::<u32>(row_index, profile_metadata_entry_field::BYTE_COUNT)?;
        let expected_payload = expected_entry.and_then(|entry| match entry.value() {
            ExtensionValueRef::Bytes { payload, .. } => Some(payload),
            _ => None,
        });
        consume_bytes(
            byte_values,
            &mut cursors.bytes,
            row.scalar::<u32>(row_index, profile_metadata_entry_field::BYTE_OFFSET)?,
            stored_byte_count,
            expected_payload,
            observe_work,
        )?;
        if shape.bytes != (value_kind == PROFILE_METADATA_BYTES)
            || !shape.bytes && stored_byte_count != 0
        {
            return None;
        }
        validate_entry_lane_refs(
            row, row_index, shape, text_ref, bytes_ref, bools_ref, i64s_ref, u64s_ref, f64s_ref,
            f32s_ref,
        )?;
        let scalar_offset =
            row.scalar::<u32>(row_index, profile_metadata_entry_field::SCALAR_OFFSET)?;
        let scalar_count =
            row.scalar::<u32>(row_index, profile_metadata_entry_field::SCALAR_COUNT)?;
        if scalar_count != shape.scalar_count {
            return None;
        }
        validate_metadata_payload(
            shape,
            scalar_offset,
            scalar_count,
            &mut cursors,
            bool_values,
            i64_values,
            u64_values,
            f64_values,
            f32_values,
            expected_entry.map(|entry| entry.value()),
            observe_work,
        )?;
        let provenance =
            row.scalar::<u32>(row_index, profile_metadata_entry_field::PROVENANCE_INDEX)?;
        if provenance >= provenance_count
            || expected_entry.is_some_and(|entry| provenance != entry.provenance().index())
        {
            return None;
        }
    }
    if cursors.text != text_count
        || cursors.bytes != byte_count
        || cursors.bools != bool_count
        || cursors.i64s != i64_count
        || cursors.u64s != u64_count
        || cursors.f64s != f64_count
        || cursors.f32s != f32_count
    {
        return None;
    }
    Some(MetadataValidatedRefs {
        entries: entries_ref,
        text: text_ref,
        bytes: bytes_ref,
        bools: bools_ref,
        i64s: i64s_ref,
        u64s: u64s_ref,
        f64s: f64s_ref,
        f32s: f32s_ref,
        entry_count,
        text_count,
        byte_count,
        bool_count,
        i64_count,
        u64_count,
        f64_count,
        f32_count,
    })
}

fn metadata_shape(value_kind: u8, array_kind: u8, logical_count: u32) -> Option<MetadataShape> {
    let scalar = |lane, scalar_count| MetadataShape {
        lane,
        scalar_count,
        auxiliary_text: false,
        bytes: false,
    };
    let shape = match value_kind {
        PROFILE_METADATA_BOOL
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::Bool, 1)
        }
        PROFILE_METADATA_I64 if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 => {
            scalar(MetadataLane::I64, 1)
        }
        PROFILE_METADATA_U64 if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 => {
            scalar(MetadataLane::U64, 1)
        }
        PROFILE_METADATA_F64 if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 => {
            scalar(MetadataLane::F64, 1)
        }
        PROFILE_METADATA_STRING
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            MetadataShape {
                lane: MetadataLane::None,
                scalar_count: 0,
                auxiliary_text: true,
                bytes: false,
            }
        }
        PROFILE_METADATA_BYTES
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            MetadataShape {
                lane: MetadataLane::None,
                scalar_count: 0,
                auxiliary_text: true,
                bytes: true,
            }
        }
        PROFILE_METADATA_VEC2
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::F32, 2)
        }
        PROFILE_METADATA_VEC3
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::F32, 3)
        }
        PROFILE_METADATA_VEC4 | PROFILE_METADATA_COLOR
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::F32, 4)
        }
        PROFILE_METADATA_MAT4
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::F32, 16)
        }
        PROFILE_METADATA_DENSE_ARRAY => {
            let (lane, width) = match array_kind {
                PROFILE_METADATA_ARRAY_BOOL => (MetadataLane::Bool, 1),
                PROFILE_METADATA_ARRAY_I64 => (MetadataLane::I64, 1),
                PROFILE_METADATA_ARRAY_U64 => (MetadataLane::U64, 1),
                PROFILE_METADATA_ARRAY_F64 => (MetadataLane::F64, 1),
                PROFILE_METADATA_ARRAY_VEC2 => (MetadataLane::F32, 2),
                PROFILE_METADATA_ARRAY_VEC3 => (MetadataLane::F32, 3),
                PROFILE_METADATA_ARRAY_VEC4 | PROFILE_METADATA_ARRAY_COLOR => {
                    (MetadataLane::F32, 4)
                }
                PROFILE_METADATA_ARRAY_MAT4 => (MetadataLane::F32, 16),
                _ => return None,
            };
            scalar(lane, logical_count.checked_mul(width)?)
        }
        _ => return None,
    };
    Some(shape)
}

fn expected_metadata_shape(value: ExtensionValueRef<'_>) -> Option<(u8, u8, u32)> {
    Some(match value {
        ExtensionValueRef::Bool(_) => (PROFILE_METADATA_BOOL, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::I64(_) => (PROFILE_METADATA_I64, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::U64(_) => (PROFILE_METADATA_U64, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::F64(_) => (PROFILE_METADATA_F64, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::String(_) => (PROFILE_METADATA_STRING, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::Bytes { .. } => (PROFILE_METADATA_BYTES, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::Vec2(_) => (PROFILE_METADATA_VEC2, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::Vec3(_) => (PROFILE_METADATA_VEC3, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::Vec4(_) => (PROFILE_METADATA_VEC4, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::Color(_) => (PROFILE_METADATA_COLOR, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::Mat4(_) => (PROFILE_METADATA_MAT4, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::DenseArray(values) => {
            let kind = match values {
                ExtensionArrayRef::Bool(_) => PROFILE_METADATA_ARRAY_BOOL,
                ExtensionArrayRef::I64(_) => PROFILE_METADATA_ARRAY_I64,
                ExtensionArrayRef::U64(_) => PROFILE_METADATA_ARRAY_U64,
                ExtensionArrayRef::F64(_) => PROFILE_METADATA_ARRAY_F64,
                ExtensionArrayRef::Vec2(_) => PROFILE_METADATA_ARRAY_VEC2,
                ExtensionArrayRef::Vec3(_) => PROFILE_METADATA_ARRAY_VEC3,
                ExtensionArrayRef::Vec4(_) => PROFILE_METADATA_ARRAY_VEC4,
                ExtensionArrayRef::Color(_) => PROFILE_METADATA_ARRAY_COLOR,
                ExtensionArrayRef::Mat4(_) => PROFILE_METADATA_ARRAY_MAT4,
            };
            (
                PROFILE_METADATA_DENSE_ARRAY,
                kind,
                u32::try_from(values.len()).ok()?,
            )
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_entry_lane_refs(
    row: CanonicalRecordView<'_>,
    index: usize,
    shape: MetadataShape,
    text: Option<CanonicalBlockRef<ProfileMetadataTextSchema>>,
    bytes: Option<CanonicalBlockRef<ProfileMetadataBytesSchema>>,
    bools: Option<CanonicalBlockRef<ProfileMetadataBoolSchema>>,
    i64s: Option<CanonicalBlockRef<ProfileMetadataI64Schema>>,
    u64s: Option<CanonicalBlockRef<ProfileMetadataU64Schema>>,
    f64s: Option<CanonicalBlockRef<ProfileMetadataF64Schema>>,
    f32s: Option<CanonicalBlockRef<ProfileMetadataF32Schema>>,
) -> Option<()> {
    require_utf8_lane(row, index, profile_metadata_entry_field::TEXTS, true, text)?;
    require_bytes_lane(
        row,
        index,
        profile_metadata_entry_field::BYTES,
        shape.bytes && row.scalar::<u32>(index, profile_metadata_entry_field::BYTE_COUNT)? != 0,
        bytes,
    )?;
    require_sequence_lane::<u8, ProfileMetadataBoolSchema>(
        row,
        index,
        profile_metadata_entry_field::BOOL_VALUES,
        shape.lane == MetadataLane::Bool && shape.scalar_count != 0,
        bools,
    )?;
    require_sequence_lane::<i64, ProfileMetadataI64Schema>(
        row,
        index,
        profile_metadata_entry_field::I64_VALUES,
        shape.lane == MetadataLane::I64 && shape.scalar_count != 0,
        i64s,
    )?;
    require_sequence_lane::<u64, ProfileMetadataU64Schema>(
        row,
        index,
        profile_metadata_entry_field::U64_VALUES,
        shape.lane == MetadataLane::U64 && shape.scalar_count != 0,
        u64s,
    )?;
    require_sequence_lane::<f64, ProfileMetadataF64Schema>(
        row,
        index,
        profile_metadata_entry_field::F64_VALUES,
        shape.lane == MetadataLane::F64 && shape.scalar_count != 0,
        f64s,
    )?;
    require_sequence_lane::<f32, ProfileMetadataF32Schema>(
        row,
        index,
        profile_metadata_entry_field::F32_VALUES,
        shape.lane == MetadataLane::F32 && shape.scalar_count != 0,
        f32s,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_metadata_payload(
    shape: MetadataShape,
    offset: u32,
    count: u32,
    cursors: &mut MetadataCursors,
    bools: &[u8],
    i64s: &[i64],
    u64s: &[u64],
    f64s: &[f64],
    f32s: &[f32],
    expected: Option<ExtensionValueRef<'_>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let count_usize = usize::try_from(count).ok()?;
    match shape.lane {
        MetadataLane::None => (offset == 0 && count == 0).then_some(()),
        MetadataLane::Bool => {
            if expected
                .is_some_and(|expected| metadata_bool_expected_len(expected) != Some(count_usize))
            {
                return None;
            }
            consume_validated_scalar_range(
                bools,
                &mut cursors.bools,
                offset,
                count,
                observe_work,
                |index, value| {
                    *value <= 1
                        && expected.is_none_or(|expected| {
                            metadata_bool_expected_at(expected, index) == Some(*value)
                        })
                },
            )?;
            Some(())
        }
        MetadataLane::I64 => {
            if expected
                .is_some_and(|expected| metadata_i64_expected_len(expected) != Some(count_usize))
            {
                return None;
            }
            consume_validated_scalar_range(
                i64s,
                &mut cursors.i64s,
                offset,
                count,
                observe_work,
                |index, value| {
                    expected.is_none_or(|expected| {
                        metadata_i64_expected_at(expected, index) == Some(*value)
                    })
                },
            )?;
            Some(())
        }
        MetadataLane::U64 => {
            if expected
                .is_some_and(|expected| metadata_u64_expected_len(expected) != Some(count_usize))
            {
                return None;
            }
            consume_validated_scalar_range(
                u64s,
                &mut cursors.u64s,
                offset,
                count,
                observe_work,
                |index, value| {
                    expected.is_none_or(|expected| {
                        metadata_u64_expected_at(expected, index) == Some(*value)
                    })
                },
            )?;
            Some(())
        }
        MetadataLane::F64 => {
            if expected
                .is_some_and(|expected| metadata_f64_expected_len(expected) != Some(count_usize))
            {
                return None;
            }
            consume_validated_scalar_range(
                f64s,
                &mut cursors.f64s,
                offset,
                count,
                observe_work,
                |index, value| {
                    value.is_finite()
                        && expected.is_none_or(|expected| {
                            metadata_f64_expected_at(expected, index)
                                .is_some_and(|expected| expected.to_bits() == value.to_bits())
                        })
                },
            )?;
            Some(())
        }
        MetadataLane::F32 => {
            if expected
                .is_some_and(|expected| metadata_f32_expected_len(expected) != Some(count_usize))
            {
                return None;
            }
            consume_validated_scalar_range(
                f32s,
                &mut cursors.f32s,
                offset,
                count,
                observe_work,
                |index, value| {
                    value.is_finite()
                        && expected.is_none_or(|expected| {
                            metadata_f32_expected_at(expected, index)
                                .is_some_and(|expected| expected.to_bits() == value.to_bits())
                        })
                },
            )?;
            Some(())
        }
    }
}

fn metadata_bool_expected_len(expected: ExtensionValueRef<'_>) -> Option<usize> {
    match expected {
        ExtensionValueRef::Bool(_) => Some(1),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::Bool(values)) => Some(values.len()),
        _ => None,
    }
}

fn metadata_bool_expected_at(expected: ExtensionValueRef<'_>, index: usize) -> Option<u8> {
    match expected {
        ExtensionValueRef::Bool(value) => (index == 0).then_some(u8::from(value)),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::Bool(values)) => {
            values.get(index).copied().map(u8::from)
        }
        _ => None,
    }
}

fn metadata_i64_expected_len(expected: ExtensionValueRef<'_>) -> Option<usize> {
    match expected {
        ExtensionValueRef::I64(_) => Some(1),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::I64(values)) => Some(values.len()),
        _ => None,
    }
}

fn metadata_i64_expected_at(expected: ExtensionValueRef<'_>, index: usize) -> Option<i64> {
    match expected {
        ExtensionValueRef::I64(value) => (index == 0).then_some(value),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::I64(values)) => values.get(index).copied(),
        _ => None,
    }
}

fn metadata_u64_expected_len(expected: ExtensionValueRef<'_>) -> Option<usize> {
    match expected {
        ExtensionValueRef::U64(_) => Some(1),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::U64(values)) => Some(values.len()),
        _ => None,
    }
}

fn metadata_u64_expected_at(expected: ExtensionValueRef<'_>, index: usize) -> Option<u64> {
    match expected {
        ExtensionValueRef::U64(value) => (index == 0).then_some(value),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::U64(values)) => values.get(index).copied(),
        _ => None,
    }
}

fn metadata_f64_expected_len(expected: ExtensionValueRef<'_>) -> Option<usize> {
    match expected {
        ExtensionValueRef::F64(_) => Some(1),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::F64(values)) => Some(values.len()),
        _ => None,
    }
}

fn metadata_f64_expected_at(expected: ExtensionValueRef<'_>, index: usize) -> Option<f64> {
    match expected {
        ExtensionValueRef::F64(value) => (index == 0).then_some(value),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::F64(values)) => values.get(index).copied(),
        _ => None,
    }
}

fn metadata_f32_expected_len(expected: ExtensionValueRef<'_>) -> Option<usize> {
    match expected {
        ExtensionValueRef::Vec2(_) => Some(2),
        ExtensionValueRef::Vec3(_) => Some(3),
        ExtensionValueRef::Vec4(_) | ExtensionValueRef::Color(_) => Some(4),
        ExtensionValueRef::Mat4(_) => Some(16),
        ExtensionValueRef::DenseArray(values) => {
            let (count, width) = match values {
                ExtensionArrayRef::Vec2(values) => (values.len(), 2),
                ExtensionArrayRef::Vec3(values) => (values.len(), 3),
                ExtensionArrayRef::Vec4(values) => (values.len(), 4),
                ExtensionArrayRef::Color(values) => (values.len(), 4),
                ExtensionArrayRef::Mat4(values) => (values.len(), 16),
                _ => return None,
            };
            count.checked_mul(width)
        }
        _ => None,
    }
}

fn metadata_f32_expected_at(expected: ExtensionValueRef<'_>, index: usize) -> Option<f32> {
    match expected {
        ExtensionValueRef::Vec2(value) => [value.x, value.y].get(index).copied(),
        ExtensionValueRef::Vec3(value) => [value.x, value.y, value.z].get(index).copied(),
        ExtensionValueRef::Vec4(value) => [value.x, value.y, value.z, value.w].get(index).copied(),
        ExtensionValueRef::Color(value) => [value.r, value.g, value.b, value.a].get(index).copied(),
        ExtensionValueRef::Mat4(value) => value
            .cols
            .get(index.checked_div(4)?)?
            .get(index.checked_rem(4)?)
            .copied(),
        ExtensionValueRef::DenseArray(values) => match values {
            ExtensionArrayRef::Vec2(values) => {
                let value = values.get(index.checked_div(2)?)?;
                [value.x, value.y].get(index.checked_rem(2)?).copied()
            }
            ExtensionArrayRef::Vec3(values) => {
                let value = values.get(index.checked_div(3)?)?;
                [value.x, value.y, value.z]
                    .get(index.checked_rem(3)?)
                    .copied()
            }
            ExtensionArrayRef::Vec4(values) => {
                let value = values.get(index.checked_div(4)?)?;
                [value.x, value.y, value.z, value.w]
                    .get(index.checked_rem(4)?)
                    .copied()
            }
            ExtensionArrayRef::Color(values) => {
                let value = values.get(index.checked_div(4)?)?;
                [value.r, value.g, value.b, value.a]
                    .get(index.checked_rem(4)?)
                    .copied()
            }
            ExtensionArrayRef::Mat4(values) => {
                let value = values.get(index.checked_div(16)?)?;
                let cell = index.checked_rem(16)?;
                value
                    .cols
                    .get(cell.checked_div(4)?)?
                    .get(cell.checked_rem(4)?)
                    .copied()
            }
            _ => None,
        },
        _ => None,
    }
}
