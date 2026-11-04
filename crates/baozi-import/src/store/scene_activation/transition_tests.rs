use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use baozi_core::SceneValidationLimits;
use baozi_operation::{
    AccountPoisonReason, CancellationSignal, CancellationToken, PhysicalStorageAccount,
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits,
};

use super::super::tests::run_scene_body;
use super::super::{
    ImportStoreBodyParts, ImportStoreBootstrapRequirements, ImportStoreInvocationRoot,
    try_import_store_bootstrap,
};
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

fn start_session<'inv>(
    parts: ImportStoreBodyParts<'inv>,
    control: ImportOperationControl,
) -> (ImportStoreSessionOwner<'inv>, PhysicalStorageAccount) {
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
    let session = ImportStoreSessionOwner::try_start_in_body(parts, control, &account, bootstrap)
        .expect("the transition fixture must start");
    (session, account)
}

#[test]
fn a_late_terminal_cancellation_preserves_the_already_selected_ledger_failure() {
    let cancellation = CancellationToken::new();
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let (prepared, account) = run_scene_body(
        &mut invocation_root,
        ResourceLimits::default(),
        ImportOperationControl::default().with_cancellation(cancellation.clone()),
        SceneParseOutcome::Rejected,
        |session, _account| {
            session
                .with_root_execution(|execution| {
                    execution
                        .tighten_dimension_policy(
                            crate::ImportDimension::Records,
                            baozi_operation::DimensionPolicy::counter(0, 0, 0),
                        )
                        .unwrap();
                    assert!(matches!(
                        execution.charge_attempted(crate::ImportDimension::Records, 1),
                        Err(ImportStoreHostError::Ledger(
                            baozi_operation::LedgerCause::LimitExceeded {
                                dimension: crate::ImportDimension::Records,
                                ..
                            }
                        ))
                    ));
                    Ok::<_, ImportStoreHostError>(())
                })
                .unwrap();
            cancellation.cancel();
        },
    );
    let terminal = prepared.unwrap().handoff();
    assert_eq!(
        terminal.evidence().cause().class(),
        ImportStoreTerminalClass::Ledger
    );
    assert!(matches!(
        terminal.control_cause(),
        Some(ControlCause::Cancelled {
            origin: baozi_operation::CancellationOrigin::Signal
        })
    ));
    let records = terminal
        .evidence()
        .dimension(crate::ImportDimension::Records)
        .unwrap();
    assert_eq!((records.attempted(), records.consumed()), (1, 0));
    assert_eq!(terminal.canonical_block_count(), None);
    drop(terminal);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert!(!account.snapshot().poisoned());
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

fn assert_account_accessible(account: &PhysicalStorageAccount) {
    let snapshot = account.snapshot();
    assert!(!snapshot.poisoned());
    assert!(snapshot.live_retained_bytes() > 0);
    assert!(snapshot.live_retained_allocations() > 0);
}

struct CaptureDropProbe<'probe> {
    account: &'probe PhysicalStorageAccount,
    drops: &'probe Cell<usize>,
}

impl Drop for CaptureDropProbe<'_> {
    fn drop(&mut self) {
        assert_account_accessible(self.account);
        self.drops.set(self.drops.get() + 1);
    }
}

fn candidate_scene<'session, 'inv>(
    session: &'session mut ImportStoreSessionOwner<'inv>,
    account: &PhysicalStorageAccount,
) -> (
    ImportStoreActivationTransition<'session, 'inv>,
    ImportPendingScene<'inv>,
) {
    let data = &mut session.storage[0];
    let (owner, transition) = ImportStoreActivationTransition::begin(&mut data.state)
        .expect("the dormant owner must enter its single activation transition");
    let slots = data
        .canonical_slots
        .take()
        .expect("the dormant session must retain its host payload slots");
    let capture_drops = Cell::new(0);
    let evidence_probe = CaptureDropProbe {
        account,
        drops: &capture_drops,
    };
    let validation_probe = CaptureDropProbe {
        account,
        drops: &capture_drops,
    };
    let pending = with_two_node_construction(|construction| {
        let (outcome, postflight) =
            activate_scene(owner, slots.output, slots.activation, construction)
                .expect("the complete node Scene must activate");
        let CanonicalActivationOutcome::Activated(activated) = outcome else {
            panic!("the fixture must retain a materialized canonical graph");
        };
        let evidence = postflight.candidate_evidence(activated.output());
        let central = activated
            .validate_centrally(
                slots.proof,
                slots.validation,
                CanonicalValidationProgress::new(
                    ImportProgressPhase::Validate,
                    ImportProgressUnit::WorkUnits,
                ),
                move |lane, _output| {
                    let _capture = &evidence_probe;
                    assert_account_accessible(account);
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
                    assert_account_accessible(account);
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
fn activation_callbacks_and_capture_drop_can_access_the_physical_account() {
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let (prepared, account) = run_scene_body(
        &mut invocation_root,
        ResourceLimits::default(),
        ImportOperationControl::default(),
        SceneParseOutcome::Rejected,
        |session, account| {
            let evidence = session.unsubmitted_evidence;
            let data = &mut session.storage[0];
            let (owner, transition) = ImportStoreActivationTransition::begin(&mut data.state)
                .expect("the dormant owner must enter activation");
            let slots = data
                .canonical_slots
                .take()
                .expect("the dormant session must retain its host payload slots");
            let capture_drops = Cell::new(0);
            let probe = CaptureDropProbe {
                account,
                drops: &capture_drops,
            };
            let callback_account = &account;
            let outcome = owner
        .plan_and_activate_scoped::<
            SceneActivationOutput,
            SceneActivationRejection,
            RecordBlock,
            SceneCanonicalRootTag,
        >(slots.output, slots.activation, move |planning| {
            let _capture = &probe;
            assert_account_accessible(callback_account);
            planning.reject(
                SceneActivationRejection::construction_invariant(),
                IMPORT_SCENE_NOT_SUBMITTED_CODE,
            )
        })
        .expect("rejecting the scoped request must retain its exact owner");
            assert_eq!(capture_drops.get(), 1);
            let CanonicalActivationOutcome::NeverActivated(owner) = outcome else {
                panic!("an explicit rejection must never activate the Scene");
            };
            transition
                .install(ImportPendingScene::NeverActivated { owner, evidence })
                .expect("the scoped rejection must install its exact pending owner");
            assert!(matches!(
                ImportStoreActivationTransition::begin(&mut session.storage[0].state),
                Err(ImportStoreHostError::OperationConsumed)
            ));
            assert!(!account.snapshot().poisoned());
        },
    );
    let terminal = prepared.unwrap().handoff();
    assert!(!terminal.is_success());
    drop(terminal);
    let released = account.snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
}

#[derive(Clone, Default)]
struct AccountCheckingCancellation {
    account: Arc<OnceLock<PhysicalStorageAccount>>,
    polls: Arc<AtomicUsize>,
    cancelled: Arc<AtomicBool>,
}

impl CancellationSignal for AccountCheckingCancellation {
    fn is_cancelled(&self) -> bool {
        if let Some(account) = self.account.get() {
            assert_account_accessible(account);
            self.polls.fetch_add(1, Ordering::AcqRel);
        }
        self.cancelled.load(Ordering::Acquire)
    }
}

fn run_account_checking_body(
    root: &mut ImportStoreInvocationRoot,
    cancellation: &AccountCheckingCancellation,
    outcome: SceneParseOutcome,
    mut body: impl for<'call> FnMut(&mut ImportStoreSessionOwner<'call>, &PhysicalStorageAccount),
) -> ImportStorePreparedScene {
    let limits = ResourceLimits::default();
    let requirements = ImportStoreBootstrapRequirements::for_node_scene(&limits).unwrap();
    let bootstrap = try_import_store_bootstrap(&limits, requirements).unwrap();
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, u64::MAX).unwrap())
        .unwrap();
    assert!(cancellation.account.set(account).is_ok());
    let account = cancellation.account.get().unwrap();
    let mut startup = Some((
        ImportOperationControl::default().with_cancellation(cancellation.clone()),
        bootstrap,
        outcome,
    ));
    let mut request = None;
    let requested = &mut request;
    let finish = root
        .run_body(move |parts, disposition| {
            let (control, bootstrap, outcome) = startup.take().unwrap();
            let mut owner =
                ImportStoreSessionOwner::try_start_in_body(parts, control, account, bootstrap)
                    .unwrap();
            body(&mut owner, account);
            *requested = Some(owner.request_finish(disposition, outcome));
        })
        .unwrap();
    prepare_scene(finish, request.unwrap().unwrap()).unwrap()
}

#[test]
fn control_and_central_validation_callbacks_can_access_the_physical_account() {
    let cancellation = AccountCheckingCancellation::default();
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let prepared = run_account_checking_body(
        &mut invocation_root,
        &cancellation,
        SceneParseOutcome::Accepted,
        |session, account| {
            let (transition, pending) = candidate_scene(session, account);
            assert!(cancellation.polls.load(Ordering::Acquire) > 0);
            transition
                .install(pending)
                .expect("the validated candidate must install once");
            assert_eq!(
                session
                    .completed_scene_boundary()
                    .expect("the completed boundary must remain observable"),
                Some(ImportStoreSceneBoundary::Validated)
            );
            assert!(!account.snapshot().poisoned());
        },
    );
    let terminal = prepared.handoff();
    assert!(terminal.is_success());
    drop(terminal);
    let released = cancellation.account.get().unwrap().snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
}

#[test]
fn cancellation_during_activation_preserves_control_failure_and_releases_storage() {
    let cancellation = AccountCheckingCancellation::default();
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let prepared = run_account_checking_body(
        &mut invocation_root,
        &cancellation,
        SceneParseOutcome::Rejected,
        |session, _account| {
            cancellation.cancelled.store(true, Ordering::Release);

            with_two_node_construction(|construction| {
                assert!(matches!(
                    session.submit_scene(construction),
                    Err(ImportStoreActivationError::Control(
                        ControlCause::Cancelled {
                            origin: baozi_operation::CancellationOrigin::Signal,
                        }
                    ))
                ));
            });
            assert!(cancellation.polls.load(Ordering::Acquire) > 0);
        },
    );
    let terminal = prepared.handoff();
    assert!(matches!(
        terminal.control_cause(),
        Some(ControlCause::Cancelled {
            origin: baozi_operation::CancellationOrigin::Signal,
        })
    ));
    assert_eq!(
        terminal.evidence().cause().class(),
        ImportStoreTerminalClass::Control
    );
    assert_eq!(terminal.canonical_block_count(), None);
    drop(terminal);
    let released = cancellation.account.get().unwrap().snapshot();
    assert_eq!(released.live_retained_bytes(), 0);
    assert_eq!(released.live_retained_allocations(), 0);
    assert!(!released.poisoned());
}

#[test]
fn poisoned_install_retains_the_candidate_and_the_first_containment() {
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let (prepared, _account) = run_scene_body(
        &mut invocation_root,
        ResourceLimits::default(),
        ImportOperationControl::default(),
        SceneParseOutcome::Rejected,
        |session, account| {
            let (transition, pending) = candidate_scene(session, account);
            let first_cause = transition.containment.contain();
            let before = account.snapshot();

            let result = transition.install(pending);
            assert!(matches!(
                result,
                Err(ImportStoreActivationError::Containment(cause)) if cause == first_cause
            ));
            assert!(matches!(session.state, ImportStoreSessionState::Contained));
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
        },
    );
    assert!(matches!(
        prepared,
        Err(ImportStoreActivationError::StoreUnavailable(
            ImportStoreHostError::OperationContained
        ))
    ));
}

#[test]
fn poisoned_install_contains_a_foreign_pending_owner_without_dropping_either_graph() {
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let mut foreign_invocation_root = ImportStoreInvocationRoot::new();
    let body = invocation_root.run_body(|parts, _disposition| {
        let mut started = Some(start_session(parts, ImportOperationControl::default()));
        let foreign_body =
            foreign_invocation_root.run_body(|foreign_parts, _foreign_disposition| {
                let (mut session, account) = started.take().unwrap();
                let session_cell_bytes = session.storage.capacity_bytes();
                let (transition, pending) = candidate_scene(&mut session, &account);
                let (mut foreign, foreign_account) =
                    start_session(foreign_parts, ImportOperationControl::default());
                let foreign_cell_bytes = foreign.storage.capacity_bytes();
                let (foreign_transition, foreign_pending) =
                    candidate_scene(&mut foreign, &foreign_account);
                let first_cause = transition.containment.retain(pending);
                let before = account.snapshot();
                let foreign_before = foreign_account.snapshot();
                assert!(before.live_retained_bytes() > 0);
                assert!(before.live_retained_allocations() > 0);
                assert!(foreign_before.live_retained_bytes() > 0);
                assert!(foreign_before.live_retained_allocations() > 0);
                assert!(!foreign_before.poisoned());

                assert!(matches!(
                    transition.install(foreign_pending),
                    Err(ImportStoreActivationError::Containment(cause)) if cause == first_cause
                ));
                assert!(matches!(session.state, ImportStoreSessionState::Contained));
                assert_eq!(account.snapshot(), before);
                assert_eq!(foreign_account.snapshot(), foreign_before);

                drop(foreign_transition);
                assert!(matches!(foreign.state, ImportStoreSessionState::Contained));
                let foreign_after = foreign_account.snapshot();
                assert!(foreign_after.poisoned());
                assert_eq!(
                    foreign_after.live_retained_bytes(),
                    foreign_before.live_retained_bytes()
                );
                assert_eq!(
                    foreign_after.live_retained_allocations(),
                    foreign_before.live_retained_allocations()
                );
                for owner in [session, foreign] {
                    assert!(matches!(
                        owner.into_body_state(),
                        Err(ImportStoreHostError::OperationContained)
                    ));
                }
                // Consuming the session destroys its known startup cell. Both uncertain candidate graphs
                // remain contained; their physical charges must not be refunded with that independent cell.
                assert_eq!(
                    account.snapshot().live_retained_bytes(),
                    before.live_retained_bytes() - session_cell_bytes
                );
                assert_eq!(
                    account.snapshot().live_retained_allocations(),
                    before.live_retained_allocations() - u64::from(session_cell_bytes != 0)
                );
                assert_eq!(
                    foreign_account.snapshot().live_retained_bytes(),
                    foreign_before.live_retained_bytes() - foreign_cell_bytes
                );
                assert_eq!(
                    foreign_account.snapshot().live_retained_allocations(),
                    foreign_before.live_retained_allocations() - u64::from(foreign_cell_bytes != 0)
                );
            });
        assert!(
            foreign_body.is_err(),
            "a contained foreign owner cannot record a disposition"
        );
    });
    assert!(
        body.is_err(),
        "a contained session cannot record a disposition"
    );
}

#[test]
fn armed_transition_drop_on_unwind_latches_physical_containment() {
    let mut invocation_root = ImportStoreInvocationRoot::new();
    let (prepared, _account) = run_scene_body(
        &mut invocation_root,
        ResourceLimits::default(),
        ImportOperationControl::default(),
        SceneParseOutcome::Rejected,
        |session, account| {
            let (owner, transition) =
                ImportStoreActivationTransition::begin(&mut session.storage[0].state)
                    .expect("the dormant owner must enter activation");
            let before = account.snapshot();
            let unwound = catch_unwind(AssertUnwindSafe(move || {
                let _transition = transition;
                panic!("leave an armed activation transition during host unwind");
            }));

            assert!(unwound.is_err());
            assert!(matches!(session.state, ImportStoreSessionState::Contained));
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
            drop(owner);
        },
    );
    assert!(matches!(
        prepared,
        Err(ImportStoreActivationError::StoreUnavailable(
            ImportStoreHostError::OperationContained
        ))
    ));
}

#[test]
fn poisoned_install_during_unwind_retains_the_candidate() {
    struct InstallOnUnwind<'session, 'inv> {
        pending: Option<(
            ImportStoreActivationTransition<'session, 'inv>,
            ImportPendingScene<'inv>,
        )>,
        contained: &'session Cell<bool>,
    }

    impl Drop for InstallOnUnwind<'_, '_> {
        fn drop(&mut self) {
            if let Some((transition, pending)) = self.pending.take() {
                self.contained.set(matches!(
                    transition.install(pending),
                    Err(ImportStoreActivationError::Containment(_))
                ));
            }
        }
    }

    let mut invocation_root = ImportStoreInvocationRoot::new();
    let (prepared, _account) = run_scene_body(
        &mut invocation_root,
        ResourceLimits::default(),
        ImportOperationControl::default(),
        SceneParseOutcome::Rejected,
        |session, account| {
            let (transition, pending) = candidate_scene(session, account);
            let _first_cause = transition.containment.contain();
            let before = account.snapshot();
            let contained = Cell::new(false);
            let cleanup = InstallOnUnwind {
                pending: Some((transition, pending)),
                contained: &contained,
            };
            let unwound = catch_unwind(AssertUnwindSafe(move || {
                let _cleanup = cleanup;
                panic!("install the activation result from a host unwind cleanup");
            }));

            assert!(unwound.is_err());
            assert!(contained.get());
            assert!(matches!(session.state, ImportStoreSessionState::Contained));
            let after = account.snapshot();
            assert!(after.poisoned());
            assert_eq!(
                after.first_poison_evidence(),
                before.first_poison_evidence()
            );
            assert_eq!(after.poison_event_count(), before.poison_event_count());
            assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
            assert_eq!(
                after.live_retained_allocations(),
                before.live_retained_allocations()
            );
        },
    );
    assert!(matches!(
        prepared,
        Err(ImportStoreActivationError::StoreUnavailable(
            ImportStoreHostError::OperationContained
        ))
    ));
}
