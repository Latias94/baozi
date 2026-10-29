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
                    .attribute(AttributeSemanticRef::SmoothingGroup)
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
                    .attribute(AttributeSemanticRef::Color(0))
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
                    .attribute(AttributeSemanticRef::SmoothingGroup)
                    .unwrap()
                    .data(),
                AttributeData::DenseU32 {
                    lanes: 1,
                    values: &[3],
                }
            );
            assert_eq!(
                third
                    .attribute(AttributeSemanticRef::Custom("ply:dense_signed"))
                    .unwrap()
                    .data(),
                AttributeData::DenseI32 {
                    lanes: 1,
                    values: &[i32::MIN, -1, i32::MAX],
                }
            );
            let sparse = third
                .attribute(AttributeSemanticRef::Custom("ply:sparse_signed"))
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
