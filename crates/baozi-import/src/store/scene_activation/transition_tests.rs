use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use baozi_core::SceneValidationLimits;
use baozi_operation::{
    AccountPoisonReason, CancellationSignal, PhysicalStorageAccount, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
};

use super::super::{ImportStoreBootstrapRequirements, try_import_store_bootstrap};
use super::*;
use crate::authoring::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AuthoredTransform,
    DeformationAuthoringBatch, GeometryAuthoringBatch, LineageAuthoringBatch,
    MetadataAuthoringBatch, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent,
    OperationKind, OptionsDigest, OwnerProvenanceAuthoring, Presence, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ProvenanceOperationAuthoring,
    SceneAuthoringBatch, SceneSpace, SpatialAuthoringBatch,
};
use crate::ledger::ImportOperationControl;
use crate::limits::ResourceLimits;

fn start_session(
    control: ImportOperationControl,
) -> (ImportStoreSessionOwner, PhysicalStorageAccount) {
    let limits = ResourceLimits::default();
    let requirements = ImportStoreBootstrapRequirements::for_node_scene(&limits)
        .expect("the node Scene requirements must be valid");
    let bootstrap = try_import_store_bootstrap(&limits, requirements)
        .expect("the transition fixture must have a valid bootstrap");
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1)
            .expect("the test factory limits must be valid"),
    )
    .expect("the test account factory must allocate");
    let account = factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(u64::MAX, u64::MAX)
                .expect("the test account limits must be valid"),
        )
        .expect("the test physical account must allocate");
    let session = ImportStoreSessionOwner::try_start(control, &account, bootstrap)
        .expect("the transition fixture must start");
    (session, account)
}

fn with_two_node_construction<R>(
    action: impl for<'input> FnOnce(SceneConstruction<'input>) -> R,
) -> R {
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
    let owners = [
        OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &provenance),
        OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &provenance),
        OwnerProvenanceAuthoring::new(ProfileOwner::Node(NodeLocalId::new(0)), &provenance),
        OwnerProvenanceAuthoring::new(ProfileOwner::Node(NodeLocalId::new(1)), &provenance),
    ];
    let roots = [NodeLocalId::new(0)];
    let nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            Presence::Present(NodeParent::new(NodeLocalId::new(0), 0)),
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        ),
    ];
    let input = SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&SceneSpace::UNKNOWN, &[], &[]),
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
        DeformationAuthoringBatch::empty(),
        AnimationAuthoringBatch::empty(),
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&[], &[], &provenance_nodes, &owners),
            MetadataAuthoringBatch::empty(),
        ),
    );
    let construction = SceneConstruction::try_new(input, SceneValidationLimits::default())
        .expect("the complete node batch must admit construction");
    action(construction)
}

fn assert_transitioning_slot_available(state: &RefCell<ImportStoreSessionState>) {
    let slot = state
        .try_borrow_mut()
        .expect("the transition must not retain a state-slot guard across a callback");
    assert!(matches!(*slot, ImportStoreSessionState::Transitioning));
}

struct CaptureDropProbe<'probe> {
    state: &'probe RefCell<ImportStoreSessionState>,
    drops: &'probe Cell<usize>,
}

impl Drop for CaptureDropProbe<'_> {
    fn drop(&mut self) {
        assert_transitioning_slot_available(self.state);
        self.drops.set(self.drops.get() + 1);
    }
}

fn candidate_scene(
    session: &ImportStoreSessionOwner,
) -> (ImportStoreActivationTransition<'_>, ImportPendingScene) {
    let (owner, transition) = ImportStoreActivationTransition::begin(&session.state)
        .expect("the dormant owner must enter its single activation transition");
    let capture_drops = Cell::new(0);
    let evidence_probe = CaptureDropProbe {
        state: &session.state,
        drops: &capture_drops,
    };
    let validation_probe = CaptureDropProbe {
        state: &session.state,
        drops: &capture_drops,
    };
    let pending = with_two_node_construction(|construction| {
        let (outcome, postflight) =
            activate_scene(owner, construction).expect("the complete node Scene must activate");
        let CanonicalActivationOutcome::Activated(activated) = outcome else {
            panic!("the fixture must retain a materialized canonical graph");
        };
        let evidence = postflight.candidate_evidence(activated.output());
        let central = activated
            .validate_centrally(
                CanonicalValidationProgress::new(
                    ImportProgressPhase::Validate,
                    ImportProgressUnit::WorkUnits,
                ),
                move |lane, _output| {
                    let _capture = &evidence_probe;
                    assert_transitioning_slot_available(&session.state);
                    match lane.try_retain_records(
                        evidence.record_plan(),
                        SceneOutcomeEvidenceTag,
                        |writer| writer.write_row(|row| evidence.write_row(row)),
                    ) {
                        Ok(root) => lane.complete(root),
                        Err(_) => lane.abort(),
                    }
                },
                move |planning, output| {
                    let _capture = &validation_probe;
                    assert_transitioning_slot_available(&session.state);
                    postflight.validate_candidate(planning, output)
                },
            )
            .expect("the transition fixture must complete central validation");
        let CanonicalCentralValidationOutcome::Candidate(owner) = central else {
            panic!("the fixture must retain its validated candidate");
        };
        ImportPendingScene::Candidate { owner, evidence }
    });
    assert_eq!(capture_drops.get(), 2);
    (transition, pending)
}

#[test]
fn activation_callbacks_and_capture_drop_can_borrow_the_transitioning_slot() {
    let (session, account) = start_session(ImportOperationControl::default());
    let (owner, transition) = ImportStoreActivationTransition::begin(&session.state)
        .expect("the dormant owner must enter activation");
    let capture_drops = Cell::new(0);
    let probe = CaptureDropProbe {
        state: &session.state,
        drops: &capture_drops,
    };
    let state = &session.state;
    let host = session.host();
    let outcome = owner
        .plan_and_activate_scoped::<
            SceneActivationOutput,
            SceneActivationRejection,
            RecordBlock,
            SceneCanonicalRootTag,
        >(move |planning| {
            let _capture = &probe;
            assert_transitioning_slot_available(state);
            assert!(matches!(
                ImportStoreActivationTransition::begin(state),
                Err(ImportStoreHostError::OperationConsumed)
            ));
            assert!(matches!(
                host.poll(),
                Err(ImportStoreHostError::OperationConsumed)
            ));
            planning.reject(
                SceneActivationRejection::construction_invariant(),
                IMPORT_SCENE_NOT_SUBMITTED_CODE,
            )
        })
        .expect("rejecting the scoped request must retain its exact owner");
    assert_eq!(capture_drops.get(), 1);
    assert_transitioning_slot_available(state);
    let CanonicalActivationOutcome::NeverActivated(owner) = outcome else {
        panic!("an explicit rejection must never activate the Scene");
    };
    transition
        .install(ImportPendingScene::NeverActivated {
            owner,
            evidence: session.unsubmitted_evidence,
        })
        .expect("rejected reentry must not replace the outer transition");
    assert!(matches!(
        ImportStoreActivationTransition::begin(state),
        Err(ImportStoreHostError::OperationConsumed)
    ));
    assert!(!account.snapshot().poisoned());
}

thread_local! {
    static CONTROL_SESSION: RefCell<Option<Rc<ImportStoreSessionOwner>>> = const { RefCell::new(None) };
    static CONTROL_POLLS: Cell<usize> = const { Cell::new(0) };
}

struct SlotCheckingCancellation;

impl CancellationSignal for SlotCheckingCancellation {
    fn is_cancelled(&self) -> bool {
        CONTROL_SESSION.with(|session| {
            if let Some(session) = session.borrow().as_ref() {
                assert_transitioning_slot_available(&session.state);
                CONTROL_POLLS.with(|polls| polls.set(polls.get() + 1));
            }
        });
        false
    }
}

#[test]
fn control_and_central_validation_callbacks_leave_the_state_slot_available() {
    let (session, account) = start_session(
        ImportOperationControl::default().with_cancellation(SlotCheckingCancellation),
    );
    let session = Rc::new(session);
    CONTROL_POLLS.with(|polls| polls.set(0));
    CONTROL_SESSION.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&session)));

    let (transition, pending) = candidate_scene(&session);
    CONTROL_POLLS.with(|polls| assert!(polls.get() > 0));
    transition
        .install(pending)
        .expect("the validated candidate must install once");
    let probe_owner = CONTROL_SESSION.with(|slot| slot.borrow_mut().take());
    drop(probe_owner);

    assert_eq!(
        session
            .completed_scene_boundary()
            .expect("the slot must be readable"),
        Some(ImportStoreSceneBoundary::Validated)
    );
    assert!(!account.snapshot().poisoned());
}

#[test]
fn begin_borrow_conflict_does_not_extract_or_poison_the_dormant_owner() {
    let (session, account) = start_session(ImportOperationControl::default());
    let slot = session.state.borrow_mut();
    assert!(matches!(
        ImportStoreActivationTransition::begin(&session.state),
        Err(ImportStoreHostError::ReentrantBorrow)
    ));
    assert!(matches!(*slot, ImportStoreSessionState::Dormant(_)));
    assert!(!account.snapshot().poisoned());
}

#[test]
fn install_borrow_conflict_retains_the_complete_candidate_graph() {
    let (session, account) = start_session(ImportOperationControl::default());
    let (transition, pending) = candidate_scene(&session);
    let before = account.snapshot();
    assert!(before.live_retained_bytes() > 0);
    assert!(before.live_retained_allocations() > 0);
    let slot = session.state.borrow_mut();

    assert!(matches!(
        transition.install(pending),
        Err(ImportStoreActivationError::Containment(_))
    ));
    assert!(matches!(*slot, ImportStoreSessionState::Transitioning));
    let contained = account.snapshot();
    assert!(contained.poisoned());
    assert_eq!(
        contained.live_retained_bytes(),
        before.live_retained_bytes()
    );
    assert_eq!(
        contained.live_retained_allocations(),
        before.live_retained_allocations()
    );
    drop(slot);
    assert!(matches!(
        session.prepare_scene(None),
        Err(ImportStoreActivationError::StoreUnavailable(
            ImportStoreHostError::OperationContained
        ))
    ));
}

#[test]
fn poisoned_install_retains_the_candidate_and_the_first_containment() {
    let (session, account) = start_session(ImportOperationControl::default());
    let (transition, pending) = candidate_scene(&session);
    let first_cause = transition.containment.contain();
    let before = account.snapshot();

    let result = transition.install(pending);
    assert!(matches!(
        result,
        Err(ImportStoreActivationError::Containment(cause)) if cause == first_cause
    ));
    assert!(matches!(
        *session.state.borrow(),
        ImportStoreSessionState::Contained
    ));
    let contained = account.snapshot();
    assert_eq!(
        contained.first_poison_evidence(),
        before.first_poison_evidence()
    );
    assert_eq!(contained.poison_event_count(), before.poison_event_count());
    assert_eq!(
        contained.live_retained_bytes(),
        before.live_retained_bytes()
    );
    assert_eq!(
        contained.live_retained_allocations(),
        before.live_retained_allocations()
    );
}

#[test]
fn install_rejects_a_foreign_owner_without_overwriting_or_dropping_it() {
    let (session, account) = start_session(ImportOperationControl::default());
    let (transition, pending) = candidate_scene(&session);
    let before = account.snapshot();
    let (foreign, foreign_account) = start_session(ImportOperationControl::default());
    let foreign_state = foreign.into_terminal_state().0;
    let foreign_before = foreign_account.snapshot();
    let displaced = {
        let mut slot = session.state.borrow_mut();
        std::mem::replace(&mut *slot, foreign_state)
    };
    assert!(matches!(displaced, ImportStoreSessionState::Transitioning));
    drop(displaced);

    assert!(matches!(
        transition.install(pending),
        Err(ImportStoreActivationError::Containment(_))
    ));
    assert!(matches!(
        *session.state.borrow(),
        ImportStoreSessionState::Dormant(_)
    ));
    let foreign_after = foreign_account.snapshot();
    assert_eq!(foreign_after, foreign_before);
    let contained = account.snapshot();
    assert!(contained.poisoned());
    assert_eq!(
        contained.live_retained_bytes(),
        before.live_retained_bytes()
    );
    assert_eq!(
        contained.live_retained_allocations(),
        before.live_retained_allocations()
    );
}

#[test]
fn guard_unwind_with_a_borrowed_slot_latches_containment_without_resetting_state() {
    let (session, account) = start_session(ImportOperationControl::default());
    let (owner, transition) = ImportStoreActivationTransition::begin(&session.state)
        .expect("the dormant owner must enter activation");
    let before = account.snapshot();
    let slot = session.state.borrow_mut();
    let unwound = catch_unwind(AssertUnwindSafe(move || {
        let _transition = transition;
        panic!("leave an armed activation transition during host unwind");
    }));

    assert!(unwound.is_err());
    assert!(matches!(*slot, ImportStoreSessionState::Transitioning));
    let contained = account.snapshot();
    assert!(contained.poisoned());
    assert_eq!(
        contained
            .first_poison_evidence()
            .map(|evidence| evidence.reason()),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );
    assert_eq!(
        contained.live_retained_bytes(),
        before.live_retained_bytes()
    );
    assert_eq!(
        contained.live_retained_allocations(),
        before.live_retained_allocations()
    );
    drop(slot);
    drop(owner);
    assert!(matches!(
        session.prepare_scene(None),
        Err(ImportStoreActivationError::StoreUnavailable(
            ImportStoreHostError::OperationContained
        ))
    ));
}

#[test]
fn install_during_unwind_with_a_borrowed_slot_retains_the_candidate() {
    struct InstallOnUnwind<'session> {
        pending: Option<(
            ImportStoreActivationTransition<'session>,
            ImportPendingScene,
        )>,
        contained: &'session Cell<bool>,
    }

    impl Drop for InstallOnUnwind<'_> {
        fn drop(&mut self) {
            if let Some((transition, pending)) = self.pending.take() {
                self.contained.set(matches!(
                    transition.install(pending),
                    Err(ImportStoreActivationError::Containment(_))
                ));
            }
        }
    }

    let (session, account) = start_session(ImportOperationControl::default());
    let (transition, pending) = candidate_scene(&session);
    let before = account.snapshot();
    let contained = Cell::new(false);
    let cleanup = InstallOnUnwind {
        pending: Some((transition, pending)),
        contained: &contained,
    };
    let slot = session.state.borrow_mut();
    let unwound = catch_unwind(AssertUnwindSafe(move || {
        let _cleanup = cleanup;
        panic!("install the activation result from a host unwind cleanup");
    }));

    assert!(unwound.is_err());
    assert!(contained.get());
    assert!(matches!(*slot, ImportStoreSessionState::Transitioning));
    let after = account.snapshot();
    assert!(after.poisoned());
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations()
    );
}
