use crate::appearance::TextureRoleKind;
use crate::image::{MipmapFilter, SamplerState, TexelFilter, TextureAddressMode};

use super::super::semantic::NodeSceneAppearanceCursor;
use super::{
    CanonicalDigestEncoder, NodeSceneSnapshotError, PRESENCE_ABSENT, PRESENCE_PRESENT, PreviewSink,
    SnapshotWorkMeter, TAG_COLLECTION_TOTAL, TAG_MATERIAL, TAG_MATERIALS, TAG_PRESENCE,
    TAG_REFERENCE, begin_geometry_collection,
};

const TAG_APPEARANCE: u8 = 51;
const TAG_IMAGES: u8 = 52;
const TAG_IMAGE: u8 = 53;
const TAG_ENCODED_PAYLOAD: u8 = 54;
const TAG_MEDIA_TYPE: u8 = 55;
const TAG_SAMPLERS: u8 = 56;
const TAG_SAMPLER: u8 = 57;
const TAG_MINIFICATION: u8 = 58;
const TAG_MAGNIFICATION: u8 = 59;
const TAG_MIPMAP: u8 = 60;
const TAG_ADDRESS_U: u8 = 61;
const TAG_ADDRESS_V: u8 = 62;
const TAG_ADDRESS_W: u8 = 63;
const TAG_TEXTURES: u8 = 64;
const TAG_TEXTURE: u8 = 65;
const TAG_IMAGE_REFERENCE: u8 = 66;
const TAG_SAMPLER_REFERENCE: u8 = 67;
const TAG_BINDINGS: u8 = 68;
const TAG_BINDING: u8 = 69;
const TAG_TEXTURE_REFERENCE: u8 = 70;
const TAG_TEXTURE_ROLE: u8 = 71;
const TAG_BINDING_OFFSET: u8 = 72;
const TAG_MEDIA_TYPE_BYTES: u8 = 73;

const SNAPSHOT_PAYLOAD_CHUNK_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy)]
pub(super) struct AppearanceCollectionCounts {
    pub(super) images: usize,
    pub(super) samplers: usize,
    pub(super) textures: usize,
    pub(super) materials: usize,
    pub(super) bindings: usize,
}

#[derive(Clone, Copy, Default)]
pub(super) struct AppearancePayloadTotals {
    pub(super) image_payload_bytes: u64,
    pub(super) media_type_bytes: u64,
}

pub(super) fn write_appearance(
    cursor: &mut NodeSceneAppearanceCursor<'_>,
    counts: AppearanceCollectionCounts,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<AppearancePayloadTotals, NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(TAG_APPEARANCE)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.push_str("appearance(present)\n");

    let mut totals = AppearancePayloadTotals::default();
    write_images(
        cursor,
        counts.images,
        transcript,
        preview,
        work,
        &mut totals,
    )?;
    write_samplers(cursor, counts.samplers, transcript, preview, work)?;
    write_textures(cursor, counts.textures, transcript, preview, work)?;
    write_materials(cursor, counts.materials, transcript, preview, work)?;
    write_bindings(cursor, counts.bindings, transcript, preview, work)?;
    preview.push_str("appearance_end\n");
    Ok(totals)
}

fn write_images(
    cursor: &mut NodeSceneAppearanceCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    totals: &mut AppearancePayloadTotals,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(transcript, preview, work, TAG_IMAGES, "images", count)?;
    for _ in 0..count {
        work.observe()?;
        let image = cursor
            .next_image()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "image collection ended before declared total",
            })?;
        transcript.write_tag(TAG_IMAGE)?;
        transcript.tagged_usize(TAG_REFERENCE, image.source_index())?;
        write_metered_bytes(
            transcript,
            work,
            TAG_ENCODED_PAYLOAD,
            image.encoded_payload(),
        )?;
        totals.image_payload_bytes = add_bytes(
            totals.image_payload_bytes,
            image.encoded_payload().len(),
            "encoded image payload bytes",
        )?;

        transcript.write_tag(TAG_MEDIA_TYPE)?;
        match image.media_type() {
            Some(media_type) => {
                transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
                write_metered_bytes(
                    transcript,
                    work,
                    TAG_MEDIA_TYPE_BYTES,
                    media_type.as_bytes(),
                )?;
                totals.media_type_bytes = add_bytes(
                    totals.media_type_bytes,
                    media_type.len(),
                    "image media-type bytes",
                )?;
            }
            None => transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?,
        }

        let render = preview.begin_geometry_item("image", image.source_index())?;
        preview.geometry_field_usize(render, "payload_bytes", image.encoded_payload().len());
        preview.geometry_field_word(render, "media_type", image.media_type().unwrap_or("absent"));
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_samplers(
    cursor: &mut NodeSceneAppearanceCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(transcript, preview, work, TAG_SAMPLERS, "samplers", count)?;
    for _ in 0..count {
        work.observe()?;
        let sampler = cursor
            .next_sampler()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "sampler collection ended before declared total",
            })?;
        let state = sampler.state();
        transcript.write_tag(TAG_SAMPLER)?;
        transcript.tagged_usize(TAG_REFERENCE, sampler.source_index())?;
        transcript.tagged_u8(TAG_MINIFICATION, encode_texel_filter(state.minification()))?;
        transcript.tagged_u8(
            TAG_MAGNIFICATION,
            encode_texel_filter(state.magnification()),
        )?;
        transcript.tagged_u8(TAG_MIPMAP, encode_mipmap_filter(state.mipmap()))?;
        transcript.tagged_u8(TAG_ADDRESS_U, encode_address_mode(state.address_u()))?;
        transcript.tagged_u8(TAG_ADDRESS_V, encode_address_mode(state.address_v()))?;
        transcript.tagged_u8(TAG_ADDRESS_W, encode_address_mode(state.address_w()))?;
        let render = preview.begin_geometry_item("sampler", sampler.source_index())?;
        preview_sampler(preview, render, state);
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_textures(
    cursor: &mut NodeSceneAppearanceCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(transcript, preview, work, TAG_TEXTURES, "textures", count)?;
    for _ in 0..count {
        work.observe()?;
        let texture = cursor
            .next_texture()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "texture collection ended before declared total",
            })?;
        transcript.write_tag(TAG_TEXTURE)?;
        transcript.tagged_usize(TAG_REFERENCE, texture.source_index())?;
        transcript.tagged_usize(TAG_IMAGE_REFERENCE, texture.image())?;
        transcript.tagged_usize(TAG_SAMPLER_REFERENCE, texture.sampler())?;
        let render = preview.begin_geometry_item("texture", texture.source_index())?;
        preview.geometry_field_usize(render, "image", texture.image());
        preview.geometry_field_usize(render, "sampler", texture.sampler());
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_materials(
    cursor: &mut NodeSceneAppearanceCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(transcript, preview, work, TAG_MATERIALS, "materials", count)?;
    for _ in 0..count {
        work.observe()?;
        let material = cursor
            .next_material()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "material collection ended before declared total",
            })?;
        transcript.write_tag(TAG_MATERIAL)?;
        transcript.tagged_usize(TAG_REFERENCE, material.source_index())?;
        transcript.tagged_usize(TAG_BINDING_OFFSET, material.binding_offset())?;
        transcript.tagged_usize(TAG_COLLECTION_TOTAL, material.binding_count())?;
        let render = preview.begin_geometry_item("material", material.source_index())?;
        preview.geometry_field_usize(render, "binding_offset", material.binding_offset());
        preview.geometry_field_usize(render, "binding_count", material.binding_count());
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_bindings(
    cursor: &mut NodeSceneAppearanceCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(transcript, preview, work, TAG_BINDINGS, "bindings", count)?;
    for _ in 0..count {
        work.observe()?;
        let binding = cursor
            .next_binding()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "material-binding collection ended before declared total",
            })?;
        let role = encode_texture_role(binding.role())?;
        transcript.write_tag(TAG_BINDING)?;
        transcript.tagged_usize(TAG_REFERENCE, binding.source_index())?;
        transcript.tagged_usize(TAG_TEXTURE_REFERENCE, binding.texture())?;
        transcript.tagged_u8(TAG_TEXTURE_ROLE, role)?;
        let render = preview.begin_geometry_item("binding", binding.source_index())?;
        preview.geometry_field_usize(render, "texture", binding.texture());
        preview.geometry_field_u32(render, "role", u32::from(role));
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_metered_bytes(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    tag: u8,
    bytes: &[u8],
) -> Result<(), NodeSceneSnapshotError> {
    transcript.write_tag(tag)?;
    let length =
        u64::try_from(bytes.len()).map_err(|_| NodeSceneSnapshotError::TranscriptLengthOverflow)?;
    transcript.write_bytes(&length.to_le_bytes())?;
    for chunk in bytes.chunks(SNAPSHOT_PAYLOAD_CHUNK_BYTES) {
        work.observe()?;
        transcript.write_bytes(chunk)?;
    }
    Ok(())
}

fn add_bytes(total: u64, count: usize, name: &'static str) -> Result<u64, NodeSceneSnapshotError> {
    total
        .checked_add(
            u64::try_from(count)
                .map_err(|_| NodeSceneSnapshotError::SemanticTotalOverflow { total: name })?,
        )
        .ok_or(NodeSceneSnapshotError::SemanticTotalOverflow { total: name })
}

const fn encode_texel_filter(value: TexelFilter) -> u8 {
    match value {
        TexelFilter::Nearest => 0,
        TexelFilter::Linear => 1,
    }
}

const fn encode_mipmap_filter(value: MipmapFilter) -> u8 {
    match value {
        MipmapFilter::None => 0,
        MipmapFilter::Nearest => 1,
        MipmapFilter::Linear => 2,
    }
}

const fn encode_address_mode(value: TextureAddressMode) -> u8 {
    match value {
        TextureAddressMode::Repeat => 0,
        TextureAddressMode::MirroredRepeat => 1,
        TextureAddressMode::ClampToEdge => 2,
        TextureAddressMode::ClampToBorder => 3,
    }
}

fn encode_texture_role(value: TextureRoleKind) -> Result<u8, NodeSceneSnapshotError> {
    match value {
        TextureRoleKind::BaseColor => Ok(0),
        TextureRoleKind::Diffuse => Ok(1),
        TextureRoleKind::Specular => Ok(2),
        TextureRoleKind::Metallic => Ok(3),
        TextureRoleKind::Roughness => Ok(4),
        TextureRoleKind::MetallicRoughness => Ok(5),
        TextureRoleKind::Normal => Ok(6),
        TextureRoleKind::Occlusion => Ok(7),
        TextureRoleKind::Emissive => Ok(8),
        TextureRoleKind::Opacity => Ok(9),
        TextureRoleKind::Height => Ok(10),
        TextureRoleKind::Displacement => Ok(11),
        TextureRoleKind::Lightmap => Ok(12),
        TextureRoleKind::Clearcoat => Ok(13),
        TextureRoleKind::ClearcoatRoughness => Ok(14),
        TextureRoleKind::ClearcoatNormal => Ok(15),
        TextureRoleKind::Transmission => Ok(16),
        TextureRoleKind::Thickness => Ok(17),
        TextureRoleKind::SheenColor => Ok(18),
        TextureRoleKind::SheenRoughness => Ok(19),
        TextureRoleKind::Namespaced => Err(unsupported("namespaced texture role")),
    }
}

fn preview_sampler(preview: &mut PreviewSink, render: bool, state: SamplerState) {
    preview.geometry_field_word(render, "min", texel_filter_name(state.minification()));
    preview.geometry_field_word(render, "mag", texel_filter_name(state.magnification()));
    preview.geometry_field_word(render, "mipmap", mipmap_filter_name(state.mipmap()));
    preview.geometry_field_word(render, "u", address_mode_name(state.address_u()));
    preview.geometry_field_word(render, "v", address_mode_name(state.address_v()));
    preview.geometry_field_word(render, "w", address_mode_name(state.address_w()));
}

fn texel_filter_name(value: TexelFilter) -> &'static str {
    match value {
        TexelFilter::Nearest => "nearest",
        TexelFilter::Linear => "linear",
    }
}

fn mipmap_filter_name(value: MipmapFilter) -> &'static str {
    match value {
        MipmapFilter::None => "none",
        MipmapFilter::Nearest => "nearest",
        MipmapFilter::Linear => "linear",
    }
}

fn address_mode_name(value: TextureAddressMode) -> &'static str {
    match value {
        TextureAddressMode::Repeat => "repeat",
        TextureAddressMode::MirroredRepeat => "mirrored_repeat",
        TextureAddressMode::ClampToEdge => "clamp_to_edge",
        TextureAddressMode::ClampToBorder => "clamp_to_border",
    }
}

const fn unsupported(reason: &'static str) -> NodeSceneSnapshotError {
    NodeSceneSnapshotError::InconsistentView { reason }
}
