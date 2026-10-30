use crate::appearance::{
    AlphaMode, ColorInterpretation, ShadingModel, TextureBlendOperation, TextureChannel,
    TextureCoordinateSource, TextureRole,
};
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
const TAG_SHADING_MODEL: u8 = 142;
const TAG_ALPHA_MODE: u8 = 143;
const TAG_ALPHA_CUTOFF_BITS: u8 = 144;
const TAG_DOUBLE_SIDED: u8 = 145;
const TAG_PBR_METALLIC_ROUGHNESS_PRESENCE: u8 = 146;
const TAG_PBR_SPECULAR_GLOSSINESS_PRESENCE: u8 = 147;
const TAG_LEGACY_PRESENCE: u8 = 148;
const TAG_FACTOR_OFFSET: u8 = 149;
const TAG_FACTOR_VALUES: u8 = 150;
const TAG_COORDINATE_SOURCE: u8 = 151;
const TAG_COORDINATE_SET: u8 = 152;
const TAG_CHANNEL_LANE_COUNT: u8 = 153;
const TAG_CHANNEL_0: u8 = 154;
const TAG_CHANNEL_1: u8 = 155;
const TAG_CHANNEL_2: u8 = 156;
const TAG_CHANNEL_3: u8 = 157;
const TAG_COLOR_INTERPRETATION: u8 = 158;
const TAG_SEMANTIC_SCALAR_OFFSET: u8 = 159;
const TAG_SEMANTIC_SCALAR_VALUES: u8 = 160;
const TAG_BLEND_OPERATION: u8 = 161;
const TAG_SOURCE_KEY: u8 = 162;
const TAG_SOURCE_KEY_OFFSET: u8 = 163;
const TAG_SOURCE_KEY_BYTES: u8 = 164;
const TAG_F32_BITS: u8 = 165;
const TAG_MATERIAL_NAME: u8 = 166;
const TAG_MATERIAL_NAME_BYTES: u8 = 167;

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
    pub(super) source_key_bytes: u64,
    pub(super) binding_semantic_scalars: u64,
    pub(super) material_factor_scalars: u64,
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
    write_materials(
        cursor,
        counts.materials,
        transcript,
        preview,
        work,
        &mut totals,
    )?;
    write_bindings(
        cursor,
        counts.bindings,
        transcript,
        preview,
        work,
        &mut totals,
    )?;
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
    totals: &mut AppearancePayloadTotals,
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
        let shading_model = encode_shading_model(*material.shading_model());
        let alpha_mode = encode_alpha_mode(*material.alpha_mode());
        let alpha_cutoff = material.alpha_mode().mask_cutoff().unwrap_or(0.0);
        let factors = material.factors();
        let mut factor_values = material.factor_values();
        let factor_count = factor_values.len();
        transcript.write_tag(TAG_MATERIAL)?;
        transcript.tagged_usize(TAG_REFERENCE, material.source_index())?;
        transcript.tagged_u8(TAG_SHADING_MODEL, shading_model)?;
        transcript.tagged_u8(TAG_ALPHA_MODE, alpha_mode)?;
        transcript.tagged_u32(TAG_ALPHA_CUTOFF_BITS, alpha_cutoff.to_bits())?;
        transcript.tagged_u8(TAG_DOUBLE_SIDED, encode_presence(material.double_sided()))?;
        transcript.tagged_u8(
            TAG_PBR_METALLIC_ROUGHNESS_PRESENCE,
            encode_presence(factors.pbr_metallic_roughness().is_some()),
        )?;
        transcript.tagged_u8(
            TAG_PBR_SPECULAR_GLOSSINESS_PRESENCE,
            encode_presence(factors.pbr_specular_glossiness().is_some()),
        )?;
        transcript.tagged_u8(
            TAG_LEGACY_PRESENCE,
            encode_presence(factors.legacy().is_some()),
        )?;
        transcript.tagged_usize(TAG_BINDING_OFFSET, material.binding_offset())?;
        transcript.tagged_usize(TAG_COLLECTION_TOTAL, material.binding_count())?;
        transcript.tagged_usize(TAG_FACTOR_OFFSET, material.factor_offset())?;
        write_metered_f32_bits(transcript, work, TAG_FACTOR_VALUES, factor_values.by_ref())?;
        factor_values
            .finish()
            .map_err(super::map_snapshot_invariant)?;
        totals.material_factor_scalars = add_bytes(
            totals.material_factor_scalars,
            factor_count,
            "material factor scalars",
        )?;
        transcript.write_tag(TAG_MATERIAL_NAME)?;
        match material.name() {
            Some(name) => {
                transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
                write_metered_bytes(transcript, work, TAG_MATERIAL_NAME_BYTES, name.as_bytes())?;
            }
            None => transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?,
        }
        let render = preview.begin_geometry_item("material", material.source_index())?;
        preview.geometry_field_u32(render, "shading", u32::from(shading_model));
        preview.geometry_field_u32(render, "alpha", u32::from(alpha_mode));
        preview.geometry_field_word(
            render,
            "double_sided",
            if material.double_sided() {
                "true"
            } else {
                "false"
            },
        );
        preview.geometry_field_usize(render, "binding_offset", material.binding_offset());
        preview.geometry_field_usize(render, "binding_count", material.binding_count());
        preview.geometry_field_usize(render, "factor_offset", material.factor_offset());
        preview.geometry_field_usize(render, "factor_count", factor_count);
        preview.begin_geometry_leaf(render, "factors", true, factor_count);
        for (ordinal, value) in material.factor_values().enumerate() {
            preview.geometry_f32_bits(render, ordinal, value.to_bits())?;
        }
        preview.end_geometry_leaf(render);
        preview.geometry_field_word(render, "name", material.name().unwrap_or("absent"));
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn write_bindings(
    cursor: &mut NodeSceneAppearanceCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    totals: &mut AppearancePayloadTotals,
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
        let role = encode_texture_role(binding.role());
        let (coordinate_source, coordinate_set) =
            encode_coordinate_source(*binding.coordinate_source());
        let swizzle = binding.channel_mapping();
        let mut channels = [encode_channel(TextureChannel::Zero); 4];
        for (destination, channel) in channels.iter_mut().zip(swizzle.as_slice().iter().copied()) {
            *destination = encode_channel(channel);
        }
        let color_interpretation = encode_color_interpretation(*binding.color_interpretation());
        let blend = encode_blend_operation(*binding.blend())?;
        transcript.write_tag(TAG_BINDING)?;
        transcript.tagged_usize(TAG_REFERENCE, binding.source_index())?;
        transcript.tagged_usize(TAG_TEXTURE_REFERENCE, binding.texture())?;
        transcript.tagged_u8(TAG_TEXTURE_ROLE, role)?;
        transcript.tagged_u8(TAG_COORDINATE_SOURCE, coordinate_source)?;
        transcript.tagged_u32(TAG_COORDINATE_SET, coordinate_set)?;
        transcript.tagged_u8(TAG_CHANNEL_LANE_COUNT, swizzle.lane_count())?;
        transcript.tagged_u8(TAG_CHANNEL_0, channels[0])?;
        transcript.tagged_u8(TAG_CHANNEL_1, channels[1])?;
        transcript.tagged_u8(TAG_CHANNEL_2, channels[2])?;
        transcript.tagged_u8(TAG_CHANNEL_3, channels[3])?;
        transcript.tagged_u8(TAG_COLOR_INTERPRETATION, color_interpretation)?;
        transcript.tagged_usize(TAG_SEMANTIC_SCALAR_OFFSET, binding.semantic_offset())?;
        let mut semantic_values = binding.semantic_values();
        let semantic_count = semantic_values.len();
        write_metered_f32_bits(
            transcript,
            work,
            TAG_SEMANTIC_SCALAR_VALUES,
            semantic_values.by_ref(),
        )?;
        semantic_values
            .finish()
            .map_err(super::map_snapshot_invariant)?;
        totals.binding_semantic_scalars = add_bytes(
            totals.binding_semantic_scalars,
            semantic_count,
            "binding semantic scalars",
        )?;
        transcript.tagged_u8(TAG_BLEND_OPERATION, blend)?;
        transcript.tagged_usize(TAG_SOURCE_KEY_OFFSET, cursor.source_key_offset())?;
        transcript.write_tag(TAG_SOURCE_KEY)?;
        match binding.source_key() {
            Some(source_key) => {
                transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
                write_metered_bytes(
                    transcript,
                    work,
                    TAG_SOURCE_KEY_BYTES,
                    source_key.as_bytes(),
                )?;
                totals.source_key_bytes = add_bytes(
                    totals.source_key_bytes,
                    source_key.len(),
                    "source-key bytes",
                )?;
            }
            None => transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?,
        }
        let render = preview.begin_geometry_item("binding", binding.source_index())?;
        preview.geometry_field_usize(render, "texture", binding.texture());
        preview.geometry_field_u32(render, "role", u32::from(role));
        preview.geometry_field_u32(render, "coordinate", u32::from(coordinate_source));
        preview.geometry_field_u32(render, "coordinate_set", coordinate_set);
        preview.geometry_field_u32(render, "channels", u32::from(swizzle.lane_count()));
        preview.geometry_field_u32(render, "color", u32::from(color_interpretation));
        preview.geometry_field_usize(render, "semantic_offset", binding.semantic_offset());
        preview.geometry_field_word(
            render,
            "source_key",
            binding.source_key().unwrap_or("absent"),
        );
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

fn write_metered_f32_bits(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    tag: u8,
    values: impl ExactSizeIterator<Item = f32>,
) -> Result<(), NodeSceneSnapshotError> {
    transcript.write_tag(tag)?;
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, values.len())?;
    for value in values {
        work.observe()?;
        transcript.tagged_u32(TAG_F32_BITS, value.to_bits())?;
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

const fn encode_presence(value: bool) -> u8 {
    if value {
        PRESENCE_PRESENT
    } else {
        PRESENCE_ABSENT
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

const fn encode_shading_model(value: ShadingModel) -> u8 {
    match value {
        ShadingModel::Unspecified => 6,
        ShadingModel::Unlit => 0,
        ShadingModel::Lambert => 1,
        ShadingModel::Phong => 2,
        ShadingModel::BlinnPhong => 3,
        ShadingModel::PbrMetallicRoughness => 4,
        ShadingModel::PbrSpecularGlossiness => 5,
    }
}

const fn encode_alpha_mode(value: AlphaMode) -> u8 {
    match value {
        AlphaMode::Opaque => 0,
        AlphaMode::Mask(_) => 1,
        AlphaMode::Blend => 2,
    }
}

const fn encode_texture_role(value: TextureRole) -> u8 {
    match value {
        TextureRole::BaseColor => 0,
        TextureRole::Diffuse => 1,
        TextureRole::Specular => 2,
        TextureRole::Metallic => 3,
        TextureRole::Roughness => 4,
        TextureRole::MetallicRoughness => 5,
        TextureRole::Normal => 6,
        TextureRole::Occlusion => 7,
        TextureRole::Emissive => 8,
        TextureRole::Opacity => 9,
        TextureRole::Displacement => 10,
        TextureRole::Clearcoat => 11,
        TextureRole::ClearcoatRoughness => 12,
        TextureRole::ClearcoatNormal => 13,
        TextureRole::Transmission => 14,
        TextureRole::Thickness => 15,
        TextureRole::SheenColor => 16,
        TextureRole::SheenRoughness => 17,
    }
}

const fn encode_coordinate_source(value: TextureCoordinateSource) -> (u8, u32) {
    match value {
        TextureCoordinateSource::UvSet(set) => (0, set as u32),
        TextureCoordinateSource::ObjectPosition => (1, 0),
        TextureCoordinateSource::ObjectNormal => (2, 0),
        TextureCoordinateSource::ReflectionVector => (3, 0),
    }
}

const fn encode_channel(value: TextureChannel) -> u8 {
    match value {
        TextureChannel::Red => 0,
        TextureChannel::Green => 1,
        TextureChannel::Blue => 2,
        TextureChannel::Alpha => 3,
        TextureChannel::Zero => 4,
        TextureChannel::One => 5,
    }
}

const fn encode_color_interpretation(value: ColorInterpretation) -> u8 {
    match value {
        ColorInterpretation::SrgbColor => 0,
        ColorInterpretation::LinearColor => 1,
        ColorInterpretation::Data => 2,
    }
}

fn encode_blend_operation(value: TextureBlendOperation) -> Result<u8, NodeSceneSnapshotError> {
    match value {
        TextureBlendOperation::Replace => Ok(0),
        TextureBlendOperation::Multiply
        | TextureBlendOperation::Add
        | TextureBlendOperation::Subtract
        | TextureBlendOperation::Screen => Err(NodeSceneSnapshotError::InconsistentView {
            reason: "noncanonical texture blend operation",
        }),
    }
}
