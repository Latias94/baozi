use baozi_operation::{
    ByteBlock, BytesPlan, CanonicalRecordRowWriter, RecordBlock, RecordPlan, StoreBuildError,
    StoreControlledError, StorePlanError, StoreRuntimeError, Utf8Block, Utf8Plan,
};

use crate::SCENE_VALIDATION_POLL_INTERVAL;
use crate::canonical_scene::store::production::{Presence, ValidatedAppearanceBatch};
use crate::canonical_scene::store::schema::appearance::{
    APPEARANCE_BINDING_FIELD_COUNT, APPEARANCE_DIRECTORY_FIELD_COUNT, APPEARANCE_IMAGE_FIELD_COUNT,
    APPEARANCE_IMAGE_PAYLOAD_ENCODED, APPEARANCE_MATERIAL_FIELD_COUNT, APPEARANCE_MEDIA_TYPE_UTF8,
    APPEARANCE_PRESENCE_ABSENT, APPEARANCE_PRESENCE_PRESENT, APPEARANCE_SAMPLER_FIELD_COUNT,
    APPEARANCE_SCHEMA_REVISION, APPEARANCE_TEXTURE_FIELD_COUNT, AppearanceBindingTableSchema,
    AppearanceDirectorySchema, AppearanceEncodedImagePayloadSchema, AppearanceImageTableSchema,
    AppearanceMaterialTableSchema, AppearanceMediaTypeSchema, AppearanceSamplerTableSchema,
    AppearanceTextureTableSchema, encode_appearance_address_mode, encode_appearance_mipmap_filter,
    encode_appearance_texel_filter,
};
use crate::canonical_scene::store::schema::{CanonicalBlockRef, CanonicalSchema};

use super::super::{SceneStoreAssembly, SceneStoreKeys};
use super::NodeSceneWorkMeter;

pub(super) struct AppearanceMaterializationPlans {
    image_payloads: Option<BytesPlan>,
    media_types: Option<Utf8Plan>,
    images: Option<RecordPlan>,
    samplers: Option<RecordPlan>,
    textures: Option<RecordPlan>,
    bindings: Option<RecordPlan>,
    materials: Option<RecordPlan>,
    directory: RecordPlan,
    image_count: u32,
    sampler_count: u32,
    texture_count: u32,
    material_count: u32,
    binding_count: u32,
    image_payload_byte_count: u32,
    media_type_byte_count: u32,
    table_block_count: usize,
    payload_block_count: u32,
    canonical_block_count: usize,
}

impl AppearanceMaterializationPlans {
    pub(super) fn new<R>(batch: &ValidatedAppearanceBatch<'_, R>) -> Result<Self, StorePlanError> {
        let image_count = batch.image_count();
        let sampler_count = batch.sampler_count();
        let texture_count = batch.texture_count();
        let material_count = batch.material_count();
        let binding_count = batch.binding_count();
        let image_payload_byte_count = batch.image_payload_byte_count();
        let media_type_byte_count = batch.media_type_byte_count();

        let image_payloads = bytes_plan_if_nonempty(image_payload_byte_count)?;
        let media_types = utf8_plan_if_nonempty(media_type_byte_count)?;
        let images = record_plan_if_nonempty(image_count, APPEARANCE_IMAGE_FIELD_COUNT)?;
        let samplers = record_plan_if_nonempty(sampler_count, APPEARANCE_SAMPLER_FIELD_COUNT)?;
        let textures = record_plan_if_nonempty(texture_count, APPEARANCE_TEXTURE_FIELD_COUNT)?;
        let bindings = record_plan_if_nonempty(binding_count, APPEARANCE_BINDING_FIELD_COUNT)?;
        let materials = record_plan_if_nonempty(material_count, APPEARANCE_MATERIAL_FIELD_COUNT)?;

        let payload_block_count = u32::from(image_payloads.is_some())
            .checked_add(u32::from(media_types.is_some()))
            .ok_or(StorePlanError::SizeOverflow)?;
        if payload_block_count != batch.payload_block_count()
            || (image_count == 0) != image_payloads.is_none()
            || (media_type_byte_count != 0 && image_count == 0)
        {
            return Err(StorePlanError::InvalidBlockTable);
        }

        let table_block_count = [
            images.is_some(),
            samplers.is_some(),
            textures.is_some(),
            bindings.is_some(),
            materials.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        let canonical_block_count = 1_usize
            .checked_add(table_block_count)
            .and_then(|count| {
                usize::try_from(payload_block_count)
                    .ok()
                    .and_then(|payloads| count.checked_add(payloads))
            })
            .ok_or(StorePlanError::SizeOverflow)?;

        Ok(Self {
            image_payloads,
            media_types,
            images,
            samplers,
            textures,
            bindings,
            materials,
            directory: RecordPlan::new(1, APPEARANCE_DIRECTORY_FIELD_COUNT)?,
            image_count,
            sampler_count,
            texture_count,
            material_count,
            binding_count,
            image_payload_byte_count,
            media_type_byte_count,
            table_block_count,
            payload_block_count,
            canonical_block_count,
        })
    }

    pub(super) const fn canonical_block_count(&self) -> usize {
        self.canonical_block_count
    }

    pub(super) const fn image_count(&self) -> u32 {
        self.image_count
    }

    pub(super) const fn sampler_count(&self) -> u32 {
        self.sampler_count
    }

    pub(super) const fn texture_count(&self) -> u32 {
        self.texture_count
    }

    pub(super) const fn material_count(&self) -> u32 {
        self.material_count
    }

    pub(super) const fn binding_count(&self) -> u32 {
        self.binding_count
    }

    pub(super) const fn image_payload_byte_count(&self) -> u32 {
        self.image_payload_byte_count
    }

    pub(super) const fn media_type_byte_count(&self) -> u32 {
        self.media_type_byte_count
    }

    pub(super) const fn table_block_count(&self) -> usize {
        self.table_block_count
    }

    pub(super) const fn payload_block_count(&self) -> u32 {
        self.payload_block_count
    }
}

#[derive(Clone, Copy)]
pub(super) struct RetainedAppearanceRefs {
    directory: CanonicalBlockRef<AppearanceDirectorySchema>,
    materials: Option<CanonicalBlockRef<AppearanceMaterialTableSchema>>,
}

impl RetainedAppearanceRefs {
    pub(super) const fn directory(self) -> CanonicalBlockRef<AppearanceDirectorySchema> {
        self.directory
    }

    pub(super) const fn materials(
        self,
    ) -> Option<CanonicalBlockRef<AppearanceMaterialTableSchema>> {
        self.materials
    }
}

#[derive(Clone, Copy)]
struct AppearanceTableRefs {
    images: Option<CanonicalBlockRef<AppearanceImageTableSchema>>,
    samplers: Option<CanonicalBlockRef<AppearanceSamplerTableSchema>>,
    textures: Option<CanonicalBlockRef<AppearanceTextureTableSchema>>,
    bindings: Option<CanonicalBlockRef<AppearanceBindingTableSchema>>,
    materials: Option<CanonicalBlockRef<AppearanceMaterialTableSchema>>,
}

#[derive(Clone, Copy)]
struct AppearancePayloadRefs {
    image_payloads: Option<CanonicalBlockRef<AppearanceEncodedImagePayloadSchema>>,
    media_types: Option<CanonicalBlockRef<AppearanceMediaTypeSchema>>,
}

pub(super) fn retain_appearance<R>(
    assembly: &mut SceneStoreAssembly,
    plans: &AppearanceMaterializationPlans,
    batch: &ValidatedAppearanceBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<RetainedAppearanceRefs, StoreRuntimeError> {
    let image_payloads = match plans.image_payloads {
        Some(plan) => Some(retain_image_payloads(
            assembly, plan, batch, keys, work, failure,
        )?),
        None => None,
    };
    let media_types = match plans.media_types {
        Some(plan) => Some(retain_media_types(
            assembly, plan, batch, keys, work, failure,
        )?),
        None => None,
    };
    let payloads = AppearancePayloadRefs {
        image_payloads,
        media_types,
    };

    let images = match plans.images {
        Some(plan) => Some(retain_image_table(
            assembly, plan, batch, payloads, keys, work, failure,
        )?),
        None => None,
    };
    let samplers = match plans.samplers {
        Some(plan) => Some(retain_sampler_table(
            assembly, plan, batch, keys, work, failure,
        )?),
        None => None,
    };
    let textures = match plans.textures {
        Some(plan) => Some(retain_texture_table(
            assembly, plan, batch, images, samplers, keys, work, failure,
        )?),
        None => None,
    };
    let bindings = match plans.bindings {
        Some(plan) => Some(retain_binding_table(
            assembly, plan, batch, textures, keys, work, failure,
        )?),
        None => None,
    };
    let materials = match plans.materials {
        Some(plan) => Some(retain_material_table(
            assembly, plan, batch, bindings, keys, work, failure,
        )?),
        None => None,
    };
    let tables = AppearanceTableRefs {
        images,
        samplers,
        textures,
        bindings,
        materials,
    };
    let directory =
        retain_appearance_directory(assembly, plans, payloads, tables, keys, work, failure)?;
    Ok(RetainedAppearanceRefs {
        directory,
        materials,
    })
}

fn retain_image_payloads<R>(
    assembly: &mut SceneStoreAssembly,
    plan: BytesPlan,
    batch: &ValidatedAppearanceBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AppearanceEncodedImagePayloadSchema>, StoreRuntimeError> {
    let chunk_size = usize::try_from(SCENE_VALIDATION_POLL_INTERVAL)
        .map_err(|_| StoreRuntimeError::AccountingOverflow)?;
    assembly
        .try_retain_canonical_bytes_controlled(
            plan,
            keys.dimension,
            AppearanceEncodedImagePayloadSchema,
            |writer, control| {
                for image in batch.images() {
                    for chunk in image.payload().chunks(chunk_size) {
                        work.observe(control, u64::try_from(chunk.len()).map_err(|_| failure)?)?;
                        writer.write_bytes(chunk)?;
                    }
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_media_types<R>(
    assembly: &mut SceneStoreAssembly,
    plan: Utf8Plan,
    batch: &ValidatedAppearanceBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AppearanceMediaTypeSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_utf8_controlled(
            plan,
            keys.dimension,
            AppearanceMediaTypeSchema,
            |writer, control| {
                for image in batch.images() {
                    if let Presence::Present(media_type) = image.media_type() {
                        let media_type = media_type.as_str();
                        work.observe(
                            control,
                            u64::try_from(media_type.len()).map_err(|_| failure)?,
                        )?;
                        writer.write_str(media_type)?;
                    }
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_image_table<R>(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    batch: &ValidatedAppearanceBatch<'_, R>,
    payloads: AppearancePayloadRefs,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AppearanceImageTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            AppearanceImageTableSchema,
            |writer, control| {
                let image_payloads = payloads.image_payloads.ok_or(failure)?;
                for image in batch.images() {
                    work.observe(control, 1)?;
                    let image_id = u32::try_from(image.input_index()).map_err(|_| failure)?;
                    let payload_count =
                        u32::try_from(image.payload().len()).map_err(|_| failure)?;
                    if payload_count != image.payload_count() || payload_count == 0 {
                        return Err(failure.into());
                    }
                    let (media_presence, media_types) =
                        match (image.media_type_present(), image.media_type()) {
                            (false, Presence::Absent) if image.media_type_count() == 0 => {
                                (APPEARANCE_PRESENCE_ABSENT, None)
                            }
                            (true, Presence::Present(media_type))
                                if u32::try_from(media_type.as_str().len())
                                    .map_err(|_| failure)?
                                    == image.media_type_count() =>
                            {
                                (
                                    APPEARANCE_PRESENCE_PRESENT,
                                    Some(payloads.media_types.ok_or(failure)?),
                                )
                            }
                            _ => return Err(failure.into()),
                        };
                    writer.write_row(|row| {
                        row.write_scalar(image_id)?;
                        row.write_scalar(APPEARANCE_IMAGE_PAYLOAD_ENCODED)?;
                        row.write_scalar(image.payload_offset())?;
                        row.write_scalar(image.payload_count())?;
                        row.write_bytes_ref(image_payloads.into_store())?;
                        row.write_scalar(media_presence)?;
                        row.write_scalar(APPEARANCE_MEDIA_TYPE_UTF8)?;
                        row.write_scalar(image.media_type_offset())?;
                        row.write_scalar(image.media_type_count())?;
                        match media_types {
                            Some(reference) => row.write_utf8_ref(reference.into_store()),
                            None => row.write_scalar(0_u8),
                        }
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_sampler_table<R>(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    batch: &ValidatedAppearanceBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AppearanceSamplerTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            AppearanceSamplerTableSchema,
            |writer, control| {
                for (sampler_index, sampler) in batch.samplers().enumerate() {
                    work.observe(control, 1)?;
                    let sampler_id = u32::try_from(sampler_index).map_err(|_| failure)?;
                    let state = sampler.state();
                    writer.write_row(|row| {
                        row.write_scalar(sampler_id)?;
                        row.write_scalar(encode_appearance_texel_filter(state.minification()))?;
                        row.write_scalar(encode_appearance_texel_filter(state.magnification()))?;
                        row.write_scalar(encode_appearance_mipmap_filter(state.mipmap()))?;
                        row.write_scalar(encode_appearance_address_mode(state.address_u()))?;
                        row.write_scalar(encode_appearance_address_mode(state.address_v()))?;
                        row.write_scalar(encode_appearance_address_mode(state.address_w()))
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_texture_table<R>(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    batch: &ValidatedAppearanceBatch<'_, R>,
    images: Option<CanonicalBlockRef<AppearanceImageTableSchema>>,
    samplers: Option<CanonicalBlockRef<AppearanceSamplerTableSchema>>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AppearanceTextureTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            AppearanceTextureTableSchema,
            |writer, control| {
                let images = images.ok_or(failure)?;
                let samplers = samplers.ok_or(failure)?;
                for (texture_index, texture) in batch.textures().enumerate() {
                    work.observe(control, 1)?;
                    let texture_id = u32::try_from(texture_index).map_err(|_| failure)?;
                    writer.write_row(|row| {
                        row.write_scalar(texture_id)?;
                        row.write_scalar(texture.image().index())?;
                        row.write_record_ref(images.into_store())?;
                        row.write_scalar(texture.sampler().index())?;
                        row.write_record_ref(samplers.into_store())
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_binding_table<R>(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    batch: &ValidatedAppearanceBatch<'_, R>,
    textures: Option<CanonicalBlockRef<AppearanceTextureTableSchema>>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AppearanceBindingTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            AppearanceBindingTableSchema,
            |writer, control| {
                let textures = textures.ok_or(failure)?;
                for binding in batch.bindings() {
                    work.observe(control, 1)?;
                    let binding_id = u32::try_from(binding.input_index()).map_err(|_| failure)?;
                    writer.write_row(|row| {
                        row.write_scalar(binding_id)?;
                        row.write_scalar(binding.texture().index())?;
                        row.write_record_ref(textures.into_store())?;
                        row.write_scalar(binding.role_tag())
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_material_table<R>(
    assembly: &mut SceneStoreAssembly,
    plan: RecordPlan,
    batch: &ValidatedAppearanceBatch<'_, R>,
    bindings: Option<CanonicalBlockRef<AppearanceBindingTableSchema>>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AppearanceMaterialTableSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records_controlled(
            plan,
            keys.dimension,
            AppearanceMaterialTableSchema,
            |writer, control| {
                for material in batch.materials() {
                    work.observe(control, 1)?;
                    let material_id = u32::try_from(material.input_index()).map_err(|_| failure)?;
                    let range = material.authored().bindings();
                    writer.write_row(|row| {
                        row.write_scalar(material_id)?;
                        row.write_scalar(range.offset())?;
                        row.write_scalar(range.count())?;
                        if range.count() == 0 {
                            row.write_scalar(0_u8)
                        } else {
                            row.write_record_ref(bindings.ok_or(failure)?.into_store())
                        }
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_appearance_directory(
    assembly: &mut SceneStoreAssembly,
    plans: &AppearanceMaterializationPlans,
    payloads: AppearancePayloadRefs,
    tables: AppearanceTableRefs,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AppearanceDirectorySchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_records_controlled(
            plans.directory,
            keys.dimension,
            AppearanceDirectorySchema,
            |writer, control| {
                work.observe(control, 1)?;
                writer
                    .write_row(|row| {
                        row.write_scalar(APPEARANCE_SCHEMA_REVISION)?;
                        write_optional_record_ref(row, plans.image_count, tables.images, failure)?;
                        write_optional_record_ref(
                            row,
                            plans.sampler_count,
                            tables.samplers,
                            failure,
                        )?;
                        write_optional_record_ref(
                            row,
                            plans.texture_count,
                            tables.textures,
                            failure,
                        )?;
                        write_optional_record_ref(
                            row,
                            plans.material_count,
                            tables.materials,
                            failure,
                        )?;
                        write_optional_record_ref(
                            row,
                            plans.binding_count,
                            tables.bindings,
                            failure,
                        )?;
                        write_optional_bytes_ref(
                            row,
                            plans.image_payload_byte_count,
                            payloads.image_payloads,
                            failure,
                        )?;
                        write_optional_utf8_ref(
                            row,
                            plans.media_type_byte_count,
                            payloads.media_types,
                            failure,
                        )
                    })
                    .map_err(StoreControlledError::from)
            },
        )
        .map(CanonicalBlockRef::from_store)
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

fn bytes_plan_if_nonempty(count: u32) -> Result<Option<BytesPlan>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(BytesPlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
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
    if count == 0 {
        if reference.is_some() {
            return Err(failure);
        }
        row.write_scalar(0_u8)
    } else {
        row.write_record_ref(reference.ok_or(failure)?.into_store())
    }
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
    if count == 0 {
        if reference.is_some() {
            return Err(failure);
        }
        row.write_scalar(0_u8)
    } else {
        row.write_bytes_ref(reference.ok_or(failure)?.into_store())
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
    if count == 0 {
        if reference.is_some() {
            return Err(failure);
        }
        row.write_scalar(0_u8)
    } else {
        row.write_utf8_ref(reference.ok_or(failure)?.into_store())
    }
}
