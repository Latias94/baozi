use super::*;

use std::sync::atomic::AtomicU64;

use crate::canonical_scene::store::construction::node_scene::semantic::deformation::{
    NodeSceneDeformationTables, take_row_read_count_for_test,
};
use crate::canonical_scene::store::construction::node_scene::tests::build_gltf_capability_scene_for_probe;
use crate::canonical_scene::store::production::{
    InfluenceAuthoring, InfluenceRowsAuthoring, MorphDeltaAuthoring, MorphDeltaPayloadAuthoring,
    MorphTargetDefinitionAuthoring, MorphTargetIdentity, PrimitiveMorphTargetAuthoring,
    SkinAuthoring, SkinLocalId,
};
use crate::identity::{LogicalMeshId, MorphTargetId, SceneEntityKind, SceneGenerationId, SkinId};
use crate::{
    MorphDeltaData, MorphDeltaSemantic, SceneLookupError, SkinMatrixCorrection, SkinMatrixSpace,
    SkinMatrixSpaces, ValidatedScene,
};

#[test]
fn public_deformation_reads_gltf_palettes_variable_influences_and_morphs_without_allocation() {
    let factory = factory();
    let account = account(&factory);
    let parts = build_gltf_capability_scene_for_probe(&account, &AtomicU64::new(0));
    let scene = ValidatedScene::from_node_scene(parts.scene);

    let allocations = allocation_counter::measure(|| {
        for _ in 0..2 {
            assert_eq!(scene.skin_count(), 1);
            assert_eq!(scene.skins().len(), 1);
            assert_eq!(scene.morph_target_definition_count(), 2);
            let skin = scene.skins().next().unwrap();
            assert_eq!(skin.name(), Some("Armature"));
            assert_eq!(scene.skin(skin.id()).unwrap().id(), skin.id());
            assert_eq!(skin.joint_count(), 2);
            let mut joints = skin.joints();
            assert_eq!(joints.len(), 2);
            let first_joint = joints.next().unwrap();
            let last_joint = joints.next_back().unwrap();
            assert_ne!(first_joint, last_joint);
            assert_eq!(skin.skeleton_root().unwrap().id(), first_joint);
            assert_eq!(scene.node(first_joint).unwrap().id(), first_joint);
            assert_eq!(scene.node(last_joint).unwrap().id(), last_joint);
            assert_eq!(skin.joints().next().unwrap(), first_joint);
            assert_eq!(skin.matrix_spaces(), SkinMatrixSpaces::canonical());
            let mut inverse_binds = skin.inverse_bind_matrices();
            assert_eq!(inverse_binds.len(), 2);
            assert_eq!(inverse_binds.next().unwrap(), Mat4::IDENTITY);
            let mut expected = Mat4::IDENTITY;
            expected.cols[3][1] = -1.0;
            assert_eq!(inverse_binds.next_back().unwrap(), expected);

            let mesh = scene.meshes().next().unwrap();
            assert_eq!(mesh.morph_target_count(), 2);
            assert_eq!(mesh.default_morph_weights(), &[0.0, 0.0]);
            let mut definitions = mesh.morph_target_definitions();
            let smile = definitions.next().unwrap();
            let blink = definitions.next_back().unwrap();
            assert_eq!(smile.name(), Some("Smile"));
            assert_eq!(blink.name(), Some("Blink"));
            assert_eq!(smile.ordinal(), 0);
            assert_eq!(blink.ordinal(), 1);
            assert_eq!(smile.stable_identity(), 0);
            assert_eq!(blink.stable_identity(), 1);
            assert_eq!(smile.mesh().id(), mesh.id());
            assert_eq!(
                scene.morph_target_definition(blink.id()).unwrap().name(),
                Some("Blink")
            );

            let first = mesh.primitives().next().unwrap();
            let second = mesh.primitives().next_back().unwrap();
            assert_eq!(first.morph_target_count(), 2);
            let rows = first.influences().unwrap();
            assert_eq!(rows.len(), 3);
            assert!(!rows.is_empty());
            assert_eq!(rows.get(0).unwrap().joint_indices(), &[0, 1, 0, 1, 0]);
            assert_eq!(
                rows.get(0).unwrap().weights(),
                &[0.40, 0.30, 0.15, 0.10, 0.05]
            );
            assert_eq!(rows.get(0).unwrap().source_orders(), &[0, 1, 2, 3, 4]);
            assert_eq!(rows.get(0).unwrap().iter().len(), 5);
            let fifth = rows.get(0).unwrap().get(4).unwrap();
            assert_eq!(fifth.palette_joint(), 0);
            assert_eq!(fifth.weight(), 0.05);
            assert_eq!(fifth.source_order(), 4);
            assert_eq!(rows.get(1).unwrap().len(), 2);
            assert_eq!(rows.iter().next_back().unwrap().vertex(), 2);
            assert_eq!(rows.get(2).unwrap().iter().next().unwrap().weight(), 1.0);
            assert!(rows.get(3).is_none());
            assert!(rows.get(usize::MAX).is_none());
            assert_eq!(
                second.influences().unwrap().get(0).unwrap().weights(),
                &[1.0]
            );

            let mut targets = first.morph_targets();
            let mut independent = first.morph_targets();
            assert_eq!(
                targets.next().unwrap().id(),
                independent.next().unwrap().id()
            );
            assert_eq!(targets.next_back().unwrap().id(), blink.id());
            let dense = first.morph_target(smile.id()).unwrap();
            let sparse = first.morph_target(blink.id()).unwrap();
            assert_eq!(dense.primitive().id(), first.id());
            assert_eq!(dense.definition().id(), smile.id());
            assert_eq!(dense.delta_count(), 1);
            assert_eq!(dense.deltas().len(), 1);
            let dense_delta = dense.delta(MorphDeltaSemantic::Position).unwrap();
            assert_eq!(dense_delta.semantic(), MorphDeltaSemantic::Position);
            assert_eq!(dense_delta.len(), 3);
            assert!(!dense_delta.is_empty());
            assert_eq!(
                dense_delta.data(),
                MorphDeltaData::Dense {
                    values: &[0.0, 0.0, 0.0, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0],
                }
            );
            assert_eq!(
                sparse.delta(MorphDeltaSemantic::Position).unwrap().data(),
                MorphDeltaData::Sparse {
                    logical_count: 3,
                    indices: &[1],
                    values: &[0.1, 0.0, 0.0],
                }
            );
            assert!(sparse.delta(MorphDeltaSemantic::Normal).is_none());
            let second_target = second.morph_target(blink.id()).unwrap();
            assert_eq!(second_target.id(), sparse.id());
            assert_eq!(second_target.definition().id(), blink.id());
            assert_eq!(
                second_target
                    .delta(MorphDeltaSemantic::Position)
                    .unwrap()
                    .data(),
                MorphDeltaData::Sparse {
                    logical_count: 3,
                    indices: &[2],
                    values: &[-0.1, 0.0, 0.0],
                }
            );
            let instance = scene.mesh_instances().next().unwrap();
            assert_eq!(instance.skin().unwrap().id(), skin.id());
            assert_eq!(instance.morph_weights(), Some(&[0.25, 0.75][..]));
        }
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

fn deformation_scene(account: &PhysicalStorageAccount) -> NodeSceneSuccessParts {
    let roots = [
        NodeLocalId::new(0),
        NodeLocalId::new(1),
        NodeLocalId::new(2),
    ];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    ); 3];
    let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let attributes = std::array::from_fn::<_, 3, _>(|_| {
        GeometryAttributeAuthoring::f32(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            3,
            &positions,
            Presence::Absent,
        )
    });
    let primitives = std::array::from_fn::<_, 3, _>(|index| {
        GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::Points,
            Presence::Absent,
            Presence::Absent,
            AuthoringRange::new(index as u32, 1),
        )
    });
    let meshes = std::array::from_fn::<_, 3, _>(|index| {
        GeometryLogicalMeshAuthoring::new(AuthoringRange::new(index as u32, 1))
    });
    let instances = std::array::from_fn::<_, 3, _>(|index| {
        GeometryMeshInstanceAuthoring::new(
            NodeLocalId::new(index as u32),
            LogicalMeshLocalId::new(index as u32),
        )
    });
    let geometry = GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances);
    let first_name = "Palette α";
    let empty_name = "";
    let joints = [NodeLocalId::new(0), NodeLocalId::new(1)];
    let last_joints = [NodeLocalId::new(2)];
    let inverse_binds = [Mat4::IDENTITY, matrix(-1.0).authored_matrix().unwrap()];
    let last_inverse_binds = [matrix(-3.0).authored_matrix().unwrap()];
    let spaces = |translation| {
        SkinMatrixSpaces::canonical().with_correction(
            SkinMatrixCorrection::new(
                SkinMatrixSpace::LogicalMeshLocalBind,
                SkinMatrixSpace::MeshInstanceLocal,
                matrix(translation).authored_matrix().unwrap(),
            )
            .unwrap(),
        )
    };
    let skins = [
        SkinAuthoring::new(
            Presence::Present(first_name),
            &joints,
            &inverse_binds,
            Presence::Present(NodeLocalId::new(0)),
            spaces(2.0),
        ),
        SkinAuthoring::new(
            Presence::Absent,
            &[],
            &[],
            Presence::Absent,
            SkinMatrixSpaces::canonical(),
        ),
        SkinAuthoring::new(
            Presence::Present(empty_name),
            &last_joints,
            &last_inverse_binds,
            Presence::Absent,
            spaces(3.0),
        ),
    ];
    let definitions = [
        MorphTargetDefinitionAuthoring::new(
            MorphTargetIdentity::new(1),
            Presence::Present(first_name),
        ),
        MorphTargetDefinitionAuthoring::new(MorphTargetIdentity::new(0), Presence::Absent),
        MorphTargetDefinitionAuthoring::new(
            MorphTargetIdentity::new(0),
            Presence::Present(empty_name),
        ),
    ];
    let defaults = [0.2, -0.0];
    let last_defaults = [0.75];
    let logical_morphs = [
        LogicalMorphAuthoring::new(AuthoringRange::new(0, 2), &defaults),
        LogicalMorphAuthoring::new(AuthoringRange::new(2, 1), &last_defaults),
        LogicalMorphAuthoring::new(AuthoringRange::new(3, 0), &[]),
    ];
    let offsets = [0, 3, 3, 4];
    let influences = [
        InfluenceAuthoring::new(0, 0.2, 2),
        InfluenceAuthoring::new(1, 0.3, 0),
        InfluenceAuthoring::new(0, 0.5, 1),
        InfluenceAuthoring::new(1, 1.0, 0),
    ];
    let empty_offsets = [0, 0, 0, 0];
    let dense = [0.0, 0.25, -0.0, 0.0, 0.0, 0.0, -0.5, 0.0, 0.0];
    let normal = [0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let sparse_indices = [2];
    let sparse_values = [-0.25, 0.5, -0.0];
    let first_deltas = [
        MorphDeltaAuthoring::new(
            MorphDeltaSemantic::Position,
            MorphDeltaPayloadAuthoring::Dense { values: &dense },
        ),
        MorphDeltaAuthoring::new(
            MorphDeltaSemantic::Normal,
            MorphDeltaPayloadAuthoring::Dense { values: &normal },
        ),
        MorphDeltaAuthoring::new(
            MorphDeltaSemantic::Tangent,
            MorphDeltaPayloadAuthoring::Sparse {
                logical_count: 3,
                indices: &[],
                values: &[],
            },
        ),
    ];
    let sparse_deltas = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Position,
        MorphDeltaPayloadAuthoring::Sparse {
            logical_count: 3,
            indices: &sparse_indices,
            values: &sparse_values,
        },
    )];
    let last_deltas = [MorphDeltaAuthoring::new(
        MorphDeltaSemantic::Position,
        MorphDeltaPayloadAuthoring::Dense { values: &normal },
    )];
    let targets = [
        PrimitiveMorphTargetAuthoring::new(&first_deltas),
        PrimitiveMorphTargetAuthoring::new(&sparse_deltas),
    ];
    let last_targets = [PrimitiveMorphTargetAuthoring::new(&last_deltas)];
    let primitive_deformations = [
        PrimitiveDeformationAuthoring::new(
            Presence::Present(InfluenceRowsAuthoring::new(&offsets, &influences)),
            &targets,
        ),
        PrimitiveDeformationAuthoring::new(Presence::Absent, &last_targets),
        PrimitiveDeformationAuthoring::new(
            Presence::Present(InfluenceRowsAuthoring::new(&empty_offsets, &[])),
            &[],
        ),
    ];
    let weights = [0.1, 0.9];
    let instance_deformations = [
        MeshInstanceDeformationAuthoring::new(
            Presence::Present(SkinLocalId::new(0)),
            Presence::Present(&weights),
        ),
        MeshInstanceDeformationAuthoring::new(Presence::Absent, Presence::Absent),
        MeshInstanceDeformationAuthoring::new(
            Presence::Present(SkinLocalId::new(2)),
            Presence::Present(&[]),
        ),
    ];
    let deformation = DeformationAuthoringBatch::new(
        &skins,
        &definitions,
        &logical_morphs,
        &primitive_deformations,
        &instance_deformations,
    );
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let nodes = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);
    let appearance = AppearanceAuthoringBatch::empty();
    let animation = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(spatial, nodes, appearance, geometry, deformation, animation);
    publish_scene_input(
        account,
        SceneAuthoringBatch::new(
            spatial,
            nodes,
            appearance,
            geometry,
            deformation,
            animation,
            profile.batch(),
        ),
    )
}

#[test]
fn public_deformation_preserves_presence_identity_order_corrections_and_payload_borrows() {
    let factory = factory();
    let account = account(&factory);
    let parts = deformation_scene(&account);
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let allocations = allocation_counter::measure(|| {
        let first_skin = scene.skins().next().unwrap();
        let empty_skin = scene.skins().nth(1).unwrap();
        let last_skin = scene.skins().next_back().unwrap();
        assert_eq!(first_skin.name(), Some("Palette α"));
        assert_eq!(empty_skin.name(), None);
        assert_eq!(empty_skin.joint_count(), 0);
        assert_eq!(empty_skin.joints().len(), 0);
        assert_eq!(empty_skin.inverse_bind_matrices().len(), 0);
        assert_eq!(last_skin.name(), Some(""));
        assert_eq!(last_skin.skeleton_root().map(|node| node.id()), None);
        assert_eq!(last_skin.joint_count(), 1);
        assert_eq!(
            last_skin.joints().next().unwrap(),
            scene.nodes().next_back().unwrap().id()
        );
        assert_eq!(
            last_skin.inverse_bind_matrices().next().unwrap(),
            matrix(-3.0).authored_matrix().unwrap()
        );
        assert_eq!(
            last_skin.matrix_spaces().correction().unwrap().matrix(),
            matrix(3.0).authored_matrix().unwrap()
        );
        assert_eq!(
            first_skin.matrix_spaces().correction().unwrap().matrix(),
            matrix(2.0).authored_matrix().unwrap()
        );
        assert_eq!(empty_skin.matrix_spaces().correction(), None);

        let first_mesh = scene.meshes().next().unwrap();
        let middle_mesh = scene.meshes().nth(1).unwrap();
        let last_mesh = scene.meshes().next_back().unwrap();
        let first_definition = first_mesh.morph_target_definitions().next().unwrap();
        assert_eq!(first_definition.stable_identity(), 1);
        assert_eq!(first_definition.ordinal(), 0);
        assert_eq!(
            first_mesh
                .morph_target_definition(first_definition.id())
                .unwrap()
                .stable_identity(),
            1
        );
        assert_eq!(
            first_mesh
                .morph_target_definitions()
                .next_back()
                .unwrap()
                .name(),
            None
        );
        assert_eq!(
            middle_mesh
                .morph_target_definitions()
                .next()
                .unwrap()
                .name(),
            Some("")
        );
        assert_eq!(
            first_mesh.default_morph_weights()[1].to_bits(),
            (-0.0_f32).to_bits()
        );
        assert_eq!(middle_mesh.default_morph_weights(), &[0.75]);
        assert!(last_mesh.default_morph_weights().is_empty());
        assert_eq!(last_mesh.morph_target_count(), 0);
        assert_eq!(last_mesh.morph_target_definitions().len(), 0);

        let first = first_mesh.primitives().next().unwrap();
        let middle = middle_mesh.primitives().next().unwrap();
        let last = last_mesh.primitives().next().unwrap();
        let rows = first.influences().unwrap();
        let retained_weights = { rows.get(0).unwrap().weights() };
        assert_eq!(retained_weights, &[0.2, 0.3, 0.5]);
        assert_eq!(rows.get(0).unwrap().source_orders(), &[2, 0, 1]);
        assert_eq!(
            rows.get(0)
                .unwrap()
                .iter()
                .next_back()
                .unwrap()
                .source_order(),
            1
        );
        assert!(rows.get(0).unwrap().get(3).is_none());
        assert!(rows.get(1).unwrap().is_empty());
        assert!(middle.influences().is_none());
        let empty_rows = last.influences().unwrap();
        assert_eq!(empty_rows.len(), 3);
        assert!(empty_rows.iter().all(|row| row.is_empty()));
        assert_eq!(last.morph_target_count(), 0);
        assert_eq!(last.morph_targets().len(), 0);
        assert!(std::ptr::eq(
            retained_weights,
            first.influences().unwrap().get(0).unwrap().weights()
        ));

        let target = first.morph_targets().next().unwrap();
        let mut deltas = target.deltas();
        assert_eq!(deltas.len(), 3);
        assert_eq!(
            deltas.next().unwrap().semantic(),
            MorphDeltaSemantic::Position
        );
        assert_eq!(
            deltas.next().unwrap().semantic(),
            MorphDeltaSemantic::Normal
        );
        assert_eq!(
            deltas.next().unwrap().semantic(),
            MorphDeltaSemantic::Tangent
        );
        let borrowed = {
            first
                .morph_targets()
                .next()
                .unwrap()
                .delta(MorphDeltaSemantic::Position)
                .unwrap()
                .data()
        };
        assert_eq!(borrowed.value(0), Some(Vec3::new(0.0, 0.25, -0.0)));
        assert_eq!(borrowed.value(3), None);
        assert_eq!(borrowed.value(usize::MAX), None);
        let MorphDeltaData::Dense { values } = borrowed else {
            panic!("expected dense delta")
        };
        assert_eq!(values[2].to_bits(), (-0.0_f32).to_bits());
        let MorphDeltaData::Dense { values: again } =
            target.delta(MorphDeltaSemantic::Position).unwrap().data()
        else {
            panic!("expected dense delta")
        };
        assert!(std::ptr::eq(values, again));
        let sparse = first
            .morph_targets()
            .next_back()
            .unwrap()
            .delta(MorphDeltaSemantic::Position)
            .unwrap()
            .data();
        assert_eq!(sparse.value(0), Some(Vec3::ZERO));
        assert_eq!(sparse.value(2), Some(Vec3::new(-0.25, 0.5, -0.0)));
        assert_eq!(sparse.value(3), None);
        assert_eq!(sparse.value(usize::MAX), None);
        let zero_sparse = target.delta(MorphDeltaSemantic::Tangent).unwrap().data();
        assert_eq!(
            zero_sparse,
            MorphDeltaData::Sparse {
                logical_count: 3,
                indices: &[],
                values: &[]
            }
        );
        assert_eq!(zero_sparse.len(), 3);
        assert!(!zero_sparse.is_empty());
        assert_eq!(zero_sparse.value(2), Some(Vec3::ZERO));

        let weights = {
            scene
                .mesh_instances()
                .next()
                .unwrap()
                .morph_weights()
                .unwrap()
        };
        assert_eq!(weights, &[0.1, 0.9]);
        assert!(std::ptr::eq(
            weights,
            scene
                .mesh_instances()
                .next()
                .unwrap()
                .morph_weights()
                .unwrap()
        ));
        let middle_instance = scene.mesh_instances().nth(1).unwrap();
        assert!(middle_instance.skin().is_none());
        assert_eq!(middle_instance.morph_weights(), None);
        let last_instance = scene.mesh_instances().next_back().unwrap();
        assert_eq!(last_instance.morph_weights(), Some(&[][..]));
        assert_eq!(last_instance.skin().unwrap().id(), last_skin.id());
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.bytes_total, 0);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn public_deformation_random_reads_decode_only_the_selected_row() {
    let factory = factory();
    let account = account(&factory);
    let parts = deformation_scene(&account);
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let owner = scene.as_node_scene();
    let proof = owner.read_proof();
    let tables = || NodeSceneDeformationTables::try_new(owner.read_canonical(), proof).unwrap();
    take_row_read_count_for_test();
    assert_eq!(scene.skin_count(), 3);
    assert_eq!(scene.morph_target_definition_count(), 3);
    assert_eq!(scene.skins().len(), 3);
    assert_eq!(take_row_read_count_for_test(), 0);

    assert_eq!(
        tables().skin_at(proof, 0).unwrap().name(),
        Some("Palette α")
    );
    assert_eq!(take_row_read_count_for_test(), 1);
    assert_eq!(tables().skin_at(proof, 2).unwrap().name(), Some(""));
    assert_eq!(take_row_read_count_for_test(), 1);
    assert_eq!(
        tables()
            .logical_morph_at(proof, 2)
            .unwrap()
            .definition_count(),
        0
    );
    assert_eq!(take_row_read_count_for_test(), 1);
    assert_eq!(
        tables().morph_definition_at(proof, 2).unwrap().name(),
        Some("")
    );
    assert_eq!(take_row_read_count_for_test(), 1);
    assert_eq!(
        tables()
            .primitive_at(proof, 2)
            .unwrap()
            .take_influences()
            .unwrap()
            .vertex_count(),
        3
    );
    assert_eq!(take_row_read_count_for_test(), 1);
    assert_eq!(tables().morph_target_at(proof, 2).unwrap().delta_count(), 1);
    assert_eq!(take_row_read_count_for_test(), 1);
    assert_eq!(
        tables().morph_delta_at(proof, 4).unwrap().semantic(),
        MorphDeltaSemantic::Position
    );
    assert_eq!(take_row_read_count_for_test(), 1);
    assert_eq!(
        tables()
            .mesh_instance_at(proof, 2)
            .unwrap()
            .take_morph_weights()
            .unwrap()
            .declared_count(),
        0
    );
    assert_eq!(take_row_read_count_for_test(), 1);
    assert!(tables().skin_at(proof, 3).is_err());
    assert!(tables().morph_delta_at(proof, usize::MAX).is_err());
    assert_eq!(take_row_read_count_for_test(), 0);
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn public_deformation_checked_lookups_validate_generation_owner_then_range() {
    let factory = factory();
    let account = account(&factory);
    let parts = deformation_scene(&account);
    let scene = ValidatedScene::from_node_scene(parts.scene);
    let mesh = scene.meshes().next().unwrap();
    let other_mesh = scene.meshes().nth(1).unwrap();
    let primitive = mesh.primitives().next().unwrap();
    let foreign_generation = SceneGenerationId::allocate().unwrap();
    let stale_skin = SkinId::new(foreign_generation, u32::MAX);
    assert!(matches!(
        scene.skin(stale_skin),
        Err(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::Skin,
            ..
        })
    ));
    assert!(matches!(
        scene.skin(SkinId::new(scene.generation(), u32::MAX)),
        Err(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::Skin,
            length: 3
        })
    ));

    let stale = MorphTargetId::new(LogicalMeshId::new(foreign_generation, u32::MAX), u32::MAX);
    assert!(matches!(
        mesh.morph_target_definition(stale),
        Err(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::MorphTarget,
            ..
        })
    ));
    assert!(matches!(
        primitive.morph_target(stale),
        Err(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::MorphTarget,
            ..
        })
    ));
    assert!(matches!(
        scene.morph_target_definition(stale),
        Err(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::MorphTarget,
            ..
        })
    ));
    let wrong_owner = MorphTargetId::new(other_mesh.id(), u32::MAX);
    assert!(matches!(
        mesh.morph_target_definition(wrong_owner),
        Err(SceneLookupError::OwnerMismatch {
            kind: SceneEntityKind::MorphTarget,
            ..
        })
    ));
    assert!(matches!(
        primitive.morph_target(wrong_owner),
        Err(SceneLookupError::OwnerMismatch {
            kind: SceneEntityKind::MorphTarget,
            ..
        })
    ));
    let out_of_range = MorphTargetId::new(mesh.id(), u32::MAX);
    assert!(matches!(
        mesh.morph_target_definition(out_of_range),
        Err(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::MorphTarget,
            length: 2
        })
    ));
    assert!(matches!(
        primitive.morph_target(out_of_range),
        Err(SceneLookupError::OutOfRange {
            kind: SceneEntityKind::MorphTarget,
            length: 2
        })
    ));
    let valid = mesh.morph_target_definitions().next_back().unwrap();
    assert_eq!(
        mesh.morph_target_definition(valid.id()).unwrap().id(),
        valid.id()
    );
    assert_eq!(primitive.morph_target(valid.id()).unwrap().id(), valid.id());
    assert_eq!(
        scene.morph_target_definition(valid.id()).unwrap().id(),
        valid.id()
    );
    drop(scene);
    drop(parts.report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}
