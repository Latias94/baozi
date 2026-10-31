use std::sync::atomic::{AtomicU64, Ordering};

use baozi_operation::{
    CancellationToken, OperationControl, PhysicalStorageAccount, PhysicalStorageAccountSnapshot,
    StorePreparationFailureClass, StoreTerminalCause,
};

use crate::canonical_scene::store::api::scene::ValidatedScene;
use crate::canonical_scene::store::production::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, CameraLocalId, DeformationAuthoringBatch,
    GeometryAuthoringBatch, LightLocalId, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId,
    Presence, SceneAuthoringBatch, SpatialAuthoringBatch,
};
use crate::canonical_scene::store::profile_provenance::{
    corrupt_profile_reference_for_test, issued_profile_segment_nonce_count_for_test,
};
use crate::canonical_scene::store::typed_storage::profile::{
    StoredOperationKind, StoredProvenanceKind,
};
use crate::entity::{
    AuthoredTransform, CameraProjection, LightDefinition, LightEmission, LightKind,
};
use crate::identity::{
    NodeId, SceneEntityKind, SceneGenerationError, SceneGenerationId, SceneLookupError,
};
use crate::math::{Color, Mat4, Quat, Vec3};
use crate::provenance::{DIGEST_LEN, OptionsDigest};
use crate::scene_space::SceneSpace;
use crate::{LocalAxes, SceneValidationLimits};

use super::super::tests::{
    build_gltf_capability_scene_for_probe, profile_fixture, single_trs_batch, test_account,
    test_factory, test_limits,
};
use super::super::{NodeSceneSuccessParts, NodeSceneTestTransaction, SceneStorage};
use super::execution::NodeSceneEditPrepareError;
use super::plan::NodeSceneEditPlanError;
use super::report::{CanonicalRemapSet, DerivedAbsenceReason, RemapChange};
use super::source::{
    NodeTransformEditStart, NodeTransformEditStartError, try_begin_node_transform_edit,
};
use super::{SceneEditCompilation, SceneNodeTransformEditRequest, compile_node_transform_edit};
use crate::canonical_scene::store::construction::SceneConstructionLimits;

fn authored_transform_at(
    scene: &SceneStorage,
    source_index: usize,
    context: &str,
) -> AuthoredTransform {
    scene
        .semantic_authored_transform_at(source_index)
        .unwrap_or_else(|error| panic!("{context}: {error:?}"))
        .authored_transform()
}

pub(super) fn build_single_trs_scene(
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
    let build = NodeSceneTestTransaction::with_account(
        account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .ok()?
    .finish_draft()
    .ok()?;
    let terminal = build
        .publish_with_generation_source_for_test(|| {
            SceneGenerationId::reserve_from(generation_source)
        })
        .ok()?;
    match terminal {
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
    let cameras = [projection];
    let lights = [LightDefinition::new(
        light_kind,
        LocalAxes::STANDARD,
        emission,
    )];
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
    let build = NodeSceneTestTransaction::with_account(
        account,
        test_limits(),
        input,
        SceneValidationLimits::default(),
    )
    .expect("spatial edit fixture admission")
    .finish_draft()
    .expect("spatial edit fixture construction");
    let terminal = build
        .publish_with_generation_source_for_test(|| {
            SceneGenerationId::reserve_from(generation_source)
        })
        .expect("spatial edit fixture publication");
    match terminal {
        super::super::NodeSceneConstructionTerminal::Success(parts) => parts,
        _ => panic!("spatial edit fixture must publish success"),
    }
}

fn spatial_fixture_values() -> (CameraProjection, LightKind, LightEmission) {
    (
        CameraProjection::perspective(LocalAxes::STANDARD, 1.0, Some(1.5), 0.1, Some(100.0))
            .expect("spatial edit camera"),
        LightKind::point(Some(50.0)).expect("spatial edit point light"),
        LightEmission::try_from_scaled_linear_srgb(Color::WHITE, 10.0)
            .expect("spatial edit emission"),
    )
}

pub(super) fn build_populated_spatial_scene(
    account: &PhysicalStorageAccount,
    generation_source: &AtomicU64,
) -> NodeSceneSuccessParts {
    let (projection, light_kind, emission) = spatial_fixture_values();
    build_spatial_scene(account, generation_source, projection, light_kind, emission)
}

fn assert_populated_spatial_scene(scene: &SceneStorage) {
    let (projection, light_kind, emission) = spatial_fixture_values();
    let storage = scene.tables.spatial.as_ref();
    assert_eq!(storage.space, SceneSpace::STANDARD_TARGET);
    assert_eq!(storage.cameras.as_slice(), &[projection]);
    assert_eq!(storage.lights.len(), 1);
    assert_eq!(storage.lights[0].kind(), light_kind);
    assert_eq!(storage.lights[0].axes(), LocalAxes::STANDARD);
    assert_eq!(storage.lights[0].tint(), emission.tint());
    assert_eq!(
        storage.lights[0].intensity().to_bits(),
        emission.intensity().to_bits()
    );

    let view = scene
        .semantic_view()
        .expect("surviving spatial semantic view");
    assert_eq!(view.camera_count(), 1);
    assert_eq!(view.light_count(), 1);
    assert_eq!(
        view.scene_space().expect("surviving SceneSpace").up_axis(),
        crate::scene_space::Axis::PositiveY,
    );
    assert_eq!(
        view.camera_at(0)
            .expect("surviving camera read")
            .expect("surviving camera zero")
            .projection(),
        projection,
    );
    let light = view
        .light_at(0)
        .expect("surviving light read")
        .expect("surviving light zero")
        .definition();
    assert_eq!(light.kind(), light_kind);
    assert_eq!(light.axes(), LocalAxes::STANDARD);
    assert_eq!(light.tint(), emission.tint());
    assert_eq!(light.intensity().to_bits(), emission.intensity().to_bits());
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

#[test]
fn integration_transform_plan_classifies_without_starting_retained_work() {
    let factory = test_factory();
    let account = test_account(&factory, 256);
    let generation_source = AtomicU64::new(1);
    let parts = build_single_trs_scene(&account, &generation_source);
    let NodeSceneSuccessParts { scene, report } = parts;
    let input = ValidatedScene::from_node_scene(scene);
    drop(report);
    let before = account.snapshot();

    let unchanged = compile_node_transform_edit(
        &input,
        SceneNodeTransformEditRequest::new(
            0,
            AuthoredTransform::identity_trs(),
            edit_options_digest(31),
        ),
    )
    .expect("an unchanged transform request must compile");
    assert!(matches!(unchanged, SceneEditCompilation::Unchanged));

    let changed = compile_node_transform_edit(
        &input,
        SceneNodeTransformEditRequest::new(0, changed_trs(7.0), edit_options_digest(32)),
    )
    .expect("a changed transform request must compile");
    let SceneEditCompilation::Changed(plan) = changed else {
        panic!("a different authored transform must produce a changed plan");
    };
    assert_eq!(plan.input_generation(), input.generation());

    let after = account.snapshot();
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations()
    );
    drop(plan);
    drop(input);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

fn edit_options_digest(seed: u8) -> OptionsDigest {
    OptionsDigest::sha256([seed; 32])
}

fn assert_transform_edit_lineage(
    source: &super::super::SceneStorage,
    output: &super::super::SceneStorage,
    target_owner_row: usize,
    options_digest: OptionsDigest,
) {
    let source_counts = source.proof.counts();
    let output_counts = output.proof.counts();
    let source_profile = source.tables.profile.state.as_ref();
    let output_profile = output.tables.profile.state.as_ref();
    assert_eq!(
        output_counts.profile_segment_count,
        source_counts.profile_segment_count + 1
    );
    assert_eq!(
        output_counts.profile_provenance_count,
        source_counts.profile_provenance_count + 1
    );
    assert_eq!(
        output_counts.profile_operation_count,
        source_counts.profile_operation_count + 1
    );
    assert_eq!(
        output_counts.canonical_block_count,
        source_counts.canonical_block_count
    );
    assert_eq!(
        output_counts.payload_block_count,
        source_counts.payload_block_count
    );
    assert_eq!(
        output_counts.shareable_leaf_block_count,
        source_counts.shareable_leaf_block_count
    );
    let source_owner = &source_profile.owners[target_owner_row];
    let source_parents = source_owner
        .attachments
        .get(&source_profile.attachments)
        .expect("source parents");
    assert_eq!(
        output_counts.profile_parent_count,
        source_counts.profile_parent_count + source_parents.len() as u32
    );
    assert_eq!(
        output_counts.profile_attachment_count,
        source_counts.profile_attachment_count - source_parents.len() as u32 + 1
    );
    let fresh = source_profile.segments.len();
    assert_eq!(output_profile.segments.len(), fresh + 1);
    for (ancestor, retained) in source_profile.segments.iter().zip(&output_profile.segments) {
        assert!(
            std::ptr::eq(ancestor.lineage.as_ref(), retained.lineage.as_ref()),
            "unchanged lineage allocations must be shared"
        );
    }
    assert!(
        std::ptr::eq(
            source.tables.profile.metadata.as_ref(),
            output.tables.profile.metadata.as_ref()
        ),
        "immutable metadata payloads must be shared"
    );
    let segment = &output_profile.segments[fresh];
    let lineage = segment.lineage.as_ref();
    assert_eq!(
        segment.prefixes.provenance,
        source_counts.profile_provenance_count as usize
    );
    assert_eq!(lineage.provenance.len(), 1);
    assert_eq!(lineage.operations.len(), 1);
    assert_eq!(lineage.parents.as_slice(), source_parents);
    let node = lineage.provenance[0];
    assert!(matches!(node.kind, StoredProvenanceKind::Operation(0)));
    assert_eq!(
        node.parents
            .get(&lineage.parents)
            .expect("operation parents"),
        source_parents
    );
    assert_eq!(node.depth, source_owner.maximum_depth + 1);
    let operation = lineage.operations[0];
    assert!(matches!(operation.kind, StoredOperationKind::Transform));
    assert_eq!(operation.options, options_digest);
    assert!(operation.evidence.is_empty());
    let output_owner = &output_profile.owners[target_owner_row];
    let attachments = output_owner
        .attachments
        .get(&output_profile.attachments)
        .expect("output attachments");
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].segment(), fresh as u32);
    assert_eq!(attachments[0].local(), 0);
    assert_eq!(
        output
            .tables
            .profile
            .resolve(attachments[0])
            .map(|(index, _)| index),
        Some(source_counts.profile_provenance_count)
    );
    assert_eq!(output_owner.maximum_depth, node.depth);
    for (ordinal, (before, after)) in source_profile
        .owners
        .iter()
        .zip(&output_profile.owners)
        .enumerate()
    {
        if ordinal != target_owner_row {
            assert_eq!(
                before.attachments.get(&source_profile.attachments),
                after.attachments.get(&output_profile.attachments)
            );
            assert_eq!(before.maximum_depth, after.maximum_depth);
        }
    }
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
    let replacement = authored_transform_at(source, 0, "single-node fixture authored transform");
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
    let target_owner_row = source
        .proof
        .counts()
        .profile_owner_layout()
        .expect("owner layout")
        .ordinal(crate::canonical_scene::store::production::CanonicalProfileOwner::Node(1))
        .expect("node owner") as usize;
    let expected_parent_delta = pending.counts.profile.target_attachment_count.as_u32();
    let expected_attachment_count = pending.counts.profile.output_attachment_count.as_u32();
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
    assert!(!source.read_geometry().primitives.is_empty());
    assert!(
        std::ptr::eq(source.read_geometry(), output.scene.read_geometry()),
        "a transform edit must share the complete geometry owner and all of its scalar pools",
    );
    assert!(
        std::ptr::eq(
            source.tables.spatial.as_ref(),
            output.scene.tables.spatial.as_ref(),
        ),
        "a transform edit must share the actual spatial owner",
    );
    assert!(
        std::ptr::eq(source.read_deformation(), output.scene.read_deformation()),
        "a transform edit must share the complete deformation owner and its payloads",
    );
    assert!(
        std::ptr::eq(source.read_animation(), output.scene.read_animation()),
        "a transform edit must share the complete animation owner and its payloads",
    );
    assert!(
        std::ptr::eq(
            source.read_appearance().rows.as_ref(),
            output.scene.read_appearance().rows.as_ref()
        ) && std::ptr::eq(
            source.read_appearance().payloads.as_ref(),
            output.scene.read_appearance().payloads.as_ref()
        ),
        "a transform edit must share the complete appearance owner and its payloads",
    );
    let first_geometry_scalar = source.read_geometry().f32_values.first().copied();
    let source_central_validation = source.proof.central_validation_plan();
    let output_central_validation = output.scene.proof.central_validation_plan();
    let source_validation = source_central_validation.scratch_plan();
    let output_validation = output_central_validation.scratch_plan();
    assert_ne!(
        output_validation, source_validation,
        "the appended lineage segment must grow the sealed validation envelope",
    );
    assert_eq!(
        output_validation.appearance_counts(),
        source_validation.appearance_counts(),
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
                .expect("source profile validation work")
                + source_profile
                    .scratch_initialization_work()
                    .expect("source profile scratch work"),
        )
        .and_then(|work| {
            work.checked_add(
                output_profile
                    .validation_work()
                    .expect("output profile validation work")
                    + output_profile
                        .scratch_initialization_work()
                        .expect("output profile scratch work"),
            )
        })
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

    assert!(
        authored_transform_at(&output.scene, 0, "edited authored transform")
            .has_same_authored_bits(replacement),
    );
    assert!(
        authored_transform_at(source, 0, "input authored transform remains readable")
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
    drop(parts);
    assert_eq!(
        output.scene.read_geometry().f32_values.first().copied(),
        first_geometry_scalar,
        "the edited generation must retain geometry after the source owner is released",
    );
    assert!(account.snapshot().live_retained_bytes() > 0);
    drop(output);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn host_unwind_releases_edited_tables_and_preserves_shared_source_domains() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    for populated_spatial in [false, true] {
        let original = if populated_spatial {
            build_populated_spatial_scene(&account, &generation_source)
        } else {
            build_gltf_capability_scene_for_probe(&account, &generation_source)
        };
        let source_snapshot = account.snapshot();
        let node = NodeId::new(original.scene.generation(), 0);
        let NodeTransformEditStart::Changed(pending) = try_begin_node_transform_edit(
            &original.scene,
            node,
            changed_trs(29.0),
            edit_options_digest(29),
        )
        .expect("unwind edit plan") else {
            panic!("the unwind fixture must change the transform");
        };
        let prepared = pending
            .prepare_with_generation_source(&original.scene, test_limits(), || {
                SceneGenerationId::reserve_from(&generation_source)
            })
            .expect("unwind edit preparation");
        let super::NodeSceneEditTerminal::Success(edited) = prepared.handoff() else {
            panic!("the unwind fixture must publish the edit");
        };
        assert!(std::ptr::eq(
            original.scene.read_geometry(),
            edited.scene.read_geometry(),
        ));
        assert!(std::ptr::eq(
            original.scene.tables.spatial.as_ref(),
            edited.scene.tables.spatial.as_ref(),
        ));
        assert!(std::ptr::eq(
            original.scene.read_deformation(),
            edited.scene.read_deformation(),
        ));
        assert!(std::ptr::eq(
            original.scene.read_animation(),
            edited.scene.read_animation(),
        ));
        assert!(std::ptr::eq(
            original.scene.read_appearance().rows.as_ref(),
            edited.scene.read_appearance().rows.as_ref()
        ));
        assert!(std::ptr::eq(
            original.scene.read_appearance().payloads.as_ref(),
            edited.scene.read_appearance().payloads.as_ref()
        ));
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _edited = edited;
            panic!("ordinary host unwind with shared Scene tables");
        }));
        assert!(caught.is_err());
        drop(caught);
        assert_no_live_storage_change(source_snapshot, &account);
        assert!(!account.snapshot().poisoned());
        assert!(
            authored_transform_at(&original.scene, 0, "source after edited owner unwind")
                .has_same_authored_bits(AuthoredTransform::identity_trs()),
        );
        if populated_spatial {
            assert_populated_spatial_scene(&original.scene);
        }
        drop(original);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }
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
    let first_owner_row = original
        .scene
        .proof
        .counts()
        .profile_owner_layout()
        .expect("owner layout")
        .ordinal(crate::canonical_scene::store::production::CanonicalProfileOwner::Node(1))
        .expect("node owner") as usize;
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
    let second_owner_row = first
        .scene
        .proof
        .counts()
        .profile_owner_layout()
        .expect("owner layout")
        .ordinal(crate::canonical_scene::store::production::CanonicalProfileOwner::Node(1))
        .expect("node owner") as usize;
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

    let segment_nonce = |scene: &super::super::SceneStorage, row: usize| {
        scene.tables.profile.state.as_ref().segments[row]
            .lineage
            .as_ref()
            .nonce
            .get()
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
    corrupt_profile_reference_for_test(0, 1, left_nonce);
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

    for (component, replacement) in [(0, 1), (1, u64::MAX), (2, 1)] {
        let NodeTransformEditStart::Changed(pending) = try_begin_node_transform_edit(
            &original.scene,
            node,
            changed_trs(57.0 + component as f32),
            edit_options_digest(56 + component as u8),
        )
        .expect("corrupted parent edit plan") else {
            panic!("corrupted parent transform must be changed");
        };
        corrupt_profile_reference_for_test(0, component, replacement);

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
fn trs_to_matrix_preserves_typed_node_layout_before_publication() {
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
        authored_transform_at(&output.scene, 0, "shape-changed authored transform")
            .has_same_authored_bits(replacement),
    );
    assert!(
        authored_transform_at(source, 0, "source authored transform remains readable")
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
    let (projection, light_kind, emission) = spatial_fixture_values();
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
    assert!(std::ptr::eq(
        source.tables.spatial.as_ref(),
        output.scene.tables.spatial.as_ref(),
    ));
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
        .expect("edited light zero")
        .definition();
    assert_eq!(light.kind(), light_kind);
    assert_eq!(light.axes(), LocalAxes::STANDARD);
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
fn physical_plan_keeps_one_root_and_shares_unchanged_typed_domains() {
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
    assert_eq!(total, 1, "all semantic domains are retained typed owners");
    assert_eq!(
        payload, 0,
        "typed payloads are not legacy record-store leaves"
    );
    assert_eq!(
        same_layout.counts.evidence.shareable_leaf_block_count, 0,
        "unchanged payloads are shared through their typed owner",
    );
    assert_eq!(
        same_layout.counts.canonical_attribution_slots.as_usize(),
        total + 1,
        "the destination table root and every actual new block require attribution",
    );
    assert_eq!(
        shape_change.counts.canonical_blocks.as_usize(),
        total,
        "both typed transform variants rebuild only the count directory",
    );
    assert_eq!(shape_change.counts.evidence.shareable_leaf_block_count, 0,);
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
        authored_transform_at(source, 0, "cancelled input transform remains readable")
            .has_same_authored_bits(AuthoredTransform::identity_trs()),
    );
    drop(failure);
    drop(parts);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn edit_generation_source_panic_burns_the_id_and_contains_the_operation() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_single_trs_scene(&account, &generation_source);
    let source = &parts.scene;
    let node = NodeId::new(source.generation(), 0);
    let before = account.snapshot();
    let NodeTransformEditStart::Changed(pending) =
        try_begin_node_transform_edit(source, node, changed_trs(23.0), edit_options_digest(9))
            .expect("generation-fault transform edit plan")
    else {
        panic!("changed transform must enter edit planning");
    };
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = pending.prepare_with_generation_source(source, test_limits(), || {
            let _generation = SceneGenerationId::reserve_from(&generation_source).unwrap();
            panic!("injected edit generation source panic after reservation")
        });
    }))
    .unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"injected edit generation source panic after reservation")
    );
    drop(payload);
    assert_eq!(generation_source.load(Ordering::Relaxed), 2);
    let after = account.snapshot();
    assert!(after.poisoned());
    assert!(after.live_retained_bytes() > before.live_retained_bytes());
    assert!(after.live_retained_allocations() > before.live_retained_allocations());
    assert!(
        authored_transform_at(
            source,
            0,
            "generation-fault input transform remains readable"
        )
        .has_same_authored_bits(AuthoredTransform::identity_trs())
    );
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
        authored_transform_at(
            source,
            0,
            "generation-exhausted input transform remains readable",
        )
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
        authored_transform_at(
            source,
            0,
            "validation-rejected input transform remains readable",
        )
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
        authored_transform_at(source, 0, "prepared-drop input transform remains readable")
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
    let parts = build_populated_spatial_scene(&account, &generation_source);
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

    assert!(std::ptr::eq(
        parts.scene.tables.spatial.as_ref(),
        output.scene.tables.spatial.as_ref(),
    ));
    drop(parts);
    assert_populated_spatial_scene(&output.scene);
    assert!(
        authored_transform_at(&output.scene, 0, "derived transform after input release")
            .has_same_authored_bits(replacement),
    );
    assert_eq!(output.report.outcome_evidence_block_count(), 1);
    let super::publication::NodeSceneEditSuccessParts { scene, report } = output;
    drop(report);
    assert_populated_spatial_scene(&scene);
    assert!(
        authored_transform_at(&scene, 0, "derived transform after report release")
            .has_same_authored_bits(replacement),
    );
    drop(scene);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn dropping_derived_scene_first_keeps_the_input_and_report_readable() {
    let factory = test_factory();
    let account = test_account(&factory, 512);
    let generation_source = AtomicU64::new(0);
    let parts = build_populated_spatial_scene(&account, &generation_source);
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
    assert!(std::ptr::eq(
        parts.scene.tables.spatial.as_ref(),
        output.scene.tables.spatial.as_ref(),
    ));
    let super::publication::NodeSceneEditSuccessParts { scene, report } = output;

    drop(scene);
    assert_populated_spatial_scene(&parts.scene);
    assert!(
        authored_transform_at(
            &parts.scene,
            0,
            "input transform after derived scene release",
        )
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
    assert!(!account.snapshot().poisoned());
}

#[test]
fn post_materialization_product_graph_rejection_preserves_rich_failure_evidence() {
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
                    if terminal_failure_class(failure.ledger_evidence())
                        == StorePreparationFailureClass::ProductGraph =>
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
        "a bounded materialization failure must preserve rich evidence once the failure graph fits",
    );
    assert!(
        observed_post_nonce_storage_rejection,
        "a product-graph rejection after profile rewrite starts must leave one invisible nonce gap",
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
