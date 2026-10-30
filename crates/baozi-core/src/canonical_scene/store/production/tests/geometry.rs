use std::cell::Cell;
use std::rc::Rc;

use crate::SceneValidationLimits;
use crate::attribute::{AttributeDomain, AttributeSemantic, CustomAttributeSemantic};
use crate::canonical_scene::store::production::plan::geometry::{
    GeometryAttributePayloadKind, GeometryAttributeScalarKind, GeometryAuthoringFailure,
    GeometryAuthoringLimit, GeometryAuthoringSemanticError, ValidatedGeometryBatch,
    validate_geometry_batch_with_control,
};
use crate::canonical_scene::store::production::{
    AuthoringRange, GeometryAttributeAuthoring, GeometryAuthoringBatch,
    GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring,
    GeometryTopologyAuthoring, LogicalMeshLocalId, MaterialLocalId, NodeLocalId, Presence,
};
use crate::geometry::TopologyKind;

use super::{TestControl, TestControlError, TestReservation};

fn expected_geometry_bytes(rows: [usize; 4], scalars: [usize; 4], name_bytes: usize) -> u64 {
    use crate::canonical_scene::store::typed_storage::{
        GeometryStorage, StoredAttribute, StoredLogicalMesh, StoredMeshInstance, StoredPrimitive,
    };
    let row_bytes: usize = rows
        .into_iter()
        .zip([
            std::mem::size_of::<StoredLogicalMesh>(),
            std::mem::size_of::<StoredPrimitive>(),
            std::mem::size_of::<StoredAttribute>(),
            std::mem::size_of::<StoredMeshInstance>(),
        ])
        .map(|(count, size)| count * size)
        .sum();
    let scalar_bytes: usize = scalars
        .into_iter()
        .zip([
            std::mem::size_of::<f32>(),
            std::mem::size_of::<f64>(),
            std::mem::size_of::<u32>(),
            std::mem::size_of::<i32>(),
        ])
        .map(|(count, size)| count * size)
        .sum();
    (std::mem::size_of::<GeometryStorage>() + row_bytes + scalar_bytes + name_bytes) as u64
}

fn expect_semantic(
    result: Result<
        ValidatedGeometryBatch<'_, TestReservation>,
        GeometryAuthoringFailure<TestControlError>,
    >,
) -> GeometryAuthoringSemanticError {
    match result {
        Err(GeometryAuthoringFailure::Semantic(error)) => error,
        Err(GeometryAuthoringFailure::Control(error)) => {
            panic!("expected semantic failure, got control failure: {error}")
        }
        Ok(batch) => panic!("expected semantic failure, got {batch:?}"),
    }
}

fn reject_geometry(
    input: GeometryAuthoringBatch<'_>,
    authored_node_count: usize,
) -> GeometryAuthoringSemanticError {
    let mut control = TestControl::default();
    let error = expect_semantic(validate_geometry_batch_with_control(
        input,
        authored_node_count,
        0,
        SceneValidationLimits::default(),
        &mut control,
    ));
    assert_eq!(control.live.get(), 0);
    error
}

#[test]
fn empty_geometry_charges_its_initialized_header_at_the_exact_limit() {
    let expected = expected_geometry_bytes([0; 4], [0; 4], 0);
    let mut control = TestControl::default();
    let limits = SceneValidationLimits::default().with_max_canonical_bytes(expected);
    let batch = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::empty(),
        0,
        0,
        limits,
        &mut control,
    )
    .unwrap();
    assert_eq!(batch.canonical_bytes(), expected);
    assert_eq!(batch.attribute_count(), 0);
    assert_eq!(batch.primitive_count(), 0);
    drop(batch);
    assert_eq!(control.live.get(), 0);
    assert!(matches!(
        expect_semantic(validate_geometry_batch_with_control(
            GeometryAuthoringBatch::empty(),
            0,
            0,
            limits.with_max_canonical_bytes(expected - 1),
            &mut control,
        )),
        GeometryAuthoringSemanticError::LimitExceeded {
            limit: GeometryAuthoringLimit::CanonicalBytes, actual, maximum,
        } if actual == expected && maximum + 1 == expected
    ));
    assert_eq!(control.live.get(), 0);
}

#[test]
fn f64_custom_attributes_charge_eight_bytes_per_scalar_and_exact_work() {
    let positions = [0.0_f32; 9];
    let values = [
        16_777_217.0_f64,
        -0.0,
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        -f64::MAX,
    ];
    let f32_values = [1.0_f32; 6];
    let semantic = AttributeSemantic::Custom(CustomAttributeSemantic::new("ply:precise").unwrap());
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f64(semantic.clone(), AttributeDomain::Vertex, 2, &values),
    ];
    let f32_attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            semantic,
            AttributeDomain::Vertex,
            2,
            &f32_values,
            Presence::Absent,
        ),
    ];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 2),
    )];
    let input = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]);
    let mut control = TestControl::default();
    let validated = validate_geometry_batch_with_control(
        input,
        0,
        0,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("finite F64 values must retain their own scalar type");
    let mut f32_control = TestControl::default();
    let f32_validated = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::new(&meshes, &primitives, &f32_attributes, &[]),
        0,
        0,
        SceneValidationLimits::default(),
        &mut f32_control,
    )
    .expect("equivalent F32 geometry shape");

    assert_eq!(validated.f32_scalar_count(), 9);
    assert_eq!(validated.f64_scalar_count(), 6);
    assert_eq!(validated.payload_block_count(), 3);
    assert_eq!(
        validated.semantic_cell_count(),
        f32_validated.semantic_cell_count()
    );
    assert_eq!(
        validated.canonical_bytes(),
        f32_validated.canonical_bytes() + 6 * 4
    );
    assert_eq!(validated.validation_work(), f32_validated.validation_work());
    assert_eq!(control.work, validated.validation_work());
    let primitive = validated.primitive(0).unwrap();
    let attribute = primitive.attribute(1).unwrap();
    assert_eq!(attribute.scalar_kind(), GeometryAttributeScalarKind::F64);
    assert_eq!(
        attribute.payload_kind(),
        GeometryAttributePayloadKind::Dense
    );
    assert_eq!(attribute.element_count(), 3);
    assert_eq!(attribute.canonical_scalar_count(), 6);

    let mut limited = TestControl::default();
    assert!(matches!(
        expect_semantic(validate_geometry_batch_with_control(
            input, 0, 0,
            SceneValidationLimits::default().with_max_canonical_bytes(validated.canonical_bytes() - 1),
            &mut limited,
        )),
        GeometryAuthoringSemanticError::LimitExceeded {
            limit: GeometryAuthoringLimit::CanonicalBytes, actual, maximum,
        } if actual == validated.canonical_bytes() && maximum + 1 == actual
    ));
    assert_eq!(limited.live.get(), 0);
}

#[test]
fn f64_custom_attributes_reject_non_finite_values_in_every_primitive_domain() {
    let positions = [0.0_f32; 9];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 2),
    )];
    for domain in [
        AttributeDomain::Vertex,
        AttributeDomain::Corner,
        AttributeDomain::Face,
    ] {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let values = [1.0, 2.0, invalid];
            let attributes = [
                GeometryAttributeAuthoring::f32(
                    AttributeSemantic::Position,
                    AttributeDomain::Vertex,
                    3,
                    &positions,
                    Presence::Absent,
                ),
                GeometryAttributeAuthoring::f64(
                    AttributeSemantic::Custom(CustomAttributeSemantic::new("ply:precise").unwrap()),
                    domain,
                    1,
                    &values,
                ),
            ];
            assert!(matches!(
                reject_geometry(
                    GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
                    0
                ),
                GeometryAuthoringSemanticError::NonFiniteAttributeValue {
                    attribute: 1,
                    scalar: 2
                }
            ));
        }
    }
}

#[test]
fn f64_custom_attributes_enforce_lanes_shape_and_domain_cardinality() {
    let positions = [0.0_f32; 9];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 2),
    )];
    for (lanes, values) in [
        (0, vec![]),
        (2, vec![1.0; 3]),
        (17, vec![1.0; 51]),
        (1, vec![1.0; 2]),
    ] {
        for domain in [
            AttributeDomain::Vertex,
            AttributeDomain::Corner,
            AttributeDomain::Face,
        ] {
            let attributes = [
                GeometryAttributeAuthoring::f32(
                    AttributeSemantic::Position,
                    AttributeDomain::Vertex,
                    3,
                    &positions,
                    Presence::Absent,
                ),
                GeometryAttributeAuthoring::f64(
                    AttributeSemantic::Custom(CustomAttributeSemantic::new("ply:precise").unwrap()),
                    domain,
                    lanes,
                    &values,
                ),
            ];
            let error = reject_geometry(
                GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
                0,
            );
            match lanes {
                0 => assert!(matches!(
                    error,
                    GeometryAuthoringSemanticError::ZeroAttributeLanes { attribute: 1 }
                )),
                2 => assert!(matches!(
                    error,
                    GeometryAuthoringSemanticError::AttributeScalarShape {
                        attribute: 1,
                        lanes: 2
                    }
                )),
                17 => assert!(matches!(
                    error,
                    GeometryAuthoringSemanticError::AttributeSchema { attribute: 1 }
                )),
                1 => assert!(
                    matches!(error, GeometryAuthoringSemanticError::AttributeCardinality { attribute: 1, domain: actual_domain, expected: 3, actual: 2 } if actual_domain == domain)
                ),
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn f64_payloads_cannot_replace_builtin_f32_semantics() {
    let values = [0.0_f64; 4];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    for (semantic, lanes) in [
        (AttributeSemantic::Position, 3),
        (AttributeSemantic::Normal, 3),
        (AttributeSemantic::Tangent, 4),
        (AttributeSemantic::Texcoord(0), 2),
        (AttributeSemantic::Color(0), 4),
    ] {
        let attributes = [GeometryAttributeAuthoring::f64(
            semantic,
            AttributeDomain::Vertex,
            lanes,
            &values[..usize::from(lanes)],
        )];
        assert!(matches!(
            reject_geometry(
                GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
                0
            ),
            GeometryAuthoringSemanticError::AttributeSchema { attribute: 0 }
        ));
    }
}

#[test]
fn compiled_geometry_facts_do_not_borrow_authored_storage() {
    let mut control = TestControl::default();
    let compiled = {
        let positions = vec![0.0_f32, 0.0, 0.0];
        let attributes = vec![GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        )];
        let primitives = vec![GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Absent,
            Presence::Absent,
            AuthoringRange::new(0, 1),
        )];
        let logical_meshes = vec![GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        let instances = vec![GeometryMeshInstanceAuthoring::new(
            NodeLocalId::new(0),
            LogicalMeshLocalId::new(0),
        )];

        validate_geometry_batch_with_control(
            GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &instances),
            1,
            0,
            SceneValidationLimits::default(),
            &mut control,
        )
        .expect("temporary authored storage must compile into owned geometry facts")
        .into_compiled_facts_for_test()
    };

    assert_eq!(compiled.0, 1);
    assert_eq!(compiled.1, 1);
    assert_eq!(compiled.2, 1);
    assert_eq!(compiled.3, 1);
    assert_eq!(compiled.4, 1);
    assert_eq!(compiled.5, 1);

    drop(compiled);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn obj_corner_domains_preserve_shared_positions_without_tuple_vertex_expansion() {
    let positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let position_indices = [0_u32, 1, 2, 0, 2, 1];
    let normal_sources = [0.0_f32, 0.0, 1.0, 0.0, 0.0, -1.0];
    let normal_indices = [0_u32, 0, 0, 1, 1, 1];
    let texcoord_sources = [0.0_f32, 0.0, 1.0, 0.0, 0.0, 1.0, 0.5, 0.5];
    let texcoord_indices = [0_u32, 1, 2, 3, 2, 1];
    let smoothing_groups = [1_u32, 0];
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Corner,
            3,
            &normal_sources,
            Presence::Present(&normal_indices),
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Texcoord(0),
            AttributeDomain::Corner,
            2,
            &texcoord_sources,
            Presence::Present(&texcoord_indices),
        ),
        GeometryAttributeAuthoring::u32(
            AttributeSemantic::SmoothingGroup,
            AttributeDomain::Face,
            1,
            &smoothing_groups,
        ),
    ];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Present(&position_indices),
        Presence::Absent,
        AuthoringRange::new(0, 4),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    let input = GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &instances);
    let mut control = TestControl::default();

    let validated = validate_geometry_batch_with_control(
        input,
        1,
        0,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("OBJ-shaped geometry must preserve independent attribute domains");

    assert_eq!(validated.logical_mesh_count(), 1);
    assert_eq!(validated.primitive_count(), 1);
    assert_eq!(validated.mesh_instance_count(), 1);
    assert_eq!(validated.attribute_count(), 4);
    assert_eq!(validated.f32_scalar_count(), 39);
    assert_eq!(validated.u32_scalar_count(), 8);
    assert_eq!(validated.payload_block_count(), 5);
    assert_eq!(
        validated.canonical_bytes(),
        expected_geometry_bytes([1, 1, 4, 1], [39, 0, 8, 0], 0)
    );
    assert_eq!(validated.validation_work(), 73);
    assert_eq!(control.work, 73);

    let primitive = validated
        .primitive(0)
        .expect("the validated primitive must remain inspectable");
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(primitive.vertex_count(), 3);
    assert_eq!(primitive.corner_count(), 6);
    assert_eq!(primitive.face_count(), 2);
    assert_eq!(primitive.index_offset(), Some(0));
    assert_eq!(primitive.index_count(), 6);
    assert_eq!(primitive.material(), None::<MaterialLocalId>);

    assert_eq!(primitive.attributes().len(), 4);
    let position = primitive.attribute(0).expect("POSITION plan");
    assert_eq!(position.semantic(), &AttributeSemantic::Position);
    assert_eq!(position.domain(), AttributeDomain::Vertex);
    assert_eq!(position.source_element_count(), 3);
    assert_eq!(position.element_count(), 3);
    let normal = primitive.attribute(1).expect("NORMAL plan");
    assert_eq!(normal.semantic(), &AttributeSemantic::Normal);
    assert_eq!(normal.domain(), AttributeDomain::Corner);
    assert_eq!(normal.source_element_count(), 2);
    assert_eq!(normal.element_count(), 6);
    let texcoord = primitive.attribute(2).expect("TEXCOORD plan");
    assert_eq!(texcoord.semantic(), &AttributeSemantic::Texcoord(0));
    assert_eq!(texcoord.domain(), AttributeDomain::Corner);
    assert_eq!(texcoord.source_element_count(), 4);
    assert_eq!(texcoord.element_count(), 6);
    let smoothing = primitive.attribute(3).expect("SMOOTHING_GROUP plan");
    assert_eq!(smoothing.semantic(), &AttributeSemantic::SmoothingGroup);
    assert_eq!(smoothing.domain(), AttributeDomain::Face);
    assert_eq!(smoothing.source_element_count(), 2);
    assert_eq!(smoothing.element_count(), 2);
    assert!(primitive.attribute(4).is_none());
}

#[test]
fn custom_dense_and_sparse_i32_attributes_preserve_signed_values_and_exact_accounting() {
    let positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let dense_values = [i32::MIN, -1, i32::MAX];
    let sparse_indices = [1_u32];
    let sparse_values = [i32::MIN];
    let sparse_default = [i32::MAX];
    let dense_semantic = CustomAttributeSemantic::new("ply:dense_signed")
        .expect("dense custom semantic must be canonical");
    let sparse_semantic = CustomAttributeSemantic::new("ply:sparse_signed")
        .expect("sparse custom semantic must be canonical");
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::i32(
            AttributeSemantic::Custom(dense_semantic),
            AttributeDomain::Vertex,
            1,
            &dense_values,
        ),
        GeometryAttributeAuthoring::sparse_i32(
            AttributeSemantic::Custom(sparse_semantic),
            AttributeDomain::Vertex,
            1,
            3,
            &sparse_indices,
            &sparse_values,
            &sparse_default,
        ),
    ];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 3),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let mut control = TestControl::default();

    let validated = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &[]),
        0,
        0,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("PLY-shaped signed custom attributes must validate losslessly");

    assert_eq!(validated.attribute_count(), 3);
    assert_eq!(validated.attribute_semantic_name_byte_count(), 33);
    assert_eq!(validated.f32_scalar_count(), 9);
    assert_eq!(validated.i32_scalar_count(), 5);
    assert_eq!(validated.u32_scalar_count(), 1);
    assert_eq!(validated.payload_block_count(), 6);
    assert_eq!(validated.semantic_cell_count(), 48);
    assert_eq!(
        validated.canonical_bytes(),
        expected_geometry_bytes([1, 1, 3, 0], [9, 0, 1, 5], 33)
    );
    assert_eq!(validated.validation_work(), 69);
    assert_eq!(control.work, 69);

    let primitive = validated.primitive(0).expect("validated primitive");
    let dense = primitive.attribute(1).expect("dense I32 attribute");
    assert_eq!(dense.scalar_kind(), GeometryAttributeScalarKind::I32);
    assert_eq!(dense.payload_kind(), GeometryAttributePayloadKind::Dense);
    assert_eq!(dense.semantic_name_offset(), 0);
    assert_eq!(dense.semantic_name_count(), 16);
    assert_eq!(dense.canonical_scalar_count(), 3);
    assert_eq!(dense.payload_block_count(), 1);

    let sparse = primitive.attribute(2).expect("sparse I32 attribute");
    assert_eq!(sparse.scalar_kind(), GeometryAttributeScalarKind::I32);
    assert_eq!(sparse.payload_kind(), GeometryAttributePayloadKind::Sparse);
    assert_eq!(sparse.semantic_name_offset(), 16);
    assert_eq!(sparse.semantic_name_count(), 17);
    assert_eq!(sparse.explicit_element_count(), 1);
    assert_eq!(sparse.canonical_scalar_count(), 2);
    assert_eq!(sparse.payload_block_count(), 3);
}

#[test]
fn absent_and_present_empty_index_streams_remain_distinct() {
    let positions = [0.0_f32, 0.0, 0.0];
    let empty_indices = [];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];

    let absent_primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let mut absent_control = TestControl::default();
    let absent = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::new(&logical_meshes, &absent_primitives, &attributes, &[]),
        0,
        0,
        SceneValidationLimits::default(),
        &mut absent_control,
    )
    .expect("absent indices describe a non-indexed point");
    let absent_primitive = absent.primitive(0).expect("absent-index primitive");
    assert_eq!(absent_primitive.index_offset(), None);
    assert_eq!(absent_primitive.index_count(), 0);
    assert_eq!(absent_primitive.corner_count(), 1);
    assert_eq!(absent_primitive.face_count(), 1);

    let empty_primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Present(&empty_indices),
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let mut empty_control = TestControl::default();
    let present_empty = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::new(&logical_meshes, &empty_primitives, &attributes, &[]),
        0,
        0,
        SceneValidationLimits::default(),
        &mut empty_control,
    )
    .expect("present-empty indices describe an indexed primitive with no corners");
    let empty_primitive = present_empty
        .primitive(0)
        .expect("present-empty-index primitive");
    assert_eq!(empty_primitive.index_offset(), Some(0));
    assert_eq!(empty_primitive.index_count(), 0);
    assert_eq!(empty_primitive.corner_count(), 0);
    assert_eq!(empty_primitive.face_count(), 0);
    assert_eq!(present_empty.payload_block_count(), 1);
    assert_eq!(present_empty.canonical_bytes(), absent.canonical_bytes());
}

#[test]
fn sparse_attribute_counts_indices_values_and_default_without_dense_expansion() {
    let positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    let sparse_indices = [1_u32];
    let sparse_values = [1.0_f32, 0.5, 0.25];
    let sparse_default = [0.0_f32, 0.0, 0.0];
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::sparse_f32(
            AttributeSemantic::Color(0),
            AttributeDomain::Face,
            3,
            2,
            &sparse_indices,
            &sparse_values,
            &sparse_default,
        ),
    ];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 2),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let mut control = TestControl::default();

    let validated = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &[]),
        0,
        0,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("sparse face color must retain sparse shape");

    assert_eq!(validated.f32_scalar_count(), 12);
    assert_eq!(validated.u32_scalar_count(), 1);
    assert_eq!(validated.payload_block_count(), 4);
    assert_eq!(
        validated.canonical_bytes(),
        expected_geometry_bytes([1, 1, 2, 0], [12, 0, 1, 0], 0)
    );
    let primitive = validated.primitive(0).expect("sparse primitive plan");
    let color = primitive.attribute(1).expect("sparse color plan");
    assert_eq!(color.source_element_count(), 1);
    assert_eq!(color.element_count(), 2);
    assert_eq!(color.explicit_element_count(), 1);
    assert_eq!(color.canonical_scalar_count(), 6);
    assert_eq!(color.payload_block_count(), 3);
}

#[test]
fn nonempty_vertex_domain_requires_position() {
    let normals = [0.0_f32, 0.0, 1.0];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Normal,
        AttributeDomain::Vertex,
        3,
        &normals,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let mut control = TestControl::default();

    assert!(matches!(
        expect_semantic(validate_geometry_batch_with_control(
            GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &[]),
            0,
            0,
            SceneValidationLimits::default(),
            &mut control,
        )),
        GeometryAuthoringSemanticError::MissingPosition {
            primitive: 0,
            vertex_count: 1
        }
    ));
}

#[test]
fn flat_ownership_and_cross_table_references_are_exact() {
    let positions = [0.0_f32, 0.0, 0.0];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let valid_primitive = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let valid_mesh = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];

    let misplaced_mesh = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(1, 1))];
    assert!(matches!(
        reject_geometry(
            GeometryAuthoringBatch::new(&misplaced_mesh, &valid_primitive, &attributes, &[]),
            0,
        ),
        GeometryAuthoringSemanticError::LogicalMeshRangeOrder {
            logical_mesh: 0,
            expected: 0,
            actual: 1,
        }
    ));

    let misplaced_attributes = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(1, 1),
    )];
    assert!(matches!(
        reject_geometry(
            GeometryAuthoringBatch::new(&valid_mesh, &misplaced_attributes, &attributes, &[]),
            0,
        ),
        GeometryAuthoringSemanticError::PrimitiveAttributeRangeOrder {
            primitive: 0,
            expected: 0,
            actual: 1,
        }
    ));

    let unknown_material = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Present(MaterialLocalId::new(0)),
        AuthoringRange::new(0, 1),
    )];
    assert!(matches!(
        reject_geometry(
            GeometryAuthoringBatch::new(&valid_mesh, &unknown_material, &attributes, &[]),
            0,
        ),
        GeometryAuthoringSemanticError::MaterialOutOfRange {
            primitive: 0,
            material: 0,
            appearance_material_count: 0,
        }
    ));

    let mut material_control = TestControl::default();
    let material_geometry = validate_geometry_batch_with_control(
        GeometryAuthoringBatch::new(&valid_mesh, &unknown_material, &attributes, &[]),
        0,
        1,
        SceneValidationLimits::default(),
        &mut material_control,
    )
    .expect("appearance-owned material identity must validate through its explicit count");
    assert_eq!(material_geometry.entity_count(), 3);
    assert_eq!(
        material_geometry
            .primitive(0)
            .expect("material primitive")
            .material(),
        Some(MaterialLocalId::new(0))
    );

    let unknown_owner = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    assert!(matches!(
        reject_geometry(
            GeometryAuthoringBatch::new(&valid_mesh, &valid_primitive, &attributes, &unknown_owner,),
            0,
        ),
        GeometryAuthoringSemanticError::InstanceOwnerOutOfRange {
            instance: 0,
            owner: 0,
            node_count: 0,
        }
    ));

    let unknown_mesh = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(1),
    )];
    assert!(matches!(
        reject_geometry(
            GeometryAuthoringBatch::new(&valid_mesh, &valid_primitive, &attributes, &unknown_mesh,),
            1,
        ),
        GeometryAuthoringSemanticError::InstanceMeshOutOfRange {
            instance: 0,
            logical_mesh: 1,
            logical_mesh_count: 1,
        }
    ));
}

#[test]
fn malformed_indices_topology_projection_and_floats_are_rejected() {
    let positions = [0.0_f32, 0.0, 0.0];
    let position = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let mesh = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];

    let bad_index = [1_u32];
    let indexed = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Present(&bad_index),
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    assert!(matches!(
        reject_geometry(
            GeometryAuthoringBatch::new(&mesh, &indexed, &position, &[]),
            0,
        ),
        GeometryAuthoringSemanticError::VertexIndexOutOfRange {
            primitive: 0,
            corner: 0,
            index: 1,
            vertex_count: 1,
        }
    ));

    let incomplete_triangle = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    assert!(matches!(
        reject_geometry(
            GeometryAuthoringBatch::new(&mesh, &incomplete_triangle, &position, &[]),
            0,
        ),
        GeometryAuthoringSemanticError::ListCornerCount {
            primitive: 0,
            topology: TopologyKind::TriangleList,
            corner_count: 1,
            corners_per_face: 3,
        }
    ));

    let normal_source = [0.0_f32, 0.0, 1.0];
    let bad_projection = [1_u32];
    let projected = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Corner,
            3,
            &normal_source,
            Presence::Present(&bad_projection),
        ),
    ];
    let projected_primitive = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 2),
    )];
    assert!(matches!(
        reject_geometry(
            GeometryAuthoringBatch::new(&mesh, &projected_primitive, &projected, &[]),
            0,
        ),
        GeometryAuthoringSemanticError::SourceProjectionOutOfRange {
            attribute: 1,
            element: 0,
            index: 1,
            source_count: 1,
        }
    ));

    let non_finite = [f32::NAN, 0.0, 0.0];
    let invalid_position = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &non_finite,
        Presence::Absent,
    )];
    let points = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    assert!(matches!(
        reject_geometry(
            GeometryAuthoringBatch::new(&mesh, &points, &invalid_position, &[]),
            0,
        ),
        GeometryAuthoringSemanticError::NonFiniteAttributeValue {
            attribute: 0,
            scalar: 0,
        }
    ));
}

#[test]
fn sparse_shape_order_and_defaults_are_rejected_before_publication() {
    let positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mesh = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let primitive = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 2),
    )];
    let sparse_indices = [0_u32];
    let sparse_values = [1.0_f32, 0.5, 0.25];
    let short_default = [0.0_f32, 0.0];
    let short_default_attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::sparse_f32(
            AttributeSemantic::Color(0),
            AttributeDomain::Face,
            3,
            2,
            &sparse_indices,
            &sparse_values,
            &short_default,
        ),
    ];
    assert!(matches!(
        reject_geometry(
            GeometryAuthoringBatch::new(&mesh, &primitive, &short_default_attributes, &[]),
            0,
        ),
        GeometryAuthoringSemanticError::SparseDefaultShape {
            attribute: 1,
            lanes: 3,
            actual: 2,
        }
    ));

    let duplicate_indices = [0_u32, 0];
    let duplicate_values = [1.0_f32, 0.5, 0.25, 0.25, 0.5, 1.0];
    let sparse_default = [0.0_f32, 0.0, 0.0];
    let duplicate_attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::sparse_f32(
            AttributeSemantic::Color(0),
            AttributeDomain::Face,
            3,
            2,
            &duplicate_indices,
            &duplicate_values,
            &sparse_default,
        ),
    ];
    assert!(matches!(
        reject_geometry(
            GeometryAuthoringBatch::new(&mesh, &primitive, &duplicate_attributes, &[]),
            0,
        ),
        GeometryAuthoringSemanticError::SparseIndexOrder {
            attribute: 1,
            element: 1,
        }
    ));
}

#[test]
fn geometry_limits_are_typed_and_fail_without_leaking_scratch() {
    let positions = [0.0_f32, 0.0, 0.0];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let input = GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &[]);
    let cases = [
        (
            SceneValidationLimits::default().with_max_entities(2),
            GeometryAuthoringLimit::Entities,
        ),
        (
            SceneValidationLimits::default().with_max_edges(0),
            GeometryAuthoringLimit::Edges,
        ),
        (
            SceneValidationLimits::default().with_max_references(0),
            GeometryAuthoringLimit::References,
        ),
        (
            SceneValidationLimits::default().with_max_payload_cells(2),
            GeometryAuthoringLimit::SemanticCells,
        ),
        (
            SceneValidationLimits::default().with_max_canonical_bytes(0),
            GeometryAuthoringLimit::CanonicalBytes,
        ),
        (
            SceneValidationLimits::default().with_max_scratch_bytes(0),
            GeometryAuthoringLimit::ScratchBytes,
        ),
        (
            SceneValidationLimits::default().with_max_work(0),
            GeometryAuthoringLimit::Work,
        ),
    ];

    for (limits, expected_limit) in cases {
        let mut control = TestControl::default();
        assert!(matches!(
            expect_semantic(validate_geometry_batch_with_control(
                input,
                0,
                0,
                limits,
                &mut control,
            )),
            GeometryAuthoringSemanticError::LimitExceeded { limit, .. }
                if limit == expected_limit
        ));
        assert_eq!(control.live.get(), 0);
    }
}

#[test]
fn zero_work_rejects_before_geometry_scratch_allocation() {
    let positions = [0.0_f32, 0.0, 0.0];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let mut control = TestControl::default();

    assert!(matches!(
        expect_semantic(validate_geometry_batch_with_control(
            GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &[]),
            0,
            0,
            SceneValidationLimits::default().with_max_work(0),
            &mut control,
        )),
        GeometryAuthoringSemanticError::LimitExceeded {
            limit: GeometryAuthoringLimit::Work,
            actual: 1,
            maximum: 0,
        }
    ));
    assert_eq!(control.reserves, 0);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn empty_primitive_validation_polls_with_bounded_scratch() {
    const PRIMITIVE_COUNT: usize = 300;

    let primitives = (0..PRIMITIVE_COUNT)
        .map(|_| {
            GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::Points,
                Presence::Absent,
                Presence::Absent,
                AuthoringRange::new(0, 0),
            )
        })
        .collect::<Vec<_>>();
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(
        0,
        u32::try_from(PRIMITIVE_COUNT).expect("test primitive count"),
    ))];
    let mut control = TestControl {
        cancel_on_poll: Some(2),
        ..TestControl::default()
    };

    assert!(matches!(
        validate_geometry_batch_with_control(
            GeometryAuthoringBatch::new(&logical_meshes, &primitives, &[], &[]),
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
    assert_eq!(control.work, 256);
    assert_eq!(control.reserves, 1);
    assert!(control.peak_bytes() > 0);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn empty_attribute_semantic_pass_is_work_bounded_and_cancellable() {
    const ATTRIBUTE_COUNT: usize = 300;

    let empty_values = [];
    let attributes = (0..ATTRIBUTE_COUNT)
        .map(|attribute| {
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Texcoord(
                    u32::try_from(attribute).expect("test attribute semantic index"),
                ),
                AttributeDomain::Vertex,
                2,
                &empty_values,
                Presence::Absent,
            )
        })
        .collect::<Vec<_>>();
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(
            0,
            u32::try_from(ATTRIBUTE_COUNT).expect("test attribute count"),
        ),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let input = || GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &[]);

    let mut baseline = TestControl::default();
    let validated = validate_geometry_batch_with_control(
        input(),
        0,
        0,
        SceneValidationLimits::default(),
        &mut baseline,
    )
    .unwrap();
    let complete_work = validated.validation_work();
    assert_eq!(complete_work, baseline.work);
    drop(validated);
    assert_eq!(baseline.live.get(), 0);
    // Stop halfway through the final cardinality pass over the empty attributes.
    let allowed_work = complete_work - ATTRIBUTE_COUNT as u64 / 2;

    let mut bounded = TestControl::default();
    assert!(matches!(
        expect_semantic(validate_geometry_batch_with_control(
            input(),
            0,
            0,
            SceneValidationLimits::default().with_max_work(allowed_work),
            &mut bounded,
        )),
        GeometryAuthoringSemanticError::LimitExceeded {
            limit: GeometryAuthoringLimit::Work,
            actual,
            maximum,
        } if actual == allowed_work + 1 && maximum == allowed_work
    ));
    assert_eq!(bounded.work, allowed_work);
    assert_eq!(
        bounded.last_work(),
        Some(crate::SceneValidationWork::Traversal)
    );
    assert_eq!(bounded.reserves, 1);
    assert_eq!(bounded.live.get(), 0);

    let last_periodic_poll = 1 + complete_work / crate::SCENE_VALIDATION_POLL_INTERVAL;
    let mut cancelled = TestControl {
        cancel_on_poll: Some(last_periodic_poll),
        ..TestControl::default()
    };
    assert!(matches!(
        validate_geometry_batch_with_control(
            input(),
            0,
            0,
            SceneValidationLimits::default(),
            &mut cancelled,
        ),
        Err(GeometryAuthoringFailure::Control(
            TestControlError::Cancelled
        ))
    ));
    assert_eq!(cancelled.polls, last_periodic_poll);
    assert_eq!(
        cancelled.work,
        (last_periodic_poll - 1) * crate::SCENE_VALIDATION_POLL_INTERVAL
    );
    assert!(cancelled.work < complete_work);
    assert_eq!(cancelled.reserves, 1);
    assert_eq!(cancelled.live.get(), 0);
}

#[test]
fn periodic_cancellation_and_drop_order_cover_compiled_geometry_storage() {
    let positions = [0.0_f32; 900];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let input = GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &[]);
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
            &mut cancelled,
        ),
        Err(GeometryAuthoringFailure::Control(
            TestControlError::Cancelled
        ))
    ));
    assert_eq!(cancelled.reserves, 1);
    assert_eq!(cancelled.live.get(), 0);

    let mut final_poll_cancelled = TestControl {
        cancel_on_poll: Some(2),
        ..TestControl::default()
    };
    assert!(matches!(
        validate_geometry_batch_with_control(
            GeometryAuthoringBatch::new(&[], &[], &[], &[]),
            0,
            0,
            SceneValidationLimits::default(),
            &mut final_poll_cancelled,
        ),
        Err(GeometryAuthoringFailure::Control(
            TestControlError::Cancelled
        ))
    ));
    assert_eq!(final_poll_cancelled.polls, 2);
    assert_eq!(final_poll_cancelled.reserves, 1);
    assert_eq!(final_poll_cancelled.live.get(), 0);

    let stage = Rc::new(Cell::new(0));
    let mut control = TestControl {
        reservation_drop_stage: Some(Rc::clone(&stage)),
        ..TestControl::default()
    };
    let mut validated = validate_geometry_batch_with_control(
        input,
        0,
        0,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("geometry plan must validate without cancellation");
    validated.install_storage_drop_probe(Rc::clone(&stage));
    drop(validated);
    assert_eq!(stage.get(), 2);
    assert_eq!(control.live.get(), 0);
}
