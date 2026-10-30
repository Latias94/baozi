use super::*;
use crate::{AttributeData, AttributeSemanticRef, ValidatedScene};

#[test]
fn borrowed_readers_preserve_dense_sparse_and_presence_without_allocation() {
    let factory = factory();
    let account = account(&factory);
    let parts = all_attribute_payload_kinds_scene(&account);
    let scene = ValidatedScene::from_node_scene(parts.scene);

    let allocations = allocation_counter::measure(|| {
        for _ in 0..2 {
            let mesh = scene.meshes().next().unwrap();
            assert_eq!(mesh.primitives().len(), 3);
            let first = mesh.primitives().next().unwrap();
            let second = mesh.primitives().nth(1).unwrap();
            let third = mesh.primitives().next_back().unwrap();
            assert_eq!(third.indices(), None);
            assert_eq!(second.indices(), Some(&[0, 1, 2][..]));
            assert_eq!(second.polygon_face_sizes(), Some(&[3][..]));
            assert_eq!(
                first.material().unwrap().shading_model(),
                ShadingModel::Unlit
            );
            assert_eq!(first.attributes().len(), 2);
            assert_eq!(
                first.positions().unwrap().data(),
                AttributeData::DenseF32 {
                    lanes: 3,
                    values: &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                }
            );
            assert_eq!(
                first
                    .attribute(AttributeSemanticRef::SmoothingGroup, AttributeDomain::Face)
                    .unwrap()
                    .data(),
                AttributeData::SparseU32 {
                    lanes: 1,
                    logical_count: 1,
                    indices: &[0],
                    values: &[7],
                    default: &[0],
                }
            );
            assert_eq!(
                second
                    .attribute(AttributeSemanticRef::Color(0), AttributeDomain::Face)
                    .unwrap()
                    .data(),
                AttributeData::SparseF32 {
                    lanes: 3,
                    logical_count: 1,
                    indices: &[0],
                    values: &[0.25, 0.5, 0.75],
                    default: &[0.0, 0.0, 0.0],
                }
            );
            assert_eq!(
                third
                    .attribute(AttributeSemanticRef::SmoothingGroup, AttributeDomain::Face)
                    .unwrap()
                    .data(),
                AttributeData::DenseU32 {
                    lanes: 1,
                    values: &[3],
                }
            );
            assert_eq!(
                third
                    .attribute(
                        AttributeSemanticRef::Custom("ply:dense_signed"),
                        AttributeDomain::Vertex
                    )
                    .unwrap()
                    .data(),
                AttributeData::DenseI32 {
                    lanes: 1,
                    values: &[i32::MIN, -1, i32::MAX],
                }
            );
            let sparse = third
                .attribute(
                    AttributeSemanticRef::Custom("ply:sparse_signed"),
                    AttributeDomain::Vertex,
                )
                .unwrap();
            assert_eq!(sparse.len(), 3);
            assert_eq!(
                sparse.data(),
                AttributeData::SparseI32 {
                    lanes: 1,
                    logical_count: 3,
                    indices: &[0],
                    values: &[i32::MIN],
                    default: &[i32::MAX],
                }
            );
            let instance = scene.mesh_instances().next().unwrap();
            assert_eq!(instance.mesh().id(), mesh.id());
            let root = scene.roots().next().unwrap();
            assert_eq!(root.id(), instance.node().id());
            assert_eq!(root.children().len(), 0);
            assert_eq!(root.authored_transform(), AuthoredTransform::identity_trs());
            assert_eq!(scene.node(root.id()).unwrap().id(), root.id());
        }
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn attribute_lookup_matches_semantic_and_domain_without_allocation() {
    let factory = factory();
    let account = account(&factory);
    let positions = [
        [0.0_f32, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let vertex_normals = [[0.0_f32, 0.0, 1.0]; 4];
    let corner_normals = [[0.0_f32, 1.0, 0.0]; 6];
    let vertex_texcoords = [[0.0_f32, 0.0]; 4];
    let corner_texcoords = [[1.0_f32, 1.0]; 6];
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            positions.as_flattened(),
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Vertex,
            3,
            vertex_normals.as_flattened(),
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Corner,
            3,
            corner_normals.as_flattened(),
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Texcoord(0),
            AttributeDomain::Vertex,
            2,
            vertex_texcoords.as_flattened(),
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Texcoord(0),
            AttributeDomain::Corner,
            2,
            corner_texcoords.as_flattened(),
            Presence::Absent,
        ),
    ];
    let indices = [0, 1, 2, 0, 2, 3];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Present(&indices),
        Presence::Absent,
        AuthoringRange::new(0, 5),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let parts = build_scene_with_geometry(
        &account,
        &[],
        &[],
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
    );
    let scene = ValidatedScene::from_node_scene(parts.scene);

    let allocations = allocation_counter::measure(|| {
        let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
        assert_eq!(primitive.vertex_count(), 4);
        assert_eq!(primitive.corner_count(), 6);
        assert_eq!(primitive.attributes().len(), 5);
        assert_eq!(
            primitive.positions().unwrap().domain(),
            AttributeDomain::Vertex
        );
        assert_eq!(
            primitive.positions().unwrap().data(),
            AttributeData::DenseF32 {
                lanes: 3,
                values: positions.as_flattened(),
            }
        );
        for (domain, normal_values, texcoord_values) in [
            (
                AttributeDomain::Vertex,
                vertex_normals.as_flattened(),
                vertex_texcoords.as_flattened(),
            ),
            (
                AttributeDomain::Corner,
                corner_normals.as_flattened(),
                corner_texcoords.as_flattened(),
            ),
        ] {
            let normals = primitive.normals(domain).unwrap();
            assert_eq!(normals.domain(), domain);
            assert_eq!(normals.len(), normal_values.len() / 3);
            assert_eq!(
                normals.data(),
                AttributeData::DenseF32 {
                    lanes: 3,
                    values: normal_values,
                }
            );
            assert_eq!(
                primitive
                    .attribute(AttributeSemanticRef::Normal, domain)
                    .unwrap()
                    .data(),
                normals.data()
            );
            let texcoords = primitive.texcoords(0, domain).unwrap();
            assert_eq!(texcoords.domain(), domain);
            assert_eq!(texcoords.len(), texcoord_values.len() / 2);
            assert_eq!(
                texcoords.data(),
                AttributeData::DenseF32 {
                    lanes: 2,
                    values: texcoord_values,
                }
            );
            assert!(primitive.texcoords(1, domain).is_none());
        }
        assert!(
            primitive
                .attribute(AttributeSemanticRef::Position, AttributeDomain::Corner)
                .is_none()
        );
        assert!(
            primitive
                .attribute(AttributeSemanticRef::Normal, AttributeDomain::Face)
                .is_none()
        );
        assert!(primitive.normals(AttributeDomain::Face).is_none());
        assert!(primitive.texcoords(0, AttributeDomain::Face).is_none());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn index_absence_and_authored_emptiness_are_distinct_in_public_views() {
    let factory = factory();
    let account = account(&factory);
    for present in [false, true] {
        let parts = point_index_presence_scene(&account, present);
        let scene = ValidatedScene::from_node_scene(parts.scene);
        let primitive = scene.meshes().next().unwrap().primitives().next().unwrap();
        assert_eq!(primitive.indices(), present.then_some(&[][..]));
        assert_eq!(primitive.vertex_count(), 1);
        drop(scene);
        drop(parts.report);
    }
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn spatial_readers_follow_node_attachments_and_preserve_unknowns() {
    let factory = factory();
    let account = account(&factory);
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        matrix(3.0),
        Presence::Present(CameraLocalId::new(0)),
        Presence::Present(LightLocalId::new(0)),
    )];
    let projection = CameraProjection::perspective(1.0, None, 0.1, None).unwrap();
    let cameras = [CameraAuthoring::new(projection)];
    let lights = [LightAuthoring::new(
        LightKind::directional(),
        white_emission(2.0),
    )];
    let parts = build_scene_with_spatial_appearance_and_geometry(
        &account,
        &roots,
        &nodes,
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &cameras, &lights),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
    );
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let node = scene.roots().next().unwrap();
    assert_eq!(
        node.authored_transform().kind(),
        crate::AuthoredTransformKind::Matrix
    );
    assert_eq!(
        node.authored_transform().authored_matrix(),
        matrix(3.0).authored_matrix()
    );
    let camera = node.camera().unwrap();
    assert_eq!(
        camera.projection().kind(),
        crate::CameraProjectionKind::Perspective
    );
    assert_eq!(
        camera.projection().perspective_parameters(),
        Some((1.0, None, 0.1, None))
    );
    assert_eq!(scene.camera(camera.id()).unwrap().id(), camera.id());
    let light = node.light().unwrap();
    assert_eq!(
        light.kind().category(),
        crate::LightKindCategory::Directional
    );
    assert_eq!(light.intensity(), 2.0);
    assert_eq!(scene.light(light.id()).unwrap().id(), light.id());
    assert_eq!(scene.space().handedness, Handedness::Unknown);
    assert_eq!(scene.space().unit_scale_to_meters, None);
    assert_eq!(scene.space().tangent_basis, crate::TangentBasisRef::Unknown);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn spatial_readers_borrow_retained_values_after_authoring_storage_is_dropped() {
    let factory = factory();
    let account = account(&factory);
    let projections = [
        CameraProjection::perspective(0.75, None, 0.125, None).unwrap(),
        CameraProjection::orthographic(2.5, 1.25, 0.25, 1024.0).unwrap(),
    ];
    let light_kinds = [
        LightKind::directional(),
        LightKind::point(None).unwrap(),
        LightKind::spot(Some(32.0), 0.125, 0.75).unwrap(),
    ];
    let parts = {
        let roots = [
            NodeLocalId::new(0),
            NodeLocalId::new(1),
            NodeLocalId::new(2),
        ];
        let nodes = std::array::from_fn::<_, 3, _>(|index| {
            NodeAuthoringNode::new(
                Presence::Absent,
                trs(index as f32),
                Presence::Present(CameraLocalId::new((index % 2) as u32)),
                Presence::Present(LightLocalId::new(index as u32)),
            )
        });
        let space = namespaced_scene_space();
        let cameras = projections.map(CameraAuthoring::new);
        let lights = light_kinds.map(|kind| LightAuthoring::new(kind, white_emission(2.0)));
        build_scene_with_spatial_appearance_and_geometry(
            &account,
            &roots,
            &nodes,
            SpatialAuthoringBatch::new(&space, &cameras, &lights),
            AppearanceAuthoringBatch::empty(),
            GeometryAuthoringBatch::empty(),
        )
    };
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let allocations = allocation_counter::measure(|| {
        let space = scene.space();
        assert_eq!(space.handedness, Handedness::Left);
        assert_eq!(space.up_axis, Axis::NegativeY);
        assert_eq!(space.front_axis, Axis::PositiveZ);
        assert_eq!(
            space.unit_scale_to_meters.map(f64::to_bits),
            Some(0.01_f64.to_bits())
        );
        assert_eq!(space.front_face_winding, FrontFaceWinding::Clockwise);
        assert_eq!(space.uv_origin, UvOrigin::LowerLeft);
        let crate::TangentBasisRef::Namespaced { name, version } = space.tangent_basis else {
            panic!("the retained tangent-basis identity must remain borrowed");
        };
        assert_eq!((name, version.get()), ("example:tangent-basis", 2));
        let crate::TangentEncodingRef::Namespaced { name, version } =
            space.tangent_handedness_encoding
        else {
            panic!("the retained tangent encoding must remain borrowed");
        };
        assert_eq!((name, version.get()), ("example:tangent-sign", 3));

        assert_eq!(scene.cameras().len(), projections.len());
        for (camera, expected) in scene.cameras().zip(projections) {
            assert_eq!(camera.projection(), expected);
            assert_eq!(scene.camera(camera.id()).unwrap().projection(), expected);
        }
        assert_eq!(scene.lights().len(), light_kinds.len());
        for (light, expected) in scene.lights().zip(light_kinds) {
            assert_eq!(light.kind(), expected);
            assert_eq!(light.tint(), Vec3::new(1.0, 1.0, 1.0));
            assert_eq!(light.intensity().to_bits(), 2.0_f32.to_bits());
            assert_eq!(scene.light(light.id()).unwrap().kind(), expected);
        }
        for (index, node) in scene.nodes().enumerate() {
            assert_eq!(node.camera().unwrap().projection(), projections[index % 2]);
            assert_eq!(node.light().unwrap().kind(), light_kinds[index]);
        }
        let first = scene.nodes().next().unwrap().camera().unwrap();
        let repeated = scene.nodes().nth(2).unwrap().camera().unwrap();
        assert_eq!(first.id(), repeated.id());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}
