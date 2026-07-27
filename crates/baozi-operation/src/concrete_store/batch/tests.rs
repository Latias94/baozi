use super::*;

use crate::ledger::{LedgerCause, LedgerLimits};
use crate::retained::test_panic_final_owner_drop_after;
use crate::{
    DimensionPolicy, FinalProgress, OperationControl, PhysicalStorageAccount,
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits, RetainedStoreBatchLimits, RetainedStoreBlockTablePlan,
    RetainedStorePublicationPlan, RetainedStoreSharePlan, RetainedStoreStartPlan,
    RetainedStoreWriterPlan, StoreDimensionKey, StoreDimensionSemanticId, StoreFallbackEnvelope,
    StoreKeyDomain, StorePhaseKey, StorePhaseSemanticId, StoreSemanticCatalogId,
};
use std::num::{NonZeroU32, NonZeroU64};

struct BatchTestOwner;

impl OperationOwner for BatchTestOwner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BatchProgress {
    Build,
}

impl ProgressPhase for BatchProgress {
    fn order(self) -> u32 {
        0
    }

    fn stable_id(self) -> &'static str {
        "build"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BatchUnit {
    Items,
}

impl ProgressUnit for BatchUnit {
    fn stable_id(self) -> &'static str {
        "items"
    }
}

#[derive(Clone, Copy)]
struct BatchTag;

#[derive(Clone, Copy)]
struct EvidenceTag;

#[derive(Clone, Copy)]
struct BatchKeys {
    phase: StorePhaseKey,
    dimension: StoreDimensionKey,
}

type BatchOperation = crate::RetainedStoreOperation<BatchTestOwner, BatchProgress, BatchUnit>;

const fn test_catalog() -> StoreSemanticCatalogId {
    StoreSemanticCatalogId::new(
        *b"baozi-batch-test",
        NonZeroU32::new(1).expect("test revision is nonzero"),
    )
}

const fn phase_semantic() -> StorePhaseSemanticId {
    StorePhaseSemanticId::new(
        test_catalog(),
        NonZeroU64::new(1).expect("test phase id is nonzero"),
    )
}

const fn dimension_semantic() -> StoreDimensionSemanticId {
    StoreDimensionSemanticId::new(
        test_catalog(),
        NonZeroU64::new(1).expect("test dimension id is nonzero"),
    )
}

fn test_domain() -> (StoreKeyDomain, BatchKeys) {
    let domain = StoreKeyDomain::try_new(test_catalog()).expect("batch test domain");
    let keys = BatchKeys {
        phase: domain
            .try_admit_phase(phase_semantic())
            .expect("batch test phase"),
        dimension: domain
            .try_admit_dimension(dimension_semantic())
            .expect("batch test dimension"),
    };
    (domain, keys)
}

fn test_factory() -> PhysicalStorageAccountFactory {
    PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).expect("batch test factory limits"),
    )
    .expect("batch test factory")
}

fn test_account(factory: &PhysicalStorageAccountFactory) -> PhysicalStorageAccount {
    factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(u64::MAX, 512).expect("batch test account limits"),
        )
        .expect("batch test account")
}

fn writer_plan(
    keys: BatchKeys,
    attributions: usize,
    commitments: usize,
) -> RetainedStoreWriterPlan {
    RetainedStoreWriterPlan::new(
        keys.phase,
        keys.dimension,
        keys.dimension,
        RetainedStoreBatchLimits::new(attributions, commitments).expect("batch test writer limits"),
    )
}

fn test_operation(account: &PhysicalStorageAccount) -> (BatchOperation, BatchKeys) {
    test_operation_with_limits(account, u64::MAX, u64::MAX)
}

fn test_operation_with_live_limit(
    account: &PhysicalStorageAccount,
    operation_live_byte_limit: u64,
) -> (BatchOperation, BatchKeys) {
    test_operation_with_limits(account, operation_live_byte_limit, u64::MAX)
}

fn test_operation_with_limits(
    account: &PhysicalStorageAccount,
    operation_live_byte_limit: u64,
    work_limit: u64,
) -> (BatchOperation, BatchKeys) {
    let (domain, keys) = test_domain();
    let policy = DimensionPolicy::reservable_bytes(work_limit, work_limit, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(operation_live_byte_limit, [(keys.dimension, policy)])
        .expect("batch ledger limits");
    let ledger =
        crate::StoreLedgerPlan::try_new(domain, limits, &[keys.phase]).expect("batch ledger plan");
    let start = RetainedStoreStartPlan::new(
        writer_plan(keys, 64, 8),
        RetainedStoreSharePlan::new(keys.dimension, keys.dimension),
        StoreFallbackEnvelope::new(37),
        keys.dimension,
        1,
        keys.dimension,
        1,
    );
    (
        BatchOperation::try_start(OperationControl::default(), ledger, account, start)
            .expect("batch operation"),
        keys,
    )
}

fn publication_plan(keys: BatchKeys, canonical_blocks: usize) -> RetainedStorePublicationPlan {
    RetainedStorePublicationPlan::success(
        RetainedStoreBlockTablePlan::new(
            writer_plan(keys, canonical_blocks + 1, 2),
            canonical_blocks,
            canonical_blocks,
        )
        .expect("batch canonical table"),
        RetainedStoreBlockTablePlan::new(writer_plan(keys, 2, 2), 1, 1)
            .expect("batch evidence table"),
    )
}

fn final_progress() -> FinalProgress<BatchProgress, BatchUnit> {
    FinalProgress::new(BatchProgress::Build, BatchUnit::Items, 1)
}

#[test]
fn batch_plans_reject_empty_inputs_without_operation_side_effects() {
    let factory = test_factory();
    let account = test_account(&factory);
    let before = account.snapshot();

    assert!(matches!(
        SequenceBatchItem::<u8>::new(&[]),
        Err(StorePlanError::EmptyPayload)
    ));
    assert!(matches!(
        BytesBatchItem::new(&[]),
        Err(StorePlanError::EmptyPayload)
    ));
    assert!(matches!(
        Utf8BatchItem::new(""),
        Err(StorePlanError::EmptyPayload)
    ));
    assert!(matches!(
        SequenceBatchPlan::<u8>::new(&[]),
        Err(StorePlanError::EmptyPayload)
    ));
    assert!(matches!(
        BytesBatchPlan::new(&[]),
        Err(StorePlanError::EmptyPayload)
    ));
    assert!(matches!(
        Utf8BatchPlan::new(&[]),
        Err(StorePlanError::EmptyPayload)
    ));

    let qualified = SequencePlan::<u8>::new(1).expect("qualified single item");
    let invalid = SequenceBatchItem {
        values: &[],
        plan: qualified,
        allocation: qualified.allocation_plan().expect("single allocation plan"),
    };
    assert!(matches!(
        SequenceBatchPlan::new(&[invalid]),
        Err(StorePlanError::EmptyPayload)
    ));
    assert_eq!(account.snapshot(), before);
}

#[test]
fn sequence_bytes_and_utf8_batches_publish_opaque_ranges() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 6))
        .expect("batch publication assembly");

    let first_sequence = [1_u16, 2];
    let second_sequence = [3_u16, 4, 5];
    let sequence_values = [first_sequence.as_slice(), second_sequence.as_slice()];
    let sequence_items = [
        SequenceBatchItem::new(sequence_values[0]).expect("first sequence item"),
        SequenceBatchItem::new(sequence_values[1]).expect("second sequence item"),
    ];
    let sequence_range = assembly
        .try_retain_canonical_sequence_batch(
            SequenceBatchPlan::new(&sequence_items).expect("sequence batch"),
            BatchTag,
        )
        .expect("retained sequence batch");

    let first_bytes = [6_u8, 7];
    let second_bytes = [8_u8, 9, 10];
    let byte_values = [first_bytes.as_slice(), second_bytes.as_slice()];
    let byte_items = [
        BytesBatchItem::new(byte_values[0]).expect("first bytes item"),
        BytesBatchItem::new(byte_values[1]).expect("second bytes item"),
    ];
    let bytes_range = assembly
        .try_retain_canonical_bytes_batch(
            BytesBatchPlan::new(&byte_items).expect("bytes batch"),
            BatchTag,
        )
        .expect("retained bytes batch");

    let utf8_items = [
        Utf8BatchItem::new("alpha").expect("first utf8 item"),
        Utf8BatchItem::new("beta").expect("second utf8 item"),
    ];
    let utf8_range = assembly
        .try_retain_canonical_utf8_batch(
            Utf8BatchPlan::new(&utf8_items).expect("utf8 batch"),
            BatchTag,
        )
        .expect("retained utf8 batch");

    assert_eq!(sequence_range.len(), 2);
    assert!(!sequence_range.is_empty());
    assert!(sequence_range.get(0).is_some());
    assert!(sequence_range.get(1).is_some());
    assert!(sequence_range.get(2).is_none());
    assert_eq!(bytes_range.len(), 2);
    assert!(bytes_range.get(0).is_some());
    assert!(bytes_range.get(1).is_some());
    assert!(bytes_range.get(2).is_none());
    assert_eq!(utf8_range.len(), 2);
    assert!(utf8_range.get(0).is_some());
    assert!(utf8_range.get(1).is_some());
    assert!(utf8_range.get(2).is_none());

    let evidence_root = assembly
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("evidence plan"),
            EvidenceTag,
            |writer| writer.push(1),
        )
        .expect("evidence root");
    let canonical_root = sequence_range.get(0).expect("canonical root");
    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, _, evidence_root| {
            for (index, expected) in sequence_values.iter().enumerate() {
                let Some(slot) = sequence_range.get(index) else {
                    return false;
                };
                let Some(view) = session.canonical_sequence(slot) else {
                    return false;
                };
                if view.as_slice() != *expected {
                    return false;
                }
            }
            for (index, expected) in byte_values.iter().enumerate() {
                let Some(slot) = bytes_range.get(index) else {
                    return false;
                };
                if session.canonical_bytes(slot) != Some(*expected) {
                    return false;
                }
            }
            for (index, expected) in ["alpha", "beta"].iter().enumerate() {
                let Some(slot) = utf8_range.get(index) else {
                    return false;
                };
                if session.canonical_utf8(slot) != Some(*expected) {
                    return false;
                }
            }
            session.outcome_evidence_sequence(evidence_root).is_some()
        },
    ) {
        crate::StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("batch graph must validate"),
    };
    let prepared = match validated
        .try_prepare(final_progress())
        .expect("batch success preparation")
    {
        crate::PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("batch success must remain successful"),
    };
    let published = prepared.handoff();

    assert_eq!(
        published
            .canonical
            .sequence(sequence_range.get(1).expect("last sequence"))
            .expect("published sequence")
            .as_slice(),
        sequence_values[1]
    );
    assert_eq!(
        published
            .canonical
            .bytes(bytes_range.get(1).expect("last bytes"))
            .expect("published bytes"),
        byte_values[1]
    );
    assert_eq!(
        published
            .canonical
            .utf8(utf8_range.get(1).expect("last utf8")),
        Some("beta")
    );

    #[cfg(feature = "_u4c-collector")]
    {
        let session =
            crate::u4c_collector::MeasurementSession::begin().expect("range-only measurement");
        assert!(sequence_range.get(sequence_range.len()).is_none());
        assert!(
            published
                .canonical
                .sequence(sequence_range.get(0).expect("measured sequence reference"))
                .is_some()
        );
        let report = session.finish().expect("range-only report");
        assert_eq!(report.completed_lookups(), 1);
        assert_eq!(report.rejected_lookups(), 0);
    }

    drop(published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn foreign_range_references_are_rejected_and_lookup_depth_is_batch_size_independent() {
    let factory = test_factory();
    let account = test_account(&factory);

    let (first_operation, first_keys) = test_operation(&account);
    let mut first = first_operation
        .try_publication_assembly(publication_plan(first_keys, 3))
        .expect("first batch assembly");
    let first_values = [[1_u8], [2_u8], [3_u8]];
    let first_items = [
        SequenceBatchItem::new(&first_values[0]).expect("first item"),
        SequenceBatchItem::new(&first_values[1]).expect("second item"),
        SequenceBatchItem::new(&first_values[2]).expect("third item"),
    ];
    let first_range = first
        .try_retain_canonical_sequence_batch(
            SequenceBatchPlan::new(&first_items).expect("first batch"),
            BatchTag,
        )
        .expect("first retained batch");
    let first_evidence = first
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("first evidence plan"),
            EvidenceTag,
            |writer| writer.push(1),
        )
        .expect("first evidence");
    let first_validated = match first.validate_success(
        first_range.get(0).expect("first root"),
        first_evidence,
        |session, _, evidence| {
            for index in 0..first_range.len() {
                let Some(slot) = first_range.get(index) else {
                    return false;
                };
                if session.canonical_sequence(slot).is_none() {
                    return false;
                }
            }
            session.outcome_evidence_sequence(evidence).is_some()
        },
    ) {
        crate::StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("first graph must validate"),
    };
    let first_prepared = match first_validated
        .try_prepare(final_progress())
        .expect("first preparation")
    {
        crate::PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("first publication must succeed"),
    };
    let first_published = first_prepared.handoff();

    let (second_operation, second_keys) = test_operation(&account);
    let mut second = second_operation
        .try_publication_assembly(publication_plan(second_keys, 1))
        .expect("second batch assembly");
    let second_value = [9_u8];
    let second_items = [SequenceBatchItem::new(&second_value).expect("second publication item")];
    let second_range = second
        .try_retain_canonical_sequence_batch(
            SequenceBatchPlan::new(&second_items).expect("second batch"),
            BatchTag,
        )
        .expect("second retained batch");
    let second_evidence = second
        .try_retain_outcome_evidence_sequence(
            SequencePlan::<u8>::new(1).expect("second evidence plan"),
            EvidenceTag,
            |writer| writer.push(2),
        )
        .expect("second evidence");
    let second_validated = match second.validate_success(
        second_range.get(0).expect("second root"),
        second_evidence,
        |session, root, evidence| {
            session.canonical_sequence(root).is_some()
                && session.outcome_evidence_sequence(evidence).is_some()
        },
    ) {
        crate::StoreSuccessValidation::Valid(validated) => validated,
        _ => panic!("second graph must validate"),
    };
    let second_prepared = match second_validated
        .try_prepare(final_progress())
        .expect("second preparation")
    {
        crate::PreparedStoreSuccessOutcome::Success(prepared) => prepared,
        _ => panic!("second publication must succeed"),
    };
    let second_published = second_prepared.handoff();

    assert!(
        second_published
            .canonical
            .sequence(first_range.get(2).expect("foreign reference"))
            .is_none()
    );
    assert!(
        first_published
            .canonical
            .sequence(second_range.get(0).expect("reverse foreign reference"))
            .is_none()
    );

    #[cfg(feature = "_u4c-collector")]
    {
        let first_session =
            crate::u4c_collector::MeasurementSession::begin().expect("first lookup measurement");
        assert!(
            first_published
                .canonical
                .sequence(first_range.get(2).expect("first lookup reference"))
                .is_some()
        );
        let first_report = first_session.finish().expect("first lookup report");

        let second_session =
            crate::u4c_collector::MeasurementSession::begin().expect("second lookup measurement");
        assert!(
            second_published
                .canonical
                .sequence(second_range.get(0).expect("second lookup reference"))
                .is_some()
        );
        let second_report = second_session.finish().expect("second lookup report");

        assert_eq!(
            first_report.max_lookup_events(),
            second_report.max_lookup_events()
        );
    }

    drop(first_published);
    drop(second_published);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn batch_preflight_rejection_is_zero_mutation_and_the_assembly_remains_retryable() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 1))
        .expect("preflight batch assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before_canonical = assembly.canonical_len();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();
    let before_physical = account.snapshot();
    let oversized_values = [[1_u8], [2_u8]];
    let oversized = [
        SequenceBatchItem::new(&oversized_values[0]).expect("first oversized item"),
        SequenceBatchItem::new(&oversized_values[1]).expect("second oversized item"),
    ];

    let error = assembly
        .try_retain_canonical_sequence_batch(
            SequenceBatchPlan::new(&oversized).expect("oversized batch plan"),
            BatchTag,
        )
        .expect_err("table preflight must reject the oversized batch");

    assert!(matches!(
        error,
        StoreBatchRuntimeError::Preflight(StoreRuntimeError::SlotLimit { .. })
    ));
    assert_eq!(assembly.canonical_len(), before_canonical);
    assert_eq!(ledger.active_reservations(), before_active);
    assert_eq!(ledger.current_live_bytes(), before_live);
    assert_eq!(account.snapshot(), before_physical);

    let retry_value = [9_u8];
    let retry = [SequenceBatchItem::new(&retry_value).expect("retry item")];
    let range = assembly
        .try_retain_canonical_sequence_batch(
            SequenceBatchPlan::new(&retry).expect("retry batch plan"),
            BatchTag,
        )
        .expect("preflight rejection must leave the assembly writable");
    assert_eq!(range.len(), 1);
    assert_eq!(assembly.canonical_len(), 1);

    let fallback = assembly
        .prepare_minimal_fallback()
        .expect("preflight retry fallback")
        .handoff();
    drop(fallback);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn aggregate_operation_headroom_rejection_is_zero_mutation_and_retryable() {
    const OPERATION_LIVE_LIMIT: u64 = 1_000_000;

    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation_with_live_limit(&account, OPERATION_LIVE_LIMIT);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 2))
        .expect("aggregate-headroom batch assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before_canonical = assembly.canonical_len();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();
    let before_attributed = assembly.raw.test_canonical_attributed_bytes();
    let before_physical = account.snapshot();

    let first = vec![1_u8; 600_000];
    let second = vec![2_u8; 600_000];
    let oversized = [
        BytesBatchItem::new(&first).expect("first aggregate item"),
        BytesBatchItem::new(&second).expect("second aggregate item"),
    ];
    let error = assembly
        .try_retain_canonical_bytes_batch(
            BytesBatchPlan::new(&oversized).expect("aggregate batch plan"),
            BatchTag,
        )
        .expect_err("aggregate operation headroom must reject the complete batch");

    assert!(matches!(
        error,
        StoreBatchRuntimeError::Preflight(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
            limit: OPERATION_LIVE_LIMIT,
            ..
        }))
    ));
    assert_eq!(assembly.canonical_len(), before_canonical);
    assert_eq!(ledger.active_reservations(), before_active);
    assert_eq!(ledger.current_live_bytes(), before_live);
    assert_eq!(
        assembly.raw.test_canonical_attributed_bytes(),
        before_attributed
    );
    assert_eq!(account.snapshot(), before_physical);

    let retry_value = [9_u8; 8];
    let retry = [BytesBatchItem::new(&retry_value).expect("headroom retry item")];
    let range = assembly
        .try_retain_canonical_bytes_batch(
            BytesBatchPlan::new(&retry).expect("headroom retry plan"),
            BatchTag,
        )
        .expect("aggregate preflight rejection leaves the assembly retryable");
    assert_eq!(range.len(), 1);

    let fallback = assembly
        .prepare_minimal_fallback()
        .expect("headroom retry fallback")
        .handoff();
    drop(fallback);
    assert_eq!(ledger.active_reservations(), 0);
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn aggregate_share_work_rejection_is_zero_mutation_and_retryable() {
    const WORK_LIMIT: u64 = 16;

    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation_with_limits(&account, u64::MAX, WORK_LIMIT);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 2))
        .expect("work-headroom batch assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before_work = ledger
        .dimension_evidence(keys.dimension)
        .expect("work evidence before batch");
    assert_eq!(before_work.attempted(), 14);
    assert_eq!(before_work.consumed(), 14);
    let before_canonical = assembly.canonical_len();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();
    let before_attributed = assembly.raw.test_canonical_attributed_bytes();
    let before_physical = account.snapshot();

    let values = [[1_u8], [2_u8]];
    let items = [
        SequenceBatchItem::new(&values[0]).expect("first work item"),
        SequenceBatchItem::new(&values[1]).expect("second work item"),
    ];
    let error = assembly
        .try_retain_canonical_sequence_batch(
            SequenceBatchPlan::new(&items).expect("work batch plan"),
            BatchTag,
        )
        .expect_err("aggregate share work must reject the complete batch");

    assert!(matches!(
        error,
        StoreBatchRuntimeError::Preflight(StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
            class: crate::CounterClass::Attempted,
            value: 18,
            limit: WORK_LIMIT,
            ..
        }))
    ));
    assert_eq!(assembly.canonical_len(), before_canonical);
    assert_eq!(ledger.active_reservations(), before_active);
    assert_eq!(ledger.current_live_bytes(), before_live);
    assert_eq!(
        assembly.raw.test_canonical_attributed_bytes(),
        before_attributed
    );
    assert_eq!(account.snapshot(), before_physical);
    let after_rejection = ledger
        .dimension_evidence(keys.dimension)
        .expect("work evidence after rejection");
    assert_eq!(after_rejection, before_work);

    let retry = [SequenceBatchItem::new(&values[0]).expect("work retry item")];
    let range = assembly
        .try_retain_canonical_sequence_batch(
            SequenceBatchPlan::new(&retry).expect("work retry plan"),
            BatchTag,
        )
        .expect("one item fits the remaining work budget");
    assert_eq!(range.len(), 1);
    let exact = ledger
        .dimension_evidence(keys.dimension)
        .expect("work evidence at exact limit");
    assert_eq!(exact.attempted(), WORK_LIMIT);
    assert_eq!(exact.consumed(), WORK_LIMIT);

    let fallback = assembly
        .prepare_minimal_fallback()
        .expect("work retry fallback")
        .handoff();
    drop(fallback);
    assert_eq!(ledger.active_reservations(), 0);
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn aggregate_share_work_exact_fit_charges_only_executed_items() {
    const WORK_LIMIT: u64 = 18;

    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation_with_limits(&account, u64::MAX, WORK_LIMIT);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 2))
        .expect("exact-fit batch assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before = ledger
        .dimension_evidence(keys.dimension)
        .expect("work evidence before exact-fit batch");
    assert_eq!(before.attempted(), 14);
    assert_eq!(before.consumed(), 14);

    let values = [[1_u8], [2_u8]];
    let items = [
        SequenceBatchItem::new(&values[0]).expect("first exact-fit item"),
        SequenceBatchItem::new(&values[1]).expect("second exact-fit item"),
    ];
    let range = assembly
        .try_retain_canonical_sequence_batch(
            SequenceBatchPlan::new(&items).expect("exact-fit batch plan"),
            BatchTag,
        )
        .expect("projected exact-fit batch");

    assert_eq!(range.len(), 2);
    let exact = ledger
        .dimension_evidence(keys.dimension)
        .expect("work evidence after exact-fit batch");
    assert_eq!(exact.attempted(), WORK_LIMIT);
    assert_eq!(exact.consumed(), WORK_LIMIT);

    let fallback = assembly
        .prepare_minimal_fallback()
        .expect("exact-fit fallback")
        .handoff();
    drop(fallback);
    assert_eq!(ledger.active_reservations(), 0);
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn mid_batch_failure_rolls_back_the_prefix_and_leaves_the_assembly_writable() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 3))
        .expect("failing batch assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before_attributions = ledger.active_reservations();
    let before_ledger_bytes = ledger.current_live_bytes();
    let before_attributed_bytes = assembly.raw.test_canonical_attributed_bytes();
    let before_physical = account.snapshot();
    let before_work = ledger
        .dimension_evidence(keys.dimension)
        .expect("work evidence before failing batch");
    let item_values = [[1_u8], [2_u8], [3_u8]];
    let items = [
        SequenceBatchItem::new(&item_values[0]).expect("first item"),
        SequenceBatchItem::new(&item_values[1]).expect("second item"),
        SequenceBatchItem::new(&item_values[2]).expect("third item"),
    ];
    test_fail_batch_build_at(1);
    let error = assembly
        .try_retain_canonical_sequence_batch(
            SequenceBatchPlan::new(&items).expect("failing batch plan"),
            BatchTag,
        )
        .expect_err("second item must fail");

    assert!(matches!(
        error,
        StoreBatchRuntimeError::Execution {
            failed_item: 1,
            cause: StoreRuntimeError::Build(cause),
        } if cause.class() == crate::StoreBuildFailureClass::IncompleteWrite
    ));
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(ledger.active_reservations(), before_attributions);
    assert_eq!(ledger.current_live_bytes(), before_ledger_bytes);
    assert_eq!(
        assembly.raw.test_canonical_attributed_bytes(),
        before_attributed_bytes
    );
    let after_physical = account.snapshot();
    assert_eq!(
        after_physical.live_retained_bytes(),
        before_physical.live_retained_bytes()
    );
    assert_eq!(
        after_physical.live_retained_allocations(),
        before_physical.live_retained_allocations()
    );
    assert!(after_physical.peak_retained_bytes() > before_physical.peak_retained_bytes());
    assert!(
        after_physical.peak_retained_allocations() > before_physical.peak_retained_allocations()
    );
    assert!(
        after_physical.total_admitted_retained_bytes()
            > before_physical.total_admitted_retained_bytes()
    );
    assert!(
        after_physical.total_admitted_retained_allocations()
            > before_physical.total_admitted_retained_allocations()
    );
    assert_eq!(after_physical.poisoned(), before_physical.poisoned());
    assert_eq!(
        after_physical.poison_event_count(),
        before_physical.poison_event_count()
    );
    assert_eq!(
        after_physical.poison_event_count_exact(),
        before_physical.poison_event_count_exact()
    );
    assert_eq!(
        after_physical.poison_event_domain_valid(),
        before_physical.poison_event_domain_valid()
    );
    assert_eq!(
        after_physical.first_poison_evidence(),
        before_physical.first_poison_evidence()
    );
    let after_work = ledger
        .dimension_evidence(keys.dimension)
        .expect("work evidence after failing batch");
    assert_eq!(after_work.attempted() - before_work.attempted(), 4);
    assert_eq!(after_work.consumed() - before_work.consumed(), 4);

    let retry_value = [7_u8];
    let retry_items = [BytesBatchItem::new(&retry_value).expect("retry bytes item")];
    let range = assembly
        .try_retain_canonical_bytes_batch(
            BytesBatchPlan::new(&retry_items).expect("retry bytes batch"),
            BatchTag,
        )
        .expect("clean prefix rollback leaves the assembly writable");
    assert_eq!(range.len(), 1);

    let fallback = assembly
        .prepare_minimal_fallback()
        .expect("retry fallback")
        .handoff();
    drop(fallback);
    assert_eq!(ledger.active_reservations(), 0);
    assert_eq!(ledger.current_live_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn unverified_prefix_rollback_enters_containment_and_preserves_live_attribution() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 3))
        .expect("rollback-containment batch assembly");
    let ledger = assembly.raw.test_ledger_probe();
    let before_active = ledger.active_reservations();
    let before_live = ledger.current_live_bytes();
    let values = [[1_u8], [2_u8], [3_u8]];
    let items = [
        SequenceBatchItem::new(&values[0]).expect("first rollback item"),
        SequenceBatchItem::new(&values[1]).expect("second rollback item"),
        SequenceBatchItem::new(&values[2]).expect("third rollback item"),
    ];

    test_fail_batch_build_at(2);
    test_panic_final_owner_drop_after(1);
    let error = assembly
        .try_retain_canonical_sequence_batch(
            SequenceBatchPlan::new(&items).expect("rollback-containment plan"),
            BatchTag,
        )
        .expect_err("unverified prefix release must abandon the batch");

    assert!(
        matches!(
            error,
            StoreBatchRuntimeError::Abandoned {
                failed_item: 2,
                cause: StoreRuntimeError::StorageDropUnverified,
            }
        ),
        "unexpected batch containment error: {error:?}"
    );
    assert_eq!(assembly.canonical_len(), 0);
    assert!(ledger.active_reservations() > before_active);
    assert!(ledger.current_live_bytes() > before_live);
    assert!(account.snapshot().poisoned());
    assert!(matches!(
        assembly.prepare_minimal_fallback(),
        Err(crate::StorePrepareError::FinalizationContainment(_))
    ));
}

#[test]
fn mid_batch_panic_uses_single_block_containment_and_never_returns_a_range() {
    let factory = test_factory();
    let account = test_account(&factory);
    let (operation, keys) = test_operation(&account);
    let mut assembly = operation
        .try_publication_assembly(publication_plan(keys, 3))
        .expect("panicking batch assembly");
    let items = [
        Utf8BatchItem::new("first").expect("first item"),
        Utf8BatchItem::new("second").expect("second item"),
        Utf8BatchItem::new("third").expect("third item"),
    ];

    test_panic_batch_build_at(1);
    let error = assembly
        .try_retain_canonical_utf8_batch(
            Utf8BatchPlan::new(&items).expect("panicking batch plan"),
            BatchTag,
        )
        .expect_err("second item panic must be contained");

    assert!(matches!(
        error,
        StoreBatchRuntimeError::Abandoned {
            failed_item: 1,
            cause: StoreRuntimeError::BuilderPanicked,
        }
    ));
    assert_eq!(assembly.canonical_len(), 0);
    assert!(account.snapshot().poisoned());
    assert!(matches!(
        assembly.try_retain_canonical_utf8_batch(
            Utf8BatchPlan::new(&[Utf8BatchItem::new("retry").expect("retry utf8 item")])
                .expect("retry utf8 batch"),
            BatchTag,
        ),
        Err(StoreBatchRuntimeError::Preflight(
            StoreRuntimeError::FallbackOnly
        ))
    ));
    assert!(matches!(
        assembly.prepare_minimal_fallback(),
        Err(crate::StorePrepareError::FinalizationContainment(_))
    ));
}
