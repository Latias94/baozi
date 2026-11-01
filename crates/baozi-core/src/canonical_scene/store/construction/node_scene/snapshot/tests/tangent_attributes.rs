use super::*;
use crate::{
    AttributeSemanticRef, TangentBasisRef, TangentConventionsRef, TangentEncodingRef,
    ValidatedScene,
};

fn build_tangent_scene(
    account: &PhysicalStorageAccount,
    vertex: TangentConventions,
    corner: TangentConventions,
) -> NodeSceneSuccessParts {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )];
    let positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let tangents = [
        1.0_f32, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0,
    ];
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
        .with_tangent_conventions(vertex),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Tangent,
            AttributeDomain::Corner,
            4,
            &tangents,
            Presence::Absent,
        )
        .with_tangent_conventions(corner),
    ];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 3),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    build_scene_with_geometry(
        account,
        &roots,
        &nodes,
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
    )
}

fn generated_conventions() -> TangentConventions {
    TangentConventions {
        basis: TangentBasisConvention::TriangleUvDerivative,
        encoding: TangentHandednessEncoding::SignInW,
    }
}

fn external_conventions(version: u32) -> TangentConventions {
    TangentConventions {
        basis: TangentBasisConvention::Namespaced {
            schema_id: ExtensionSchemaId::new("test:external-basis", version).unwrap(),
        },
        encoding: TangentHandednessEncoding::Namespaced {
            schema_id: ExtensionSchemaId::new("test:external-sign", 3).unwrap(),
        },
    }
}

#[test]
fn mixed_tangent_domains_publish_explicit_unknown_and_generated_declarations_independently() {
    let factory = factory();
    let account = account(&factory);
    let parts = build_tangent_scene(
        &account,
        TangentConventions::UNKNOWN,
        generated_conventions(),
    );
    let full = capture_node_scene(&parts.scene, snapshot_limits(16_384, 64), None).unwrap();
    let bounded = capture_node_scene(&parts.scene, snapshot_limits(0, 0), None).unwrap();
    assert_eq!(full.digest, bounded.digest);
    let vertex = full
        .preview
        .text
        .lines()
        .find(|line| line.contains("attribute[1]"))
        .unwrap();
    let corner = full
        .preview
        .text
        .lines()
        .find(|line| line.contains("attribute[2]"))
        .unwrap();
    assert!(vertex.contains("domain=vertex"));
    assert!(vertex.contains("tangent_basis=unknown tangent_handedness_encoding=unknown"));
    assert!(corner.contains("domain=corner"));
    assert!(
        corner
            .contains("tangent_basis=triangle_uv_derivative tangent_handedness_encoding=sign_in_w")
    );
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let allocations = allocation_counter::measure(|| {
        let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
        assert_eq!(primitive.positions().unwrap().tangent_conventions(), None);
        assert_eq!(
            primitive
                .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
                .unwrap()
                .tangent_conventions(),
            Some(TangentConventionsRef {
                basis: TangentBasisRef::Unknown,
                encoding: TangentEncodingRef::Unknown
            }),
        );
        assert_eq!(
            primitive
                .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Corner)
                .unwrap()
                .tangent_conventions(),
            Some(TangentConventionsRef {
                basis: TangentBasisRef::TriangleUvDerivative,
                encoding: TangentEncodingRef::SignInW
            }),
        );
    });
    assert_eq!(allocations.count_total, 0);
}

#[test]
fn tangent_schema_identity_is_borrowed_from_geometry_and_committed_per_attribute() {
    let factory = factory();
    let account = account(&factory);
    let parts = build_tangent_scene(&account, external_conventions(2), generated_conventions());
    let full = capture_node_scene(&parts.scene, snapshot_limits(16_384, 64), None).unwrap();
    assert_eq!(
        full.totals.tangent_schema_name_bytes,
        ("test:external-basis".len() + "test:external-sign".len()) as u64
    );
    assert!(
        full.preview.text.contains(
            "tangent_basis_schema_name=test:external-basis tangent_basis_schema_version=2"
        )
    );
    assert!(full.preview.text.contains(
        "tangent_encoding_schema_name=test:external-sign tangent_encoding_schema_version=3"
    ));
    let repeated = build_tangent_scene(&account, external_conventions(2), generated_conventions());
    assert_eq!(
        full.digest,
        capture_node_scene(&repeated.scene, snapshot_limits(0, 0), None)
            .unwrap()
            .digest
    );
    let version_changed =
        build_tangent_scene(&account, external_conventions(4), generated_conventions());
    assert_ne!(
        full.digest,
        capture_node_scene(&version_changed.scene, snapshot_limits(0, 0), None)
            .unwrap()
            .digest
    );
    let owner_changed =
        build_tangent_scene(&account, generated_conventions(), external_conventions(2));
    assert_ne!(
        full.digest,
        capture_node_scene(&owner_changed.scene, snapshot_limits(0, 0), None)
            .unwrap()
            .digest
    );
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let name_pool = scene.as_node_scene().read_geometry().names.as_ptr_range();
    let allocations = allocation_counter::measure(|| {
        let declarations = scene
            .meshes()
            .next()
            .unwrap()
            .primitives()
            .next()
            .unwrap()
            .attribute(AttributeSemanticRef::Tangent, AttributeDomain::Vertex)
            .unwrap()
            .tangent_conventions()
            .unwrap();
        let TangentBasisRef::Namespaced { name, version } = declarations.basis else {
            panic!("external basis")
        };
        assert_eq!(name, "test:external-basis");
        assert_eq!(version.get(), 2);
        assert!(name_pool.contains(&name.as_ptr()));
        let TangentEncodingRef::Namespaced { name, version } = declarations.encoding else {
            panic!("external encoding")
        };
        assert_eq!(name, "test:external-sign");
        assert_eq!(version.get(), 3);
        assert!(name_pool.contains(&name.as_ptr()));
    });
    assert_eq!(allocations.count_total, 0);
}
