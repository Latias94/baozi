use baozi_operation::__canonical_activation::{CanonicalLane, CanonicalRecordBuildRow};
use baozi_operation::{
    OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, RecordPlan, StoreBuildError,
    StoreControlledError, StorePlanError, StoreRuntimeError, Utf8Block, Utf8Plan,
};

use crate::canonical_scene::store::production::ValidatedSpatialBatch;
use crate::canonical_scene::store::schema::spatial::{
    SPATIAL_CAMERA_FIELD_COUNT, SPATIAL_DIRECTORY_FIELD_COUNT, SPATIAL_LIGHT_FIELD_COUNT,
    SPATIAL_SCHEMA_REVISION, SpatialCameraTableSchema, SpatialDirectorySchema,
    SpatialLightTableSchema, SpatialTangentBasisSchemaNameSchema,
    SpatialTangentEncodingSchemaNameSchema, encode_axis, encode_camera_projection,
    encode_handedness, encode_light_kind, encode_tangent_basis, encode_tangent_encoding,
    encode_transform_convention, encode_uv_origin, encode_winding,
};
use crate::canonical_scene::store::schema::{CanonicalBlockRef, CanonicalSchema};
use crate::entity::{CameraProjectionKind, LightKindCategory};

use super::super::{
    checked_validation_work_sum, optional_record_validation_work, optional_utf8_validation_work,
    record_validation_work,
};

pub(super) struct SpatialMaterializationPlans {
    tangent_basis_schema_name: Option<Utf8Plan>,
    tangent_encoding_schema_name: Option<Utf8Plan>,
    cameras: Option<RecordPlan>,
    lights: Option<RecordPlan>,
    directory: RecordPlan,
    camera_count: u32,
    light_count: u32,
    tangent_basis_schema_name_bytes: u32,
    tangent_encoding_schema_name_bytes: u32,
    canonical_block_count: usize,
    payload_block_count: u32,
}

impl SpatialMaterializationPlans {
    pub(super) fn new<R>(batch: &ValidatedSpatialBatch<'_, R>) -> Result<Self, StorePlanError> {
        let camera_count = batch.camera_count();
        let light_count = batch.light_count();
        let tangent_basis_schema_name_bytes = batch.tangent_basis_schema_name_bytes();
        let tangent_encoding_schema_name_bytes = batch.tangent_encoding_schema_name_bytes();
        let tangent_basis_schema_name = utf8_plan_if_nonempty(tangent_basis_schema_name_bytes)?;
        let tangent_encoding_schema_name =
            utf8_plan_if_nonempty(tangent_encoding_schema_name_bytes)?;
        let cameras = record_plan_if_nonempty(camera_count, SPATIAL_CAMERA_FIELD_COUNT)?;
        let lights = record_plan_if_nonempty(light_count, SPATIAL_LIGHT_FIELD_COUNT)?;
        let payload_block_count = u32::from(tangent_basis_schema_name.is_some())
            .checked_add(u32::from(tangent_encoding_schema_name.is_some()))
            .ok_or(StorePlanError::SizeOverflow)?;
        if payload_block_count != batch.payload_block_count() {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let canonical_block_count = 1_usize
            .checked_add(usize::from(cameras.is_some()))
            .and_then(|count| count.checked_add(usize::from(lights.is_some())))
            .and_then(|count| {
                usize::try_from(payload_block_count)
                    .ok()?
                    .checked_add(count)
            })
            .ok_or(StorePlanError::SizeOverflow)?;
        Ok(Self {
            tangent_basis_schema_name,
            tangent_encoding_schema_name,
            cameras,
            lights,
            directory: RecordPlan::new(1, SPATIAL_DIRECTORY_FIELD_COUNT)?,
            camera_count,
            light_count,
            tangent_basis_schema_name_bytes,
            tangent_encoding_schema_name_bytes,
            canonical_block_count,
            payload_block_count,
        })
    }

    pub(super) const fn canonical_block_count(&self) -> usize {
        self.canonical_block_count
    }

    pub(super) const fn payload_block_count(&self) -> u32 {
        self.payload_block_count
    }

    pub(super) const fn camera_count(&self) -> u32 {
        self.camera_count
    }

    pub(super) const fn light_count(&self) -> u32 {
        self.light_count
    }

    pub(super) fn validation_work_units(&self) -> Result<u64, StorePlanError> {
        checked_validation_work_sum([
            optional_utf8_validation_work(self.tangent_basis_schema_name)?,
            optional_utf8_validation_work(self.tangent_encoding_schema_name)?,
            optional_record_validation_work(self.cameras)?,
            optional_record_validation_work(self.lights)?,
            record_validation_work(self.directory)?,
        ])
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct RetainedSpatialRefs {
    pub(in crate::canonical_scene::store) directory: CanonicalBlockRef<SpatialDirectorySchema>,
    pub(in crate::canonical_scene::store) cameras:
        Option<CanonicalBlockRef<SpatialCameraTableSchema>>,
    pub(in crate::canonical_scene::store) lights:
        Option<CanonicalBlockRef<SpatialLightTableSchema>>,
    pub(in crate::canonical_scene::store) tangent_basis_schema_name:
        Option<CanonicalBlockRef<SpatialTangentBasisSchemaNameSchema>>,
    pub(in crate::canonical_scene::store) tangent_encoding_schema_name:
        Option<CanonicalBlockRef<SpatialTangentEncodingSchemaNameSchema>>,
}

impl RetainedSpatialRefs {
    pub(super) const fn directory(self) -> CanonicalBlockRef<SpatialDirectorySchema> {
        self.directory
    }

    pub(super) const fn cameras(self) -> Option<CanonicalBlockRef<SpatialCameraTableSchema>> {
        self.cameras
    }

    pub(super) const fn lights(self) -> Option<CanonicalBlockRef<SpatialLightTableSchema>> {
        self.lights
    }
}

pub(super) fn retain_spatial<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &SpatialMaterializationPlans,
    batch: &ValidatedSpatialBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<RetainedSpatialRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let tangent_basis_schema_name = match plans.tangent_basis_schema_name {
        Some(plan) => Some(retain_schema_name(
            lane,
            plan,
            batch
                .scene_space()
                .tangent_basis
                .schema_id()
                .ok_or(StoreRuntimeError::Build(failure))?
                .name()
                .as_str(),
            SpatialTangentBasisSchemaNameSchema,
        )?),
        None => None,
    };
    let tangent_encoding_schema_name = match plans.tangent_encoding_schema_name {
        Some(plan) => Some(retain_schema_name(
            lane,
            plan,
            batch
                .scene_space()
                .tangent_handedness_encoding
                .schema_id()
                .ok_or(StoreRuntimeError::Build(failure))?
                .name()
                .as_str(),
            SpatialTangentEncodingSchemaNameSchema,
        )?),
        None => None,
    };
    let cameras = match plans.cameras {
        Some(plan) => Some(retain_cameras(lane, plan, batch, failure)?),
        None => None,
    };
    let lights = match plans.lights {
        Some(plan) => Some(retain_lights(lane, plan, batch, failure)?),
        None => None,
    };
    let directory = retain_spatial_directory(
        lane,
        plans,
        batch,
        tangent_basis_schema_name,
        tangent_encoding_schema_name,
        cameras,
        lights,
        failure,
    )?;
    Ok(RetainedSpatialRefs {
        directory,
        cameras,
        lights,
        tangent_basis_schema_name,
        tangent_encoding_schema_name,
    })
}

fn retain_schema_name<'lane, O, PP, U, L, Tag>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: Utf8Plan,
    value: &str,
    marker: Tag,
) -> Result<CanonicalBlockRef<Tag>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    lane.try_retain_utf8(plan, marker, |control| control.write_str(value))
        .map(CanonicalBlockRef::from_store)
}

fn retain_cameras<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    batch: &ValidatedSpatialBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<SpatialCameraTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, SpatialCameraTableSchema, |control| {
        for (camera, authored) in batch.cameras().enumerate() {
            let projection = authored.projection();
            control.write_row(|row| {
                row.write_scalar(u32::try_from(camera).map_err(|_| failure)?)?;
                row.write_scalar(encode_camera_projection(projection))?;
                match projection.kind() {
                    CameraProjectionKind::Perspective => {
                        let (primary, secondary, near, far) =
                            projection.perspective_parameters().ok_or(failure)?;
                        row.write_scalar(primary)?;
                        write_optional_f32(row, secondary)?;
                        row.write_scalar(near)?;
                        write_optional_f32(row, far)
                    }
                    CameraProjectionKind::Orthographic => {
                        let (primary, secondary, near, far) =
                            projection.orthographic_parameters().ok_or(failure)?;
                        row.write_scalar(primary)?;
                        write_optional_f32(row, Some(secondary))?;
                        row.write_scalar(near)?;
                        write_optional_f32(row, Some(far))
                    }
                }
            })?;
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_lights<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    batch: &ValidatedSpatialBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<SpatialLightTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, SpatialLightTableSchema, |control| {
        for (light, authored) in batch.lights().enumerate() {
            let kind = authored.kind();
            let (range, inner, outer) = match kind.category() {
                LightKindCategory::Directional => (None, None, None),
                LightKindCategory::Point => (kind.range(), None, None),
                LightKindCategory::Spot => (
                    kind.range(),
                    kind.inner_cone_radians(),
                    kind.outer_cone_radians(),
                ),
            };
            let emission = authored.emission();
            let tint = emission.tint();
            control.write_row(|row| {
                row.write_scalar(u32::try_from(light).map_err(|_| failure)?)?;
                row.write_scalar(encode_light_kind(kind))?;
                write_optional_f32(row, range)?;
                write_optional_f32(row, inner)?;
                write_optional_f32(row, outer)?;
                row.write_scalar(tint.x)?;
                row.write_scalar(tint.y)?;
                row.write_scalar(tint.z)?;
                row.write_scalar(emission.intensity())
            })?;
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_spatial_directory<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &SpatialMaterializationPlans,
    batch: &ValidatedSpatialBatch<'_, R>,
    tangent_basis_schema_name: Option<CanonicalBlockRef<SpatialTangentBasisSchemaNameSchema>>,
    tangent_encoding_schema_name: Option<CanonicalBlockRef<SpatialTangentEncodingSchemaNameSchema>>,
    cameras: Option<CanonicalBlockRef<SpatialCameraTableSchema>>,
    lights: Option<CanonicalBlockRef<SpatialLightTableSchema>>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<SpatialDirectorySchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let space = batch.scene_space();
    lane.try_retain_records(plans.directory, SpatialDirectorySchema, |control| {
        control.write_row(|row| {
            row.write_scalar(SPATIAL_SCHEMA_REVISION)?;
            row.write_scalar(encode_handedness(space.handedness))?;
            row.write_scalar(encode_axis(space.up_axis))?;
            row.write_scalar(encode_axis(space.front_axis))?;
            write_optional_f64(row, space.unit_scale_to_meters)?;
            row.write_scalar(encode_winding(space.front_face_winding))?;
            row.write_scalar(encode_uv_origin(space.uv_origin))?;
            row.write_scalar(encode_transform_convention(space.transform_convention))?;
            row.write_scalar(encode_tangent_basis(&space.tangent_basis))?;
            write_optional_utf8_ref(
                row,
                plans.tangent_basis_schema_name_bytes,
                tangent_basis_schema_name,
                failure,
            )?;
            row.write_scalar(
                space
                    .tangent_basis
                    .schema_id()
                    .map_or(0, |schema| schema.version().get()),
            )?;
            row.write_scalar(encode_tangent_encoding(&space.tangent_handedness_encoding))?;
            write_optional_utf8_ref(
                row,
                plans.tangent_encoding_schema_name_bytes,
                tangent_encoding_schema_name,
                failure,
            )?;
            row.write_scalar(
                space
                    .tangent_handedness_encoding
                    .schema_id()
                    .map_or(0, |schema| schema.version().get()),
            )?;
            write_optional_record_ref(row, plans.camera_count, cameras, failure)?;
            write_optional_record_ref(row, plans.light_count, lights, failure)
        })
    })
    .map(CanonicalBlockRef::from_store)
}

fn write_optional_f32(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    value: Option<f32>,
) -> Result<(), StoreControlledError> {
    match value {
        Some(value) => {
            row.write_scalar(super::super::super::super::schema::PRESENCE_PRESENT)?;
            row.write_scalar(value)
        }
        None => {
            row.write_scalar(super::super::super::super::schema::PRESENCE_ABSENT)?;
            row.write_scalar(0_u8)
        }
    }
}

fn write_optional_f64(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    value: Option<f64>,
) -> Result<(), StoreControlledError> {
    match value {
        Some(value) => {
            row.write_scalar(super::super::super::super::schema::PRESENCE_PRESENT)?;
            row.write_scalar(value)
        }
        None => {
            row.write_scalar(super::super::super::super::schema::PRESENCE_ABSENT)?;
            row.write_scalar(0_u8)
        }
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
    if count == 0 {
        if reference.is_some() {
            return Err(failure.into());
        }
        row.write_scalar(super::super::super::super::schema::PRESENCE_ABSENT)?;
        row.write_scalar(0_u8)
    } else {
        row.write_scalar(super::super::super::super::schema::PRESENCE_PRESENT)?;
        row.write_record_ref(reference.ok_or(failure)?.into_store())
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
    if count == 0 {
        if reference.is_some() {
            return Err(failure.into());
        }
        row.write_scalar(0_u8)
    } else {
        row.write_utf8_ref(reference.ok_or(failure)?.into_store())
    }
}

fn record_plan_if_nonempty(
    count: u32,
    fields: usize,
) -> Result<Option<RecordPlan>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(RecordPlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
            fields,
        )?))
    }
}

fn utf8_plan_if_nonempty(count: u32) -> Result<Option<Utf8Plan>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(Utf8Plan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}
