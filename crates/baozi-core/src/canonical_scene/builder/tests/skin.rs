use super::*;
use crate::canonical_validation::validate_canonical_scene;
use crate::deform::{CanonicalSkinDescriptor, SkinMatrixSpaces};
use crate::identity::{NodeId, SceneEntityKind, SceneLookupError};
use crate::math::Mat4;
use crate::validation_control::SceneValidationLimits;

fn bind_with_scale(scale: f32) -> Mat4 {
    let mut matrix = Mat4::IDENTITY;
    matrix.cols[0][0] = scale;
    matrix
}

fn descriptor<const N: usize>(
    joints: [NodeId; N],
    inverse_bind_matrices: [Mat4; N],
    skeleton_root_hint: Option<NodeId>,
    provenance: Provenance,
) -> CanonicalSkinDescriptor {
    CanonicalSkinDescriptor::try_new(
        None,
        joints,
        inverse_bind_matrices,
        skeleton_root_hint,
        SkinMatrixSpaces::canonical(),
        provenance,
    )
    .expect("skin descriptor fixture should be valid")
}

#[test]
fn skin_construction_lowers_ordered_qualified_references_and_returns_skin_id() {
    let (mut builder, provenance) = scene_builder();
    let nodes = produce_nodes(&mut builder, provenance, 3);
    let [first, second, third] = nodes.as_slice() else {
        panic!("three requested node fixtures should commit");
    };
    let (first, second, third) = (*first, *second, *third);
    let ordered_joints = [third, first, second];
    let inverse_binds = [
        bind_with_scale(3.0),
        bind_with_scale(1.0),
        bind_with_scale(2.0),
    ];

    let skin = builder
        .try_push_skin(descriptor(
            ordered_joints,
            inverse_binds,
            Some(second),
            provenance,
        ))
        .expect("qualified skin should be stored");

    assert_eq!(skin.generation(), builder.generation());
    assert_eq!(skin.index(), 0);
    let stored = builder.data.skins[0].as_ref();
    let view = SkinView {
        skin: stored,
        generation: builder.generation(),
    };
    assert_eq!(view.joints().collect::<Vec<_>>(), ordered_joints);
    assert_eq!(view.inverse_bind_matrices(), inverse_binds);
    assert_eq!(view.skeleton_root_hint(), Some(second));
    assert_eq!(view.matrix_spaces(), SkinMatrixSpaces::canonical());
    assert_eq!(view.provenance(), provenance);
}

#[test]
fn second_skin_id_and_payload_round_trip_through_canonical_validation() {
    let (mut builder, provenance) = scene_builder();
    let joint = produce_nodes(&mut builder, provenance, 1)[0];
    let generation = builder.generation();
    let first_bind = bind_with_scale(2.0);
    let second_bind = bind_with_scale(3.0);

    let first = builder
        .try_push_skin(descriptor([joint], [first_bind], None, provenance))
        .expect("first qualified skin should be stored");
    let second = builder
        .try_push_skin(descriptor([joint], [second_bind], Some(joint), provenance))
        .expect("second qualified skin should be stored");

    assert_eq!(first.generation(), generation);
    assert_eq!(first.index(), 0);
    assert_eq!(second.generation(), generation);
    assert_eq!(second.index(), 1);

    let scene = validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
        .expect("single-node scene with two valid skins should validate");
    let second_view = scene
        .skin(second)
        .expect("the second skin id should resolve after validation");
    assert_eq!(second_view.joints().collect::<Vec<_>>(), [joint]);
    assert_eq!(second_view.inverse_bind_matrices(), [second_bind]);
    assert_eq!(second_view.skeleton_root_hint(), Some(joint));
    assert_eq!(second_view.provenance(), provenance);
}

#[test]
fn skin_descriptor_reports_first_different_joint_and_root_generations_without_mutation() {
    let (mut first_foreign, first_foreign_provenance) = scene_builder();
    let first_foreign_joint = produce_nodes(&mut first_foreign, first_foreign_provenance, 1)[0];
    let (mut second_foreign, second_foreign_provenance) = scene_builder();
    let second_foreign_joint = produce_nodes(&mut second_foreign, second_foreign_provenance, 1)[0];
    assert_ne!(
        first_foreign_joint.generation(),
        second_foreign_joint.generation()
    );

    let (mut builder, provenance) = scene_builder();
    let local_joint = produce_nodes(&mut builder, provenance, 1)[0];
    let local_out_of_range = NodeId::new(builder.generation(), u32::MAX);
    assert_eq!(first_foreign_joint.index(), local_joint.index());
    assert_ne!(first_foreign_joint.generation(), builder.generation());
    assert_ne!(second_foreign_joint.generation(), builder.generation());
    let original_len = builder.data.skins.len();
    let original_capacity = builder.data.skins.capacity();

    let source_order_error = builder
        .try_push_skin(descriptor(
            [
                local_out_of_range,
                first_foreign_joint,
                second_foreign_joint,
            ],
            [Mat4::IDENTITY; 3],
            None,
            provenance,
        ))
        .unwrap_err();
    assert_eq!(
        source_order_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::Node,
            expected: builder.generation(),
            actual: first_foreign_joint.generation(),
        })
    );
    assert_eq!(builder.data.skins.len(), original_len);
    assert_eq!(builder.data.skins.capacity(), original_capacity);

    let root_error = builder
        .try_push_skin(descriptor(
            [local_joint],
            [Mat4::IDENTITY],
            Some(second_foreign_joint),
            provenance,
        ))
        .unwrap_err();
    assert_eq!(
        root_error,
        CanonicalSceneBuildError::Reference(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::Node,
            expected: builder.generation(),
            actual: second_foreign_joint.generation(),
        })
    );
    assert_eq!(builder.data.skins.len(), original_len);
    assert_eq!(builder.data.skins.capacity(), original_capacity);
}

#[test]
fn skin_descriptor_checks_all_generations_before_any_range() {
    let (mut foreign, foreign_provenance) = scene_builder();
    let foreign_joint = produce_nodes(&mut foreign, foreign_provenance, 1)[0];

    let (mut builder, provenance) = scene_builder();
    produce_nodes(&mut builder, provenance, 1);
    let generation = builder.generation();
    let missing_joint = NodeId::new(generation, 1);

    let later_joint_error = builder
        .try_push_skin(descriptor(
            [missing_joint, foreign_joint],
            [Mat4::IDENTITY; 2],
            None,
            provenance,
        ))
        .unwrap_err();
    assert_generation_mismatch_for_test(later_joint_error, SceneEntityKind::Node);
    assert!(builder.data.skins.is_empty());

    let root_error = builder
        .try_push_skin(descriptor(
            [missing_joint],
            [Mat4::IDENTITY],
            Some(foreign_joint),
            provenance,
        ))
        .unwrap_err();
    assert_generation_mismatch_for_test(root_error, SceneEntityKind::Node);
    assert!(builder.data.skins.is_empty());
}

#[test]
fn skin_descriptor_checks_joint_ranges_in_source_order_before_root_range() {
    let (mut builder, provenance) = scene_builder();
    let nodes = produce_nodes(&mut builder, provenance, 3);
    let generation = builder.generation();
    let first = nodes[0];
    let second = nodes[1];
    let root = nodes[2];
    let missing = NodeId::new(generation, 3);

    for (joints, skeleton_root_hint) in [
        ([missing, second], Some(root)),
        ([first, missing], Some(root)),
        ([first, second], Some(missing)),
    ] {
        let error = builder
            .try_push_skin(descriptor(
                joints,
                [Mat4::IDENTITY; 2],
                skeleton_root_hint,
                provenance,
            ))
            .unwrap_err();
        assert_eq!(
            error,
            CanonicalSceneBuildError::Reference(SceneLookupError::OutOfRange {
                kind: SceneEntityKind::Node,
                length: nodes.len(),
            })
        );
        assert!(builder.data.skins.is_empty());
    }

    let skin = builder
        .try_push_skin(descriptor(
            [first, second],
            [Mat4::IDENTITY; 2],
            Some(root),
            provenance,
        ))
        .expect("all references should resolve once the root enters range");
    assert_eq!(skin.generation(), generation);
    assert_eq!(skin.index(), 0);
    let view = SkinView {
        skin: builder.data.skins[0].as_ref(),
        generation,
    };
    assert_eq!(view.joints().collect::<Vec<_>>(), [first, second]);
    assert_eq!(view.skeleton_root_hint(), Some(root));
}

#[test]
fn prepared_skin_never_publishes_before_commit_and_remains_retryable() {
    let (mut builder, provenance) = scene_builder();
    let joint = produce_nodes(&mut builder, provenance, 1)[0];
    let generation = builder.generation();
    let plan = builder
        .collection_capacity_plan(CanonicalSceneCollection::Skins)
        .expect("skin insertion should have a capacity plan");
    let collection_admitted = plan
        .collection_envelope_delta_bytes()
        .expect("skin collection envelope should fit");
    let value_admitted = plan.value_storage_envelope_bytes();

    let prepared = builder
        .prepare_push_skin(
            descriptor([joint], [Mat4::IDENTITY], Some(joint), provenance),
            plan,
        )
        .expect("valid skin should prepare off to the side");
    assert!(prepared.replaces_collection_allocation());
    assert!(prepared.collection_retained_allocation_bytes() <= collection_admitted);
    assert!(prepared.value_storage_retained_allocation_bytes() <= value_admitted);
    drop(prepared);
    assert!(builder.data.skins.is_empty());
    assert_eq!(builder.data.skins.capacity(), 0);

    let skin = builder
        .try_push_skin(descriptor(
            [joint],
            [Mat4::IDENTITY],
            Some(joint),
            provenance,
        ))
        .expect("dropping a prepared skin must leave the builder reusable");
    assert_eq!(skin.generation(), generation);
    assert_eq!(skin.index(), 0);
    assert_eq!(builder.data.skins.len(), 1);
}

#[test]
fn skin_descriptor_discards_hostile_input_capacity() {
    let (mut builder, provenance) = scene_builder();
    let joint = produce_nodes(&mut builder, provenance, 1)[0];
    let mut joints = Vec::with_capacity(1_024);
    joints.push(joint);
    let mut inverse_binds = Vec::with_capacity(1_024);
    inverse_binds.push(Mat4::IDENTITY);
    let hostile_bytes = joints.capacity() * std::mem::size_of::<NodeId>()
        + inverse_binds.capacity() * std::mem::size_of::<Mat4>();

    let skin = CanonicalSkinDescriptor::try_new(
        None,
        joints,
        inverse_binds,
        None,
        SkinMatrixSpaces::canonical(),
        provenance,
    )
    .expect("bounded hostile-capacity input should be compacted");
    builder
        .try_push_skin(skin)
        .expect("compacted skin should be stored");

    assert!(builder.data.skins[0].retained_allocation_bytes() < hostile_bytes);
}
