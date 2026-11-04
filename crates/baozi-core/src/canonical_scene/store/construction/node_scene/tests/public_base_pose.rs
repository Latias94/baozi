use std::mem::size_of;
use std::sync::atomic::AtomicU64;

use baozi_operation::{CancellationToken, PhysicalStorageAccount};

use super::gltf_capability::build_gltf_capability_scene_for_probe;
use super::{
    SINGLE_TRS_NODES, SINGLE_TRS_ROOTS, parent, profile_fixture, successful_parts, test_account,
    test_factory, test_limits,
};
use crate::canonical_scene::store::construction::node_scene::NodeSceneTestTransaction;
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationTargetAuthoring, AppearanceAuthoringBatch, AuthoringRange, DeformationAuthoringBatch,
    GeometryAttributeAuthoring, GeometryAuthoringBatch, GeometryLogicalMeshAuthoring,
    GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    LogicalMeshLocalId, LogicalMorphAuthoring, MeshInstanceDeformationAuthoring,
    MorphDeltaAuthoring, MorphDeltaPayloadAuthoring, MorphTargetDefinitionAuthoring,
    MorphTargetIdentity, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, Presence,
    PrimitiveDeformationAuthoring, PrimitiveMorphTargetAuthoring, SceneAuthoringBatch,
    SpatialAuthoringBatch,
};
use crate::{
    Aabb, AnimationExtrapolation, AnimationInterpolation, AnimationTimeDomain, AttributeDomain,
    AttributeSemantic, AuthoredTransform, Mat4, MorphDeltaSemantic, NodeId, NumericConversionError,
    Quat, SceneEntityKind, SceneLookupError, SceneSnapshotLimits, SceneSpace,
    SceneValidationLimits, SpatialQueryError, SpatialQueryLimit, SpatialQueryLimits,
    ValidatedScene, Vec3,
};

const UNLIMITED: SpatialQueryLimits = SpatialQueryLimits::new(u64::MAX, u64::MAX);

fn publish_scene(
    account: &PhysicalStorageAccount,
    nodes: NodeAuthoringBatch<'_>,
    geometry: GeometryAuthoringBatch<'_>,
    deformation: DeformationAuthoringBatch<'_>,
    animation: AnimationAuthoringBatch<'_>,
) -> ValidatedScene {
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let appearance = AppearanceAuthoringBatch::empty();
    let profile = profile_fixture(spatial, nodes, appearance, geometry, deformation, animation);
    let mut construction_root_1 =
        crate::canonical_scene::store::construction::SceneCanonicalInvocationHost::new();
    let parts = successful_parts(
        NodeSceneTestTransaction::with_account(
            &mut construction_root_1,
            account,
            test_limits(),
            SceneAuthoringBatch::new(
                spatial,
                nodes,
                appearance,
                geometry,
                deformation,
                animation,
                profile.batch(),
            ),
            SceneValidationLimits::default(),
        )
        .expect("base-pose fixture admission")
        .finish_draft()
        .expect("base-pose fixture activation")
        .publish_for_test()
        .expect("base-pose fixture publication"),
    );
    let scene = ValidatedScene::from_node_scene(parts.scene);
    drop(parts.report);
    scene
}

fn publish_static_scene(
    account: &PhysicalStorageAccount,
    nodes: NodeAuthoringBatch<'_>,
    geometry: GeometryAuthoringBatch<'_>,
) -> ValidatedScene {
    let meshes = vec![
        LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[]);
        geometry.logical_meshes().len()
    ];
    let primitives = vec![
        PrimitiveDeformationAuthoring::new(Presence::Absent, &[]);
        geometry.primitives().len()
    ];
    let instances = vec![
        MeshInstanceDeformationAuthoring::new(Presence::Absent, Presence::Absent);
        geometry.mesh_instances().len()
    ];
    publish_scene(
        account,
        nodes,
        geometry,
        DeformationAuthoringBatch::new(&[], &[], &meshes, &primitives, &instances),
        AnimationAuthoringBatch::empty(),
    )
}

fn snapshot_digest(scene: &ValidatedScene) -> [u8; 32] {
    // The digest traverses every payload even with no preview, using the same work bound
    // as the complete glTF capability fixture's existing snapshot checks.
    scene
        .capture_snapshot(SceneSnapshotLimits::try_new(0, 0, 1_000_000).unwrap())
        .unwrap()
        .digest()
}

// Canonical attribute order puts Position first. Normal and Texcoord remain unvisited by the
// bounds query, so the work budget distinguishes lookup from scanning the complete range.
fn shared_triangle_scene(account: &PhysicalStorageAccount) -> ValidatedScene {
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Normal,
            AttributeDomain::Vertex,
            3,
            &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Texcoord(0),
            AttributeDomain::Vertex,
            2,
            &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            Presence::Absent,
        ),
    ];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 3),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let instances = [
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(0), LogicalMeshLocalId::new(0)),
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(0), LogicalMeshLocalId::new(0)),
    ];
    publish_static_scene(
        account,
        NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &SINGLE_TRS_NODES),
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
    )
}

#[test]
fn precision_contract_public_world_transforms_retain_f64_residual() {
    let factory = test_factory();
    let account = test_account(&factory, 1024);
    let translations = [1.0, 2.0_f32.powi(-30)];
    let nodes: [_; 2] = std::array::from_fn(|index| {
        NodeAuthoringNode::new(
            if index == 0 {
                Presence::Absent
            } else {
                parent(0, 0)
            },
            AuthoredTransform::try_trs(
                Vec3::new(translations[index], 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::new(1.0, 1.0, 1.0),
            )
            .unwrap(),
            Presence::Absent,
            Presence::Absent,
        )
    });
    let scene = publish_static_scene(
        &account,
        NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &nodes),
        GeometryAuthoringBatch::empty(),
    );
    let before = snapshot_digest(&scene);
    let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
    let child = scene.roots().next().unwrap().children().next().unwrap();

    assert_eq!(
        table.node_transform(child.id()).unwrap()[3][0],
        1.0 + 2.0_f64.powi(-30)
    );
    assert_eq!(snapshot_digest(&scene), before);
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn precision_contract_public_bounds_retain_f64_derived_values() {
    let factory = test_factory();
    let account = test_account(&factory, 1024);
    for (translation, scale, position, expected) in [
        (1.0, 1.0, 2.0_f32.powi(-30), 1.0 + 2.0_f64.powi(-30)),
        (0.0, f32::MAX, 2.0, f64::from(f32::MAX) * 2.0),
        (
            0.0,
            0.5,
            f32::from_bits(1),
            f64::from(f32::from_bits(1)) * 0.5,
        ),
    ] {
        let nodes = [NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::try_trs(
                Vec3::new(translation, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::new(scale, 1.0, 1.0),
            )
            .unwrap(),
            Presence::Absent,
            Presence::Absent,
        )];
        let positions = [position, 0.0, 0.0];
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
            crate::MaterialAssignment::Unspecified,
            AuthoringRange::new(0, 1),
        )];
        let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
        let instances = [GeometryMeshInstanceAuthoring::new(
            NodeLocalId::new(0),
            LogicalMeshLocalId::new(0),
        )];
        let scene = publish_static_scene(
            &account,
            NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &nodes),
            GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
        );
        let before = snapshot_digest(&scene);
        let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
        let instance = scene.mesh_instances().next().unwrap();
        let bounds = table
            .instance_bounds(instance, UNLIMITED, None)
            .unwrap()
            .unwrap();

        assert_eq!(bounds.min[0], expected);
        assert_eq!(bounds.max[0], expected);
        assert_eq!(
            table.scene_bounds(&scene, UNLIMITED, None).unwrap(),
            Some(bounds)
        );
        assert_eq!(snapshot_digest(&scene), before);
        drop(scene);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }
}

#[test]
fn world_transforms_follow_authored_hierarchy_and_bound_each_shared_placement() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let translation = |x, y, z| {
        AuthoredTransform::try_trs(Vec3::new(x, y, z), Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0))
            .unwrap()
    };
    // Source IDs deliberately place the parent after its child. The two root orders also
    // differ from source order; canonical publication remaps instance owners with the nodes.
    let roots = [NodeLocalId::new(2), NodeLocalId::new(1)];
    let nodes = [
        NodeAuthoringNode::new(
            parent(2, 0),
            translation(1.0, 2.0, -4.0),
            Presence::Absent,
            Presence::Absent,
        )
        .with_name("middle"),
        NodeAuthoringNode::new(
            Presence::Absent,
            translation(-10.0, 0.0, 0.0),
            Presence::Absent,
            Presence::Absent,
        )
        .with_name("other-root"),
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::try_matrix(Mat4 {
                cols: [
                    [0.0, -2.0, 0.0, 0.0],
                    [-3.0, 0.0, 0.0, 0.0],
                    [0.0, 0.0, 0.5, 0.0],
                    [10.0, -2.0, 3.0, 1.0],
                ],
            })
            .unwrap(),
            Presence::Absent,
            Presence::Absent,
        )
        .with_name("root"),
        NodeAuthoringNode::new(
            parent(0, 0),
            translation(-1.0, 0.0, 2.0),
            Presence::Absent,
            Presence::Absent,
        )
        .with_name("leaf"),
    ];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &[0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0, 4.0],
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 1),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let instances = [
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(3), LogicalMeshLocalId::new(0)),
        GeometryMeshInstanceAuthoring::new(NodeLocalId::new(1), LogicalMeshLocalId::new(0)),
    ];
    let scene = publish_static_scene(
        &account,
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
    );
    let before = snapshot_digest(&scene);
    let retained = account.snapshot();
    for maximum in [0, 7] {
        let allocations = allocation_counter::measure(|| {
            assert_eq!(
                scene
                    .base_pose_world_transforms(
                        SpatialQueryLimits::new(maximum, 4 * size_of::<[[f64; 4]; 4]>() as u64),
                        None,
                    )
                    .unwrap_err(),
                SpatialQueryError::LimitExceeded {
                    limit: SpatialQueryLimit::Work,
                    actual: 8,
                    maximum,
                }
            );
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
    }
    let table = scene
        .base_pose_world_transforms(
            SpatialQueryLimits::new(8, 4 * size_of::<[[f64; 4]; 4]>() as u64),
            None,
        )
        .unwrap();
    assert_eq!(table.generation(), scene.generation());
    assert_eq!(table.node_count(), 4);
    for (name, translation) in [
        ("root", [10.0, -2.0, 3.0, 1.0]),
        ("middle", [4.0, -4.0, 1.0, 1.0]),
        ("leaf", [4.0, -2.0, 2.0, 1.0]),
    ] {
        let node = scene
            .nodes()
            .find(|node| node.name() == Some(name))
            .unwrap();
        assert_eq!(
            table.node_transform(node.id()).unwrap(),
            [
                [0.0, -2.0, 0.0, 0.0],
                [-3.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.5, 0.0],
                translation,
            ]
        );
    }
    let leaf = scene.mesh_instances().next().unwrap();
    let other = scene.mesh_instances().nth(1).unwrap();
    assert_eq!(leaf.node().name(), Some("leaf"));
    assert_eq!(other.node().name(), Some("other-root"));
    assert_eq!(leaf.mesh().id(), other.mesh().id());
    assert_eq!(
        table.instance_bounds(leaf, UNLIMITED, None).unwrap(),
        Some(Aabb {
            min: [1.0, -6.0, 2.0],
            max: [4.0, -2.0, 4.0],
        })
    );
    assert_eq!(
        table.instance_bounds(other, UNLIMITED, None).unwrap(),
        Some(Aabb {
            min: [-10.0, 0.0, 0.0],
            max: [-8.0, 1.0, 4.0],
        })
    );
    assert_eq!(
        table.scene_bounds(&scene, UNLIMITED, None).unwrap(),
        Some(Aabb {
            min: [-10.0, -6.0, 0.0],
            max: [4.0, 1.0, 4.0],
        })
    );
    assert_eq!(snapshot_digest(&scene), before);
    assert_eq!(account.snapshot(), retained);
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn authored_trs_supports_rotation_negative_and_zero_scale_without_sampling_animation() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let transform = AuthoredTransform::try_trs(
        Vec3::new(5.0, 6.0, 7.0),
        Quat::from_xyzw(0.5, 0.5, 0.5, 0.5),
        Vec3::new(-2.0, 3.0, 0.0),
    )
    .unwrap();
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        transform,
        Presence::Absent,
        Presence::Absent,
    )];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &[1.0, 2.0, 9.0, -1.0, -2.0, -9.0],
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 1),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    let morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
    let overrides = [MeshInstanceDeformationAuthoring::new(
        Presence::Absent,
        Presence::Absent,
    )];
    let channels = [AnimationChannelAuthoring::new(
        0,
        AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
        AnimationInterpolation::Linear,
        AnimationExtrapolation::Constant,
        AnimationExtrapolation::Constant,
        &[0.0, 1.0],
        &[100.0, 200.0, 300.0, 400.0, 500.0, 600.0],
        Presence::Absent,
        Presence::Absent,
    )];
    let clips = [AnimationClipAuthoring::new(
        0,
        1.0,
        AnimationTimeDomain::Seconds,
        AuthoringRange::new(0, 1),
    )];
    let scene = publish_scene(
        &account,
        NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &nodes),
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
        DeformationAuthoringBatch::new(&[], &[], &morphs, &deformations, &overrides),
        AnimationAuthoringBatch::new(&clips, &channels),
    );
    let before = snapshot_digest(&scene);
    let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
    let node = scene.nodes().next().unwrap();
    assert_eq!(scene.animation_clip_count(), 1);
    assert_eq!(node.authored_transform(), transform);
    assert_eq!(
        table.node_transform(node.id()).unwrap(),
        [
            [0.0, -2.0, 0.0, 0.0],
            [0.0, 0.0, 3.0, 0.0],
            [0.0, 0.0, 0.0, 0.0],
            [5.0, 6.0, 7.0, 1.0],
        ]
    );
    assert_eq!(
        table.scene_bounds(&scene, UNLIMITED, None).unwrap(),
        Some(Aabb {
            min: [5.0, 4.0, 1.0],
            max: [5.0, 8.0, 13.0],
        })
    );
    assert_eq!(snapshot_digest(&scene), before);
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn shear_bounds_transform_actual_vertices_without_introducing_local_box_corners() {
    let factory = test_factory();
    let account = test_account(&factory, 1024);
    let shear = Mat4 {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [1.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::try_matrix(shear).unwrap(),
        Presence::Absent,
        Presence::Absent,
    )];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 1),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    let scene = publish_static_scene(
        &account,
        NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &nodes),
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
    );
    let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
    let node = scene.nodes().next().unwrap();
    assert_eq!(
        table.node_transform(node.id()).unwrap(),
        shear.cols.map(|column| column.map(f64::from))
    );
    assert_eq!(node.authored_transform().authored_matrix(), Some(shear));
    assert_eq!(
        table.scene_bounds(&scene, UNLIMITED, None).unwrap(),
        Some(Aabb {
            min: [0.0; 3],
            max: [1.0, 1.0, 0.0],
        })
    );
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn sparse_positions_match_dense_vertices_and_ignore_defaults_without_omissions() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::try_trs(
            Vec3::new(1.0, 2.0, -1.0),
            Quat::IDENTITY,
            Vec3::new(2.0, -1.0, 0.5),
        )
        .unwrap(),
        Presence::Absent,
        Presence::Absent,
    )];
    let sparse_values = [-2.0, 4.0, 0.0, 3.0, 2.0, -5.0];
    let attributes = [
        GeometryAttributeAuthoring::sparse_f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            4,
            &[1, 3],
            &sparse_values,
            &[10.0, -1.0, 2.0],
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &[
                10.0, -1.0, 2.0, -2.0, 4.0, 0.0, 10.0, -1.0, 2.0, 3.0, 2.0, -5.0,
            ],
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::sparse_f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            2,
            &[0, 1],
            &sparse_values,
            &[f32::MAX; 3],
        ),
        GeometryAttributeAuthoring::sparse_f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            2,
            &[],
            &[],
            &[10.0, -1.0, 2.0],
        ),
        GeometryAttributeAuthoring::sparse_f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            0,
            &[],
            &[],
            &[f32::MAX; 3],
        ),
    ];
    let primitives: [_; 5] = std::array::from_fn(|index| {
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Absent,
            crate::MaterialAssignment::Unspecified,
            AuthoringRange::new(index as u32, 1),
        )
    });
    let meshes: [_; 5] = std::array::from_fn(|index| {
        GeometryLogicalMeshAuthoring::new(AuthoringRange::new(index as u32, 1))
    });
    let instances: [_; 5] = std::array::from_fn(|index| {
        GeometryMeshInstanceAuthoring::new(
            NodeLocalId::new(0),
            LogicalMeshLocalId::new(index as u32),
        )
    });
    let scene = publish_static_scene(
        &account,
        NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &nodes),
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
    );
    let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
    let dense_and_sparse = Some(Aabb {
        min: [-3.0, -2.0, -3.5],
        max: [21.0, 3.0, 0.0],
    });
    let expected = [
        dense_and_sparse,
        dense_and_sparse,
        Some(Aabb {
            min: [-3.0, -2.0, -3.5],
            max: [7.0, 0.0, -1.0],
        }),
        Some(Aabb {
            min: [21.0, 3.0, 0.0],
            max: [21.0, 3.0, 0.0],
        }),
        None,
    ];
    for (instance, expected) in scene.mesh_instances().zip(expected) {
        assert_eq!(
            table.instance_bounds(instance, UNLIMITED, None).unwrap(),
            expected
        );
    }
    assert_eq!(
        table.scene_bounds(&scene, UNLIMITED, None).unwrap(),
        dense_and_sparse
    );
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn precision_contract_public_f64_dense_and_sparse_bounds_preserve_logical_values() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let first = [16_777_216.0, -1.0e-100, f64::from_bits(1)];
    let second = [
        f64::from_bits(first[0].to_bits() + 1),
        1.0e-100,
        -f64::from_bits(1),
    ];
    let values = [
        first[0], first[1], first[2], second[0], second[1], second[2],
    ];
    let dense = [
        first[0], first[1], first[2], second[0], second[1], second[2], first[0], first[1], first[2],
    ];
    let attributes = [
        GeometryAttributeAuthoring::f64(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &dense,
        ),
        GeometryAttributeAuthoring::sparse_f64(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            3,
            &[1],
            &second,
            &first,
        ),
        GeometryAttributeAuthoring::sparse_f64(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            2,
            &[0, 1],
            &values,
            &[f64::MAX; 3],
        ),
        GeometryAttributeAuthoring::sparse_f64(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            2,
            &[],
            &[],
            &first,
        ),
        GeometryAttributeAuthoring::sparse_f64(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            0,
            &[],
            &[],
            &[f64::MAX; 3],
        ),
    ];
    let primitives: [_; 5] = std::array::from_fn(|index| {
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Absent,
            crate::MaterialAssignment::Unspecified,
            AuthoringRange::new(index as u32, 1),
        )
    });
    let meshes: [_; 5] = std::array::from_fn(|index| {
        GeometryLogicalMeshAuthoring::new(AuthoringRange::new(index as u32, 1))
    });
    let instances: [_; 5] = std::array::from_fn(|index| {
        GeometryMeshInstanceAuthoring::new(
            NodeLocalId::new(0),
            LogicalMeshLocalId::new(index as u32),
        )
    });
    let scene = publish_static_scene(
        &account,
        NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &SINGLE_TRS_NODES),
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
    );
    let before = snapshot_digest(&scene);
    let retained = account.snapshot();
    let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
    let extent = Some(Aabb {
        min: [first[0], first[1], second[2]],
        max: [second[0], second[1], first[2]],
    });
    let expected = [
        extent,
        extent,
        extent,
        Some(Aabb {
            min: first,
            max: first,
        }),
        None,
    ];
    let allocations = allocation_counter::measure(|| {
        for ((instance, expected), logical_count) in
            scene.mesh_instances().zip(expected).zip([3, 3, 2, 2, 0])
        {
            let work = 3 + logical_count;
            assert_eq!(
                table.instance_bounds(instance, SpatialQueryLimits::new(work, 0), None),
                Ok(expected)
            );
            assert_eq!(
                table.instance_bounds(instance, SpatialQueryLimits::new(work - 1, 0), None),
                Err(SpatialQueryError::LimitExceeded {
                    limit: SpatialQueryLimit::Work,
                    actual: work,
                    maximum: work - 1,
                })
            );
        }
        assert_eq!(
            table.scene_bounds(&scene, SpatialQueryLimits::new(25, 0), None),
            Ok(extent)
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(snapshot_digest(&scene), before);
    assert_eq!(account.snapshot(), retained);
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn precision_contract_public_f64_positions_use_composed_f64_world_matrices() {
    let factory = test_factory();
    let account = test_account(&factory, 1024);
    let nodes: [_; 2] = std::array::from_fn(|index| {
        NodeAuthoringNode::new(
            if index == 0 {
                Presence::Absent
            } else {
                parent(0, 0)
            },
            AuthoredTransform::try_trs(
                Vec3::new(if index == 0 { 1.0 } else { 2.0_f32.powi(-30) }, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::new(1.0, 1.0, 1.0),
            )
            .unwrap(),
            Presence::Absent,
            Presence::Absent,
        )
    });
    let positions = [
        0.0,
        -1.0e-100,
        f64::from_bits(1),
        2.0_f64.powi(-31),
        1.0e-100,
        -f64::from_bits(1),
    ];
    let attributes = [GeometryAttributeAuthoring::f64(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        crate::MaterialAssignment::Unspecified,
        AuthoringRange::new(0, 1),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(1),
        LogicalMeshLocalId::new(0),
    )];
    let scene = publish_static_scene(
        &account,
        NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &nodes),
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
    );
    let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
    assert_eq!(
        table.scene_bounds(&scene, UNLIMITED, None).unwrap(),
        Some(Aabb {
            min: [1.0 + 2.0_f64.powi(-30), -1.0e-100, -f64::from_bits(1)],
            max: [
                1.0 + 2.0_f64.powi(-30) + 2.0_f64.powi(-31),
                1.0e-100,
                f64::from_bits(1),
            ],
        })
    );
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn bounds_distinguish_empty_points_indices_and_uninstanced_meshes() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let attributes = [
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &[
                0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 100.0, -100.0, 50.0,
            ],
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &[7.0, 8.0, 9.0],
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &[0.0, 0.0, 0.0],
            Presence::Absent,
        ),
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &[f32::MAX; 3],
            Presence::Absent,
        ),
    ];
    let primitives = [
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(&[0, 1, 2]),
            crate::MaterialAssignment::Unspecified,
            AuthoringRange::new(0, 1),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Present(&[]),
            crate::MaterialAssignment::Unspecified,
            AuthoringRange::new(1, 1),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Absent,
            crate::MaterialAssignment::Unspecified,
            AuthoringRange::new(2, 0),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Absent,
            crate::MaterialAssignment::Unspecified,
            AuthoringRange::new(2, 1),
        ),
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Absent,
            crate::MaterialAssignment::Unspecified,
            AuthoringRange::new(3, 1),
        ),
    ];
    let meshes: [_; 5] = std::array::from_fn(|index| {
        GeometryLogicalMeshAuthoring::new(AuthoringRange::new(index as u32, 1))
    });
    let instances: [_; 4] = std::array::from_fn(|index| {
        GeometryMeshInstanceAuthoring::new(
            NodeLocalId::new(0),
            LogicalMeshLocalId::new(index as u32),
        )
    });
    let geometry = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances);
    let nodes = NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &SINGLE_TRS_NODES);
    let scene = publish_static_scene(&account, nodes, geometry);
    let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
    let expected = [
        Some(Aabb {
            min: [0.0, -100.0, 0.0],
            max: [100.0, 1.0, 50.0],
        }),
        Some(Aabb {
            min: [7.0, 8.0, 9.0],
            max: [7.0, 8.0, 9.0],
        }),
        None,
        Some(Aabb {
            min: [0.0; 3],
            max: [0.0; 3],
        }),
    ];
    for (instance, expected) in scene.mesh_instances().zip(expected) {
        assert_eq!(
            table.instance_bounds(instance, UNLIMITED, None).unwrap(),
            expected
        );
    }
    assert_eq!(
        table.scene_bounds(&scene, UNLIMITED, None).unwrap(),
        Some(Aabb {
            min: [0.0, -100.0, 0.0],
            max: [100.0, 8.0, 50.0],
        })
    );
    let uninstanced = publish_static_scene(
        &account,
        nodes,
        GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &[]),
    );
    let table = uninstanced
        .base_pose_world_transforms(UNLIMITED, None)
        .unwrap();
    assert_eq!(
        table.scene_bounds(&uninstanced, SpatialQueryLimits::new(0, 0), None),
        Ok(None)
    );
    let empty = publish_static_scene(
        &account,
        NodeAuthoringBatch::new(Presence::Present(&[]), &[]),
        GeometryAuthoringBatch::empty(),
    );
    let empty_table = empty
        .base_pose_world_transforms(SpatialQueryLimits::new(0, 0), None)
        .unwrap();
    assert_eq!(empty_table.node_count(), 0);
    assert_eq!(empty_table.allocated_bytes(), 0);
    assert_eq!(empty_table.generation(), empty.generation());
    assert_eq!(
        empty_table.scene_bounds(&empty, SpatialQueryLimits::new(0, 0), None),
        Ok(None)
    );
    drop(empty);
    drop(uninstanced);
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn morph_overrides_take_precedence_and_nonzero_weights_reject_even_empty_geometry() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    for positions in [&[2.0, 3.0, 4.0][..], &[][..]] {
        for defaults in [[0.25, -0.5], [0.0, -0.0]] {
            let attributes = [GeometryAttributeAuthoring::f32(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                positions,
                Presence::Absent,
            )];
            let primitives = [GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::Points,
                Presence::Absent,
                crate::MaterialAssignment::Unspecified,
                AuthoringRange::new(0, 1),
            )];
            let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
            let instances = [GeometryMeshInstanceAuthoring::new(
                NodeLocalId::new(0),
                LogicalMeshLocalId::new(0),
            ); 3];
            let definitions = [
                MorphTargetDefinitionAuthoring::new(MorphTargetIdentity::new(0), Presence::Absent),
                MorphTargetDefinitionAuthoring::new(MorphTargetIdentity::new(1), Presence::Absent),
            ];
            let morphs = [LogicalMorphAuthoring::new(
                AuthoringRange::new(0, 2),
                &defaults,
            )];
            let deltas = [MorphDeltaAuthoring::new(
                MorphDeltaSemantic::Position,
                MorphDeltaPayloadAuthoring::Dense {
                    values: if positions.is_empty() {
                        &[]
                    } else {
                        &[99.0; 3]
                    },
                },
            )];
            let targets = [
                PrimitiveMorphTargetAuthoring::new(&deltas),
                PrimitiveMorphTargetAuthoring::new(&[]),
            ];
            let deformations = [PrimitiveDeformationAuthoring::new(
                Presence::Absent,
                &targets,
            )];
            let overrides = [
                MeshInstanceDeformationAuthoring::new(
                    Presence::Absent,
                    Presence::Present(&[0.0, -0.0]),
                ),
                MeshInstanceDeformationAuthoring::new(Presence::Absent, Presence::Absent),
                MeshInstanceDeformationAuthoring::new(
                    Presence::Absent,
                    Presence::Present(&[0.0, 0.125]),
                ),
            ];
            let scene = publish_scene(
                &account,
                NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &SINGLE_TRS_NODES),
                GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
                DeformationAuthoringBatch::new(
                    &[],
                    &definitions,
                    &morphs,
                    &deformations,
                    &overrides,
                ),
                AnimationAuthoringBatch::empty(),
            );
            let before = snapshot_digest(&scene);
            let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
            let zero = scene.mesh_instances().next().unwrap();
            let inherited = scene.mesh_instances().nth(1).unwrap();
            let nonzero = scene.mesh_instances().nth(2).unwrap();
            let expected = (!positions.is_empty()).then_some(Aabb {
                min: [2.0, 3.0, 4.0],
                max: [2.0, 3.0, 4.0],
            });
            // Instance + two effective weights + primitive + Position + logical vertices.
            let exact_work = if positions.is_empty() { 5 } else { 6 };
            assert_eq!(
                table.instance_bounds(zero, SpatialQueryLimits::new(exact_work, 0), None),
                Ok(expected)
            );
            assert_eq!(
                table.instance_bounds(zero, SpatialQueryLimits::new(exact_work - 1, 0), None),
                Err(SpatialQueryError::LimitExceeded {
                    limit: SpatialQueryLimit::Work,
                    actual: exact_work,
                    maximum: exact_work - 1,
                })
            );
            let inherited_expected = if defaults[0] == 0.0 {
                Ok(expected)
            } else {
                Err(SpatialQueryError::NonzeroMorphWeight {
                    instance: inherited.id(),
                    target: 0,
                })
            };
            assert_eq!(
                table.instance_bounds(inherited, UNLIMITED, None),
                inherited_expected
            );
            assert_eq!(
                table.instance_bounds(nonzero, UNLIMITED, None),
                Err(SpatialQueryError::NonzeroMorphWeight {
                    instance: nonzero.id(),
                    target: 1
                })
            );
            assert_eq!(
                table.scene_bounds(&scene, UNLIMITED, None),
                Err(SpatialQueryError::NonzeroMorphWeight {
                    instance: if defaults[0] == 0.0 {
                        nonzero.id()
                    } else {
                        inherited.id()
                    },
                    target: if defaults[0] == 0.0 { 1 } else { 0 },
                })
            );
            assert_eq!(snapshot_digest(&scene), before);
            drop(scene);
            assert_eq!(account.snapshot().live_retained_bytes(), 0);
        }
    }
}

#[test]
fn skinned_animated_scene_allows_base_transforms_but_rejects_static_bounds() {
    let factory = test_factory();
    let account = test_account(&factory, 4096);
    let parts = build_gltf_capability_scene_for_probe(&account, &AtomicU64::new(0));
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let before = snapshot_digest(&scene);
    assert!(scene.animation_clip_count() > 0);
    let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
    for node in scene.nodes() {
        assert_eq!(
            table.node_transform(node.id()).unwrap(),
            Mat4::IDENTITY.cols.map(|column| column.map(f64::from))
        );
    }
    let instance = scene.mesh_instances().next().unwrap();
    assert!(instance.skin().is_some());
    let expected = Err(SpatialQueryError::SkinUnsupported {
        instance: instance.id(),
    });
    assert_eq!(table.instance_bounds(instance, UNLIMITED, None), expected);
    assert_eq!(table.scene_bounds(&scene, UNLIMITED, None), expected);
    assert_eq!(snapshot_digest(&scene), before);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn spatial_limits_charge_exact_transform_bytes_and_complete_query_work() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let scene = shared_triangle_scene(&account);
    let before = snapshot_digest(&scene);
    let retained = account.snapshot();
    let bytes = size_of::<[[f64; 4]; 4]>() as u64;
    assert_eq!(
        scene
            .base_pose_world_transforms(SpatialQueryLimits::new(2, bytes - 1), None)
            .unwrap_err(),
        SpatialQueryError::LimitExceeded {
            limit: SpatialQueryLimit::Bytes,
            actual: bytes,
            maximum: bytes - 1,
        }
    );
    assert_eq!(
        scene
            .base_pose_world_transforms(SpatialQueryLimits::new(1, bytes), None)
            .unwrap_err(),
        SpatialQueryError::LimitExceeded {
            limit: SpatialQueryLimit::Work,
            actual: 2,
            maximum: 1,
        }
    );
    let mut table = None;
    let allocations = allocation_counter::measure(|| {
        table = Some(
            scene
                .base_pose_world_transforms(SpatialQueryLimits::new(2, bytes), None)
                .unwrap(),
        );
    });
    assert_eq!(allocations.count_total, 1);
    assert_eq!(allocations.bytes_total, bytes);
    let table = table.unwrap();
    assert_eq!(table.allocated_bytes() as u64, bytes);
    let instance = scene.mesh_instances().next().unwrap();
    let expected = Some(Aabb {
        min: [0.0; 3],
        max: [1.0, 1.0, 0.0],
    });
    // One instance, one primitive, the Position descriptor, three vertices.
    let allocations = allocation_counter::measure(|| {
        assert_eq!(
            table.instance_bounds(instance, SpatialQueryLimits::new(6, 0), None),
            Ok(expected)
        );
        assert_eq!(
            table.instance_bounds(instance, SpatialQueryLimits::new(5, 0), None),
            Err(SpatialQueryError::LimitExceeded {
                limit: SpatialQueryLimit::Work,
                actual: 6,
                maximum: 5
            })
        );
        assert_eq!(
            table.scene_bounds(&scene, SpatialQueryLimits::new(12, 0), None),
            Ok(expected)
        );
        assert_eq!(
            table.scene_bounds(&scene, SpatialQueryLimits::new(11, 0), None),
            Err(SpatialQueryError::LimitExceeded {
                limit: SpatialQueryLimit::Work,
                actual: 12,
                maximum: 11
            })
        );
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    assert_eq!(snapshot_digest(&scene), before);
    assert_eq!(account.snapshot(), retained);
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn world_transform_table_checks_generation_range_and_outlives_source_scene() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let scene = shared_triangle_scene(&account);
    let other = shared_triangle_scene(&account);
    let node = scene.nodes().next().unwrap().id();
    let other_node = other.nodes().next().unwrap().id();
    let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
    assert_eq!(
        table.node_transform(other_node),
        Err(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::Node,
            expected: scene.generation(),
            actual: other.generation(),
        })
    );
    for index in [1, u32::MAX] {
        assert_eq!(
            table.node_transform(NodeId::new(scene.generation(), index)),
            Err(SceneLookupError::OutOfRange {
                kind: SceneEntityKind::Node,
                length: 1
            })
        );
    }
    assert_eq!(
        table.instance_bounds(other.mesh_instances().next().unwrap(), UNLIMITED, None),
        Err(SpatialQueryError::Lookup(
            SceneLookupError::GenerationMismatch {
                kind: SceneEntityKind::MeshInstance,
                expected: scene.generation(),
                actual: other.generation(),
            }
        ))
    );
    assert_eq!(
        table.scene_bounds(&other, UNLIMITED, None),
        Err(SpatialQueryError::Lookup(
            SceneLookupError::GenerationMismatch {
                kind: SceneEntityKind::Node,
                expected: scene.generation(),
                actual: other.generation(),
            }
        ))
    );
    drop(other);
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(table.generation(), node.generation());
    assert_eq!(table.node_count(), 1);
    assert_eq!(
        table.node_transform(node).unwrap(),
        Mat4::IDENTITY.cols.map(|column| column.map(f64::from))
    );
}

#[test]
fn precancelled_queries_return_no_result_without_allocation_or_scene_mutation() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let scene = shared_triangle_scene(&account);
    let empty = publish_static_scene(
        &account,
        NodeAuthoringBatch::new(Presence::Present(&[]), &[]),
        GeometryAuthoringBatch::empty(),
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    for scene in [&scene, &empty] {
        let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
        let before = snapshot_digest(scene);
        let retained = account.snapshot();
        let allocations = allocation_counter::measure(|| {
            assert_eq!(
                scene
                    .base_pose_world_transforms(SpatialQueryLimits::new(0, 0), Some(&cancellation))
                    .unwrap_err(),
                SpatialQueryError::Cancelled
            );
            assert_eq!(
                table.scene_bounds(scene, SpatialQueryLimits::new(0, 0), Some(&cancellation)),
                Err(SpatialQueryError::Cancelled)
            );
            if let Some(instance) = scene.mesh_instances().next() {
                assert_eq!(
                    table.instance_bounds(
                        instance,
                        SpatialQueryLimits::new(0, 0),
                        Some(&cancellation)
                    ),
                    Err(SpatialQueryError::Cancelled)
                );
            }
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.bytes_total, 0);
        assert_eq!(snapshot_digest(scene), before);
        assert_eq!(account.snapshot(), retained);
    }
    drop(empty);
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn derived_transforms_exceeding_f32_range_preserve_the_scene() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    let almost_unit = Quat::from_xyzw(0.0, 1.00002, 0.0, 0.0);
    assert!(almost_unit.is_unit());
    let roots = [NodeLocalId::new(0)];
    let root_overflow = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::try_trs(Vec3::ZERO, almost_unit, Vec3::new(f32::MAX, 1.0, 1.0)).unwrap(),
        Presence::Absent,
        Presence::Absent,
    )];
    let chain_overflow = [
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::try_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::new(2.0, 1.0, 1.0))
                .unwrap(),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            parent(0, 0),
            AuthoredTransform::try_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::new(f32::MAX, 1.0, 1.0))
                .unwrap(),
            Presence::Absent,
            Presence::Absent,
        ),
    ];
    let y = f64::from(almost_unit.y);
    for (nodes, inspected_node, expected) in [
        (
            &root_overflow[..],
            0,
            (1.0 - y * (y + y)) * f64::from(f32::MAX),
        ),
        (&chain_overflow[..], 1, 2.0 * f64::from(f32::MAX)),
    ] {
        let scene = publish_static_scene(
            &account,
            NodeAuthoringBatch::new(Presence::Present(&roots), nodes),
            GeometryAuthoringBatch::empty(),
        );
        let before = snapshot_digest(&scene);
        let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
        let node = scene.nodes().nth(inspected_node).unwrap();
        assert_eq!(table.node_transform(node.id()).unwrap()[0][0], expected);
        assert_eq!(snapshot_digest(&scene), before);
        drop(scene);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }
}

#[test]
fn numeric_transform_failures_identify_the_descendant_without_changing_scene() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    for (scale, source) in [
        (2.0_f32.powi(100), NumericConversionError::NonFiniteOutput),
        (2.0_f32.powi(-100), NumericConversionError::UnderflowToZero),
    ] {
        let nodes: Vec<_> = (0..11)
            .map(|index| {
                NodeAuthoringNode::new(
                    if index == 0 {
                        Presence::Absent
                    } else {
                        parent(index - 1, 0)
                    },
                    AuthoredTransform::try_trs(
                        Vec3::ZERO,
                        Quat::IDENTITY,
                        Vec3::new(scale, 1.0, 1.0),
                    )
                    .unwrap(),
                    Presence::Absent,
                    Presence::Absent,
                )
            })
            .collect();
        let scene = publish_static_scene(
            &account,
            NodeAuthoringBatch::new(Presence::Present(&SINGLE_TRS_ROOTS), &nodes),
            GeometryAuthoringBatch::empty(),
        );
        let before = snapshot_digest(&scene);
        assert_eq!(
            scene
                .base_pose_world_transforms(UNLIMITED, None)
                .unwrap_err(),
            SpatialQueryError::TransformNumeric {
                node: scene.nodes().nth(10).unwrap().id(),
                source,
            }
        );
        assert_eq!(snapshot_digest(&scene), before);
        drop(scene);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
    }
}

#[test]
fn numeric_position_failures_identify_instance_primitive_and_logical_vertex() {
    let factory = test_factory();
    let account = test_account(&factory, 2048);
    for (scale, position, source) in [
        (2.0, f64::MAX, NumericConversionError::NonFiniteOutput),
        (
            0.5,
            f64::from_bits(1),
            NumericConversionError::UnderflowToZero,
        ),
    ] {
        let roots = [NodeLocalId::new(0), NodeLocalId::new(1)];
        let nodes = [
            SINGLE_TRS_NODES[0],
            NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::try_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::new(scale, 1.0, 1.0))
                    .unwrap(),
                Presence::Absent,
                Presence::Absent,
            ),
        ];
        let positions = [0.0, 0.0, 0.0, position, 0.0, 0.0];
        let failing_position = [position, 0.0, 0.0];
        for failing_attribute in [
            GeometryAttributeAuthoring::f64(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                &positions,
            ),
            GeometryAttributeAuthoring::sparse_f64(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                2,
                &[1],
                &failing_position,
                &[0.0; 3],
            ),
            GeometryAttributeAuthoring::sparse_f64(
                AttributeSemantic::Position,
                AttributeDomain::Vertex,
                3,
                2,
                &[0],
                &[0.0; 3],
                &failing_position,
            ),
        ] {
            let attributes = [
                GeometryAttributeAuthoring::f32(
                    AttributeSemantic::Position,
                    AttributeDomain::Vertex,
                    3,
                    &[0.0; 3],
                    Presence::Absent,
                ),
                failing_attribute,
            ];
            let primitives: [_; 2] = std::array::from_fn(|index| {
                GeometryPrimitiveAuthoring::new(
                    GeometryTopologyAuthoring::Points,
                    Presence::Absent,
                    crate::MaterialAssignment::Unspecified,
                    AuthoringRange::new(index as u32, 1),
                )
            });
            let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 2))];
            let instances = [
                GeometryMeshInstanceAuthoring::new(NodeLocalId::new(0), LogicalMeshLocalId::new(0)),
                GeometryMeshInstanceAuthoring::new(NodeLocalId::new(1), LogicalMeshLocalId::new(0)),
            ];
            let scene = publish_static_scene(
                &account,
                NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
                GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances),
            );
            let before = snapshot_digest(&scene);
            let table = scene.base_pose_world_transforms(UNLIMITED, None).unwrap();
            let instance = scene.mesh_instances().nth(1).unwrap();
            let expected = Err(SpatialQueryError::PositionNumeric {
                instance: instance.id(),
                primitive: instance.mesh().primitives().nth(1).unwrap().id(),
                vertex: 1,
                source,
            });
            assert_eq!(table.instance_bounds(instance, UNLIMITED, None), expected);
            assert_eq!(table.scene_bounds(&scene, UNLIMITED, None), expected);
            assert_eq!(snapshot_digest(&scene), before);
            drop(scene);
            assert_eq!(account.snapshot().live_retained_bytes(), 0);
        }
    }
}
