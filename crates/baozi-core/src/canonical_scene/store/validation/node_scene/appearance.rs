use baozi_operation::{
    CanonicalBytesRef, CanonicalRecordRef, CanonicalRecordView, CanonicalSequenceRef,
    CanonicalUtf8Ref, StoreValidationSession,
};

use crate::appearance::{
    MaterialFactors, MaterialSemanticsRef, TextureBindingSemanticsRef, TextureChannel,
    TextureChannelMapping, TextureChannelSwizzle, TextureRoleKind, TextureSourceKey,
    validate_material_semantics, validate_texture_binding_semantics,
};
use crate::canonical_scene::store::production::{Presence, ValidatedAppearanceBatch};
use crate::canonical_scene::store::schema::CanonicalBlockRef;
use crate::canonical_scene::store::schema::appearance::{
    APPEARANCE_ADDRESS_CLAMP_TO_BORDER, APPEARANCE_BINDING_FIELD_COUNT,
    APPEARANCE_BINDING_SEMANTIC_STRIDE, APPEARANCE_CHANNEL_ZERO, APPEARANCE_DIRECTORY_FIELD_COUNT,
    APPEARANCE_IMAGE_FIELD_COUNT, APPEARANCE_IMAGE_PAYLOAD_ENCODED,
    APPEARANCE_MATERIAL_FACTOR_STRIDE, APPEARANCE_MATERIAL_FIELD_COUNT, APPEARANCE_MEDIA_TYPE_UTF8,
    APPEARANCE_MIPMAP_FILTER_LINEAR, APPEARANCE_PRESENCE_ABSENT, APPEARANCE_PRESENCE_PRESENT,
    APPEARANCE_ROLE_METALLIC, APPEARANCE_ROLE_METALLIC_ROUGHNESS, APPEARANCE_ROLE_ROUGHNESS,
    APPEARANCE_SAMPLER_FIELD_COUNT, APPEARANCE_SCHEMA_REVISION, APPEARANCE_TEXEL_FILTER_LINEAR,
    APPEARANCE_TEXTURE_FIELD_COUNT, AppearanceBindingSemanticScalarSchema,
    AppearanceBindingTableSchema, AppearanceDirectorySchema, AppearanceEncodedImagePayloadSchema,
    AppearanceImageTableSchema, AppearanceMaterialFactorScalarSchema,
    AppearanceMaterialTableSchema, AppearanceMediaTypeSchema, AppearanceSamplerTableSchema,
    AppearanceSourceKeySchema, AppearanceTextureTableSchema, appearance_binding_field,
    appearance_directory_field, appearance_image_field, appearance_material_factor_scalar,
    appearance_material_field, appearance_sampler_field, appearance_texture_field,
    decode_appearance_alpha_mode, decode_appearance_binding_numeric_semantics,
    decode_appearance_blend_operation, decode_appearance_channel,
    decode_appearance_color_interpretation, decode_appearance_coordinate_source,
    decode_appearance_material_factors, decode_appearance_shading_model,
    decode_appearance_texture_role, encode_appearance_address_mode,
    encode_appearance_mipmap_filter, encode_appearance_texel_filter,
};
use crate::extension::ExtensionMediaType;

use super::super::require_scalar;

#[derive(Clone, Copy)]
pub(super) struct AppearanceValidatedRefs {
    pub(super) images: Option<CanonicalBlockRef<AppearanceImageTableSchema>>,
    pub(super) samplers: Option<CanonicalBlockRef<AppearanceSamplerTableSchema>>,
    pub(super) textures: Option<CanonicalBlockRef<AppearanceTextureTableSchema>>,
    pub(super) materials: Option<CanonicalBlockRef<AppearanceMaterialTableSchema>>,
    pub(super) bindings: Option<CanonicalBlockRef<AppearanceBindingTableSchema>>,
    pub(super) image_payloads: Option<CanonicalBlockRef<AppearanceEncodedImagePayloadSchema>>,
    pub(super) media_types: Option<CanonicalBlockRef<AppearanceMediaTypeSchema>>,
    pub(super) source_keys: Option<CanonicalBlockRef<AppearanceSourceKeySchema>>,
    pub(super) binding_semantic_scalars:
        Option<CanonicalBlockRef<AppearanceBindingSemanticScalarSchema>>,
    pub(super) material_factor_scalars:
        Option<CanonicalBlockRef<AppearanceMaterialFactorScalarSchema>>,
}

#[derive(Clone, Copy)]
pub(super) struct AppearanceCanonicalCounts {
    images: u32,
    samplers: u32,
    textures: u32,
    materials: u32,
    bindings: u32,
}

impl AppearanceCanonicalCounts {
    pub(super) const fn new(
        images: u32,
        samplers: u32,
        textures: u32,
        materials: u32,
        bindings: u32,
    ) -> Self {
        Self {
            images,
            samplers,
            textures,
            materials,
            bindings,
        }
    }
}

trait AppearanceEquivalence {
    fn validate_directory(&mut self, directory: CanonicalRecordView<'_>) -> Option<()>;

    fn validate_image(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        payload_ref: CanonicalBytesRef<AppearanceEncodedImagePayloadSchema>,
        payload: &[u8],
        media: Option<(CanonicalUtf8Ref<AppearanceMediaTypeSchema>, &str)>,
    ) -> Option<()>;

    fn validate_sampler(&mut self, row: usize, table: CanonicalRecordView<'_>) -> Option<()>;

    fn validate_texture(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        images: CanonicalRecordRef<AppearanceImageTableSchema>,
        samplers: CanonicalRecordRef<AppearanceSamplerTableSchema>,
    ) -> Option<()>;

    fn validate_binding(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        textures: CanonicalRecordRef<AppearanceTextureTableSchema>,
        semantic_values: &[f32],
        source_key: Option<&str>,
    ) -> Option<()>;

    fn validate_material(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        bindings: Option<CanonicalRecordRef<AppearanceBindingTableSchema>>,
        factor_values: &[f32],
    ) -> Option<()>;
}

struct CanonicalAppearanceEquivalence;

impl AppearanceEquivalence for CanonicalAppearanceEquivalence {
    fn validate_directory(&mut self, _directory: CanonicalRecordView<'_>) -> Option<()> {
        Some(())
    }

    fn validate_image(
        &mut self,
        _row: usize,
        _table: CanonicalRecordView<'_>,
        _payload_ref: CanonicalBytesRef<AppearanceEncodedImagePayloadSchema>,
        _payload: &[u8],
        _media: Option<(CanonicalUtf8Ref<AppearanceMediaTypeSchema>, &str)>,
    ) -> Option<()> {
        Some(())
    }

    fn validate_sampler(&mut self, _row: usize, _table: CanonicalRecordView<'_>) -> Option<()> {
        Some(())
    }

    fn validate_texture(
        &mut self,
        _row: usize,
        _table: CanonicalRecordView<'_>,
        _images: CanonicalRecordRef<AppearanceImageTableSchema>,
        _samplers: CanonicalRecordRef<AppearanceSamplerTableSchema>,
    ) -> Option<()> {
        Some(())
    }

    fn validate_binding(
        &mut self,
        _row: usize,
        _table: CanonicalRecordView<'_>,
        _textures: CanonicalRecordRef<AppearanceTextureTableSchema>,
        _semantic_values: &[f32],
        _source_key: Option<&str>,
    ) -> Option<()> {
        Some(())
    }

    fn validate_material(
        &mut self,
        _row: usize,
        _table: CanonicalRecordView<'_>,
        _bindings: Option<CanonicalRecordRef<AppearanceBindingTableSchema>>,
        _factor_values: &[f32],
    ) -> Option<()> {
        Some(())
    }
}

struct FreshAppearanceEquivalence<'batch, 'input, R> {
    expected: &'batch ValidatedAppearanceBatch<'input, R>,
}

impl<R> AppearanceEquivalence for FreshAppearanceEquivalence<'_, '_, R> {
    fn validate_directory(&mut self, directory: CanonicalRecordView<'_>) -> Option<()> {
        for (field, expected) in [
            (
                appearance_directory_field::IMAGE_COUNT,
                self.expected.image_count(),
            ),
            (
                appearance_directory_field::SAMPLER_COUNT,
                self.expected.sampler_count(),
            ),
            (
                appearance_directory_field::TEXTURE_COUNT,
                self.expected.texture_count(),
            ),
            (
                appearance_directory_field::MATERIAL_COUNT,
                self.expected.material_count(),
            ),
            (
                appearance_directory_field::BINDING_COUNT,
                self.expected.binding_count(),
            ),
            (
                appearance_directory_field::IMAGE_PAYLOAD_BYTE_COUNT,
                self.expected.image_payload_byte_count(),
            ),
            (
                appearance_directory_field::MEDIA_TYPE_BYTE_COUNT,
                self.expected.media_type_byte_count(),
            ),
            (
                appearance_directory_field::SOURCE_KEY_BYTE_COUNT,
                self.expected.source_key_byte_count(),
            ),
            (
                appearance_directory_field::BINDING_SEMANTIC_SCALAR_COUNT,
                self.expected.binding_semantic_scalar_count(),
            ),
            (
                appearance_directory_field::MATERIAL_FACTOR_SCALAR_COUNT,
                self.expected.material_factor_scalar_count(),
            ),
        ] {
            require_scalar(directory.scalar::<u32>(0, field), expected)?;
        }
        Some(())
    }

    fn validate_image(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        payload_ref: CanonicalBytesRef<AppearanceEncodedImagePayloadSchema>,
        payload: &[u8],
        media: Option<(CanonicalUtf8Ref<AppearanceMediaTypeSchema>, &str)>,
    ) -> Option<()> {
        let image = self.expected.image(row)?;
        require_scalar(
            table.scalar::<u32>(row, appearance_image_field::PAYLOAD_OFFSET),
            image.payload_offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_image_field::PAYLOAD_COUNT),
            image.payload_count(),
        )?;
        require_scalar(
            table.bytes_ref::<AppearanceEncodedImagePayloadSchema>(
                row,
                appearance_image_field::PAYLOADS,
            ),
            payload_ref,
        )?;
        let payload_range =
            checked_range(image.payload_offset(), image.payload_count(), payload.len())?;
        if payload.get(payload_range)? != image.payload() {
            return None;
        }

        let (presence, media_ref, expected_media) = match image.media_type() {
            Presence::Absent if !image.media_type_present() && image.media_type_count() == 0 => {
                (APPEARANCE_PRESENCE_ABSENT, None, None)
            }
            Presence::Present(expected_media) if image.media_type_present() => {
                let (reference, bytes) = media?;
                (
                    APPEARANCE_PRESENCE_PRESENT,
                    Some(reference),
                    Some((expected_media, bytes)),
                )
            }
            Presence::Absent | Presence::Present(_) => return None,
        };
        require_scalar(
            table.scalar::<u8>(row, appearance_image_field::MEDIA_TYPE_PRESENCE),
            presence,
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_image_field::MEDIA_TYPE_OFFSET),
            image.media_type_offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_image_field::MEDIA_TYPE_COUNT),
            image.media_type_count(),
        )?;
        match (media_ref, expected_media) {
            (None, None) => Some(()),
            (Some(reference), Some((expected_media, bytes))) => {
                require_scalar(
                    table.utf8_ref::<AppearanceMediaTypeSchema>(
                        row,
                        appearance_image_field::MEDIA_TYPES,
                    ),
                    reference,
                )?;
                let range = checked_range(
                    image.media_type_offset(),
                    image.media_type_count(),
                    bytes.len(),
                )?;
                (bytes.as_bytes().get(range)? == expected_media.as_str().as_bytes()).then_some(())
            }
            _ => None,
        }
    }

    fn validate_sampler(&mut self, row: usize, table: CanonicalRecordView<'_>) -> Option<()> {
        let sampler = self.expected.samplers().nth(row)?;
        let state = sampler.state();
        for (field, expected) in [
            (
                appearance_sampler_field::MINIFICATION,
                encode_appearance_texel_filter(state.minification()),
            ),
            (
                appearance_sampler_field::MAGNIFICATION,
                encode_appearance_texel_filter(state.magnification()),
            ),
            (
                appearance_sampler_field::MIPMAP,
                encode_appearance_mipmap_filter(state.mipmap()),
            ),
            (
                appearance_sampler_field::ADDRESS_U,
                encode_appearance_address_mode(state.address_u()),
            ),
            (
                appearance_sampler_field::ADDRESS_V,
                encode_appearance_address_mode(state.address_v()),
            ),
            (
                appearance_sampler_field::ADDRESS_W,
                encode_appearance_address_mode(state.address_w()),
            ),
        ] {
            require_scalar(table.scalar::<u8>(row, field), expected)?;
        }
        Some(())
    }

    fn validate_texture(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        images: CanonicalRecordRef<AppearanceImageTableSchema>,
        samplers: CanonicalRecordRef<AppearanceSamplerTableSchema>,
    ) -> Option<()> {
        let texture = self.expected.textures().nth(row)?;
        require_scalar(
            table.scalar::<u32>(row, appearance_texture_field::IMAGE),
            texture.image().index(),
        )?;
        require_scalar(
            table.record_ref::<AppearanceImageTableSchema>(row, appearance_texture_field::IMAGES),
            images,
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_texture_field::SAMPLER),
            texture.sampler().index(),
        )?;
        require_scalar(
            table.record_ref::<AppearanceSamplerTableSchema>(
                row,
                appearance_texture_field::SAMPLERS,
            ),
            samplers,
        )
    }

    fn validate_binding(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        textures: CanonicalRecordRef<AppearanceTextureTableSchema>,
        semantic_values: &[f32],
        source_key: Option<&str>,
    ) -> Option<()> {
        let binding = self.expected.bindings().nth(row)?;
        require_scalar(
            table.scalar::<u32>(row, appearance_binding_field::TEXTURE),
            binding.texture().index(),
        )?;
        require_scalar(
            table.record_ref::<AppearanceTextureTableSchema>(
                row,
                appearance_binding_field::TEXTURES,
            ),
            textures,
        )?;
        require_scalar(
            table.scalar::<u8>(row, appearance_binding_field::ROLE),
            binding.role_tag(),
        )?;
        for (field, expected) in [
            (
                appearance_binding_field::COORDINATE_SOURCE,
                binding.coordinate_source_tag(),
            ),
            (
                appearance_binding_field::CHANNEL_LANE_COUNT,
                binding.channel_lane_count(),
            ),
            (
                appearance_binding_field::COLOR_INTERPRETATION,
                binding.color_interpretation_tag(),
            ),
            (
                appearance_binding_field::BLEND_OPERATION,
                binding.blend_operation_tag(),
            ),
        ] {
            require_scalar(table.scalar::<u8>(row, field), expected)?;
        }
        require_scalar(
            table.scalar::<u32>(row, appearance_binding_field::COORDINATE_SET),
            binding.coordinate_set(),
        )?;
        for (field, expected) in [
            appearance_binding_field::CHANNEL_0,
            appearance_binding_field::CHANNEL_1,
            appearance_binding_field::CHANNEL_2,
            appearance_binding_field::CHANNEL_3,
        ]
        .into_iter()
        .zip(binding.channel_tags())
        {
            require_scalar(table.scalar::<u8>(row, field), expected)?;
        }
        require_scalar(
            table.scalar::<u32>(row, appearance_binding_field::SEMANTIC_SCALAR_OFFSET),
            binding.semantic_scalar_offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_binding_field::SOURCE_KEY_OFFSET),
            binding.source_key_offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_binding_field::SOURCE_KEY_COUNT),
            binding.source_key_count(),
        )?;

        let semantics = binding.semantics();
        let transform = semantics.transform();
        let offset = transform.offset();
        let scale = transform.scale();
        let expected_values = [
            offset.x,
            offset.y,
            transform.rotation_radians(),
            scale.x,
            scale.y,
            semantics.strength().get(),
        ];
        if !f32_slices_have_same_bits(semantic_values, &expected_values) {
            return None;
        }
        match (source_key, semantics.source_key()) {
            (None, None) => Some(()),
            (Some(actual), Some(expected)) if actual == expected.as_str() => Some(()),
            _ => None,
        }
    }

    fn validate_material(
        &mut self,
        row: usize,
        table: CanonicalRecordView<'_>,
        bindings: Option<CanonicalRecordRef<AppearanceBindingTableSchema>>,
        factor_values: &[f32],
    ) -> Option<()> {
        let material = self.expected.material(row)?;
        let range = material.authored().bindings();
        for (field, expected) in [
            (
                appearance_material_field::SHADING_MODEL,
                material.shading_model_tag(),
            ),
            (
                appearance_material_field::ALPHA_MODE,
                material.alpha_mode_tag(),
            ),
            (
                appearance_material_field::DOUBLE_SIDED,
                material.double_sided_tag(),
            ),
            (
                appearance_material_field::PBR_METALLIC_ROUGHNESS_PRESENCE,
                material.pbr_metallic_roughness_presence(),
            ),
            (
                appearance_material_field::PBR_SPECULAR_GLOSSINESS_PRESENCE,
                material.pbr_specular_glossiness_presence(),
            ),
            (
                appearance_material_field::LEGACY_PRESENCE,
                material.legacy_presence(),
            ),
        ] {
            require_scalar(table.scalar::<u8>(row, field), expected)?;
        }
        require_f32_bits(
            table.scalar::<f32>(row, appearance_material_field::ALPHA_CUTOFF),
            material.alpha_cutoff(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_material_field::BINDING_OFFSET),
            range.offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_material_field::BINDING_COUNT),
            range.count(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_material_field::FACTOR_OFFSET),
            material.factor_offset(),
        )?;
        let expected_values = material_factor_values(material.semantics().factors());
        if !f32_slices_have_same_bits(factor_values, &expected_values) {
            return None;
        }
        if range.count() == 0 {
            require_scalar(
                table.scalar::<u8>(row, appearance_material_field::BINDINGS),
                0_u8,
            )
        } else {
            require_scalar(
                table.record_ref::<AppearanceBindingTableSchema>(
                    row,
                    appearance_material_field::BINDINGS,
                ),
                bindings?,
            )
        }
    }
}

pub(super) fn validate_appearance_canonical_structure(
    session: &mut StoreValidationSession<'_, '_>,
    appearance_ref: CanonicalRecordRef<AppearanceDirectorySchema>,
    counts: AppearanceCanonicalCounts,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<AppearanceValidatedRefs> {
    validate_appearance_graph_inner(
        session,
        appearance_ref,
        counts,
        &mut CanonicalAppearanceEquivalence,
        observe_work,
    )
}

pub(super) fn validate_appearance_graph<R>(
    session: &mut StoreValidationSession<'_, '_>,
    appearance_ref: CanonicalRecordRef<AppearanceDirectorySchema>,
    expected: &ValidatedAppearanceBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<AppearanceValidatedRefs> {
    validate_appearance_graph_inner(
        session,
        appearance_ref,
        AppearanceCanonicalCounts::new(
            expected.image_count(),
            expected.sampler_count(),
            expected.texture_count(),
            expected.material_count(),
            expected.binding_count(),
        ),
        &mut FreshAppearanceEquivalence { expected },
        observe_work,
    )
}

#[allow(clippy::too_many_lines)]
fn validate_appearance_graph_inner(
    session: &mut StoreValidationSession<'_, '_>,
    appearance_ref: CanonicalRecordRef<AppearanceDirectorySchema>,
    counts: AppearanceCanonicalCounts,
    equivalence: &mut impl AppearanceEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<AppearanceValidatedRefs> {
    observe_work(1)?;
    let directory = session.canonical_records(appearance_ref)?;
    if directory.rows() != 1 || directory.fields() != APPEARANCE_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, appearance_directory_field::SCHEMA_REVISION),
        APPEARANCE_SCHEMA_REVISION,
    )?;
    equivalence.validate_directory(directory)?;

    let images = optional_record_ref::<AppearanceImageTableSchema>(
        session,
        directory,
        appearance_directory_field::IMAGE_COUNT,
        appearance_directory_field::IMAGES,
        counts.images,
        APPEARANCE_IMAGE_FIELD_COUNT,
    )?;
    let samplers = optional_record_ref::<AppearanceSamplerTableSchema>(
        session,
        directory,
        appearance_directory_field::SAMPLER_COUNT,
        appearance_directory_field::SAMPLERS,
        counts.samplers,
        APPEARANCE_SAMPLER_FIELD_COUNT,
    )?;
    let textures = optional_record_ref::<AppearanceTextureTableSchema>(
        session,
        directory,
        appearance_directory_field::TEXTURE_COUNT,
        appearance_directory_field::TEXTURES,
        counts.textures,
        APPEARANCE_TEXTURE_FIELD_COUNT,
    )?;
    let materials = optional_record_ref::<AppearanceMaterialTableSchema>(
        session,
        directory,
        appearance_directory_field::MATERIAL_COUNT,
        appearance_directory_field::MATERIALS,
        counts.materials,
        APPEARANCE_MATERIAL_FIELD_COUNT,
    )?;
    let bindings = optional_record_ref::<AppearanceBindingTableSchema>(
        session,
        directory,
        appearance_directory_field::BINDING_COUNT,
        appearance_directory_field::BINDINGS,
        counts.bindings,
        APPEARANCE_BINDING_FIELD_COUNT,
    )?;
    let image_payloads = optional_bytes_ref::<AppearanceEncodedImagePayloadSchema>(
        session,
        directory,
        appearance_directory_field::IMAGE_PAYLOAD_BYTE_COUNT,
        appearance_directory_field::IMAGE_PAYLOADS,
        directory.scalar::<u32>(0, appearance_directory_field::IMAGE_PAYLOAD_BYTE_COUNT)?,
    )?;
    let media_types = optional_utf8_ref::<AppearanceMediaTypeSchema>(
        session,
        directory,
        appearance_directory_field::MEDIA_TYPE_BYTE_COUNT,
        appearance_directory_field::MEDIA_TYPES,
        directory.scalar::<u32>(0, appearance_directory_field::MEDIA_TYPE_BYTE_COUNT)?,
    )?;
    let source_keys = optional_utf8_ref::<AppearanceSourceKeySchema>(
        session,
        directory,
        appearance_directory_field::SOURCE_KEY_BYTE_COUNT,
        appearance_directory_field::SOURCE_KEYS,
        directory.scalar::<u32>(0, appearance_directory_field::SOURCE_KEY_BYTE_COUNT)?,
    )?;
    let expected_binding_semantic_scalar_count = counts
        .bindings
        .checked_mul(u32::try_from(APPEARANCE_BINDING_SEMANTIC_STRIDE).ok()?)?;
    let binding_semantic_scalars = optional_sequence_ref::<AppearanceBindingSemanticScalarSchema>(
        session,
        directory,
        appearance_directory_field::BINDING_SEMANTIC_SCALAR_COUNT,
        appearance_directory_field::BINDING_SEMANTIC_SCALARS,
        expected_binding_semantic_scalar_count,
    )?;
    let expected_material_factor_scalar_count = counts
        .materials
        .checked_mul(u32::try_from(APPEARANCE_MATERIAL_FACTOR_STRIDE).ok()?)?;
    let material_factor_scalars = optional_sequence_ref::<AppearanceMaterialFactorScalarSchema>(
        session,
        directory,
        appearance_directory_field::MATERIAL_FACTOR_SCALAR_COUNT,
        appearance_directory_field::MATERIAL_FACTOR_SCALARS,
        expected_material_factor_scalar_count,
    )?;

    validate_images_structure(
        session,
        images,
        image_payloads,
        media_types,
        equivalence,
        observe_work,
    )?;
    validate_samplers_structure(session, samplers, equivalence, observe_work)?;
    validate_textures_structure(
        session,
        textures,
        images,
        counts.images,
        samplers,
        counts.samplers,
        equivalence,
        observe_work,
    )?;
    validate_bindings_structure(
        session,
        bindings,
        textures,
        counts.textures,
        source_keys,
        binding_semantic_scalars,
        equivalence,
        observe_work,
    )?;
    validate_materials_structure(
        session,
        materials,
        bindings,
        counts.bindings,
        material_factor_scalars,
        equivalence,
        observe_work,
    )?;

    Some(AppearanceValidatedRefs {
        images: images.map(CanonicalBlockRef::from_store),
        samplers: samplers.map(CanonicalBlockRef::from_store),
        textures: textures.map(CanonicalBlockRef::from_store),
        materials: materials.map(CanonicalBlockRef::from_store),
        bindings: bindings.map(CanonicalBlockRef::from_store),
        image_payloads: image_payloads.map(CanonicalBlockRef::from_store),
        media_types: media_types.map(CanonicalBlockRef::from_store),
        source_keys: source_keys.map(CanonicalBlockRef::from_store),
        binding_semantic_scalars: binding_semantic_scalars.map(CanonicalBlockRef::from_store),
        material_factor_scalars: material_factor_scalars.map(CanonicalBlockRef::from_store),
    })
}

fn optional_record_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
    expected_count: u32,
    expected_fields: usize,
) -> Option<Option<CanonicalRecordRef<Tag>>>
where
    Tag: Copy + Send + Sync + 'static,
{
    require_scalar(directory.scalar::<u32>(0, count_field), expected_count)?;
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        let reference = directory.record_ref::<Tag>(0, reference_field)?;
        let table = session.canonical_records(reference)?;
        if table.rows() != usize::try_from(expected_count).ok()?
            || table.fields() != expected_fields
        {
            return None;
        }
        Some(Some(reference))
    }
}

fn optional_bytes_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
    expected_count: u32,
) -> Option<Option<CanonicalBytesRef<Tag>>>
where
    Tag: Copy + Send + Sync + 'static,
{
    require_scalar(directory.scalar::<u32>(0, count_field), expected_count)?;
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        let reference = directory.bytes_ref::<Tag>(0, reference_field)?;
        let payload = session.canonical_bytes(reference)?;
        (payload.len() == usize::try_from(expected_count).ok()?).then_some(Some(reference))
    }
}

fn optional_utf8_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
    expected_count: u32,
) -> Option<Option<CanonicalUtf8Ref<Tag>>>
where
    Tag: Copy + Send + Sync + 'static,
{
    require_scalar(directory.scalar::<u32>(0, count_field), expected_count)?;
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        let reference = directory.utf8_ref::<Tag>(0, reference_field)?;
        let payload = session.canonical_utf8(reference)?;
        (payload.len() == usize::try_from(expected_count).ok()?).then_some(Some(reference))
    }
}

fn optional_sequence_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
    expected_count: u32,
) -> Option<Option<CanonicalSequenceRef<f32, Tag>>>
where
    Tag: Copy + Send + Sync + 'static,
{
    require_scalar(directory.scalar::<u32>(0, count_field), expected_count)?;
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        let reference = directory.sequence_ref::<f32, Tag>(0, reference_field)?;
        let payload = session.canonical_sequence(reference)?;
        (payload.len() == usize::try_from(expected_count).ok()?).then_some(Some(reference))
    }
}

fn validate_images_structure(
    session: &mut StoreValidationSession<'_, '_>,
    images: Option<CanonicalRecordRef<AppearanceImageTableSchema>>,
    image_payloads: Option<CanonicalBytesRef<AppearanceEncodedImagePayloadSchema>>,
    media_types: Option<CanonicalUtf8Ref<AppearanceMediaTypeSchema>>,
    equivalence: &mut impl AppearanceEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(images) = images else {
        return (image_payloads.is_none() && media_types.is_none()).then_some(());
    };
    let table = session.canonical_records(images)?;
    let payload_ref = image_payloads?;
    let payload = session.canonical_bytes(payload_ref)?;
    let media = match media_types {
        Some(reference) => Some((reference, session.canonical_utf8(reference)?)),
        None => None,
    };
    let mut payload_cursor = 0_usize;
    let mut media_cursor = 0_usize;
    for row in 0..table.rows() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, appearance_image_field::IMAGE),
            u32::try_from(row).ok()?,
        )?;
        require_scalar(
            table.scalar::<u8>(row, appearance_image_field::PAYLOAD_KIND),
            APPEARANCE_IMAGE_PAYLOAD_ENCODED,
        )?;
        let payload_offset = table.scalar::<u32>(row, appearance_image_field::PAYLOAD_OFFSET)?;
        let payload_count = table.scalar::<u32>(row, appearance_image_field::PAYLOAD_COUNT)?;
        if payload_count == 0 {
            return None;
        }
        require_scalar(Some(payload_offset), u32::try_from(payload_cursor).ok()?)?;
        require_scalar(
            table.bytes_ref::<AppearanceEncodedImagePayloadSchema>(
                row,
                appearance_image_field::PAYLOADS,
            ),
            payload_ref,
        )?;
        let payload_range = checked_range(payload_offset, payload_count, payload.len())?;
        observe_work(u64::try_from(payload_range.len()).ok()?)?;
        payload_cursor = payload_range.end;

        let presence = table.scalar::<u8>(row, appearance_image_field::MEDIA_TYPE_PRESENCE)?;
        require_scalar(
            table.scalar::<u8>(row, appearance_image_field::MEDIA_TYPE_ENCODING),
            APPEARANCE_MEDIA_TYPE_UTF8,
        )?;
        let media_offset = table.scalar::<u32>(row, appearance_image_field::MEDIA_TYPE_OFFSET)?;
        let media_count = table.scalar::<u32>(row, appearance_image_field::MEDIA_TYPE_COUNT)?;
        require_scalar(Some(media_offset), u32::try_from(media_cursor).ok()?)?;
        match (presence, media_count, media) {
            (APPEARANCE_PRESENCE_ABSENT, 0, None) => require_scalar(
                table.scalar::<u8>(row, appearance_image_field::MEDIA_TYPES),
                0_u8,
            )?,
            (APPEARANCE_PRESENCE_ABSENT, 0, Some(_)) => require_scalar(
                table.scalar::<u8>(row, appearance_image_field::MEDIA_TYPES),
                0_u8,
            )?,
            (APPEARANCE_PRESENCE_PRESENT, count, Some((reference, bytes))) if count != 0 => {
                require_scalar(
                    table.utf8_ref::<AppearanceMediaTypeSchema>(
                        row,
                        appearance_image_field::MEDIA_TYPES,
                    ),
                    reference,
                )?;
                let range = checked_range(media_offset, count, bytes.len())?;
                observe_work(u64::try_from(range.len()).ok()?)?;
                ExtensionMediaType::new(bytes.get(range.clone())?).ok()?;
                media_cursor = range.end;
            }
            _ => return None,
        }
        equivalence.validate_image(row, table, payload_ref, payload, media)?;
    }
    (payload_cursor == payload.len() && media_cursor == media.map_or(0, |(_, bytes)| bytes.len()))
        .then_some(())
}

fn validate_samplers_structure(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<AppearanceSamplerTableSchema>>,
    equivalence: &mut impl AppearanceEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return Some(());
    };
    let table = session.canonical_records(reference)?;
    for row in 0..table.rows() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, appearance_sampler_field::SAMPLER),
            u32::try_from(row).ok()?,
        )?;
        for field in [
            appearance_sampler_field::MINIFICATION,
            appearance_sampler_field::MAGNIFICATION,
        ] {
            if table.scalar::<u8>(row, field)? > APPEARANCE_TEXEL_FILTER_LINEAR {
                return None;
            }
        }
        if table.scalar::<u8>(row, appearance_sampler_field::MIPMAP)?
            > APPEARANCE_MIPMAP_FILTER_LINEAR
        {
            return None;
        }
        for field in [
            appearance_sampler_field::ADDRESS_U,
            appearance_sampler_field::ADDRESS_V,
            appearance_sampler_field::ADDRESS_W,
        ] {
            if table.scalar::<u8>(row, field)? > APPEARANCE_ADDRESS_CLAMP_TO_BORDER {
                return None;
            }
        }
        equivalence.validate_sampler(row, table)?;
    }
    Some(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "texture validation keeps image and sampler proof references, counts, equivalence, and work authority explicit"
)]
fn validate_textures_structure(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<AppearanceTextureTableSchema>>,
    images: Option<CanonicalRecordRef<AppearanceImageTableSchema>>,
    image_count: u32,
    samplers: Option<CanonicalRecordRef<AppearanceSamplerTableSchema>>,
    sampler_count: u32,
    equivalence: &mut impl AppearanceEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return Some(());
    };
    let images_ref = images?;
    let samplers_ref = samplers?;
    let table = session.canonical_records(reference)?;
    for row in 0..table.rows() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, appearance_texture_field::TEXTURE),
            u32::try_from(row).ok()?,
        )?;
        let image = table.scalar::<u32>(row, appearance_texture_field::IMAGE)?;
        let sampler = table.scalar::<u32>(row, appearance_texture_field::SAMPLER)?;
        if image >= image_count || sampler >= sampler_count {
            return None;
        }
        require_scalar(
            table.record_ref::<AppearanceImageTableSchema>(row, appearance_texture_field::IMAGES),
            images_ref,
        )?;
        require_scalar(
            table.record_ref::<AppearanceSamplerTableSchema>(
                row,
                appearance_texture_field::SAMPLERS,
            ),
            samplers_ref,
        )?;
        equivalence.validate_texture(row, table, images_ref, samplers_ref)?;
    }
    Some(())
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn validate_bindings_structure(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<AppearanceBindingTableSchema>>,
    textures: Option<CanonicalRecordRef<AppearanceTextureTableSchema>>,
    texture_count: u32,
    source_keys: Option<CanonicalUtf8Ref<AppearanceSourceKeySchema>>,
    semantic_scalars: Option<CanonicalSequenceRef<f32, AppearanceBindingSemanticScalarSchema>>,
    equivalence: &mut impl AppearanceEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (source_keys.is_none() && semantic_scalars.is_none()).then_some(());
    };
    let textures_ref = textures?;
    let table = session.canonical_records(reference)?;
    let source_keys = match source_keys {
        Some(reference) => Some((reference, session.canonical_utf8(reference)?)),
        None => None,
    };
    let semantic_scalars_ref = semantic_scalars?;
    let semantic_scalars = session.canonical_sequence(semantic_scalars_ref)?.as_slice();
    let semantic_stride = u32::try_from(APPEARANCE_BINDING_SEMANTIC_STRIDE).ok()?;
    let mut source_key_cursor = 0_usize;
    let mut semantic_cursor = 0_usize;
    for row in 0..table.rows() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, appearance_binding_field::BINDING),
            u32::try_from(row).ok()?,
        )?;
        let texture = table.scalar::<u32>(row, appearance_binding_field::TEXTURE)?;
        if texture >= texture_count {
            return None;
        }
        require_scalar(
            table.record_ref::<AppearanceTextureTableSchema>(
                row,
                appearance_binding_field::TEXTURES,
            ),
            textures_ref,
        )?;

        let role = decode_appearance_texture_role(
            table.scalar::<u8>(row, appearance_binding_field::ROLE)?,
        )?;
        let coordinate_source = decode_appearance_coordinate_source(
            table.scalar::<u8>(row, appearance_binding_field::COORDINATE_SOURCE)?,
            table.scalar::<u32>(row, appearance_binding_field::COORDINATE_SET)?,
        )?;
        let channels = [
            table.scalar::<u8>(row, appearance_binding_field::CHANNEL_0)?,
            table.scalar::<u8>(row, appearance_binding_field::CHANNEL_1)?,
            table.scalar::<u8>(row, appearance_binding_field::CHANNEL_2)?,
            table.scalar::<u8>(row, appearance_binding_field::CHANNEL_3)?,
        ];
        let channel_mapping = decode_channel_mapping(
            table.scalar::<u8>(row, appearance_binding_field::CHANNEL_LANE_COUNT)?,
            channels,
        )?;
        let color_interpretation = decode_appearance_color_interpretation(
            table.scalar::<u8>(row, appearance_binding_field::COLOR_INTERPRETATION)?,
        )?;
        let blend = decode_appearance_blend_operation(
            table.scalar::<u8>(row, appearance_binding_field::BLEND_OPERATION)?,
        )?;

        let semantic_offset =
            table.scalar::<u32>(row, appearance_binding_field::SEMANTIC_SCALAR_OFFSET)?;
        let semantic_count =
            table.scalar::<u32>(row, appearance_binding_field::SEMANTIC_SCALAR_COUNT)?;
        require_scalar(Some(semantic_offset), u32::try_from(semantic_cursor).ok()?)?;
        require_scalar(Some(semantic_count), semantic_stride)?;
        require_scalar(
            table.sequence_ref::<f32, AppearanceBindingSemanticScalarSchema>(
                row,
                appearance_binding_field::SEMANTIC_SCALARS,
            ),
            semantic_scalars_ref,
        )?;
        let semantic_range =
            checked_range(semantic_offset, semantic_count, semantic_scalars.len())?;
        observe_work(u64::try_from(semantic_range.len()).ok()?)?;
        let semantic_values = semantic_scalars.get(semantic_range.clone())?;
        semantic_cursor = semantic_range.end;
        let (transform, strength) = decode_appearance_binding_numeric_semantics(semantic_values)?;

        let source_key_offset =
            table.scalar::<u32>(row, appearance_binding_field::SOURCE_KEY_OFFSET)?;
        let source_key_count =
            table.scalar::<u32>(row, appearance_binding_field::SOURCE_KEY_COUNT)?;
        require_scalar(
            Some(source_key_offset),
            u32::try_from(source_key_cursor).ok()?,
        )?;
        let (source_key, source_key_text) = if source_key_count == 0 {
            require_scalar(
                table.scalar::<u8>(row, appearance_binding_field::SOURCE_KEYS),
                0_u8,
            )?;
            (None, None)
        } else {
            let (source_keys_ref, source_key_payload) = source_keys?;
            require_scalar(
                table.utf8_ref::<AppearanceSourceKeySchema>(
                    row,
                    appearance_binding_field::SOURCE_KEYS,
                ),
                source_keys_ref,
            )?;
            let source_key_range = checked_range(
                source_key_offset,
                source_key_count,
                source_key_payload.len(),
            )?;
            observe_work(u64::try_from(source_key_range.len()).ok()?)?;
            let source_key_text = source_key_payload.get(source_key_range.clone())?;
            source_key_cursor = source_key_range.end;
            (
                Some(TextureSourceKey::try_new(source_key_text).ok()?),
                Some(source_key_text),
            )
        };
        let semantics = TextureBindingSemanticsRef::new(
            &role,
            &coordinate_source,
            &channel_mapping,
            &color_interpretation,
            transform,
            strength,
            &blend,
            source_key.as_ref(),
        );
        validate_texture_binding_semantics(semantics).ok()?;
        equivalence.validate_binding(row, table, textures_ref, semantic_values, source_key_text)?;
    }
    (semantic_cursor == semantic_scalars.len()
        && source_key_cursor == source_keys.map_or(0, |(_, payload)| payload.len()))
    .then_some(())
}

fn validate_materials_structure(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
    bindings: Option<CanonicalRecordRef<AppearanceBindingTableSchema>>,
    binding_count: u32,
    factor_scalars: Option<CanonicalSequenceRef<f32, AppearanceMaterialFactorScalarSchema>>,
    equivalence: &mut impl AppearanceEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (binding_count == 0 && factor_scalars.is_none()).then_some(());
    };
    let binding_rows = usize::try_from(binding_count).ok()?;
    let binding_table = match bindings {
        Some(reference) => Some(session.canonical_records(reference)?),
        None => None,
    };
    let table = session.canonical_records(reference)?;
    let factor_scalars_ref = factor_scalars?;
    let factor_scalars = session.canonical_sequence(factor_scalars_ref)?.as_slice();
    let factor_stride = u32::try_from(APPEARANCE_MATERIAL_FACTOR_STRIDE).ok()?;
    let mut binding_cursor = 0_usize;
    let mut factor_cursor = 0_usize;
    for row in 0..table.rows() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, appearance_material_field::MATERIAL),
            u32::try_from(row).ok()?,
        )?;
        let shading_model = decode_appearance_shading_model(
            table.scalar::<u8>(row, appearance_material_field::SHADING_MODEL)?,
        )?;
        let alpha_mode = decode_appearance_alpha_mode(
            table.scalar::<u8>(row, appearance_material_field::ALPHA_MODE)?,
            table.scalar::<f32>(row, appearance_material_field::ALPHA_CUTOFF)?,
        )?;
        let double_sided =
            decode_presence(table.scalar::<u8>(row, appearance_material_field::DOUBLE_SIDED)?)?;
        let pbr_metallic_roughness_presence = table.scalar::<u8>(
            row,
            appearance_material_field::PBR_METALLIC_ROUGHNESS_PRESENCE,
        )?;
        let pbr_specular_glossiness_presence = table.scalar::<u8>(
            row,
            appearance_material_field::PBR_SPECULAR_GLOSSINESS_PRESENCE,
        )?;
        let legacy_presence =
            table.scalar::<u8>(row, appearance_material_field::LEGACY_PRESENCE)?;
        let offset = table.scalar::<u32>(row, appearance_material_field::BINDING_OFFSET)?;
        let count = table.scalar::<u32>(row, appearance_material_field::BINDING_COUNT)?;
        require_scalar(Some(offset), u32::try_from(binding_cursor).ok()?)?;
        let binding_range = checked_range(offset, count, binding_rows)?;
        binding_cursor = binding_range.end;
        if count == 0 {
            require_scalar(
                table.scalar::<u8>(row, appearance_material_field::BINDINGS),
                0_u8,
            )?;
        } else {
            require_scalar(
                table.record_ref::<AppearanceBindingTableSchema>(
                    row,
                    appearance_material_field::BINDINGS,
                ),
                bindings?,
            )?;
        }

        let factor_offset = table.scalar::<u32>(row, appearance_material_field::FACTOR_OFFSET)?;
        let factor_count = table.scalar::<u32>(row, appearance_material_field::FACTOR_COUNT)?;
        require_scalar(Some(factor_offset), u32::try_from(factor_cursor).ok()?)?;
        require_scalar(Some(factor_count), factor_stride)?;
        require_scalar(
            table.sequence_ref::<f32, AppearanceMaterialFactorScalarSchema>(
                row,
                appearance_material_field::FACTORS,
            ),
            factor_scalars_ref,
        )?;
        let factor_range = checked_range(factor_offset, factor_count, factor_scalars.len())?;
        observe_work(u64::try_from(factor_range.len()).ok()?)?;
        let factor_values = factor_scalars.get(factor_range.clone())?;
        factor_cursor = factor_range.end;
        let factors = decode_appearance_material_factors(
            factor_values,
            pbr_metallic_roughness_presence,
            pbr_specular_glossiness_presence,
            legacy_presence,
        )?;
        validate_material_semantics(MaterialSemanticsRef::new(
            &shading_model,
            &alpha_mode,
            double_sided,
            factors,
        ))
        .ok()?;
        validate_material_binding_authority(binding_table, binding_range, observe_work)?;
        equivalence.validate_material(row, table, bindings, factor_values)?;
    }
    (binding_cursor == binding_rows && factor_cursor == factor_scalars.len()).then_some(())
}

fn checked_range(offset: u32, count: u32, len: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(offset).ok()?;
    let count = usize::try_from(count).ok()?;
    let end = start.checked_add(count)?;
    (end <= len).then_some(start..end)
}

fn decode_channel_mapping(lane_count: u8, tags: [u8; 4]) -> Option<TextureChannelMapping> {
    let lane_count = usize::from(lane_count);
    if !(1..=tags.len()).contains(&lane_count)
        || tags
            .get(lane_count..)?
            .iter()
            .any(|&tag| tag != APPEARANCE_CHANNEL_ZERO)
    {
        return None;
    }
    let mut channels = [TextureChannel::Zero; 4];
    for (channel, &tag) in channels.get_mut(..lane_count)?.iter_mut().zip(&tags) {
        *channel = decode_appearance_channel(tag)?;
    }
    TextureChannelSwizzle::try_new(channels.get(..lane_count)?)
        .ok()
        .map(TextureChannelMapping::swizzle)
}

fn decode_presence(tag: u8) -> Option<bool> {
    match tag {
        APPEARANCE_PRESENCE_ABSENT => Some(false),
        APPEARANCE_PRESENCE_PRESENT => Some(true),
        _ => None,
    }
}

fn validate_material_binding_authority(
    table: Option<CanonicalRecordView<'_>>,
    range: std::ops::Range<usize>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    if range.is_empty() {
        return Some(());
    }
    let table = table?;
    let mut seen = [false; TextureRoleKind::BUILTIN_COUNT];
    let mut metallic = false;
    let mut roughness = false;
    let mut metallic_roughness = false;
    for row in range {
        observe_work(1)?;
        let tag = table.scalar::<u8>(row, appearance_binding_field::ROLE)?;
        let role_seen = seen.get_mut(usize::from(tag))?;
        if *role_seen {
            return None;
        }
        *role_seen = true;
        match tag {
            APPEARANCE_ROLE_METALLIC if metallic_roughness => return None,
            APPEARANCE_ROLE_ROUGHNESS if metallic_roughness => return None,
            APPEARANCE_ROLE_METALLIC_ROUGHNESS if metallic || roughness => return None,
            APPEARANCE_ROLE_METALLIC => metallic = true,
            APPEARANCE_ROLE_ROUGHNESS => roughness = true,
            APPEARANCE_ROLE_METALLIC_ROUGHNESS => metallic_roughness = true,
            _ => {}
        }
    }
    Some(())
}

fn material_factor_values(factors: MaterialFactors) -> [f32; APPEARANCE_MATERIAL_FACTOR_STRIDE] {
    let mut values = [0.0_f32; APPEARANCE_MATERIAL_FACTOR_STRIDE];
    let emissive = factors.emissive();
    values[appearance_material_factor_scalar::EMISSIVE_X] = emissive.x;
    values[appearance_material_factor_scalar::EMISSIVE_Y] = emissive.y;
    values[appearance_material_factor_scalar::EMISSIVE_Z] = emissive.z;
    if let Some(pbr) = factors.pbr_metallic_roughness() {
        let base = pbr.base_color();
        values[appearance_material_factor_scalar::PBR_BASE_COLOR_R] = base.r;
        values[appearance_material_factor_scalar::PBR_BASE_COLOR_G] = base.g;
        values[appearance_material_factor_scalar::PBR_BASE_COLOR_B] = base.b;
        values[appearance_material_factor_scalar::PBR_BASE_COLOR_A] = base.a;
        values[appearance_material_factor_scalar::METALLIC] = pbr.metallic();
        values[appearance_material_factor_scalar::ROUGHNESS] = pbr.roughness();
    }
    if let Some(pbr) = factors.pbr_specular_glossiness() {
        let diffuse = pbr.diffuse();
        values[appearance_material_factor_scalar::PBR_DIFFUSE_R] = diffuse.r;
        values[appearance_material_factor_scalar::PBR_DIFFUSE_G] = diffuse.g;
        values[appearance_material_factor_scalar::PBR_DIFFUSE_B] = diffuse.b;
        values[appearance_material_factor_scalar::PBR_DIFFUSE_A] = diffuse.a;
        let specular = pbr.specular();
        values[appearance_material_factor_scalar::PBR_SPECULAR_X] = specular.x;
        values[appearance_material_factor_scalar::PBR_SPECULAR_Y] = specular.y;
        values[appearance_material_factor_scalar::PBR_SPECULAR_Z] = specular.z;
        values[appearance_material_factor_scalar::GLOSSINESS] = pbr.glossiness();
    }
    if let Some(legacy) = factors.legacy() {
        let ambient = legacy.ambient();
        values[appearance_material_factor_scalar::LEGACY_AMBIENT_X] = ambient.x;
        values[appearance_material_factor_scalar::LEGACY_AMBIENT_Y] = ambient.y;
        values[appearance_material_factor_scalar::LEGACY_AMBIENT_Z] = ambient.z;
        let diffuse = legacy.diffuse();
        values[appearance_material_factor_scalar::LEGACY_DIFFUSE_X] = diffuse.x;
        values[appearance_material_factor_scalar::LEGACY_DIFFUSE_Y] = diffuse.y;
        values[appearance_material_factor_scalar::LEGACY_DIFFUSE_Z] = diffuse.z;
        let specular = legacy.specular();
        values[appearance_material_factor_scalar::LEGACY_SPECULAR_X] = specular.x;
        values[appearance_material_factor_scalar::LEGACY_SPECULAR_Y] = specular.y;
        values[appearance_material_factor_scalar::LEGACY_SPECULAR_Z] = specular.z;
        let legacy_emissive = legacy.emissive();
        values[appearance_material_factor_scalar::LEGACY_EMISSIVE_X] = legacy_emissive.x;
        values[appearance_material_factor_scalar::LEGACY_EMISSIVE_Y] = legacy_emissive.y;
        values[appearance_material_factor_scalar::LEGACY_EMISSIVE_Z] = legacy_emissive.z;
        values[appearance_material_factor_scalar::LEGACY_SHININESS] = legacy.shininess();
        values[appearance_material_factor_scalar::LEGACY_OPACITY] = legacy.opacity();
    }
    let supplemental = factors.supplemental_pbr();
    values[appearance_material_factor_scalar::CLEARCOAT] = supplemental.clearcoat();
    values[appearance_material_factor_scalar::CLEARCOAT_ROUGHNESS] =
        supplemental.clearcoat_roughness();
    values[appearance_material_factor_scalar::TRANSMISSION] = supplemental.transmission();
    values[appearance_material_factor_scalar::THICKNESS] = supplemental.thickness();
    let sheen = supplemental.sheen_color();
    values[appearance_material_factor_scalar::SHEEN_COLOR_X] = sheen.x;
    values[appearance_material_factor_scalar::SHEEN_COLOR_Y] = sheen.y;
    values[appearance_material_factor_scalar::SHEEN_COLOR_Z] = sheen.z;
    values[appearance_material_factor_scalar::SHEEN_ROUGHNESS] = supplemental.sheen_roughness();
    values
}

fn require_f32_bits(actual: Option<f32>, expected: f32) -> Option<()> {
    (actual?.to_bits() == expected.to_bits()).then_some(())
}

fn f32_slices_have_same_bits(actual: &[f32], expected: &[f32]) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected)
            .all(|(&actual, &expected)| actual.to_bits() == expected.to_bits())
}
