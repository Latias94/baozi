//! Private canonical Scene spatial-profile schema.

use baozi_operation::{RecordBlock, Utf8Block};

use super::CanonicalSchema;
use crate::entity::{CameraProjection, CameraProjectionKind, LightKind, LightKindCategory};
use crate::scene_space::{
    Axis, FrontFaceWinding, Handedness, TangentBasisConvention, TangentHandednessEncoding,
    TransformConvention, UvOrigin,
};

pub(in super::super) const SPATIAL_SCHEMA_REVISION: u32 = 1;
pub(in super::super) const SPATIAL_DIRECTORY_FIELD_COUNT: usize = 23;
pub(in super::super) const SPATIAL_CAMERA_FIELD_COUNT: usize = 8;
pub(in super::super) const SPATIAL_LIGHT_FIELD_COUNT: usize = 12;

pub(in super::super) const SPATIAL_HANDEDNESS_UNKNOWN: u8 = 0;
pub(in super::super) const SPATIAL_HANDEDNESS_RIGHT: u8 = 1;
pub(in super::super) const SPATIAL_HANDEDNESS_LEFT: u8 = 2;

pub(in super::super) const SPATIAL_AXIS_UNKNOWN: u8 = 0;
pub(in super::super) const SPATIAL_AXIS_POSITIVE_X: u8 = 1;
pub(in super::super) const SPATIAL_AXIS_NEGATIVE_X: u8 = 2;
pub(in super::super) const SPATIAL_AXIS_POSITIVE_Y: u8 = 3;
pub(in super::super) const SPATIAL_AXIS_NEGATIVE_Y: u8 = 4;
pub(in super::super) const SPATIAL_AXIS_POSITIVE_Z: u8 = 5;
pub(in super::super) const SPATIAL_AXIS_NEGATIVE_Z: u8 = 6;

pub(in super::super) const SPATIAL_WINDING_UNKNOWN: u8 = 0;
pub(in super::super) const SPATIAL_WINDING_CLOCKWISE: u8 = 1;
pub(in super::super) const SPATIAL_WINDING_COUNTER_CLOCKWISE: u8 = 2;

pub(in super::super) const SPATIAL_UV_ORIGIN_UNKNOWN: u8 = 0;
pub(in super::super) const SPATIAL_UV_ORIGIN_UPPER_LEFT: u8 = 1;
pub(in super::super) const SPATIAL_UV_ORIGIN_LOWER_LEFT: u8 = 2;

pub(in super::super) const SPATIAL_TRANSFORM_UNKNOWN: u8 = 0;
pub(in super::super) const SPATIAL_TRANSFORM_COLUMN_VECTOR_LOCAL_TO_PARENT: u8 = 1;

pub(in super::super) const SPATIAL_TANGENT_BASIS_UNKNOWN: u8 = 0;
pub(in super::super) const SPATIAL_TANGENT_BASIS_MIKKTSPACE: u8 = 1;
pub(in super::super) const SPATIAL_TANGENT_BASIS_NAMESPACED: u8 = 2;

pub(in super::super) const SPATIAL_TANGENT_ENCODING_UNKNOWN: u8 = 0;
pub(in super::super) const SPATIAL_TANGENT_ENCODING_SIGN_IN_W: u8 = 1;
pub(in super::super) const SPATIAL_TANGENT_ENCODING_NAMESPACED: u8 = 2;

pub(in super::super) const SPATIAL_CAMERA_PERSPECTIVE: u8 = 0;
pub(in super::super) const SPATIAL_CAMERA_ORTHOGRAPHIC: u8 = 1;

pub(in super::super) const SPATIAL_LIGHT_DIRECTIONAL: u8 = 0;
pub(in super::super) const SPATIAL_LIGHT_POINT: u8 = 1;
pub(in super::super) const SPATIAL_LIGHT_SPOT: u8 = 2;

pub(in super::super) const fn encode_handedness(value: Handedness) -> u8 {
    match value {
        Handedness::Unknown => SPATIAL_HANDEDNESS_UNKNOWN,
        Handedness::Right => SPATIAL_HANDEDNESS_RIGHT,
        Handedness::Left => SPATIAL_HANDEDNESS_LEFT,
    }
}

pub(in super::super) const fn encode_axis(value: Axis) -> u8 {
    match value {
        Axis::Unknown => SPATIAL_AXIS_UNKNOWN,
        Axis::PositiveX => SPATIAL_AXIS_POSITIVE_X,
        Axis::NegativeX => SPATIAL_AXIS_NEGATIVE_X,
        Axis::PositiveY => SPATIAL_AXIS_POSITIVE_Y,
        Axis::NegativeY => SPATIAL_AXIS_NEGATIVE_Y,
        Axis::PositiveZ => SPATIAL_AXIS_POSITIVE_Z,
        Axis::NegativeZ => SPATIAL_AXIS_NEGATIVE_Z,
    }
}

pub(in super::super) const fn encode_winding(value: FrontFaceWinding) -> u8 {
    match value {
        FrontFaceWinding::Unknown => SPATIAL_WINDING_UNKNOWN,
        FrontFaceWinding::Clockwise => SPATIAL_WINDING_CLOCKWISE,
        FrontFaceWinding::CounterClockwise => SPATIAL_WINDING_COUNTER_CLOCKWISE,
    }
}

pub(in super::super) const fn encode_uv_origin(value: UvOrigin) -> u8 {
    match value {
        UvOrigin::Unknown => SPATIAL_UV_ORIGIN_UNKNOWN,
        UvOrigin::UpperLeft => SPATIAL_UV_ORIGIN_UPPER_LEFT,
        UvOrigin::LowerLeft => SPATIAL_UV_ORIGIN_LOWER_LEFT,
    }
}

pub(in super::super) const fn encode_transform_convention(value: TransformConvention) -> u8 {
    match value {
        TransformConvention::Unknown => SPATIAL_TRANSFORM_UNKNOWN,
        TransformConvention::ColumnVectorLocalToParent => {
            SPATIAL_TRANSFORM_COLUMN_VECTOR_LOCAL_TO_PARENT
        }
    }
}

pub(in super::super) const fn encode_tangent_basis(value: &TangentBasisConvention) -> u8 {
    match value {
        TangentBasisConvention::Unknown => SPATIAL_TANGENT_BASIS_UNKNOWN,
        TangentBasisConvention::MikkTspace => SPATIAL_TANGENT_BASIS_MIKKTSPACE,
        TangentBasisConvention::Namespaced { .. } => SPATIAL_TANGENT_BASIS_NAMESPACED,
    }
}

pub(in super::super) const fn encode_tangent_encoding(value: &TangentHandednessEncoding) -> u8 {
    match value {
        TangentHandednessEncoding::Unknown => SPATIAL_TANGENT_ENCODING_UNKNOWN,
        TangentHandednessEncoding::SignInW => SPATIAL_TANGENT_ENCODING_SIGN_IN_W,
        TangentHandednessEncoding::Namespaced { .. } => SPATIAL_TANGENT_ENCODING_NAMESPACED,
    }
}

pub(in super::super) const fn encode_camera_projection(value: CameraProjection) -> u8 {
    match value.kind() {
        CameraProjectionKind::Perspective => SPATIAL_CAMERA_PERSPECTIVE,
        CameraProjectionKind::Orthographic => SPATIAL_CAMERA_ORTHOGRAPHIC,
    }
}

pub(in super::super) const fn encode_light_kind(value: LightKind) -> u8 {
    match value.category() {
        LightKindCategory::Directional => SPATIAL_LIGHT_DIRECTIONAL,
        LightKindCategory::Point => SPATIAL_LIGHT_POINT,
        LightKindCategory::Spot => SPATIAL_LIGHT_SPOT,
    }
}

pub(in super::super) mod spatial_directory_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const HANDEDNESS: usize = 1;
    pub(crate) const UP_AXIS: usize = 2;
    pub(crate) const FRONT_AXIS: usize = 3;
    pub(crate) const UNIT_SCALE_PRESENCE: usize = 4;
    pub(crate) const UNIT_SCALE_TO_METERS: usize = 5;
    pub(crate) const FRONT_FACE_WINDING: usize = 6;
    pub(crate) const UV_ORIGIN: usize = 7;
    pub(crate) const TRANSFORM_CONVENTION: usize = 8;
    pub(crate) const TANGENT_BASIS: usize = 9;
    pub(crate) const TANGENT_BASIS_SCHEMA_NAME_COUNT: usize = 10;
    pub(crate) const TANGENT_BASIS_SCHEMA_NAME: usize = 11;
    pub(crate) const TANGENT_BASIS_SCHEMA_VERSION: usize = 12;
    pub(crate) const TANGENT_ENCODING: usize = 13;
    pub(crate) const TANGENT_ENCODING_SCHEMA_NAME_COUNT: usize = 14;
    pub(crate) const TANGENT_ENCODING_SCHEMA_NAME: usize = 15;
    pub(crate) const TANGENT_ENCODING_SCHEMA_VERSION: usize = 16;
    pub(crate) const CAMERA_COUNT: usize = 17;
    pub(crate) const CAMERAS_PRESENCE: usize = 18;
    pub(crate) const CAMERAS: usize = 19;
    pub(crate) const LIGHT_COUNT: usize = 20;
    pub(crate) const LIGHTS_PRESENCE: usize = 21;
    pub(crate) const LIGHTS: usize = 22;
}

pub(in super::super) mod spatial_camera_field {
    pub(crate) const CAMERA: usize = 0;
    pub(crate) const PROJECTION: usize = 1;
    pub(crate) const PRIMARY: usize = 2;
    pub(crate) const SECONDARY_PRESENCE: usize = 3;
    pub(crate) const SECONDARY: usize = 4;
    pub(crate) const NEAR: usize = 5;
    pub(crate) const FAR_PRESENCE: usize = 6;
    pub(crate) const FAR: usize = 7;
}

pub(in super::super) mod spatial_light_field {
    pub(crate) const LIGHT: usize = 0;
    pub(crate) const KIND: usize = 1;
    pub(crate) const RANGE_PRESENCE: usize = 2;
    pub(crate) const RANGE: usize = 3;
    pub(crate) const INNER_CONE_PRESENCE: usize = 4;
    pub(crate) const INNER_CONE: usize = 5;
    pub(crate) const OUTER_CONE_PRESENCE: usize = 6;
    pub(crate) const OUTER_CONE: usize = 7;
    pub(crate) const TINT_RED: usize = 8;
    pub(crate) const TINT_GREEN: usize = 9;
    pub(crate) const TINT_BLUE: usize = 10;
    pub(crate) const INTENSITY: usize = 11;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct SpatialDirectorySchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct SpatialCameraTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct SpatialLightTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct SpatialTangentBasisSchemaNameSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct SpatialTangentEncodingSchemaNameSchema;

impl CanonicalSchema for SpatialDirectorySchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for SpatialCameraTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for SpatialLightTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for SpatialTangentBasisSchemaNameSchema {
    type StoreKind = Utf8Block;
}

impl CanonicalSchema for SpatialTangentEncodingSchemaNameSchema {
    type StoreKind = Utf8Block;
}
