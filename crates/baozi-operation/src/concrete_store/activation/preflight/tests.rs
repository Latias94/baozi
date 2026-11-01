use super::*;
use crate::concrete_store::{RetainedStoreSharePlan, RetainedStoreStartPlan};
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

type Operation = RetainedStoreOperation<Owner, Progress, Unit>;

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

    fn session(
        self,
        operation: Operation,
    ) -> RetainedStoreSessionOutcomeOwner<Owner, Progress, Unit> {
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

fn operation(
    total_attempted: u64,
    total_consumed: u64,
    control: OperationControl<Progress, Unit>,
) -> (Operation, Keys, PhysicalStorageAccount) {
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
    let operation = Operation::try_start(control, ledger, &account, start).unwrap();
    (operation, keys, account)
}

fn counters(operation: &mut Operation, keys: Keys) -> (u64, u64) {
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
    let (mut operation, keys, account) = operation(512, 500, OperationControl::default());
    {
        let mut host = operation.producer_host();
        let mut work = host.try_begin_observed_work([(keys.work, 7)]).unwrap();
        work.complete([4]).unwrap();
        work.finish_partial();
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
    let (mut operation, keys, account) = operation(64, 64, OperationControl::default());
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
    let (mut operation, keys, account) = operation(128, 128, OperationControl::default());
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
        let (mut operation, keys, account) = operation(128, 128, OperationControl::default());
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
    let (mut operation, keys, account) = operation(128, 128, control);
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
    let (mut operation, keys, account) = operation(128, 128, control);
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
