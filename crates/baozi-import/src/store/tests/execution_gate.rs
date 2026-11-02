//! Concrete lifetime entry gates for the P3/P4 cutover, not the final parse-context API.

use super::*;
use crate::authoring::{
    AnimationChannelAuthoring, AnimationClipAuthoring, AnimationExtrapolation,
    AnimationInterpolation, AnimationTargetAuthoring, AnimationTimeDomain, AuthoringRange,
};
use crate::dependency::DependencyOriginIssuer;
use baozi_operation::__retained_store_integration::activation::{
    StoreExecutionControl, StoreExecutionStorage, StoreExecutionWork, StoreParserWorkScope,
};

type ExecutionControl<'region> =
    StoreExecutionControl<'region, ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>;

fn with_execution<R>(
    owner: &mut ImportStoreSessionOwner,
    action: impl for<'region> FnOnce(
        &StoreExecutionWork<'region, ImportOperationOwner>,
        &StoreExecutionStorage<'region, ImportOperationOwner>,
        &mut ExecutionControl<'region>,
        &ImportStoreVocabulary,
    ) -> R,
) -> R {
    let data = &mut owner.storage[0];
    let vocabulary = &data.vocabulary;
    // Exclusive access to the real slot keeps dynamic state guards out of callbacks.
    let ImportStoreSessionState::Dormant(outcome) = data.state.get_mut() else {
        panic!("the execution gate requires the actual dormant session owner")
    };
    let (work, storage, mut control) = outcome
        .execution()
        .expect("the dormant owner must lend its existing execution capabilities");
    let result = action(&work, &storage, &mut control, vocabulary);
    control
        .checkpoint()
        .expect("region success must pass the existing control checkpoint");
    work.finish_region()
        .expect("every actual parser channel must resolve before region exit");
    result
}

struct SessionVec<'session, T> {
    values: AccountedVec<T>,
    issuer: &'session DependencyOriginIssuer,
}

impl<'session, T> SessionVec<'session, T> {
    fn new(issuer: &'session DependencyOriginIssuer) -> Self {
        Self {
            values: AccountedVec::new(),
            issuer,
        }
    }

    fn reserve_capacity(
        &mut self,
        issuer: &DependencyOriginIssuer,
        storage: &StoreExecutionStorage<'_, ImportOperationOwner>,
        control: &mut ExecutionControl<'_>,
        vocabulary: &ImportStoreVocabulary,
        required_capacity: usize,
    ) {
        assert!(
            std::ptr::eq(self.issuer, issuer),
            "storage growth must use the independent originating session identity"
        );
        self.values
            .try_reserve_capacity(required_capacity, "p4-execution-gate", |bytes| {
                storage
                    .reserve_retained(
                        control,
                        vocabulary.phase(ImportLedgerPhase::FormatNormalization),
                        vocabulary.dimension(ImportDimension::ScratchBytes),
                        bytes,
                        1,
                    )
                    .map(|raw| ImportStoreReservation {
                        raw: Some(raw),
                        bytes,
                    })
                    .map_err(|cause| {
                        project_store_runtime_error(vocabulary, cause).into_scope_abort_cause()
                    })
            })
            .expect("the existing allocator must admit and qualify the real backing capacity");
    }
}

#[test]
fn concrete_parser_channels_return_owned_storage_before_scene_submission() {
    let (mut owner, account) = start_store_owner_with_account();
    let issuer = DependencyOriginIssuer::new();
    let baseline = account.snapshot();

    let decoded = with_execution(&mut owner, |work, storage, control, vocabulary| {
        let parent_group = work
            .bind_group([vocabulary.dimension(ImportDimension::Records)])
            .expect("the semantic parent layout must bind once");
        let child_group = work
            .bind_group([
                vocabulary.dimension(ImportDimension::Tokens),
                vocabulary.dimension(ImportDimension::DecodeWork),
            ])
            .expect("the repeated child layout must bind once");
        let mut parent = parent_group
            .begin_parser_parent([2], [None])
            .expect("the real parent channel must admit attempted work");
        let mut decoded = SessionVec::new(&issuer);
        let mut append = |storage: &StoreExecutionStorage<'_, ImportOperationOwner>,
                          control: &mut ExecutionControl<'_>,
                          child: &mut StoreParserWorkScope<'_, 2>,
                          value| {
            assert_eq!(child.remaining(), [1, 1]);
            let previous_capacity = decoded.values.capacity();
            let previous_len = decoded.values.len();
            decoded.reserve_capacity(&issuer, storage, control, vocabulary, previous_capacity + 1);
            assert!(decoded.values.capacity() > previous_capacity);
            assert_eq!(decoded.values.len(), previous_len);
            decoded.values.push_reserved_infallible(value);
            child
                .complete([1, 1])
                .expect("the callback must immediately record its actual physical prefix");
            control.completed_action();
        };

        for value in [7_u8, 11] {
            control
                .before_action()
                .expect("each child must pass the existing bounded control cadence");
            let mut child = child_group
                .begin_parser_child([1, 1], [None, None])
                .expect("sequential children must use the same parent and work owner");
            append(storage, control, &mut child, value);
            assert_eq!(child.remaining(), [0, 0]);
            child
                .finish_exact()
                .expect("each child must close before the next child or its parent");
        }

        control
            .checkpoint()
            .expect("parent completion must follow a full checkpoint");
        parent
            .complete([2])
            .expect("the semantic parent must complete after its children");
        parent
            .finish_exact()
            .expect("the parent must resolve on the sole work owner");
        decoded
    });

    assert_eq!(&decoded.values[..], &[7, 11]);
    assert!(std::ptr::eq(decoded.issuer, &issuer));
    let retained = account.snapshot();
    assert_eq!(
        retained.live_retained_bytes(),
        baseline.live_retained_bytes() + decoded.values.capacity_bytes()
    );
    assert_eq!(
        retained.live_retained_allocations(),
        baseline.live_retained_allocations() + 1
    );

    let draft = submit_two_node(&owner)
        .expect("the execution borrow must end before actual Scene construction submission");
    drop(decoded);
    let terminal = owner
        .finish_scene(Some(draft))
        .expect("submission must finish through the existing consuming terminal path");
    assert!(terminal.is_success());
    for dimension in [
        ImportDimension::Records,
        ImportDimension::Tokens,
        ImportDimension::DecodeWork,
    ] {
        let evidence = terminal
            .ledger_evidence()
            .dimension_by_semantic(import_dimension_semantic_id(dimension))
            .expect("the terminal ledger must retain the actual parser channel accounting");
        assert_eq!((evidence.attempted(), evidence.consumed()), (2, 2));
    }
    drop(terminal);
    let released = account.snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
}

#[test]
fn staged_animation_descriptors_borrow_outer_payloads_before_complete_submission() {
    let (mut owner, account) = start_store_owner_with_account();
    let issuer = DependencyOriginIssuer::new();
    let (times, values) = with_execution(&mut owner, |_, storage, control, vocabulary| {
        let mut times = SessionVec::new(&issuer);
        times.reserve_capacity(&issuer, storage, control, vocabulary, 2);
        times
            .values
            .try_extend_from_slice_reserved(&[0.0_f64, 1.0])
            .expect("sample times must use admitted payload storage");
        let mut values = SessionVec::new(&issuer);
        values.reserve_capacity(&issuer, storage, control, vocabulary, 6);
        values
            .values
            .try_extend_from_slice_reserved(&[0.0_f32, 0.0, 0.0, 1.0, 2.0, 3.0])
            .expect("translation samples must use admitted payload storage");
        (times, values)
    });

    let (clips, channels) = with_execution(&mut owner, |_, storage, control, vocabulary| {
        let mut channels = SessionVec::new(&issuer);
        channels.reserve_capacity(&issuer, storage, control, vocabulary, 1);
        channels
            .values
            .push_reserved_infallible(AnimationChannelAuthoring::new(
                0,
                AnimationTargetAuthoring::NodeTranslation(NodeLocalId::new(0)),
                AnimationInterpolation::Linear,
                AnimationExtrapolation::Constant,
                AnimationExtrapolation::Constant,
                &times.values,
                &values.values,
                Presence::Absent,
                Presence::Absent,
            ));
        let mut clips = SessionVec::new(&issuer);
        clips.reserve_capacity(&issuer, storage, control, vocabulary, 1);
        clips
            .values
            .push_reserved_infallible(AnimationClipAuthoring::new(
                0,
                1.0,
                AnimationTimeDomain::Seconds,
                AuthoringRange::new(0, 1),
            ));
        (clips, channels)
    });

    // The batch borrows outer descriptor owners, which in turn borrow outer retained samples.
    let animations = AnimationAuthoringBatch::new(&clips.values, &channels.values);
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )];
    let provenance = [ProvenanceLocalId::new(0)];
    let operation_kind = OperationKind::Generated;
    let provenance_nodes = [ProvenanceNodeAuthoring::operation(
        ProvenanceOperationAuthoring::new(
            &operation_kind,
            OptionsDigest::sha256([0; 32]),
            &[],
            &[],
        ),
    )];
    let owner_provenance = [
        ProfileOwner::Scene,
        ProfileOwner::SyntheticRoot,
        ProfileOwner::Node(NodeLocalId::new(0)),
        ProfileOwner::AnimationClip(0),
        ProfileOwner::AnimationChannel(0),
    ]
    .map(|owner| OwnerProvenanceAuthoring::new(owner, &provenance));
    let batch = SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
        DeformationAuthoringBatch::empty(),
        animations,
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&[], &[], &provenance_nodes, &owner_provenance),
            MetadataAuthoringBatch::empty(),
        ),
    );
    let construction = SceneConstruction::try_new(batch, SceneValidationLimits::default())
        .expect("the staged buffers must form an actual complete Scene construction");
    let draft = owner
        .submit_scene(construction)
        .expect("complete borrowed input must submit after both execution regions end");
    drop((clips, channels));
    drop((times, values));
    let terminal = owner
        .finish_scene(Some(draft))
        .expect("borrowed submission must finish after all authoring storage is released");
    let ImportStoreTerminalOutcome::Success { scene, .. } = &terminal else {
        panic!("the actual complete authoring batch must publish a canonical Scene")
    };
    let clip = scene
        .animation_clips()
        .next()
        .expect("the submitted animation must survive descriptor and payload destruction");
    let channel = clip
        .channel(0)
        .expect("the actual channel must be retained");
    assert_eq!(channel.key_times(), &[0.0, 1.0]);
    assert_eq!(channel.values(), &[0.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
    drop(terminal);
    let released = account.snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
}
