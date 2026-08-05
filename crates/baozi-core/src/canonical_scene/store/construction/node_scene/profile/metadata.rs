use baozi_operation::{
    ByteBlock, CanonicalRecordRowWriter, RecordBlock, ScalarSequence, SequenceWriter,
    StoreBuildError, StoreControlledError, StoreRuntimeError, StoredScalar, Utf8Block, Utf8Writer,
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

use super::{MetadataLeafRefs, ProfileBuildFault, ProfileMaterializationPlans, encode_owner};
use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneWorkMeter, process_slice_in_work_chunks, process_str_in_work_chunks,
};
use crate::canonical_scene::store::construction::{SceneStoreAssembly, SceneStoreKeys};

#[derive(Clone, Copy)]
pub(super) struct RetainedMetadataRefs {
    pub(super) directory: CanonicalBlockRef<ProfileMetadataDirectorySchema>,
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

pub(super) fn retain_metadata<R>(
    assembly: &mut SceneStoreAssembly,
    plans: &ProfileMaterializationPlans,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    faults: &mut ProfileBuildFault,
    failure: StoreBuildError,
) -> Result<RetainedMetadataRefs, StoreRuntimeError> {
    let text = match plans.metadata_text {
        Some(plan) => Some(retain_text(assembly, plan, batch, keys, work, failure)?),
        None => None,
    };
    let bytes = match plans.metadata_bytes {
        Some(plan) => Some(retain_bytes(assembly, plan, batch, keys, work, failure)?),
        None => None,
    };
    let bools = match plans.metadata_bool {
        Some(plan) => Some(retain_bools(assembly, plan, batch, keys, work, failure)?),
        None => None,
    };
    let i64s = match plans.metadata_i64 {
        Some(plan) => Some(retain_i64s(assembly, plan, batch, keys, work, failure)?),
        None => None,
    };
    let u64s = match plans.metadata_u64 {
        Some(plan) => Some(retain_u64s(assembly, plan, batch, keys, work, failure)?),
        None => None,
    };
    let f64s = match plans.metadata_f64 {
        Some(plan) => Some(retain_f64s(assembly, plan, batch, keys, work, failure)?),
        None => None,
    };
    let f32s = match plans.metadata_f32 {
        Some(plan) => Some(retain_f32s(assembly, plan, batch, keys, work, failure)?),
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
            assembly, plan, batch, refs, keys, work, faults, failure,
        )?),
        None => None,
    };
    let directory = retain_directory(assembly, plans, batch, refs, keys, work, faults, failure)?;
    Ok(RetainedMetadataRefs { directory })
}

fn retain_text<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::Utf8Plan,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataTextSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_utf8_controlled(
            plan,
            keys.dimension,
            ProfileMetadataTextSchema,
            |writer, control| {
                for index in 0..batch.metadata_entry_count() {
                    work.observe(control, 1)?;
                    let entry = batch
                        .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                        .ok_or(failure)?;
                    write_text(writer, control, work, entry.key().as_str(), failure)?;
                    match entry.value() {
                        ExtensionValueRef::String(value) => {
                            write_text(writer, control, work, value, failure)?;
                        }
                        ExtensionValueRef::Bytes { media_type, .. } => {
                            write_text(writer, control, work, media_type.as_str(), failure)?;
                        }
                        _ => {}
                    }
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn write_text(
    writer: &mut Utf8Writer,
    control: &mut baozi_operation::RetainedStoreWorkControl<
        '_,
        crate::canonical_scene::store::construction::SceneConstructionOwner,
        crate::canonical_scene::store::construction::SceneConstructionPhase,
        crate::canonical_scene::store::construction::SceneConstructionUnit,
    >,
    work: &mut NodeSceneWorkMeter,
    value: &str,
    failure: StoreBuildError,
) -> Result<(), baozi_operation::StoreControlledError> {
    process_str_in_work_chunks(value, control, work, failure, |chunk| {
        writer.write_str(chunk)?;
        Ok(())
    })
}

fn retain_bytes<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::BytesPlan,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataBytesSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_bytes_controlled(
            plan,
            keys.dimension,
            ProfileMetadataBytesSchema,
            |writer, control| {
                for index in 0..batch.metadata_entry_count() {
                    work.observe(control, 1)?;
                    let entry = batch
                        .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                        .ok_or(failure)?;
                    if let ExtensionValueRef::Bytes { payload, .. } = entry.value() {
                        process_slice_in_work_chunks(payload, control, work, failure, |chunk| {
                            writer.write_bytes(chunk)
                        })?;
                    }
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_bools<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<u8>,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataBoolSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            ProfileMetadataBoolSchema,
            |writer, control| {
                for index in 0..batch.metadata_entry_count() {
                    work.observe(control, 1)?;
                    let value = batch
                        .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                        .ok_or(failure)?
                        .value();
                    match value {
                        ExtensionValueRef::Bool(value) => {
                            work.observe(control, 1)?;
                            writer.extend_from_slice(&[u8::from(value)])?;
                        }
                        ExtensionValueRef::DenseArray(ExtensionArrayRef::Bool(values)) => {
                            process_slice_in_work_chunks(
                                values,
                                control,
                                work,
                                failure,
                                |chunk| {
                                    for value in chunk {
                                        writer.push(u8::from(*value))?;
                                    }
                                    Ok(())
                                },
                            )?;
                        }
                        _ => {}
                    }
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_i64s<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<i64>,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataI64Schema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            ProfileMetadataI64Schema,
            |writer, control| {
                for index in 0..batch.metadata_entry_count() {
                    work.observe(control, 1)?;
                    let value = batch
                        .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                        .ok_or(failure)?
                        .value();
                    match value {
                        ExtensionValueRef::I64(value) => {
                            work.observe(control, 1)?;
                            writer.push(value)?;
                        }
                        ExtensionValueRef::DenseArray(ExtensionArrayRef::I64(values)) => {
                            process_slice_in_work_chunks(
                                values,
                                control,
                                work,
                                failure,
                                |chunk| writer.extend_from_slice(chunk),
                            )?;
                        }
                        _ => {}
                    }
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_u64s<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<u64>,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataU64Schema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            ProfileMetadataU64Schema,
            |writer, control| {
                for index in 0..batch.metadata_entry_count() {
                    work.observe(control, 1)?;
                    let value = batch
                        .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                        .ok_or(failure)?
                        .value();
                    match value {
                        ExtensionValueRef::U64(value) => {
                            work.observe(control, 1)?;
                            writer.push(value)?;
                        }
                        ExtensionValueRef::DenseArray(ExtensionArrayRef::U64(values)) => {
                            process_slice_in_work_chunks(
                                values,
                                control,
                                work,
                                failure,
                                |chunk| writer.extend_from_slice(chunk),
                            )?;
                        }
                        _ => {}
                    }
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_f64s<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<f64>,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataF64Schema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            ProfileMetadataF64Schema,
            |writer, control| {
                for index in 0..batch.metadata_entry_count() {
                    work.observe(control, 1)?;
                    let value = batch
                        .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                        .ok_or(failure)?
                        .value();
                    match value {
                        ExtensionValueRef::F64(value) => {
                            work.observe(control, 1)?;
                            writer.push(value)?;
                        }
                        ExtensionValueRef::DenseArray(ExtensionArrayRef::F64(values)) => {
                            process_slice_in_work_chunks(
                                values,
                                control,
                                work,
                                failure,
                                |chunk| writer.extend_from_slice(chunk),
                            )?;
                        }
                        _ => {}
                    }
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_f32s<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<f32>,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataF32Schema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            ProfileMetadataF32Schema,
            |writer, control| {
                for index in 0..batch.metadata_entry_count() {
                    work.observe(control, 1)?;
                    let value = batch
                        .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                        .ok_or(failure)?
                        .value();
                    write_f32_value(writer, control, work, value, failure)?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn write_f32_value(
    writer: &mut SequenceWriter<f32>,
    control: &mut baozi_operation::RetainedStoreWorkControl<
        '_,
        crate::canonical_scene::store::construction::SceneConstructionOwner,
        crate::canonical_scene::store::construction::SceneConstructionPhase,
        crate::canonical_scene::store::construction::SceneConstructionUnit,
    >,
    work: &mut NodeSceneWorkMeter,
    value: ExtensionValueRef<'_>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError> {
    macro_rules! write_values {
        ($values:expr) => {{
            let values = $values;
            work.observe(control, u64::try_from(values.len()).map_err(|_| failure)?)?;
            writer.extend_from_slice(&values)?;
        }};
    }
    macro_rules! write_matrix {
        ($value:expr) => {{
            let value = $value;
            work.observe(control, 16)?;
            for column in &value.cols {
                writer.extend_from_slice(column)?;
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

#[allow(clippy::too_many_arguments)]
fn retain_entries<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::RecordPlan,
    batch: &ValidatedProfileBatch<'_, R>,
    refs: MetadataLeafRefs,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    faults: &mut ProfileBuildFault,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataEntryTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            ProfileMetadataEntryTableSchema,
            |writer, control| {
                let mut cursors = MetadataCursors::default();
                for index in 0..batch.metadata_entry_count() {
                    work.observe(control, 1)?;
                    let entry = batch
                        .metadata_entry(usize::try_from(index).map_err(|_| failure)?)
                        .ok_or(failure)?;
                    let encoding = metadata_encoding(entry.value(), failure)?;
                    let key_offset = cursors.text;
                    let key_count =
                        u32::try_from(entry.key().as_str().len()).map_err(|_| failure)?;
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
                    advance_scalar_cursor(
                        &mut cursors,
                        encoding.lane,
                        encoding.scalar_count,
                        failure,
                    )?;
                    let (owner_kind, owner_index) = encode_owner(entry.owner());
                    let stored_scalar_count = faults.metadata_scalar_count(encoding.scalar_count);
                    let omit_bool_lane_reference = matches!(encoding.lane, ScalarLane::Bool)
                        && faults.omit_metadata_bool_lane_reference();
                    work.observe(control, 1)?;
                    writer.write_row(|row| {
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
                        write_sequence_lane(
                            row,
                            matches!(encoding.lane, ScalarLane::Bool)
                                && encoding.scalar_count != 0
                                && !omit_bool_lane_reference,
                            refs.bools,
                            failure,
                        )?;
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
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the admitted writer boundary keeps storage, work, fault, and failure authority explicit"
)]
fn retain_directory<R>(
    assembly: &mut SceneStoreAssembly,
    plans: &ProfileMaterializationPlans,
    batch: &ValidatedProfileBatch<'_, R>,
    refs: MetadataLeafRefs,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    faults: &mut ProfileBuildFault,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileMetadataDirectorySchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records_controlled(
            plans.metadata_directory,
            keys.dimension,
            ProfileMetadataDirectorySchema,
            |writer, control| {
                work.observe(control, 1)?;
                writer
                    .write_row(|row| {
                        row.write_scalar(PROFILE_METADATA_SCHEMA_REVISION)?;
                        let shape = batch.retention_shape();
                        if faults.omit_metadata_entries_reference() {
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
                        write_optional_utf8_ref(
                            row,
                            shape.metadata_text_bytes(),
                            refs.text,
                            failure,
                        )?;
                        write_optional_bytes_ref(
                            row,
                            shape.metadata_payload_bytes(),
                            refs.bytes,
                            failure,
                        )?;
                        write_optional_sequence_ref(
                            row,
                            shape.metadata_bool_count(),
                            refs.bools,
                            failure,
                        )?;
                        write_optional_sequence_ref(
                            row,
                            shape.metadata_i64_count(),
                            refs.i64s,
                            failure,
                        )?;
                        write_optional_sequence_ref(
                            row,
                            shape.metadata_u64_count(),
                            refs.u64s,
                            failure,
                        )?;
                        write_optional_sequence_ref(
                            row,
                            shape.metadata_f64_count(),
                            refs.f64s,
                            failure,
                        )?;
                        write_optional_sequence_ref(
                            row,
                            shape.metadata_f32_count(),
                            refs.f32s,
                            failure,
                        )
                    })
                    .map_err(StoreControlledError::from)
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
    row: &mut CanonicalRecordRowWriter<'_>,
    used: bool,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    match (used, reference) {
        (true, Some(reference)) => row.write_utf8_ref(reference.into_store()),
        (false, _) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (true, None) => Err(failure),
    }
}
fn write_bytes_lane<Tag>(
    row: &mut CanonicalRecordRowWriter<'_>,
    used: bool,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = ByteBlock>,
{
    match (used, reference) {
        (true, Some(reference)) => row.write_bytes_ref(reference.into_store()),
        (false, _) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (true, None) => Err(failure),
    }
}
fn write_sequence_lane<Tag, T>(
    row: &mut CanonicalRecordRowWriter<'_>,
    used: bool,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
    T: StoredScalar,
{
    match (used, reference) {
        (true, Some(reference)) => row.write_sequence_ref(reference.into_store()),
        (false, _) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (true, None) => Err(failure),
    }
}

fn write_optional_record_ref<Tag>(
    row: &mut CanonicalRecordRowWriter<'_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    row.write_scalar(count)?;
    match (count, reference) {
        (0, None) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (0, Some(_)) | (1.., None) => Err(failure),
        (1.., Some(reference)) => row.write_record_ref(reference.into_store()),
    }
}
fn write_optional_utf8_ref<Tag>(
    row: &mut CanonicalRecordRowWriter<'_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    row.write_scalar(count)?;
    write_utf8_lane(row, count != 0, reference, failure)
}
fn write_optional_bytes_ref<Tag>(
    row: &mut CanonicalRecordRowWriter<'_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = ByteBlock>,
{
    row.write_scalar(count)?;
    write_bytes_lane(row, count != 0, reference, failure)
}
fn write_optional_sequence_ref<Tag, T>(
    row: &mut CanonicalRecordRowWriter<'_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
    T: StoredScalar,
{
    row.write_scalar(count)?;
    write_sequence_lane(row, count != 0, reference, failure)
}
