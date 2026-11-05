use super::*;
use crate::ExtensionSchemaId;
use crate::attribute::{
    TangentBasisConvention, TangentConventions, TangentConventionsValidationError,
    TangentHandednessEncoding,
};

#[test]
fn vertex_and_corner_tangents_keep_independent_conventions_on_one_primitive() {
    let positions = [0.0_f32; 9];
    let tangents = [1.0_f32; 12];
    let generated = TangentConventions {
        basis: TangentBasisConvention::TriangleUvDerivative,
        encoding: TangentHandednessEncoding::SignInW,
    };
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Tangent,
            AttributeDomain::Vertex,
            4,
            &tangents,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Tangent,
            AttributeDomain::Corner,
            4,
            &tangents,
            Presence::Absent,
        )
        .with_tangent_conventions(generated.clone()),
    ];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 3),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let mut control = TestControl::default();
    let batch = prepare_geometry_batch_with_control(
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
        0,
        0,
        SceneValidationLimits::default(),
        &mut control,
    )
    .unwrap();
    assert_eq!(batch.validation_work(), 21);
    assert_eq!(control.work, 21);
    assert_eq!(batch.attribute(0).unwrap().tangent_conventions(), None);
    assert_eq!(
        batch.attribute(1).unwrap().tangent_conventions(),
        Some(&TangentConventions::UNKNOWN)
    );
    assert_eq!(
        batch.attribute(2).unwrap().tangent_conventions(),
        Some(&generated)
    );
    assert_eq!(batch.tangent_schema_name_byte_count(), 0);
}

#[test]
fn tangent_schema_names_are_admitted_as_geometry_text_at_exact_limits() {
    let positions = [0.0_f32; 9];
    let tangents = [1.0_f32; 12];
    let basis = "test:triangle-basis";
    let encoding = "test:bitangent-sign";
    let schema_bytes = basis.len() + encoding.len();
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Tangent,
            AttributeDomain::Corner,
            4,
            &tangents,
            Presence::Absent,
        )
        .with_tangent_conventions(TangentConventions {
            basis: TangentBasisConvention::Namespaced {
                schema_id: ExtensionSchemaId::new(basis, 2).unwrap(),
            },
            encoding: TangentHandednessEncoding::Namespaced {
                schema_id: ExtensionSchemaId::new(encoding, 3).unwrap(),
            },
        }),
    ];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 2),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let input = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]);
    let canonical_bytes = expected_geometry_bytes([1, 1, 2, 0], [21, 0, 0, 0], schema_bytes);
    let cells = 21 + schema_bytes as u64;
    let work = 17 + schema_bytes as u64;
    let limits = SceneValidationLimits::default()
        .with_max_canonical_bytes(canonical_bytes)
        .with_max_payload_cells(cells)
        .with_max_work(work);
    let mut control = TestControl::default();
    let batch = prepare_geometry_batch_with_control(input, 0, 0, limits, &mut control).unwrap();
    assert_eq!(batch.tangent_schema_name_byte_count(), schema_bytes as u32);
    assert_eq!(batch.attribute_semantic_name_byte_count(), 0);
    assert_eq!(batch.semantic_cell_count(), cells);
    assert_eq!(batch.canonical_bytes(), canonical_bytes);
    assert_eq!(batch.validation_work(), work);
    assert_eq!(control.work, work);
    drop(batch);
    for (limits, dimension) in [
        (
            limits.with_max_payload_cells(cells - 1),
            GeometryAuthoringLimit::SemanticCells,
        ),
        (
            limits.with_max_canonical_bytes(canonical_bytes - 1),
            GeometryAuthoringLimit::CanonicalBytes,
        ),
        (limits.with_max_work(work - 1), GeometryAuthoringLimit::Work),
    ] {
        assert!(matches!(
            expect_semantic(prepare_geometry_batch_with_control(input, 0, 0, limits, &mut control)),
            GeometryAuthoringSemanticError::LimitExceeded { limit, actual, maximum }
                if limit == dimension && actual == maximum + 1
        ));
        assert_eq!(control.live.get(), 0);
    }
}

#[test]
fn admission_rejects_incompatible_mikk_encoding_and_non_tangent_declarations() {
    let positions = [0.0_f32; 9];
    let tangents = [1.0_f32; 12];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 2),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    for encoding in [
        TangentHandednessEncoding::Unknown,
        TangentHandednessEncoding::Namespaced {
            schema_id: ExtensionSchemaId::new("test:sign", 1).unwrap(),
        },
    ] {
        let attributes = [
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &positions,
                Presence::Absent,
            ),
            GeometryAttributeAuthoring::f32(
                AttributeSemantic::Tangent,
                AttributeDomain::Vertex,
                4,
                &tangents,
                Presence::Absent,
            )
            .with_tangent_conventions(TangentConventions {
                basis: TangentBasisConvention::MikkTspace,
                encoding,
            }),
        ];
        assert!(matches!(
            reject_geometry(
                GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
                0
            ),
            GeometryAuthoringSemanticError::TangentConventions {
                attribute: 1,
                source: TangentConventionsValidationError::MikkTspaceRequiresSignInW,
            }
        ));
    }
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )
    .with_tangent_conventions(TangentConventions::UNKNOWN)];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 1),
    )];
    assert!(matches!(
        reject_geometry(
            GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
            0
        ),
        GeometryAuthoringSemanticError::TangentConventions {
            attribute: 0,
            source: TangentConventionsValidationError::UnexpectedConventions,
        }
    ));
}
