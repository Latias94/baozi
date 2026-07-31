use baozi_operation::{
    CanonicalRecordView, PublishedCanonicalStore, RecordBlock, ScalarSequence, SequenceView,
    StoredScalar, Utf8Block,
};

use crate::appearance::{
    AlphaMode, ColorInterpretation, MaterialFactors, MaterialSemanticsRef, ShadingModel,
    TextureBindingSemanticsRef, TextureBlendOperation, TextureChannel, TextureChannelMapping,
    TextureChannelSwizzle, TextureCoordinateSource, TextureRole, TextureRoleKind, TextureSourceKey,
    TextureStrength, TextureTransform, validate_material_semantics,
    validate_texture_binding_semantics,
};
use crate::canonical_scene::store::schema::appearance::{
    APPEARANCE_ADDRESS_CLAMP_TO_BORDER, APPEARANCE_ADDRESS_CLAMP_TO_EDGE,
    APPEARANCE_ADDRESS_MIRRORED_REPEAT, APPEARANCE_ADDRESS_REPEAT, APPEARANCE_BINDING_FIELD_COUNT,
    APPEARANCE_BINDING_SEMANTIC_STRIDE, APPEARANCE_CHANNEL_ZERO, APPEARANCE_IMAGE_FIELD_COUNT,
    APPEARANCE_IMAGE_PAYLOAD_ENCODED, APPEARANCE_MATERIAL_FACTOR_STRIDE,
    APPEARANCE_MATERIAL_FIELD_COUNT, APPEARANCE_MEDIA_TYPE_UTF8, APPEARANCE_MIPMAP_FILTER_LINEAR,
    APPEARANCE_MIPMAP_FILTER_NEAREST, APPEARANCE_MIPMAP_FILTER_NONE, APPEARANCE_PRESENCE_ABSENT,
    APPEARANCE_PRESENCE_PRESENT, APPEARANCE_SAMPLER_FIELD_COUNT, APPEARANCE_TEXEL_FILTER_LINEAR,
    APPEARANCE_TEXEL_FILTER_NEAREST, APPEARANCE_TEXTURE_FIELD_COUNT,
    AppearanceBindingSemanticScalarSchema, AppearanceEncodedImagePayloadSchema,
    AppearanceMaterialFactorScalarSchema, AppearanceMediaTypeSchema, AppearanceSourceKeySchema,
    appearance_binding_field, appearance_image_field, appearance_material_field,
    appearance_sampler_field, appearance_texture_field, decode_appearance_alpha_mode,
    decode_appearance_binding_numeric_semantics, decode_appearance_blend_operation,
    decode_appearance_channel, decode_appearance_color_interpretation,
    decode_appearance_coordinate_source, decode_appearance_material_factors,
    decode_appearance_shading_model, decode_appearance_texture_role,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, SceneDirectorySchema,
};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;
use crate::image::{MipmapFilter, SamplerState, TexelFilter, TextureAddressMode};

use super::{NodeSceneSemanticResult, invariant, read_scalar, read_u32_as_usize};

type SceneCanonicalStore = PublishedCanonicalStore<RecordBlock, SceneDirectorySchema>;

pub(super) struct NodeSceneAppearanceTables<'scene> {
    images: Option<CanonicalRecordView<'scene>>,
    samplers: Option<CanonicalRecordView<'scene>>,
    textures: Option<CanonicalRecordView<'scene>>,
    materials: Option<CanonicalRecordView<'scene>>,
    bindings: Option<CanonicalRecordView<'scene>>,
    image_payloads: Option<&'scene [u8]>,
    media_types: Option<&'scene str>,
    source_keys: Option<&'scene str>,
    binding_semantic_scalars: Option<&'scene [f32]>,
    material_factor_scalars: Option<&'scene [f32]>,
}

impl<'scene> NodeSceneAppearanceTables<'scene> {
    pub(super) fn try_new(
        canonical: &'scene SceneCanonicalStore,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        let binding_semantic_scalars = resolve_sequence(
            canonical,
            proof.appearance_binding_semantic_scalars(),
            "validated binding-semantic payload is unavailable or mistyped",
        )?;
        let material_factor_scalars = resolve_sequence(
            canonical,
            proof.appearance_material_factor_scalars(),
            "validated material-factor payload is unavailable or mistyped",
        )?;
        require_exact_payload_len(
            binding_semantic_scalars,
            proof.material_binding_count(),
            APPEARANCE_BINDING_SEMANTIC_STRIDE,
            "validated binding-semantic payload length disagrees with validation proof",
        )?;
        require_exact_payload_len(
            material_factor_scalars,
            proof.material_count(),
            APPEARANCE_MATERIAL_FACTOR_STRIDE,
            "validated material-factor payload length disagrees with validation proof",
        )?;

        let source_keys = resolve_utf8(
            canonical,
            proof.appearance_source_keys(),
            "validated source-key payload is unavailable or mistyped",
        )?;
        if source_keys.is_some_and(str::is_empty) {
            return Err(invariant(
                "validated present source-key payload has no UTF-8 bytes",
            ));
        }

        Ok(Self {
            images: resolve_table(
                canonical,
                proof.appearance_images(),
                proof.image_count(),
                APPEARANCE_IMAGE_FIELD_COUNT,
                "validated image table is unavailable or mistyped",
                "validated image table presence disagrees with validation proof",
                "validated image table layout disagrees with validation proof",
            )?,
            samplers: resolve_table(
                canonical,
                proof.appearance_samplers(),
                proof.sampler_count(),
                APPEARANCE_SAMPLER_FIELD_COUNT,
                "validated sampler table is unavailable or mistyped",
                "validated sampler table presence disagrees with validation proof",
                "validated sampler table layout disagrees with validation proof",
            )?,
            textures: resolve_table(
                canonical,
                proof.appearance_textures(),
                proof.texture_count(),
                APPEARANCE_TEXTURE_FIELD_COUNT,
                "validated texture table is unavailable or mistyped",
                "validated texture table presence disagrees with validation proof",
                "validated texture table layout disagrees with validation proof",
            )?,
            materials: resolve_table(
                canonical,
                proof.materials(),
                proof.material_count(),
                APPEARANCE_MATERIAL_FIELD_COUNT,
                "validated material table is unavailable or mistyped",
                "validated material table presence disagrees with validation proof",
                "validated material table layout disagrees with validation proof",
            )?,
            bindings: resolve_table(
                canonical,
                proof.appearance_bindings(),
                proof.material_binding_count(),
                APPEARANCE_BINDING_FIELD_COUNT,
                "validated material-binding table is unavailable or mistyped",
                "validated material-binding table presence disagrees with validation proof",
                "validated material-binding table layout disagrees with validation proof",
            )?,
            image_payloads: resolve_bytes(
                canonical,
                proof.appearance_image_payloads(),
                "validated encoded-image payload is unavailable or mistyped",
            )?,
            media_types: resolve_utf8(
                canonical,
                proof.appearance_media_types(),
                "validated image media-type payload is unavailable or mistyped",
            )?,
            source_keys,
            binding_semantic_scalars,
            material_factor_scalars,
        })
    }

    pub(super) const fn into_cursor(
        self,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneAppearanceCursor<'scene> {
        NodeSceneAppearanceCursor {
            tables: self,
            proof,
            phase: AppearanceCursorPhase::Images,
            next_image: 0,
            next_image_payload_offset: 0,
            next_media_type_offset: 0,
            next_sampler: 0,
            next_texture: 0,
            next_material: 0,
            next_binding: 0,
            next_material_binding_offset: 0,
            next_material_factor_offset: 0,
            next_binding_semantic_offset: 0,
            next_source_key_offset: 0,
        }
    }
}

fn resolve_table<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<Tag>>,
    expected_rows: usize,
    expected_fields: usize,
    unavailable_reason: &'static str,
    presence_reason: &'static str,
    layout_reason: &'static str,
) -> NodeSceneSemanticResult<Option<CanonicalRecordView<'scene>>>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    match (expected_rows, reference) {
        (0, None) => Ok(None),
        (0, Some(_)) | (_, None) => Err(invariant(presence_reason)),
        (_, Some(reference)) => {
            let table = canonical
                .records(reference.into_store())
                .ok_or_else(|| invariant(unavailable_reason))?;
            if table.rows() != expected_rows || table.fields() != expected_fields {
                return Err(invariant(layout_reason));
            }
            Ok(Some(table))
        }
    }
}

fn resolve_bytes<'scene>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<AppearanceEncodedImagePayloadSchema>>,
    unavailable_reason: &'static str,
) -> NodeSceneSemanticResult<Option<&'scene [u8]>> {
    reference
        .map(|reference| {
            canonical
                .bytes(reference.into_store())
                .ok_or_else(|| invariant(unavailable_reason))
        })
        .transpose()
}

fn resolve_utf8<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<Tag>>,
    unavailable_reason: &'static str,
) -> NodeSceneSemanticResult<Option<&'scene str>>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    reference
        .map(|reference| {
            canonical
                .utf8(reference.into_store())
                .ok_or_else(|| invariant(unavailable_reason))
        })
        .transpose()
}

fn resolve_sequence<'scene, T, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<Tag>>,
    unavailable_reason: &'static str,
) -> NodeSceneSemanticResult<Option<&'scene [T]>>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    reference
        .map(|reference| {
            let sequence: SequenceView<'scene, T> = canonical
                .sequence(reference.into_store())
                .ok_or_else(|| invariant(unavailable_reason))?;
            if sequence.as_slice().is_empty() {
                return Err(invariant(
                    "validated present appearance payload has no scalar values",
                ));
            }
            Ok(sequence.as_slice())
        })
        .transpose()
}

fn require_exact_payload_len<T>(
    payload: Option<&[T]>,
    rows: usize,
    stride: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    let expected = rows.checked_mul(stride).ok_or_else(|| invariant(reason))?;
    if payload.map_or(0, <[T]>::len) != expected || (expected == 0) != payload.is_none() {
        return Err(invariant(reason));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AppearanceCursorPhase {
    Images,
    Samplers,
    Textures,
    Materials,
    Bindings,
}

#[must_use = "the appearance cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceCursor<'scene> {
    tables: NodeSceneAppearanceTables<'scene>,
    proof: NodeSceneValidationProof,
    phase: AppearanceCursorPhase,
    next_image: usize,
    next_image_payload_offset: usize,
    next_media_type_offset: usize,
    next_sampler: usize,
    next_texture: usize,
    next_material: usize,
    next_binding: usize,
    next_material_binding_offset: usize,
    next_material_factor_offset: usize,
    next_binding_semantic_offset: usize,
    next_source_key_offset: usize,
}

impl<'scene> NodeSceneAppearanceCursor<'scene> {
    pub(in crate::canonical_scene::store) fn image_count(&self) -> usize {
        self.proof.image_count()
    }

    pub(in crate::canonical_scene::store) fn sampler_count(&self) -> usize {
        self.proof.sampler_count()
    }

    pub(in crate::canonical_scene::store) fn texture_count(&self) -> usize {
        self.proof.texture_count()
    }

    pub(in crate::canonical_scene::store) fn material_count(&self) -> usize {
        self.proof.material_count()
    }

    pub(in crate::canonical_scene::store) fn binding_count(&self) -> usize {
        self.proof.material_binding_count()
    }

    pub(in crate::canonical_scene::store) fn next_image(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAppearanceImage<'scene>>> {
        self.enter_phase(AppearanceCursorPhase::Images)?;
        let Some(row) = next_row(
            self.next_image,
            self.proof.image_count(),
            "image cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .images
            .ok_or_else(|| invariant("validated image table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            appearance_image_field::IMAGE,
            "validated image identity is unreadable",
            "validated image identity disagrees with its row",
        )?;
        if read_scalar::<u8>(
            table,
            row,
            appearance_image_field::PAYLOAD_KIND,
            "validated image payload kind is unreadable",
        )? != APPEARANCE_IMAGE_PAYLOAD_ENCODED
        {
            return Err(invariant("validated image payload kind is unsupported"));
        }
        let payload_offset = read_u32_as_usize(
            table,
            row,
            appearance_image_field::PAYLOAD_OFFSET,
            "validated image payload offset is unreadable",
        )?;
        let payload_count = read_u32_as_usize(
            table,
            row,
            appearance_image_field::PAYLOAD_COUNT,
            "validated image payload count is unreadable",
        )?;
        let payload_end = payload_offset
            .checked_add(payload_count)
            .ok_or_else(|| invariant("validated image payload range overflows"))?;
        if payload_offset != self.next_image_payload_offset {
            return Err(invariant(
                "validated image payload ranges are not contiguous",
            ));
        }
        let payload_ref = table
            .bytes_ref::<AppearanceEncodedImagePayloadSchema>(row, appearance_image_field::PAYLOADS)
            .ok_or_else(|| invariant("validated image payload reference is unreadable"))?;
        if Some(CanonicalBlockRef::from_store(payload_ref))
            != self.proof.appearance_image_payloads()
        {
            return Err(invariant(
                "validated image payload reference disagrees with validation proof",
            ));
        }
        let payload = self
            .tables
            .image_payloads
            .and_then(|payloads| payloads.get(payload_offset..payload_end))
            .ok_or_else(|| invariant("validated image payload range is unreadable"))?;

        let media_type = match read_scalar::<u8>(
            table,
            row,
            appearance_image_field::MEDIA_TYPE_PRESENCE,
            "validated image media-type presence is unreadable",
        )? {
            APPEARANCE_PRESENCE_ABSENT => {
                require_absent_media_type(table, row, self.next_media_type_offset)?;
                None
            }
            APPEARANCE_PRESENCE_PRESENT => {
                if read_scalar::<u8>(
                    table,
                    row,
                    appearance_image_field::MEDIA_TYPE_ENCODING,
                    "validated image media-type encoding is unreadable",
                )? != APPEARANCE_MEDIA_TYPE_UTF8
                {
                    return Err(invariant(
                        "validated image media-type encoding is unsupported",
                    ));
                }
                let offset = read_u32_as_usize(
                    table,
                    row,
                    appearance_image_field::MEDIA_TYPE_OFFSET,
                    "validated image media-type offset is unreadable",
                )?;
                let count = read_u32_as_usize(
                    table,
                    row,
                    appearance_image_field::MEDIA_TYPE_COUNT,
                    "validated image media-type count is unreadable",
                )?;
                let end = offset
                    .checked_add(count)
                    .ok_or_else(|| invariant("validated image media-type range overflows"))?;
                if offset != self.next_media_type_offset {
                    return Err(invariant(
                        "validated image media-type ranges are not contiguous",
                    ));
                }
                let reference = table
                    .utf8_ref::<AppearanceMediaTypeSchema>(row, appearance_image_field::MEDIA_TYPES)
                    .ok_or_else(|| {
                        invariant("validated image media-type reference is unreadable")
                    })?;
                if Some(CanonicalBlockRef::from_store(reference))
                    != self.proof.appearance_media_types()
                {
                    return Err(invariant(
                        "validated image media-type reference disagrees with validation proof",
                    ));
                }
                self.next_media_type_offset = end;
                Some(
                    self.tables
                        .media_types
                        .and_then(|media_types| media_types.get(offset..end))
                        .ok_or_else(|| {
                            invariant("validated image media-type range is unreadable")
                        })?,
                )
            }
            _ => return Err(invariant("validated image media-type presence is invalid")),
        };
        self.next_image_payload_offset = payload_end;
        self.next_image = advance_row(row, "image cursor index overflows")?;
        Ok(Some(NodeSceneAppearanceImage {
            source_index: row,
            encoded_payload: payload,
            media_type,
        }))
    }

    pub(in crate::canonical_scene::store) fn next_sampler(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAppearanceSampler>> {
        self.enter_phase(AppearanceCursorPhase::Samplers)?;
        let Some(row) = next_row(
            self.next_sampler,
            self.proof.sampler_count(),
            "sampler cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .samplers
            .ok_or_else(|| invariant("validated sampler table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            appearance_sampler_field::SAMPLER,
            "validated sampler identity is unreadable",
            "validated sampler identity disagrees with its row",
        )?;
        let sampler = NodeSceneAppearanceSampler {
            source_index: row,
            state: SamplerState::new(
                decode_texel_filter(read_scalar(
                    table,
                    row,
                    appearance_sampler_field::MINIFICATION,
                    "validated sampler minification is unreadable",
                )?)?,
                decode_texel_filter(read_scalar(
                    table,
                    row,
                    appearance_sampler_field::MAGNIFICATION,
                    "validated sampler magnification is unreadable",
                )?)?,
                decode_mipmap_filter(read_scalar(
                    table,
                    row,
                    appearance_sampler_field::MIPMAP,
                    "validated sampler mipmap filter is unreadable",
                )?)?,
                decode_address_mode(read_scalar(
                    table,
                    row,
                    appearance_sampler_field::ADDRESS_U,
                    "validated sampler U address mode is unreadable",
                )?)?,
                decode_address_mode(read_scalar(
                    table,
                    row,
                    appearance_sampler_field::ADDRESS_V,
                    "validated sampler V address mode is unreadable",
                )?)?,
                decode_address_mode(read_scalar(
                    table,
                    row,
                    appearance_sampler_field::ADDRESS_W,
                    "validated sampler W address mode is unreadable",
                )?)?,
            ),
        };
        self.next_sampler = advance_row(row, "sampler cursor index overflows")?;
        Ok(Some(sampler))
    }

    pub(in crate::canonical_scene::store) fn next_texture(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAppearanceTexture>> {
        self.enter_phase(AppearanceCursorPhase::Textures)?;
        let Some(row) = next_row(
            self.next_texture,
            self.proof.texture_count(),
            "texture cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .textures
            .ok_or_else(|| invariant("validated texture table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            appearance_texture_field::TEXTURE,
            "validated texture identity is unreadable",
            "validated texture identity disagrees with its row",
        )?;
        let image = read_u32_as_usize(
            table,
            row,
            appearance_texture_field::IMAGE,
            "validated texture image index is unreadable",
        )?;
        require_record_ref(
            table,
            row,
            appearance_texture_field::IMAGES,
            self.proof.appearance_images(),
            "validated texture image reference is unreadable",
            "validated texture image reference disagrees with validation proof",
        )?;
        if image >= self.proof.image_count() {
            return Err(invariant(
                "validated texture image index is outside image space",
            ));
        }
        let sampler = read_u32_as_usize(
            table,
            row,
            appearance_texture_field::SAMPLER,
            "validated texture sampler index is unreadable",
        )?;
        require_record_ref(
            table,
            row,
            appearance_texture_field::SAMPLERS,
            self.proof.appearance_samplers(),
            "validated texture sampler reference is unreadable",
            "validated texture sampler reference disagrees with validation proof",
        )?;
        if sampler >= self.proof.sampler_count() {
            return Err(invariant(
                "validated texture sampler index is outside sampler space",
            ));
        }
        self.next_texture = advance_row(row, "texture cursor index overflows")?;
        Ok(Some(NodeSceneAppearanceTexture {
            source_index: row,
            image,
            sampler,
        }))
    }

    #[allow(clippy::too_many_lines)]
    pub(in crate::canonical_scene::store) fn next_material(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAppearanceMaterial<'scene>>> {
        self.enter_phase(AppearanceCursorPhase::Materials)?;
        let Some(row) = next_row(
            self.next_material,
            self.proof.material_count(),
            "material cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .materials
            .ok_or_else(|| invariant("validated material table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            appearance_material_field::MATERIAL,
            "validated material identity is unreadable",
            "validated material identity disagrees with its row",
        )?;
        let shading_model = decode_appearance_shading_model(read_scalar(
            table,
            row,
            appearance_material_field::SHADING_MODEL,
            "validated material shading model is unreadable",
        )?)
        .ok_or_else(|| invariant("validated material shading-model code is unknown"))?;
        let alpha_cutoff = read_scalar::<f32>(
            table,
            row,
            appearance_material_field::ALPHA_CUTOFF,
            "validated material alpha cutoff is unreadable",
        )?;
        let alpha_mode = decode_appearance_alpha_mode(
            read_scalar(
                table,
                row,
                appearance_material_field::ALPHA_MODE,
                "validated material alpha mode is unreadable",
            )?,
            alpha_cutoff,
        )
        .ok_or_else(|| invariant("validated material alpha-mode encoding is invalid"))?;
        let double_sided = decode_presence(
            read_scalar(
                table,
                row,
                appearance_material_field::DOUBLE_SIDED,
                "validated material double-sided flag is unreadable",
            )?,
            "validated material double-sided flag is invalid",
        )?;
        let pbr_metallic_roughness_presence = read_scalar(
            table,
            row,
            appearance_material_field::PBR_METALLIC_ROUGHNESS_PRESENCE,
            "validated metallic-roughness presence is unreadable",
        )?;
        let pbr_specular_glossiness_presence = read_scalar(
            table,
            row,
            appearance_material_field::PBR_SPECULAR_GLOSSINESS_PRESENCE,
            "validated specular-glossiness presence is unreadable",
        )?;
        let legacy_presence = read_scalar(
            table,
            row,
            appearance_material_field::LEGACY_PRESENCE,
            "validated legacy-factor presence is unreadable",
        )?;
        let binding_offset = read_u32_as_usize(
            table,
            row,
            appearance_material_field::BINDING_OFFSET,
            "validated material binding offset is unreadable",
        )?;
        let binding_count = read_u32_as_usize(
            table,
            row,
            appearance_material_field::BINDING_COUNT,
            "validated material binding count is unreadable",
        )?;
        if binding_offset != self.next_material_binding_offset {
            return Err(invariant(
                "validated material binding ranges are not contiguous",
            ));
        }
        let binding_end = binding_offset
            .checked_add(binding_count)
            .filter(|end| *end <= self.proof.material_binding_count())
            .ok_or_else(|| {
                invariant("validated material binding range is outside binding space")
            })?;
        if binding_count == 0 {
            if table.scalar::<u8>(row, appearance_material_field::BINDINGS) != Some(0) {
                return Err(invariant(
                    "validated empty material binding range has a nonzero sentinel",
                ));
            }
        } else {
            require_record_ref(
                table,
                row,
                appearance_material_field::BINDINGS,
                self.proof.appearance_bindings(),
                "validated material binding reference is unreadable",
                "validated material binding reference disagrees with validation proof",
            )?;
        }

        let factor_offset = read_u32_as_usize(
            table,
            row,
            appearance_material_field::FACTOR_OFFSET,
            "validated material factor offset is unreadable",
        )?;
        let factor_count = read_u32_as_usize(
            table,
            row,
            appearance_material_field::FACTOR_COUNT,
            "validated material factor count is unreadable",
        )?;
        if factor_offset != self.next_material_factor_offset
            || factor_count != APPEARANCE_MATERIAL_FACTOR_STRIDE
        {
            return Err(invariant(
                "validated material factor range is not canonical",
            ));
        }
        require_sequence_ref::<AppearanceMaterialFactorScalarSchema>(
            table,
            row,
            appearance_material_field::FACTORS,
            self.proof.appearance_material_factor_scalars(),
            "validated material factor reference is unreadable",
            "validated material factor reference disagrees with validation proof",
        )?;
        let factor_values = slice_payload(
            self.tables.material_factor_scalars,
            factor_offset,
            factor_count,
            "validated material factor range is unreadable",
        )?;
        let factors = decode_appearance_material_factors(
            factor_values,
            pbr_metallic_roughness_presence,
            pbr_specular_glossiness_presence,
            legacy_presence,
        )
        .ok_or_else(|| invariant("validated material factors are invalid"))?;
        validate_material_semantics(MaterialSemanticsRef::new(
            &shading_model,
            &alpha_mode,
            double_sided,
            factors,
        ))
        .map_err(|_| invariant("validated material semantics are inconsistent"))?;

        let factor_end = factor_offset
            .checked_add(factor_count)
            .ok_or_else(|| invariant("validated material factor range overflows"))?;
        self.next_material_binding_offset = binding_end;
        self.next_material_factor_offset = factor_end;
        self.next_material = advance_row(row, "material cursor index overflows")?;
        Ok(Some(NodeSceneAppearanceMaterial {
            source_index: row,
            shading_model,
            alpha_mode,
            double_sided,
            factors,
            binding_offset,
            binding_count,
            factor_offset,
            factor_values,
        }))
    }

    #[allow(clippy::too_many_lines)]
    pub(in crate::canonical_scene::store) fn next_binding(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAppearanceBinding<'scene>>> {
        self.enter_phase(AppearanceCursorPhase::Bindings)?;
        let Some(row) = next_row(
            self.next_binding,
            self.proof.material_binding_count(),
            "material-binding cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .bindings
            .ok_or_else(|| invariant("validated material-binding table view is absent"))?;
        require_row_ordinal(
            table,
            row,
            appearance_binding_field::BINDING,
            "validated material-binding identity is unreadable",
            "validated material-binding identity disagrees with its row",
        )?;
        let texture = read_u32_as_usize(
            table,
            row,
            appearance_binding_field::TEXTURE,
            "validated material-binding texture index is unreadable",
        )?;
        require_record_ref(
            table,
            row,
            appearance_binding_field::TEXTURES,
            self.proof.appearance_textures(),
            "validated material-binding texture reference is unreadable",
            "validated material-binding texture reference disagrees with validation proof",
        )?;
        if texture >= self.proof.texture_count() {
            return Err(invariant(
                "validated material-binding texture index is outside texture space",
            ));
        }
        let role = decode_texture_role(read_scalar(
            table,
            row,
            appearance_binding_field::ROLE,
            "validated material-binding role is unreadable",
        )?)?;
        let coordinate_source = decode_appearance_coordinate_source(
            read_scalar(
                table,
                row,
                appearance_binding_field::COORDINATE_SOURCE,
                "validated material-binding coordinate source is unreadable",
            )?,
            read_scalar(
                table,
                row,
                appearance_binding_field::COORDINATE_SET,
                "validated material-binding coordinate set is unreadable",
            )?,
        )
        .ok_or_else(|| invariant("validated material-binding coordinate source is invalid"))?;
        let lane_count = usize::from(read_scalar::<u8>(
            table,
            row,
            appearance_binding_field::CHANNEL_LANE_COUNT,
            "validated material-binding channel count is unreadable",
        )?);
        if !(1..=4).contains(&lane_count) {
            return Err(invariant(
                "validated material-binding channel count is invalid",
            ));
        }
        let channel_fields = [
            appearance_binding_field::CHANNEL_0,
            appearance_binding_field::CHANNEL_1,
            appearance_binding_field::CHANNEL_2,
            appearance_binding_field::CHANNEL_3,
        ];
        let mut channels = [TextureChannel::Zero; 4];
        for (index, field) in channel_fields.into_iter().enumerate() {
            let tag = read_scalar::<u8>(
                table,
                row,
                field,
                "validated material-binding channel is unreadable",
            )?;
            if index >= lane_count && tag != APPEARANCE_CHANNEL_ZERO {
                return Err(invariant(
                    "validated material-binding channel padding is not canonical",
                ));
            }
            channels[index] = decode_appearance_channel(tag)
                .ok_or_else(|| invariant("validated material-binding channel code is unknown"))?;
        }
        let channel_mapping = TextureChannelMapping::swizzle(
            TextureChannelSwizzle::try_new(&channels[..lane_count])
                .map_err(|_| invariant("validated material-binding channel mapping is invalid"))?,
        );
        let color_interpretation = decode_appearance_color_interpretation(read_scalar(
            table,
            row,
            appearance_binding_field::COLOR_INTERPRETATION,
            "validated material-binding color interpretation is unreadable",
        )?)
        .ok_or_else(|| invariant("validated material-binding color interpretation is invalid"))?;

        let semantic_offset = read_u32_as_usize(
            table,
            row,
            appearance_binding_field::SEMANTIC_SCALAR_OFFSET,
            "validated material-binding semantic offset is unreadable",
        )?;
        let semantic_count = read_u32_as_usize(
            table,
            row,
            appearance_binding_field::SEMANTIC_SCALAR_COUNT,
            "validated material-binding semantic count is unreadable",
        )?;
        if semantic_offset != self.next_binding_semantic_offset
            || semantic_count != APPEARANCE_BINDING_SEMANTIC_STRIDE
        {
            return Err(invariant(
                "validated material-binding semantic range is not canonical",
            ));
        }
        require_sequence_ref::<AppearanceBindingSemanticScalarSchema>(
            table,
            row,
            appearance_binding_field::SEMANTIC_SCALARS,
            self.proof.appearance_binding_semantic_scalars(),
            "validated material-binding semantic reference is unreadable",
            "validated material-binding semantic reference disagrees with validation proof",
        )?;
        let semantic_values = slice_payload(
            self.tables.binding_semantic_scalars,
            semantic_offset,
            semantic_count,
            "validated material-binding semantic range is unreadable",
        )?;
        let (transform, strength) = decode_appearance_binding_numeric_semantics(semantic_values)
            .ok_or_else(|| invariant("validated material-binding numeric semantics are invalid"))?;
        let blend = decode_appearance_blend_operation(read_scalar(
            table,
            row,
            appearance_binding_field::BLEND_OPERATION,
            "validated material-binding blend operation is unreadable",
        )?)
        .ok_or_else(|| invariant("validated material-binding blend operation is invalid"))?;

        let source_key_offset = read_u32_as_usize(
            table,
            row,
            appearance_binding_field::SOURCE_KEY_OFFSET,
            "validated material-binding source-key offset is unreadable",
        )?;
        let source_key_count = read_u32_as_usize(
            table,
            row,
            appearance_binding_field::SOURCE_KEY_COUNT,
            "validated material-binding source-key count is unreadable",
        )?;
        if source_key_offset != self.next_source_key_offset {
            return Err(invariant(
                "validated material-binding source-key ranges are not contiguous",
            ));
        }
        let source_key = if source_key_count == 0 {
            if table.scalar::<u8>(row, appearance_binding_field::SOURCE_KEYS) != Some(0) {
                return Err(invariant(
                    "validated absent material-binding source key has a nonzero sentinel",
                ));
            }
            None
        } else {
            require_utf8_ref::<AppearanceSourceKeySchema>(
                table,
                row,
                appearance_binding_field::SOURCE_KEYS,
                self.proof.appearance_source_keys(),
                "validated material-binding source-key reference is unreadable",
                "validated material-binding source-key reference disagrees with validation proof",
            )?;
            Some(slice_utf8_payload(
                self.tables.source_keys,
                source_key_offset,
                source_key_count,
                "validated material-binding source-key range is unreadable",
            )?)
        };
        let source_key_value = source_key
            .map(TextureSourceKey::try_new)
            .transpose()
            .map_err(|_| invariant("validated material-binding source key is invalid"))?;
        validate_texture_binding_semantics(TextureBindingSemanticsRef::new(
            &role,
            &coordinate_source,
            &channel_mapping,
            &color_interpretation,
            transform,
            strength,
            &blend,
            source_key_value.as_ref(),
        ))
        .map_err(|_| invariant("validated material-binding semantics are inconsistent"))?;

        self.next_binding_semantic_offset = semantic_offset
            .checked_add(semantic_count)
            .ok_or_else(|| invariant("validated material-binding semantic range overflows"))?;
        self.next_source_key_offset = source_key_offset
            .checked_add(source_key_count)
            .ok_or_else(|| invariant("validated material-binding source-key range overflows"))?;
        self.next_binding = advance_row(row, "material-binding cursor index overflows")?;
        Ok(Some(NodeSceneAppearanceBinding {
            source_index: row,
            texture,
            role,
            coordinate_source,
            channel_mapping,
            color_interpretation,
            transform,
            strength,
            semantic_offset,
            semantic_values,
            blend,
            source_key_offset,
            source_key,
        }))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if self.next_image != self.proof.image_count()
            || self.next_sampler != self.proof.sampler_count()
            || self.next_texture != self.proof.texture_count()
            || self.next_material != self.proof.material_count()
            || self.next_binding != self.proof.material_binding_count()
        {
            return Err(invariant(
                "appearance cursor ended before all proof-qualified tables were exhausted",
            ));
        }
        if self.next_image_payload_offset != self.tables.image_payloads.map_or(0, <[u8]>::len)
            || self.next_media_type_offset != self.tables.media_types.map_or(0, str::len)
            || self.next_material_binding_offset != self.proof.material_binding_count()
            || self.next_material_factor_offset
                != self.tables.material_factor_scalars.map_or(0, <[f32]>::len)
            || self.next_binding_semantic_offset
                != self.tables.binding_semantic_scalars.map_or(0, <[f32]>::len)
            || self.next_source_key_offset != self.tables.source_keys.map_or(0, str::len)
        {
            return Err(invariant(
                "appearance payload cursors ended before their proof-bound storage",
            ));
        }
        Ok(())
    }

    fn enter_phase(&mut self, target: AppearanceCursorPhase) -> NodeSceneSemanticResult<()> {
        if target < self.phase {
            return Err(invariant(
                "appearance cursor cannot return to an earlier table phase",
            ));
        }
        if target >= AppearanceCursorPhase::Samplers && self.next_image != self.proof.image_count()
        {
            return Err(invariant("image table must be exhausted before samplers"));
        }
        if target >= AppearanceCursorPhase::Textures
            && self.next_sampler != self.proof.sampler_count()
        {
            return Err(invariant("sampler table must be exhausted before textures"));
        }
        if target >= AppearanceCursorPhase::Materials
            && self.next_texture != self.proof.texture_count()
        {
            return Err(invariant(
                "texture table must be exhausted before materials",
            ));
        }
        if target >= AppearanceCursorPhase::Bindings
            && self.next_material != self.proof.material_count()
        {
            return Err(invariant(
                "material table must be exhausted before bindings",
            ));
        }
        self.phase = target;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceImage<'scene> {
    source_index: usize,
    encoded_payload: &'scene [u8],
    media_type: Option<&'scene str>,
}

impl<'scene> NodeSceneAppearanceImage<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn encoded_payload(self) -> &'scene [u8] {
        self.encoded_payload
    }

    pub(in crate::canonical_scene::store) const fn media_type(self) -> Option<&'scene str> {
        self.media_type
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceSampler {
    source_index: usize,
    state: SamplerState,
}

impl NodeSceneAppearanceSampler {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn state(self) -> SamplerState {
        self.state
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceTexture {
    source_index: usize,
    image: usize,
    sampler: usize,
}

impl NodeSceneAppearanceTexture {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn image(self) -> usize {
        self.image
    }

    pub(in crate::canonical_scene::store) const fn sampler(self) -> usize {
        self.sampler
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceMaterial<'scene> {
    source_index: usize,
    shading_model: ShadingModel,
    alpha_mode: AlphaMode,
    double_sided: bool,
    factors: MaterialFactors,
    binding_offset: usize,
    binding_count: usize,
    factor_offset: usize,
    factor_values: &'scene [f32],
}

impl<'scene> NodeSceneAppearanceMaterial<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn shading_model(&self) -> &ShadingModel {
        &self.shading_model
    }

    pub(in crate::canonical_scene::store) const fn alpha_mode(&self) -> &AlphaMode {
        &self.alpha_mode
    }

    pub(in crate::canonical_scene::store) const fn double_sided(&self) -> bool {
        self.double_sided
    }

    pub(in crate::canonical_scene::store) const fn factors(&self) -> MaterialFactors {
        self.factors
    }

    pub(in crate::canonical_scene::store) const fn binding_offset(&self) -> usize {
        self.binding_offset
    }

    pub(in crate::canonical_scene::store) const fn binding_count(&self) -> usize {
        self.binding_count
    }

    pub(in crate::canonical_scene::store) const fn factor_offset(&self) -> usize {
        self.factor_offset
    }

    pub(in crate::canonical_scene::store) const fn factor_values(&self) -> &'scene [f32] {
        self.factor_values
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceBinding<'scene> {
    source_index: usize,
    texture: usize,
    role: TextureRole,
    coordinate_source: TextureCoordinateSource,
    channel_mapping: TextureChannelMapping,
    color_interpretation: ColorInterpretation,
    transform: TextureTransform,
    strength: TextureStrength,
    semantic_offset: usize,
    semantic_values: &'scene [f32],
    blend: TextureBlendOperation,
    source_key_offset: usize,
    source_key: Option<&'scene str>,
}

impl<'scene> NodeSceneAppearanceBinding<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(&self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn texture(&self) -> usize {
        self.texture
    }

    pub(in crate::canonical_scene::store) const fn role(&self) -> TextureRoleKind {
        self.role.kind()
    }

    pub(in crate::canonical_scene::store) const fn role_semantics(&self) -> &TextureRole {
        &self.role
    }

    pub(in crate::canonical_scene::store) const fn coordinate_source(
        &self,
    ) -> &TextureCoordinateSource {
        &self.coordinate_source
    }

    pub(in crate::canonical_scene::store) const fn channel_mapping(
        &self,
    ) -> &TextureChannelMapping {
        &self.channel_mapping
    }

    pub(in crate::canonical_scene::store) const fn color_interpretation(
        &self,
    ) -> &ColorInterpretation {
        &self.color_interpretation
    }

    pub(in crate::canonical_scene::store) const fn transform(&self) -> TextureTransform {
        self.transform
    }

    pub(in crate::canonical_scene::store) const fn strength(&self) -> TextureStrength {
        self.strength
    }

    pub(in crate::canonical_scene::store) const fn semantic_offset(&self) -> usize {
        self.semantic_offset
    }

    pub(in crate::canonical_scene::store) const fn semantic_values(&self) -> &'scene [f32] {
        self.semantic_values
    }

    pub(in crate::canonical_scene::store) const fn blend(&self) -> &TextureBlendOperation {
        &self.blend
    }

    pub(in crate::canonical_scene::store) const fn source_key_offset(&self) -> usize {
        self.source_key_offset
    }

    pub(in crate::canonical_scene::store) const fn source_key(&self) -> Option<&'scene str> {
        self.source_key
    }
}

fn require_absent_media_type(
    table: CanonicalRecordView<'_>,
    row: usize,
    expected_offset: usize,
) -> NodeSceneSemanticResult<()> {
    if table.scalar::<u8>(row, appearance_image_field::MEDIA_TYPE_ENCODING)
        != Some(APPEARANCE_MEDIA_TYPE_UTF8)
        || read_u32_as_usize(
            table,
            row,
            appearance_image_field::MEDIA_TYPE_OFFSET,
            "validated absent image media-type offset is unreadable",
        )? != expected_offset
        || table.scalar::<u32>(row, appearance_image_field::MEDIA_TYPE_COUNT) != Some(0)
        || table.scalar::<u8>(row, appearance_image_field::MEDIA_TYPES) != Some(0)
    {
        return Err(invariant(
            "validated absent image media type has an invalid sentinel",
        ));
    }
    Ok(())
}

fn decode_texel_filter(value: u8) -> NodeSceneSemanticResult<TexelFilter> {
    match value {
        APPEARANCE_TEXEL_FILTER_NEAREST => Ok(TexelFilter::Nearest),
        APPEARANCE_TEXEL_FILTER_LINEAR => Ok(TexelFilter::Linear),
        _ => Err(invariant("validated sampler texel-filter code is unknown")),
    }
}

fn decode_mipmap_filter(value: u8) -> NodeSceneSemanticResult<MipmapFilter> {
    match value {
        APPEARANCE_MIPMAP_FILTER_NONE => Ok(MipmapFilter::None),
        APPEARANCE_MIPMAP_FILTER_NEAREST => Ok(MipmapFilter::Nearest),
        APPEARANCE_MIPMAP_FILTER_LINEAR => Ok(MipmapFilter::Linear),
        _ => Err(invariant("validated sampler mipmap-filter code is unknown")),
    }
}

fn decode_address_mode(value: u8) -> NodeSceneSemanticResult<TextureAddressMode> {
    match value {
        APPEARANCE_ADDRESS_REPEAT => Ok(TextureAddressMode::Repeat),
        APPEARANCE_ADDRESS_MIRRORED_REPEAT => Ok(TextureAddressMode::MirroredRepeat),
        APPEARANCE_ADDRESS_CLAMP_TO_EDGE => Ok(TextureAddressMode::ClampToEdge),
        APPEARANCE_ADDRESS_CLAMP_TO_BORDER => Ok(TextureAddressMode::ClampToBorder),
        _ => Err(invariant("validated sampler address-mode code is unknown")),
    }
}

fn decode_texture_role(value: u8) -> NodeSceneSemanticResult<TextureRole> {
    decode_appearance_texture_role(value)
        .ok_or_else(|| invariant("validated material-binding texture-role code is unknown"))
}

fn decode_presence(value: u8, reason: &'static str) -> NodeSceneSemanticResult<bool> {
    match value {
        APPEARANCE_PRESENCE_ABSENT => Ok(false),
        APPEARANCE_PRESENCE_PRESENT => Ok(true),
        _ => Err(invariant(reason)),
    }
}

fn slice_payload<'scene, T>(
    payload: Option<&'scene [T]>,
    offset: usize,
    count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene [T]> {
    let end = offset.checked_add(count).ok_or_else(|| invariant(reason))?;
    payload
        .and_then(|payload| payload.get(offset..end))
        .ok_or_else(|| invariant(reason))
}

fn slice_utf8_payload<'scene>(
    payload: Option<&'scene str>,
    offset: usize,
    count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene str> {
    let end = offset.checked_add(count).ok_or_else(|| invariant(reason))?;
    payload
        .and_then(|payload| payload.get(offset..end))
        .ok_or_else(|| invariant(reason))
}

fn require_sequence_ref<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    expected: Option<CanonicalBlockRef<Tag>>,
    unreadable_reason: &'static str,
    mismatch_reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<f32>>,
{
    let reference = table
        .sequence_ref::<f32, Tag>(row, field)
        .ok_or_else(|| invariant(unreadable_reason))?;
    if Some(CanonicalBlockRef::from_store(reference)) != expected {
        return Err(invariant(mismatch_reason));
    }
    Ok(())
}

fn require_utf8_ref<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    expected: Option<CanonicalBlockRef<Tag>>,
    unreadable_reason: &'static str,
    mismatch_reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    let reference = table
        .utf8_ref::<Tag>(row, field)
        .ok_or_else(|| invariant(unreadable_reason))?;
    if Some(CanonicalBlockRef::from_store(reference)) != expected {
        return Err(invariant(mismatch_reason));
    }
    Ok(())
}

fn require_record_ref<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    expected: Option<CanonicalBlockRef<Tag>>,
    unreadable_reason: &'static str,
    mismatch_reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    let reference = table
        .record_ref::<Tag>(row, field)
        .ok_or_else(|| invariant(unreadable_reason))?;
    if Some(CanonicalBlockRef::from_store(reference)) != expected {
        return Err(invariant(mismatch_reason));
    }
    Ok(())
}

fn require_row_ordinal(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    unreadable_reason: &'static str,
    mismatch_reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    let value = read_u32_as_usize(table, row, field, unreadable_reason)?;
    if value != row {
        return Err(invariant(mismatch_reason));
    }
    Ok(())
}

fn next_row(
    next: usize,
    expected: usize,
    overflow_reason: &'static str,
) -> NodeSceneSemanticResult<Option<usize>> {
    if next == expected {
        return Ok(None);
    }
    if next > expected {
        return Err(invariant(overflow_reason));
    }
    Ok(Some(next))
}

fn advance_row(row: usize, reason: &'static str) -> NodeSceneSemanticResult<usize> {
    row.checked_add(1).ok_or_else(|| invariant(reason))
}
