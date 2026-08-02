use crate::entity::{CameraProjectionKind, LightKindCategory};
use crate::scene_space::{Axis, FrontFaceWinding, Handedness, TransformConvention, UvOrigin};

use super::super::semantic::{
    NodeSceneSemanticView, NodeSceneSpatialSceneSpace, NodeSceneSpatialTangentBasis,
    NodeSceneSpatialTangentEncoding,
};
use super::{
    CanonicalDigestEncoder, NodeSceneSnapshotError, PRESENCE_ABSENT, PRESENCE_PRESENT, PreviewSink,
    SnapshotWorkMeter, TAG_COLLECTION_TOTAL, TAG_PRESENCE, TAG_REFERENCE, map_snapshot_invariant,
};

const TAG_SPATIAL: u8 = 166;
const TAG_SCENE_SPACE: u8 = 167;
const TAG_HANDEDNESS: u8 = 168;
const TAG_UP_AXIS: u8 = 169;
const TAG_FRONT_AXIS: u8 = 170;
const TAG_UNIT_SCALE: u8 = 171;
const TAG_FRONT_FACE_WINDING: u8 = 172;
const TAG_UV_ORIGIN: u8 = 173;
const TAG_TRANSFORM_CONVENTION: u8 = 174;
const TAG_TANGENT_BASIS: u8 = 175;
const TAG_SCHEMA_NAME: u8 = 176;
const TAG_SCHEMA_VERSION: u8 = 177;
const TAG_TANGENT_ENCODING: u8 = 178;
const TAG_CAMERAS: u8 = 179;
const TAG_CAMERA: u8 = 180;
const TAG_PROJECTION: u8 = 181;
const TAG_PRIMARY: u8 = 182;
const TAG_SECONDARY: u8 = 183;
const TAG_NEAR: u8 = 184;
const TAG_FAR: u8 = 185;
const TAG_LIGHTS: u8 = 186;
const TAG_LIGHT: u8 = 187;
const TAG_LIGHT_KIND: u8 = 188;
const TAG_RANGE: u8 = 189;
const TAG_INNER_CONE: u8 = 190;
const TAG_OUTER_CONE: u8 = 191;
const TAG_TINT_RED: u8 = 192;
const TAG_TINT_GREEN: u8 = 193;
const TAG_TINT_BLUE: u8 = 194;
const TAG_INTENSITY: u8 = 195;
pub(super) const TAG_CAMERA_ATTACHMENT: u8 = 196;
pub(super) const TAG_LIGHT_ATTACHMENT: u8 = 197;
const TAG_F64_BITS: u8 = 198;
const TAG_F32_BITS: u8 = 199;

const HANDEDNESS_UNKNOWN: u8 = 0;
const HANDEDNESS_RIGHT: u8 = 1;
const HANDEDNESS_LEFT: u8 = 2;
const AXIS_UNKNOWN: u8 = 0;
const AXIS_POSITIVE_X: u8 = 1;
const AXIS_NEGATIVE_X: u8 = 2;
const AXIS_POSITIVE_Y: u8 = 3;
const AXIS_NEGATIVE_Y: u8 = 4;
const AXIS_POSITIVE_Z: u8 = 5;
const AXIS_NEGATIVE_Z: u8 = 6;
const WINDING_UNKNOWN: u8 = 0;
const WINDING_CLOCKWISE: u8 = 1;
const WINDING_COUNTER_CLOCKWISE: u8 = 2;
const UV_ORIGIN_UNKNOWN: u8 = 0;
const UV_ORIGIN_UPPER_LEFT: u8 = 1;
const UV_ORIGIN_LOWER_LEFT: u8 = 2;
const TRANSFORM_UNKNOWN: u8 = 0;
const TRANSFORM_COLUMN_VECTOR_LOCAL_TO_PARENT: u8 = 1;
const TANGENT_BASIS_UNKNOWN: u8 = 0;
const TANGENT_BASIS_MIKKTSPACE: u8 = 1;
const TANGENT_BASIS_NAMESPACED: u8 = 2;
const TANGENT_ENCODING_UNKNOWN: u8 = 0;
const TANGENT_ENCODING_SIGN_IN_W: u8 = 1;
const TANGENT_ENCODING_NAMESPACED: u8 = 2;
const CAMERA_PERSPECTIVE: u8 = 0;
const CAMERA_ORTHOGRAPHIC: u8 = 1;
const LIGHT_DIRECTIONAL: u8 = 0;
const LIGHT_POINT: u8 = 1;
const LIGHT_SPOT: u8 = 2;

#[derive(Clone, Copy)]
pub(super) struct SpatialCollectionCounts {
    pub(super) cameras: usize,
    pub(super) lights: usize,
}

pub(super) fn write_spatial(
    source: &NodeSceneSemanticView<'_>,
    counts: SpatialCollectionCounts,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(TAG_SPATIAL)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.push_str("spatial(present)\n");
    write_scene_space(
        source.scene_space().map_err(map_snapshot_invariant)?,
        transcript,
        preview,
        work,
    )?;

    work.observe()?;
    transcript.write_tag(TAG_CAMERAS)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.begin_geometry_collection("cameras", counts.cameras);
    for source_index in 0..counts.cameras {
        work.observe()?;
        let camera = source
            .camera_at(source_index)
            .map_err(map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "camera collection ended before declared total",
            })?;
        transcript.write_tag(TAG_CAMERA)?;
        transcript.tagged_usize(TAG_REFERENCE, camera.source_index())?;
        let render = preview.begin_geometry_item("camera", source_index)?;
        let projection = camera.projection();
        match projection.kind() {
            CameraProjectionKind::Perspective => {
                let (primary, secondary, near, far) = projection.perspective_parameters().ok_or(
                    NodeSceneSnapshotError::InconsistentView {
                        reason: "perspective camera lost its canonical parameters",
                    },
                )?;
                transcript.tagged_u8(TAG_PROJECTION, CAMERA_PERSPECTIVE)?;
                preview.geometry_field_word(render, "projection", "perspective");
                write_f32(
                    TAG_PRIMARY,
                    primary,
                    transcript,
                    preview,
                    work,
                    render,
                    "vertical_fov",
                )?;
                write_optional_f32(
                    TAG_SECONDARY,
                    secondary,
                    transcript,
                    preview,
                    work,
                    render,
                    "aspect_ratio",
                )?;
                write_f32(
                    TAG_NEAR,
                    near,
                    transcript,
                    preview,
                    work,
                    render,
                    "near_plane",
                )?;
                write_optional_f32(TAG_FAR, far, transcript, preview, work, render, "far_plane")?;
            }
            CameraProjectionKind::Orthographic => {
                let (primary, secondary, near, far) = projection.orthographic_parameters().ok_or(
                    NodeSceneSnapshotError::InconsistentView {
                        reason: "orthographic camera lost its canonical parameters",
                    },
                )?;
                transcript.tagged_u8(TAG_PROJECTION, CAMERA_ORTHOGRAPHIC)?;
                preview.geometry_field_word(render, "projection", "orthographic");
                write_f32(
                    TAG_PRIMARY,
                    primary,
                    transcript,
                    preview,
                    work,
                    render,
                    "horizontal_half_extent",
                )?;
                write_optional_f32(
                    TAG_SECONDARY,
                    Some(secondary),
                    transcript,
                    preview,
                    work,
                    render,
                    "vertical_half_extent",
                )?;
                write_f32(
                    TAG_NEAR,
                    near,
                    transcript,
                    preview,
                    work,
                    render,
                    "near_plane",
                )?;
                write_optional_f32(
                    TAG_FAR,
                    Some(far),
                    transcript,
                    preview,
                    work,
                    render,
                    "far_plane",
                )?;
            }
        }
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, counts.cameras)?;

    work.observe()?;
    transcript.write_tag(TAG_LIGHTS)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.begin_geometry_collection("lights", counts.lights);
    for source_index in 0..counts.lights {
        work.observe()?;
        let light = source
            .light_at(source_index)
            .map_err(map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "light collection ended before declared total",
            })?;
        transcript.write_tag(TAG_LIGHT)?;
        transcript.tagged_usize(TAG_REFERENCE, light.source_index())?;
        let render = preview.begin_geometry_item("light", source_index)?;
        let light_kind = light.kind();
        let (kind, range, inner, outer, kind_name) = match light_kind.category() {
            LightKindCategory::Directional => (LIGHT_DIRECTIONAL, None, None, None, "directional"),
            LightKindCategory::Point => (LIGHT_POINT, light_kind.range(), None, None, "point"),
            LightKindCategory::Spot => (
                LIGHT_SPOT,
                light_kind.range(),
                light_kind.inner_cone_radians(),
                light_kind.outer_cone_radians(),
                "spot",
            ),
        };
        transcript.tagged_u8(TAG_LIGHT_KIND, kind)?;
        preview.geometry_field_word(render, "kind", kind_name);
        write_optional_f32(TAG_RANGE, range, transcript, preview, work, render, "range")?;
        write_optional_f32(
            TAG_INNER_CONE,
            inner,
            transcript,
            preview,
            work,
            render,
            "inner_cone",
        )?;
        write_optional_f32(
            TAG_OUTER_CONE,
            outer,
            transcript,
            preview,
            work,
            render,
            "outer_cone",
        )?;
        let tint = light.tint();
        write_f32(
            TAG_TINT_RED,
            tint.x,
            transcript,
            preview,
            work,
            render,
            "tint_r",
        )?;
        write_f32(
            TAG_TINT_GREEN,
            tint.y,
            transcript,
            preview,
            work,
            render,
            "tint_g",
        )?;
        write_f32(
            TAG_TINT_BLUE,
            tint.z,
            transcript,
            preview,
            work,
            render,
            "tint_b",
        )?;
        write_f32(
            TAG_INTENSITY,
            light.intensity(),
            transcript,
            preview,
            work,
            render,
            "intensity",
        )?;
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, counts.lights)?;
    preview.push_str("spatial_end\n");
    Ok(())
}

pub(super) fn write_node_attachment(
    tag: u8,
    name: &'static str,
    value: Option<usize>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    render: bool,
) -> Result<(), NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(tag)?;
    transcript.tagged_u8(
        TAG_PRESENCE,
        if value.is_some() {
            PRESENCE_PRESENT
        } else {
            PRESENCE_ABSENT
        },
    )?;
    if let Some(value) = value {
        transcript.tagged_usize(TAG_REFERENCE, value)?;
    }
    preview.geometry_optional_reference(render, name, value);
    Ok(())
}

fn write_scene_space(
    scene_space: NodeSceneSpatialSceneSpace<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(TAG_SCENE_SPACE)?;
    transcript.tagged_u8(TAG_HANDEDNESS, encode_handedness(scene_space.handedness()))?;
    transcript.tagged_u8(TAG_UP_AXIS, encode_axis(scene_space.up_axis()))?;
    transcript.tagged_u8(TAG_FRONT_AXIS, encode_axis(scene_space.front_axis()))?;
    transcript.write_tag(TAG_UNIT_SCALE)?;
    transcript.tagged_u8(
        TAG_PRESENCE,
        if scene_space.unit_scale_to_meters().is_some() {
            PRESENCE_PRESENT
        } else {
            PRESENCE_ABSENT
        },
    )?;
    if let Some(scale) = scene_space.unit_scale_to_meters() {
        transcript.tagged_u64(TAG_F64_BITS, scale.to_bits())?;
    }
    transcript.tagged_u8(
        TAG_FRONT_FACE_WINDING,
        encode_winding(scene_space.front_face_winding()),
    )?;
    transcript.tagged_u8(TAG_UV_ORIGIN, encode_uv_origin(scene_space.uv_origin()))?;
    transcript.tagged_u8(
        TAG_TRANSFORM_CONVENTION,
        encode_transform(scene_space.transform_convention()),
    )?;
    preview.push_str("scene_space handedness=");
    preview.push_str(handedness_name(scene_space.handedness()));
    preview.push_str(" up_axis=");
    preview.push_str(axis_name(scene_space.up_axis()));
    preview.push_str(" front_axis=");
    preview.push_str(axis_name(scene_space.front_axis()));
    preview.push_str(" unit_scale_to_meters_bits=");
    match scene_space.unit_scale_to_meters() {
        Some(value) => push_hex_u64(preview, value.to_bits()),
        None => preview.push_str("absent"),
    }
    preview.push_str(" front_face_winding=");
    preview.push_str(winding_name(scene_space.front_face_winding()));
    preview.push_str(" uv_origin=");
    preview.push_str(uv_origin_name(scene_space.uv_origin()));
    preview.push_str(" transform_convention=");
    preview.push_str(transform_name(scene_space.transform_convention()));
    write_tangent_basis(scene_space.tangent_basis(), transcript, preview)?;
    write_tangent_encoding(scene_space.tangent_encoding(), transcript, preview)?;
    preview.push_str("\n");
    Ok(())
}

fn write_tangent_basis(
    value: NodeSceneSpatialTangentBasis<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
) -> Result<(), NodeSceneSnapshotError> {
    let (tag, name, schema) = match value {
        NodeSceneSpatialTangentBasis::Unknown => (TANGENT_BASIS_UNKNOWN, "unknown", None),
        NodeSceneSpatialTangentBasis::MikkTspace => (TANGENT_BASIS_MIKKTSPACE, "mikk_tspace", None),
        NodeSceneSpatialTangentBasis::Namespaced(schema) => {
            (TANGENT_BASIS_NAMESPACED, "namespaced", Some(schema))
        }
    };
    transcript.tagged_u8(TAG_TANGENT_BASIS, tag)?;
    preview.push_str(" tangent_basis=");
    preview.push_str(name);
    if let Some(schema) = schema {
        transcript.tagged_bytes(TAG_SCHEMA_NAME, schema.name().as_bytes())?;
        transcript.tagged_u32(TAG_SCHEMA_VERSION, schema.version().get())?;
        preview.push_str(" tangent_basis_schema_name=");
        preview.push_str(schema.name());
        preview.push_str(" tangent_basis_schema_version=");
        preview.push_u64(u64::from(schema.version().get()));
    }
    Ok(())
}

fn write_tangent_encoding(
    value: NodeSceneSpatialTangentEncoding<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
) -> Result<(), NodeSceneSnapshotError> {
    let (tag, name, schema) = match value {
        NodeSceneSpatialTangentEncoding::Unknown => (TANGENT_ENCODING_UNKNOWN, "unknown", None),
        NodeSceneSpatialTangentEncoding::SignInW => (TANGENT_ENCODING_SIGN_IN_W, "sign_in_w", None),
        NodeSceneSpatialTangentEncoding::Namespaced(schema) => {
            (TANGENT_ENCODING_NAMESPACED, "namespaced", Some(schema))
        }
    };
    transcript.tagged_u8(TAG_TANGENT_ENCODING, tag)?;
    preview.push_str(" tangent_handedness_encoding=");
    preview.push_str(name);
    if let Some(schema) = schema {
        transcript.tagged_bytes(TAG_SCHEMA_NAME, schema.name().as_bytes())?;
        transcript.tagged_u32(TAG_SCHEMA_VERSION, schema.version().get())?;
        preview.push_str(" tangent_handedness_encoding_schema_name=");
        preview.push_str(schema.name());
        preview.push_str(" tangent_handedness_encoding_schema_version=");
        preview.push_u64(u64::from(schema.version().get()));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_f32(
    tag: u8,
    value: f32,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    render: bool,
    name: &'static str,
) -> Result<(), NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(tag)?;
    transcript.tagged_u32(TAG_F32_BITS, value.to_bits())?;
    preview_f32_bits(preview, render, name, value.to_bits());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_optional_f32(
    tag: u8,
    value: Option<f32>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    render: bool,
    name: &'static str,
) -> Result<(), NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(tag)?;
    transcript.tagged_u8(
        TAG_PRESENCE,
        if value.is_some() {
            PRESENCE_PRESENT
        } else {
            PRESENCE_ABSENT
        },
    )?;
    if let Some(value) = value {
        transcript.tagged_u32(TAG_F32_BITS, value.to_bits())?;
        preview_f32_bits(preview, render, name, value.to_bits());
    } else {
        preview.geometry_field_word(render, name, "absent");
    }
    Ok(())
}

fn preview_f32_bits(preview: &mut PreviewSink, render: bool, name: &'static str, bits: u32) {
    if render {
        preview.geometry_field_word(true, name, "");
        preview.push_hex_u32(bits);
    }
}

fn push_hex_u64(preview: &mut PreviewSink, value: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = *b"0x0000000000000000";
    for index in 0..16 {
        let shift = 60 - index * 4;
        bytes[index + 2] = HEX[((value >> shift) & 0x0f) as usize];
    }
    preview.push_str(std::str::from_utf8(&bytes).unwrap_or(""));
}

const fn handedness_name(value: Handedness) -> &'static str {
    match value {
        Handedness::Unknown => "unknown",
        Handedness::Right => "right",
        Handedness::Left => "left",
    }
}

const fn axis_name(value: Axis) -> &'static str {
    match value {
        Axis::Unknown => "unknown",
        Axis::PositiveX => "positive_x",
        Axis::NegativeX => "negative_x",
        Axis::PositiveY => "positive_y",
        Axis::NegativeY => "negative_y",
        Axis::PositiveZ => "positive_z",
        Axis::NegativeZ => "negative_z",
    }
}

const fn winding_name(value: FrontFaceWinding) -> &'static str {
    match value {
        FrontFaceWinding::Unknown => "unknown",
        FrontFaceWinding::Clockwise => "clockwise",
        FrontFaceWinding::CounterClockwise => "counter_clockwise",
    }
}

const fn uv_origin_name(value: UvOrigin) -> &'static str {
    match value {
        UvOrigin::Unknown => "unknown",
        UvOrigin::UpperLeft => "upper_left",
        UvOrigin::LowerLeft => "lower_left",
    }
}

const fn transform_name(value: TransformConvention) -> &'static str {
    match value {
        TransformConvention::Unknown => "unknown",
        TransformConvention::ColumnVectorLocalToParent => "column_vector_local_to_parent",
    }
}

const fn encode_handedness(value: Handedness) -> u8 {
    match value {
        Handedness::Unknown => HANDEDNESS_UNKNOWN,
        Handedness::Right => HANDEDNESS_RIGHT,
        Handedness::Left => HANDEDNESS_LEFT,
    }
}
const fn encode_axis(value: Axis) -> u8 {
    match value {
        Axis::Unknown => AXIS_UNKNOWN,
        Axis::PositiveX => AXIS_POSITIVE_X,
        Axis::NegativeX => AXIS_NEGATIVE_X,
        Axis::PositiveY => AXIS_POSITIVE_Y,
        Axis::NegativeY => AXIS_NEGATIVE_Y,
        Axis::PositiveZ => AXIS_POSITIVE_Z,
        Axis::NegativeZ => AXIS_NEGATIVE_Z,
    }
}
const fn encode_winding(value: FrontFaceWinding) -> u8 {
    match value {
        FrontFaceWinding::Unknown => WINDING_UNKNOWN,
        FrontFaceWinding::Clockwise => WINDING_CLOCKWISE,
        FrontFaceWinding::CounterClockwise => WINDING_COUNTER_CLOCKWISE,
    }
}
const fn encode_uv_origin(value: UvOrigin) -> u8 {
    match value {
        UvOrigin::Unknown => UV_ORIGIN_UNKNOWN,
        UvOrigin::UpperLeft => UV_ORIGIN_UPPER_LEFT,
        UvOrigin::LowerLeft => UV_ORIGIN_LOWER_LEFT,
    }
}
const fn encode_transform(value: TransformConvention) -> u8 {
    match value {
        TransformConvention::Unknown => TRANSFORM_UNKNOWN,
        TransformConvention::ColumnVectorLocalToParent => TRANSFORM_COLUMN_VECTOR_LOCAL_TO_PARENT,
    }
}
