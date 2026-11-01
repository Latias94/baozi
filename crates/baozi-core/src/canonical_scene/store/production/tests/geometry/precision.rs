use super::*;

#[test]
fn precision_contract_sparse_f64_charges_explicit_values_defaults_and_indices_at_exact_limits() {
    let indices = [0, u32::MAX - 1];
    let values = [
        16_777_217.0_f64,
        -0.0,
        f64::from_bits(1),
        f64::MAX,
        1.0,
        -1.0,
    ];
    let default = [-0.0, f64::from_bits(1), 0.0];
    let attributes = [GeometryAttributeAuthoring::sparse_f64(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        u32::MAX,
        &indices,
        &values,
        &default,
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let input = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]);
    let bytes = expected_geometry_bytes([1, 1, 1, 0], [0, 9, 2, 0], 0);
    let limits = SceneValidationLimits::default()
        .with_max_canonical_bytes(bytes)
        .with_max_payload_cells(11);
    let mut control = TestControl::default();
    let batch = validate_geometry_batch_with_control(input, 0, 0, limits, &mut control)
        .expect("sparse F64 storage must depend only on explicit data and one default");
    assert_eq!(batch.canonical_bytes(), bytes);
    assert_eq!(batch.semantic_cell_count(), 11);
    assert_eq!(batch.f32_scalar_count(), 0);
    assert_eq!(batch.f64_scalar_count(), 9);
    assert_eq!(batch.u32_scalar_count(), 2);
    assert_eq!(batch.payload_block_count(), 3);
    assert_eq!(batch.primitive(0).unwrap().vertex_count(), u32::MAX);
    let attribute = batch.attribute(0).unwrap();
    assert_eq!(attribute.scalar_kind(), GeometryAttributeScalarKind::F64);
    assert_eq!(
        attribute.payload_kind(),
        GeometryAttributePayloadKind::Sparse
    );
    assert_eq!(attribute.element_count(), u32::MAX);
    assert_eq!(attribute.explicit_element_count(), 2);
    assert_eq!(attribute.canonical_scalar_count(), 9);
    let work = batch.validation_work();
    assert_eq!(control.work, work);
    assert!(work < crate::SCENE_VALIDATION_POLL_INTERVAL);
    drop(batch);
    assert_eq!(control.live.get(), 0);

    for (limits, expected_limit, expected_actual) in [
        (
            limits.with_max_canonical_bytes(bytes - 1),
            GeometryAuthoringLimit::CanonicalBytes,
            bytes,
        ),
        (
            limits.with_max_payload_cells(10),
            GeometryAuthoringLimit::SemanticCells,
            11,
        ),
        (
            limits.with_max_work(work - 1),
            GeometryAuthoringLimit::Work,
            work,
        ),
    ] {
        let mut control = TestControl::default();
        assert!(matches!(
            expect_semantic(validate_geometry_batch_with_control(input, 0, 0, limits, &mut control)),
            GeometryAuthoringSemanticError::LimitExceeded { limit, actual, maximum }
                if limit == expected_limit && actual == expected_actual && maximum + 1 == actual
        ));
        assert_eq!(control.live.get(), 0);
    }
}

#[test]
fn precision_contract_f64_builtins_reject_nonfinite_dense_values_sparse_values_and_defaults() {
    let positions = [0.0_f32; 3];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let one_attribute = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let two_attributes = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 2),
    )];
    for (semantic, lanes) in [
        (AttributeSemantic::Position, 3),
        (AttributeSemantic::Normal, 3),
        (AttributeSemantic::Tangent, 4),
        (AttributeSemantic::Texcoord(1), 2),
    ] {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for location in 0..3 {
                let mut values = [1.0_f64; 4];
                let mut default = [0.0_f64; 4];
                let lane_count = usize::from(lanes);
                if location == 2 {
                    default[lane_count - 1] = invalid;
                } else {
                    values[lane_count - 1] = invalid;
                }
                let attribute = if location == 0 {
                    GeometryAttributeAuthoring::f64(
                        semantic.clone(),
                        AttributeDomain::Vertex,
                        lanes,
                        &values[..lane_count],
                    )
                } else {
                    GeometryAttributeAuthoring::sparse_f64(
                        semantic.clone(),
                        AttributeDomain::Vertex,
                        lanes,
                        1,
                        &[0],
                        &values[..lane_count],
                        &default[..lane_count],
                    )
                };
                let attributes = [
                    GeometryAttributeAuthoring::f32(
                        AttributeSemantic::Position,
                        AttributeDomain::Vertex,
                        3,
                        &positions,
                        Presence::Absent,
                    ),
                    attribute,
                ];
                let (primitives, attributes, attribute_index) =
                    if semantic == AttributeSemantic::Position {
                        (&one_attribute, &attributes[1..], 0)
                    } else {
                        (&two_attributes, &attributes[..], 1)
                    };
                let scalar_index = if location == 2 {
                    2 * lane_count - 1
                } else {
                    lane_count - 1
                };
                assert!(matches!(
                    reject_geometry(GeometryAuthoringBatch::new(&meshes, primitives, attributes, &[]), 0),
                    GeometryAuthoringSemanticError::NonFiniteAttributeValue { attribute, scalar }
                        if attribute == attribute_index && scalar == scalar_index
                ));
            }
        }
    }
}

#[test]
fn precision_contract_f64_admission_keeps_builtin_lanes_domains_and_color_restrictions() {
    let values = [0.0_f64; 4];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    for (semantic, domain, lanes) in [
        (AttributeSemantic::Position, AttributeDomain::Corner, 3),
        (AttributeSemantic::Position, AttributeDomain::Vertex, 2),
        (AttributeSemantic::Normal, AttributeDomain::Face, 3),
        (AttributeSemantic::Normal, AttributeDomain::Vertex, 2),
        (AttributeSemantic::Tangent, AttributeDomain::Face, 4),
        (AttributeSemantic::Tangent, AttributeDomain::Corner, 3),
        (AttributeSemantic::Texcoord(0), AttributeDomain::Face, 2),
        (AttributeSemantic::Texcoord(0), AttributeDomain::Vertex, 4),
        (AttributeSemantic::Color(0), AttributeDomain::Face, 3),
        (AttributeSemantic::SmoothingGroup, AttributeDomain::Face, 1),
        (AttributeSemantic::Normal, AttributeDomain::Instance, 3),
    ] {
        for sparse in [false, true] {
            let attributes = [if sparse {
                GeometryAttributeAuthoring::sparse_f64(
                    semantic.clone(),
                    domain,
                    lanes,
                    1,
                    &[],
                    &[],
                    &values[..usize::from(lanes)],
                )
            } else {
                GeometryAttributeAuthoring::f64(
                    semantic.clone(),
                    domain,
                    lanes,
                    &values[..usize::from(lanes)],
                )
            }];
            assert!(matches!(
                reject_geometry(
                    GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
                    0
                ),
                GeometryAuthoringSemanticError::AttributeSchema { attribute: 0 }
            ));
        }
    }
}

#[test]
fn precision_contract_sparse_f64_checks_indices_tuple_shapes_and_domain_cardinality() {
    let positions = [0.0_f32; 9];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 2),
    )];
    let values = [1.0_f64; 6];
    let default = [0.0_f64; 3];
    for case in 0..7 {
        let (logical_count, indices, values, default): (u32, &[u32], &[f64], &[f64]) = match case {
            0 => (3, &[0, 0], &values, &default),
            1 => (3, &[2, 1], &values, &default),
            2 => (3, &[3], &values[..3], &default),
            3 => (3, &[0, 1], &values[..3], &default),
            4 => (3, &[0], &values[..2], &default),
            5 => (3, &[0], &values[..3], &default[..2]),
            6 => (2, &[0], &values[..3], &default),
            _ => unreachable!(),
        };
        let attributes = [
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &positions,
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::sparse_f64(
                AttributeSemantic::Normal,
                AttributeDomain::Corner,
                3,
                logical_count,
                indices,
                values,
                default,
            ),
        ];
        let error = reject_geometry(
            GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
            0,
        );
        match case {
            0 | 1 => assert!(matches!(
                error,
                GeometryAuthoringSemanticError::SparseIndexOrder {
                    attribute: 1,
                    element: 1
                }
            )),
            2 => assert!(matches!(
                error,
                GeometryAuthoringSemanticError::SparseIndexOutOfRange {
                    attribute: 1,
                    element: 0,
                    index: 3,
                    logical_count: 3
                }
            )),
            3 => assert!(matches!(
                error,
                GeometryAuthoringSemanticError::SparseValueCount {
                    attribute: 1,
                    index_count: 2,
                    value_elements: 1
                }
            )),
            4 => assert!(matches!(
                error,
                GeometryAuthoringSemanticError::AttributeScalarShape {
                    attribute: 1,
                    lanes: 3
                }
            )),
            5 => assert!(matches!(
                error,
                GeometryAuthoringSemanticError::SparseDefaultShape {
                    attribute: 1,
                    lanes: 3,
                    actual: 2
                }
            )),
            6 => assert!(matches!(
                error,
                GeometryAuthoringSemanticError::AttributeCardinality {
                    attribute: 1,
                    domain: AttributeDomain::Corner,
                    expected: 3,
                    actual: 2
                }
            )),
            _ => unreachable!(),
        }
    }
}

#[test]
fn precision_contract_sparse_f64_default_only_geometry_is_bounded_and_cancellable() {
    let default = [-0.0_f64, f64::from_bits(1), 16_777_217.0];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    for count in [0, u32::MAX] {
        let attributes = [GeometryAttributeAuthoring::sparse_f64(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            count,
            &[],
            &[],
            &default,
        )];
        let input = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]);
        let mut control = TestControl::default();
        let batch = validate_geometry_batch_with_control(
            input,
            0,
            0,
            SceneValidationLimits::default(),
            &mut control,
        )
        .unwrap();
        assert_eq!(batch.f64_scalar_count(), 3);
        assert_eq!(batch.u32_scalar_count(), 0);
        assert_eq!(batch.payload_block_count(), 1);
        assert_eq!(
            batch.canonical_bytes(),
            expected_geometry_bytes([1, 1, 1, 0], [0, 3, 0, 0], 0)
        );
        assert!(batch.validation_work() < crate::SCENE_VALIDATION_POLL_INTERVAL);
        drop(batch);
        assert_eq!(control.live.get(), 0);

        let mut cancelled = TestControl {
            cancel_on_poll: Some(2),
            ..TestControl::default()
        };
        assert!(matches!(
            validate_geometry_batch_with_control(
                input,
                0,
                0,
                SceneValidationLimits::default(),
                &mut cancelled
            ),
            Err(GeometryAuthoringFailure::Control(
                TestControlError::Cancelled
            ))
        ));
        assert_eq!(cancelled.live.get(), 0);
    }
}

#[test]
fn precision_contract_sparse_f64_scalar_validation_polls_before_finishing_the_values() {
    let indices: [u32; 300] = std::array::from_fn(|index| index as u32);
    let values = [f64::from_bits(1); 900];
    let default = [-0.0_f64; 3];
    let attributes = [GeometryAttributeAuthoring::sparse_f64(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        300,
        &indices,
        &values,
        &default,
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let mut control = TestControl {
        cancel_on_poll: Some(2),
        ..TestControl::default()
    };
    assert!(matches!(
        validate_geometry_batch_with_control(
            GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
            0,
            0,
            SceneValidationLimits::default(),
            &mut control,
        ),
        Err(GeometryAuthoringFailure::Control(
            TestControlError::Cancelled
        ))
    ));
    assert_eq!(control.polls, 2);
    assert_eq!(control.work, crate::SCENE_VALIDATION_POLL_INTERVAL);
    assert_eq!(
        control.last_work(),
        Some(crate::SceneValidationWork::PayloadCell)
    );
    assert_eq!(control.live.get(), 0);
}
