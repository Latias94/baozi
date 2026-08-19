use baozi_operation::__retained_store_integration::activation::{
    CanonicalValidationMember, CanonicalValidationSession,
};
use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, CanonicalUtf8Ref, OperationOwner, ProgressPhase,
    ProgressUnit, RecordBlock, StoreValidationSession, StoreWorkError, Utf8Block,
};

use crate::canonical_scene::store::production::ValidatedSpatialBatch;
use crate::canonical_scene::store::schema::spatial::{
    SPATIAL_AXIS_NEGATIVE_X, SPATIAL_AXIS_NEGATIVE_Y, SPATIAL_AXIS_NEGATIVE_Z,
    SPATIAL_AXIS_POSITIVE_X, SPATIAL_AXIS_POSITIVE_Y, SPATIAL_AXIS_POSITIVE_Z,
    SPATIAL_AXIS_UNKNOWN, SPATIAL_CAMERA_FIELD_COUNT, SPATIAL_CAMERA_ORTHOGRAPHIC,
    SPATIAL_CAMERA_PERSPECTIVE, SPATIAL_DIRECTORY_FIELD_COUNT, SPATIAL_HANDEDNESS_LEFT,
    SPATIAL_HANDEDNESS_RIGHT, SPATIAL_HANDEDNESS_UNKNOWN, SPATIAL_LIGHT_DIRECTIONAL,
    SPATIAL_LIGHT_FIELD_COUNT, SPATIAL_LIGHT_POINT, SPATIAL_LIGHT_SPOT, SPATIAL_SCHEMA_REVISION,
    SPATIAL_TANGENT_BASIS_MIKKTSPACE, SPATIAL_TANGENT_BASIS_NAMESPACED,
    SPATIAL_TANGENT_BASIS_UNKNOWN, SPATIAL_TANGENT_ENCODING_NAMESPACED,
    SPATIAL_TANGENT_ENCODING_SIGN_IN_W, SPATIAL_TANGENT_ENCODING_UNKNOWN,
    SPATIAL_TRANSFORM_COLUMN_VECTOR_LOCAL_TO_PARENT, SPATIAL_TRANSFORM_UNKNOWN,
    SPATIAL_UV_ORIGIN_LOWER_LEFT, SPATIAL_UV_ORIGIN_UNKNOWN, SPATIAL_UV_ORIGIN_UPPER_LEFT,
    SPATIAL_WINDING_CLOCKWISE, SPATIAL_WINDING_COUNTER_CLOCKWISE, SPATIAL_WINDING_UNKNOWN,
    SpatialCameraTableSchema, SpatialDirectorySchema, SpatialLightTableSchema,
    SpatialTangentBasisSchemaNameSchema, SpatialTangentEncodingSchemaNameSchema,
    spatial_camera_field, spatial_directory_field, spatial_light_field,
};
use crate::canonical_scene::store::schema::{CanonicalBlockRef, PRESENCE_ABSENT, PRESENCE_PRESENT};
use crate::entity::{
    CameraProjection, LightKind, LightKindCategory, validate_canonical_light_emission,
};
use crate::extension::ExtensionSchemaId;
use crate::math::Vec3;
use crate::namespaced::MAX_NAMESPACED_NAME_LEN;
use crate::scene_space::{
    Axis, FrontFaceWinding, Handedness, SceneSpace, TangentBasisConvention,
    TangentHandednessEncoding, TransformConvention, UvOrigin,
};

use super::super::{require_optional_collection, require_scalar};

#[derive(Clone, Copy)]
pub(super) struct SpatialValidatedRefs {
    pub(super) directory: CanonicalBlockRef<SpatialDirectorySchema>,
    pub(super) cameras: Option<CanonicalBlockRef<SpatialCameraTableSchema>>,
    pub(super) lights: Option<CanonicalBlockRef<SpatialLightTableSchema>>,
    pub(super) tangent_basis_schema_name:
        Option<CanonicalBlockRef<SpatialTangentBasisSchemaNameSchema>>,
    pub(super) tangent_encoding_schema_name:
        Option<CanonicalBlockRef<SpatialTangentEncodingSchemaNameSchema>>,
    pub(super) camera_count: u32,
    pub(super) light_count: u32,
}

#[derive(Clone, Copy)]
pub(super) struct SpatialValidatedMembers<'session> {
    pub(super) directory: CanonicalValidationMember<'session, RecordBlock, SpatialDirectorySchema>,
    pub(super) cameras:
        Option<CanonicalValidationMember<'session, RecordBlock, SpatialCameraTableSchema>>,
    pub(super) lights:
        Option<CanonicalValidationMember<'session, RecordBlock, SpatialLightTableSchema>>,
    pub(super) tangent_basis_schema_name:
        Option<CanonicalValidationMember<'session, Utf8Block, SpatialTangentBasisSchemaNameSchema>>,
    pub(super) tangent_encoding_schema_name: Option<
        CanonicalValidationMember<'session, Utf8Block, SpatialTangentEncodingSchemaNameSchema>,
    >,
    pub(super) camera_count: u32,
    pub(super) light_count: u32,
}

pub(super) fn validate_spatial_graph<R>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SpatialDirectorySchema>,
    expected: &ValidatedSpatialBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<SpatialValidatedRefs> {
    validate_spatial_structure(
        session,
        directory_ref,
        expected.camera_count(),
        expected.light_count(),
        Some(expected),
        observe_work,
    )
}

pub(super) fn validate_spatial_canonical_structure(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SpatialDirectorySchema>,
    camera_count: u32,
    light_count: u32,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<SpatialValidatedRefs> {
    validate_spatial_structure::<()>(
        session,
        directory_ref,
        camera_count,
        light_count,
        None,
        observe_work,
    )
}

pub(super) fn validate_spatial_canonical_structure_metered<
    'session,
    'storage,
    'control,
    'operation,
    O,
    PP,
    U,
>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: CanonicalValidationMember<'session, RecordBlock, SpatialDirectorySchema>,
    camera_count: u32,
    light_count: u32,
) -> Result<Option<SpatialValidatedMembers<'session>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mut schema_revision = None;
    let mut handedness = None;
    let mut up_axis = None;
    let mut front_axis = None;
    let mut unit_scale_presence = None;
    let mut unit_scale_absent = None;
    let mut unit_scale_to_meters = None;
    let mut front_face_winding = None;
    let mut uv_origin = None;
    let mut transform_convention = None;
    let mut tangent_basis_tag = None;
    let mut tangent_basis_name_count = None;
    let mut tangent_basis_schema_name = None;
    let mut tangent_basis_name_absent = None;
    let mut tangent_basis_schema_version = None;
    let mut tangent_encoding_tag = None;
    let mut tangent_encoding_name_count = None;
    let mut tangent_encoding_schema_name = None;
    let mut tangent_encoding_name_absent = None;
    let mut tangent_encoding_schema_version = None;
    let mut stored_camera_count = None;
    let mut cameras_presence = None;
    let mut cameras = None;
    let mut cameras_absent = None;
    let mut stored_light_count = None;
    let mut lights_presence = None;
    let mut lights = None;
    let mut lights_absent = None;

    let directory_complete = session.visit_canonical_records(
        directory,
        1,
        SPATIAL_DIRECTORY_FIELD_COUNT,
        |_row_index, row| {
            schema_revision =
                row.scalar(spatial_directory_field::SCHEMA_REVISION, |value: u32| value)?;
            handedness = row.scalar(spatial_directory_field::HANDEDNESS, |value: u8| value)?;
            up_axis = row.scalar(spatial_directory_field::UP_AXIS, |value: u8| value)?;
            front_axis = row.scalar(spatial_directory_field::FRONT_AXIS, |value: u8| value)?;
            unit_scale_presence = row
                .scalar(spatial_directory_field::UNIT_SCALE_PRESENCE, |value: u8| {
                    value
                })?;
            match unit_scale_presence {
                Some(PRESENCE_PRESENT) => {
                    unit_scale_to_meters = row.scalar(
                        spatial_directory_field::UNIT_SCALE_TO_METERS,
                        |value: f64| value,
                    )?;
                }
                _ => {
                    unit_scale_absent = row.scalar(
                        spatial_directory_field::UNIT_SCALE_TO_METERS,
                        |value: u8| value,
                    )?;
                }
            }
            front_face_winding = row
                .scalar(spatial_directory_field::FRONT_FACE_WINDING, |value: u8| {
                    value
                })?;
            uv_origin = row.scalar(spatial_directory_field::UV_ORIGIN, |value: u8| value)?;
            transform_convention = row.scalar(
                spatial_directory_field::TRANSFORM_CONVENTION,
                |value: u8| value,
            )?;
            tangent_basis_tag =
                row.scalar(spatial_directory_field::TANGENT_BASIS, |value: u8| value)?;
            tangent_basis_name_count = row.scalar(
                spatial_directory_field::TANGENT_BASIS_SCHEMA_NAME_COUNT,
                |value: u32| value,
            )?;
            if tangent_basis_name_count.is_some_and(|count| count != 0) {
                tangent_basis_schema_name = row.utf8_ref(
                    spatial_directory_field::TANGENT_BASIS_SCHEMA_NAME,
                    |value| value,
                )?;
            } else {
                tangent_basis_name_absent = row.scalar(
                    spatial_directory_field::TANGENT_BASIS_SCHEMA_NAME,
                    |value: u8| value,
                )?;
            }
            tangent_basis_schema_version = row.scalar(
                spatial_directory_field::TANGENT_BASIS_SCHEMA_VERSION,
                |value: u32| value,
            )?;
            tangent_encoding_tag =
                row.scalar(spatial_directory_field::TANGENT_ENCODING, |value: u8| value)?;
            tangent_encoding_name_count = row.scalar(
                spatial_directory_field::TANGENT_ENCODING_SCHEMA_NAME_COUNT,
                |value: u32| value,
            )?;
            if tangent_encoding_name_count.is_some_and(|count| count != 0) {
                tangent_encoding_schema_name = row.utf8_ref(
                    spatial_directory_field::TANGENT_ENCODING_SCHEMA_NAME,
                    |value| value,
                )?;
            } else {
                tangent_encoding_name_absent = row.scalar(
                    spatial_directory_field::TANGENT_ENCODING_SCHEMA_NAME,
                    |value: u8| value,
                )?;
            }
            tangent_encoding_schema_version = row.scalar(
                spatial_directory_field::TANGENT_ENCODING_SCHEMA_VERSION,
                |value: u32| value,
            )?;
            stored_camera_count =
                row.scalar(spatial_directory_field::CAMERA_COUNT, |value: u32| value)?;
            cameras_presence =
                row.scalar(spatial_directory_field::CAMERAS_PRESENCE, |value: u8| value)?;
            if camera_count == 0 {
                cameras_absent = row.scalar(spatial_directory_field::CAMERAS, |value: u8| value)?;
            } else {
                cameras = row.record_ref(spatial_directory_field::CAMERAS, |value| value)?;
            }
            stored_light_count =
                row.scalar(spatial_directory_field::LIGHT_COUNT, |value: u32| value)?;
            lights_presence =
                row.scalar(spatial_directory_field::LIGHTS_PRESENCE, |value: u8| value)?;
            if light_count == 0 {
                lights_absent = row.scalar(spatial_directory_field::LIGHTS, |value: u8| value)?;
            } else {
                lights = row.record_ref(spatial_directory_field::LIGHTS, |value| value)?;
            }
            Ok(true)
        },
    )?;
    if !directory_complete
        || require_scalar(schema_revision, SPATIAL_SCHEMA_REVISION).is_none()
        || require_optional_collection(
            cameras_presence,
            stored_camera_count,
            if camera_count == 0 {
                PRESENCE_ABSENT
            } else {
                PRESENCE_PRESENT
            },
            camera_count,
        )
        .is_none()
        || require_optional_collection(
            lights_presence,
            stored_light_count,
            if light_count == 0 {
                PRESENCE_ABSENT
            } else {
                PRESENCE_PRESENT
            },
            light_count,
        )
        .is_none()
        || (camera_count == 0 && require_scalar(cameras_absent, 0_u8).is_none())
        || (light_count == 0 && require_scalar(lights_absent, 0_u8).is_none())
        || (camera_count != 0 && cameras.is_none())
        || (light_count != 0 && lights.is_none())
    {
        return Ok(None);
    }

    let unit_scale_to_meters =
        decode_optional_f64(unit_scale_presence, unit_scale_absent, unit_scale_to_meters);
    let Some(unit_scale_to_meters) = unit_scale_to_meters else {
        return Ok(None);
    };
    let tangent_basis_schema = validate_schema_id_metered(
        session,
        tangent_basis_name_count,
        tangent_basis_name_absent,
        tangent_basis_schema_name,
        tangent_basis_schema_version,
    )?;
    let Some(tangent_basis_schema) = tangent_basis_schema else {
        return Ok(None);
    };
    let tangent_encoding_schema = validate_schema_id_metered(
        session,
        tangent_encoding_name_count,
        tangent_encoding_name_absent,
        tangent_encoding_schema_name,
        tangent_encoding_schema_version,
    )?;
    let Some(tangent_encoding_schema) = tangent_encoding_schema else {
        return Ok(None);
    };
    let tangent_basis = match (tangent_basis_tag, tangent_basis_schema) {
        (Some(SPATIAL_TANGENT_BASIS_UNKNOWN), None) => TangentBasisConvention::Unknown,
        (Some(SPATIAL_TANGENT_BASIS_MIKKTSPACE), None) => TangentBasisConvention::MikkTspace,
        (Some(SPATIAL_TANGENT_BASIS_NAMESPACED), Some(schema_id)) => {
            let Ok(value) = TangentBasisConvention::try_namespaced(Some(schema_id)) else {
                return Ok(None);
            };
            value
        }
        _ => return Ok(None),
    };
    let tangent_handedness_encoding = match (tangent_encoding_tag, tangent_encoding_schema) {
        (Some(SPATIAL_TANGENT_ENCODING_UNKNOWN), None) => TangentHandednessEncoding::Unknown,
        (Some(SPATIAL_TANGENT_ENCODING_SIGN_IN_W), None) => TangentHandednessEncoding::SignInW,
        (Some(SPATIAL_TANGENT_ENCODING_NAMESPACED), Some(schema_id)) => {
            let Ok(value) = TangentHandednessEncoding::try_namespaced(Some(schema_id)) else {
                return Ok(None);
            };
            value
        }
        _ => return Ok(None),
    };
    let (Some(handedness), Some(up_axis), Some(front_axis), Some(front_face_winding)) = (
        handedness.and_then(decode_handedness),
        up_axis.and_then(decode_axis),
        front_axis.and_then(decode_axis),
        front_face_winding.and_then(decode_winding),
    ) else {
        return Ok(None);
    };
    let (Some(uv_origin), Some(transform_convention)) = (
        uv_origin.and_then(decode_uv_origin),
        transform_convention.and_then(decode_transform_convention),
    ) else {
        return Ok(None);
    };
    let scene_space = SceneSpace {
        handedness,
        up_axis,
        front_axis,
        unit_scale_to_meters,
        front_face_winding,
        uv_origin,
        transform_convention,
        tangent_basis,
        tangent_handedness_encoding,
    };
    if scene_space.validate().is_err()
        || !validate_cameras_metered(session, cameras, camera_count)?
        || !validate_lights_metered(session, lights, light_count)?
    {
        return Ok(None);
    }

    Ok(Some(SpatialValidatedMembers {
        directory,
        cameras,
        lights,
        tangent_basis_schema_name,
        tangent_encoding_schema_name,
        camera_count,
        light_count,
    }))
}

fn validate_schema_id_metered<'session, 'storage, 'control, 'operation, O, PP, U, Tag>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    count: Option<u32>,
    absent: Option<u8>,
    member: Option<CanonicalValidationMember<'session, Utf8Block, Tag>>,
    version: Option<u32>,
) -> Result<Option<Option<ExtensionSchemaId>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: Copy + Send + Sync + 'static,
{
    let (Some(count), Some(version)) = (count, version) else {
        return Ok(None);
    };
    if count == 0 {
        return Ok((require_scalar(absent, 0_u8).is_some()
            && require_scalar(Some(version), 0_u32).is_some()
            && member.is_none())
        .then_some(None));
    }
    let Some(member) = member else {
        return Ok(None);
    };
    let Ok(length) = usize::try_from(count) else {
        return Ok(None);
    };
    if length > MAX_NAMESPACED_NAME_LEN {
        return Ok(None);
    }
    let mut bytes = [0_u8; MAX_NAMESPACED_NAME_LEN];
    let complete = session.visit_canonical_utf8_bytes(member, length, |index, value| {
        bytes[index] = value;
        true
    })?;
    if !complete {
        return Ok(None);
    }
    let Some(name) = std::str::from_utf8(&bytes[..length]).ok() else {
        return Ok(None);
    };
    Ok(ExtensionSchemaId::new(name, version).ok().map(Some))
}

fn validate_cameras_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<CanonicalValidationMember<'session, RecordBlock, SpatialCameraTableSchema>>,
    count: u32,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if count == 0 {
        return Ok(member.is_none());
    }
    let Some(member) = member else {
        return Ok(false);
    };
    let Ok(expected_rows) = usize::try_from(count) else {
        return Ok(false);
    };
    session.visit_canonical_records(
        member,
        expected_rows,
        SPATIAL_CAMERA_FIELD_COUNT,
        |row_index, row| {
            let camera = row.scalar(spatial_camera_field::CAMERA, |value: u32| value)?;
            let projection = row.scalar(spatial_camera_field::PROJECTION, |value: u8| value)?;
            let primary = row.scalar(spatial_camera_field::PRIMARY, |value: f32| value)?;
            let secondary_presence =
                row.scalar(spatial_camera_field::SECONDARY_PRESENCE, |value: u8| value)?;
            let secondary: Option<Option<f32>> = match secondary_presence {
                Some(PRESENCE_PRESENT) => row
                    .scalar(spatial_camera_field::SECONDARY, |value: f32| value)?
                    .map(Some),
                _ => {
                    let absent = row.scalar(spatial_camera_field::SECONDARY, |value: u8| value)?;
                    (secondary_presence == Some(PRESENCE_ABSENT)
                        && require_scalar(absent, 0_u8).is_some())
                    .then_some(None)
                }
            };
            let near = row.scalar(spatial_camera_field::NEAR, |value: f32| value)?;
            let far_presence = row.scalar(spatial_camera_field::FAR_PRESENCE, |value: u8| value)?;
            let far: Option<Option<f32>> = match far_presence {
                Some(PRESENCE_PRESENT) => row
                    .scalar(spatial_camera_field::FAR, |value: f32| value)?
                    .map(Some),
                _ => {
                    let absent = row.scalar(spatial_camera_field::FAR, |value: u8| value)?;
                    (far_presence == Some(PRESENCE_ABSENT)
                        && require_scalar(absent, 0_u8).is_some())
                    .then_some(None)
                }
            };
            let Some(expected_camera) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            if require_scalar(camera, expected_camera).is_none() {
                return Ok(false);
            }
            let valid = match (projection, primary, secondary, near, far) {
                (
                    Some(SPATIAL_CAMERA_PERSPECTIVE),
                    Some(primary),
                    Some(secondary),
                    Some(near),
                    Some(far),
                ) => CameraProjection::perspective(primary, secondary, near, far).is_ok(),
                (
                    Some(SPATIAL_CAMERA_ORTHOGRAPHIC),
                    Some(primary),
                    Some(Some(secondary)),
                    Some(near),
                    Some(Some(far)),
                ) => {
                    (near != 0.0 || near.to_bits() == 0.0_f32.to_bits())
                        && CameraProjection::orthographic(primary, secondary, near, far).is_ok()
                }
                _ => false,
            };
            Ok(valid)
        },
    )
}

fn validate_lights_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<CanonicalValidationMember<'session, RecordBlock, SpatialLightTableSchema>>,
    count: u32,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if count == 0 {
        return Ok(member.is_none());
    }
    let Some(member) = member else {
        return Ok(false);
    };
    let Ok(expected_rows) = usize::try_from(count) else {
        return Ok(false);
    };
    session.visit_canonical_records(
        member,
        expected_rows,
        SPATIAL_LIGHT_FIELD_COUNT,
        |row_index, row| {
            let light = row.scalar(spatial_light_field::LIGHT, |value: u32| value)?;
            let kind = row.scalar(spatial_light_field::KIND, |value: u8| value)?;
            let range_presence =
                row.scalar(spatial_light_field::RANGE_PRESENCE, |value: u8| value)?;
            let range: Option<Option<f32>> = match range_presence {
                Some(PRESENCE_PRESENT) => row
                    .scalar(spatial_light_field::RANGE, |value: f32| value)?
                    .map(Some),
                _ => {
                    let absent = row.scalar(spatial_light_field::RANGE, |value: u8| value)?;
                    (range_presence == Some(PRESENCE_ABSENT)
                        && require_scalar(absent, 0_u8).is_some())
                    .then_some(None)
                }
            };
            let inner_presence =
                row.scalar(spatial_light_field::INNER_CONE_PRESENCE, |value: u8| value)?;
            let inner: Option<Option<f32>> = match inner_presence {
                Some(PRESENCE_PRESENT) => row
                    .scalar(spatial_light_field::INNER_CONE, |value: f32| value)?
                    .map(Some),
                _ => {
                    let absent = row.scalar(spatial_light_field::INNER_CONE, |value: u8| value)?;
                    (inner_presence == Some(PRESENCE_ABSENT)
                        && require_scalar(absent, 0_u8).is_some())
                    .then_some(None)
                }
            };
            let outer_presence =
                row.scalar(spatial_light_field::OUTER_CONE_PRESENCE, |value: u8| value)?;
            let outer: Option<Option<f32>> = match outer_presence {
                Some(PRESENCE_PRESENT) => row
                    .scalar(spatial_light_field::OUTER_CONE, |value: f32| value)?
                    .map(Some),
                _ => {
                    let absent = row.scalar(spatial_light_field::OUTER_CONE, |value: u8| value)?;
                    (outer_presence == Some(PRESENCE_ABSENT)
                        && require_scalar(absent, 0_u8).is_some())
                    .then_some(None)
                }
            };
            let tint_red = row.scalar(spatial_light_field::TINT_RED, |value: f32| value)?;
            let tint_green = row.scalar(spatial_light_field::TINT_GREEN, |value: f32| value)?;
            let tint_blue = row.scalar(spatial_light_field::TINT_BLUE, |value: f32| value)?;
            let intensity = row.scalar(spatial_light_field::INTENSITY, |value: f32| value)?;
            let Some(expected_light) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            if require_scalar(light, expected_light).is_none() {
                return Ok(false);
            }
            let (Some(kind), Some(range), Some(inner), Some(outer)) = (kind, range, inner, outer)
            else {
                return Ok(false);
            };
            if decode_canonical_light_kind(kind, range, inner, outer).is_none() {
                return Ok(false);
            }
            let (Some(tint_red), Some(tint_green), Some(tint_blue), Some(intensity)) =
                (tint_red, tint_green, tint_blue, intensity)
            else {
                return Ok(false);
            };
            let tint = Vec3::new(tint_red, tint_green, tint_blue);
            Ok(validate_canonical_light_emission(tint, intensity).is_ok())
        },
    )
}

fn validate_spatial_structure<R>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<SpatialDirectorySchema>,
    camera_count: u32,
    light_count: u32,
    expected: Option<&ValidatedSpatialBatch<'_, R>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<SpatialValidatedRefs> {
    observe_work(1)?;
    let directory = session.canonical_records(directory_ref)?;
    if directory.rows() != 1 || directory.fields() != SPATIAL_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, spatial_directory_field::SCHEMA_REVISION),
        SPATIAL_SCHEMA_REVISION,
    )?;

    let unit_scale_to_meters = read_optional_f64(
        directory,
        spatial_directory_field::UNIT_SCALE_PRESENCE,
        spatial_directory_field::UNIT_SCALE_TO_METERS,
    )?;
    let (tangent_basis, tangent_basis_schema_name) =
        read_tangent_basis(session, directory, observe_work)?;
    let (tangent_handedness_encoding, tangent_encoding_schema_name) =
        read_tangent_encoding(session, directory, observe_work)?;
    let scene_space = SceneSpace {
        handedness: decode_handedness(
            directory.scalar::<u8>(0, spatial_directory_field::HANDEDNESS)?,
        )?,
        up_axis: decode_axis(directory.scalar::<u8>(0, spatial_directory_field::UP_AXIS)?)?,
        front_axis: decode_axis(directory.scalar::<u8>(0, spatial_directory_field::FRONT_AXIS)?)?,
        unit_scale_to_meters,
        front_face_winding: decode_winding(
            directory.scalar::<u8>(0, spatial_directory_field::FRONT_FACE_WINDING)?,
        )?,
        uv_origin: decode_uv_origin(
            directory.scalar::<u8>(0, spatial_directory_field::UV_ORIGIN)?,
        )?,
        transform_convention: decode_transform_convention(
            directory.scalar::<u8>(0, spatial_directory_field::TRANSFORM_CONVENTION)?,
        )?,
        tangent_basis,
        tangent_handedness_encoding,
    };
    scene_space.validate().ok()?;
    if expected.is_some_and(|expected| &scene_space != expected.scene_space()) {
        return None;
    }

    let cameras = optional_record_ref::<SpatialCameraTableSchema>(
        session,
        directory,
        spatial_directory_field::CAMERA_COUNT,
        spatial_directory_field::CAMERAS_PRESENCE,
        spatial_directory_field::CAMERAS,
        camera_count,
        SPATIAL_CAMERA_FIELD_COUNT,
    )?;
    let lights = optional_record_ref::<SpatialLightTableSchema>(
        session,
        directory,
        spatial_directory_field::LIGHT_COUNT,
        spatial_directory_field::LIGHTS_PRESENCE,
        spatial_directory_field::LIGHTS,
        light_count,
        SPATIAL_LIGHT_FIELD_COUNT,
    )?;
    validate_cameras(session, cameras, expected, observe_work)?;
    validate_lights(session, lights, expected, observe_work)?;

    Some(SpatialValidatedRefs {
        directory: CanonicalBlockRef::from_store(directory_ref),
        cameras: cameras.map(CanonicalBlockRef::from_store),
        lights: lights.map(CanonicalBlockRef::from_store),
        tangent_basis_schema_name: tangent_basis_schema_name.map(CanonicalBlockRef::from_store),
        tangent_encoding_schema_name: tangent_encoding_schema_name
            .map(CanonicalBlockRef::from_store),
        camera_count,
        light_count,
    })
}

fn read_tangent_basis(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<(
    TangentBasisConvention,
    Option<CanonicalUtf8Ref<SpatialTangentBasisSchemaNameSchema>>,
)> {
    let tag = directory.scalar::<u8>(0, spatial_directory_field::TANGENT_BASIS)?;
    let (schema, reference) = read_schema_id::<SpatialTangentBasisSchemaNameSchema>(
        session,
        directory,
        spatial_directory_field::TANGENT_BASIS_SCHEMA_NAME_COUNT,
        spatial_directory_field::TANGENT_BASIS_SCHEMA_NAME,
        spatial_directory_field::TANGENT_BASIS_SCHEMA_VERSION,
        observe_work,
    )?;
    let value = match (tag, schema) {
        (SPATIAL_TANGENT_BASIS_UNKNOWN, None) => TangentBasisConvention::Unknown,
        (SPATIAL_TANGENT_BASIS_MIKKTSPACE, None) => TangentBasisConvention::MikkTspace,
        (SPATIAL_TANGENT_BASIS_NAMESPACED, Some(schema_id)) => {
            TangentBasisConvention::try_namespaced(Some(schema_id)).ok()?
        }
        _ => return None,
    };
    Some((value, reference))
}

fn read_tangent_encoding(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<(
    TangentHandednessEncoding,
    Option<CanonicalUtf8Ref<SpatialTangentEncodingSchemaNameSchema>>,
)> {
    let tag = directory.scalar::<u8>(0, spatial_directory_field::TANGENT_ENCODING)?;
    let (schema, reference) = read_schema_id::<SpatialTangentEncodingSchemaNameSchema>(
        session,
        directory,
        spatial_directory_field::TANGENT_ENCODING_SCHEMA_NAME_COUNT,
        spatial_directory_field::TANGENT_ENCODING_SCHEMA_NAME,
        spatial_directory_field::TANGENT_ENCODING_SCHEMA_VERSION,
        observe_work,
    )?;
    let value = match (tag, schema) {
        (SPATIAL_TANGENT_ENCODING_UNKNOWN, None) => TangentHandednessEncoding::Unknown,
        (SPATIAL_TANGENT_ENCODING_SIGN_IN_W, None) => TangentHandednessEncoding::SignInW,
        (SPATIAL_TANGENT_ENCODING_NAMESPACED, Some(schema_id)) => {
            TangentHandednessEncoding::try_namespaced(Some(schema_id)).ok()?
        }
        _ => return None,
    };
    Some((value, reference))
}

fn read_schema_id<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    reference_field: usize,
    version_field: usize,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<(Option<ExtensionSchemaId>, Option<CanonicalUtf8Ref<Tag>>)>
where
    Tag: Copy + Send + Sync + 'static,
{
    let count = directory.scalar::<u32>(0, count_field)?;
    let version = directory.scalar::<u32>(0, version_field)?;
    if count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        require_scalar(Some(version), 0_u32)?;
        return Some((None, None));
    }
    let reference = directory.utf8_ref::<Tag>(0, reference_field)?;
    let name = session.canonical_utf8(reference)?;
    if name.len() != usize::try_from(count).ok()? {
        return None;
    }
    observe_work(u64::from(count))?;
    let schema = ExtensionSchemaId::new(name, version).ok()?;
    Some((Some(schema), Some(reference)))
}

fn optional_record_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    presence_field: usize,
    reference_field: usize,
    expected_count: u32,
    expected_fields: usize,
) -> Option<Option<CanonicalRecordRef<Tag>>>
where
    Tag: Copy + Send + Sync + 'static,
{
    require_optional_collection(
        directory.scalar::<u8>(0, presence_field),
        directory.scalar::<u32>(0, count_field),
        if expected_count == 0 {
            PRESENCE_ABSENT
        } else {
            PRESENCE_PRESENT
        },
        expected_count,
    )?;
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        let reference = directory.record_ref::<Tag>(0, reference_field)?;
        let table = session.canonical_records(reference)?;
        (table.rows() == usize::try_from(expected_count).ok()? && table.fields() == expected_fields)
            .then_some(Some(reference))
    }
}

fn validate_cameras<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<SpatialCameraTableSchema>>,
    expected: Option<&ValidatedSpatialBatch<'_, R>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return expected
            .is_none_or(|expected| expected.camera_count() == 0)
            .then_some(());
    };
    let table = session.canonical_records(reference)?;
    for row in 0..table.rows() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, spatial_camera_field::CAMERA),
            u32::try_from(row).ok()?,
        )?;
        let primary = table.scalar::<f32>(row, spatial_camera_field::PRIMARY)?;
        let secondary = read_optional_f32(
            table,
            row,
            spatial_camera_field::SECONDARY_PRESENCE,
            spatial_camera_field::SECONDARY,
        )?;
        let near = table.scalar::<f32>(row, spatial_camera_field::NEAR)?;
        let far = read_optional_f32(
            table,
            row,
            spatial_camera_field::FAR_PRESENCE,
            spatial_camera_field::FAR,
        )?;
        let projection = match table.scalar::<u8>(row, spatial_camera_field::PROJECTION)? {
            SPATIAL_CAMERA_PERSPECTIVE => {
                CameraProjection::perspective(primary, secondary, near, far).ok()?
            }
            SPATIAL_CAMERA_ORTHOGRAPHIC => {
                if near == 0.0 && near.to_bits() != 0.0_f32.to_bits() {
                    return None;
                }
                CameraProjection::orthographic(primary, secondary?, near, far?).ok()?
            }
            _ => return None,
        };
        if let Some(expected) = expected {
            let expected = expected.input().cameras().get(row)?.projection();
            same_projection_bits(projection, expected).then_some(())?;
        }
    }
    Some(())
}

fn validate_lights<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<SpatialLightTableSchema>>,
    expected: Option<&ValidatedSpatialBatch<'_, R>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return expected
            .is_none_or(|expected| expected.light_count() == 0)
            .then_some(());
    };
    let table = session.canonical_records(reference)?;
    for row in 0..table.rows() {
        observe_work(1)?;
        require_scalar(
            table.scalar::<u32>(row, spatial_light_field::LIGHT),
            u32::try_from(row).ok()?,
        )?;
        let range = read_optional_f32(
            table,
            row,
            spatial_light_field::RANGE_PRESENCE,
            spatial_light_field::RANGE,
        )?;
        let inner = read_optional_f32(
            table,
            row,
            spatial_light_field::INNER_CONE_PRESENCE,
            spatial_light_field::INNER_CONE,
        )?;
        let outer = read_optional_f32(
            table,
            row,
            spatial_light_field::OUTER_CONE_PRESENCE,
            spatial_light_field::OUTER_CONE,
        )?;
        let kind = decode_canonical_light_kind(
            table.scalar::<u8>(row, spatial_light_field::KIND)?,
            range,
            inner,
            outer,
        )?;
        let tint = Vec3::new(
            table.scalar::<f32>(row, spatial_light_field::TINT_RED)?,
            table.scalar::<f32>(row, spatial_light_field::TINT_GREEN)?,
            table.scalar::<f32>(row, spatial_light_field::TINT_BLUE)?,
        );
        let intensity = table.scalar::<f32>(row, spatial_light_field::INTENSITY)?;
        validate_canonical_light_emission(tint, intensity).ok()?;
        if let Some(expected) = expected {
            let expected = expected.input().lights().get(row)?;
            same_light_kind_bits(kind, expected.kind()).then_some(())?;
            let expected_emission = expected.emission();
            same_vec3_bits(tint, expected_emission.tint()).then_some(())?;
            (intensity.to_bits() == expected_emission.intensity().to_bits()).then_some(())?;
        }
    }
    Some(())
}

fn read_optional_f32(
    records: CanonicalRecordView<'_>,
    row: usize,
    presence_field: usize,
    value_field: usize,
) -> Option<Option<f32>> {
    match records.scalar::<u8>(row, presence_field)? {
        PRESENCE_ABSENT => {
            require_scalar(records.scalar::<u8>(row, value_field), 0_u8)?;
            Some(None)
        }
        PRESENCE_PRESENT => Some(Some(records.scalar::<f32>(row, value_field)?)),
        _ => None,
    }
}

fn decode_canonical_light_kind(
    tag: u8,
    range: Option<f32>,
    inner: Option<f32>,
    outer: Option<f32>,
) -> Option<LightKind> {
    match tag {
        SPATIAL_LIGHT_DIRECTIONAL if range.is_none() && inner.is_none() && outer.is_none() => {
            Some(LightKind::directional())
        }
        SPATIAL_LIGHT_POINT if inner.is_none() && outer.is_none() => LightKind::point(range).ok(),
        SPATIAL_LIGHT_SPOT => {
            let inner = inner?;
            if inner == 0.0 && inner.to_bits() != 0.0_f32.to_bits() {
                return None;
            }
            LightKind::spot(range, inner, outer?).ok()
        }
        _ => None,
    }
}

fn read_optional_f64(
    records: CanonicalRecordView<'_>,
    presence_field: usize,
    value_field: usize,
) -> Option<Option<f64>> {
    match records.scalar::<u8>(0, presence_field)? {
        PRESENCE_ABSENT => {
            require_scalar(records.scalar::<u8>(0, value_field), 0_u8)?;
            Some(None)
        }
        PRESENCE_PRESENT => Some(Some(records.scalar::<f64>(0, value_field)?)),
        _ => None,
    }
}

fn decode_optional_f64(
    presence: Option<u8>,
    absent: Option<u8>,
    value: Option<f64>,
) -> Option<Option<f64>> {
    match presence? {
        PRESENCE_ABSENT => {
            require_scalar(absent, 0_u8)?;
            Some(None)
        }
        PRESENCE_PRESENT => Some(Some(value?)),
        _ => None,
    }
}

const fn decode_handedness(tag: u8) -> Option<Handedness> {
    match tag {
        SPATIAL_HANDEDNESS_UNKNOWN => Some(Handedness::Unknown),
        SPATIAL_HANDEDNESS_RIGHT => Some(Handedness::Right),
        SPATIAL_HANDEDNESS_LEFT => Some(Handedness::Left),
        _ => None,
    }
}

const fn decode_axis(tag: u8) -> Option<Axis> {
    match tag {
        SPATIAL_AXIS_UNKNOWN => Some(Axis::Unknown),
        SPATIAL_AXIS_POSITIVE_X => Some(Axis::PositiveX),
        SPATIAL_AXIS_NEGATIVE_X => Some(Axis::NegativeX),
        SPATIAL_AXIS_POSITIVE_Y => Some(Axis::PositiveY),
        SPATIAL_AXIS_NEGATIVE_Y => Some(Axis::NegativeY),
        SPATIAL_AXIS_POSITIVE_Z => Some(Axis::PositiveZ),
        SPATIAL_AXIS_NEGATIVE_Z => Some(Axis::NegativeZ),
        _ => None,
    }
}

const fn decode_winding(tag: u8) -> Option<FrontFaceWinding> {
    match tag {
        SPATIAL_WINDING_UNKNOWN => Some(FrontFaceWinding::Unknown),
        SPATIAL_WINDING_CLOCKWISE => Some(FrontFaceWinding::Clockwise),
        SPATIAL_WINDING_COUNTER_CLOCKWISE => Some(FrontFaceWinding::CounterClockwise),
        _ => None,
    }
}

const fn decode_uv_origin(tag: u8) -> Option<UvOrigin> {
    match tag {
        SPATIAL_UV_ORIGIN_UNKNOWN => Some(UvOrigin::Unknown),
        SPATIAL_UV_ORIGIN_UPPER_LEFT => Some(UvOrigin::UpperLeft),
        SPATIAL_UV_ORIGIN_LOWER_LEFT => Some(UvOrigin::LowerLeft),
        _ => None,
    }
}

const fn decode_transform_convention(tag: u8) -> Option<TransformConvention> {
    match tag {
        SPATIAL_TRANSFORM_UNKNOWN => Some(TransformConvention::Unknown),
        SPATIAL_TRANSFORM_COLUMN_VECTOR_LOCAL_TO_PARENT => {
            Some(TransformConvention::ColumnVectorLocalToParent)
        }
        _ => None,
    }
}

fn same_projection_bits(left: CameraProjection, right: CameraProjection) -> bool {
    match (
        left.perspective_parameters(),
        right.perspective_parameters(),
    ) {
        (Some(left), Some(right)) => {
            same_f32_bits(left.0, right.0)
                && same_optional_f32_bits(left.1, right.1)
                && same_f32_bits(left.2, right.2)
                && same_optional_f32_bits(left.3, right.3)
        }
        (None, None) => match (
            left.orthographic_parameters(),
            right.orthographic_parameters(),
        ) {
            (Some(left), Some(right)) => {
                same_f32_bits(left.0, right.0)
                    && same_f32_bits(left.1, right.1)
                    && same_f32_bits(left.2, right.2)
                    && same_f32_bits(left.3, right.3)
            }
            _ => false,
        },
        _ => false,
    }
}

fn same_light_kind_bits(left: LightKind, right: LightKind) -> bool {
    if left.category() != right.category() {
        return false;
    }
    match left.category() {
        LightKindCategory::Directional => true,
        LightKindCategory::Point => same_optional_f32_bits(left.range(), right.range()),
        LightKindCategory::Spot => {
            same_optional_f32_bits(left.range(), right.range())
                && same_optional_f32_bits(left.inner_cone_radians(), right.inner_cone_radians())
                && same_optional_f32_bits(left.outer_cone_radians(), right.outer_cone_radians())
        }
    }
}

fn same_vec3_bits(left: Vec3, right: Vec3) -> bool {
    same_f32_bits(left.x, right.x)
        && same_f32_bits(left.y, right.y)
        && same_f32_bits(left.z, right.z)
}

fn same_optional_f32_bits(left: Option<f32>, right: Option<f32>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => same_f32_bits(left, right),
        (None, None) => true,
        _ => false,
    }
}

const fn same_f32_bits(left: f32, right: f32) -> bool {
    left.to_bits() == right.to_bits()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_light_kind_decode_rejects_noncanonical_and_inconsistent_fields() {
        let canonical_spot =
            decode_canonical_light_kind(SPATIAL_LIGHT_SPOT, Some(10.0), Some(0.0), Some(1.0))
                .expect("canonical raw spot fields");
        assert_eq!(canonical_spot.category(), LightKindCategory::Spot);
        assert!(
            decode_canonical_light_kind(SPATIAL_LIGHT_SPOT, None, Some(-0.0), Some(1.0),).is_none()
        );
        assert!(
            decode_canonical_light_kind(SPATIAL_LIGHT_DIRECTIONAL, Some(1.0), None, None,)
                .is_none()
        );
        assert!(decode_canonical_light_kind(SPATIAL_LIGHT_POINT, None, Some(0.0), None,).is_none());
        assert!(decode_canonical_light_kind(SPATIAL_LIGHT_POINT, Some(-1.0), None, None).is_none());
        assert!(
            decode_canonical_light_kind(SPATIAL_LIGHT_POINT, Some(f32::NAN), None, None).is_none()
        );
        assert!(
            decode_canonical_light_kind(SPATIAL_LIGHT_SPOT, None, Some(0.5), Some(0.25),).is_none()
        );
        assert!(
            decode_canonical_light_kind(
                SPATIAL_LIGHT_SPOT,
                None,
                Some(0.0),
                Some(std::f32::consts::FRAC_PI_2 + f32::EPSILON),
            )
            .is_none()
        );
        assert!(decode_canonical_light_kind(u8::MAX, None, None, None).is_none());
    }
}
