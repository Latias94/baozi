use super::*;
use crate::{Mat4, Quat, Vec3};

#[test]
fn precision_contract_composed_translations_retain_f64_residual() {
    let mut parent = Mat4::IDENTITY;
    parent.cols[3][0] = 1.0;
    let mut local = Mat4::IDENTITY;
    local.cols[3][0] = 2.0_f32.powi(-30);

    let world = compose(
        parent.cols.map(|column| column.map(f64::from)),
        local.cols.map(|column| column.map(f64::from)),
    )
    .unwrap();

    assert_eq!(world[3][0], 1.0 + 2.0_f64.powi(-30));
}

#[test]
fn precision_contract_world_positions_retain_f64_residual() {
    let mut matrix = Mat4::IDENTITY;
    matrix.cols[3][0] = 1.0;

    let point = transform_point(
        matrix.cols.map(|column| column.map(f64::from)),
        [2.0_f32.powi(-30), 0.0, 0.0].map(f64::from),
    )
    .unwrap();

    assert_eq!(point[0], 1.0 + 2.0_f64.powi(-30));
}

#[test]
fn precision_contract_finite_world_transforms_exceeding_f32_range_are_retained() {
    let mut parent = Mat4::IDENTITY;
    parent.cols[0][0] = f32::MAX;
    let mut local = Mat4::IDENTITY;
    local.cols[0][0] = 2.0;

    let world = compose(
        parent.cols.map(|column| column.map(f64::from)),
        local.cols.map(|column| column.map(f64::from)),
    )
    .unwrap();

    assert_eq!(world[0][0], f64::from(f32::MAX) * 2.0);
}

#[test]
fn precision_contract_finite_world_positions_outside_f32_range_are_retained() {
    for (scale, position, expected) in [
        (f32::MAX, 2.0, f64::from(f32::MAX) * 2.0),
        (0.5, f32::from_bits(1), f64::from(f32::from_bits(1)) * 0.5),
    ] {
        let mut matrix = Mat4::IDENTITY;
        matrix.cols[0][0] = scale;

        let point = transform_point(
            matrix.cols.map(|column| column.map(f64::from)),
            [position, 0.0, 0.0].map(f64::from),
        )
        .unwrap();

        assert_eq!(point[0], expected);
    }
}

#[test]
fn trs_rotation_negative_and_zero_scales_need_no_inverse() {
    let authored = AuthoredTransform::try_trs(
        Vec3::new(5.0, 6.0, 7.0),
        Quat::from_xyzw(0.5, 0.5, 0.5, 0.5),
        Vec3::new(-2.0, 3.0, 0.0),
    )
    .unwrap();
    let matrix = checked_local(authored).unwrap();
    assert_eq!(
        matrix,
        authored
            .local_matrix()
            .cols
            .map(|column| column.map(f64::from))
    );
    assert_eq!(
        transform_point(matrix, [1.0, 2.0, 9.0]).unwrap(),
        [5.0, 4.0, 13.0]
    );
    assert_eq!(
        transform_point(matrix, [-1.0, -2.0, -9.0]).unwrap(),
        [5.0, 8.0, 1.0]
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
    let world = compose(
        parent.cols.map(|column| column.map(f64::from)),
        local.cols.map(|column| column.map(f64::from)),
    )
    .unwrap();
    assert_eq!(world[3], [4.0, -4.0, 1.0, 1.0]);
    assert_eq!(world.map(|column| column[3]), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(
        transform_point(world, [2.0, 1.0, 4.0]).unwrap(),
        [1.0, -8.0, 3.0]
    );

    let shear = Mat4 {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [1.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let shear = shear.cols.map(|column| column.map(f64::from));
    assert_eq!(compose(IDENTITY, shear).unwrap(), shear);
    let mut bounds = None;
    for point in [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        include_point(&mut bounds, transform_point(shear, point).unwrap());
    }
    assert_eq!(
        bounds,
        Some(Aabb {
            min: [0.0; 3],
            max: [1.0, 1.0, 0.0]
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
    let matrix = matrix.cols.map(|column| column.map(f64::from));
    assert_eq!(
        transform_point(matrix, [2.0, -1.0, 0.0]).unwrap(),
        [0.0, -1.0, 0.0]
    );
    let mut local = Mat4::IDENTITY;
    local.cols[0] = [2.0, -1.0, 0.0, 0.0];
    assert_eq!(
        compose(matrix, local.cols.map(|column| column.map(f64::from))).unwrap()[0],
        [f64::from(large), -1.0, 0.0, 0.0]
    );
}

#[test]
fn bounds_describe_computed_f64_world_vertices() {
    let mut matrix = IDENTITY;
    matrix[1][0] = 1.0;
    assert_eq!(
        transform_point(matrix, [1.0, 2.0_f64.powi(-25), 0.0]).unwrap(),
        [1.0 + 2.0_f64.powi(-25), 2.0_f64.powi(-25), 0.0]
    );
}

#[test]
fn f64_overflow_and_nonzero_underflow_are_explicit_numeric_errors() {
    let mut scale = IDENTITY;
    scale[0][0] = f64::MAX;
    assert_eq!(
        transform_point(scale, [2.0, 0.0, 0.0]),
        Err(NumericConversionError::NonFiniteOutput)
    );
    let mut twice = IDENTITY;
    twice[0][0] = 2.0;
    assert_eq!(
        compose(twice, scale),
        Err(NumericConversionError::NonFiniteOutput)
    );
    scale[0][0] = 0.5;
    assert_eq!(
        transform_point(scale, [f64::from_bits(1), 0.0, 0.0]),
        Err(NumericConversionError::UnderflowToZero)
    );
    twice[0][0] = f64::from_bits(1);
    assert_eq!(
        compose(twice, scale),
        Err(NumericConversionError::UnderflowToZero)
    );
    assert_eq!(
        checked_product(f64::from_bits(1), 1.0),
        Ok(f64::from_bits(1))
    );
    assert_eq!(
        checked_product(-0.0, 1.0).unwrap().to_bits(),
        (-0.0_f64).to_bits()
    );
    assert_eq!(
        transform_point(IDENTITY, [f64::from_bits(1), 0.0, 0.0]),
        Ok([f64::from_bits(1), 0.0, 0.0])
    );
}

#[test]
fn nonfinite_accumulation_rejects_and_exact_cancellation_is_accepted() {
    let mut matrix = IDENTITY;
    matrix[0][0] = f64::MAX;
    matrix[1][0] = f64::MAX;
    assert_eq!(
        transform_point(matrix, [1.0, 1.0, 0.0]),
        Err(NumericConversionError::NonFiniteOutput)
    );
    assert_eq!(
        transform_point(matrix, [1.0, -1.0, 0.0]),
        Ok([0.0, -1.0, 0.0])
    );
    let mut local = IDENTITY;
    local[0] = [1.0, 1.0, 0.0, 0.0];
    assert_eq!(
        compose(matrix, local),
        Err(NumericConversionError::NonFiniteOutput)
    );
}

#[test]
fn accepted_trs_retains_derived_cells_exceeding_f32_range() {
    let rotation = Quat::from_xyzw(0.0, 1.00002, 0.0, 0.0);
    assert!(rotation.is_unit());
    let authored =
        AuthoredTransform::try_trs(Vec3::ZERO, rotation, Vec3::new(f32::MAX, 1.0, 1.0)).unwrap();
    let matrix = checked_local(authored).unwrap();
    let y = f64::from(rotation.y);
    assert_eq!(matrix[0][0], (1.0 - y * (y + y)) * f64::from(f32::MAX));
    assert!(matrix[0][0] < -f64::from(f32::MAX));
}

#[test]
fn a_nonzero_trs_cell_below_f32_range_is_retained() {
    let authored = AuthoredTransform::try_trs(
        Vec3::ZERO,
        Quat::from_xyzw(0.0, 0.0, 2.0_f32.powi(-75), 1.0),
        Vec3::new(f32::from_bits(1), 1.0, 1.0),
    )
    .unwrap();
    let matrix = checked_local(authored).unwrap();
    assert_eq!(matrix[0][1], 2.0_f64.powi(-223));
}

#[test]
fn lookup_checks_generation_before_range_and_survives_without_scene() {
    let generation = SceneGenerationId::allocate().unwrap();
    let table = BasePoseWorldTransforms {
        generation,
        transforms: vec![IDENTITY],
    };
    assert_eq!(
        table.node_transform(NodeId::new(generation, 0)).unwrap(),
        IDENTITY
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
