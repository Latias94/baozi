use baozi_operation::{
    ByteBlock, BytesPlan, CanonicalRecordRowWriter, RecordBlock, RecordPlan, ScalarSequence,
    SequencePlan, StoreBuildError, StoreControlledError, StorePlanError, StoreRuntimeError,
    Utf8Block, Utf8Plan,
};

use crate::SCENE_VALIDATION_POLL_INTERVAL;
use crate::canonical_scene::store::production::{Presence, ValidatedAppearanceBatch};
use crate::canonical_scene::store::schema::appearance::{
    APPEARANCE_BINDING_FIELD_COUNT, APPEARANCE_BINDING_SEMANTIC_STRIDE,
    APPEARANCE_DIRECTORY_FIELD_COUNT, APPEARANCE_IMAGE_FIELD_COUNT,
    APPEARANCE_IMAGE_PAYLOAD_ENCODED, APPEARANCE_MATERIAL_FACTOR_STRIDE,
    APPEARANCE_MATERIAL_FIELD_COUNT, APPEARANCE_MEDIA_TYPE_UTF8, APPEARANCE_PRESENCE_ABSENT,
    APPEARANCE_PRESENCE_PRESENT, APPEARANCE_SAMPLER_FIELD_COUNT, APPEARANCE_SCHEMA_REVISION,
    APPEARANCE_TEXTURE_FIELD_COUNT, AppearanceBindingSemanticScalarSchema,
    AppearanceBindingTableSchema, AppearanceDirectorySchema, AppearanceEncodedImagePayloadSchema,
    AppearanceImageTableSchema, AppearanceMaterialFactorScalarSchema,
    AppearanceMaterialTableSchema, AppearanceMediaTypeSchema, AppearanceSamplerTableSchema,
    AppearanceSourceKeySchema, AppearanceTextureTableSchema,
    appearance_material_factor_scalar as factor_scalar, encode_appearance_address_mode,
    encode_appearance_mipmap_filter, encode_appearance_texel_filter,
};
use crate::canonical_scene::store::schema::{CanonicalBlockRef, CanonicalSchema};

use super::super::{SceneStoreAssembly, SceneStoreKeys};
use super::NodeSceneWorkMeter;

pub(super) struct AppearanceMaterializationPlans {
    image_payloads: Option<BytesPlan>,
    media_types: Option<Utf8Plan>,
    source_keys: Option<Utf8Plan>,
    binding_semantic_scalars: Option<SequencePlan<f32>>,
    material_factor_scalars: Option<SequencePlan<f32>>,
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
    source_key_byte_count: u32,
    binding_semantic_scalar_count: u32,
    material_factor_scalar_count: u32,
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
        let source_key_byte_count = batch.source_key_byte_count();
        let binding_semantic_scalar_count = batch.binding_semantic_scalar_count();
        let material_factor_scalar_count = batch.material_factor_scalar_count();

        let image_payloads = bytes_plan_if_nonempty(image_payload_byte_count)?;
        let media_types = utf8_plan_if_nonempty(media_type_byte_count)?;
        let source_keys = utf8_plan_if_nonempty(source_key_byte_count)?;
        let binding_semantic_scalars = sequence_plan_if_nonempty(binding_semantic_scalar_count)?;
        let material_factor_scalars = sequence_plan_if_nonempty(material_factor_scalar_count)?;
        let images = record_plan_if_nonempty(image_count, APPEARANCE_IMAGE_FIELD_COUNT)?;
        let samplers = record_plan_if_nonempty(sampler_count, APPEARANCE_SAMPLER_FIELD_COUNT)?;
        let textures = record_plan_if_nonempty(texture_count, APPEARANCE_TEXTURE_FIELD_COUNT)?;
        let bindings = record_plan_if_nonempty(binding_count, APPEARANCE_BINDING_FIELD_COUNT)?;
        let materials = record_plan_if_nonempty(material_count, APPEARANCE_MATERIAL_FIELD_COUNT)?;
        let expected_binding_semantic_scalars = binding_count
            .checked_mul(
                u32::try_from(APPEARANCE_BINDING_SEMANTIC_STRIDE)
                    .map_err(|_| StorePlanError::SizeOverflow)?,
            )
            .ok_or(StorePlanError::SizeOverflow)?;
        let expected_material_factor_scalars = material_count
            .checked_mul(
                u32::try_from(APPEARANCE_MATERIAL_FACTOR_STRIDE)
                    .map_err(|_| StorePlanError::SizeOverflow)?,
            )
            .ok_or(StorePlanError::SizeOverflow)?;

        let payload_block_count = u32::from(image_payloads.is_some())
            .checked_add(u32::from(media_types.is_some()))
            .and_then(|count| count.checked_add(u32::from(source_keys.is_some())))
            .and_then(|count| count.checked_add(u32::from(binding_semantic_scalars.is_some())))
            .and_then(|count| count.checked_add(u32::from(material_factor_scalars.is_some())))
            .ok_or(StorePlanError::SizeOverflow)?;
        if payload_block_count != batch.payload_block_count()
            || (image_count == 0) != image_payloads.is_none()
            || (media_type_byte_count != 0 && image_count == 0)
            || (source_key_byte_count != 0 && binding_count == 0)
            || binding_semantic_scalar_count != expected_binding_semantic_scalars
            || material_factor_scalar_count != expected_material_factor_scalars
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
            source_keys,
            binding_semantic_scalars,
            material_factor_scalars,
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
            source_key_byte_count,
            binding_semantic_scalar_count,
            material_factor_scalar_count,
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

    pub(super) const fn source_key_byte_count(&self) -> u32 {
        self.source_key_byte_count
    }

    pub(super) const fn binding_semantic_scalar_count(&self) -> u32 {
        self.binding_semantic_scalar_count
    }

    pub(super) const fn material_factor_scalar_count(&self) -> u32 {
        self.material_factor_scalar_count
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
    source_keys: Option<CanonicalBlockRef<AppearanceSourceKeySchema>>,
    binding_semantic_scalars: Option<CanonicalBlockRef<AppearanceBindingSemanticScalarSchema>>,
    material_factor_scalars: Option<CanonicalBlockRef<AppearanceMaterialFactorScalarSchema>>,
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
    let source_keys = match plans.source_keys {
        Some(plan) => Some(retain_source_keys(
            assembly, plan, batch, keys, work, failure,
        )?),
        None => None,
    };
    let binding_semantic_scalars = match plans.binding_semantic_scalars {
        Some(plan) => Some(retain_binding_semantic_scalars(
            assembly, plan, batch, keys, work, failure,
        )?),
        None => None,
    };
    let material_factor_scalars = match plans.material_factor_scalars {
        Some(plan) => Some(retain_material_factor_scalars(
            assembly, plan, batch, keys, work, failure,
        )?),
        None => None,
    };
    let payloads = AppearancePayloadRefs {
        image_payloads,
        media_types,
        source_keys,
        binding_semantic_scalars,
        material_factor_scalars,
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
            assembly, plan, batch, textures, payloads, keys, work, failure,
        )?),
        None => None,
    };
    let materials = match plans.materials {
        Some(plan) => Some(retain_material_table(
            assembly, plan, batch, bindings, payloads, keys, work, failure,
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

fn retain_source_keys<R>(
    assembly: &mut SceneStoreAssembly,
    plan: Utf8Plan,
    batch: &ValidatedAppearanceBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AppearanceSourceKeySchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_utf8_controlled(
            plan,
            keys.dimension,
            AppearanceSourceKeySchema,
            |writer, control| {
                let mut expected_offset = 0_u32;
                for binding in batch.bindings() {
                    if binding.source_key_offset() != expected_offset {
                        return Err(failure.into());
                    }
                    if let Some(source_key) = binding.semantics().source_key() {
                        let value = source_key.as_str();
                        let count = u32::try_from(value.len()).map_err(|_| failure)?;
                        if count == 0 || binding.source_key_count() != count {
                            return Err(failure.into());
                        }
                        work.observe(control, u64::from(count))?;
                        writer.write_str(value)?;
                        expected_offset = expected_offset.checked_add(count).ok_or(failure)?;
                    } else if binding.source_key_count() != 0 {
                        return Err(failure.into());
                    }
                }
                if expected_offset != batch.source_key_byte_count() {
                    return Err(failure.into());
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_binding_semantic_scalars<R>(
    assembly: &mut SceneStoreAssembly,
    plan: SequencePlan<f32>,
    batch: &ValidatedAppearanceBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AppearanceBindingSemanticScalarSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            AppearanceBindingSemanticScalarSchema,
            |writer, control| {
                let stride =
                    u32::try_from(APPEARANCE_BINDING_SEMANTIC_STRIDE).map_err(|_| failure)?;
                let mut expected_offset = 0_u32;
                for binding in batch.bindings() {
                    if binding.semantic_scalar_offset() != expected_offset {
                        return Err(failure.into());
                    }
                    let semantics = binding.semantics();
                    let transform = semantics.transform();
                    let offset = transform.offset();
                    let scale = transform.scale();
                    let values = [
                        offset.x,
                        offset.y,
                        transform.rotation_radians(),
                        scale.x,
                        scale.y,
                        semantics.strength().get(),
                    ];
                    work.observe(control, u64::from(stride))?;
                    writer.extend_from_slice(&values)?;
                    expected_offset = expected_offset.checked_add(stride).ok_or(failure)?;
                }
                if expected_offset != batch.binding_semantic_scalar_count() {
                    return Err(failure.into());
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_material_factor_scalars<R>(
    assembly: &mut SceneStoreAssembly,
    plan: SequencePlan<f32>,
    batch: &ValidatedAppearanceBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AppearanceMaterialFactorScalarSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            AppearanceMaterialFactorScalarSchema,
            |writer, control| {
                let stride =
                    u32::try_from(APPEARANCE_MATERIAL_FACTOR_STRIDE).map_err(|_| failure)?;
                let mut expected_offset = 0_u32;
                for material in batch.materials() {
                    if material.factor_offset() != expected_offset {
                        return Err(failure.into());
                    }
                    let factors = material.semantics().factors();
                    let mut values = [0.0_f32; APPEARANCE_MATERIAL_FACTOR_STRIDE];
                    let emissive = factors.emissive();
                    values[factor_scalar::EMISSIVE_X] = emissive.x;
                    values[factor_scalar::EMISSIVE_Y] = emissive.y;
                    values[factor_scalar::EMISSIVE_Z] = emissive.z;
                    if let Some(pbr) = factors.pbr_metallic_roughness() {
                        let base = pbr.base_color();
                        values[factor_scalar::PBR_BASE_COLOR_R] = base.r;
                        values[factor_scalar::PBR_BASE_COLOR_G] = base.g;
                        values[factor_scalar::PBR_BASE_COLOR_B] = base.b;
                        values[factor_scalar::PBR_BASE_COLOR_A] = base.a;
                        values[factor_scalar::METALLIC] = pbr.metallic();
                        values[factor_scalar::ROUGHNESS] = pbr.roughness();
                    }
                    if let Some(pbr) = factors.pbr_specular_glossiness() {
                        let diffuse = pbr.diffuse();
                        values[factor_scalar::PBR_DIFFUSE_R] = diffuse.r;
                        values[factor_scalar::PBR_DIFFUSE_G] = diffuse.g;
                        values[factor_scalar::PBR_DIFFUSE_B] = diffuse.b;
                        values[factor_scalar::PBR_DIFFUSE_A] = diffuse.a;
                        let specular = pbr.specular();
                        values[factor_scalar::PBR_SPECULAR_X] = specular.x;
                        values[factor_scalar::PBR_SPECULAR_Y] = specular.y;
                        values[factor_scalar::PBR_SPECULAR_Z] = specular.z;
                        values[factor_scalar::GLOSSINESS] = pbr.glossiness();
                    }
                    if let Some(legacy) = factors.legacy() {
                        let ambient = legacy.ambient();
                        values[factor_scalar::LEGACY_AMBIENT_X] = ambient.x;
                        values[factor_scalar::LEGACY_AMBIENT_Y] = ambient.y;
                        values[factor_scalar::LEGACY_AMBIENT_Z] = ambient.z;
                        let diffuse = legacy.diffuse();
                        values[factor_scalar::LEGACY_DIFFUSE_X] = diffuse.x;
                        values[factor_scalar::LEGACY_DIFFUSE_Y] = diffuse.y;
                        values[factor_scalar::LEGACY_DIFFUSE_Z] = diffuse.z;
                        let specular = legacy.specular();
                        values[factor_scalar::LEGACY_SPECULAR_X] = specular.x;
                        values[factor_scalar::LEGACY_SPECULAR_Y] = specular.y;
                        values[factor_scalar::LEGACY_SPECULAR_Z] = specular.z;
                        let legacy_emissive = legacy.emissive();
                        values[factor_scalar::LEGACY_EMISSIVE_X] = legacy_emissive.x;
                        values[factor_scalar::LEGACY_EMISSIVE_Y] = legacy_emissive.y;
                        values[factor_scalar::LEGACY_EMISSIVE_Z] = legacy_emissive.z;
                        values[factor_scalar::LEGACY_SHININESS] = legacy.shininess();
                        values[factor_scalar::LEGACY_OPACITY] = legacy.opacity();
                    }
                    let supplemental = factors.supplemental_pbr();
                    values[factor_scalar::CLEARCOAT] = supplemental.clearcoat();
                    values[factor_scalar::CLEARCOAT_ROUGHNESS] = supplemental.clearcoat_roughness();
                    values[factor_scalar::TRANSMISSION] = supplemental.transmission();
                    values[factor_scalar::THICKNESS] = supplemental.thickness();
                    let sheen = supplemental.sheen_color();
                    values[factor_scalar::SHEEN_COLOR_X] = sheen.x;
                    values[factor_scalar::SHEEN_COLOR_Y] = sheen.y;
                    values[factor_scalar::SHEEN_COLOR_Z] = sheen.z;
                    values[factor_scalar::SHEEN_ROUGHNESS] = supplemental.sheen_roughness();
                    work.observe(control, u64::from(stride))?;
                    writer.extend_from_slice(&values)?;
                    expected_offset = expected_offset.checked_add(stride).ok_or(failure)?;
                }
                if expected_offset != batch.material_factor_scalar_count() {
                    return Err(failure.into());
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
    payloads: AppearancePayloadRefs,
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
                let semantic_scalars = payloads.binding_semantic_scalars.ok_or(failure)?;
                let semantic_scalar_count =
                    u32::try_from(APPEARANCE_BINDING_SEMANTIC_STRIDE).map_err(|_| failure)?;
                for binding in batch.bindings() {
                    work.observe(control, 1)?;
                    let binding_id = u32::try_from(binding.input_index()).map_err(|_| failure)?;
                    writer.write_row(|row| {
                        row.write_scalar(binding_id)?;
                        row.write_scalar(binding.texture().index())?;
                        row.write_record_ref(textures.into_store())?;
                        row.write_scalar(binding.role_tag())?;
                        row.write_scalar(binding.coordinate_source_tag())?;
                        row.write_scalar(binding.coordinate_set())?;
                        row.write_scalar(binding.channel_lane_count())?;
                        for channel in binding.channel_tags() {
                            row.write_scalar(channel)?;
                        }
                        row.write_scalar(binding.color_interpretation_tag())?;
                        row.write_scalar(binding.semantic_scalar_offset())?;
                        row.write_scalar(semantic_scalar_count)?;
                        row.write_sequence_ref(semantic_scalars.into_store())?;
                        row.write_scalar(binding.blend_operation_tag())?;
                        row.write_scalar(binding.source_key_offset())?;
                        let source_keys = if binding.source_key_count() == 0 {
                            None
                        } else {
                            payloads.source_keys
                        };
                        write_optional_utf8_ref(
                            row,
                            binding.source_key_count(),
                            source_keys,
                            failure,
                        )
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
    payloads: AppearancePayloadRefs,
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
                let factor_scalars = payloads.material_factor_scalars.ok_or(failure)?;
                let factor_count =
                    u32::try_from(APPEARANCE_MATERIAL_FACTOR_STRIDE).map_err(|_| failure)?;
                for material in batch.materials() {
                    work.observe(control, 1)?;
                    let material_id = u32::try_from(material.input_index()).map_err(|_| failure)?;
                    let range = material.authored().bindings();
                    writer.write_row(|row| {
                        row.write_scalar(material_id)?;
                        row.write_scalar(material.shading_model_tag())?;
                        row.write_scalar(material.alpha_mode_tag())?;
                        row.write_scalar(material.alpha_cutoff())?;
                        row.write_scalar(material.double_sided_tag())?;
                        row.write_scalar(material.pbr_metallic_roughness_presence())?;
                        row.write_scalar(material.pbr_specular_glossiness_presence())?;
                        row.write_scalar(material.legacy_presence())?;
                        row.write_scalar(range.offset())?;
                        row.write_scalar(range.count())?;
                        if range.count() == 0 {
                            row.write_scalar(0_u8)?;
                        } else {
                            row.write_record_ref(bindings.ok_or(failure)?.into_store())?;
                        }
                        row.write_scalar(material.factor_offset())?;
                        row.write_scalar(factor_count)?;
                        row.write_sequence_ref(factor_scalars.into_store())
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
                        )?;
                        write_optional_utf8_ref(
                            row,
                            plans.source_key_byte_count,
                            payloads.source_keys,
                            failure,
                        )?;
                        write_optional_sequence_ref(
                            row,
                            plans.binding_semantic_scalar_count,
                            payloads.binding_semantic_scalars,
                            failure,
                        )?;
                        write_optional_sequence_ref(
                            row,
                            plans.material_factor_scalar_count,
                            payloads.material_factor_scalars,
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

fn sequence_plan_if_nonempty(count: u32) -> Result<Option<SequencePlan<f32>>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(SequencePlan::new(
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

fn write_optional_sequence_ref<Tag>(
    row: &mut CanonicalRecordRowWriter<'_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<f32>>,
{
    row.write_scalar(count)?;
    if count == 0 {
        if reference.is_some() {
            return Err(failure);
        }
        row.write_scalar(0_u8)
    } else {
        row.write_sequence_ref(reference.ok_or(failure)?.into_store())
    }
}
