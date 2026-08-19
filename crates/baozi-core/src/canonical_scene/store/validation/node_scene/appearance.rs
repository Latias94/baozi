use baozi_operation::__retained_store_integration::activation::{
    CanonicalValidationMember, CanonicalValidationSession,
};
use baozi_operation::{
    ByteBlock, CanonicalBytesRef, CanonicalRecordRef, CanonicalRecordView, CanonicalSequenceRef,
    CanonicalUtf8Ref, OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, ScalarSequence,
    StoreValidationSession, StoreWorkError, Utf8Block,
};

use crate::appearance::{
    MAX_TEXTURE_SOURCE_KEY_BYTES, MaterialFactors, MaterialSemanticsRef,
    TextureBindingSemanticsRef, TextureChannel, TextureChannelMapping, TextureChannelSwizzle,
    TextureRoleKind, TextureSourceKey, validate_material_semantics,
    validate_texture_binding_semantics,
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
    AppearanceImageTableSchema, AppearanceMaterialFactorScalarSchema, AppearanceMaterialNameSchema,
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
use crate::entity::{MAX_ENTITY_NAME_BYTES, validate_entity_name_str};
use crate::extension::{ExtensionMediaType, MAX_EXTENSION_MEDIA_TYPE_BYTES};

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
    pub(super) material_names: Option<CanonicalBlockRef<AppearanceMaterialNameSchema>>,
    pub(super) binding_semantic_scalars:
        Option<CanonicalBlockRef<AppearanceBindingSemanticScalarSchema>>,
    pub(super) material_factor_scalars:
        Option<CanonicalBlockRef<AppearanceMaterialFactorScalarSchema>>,
}

/// Validation-scoped appearance members retained for cross-domain checks.
#[derive(Clone, Copy)]
pub(super) struct AppearanceValidationMembers<'session> {
    pub(super) images:
        Option<CanonicalValidationMember<'session, RecordBlock, AppearanceImageTableSchema>>,
    pub(super) samplers:
        Option<CanonicalValidationMember<'session, RecordBlock, AppearanceSamplerTableSchema>>,
    pub(super) textures:
        Option<CanonicalValidationMember<'session, RecordBlock, AppearanceTextureTableSchema>>,
    pub(super) materials:
        Option<CanonicalValidationMember<'session, RecordBlock, AppearanceMaterialTableSchema>>,
    pub(super) bindings:
        Option<CanonicalValidationMember<'session, RecordBlock, AppearanceBindingTableSchema>>,
    pub(super) image_payloads:
        Option<CanonicalValidationMember<'session, ByteBlock, AppearanceEncodedImagePayloadSchema>>,
    pub(super) media_types:
        Option<CanonicalValidationMember<'session, Utf8Block, AppearanceMediaTypeSchema>>,
    pub(super) source_keys:
        Option<CanonicalValidationMember<'session, Utf8Block, AppearanceSourceKeySchema>>,
    pub(super) material_names:
        Option<CanonicalValidationMember<'session, Utf8Block, AppearanceMaterialNameSchema>>,
    pub(super) binding_semantic_scalars: Option<
        CanonicalValidationMember<
            'session,
            ScalarSequence<f32>,
            AppearanceBindingSemanticScalarSchema,
        >,
    >,
    pub(super) material_factor_scalars: Option<
        CanonicalValidationMember<
            'session,
            ScalarSequence<f32>,
            AppearanceMaterialFactorScalarSchema,
        >,
    >,
    pub(super) counts: AppearanceCanonicalCounts,
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct AppearanceCanonicalCounts {
    pub(super) images: u32,
    pub(super) samplers: u32,
    pub(super) textures: u32,
    pub(super) materials: u32,
    pub(super) bindings: u32,
}

impl AppearanceCanonicalCounts {
    pub(in crate::canonical_scene::store) const fn new(
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

/// Exact borrowed scratch shape required by typed appearance validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct AppearanceValidationScratchRequirements {
    image_media_counts: usize,
    bindings: usize,
    materials: usize,
}

impl AppearanceValidationScratchRequirements {
    pub(in crate::canonical_scene::store) fn from_counts(
        counts: AppearanceCanonicalCounts,
    ) -> Option<Self> {
        Some(Self {
            image_media_counts: usize::try_from(counts.images).ok()?,
            bindings: usize::try_from(counts.bindings).ok()?,
            materials: usize::try_from(counts.materials).ok()?,
        })
    }

    pub(in crate::canonical_scene::store) const fn image_media_counts(self) -> usize {
        self.image_media_counts
    }

    pub(in crate::canonical_scene::store) const fn bindings(self) -> usize {
        self.bindings
    }

    pub(in crate::canonical_scene::store) const fn materials(self) -> usize {
        self.materials
    }

    pub(in crate::canonical_scene::store) fn initialization_work_units(self) -> Option<u64> {
        [self.image_media_counts, self.bindings, self.materials]
            .into_iter()
            .try_fold(0_u64, |total, count| {
                total.checked_add(u64::try_from(count).ok()?)
            })
    }

    pub(in crate::canonical_scene::store) fn semantic_projection_work_units(self) -> Option<u64> {
        u64::try_from(self.bindings).ok()
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct AppearanceBindingValidationScratch {
    role: u8,
    coordinate_source: u8,
    coordinate_set: u32,
    channel_lane_count: u8,
    channels: [u8; 4],
    color_interpretation: u8,
    blend_operation: u8,
    semantic_values: [f32; APPEARANCE_BINDING_SEMANTIC_STRIDE],
    source_key_count: u32,
}

impl AppearanceBindingValidationScratch {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        role: 0,
        coordinate_source: 0,
        coordinate_set: 0,
        channel_lane_count: 0,
        channels: [0; 4],
        color_interpretation: 0,
        blend_operation: 0,
        semantic_values: [0.0; APPEARANCE_BINDING_SEMANTIC_STRIDE],
        source_key_count: 0,
    };
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct AppearanceMaterialValidationScratch {
    shading_model: u8,
    alpha_mode: u8,
    alpha_cutoff: f32,
    double_sided: u8,
    pbr_metallic_roughness_presence: u8,
    pbr_specular_glossiness_presence: u8,
    legacy_presence: u8,
    binding_offset: u32,
    binding_count: u32,
    name_presence: u8,
    name_offset: u32,
    name_count: u32,
}

impl AppearanceMaterialValidationScratch {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        shading_model: 0,
        alpha_mode: 0,
        alpha_cutoff: 0.0,
        double_sided: 0,
        pbr_metallic_roughness_presence: 0,
        pbr_specular_glossiness_presence: 0,
        legacy_presence: 0,
        binding_offset: 0,
        binding_count: 0,
        name_presence: 0,
        name_offset: 0,
        name_count: 0,
    };
}

/// Borrowed, reservation-backed scratch used only during typed appearance validation.
pub(in crate::canonical_scene::store) struct AppearanceValidationScratch<'scratch> {
    image_media_counts: &'scratch mut [u32],
    bindings: &'scratch mut [AppearanceBindingValidationScratch],
    materials: &'scratch mut [AppearanceMaterialValidationScratch],
}

type AppearanceScratchParts<'scratch> = (
    &'scratch mut [u32],
    &'scratch mut [AppearanceBindingValidationScratch],
    &'scratch mut [AppearanceMaterialValidationScratch],
);

impl<'scratch> AppearanceValidationScratch<'scratch> {
    pub(in crate::canonical_scene::store) const fn new(
        image_media_counts: &'scratch mut [u32],
        bindings: &'scratch mut [AppearanceBindingValidationScratch],
        materials: &'scratch mut [AppearanceMaterialValidationScratch],
    ) -> Self {
        Self {
            image_media_counts,
            bindings,
            materials,
        }
    }

    fn parts(
        &mut self,
        requirements: AppearanceValidationScratchRequirements,
    ) -> Option<AppearanceScratchParts<'_>> {
        if self.image_media_counts.len() != requirements.image_media_counts
            || self.bindings.len() != requirements.bindings
            || self.materials.len() != requirements.materials
        {
            return None;
        }
        Some((
            &mut *self.image_media_counts,
            &mut *self.bindings,
            &mut *self.materials,
        ))
    }
}

type AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U> =
    CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>;

#[derive(Clone, Copy)]
struct AppearanceDirectoryMembers<'session> {
    images: Option<CanonicalValidationMember<'session, RecordBlock, AppearanceImageTableSchema>>,
    samplers:
        Option<CanonicalValidationMember<'session, RecordBlock, AppearanceSamplerTableSchema>>,
    textures:
        Option<CanonicalValidationMember<'session, RecordBlock, AppearanceTextureTableSchema>>,
    materials:
        Option<CanonicalValidationMember<'session, RecordBlock, AppearanceMaterialTableSchema>>,
    bindings:
        Option<CanonicalValidationMember<'session, RecordBlock, AppearanceBindingTableSchema>>,
    image_payloads:
        Option<CanonicalValidationMember<'session, ByteBlock, AppearanceEncodedImagePayloadSchema>>,
    media_types: Option<CanonicalValidationMember<'session, Utf8Block, AppearanceMediaTypeSchema>>,
    source_keys: Option<CanonicalValidationMember<'session, Utf8Block, AppearanceSourceKeySchema>>,
    material_names:
        Option<CanonicalValidationMember<'session, Utf8Block, AppearanceMaterialNameSchema>>,
    binding_semantic_scalars: Option<
        CanonicalValidationMember<
            'session,
            ScalarSequence<f32>,
            AppearanceBindingSemanticScalarSchema,
        >,
    >,
    material_factor_scalars: Option<
        CanonicalValidationMember<
            'session,
            ScalarSequence<f32>,
            AppearanceMaterialFactorScalarSchema,
        >,
    >,
    image_payload_byte_count: usize,
    media_type_byte_count: usize,
    source_key_byte_count: usize,
    material_name_byte_count: usize,
    binding_semantic_scalar_count: usize,
    material_factor_scalar_count: usize,
}

impl<'session> AppearanceDirectoryMembers<'session> {
    const fn validated(
        self,
        counts: AppearanceCanonicalCounts,
    ) -> AppearanceValidationMembers<'session> {
        AppearanceValidationMembers {
            images: self.images,
            samplers: self.samplers,
            textures: self.textures,
            materials: self.materials,
            bindings: self.bindings,
            image_payloads: self.image_payloads,
            media_types: self.media_types,
            source_keys: self.source_keys,
            material_names: self.material_names,
            binding_semantic_scalars: self.binding_semantic_scalars,
            material_factor_scalars: self.material_factor_scalars,
            counts,
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
        name: Option<&str>,
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
        _name: Option<&str>,
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
                appearance_directory_field::MATERIAL_NAME_BYTE_COUNT,
                self.expected.material_name_byte_count(),
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
        name: Option<&str>,
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
        require_scalar(
            table.scalar::<u32>(row, appearance_material_field::NAME_OFFSET),
            material.name_offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, appearance_material_field::NAME_COUNT),
            material.name_count(),
        )?;
        let expected_values = material_factor_values(material.semantics().factors());
        if !f32_slices_have_same_bits(factor_values, &expected_values) {
            return None;
        }
        if range.count() == 0 {
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
        match (material.authored().name(), name) {
            (Presence::Absent, None) => require_scalar(
                table.scalar::<u8>(row, appearance_material_field::NAME_PRESENCE),
                APPEARANCE_PRESENCE_ABSENT,
            ),
            (Presence::Present(expected), Some(actual)) if actual == expected => require_scalar(
                table.scalar::<u8>(row, appearance_material_field::NAME_PRESENCE),
                APPEARANCE_PRESENCE_PRESENT,
            ),
            _ => None,
        }
    }
}

/// Validates the canonical appearance closure exclusively through typed, metered visits.
#[allow(clippy::too_many_arguments)]
pub(super) fn validate_appearance_canonical_structure_metered<
    'session,
    'storage,
    'control,
    'operation,
    O,
    PP,
    U,
>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    appearance: CanonicalValidationMember<'session, RecordBlock, AppearanceDirectorySchema>,
    counts: AppearanceCanonicalCounts,
    scratch: &mut AppearanceValidationScratch<'_>,
) -> Result<Option<AppearanceValidationMembers<'session>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(requirements) = AppearanceValidationScratchRequirements::from_counts(counts) else {
        return Ok(None);
    };
    let Some((image_media_counts, binding_scratch, material_scratch)) = scratch.parts(requirements)
    else {
        return Ok(None);
    };
    let Some(directory) = visit_appearance_directory(session, appearance, counts)? else {
        return Ok(None);
    };

    if !visit_appearance_images(session, directory, image_media_counts)?
        || !visit_appearance_samplers(session, directory, counts)?
        || !visit_appearance_textures(session, directory, counts)?
        || !visit_appearance_bindings(session, directory, counts, binding_scratch)?
        || !visit_appearance_materials(session, directory, counts, material_scratch)?
        || !visit_appearance_image_payloads(session, directory)?
        || !visit_appearance_media_types(session, directory, image_media_counts)?
        || !visit_appearance_binding_semantics(session, directory, binding_scratch)?
        || !visit_appearance_source_keys(session, directory, binding_scratch)?
        || !visit_appearance_material_names(session, directory, material_scratch)?
        || !visit_appearance_material_factors(session, directory, material_scratch)?
        || !validate_material_binding_authority_metered(session, binding_scratch, material_scratch)?
    {
        return Ok(None);
    }

    Ok(Some(directory.validated(counts)))
}

#[allow(clippy::too_many_lines)]
fn visit_appearance_directory<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    appearance: CanonicalValidationMember<'session, RecordBlock, AppearanceDirectorySchema>,
    counts: AppearanceCanonicalCounts,
) -> Result<Option<AppearanceDirectoryMembers<'session>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let (Ok(binding_stride), Ok(factor_stride)) = (
        u32::try_from(APPEARANCE_BINDING_SEMANTIC_STRIDE),
        u32::try_from(APPEARANCE_MATERIAL_FACTOR_STRIDE),
    ) else {
        return Ok(None);
    };
    let expected_binding_semantic_scalar_count = counts.bindings.checked_mul(binding_stride);
    let expected_material_factor_scalar_count = counts.materials.checked_mul(factor_stride);
    let (Some(expected_binding_semantic_scalar_count), Some(expected_material_factor_scalar_count)) = (
        expected_binding_semantic_scalar_count,
        expected_material_factor_scalar_count,
    ) else {
        return Ok(None);
    };

    let mut schema_revision = None;
    let mut image_count = None;
    let mut images = None;
    let mut images_absent = None;
    let mut sampler_count = None;
    let mut samplers = None;
    let mut samplers_absent = None;
    let mut texture_count = None;
    let mut textures = None;
    let mut textures_absent = None;
    let mut material_count = None;
    let mut materials = None;
    let mut materials_absent = None;
    let mut binding_count = None;
    let mut bindings = None;
    let mut bindings_absent = None;
    let mut image_payload_byte_count = None;
    let mut image_payloads = None;
    let mut image_payloads_absent = None;
    let mut media_type_byte_count = None;
    let mut media_types = None;
    let mut media_types_absent = None;
    let mut source_key_byte_count = None;
    let mut source_keys = None;
    let mut source_keys_absent = None;
    let mut material_name_byte_count = None;
    let mut material_names = None;
    let mut material_names_absent = None;
    let mut binding_semantic_scalar_count = None;
    let mut binding_semantic_scalars = None;
    let mut binding_semantic_scalars_absent = None;
    let mut material_factor_scalar_count = None;
    let mut material_factor_scalars = None;
    let mut material_factor_scalars_absent = None;

    let complete = session.visit_canonical_records(
        appearance,
        1,
        APPEARANCE_DIRECTORY_FIELD_COUNT,
        |_row_index, row| {
            schema_revision = row
                .scalar(appearance_directory_field::SCHEMA_REVISION, |value: u32| {
                    value
                })?;
            image_count =
                row.scalar(appearance_directory_field::IMAGE_COUNT, |value: u32| value)?;
            if image_count == Some(0) {
                images_absent =
                    row.scalar(appearance_directory_field::IMAGES, |value: u8| value)?;
            } else {
                images = row.record_ref::<AppearanceImageTableSchema, _>(
                    appearance_directory_field::IMAGES,
                    |value| value,
                )?;
            }
            sampler_count = row
                .scalar(appearance_directory_field::SAMPLER_COUNT, |value: u32| {
                    value
                })?;
            if sampler_count == Some(0) {
                samplers_absent =
                    row.scalar(appearance_directory_field::SAMPLERS, |value: u8| value)?;
            } else {
                samplers = row.record_ref::<AppearanceSamplerTableSchema, _>(
                    appearance_directory_field::SAMPLERS,
                    |value| value,
                )?;
            }
            texture_count = row
                .scalar(appearance_directory_field::TEXTURE_COUNT, |value: u32| {
                    value
                })?;
            if texture_count == Some(0) {
                textures_absent =
                    row.scalar(appearance_directory_field::TEXTURES, |value: u8| value)?;
            } else {
                textures = row.record_ref::<AppearanceTextureTableSchema, _>(
                    appearance_directory_field::TEXTURES,
                    |value| value,
                )?;
            }
            material_count = row
                .scalar(appearance_directory_field::MATERIAL_COUNT, |value: u32| {
                    value
                })?;
            if material_count == Some(0) {
                materials_absent =
                    row.scalar(appearance_directory_field::MATERIALS, |value: u8| value)?;
            } else {
                materials = row.record_ref::<AppearanceMaterialTableSchema, _>(
                    appearance_directory_field::MATERIALS,
                    |value| value,
                )?;
            }
            binding_count = row
                .scalar(appearance_directory_field::BINDING_COUNT, |value: u32| {
                    value
                })?;
            if binding_count == Some(0) {
                bindings_absent =
                    row.scalar(appearance_directory_field::BINDINGS, |value: u8| value)?;
            } else {
                bindings = row.record_ref::<AppearanceBindingTableSchema, _>(
                    appearance_directory_field::BINDINGS,
                    |value| value,
                )?;
            }
            image_payload_byte_count = row.scalar(
                appearance_directory_field::IMAGE_PAYLOAD_BYTE_COUNT,
                |value: u32| value,
            )?;
            if image_payload_byte_count == Some(0) {
                image_payloads_absent = row
                    .scalar(appearance_directory_field::IMAGE_PAYLOADS, |value: u8| {
                        value
                    })?;
            } else {
                image_payloads = row.bytes_ref::<AppearanceEncodedImagePayloadSchema, _>(
                    appearance_directory_field::IMAGE_PAYLOADS,
                    |value| value,
                )?;
            }
            media_type_byte_count = row.scalar(
                appearance_directory_field::MEDIA_TYPE_BYTE_COUNT,
                |value: u32| value,
            )?;
            if media_type_byte_count == Some(0) {
                media_types_absent =
                    row.scalar(appearance_directory_field::MEDIA_TYPES, |value: u8| value)?;
            } else {
                media_types = row.utf8_ref::<AppearanceMediaTypeSchema, _>(
                    appearance_directory_field::MEDIA_TYPES,
                    |value| value,
                )?;
            }
            source_key_byte_count = row.scalar(
                appearance_directory_field::SOURCE_KEY_BYTE_COUNT,
                |value: u32| value,
            )?;
            if source_key_byte_count == Some(0) {
                source_keys_absent =
                    row.scalar(appearance_directory_field::SOURCE_KEYS, |value: u8| value)?;
            } else {
                source_keys = row.utf8_ref::<AppearanceSourceKeySchema, _>(
                    appearance_directory_field::SOURCE_KEYS,
                    |value| value,
                )?;
            }
            material_name_byte_count = row.scalar(
                appearance_directory_field::MATERIAL_NAME_BYTE_COUNT,
                |value: u32| value,
            )?;
            if material_name_byte_count == Some(0) {
                material_names_absent = row
                    .scalar(appearance_directory_field::MATERIAL_NAMES, |value: u8| {
                        value
                    })?;
            } else {
                material_names = row.utf8_ref::<AppearanceMaterialNameSchema, _>(
                    appearance_directory_field::MATERIAL_NAMES,
                    |value| value,
                )?;
            }
            binding_semantic_scalar_count = row.scalar(
                appearance_directory_field::BINDING_SEMANTIC_SCALAR_COUNT,
                |value: u32| value,
            )?;
            if binding_semantic_scalar_count == Some(0) {
                binding_semantic_scalars_absent = row.scalar(
                    appearance_directory_field::BINDING_SEMANTIC_SCALARS,
                    |value: u8| value,
                )?;
            } else {
                binding_semantic_scalars = row
                    .sequence_ref::<f32, AppearanceBindingSemanticScalarSchema, _>(
                        appearance_directory_field::BINDING_SEMANTIC_SCALARS,
                        |value| value,
                    )?;
            }
            material_factor_scalar_count = row.scalar(
                appearance_directory_field::MATERIAL_FACTOR_SCALAR_COUNT,
                |value: u32| value,
            )?;
            if material_factor_scalar_count == Some(0) {
                material_factor_scalars_absent = row.scalar(
                    appearance_directory_field::MATERIAL_FACTOR_SCALARS,
                    |value: u8| value,
                )?;
            } else {
                material_factor_scalars = row
                    .sequence_ref::<f32, AppearanceMaterialFactorScalarSchema, _>(
                        appearance_directory_field::MATERIAL_FACTOR_SCALARS,
                        |value| value,
                    )?;
            }
            Ok(true)
        },
    )?;

    if !complete
        || schema_revision != Some(APPEARANCE_SCHEMA_REVISION)
        || !optional_directory_member_valid(image_count, counts.images, images, images_absent)
        || !optional_directory_member_valid(
            sampler_count,
            counts.samplers,
            samplers,
            samplers_absent,
        )
        || !optional_directory_member_valid(
            texture_count,
            counts.textures,
            textures,
            textures_absent,
        )
        || !optional_directory_member_valid(
            material_count,
            counts.materials,
            materials,
            materials_absent,
        )
        || !optional_directory_member_valid(
            binding_count,
            counts.bindings,
            bindings,
            bindings_absent,
        )
        || !optional_payload_member_valid(
            image_payload_byte_count,
            image_payloads,
            image_payloads_absent,
        )
        || !optional_payload_member_valid(media_type_byte_count, media_types, media_types_absent)
        || !optional_payload_member_valid(source_key_byte_count, source_keys, source_keys_absent)
        || !optional_payload_member_valid(
            material_name_byte_count,
            material_names,
            material_names_absent,
        )
        || !optional_directory_member_valid(
            binding_semantic_scalar_count,
            expected_binding_semantic_scalar_count,
            binding_semantic_scalars,
            binding_semantic_scalars_absent,
        )
        || !optional_directory_member_valid(
            material_factor_scalar_count,
            expected_material_factor_scalar_count,
            material_factor_scalars,
            material_factor_scalars_absent,
        )
    {
        return Ok(None);
    }

    let (
        Some(image_payload_byte_count),
        Some(media_type_byte_count),
        Some(source_key_byte_count),
        Some(material_name_byte_count),
        Some(binding_semantic_scalar_count),
        Some(material_factor_scalar_count),
    ) = (
        image_payload_byte_count.and_then(|count| usize::try_from(count).ok()),
        media_type_byte_count.and_then(|count| usize::try_from(count).ok()),
        source_key_byte_count.and_then(|count| usize::try_from(count).ok()),
        material_name_byte_count.and_then(|count| usize::try_from(count).ok()),
        binding_semantic_scalar_count.and_then(|count| usize::try_from(count).ok()),
        material_factor_scalar_count.and_then(|count| usize::try_from(count).ok()),
    )
    else {
        return Ok(None);
    };

    Ok(Some(AppearanceDirectoryMembers {
        images,
        samplers,
        textures,
        materials,
        bindings,
        image_payloads,
        media_types,
        source_keys,
        material_names,
        binding_semantic_scalars,
        material_factor_scalars,
        image_payload_byte_count,
        media_type_byte_count,
        source_key_byte_count,
        material_name_byte_count,
        binding_semantic_scalar_count,
        material_factor_scalar_count,
    }))
}

fn optional_directory_member_valid<T: Copy>(
    actual_count: Option<u32>,
    expected_count: u32,
    member: Option<T>,
    absent: Option<u8>,
) -> bool {
    actual_count == Some(expected_count)
        && if expected_count == 0 {
            member.is_none() && absent == Some(0)
        } else {
            member.is_some() && absent.is_none()
        }
}

fn optional_payload_member_valid<T: Copy>(
    count: Option<u32>,
    member: Option<T>,
    absent: Option<u8>,
) -> bool {
    match count {
        Some(0) => member.is_none() && absent == Some(0),
        Some(_) => member.is_some() && absent.is_none(),
        None => false,
    }
}

fn visit_appearance_images<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: AppearanceDirectoryMembers<'session>,
    image_media_counts: &mut [u32],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(images) = directory.images else {
        return Ok(image_media_counts.is_empty()
            && directory.image_payloads.is_none()
            && directory.media_types.is_none()
            && directory.image_payload_byte_count == 0
            && directory.media_type_byte_count == 0);
    };
    let Some(payloads) = directory.image_payloads else {
        return Ok(false);
    };
    let mut payload_cursor = 0_usize;
    let mut media_cursor = 0_usize;
    let complete = session.visit_canonical_records(
        images,
        image_media_counts.len(),
        APPEARANCE_IMAGE_FIELD_COUNT,
        |row_index, row| {
            let image = row.scalar(appearance_image_field::IMAGE, |value: u32| value)?;
            let payload_kind =
                row.scalar(appearance_image_field::PAYLOAD_KIND, |value: u8| value)?;
            let payload_offset =
                row.scalar(appearance_image_field::PAYLOAD_OFFSET, |value: u32| value)?;
            let payload_count =
                row.scalar(appearance_image_field::PAYLOAD_COUNT, |value: u32| value)?;
            let mut payload_ref = None;
            let mut payload_absent = None;
            if payload_count == Some(0) {
                payload_absent = row.scalar(appearance_image_field::PAYLOADS, |value: u8| value)?;
            } else {
                payload_ref = row.bytes_ref::<AppearanceEncodedImagePayloadSchema, _>(
                    appearance_image_field::PAYLOADS,
                    |value| value,
                )?;
            }
            let media_presence = row
                .scalar(appearance_image_field::MEDIA_TYPE_PRESENCE, |value: u8| {
                    value
                })?;
            let media_encoding = row
                .scalar(appearance_image_field::MEDIA_TYPE_ENCODING, |value: u8| {
                    value
                })?;
            let media_offset = row
                .scalar(appearance_image_field::MEDIA_TYPE_OFFSET, |value: u32| {
                    value
                })?;
            let media_count =
                row.scalar(appearance_image_field::MEDIA_TYPE_COUNT, |value: u32| value)?;
            let mut media_ref = None;
            let mut media_absent = None;
            if media_count == Some(0) {
                media_absent =
                    row.scalar(appearance_image_field::MEDIA_TYPES, |value: u8| value)?;
            } else {
                media_ref = row.utf8_ref::<AppearanceMediaTypeSchema, _>(
                    appearance_image_field::MEDIA_TYPES,
                    |value| value,
                )?;
            }

            let Some(payload_count) = payload_count.and_then(|count| usize::try_from(count).ok())
            else {
                return Ok(false);
            };
            let Some(media_count_usize) = media_count.and_then(|count| usize::try_from(count).ok())
            else {
                return Ok(false);
            };
            let Some(next_payload_cursor) = payload_cursor.checked_add(payload_count) else {
                return Ok(false);
            };
            let Some(next_media_cursor) = media_cursor.checked_add(media_count_usize) else {
                return Ok(false);
            };
            let valid = image == u32::try_from(row_index).ok()
                && payload_kind == Some(APPEARANCE_IMAGE_PAYLOAD_ENCODED)
                && payload_offset == u32::try_from(payload_cursor).ok()
                && payload_count != 0
                && payload_ref == Some(payloads)
                && payload_absent.is_none()
                && media_encoding == Some(APPEARANCE_MEDIA_TYPE_UTF8)
                && media_offset == u32::try_from(media_cursor).ok()
                && match media_count {
                    Some(0) => {
                        media_presence == Some(APPEARANCE_PRESENCE_ABSENT)
                            && media_ref.is_none()
                            && media_absent == Some(0)
                    }
                    Some(_) => {
                        media_presence == Some(APPEARANCE_PRESENCE_PRESENT)
                            && media_ref == directory.media_types
                            && media_ref.is_some()
                            && media_absent.is_none()
                    }
                    None => false,
                };
            let Some(slot) = image_media_counts.get_mut(row_index) else {
                return Ok(false);
            };
            *slot = media_count.unwrap_or(0);
            payload_cursor = next_payload_cursor;
            media_cursor = next_media_cursor;
            Ok(valid)
        },
    )?;
    Ok(complete
        && payload_cursor == directory.image_payload_byte_count
        && media_cursor == directory.media_type_byte_count)
}

fn visit_appearance_samplers<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: AppearanceDirectoryMembers<'session>,
    counts: AppearanceCanonicalCounts,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(samplers) = directory.samplers else {
        return Ok(counts.samplers == 0);
    };
    let Some(expected_rows) = usize::try_from(counts.samplers).ok() else {
        return Ok(false);
    };
    session.visit_canonical_records(
        samplers,
        expected_rows,
        APPEARANCE_SAMPLER_FIELD_COUNT,
        |row_index, row| {
            let sampler = row.scalar(appearance_sampler_field::SAMPLER, |value: u32| value)?;
            let minification =
                row.scalar(appearance_sampler_field::MINIFICATION, |value: u8| value)?;
            let magnification =
                row.scalar(appearance_sampler_field::MAGNIFICATION, |value: u8| value)?;
            let mipmap = row.scalar(appearance_sampler_field::MIPMAP, |value: u8| value)?;
            let address_u = row.scalar(appearance_sampler_field::ADDRESS_U, |value: u8| value)?;
            let address_v = row.scalar(appearance_sampler_field::ADDRESS_V, |value: u8| value)?;
            let address_w = row.scalar(appearance_sampler_field::ADDRESS_W, |value: u8| value)?;
            Ok(sampler == u32::try_from(row_index).ok()
                && minification.is_some_and(|tag| tag <= APPEARANCE_TEXEL_FILTER_LINEAR)
                && magnification.is_some_and(|tag| tag <= APPEARANCE_TEXEL_FILTER_LINEAR)
                && mipmap.is_some_and(|tag| tag <= APPEARANCE_MIPMAP_FILTER_LINEAR)
                && address_u.is_some_and(|tag| tag <= APPEARANCE_ADDRESS_CLAMP_TO_BORDER)
                && address_v.is_some_and(|tag| tag <= APPEARANCE_ADDRESS_CLAMP_TO_BORDER)
                && address_w.is_some_and(|tag| tag <= APPEARANCE_ADDRESS_CLAMP_TO_BORDER))
        },
    )
}

fn visit_appearance_textures<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: AppearanceDirectoryMembers<'session>,
    counts: AppearanceCanonicalCounts,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(textures) = directory.textures else {
        return Ok(counts.textures == 0);
    };
    let (Some(images), Some(samplers), Some(expected_rows)) = (
        directory.images,
        directory.samplers,
        usize::try_from(counts.textures).ok(),
    ) else {
        return Ok(false);
    };
    session.visit_canonical_records(
        textures,
        expected_rows,
        APPEARANCE_TEXTURE_FIELD_COUNT,
        |row_index, row| {
            let texture = row.scalar(appearance_texture_field::TEXTURE, |value: u32| value)?;
            let image = row.scalar(appearance_texture_field::IMAGE, |value: u32| value)?;
            let image_ref = row.record_ref::<AppearanceImageTableSchema, _>(
                appearance_texture_field::IMAGES,
                |value| value,
            )?;
            let sampler = row.scalar(appearance_texture_field::SAMPLER, |value: u32| value)?;
            let sampler_ref = row.record_ref::<AppearanceSamplerTableSchema, _>(
                appearance_texture_field::SAMPLERS,
                |value| value,
            )?;
            Ok(texture == u32::try_from(row_index).ok()
                && image.is_some_and(|index| index < counts.images)
                && image_ref == Some(images)
                && sampler.is_some_and(|index| index < counts.samplers)
                && sampler_ref == Some(samplers))
        },
    )
}

#[allow(clippy::too_many_lines)]
fn visit_appearance_bindings<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: AppearanceDirectoryMembers<'session>,
    counts: AppearanceCanonicalCounts,
    scratch: &mut [AppearanceBindingValidationScratch],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(bindings) = directory.bindings else {
        return Ok(scratch.is_empty()
            && counts.bindings == 0
            && directory.binding_semantic_scalars.is_none()
            && directory.source_keys.is_none());
    };
    let (Some(textures), Some(semantic_scalars)) =
        (directory.textures, directory.binding_semantic_scalars)
    else {
        return Ok(false);
    };
    let Ok(semantic_stride) = u32::try_from(APPEARANCE_BINDING_SEMANTIC_STRIDE) else {
        return Ok(false);
    };
    let mut semantic_cursor = 0_usize;
    let mut source_key_cursor = 0_usize;
    let complete = session.visit_canonical_records(
        bindings,
        scratch.len(),
        APPEARANCE_BINDING_FIELD_COUNT,
        |row_index, row| {
            let binding = row.scalar(appearance_binding_field::BINDING, |value: u32| value)?;
            let texture = row.scalar(appearance_binding_field::TEXTURE, |value: u32| value)?;
            let textures_ref = row.record_ref::<AppearanceTextureTableSchema, _>(
                appearance_binding_field::TEXTURES,
                |value| value,
            )?;
            let role = row.scalar(appearance_binding_field::ROLE, |value: u8| value)?;
            let coordinate_source = row
                .scalar(appearance_binding_field::COORDINATE_SOURCE, |value: u8| {
                    value
                })?;
            let coordinate_set =
                row.scalar(appearance_binding_field::COORDINATE_SET, |value: u32| value)?;
            let channel_lane_count = row
                .scalar(appearance_binding_field::CHANNEL_LANE_COUNT, |value: u8| {
                    value
                })?;
            let channel_0 = row.scalar(appearance_binding_field::CHANNEL_0, |value: u8| value)?;
            let channel_1 = row.scalar(appearance_binding_field::CHANNEL_1, |value: u8| value)?;
            let channel_2 = row.scalar(appearance_binding_field::CHANNEL_2, |value: u8| value)?;
            let channel_3 = row.scalar(appearance_binding_field::CHANNEL_3, |value: u8| value)?;
            let color_interpretation = row.scalar(
                appearance_binding_field::COLOR_INTERPRETATION,
                |value: u8| value,
            )?;
            let semantic_offset = row.scalar(
                appearance_binding_field::SEMANTIC_SCALAR_OFFSET,
                |value: u32| value,
            )?;
            let semantic_count = row.scalar(
                appearance_binding_field::SEMANTIC_SCALAR_COUNT,
                |value: u32| value,
            )?;
            let mut semantic_ref = None;
            let mut semantic_absent = None;
            if semantic_count == Some(0) {
                semantic_absent = row
                    .scalar(appearance_binding_field::SEMANTIC_SCALARS, |value: u8| {
                        value
                    })?;
            } else {
                semantic_ref = row.sequence_ref::<f32, AppearanceBindingSemanticScalarSchema, _>(
                    appearance_binding_field::SEMANTIC_SCALARS,
                    |value| value,
                )?;
            }
            let blend_operation =
                row.scalar(appearance_binding_field::BLEND_OPERATION, |value: u8| value)?;
            let source_key_offset = row
                .scalar(appearance_binding_field::SOURCE_KEY_OFFSET, |value: u32| {
                    value
                })?;
            let source_key_count = row
                .scalar(appearance_binding_field::SOURCE_KEY_COUNT, |value: u32| {
                    value
                })?;
            let mut source_key_ref = None;
            let mut source_key_absent = None;
            if source_key_count == Some(0) {
                source_key_absent =
                    row.scalar(appearance_binding_field::SOURCE_KEYS, |value: u8| value)?;
            } else {
                source_key_ref = row.utf8_ref::<AppearanceSourceKeySchema, _>(
                    appearance_binding_field::SOURCE_KEYS,
                    |value| value,
                )?;
            }

            let channels = [
                channel_0.unwrap_or(0),
                channel_1.unwrap_or(0),
                channel_2.unwrap_or(0),
                channel_3.unwrap_or(0),
            ];
            let Some(semantic_count_usize) =
                semantic_count.and_then(|count| usize::try_from(count).ok())
            else {
                return Ok(false);
            };
            let Some(source_key_count_usize) =
                source_key_count.and_then(|count| usize::try_from(count).ok())
            else {
                return Ok(false);
            };
            let Some(next_semantic_cursor) = semantic_cursor.checked_add(semantic_count_usize)
            else {
                return Ok(false);
            };
            let Some(next_source_key_cursor) =
                source_key_cursor.checked_add(source_key_count_usize)
            else {
                return Ok(false);
            };
            let Some(slot) = scratch.get_mut(row_index) else {
                return Ok(false);
            };
            *slot = AppearanceBindingValidationScratch {
                role: role.unwrap_or(0),
                coordinate_source: coordinate_source.unwrap_or(0),
                coordinate_set: coordinate_set.unwrap_or(0),
                channel_lane_count: channel_lane_count.unwrap_or(0),
                channels,
                color_interpretation: color_interpretation.unwrap_or(0),
                blend_operation: blend_operation.unwrap_or(0),
                semantic_values: [0.0; APPEARANCE_BINDING_SEMANTIC_STRIDE],
                source_key_count: source_key_count.unwrap_or(0),
            };
            let source_key_valid = match source_key_count {
                Some(0) => source_key_ref.is_none() && source_key_absent == Some(0),
                Some(_) => {
                    source_key_ref == directory.source_keys
                        && source_key_ref.is_some()
                        && source_key_absent.is_none()
                }
                None => false,
            };
            let valid = binding == u32::try_from(row_index).ok()
                && texture.is_some_and(|index| index < counts.textures)
                && textures_ref == Some(textures)
                && role.and_then(decode_appearance_texture_role).is_some()
                && coordinate_source
                    .zip(coordinate_set)
                    .and_then(|(tag, set)| decode_appearance_coordinate_source(tag, set))
                    .is_some()
                && channel_lane_count
                    .and_then(|lane_count| decode_channel_mapping(lane_count, channels))
                    .is_some()
                && color_interpretation
                    .and_then(decode_appearance_color_interpretation)
                    .is_some()
                && blend_operation
                    .and_then(decode_appearance_blend_operation)
                    .is_some()
                && semantic_offset == u32::try_from(semantic_cursor).ok()
                && semantic_count == Some(semantic_stride)
                && semantic_ref == Some(semantic_scalars)
                && semantic_absent.is_none()
                && source_key_offset == u32::try_from(source_key_cursor).ok()
                && source_key_valid;
            semantic_cursor = next_semantic_cursor;
            source_key_cursor = next_source_key_cursor;
            Ok(valid)
        },
    )?;
    Ok(complete
        && semantic_cursor == directory.binding_semantic_scalar_count
        && source_key_cursor == directory.source_key_byte_count)
}

#[allow(clippy::too_many_lines)]
fn visit_appearance_materials<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: AppearanceDirectoryMembers<'session>,
    counts: AppearanceCanonicalCounts,
    scratch: &mut [AppearanceMaterialValidationScratch],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(materials) = directory.materials else {
        return Ok(scratch.is_empty()
            && counts.materials == 0
            && counts.bindings == 0
            && directory.material_factor_scalars.is_none()
            && directory.material_names.is_none()
            && directory.material_name_byte_count == 0);
    };
    let Some(factor_scalars) = directory.material_factor_scalars else {
        return Ok(false);
    };
    let (Ok(factor_stride), Ok(expected_binding_rows)) = (
        u32::try_from(APPEARANCE_MATERIAL_FACTOR_STRIDE),
        usize::try_from(counts.bindings),
    ) else {
        return Ok(false);
    };
    let mut binding_cursor = 0_usize;
    let mut factor_cursor = 0_usize;
    let mut name_cursor = 0_usize;
    let complete = session.visit_canonical_records(
        materials,
        scratch.len(),
        APPEARANCE_MATERIAL_FIELD_COUNT,
        |row_index, row| {
            let material = row.scalar(appearance_material_field::MATERIAL, |value: u32| value)?;
            let shading_model =
                row.scalar(appearance_material_field::SHADING_MODEL, |value: u8| value)?;
            let alpha_mode =
                row.scalar(appearance_material_field::ALPHA_MODE, |value: u8| value)?;
            let alpha_cutoff =
                row.scalar(appearance_material_field::ALPHA_CUTOFF, |value: f32| value)?;
            let double_sided =
                row.scalar(appearance_material_field::DOUBLE_SIDED, |value: u8| value)?;
            let pbr_metallic_roughness_presence = row.scalar(
                appearance_material_field::PBR_METALLIC_ROUGHNESS_PRESENCE,
                |value: u8| value,
            )?;
            let pbr_specular_glossiness_presence = row.scalar(
                appearance_material_field::PBR_SPECULAR_GLOSSINESS_PRESENCE,
                |value: u8| value,
            )?;
            let legacy_presence = row
                .scalar(appearance_material_field::LEGACY_PRESENCE, |value: u8| {
                    value
                })?;
            let binding_offset = row
                .scalar(appearance_material_field::BINDING_OFFSET, |value: u32| {
                    value
                })?;
            let binding_count =
                row.scalar(appearance_material_field::BINDING_COUNT, |value: u32| value)?;
            let mut bindings_ref = None;
            let mut bindings_absent = None;
            if binding_count == Some(0) {
                bindings_absent =
                    row.scalar(appearance_material_field::BINDINGS, |value: u8| value)?;
            } else {
                bindings_ref = row.record_ref::<AppearanceBindingTableSchema, _>(
                    appearance_material_field::BINDINGS,
                    |value| value,
                )?;
            }
            let factor_offset =
                row.scalar(appearance_material_field::FACTOR_OFFSET, |value: u32| value)?;
            let factor_count =
                row.scalar(appearance_material_field::FACTOR_COUNT, |value: u32| value)?;
            let mut factors_ref = None;
            let mut factors_absent = None;
            if factor_count == Some(0) {
                factors_absent =
                    row.scalar(appearance_material_field::FACTORS, |value: u8| value)?;
            } else {
                factors_ref = row.sequence_ref::<f32, AppearanceMaterialFactorScalarSchema, _>(
                    appearance_material_field::FACTORS,
                    |value| value,
                )?;
            }
            let name_presence =
                row.scalar(appearance_material_field::NAME_PRESENCE, |value: u8| value)?;
            let name_offset =
                row.scalar(appearance_material_field::NAME_OFFSET, |value: u32| value)?;
            let name_count =
                row.scalar(appearance_material_field::NAME_COUNT, |value: u32| value)?;
            let mut names_ref = None;
            let mut names_absent = None;
            if name_count == Some(0) {
                names_absent = row.scalar(appearance_material_field::NAMES, |value: u8| value)?;
            } else {
                names_ref = row.utf8_ref::<AppearanceMaterialNameSchema, _>(
                    appearance_material_field::NAMES,
                    |value| value,
                )?;
            }

            let Some(binding_count_usize) =
                binding_count.and_then(|count| usize::try_from(count).ok())
            else {
                return Ok(false);
            };
            let Some(factor_count_usize) =
                factor_count.and_then(|count| usize::try_from(count).ok())
            else {
                return Ok(false);
            };
            let Some(next_binding_cursor) = binding_cursor.checked_add(binding_count_usize) else {
                return Ok(false);
            };
            let Some(next_factor_cursor) = factor_cursor.checked_add(factor_count_usize) else {
                return Ok(false);
            };
            let Some(name_count_usize) = name_count.and_then(|count| usize::try_from(count).ok())
            else {
                return Ok(false);
            };
            let Some(next_name_cursor) = name_cursor.checked_add(name_count_usize) else {
                return Ok(false);
            };
            let Some(slot) = scratch.get_mut(row_index) else {
                return Ok(false);
            };
            *slot = AppearanceMaterialValidationScratch {
                shading_model: shading_model.unwrap_or(0),
                alpha_mode: alpha_mode.unwrap_or(0),
                alpha_cutoff: alpha_cutoff.unwrap_or(0.0),
                double_sided: double_sided.unwrap_or(0),
                pbr_metallic_roughness_presence: pbr_metallic_roughness_presence.unwrap_or(0),
                pbr_specular_glossiness_presence: pbr_specular_glossiness_presence.unwrap_or(0),
                legacy_presence: legacy_presence.unwrap_or(0),
                binding_offset: binding_offset.unwrap_or(0),
                binding_count: binding_count.unwrap_or(0),
                name_presence: name_presence.unwrap_or(0),
                name_offset: name_offset.unwrap_or(0),
                name_count: name_count.unwrap_or(0),
            };
            let binding_ref_valid = match binding_count {
                Some(0) => bindings_ref.is_none() && bindings_absent == Some(0),
                Some(_) => {
                    bindings_ref == directory.bindings
                        && bindings_ref.is_some()
                        && bindings_absent.is_none()
                }
                None => false,
            };
            let name_valid = match (name_presence, name_count) {
                (Some(APPEARANCE_PRESENCE_ABSENT), Some(0)) => {
                    names_ref.is_none() && names_absent == Some(0)
                }
                (Some(APPEARANCE_PRESENCE_PRESENT), Some(0)) => {
                    names_ref.is_none() && names_absent == Some(0)
                }
                (Some(APPEARANCE_PRESENCE_PRESENT), Some(_)) => {
                    names_ref == directory.material_names
                        && names_ref.is_some()
                        && names_absent.is_none()
                }
                _ => false,
            };
            let valid = material == u32::try_from(row_index).ok()
                && shading_model
                    .and_then(decode_appearance_shading_model)
                    .is_some()
                && alpha_mode
                    .zip(alpha_cutoff)
                    .and_then(|(mode, cutoff)| decode_appearance_alpha_mode(mode, cutoff))
                    .is_some()
                && double_sided.and_then(decode_presence).is_some()
                && pbr_metallic_roughness_presence
                    .and_then(decode_presence)
                    .is_some()
                && pbr_specular_glossiness_presence
                    .and_then(decode_presence)
                    .is_some()
                && legacy_presence.and_then(decode_presence).is_some()
                && binding_offset == u32::try_from(binding_cursor).ok()
                && next_binding_cursor <= expected_binding_rows
                && binding_ref_valid
                && factor_offset == u32::try_from(factor_cursor).ok()
                && factor_count == Some(factor_stride)
                && factors_ref == Some(factor_scalars)
                && factors_absent.is_none()
                && name_offset == u32::try_from(name_cursor).ok()
                && name_valid;
            binding_cursor = next_binding_cursor;
            factor_cursor = next_factor_cursor;
            name_cursor = next_name_cursor;
            Ok(valid)
        },
    )?;
    Ok(complete
        && binding_cursor == expected_binding_rows
        && factor_cursor == directory.material_factor_scalar_count
        && name_cursor == directory.material_name_byte_count)
}

fn visit_appearance_image_payloads<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: AppearanceDirectoryMembers<'session>,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match directory.image_payloads {
        Some(payloads) => session.visit_canonical_bytes(
            payloads,
            directory.image_payload_byte_count,
            |_index, _byte| true,
        ),
        None => Ok(directory.image_payload_byte_count == 0),
    }
}

fn visit_appearance_media_types<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: AppearanceDirectoryMembers<'session>,
    image_media_counts: &[u32],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(media_types) = directory.media_types else {
        return Ok(directory.media_type_byte_count == 0
            && image_media_counts.iter().all(|&count| count == 0));
    };
    let mut image_index = 0_usize;
    let mut segment_len = 0_usize;
    let mut segment = [0_u8; MAX_EXTENSION_MEDIA_TYPE_BYTES];
    let complete = session.visit_canonical_utf8_bytes(
        media_types,
        directory.media_type_byte_count,
        |_byte_index, byte| {
            while image_media_counts.get(image_index) == Some(&0) {
                image_index += 1;
            }
            let Some(segment_count) = image_media_counts
                .get(image_index)
                .and_then(|&count| usize::try_from(count).ok())
            else {
                return false;
            };
            if segment_count > segment.len() || segment_len >= segment_count {
                return false;
            }
            segment[segment_len] = byte;
            segment_len += 1;
            if segment_len != segment_count {
                return true;
            }
            let valid = std::str::from_utf8(&segment[..segment_len])
                .ok()
                .and_then(|value| ExtensionMediaType::new(value).ok())
                .is_some();
            image_index += 1;
            segment_len = 0;
            valid
        },
    )?;
    while image_media_counts.get(image_index) == Some(&0) {
        image_index += 1;
    }
    Ok(complete && segment_len == 0 && image_index == image_media_counts.len())
}

fn visit_appearance_binding_semantics<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: AppearanceDirectoryMembers<'session>,
    scratch: &mut [AppearanceBindingValidationScratch],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(semantic_scalars) = directory.binding_semantic_scalars else {
        return Ok(directory.binding_semantic_scalar_count == 0 && scratch.is_empty());
    };
    let complete = session.visit_canonical_sequence(
        semantic_scalars,
        directory.binding_semantic_scalar_count,
        |index, value| {
            let binding_index = index / APPEARANCE_BINDING_SEMANTIC_STRIDE;
            let scalar_index = index % APPEARANCE_BINDING_SEMANTIC_STRIDE;
            let Some(binding) = scratch.get_mut(binding_index) else {
                return false;
            };
            binding.semantic_values[scalar_index] = value;
            scalar_index + 1 != APPEARANCE_BINDING_SEMANTIC_STRIDE
                || binding.source_key_count != 0
                || validate_binding_scratch(binding, None)
        },
    )?;
    Ok(complete)
}

fn visit_appearance_source_keys<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: AppearanceDirectoryMembers<'session>,
    scratch: &[AppearanceBindingValidationScratch],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(source_keys) = directory.source_keys else {
        return Ok(directory.source_key_byte_count == 0
            && scratch.iter().all(|binding| binding.source_key_count == 0));
    };
    let mut binding_index = 0_usize;
    let mut segment_len = 0_usize;
    let mut segment = [0_u8; MAX_TEXTURE_SOURCE_KEY_BYTES];
    let complete = session.visit_canonical_utf8_bytes(
        source_keys,
        directory.source_key_byte_count,
        |_byte_index, byte| {
            while scratch
                .get(binding_index)
                .is_some_and(|binding| binding.source_key_count == 0)
            {
                binding_index += 1;
            }
            let Some((binding, segment_count)) = scratch.get(binding_index).and_then(|binding| {
                usize::try_from(binding.source_key_count)
                    .ok()
                    .map(|count| (binding, count))
            }) else {
                return false;
            };
            if segment_count > segment.len() || segment_len >= segment_count {
                return false;
            }
            segment[segment_len] = byte;
            segment_len += 1;
            if segment_len != segment_count {
                return true;
            }
            let source_key = std::str::from_utf8(&segment[..segment_len])
                .ok()
                .and_then(|value| TextureSourceKey::try_new(value).ok());
            let valid = source_key
                .as_ref()
                .is_some_and(|source_key| validate_binding_scratch(binding, Some(source_key)));
            binding_index += 1;
            segment_len = 0;
            valid
        },
    )?;
    while scratch
        .get(binding_index)
        .is_some_and(|binding| binding.source_key_count == 0)
    {
        binding_index += 1;
    }
    Ok(complete && segment_len == 0 && binding_index == scratch.len())
}

fn validate_binding_scratch(
    binding: &AppearanceBindingValidationScratch,
    source_key: Option<&TextureSourceKey>,
) -> bool {
    let Some(role) = decode_appearance_texture_role(binding.role) else {
        return false;
    };
    let Some(coordinate_source) =
        decode_appearance_coordinate_source(binding.coordinate_source, binding.coordinate_set)
    else {
        return false;
    };
    let Some(channel_mapping) =
        decode_channel_mapping(binding.channel_lane_count, binding.channels)
    else {
        return false;
    };
    let Some(color_interpretation) =
        decode_appearance_color_interpretation(binding.color_interpretation)
    else {
        return false;
    };
    let Some(blend) = decode_appearance_blend_operation(binding.blend_operation) else {
        return false;
    };
    let Some((transform, strength)) =
        decode_appearance_binding_numeric_semantics(&binding.semantic_values)
    else {
        return false;
    };
    validate_texture_binding_semantics(TextureBindingSemanticsRef::new(
        &role,
        &coordinate_source,
        &channel_mapping,
        &color_interpretation,
        transform,
        strength,
        &blend,
        source_key,
    ))
    .is_ok()
}

fn visit_appearance_material_names<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: AppearanceDirectoryMembers<'session>,
    materials: &[AppearanceMaterialValidationScratch],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(names) = directory.material_names else {
        return Ok(directory.material_name_byte_count == 0
            && materials.iter().all(|material| {
                material.name_count == 0
                    && matches!(
                        material.name_presence,
                        APPEARANCE_PRESENCE_ABSENT | APPEARANCE_PRESENCE_PRESENT
                    )
                    && (material.name_presence != APPEARANCE_PRESENCE_PRESENT
                        || validate_entity_name_str("").is_ok())
            }));
    };
    let mut material_index = 0_usize;
    let mut segment_len = 0_usize;
    let mut segment = [0_u8; MAX_ENTITY_NAME_BYTES];
    let complete = session.visit_canonical_utf8_bytes(
        names,
        directory.material_name_byte_count,
        |_byte_index, byte| {
            while let Some(material) = materials.get(material_index) {
                if material.name_count != 0 {
                    break;
                }
                if !matches!(
                    material.name_presence,
                    APPEARANCE_PRESENCE_ABSENT | APPEARANCE_PRESENCE_PRESENT
                ) || (material.name_presence == APPEARANCE_PRESENCE_PRESENT
                    && validate_entity_name_str("").is_err())
                {
                    return false;
                }
                material_index += 1;
            }
            let Some(material) = materials.get(material_index) else {
                return false;
            };
            let Some(segment_count) = usize::try_from(material.name_count).ok() else {
                return false;
            };
            if material.name_presence != APPEARANCE_PRESENCE_PRESENT
                || segment_count > segment.len()
                || segment_len >= segment_count
            {
                return false;
            }
            segment[segment_len] = byte;
            segment_len += 1;
            if segment_len != segment_count {
                return true;
            }
            let valid = std::str::from_utf8(&segment[..segment_len])
                .ok()
                .is_some_and(|name| validate_entity_name_str(name).is_ok());
            material_index += 1;
            segment_len = 0;
            valid
        },
    )?;
    while let Some(material) = materials.get(material_index) {
        if material.name_count != 0
            || !matches!(
                material.name_presence,
                APPEARANCE_PRESENCE_ABSENT | APPEARANCE_PRESENCE_PRESENT
            )
            || (material.name_presence == APPEARANCE_PRESENCE_PRESENT
                && validate_entity_name_str("").is_err())
        {
            return Ok(false);
        }
        material_index += 1;
    }
    Ok(complete && segment_len == 0 && material_index == materials.len())
}

fn visit_appearance_material_factors<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: AppearanceDirectoryMembers<'session>,
    materials: &[AppearanceMaterialValidationScratch],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(factor_scalars) = directory.material_factor_scalars else {
        return Ok(directory.material_factor_scalar_count == 0 && materials.is_empty());
    };
    let mut values = [0.0_f32; APPEARANCE_MATERIAL_FACTOR_STRIDE];
    session.visit_canonical_sequence(
        factor_scalars,
        directory.material_factor_scalar_count,
        |index, value| {
            let material_index = index / APPEARANCE_MATERIAL_FACTOR_STRIDE;
            let scalar_index = index % APPEARANCE_MATERIAL_FACTOR_STRIDE;
            values[scalar_index] = value;
            scalar_index + 1 != APPEARANCE_MATERIAL_FACTOR_STRIDE
                || materials
                    .get(material_index)
                    .is_some_and(|material| validate_material_scratch(material, &values))
        },
    )
}

fn validate_material_scratch(
    material: &AppearanceMaterialValidationScratch,
    factor_values: &[f32; APPEARANCE_MATERIAL_FACTOR_STRIDE],
) -> bool {
    let Some(shading_model) = decode_appearance_shading_model(material.shading_model) else {
        return false;
    };
    let Some(alpha_mode) = decode_appearance_alpha_mode(material.alpha_mode, material.alpha_cutoff)
    else {
        return false;
    };
    let Some(double_sided) = decode_presence(material.double_sided) else {
        return false;
    };
    let Some(factors) = decode_appearance_material_factors(
        factor_values,
        material.pbr_metallic_roughness_presence,
        material.pbr_specular_glossiness_presence,
        material.legacy_presence,
    ) else {
        return false;
    };
    validate_material_semantics(MaterialSemanticsRef::new(
        &shading_model,
        &alpha_mode,
        double_sided,
        factors,
    ))
    .is_ok()
}

fn validate_material_binding_authority_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut AppearanceValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    bindings: &[AppearanceBindingValidationScratch],
    materials: &[AppearanceMaterialValidationScratch],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    for material in materials {
        let Some(range) = checked_range(
            material.binding_offset,
            material.binding_count,
            bindings.len(),
        ) else {
            return Ok(false);
        };
        let mut seen = [false; TextureRoleKind::BUILTIN_COUNT];
        let mut metallic = false;
        let mut roughness = false;
        let mut metallic_roughness = false;
        for binding in &bindings[range] {
            let Some(valid) = session.run_validation_unit(|| {
                let tag = binding.role;
                let Some(role_seen) = seen.get_mut(usize::from(tag)) else {
                    return false;
                };
                if *role_seen {
                    return false;
                }
                *role_seen = true;
                match tag {
                    APPEARANCE_ROLE_METALLIC if metallic_roughness => false,
                    APPEARANCE_ROLE_ROUGHNESS if metallic_roughness => false,
                    APPEARANCE_ROLE_METALLIC_ROUGHNESS if metallic || roughness => false,
                    APPEARANCE_ROLE_METALLIC => {
                        metallic = true;
                        true
                    }
                    APPEARANCE_ROLE_ROUGHNESS => {
                        roughness = true;
                        true
                    }
                    APPEARANCE_ROLE_METALLIC_ROUGHNESS => {
                        metallic_roughness = true;
                        true
                    }
                    _ => true,
                }
            })?
            else {
                return Ok(false);
            };
            if !valid {
                return Ok(false);
            }
        }
    }
    Ok(true)
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
    let material_names = optional_utf8_ref::<AppearanceMaterialNameSchema>(
        session,
        directory,
        appearance_directory_field::MATERIAL_NAME_BYTE_COUNT,
        appearance_directory_field::MATERIAL_NAMES,
        directory.scalar::<u32>(0, appearance_directory_field::MATERIAL_NAME_BYTE_COUNT)?,
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
        material_names,
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
        material_names: material_names.map(CanonicalBlockRef::from_store),
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

#[allow(clippy::too_many_arguments)]
fn validate_materials_structure(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<AppearanceMaterialTableSchema>>,
    bindings: Option<CanonicalRecordRef<AppearanceBindingTableSchema>>,
    binding_count: u32,
    factor_scalars: Option<CanonicalSequenceRef<f32, AppearanceMaterialFactorScalarSchema>>,
    material_names: Option<CanonicalUtf8Ref<AppearanceMaterialNameSchema>>,
    equivalence: &mut impl AppearanceEquivalence,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (binding_count == 0 && factor_scalars.is_none() && material_names.is_none())
            .then_some(());
    };
    let binding_rows = usize::try_from(binding_count).ok()?;
    let binding_table = match bindings {
        Some(reference) => Some(session.canonical_records(reference)?),
        None => None,
    };
    let table = session.canonical_records(reference)?;
    let factor_scalars_ref = factor_scalars?;
    let factor_scalars = session.canonical_sequence(factor_scalars_ref)?.as_slice();
    let material_names = match material_names {
        Some(reference) => Some((reference, session.canonical_utf8(reference)?)),
        None => None,
    };
    let factor_stride = u32::try_from(APPEARANCE_MATERIAL_FACTOR_STRIDE).ok()?;
    let mut binding_cursor = 0_usize;
    let mut factor_cursor = 0_usize;
    let mut name_cursor = 0_usize;
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
        let name_presence = table.scalar::<u8>(row, appearance_material_field::NAME_PRESENCE)?;
        let name_offset = table.scalar::<u32>(row, appearance_material_field::NAME_OFFSET)?;
        let name_count = table.scalar::<u32>(row, appearance_material_field::NAME_COUNT)?;
        require_scalar(Some(name_offset), u32::try_from(name_cursor).ok()?)?;
        let (name, next_name_cursor) = match (name_presence, name_count) {
            (APPEARANCE_PRESENCE_ABSENT, 0) => {
                require_scalar(
                    table.scalar::<u8>(row, appearance_material_field::NAMES),
                    0_u8,
                )?;
                (None, name_cursor)
            }
            (APPEARANCE_PRESENCE_PRESENT, 0) => {
                require_scalar(
                    table.scalar::<u8>(row, appearance_material_field::NAMES),
                    0_u8,
                )?;
                validate_entity_name_str("").ok()?;
                (Some(""), name_cursor)
            }
            (APPEARANCE_PRESENCE_PRESENT, _) => {
                let (reference, payload) = material_names?;
                require_scalar(
                    table.utf8_ref::<AppearanceMaterialNameSchema>(
                        row,
                        appearance_material_field::NAMES,
                    ),
                    reference,
                )?;
                let range = checked_range(name_offset, name_count, payload.len())?;
                observe_work(u64::try_from(range.len()).ok()?)?;
                let name = payload.get(range.clone())?;
                validate_entity_name_str(name).ok()?;
                (Some(name), range.end)
            }
            _ => return None,
        };
        name_cursor = next_name_cursor;
        equivalence.validate_material(row, table, bindings, factor_values, name)?;
    }
    (binding_cursor == binding_rows
        && factor_cursor == factor_scalars.len()
        && name_cursor == material_names.map_or(0, |(_, payload)| payload.len()))
    .then_some(())
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
