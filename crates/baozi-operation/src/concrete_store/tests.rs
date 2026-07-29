use super::*;

use std::num::{NonZeroU32, NonZeroU64};

use crate::ledger::LedgerLimits;
use crate::retained::{test_fail_next_membership_assignment, test_panic_final_owner_drop_after};
use crate::{
    AccountPoisonReason, DimensionPolicy, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits,
};

struct TestOwner;

impl OperationOwner for TestOwner {}

const fn test_catalog() -> StoreSemanticCatalogId {
    StoreSemanticCatalogId::new(
        *b"baozi-unit-store",
        NonZeroU32::new(1).expect("test catalog revision is nonzero"),
    )
}

const fn test_phase_semantic(local: u64) -> StorePhaseSemanticId {
    StorePhaseSemanticId::new(
        test_catalog(),
        NonZeroU64::new(local).expect("test phase local id is nonzero"),
    )
}

const fn test_dimension_semantic(local: u64) -> StoreDimensionSemanticId {
    StoreDimensionSemanticId::new(
        test_catalog(),
        NonZeroU64::new(local).expect("test dimension local id is nonzero"),
    )
}

#[derive(Clone, Copy)]
struct TestKeys {
    phase: StorePhaseKey,
    dimension: StoreDimensionKey,
    work_dimension: StoreDimensionKey,
}

fn test_vocabulary() -> (StoreKeyDomain, TestKeys) {
    let domain = StoreKeyDomain::try_new(test_catalog()).expect("test store key domain");
    let keys = TestKeys {
        phase: domain
            .try_admit_phase(test_phase_semantic(1))
            .expect("test phase key"),
        dimension: domain
            .try_admit_dimension(test_dimension_semantic(1))
            .expect("test dimension key"),
        work_dimension: domain
            .try_admit_dimension(test_dimension_semantic(2))
            .expect("test work dimension key"),
    };
    (domain, keys)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TestProgress {
    Build,
}

impl ProgressPhase for TestProgress {
    fn order(self) -> u32 {
        0
    }

    fn stable_id(self) -> &'static str {
        "build"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TestUnit {
    Items,
}

impl ProgressUnit for TestUnit {
    fn stable_id(self) -> &'static str {
        "items"
    }
}

#[derive(Clone, Copy)]
struct TestTag;

#[derive(Clone, Copy)]
struct TestRootTag;

type TestOperation = RetainedStoreOperation<TestOwner, TestProgress, TestUnit>;
type TestAssembly = RetainedStorePublicationAssembly<TestOwner, TestProgress, TestUnit>;

fn test_factory() -> PhysicalStorageAccountFactory {
    PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).expect("test factory limits"),
    )
    .expect("test factory")
}

fn test_account(factory: &PhysicalStorageAccountFactory) -> PhysicalStorageAccount {
    factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(u64::MAX, 256).expect("test account limits"),
        )
        .expect("test account")
}

fn test_writer_plan(
    keys: TestKeys,
    attributions: usize,
    commitments: usize,
) -> RetainedStoreWriterPlan {
    RetainedStoreWriterPlan::new(
        keys.phase,
        keys.dimension,
        keys.dimension,
        RetainedStoreBatchLimits::new(attributions, commitments).expect("test writer limits"),
    )
}

fn test_operation(
    account: &PhysicalStorageAccount,
    evidence_limit: u64,
) -> (TestOperation, TestKeys) {
    test_operation_with_control_and_work_limits(
        account,
        evidence_limit,
        OperationControl::default(),
        u64::MAX,
        u64::MAX,
    )
}

fn test_operation_with_control_and_work_limits(
    account: &PhysicalStorageAccount,
    evidence_limit: u64,
    control: OperationControl<TestProgress, TestUnit>,
    attempted_limit: u64,
    consumed_limit: u64,
) -> (TestOperation, TestKeys) {
    let (domain, keys) = test_vocabulary();
    let storage_policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(evidence_limit);
    let work_policy =
        DimensionPolicy::reservable_bytes(attempted_limit, consumed_limit, u64::MAX, u64::MAX)
            .with_published_outcome_evidence_limit(u64::MAX);
    let ledger_limits = LedgerLimits::new(
        u64::MAX,
        [
            (keys.dimension, storage_policy),
            (keys.work_dimension, work_policy),
        ],
    )
    .expect("test ledger limits");
    let ledger_plan = StoreLedgerPlan::try_new(domain, ledger_limits, &[keys.phase])
        .expect("test store ledger plan");
    let start = RetainedStoreStartPlan::new(
        test_writer_plan(keys, 32, 8),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        1,
        keys.dimension,
        1,
    );
    (
        TestOperation::try_start(control, ledger_plan, account, start).expect("test operation"),
        keys,
    )
}

fn test_simple_controlled_validation_graph(
    assembly: &mut TestAssembly,
) -> (
    CanonicalSequenceRef<u8, TestRootTag>,
    OutcomeEvidenceSequenceRef<u8, TestTag>,
) {
    let canonical_root = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("canonical sequence plan"),
            TestRootTag,
            |writer| writer.push(7),
        )
        .expect("canonical root");
    let evidence_root = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("evidence sequence plan"),
            TestTag,
            |writer| writer.push(9),
        )
        .expect("evidence root");
    (canonical_root, evidence_root)
}

fn test_publication_plan(
    keys: TestKeys,
    canonical_blocks: usize,
    evidence_blocks: usize,
) -> RetainedStorePublicationPlan {
    RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(
            test_writer_plan(keys, canonical_blocks + 1, 2),
            canonical_blocks,
            canonical_blocks,
        )
        .expect("test canonical table plan"),
        RetainedStoreBlockTablePlan::new(
            test_writer_plan(keys, evidence_blocks + 1, 2),
            evidence_blocks,
            evidence_blocks,
        )
        .expect("test evidence table plan"),
    )
}

fn test_final_progress() -> FinalProgress<TestProgress, TestUnit> {
    FinalProgress::new(TestProgress::Build, TestUnit::Items, 1)
}

fn failure_outcome_header<EvidenceKind, EvidenceTag>(
    outcome: PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag>,
) -> StoreLedgerEvidenceHeader {
    match outcome {
        PreparedStoreFailureOutcome::RichFailure(prepared) => {
            prepared.handoff().ledger_evidence.header()
        }
        PreparedStoreFailureOutcome::MinimalFallback(prepared) => {
            prepared.handoff().ledger_evidence.header()
        }
    }
}

fn test_physical_delta(
    before: crate::PhysicalStorageAccountSnapshot,
    after: crate::PhysicalStorageAccountSnapshot,
) -> (u64, u64) {
    (
        after
            .live_retained_bytes()
            .checked_sub(before.live_retained_bytes())
            .expect("test retained bytes grow monotonically"),
        after
            .live_retained_allocations()
            .checked_sub(before.live_retained_allocations())
            .expect("test retained allocations grow monotonically"),
    )
}

#[test]
fn publication_assembly_minimal_fallback_freezes_requested_domain_cause() {
    const REQUESTED_CAUSE: u32 = 0x51f0_0001;

    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, 16);
    let assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("minimal-fallback assembly");

    let published = assembly
        .prepare_minimal_fallback_with(REQUESTED_CAUSE)
        .expect("requested-cause fallback")
        .handoff();
    let StoreTerminalCause::Failure(cause) = published.ledger_evidence.header().terminal_cause()
    else {
        panic!("minimal fallback must freeze a failure cause");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
    assert_eq!(cause.code(), REQUESTED_CAUSE);
}

#[test]
fn failure_assembly_minimal_fallback_freezes_requested_domain_cause() {
    const REQUESTED_CAUSE: u32 = 0x51f0_0002;

    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, 16);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("failure assembly");
    let (canonical_root, evidence_root) = test_simple_controlled_validation_graph(&mut assembly);
    let failure = match assembly.validate_success(canonical_root, evidence_root, |_, _, _| false) {
        StoreSuccessValidation::Rejected(failure) => failure,
        _ => panic!("semantic rejection must preserve a failure assembly"),
    };

    let published = failure
        .prepare_minimal_fallback_with(REQUESTED_CAUSE)
        .expect("requested-cause failure fallback")
        .handoff();
    let StoreTerminalCause::Failure(cause) = published.ledger_evidence.header().terminal_cause()
    else {
        panic!("failure fallback must freeze a failure cause");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
    assert_eq!(cause.code(), REQUESTED_CAUSE);
}

#[test]
fn admission_downgrade_cause_overrides_later_minimal_fallback_code() {
    const DOWNGRADE_CAUSE: u32 = 0x51f0_0003;
    const LATER_CAUSE: u32 = 0x51f0_0004;

    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, 16);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("validated success assembly");
    let (canonical_root, evidence_root) = test_simple_controlled_validation_graph(&mut assembly);
    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical_root, evidence_root| {
            session.canonical_sequence(canonical_root).is_some()
                && session.outcome_evidence_sequence(evidence_root).is_some()
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("complete graph must validate before admission downgrade"),
    };

    let published = validated
        .downgrade_to_rich_failure(DOWNGRADE_CAUSE)
        .prepare_minimal_fallback_with(LATER_CAUSE)
        .expect("admission-downgrade fallback")
        .handoff();
    let StoreTerminalCause::Failure(cause) = published.ledger_evidence.header().terminal_cause()
    else {
        panic!("admission downgrade must freeze a failure cause");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
    assert_eq!(cause.code(), DOWNGRADE_CAUSE);
}

fn public_method_block<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("missing public method signature `{signature}`"));
    let tail = &source[start..];
    let end = tail
        .find("\n    }\n")
        .unwrap_or_else(|| panic!("unterminated public method `{signature}`"));
    &tail[..end + "\n    }\n".len()]
}

fn implementation_block<'a>(source: &'a str, marker: &str) -> &'a str {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("missing implementation marker `{marker}`"));
    let tail = &source[start..];
    let end = tail
        .find("\n}\n")
        .unwrap_or_else(|| panic!("unterminated implementation `{marker}`"));
    &tail[..end + "\n}\n".len()]
}

fn type_declaration<'a>(source: &'a str, marker: &str) -> &'a str {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("missing type declaration `{marker}`"));
    let tail = &source[start..];
    let braced_end = tail.find("\n}").map(|end| end + "\n}".len());
    let tuple_end = tail.find(";\n").map(|end| end + 1);
    let end = match (braced_end, tuple_end) {
        (Some(braced), Some(tuple)) => braced.min(tuple),
        (Some(braced), None) => braced,
        (None, Some(tuple)) => tuple,
        (None, None) => panic!("unterminated type declaration `{marker}`"),
    };
    &tail[..end]
}

fn concrete_store_source() -> String {
    let root = include_str!("../concrete_store.rs");
    let batch = include_str!("batch.rs");
    let validation = include_str!("validation.rs");
    let vocabulary = include_str!("vocabulary.rs");
    let terminal = include_str!("terminal_output.rs");
    format!("{root}\n{batch}\n{validation}\n{vocabulary}\n{terminal}")
}

#[test]
fn poisoned_account_rejects_before_frozen_phase_catalog_construction() {
    let factory = test_factory();
    let account = test_account(&factory);
    let _ = account.test_record_poison(AccountPoisonReason::OwnershipGraphUnverified);
    let before_account = account.snapshot();
    let (domain, keys) = test_vocabulary();
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits =
        LedgerLimits::new(u64::MAX, [(keys.dimension, policy)]).expect("test ledger limits");
    let ledger_plan =
        StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).expect("test store ledger plan");
    let start = RetainedStoreStartPlan::new(
        test_writer_plan(keys, 8, 4),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        1,
        keys.dimension,
        1,
    );
    let before_catalog_entries = crate::ledger::test_phase_catalog_construction_entries();

    let rejected =
        TestOperation::try_start(OperationControl::default(), ledger_plan, &account, start);

    assert!(matches!(
        rejected,
        Err(RetainedStoreStartError::Runtime(
            StoreRuntimeError::AccountPoisoned(_)
        ))
    ));
    assert_eq!(
        crate::ledger::test_phase_catalog_construction_entries(),
        before_catalog_entries
    );
    assert_eq!(account.snapshot(), before_account);
}

#[test]
fn public_store_start_enters_retained_allocators_after_ready_only() {
    crate::retained::reset_startup_payload_allocation_attempts();
    crate::retained::reset_retained_final_owner_allocator_entries();
    let _ = crate::retained::take_startup_ready_final_owner_allocator_entries();
    let factory = test_factory();
    let account = test_account(&factory);

    let (operation, _) = test_operation(&account, u64::MAX);

    assert_eq!(
        crate::retained::take_startup_ready_final_owner_allocator_entries(),
        Some(0),
        "the public startup seam must reach Ready before retained payload allocation"
    );
    assert_eq!(
        crate::retained::test_retained_final_owner_allocator_entries(),
        2,
        "one final-owner allocation must occur for fallback and evidence"
    );
    assert_eq!(
        crate::retained::test_startup_payload_allocation_attempts(),
        (1, 1)
    );

    drop(operation);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn public_store_start_rejection_never_enters_retained_allocator() {
    crate::retained::reset_startup_payload_allocation_attempts();
    crate::retained::reset_retained_final_owner_allocator_entries();
    let _ = crate::retained::take_startup_ready_final_owner_allocator_entries();
    let factory = test_factory();
    let account = factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(1, 1).expect("minimal retained account limits"),
        )
        .expect("minimal retained account");
    let before = account.snapshot();
    let (domain, keys) = test_vocabulary();
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits =
        LedgerLimits::new(u64::MAX, [(keys.dimension, policy)]).expect("test ledger limits");
    let ledger_plan =
        StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).expect("test store ledger plan");
    let start = RetainedStoreStartPlan::new(
        test_writer_plan(keys, 8, 4),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        1,
        keys.dimension,
        1,
    );

    let rejected =
        TestOperation::try_start(OperationControl::default(), ledger_plan, &account, start);

    assert!(rejected.is_err());
    assert_eq!(
        crate::retained::test_retained_final_owner_allocator_entries(),
        0
    );
    assert_eq!(
        crate::retained::test_startup_payload_allocation_attempts(),
        (0, 0)
    );
    assert_eq!(
        crate::retained::take_startup_ready_final_owner_allocator_entries(),
        None
    );
    assert_eq!(account.snapshot(), before);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn phase_completion_validates_domain_and_catalog_before_monotonic_progress() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (domain, keys) = test_vocabulary();
    let finalize_phase = domain
        .try_admit_phase(test_phase_semantic(2))
        .expect("finalize phase key");
    let unknown_phase = domain
        .try_admit_phase(test_phase_semantic(3))
        .expect("unknown phase key");
    let foreign_domain = StoreKeyDomain::try_new(test_catalog()).expect("foreign key domain");
    let foreign_phase = foreign_domain
        .try_admit_phase(test_phase_semantic(1))
        .expect("foreign phase key");
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits =
        LedgerLimits::new(u64::MAX, [(keys.dimension, policy)]).expect("test ledger limits");
    let ledger_plan = StoreLedgerPlan::try_new(domain, limits, &[keys.phase, finalize_phase])
        .expect("two-phase store ledger plan");
    let start = RetainedStoreStartPlan::new(
        test_writer_plan(keys, 32, 8),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        1,
        keys.dimension,
        1,
    );
    let operation =
        TestOperation::try_start(OperationControl::default(), ledger_plan, &account, start)
            .expect("test operation");

    assert_eq!(
        operation.try_complete_phase(foreign_phase),
        Err(StoreRuntimeError::ForeignPhaseDomain {
            phase: foreign_phase,
        })
    );
    assert_eq!(
        operation.try_complete_phase(unknown_phase),
        Err(StoreRuntimeError::Ledger(LedgerCause::UnknownPhase))
    );
    operation
        .try_complete_phase(keys.phase)
        .expect("build completes");
    operation
        .try_complete_phase(finalize_phase)
        .expect("finalize completes");
    operation
        .try_complete_phase(finalize_phase)
        .expect("repeating finalize is idempotent");
    assert_eq!(
        operation.try_complete_phase(keys.phase),
        Err(StoreRuntimeError::PhaseCompletionRegression {
            last_completed_phase: finalize_phase,
            phase: keys.phase,
        })
    );
    operation
        .try_complete_phase(finalize_phase)
        .expect("regression rejection preserves the completed phase");
}

#[test]
fn phase_completion_mapping_preserves_lifecycle_and_catalog_failures() {
    assert_eq!(
        map_phase_completion_error(PhaseCompletionError::<StorePhaseKey>::PublicationPending),
        StoreRuntimeError::PhaseCompletionPublicationPending
    );
    assert_eq!(
        map_phase_completion_error(PhaseCompletionError::<StorePhaseKey>::PublicationCompleted),
        StoreRuntimeError::PhaseCompletionPublicationCompleted
    );
    assert_eq!(
        map_phase_completion_error(PhaseCompletionError::<StorePhaseKey>::CatalogShapeMismatch),
        StoreRuntimeError::PhaseCompletionCatalogShapeMismatch
    );
}

#[test]
fn frozen_phase_catalog_allocation_failure_rejects_without_physical_admission() {
    let _guard = crate::ledger::test_evidence_allocation_guard();
    let factory = test_factory();
    let account = test_account(&factory);
    let (domain, keys) = test_vocabulary();
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits =
        LedgerLimits::new(u64::MAX, [(keys.dimension, policy)]).expect("test ledger limits");
    let ledger_plan =
        StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).expect("test store ledger plan");
    let start = RetainedStoreStartPlan::new(
        test_writer_plan(keys, 8, 4),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(91),
        keys.dimension,
        1,
        keys.dimension,
        1,
    );
    let before_account = account.snapshot();
    let before_factory = factory.snapshot();
    let before_catalog_entries = crate::ledger::test_phase_catalog_construction_entries();
    crate::ledger::test_fail_next_evidence_allocation();

    let rejected =
        TestOperation::try_start(OperationControl::default(), ledger_plan, &account, start);

    assert!(matches!(
        rejected,
        Err(RetainedStoreStartError::Runtime(StoreRuntimeError::Ledger(
            LedgerCause::Evidence { .. }
        )))
    ));
    assert_eq!(
        crate::ledger::test_phase_catalog_construction_entries(),
        before_catalog_entries + 1
    );
    assert_eq!(account.snapshot(), before_account);
    assert_eq!(factory.snapshot(), before_factory);
}

#[test]
fn scalar_vocabulary_is_closed_and_record_cells_are_allocation_free() {
    fn assert_scalar<T: StoredScalar>() {}
    fn assert_copy<T: Copy>() {}

    assert_scalar::<u8>();
    assert_scalar::<i8>();
    assert_scalar::<u16>();
    assert_scalar::<i16>();
    assert_scalar::<u32>();
    assert_scalar::<i32>();
    assert_scalar::<u64>();
    assert_scalar::<i64>();
    assert_scalar::<f32>();
    assert_scalar::<f64>();
    assert_copy::<RecordCell>();
}

#[test]
fn record_capacity_plan_allocator_and_physical_accounting_are_exact() {
    for (fields, desired, envelope) in [(3, 3, 4), (4, 4, 4), (5, 5, 8)] {
        let plan = RecordPlan::new(1, fields).expect("record plan");
        assert_eq!(plan.cells, desired);
        assert_eq!(plan.capacity.desired_capacity(), desired);
        assert_eq!(plan.capacity.envelope_capacity(), envelope);
        let writer = CanonicalRecordWriter::new(plan).expect("record writer");
        let actual = writer.core.storage.cells.capacity();
        assert!(
            (desired..=envelope).contains(&actual),
            "allocator capacity {actual} must stay within {desired}..={envelope}"
        );
        assert_eq!(writer.core.storage.qualified_capacity, actual);
    }

    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, u64::MAX);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 3, 1))
        .expect("record accounting assembly");
    let mut snapshots = Vec::with_capacity(4);
    snapshots.push(account.snapshot());
    for fields in [3, 4, 5] {
        let _ = assembly
            .try_retain_canonical_records(
                RecordPlan::new(1, fields).expect("record plan"),
                TestTag,
                |writer| {
                    writer.write_row(|row| {
                        for value in 0..fields {
                            row.write_scalar(u8::try_from(value).expect("small test value"))?;
                        }
                        Ok(())
                    })
                },
            )
            .expect("record block");
        snapshots.push(account.snapshot());
    }
    let records_three = test_physical_delta(snapshots[0], snapshots[1]);
    let records_four = test_physical_delta(snapshots[1], snapshots[2]);
    let records_five = test_physical_delta(snapshots[2], snapshots[3]);
    assert_eq!(records_three, records_four);
    assert_eq!(
        records_five.0 - records_four.0,
        u64::try_from(4 * std::mem::size_of::<RecordCell>()).expect("record-cell delta fits u64")
    );
    assert_eq!(records_three.1, 2);
    assert_eq!(records_four.1, 2);
    assert_eq!(records_five.1, 2);

    drop(
        assembly
            .prepare_minimal_fallback()
            .expect("record assembly fallback")
            .handoff(),
    );
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn membership_rollback_storage_failure_poisoned_the_store_and_quarantines_accounting() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, u64::MAX);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("membership rollback assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();
    let before_physical = account.snapshot();
    let before_unverified = account
        .test_unverified_drop_count()
        .expect("unverified-drop counter");

    test_fail_next_membership_assignment();
    test_panic_final_owner_drop_after(0);
    let error = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("sequence plan"),
            TestTag,
            |writer| writer.push(7),
        )
        .expect_err("membership rollback storage release must be unverified");
    assert!(matches!(error, StoreRuntimeError::StorageDropUnverified));
    assert_eq!(
        account.test_unverified_drop_count(),
        Some(before_unverified + 1)
    );
    assert_eq!(ledger.active_reservations(), before_active + 1);
    assert!(ledger.current_live_bytes() > before_live);
    let after_physical = account.snapshot();
    assert!(after_physical.live_retained_bytes() > before_physical.live_retained_bytes());
    assert!(
        after_physical.live_retained_allocations() > before_physical.live_retained_allocations()
    );
    assert!(matches!(
        assembly.try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("sequence plan"),
            TestTag,
            |writer| writer.push(8),
        ),
        Err(StoreRuntimeError::FallbackOnly)
    ));
    assert!(matches!(
        assembly.prepare_minimal_fallback(),
        Err(StorePrepareError::FinalizationContainment(cause))
            if cause.reason()
                == crate::AccountPoisonReason::RetainedStorageReleaseUnverified
    ));

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn containment_mapping_preserves_retained_charge_and_restricts_the_facade_immediately() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, u64::MAX);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("containment mapping assembly");
    let poisoned = account.test_record_poison(crate::AccountPoisonReason::OwnershipGraphUnverified);
    let error = RetainedPublicationError::Writer(RetainedWriterError::Physical(
        RetainedAllocationError::AccountPoisonedWithRetainedCharge(poisoned),
    ));

    let mapped: Result<(), StoreRuntimeError> = assembly.map_action(Err(error));
    assert!(matches!(
        mapped,
        Err(StoreRuntimeError::AccountPoisonedWithRetainedCharge(cause))
            if cause == poisoned
    ));
    assert!(assembly.is_fallback_only());
    assert!(matches!(
        assembly.ensure_writable(),
        Err(StoreRuntimeError::FallbackOnly)
    ));
    assert!(matches!(
        assembly.try_control_checkpoint(),
        Err(StoreRuntimeError::FallbackOnly)
    ));
    assert!(publication_error_requires_containment(
        &RetainedPublicationError::<StoreDimensionKey>::Writer(
            RetainedWriterError::AccountPoisoned(poisoned)
        )
    ));
}

#[test]
fn builder_panic_retains_physical_and_operation_charges_fail_closed() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, u64::MAX);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("builder-panic assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();
    let before_physical = account.snapshot();

    let error = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("sequence plan"),
            TestTag,
            |_writer| -> Result<(), StoreBuildError> { panic!("contained concrete builder panic") },
        )
        .expect_err("builder panic must reach the raw fatal containment seam");

    assert!(matches!(error, StoreRuntimeError::BuilderPanicked));
    assert!(ledger.active_reservations() > before_active);
    assert!(ledger.current_live_bytes() > before_live);
    let after_physical = account.snapshot();
    assert!(after_physical.live_retained_bytes() > before_physical.live_retained_bytes());
    assert!(
        after_physical.live_retained_allocations() > before_physical.live_retained_allocations()
    );
    assert_eq!(
        after_physical
            .first_poison_evidence()
            .map(crate::AccountPoisonEvidence::reason),
        Some(crate::AccountPoisonReason::RetainedBuilderPanicked)
    );
    assert!(matches!(
        assembly.prepare_minimal_fallback(),
        Err(StorePrepareError::FinalizationContainment(cause))
            if cause.reason() == crate::AccountPoisonReason::RetainedBuilderPanicked
    ));

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn concrete_builder_callbacks_use_only_the_raw_fatal_panic_boundary() {
    let source = include_str!("../concrete_store.rs");
    let start = source
        .find("fn finish_scoped_writer<Writer>(")
        .expect("scoped-writer helper");
    let end = source[start..]
        .find("\nenum SequenceStorage")
        .map(|offset| start + offset)
        .expect("scoped-writer helper boundary");
    let helper = &source[start..end];

    assert!(helper.contains("build(&mut writer)?;"));
    for forbidden in ["catch_unwind", "builder_panicked", "BuilderPanicked"] {
        assert!(
            !helper.contains(forbidden),
            "concrete builder helper must not recover fatal panic token `{forbidden}`"
        );
    }
}

#[test]
fn clean_membership_rejection_releases_storage_and_attribution_once() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, u64::MAX);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("clean membership rollback assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();
    let before_physical = account.snapshot();
    let before_faults = account
        .test_unverified_drop_count()
        .expect("account fault counter");

    test_fail_next_membership_assignment();
    let error = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("sequence plan"),
            TestTag,
            |writer| writer.push(7),
        )
        .expect_err("membership invariant must reject the new block");

    assert!(matches!(error, StoreRuntimeError::OwnershipUnverified));
    assert_eq!(ledger.active_reservations(), before_active);
    assert_eq!(ledger.current_live_bytes(), before_live);
    let after_physical = account.snapshot();
    assert_eq!(
        after_physical.live_retained_bytes(),
        before_physical.live_retained_bytes()
    );
    assert_eq!(
        after_physical.live_retained_allocations(),
        before_physical.live_retained_allocations()
    );
    assert_eq!(
        account.test_unverified_drop_count(),
        Some(before_faults + 1)
    );
    assert!(matches!(
        assembly.try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("sequence plan"),
            TestTag,
            |writer| writer.push(8),
        ),
        Err(StoreRuntimeError::FallbackOnly)
    ));
}

#[test]
fn primary_rejection_dependency_unwind_stops_before_attribution_ack() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, 4);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 2, 1))
        .expect("primary rejection assembly");
    let leaf = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("sequence plan"),
            TestTag,
            |writer| writer.push(7),
        )
        .expect("canonical leaf");
    let canonical_root = assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, 1).expect("record plan"),
            TestRootTag,
            |writer| writer.write_row(|row| row.write_sequence_ref(leaf)),
        )
        .expect("canonical root");
    let evidence_root = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("sequence plan"),
            TestTag,
            |writer| writer.push(9),
        )
        .expect("evidence root");
    assembly
        .try_commit_outcome_evidence(keys.dimension, 4)
        .expect("primary commitment");
    let ledger = assembly.raw.test_ledger_probe();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();
    let before_physical = account.snapshot();
    let before_unverified = account
        .test_unverified_drop_count()
        .expect("unverified-drop counter");
    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical_root, evidence_root| {
            let Some(root) = session.canonical_records(canonical_root) else {
                return false;
            };
            let Some(leaf) = root.sequence_ref::<u8, TestTag>(0, 0) else {
                return false;
            };
            session.canonical_sequence(leaf).is_some()
                && session.outcome_evidence_sequence(evidence_root).is_some()
        },
    ) {
        StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("primary rejection graph must validate"),
    };

    test_panic_final_owner_drop_after(1);
    let result = validated.try_prepare(test_final_progress());
    assert!(matches!(
        result,
        Err(StorePrepareError::FinalizationContainment(cause))
            if cause.reason() == crate::AccountPoisonReason::RetainedStorageReleaseUnverified
    ));
    assert!(
        account
            .test_unverified_drop_count()
            .is_some_and(|count| count > before_unverified)
    );
    assert_eq!(ledger.active_reservations(), before_active);
    assert_eq!(ledger.current_live_bytes(), before_live);
    let after_physical = account.snapshot();
    assert!(after_physical.live_retained_bytes() > 0);
    assert!(after_physical.live_retained_bytes() < before_physical.live_retained_bytes());
    assert!(after_physical.live_retained_allocations() > 0);
    assert!(
        after_physical.live_retained_allocations() < before_physical.live_retained_allocations()
    );
    assert_eq!(
        after_physical.total_admitted_retained_bytes(),
        before_physical.total_admitted_retained_bytes()
    );
    assert_eq!(
        after_physical.total_admitted_retained_allocations(),
        before_physical.total_admitted_retained_allocations()
    );
    assert!(after_physical.poisoned());
    assert!(after_physical.poison_event_count() > 0);
    assert!(after_physical.poison_event_count_exact());
    assert!(after_physical.poison_event_domain_valid());
    assert_eq!(
        after_physical
            .first_poison_evidence()
            .map(|evidence| evidence.reason()),
        Some(crate::AccountPoisonReason::RetainedStorageReleaseUnverified)
    );

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn controlled_builders_and_validation_poll_and_charge_exact_work() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, 16);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 2, 1))
        .expect("controlled assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let leaf = assembly
        .try_retain_canonical_sequence_controlled(
            SequencePlan::<u8>::new(1).expect("sequence plan"),
            keys.work_dimension,
            TestTag,
            |writer, control| {
                control.poll()?;
                control.charge_observed(2)?;
                writer.push(7)?;
                Ok(())
            },
        )
        .expect("controlled sequence");
    let canonical_root = assembly
        .try_retain_canonical_records_controlled(
            RecordPlan::new(1, 1).expect("record plan"),
            keys.work_dimension,
            TestRootTag,
            |writer, control| {
                control.poll()?;
                control.charge_observed(3)?;
                writer.write_row(|row| row.write_sequence_ref(leaf))?;
                Ok(())
            },
        )
        .expect("controlled records");
    let evidence_root = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("evidence plan"),
            TestTag,
            |writer| writer.push(9),
        )
        .expect("evidence root");
    let validated = assembly.validate_success_controlled(
        keys.work_dimension,
        canonical_root,
        evidence_root,
        |session, canonical_root, evidence_root, control| {
            control.poll()?;
            control.charge_observed(5)?;
            let Some(root) = session.canonical_records(canonical_root) else {
                return Ok(false);
            };
            let Some(leaf) = root.sequence_ref::<u8, TestTag>(0, 0) else {
                return Ok(false);
            };
            Ok(session.canonical_sequence(leaf).is_some()
                && session.outcome_evidence_sequence(evidence_root).is_some())
        },
    );
    assert!(matches!(
        validated,
        StoreControlledSuccessValidation::Valid(_)
    ));
    let dimension = ledger
        .dimension_evidence(keys.work_dimension)
        .expect("work dimension evidence");
    assert_eq!(dimension.attempted(), 10);
    assert_eq!(dimension.consumed(), 10);
}

#[test]
fn controlled_bytes_and_utf8_write_exact_payloads_and_charge_exact_work() {
    const BINARY_PAYLOAD: &[u8] = b"\0\x01\x02\x03\x04\x05";
    const TEXT_PAYLOAD: &str = "baozi!";

    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, 16);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 3, 1))
        .expect("controlled payload assembly");
    let ledger = assembly.raw.test_ledger_probe();

    let before_binary = account.snapshot();
    let binary = assembly
        .try_retain_canonical_bytes_controlled(
            BytesPlan::new(BINARY_PAYLOAD.len()).expect("binary plan"),
            keys.work_dimension,
            TestTag,
            |writer, control| {
                control.poll()?;
                control.charge_observed(
                    u64::try_from(BINARY_PAYLOAD.len()).expect("binary length fits u64"),
                )?;
                writer.write_bytes(BINARY_PAYLOAD)?;
                Ok(())
            },
        )
        .expect("controlled binary block");
    let after_binary = account.snapshot();

    let text = assembly
        .try_retain_canonical_utf8_controlled(
            Utf8Plan::new(TEXT_PAYLOAD.len()).expect("UTF-8 plan"),
            keys.work_dimension,
            TestTag,
            |writer, control| {
                control.poll()?;
                control.charge_observed(
                    u64::try_from(TEXT_PAYLOAD.len()).expect("UTF-8 length fits u64"),
                )?;
                writer.write_str(TEXT_PAYLOAD)?;
                Ok(())
            },
        )
        .expect("controlled UTF-8 block");
    let after_text = account.snapshot();

    let canonical_root = assembly
        .try_retain_canonical_records(
            RecordPlan::new(1, 2).expect("canonical root plan"),
            TestRootTag,
            |writer| {
                writer.write_row(|row| {
                    row.write_bytes_ref(binary)?;
                    row.write_utf8_ref(text)
                })
            },
        )
        .expect("canonical root");
    let evidence_root = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("evidence plan"),
            TestTag,
            |writer| writer.push(9),
        )
        .expect("evidence root");

    assert!(matches!(
        assembly.validate_success(
            canonical_root,
            evidence_root,
            |session, canonical_root, evidence_root| {
                let Some(root) = session.canonical_records(canonical_root) else {
                    return false;
                };
                let Some(binary) = root.bytes_ref::<TestTag>(0, 0) else {
                    return false;
                };
                let Some(text) = root.utf8_ref::<TestTag>(0, 1) else {
                    return false;
                };
                session.canonical_bytes(binary) == Some(BINARY_PAYLOAD)
                    && session.canonical_utf8(text) == Some(TEXT_PAYLOAD)
                    && session.outcome_evidence_sequence(evidence_root).is_some()
            },
        ),
        StoreSuccessValidation::Valid(_)
    ));

    let binary_charge = test_physical_delta(before_binary, after_binary);
    let text_charge = test_physical_delta(after_binary, after_text);
    assert_eq!(binary_charge, text_charge);
    assert_eq!(binary_charge.1, 2);
    let dimension = ledger
        .dimension_evidence(keys.work_dimension)
        .expect("work dimension evidence");
    assert_eq!(dimension.attempted(), 12);
    assert_eq!(dimension.consumed(), 12);
}

#[test]
fn controlled_bytes_map_work_rejection_like_controlled_sequences() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation_with_control_and_work_limits(
        &account,
        16,
        OperationControl::default(),
        0,
        0,
    );
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("work-rejection assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before_physical = account.snapshot();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();

    let error = assembly
        .try_retain_canonical_bytes_controlled(
            BytesPlan::new(1).expect("binary plan"),
            keys.work_dimension,
            TestTag,
            |writer, control| {
                control.charge_observed(1)?;
                writer.write_bytes(&[7])?;
                Ok(())
            },
        )
        .expect_err("work budget must reject controlled bytes");

    assert!(matches!(error, StoreRuntimeError::Ledger(_)));
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(ledger.active_reservations(), before_active);
    assert_eq!(ledger.current_live_bytes(), before_live);
    let after_physical = account.snapshot();
    assert_eq!(
        after_physical.live_retained_bytes(),
        before_physical.live_retained_bytes()
    );
    assert_eq!(
        after_physical.live_retained_allocations(),
        before_physical.live_retained_allocations()
    );
    assert!(matches!(
        assembly.try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("retry sequence plan"),
            TestTag,
            |writer| writer.push(8),
        ),
        Err(StoreRuntimeError::SuccessOutcomeRejected)
    ));
}

#[test]
fn controlled_utf8_maps_cancellation_like_controlled_sequences() {
    let factory = test_factory();
    let account = test_account(&factory);
    let cancellation = crate::CancellationToken::new();
    let (operation, keys) = test_operation_with_control_and_work_limits(
        &account,
        16,
        OperationControl::default().with_cancellation(cancellation.clone()),
        u64::MAX,
        u64::MAX,
    );
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("cancellation assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before_physical = account.snapshot();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();
    cancellation.cancel();

    let error = assembly
        .try_retain_canonical_utf8_controlled(
            Utf8Plan::new(1).expect("UTF-8 plan"),
            keys.work_dimension,
            TestTag,
            |writer, control| {
                control.poll()?;
                writer.write_str("x")?;
                Ok(())
            },
        )
        .expect_err("cancellation must reject controlled UTF-8");

    assert!(matches!(error, StoreRuntimeError::Control(_)));
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(ledger.active_reservations(), before_active);
    assert_eq!(ledger.current_live_bytes(), before_live);
    let after_physical = account.snapshot();
    assert_eq!(
        after_physical.live_retained_bytes(),
        before_physical.live_retained_bytes()
    );
    assert_eq!(
        after_physical.live_retained_allocations(),
        before_physical.live_retained_allocations()
    );
    assert!(matches!(
        assembly.try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("retry sequence plan"),
            TestTag,
            |writer| writer.push(8),
        ),
        Err(StoreRuntimeError::SuccessOutcomeRejected)
    ));
}

#[test]
fn controlled_bytes_underwrite_rolls_back_and_remains_retryable() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, 16);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("binary retry assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before_physical = account.snapshot();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();

    let error = assembly
        .try_retain_canonical_bytes_controlled(
            BytesPlan::new(3).expect("underwritten binary plan"),
            keys.work_dimension,
            TestTag,
            |writer, _| {
                writer.write_bytes(&[1, 2])?;
                Ok(())
            },
        )
        .expect_err("underwritten binary block must fail");

    assert_eq!(
        error,
        StoreRuntimeError::Build(StoreBuildError::incomplete_write())
    );
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(ledger.active_reservations(), before_active);
    assert_eq!(ledger.current_live_bytes(), before_live);
    let after_physical = account.snapshot();
    assert_eq!(
        after_physical.live_retained_bytes(),
        before_physical.live_retained_bytes()
    );
    assert_eq!(
        after_physical.live_retained_allocations(),
        before_physical.live_retained_allocations()
    );
    assert!(!assembly.is_fallback_only());

    let _ = assembly
        .try_retain_canonical_bytes_controlled(
            BytesPlan::new(3).expect("retry binary plan"),
            keys.work_dimension,
            TestTag,
            |writer, control| {
                control.poll()?;
                writer.write_bytes(&[1, 2, 3])?;
                Ok(())
            },
        )
        .expect("binary retry succeeds");
    assert_eq!(assembly.canonical_len(), 1);
}

#[test]
fn controlled_utf8_rejects_non_fitting_multibyte_text_and_remains_retryable() {
    const MULTIBYTE_TEXT: &str = "\u{e9}";

    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, 16);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("UTF-8 retry assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before_physical = account.snapshot();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();

    let error = assembly
        .try_retain_canonical_utf8_controlled(
            Utf8Plan::new(1).expect("undersized UTF-8 plan"),
            keys.work_dimension,
            TestTag,
            |writer, _| {
                writer.write_str(MULTIBYTE_TEXT)?;
                Ok(())
            },
        )
        .expect_err("multi-byte text must not exceed its encoded-byte plan");

    assert_eq!(
        error,
        StoreRuntimeError::Build(StoreBuildError::capacity_exceeded())
    );
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(ledger.active_reservations(), before_active);
    assert_eq!(ledger.current_live_bytes(), before_live);
    let after_physical = account.snapshot();
    assert_eq!(
        after_physical.live_retained_bytes(),
        before_physical.live_retained_bytes()
    );
    assert_eq!(
        after_physical.live_retained_allocations(),
        before_physical.live_retained_allocations()
    );
    assert!(!assembly.is_fallback_only());

    let _ = assembly
        .try_retain_canonical_utf8_controlled(
            Utf8Plan::new(MULTIBYTE_TEXT.len()).expect("retry UTF-8 plan"),
            keys.work_dimension,
            TestTag,
            |writer, control| {
                control.poll()?;
                writer.write_str(MULTIBYTE_TEXT)?;
                Ok(())
            },
        )
        .expect("UTF-8 retry succeeds");
    assert_eq!(assembly.canonical_len(), 1);
}

#[test]
fn controlled_builder_domain_rejection_rolls_back_and_remains_retryable() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, 16);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("controlled retry assembly");
    let rejection = StoreBuildError::domain(7).expect("test domain code");
    let error = assembly
        .try_retain_canonical_sequence_controlled(
            SequencePlan::<u8>::new(1).expect("rejected sequence plan"),
            keys.work_dimension,
            TestTag,
            |_, _| Err(rejection.into()),
        )
        .expect_err("domain builder rejection");
    assert_eq!(error, StoreRuntimeError::Build(rejection));
    assert_eq!(assembly.canonical_len(), 0);
    assert!(!assembly.is_fallback_only());

    let canonical_root = assembly
        .try_retain_canonical_sequence_controlled(
            SequencePlan::<u8>::new(1).expect("retry sequence plan"),
            keys.work_dimension,
            TestRootTag,
            |writer, control| {
                control.poll()?;
                writer.push(7)?;
                Ok(())
            },
        )
        .expect("retry succeeds");
    let evidence_root = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("evidence plan"),
            TestTag,
            |writer| writer.push(9),
        )
        .expect("evidence root");
    assert!(matches!(
        assembly.validate_success(
            canonical_root,
            evidence_root,
            |session, canonical_root, evidence_root| {
                session.canonical_sequence(canonical_root).is_some()
                    && session.outcome_evidence_sequence(evidence_root).is_some()
            },
        ),
        StoreSuccessValidation::Valid(_)
    ));
}

#[test]
fn ignored_control_error_is_sticky_and_permanently_rejects_success() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation_with_control_and_work_limits(
        &account,
        16,
        OperationControl::default(),
        0,
        0,
    );
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 2, 1))
        .expect("sticky assembly");
    let canonical_root = assembly
        .try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("canonical root plan"),
            TestRootTag,
            |writer| writer.push(7),
        )
        .expect("complete canonical root");
    let evidence_root = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("evidence sequence plan"),
            TestTag,
            |writer| writer.push(9),
        )
        .expect("neutral evidence");
    let ledger = assembly.raw.test_ledger_probe();
    let before_physical = account.snapshot();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();
    let error = assembly
        .try_retain_canonical_sequence_controlled(
            SequencePlan::<u8>::new(1).expect("sequence plan"),
            keys.work_dimension,
            TestTag,
            |writer, control| {
                let _ = control.charge_observed(1);
                writer.push(7)?;
                Ok(())
            },
        )
        .expect_err("ignored budget error remains sticky");
    assert!(matches!(error, StoreRuntimeError::Ledger(_)));
    assert_eq!(assembly.canonical_len(), 1);
    assert_eq!(ledger.active_reservations(), before_active);
    assert_eq!(ledger.current_live_bytes(), before_live);
    let after_physical = account.snapshot();
    assert_eq!(
        after_physical.live_retained_bytes(),
        before_physical.live_retained_bytes()
    );
    assert_eq!(
        after_physical.live_retained_allocations(),
        before_physical.live_retained_allocations()
    );
    assert!(matches!(
        assembly.try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("retry sequence plan"),
            TestTag,
            |writer| writer.push(8),
        ),
        Err(StoreRuntimeError::SuccessOutcomeRejected)
    ));
    assert!(matches!(
        assembly.try_control_checkpoint(),
        Err(StoreRuntimeError::SuccessOutcomeRejected)
    ));
    let failure = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical_root, evidence_root| {
            session.canonical_sequence(canonical_root).is_some()
                && session.outcome_evidence_sequence(evidence_root).is_some()
        },
    ) {
        StoreSuccessValidation::Rejected(failure) => failure,
        _ => panic!("a prior budget rejection must seal an otherwise complete success graph"),
    };
    let validated = match failure.validate_rich_failure(evidence_root, |session, root| {
        session.sequence(root).is_some()
    }) {
        StoreFailureValidation::Valid(validated) => validated,
        _ => panic!("budget rejection evidence must validate"),
    };
    let header = failure_outcome_header(
        validated
            .try_prepare(0xdead_beef, test_final_progress())
            .expect("budget rejection rich failure"),
    );
    let StoreTerminalCause::Failure(cause) = header.terminal_cause() else {
        panic!("budget rejection must freeze a failure cause");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Ledger);
    assert_eq!(cause.code(), 1);
}

#[test]
fn direct_rich_failure_preserves_work_cause_across_rejection_and_fallback() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation_with_control_and_work_limits(
        &account,
        16,
        OperationControl::default(),
        0,
        0,
    );
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 2))
        .expect("direct rich-failure assembly");
    let evidence_root = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("evidence plan"),
            TestTag,
            |writer| writer.push(9),
        )
        .expect("evidence root");
    let evidence_detail = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("evidence detail plan"),
            TestRootTag,
            |writer| writer.push(10),
        )
        .expect("evidence detail");
    let error = assembly
        .try_retain_canonical_sequence_controlled(
            SequencePlan::<u8>::new(1).expect("canonical plan"),
            keys.work_dimension,
            TestRootTag,
            |_, control| {
                control.charge_observed(1)?;
                Ok(())
            },
        )
        .expect_err("work budget must reject canonical construction");
    assert!(matches!(error, StoreRuntimeError::Ledger(_)));

    let failure = match assembly.validate_rich_failure(evidence_root, |session, root| {
        let _ = session.sequence(root);
        false
    }) {
        StoreFailureValidation::Rejected(failure) => failure,
        _ => panic!("first rich-failure validation must reject semantically"),
    };
    let validated = match failure.validate_rich_failure(evidence_root, |session, root| {
        session.sequence(root).is_some() && session.sequence(evidence_detail).is_some()
    }) {
        StoreFailureValidation::Valid(validated) => validated,
        _ => panic!("second rich-failure validation must use a fresh epoch"),
    };
    let published = validated
        .prepare_minimal_fallback()
        .expect("forced-cause fallback")
        .handoff();
    let StoreTerminalCause::Failure(cause) = published.ledger_evidence.header().terminal_cause()
    else {
        panic!("forced work cause must survive rich-failure fallback");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Ledger);
    assert_eq!(cause.code(), 1);
}

#[test]
fn controlled_error_from_another_operation_is_not_a_verified_callback_cause() {
    let first_factory = test_factory();
    let first_account = test_account(&first_factory);
    let (first_operation, first_keys) = test_operation_with_control_and_work_limits(
        &first_account,
        16,
        OperationControl::default(),
        0,
        0,
    );
    let mut first_assembly = first_operation
        .try_publication_assembly(test_publication_plan(first_keys, 1, 1))
        .expect("first assembly");
    let mut captured = None;
    let first_error = first_assembly
        .try_retain_canonical_sequence_controlled(
            SequencePlan::<u8>::new(1).expect("first sequence plan"),
            first_keys.work_dimension,
            TestTag,
            |_, control| {
                let cause = control
                    .charge_observed(1)
                    .expect_err("first work limit must reject");
                captured = Some(cause.clone());
                Err(cause.into())
            },
        )
        .expect_err("first controlled block must be rejected");
    assert!(matches!(first_error, StoreRuntimeError::Ledger(_)));

    let second_factory = test_factory();
    let second_account = test_account(&second_factory);
    let (second_operation, second_keys) = test_operation(&second_account, 16);
    let mut second_assembly = second_operation
        .try_publication_assembly(test_publication_plan(second_keys, 1, 1))
        .expect("second assembly");
    let replay_error = second_assembly
        .try_retain_canonical_sequence_controlled(
            SequencePlan::<u8>::new(1).expect("second sequence plan"),
            second_keys.work_dimension,
            TestTag,
            |_, _| Err(captured.take().expect("captured controlled cause").into()),
        )
        .expect_err("foreign controlled cause must fail closed");
    assert!(matches!(
        replay_error,
        StoreRuntimeError::UnverifiedControlledCause
    ));
    assert!(matches!(
        second_assembly.try_retain_canonical_sequence(
            SequencePlan::<u8>::new(1).expect("ordinary sequence plan"),
            TestTag,
            |writer| writer.push(7),
        ),
        Err(StoreRuntimeError::FallbackOnly)
    ));
    let published = second_assembly
        .prepare_minimal_fallback()
        .expect("unverified-cause fallback")
        .handoff();
    let StoreTerminalCause::Failure(cause) = published.ledger_evidence.header().terminal_cause()
    else {
        panic!("unverified controlled cause must freeze a failure cause");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
    assert_eq!(cause.code(), UNVERIFIED_CONTROLLED_CAUSE_FALLBACK_CODE);
}

#[test]
fn controlled_validation_rejects_a_work_cause_from_another_operation() {
    let first_factory = test_factory();
    let first_account = test_account(&first_factory);
    let (first_operation, first_keys) = test_operation_with_control_and_work_limits(
        &first_account,
        16,
        OperationControl::default(),
        0,
        0,
    );
    let mut first_assembly = first_operation
        .try_publication_assembly(test_publication_plan(first_keys, 1, 1))
        .expect("first assembly");
    let mut captured = None;
    let first_error = first_assembly
        .try_retain_canonical_sequence_controlled(
            SequencePlan::<u8>::new(1).expect("first sequence plan"),
            first_keys.work_dimension,
            TestTag,
            |_, control| {
                let cause = control
                    .charge_observed(1)
                    .expect_err("first work limit must reject");
                captured = Some(cause.clone());
                Err(cause.into())
            },
        )
        .expect_err("first controlled block must be rejected");
    assert!(matches!(first_error, StoreRuntimeError::Ledger(_)));
    drop(first_assembly);

    let second_factory = test_factory();
    let second_account = test_account(&second_factory);
    let (second_operation, second_keys) = test_operation(&second_account, 16);
    let mut second_assembly = second_operation
        .try_publication_assembly(test_publication_plan(second_keys, 1, 1))
        .expect("second assembly");
    let (canonical_root, evidence_root) =
        test_simple_controlled_validation_graph(&mut second_assembly);
    let fallback = match second_assembly.validate_success_controlled(
        second_keys.work_dimension,
        canonical_root,
        evidence_root,
        |_, _, _, _| Err(captured.take().expect("captured work cause")),
    ) {
        StoreControlledSuccessValidation::FallbackOnly(fallback) => fallback,
        _ => panic!("a foreign validator work cause must fail closed"),
    };
    let published = fallback
        .prepare_minimal_fallback()
        .expect("unverified validator fallback")
        .handoff();
    let StoreTerminalCause::Failure(cause) = published.ledger_evidence.header().terminal_cause()
    else {
        panic!("unverified validator cause must freeze a failure cause");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Requested);
    assert_eq!(cause.code(), UNVERIFIED_CONTROLLED_CAUSE_FALLBACK_CODE);
}

#[test]
fn cancellation_interruption_preserves_rich_failure_owner() {
    let factory = test_factory();
    let account = test_account(&factory);
    let cancellation = crate::CancellationToken::new();
    let (operation, keys) = test_operation_with_control_and_work_limits(
        &account,
        16,
        OperationControl::default().with_cancellation(cancellation.clone()),
        u64::MAX,
        u64::MAX,
    );
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("cancellation assembly");
    let (canonical_root, evidence_root) = test_simple_controlled_validation_graph(&mut assembly);
    cancellation.cancel();
    let (failure, cause) = match assembly.validate_success_controlled(
        keys.work_dimension,
        canonical_root,
        evidence_root,
        |_, _, _, control| {
            control.poll()?;
            Ok(true)
        },
    ) {
        StoreControlledSuccessValidation::Interrupted { failure, cause } => (failure, cause),
        _ => panic!("cancellation must interrupt without poisoning"),
    };
    assert!(cause.control_cause().is_some());
    let validated = match failure.validate_rich_failure(evidence_root, |session, root| {
        session.sequence(root).is_some()
    }) {
        StoreFailureValidation::Valid(validated) => validated,
        _ => panic!("cancellation evidence must validate"),
    };
    let header = failure_outcome_header(
        validated
            .try_prepare(0xdead_beef, test_final_progress())
            .expect("cancellation rich failure"),
    );
    let StoreTerminalCause::Failure(cause) = header.terminal_cause() else {
        panic!("cancellation must freeze a failure cause");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Control);
}

#[test]
fn budget_interruption_preserves_rich_failure_owner() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation_with_control_and_work_limits(
        &account,
        16,
        OperationControl::default(),
        0,
        0,
    );
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("budget assembly");
    let (canonical_root, evidence_root) = test_simple_controlled_validation_graph(&mut assembly);
    let (failure, cause) = match assembly.validate_success_controlled(
        keys.work_dimension,
        canonical_root,
        evidence_root,
        |_, _, _, control| {
            control.charge_observed(1)?;
            Ok(true)
        },
    ) {
        StoreControlledSuccessValidation::Interrupted { failure, cause } => (failure, cause),
        _ => panic!("budget rejection must interrupt without poisoning"),
    };
    assert!(cause.ledger_cause().is_some());
    let validated = match failure.validate_rich_failure(evidence_root, |session, root| {
        session.sequence(root).is_some()
    }) {
        StoreFailureValidation::Valid(validated) => validated,
        _ => panic!("budget evidence must validate"),
    };
    let header = failure_outcome_header(
        validated
            .try_prepare(0xdead_beef, test_final_progress())
            .expect("budget rich failure"),
    );
    let StoreTerminalCause::Failure(cause) = header.terminal_cause() else {
        panic!("budget interruption must freeze a failure cause");
    };
    assert_eq!(cause.class(), StorePreparationFailureClass::Ledger);
}

#[test]
fn invalid_controlled_validation_dimension_returns_the_writable_assembly() {
    let first_factory = test_factory();
    let first_account = test_account(&first_factory);
    let second_factory = test_factory();
    let second_account = test_account(&second_factory);
    let (first_operation, first_keys) = test_operation(&first_account, 16);
    let (_second_operation, second_keys) = test_operation(&second_account, 16);
    let mut assembly = first_operation
        .try_publication_assembly(test_publication_plan(first_keys, 1, 1))
        .expect("first assembly");
    let (canonical_root, evidence_root) = test_simple_controlled_validation_graph(&mut assembly);

    let (assembly, cause) = match assembly.validate_success_controlled(
        second_keys.work_dimension,
        canonical_root,
        evidence_root,
        |_, _, _, _| Ok(true),
    ) {
        StoreControlledSuccessValidation::InvalidWorkDimension { assembly, cause } => {
            (assembly, cause)
        }
        _ => panic!("a foreign work dimension must return the untouched assembly"),
    };
    assert!(matches!(
        cause,
        StoreRuntimeError::ForeignDimensionDomain { .. }
    ));
    assert!(matches!(
        assembly.validate_success_controlled(
            first_keys.work_dimension,
            canonical_root,
            evidence_root,
            |session, canonical_root, evidence_root, _| {
                Ok(session.canonical_sequence(canonical_root).is_some()
                    && session.outcome_evidence_sequence(evidence_root).is_some())
            },
        ),
        StoreControlledSuccessValidation::Valid(_)
    ));
}

#[test]
fn controlled_validation_distinguishes_semantic_rejection_from_panic() {
    {
        let factory = test_factory();
        let account = test_account(&factory);
        let (operation, keys) = test_operation(&account, 16);
        let mut assembly = operation
            .try_publication_assembly(test_publication_plan(keys, 1, 1))
            .expect("semantic assembly");
        let (canonical_root, evidence_root) =
            test_simple_controlled_validation_graph(&mut assembly);
        assert!(matches!(
            assembly.validate_success_controlled(
                keys.work_dimension,
                canonical_root,
                evidence_root,
                |_, _, _, _| Ok(false),
            ),
            StoreControlledSuccessValidation::Rejected(_)
        ));
    }
    {
        let factory = test_factory();
        let account = test_account(&factory);
        let (operation, keys) = test_operation(&account, 16);
        let mut assembly = operation
            .try_publication_assembly(test_publication_plan(keys, 1, 1))
            .expect("panic assembly");
        let (canonical_root, evidence_root) =
            test_simple_controlled_validation_graph(&mut assembly);
        assert!(matches!(
            assembly.validate_success_controlled(
                keys.work_dimension,
                canonical_root,
                evidence_root,
                |_, _, _, _| -> Result<bool, StoreWorkError> {
                    panic!("controlled validator panic")
                },
            ),
            StoreControlledSuccessValidation::FallbackOnly(_)
        ));
    }
}

#[test]
fn public_store_surface_does_not_export_raw_or_generic_owning_authority() {
    let root = include_str!("../lib.rs");
    let source = include_str!("../concrete_store.rs");
    let terminal = include_str!("terminal_output.rs");
    for forbidden in [
        "RetainedAllocation",
        "RetainedAllocationPlan",
        "BlockEmplacer",
        "StoredBlock,",
    ] {
        assert!(
            !root.contains(forbidden),
            "raw retained authority `{forbidden}` escaped the crate root"
        );
    }
    assert!(source.contains("pub trait StoredScalar: sealed::StoredScalar {}"));
    assert!(!source.contains("pub trait StoredScalar:\n"));
    assert!(!root.contains("CanonicalBlockRef"));
    assert!(!root.contains("OutcomeEvidenceBlockRef"));
    assert!(!terminal.contains("pub(super) raw:"));
}

#[test]
fn public_store_reference_and_range_brands_are_invariant() {
    let root = include_str!("../concrete_store.rs");
    let batch = include_str!("batch.rs");

    assert!(root.contains("pub struct ScalarSequence<T: StoredScalar>(PhantomData<fn(T) -> T>);"));
    assert!(root.contains("marker: PhantomData<fn(Kind) -> Kind>"));
    assert!(batch.contains("marker: PhantomData<fn(Kind) -> Kind>"));
    assert!(!root.contains("marker: PhantomData<fn() -> Kind>"));
    assert!(!batch.contains("marker: PhantomData<fn() -> Kind>"));
}

#[test]
fn retained_store_control_checkpoint_is_narrow_and_validates_keys_before_forwarding() {
    let source = include_str!("../concrete_store.rs");
    let declaration = type_declaration(source, "pub struct RetainedStoreControlCheckpoint");
    let reservation_declaration = type_declaration(source, "pub struct RetainedStoreReservation");
    let implementation =
        implementation_block(source, "impl<O, PP, U> RetainedStoreControlCheckpoint");

    assert!(!declaration.contains("pub raw:"));
    assert!(!reservation_declaration.contains("pub raw:"));
    assert!(!declaration.contains("OperationCheckpoint"));
    assert!(!implementation.contains("LiveReservation<"));
    for required in [
        "pub fn poll(",
        "pub fn charge_observed(",
        "pub fn reserve(",
        "pub fn resize(",
        "pub fn transition(",
        "pub fn validate_live_owner(",
    ] {
        assert!(
            implementation.contains(required),
            "checkpoint surface is missing `{required}`"
        );
    }
    for forbidden in [
        "report_progress",
        "prepare_publication",
        "try_publication_assembly",
        "canonical",
        "outcome_evidence",
        "pub fn split(",
        "pub fn transfer(",
        "pub fn charge_attempted(",
        "pub fn charge_consumed(",
    ] {
        assert!(
            !implementation.contains(forbidden),
            "checkpoint regained excluded authority through `{forbidden}`"
        );
    }

    let reserve = public_method_block(source, "pub fn reserve(");
    let phase_validation = reserve
        .find("self.validate_phase(phase)?")
        .expect("reserve validates its phase");
    let dimension_validation = reserve
        .find("self.validate_dimension(dimension)?")
        .expect("reserve validates its dimension");
    let forwarding = reserve
        .find("self.raw")
        .expect("reserve forwards only after key validation");
    assert!(phase_validation < dimension_validation && dimension_validation < forwarding);
}

#[test]
fn retained_store_work_control_exposes_only_poll_and_prebound_work_charge() {
    let source = include_str!("../concrete_store.rs");
    let declaration = type_declaration(source, "pub struct RetainedStoreWorkControl");
    let implementation = implementation_block(source, "impl<O, PP, U> RetainedStoreWorkControl");
    assert!(!declaration.contains("pub raw:"));
    assert!(!declaration.contains("pub sticky:"));
    for required in ["pub fn poll(", "pub fn charge_observed("] {
        assert!(
            implementation.contains(required),
            "work control is missing `{required}`"
        );
    }
    for forbidden in [
        "pub fn reserve(",
        "pub fn resize(",
        "pub fn transition(",
        "pub fn report_progress(",
        "pub fn try_commit",
        "pub fn try_retain",
        "pub fn validate_",
        "pub fn prepare_",
        "pub fn publish",
    ] {
        assert!(
            !implementation.contains(forbidden),
            "work control regained excluded authority through `{forbidden}`"
        );
    }
}

#[test]
fn retained_store_control_checkpoint_enforces_domain_catalog_and_live_owner() {
    let first_factory = test_factory();
    let first_account = test_account(&first_factory);
    let second_factory = test_factory();
    let second_account = test_account(&second_factory);
    let (mut first, first_keys) = test_operation(&first_account, u64::MAX);
    let (mut second, second_keys) = test_operation(&second_account, u64::MAX);

    let mut second_reservation = second
        .control_checkpoint()
        .reserve(second_keys.phase, second_keys.dimension, 3)
        .expect("second operation reservation");

    {
        let mut first_checkpoint = first.control_checkpoint();
        first_checkpoint.poll().expect("checkpoint poll");
        assert!(matches!(
            first_checkpoint.charge_observed(second_keys.dimension, 1),
            Err(StoreRuntimeError::ForeignDimensionDomain { .. })
        ));
        assert!(matches!(
            first_checkpoint.reserve(second_keys.phase, second_keys.dimension, 1),
            Err(StoreRuntimeError::ForeignPhaseDomain { .. })
        ));
        assert!(matches!(
            first_checkpoint.validate_live_owner(&second_reservation),
            Err(StoreRuntimeError::Ledger(LedgerCause::OwnerMismatch))
        ));
        assert!(matches!(
            first_checkpoint.resize(&mut second_reservation, 4),
            Err(StoreRuntimeError::Ledger(LedgerCause::OwnerMismatch))
        ));
        assert_eq!(second_reservation.covered_bytes_for_test(), 3);

        first_checkpoint
            .charge_observed(first_keys.dimension, 1)
            .expect("same-domain observed work");
        let mut first_reservation = first_checkpoint
            .reserve(first_keys.phase, first_keys.dimension, 2)
            .expect("same-domain reservation");
        first_checkpoint
            .validate_live_owner(&first_reservation)
            .expect("same-operation reservation owner");
        first_checkpoint
            .resize(&mut first_reservation, 4)
            .expect("same-operation resize");
        assert_eq!(first_reservation.covered_bytes_for_test(), 4);
        first_checkpoint
            .transition(
                &mut first_reservation,
                first_keys.phase,
                first_keys.dimension,
                5,
            )
            .expect("same-domain transition");
        assert_eq!(first_reservation.covered_bytes_for_test(), 5);
    }
    drop(second_reservation);

    let first_fallback = first
        .prepare_minimal_fallback(1)
        .expect("first checkpoint fallback")
        .handoff();
    let second_fallback = second
        .prepare_minimal_fallback(2)
        .expect("second checkpoint fallback")
        .handoff();
    drop((first_fallback, second_fallback));
    assert_eq!(first_account.snapshot().live_retained_bytes(), 0);
    assert_eq!(first_account.snapshot().live_retained_allocations(), 0);
    assert_eq!(second_account.snapshot().live_retained_bytes(), 0);
    assert_eq!(second_account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn publication_assembly_lends_transient_control_without_mutating_tables() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account, u64::MAX);
    let mut assembly = operation
        .try_publication_assembly(test_publication_plan(keys, 1, 1))
        .expect("transient-control assembly");

    let mut reservation = {
        let mut checkpoint = assembly
            .try_control_checkpoint()
            .expect("writable assembly checkpoint");
        checkpoint.poll().expect("assembly checkpoint poll");
        checkpoint
            .charge_observed(keys.dimension, 1)
            .expect("assembly checkpoint work");
        let mut reservation = checkpoint
            .reserve(keys.phase, keys.dimension, 2)
            .expect("assembly checkpoint reservation");
        checkpoint
            .resize(&mut reservation, 4)
            .expect("assembly checkpoint resize");
        reservation
    };
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(assembly.outcome_evidence_len(), 0);

    {
        let mut checkpoint = assembly
            .try_control_checkpoint()
            .expect("second writable assembly checkpoint");
        checkpoint
            .validate_live_owner(&reservation)
            .expect("assembly retains the reservation owner");
        checkpoint
            .resize(&mut reservation, 0)
            .expect("assembly checkpoint releases live bytes");
    }
    drop(reservation);
    drop(
        assembly
            .prepare_minimal_fallback()
            .expect("assembly checkpoint fallback")
            .handoff(),
    );
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn store_key_authority_is_unique_and_raw_brand_access_stays_private() {
    let root = include_str!("../lib.rs");
    let source = include_str!("vocabulary.rs");
    let catalog_start = source
        .find("pub struct StoreSemanticCatalogId")
        .expect("public store semantic catalog identity");
    let phase_semantic_start = source
        .find("pub struct StorePhaseSemanticId")
        .expect("public store phase semantic identity");
    let dimension_semantic_start = source
        .find("pub struct StoreDimensionSemanticId")
        .expect("public store dimension semantic identity");
    let domain_start = source
        .find("pub struct StoreKeyDomain")
        .expect("public store key authority");
    let phase_start = source
        .find("pub struct StorePhaseKey")
        .expect("public store phase key");
    let dimension_start = source
        .find("pub struct StoreDimensionKey")
        .expect("public store dimension key");
    let plan_start = source
        .find("pub struct StoreLedgerPlan")
        .expect("public store ledger plan");
    let authority_declaration_start = domain_start.saturating_sub(96);
    let authority_declaration_end = domain_start
        + source[domain_start..]
            .find("\n}")
            .expect("complete store key authority declaration")
        + "\n}".len();
    let authority_declaration = &source[authority_declaration_start..authority_declaration_end];
    let catalog_surface = &source[catalog_start..phase_semantic_start];
    let phase_semantic_surface = &source[phase_semantic_start..dimension_semantic_start];
    let dimension_semantic_surface = &source[dimension_semantic_start..domain_start];
    let domain_surface = &source[domain_start..phase_start];
    let phase_surface = &source[phase_start..dimension_start];
    let dimension_surface = &source[dimension_start..plan_start];

    assert!(root.contains("StoreKeyDomain"));
    assert!(!authority_declaration.contains("#[derive("));
    assert!(!domain_surface.contains("impl Clone for StoreKeyDomain"));
    assert!(!domain_surface.contains("impl Copy for StoreKeyDomain"));
    assert!(domain_surface.contains("pub fn try_admit_phase("));
    assert!(domain_surface.contains("pub fn try_admit_dimension("));
    assert!(!domain_surface.contains("pub fn phase_key("));
    assert!(!domain_surface.contains("pub fn dimension_key("));
    let plan_declaration = &source[plan_start.saturating_sub(128)..plan_start];
    assert!(!plan_declaration.contains("#[derive(PartialEq, Eq)]"));
    let plan_surface = &source[plan_start..];
    assert!(plan_surface.contains("This is an attempt-once authority transfer"));
    assert!(plan_surface.contains("A retry must create a fresh domain"));
    for forbidden in [
        "pub const fn id(",
        "pub fn id(",
        "pub const fn domain_id(",
        "pub fn domain_id(",
    ] {
        assert!(
            !domain_surface.contains(forbidden),
            "store key authority leaked its private brand through `{forbidden}`"
        );
    }
    for surface in [phase_surface, dimension_surface] {
        assert!(!surface.contains("pub const fn new("));
        assert!(!surface.contains("pub fn new("));
        assert!(!surface.contains("pub const fn get("));
        assert!(!surface.contains("pub fn get("));
        for forbidden in [
            "pub const fn local_id(",
            "pub fn local_id(",
            "pub const fn brand(",
            "pub fn brand(",
            "pub const fn raw(",
            "pub fn raw(",
        ] {
            assert!(
                !surface.contains(forbidden),
                "store key leaked process-local identity through `{forbidden}`"
            );
        }
        assert!(surface.contains("pub const fn semantic_id("));
        assert_eq!(surface.matches("pub const fn ").count(), 1);
        assert_eq!(surface.matches("pub fn ").count(), 0);
    }
    for surface in [
        catalog_surface,
        phase_semantic_surface,
        dimension_semantic_surface,
    ] {
        assert!(
            surface.contains("pub const fn to_canonical_bytes("),
            "every exported semantic identity must expose a canonical byte encoding"
        );
    }
    assert!(phase_surface.contains("impl Hash for StorePhaseKey"));
    assert!(dimension_surface.contains("impl Hash for StoreDimensionKey"));
}

#[test]
fn semantic_authority_and_terminal_key_cells_cannot_retain_executable_state() {
    let vocabulary = include_str!("vocabulary.rs");
    let store_source = concrete_store_source();
    let evidence = include_str!("../retained/evidence.rs");
    let declarations = [
        (
            "StoreSemanticCatalogId",
            type_declaration(vocabulary, "pub struct StoreSemanticCatalogId"),
            store_source.as_str(),
        ),
        (
            "StorePhaseSemanticId",
            type_declaration(vocabulary, "pub struct StorePhaseSemanticId"),
            store_source.as_str(),
        ),
        (
            "StoreDimensionSemanticId",
            type_declaration(vocabulary, "pub struct StoreDimensionSemanticId"),
            store_source.as_str(),
        ),
        (
            "StoreKeyDomain",
            type_declaration(vocabulary, "pub struct StoreKeyDomain"),
            store_source.as_str(),
        ),
        (
            "StorePhaseKey",
            type_declaration(vocabulary, "pub struct StorePhaseKey"),
            store_source.as_str(),
        ),
        (
            "StoreDimensionKey",
            type_declaration(vocabulary, "pub struct StoreDimensionKey"),
            store_source.as_str(),
        ),
        (
            "FallbackDimensionCell",
            type_declaration(evidence, "pub(super) struct FallbackDimensionCell"),
            evidence,
        ),
        (
            "FallbackPhaseCell",
            type_declaration(evidence, "pub(super) struct FallbackPhaseCell"),
            evidence,
        ),
    ];

    for (name, declaration, source) in declarations {
        for forbidden in [
            "Arc<", "Rc<", "dyn ", "fn(", "fn ", "*const", "*mut", "callback",
        ] {
            assert!(
                !declaration.contains(forbidden),
                "terminal semantic authority `{name}` retained executable state through `{forbidden}`"
            );
        }
        assert!(
            !source.contains(&format!("Drop for {name}")),
            "terminal semantic authority `{name}` acquired executable drop behavior"
        );
    }
}

#[test]
fn prepared_store_surfaces_expose_only_consuming_classification_or_handoff() {
    let store = include_str!("terminal_output.rs");
    let retained = include_str!("../retained/terminal_output.rs");
    let concrete_impls = [
        implementation_block(
            store,
            "impl<CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>\n    PreparedStoreSuccess",
        ),
        implementation_block(
            store,
            "impl<EvidenceKind, EvidenceTag> PreparedStoreFailure",
        ),
        implementation_block(store, "impl PreparedStoreFallback"),
    ];
    let prepared_struct_marker = ["pub struct ", "PreparedStore"].concat();
    assert_eq!(store.matches(&prepared_struct_marker).count(), 3);
    for surface in concrete_impls {
        assert_eq!(surface.matches("pub fn ").count(), 1);
        assert!(surface.contains("pub fn handoff("));
        assert!(!surface.contains("&self"));
        assert!(!surface.contains("&mut self"));
        for forbidden in [
            "pub fn canonical",
            "pub fn outcome_evidence",
            "pub fn evidence",
            "pub fn root",
            "pub fn aggregate",
            "pub fn value",
            "pub fn ledger_evidence",
        ] {
            assert!(
                !surface.contains(forbidden),
                "prepared concrete owner leaked content through `{forbidden}`"
            );
        }
    }

    let retained_publication = implementation_block(
        retained,
        "impl<CanonicalBlock, EvidenceBlock, F, P, D>\n    PreparedRetainedPublication",
    );
    assert_eq!(retained_publication.matches("pub fn ").count(), 1);
    assert!(retained_publication.contains("pub fn into_outcome("));
    assert!(!retained_publication.contains("&self"));
    assert!(!retained_publication.contains("&mut self"));
    for forbidden in [
        "canonical_root(",
        "outcome_evidence_root(",
        "evidence(",
        "root(",
        "aggregate(",
        "value(",
    ] {
        assert!(
            !retained_publication.contains(forbidden),
            "prepared retained classifier leaked content through `{forbidden}`"
        );
    }
    let retained_impls = [
        implementation_block(
            retained,
            "impl<CanonicalBlock, EvidenceBlock, P, D>\n    PreparedRetainedSuccess",
        ),
        implementation_block(
            retained,
            "impl<EvidenceBlock, P, D> PreparedRetainedRichFailure",
        ),
        implementation_block(retained, "impl<F, P, D> PreparedRetainedFallback"),
    ];
    for surface in retained_impls {
        assert_eq!(surface.matches("pub fn ").count(), 1);
        assert!(surface.contains("pub fn handoff("));
        assert!(!surface.contains("&self"));
        assert!(!surface.contains("&mut self"));
        for forbidden in [
            "canonical_root(",
            "outcome_evidence_root(",
            "evidence(",
            "root(",
            "aggregate(",
            "value(",
        ] {
            assert!(
                !surface.contains(forbidden),
                "prepared retained owner leaked content through `{forbidden}`"
            );
        }
    }
}

#[test]
fn canonical_share_api_requires_the_sealed_shareable_kind_at_its_boundary() {
    let source = include_str!("../concrete_store.rs");
    let method = public_method_block(
        source,
        "pub fn try_share_canonical<SourceRootKind, SourceRootTag, SlotKind, SlotTag>",
    );
    assert!(method.contains("source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>"));
    assert!(method.contains("source_ref: CanonicalStoreRef<SlotKind, SlotTag>"));
    assert!(method.contains("SlotKind: ShareableStoreKind"));
}

#[test]
fn success_evidence_share_api_requires_the_sealed_shareable_kind_at_its_boundary() {
    let source = include_str!("../concrete_store.rs");
    let method = public_method_block(
        source,
        "pub fn try_share_outcome_evidence<SourceRootKind, SourceRootTag, SlotKind, SlotTag>",
    );
    assert!(
        method.contains("source: &PublishedOutcomeEvidenceStore<SourceRootKind, SourceRootTag>")
    );
    assert!(method.contains("source_ref: OutcomeEvidenceStoreRef<SlotKind, SlotTag>"));
    assert!(method.contains("SlotKind: ShareableStoreKind"));
}

#[test]
fn failure_evidence_share_api_requires_the_sealed_shareable_kind_at_its_boundary() {
    let source = include_str!("../concrete_store.rs");
    let method = public_method_block(
        source,
        "pub fn try_share<SourceRootKind, SourceRootTag, SlotKind, SlotTag>",
    );
    assert!(
        method.contains("source: &PublishedOutcomeEvidenceStore<SourceRootKind, SourceRootTag>")
    );
    assert!(method.contains("source_ref: OutcomeEvidenceStoreRef<SlotKind, SlotTag>"));
    assert!(method.contains("SlotKind: ShareableStoreKind"));
}
