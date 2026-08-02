use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, CanonicalUtf8Ref, StoreValidationSession,
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
