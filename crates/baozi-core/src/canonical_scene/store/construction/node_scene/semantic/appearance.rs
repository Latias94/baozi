use baozi_operation::{CanonicalRecordView, PublishedCanonicalStore, RecordBlock};

use crate::appearance::TextureRoleKind;
use crate::canonical_scene::store::schema::appearance::{
    APPEARANCE_ADDRESS_CLAMP_TO_BORDER, APPEARANCE_ADDRESS_CLAMP_TO_EDGE,
    APPEARANCE_ADDRESS_MIRRORED_REPEAT, APPEARANCE_ADDRESS_REPEAT, APPEARANCE_BINDING_FIELD_COUNT,
    APPEARANCE_IMAGE_FIELD_COUNT, APPEARANCE_IMAGE_PAYLOAD_ENCODED,
    APPEARANCE_MATERIAL_FIELD_COUNT, APPEARANCE_MEDIA_TYPE_UTF8, APPEARANCE_MIPMAP_FILTER_LINEAR,
    APPEARANCE_MIPMAP_FILTER_NEAREST, APPEARANCE_MIPMAP_FILTER_NONE, APPEARANCE_PRESENCE_ABSENT,
    APPEARANCE_PRESENCE_PRESENT, APPEARANCE_ROLE_BASE_COLOR, APPEARANCE_ROLE_CLEARCOAT,
    APPEARANCE_ROLE_CLEARCOAT_NORMAL, APPEARANCE_ROLE_CLEARCOAT_ROUGHNESS, APPEARANCE_ROLE_DIFFUSE,
    APPEARANCE_ROLE_DISPLACEMENT, APPEARANCE_ROLE_EMISSIVE, APPEARANCE_ROLE_HEIGHT,
    APPEARANCE_ROLE_LIGHTMAP, APPEARANCE_ROLE_METALLIC, APPEARANCE_ROLE_METALLIC_ROUGHNESS,
    APPEARANCE_ROLE_NORMAL, APPEARANCE_ROLE_OCCLUSION, APPEARANCE_ROLE_OPACITY,
    APPEARANCE_ROLE_ROUGHNESS, APPEARANCE_ROLE_SHEEN_COLOR, APPEARANCE_ROLE_SHEEN_ROUGHNESS,
    APPEARANCE_ROLE_SPECULAR, APPEARANCE_ROLE_THICKNESS, APPEARANCE_ROLE_TRANSMISSION,
    APPEARANCE_SAMPLER_FIELD_COUNT, APPEARANCE_TEXEL_FILTER_LINEAR,
    APPEARANCE_TEXEL_FILTER_NEAREST, APPEARANCE_TEXTURE_FIELD_COUNT,
    AppearanceEncodedImagePayloadSchema, AppearanceMediaTypeSchema, appearance_binding_field,
    appearance_image_field, appearance_material_field, appearance_sampler_field,
    appearance_texture_field,
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
}

impl<'scene> NodeSceneAppearanceTables<'scene> {
    pub(super) fn try_new(
        canonical: &'scene SceneCanonicalStore,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
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

fn resolve_utf8<'scene>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<AppearanceMediaTypeSchema>>,
    unavailable_reason: &'static str,
) -> NodeSceneSemanticResult<Option<&'scene str>> {
    reference
        .map(|reference| {
            canonical
                .utf8(reference.into_store())
                .ok_or_else(|| invariant(unavailable_reason))
        })
        .transpose()
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

    pub(in crate::canonical_scene::store) fn next_material(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAppearanceMaterial>> {
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
        if binding_offset
            .checked_add(binding_count)
            .is_none_or(|end| end > self.proof.material_binding_count())
        {
            return Err(invariant(
                "validated material binding range is outside binding space",
            ));
        }
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
        self.next_material = advance_row(row, "material cursor index overflows")?;
        Ok(Some(NodeSceneAppearanceMaterial {
            source_index: row,
            binding_offset,
            binding_count,
        }))
    }

    pub(in crate::canonical_scene::store) fn next_binding(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAppearanceBinding>> {
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
        self.next_binding = advance_row(row, "material-binding cursor index overflows")?;
        Ok(Some(NodeSceneAppearanceBinding {
            source_index: row,
            texture,
            role,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceMaterial {
    source_index: usize,
    binding_offset: usize,
    binding_count: usize,
}

impl NodeSceneAppearanceMaterial {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn binding_offset(self) -> usize {
        self.binding_offset
    }

    pub(in crate::canonical_scene::store) const fn binding_count(self) -> usize {
        self.binding_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneAppearanceBinding {
    source_index: usize,
    texture: usize,
    role: TextureRoleKind,
}

impl NodeSceneAppearanceBinding {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn texture(self) -> usize {
        self.texture
    }

    pub(in crate::canonical_scene::store) const fn role(self) -> TextureRoleKind {
        self.role
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

fn decode_texture_role(value: u8) -> NodeSceneSemanticResult<TextureRoleKind> {
    match value {
        APPEARANCE_ROLE_BASE_COLOR => Ok(TextureRoleKind::BaseColor),
        APPEARANCE_ROLE_DIFFUSE => Ok(TextureRoleKind::Diffuse),
        APPEARANCE_ROLE_SPECULAR => Ok(TextureRoleKind::Specular),
        APPEARANCE_ROLE_METALLIC => Ok(TextureRoleKind::Metallic),
        APPEARANCE_ROLE_ROUGHNESS => Ok(TextureRoleKind::Roughness),
        APPEARANCE_ROLE_METALLIC_ROUGHNESS => Ok(TextureRoleKind::MetallicRoughness),
        APPEARANCE_ROLE_NORMAL => Ok(TextureRoleKind::Normal),
        APPEARANCE_ROLE_OCCLUSION => Ok(TextureRoleKind::Occlusion),
        APPEARANCE_ROLE_EMISSIVE => Ok(TextureRoleKind::Emissive),
        APPEARANCE_ROLE_OPACITY => Ok(TextureRoleKind::Opacity),
        APPEARANCE_ROLE_HEIGHT => Ok(TextureRoleKind::Height),
        APPEARANCE_ROLE_DISPLACEMENT => Ok(TextureRoleKind::Displacement),
        APPEARANCE_ROLE_LIGHTMAP => Ok(TextureRoleKind::Lightmap),
        APPEARANCE_ROLE_CLEARCOAT => Ok(TextureRoleKind::Clearcoat),
        APPEARANCE_ROLE_CLEARCOAT_ROUGHNESS => Ok(TextureRoleKind::ClearcoatRoughness),
        APPEARANCE_ROLE_CLEARCOAT_NORMAL => Ok(TextureRoleKind::ClearcoatNormal),
        APPEARANCE_ROLE_TRANSMISSION => Ok(TextureRoleKind::Transmission),
        APPEARANCE_ROLE_THICKNESS => Ok(TextureRoleKind::Thickness),
        APPEARANCE_ROLE_SHEEN_COLOR => Ok(TextureRoleKind::SheenColor),
        APPEARANCE_ROLE_SHEEN_ROUGHNESS => Ok(TextureRoleKind::SheenRoughness),
        _ => Err(invariant(
            "validated material-binding texture-role code is unknown",
        )),
    }
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
