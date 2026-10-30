use baozi::{
    AttributeData, AttributeDomain, AttributeSemanticRef, Importer, SceneLookupError, TopologyKind,
};

#[cfg(feature = "format-obj")]
#[test]
fn obj_geometry_and_hierarchy_are_directly_readable() {
    let report = Importer::new()
        .unwrap()
        .read_bytes(
            "triangle.obj",
            b"o triangle\nv 0 0 0\nv 2 0 0\nv 0 3 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n",
        )
        .unwrap();
    let scene = report.scene();
    assert_eq!(scene.mesh_count(), 1);
    assert_eq!(scene.primitive_count(), 1);
    assert_eq!(scene.mesh_instance_count(), 1);
    assert_eq!(scene.node_count(), scene.nodes().len());
    assert_eq!(scene.root_count(), scene.roots().len());
    let mesh = scene.meshes().next().unwrap();
    assert_eq!(mesh.name(), Some("triangle"));
    assert_eq!(mesh.primitive_count(), 1);
    let primitive = mesh.primitives().next().unwrap();
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(primitive.vertex_count(), 3);
    assert_eq!(primitive.corner_count(), 3);
    assert_eq!(primitive.face_count(), 1);
    assert_eq!(primitive.indices(), Some(&[0, 1, 2][..]));
    assert_eq!(primitive.polygon_face_sizes(), None);
    let positions = primitive.positions().unwrap();
    assert_eq!(positions.semantic(), AttributeSemanticRef::Position);
    assert_eq!(positions.domain(), AttributeDomain::Vertex);
    assert_eq!(positions.len(), 3);
    let AttributeData::DenseF32 { lanes: 3, values } = positions.data() else {
        panic!("expected dense positions")
    };
    assert_eq!(values, &[0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0, 0.0]);
    let texcoords = primitive.texcoords(0, AttributeDomain::Corner).unwrap();
    assert_eq!(texcoords.domain(), AttributeDomain::Corner);
    let AttributeData::DenseF32 {
        lanes: 2,
        values: uv,
    } = texcoords.data()
    else {
        panic!("expected dense UVs")
    };
    assert_eq!(uv, &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
    assert!(primitive.normals(AttributeDomain::Corner).is_none());
    assert!(primitive.material().is_none());
    let instance = scene.mesh_instances().next().unwrap();
    assert_eq!(instance.mesh().id(), mesh.id());
    assert_eq!(
        scene.node(instance.node().id()).unwrap().id(),
        instance.node().id()
    );
    assert_eq!(
        instance.node().authored_transform().local_matrix(),
        baozi::Mat4::IDENTITY
    );
    assert_eq!(scene.mesh(mesh.id()).unwrap().id(), mesh.id());
    assert_eq!(
        scene.primitive(primitive.id()).unwrap().id(),
        primitive.id()
    );
    assert_eq!(
        scene.mesh_instance(instance.id()).unwrap().id(),
        instance.id()
    );
    assert_eq!(mesh.primitive(primitive.id()).unwrap().id(), primitive.id());

    // Slices borrow the Scene, independently of the wrappers and subsequent traversals.
    let mut first = scene.meshes();
    let second = scene.meshes();
    assert_eq!(
        first.next().unwrap().id(),
        second.clone().next().unwrap().id()
    );
    assert!(first.next().is_none());
    assert_eq!(values[3], 2.0);
    assert_eq!(scene.meshes().len(), 1);
    let root = scene.roots().next().unwrap();
    let _children = root.children();
}

#[cfg(feature = "format-obj")]
#[test]
fn scene_and_mesh_qualified_ids_reject_foreign_owners() {
    let importer = Importer::new().unwrap();
    let report = importer
        .read_bytes(
            "two.obj",
            b"o first\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\no second\nf 3 2 1\n",
        )
        .unwrap();
    let mut meshes = report.scene().meshes();
    let first = meshes.next().unwrap();
    let second = meshes.next().unwrap();
    let first_primitive = first.primitives().next().unwrap();
    assert!(matches!(
        second.primitive(first_primitive.id()),
        Err(SceneLookupError::OwnerMismatch { .. })
    ));
    let other = importer
        .read_bytes("other.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n")
        .unwrap();
    assert!(matches!(
        other.scene().mesh(first.id()),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
    assert!(matches!(
        other.scene().primitive(first_primitive.id()),
        Err(SceneLookupError::GenerationMismatch {
            kind: baozi::SceneEntityKind::Primitive,
            ..
        })
    ));
    let node = report.scene().nodes().next().unwrap();
    assert!(matches!(
        other.scene().node(node.id()),
        Err(SceneLookupError::GenerationMismatch { .. })
    ));
}

#[cfg(feature = "format-ply")]
#[test]
fn ply_custom_precision_is_visible_without_a_snapshot() {
    let report = Importer::new().unwrap().read_bytes("points.ply", b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nproperty double temperature\nend_header\n1 2 3 16777217\n").unwrap();
    let primitive = report
        .scene()
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    assert_eq!(primitive.topology(), TopologyKind::Points);
    let custom = primitive
        .attribute(
            AttributeSemanticRef::Custom("ply:temperature"),
            AttributeDomain::Vertex,
        )
        .unwrap();
    let AttributeData::DenseF64 { lanes: 1, values } = custom.data() else {
        panic!("expected retained F64")
    };
    assert_eq!(values[0].to_bits(), 16_777_217.0_f64.to_bits());
}

#[cfg(feature = "format-stl")]
#[test]
fn stl_multiword_name_and_positions_are_typed() {
    let report = Importer::new().unwrap().read_bytes("triangle.stl", b"solid part one\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid part one\n").unwrap();
    let mesh = report.scene().meshes().next().unwrap();
    assert_eq!(mesh.name(), Some("part one"));
    let primitive = mesh.primitives().next().unwrap();
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(primitive.vertex_count(), 3);
    let AttributeData::DenseF32 { lanes: 3, values } = primitive.positions().unwrap().data() else {
        panic!("expected F32 positions")
    };
    assert_eq!(values, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
}
