use super::*;
use crate::concrete_store::{RetainedStoreSharePlan, RetainedStoreStartPlan, StoreInvocationRoot};
use crate::{
    DimensionPolicy, LedgerCause, LedgerLimits, OperationControl, PhysicalStorageAccount,
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits, StoreDimensionSemanticId, StoreFallbackEnvelope, StoreLedgerPlan,
    StorePhaseSemanticId, StoreSemanticCatalogId,
};
use std::num::NonZeroU32;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Owner;
impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Progress;
impl ProgressPhase for Progress {
    fn order(self) -> u32 {
        0
    }
    fn stable_id(self) -> &'static str {
        "build"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Unit;
impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        "items"
    }
}

type InvocationRoot = StoreInvocationRoot<Owner, Progress, Unit>;
type InvocationHost = CanonicalInvocationHost<
    Owner,
    Progress,
    Unit,
    (),
    (),
    (),
    (),
    (),
    ScalarSequence<u8>,
    Tag,
    ScalarSequence<u8>,
    Tag,
>;
type Operation<'inv> = RetainedStoreOperation<'inv, Owner, Progress, Unit>;
type BodyOperation<'inv> = CanonicalBodyOperation<'inv, Owner, Progress, Unit>;

#[derive(Clone, Copy)]
struct Keys {
    phase: StorePhaseKey,
    work: StoreDimensionKey,
    storage: StoreDimensionKey,
    terminal: StoreDimensionKey,
}

impl Keys {
    fn writer(self, members: usize) -> RetainedStoreWriterPlan {
        RetainedStoreWriterPlan::new(
            self.phase,
            self.storage,
            self.storage,
            RetainedStoreBatchLimits::new(members + 1, 4).unwrap(),
        )
    }

    fn session<'inv>(
        self,
        operation: Operation<'inv>,
    ) -> RetainedStoreSessionOutcomeOwner<'inv, Owner, Progress, Unit> {
        operation
            .into_session_outcome_owner(
                CanonicalActivationSessionPolicy::new(
                    RetainedStoreWriterRole::new(self.phase, self.storage, self.storage),
                    RetainedStoreWriterRole::new(self.phase, self.storage, self.storage),
                    CanonicalMaterializationRole::new(self.work, self.storage),
                    CanonicalValidationRole::new(self.phase, self.work, self.storage),
                    self.storage,
                    self.storage,
                ),
                RetainedStoreBatchLimits::new(5, 4).unwrap(),
                CanonicalFailureRequirements::new(CanonicalTableShape::new(4, 4).unwrap()),
            )
            .unwrap()
    }

    fn body_session<'inv>(
        self,
        operation: BodyOperation<'inv>,
    ) -> RetainedStoreSessionOutcomeOwner<'inv, Owner, Progress, Unit> {
        operation
            .into_session_outcome_owner(
                CanonicalActivationSessionPolicy::new(
                    RetainedStoreWriterRole::new(self.phase, self.storage, self.storage),
                    RetainedStoreWriterRole::new(self.phase, self.storage, self.storage),
                    CanonicalMaterializationRole::new(self.work, self.storage),
                    CanonicalValidationRole::new(self.phase, self.work, self.storage),
                    self.storage,
                    self.storage,
                ),
                RetainedStoreBatchLimits::new(5, 4).unwrap(),
                CanonicalFailureRequirements::new(CanonicalTableShape::new(4, 4).unwrap()),
            )
            .unwrap()
    }
}

fn operation_plan(
    total_attempted: u64,
    total_consumed: u64,
) -> (
    StoreLedgerPlan,
    RetainedStoreStartPlan,
    Keys,
    PhysicalStorageAccount,
) {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(1 << 20, 1).unwrap(),
    )
    .unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(1 << 20, 128).unwrap())
        .unwrap();
    let catalog = StoreSemanticCatalogId::new(*b"terminal-qualify", NonZeroU32::MIN);
    let domain = StoreKeyDomain::try_new(catalog).unwrap();
    let dimension = |local| {
        domain
            .try_admit_dimension(StoreDimensionSemanticId::new(
                catalog,
                NonZeroU64::new(local).unwrap(),
            ))
            .unwrap()
    };
    let keys = Keys {
        phase: domain
            .try_admit_phase(StorePhaseSemanticId::new(catalog, NonZeroU64::MIN))
            .unwrap(),
        work: dimension(1),
        storage: dimension(2),
        terminal: dimension(3),
    };
    let terminal = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
        keys.terminal,
        2,
        5,
        5,
        total_attempted,
        total_consumed,
    )
    .unwrap();
    let limits = LedgerLimits::new(
        1 << 20,
        [
            (
                keys.work,
                DimensionPolicy::counter(
                    terminal.ordinary_attempted_work_limit(),
                    terminal.ordinary_consumed_work_limit(),
                    0,
                ),
            ),
            (
                keys.storage,
                DimensionPolicy::reservable_bytes(0, 0, 1 << 20, 1 << 20)
                    .with_published_outcome_evidence_limit(1 << 20),
            ),
            (keys.terminal, terminal.requirement().dimension_policy()),
        ],
    )
    .unwrap();
    let ledger = StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).unwrap();
    let start = RetainedStoreStartPlan::new(
        keys.writer(4),
        RetainedStoreSharePlan::new(keys.work, keys.work),
        StoreFallbackEnvelope::new(7),
        keys.storage,
        1,
        keys.storage,
        1,
        terminal,
    );
    (ledger, start, keys, account)
}

fn operation<'inv>(
    root: impl Into<crate::concrete_store::StoreInvocationStart<'inv, Owner, Progress, Unit>>,
    total_attempted: u64,
    total_consumed: u64,
    control: OperationControl<Progress, Unit>,
) -> (Operation<'inv>, Keys, PhysicalStorageAccount) {
    let (ledger, start, keys, account) = operation_plan(total_attempted, total_consumed);
    let operation = Operation::try_start(root, control, ledger, &account, start).unwrap();
    (operation, keys, account)
}

fn body_operation<'inv>(
    root: CanonicalBodyStart<'inv, Owner, Progress, Unit>,
    total_attempted: u64,
    total_consumed: u64,
    control: OperationControl<Progress, Unit>,
) -> (BodyOperation<'inv>, Keys, PhysicalStorageAccount) {
    let (ledger, start, keys, account) = operation_plan(total_attempted, total_consumed);
    let operation = BodyOperation::try_start(root, control, ledger, &account, start).unwrap();
    (operation, keys, account)
}

fn counters<'inv>(operation: &mut Operation<'inv>, keys: Keys) -> (u64, u64) {
    let state = operation
        .producer_host()
        .dimension_state(keys.work)
        .unwrap();
    (state.attempted(), state.consumed())
}

#[derive(Clone, Copy)]
struct Tag;

#[test]
fn exact_transfer_preserves_work_and_qualifies_four_real_evidence_members() {
    let mut invocation_root = InvocationRoot::new();
    let (mut operation, keys, account) =
        operation(&mut invocation_root, 512, 500, OperationControl::default());
    {
        let mut parts = operation
            .raw
            .exclusive_parts()
            .unwrap_or_else(|_| panic!("the original operation lends its numeric work"));
        let layout = parts.work.bind([keys.work]).unwrap();
        parts.ordinary.poll.poll().unwrap();
        let mut frame = layout
            .admit_observed(&mut parts.work.work, [7], crate::ledger::WorkLane::Producer)
            .unwrap();
        frame
            .complete_batch_with_outcome(&layout, &mut parts.work.work, [4])
            .into_result()
            .unwrap();
        frame.resolve(&layout, &mut parts.work.work, false).unwrap();
    }
    let before = counters(&mut operation, keys);
    // Two record members (2 + 3 fields) and two scalar members (6 + 20 fields).
    operation
        .qualify_failure_validation_work(keys.work, 4, 31)
        .unwrap();
    assert_eq!(counters(&mut operation, keys), before);
    assert_eq!(
        operation.raw.failure_terminal_work_limits(),
        Some((4, 5, 31))
    );
    let RetainedStoreSessionOutcomeOwner {
        dormant,
        mut failure_work,
        ..
    } = keys.session(operation);
    assert_eq!(failure_work.remaining_members, 4);
    assert_eq!(failure_work.remaining_validation_work, 31);
    let operation = dormant.into_operation().unwrap();
    let plan = RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(keys.writer(1), 1, 1).unwrap(),
        RetainedStoreBlockTablePlan::new(keys.writer(4), 4, 4).unwrap(),
    );
    let mut assembly = operation.try_publication_assembly(plan).unwrap();
    let mut fault = None;
    {
        let mut lane = CanonicalOutcomeEvidenceLane {
            assembly: &mut assembly,
            first_fault: &mut fault,
            failure_work: &mut failure_work,
            work_dimension: keys.work,
        };
        let counts = lane
            .try_retain_records(RecordPlan::new(1, 2).unwrap(), Tag, |writer| {
                writer.write_row(|row| {
                    row.write_scalar(2_u32)?;
                    row.write_scalar(6_u32)
                })
            })
            .unwrap();
        let corners = lane
            .try_retain_sequence(SequencePlan::<u32>::new(6).unwrap(), Tag, |writer| {
                writer.extend_from_slice(&[0, 1, 2, 0, 2, 3])
            })
            .unwrap();
        let faces = lane
            .try_retain_sequence(SequencePlan::<u32>::new(20).unwrap(), Tag, |writer| {
                writer.extend_from_slice(&[0; 20])
            })
            .unwrap();
        let _root = lane
            .try_retain_records(RecordPlan::new(1, 3).unwrap(), Tag, |writer| {
                writer.write_row(|row| {
                    row.write_record_ref(counts)?;
                    row.write_sequence_ref(corners)?;
                    row.write_sequence_ref(faces)
                })
            })
            .unwrap();
    }
    assert!(fault.is_none());
    assert_eq!(failure_work.remaining_members, 0);
    assert_eq!(failure_work.remaining_validation_work, 0);
    assert_eq!(failure_work.planned_validation_work().unwrap().get(), 31);
    assert!(failure_work.with_existing_member_validation(1).is_err());
    let published = assembly.prepare_minimal_fallback_with(7).unwrap().handoff();
    let ordinary = published.ledger_evidence.dimension(keys.work).unwrap();
    let terminal = published.ledger_evidence.dimension(keys.terminal).unwrap();
    assert!(ordinary.attempted() >= before.0 + 31);
    assert!(ordinary.consumed() >= before.1 + 31);
    assert_eq!((terminal.attempted(), terminal.consumed()), (0, 0));
    assert!(ordinary.attempted() + terminal.attempted() <= 512);
    assert!(ordinary.consumed() + terminal.consumed() <= 500);
    drop(published);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn insufficient_unused_work_preserves_startup_guarantee_and_fallback() {
    let mut invocation_root = InvocationRoot::new();
    let (mut operation, keys, account) =
        operation(&mut invocation_root, 64, 64, OperationControl::default());
    operation
        .producer_host()
        .charge_observed_batch([(keys.work, 30)])
        .unwrap();
    let before = counters(&mut operation, keys);
    assert!(matches!(
        operation.qualify_failure_validation_work(keys.work, 4, 31),
        Err(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded { .. }))
    ));
    assert_eq!(counters(&mut operation, keys), before);
    assert_eq!(
        operation.raw.failure_terminal_work_limits(),
        Some((2, 5, 5))
    );
    let published = operation.prepare_minimal_fallback(7).unwrap().handoff();
    assert_eq!(published.ledger_evidence.header().current_live_bytes(), 0);
    assert_eq!(published.ledger_evidence.header().active_reservations(), 0);
    let ordinary = published.ledger_evidence.dimension(keys.work).unwrap();
    assert_eq!((ordinary.attempted(), ordinary.consumed()), before);
    drop(published);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn qualification_is_one_time_and_cannot_reduce_startup_guarantees() {
    let mut invocation_root = InvocationRoot::new();
    let (mut operation, keys, account) =
        operation(&mut invocation_root, 128, 128, OperationControl::default());
    operation
        .qualify_failure_validation_work(keys.work, 1, 1)
        .unwrap();
    assert_eq!(
        operation.raw.failure_terminal_work_limits(),
        Some((2, 5, 5))
    );
    let before = counters(&mut operation, keys);
    assert!(matches!(
        operation.qualify_failure_validation_work(keys.work, 4, 31),
        Err(StoreRuntimeError::InvalidPublicationPlan)
    ));
    assert_eq!(counters(&mut operation, keys), before);
    assert_eq!(
        operation.raw.failure_terminal_work_limits(),
        Some((2, 5, 5))
    );
    drop(operation.prepare_minimal_fallback(7).unwrap());
    assert!(!account.snapshot().poisoned());
}

#[test]
fn unrepresentable_qualification_preserves_counters_and_fallback() {
    for (members, fields) in [(usize::MAX, 31), (4, u64::MAX)] {
        let mut invocation_root = InvocationRoot::new();
        let (mut operation, keys, account) =
            operation(&mut invocation_root, 128, 128, OperationControl::default());
        let before = counters(&mut operation, keys);
        assert!(matches!(
            operation.qualify_failure_validation_work(keys.work, members, fields),
            Err(StoreRuntimeError::InvalidPublicationPlan)
        ));
        assert_eq!(counters(&mut operation, keys), before);
        assert_eq!(
            operation.raw.failure_terminal_work_limits(),
            Some((2, 5, 5))
        );
        drop(operation.prepare_minimal_fallback(7).unwrap());
        assert!(!account.snapshot().poisoned());
    }
}

struct Stop {
    cancelled: Arc<AtomicBool>,
    polls: Arc<AtomicUsize>,
}
impl crate::CancellationSignal for Stop {
    fn is_cancelled(&self) -> bool {
        self.polls.fetch_add(1, Ordering::Relaxed);
        self.cancelled.load(Ordering::Relaxed)
    }
}

#[test]
fn cancellation_before_qualification_retains_startup_fallback() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let control = OperationControl::default().with_cancellation(Stop {
        cancelled: cancelled.clone(),
        polls: Arc::new(AtomicUsize::new(0)),
    });
    let mut invocation_root = InvocationRoot::new();
    let (mut operation, keys, account) = operation(&mut invocation_root, 128, 128, control);
    let before = counters(&mut operation, keys);
    cancelled.store(true, Ordering::Relaxed);
    assert!(matches!(
        operation.qualify_failure_validation_work(keys.work, 4, 31),
        Err(StoreRuntimeError::Control(_))
    ));
    assert_eq!(counters(&mut operation, keys), before);
    assert_eq!(
        operation.raw.failure_terminal_work_limits(),
        Some((2, 5, 5))
    );
    let published = operation.prepare_minimal_fallback(7).unwrap().handoff();
    assert!(
        matches!(published.ledger_evidence.header().terminal_cause(),
        crate::StoreTerminalCause::Failure(cause)
            if cause.class() == crate::StorePreparationFailureClass::Control)
    );
    drop(published);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn poison_is_checked_before_qualification_poll_or_policy_changes() {
    let polls = Arc::new(AtomicUsize::new(0));
    let control = OperationControl::default().with_cancellation(Stop {
        cancelled: Arc::new(AtomicBool::new(false)),
        polls: polls.clone(),
    });
    let mut invocation_root = InvocationRoot::new();
    let (mut operation, keys, account) = operation(&mut invocation_root, 128, 128, control);
    let _ = account.test_record_poison(crate::AccountPoisonReason::OwnershipGraphUnverified);
    let before = account.snapshot();
    let before_polls = polls.load(Ordering::Relaxed);
    assert!(matches!(
        operation.qualify_failure_validation_work(keys.work, 4, 31),
        Err(StoreRuntimeError::AccountPoisoned(_))
    ));
    assert_eq!(account.snapshot(), before);
    assert_eq!(polls.load(Ordering::Relaxed), before_polls);
    assert_eq!(
        operation.raw.failure_terminal_work_limits(),
        Some((2, 5, 5))
    );
}

type SemanticOutcome<'inv> =
    CanonicalActivationOutcome<'inv, Owner, Progress, Unit, (), (), ScalarSequence<u8>, Tag>;

type SemanticFinalOutcome<'inv> = CanonicalInvocationOutcome<
    'inv,
    Owner,
    Progress,
    Unit,
    (),
    (),
    (),
    (),
    ScalarSequence<u8>,
    Tag,
    ScalarSequence<u8>,
    Tag,
>;

fn request_semantic_rejection<'inv>(
    outcome: SemanticOutcome<'inv>,
    disposition: CanonicalDispositionSlot<'inv, ScalarSequence<u8>, Tag, ScalarSequence<u8>, Tag>,
) {
    let CanonicalActivationOutcome::NeverActivated(rejected) = outcome else {
        panic!("semantic rejection must not activate canonical storage");
    };
    rejected.request_finish(disposition).unwrap();
}

fn release_semantic_rejection<'inv>(
    outcome: SemanticFinalOutcome<'inv>,
    account: &PhysicalStorageAccount,
) {
    let CanonicalInvocationOutcome::NeverActivated(rejected) = outcome else {
        panic!("semantic rejection must not activate canonical storage");
    };
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    drop(prepared.handoff());
    let snapshot = account.snapshot();
    assert!(!snapshot.poisoned());
    assert_eq!(snapshot.live_retained_bytes(), 0);
    assert_eq!(snapshot.live_retained_allocations(), 0);
}

#[test]
fn prior_ledger_failure_skips_the_transaction_and_preserves_healthy_failure_evidence() {
    let mut invocation_host = InvocationHost::new();
    let mut retained_account = None;
    let finish = invocation_host
        .run_body(|parts, disposition| {
            let CanonicalBodyParts {
                startup: invocation_root,
                output,
                activation,
                ..
            } = parts;
            let (mut operation, keys, account) =
                body_operation(invocation_root, 4096, 4096, OperationControl::default());
            assert!(
                operation
                    .producer_host()
                    .charge_attempted(keys.work, u64::MAX)
                    .is_err()
            );
            let transaction_ran = std::cell::Cell::new(false);
            let outcome: SemanticOutcome<'_> = keys
                .body_session(operation)
                .plan_and_activate_scoped(output, activation, |planning| {
                    transaction_ran.set(true);
                    planning.reject((), 88)
                })
                .unwrap();
            assert!(!transaction_ran.get());
            let CanonicalActivationOutcome::NeverActivated(rejected) = outcome else {
                panic!("prior ledger rejection must retain its dormant failure owner");
            };
            assert!(matches!(
                rejected.cause(),
                StoreNeverActivatedCause::Runtime(StoreRuntimeError::SuccessOutcomeRejected)
            ));
            rejected.request_finish(disposition).unwrap();
            retained_account = Some(account);
        })
        .unwrap();
    let account = retained_account.unwrap();
    let CanonicalInvocationOutcome::NeverActivated(rejected) = finish.resume().outcome else {
        panic!("prior ledger rejection must resume its dormant failure owner");
    };
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert!(matches!(
        published.ledger_evidence.header().terminal_cause(),
        crate::concrete_store::StoreTerminalCause::Failure(cause)
            if cause.class() == crate::concrete_store::StorePreparationFailureClass::Ledger
    ));
    drop(published);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn prior_failure_still_contains_a_panicking_transaction_capture_drop() {
    let mut invocation_host = InvocationHost::new();
    let body = invocation_host.run_body(|parts, _disposition| {
        let CanonicalBodyParts {
            startup: invocation_root,
            output,
            activation,
            ..
        } = parts;
        let (mut operation, keys, account) =
            body_operation(invocation_root, 4096, 4096, OperationControl::default());
        assert!(
            operation
                .producer_host()
                .charge_attempted(keys.work, u64::MAX)
                .is_err()
        );
        let transaction_ran = std::cell::Cell::new(false);
        let dropped = std::cell::Cell::new(false);
        let capture = SemanticPanicDrop(&dropped);
        let ran = &transaction_ran;
        let outcome: Result<SemanticOutcome<'_>, _> = keys
            .body_session(operation)
            .plan_and_activate_scoped(output, activation, move |planning| {
                let _capture = &capture;
                ran.set(true);
                planning.reject((), 89)
            });
        assert!(!transaction_ran.get());
        assert!(dropped.get());
        assert!(outcome.is_err());
        assert!(account.snapshot().poisoned());
    });
    assert!(body.is_err());
}

#[test]
fn forgotten_zero_or_completed_action_loan_keeps_host_cleanup_and_healthy_rejection() {
    for completed in [false, true] {
        let mut invocation_host = InvocationHost::new();
        let mut retained_account = None;
        let finish = invocation_host
            .run_body(|parts, disposition| {
                let CanonicalBodyParts {
                    startup,
                    output,
                    activation,
                    ..
                } = parts;
                let (mut operation, keys, account) =
                    body_operation(startup, 4096, 4096, OperationControl::default());
                operation
                    .with_action_execution::<_, StoreRuntimeError>(keys.work, |actions| {
                        if completed {
                            actions.begin_action(1)?;
                            actions.complete_action()?;
                        }
                        let _forgotten = std::mem::ManuallyDrop::new(actions);
                        Ok(())
                    })
                    .unwrap();
                let transaction_ran = std::cell::Cell::new(false);
                let outcome: SemanticOutcome<'_> = keys
                    .body_session(operation)
                    .plan_and_activate_scoped(output, activation, |planning| {
                        transaction_ran.set(true);
                        planning.reject((), 90)
                    })
                    .unwrap();
                assert!(transaction_ran.get());
                assert!(!account.snapshot().poisoned());
                request_semantic_rejection(outcome, disposition);
                retained_account = Some(account);
            })
            .unwrap();
        release_semantic_rejection(finish.resume().outcome, retained_account.as_ref().unwrap());
    }
}

fn semantic_counters(
    semantic: &CanonicalSemanticWorkSession<'_, '_, Owner, Progress, Unit>,
    keys: Keys,
) -> (u64, u64) {
    let (attempted, consumed, _) = semantic.context.work_counters(keys.work).unwrap();
    (attempted, consumed)
}

#[test]
fn semantic_execution_polls_at_entry_256_actions_and_finish() {
    for (copy, actions) in [false, true]
        .into_iter()
        .flat_map(|copy| [255_u64, 256, 257].map(|actions| (copy, actions)))
    {
        let polls = Arc::new(AtomicUsize::new(0));
        let control = OperationControl::default().with_cancellation(Stop {
            cancelled: Arc::new(AtomicBool::new(false)),
            polls: polls.clone(),
        });
        let mut invocation_host = InvocationHost::new();
        let mut control = Some(control);
        let mut retained_account = None;
        let finish = invocation_host
            .run_body(|parts, disposition| {
                let CanonicalBodyParts {
                    startup: invocation_root,
                    output,
                    activation,
                    ..
                } = parts;
                let (operation, keys, account) =
                    body_operation(invocation_root, 4096, 4096, control.take().unwrap());
                let outcome: SemanticOutcome<'_> = keys
                    .body_session(operation)
                    .plan_and_activate_scoped(output, activation, |mut planning| {
                        let before_entry = polls.load(Ordering::Relaxed);
                        let mut before_finish = 0;
                        let finished = planning.with_semantic_work(|semantic| {
                            assert_eq!(polls.load(Ordering::Relaxed), before_entry + 1);
                            let baseline = semantic_counters(semantic, keys);
                            for completed in 1..=actions {
                                if copy {
                                    semantic.run_copy_unit(|| ()).unwrap();
                                } else {
                                    semantic.run_unit(|| ()).unwrap();
                                }
                                assert_eq!(
                                    semantic_counters(semantic, keys),
                                    (baseline.0 + completed, baseline.1 + completed)
                                );
                                let block_polls = usize::try_from((completed - 1) / 256).unwrap();
                                assert_eq!(
                                    polls.load(Ordering::Relaxed),
                                    before_entry + 1 + block_polls
                                );
                            }
                            before_finish = polls.load(Ordering::Relaxed);
                            Err::<(), _>(())
                        });
                        assert!(matches!(
                            finished,
                            Err(CanonicalSemanticWorkError::Rejected(()))
                        ));
                        assert_eq!(polls.load(Ordering::Relaxed), before_finish + 1);
                        planning.reject((), 81)
                    })
                    .unwrap();
                request_semantic_rejection(outcome, disposition);
                retained_account = Some(account);
            })
            .unwrap();
        release_semantic_rejection(finish.resume().outcome, retained_account.as_ref().unwrap());
    }
}

#[test]
fn semantic_returned_error_completes_before_any_checkpoint() {
    for copy in [false, true] {
        let mut invocation_host = InvocationHost::new();
        let mut retained_account = None;
        let finish = invocation_host
            .run_body(|parts, disposition| {
                let CanonicalBodyParts {
                    startup: invocation_root,
                    output,
                    activation,
                    ..
                } = parts;
                let (operation, keys, account) =
                    body_operation(invocation_root, 4096, 4096, OperationControl::default());
                let outcome: SemanticOutcome<'_> = keys
                    .body_session(operation)
                    .plan_and_activate_scoped(output, activation, |mut planning| {
                        let finished = planning.with_semantic_work(|semantic| {
                            let baseline = semantic_counters(semantic, keys);
                            assert_eq!(
                                if copy {
                                    semantic.run_copy_unit(|| Err::<(), _>("invalid domain value"))
                                } else {
                                    semantic.run_unit(|| Err::<(), _>("invalid domain value"))
                                }
                                .unwrap(),
                                Err("invalid domain value")
                            );
                            assert_eq!(
                                semantic_counters(semantic, keys),
                                (baseline.0 + 1, baseline.1 + 1)
                            );
                            Err::<(), _>(())
                        });
                        assert!(matches!(
                            finished,
                            Err(CanonicalSemanticWorkError::Rejected(()))
                        ));
                        planning.reject((), 82)
                    })
                    .unwrap();
                request_semantic_rejection(outcome, disposition);
                retained_account = Some(account);
            })
            .unwrap();
        release_semantic_rejection(finish.resume().outcome, retained_account.as_ref().unwrap());
    }
}

#[test]
fn semantic_consumed_limit_denies_the_callback_before_execution() {
    for (copy, completed_prefix) in [(false, 0_u64), (true, 0), (true, 1)] {
        let mut invocation_host = InvocationHost::new();
        let mut retained_account = None;
        let finish = invocation_host
            .run_body(|parts, disposition| {
                let CanonicalBodyParts {
                    startup: invocation_root,
                    output,
                    activation,
                    ..
                } = parts;
                let (operation, keys, account) =
                    body_operation(invocation_root, 4096, 4096, OperationControl::default());
                let executed = std::cell::Cell::new(false);
                let outcome: SemanticOutcome<'_> = keys
                    .body_session(operation)
                    .plan_and_activate_scoped(output, activation, |mut planning| {
                        let finished = planning.with_semantic_work(|semantic| {
                            let baseline = semantic_counters(semantic, keys);
                            semantic
                                .context
                                .tighten_dimension_policy(
                                    keys.work,
                                    DimensionPolicy::counter(
                                        baseline.0 + 8,
                                        baseline.1 + completed_prefix,
                                        0,
                                    ),
                                )
                                .unwrap();
                            for _ in 0..completed_prefix {
                                semantic.run_copy_unit(|| ()).unwrap();
                            }
                            let denial = if copy {
                                semantic.run_copy_unit(|| executed.set(true))
                            } else {
                                semantic.run_unit(|| executed.set(true))
                            };
                            assert!(matches!(
                                denial,
                                Err(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
                                    class: crate::CounterClass::Consumed,
                                    ..
                                }))
                            ));
                            assert!(!executed.get());
                            assert_eq!(
                                semantic.context.work_counters(keys.work).unwrap(),
                                (
                                    baseline.0 + completed_prefix + 1,
                                    baseline.1 + completed_prefix,
                                    0
                                )
                            );
                            Ok::<(), ()>(())
                        });
                        assert!(matches!(
                            finished,
                            Err(CanonicalSemanticWorkError::Runtime(
                                StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
                                    class: crate::CounterClass::Consumed,
                                    ..
                                })
                            ))
                        ));
                        planning.reject((), 83)
                    })
                    .unwrap();
                request_semantic_rejection(outcome, disposition);
                retained_account = Some(account);
            })
            .unwrap();
        release_semantic_rejection(finish.resume().outcome, retained_account.as_ref().unwrap());
    }
}

#[test]
fn semantic_copy_panic_preserves_the_numeric_prefix_and_denies_reuse() {
    for completed_prefix in [0_u64, 1] {
        let mut invocation_host = InvocationHost::new();
        let body = invocation_host.run_body(|parts, _disposition| {
            let CanonicalBodyParts {
                startup: invocation_root,
                output,
                activation,
                ..
            } = parts;
            let (operation, keys, account) =
                body_operation(invocation_root, 4096, 4096, OperationControl::default());
            let result: Result<SemanticOutcome<'_>, _> = keys
                .body_session(operation)
                .plan_and_activate_scoped(output, activation, |mut planning| {
                    let finished = planning.with_semantic_work(|semantic| {
                        let baseline = semantic_counters(semantic, keys);
                        for _ in 0..completed_prefix {
                            semantic.run_copy_unit(|| ()).unwrap();
                        }
                        let panicked = catch_unwind(AssertUnwindSafe(|| {
                            let _ = semantic.run_copy_unit(|| -> () {
                                panic!("Copy semantic leaf panic");
                            });
                        }));
                        assert!(panicked.is_err());
                        let expected = (
                            baseline.0 + completed_prefix + 1,
                            baseline.1 + completed_prefix,
                            0,
                        );
                        assert_eq!(semantic.context.work_counters(keys.work).unwrap(), expected);
                        let executed = std::cell::Cell::new(false);
                        assert!(matches!(
                            semantic.run_copy_unit(|| executed.set(true)),
                            Err(StoreRuntimeError::ConstructionInvariantUnverified)
                        ));
                        assert!(!executed.get());
                        assert_eq!(semantic.context.work_counters(keys.work).unwrap(), expected);
                        Ok::<(), ()>(())
                    });
                    assert!(matches!(
                        finished,
                        Err(CanonicalSemanticWorkError::Runtime(
                            StoreRuntimeError::ConstructionInvariantUnverified
                        ))
                    ));
                    planning.reject((), 88)
                });
            assert!(result.is_err());
            assert!(account.snapshot().poisoned());
        });
        assert!(body.is_err());
    }
}

#[test]
fn semantic_copy_units_can_cross_storage_and_owned_callback_boundaries() {
    let mut invocation_host = InvocationHost::new();
    let mut retained_account = None;
    let finish = invocation_host
        .run_body(|parts, disposition| {
            let CanonicalBodyParts {
                startup: invocation_root,
                output,
                activation,
                ..
            } = parts;
            let (operation, keys, account) =
                body_operation(invocation_root, 4096, 4096, OperationControl::default());
            let outcome: SemanticOutcome<'_> = keys
                .body_session(operation)
                .plan_and_activate_scoped(output, activation, |mut planning| {
                    let finished = planning.with_semantic_work(|semantic| {
                        let baseline = semantic_counters(semantic, keys);
                        assert_eq!(semantic.run_copy_unit(|| 7).unwrap(), 7);
                        let mut scratch = semantic.reserve(8).unwrap();
                        semantic.run_copy_unit(|| ()).unwrap();
                        semantic.resize(&mut scratch, 4).unwrap();
                        semantic.run_copy_unit(|| ()).unwrap();
                        semantic.validate_live_owner(&scratch).unwrap();
                        semantic.run_copy_unit(|| ()).unwrap();
                        let owned = String::from("owned semantic capture");
                        assert_eq!(semantic.run_unit(move || owned.len()).unwrap(), 22);
                        assert!(!semantic.run_copy_unit(|| false).unwrap());
                        semantic.poll_if_due().unwrap();
                        semantic.context.check_work_quiescent().unwrap();
                        assert_eq!(
                            semantic_counters(semantic, keys),
                            (baseline.0 + 6, baseline.1 + 6)
                        );
                        drop(scratch);
                        Err::<(), _>(())
                    });
                    assert!(matches!(
                        finished,
                        Err(CanonicalSemanticWorkError::Rejected(()))
                    ));
                    planning.reject((), 87)
                })
                .unwrap();
            request_semantic_rejection(outcome, disposition);
            retained_account = Some(account);
        })
        .unwrap();
    release_semantic_rejection(finish.resume().outcome, retained_account.as_ref().unwrap());
}

#[test]
fn semantic_scratch_boundaries_poll_before_the_action_cadence_is_due() {
    for boundary in ["reserve", "resize", "validate"] {
        let cancelled = Arc::new(AtomicBool::new(false));
        let control = OperationControl::default().with_cancellation(Stop {
            cancelled: cancelled.clone(),
            polls: Arc::new(AtomicUsize::new(0)),
        });
        let mut invocation_host = InvocationHost::new();
        let mut control = Some(control);
        let mut retained_account = None;
        let finish = invocation_host
            .run_body(|parts, disposition| {
                let CanonicalBodyParts {
                    startup: invocation_root,
                    output,
                    activation,
                    ..
                } = parts;
                let (operation, keys, account) =
                    body_operation(invocation_root, 4096, 4096, control.take().unwrap());
                let outcome: SemanticOutcome<'_> = keys
                    .body_session(operation)
                    .plan_and_activate_scoped(output, activation, |mut planning| {
                        let finished = planning.with_semantic_work(|semantic| {
                            let mut scratch = semantic.reserve(4).unwrap();
                            semantic.run_copy_unit(|| ()).unwrap();
                            let before = scratch.raw.covered_bytes_for_test();
                            let physical_before = account.snapshot();
                            cancelled.store(true, Ordering::Relaxed);
                            let result = match boundary {
                                "reserve" => semantic.reserve(8).map(drop),
                                "resize" => semantic.resize(&mut scratch, 8),
                                "validate" => semantic.validate_live_owner(&scratch),
                                _ => unreachable!(),
                            };
                            assert!(matches!(result, Err(StoreRuntimeError::Control(_))));
                            assert_eq!(scratch.raw.covered_bytes_for_test(), before);
                            assert_eq!(account.snapshot(), physical_before);
                            drop(scratch);
                            Ok::<(), ()>(())
                        });
                        assert!(matches!(
                            finished,
                            Err(CanonicalSemanticWorkError::Runtime(
                                StoreRuntimeError::Control(_)
                            ))
                        ));
                        planning.reject((), 84)
                    })
                    .unwrap();
                request_semantic_rejection(outcome, disposition);
                retained_account = Some(account);
            })
            .unwrap();
        release_semantic_rejection(finish.resume().outcome, retained_account.as_ref().unwrap());
    }
}

struct SemanticPanicDrop<'a>(&'a std::cell::Cell<bool>);

impl Drop for SemanticPanicDrop<'_> {
    fn drop(&mut self) {
        self.0.set(true);
        panic!("semantic capture drop panic");
    }
}

#[test]
fn semantic_capture_drop_failure_retains_the_returned_action_prefix() {
    let mut invocation_host = InvocationHost::new();
    let body = invocation_host.run_body(|parts, _disposition| {
        let CanonicalBodyParts {
            startup: invocation_root,
            output,
            activation,
            ..
        } = parts;
        let (operation, keys, account) =
            body_operation(invocation_root, 4096, 4096, OperationControl::default());
        let dropped = std::cell::Cell::new(false);
        let result: Result<SemanticOutcome<'_>, _> = keys
            .body_session(operation)
            .plan_and_activate_scoped(output, activation, |mut planning| {
                let finished = planning.with_semantic_work(|semantic| {
                    let baseline = semantic_counters(semantic, keys);
                    let capture = SemanticPanicDrop(&dropped);
                    assert!(matches!(
                        semantic.run_unit(move || {
                            let _capture = &capture;
                        }),
                        Err(StoreRuntimeError::ConstructionInvariantUnverified)
                    ));
                    assert!(dropped.get());
                    assert_eq!(
                        semantic_counters(semantic, keys),
                        (baseline.0 + 1, baseline.1 + 1)
                    );
                    assert!(semantic.first_fault.as_ref().unwrap().contract_seen);
                    Ok::<(), ()>(())
                });
                assert!(matches!(
                    finished,
                    Err(CanonicalSemanticWorkError::Runtime(
                        StoreRuntimeError::ConstructionInvariantUnverified
                    ))
                ));
                planning.reject((), 85)
            });
        assert!(result.is_err());
        assert!(account.snapshot().poisoned());
    });
    assert!(body.is_err());
}

#[test]
fn semantic_denied_admission_catches_capture_drop_without_replacing_ledger_cause() {
    let mut invocation_host = InvocationHost::new();
    let body = invocation_host.run_body(|parts, _disposition| {
        let CanonicalBodyParts {
            startup: invocation_root,
            output,
            activation,
            ..
        } = parts;
        let (operation, keys, account) =
            body_operation(invocation_root, 4096, 4096, OperationControl::default());
        let dropped = std::cell::Cell::new(false);
        let executed = std::cell::Cell::new(false);
        let result: Result<SemanticOutcome<'_>, _> = keys
            .body_session(operation)
            .plan_and_activate_scoped(output, activation, |mut planning| {
                let finished = planning.with_semantic_work(|semantic| {
                    let baseline = semantic_counters(semantic, keys);
                    semantic
                        .context
                        .tighten_dimension_policy(
                            keys.work,
                            DimensionPolicy::counter(baseline.0 + 8, baseline.1, 0),
                        )
                        .unwrap();
                    let capture = SemanticPanicDrop(&dropped);
                    let executed_ref = &executed;
                    assert!(matches!(
                        semantic.run_unit(move || {
                            let _capture = &capture;
                            executed_ref.set(true);
                        }),
                        Err(StoreRuntimeError::Ledger(_))
                    ));
                    assert!(dropped.get());
                    assert!(!executed.get());
                    assert_eq!(
                        semantic_counters(semantic, keys),
                        (baseline.0 + 1, baseline.1)
                    );
                    let fault = semantic.first_fault.as_ref().unwrap();
                    assert!(matches!(&fault.cause, StoreRuntimeError::Ledger(_)));
                    assert!(fault.contract_seen);
                    Ok::<(), ()>(())
                });
                assert!(matches!(
                    finished,
                    Err(CanonicalSemanticWorkError::Runtime(
                        StoreRuntimeError::Ledger(_)
                    ))
                ));
                planning.reject((), 86)
            });
        assert!(result.is_err());
        assert!(account.snapshot().poisoned());
    });
    assert!(body.is_err());
}

#[test]
fn semantic_body_and_capture_drop_panics_keep_only_the_completed_prefix() {
    for completed_prefix in [0_u64, 1] {
        let mut invocation_host = InvocationHost::new();
        let body = invocation_host.run_body(|parts, _disposition| {
            let CanonicalBodyParts {
                startup: invocation_root,
                output,
                activation,
                ..
            } = parts;
            let (operation, keys, account) =
                body_operation(invocation_root, 4096, 4096, OperationControl::default());
            let dropped = std::cell::Cell::new(false);
            let result: Result<SemanticOutcome<'_>, _> = keys
                .body_session(operation)
                .plan_and_activate_scoped(output, activation, |mut planning| {
                    let finished = planning.with_semantic_work(|semantic| {
                        let baseline = semantic_counters(semantic, keys);
                        if completed_prefix == 1 {
                            semantic.run_unit(|| ()).unwrap();
                        }
                        let capture = SemanticPanicDrop(&dropped);
                        let panicked = catch_unwind(AssertUnwindSafe(|| {
                            let _ = semantic.run_unit(move || -> () {
                                let _capture = &capture;
                                panic!("semantic body panic");
                            });
                        }));
                        assert!(panicked.is_err());
                        assert!(dropped.get());
                        assert_eq!(
                            semantic_counters(semantic, keys),
                            (
                                baseline.0 + completed_prefix + 1,
                                baseline.1 + completed_prefix,
                            )
                        );
                        Ok::<(), ()>(())
                    });
                    assert!(matches!(
                        finished,
                        Err(CanonicalSemanticWorkError::Runtime(
                            StoreRuntimeError::ConstructionInvariantUnverified
                        ))
                    ));
                    planning.reject((), 87)
                });
            assert!(result.is_err());
            assert!(account.snapshot().poisoned());
        });
        assert!(body.is_err());
    }
}

#[test]
fn semantic_forgotten_borrow_retains_the_real_host_and_outer_reservation() {
    let mut invocation_host = InvocationHost::new();
    let mut retained_account = None;
    let finish = invocation_host.run_body(|parts, disposition| {
        let CanonicalBodyParts { startup, output, activation, .. } = parts;
        let (operation, keys, account) = body_operation(startup, 4096, 4096, OperationControl::default());
        let outcome: SemanticOutcome<'_> = keys.body_session(operation)
            .plan_and_activate_scoped(output, activation, |mut planning| {
                let (reservation, _) = planning.with_semantic_work(|semantic| {
                    let baseline = semantic_counters(semantic, keys);
                    let reservation = semantic.reserve(4).unwrap();
                    semantic.run_unit(|| ()).unwrap();
                    assert_eq!(semantic_counters(semantic, keys), (baseline.0 + 1, baseline.1 + 1));
                    #[allow(forgetting_references, clippy::forget_non_drop, reason = "exercise legal forgetting of a client loan while the library retains its actual host")]
                    std::mem::forget(semantic);
                    Ok::<_, ()>(reservation)
                }).unwrap();
                assert_eq!(reservation.raw.covered_bytes_for_test(), 4);
                drop(reservation);
                assert!(!account.snapshot().poisoned());
                planning.reject((), 88)
            }).unwrap();
        request_semantic_rejection(outcome, disposition);
        retained_account = Some(account);
    }).unwrap();
    release_semantic_rejection(finish.resume().outcome, retained_account.as_ref().unwrap());
}

struct SemanticCancelOnDrop {
    cancelled: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}

impl Drop for SemanticCancelOnDrop {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Relaxed);
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[test]
fn semantic_callback_capture_cleanup_precedes_the_final_control_checkpoint() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicBool::new(false));
    let polls = Arc::new(AtomicUsize::new(0));
    let mut control = Some(OperationControl::default().with_cancellation(Stop {
        cancelled: Arc::clone(&cancelled),
        polls: Arc::clone(&polls),
    }));
    let mut invocation_host = InvocationHost::new();
    let mut retained_account = None;
    let finish = invocation_host
        .run_body(|parts, disposition| {
            let CanonicalBodyParts {
                startup,
                output,
                activation,
                ..
            } = parts;
            let (operation, keys, account) =
                body_operation(startup, 4096, 4096, control.take().unwrap());
            let outcome: SemanticOutcome<'_> = keys
                .body_session(operation)
                .plan_and_activate_scoped(output, activation, |mut planning| {
                    let before_entry = polls.load(Ordering::Relaxed);
                    let capture = SemanticCancelOnDrop {
                        cancelled: Arc::clone(&cancelled),
                        dropped: Arc::clone(&dropped),
                    };
                    let finished = planning.with_semantic_work(move |semantic| {
                        let _capture = &capture;
                        let baseline = semantic_counters(semantic, keys);
                        semantic.run_copy_unit(|| ()).unwrap();
                        assert_eq!(
                            semantic_counters(semantic, keys),
                            (baseline.0 + 1, baseline.1 + 1)
                        );
                        Ok::<(), ()>(())
                    });
                    assert!(dropped.load(Ordering::Relaxed));
                    assert!(matches!(
                        finished,
                        Err(CanonicalSemanticWorkError::Runtime(
                            StoreRuntimeError::Control(_)
                        ))
                    ));
                    assert_eq!(polls.load(Ordering::Relaxed), before_entry + 2);
                    planning.reject((), 89)
                })
                .unwrap();
            request_semantic_rejection(outcome, disposition);
            retained_account = Some(account);
        })
        .unwrap();
    release_semantic_rejection(finish.resume().outcome, retained_account.as_ref().unwrap());
}

#[test]
fn semantic_second_entry_is_rejected_before_callback_or_control_work() {
    let polls = Arc::new(AtomicUsize::new(0));
    let mut control = Some(OperationControl::default().with_cancellation(Stop {
        cancelled: Arc::new(AtomicBool::new(false)),
        polls: Arc::clone(&polls),
    }));
    let mut invocation_host = InvocationHost::new();
    let body = invocation_host.run_body(|parts, _disposition| {
        let CanonicalBodyParts {
            startup,
            output,
            activation,
            ..
        } = parts;
        let (operation, keys, account) =
            body_operation(startup, 4096, 4096, control.take().unwrap());
        let entered = std::cell::Cell::new(false);
        let result: Result<SemanticOutcome<'_>, _> = keys
            .body_session(operation)
            .plan_and_activate_scoped(output, activation, |mut planning| {
                let _ = planning
                    .with_semantic_work(|semantic| {
                        semantic.run_unit(|| ()).unwrap();
                        Ok::<(), ()>(())
                    })
                    .unwrap();
                let before_second_entry = polls.load(Ordering::Relaxed);
                let second = planning.with_semantic_work(|_semantic| {
                    entered.set(true);
                    Ok::<(), ()>(())
                });
                assert!(matches!(
                    second,
                    Err(CanonicalSemanticWorkError::Runtime(
                        StoreRuntimeError::ConstructionInvariantUnverified
                    ))
                ));
                assert!(!entered.get());
                assert_eq!(polls.load(Ordering::Relaxed), before_second_entry);
                planning.reject((), 90)
            });
        assert!(result.is_err());
        assert!(account.snapshot().poisoned());
    });
    assert!(body.is_err());
}

#[test]
fn semantic_denied_entry_disposes_callback_capture_without_replacing_control() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut control = Some(OperationControl::default().with_cancellation(Stop {
        cancelled: Arc::clone(&cancelled),
        polls: Arc::new(AtomicUsize::new(0)),
    }));
    let mut invocation_host = InvocationHost::new();
    let body = invocation_host.run_body(|parts, _disposition| {
        let CanonicalBodyParts {
            startup,
            output,
            activation,
            ..
        } = parts;
        let (operation, keys, account) =
            body_operation(startup, 4096, 4096, control.take().unwrap());
        let dropped = std::cell::Cell::new(false);
        let executed = std::cell::Cell::new(false);
        let result: Result<SemanticOutcome<'_>, _> = keys
            .body_session(operation)
            .plan_and_activate_scoped(output, activation, |mut planning| {
                cancelled.store(true, Ordering::Relaxed);
                let capture = SemanticPanicDrop(&dropped);
                let executed = &executed;
                let finished = planning.with_semantic_work(move |_semantic| {
                    let _capture = &capture;
                    executed.set(true);
                    Ok::<(), ()>(())
                });
                assert!(matches!(
                    finished,
                    Err(CanonicalSemanticWorkError::Runtime(
                        StoreRuntimeError::Control(_)
                    ))
                ));
                assert!(!executed.get());
                assert!(dropped.get());
                let fault = planning.driver.first_fault.as_ref().unwrap();
                assert!(matches!(&fault.cause, StoreRuntimeError::Control(_)));
                assert!(fault.contract_seen);
                planning.reject((), 91)
            });
        assert!(result.is_err());
        assert!(account.snapshot().poisoned());
    });
    assert!(body.is_err());
}

#[test]
fn semantic_callback_body_and_capture_panics_preserve_the_actual_completed_prefix() {
    for panic_body in [false, true] {
        let mut invocation_host = InvocationHost::new();
        let body = invocation_host.run_body(|parts, _disposition| {
            let CanonicalBodyParts {
                startup,
                output,
                activation,
                ..
            } = parts;
            let (operation, keys, account) =
                body_operation(startup, 4096, 4096, OperationControl::default());
            let dropped = std::cell::Cell::new(false);
            let baseline = std::cell::Cell::new((0, 0));
            let result: Result<SemanticOutcome<'_>, _> = keys
                .body_session(operation)
                .plan_and_activate_scoped(output, activation, |mut planning| {
                    let capture = SemanticPanicDrop(&dropped);
                    let baseline_ref = &baseline;
                    match catch_unwind(AssertUnwindSafe(|| {
                        planning.with_semantic_work(move |semantic| {
                            let _capture = &capture;
                            baseline_ref.set(semantic_counters(semantic, keys));
                            semantic.run_copy_unit(|| ()).unwrap();
                            if panic_body {
                                panic!("semantic pass body panic");
                            }
                            Ok::<(), ()>(())
                        })
                    })) {
                        Ok(Err(CanonicalSemanticWorkError::Runtime(
                            StoreRuntimeError::ConstructionInvariantUnverified,
                        ))) => assert!(!panic_body),
                        Err(payload) => {
                            assert!(panic_body);
                            assert_eq!(
                                payload.downcast_ref::<&'static str>(),
                                Some(&"semantic pass body panic")
                            );
                        }
                        _ => panic!("unexpected semantic callback result"),
                    }
                    assert!(dropped.get());
                    let ScopedActivationState::Dormant(dormant) = &mut planning.driver.state else {
                        panic!("semantic callback must retain the actual dormant owner");
                    };
                    let state = dormant
                        .operation_mut()
                        .producer_host()
                        .dimension_state(keys.work)
                        .unwrap();
                    assert_eq!(
                        (state.attempted(), state.consumed()),
                        (baseline.get().0 + 1, baseline.get().1 + 1)
                    );
                    assert!(planning.driver.first_fault.as_ref().unwrap().contract_seen);
                    planning.reject((), 92)
                });
            assert!(result.is_err());
            assert!(account.snapshot().poisoned());
        });
        assert!(body.is_err());
    }
}

struct SemanticOwnedDropProbe<'probe> {
    id: u32,
    drops: &'probe std::cell::Cell<usize>,
    panic_on_drop: bool,
}

impl Drop for SemanticOwnedDropProbe<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
        if self.panic_on_drop {
            panic!("semantic owned payload drop panic");
        }
    }
}

#[test]
fn semantic_success_finish_failure_contains_panicking_returned_value() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let capture_dropped = Arc::new(AtomicBool::new(false));
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = std::cell::Cell::new(0);
    let baseline = std::cell::Cell::new((0, 0));
    let semantic_checked = std::cell::Cell::new(false);
    let mut control = Some(OperationControl::default().with_cancellation(Stop {
        cancelled: Arc::clone(&cancelled),
        polls: Arc::clone(&polls),
    }));
    let mut invocation_host = InvocationHost::new();
    let body = invocation_host.run_body(|parts, _disposition| {
        let CanonicalBodyParts {
            startup,
            output,
            activation,
            ..
        } = parts;
        let (operation, keys, account) =
            body_operation(startup, 4096, 4096, control.take().unwrap());
        let result: Result<SemanticOutcome<'_>, _> = keys
            .body_session(operation)
            .plan_and_activate_scoped(output, activation, |mut planning| {
                let before_entry = polls.load(Ordering::Relaxed);
                let capture = SemanticCancelOnDrop {
                    cancelled: Arc::clone(&cancelled),
                    dropped: Arc::clone(&capture_dropped),
                };
                let baseline_ref = &baseline;
                let drops_ref = &drops;
                let finished = planning.with_semantic_work(move |semantic| {
                    let _capture = &capture;
                    baseline_ref.set(semantic_counters(semantic, keys));
                    semantic.run_unit(|| ()).unwrap();
                    assert_eq!(
                        semantic_counters(semantic, keys),
                        (baseline_ref.get().0 + 1, baseline_ref.get().1 + 1)
                    );
                    assert!(semantic.first_fault.is_none());
                    Ok::<_, ()>(SemanticOwnedDropProbe {
                        id: 93,
                        drops: drops_ref,
                        panic_on_drop: true,
                    })
                });
                assert!(matches!(
                    finished,
                    Err(CanonicalSemanticWorkError::Runtime(
                        StoreRuntimeError::Control(crate::ControlCause::Cancelled {
                            origin: crate::CancellationOrigin::Signal
                        })
                    ))
                ));
                assert!(capture_dropped.load(Ordering::Relaxed));
                assert!(cancelled.load(Ordering::Relaxed));
                assert_eq!(drops.get(), 1);
                assert_eq!(polls.load(Ordering::Relaxed), before_entry + 2);
                let fault = planning.driver.first_fault.as_ref().unwrap();
                assert!(matches!(
                    fault.cause,
                    StoreRuntimeError::Control(crate::ControlCause::Cancelled {
                        origin: crate::CancellationOrigin::Signal
                    })
                ));
                assert!(fault.contract_seen);
                assert!(matches!(
                    planning.driver.semantic_state,
                    ScopedSemanticState::Rejected
                ));
                let ScopedActivationState::Dormant(dormant) = &mut planning.driver.state else {
                    panic!("semantic finish failure must retain its dormant owner");
                };
                let state = dormant
                    .operation_mut()
                    .producer_host()
                    .dimension_state(keys.work)
                    .unwrap();
                assert_eq!(
                    (state.attempted(), state.consumed()),
                    (baseline.get().0 + 1, baseline.get().1 + 1)
                );
                assert!(!account.snapshot().poisoned());
                // The activation entry catches panics; prove every immediate oracle ran.
                semantic_checked.set(true);
                planning.reject((), 93)
            });
        assert!(result.is_err());
        assert!(account.snapshot().poisoned());
    });
    assert!(body.is_err());
    assert!(semantic_checked.get());
    assert_eq!(drops.get(), 1);
}

#[test]
fn semantic_domain_owner_survives_rejection_finish_control_failure() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let capture_dropped = Arc::new(AtomicBool::new(false));
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = std::cell::Cell::new(0);
    let baseline = std::cell::Cell::new((0, 0));
    let semantic_checked = std::cell::Cell::new(false);
    let mut control = Some(OperationControl::default().with_cancellation(Stop {
        cancelled: Arc::clone(&cancelled),
        polls: Arc::clone(&polls),
    }));
    let mut invocation_host = InvocationHost::new();
    let mut retained_account = None;
    let finish = invocation_host
        .run_body(|parts, disposition| {
            let CanonicalBodyParts {
                startup,
                output,
                activation,
                ..
            } = parts;
            let (operation, keys, account) =
                body_operation(startup, 4096, 4096, control.take().unwrap());
            let outcome: SemanticOutcome<'_> = keys
                .body_session(operation)
                .plan_and_activate_scoped(output, activation, |mut planning| {
                    let before_entry = polls.load(Ordering::Relaxed);
                    let capture = SemanticCancelOnDrop {
                        cancelled: Arc::clone(&cancelled),
                        dropped: Arc::clone(&capture_dropped),
                    };
                    let baseline_ref = &baseline;
                    let drops_ref = &drops;
                    let finished = planning.with_semantic_work(move |semantic| {
                        let _capture = &capture;
                        baseline_ref.set(semantic_counters(semantic, keys));
                        semantic.run_unit(|| ()).unwrap();
                        assert_eq!(
                            semantic_counters(semantic, keys),
                            (baseline_ref.get().0 + 1, baseline_ref.get().1 + 1)
                        );
                        assert!(semantic.first_fault.is_none());
                        Err::<(), _>(SemanticOwnedDropProbe {
                            id: 94,
                            drops: drops_ref,
                            panic_on_drop: false,
                        })
                    });
                    let Err(CanonicalSemanticWorkError::Rejected(rejection)) = finished else {
                        panic!("rejection finish must return the actual domain owner");
                    };
                    assert_eq!(rejection.id, 94);
                    assert!(std::ptr::eq(rejection.drops, &drops));
                    assert_eq!(drops.get(), 0);
                    assert!(capture_dropped.load(Ordering::Relaxed));
                    assert!(cancelled.load(Ordering::Relaxed));
                    assert_eq!(polls.load(Ordering::Relaxed), before_entry + 2);
                    let fault = planning.driver.first_fault.as_ref().unwrap();
                    assert!(matches!(
                        fault.cause,
                        StoreRuntimeError::Control(crate::ControlCause::Cancelled {
                            origin: crate::CancellationOrigin::Signal
                        })
                    ));
                    assert!(!fault.contract_seen);
                    assert!(matches!(
                        planning.driver.semantic_state,
                        ScopedSemanticState::Rejected
                    ));
                    let ScopedActivationState::Dormant(dormant) = &mut planning.driver.state else {
                        panic!("domain rejection must retain its dormant owner");
                    };
                    let state = dormant
                        .operation_mut()
                        .producer_host()
                        .dimension_state(keys.work)
                        .unwrap();
                    assert_eq!(
                        (state.attempted(), state.consumed()),
                        (baseline.get().0 + 1, baseline.get().1 + 1)
                    );
                    assert!(!account.snapshot().poisoned());
                    drop(rejection);
                    assert_eq!(drops.get(), 1);
                    semantic_checked.set(true);
                    planning.reject((), 94)
                })
                .unwrap();
            let CanonicalActivationOutcome::NeverActivated(rejected) = &outcome else {
                panic!("rejection finish must not activate canonical storage");
            };
            assert!(matches!(
                rejected.cause(),
                StoreNeverActivatedCause::Runtime(StoreRuntimeError::Control(
                    crate::ControlCause::Cancelled {
                        origin: crate::CancellationOrigin::Signal
                    }
                ))
            ));
            request_semantic_rejection(outcome, disposition);
            retained_account = Some(account);
        })
        .unwrap();
    assert!(semantic_checked.get());
    let account = retained_account.unwrap();
    let CanonicalInvocationOutcome::NeverActivated(rejected) = finish.resume().outcome else {
        panic!("rejection finish must resume the dormant failure owner");
    };
    let (prepared, _) = rejected.prepare_minimal_fallback().unwrap().into_parts();
    let published = prepared.handoff();
    assert!(matches!(
        published.ledger_evidence.header().terminal_cause(),
        crate::concrete_store::StoreTerminalCause::Failure(cause)
            if cause.class() == crate::concrete_store::StorePreparationFailureClass::Control
    ));
    drop(published);
    assert_eq!(drops.get(), 1);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn semantic_capture_drop_failure_disposes_returned_rejection_once() {
    for prior_control in [false, true] {
        let cancelled = Arc::new(AtomicBool::new(false));
        let polls = Arc::new(AtomicUsize::new(0));
        let capture_dropped = std::cell::Cell::new(false);
        let drops = std::cell::Cell::new(0);
        let baseline = std::cell::Cell::new((0, 0));
        let semantic_checked = std::cell::Cell::new(false);
        let mut control = Some(OperationControl::default().with_cancellation(Stop {
            cancelled: Arc::clone(&cancelled),
            polls: Arc::clone(&polls),
        }));
        let mut invocation_host = InvocationHost::new();
        let body = invocation_host.run_body(|parts, _disposition| {
            let CanonicalBodyParts {
                startup,
                output,
                activation,
                ..
            } = parts;
            let (operation, keys, account) =
                body_operation(startup, 4096, 4096, control.take().unwrap());
            let result: Result<SemanticOutcome<'_>, _> = keys
                .body_session(operation)
                .plan_and_activate_scoped(output, activation, |mut planning| {
                    let before_entry = polls.load(Ordering::Relaxed);
                    let capture = SemanticPanicDrop(&capture_dropped);
                    let cancelled_ref = &cancelled;
                    let baseline_ref = &baseline;
                    let drops_ref = &drops;
                    let finished = planning.with_semantic_work(move |semantic| {
                        let _capture = &capture;
                        baseline_ref.set(semantic_counters(semantic, keys));
                        semantic.run_copy_unit(|| ()).unwrap();
                        assert!(semantic.first_fault.is_none());
                        if prior_control {
                            cancelled_ref.store(true, Ordering::Relaxed);
                            assert!(matches!(
                                semantic.reserve(4),
                                Err(StoreRuntimeError::Control(crate::ControlCause::Cancelled {
                                    origin: crate::CancellationOrigin::Signal
                                }))
                            ));
                            let fault = semantic.first_fault.as_ref().unwrap();
                            assert!(matches!(
                                fault.cause,
                                StoreRuntimeError::Control(crate::ControlCause::Cancelled {
                                    origin: crate::CancellationOrigin::Signal
                                })
                            ));
                            assert!(!fault.contract_seen);
                        }
                        assert_eq!(
                            semantic_counters(semantic, keys),
                            (baseline_ref.get().0 + 1, baseline_ref.get().1 + 1)
                        );
                        Err::<(), _>(SemanticOwnedDropProbe {
                            id: 95,
                            drops: drops_ref,
                            panic_on_drop: true,
                        })
                    });
                    let Err(CanonicalSemanticWorkError::Runtime(cause)) = finished else {
                        panic!("failed callback cleanup must contain the domain owner");
                    };
                    let fault = planning.driver.first_fault.as_ref().unwrap();
                    if prior_control {
                        assert!(matches!(
                            cause,
                            StoreRuntimeError::Control(crate::ControlCause::Cancelled {
                                origin: crate::CancellationOrigin::Signal
                            })
                        ));
                        assert!(matches!(
                            fault.cause,
                            StoreRuntimeError::Control(crate::ControlCause::Cancelled {
                                origin: crate::CancellationOrigin::Signal
                            })
                        ));
                    } else {
                        assert!(matches!(
                            cause,
                            StoreRuntimeError::ConstructionInvariantUnverified
                        ));
                        assert!(matches!(
                            fault.cause,
                            StoreRuntimeError::ConstructionInvariantUnverified
                        ));
                    }
                    assert!(fault.contract_seen);
                    assert!(capture_dropped.get());
                    assert_eq!(drops.get(), 1);
                    assert_eq!(
                        polls.load(Ordering::Relaxed),
                        before_entry + if prior_control { 2 } else { 1 }
                    );
                    assert!(matches!(
                        planning.driver.semantic_state,
                        ScopedSemanticState::Rejected
                    ));
                    let ScopedActivationState::Dormant(dormant) = &mut planning.driver.state else {
                        panic!("capture cleanup failure must retain its dormant owner");
                    };
                    let state = dormant
                        .operation_mut()
                        .producer_host()
                        .dimension_state(keys.work)
                        .unwrap();
                    assert_eq!(
                        (state.attempted(), state.consumed()),
                        (baseline.get().0 + 1, baseline.get().1 + 1)
                    );
                    assert!(!account.snapshot().poisoned());
                    semantic_checked.set(true);
                    planning.reject((), 95)
                });
            assert!(result.is_err());
            assert!(account.snapshot().poisoned());
        });
        assert!(body.is_err());
        assert!(semantic_checked.get());
        assert_eq!(drops.get(), 1);
    }
}
