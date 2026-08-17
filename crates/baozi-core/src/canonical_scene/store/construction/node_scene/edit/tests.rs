use std::sync::atomic::{AtomicU64, Ordering};

use baozi_operation::{
    CancellationToken, OperationControl, PhysicalStorageAccount, PhysicalStorageAccountSnapshot,
    StorePreparationFailureClass, StoreTerminalCause,
};

use crate::SceneValidationLimits;
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, CameraAuthoring, CameraLocalId,
    DeformationAuthoringBatch, GeometryAuthoringBatch, LightAuthoring, LightLocalId,
    NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, Presence, SceneAuthoringBatch,
    SpatialAuthoringBatch,
};
use crate::canonical_scene::store::profile_provenance::{
    PROFILE_PROVENANCE_ADDRESS_SCALARS, corrupt_profile_address_for_test,
    issued_profile_segment_nonce_count_for_test,
};
use crate::canonical_scene::store::schema::profile::{
    PROFILE_DIGEST_SHA256, PROFILE_OPERATION_TRANSFORM, PROFILE_PROVENANCE_OPERATION,
    ProfileLineageAttachmentSchema, ProfileLineageDigestSchema, ProfileLineageOperationSchema,
    ProfileLineageParentSchema, ProfileLineageProvenanceSchema, profile_owner_field,
    profile_provenance_scalar, profile_segment_field,
};
use crate::derived::DerivedAbsenceReason;
use crate::entity::{AuthoredTransform, CameraProjection, LightEmission, LightKind};
use crate::identity::{
    NodeId, SceneEntityKind, SceneGenerationError, SceneGenerationId, SceneLookupError,
};
use crate::math::{Color, Mat4, Quat, Vec3};
use crate::provenance::{DIGEST_LEN, OptionsDigest};
use crate::remap::RemapChange;
use crate::scene_space::SceneSpace;

use super::super::tests::{
    build_gltf_capability_scene_for_probe, profile_fixture, single_trs_batch, test_account,
    test_factory, test_limits,
};
use super::super::{NodeSceneDraftBuild, NodeSceneSuccessParts};
use super::CanonicalRemapSet;
use super::execution::NodeSceneEditPrepareError;
use super::plan::NodeSceneEditPlanError;
use super::source::{
    NodeTransformEditStart, NodeTransformEditStartError, try_begin_node_transform_edit,
};
use crate::canonical_scene::store::construction::{
    SceneConstructionLimits, SceneConstructionOperation,
};

fn build_single_trs_scene(
    account: &PhysicalStorageAccount,
    generation_source: &AtomicU64,
) -> NodeSceneSuccessParts {
    try_build_single_trs_scene(account, generation_source)
        .expect("single-node edit fixture must publish success")
}

fn try_build_single_trs_scene(
    account: &PhysicalStorageAccount,
    generation_source: &AtomicU64,
) -> Option<NodeSceneSuccessParts> {
    let spatial = SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]);
    let nodes = single_trs_batch();
    let appearance = AppearanceAuthoringBatch::empty();
    let geometry = GeometryAuthoringBatch::empty();
    let deformation = DeformationAuthoringBatch::empty();
    let animations = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(
        spatial,
        nodes,
        appearance,
        geometry,
        deformation,
        animations,
    );
    let input = SceneAuthoringBatch::new(
        spatial,
        nodes,
        appearance,
        geometry,
        deformation,
        animations,
        profile.batch(),
    );
    let build = SceneConstructionOperation::with_account_for_scene_input(
        account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .ok()?
    .finish_draft()
    .ok()?;
    let prepared = match build {
        NodeSceneDraftBuild::Draft { draft, owner } => owner
            .prepare_with_generation_source(draft, || {
                SceneGenerationId::reserve_from(generation_source)
            })
            .ok()?,
        NodeSceneDraftBuild::Failure { prepared, .. } => {
            drop(prepared);
            return None;
        }
    };
    match prepared.handoff() {
        super::super::NodeSceneConstructionTerminal::Success(parts) => Some(parts),
        terminal => {
            drop(terminal);
            None
        }
    }
}

fn build_spatial_scene(
    account: &PhysicalStorageAccount,
    generation_source: &AtomicU64,
    projection: CameraProjection,
    light_kind: LightKind,
    emission: LightEmission,
) -> NodeSceneSuccessParts {
    let scene_space = SceneSpace::STANDARD_TARGET;
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Present(CameraLocalId::new(0)),
        Presence::Present(LightLocalId::new(0)),
    )];
    let cameras = [CameraAuthoring::new(projection)];
    let lights = [LightAuthoring::new(light_kind, emission)];
    let spatial = SpatialAuthoringBatch::new(&scene_space, &cameras, &lights);
    let node_batch = NodeAuthoringBatch::new(Presence::Present(&roots), &nodes);
    let appearance = AppearanceAuthoringBatch::empty();
    let geometry = GeometryAuthoringBatch::empty();
    let deformation = DeformationAuthoringBatch::empty();
    let animations = AnimationAuthoringBatch::empty();
    let profile = profile_fixture(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
    );
    let input = SceneAuthoringBatch::new(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
        profile.batch(),
    );
    let build = SceneConstructionOperation::with_account_for_scene_input(
        account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .expect("spatial edit fixture admission")
    .finish_draft()
    .expect("spatial edit fixture construction");
    let prepared = match build {
        NodeSceneDraftBuild::Draft { draft, owner } => owner
            .prepare_with_generation_source(draft, || {
                SceneGenerationId::reserve_from(generation_source)
            })
            .expect("spatial edit fixture preparation"),
        NodeSceneDraftBuild::Failure {
            authoring_rejection,
            prepared,
        } => {
            drop(prepared);
            panic!("spatial edit fixture construction failed: {authoring_rejection:?}");
        }
    };
    match prepared.handoff() {
        super::super::NodeSceneConstructionTerminal::Success(parts) => parts,
        _ => panic!("spatial edit fixture must publish success"),
    }
}

fn changed_trs(translation_x: f32) -> AuthoredTransform {
    AuthoredTransform::try_trs(
        Vec3::new(translation_x, 0.0, 0.0),
        Quat::IDENTITY,
        Vec3::new(1.0, 1.0, 1.0),
    )
    .expect("finite transform-edit fixture")
}

fn changed_matrix(translation_x: f32) -> AuthoredTransform {
    AuthoredTransform::try_matrix(Mat4 {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [translation_x, 0.0, 0.0, 1.0],
        ],
    })
    .expect("finite affine transform-edit fixture")
}

fn edit_options_digest(seed: u8) -> OptionsDigest {
    OptionsDigest::sha256([seed; 32])
}

fn assert_transform_edit_lineage(
    source: &super::super::ValidatedNodeScene,
    output: &super::super::ValidatedNodeScene,
    target_owner_row: usize,
    options_digest: OptionsDigest,
) {
    let source_counts = source.proof.counts();
    let output_counts = output.proof.counts();
    assert_eq!(
        output.proof.profile_segment_count(),
        source.proof.profile_segment_count() + 1,
    );
    assert_eq!(
        output_counts.profile_segment_count,
        source_counts.profile_segment_count + 1,
    );
    assert_eq!(
        output_counts.profile_provenance_count,
        source_counts.profile_provenance_count + 1,
    );
    assert_eq!(
        output_counts.profile_operation_count,
        source_counts.profile_operation_count + 1,
    );
    assert_eq!(
        output_counts.canonical_block_count,
        source_counts.canonical_block_count + 4,
    );
    assert_eq!(
        output_counts.payload_block_count,
        source_counts.payload_block_count + 1,
    );
    assert_eq!(
        output_counts.shareable_leaf_block_count,
        source_counts.shareable_leaf_block_count + 4,
    );

    let source_owners = source
        .canonical
        .records(source.proof.profile_owners().into_store())
        .expect("source profile owner table");
    let source_offset_u32 = source_owners
        .scalar::<u32>(target_owner_row, profile_owner_field::ATTACHMENT_OFFSET)
        .expect("source target attachment offset");
    let source_offset =
        usize::try_from(source_offset_u32).expect("source target attachment offset fits usize");
    let source_count_u32 = source_owners
        .scalar::<u32>(target_owner_row, profile_owner_field::ATTACHMENT_COUNT)
        .expect("source target attachment count");
    let source_count =
        usize::try_from(source_count_u32).expect("source target attachment count fits usize");
    let source_attachment_ref = source_owners
        .sequence_ref::<u64, ProfileLineageAttachmentSchema>(
            target_owner_row,
            profile_owner_field::ATTACHMENTS,
        )
        .expect("source profile attachment reference");
    let source_attachments = source
        .canonical
        .sequence(source_attachment_ref)
        .expect("source profile attachments");
    let source_scalar_offset = source_offset * PROFILE_PROVENANCE_ADDRESS_SCALARS;
    let source_scalar_count = source_count * PROFILE_PROVENANCE_ADDRESS_SCALARS;
    let source_target_parents = &source_attachments.as_slice()
        [source_scalar_offset..source_scalar_offset + source_scalar_count];

    assert_eq!(
        output_counts.profile_parent_count,
        source_counts.profile_parent_count + source_count_u32,
    );
    assert_eq!(
        output_counts.profile_attachment_count,
        source_counts.profile_attachment_count - source_count_u32 + 1,
    );

    let segments = output
        .canonical
        .records(output.proof.profile_segments().into_store())
        .expect("edited profile segment table");
    let fresh_segment = source.proof.profile_segment_count();
    assert_eq!(segments.rows(), fresh_segment + 1);
    let source_segments = source
        .canonical
        .records(source.proof.profile_segments().into_store())
        .expect("source profile segment table");
    for row in 0..fresh_segment {
        assert_eq!(
            segments.scalar::<u64>(row, profile_segment_field::NONCE),
            source_segments.scalar::<u64>(row, profile_segment_field::NONCE),
        );
    }
    assert_eq!(
        segments.scalar::<u32>(fresh_segment, profile_segment_field::SEGMENT),
        Some(u32::try_from(fresh_segment).expect("fresh segment index fits u32")),
    );
    assert_eq!(
        segments.scalar::<u32>(fresh_segment, profile_segment_field::PROVENANCE_COUNT),
        Some(1),
    );
    assert_eq!(
        segments.scalar::<u32>(fresh_segment, profile_segment_field::OPERATION_COUNT),
        Some(1),
    );
    assert_eq!(
        segments.scalar::<u32>(fresh_segment, profile_segment_field::PARENT_COUNT),
        Some(source_count_u32),
    );
    let fresh_nonce = segments
        .scalar::<u64>(fresh_segment, profile_segment_field::NONCE)
        .expect("fresh segment nonce");
    assert_ne!(fresh_nonce, 0);
    assert_eq!(
        segments.scalar::<u32>(fresh_segment, profile_segment_field::PROVENANCE_BASE),
        Some(source_counts.profile_provenance_count),
    );

    let parent_ref = segments
        .sequence_ref::<u64, ProfileLineageParentSchema>(
            fresh_segment,
            profile_segment_field::PARENTS,
        )
        .expect("fresh transform parent reference");
    assert_eq!(
        output
            .canonical
            .sequence(parent_ref)
            .expect("fresh transform parents")
            .as_slice(),
        source_target_parents,
    );

    let provenance_ref = segments
        .sequence_ref::<u32, ProfileLineageProvenanceSchema>(
            fresh_segment,
            profile_segment_field::PROVENANCE_NODES,
        )
        .expect("fresh transform provenance reference");
    let provenance = output
        .canonical
        .sequence(provenance_ref)
        .expect("fresh transform provenance");
    assert_eq!(
        provenance.as_slice()[profile_provenance_scalar::KIND],
        u32::from(PROFILE_PROVENANCE_OPERATION),
    );
    assert_eq!(
        provenance.as_slice()[profile_provenance_scalar::VARIANT_INDEX],
        0,
    );
    assert_eq!(
        provenance.as_slice()[profile_provenance_scalar::PARENT_OFFSET],
        0,
    );
    assert_eq!(
        provenance.as_slice()[profile_provenance_scalar::PARENT_COUNT],
        source_count_u32,
    );

    let operation_ref = segments
        .sequence_ref::<u32, ProfileLineageOperationSchema>(
            fresh_segment,
            profile_segment_field::OPERATIONS,
        )
        .expect("fresh transform operation reference");
    let operation = output
        .canonical
        .sequence(operation_ref)
        .expect("fresh transform operation");
    assert_eq!(
        operation.as_slice(),
        &[
            u32::from(PROFILE_OPERATION_TRANSFORM),
            0,
            0,
            u32::from(PROFILE_DIGEST_SHA256),
            0,
            32,
            0,
            0,
        ],
    );
    let digest_ref = segments
        .bytes_ref::<ProfileLineageDigestSchema>(fresh_segment, profile_segment_field::DIGESTS)
        .expect("fresh transform digest reference");
    assert_eq!(
        output
            .canonical
            .bytes(digest_ref)
            .expect("fresh transform options digest"),
        options_digest.as_bytes(),
    );

    let output_owners = output
        .canonical
        .records(output.proof.profile_owners().into_store())
        .expect("edited profile owner table");
    assert_eq!(
        output_owners.scalar::<u32>(target_owner_row, profile_owner_field::ATTACHMENT_COUNT),
        Some(1),
    );
    let output_attachment_ref = output_owners
        .sequence_ref::<u64, ProfileLineageAttachmentSchema>(
            target_owner_row,
            profile_owner_field::ATTACHMENTS,
        )
        .expect("edited profile attachment reference");
    let output_attachments = output
        .canonical
        .sequence(output_attachment_ref)
        .expect("edited profile attachments");
    let fresh_provenance = source_counts.profile_provenance_count;
    let output_scalar_offset = source_offset * PROFILE_PROVENANCE_ADDRESS_SCALARS;
    assert_eq!(
        &output_attachments.as_slice()[..output_scalar_offset],
        &source_attachments.as_slice()[..source_scalar_offset],
    );
    assert_eq!(
        &output_attachments.as_slice()
            [output_scalar_offset..output_scalar_offset + PROFILE_PROVENANCE_ADDRESS_SCALARS],
        &[
            u64::try_from(fresh_segment).expect("fresh segment ordinal fits u64"),
            fresh_nonce,
            0,
            u64::from(fresh_provenance),
        ],
    );
    let source_suffix = (source_offset + source_count) * PROFILE_PROVENANCE_ADDRESS_SCALARS;
    assert_eq!(
        &output_attachments.as_slice()[output_scalar_offset + PROFILE_PROVENANCE_ADDRESS_SCALARS..],
        &source_attachments.as_slice()[source_suffix..],
    );
}

fn assert_no_physical_change(
    before: PhysicalStorageAccountSnapshot,
    account: &PhysicalStorageAccount,
) {
    assert_eq!(account.snapshot(), before);
}

fn assert_no_live_storage_change(
    before: PhysicalStorageAccountSnapshot,
    account: &PhysicalStorageAccount,
) {
    let after = account.snapshot();
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations(),
    );
}

#[test]
fn unchanged_authored_bits_return_the_exact_input_without_reserving_a_generation() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_single_trs_scene(&account, &generation_source);
    let source = &parts.scene;
    let node = NodeId::new(source.generation(), 0);
    let replacement = source
        .authored_transform(0)
        .expect("single-node fixture authored transform");
    let generation_before = generation_source.load(Ordering::Relaxed);
    let account_before = account.snapshot();
    let nonce_count_before = issued_profile_segment_nonce_count_for_test();

    let started = try_begin_node_transform_edit(source, node, replacement, edit_options_digest(1))
        .expect("identical authored bits must classify without failure");

    let NodeTransformEditStart::Unchanged(returned) = started else {
        panic!("identical authored bits must not open a changed edit");
    };
    assert!(std::ptr::eq(returned, source));
    assert_eq!(
        generation_source.load(Ordering::Relaxed),
        generation_before,
        "unchanged classification must not reserve a generation",
    );
    assert_eq!(
        issued_profile_segment_nonce_count_for_test(),
        nonce_count_before,
        "unchanged classification must not issue a provenance segment nonce",
    );
    assert_no_physical_change(account_before, &account);
}

#[test]
fn changed_trs_publishes_one_exact_lineage_generation_with_complete_unchanged_remaps() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_gltf_capability_scene_for_probe(&account, &generation_source);
    let source = &parts.scene;
    let input_generation = source.generation();
    let node = NodeId::new(input_generation, 0);
    let replacement = changed_trs(7.0);
    let options_digest = edit_options_digest(2);
    let NodeTransformEditStart::Changed(pending) =
        try_begin_node_transform_edit(source, node, replacement, options_digest)
            .expect("changed authored bits must enter edit planning")
    else {
        panic!("changed authored bits must not classify as unchanged");
    };
    let expected_new_allocations = pending
        .counts
        .canonical_attribution_slots
        .as_u32()
        .checked_add(pending.counts.outcome_evidence_attribution_slots.as_u32())
        .expect("test allocation delta");
    let target_owner_row = pending.counts.profile.target_owner_row.as_usize();
    let expected_parent_delta = pending.counts.profile.target_attachment_count.as_u32();
    let expected_attachment_count = pending.counts.profile.output_attachment_count.as_u32();
    let source_transform_work = u64::try_from(pending.counts.source_transform_scalars.as_usize())
        .expect("source transform validation work");
    let output_transform_work = u64::try_from(pending.counts.output_transform_scalars.as_usize())
        .expect("output transform validation work");
    let physical_before = account.snapshot();

    let prepared = pending
        .prepare_with_generation_source(source, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("valid exact-input transform edit must prepare");
    let super::NodeSceneEditTerminal::Success(output) = prepared.handoff() else {
        panic!("valid exact-input transform edit must publish success");
    };

    let output_generation = output.scene.generation();
    assert_ne!(output_generation, input_generation);
    assert_eq!(generation_source.load(Ordering::Relaxed), 2);
    assert!(output.scene.account().is_same_account(source.account()));
    let source_central_validation = source.proof.central_validation_plan();
    let output_central_validation = output.scene.proof.central_validation_plan();
    let source_validation = source_central_validation.scratch_plan();
    let output_validation = output_central_validation.scratch_plan();
    assert_ne!(
        output_validation, source_validation,
        "the appended lineage segment must grow the sealed validation envelope",
    );
    assert_eq!(
        output_validation.appearance_requirements(),
        source_validation.appearance_requirements(),
    );
    assert_eq!(
        output_validation.geometry_requirements(),
        source_validation.geometry_requirements(),
    );
    assert_eq!(
        output_validation.deformation_requirements(),
        source_validation.deformation_requirements(),
    );
    assert_eq!(
        output_validation.animation_counts(),
        source_validation.animation_counts(),
    );
    let source_profile = source_validation.profile_counts();
    let output_profile = output_validation.profile_counts();
    let expected_validation_work = source_central_validation
        .work()
        .get()
        .checked_sub(
            source_profile
                .validation_work()
                .expect("source profile validation work"),
        )
        .and_then(|work| work.checked_sub(source_transform_work))
        .and_then(|work| {
            work.checked_add(
                output_profile
                    .validation_work()
                    .expect("output profile validation work"),
            )
        })
        .and_then(|work| work.checked_add(output_transform_work))
        .expect("output central validation work");
    assert_eq!(
        output_central_validation.work().get(),
        expected_validation_work,
        "the published proof must retain the exact plan-derived central-validation envelope",
    );
    assert_eq!(
        output_profile.provenance_count,
        source_profile.provenance_count + 1
    );
    assert_eq!(
        output_profile.operation_count,
        source_profile.operation_count + 1
    );
    assert_eq!(
        output_profile.parent_count,
        source_profile.parent_count + expected_parent_delta
    );
    assert_eq!(output_profile.attachment_count, expected_attachment_count);
    assert_eq!(
        output_profile.lineage_digest_bytes,
        source_profile.lineage_digest_bytes + u32::try_from(DIGEST_LEN).expect("digest length"),
    );
    assert_eq!(
        output_profile.canonical_block_count,
        source_profile.canonical_block_count + 4
    );
    assert_eq!(
        output_profile.payload_leaf_count,
        source_profile.payload_leaf_count + 1
    );
    assert_eq!(
        output_profile.shareable_leaf_count,
        source_profile.shareable_leaf_count + 4
    );
    assert!(
        output
            .scene
            .authored_transform(0)
            .expect("edited authored transform")
            .has_same_authored_bits(replacement),
    );
    assert!(
        source
            .authored_transform(0)
            .expect("input authored transform remains readable")
            .has_same_authored_bits(AuthoredTransform::identity_trs()),
    );
    let live_allocation_delta = account
        .snapshot()
        .live_retained_allocations()
        .checked_sub(physical_before.live_retained_allocations())
        .expect("a successful edit cannot release source-owned physical storage");
    assert!(
        live_allocation_delta >= u64::from(expected_new_allocations),
        "every attributed destination owner must remain physically accounted; nested backing allocations are counted separately",
    );
    assert_eq!(
        output.report.invalidations().scene_bounds(),
        Some(DerivedAbsenceReason::Invalidated),
    );
    assert_all_subjects_unchanged(output.report.remaps(), input_generation, output_generation);
    assert_transform_edit_lineage(source, &output.scene, target_owner_row, options_digest);
    assert_eq!(
        output
            .report
            .remaps()
            .nodes()
            .try_rebind_unchanged(node)
            .expect("unchanged node relation must remain valid"),
        Some(NodeId::new(output_generation, 0)),
    );
}

#[test]
fn chained_transform_edits_parent_each_new_segment_to_the_previous_attachment() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let original = build_single_trs_scene(&account, &generation_source);
    let first_digest = edit_options_digest(40);
    let first_node = NodeId::new(original.scene.generation(), 0);
    let NodeTransformEditStart::Changed(first_pending) =
        try_begin_node_transform_edit(&original.scene, first_node, changed_trs(41.0), first_digest)
            .expect("first chained transform edit plan")
    else {
        panic!("first chained transform must be changed");
    };
    let first_owner_row = first_pending.counts.profile.target_owner_row.as_usize();
    let first_prepared = first_pending
        .prepare_with_generation_source(&original.scene, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("first chained transform edit preparation");
    let super::NodeSceneEditTerminal::Success(first) = first_prepared.handoff() else {
        panic!("first chained transform edit must publish success");
    };
    assert_transform_edit_lineage(&original.scene, &first.scene, first_owner_row, first_digest);

    let second_digest = edit_options_digest(42);
    let second_node = NodeId::new(first.scene.generation(), 0);
    let NodeTransformEditStart::Changed(second_pending) =
        try_begin_node_transform_edit(&first.scene, second_node, changed_trs(43.0), second_digest)
            .expect("second chained transform edit plan")
    else {
        panic!("second chained transform must be changed");
    };
    let second_owner_row = second_pending.counts.profile.target_owner_row.as_usize();
    let second_prepared = second_pending
        .prepare_with_generation_source(&first.scene, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("second chained transform edit preparation");
    let super::NodeSceneEditTerminal::Success(second) = second_prepared.handoff() else {
        panic!("second chained transform edit must publish success");
    };
    assert_transform_edit_lineage(&first.scene, &second.scene, second_owner_row, second_digest);

    drop(second);
    drop(first);
    drop(original);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn sibling_transform_edits_preserve_the_ancestor_nonce_and_mint_distinct_branch_nonces() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let original = build_single_trs_scene(&account, &generation_source);
    let node = NodeId::new(original.scene.generation(), 0);

    let NodeTransformEditStart::Changed(left_pending) = try_begin_node_transform_edit(
        &original.scene,
        node,
        changed_trs(51.0),
        edit_options_digest(50),
    )
    .expect("left sibling edit plan") else {
        panic!("left sibling transform must be changed");
    };
    let left_prepared = left_pending
        .prepare_with_generation_source(&original.scene, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("left sibling edit preparation");
    let super::NodeSceneEditTerminal::Success(left) = left_prepared.handoff() else {
        panic!("left sibling edit must publish success");
    };

    let NodeTransformEditStart::Changed(right_pending) = try_begin_node_transform_edit(
        &original.scene,
        node,
        changed_trs(53.0),
        edit_options_digest(52),
    )
    .expect("right sibling edit plan") else {
        panic!("right sibling transform must be changed");
    };
    let right_prepared = right_pending
        .prepare_with_generation_source(&original.scene, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("right sibling edit preparation");
    let super::NodeSceneEditTerminal::Success(right) = right_prepared.handoff() else {
        panic!("right sibling edit must publish success");
    };

    let segment_nonce = |scene: &super::super::ValidatedNodeScene, row: usize| {
        scene
            .canonical
            .records(scene.proof.profile_segments().into_store())
            .and_then(|segments| segments.scalar::<u64>(row, profile_segment_field::NONCE))
            .expect("validated profile segment nonce")
    };
    let ancestor_nonce = segment_nonce(&original.scene, 0);
    assert_eq!(segment_nonce(&left.scene, 0), ancestor_nonce);
    assert_eq!(segment_nonce(&right.scene, 0), ancestor_nonce);
    let left_nonce = segment_nonce(&left.scene, 1);
    let right_nonce = segment_nonce(&right.scene, 1);
    assert_ne!(left_nonce, ancestor_nonce);
    assert_ne!(right_nonce, ancestor_nonce);
    assert_ne!(left_nonce, right_nonce);

    let retained_siblings = account.snapshot();
    let NodeTransformEditStart::Changed(substituted_pending) = try_begin_node_transform_edit(
        &original.scene,
        node,
        changed_trs(55.0),
        edit_options_digest(54),
    )
    .expect("substituted sibling edit plan") else {
        panic!("substituted sibling transform must be changed");
    };
    corrupt_profile_address_for_test(0, 1, left_nonce);
    let substituted_prepared = substituted_pending
        .prepare_with_generation_source(&original.scene, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("substituted sibling address must prepare rich failure");
    let super::NodeSceneEditTerminal::RichFailure(substituted_failure) =
        substituted_prepared.handoff()
    else {
        panic!("substituted sibling address must fail central validation");
    };
    assert_eq!(
        terminal_failure_code(substituted_failure.ledger_evidence()),
        super::super::NODE_SCENE_BUILD_INVARIANT_CODE,
    );
    drop(substituted_failure);
    assert_no_live_storage_change(retained_siblings, &account);

    drop(right);
    drop(left);
    drop(original);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn central_validation_rejects_each_corrupted_parent_address_component() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let original = build_single_trs_scene(&account, &generation_source);
    let node = NodeId::new(original.scene.generation(), 0);
    let retained_source = account.snapshot();

    for (component, replacement) in [(0, 1), (1, u64::MAX), (2, 1), (3, 1)] {
        let NodeTransformEditStart::Changed(pending) = try_begin_node_transform_edit(
            &original.scene,
            node,
            changed_trs(57.0 + component as f32),
            edit_options_digest(56 + component as u8),
        )
        .expect("corrupted parent edit plan") else {
            panic!("corrupted parent transform must be changed");
        };
        corrupt_profile_address_for_test(1, component, replacement);

        let prepared = pending
            .prepare_with_generation_source(&original.scene, test_limits(), || {
                SceneGenerationId::reserve_from(&generation_source)
            })
            .expect("corrupted parent address must prepare rich failure");
        let super::NodeSceneEditTerminal::RichFailure(failure) = prepared.handoff() else {
            panic!("corrupted parent address must fail central validation");
        };
        assert_eq!(
            terminal_failure_code(failure.ledger_evidence()),
            super::super::NODE_SCENE_BUILD_INVARIANT_CODE,
        );
        drop(failure);
        assert_no_live_storage_change(retained_source, &account);
    }

    drop(original);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn trs_to_matrix_rewrites_node_offsets_and_directory_shape_before_publication() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_single_trs_scene(&account, &generation_source);
    let source = &parts.scene;
    let input_generation = source.generation();
    let node = NodeId::new(input_generation, 0);
    let replacement = changed_matrix(5.0);
    let NodeTransformEditStart::Changed(pending) =
        try_begin_node_transform_edit(source, node, replacement, edit_options_digest(3))
            .expect("TRS-to-matrix authored bits must enter edit planning")
    else {
        panic!("TRS-to-matrix replacement cannot classify as unchanged");
    };

    let prepared = pending
        .prepare_with_generation_source(source, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("shape-changing exact-input transform edit must prepare");
    let super::NodeSceneEditTerminal::Success(output) = prepared.handoff() else {
        panic!("shape-changing exact-input transform edit must publish success");
    };

    assert_ne!(output.scene.generation(), input_generation);
    assert!(
        output
            .scene
            .authored_transform(0)
            .expect("shape-changed authored transform")
            .has_same_authored_bits(replacement),
    );
    assert!(
        source
            .authored_transform(0)
            .expect("source authored transform remains readable")
            .has_same_authored_bits(AuthoredTransform::identity_trs()),
    );
    assert_all_subjects_unchanged(
        output.report.remaps(),
        input_generation,
        output.scene.generation(),
    );
}

#[test]
fn shape_changing_edit_preserves_spatial_values_and_attachment_closure() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let projection = CameraProjection::perspective(1.0, Some(1.5), 0.1, Some(100.0))
        .expect("spatial edit camera");
    let light_kind = LightKind::point(Some(50.0)).expect("spatial edit point light");
    let emission = LightEmission::try_from_scaled_linear_srgb(Color::WHITE, 10.0)
        .expect("spatial edit emission");
    let parts = build_spatial_scene(
        &account,
        &generation_source,
        projection,
        light_kind,
        emission,
    );
    let source = &parts.scene;
    let input_generation = source.generation();
    let node = NodeId::new(input_generation, 0);
    let NodeTransformEditStart::Changed(pending) =
        try_begin_node_transform_edit(source, node, changed_matrix(17.0), edit_options_digest(4))
            .expect("shape-changing spatial edit plan")
    else {
        panic!("shape-changing spatial edit must not classify as unchanged");
    };

    let prepared = pending
        .prepare_with_generation_source(source, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("shape-changing spatial edit preparation");
    let super::NodeSceneEditTerminal::Success(output) = prepared.handoff() else {
        panic!("shape-changing spatial edit must publish success");
    };

    assert_ne!(output.scene.generation(), input_generation);
    let view = output
        .scene
        .semantic_view()
        .expect("edited spatial semantic view");
    assert_eq!(view.camera_count(), 1);
    assert_eq!(view.light_count(), 1);
    assert_eq!(
        view.scene_space().expect("edited SceneSpace").up_axis(),
        crate::scene_space::Axis::PositiveY,
    );
    assert_eq!(
        view.camera_at(0)
            .expect("edited camera view")
            .expect("edited camera zero")
            .projection(),
        projection,
    );
    let light = view
        .light_at(0)
        .expect("edited light view")
        .expect("edited light zero");
    assert_eq!(light.kind(), light_kind);
    assert_eq!(light.tint(), emission.tint());
    assert_eq!(light.intensity().to_bits(), emission.intensity().to_bits());

    let mut camera_placements = view.camera_placements(0).expect("edited camera placements");
    assert_eq!(camera_placements.next().expect("camera placement"), Some(0));
    assert_eq!(
        camera_placements.next().expect("camera placement end"),
        None,
    );
    camera_placements
        .finish()
        .expect("complete edited camera placement scan");
    let mut light_placements = view.light_placements(0).expect("edited light placements");
    assert_eq!(light_placements.next().expect("light placement"), Some(0));
    assert_eq!(light_placements.next().expect("light placement end"), None,);
    light_placements
        .finish()
        .expect("complete edited light placement scan");

    let mut cursor = view.into_cursor().expect("edited node cursor");
    let mut roots = cursor.take_roots().expect("edited roots cursor");
    assert_eq!(roots.next().expect("edited root"), Some(0));
    assert_eq!(roots.next().expect("edited roots end"), None);
    roots.finish().expect("complete edited roots scan");
    let node = cursor
        .next_node()
        .expect("edited node read")
        .expect("edited node zero");
    assert_eq!(node.camera(), Some(0));
    assert_eq!(node.light(), Some(0));
}

#[test]
fn generation_mismatch_rejects_before_operation_or_allocation() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_single_trs_scene(&account, &generation_source);
    let source = &parts.scene;
    let foreign_generation = SceneGenerationId::reserve_from(&generation_source)
        .expect("foreign test generation")
        .into_id();
    let foreign_node = NodeId::new(foreign_generation, 0);
    let generation_before = generation_source.load(Ordering::Relaxed);
    let account_before = account.snapshot();

    let error = match try_begin_node_transform_edit(
        source,
        foreign_node,
        changed_trs(3.0),
        edit_options_digest(5),
    ) {
        Ok(_) => panic!("a foreign-generation node cannot open an edit"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        NodeTransformEditStartError::Lookup(SceneLookupError::GenerationMismatch {
            kind: SceneEntityKind::Node,
            expected,
            actual,
        }) if expected == source.generation() && actual == foreign_generation
    ));
    assert_eq!(
        generation_source.load(Ordering::Relaxed),
        generation_before,
        "generation rejection must precede reservation",
    );
    assert_no_physical_change(account_before, &account);
}

#[test]
fn sealed_plan_rejects_same_generation_sibling_before_operation_or_allocation() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let first_generation_source = AtomicU64::new(0);
    let second_generation_source = AtomicU64::new(0);
    let first = build_single_trs_scene(&account, &first_generation_source);
    let second = build_single_trs_scene(&account, &second_generation_source);
    assert_eq!(
        first.scene.generation(),
        second.scene.generation(),
        "independent test generation sources must create the same numeric generation",
    );

    let node = NodeId::new(first.scene.generation(), 0);
    let NodeTransformEditStart::Changed(pending) = try_begin_node_transform_edit(
        &first.scene,
        node,
        changed_trs(4.0),
        edit_options_digest(44),
    )
    .expect("same-generation sibling test edit plan") else {
        panic!("different authored transform must produce a sealed edit plan");
    };
    assert_eq!(pending.input_generation(), first.scene.generation());
    let account_before = account.snapshot();
    let nonce_count_before = issued_profile_segment_nonce_count_for_test();

    let rejected = pending.prepare_with_generation_source(&second.scene, test_limits(), || {
        panic!("source binding must fail before generation reservation")
    });

    assert!(matches!(
        rejected,
        Err(NodeSceneEditPrepareError::Plan(
            NodeSceneEditPlanError::InputSourceMismatch { expected, actual }
        )) if expected == first.scene.generation() && actual == second.scene.generation()
    ));
    assert_eq!(
        issued_profile_segment_nonce_count_for_test(),
        nonce_count_before,
        "source binding must fail before provenance nonce issuance",
    );
    assert_no_physical_change(account_before, &account);

    drop(second);
    drop(first);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn operation_start_rejection_does_not_consume_a_profile_segment_nonce() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_single_trs_scene(&account, &generation_source);
    let source = &parts.scene;
    let node = NodeId::new(source.generation(), 0);
    let NodeTransformEditStart::Changed(pending) =
        try_begin_node_transform_edit(source, node, changed_trs(5.0), edit_options_digest(6))
            .expect("start-rejected transform edit plan")
    else {
        panic!("changed transform must enter edit planning");
    };
    let nonce_count_before = issued_profile_segment_nonce_count_for_test();
    let generation_before = generation_source.load(Ordering::Relaxed);
    let rejecting_limits = SceneConstructionLimits {
        attempted_work: 0,
        consumed_work: 0,
        ..test_limits()
    };

    let rejected = pending.prepare_with_generation_source(source, rejecting_limits, || {
        SceneGenerationId::reserve_from(&generation_source)
    });

    assert!(matches!(rejected, Err(NodeSceneEditPrepareError::Start(_))));
    assert_eq!(
        issued_profile_segment_nonce_count_for_test(),
        nonce_count_before,
        "operation-start rejection must precede nonce issuance",
    );
    assert_eq!(
        generation_source.load(Ordering::Relaxed),
        generation_before,
        "operation-start rejection must precede generation reservation",
    );

    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn physical_plan_rebuilds_every_record_and_inherits_each_unchanged_shareable_leaf() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_gltf_capability_scene_for_probe(&account, &generation_source);
    let source = &parts.scene;
    let node = NodeId::new(source.generation(), 0);

    let NodeTransformEditStart::Changed(same_layout) =
        try_begin_node_transform_edit(source, node, changed_trs(11.0), edit_options_digest(6))
            .expect("TRS replacement plan")
    else {
        panic!("changed TRS fixture must enter edit planning");
    };
    let NodeTransformEditStart::Changed(shape_change) =
        try_begin_node_transform_edit(source, node, changed_matrix(13.0), edit_options_digest(7))
            .expect("TRS-to-matrix replacement plan")
    else {
        panic!("TRS-to-matrix fixture must enter edit planning");
    };

    let total = same_layout.counts.canonical_blocks.as_usize();
    let payload = usize::try_from(same_layout.counts.evidence.payload_block_count)
        .expect("payload block count must fit the platform");
    let leaf_plan = same_layout.counts.evidence_leaves;
    let shareable = leaf_plan.inherited.as_usize();
    assert!(payload > 0, "fixture must contain shareable payload leaves");
    assert!(
        shareable > payload,
        "unchanged profile lineage leaves remain shareable while the attachment leaf is rewritten"
    );
    assert!(
        total > shareable,
        "fixture must contain structural RecordBlock values"
    );
    assert_eq!(
        same_layout.counts.new_canonical_blocks.as_usize(),
        total - shareable,
        "every RecordBlock and edited semantic leaf must be rebuilt",
    );
    assert_eq!(
        same_layout.counts.canonical_attribution_slots.as_usize(),
        total - shareable + 1,
        "the destination table root and every actual new block require attribution",
    );
    assert_eq!(
        same_layout
            .program
            .shape()
            .operation_specific_rewrite_blocks(),
        1,
    );
    assert_eq!(
        shape_change
            .program
            .shape()
            .operation_specific_rewrite_blocks(),
        3,
    );
    assert_eq!(
        shape_change.counts.new_canonical_blocks.as_usize(),
        total - shareable,
        "the one/three custom rewrite distinction must not undercount physical new blocks",
    );
    assert_eq!(
        shape_change.counts.evidence_leaves.inherited.as_usize(),
        shareable,
    );
    assert!(leaf_plan.is_exact());
    assert_eq!(leaf_plan.rewritten.as_usize(), 1);
    assert_eq!(leaf_plan.added.as_usize(), 4);
    assert_eq!(leaf_plan.deleted.as_usize(), 0);
    assert_eq!(
        same_layout.counts.inherited_leaf_scan.inspection_work(),
        same_layout.counts.primitives.as_usize()
            + same_layout.counts.attributes.as_usize()
            + same_layout.counts.profile.source_segment_count.as_usize()
            + 1,
    );
    assert_eq!(
        leaf_plan.source.as_usize(),
        leaf_plan.inherited.as_usize() + leaf_plan.rewritten.as_usize(),
    );
    assert_eq!(
        leaf_plan.output.as_usize(),
        leaf_plan.inherited.as_usize()
            + leaf_plan.rewritten.as_usize()
            + leaf_plan.added.as_usize(),
    );
}

#[test]
fn cancellation_after_evidence_commit_preserves_rich_failure_and_the_input() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_single_trs_scene(&account, &generation_source);
    let source = &parts.scene;
    let node = NodeId::new(source.generation(), 0);
    let NodeTransformEditStart::Changed(pending) =
        try_begin_node_transform_edit(source, node, changed_trs(19.0), edit_options_digest(8))
            .expect("cancelled transform edit plan")
    else {
        panic!("changed transform must enter edit planning");
    };
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let prepared = pending
        .prepare_with_control_and_generation_source_for_test(
            source,
            test_limits(),
            OperationControl::default().with_cancellation(cancellation),
            || SceneGenerationId::reserve_from(&generation_source),
        )
        .expect("cooperative cancellation must prepare an exact terminal");
    let super::NodeSceneEditTerminal::RichFailure(failure) = prepared.handoff() else {
        panic!("post-evidence cancellation must preserve rich failure evidence");
    };

    assert_eq!(failure.outcome_evidence_block_count(), 1);
    assert_eq!(
        terminal_failure_class(failure.ledger_evidence()),
        StorePreparationFailureClass::Control,
    );
    assert_eq!(generation_source.load(Ordering::Relaxed), 1);
    assert!(
        source
            .authored_transform(0)
            .expect("cancelled input transform remains readable")
            .has_same_authored_bits(AuthoredTransform::identity_trs()),
    );
    drop(failure);
    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn generation_exhaustion_after_admission_leaves_one_invisible_nonce_gap() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_single_trs_scene(&account, &generation_source);
    let source = &parts.scene;
    let node = NodeId::new(source.generation(), 0);
    let nonce_count_before = issued_profile_segment_nonce_count_for_test();
    let NodeTransformEditStart::Changed(pending) =
        try_begin_node_transform_edit(source, node, changed_trs(23.0), edit_options_digest(9))
            .expect("generation-exhaustion transform edit plan")
    else {
        panic!("changed transform must enter edit planning");
    };
    assert_eq!(
        issued_profile_segment_nonce_count_for_test(),
        nonce_count_before,
        "pure edit preflight must not issue a provenance segment nonce",
    );

    let prepared = pending
        .prepare_with_generation_source(source, test_limits(), || {
            Err(SceneGenerationError::Exhausted)
        })
        .expect("generation exhaustion must prepare rich failure");
    let super::NodeSceneEditTerminal::RichFailure(failure) = prepared.handoff() else {
        panic!("generation exhaustion must preserve rich evidence");
    };

    assert_eq!(failure.outcome_evidence_block_count(), 1);
    assert_eq!(generation_source.load(Ordering::Relaxed), 1);
    assert_eq!(
        issued_profile_segment_nonce_count_for_test(),
        nonce_count_before + 1,
        "a generation failure after retained assembly admission leaves one invisible nonce gap",
    );
    assert!(
        source
            .authored_transform(0)
            .expect("generation-exhausted input transform remains readable")
            .has_same_authored_bits(AuthoredTransform::identity_trs()),
    );
    drop(failure);
    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn central_validation_rejection_preserves_rich_evidence_and_the_input() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_single_trs_scene(&account, &generation_source);
    let source = &parts.scene;
    let node = NodeId::new(source.generation(), 0);
    let NodeTransformEditStart::Changed(mut pending) =
        try_begin_node_transform_edit(source, node, changed_trs(27.0), edit_options_digest(10))
            .expect("central-validation transform edit plan")
    else {
        panic!("changed transform must enter edit planning");
    };
    pending.counts.evidence.node_count = pending
        .counts
        .evidence
        .node_count
        .checked_add(1)
        .expect("test evidence count remains representable");

    let prepared = pending
        .prepare_with_generation_source(source, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("central validation rejection must prepare rich failure");
    let super::NodeSceneEditTerminal::RichFailure(failure) = prepared.handoff() else {
        panic!("central validation rejection must preserve rich evidence");
    };

    assert_eq!(failure.outcome_evidence_block_count(), 1);
    assert_eq!(
        terminal_failure_code(failure.ledger_evidence()),
        super::super::NODE_SCENE_BUILD_INVARIANT_CODE,
    );
    assert_eq!(generation_source.load(Ordering::Relaxed), 1);
    assert!(
        source
            .authored_transform(0)
            .expect("validation-rejected input transform remains readable")
            .has_same_authored_bits(AuthoredTransform::identity_trs()),
    );
    drop(failure);
    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn dropping_prepared_success_burns_one_gap_and_releases_only_unpublished_output() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_single_trs_scene(&account, &generation_source);
    let source = &parts.scene;
    let node = NodeId::new(source.generation(), 0);
    let live_source = account.snapshot();
    let NodeTransformEditStart::Changed(pending) =
        try_begin_node_transform_edit(source, node, changed_trs(29.0), edit_options_digest(11))
            .expect("prepared-drop transform edit plan")
    else {
        panic!("changed transform must enter edit planning");
    };
    let nonce_count_before = issued_profile_segment_nonce_count_for_test();

    let prepared = pending
        .prepare_with_generation_source(source, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("valid transform edit must prepare success");
    assert_eq!(
        issued_profile_segment_nonce_count_for_test(),
        nonce_count_before + 1,
        "one admitted changed edit must issue exactly one provenance segment nonce",
    );
    assert_eq!(generation_source.load(Ordering::Relaxed), 2);
    drop(prepared);

    let after_drop = account.snapshot();
    assert_eq!(
        after_drop.live_retained_bytes(),
        live_source.live_retained_bytes()
    );
    assert_eq!(
        after_drop.live_retained_allocations(),
        live_source.live_retained_allocations(),
    );
    assert!(
        source
            .authored_transform(0)
            .expect("prepared-drop input transform remains readable")
            .has_same_authored_bits(AuthoredTransform::identity_trs()),
    );
    let next = SceneGenerationId::reserve_from(&generation_source)
        .expect("generation after prepared-drop gap remains reservable")
        .into_id();
    assert_ne!(next, source.generation());
    assert_eq!(generation_source.load(Ordering::Relaxed), 3);

    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn dropping_input_first_keeps_the_derived_scene_and_report_readable() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_gltf_capability_scene_for_probe(&account, &generation_source);
    let node = NodeId::new(parts.scene.generation(), 0);
    let replacement = changed_trs(31.0);
    let NodeTransformEditStart::Changed(pending) =
        try_begin_node_transform_edit(&parts.scene, node, replacement, edit_options_digest(12))
            .expect("input-first release transform edit plan")
    else {
        panic!("changed transform must enter edit planning");
    };
    let prepared = pending
        .prepare_with_generation_source(&parts.scene, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("input-first release transform edit must prepare");
    let super::NodeSceneEditTerminal::Success(output) = prepared.handoff() else {
        panic!("input-first release transform edit must publish success");
    };

    drop(parts);
    assert!(
        output
            .scene
            .authored_transform(0)
            .expect("derived transform after input release")
            .has_same_authored_bits(replacement),
    );
    assert_eq!(output.report.outcome_evidence_block_count(), 1);
    let super::publication::NodeSceneEditSuccessParts { scene, report } = output;
    drop(report);
    assert!(
        scene
            .authored_transform(0)
            .expect("derived transform after report release")
            .has_same_authored_bits(replacement),
    );
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn dropping_derived_scene_first_keeps_the_input_and_report_readable() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_gltf_capability_scene_for_probe(&account, &generation_source);
    let node = NodeId::new(parts.scene.generation(), 0);
    let NodeTransformEditStart::Changed(pending) = try_begin_node_transform_edit(
        &parts.scene,
        node,
        changed_trs(37.0),
        edit_options_digest(13),
    )
    .expect("output-first release transform edit plan") else {
        panic!("changed transform must enter edit planning");
    };
    let prepared = pending
        .prepare_with_generation_source(&parts.scene, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        })
        .expect("output-first release transform edit must prepare");
    let super::NodeSceneEditTerminal::Success(output) = prepared.handoff() else {
        panic!("output-first release transform edit must publish success");
    };
    let super::publication::NodeSceneEditSuccessParts { scene, report } = output;

    drop(scene);
    assert!(
        parts
            .scene
            .authored_transform(0)
            .expect("input transform after derived scene release")
            .has_same_authored_bits(AuthoredTransform::identity_trs()),
    );
    assert_eq!(report.outcome_evidence_block_count(), 1);
    assert!(matches!(
        report.ledger_evidence().header().terminal_cause(),
        StoreTerminalCause::Success,
    ));
    drop(parts);
    assert!(account.snapshot().live_retained_bytes() > 0);
    drop(report);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn post_evidence_storage_rejection_preserves_rich_failure_evidence() {
    let factory = test_factory();
    let mut observed_rich_storage_rejection = false;
    let mut observed_post_nonce_storage_rejection = false;

    for max_live_allocations in 1..=128 {
        let account = test_account(&factory, max_live_allocations);
        let generation_source = AtomicU64::new(0);
        let Some(parts) = try_build_single_trs_scene(&account, &generation_source) else {
            assert_eq!(account.snapshot().live_retained_bytes(), 0);
            assert_eq!(account.snapshot().live_retained_allocations(), 0);
            drop(account);
            continue;
        };
        let source = &parts.scene;
        let node = NodeId::new(source.generation(), 0);
        let NodeTransformEditStart::Changed(pending) =
            try_begin_node_transform_edit(source, node, changed_trs(17.0), edit_options_digest(14))
                .expect("bounded transform edit plan")
        else {
            panic!("changed transform must enter edit planning");
        };
        let nonce_count_before = issued_profile_segment_nonce_count_for_test();

        if let Ok(prepared) = pending.prepare_with_generation_source(source, test_limits(), || {
            SceneGenerationId::reserve_from(&generation_source)
        }) {
            match prepared.handoff() {
                super::NodeSceneEditTerminal::RichFailure(failure)
                    if terminal_failure_code(failure.ledger_evidence())
                        == super::super::NODE_SCENE_STORAGE_REJECTION_CODE =>
                {
                    assert_eq!(failure.outcome_evidence_block_count(), 1);
                    let issued_nonce_count = issued_profile_segment_nonce_count_for_test();
                    let issued_nonce_delta = issued_nonce_count
                        .checked_sub(nonce_count_before)
                        .expect("profile nonce issuance count is monotonic on the test thread");
                    assert!(
                        issued_nonce_delta <= 1,
                        "one edit attempt can issue at most one profile segment nonce",
                    );
                    observed_post_nonce_storage_rejection |= issued_nonce_delta == 1;
                    assert!(failure.account().is_same_account(&account));
                    drop(failure);
                    observed_rich_storage_rejection = true;
                }
                terminal => drop(terminal),
            }
        }

        drop(parts);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
        drop(account);

        if observed_rich_storage_rejection && observed_post_nonce_storage_rejection {
            break;
        }
    }

    assert!(
        observed_rich_storage_rejection,
        "a storage budget that admits edit evidence but rejects canonical output must preserve rich evidence",
    );
    assert!(
        observed_post_nonce_storage_rejection,
        "a storage rejection after profile rewrite starts must leave one invisible nonce gap",
    );
}

fn terminal_failure_code(evidence: &baozi_operation::StoreLedgerEvidence) -> u32 {
    match evidence.header().terminal_cause() {
        StoreTerminalCause::Failure(cause) => cause.code(),
        StoreTerminalCause::Success => {
            panic!("a rejected node transform edit must freeze a failure cause")
        }
        _ => panic!("an unknown terminal cause cannot prove a node transform edit failure"),
    }
}

fn terminal_failure_class(
    evidence: &baozi_operation::StoreLedgerEvidence,
) -> StorePreparationFailureClass {
    match evidence.header().terminal_cause() {
        StoreTerminalCause::Failure(cause) => cause.class(),
        StoreTerminalCause::Success => {
            panic!("a rejected node transform edit must freeze a failure cause")
        }
        _ => panic!("an unknown terminal cause cannot prove a node transform edit failure"),
    }
}

fn assert_all_subjects_unchanged(
    remaps: &CanonicalRemapSet,
    input_generation: SceneGenerationId,
    output_generation: SceneGenerationId,
) {
    macro_rules! assert_unchanged {
        ($remap:expr) => {{
            let remap = $remap;
            assert_eq!(remap.input_generation(), input_generation);
            assert_eq!(remap.output_generation(), output_generation);
            assert_eq!(remap.change(), RemapChange::ExplicitlyUnchanged);
        }};
    }

    assert_unchanged!(remaps.nodes());
    assert_unchanged!(remaps.mesh_instances());
    assert_unchanged!(remaps.logical_meshes());
    assert_unchanged!(remaps.primitives());
    assert_unchanged!(remaps.morph_targets());
    assert_unchanged!(remaps.materials());
    assert_unchanged!(remaps.images());
    assert_unchanged!(remaps.samplers());
    assert_unchanged!(remaps.textures());
    assert_unchanged!(remaps.skins());
    assert_unchanged!(remaps.animation_clips());
    assert_unchanged!(remaps.cameras());
    assert_unchanged!(remaps.lights());
    assert_unchanged!(remaps.vertices());
    assert_unchanged!(remaps.corners());
    assert_unchanged!(remaps.faces());
    assert_unchanged!(remaps.instances());
    assert_eq!(remaps.retained_allocation_bytes(), 0);
}
