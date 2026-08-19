use baozi_operation::__retained_store_integration::activation::{
    CanonicalBuildControl, CanonicalLane, CanonicalRecordBuildRow,
};
use baozi_operation::{
    ByteBlock, OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, ScalarSequence,
    SequenceWriter, StoreBuildError, StoreControlledError, StoreRuntimeError, StoredScalar,
    Utf8Block,
};

use crate::canonical_scene::store::production::ValidatedProfileBatch;
use crate::canonical_scene::store::schema::profile::{
    PROFILE_ABSENT_REFERENCE, PROFILE_METADATA_ARRAY_BOOL, PROFILE_METADATA_ARRAY_COLOR,
    PROFILE_METADATA_ARRAY_F64, PROFILE_METADATA_ARRAY_I64, PROFILE_METADATA_ARRAY_MAT4,
    PROFILE_METADATA_ARRAY_U64, PROFILE_METADATA_ARRAY_VEC2, PROFILE_METADATA_ARRAY_VEC3,
    PROFILE_METADATA_ARRAY_VEC4, PROFILE_METADATA_BOOL, PROFILE_METADATA_BYTES,
    PROFILE_METADATA_COLOR, PROFILE_METADATA_DENSE_ARRAY, PROFILE_METADATA_F64,
    PROFILE_METADATA_I64, PROFILE_METADATA_MAT4, PROFILE_METADATA_SCHEMA_REVISION,
    PROFILE_METADATA_STRING, PROFILE_METADATA_U64, PROFILE_METADATA_VEC2, PROFILE_METADATA_VEC3,
    PROFILE_METADATA_VEC4, ProfileMetadataBoolSchema, ProfileMetadataBytesSchema,
    ProfileMetadataDirectorySchema, ProfileMetadataEntryTableSchema, ProfileMetadataF32Schema,
    ProfileMetadataF64Schema, ProfileMetadataI64Schema, ProfileMetadataTextSchema,
    ProfileMetadataU64Schema,
};
use crate::canonical_scene::store::schema::{CanonicalBlockRef, CanonicalSchema};
use crate::extension::{ExtensionArrayRef, ExtensionValueRef};

#[cfg(test)]
use super::ProfileBuildFault;
use super::{MetadataLeafRefs, ProfileMaterializationPlans, encode_owner};

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct RetainedMetadataRefs {
    pub(in crate::canonical_scene::store) directory:
        CanonicalBlockRef<ProfileMetadataDirectorySchema>,
    pub(in crate::canonical_scene::store) leaves: MetadataLeafRefs,
}

#[derive(Clone, Copy)]
enum ScalarLane {
    None,
    Bool,
    I64,
    U64,
    F64,
    F32,
}

#[derive(Clone, Copy)]
struct MetadataEncoding {
    value_kind: u8,
    array_kind: u8,
    logical_count: u32,
    auxiliary_text_count: u32,
    byte_count: u32,
    scalar_count: u32,
    lane: ScalarLane,
}

#[derive(Default)]
struct MetadataCursors {
    text: u32,
    bytes: u32,
    bools: u32,
    i64s: u32,
    u64s: u32,
    f64s: u32,
    f32s: u32,
}

pub(super) fn retain_metadata<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &ProfileMaterializationPlans,
    batch: &ValidatedProfileBatch<'_, R>,
    #[cfg(test)] faults: &mut ProfileBuildFault,
    failure: StoreBuildError,
) -> Result<RetainedMetadataRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let text = match plans.metadata_text {
        Some(plan) => Some(retain_text(lane, plan, batch, failure)?),
        None => None,
    };
    let bytes = match plans.metadata_bytes {
        Some(plan) => Some(retain_bytes(lane, plan, batch, failure)?),
        None => None,
    };
    let bools = match plans.metadata_bool {
        Some(plan) => Some(retain_bools(lane, plan, batch, failure)?),
        None => None,
    };
    let i64s = match plans.metadata_i64 {
        Some(plan) => Some(retain_i64s(lane, plan, batch, failure)?),
        None => None,
    };
    let u64s = match plans.metadata_u64 {
        Some(plan) => Some(retain_u64s(lane, plan, batch, failure)?),
        None => None,
    };
    let f64s = match plans.metadata_f64 {
        Some(plan) => Some(retain_f64s(lane, plan, batch, failure)?),
        None => None,
    };
    let f32s = match plans.metadata_f32 {
        Some(plan) => Some(retain_f32s(lane, plan, batch, failure)?),
        None => None,
    };
    let mut refs = MetadataLeafRefs {
        entries: None,
        text,
        bytes,
        bools,
        i64s,
        u64s,
        f64s,
        f32s,
    };
    refs.entries = match plans.metadata_entries {
        Some(plan) => Some(retain_entries(
            lane,
            plan,
            batch,
            refs,
            #[cfg(test)]
            faults,
            failure,
        )?),
        None => None,
    };
    let directory = retain_directory(
        lane,
        plans,
        batch,
        refs,
        #[cfg(test)]
        faults,
        failure,
    )?;
    Ok(RetainedMetadataRefs {
        directory,
        leaves: refs,
    })
}

fn retain_text<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: baozi_operation::Utf8Plan,
    batch: &ValidatedProfileBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataTextSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_utf8(plan, ProfileMetadataTextSchema, |control| {
        for index in 0..batch.metadata_entry_count() {
            let entry = batch
                .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                .ok_or(failure)?;
            control.write_str(entry.key().as_str())?;
            match entry.value() {
                ExtensionValueRef::String(value) => {
                    control.write_str(value)?;
                }
                ExtensionValueRef::Bytes { media_type, .. } => {
                    control.write_str(media_type.as_str())?;
                }
                _ => {}
            }
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_bytes<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: baozi_operation::BytesPlan,
    batch: &ValidatedProfileBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataBytesSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_bytes(plan, ProfileMetadataBytesSchema, |control| {
        for index in 0..batch.metadata_entry_count() {
            let entry = batch
                .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                .ok_or(failure)?;
            if let ExtensionValueRef::Bytes { payload, .. } = entry.value() {
                control.write_bytes(payload)?;
            }
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_bools<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: baozi_operation::SequencePlan<u8>,
    batch: &ValidatedProfileBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataBoolSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, ProfileMetadataBoolSchema, |control| {
        for index in 0..batch.metadata_entry_count() {
            let value = batch
                .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                .ok_or(failure)?
                .value();
            match value {
                ExtensionValueRef::Bool(value) => {
                    control.push(u8::from(value))?;
                }
                ExtensionValueRef::DenseArray(ExtensionArrayRef::Bool(values)) => {
                    for value in values {
                        control.push(u8::from(*value))?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_i64s<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: baozi_operation::SequencePlan<i64>,
    batch: &ValidatedProfileBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataI64Schema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, ProfileMetadataI64Schema, |control| {
        for index in 0..batch.metadata_entry_count() {
            let value = batch
                .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                .ok_or(failure)?
                .value();
            match value {
                ExtensionValueRef::I64(value) => {
                    control.push(value)?;
                }
                ExtensionValueRef::DenseArray(ExtensionArrayRef::I64(values)) => {
                    control.extend_from_slice(values)?;
                }
                _ => {}
            }
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_u64s<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: baozi_operation::SequencePlan<u64>,
    batch: &ValidatedProfileBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataU64Schema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, ProfileMetadataU64Schema, |control| {
        for index in 0..batch.metadata_entry_count() {
            let value = batch
                .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                .ok_or(failure)?
                .value();
            match value {
                ExtensionValueRef::U64(value) => {
                    control.push(value)?;
                }
                ExtensionValueRef::DenseArray(ExtensionArrayRef::U64(values)) => {
                    control.extend_from_slice(values)?;
                }
                _ => {}
            }
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_f64s<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: baozi_operation::SequencePlan<f64>,
    batch: &ValidatedProfileBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataF64Schema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, ProfileMetadataF64Schema, |control| {
        for index in 0..batch.metadata_entry_count() {
            let value = batch
                .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                .ok_or(failure)?
                .value();
            match value {
                ExtensionValueRef::F64(value) => {
                    control.push(value)?;
                }
                ExtensionValueRef::DenseArray(ExtensionArrayRef::F64(values)) => {
                    control.extend_from_slice(values)?;
                }
                _ => {}
            }
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_f32s<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: baozi_operation::SequencePlan<f32>,
    batch: &ValidatedProfileBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataF32Schema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, ProfileMetadataF32Schema, |control| {
        for index in 0..batch.metadata_entry_count() {
            let value = batch
                .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                .ok_or(failure)?
                .value();
            write_f32_value(control, value)?;
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn write_f32_value<O, PP, U>(
    control: &mut CanonicalBuildControl<'_, '_, '_, O, PP, U, SequenceWriter<f32>>,
    value: ExtensionValueRef<'_>,
) -> Result<(), StoreControlledError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    macro_rules! write_values {
        ($values:expr) => {{
            let values = $values;
            control.extend_from_slice(&values)?;
        }};
    }
    macro_rules! write_matrix {
        ($value:expr) => {{
            let value = $value;
            for column in &value.cols {
                control.extend_from_slice(column)?;
            }
        }};
    }
    match value {
        ExtensionValueRef::Vec2(value) => write_values!([value.x, value.y]),
        ExtensionValueRef::Vec3(value) => write_values!([value.x, value.y, value.z]),
        ExtensionValueRef::Vec4(value) => write_values!([value.x, value.y, value.z, value.w]),
        ExtensionValueRef::Color(value) => write_values!([value.r, value.g, value.b, value.a]),
        ExtensionValueRef::Mat4(value) => write_matrix!(value),
        ExtensionValueRef::DenseArray(values) => match values {
            ExtensionArrayRef::Vec2(values) => {
                for value in values {
                    write_values!([value.x, value.y]);
                }
            }
            ExtensionArrayRef::Vec3(values) => {
                for value in values {
                    write_values!([value.x, value.y, value.z]);
                }
            }
            ExtensionArrayRef::Vec4(values) => {
                for value in values {
                    write_values!([value.x, value.y, value.z, value.w]);
                }
            }
            ExtensionArrayRef::Color(values) => {
                for value in values {
                    write_values!([value.r, value.g, value.b, value.a]);
                }
            }
            ExtensionArrayRef::Mat4(values) => {
                for value in values {
                    write_matrix!(value);
                }
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

fn retain_entries<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: baozi_operation::RecordPlan,
    batch: &ValidatedProfileBatch<'_, R>,
    refs: MetadataLeafRefs,
    #[cfg(test)] faults: &mut ProfileBuildFault,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataEntryTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, ProfileMetadataEntryTableSchema, |control| {
        let mut cursors = MetadataCursors::default();
        for index in 0..batch.metadata_entry_count() {
            let entry = batch
                .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                .ok_or(failure)?;
            let encoding = metadata_encoding(entry.value(), failure)?;
            let key_offset = cursors.text;
            let key_count = u32::try_from(entry.key().as_str().len()).map_err(|_| failure)?;
            cursors.text = cursors.text.checked_add(key_count).ok_or(failure)?;
            let auxiliary_text_offset = cursors.text;
            cursors.text = cursors
                .text
                .checked_add(encoding.auxiliary_text_count)
                .ok_or(failure)?;
            let byte_offset = cursors.bytes;
            cursors.bytes = cursors
                .bytes
                .checked_add(encoding.byte_count)
                .ok_or(failure)?;
            let scalar_offset = scalar_cursor(&cursors, encoding.lane);
            advance_scalar_cursor(&mut cursors, encoding.lane, encoding.scalar_count, failure)?;
            let (owner_kind, owner_index) = encode_owner(entry.owner());
            let stored_scalar_count = encoding.scalar_count;
            #[cfg(test)]
            let stored_scalar_count = faults.metadata_scalar_count(stored_scalar_count);
            let write_bool_lane =
                matches!(encoding.lane, ScalarLane::Bool) && encoding.scalar_count != 0;
            #[cfg(test)]
            let write_bool_lane = write_bool_lane && !faults.omit_metadata_bool_lane_reference();
            control.write_row(|row| {
                row.write_scalar(owner_kind)?;
                row.write_scalar(owner_index)?;
                row.write_scalar(key_offset)?;
                row.write_scalar(key_count)?;
                row.write_scalar(encoding.value_kind)?;
                row.write_scalar(encoding.array_kind)?;
                row.write_scalar(encoding.logical_count)?;
                row.write_scalar(auxiliary_text_offset)?;
                row.write_scalar(encoding.auxiliary_text_count)?;
                row.write_scalar(byte_offset)?;
                row.write_scalar(encoding.byte_count)?;
                row.write_scalar(scalar_offset)?;
                row.write_scalar(stored_scalar_count)?;
                write_utf8_lane(row, true, refs.text, failure)?;
                write_bytes_lane(row, encoding.byte_count != 0, refs.bytes, failure)?;
                write_sequence_lane(row, write_bool_lane, refs.bools, failure)?;
                write_sequence_lane(
                    row,
                    matches!(encoding.lane, ScalarLane::I64) && encoding.scalar_count != 0,
                    refs.i64s,
                    failure,
                )?;
                write_sequence_lane(
                    row,
                    matches!(encoding.lane, ScalarLane::U64) && encoding.scalar_count != 0,
                    refs.u64s,
                    failure,
                )?;
                write_sequence_lane(
                    row,
                    matches!(encoding.lane, ScalarLane::F64) && encoding.scalar_count != 0,
                    refs.f64s,
                    failure,
                )?;
                write_sequence_lane(
                    row,
                    matches!(encoding.lane, ScalarLane::F32) && encoding.scalar_count != 0,
                    refs.f32s,
                    failure,
                )
            })?;
        }
        let shape = batch.retention_shape();
        if cursors.text != shape.metadata_text_bytes()
            || cursors.bytes != shape.metadata_payload_bytes()
            || cursors.bools != shape.metadata_bool_count()
            || cursors.i64s != shape.metadata_i64_count()
            || cursors.u64s != shape.metadata_u64_count()
            || cursors.f64s != shape.metadata_f64_count()
            || cursors.f32s != shape.metadata_f32_count()
        {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_directory<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &ProfileMaterializationPlans,
    batch: &ValidatedProfileBatch<'_, R>,
    refs: MetadataLeafRefs,
    #[cfg(test)] faults: &mut ProfileBuildFault,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataDirectorySchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(
        plans.metadata_directory,
        ProfileMetadataDirectorySchema,
        |control| {
            control.write_row(|row| {
                row.write_scalar(PROFILE_METADATA_SCHEMA_REVISION)?;
                let shape = batch.retention_shape();
                #[cfg(test)]
                let omit_entries_reference = faults.omit_metadata_entries_reference();
                #[cfg(not(test))]
                let omit_entries_reference = false;
                if omit_entries_reference {
                    row.write_scalar(batch.metadata_entry_count())?;
                    row.write_scalar(PROFILE_ABSENT_REFERENCE)?;
                } else {
                    write_optional_record_ref(
                        row,
                        batch.metadata_entry_count(),
                        refs.entries,
                        failure,
                    )?;
                }
                write_optional_utf8_ref(row, shape.metadata_text_bytes(), refs.text, failure)?;
                write_optional_bytes_ref(row, shape.metadata_payload_bytes(), refs.bytes, failure)?;
                write_optional_sequence_ref(row, shape.metadata_bool_count(), refs.bools, failure)?;
                write_optional_sequence_ref(row, shape.metadata_i64_count(), refs.i64s, failure)?;
                write_optional_sequence_ref(row, shape.metadata_u64_count(), refs.u64s, failure)?;
                write_optional_sequence_ref(row, shape.metadata_f64_count(), refs.f64s, failure)?;
                write_optional_sequence_ref(row, shape.metadata_f32_count(), refs.f32s, failure)
            })
        },
    )
    .map(CanonicalBlockRef::from_store)
}

fn metadata_encoding(
    value: ExtensionValueRef<'_>,
    failure: StoreBuildError,
) -> Result<MetadataEncoding, StoreBuildError> {
    let scalar = |value_kind: u8, scalar_count: u32, lane: ScalarLane| MetadataEncoding {
        value_kind,
        array_kind: PROFILE_METADATA_ARRAY_BOOL,
        logical_count: 1,
        auxiliary_text_count: 0,
        byte_count: 0,
        scalar_count,
        lane,
    };
    let encoding = match value {
        ExtensionValueRef::Bool(_) => scalar(PROFILE_METADATA_BOOL, 1, ScalarLane::Bool),
        ExtensionValueRef::I64(_) => scalar(PROFILE_METADATA_I64, 1, ScalarLane::I64),
        ExtensionValueRef::U64(_) => scalar(PROFILE_METADATA_U64, 1, ScalarLane::U64),
        ExtensionValueRef::F64(_) => scalar(PROFILE_METADATA_F64, 1, ScalarLane::F64),
        ExtensionValueRef::String(value) => MetadataEncoding {
            value_kind: PROFILE_METADATA_STRING,
            array_kind: PROFILE_METADATA_ARRAY_BOOL,
            logical_count: 1,
            auxiliary_text_count: u32::try_from(value.len()).map_err(|_| failure)?,
            byte_count: 0,
            scalar_count: 0,
            lane: ScalarLane::None,
        },
        ExtensionValueRef::Bytes {
            media_type,
            payload,
        } => MetadataEncoding {
            value_kind: PROFILE_METADATA_BYTES,
            array_kind: PROFILE_METADATA_ARRAY_BOOL,
            logical_count: 1,
            auxiliary_text_count: u32::try_from(media_type.as_str().len()).map_err(|_| failure)?,
            byte_count: u32::try_from(payload.len()).map_err(|_| failure)?,
            scalar_count: 0,
            lane: ScalarLane::None,
        },
        ExtensionValueRef::Vec2(_) => scalar(PROFILE_METADATA_VEC2, 2, ScalarLane::F32),
        ExtensionValueRef::Vec3(_) => scalar(PROFILE_METADATA_VEC3, 3, ScalarLane::F32),
        ExtensionValueRef::Vec4(_) => scalar(PROFILE_METADATA_VEC4, 4, ScalarLane::F32),
        ExtensionValueRef::Color(_) => scalar(PROFILE_METADATA_COLOR, 4, ScalarLane::F32),
        ExtensionValueRef::Mat4(_) => scalar(PROFILE_METADATA_MAT4, 16, ScalarLane::F32),
        ExtensionValueRef::DenseArray(values) => {
            let (array_kind, physical_width, lane) = match values {
                ExtensionArrayRef::Bool(_) => (PROFILE_METADATA_ARRAY_BOOL, 1, ScalarLane::Bool),
                ExtensionArrayRef::I64(_) => (PROFILE_METADATA_ARRAY_I64, 1, ScalarLane::I64),
                ExtensionArrayRef::U64(_) => (PROFILE_METADATA_ARRAY_U64, 1, ScalarLane::U64),
                ExtensionArrayRef::F64(_) => (PROFILE_METADATA_ARRAY_F64, 1, ScalarLane::F64),
                ExtensionArrayRef::Vec2(_) => (PROFILE_METADATA_ARRAY_VEC2, 2, ScalarLane::F32),
                ExtensionArrayRef::Vec3(_) => (PROFILE_METADATA_ARRAY_VEC3, 3, ScalarLane::F32),
                ExtensionArrayRef::Vec4(_) => (PROFILE_METADATA_ARRAY_VEC4, 4, ScalarLane::F32),
                ExtensionArrayRef::Color(_) => (PROFILE_METADATA_ARRAY_COLOR, 4, ScalarLane::F32),
                ExtensionArrayRef::Mat4(_) => (PROFILE_METADATA_ARRAY_MAT4, 16, ScalarLane::F32),
            };
            let logical_count = u32::try_from(values.len()).map_err(|_| failure)?;
            MetadataEncoding {
                value_kind: PROFILE_METADATA_DENSE_ARRAY,
                array_kind,
                logical_count,
                auxiliary_text_count: 0,
                byte_count: 0,
                scalar_count: logical_count.checked_mul(physical_width).ok_or(failure)?,
                lane,
            }
        }
    };
    Ok(encoding)
}

fn scalar_cursor(cursors: &MetadataCursors, lane: ScalarLane) -> u32 {
    match lane {
        ScalarLane::None => 0,
        ScalarLane::Bool => cursors.bools,
        ScalarLane::I64 => cursors.i64s,
        ScalarLane::U64 => cursors.u64s,
        ScalarLane::F64 => cursors.f64s,
        ScalarLane::F32 => cursors.f32s,
    }
}

fn advance_scalar_cursor(
    cursors: &mut MetadataCursors,
    lane: ScalarLane,
    count: u32,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError> {
    let cursor = match lane {
        ScalarLane::None => return if count == 0 { Ok(()) } else { Err(failure) },
        ScalarLane::Bool => &mut cursors.bools,
        ScalarLane::I64 => &mut cursors.i64s,
        ScalarLane::U64 => &mut cursors.u64s,
        ScalarLane::F64 => &mut cursors.f64s,
        ScalarLane::F32 => &mut cursors.f32s,
    };
    *cursor = cursor.checked_add(count).ok_or(failure)?;
    Ok(())
}

fn write_utf8_lane<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    used: bool,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    match (used, reference) {
        (true, Some(reference)) => row.write_utf8_ref(reference.into_store()),
        (false, _) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (true, None) => Err(failure.into()),
    }
}
fn write_bytes_lane<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    used: bool,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = ByteBlock>,
{
    match (used, reference) {
        (true, Some(reference)) => row.write_bytes_ref(reference.into_store()),
        (false, _) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (true, None) => Err(failure.into()),
    }
}
fn write_sequence_lane<Tag, T>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    used: bool,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
    T: StoredScalar,
{
    match (used, reference) {
        (true, Some(reference)) => row.write_sequence_ref(reference.into_store()),
        (false, _) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (true, None) => Err(failure.into()),
    }
}

fn write_optional_record_ref<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    row.write_scalar(count)?;
    match (count, reference) {
        (0, None) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (0, Some(_)) | (1.., None) => Err(failure.into()),
        (1.., Some(reference)) => row.write_record_ref(reference.into_store()),
    }
}
fn write_optional_utf8_ref<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    row.write_scalar(count)?;
    write_utf8_lane(row, count != 0, reference, failure)
}
fn write_optional_bytes_ref<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = ByteBlock>,
{
    row.write_scalar(count)?;
    write_bytes_lane(row, count != 0, reference, failure)
}
fn write_optional_sequence_ref<Tag, T>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
    T: StoredScalar,
{
    row.write_scalar(count)?;
    write_sequence_lane(row, count != 0, reference, failure)
}
