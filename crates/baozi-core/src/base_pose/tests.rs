use super::*;
use crate::Quat;

#[test]
fn trs_rotation_negative_and_zero_scales_need_no_inverse() {
    let authored = AuthoredTransform::try_trs(
        Vec3::new(5.0, 6.0, 7.0),
        Quat::from_xyzw(0.5, 0.5, 0.5, 0.5),
        Vec3::new(-2.0, 3.0, 0.0),
    )
    .unwrap();
    let matrix = checked_local(authored).unwrap();
    assert_eq!(matrix, authored.local_matrix());
    assert_eq!(
        transform_point(matrix, &[1.0, 2.0, 9.0]).unwrap(),
        Vec3::new(5.0, 4.0, 13.0)
    );
    assert_eq!(
        transform_point(matrix, &[-1.0, -2.0, -9.0]).unwrap(),
        Vec3::new(5.0, 8.0, 1.0)
    );
}

#[test]
fn composition_applies_local_before_parent_and_preserves_shear() {
    let parent = Mat4 {
        cols: [
            [0.0, -2.0, 0.0, 0.0],
            [-3.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.5, 0.0],
            [10.0, -2.0, 3.0, 1.0],
        ],
    };
    let local = Mat4::from_trs(
        Vec3::new(1.0, 2.0, -4.0),
        Quat::IDENTITY,
        Vec3::new(1.0, 1.0, 1.0),
    );
    let world = compose(parent, local).unwrap();
    assert_eq!(world.cols[3], [4.0, -4.0, 1.0, 1.0]);
    assert!(world.is_affine());
    assert_eq!(
        transform_point(world, &[2.0, 1.0, 4.0]).unwrap(),
        Vec3::new(1.0, -8.0, 3.0)
    );

    let shear = Mat4 {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [1.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    assert_eq!(compose(Mat4::IDENTITY, shear).unwrap(), shear);
    let mut bounds = None;
    for point in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        include_point(&mut bounds, transform_point(shear, &point).unwrap());
    }
    assert_eq!(
        bounds,
        Some(Aabb {
            min: Vec3::ZERO,
            max: Vec3::new(1.0, 1.0, 0.0)
        })
    );
}

#[test]
fn widened_dot_products_do_not_round_each_product_to_f32() {
    let large = 2.0_f32.powi(127);
    let matrix = Mat4 {
        cols: [
            [large, 0.0, 0.0, 0.0],
            [large, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-large, 0.0, 0.0, 1.0],
        ],
    };
    assert_eq!(
        transform_point(matrix, &[2.0, -1.0, 0.0]).unwrap(),
        Vec3::new(0.0, -1.0, 0.0)
    );
    let mut local = Mat4::IDENTITY;
    local.cols[0] = [2.0, -1.0, 0.0, 0.0];
    assert_eq!(
        compose(matrix, local).unwrap().cols[0],
        [large, -1.0, 0.0, 0.0]
    );
}

#[test]
fn bounds_describe_nearest_rounded_world_vertices() {
    let mut matrix = Mat4::IDENTITY;
    matrix.cols[1][0] = 1.0;
    assert_eq!(
        transform_point(matrix, &[1.0, 2.0_f32.powi(-25), 0.0]).unwrap(),
        Vec3::new(1.0, 2.0_f32.powi(-25), 0.0)
    );
}

#[test]
fn overflow_and_underflow_are_explicit_numeric_errors() {
    let mut scale = Mat4::IDENTITY;
    scale.cols[0][0] = f32::MAX;
    assert_eq!(
        transform_point(scale, &[2.0, 0.0, 0.0]),
        Err(NumericConversionError::NonFiniteOutput)
    );
    let mut twice = Mat4::IDENTITY;
    twice.cols[0][0] = 2.0;
    assert_eq!(
        compose(twice, scale),
        Err(NumericConversionError::NonFiniteOutput)
    );
    scale.cols[0][0] = 0.5;
    assert_eq!(
        transform_point(scale, &[f32::from_bits(1), 0.0, 0.0]),
        Err(NumericConversionError::UnderflowToZero)
    );
    twice.cols[0][0] = f32::from_bits(1);
    assert_eq!(
        compose(twice, scale),
        Err(NumericConversionError::UnderflowToZero)
    );
    assert_eq!(round(f64::from(f32::from_bits(1))), Ok(f32::from_bits(1)));
    assert_eq!(round(-0.0).unwrap().to_bits(), (-0.0_f32).to_bits());
}

#[test]
fn accepted_trs_can_still_have_an_unrepresentable_derived_matrix() {
    let rotation = Quat::from_xyzw(0.0, 1.00002, 0.0, 0.0);
    assert!(rotation.is_unit());
    let authored =
        AuthoredTransform::try_trs(Vec3::ZERO, rotation, Vec3::new(f32::MAX, 1.0, 1.0)).unwrap();
    assert_eq!(
        checked_local(authored),
        Err(NumericConversionError::NonFiniteOutput)
    );
}

#[test]
fn a_nonzero_trs_cell_rounding_to_zero_is_rejected() {
    let authored = AuthoredTransform::try_trs(
        Vec3::ZERO,
        Quat::from_xyzw(0.0, 0.0, 2.0_f32.powi(-75), 1.0),
        Vec3::new(f32::from_bits(1), 1.0, 1.0),
    )
    .unwrap();
    // A nonzero derived off-diagonal component falls below the smallest representable F32.
    assert_eq!(
        checked_local(authored),
        Err(NumericConversionError::UnderflowToZero)
    );
}

#[test]
fn lookup_checks_generation_before_range_and_survives_without_scene() {
    let generation = SceneGenerationId::allocate().unwrap();
    let table = BasePoseWorldTransforms {
        generation,
        transforms: vec![Mat4::IDENTITY],
    };
    assert_eq!(
        table.node_transform(NodeId::new(generation, 0)).unwrap(),
        Mat4::IDENTITY
    );
    assert_eq!(
        table.node_transform(NodeId::new(generation, 1)),
        Err(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Node,
            length: 1
        })
    );
    assert!(matches!(
        table.node_transform(NodeId::new(
            SceneGenerationId::allocate().unwrap(),
            u32::MAX
        )),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
}
