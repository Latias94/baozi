use super::*;
use crate::DimensionPolicy;
use crate::ledger::LedgerTestProbe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc as StdArc, Barrier, Mutex as StdMutex};
use std::thread;

const BUILD_FAILED: RetainedBuildFailureCode = RetainedBuildFailureCode::new(1);

fn retained_root_production_source() -> &'static str {
    let source = include_str!("../retained.rs");
    let test_attribute = ["#[cfg(", "test)]"].concat();
    let (production, test_module) = source
        .rsplit_once(&test_attribute)
        .expect("retained source contains one final test module boundary");
    assert!(test_module.trim_start().starts_with("mod tests"));
    production
}

fn retained_production_source() -> String {
    [
        retained_root_production_source(),
        include_str!("account.rs"),
        include_str!("evidence.rs"),
        include_str!("final_owner.rs"),
        include_str!("ownership.rs"),
    ]
    .join("\n")
}

fn retained_non_final_owner_production_source() -> String {
    [
        retained_root_production_source(),
        include_str!("account.rs"),
        include_str!("evidence.rs"),
        include_str!("ownership.rs"),
    ]
    .join("\n")
}

struct PanickingDropProbe(&'static AtomicU64);

impl Drop for PanickingDropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::panic::panic_any(vec![0_u8; 8]);
    }
}

struct DropCounter(&'static AtomicU64);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct LiveDropObservation {
    ledger: LedgerTestProbe<TestPhase, TestDimension>,
    account: PhysicalStorageAccount,
    observed_active: &'static AtomicU64,
    observed_operation_live: &'static AtomicU64,
    observed_physical_bytes: &'static AtomicU64,
    observed_physical_allocations: &'static AtomicU64,
}

impl Drop for LiveDropObservation {
    fn drop(&mut self) {
        self.observed_active
            .store(self.ledger.active_reservations(), Ordering::SeqCst);
        self.observed_operation_live
            .store(self.ledger.current_live_bytes(), Ordering::SeqCst);
        let physical = self.account.snapshot();
        self.observed_physical_bytes
            .store(physical.live_retained_bytes(), Ordering::SeqCst);
        self.observed_physical_allocations
            .store(physical.live_retained_allocations(), Ordering::SeqCst);
    }
}

struct DeferredLiveDropObservation {
    ledger: StdArc<StdMutex<Option<LedgerTestProbe<TestPhase, TestDimension>>>>,
    account: PhysicalStorageAccount,
    observed_active: &'static AtomicU64,
    observed_operation_live: &'static AtomicU64,
    observed_physical_bytes: &'static AtomicU64,
    observed_physical_allocations: &'static AtomicU64,
}

impl Drop for DeferredLiveDropObservation {
    fn drop(&mut self) {
        let ledger = lock_recovering_poison(&self.ledger);
        let ledger = ledger
            .as_ref()
            .expect("baseline drop probe must be installed before destruction");
        self.observed_active
            .store(ledger.active_reservations(), Ordering::SeqCst);
        self.observed_operation_live
            .store(ledger.current_live_bytes(), Ordering::SeqCst);
        let physical = self.account.snapshot();
        self.observed_physical_bytes
            .store(physical.live_retained_bytes(), Ordering::SeqCst);
        self.observed_physical_allocations
            .store(physical.live_retained_allocations(), Ordering::SeqCst);
    }
}

struct PhysicalDropObservation {
    account: PhysicalStorageAccount,
    drop_count: &'static AtomicU64,
    observed_bytes: &'static AtomicU64,
    observed_allocations: &'static AtomicU64,
}

impl Drop for PhysicalDropObservation {
    fn drop(&mut self) {
        self.drop_count.fetch_add(1, Ordering::SeqCst);
        let physical = self.account.snapshot();
        self.observed_bytes
            .store(physical.live_retained_bytes(), Ordering::SeqCst);
        self.observed_allocations
            .store(physical.live_retained_allocations(), Ordering::SeqCst);
    }
}

struct TestOwner;

impl OperationOwner for TestOwner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TestPhase {
    Build,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TestDimension {
    Bytes,
}

static PANIC_ON_PRIMARY_DIMENSION_ORD: AtomicBool = AtomicBool::new(false);
static PRIMARY_DIMENSION_ORD_TEST_LOCK: StdMutex<()> = StdMutex::new(());

struct PrimaryDimensionOrdReset;

impl Drop for PrimaryDimensionOrdReset {
    fn drop(&mut self) {
        PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrimaryPanicDimension(u8);

impl PartialOrd for PrimaryPanicDimension {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrimaryPanicDimension {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        assert!(
            !PANIC_ON_PRIMARY_DIMENSION_ORD.load(Ordering::SeqCst),
            "primary bind invoked owner-defined dimension ordering"
        );
        self.0.cmp(&other.0)
    }
}

struct PrimaryBindDropObservation {
    ledger: StdArc<StdMutex<Option<LedgerTestProbe<TestPhase, PrimaryPanicDimension>>>>,
    drop_count: &'static AtomicU64,
    observed_active: &'static AtomicU64,
    observed_live: &'static AtomicU64,
}

impl Drop for PrimaryBindDropObservation {
    fn drop(&mut self) {
        let ledger = lock_recovering_poison(&self.ledger);
        let ledger = ledger
            .as_ref()
            .expect("primary bind probe must remain installed through storage drop");
        self.drop_count.fetch_add(1, Ordering::SeqCst);
        self.observed_active
            .store(ledger.active_reservations(), Ordering::SeqCst);
        self.observed_live
            .store(ledger.current_live_bytes(), Ordering::SeqCst);
    }
}

#[derive(Clone, Copy)]
struct CanonicalRootTag;

#[derive(Clone, Copy)]
struct CanonicalLeafTag;

#[derive(Clone, Copy)]
struct EvidenceRootTag;

#[derive(Clone, Copy)]
struct EvidenceDetailTag;

#[derive(Debug, PartialEq, Eq)]
enum TestCanonicalBlock {
    Root {
        leaf: CanonicalRetainedSlot<TestCanonicalBlock, CanonicalLeafTag>,
    },
    Leaf(u64),
}

#[derive(Debug, PartialEq, Eq)]
enum TestEvidenceBlock {
    Root {
        detail: OutcomeEvidenceRetainedSlot<TestEvidenceBlock, EvidenceDetailTag>,
    },
    Detail(u64),
}

type TestOperation =
    RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension, u32>;
type TestPublication<CanonicalBlock, EvidenceBlock> = RetainedPublicationAssembly<
    TestOwner,
    TestPhase,
    TestProgress,
    TestUnit,
    TestDimension,
    u32,
    CanonicalBlock,
    EvidenceBlock,
>;

enum PublishedRetainedPublicationParts<CanonicalBlock, EvidenceBlock, F, D> {
    Success {
        canonical: PublishedCanonicalOutput<CanonicalBlock>,
        outcome_evidence: PublishedOutcomeEvidenceOutput<EvidenceBlock>,
        ledger_evidence: PublishedLedgerEvidence<D>,
    },
    RichFailure {
        outcome_evidence: PublishedRichFailureOutput<EvidenceBlock>,
        ledger_evidence: PublishedLedgerEvidence<D>,
    },
    MinimalFallback {
        fallback: PublishedFallbackOutput<F>,
        ledger_evidence: PublishedLedgerEvidence<D>,
    },
}

fn handoff_for_test<CanonicalBlock, EvidenceBlock, F, D>(
    prepared: PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, D>,
) -> PublishedRetainedPublicationParts<CanonicalBlock, EvidenceBlock, F, D>
where
    D: Copy + Ord,
{
    match prepared.into_outcome() {
        PreparedRetainedPublicationOutcome::Success(prepared) => {
            let published = prepared.handoff();
            PublishedRetainedPublicationParts::Success {
                canonical: published.canonical,
                outcome_evidence: published.outcome_evidence,
                ledger_evidence: published.ledger_evidence,
            }
        }
        PreparedRetainedPublicationOutcome::RichFailure(prepared) => {
            let published = prepared.handoff();
            PublishedRetainedPublicationParts::RichFailure {
                outcome_evidence: published.outcome_evidence,
                ledger_evidence: published.ledger_evidence,
            }
        }
        PreparedRetainedPublicationOutcome::MinimalFallback(prepared) => {
            let published = prepared.handoff();
            PublishedRetainedPublicationParts::MinimalFallback {
                fallback: published.fallback,
                ledger_evidence: published.ledger_evidence,
            }
        }
    }
}

fn share_work() -> RetainedHandleSharePlan<TestDimension> {
    RetainedHandleSharePlan::new(TestDimension::Bytes, TestDimension::Bytes)
}

fn writer_plan(
    attributions: usize,
    commitments: usize,
) -> RetainedWriterPlan<TestPhase, TestDimension> {
    RetainedWriterPlan::new(
        TestPhase::Build,
        TestDimension::Bytes,
        TestDimension::Bytes,
        RetainedBatchLimits::new(attributions, commitments),
    )
}

fn publication_plan(
    canonical_blocks: usize,
    evidence_blocks: usize,
) -> RetainedPublicationPlan<TestPhase, TestDimension> {
    RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(
            writer_plan(canonical_blocks + 1, 2),
            canonical_blocks,
            canonical_blocks,
        ),
        RetainedBlockTablePlan::new(
            writer_plan(evidence_blocks + 1, 2),
            evidence_blocks,
            evidence_blocks,
        ),
    )
}

fn test_operation(factory: &PhysicalStorageAccountFactory, evidence_limit: u64) -> TestOperation {
    test_operation_with_writer_limits(factory, evidence_limit, RetainedBatchLimits::new(2, 2))
}

fn test_operation_with_writer_limits(
    factory: &PhysicalStorageAccountFactory,
    evidence_limit: u64,
    writer_limits: RetainedBatchLimits,
) -> TestOperation {
    let account = account(factory, u64::MAX, 128);
    test_operation_on_account_with_control(
        &account,
        evidence_limit,
        writer_limits,
        OperationControl::default(),
    )
}

fn test_operation_on_account_with_control(
    account: &PhysicalStorageAccount,
    evidence_limit: u64,
    writer_limits: RetainedBatchLimits,
    control: OperationControl<TestProgress, TestUnit>,
) -> TestOperation {
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(evidence_limit);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(
            writer_limits.attribution_slots(),
            writer_limits.commitment_slots(),
        ),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    match TestOperation::try_start(control, limits, account, plan, || Ok(7_u32)) {
        Ok(operation) => operation,
        Err(_) => panic!("test operation bootstrap must succeed"),
    }
}

fn test_operation_with_fallback<F>(
    factory: &PhysicalStorageAccountFactory,
    fallback: F,
) -> RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension, F>
where
    F: Send + Sync + 'static,
{
    let account = account(factory, u64::MAX, 128);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    RetainedOperation::try_start(OperationControl::default(), limits, &account, plan, || {
        Ok(fallback)
    })
    .unwrap_or_else(|_| panic!("test operation bootstrap must succeed"))
}

fn final_progress() -> FinalProgress<TestProgress, TestUnit> {
    FinalProgress::new(TestProgress::Build, TestUnit::Items, 1)
}

fn account_factory(max_live_accounts: u64) -> PhysicalStorageAccountFactory {
    let limits = PhysicalStorageAccountFactoryLimits::new(u64::MAX, max_live_accounts).unwrap();
    PhysicalStorageAccountFactory::try_new(limits).unwrap()
}

fn account(
    factory: &PhysicalStorageAccountFactory,
    bytes: u64,
    allocations: u64,
) -> PhysicalStorageAccount {
    factory
        .try_create_account(PhysicalStorageAccountLimits::new(bytes, allocations).unwrap())
        .unwrap()
}

fn admitted_startup_share_work(account: &PhysicalStorageAccount) -> StartupAccountOwners {
    let share = || PhysicalStorageAccount {
        inner: account.inner.share(),
    };
    StartupAccountOwners {
        fallback_account: share(),
        evidence_account: share(),
        fallback_lease_owner: share(),
        evidence_lease_owner: share(),
    }
}

fn startup_limits_and_plan() -> (
    LedgerLimits<TestDimension>,
    RetainedOperationStartPlan<TestPhase, TestDimension>,
) {
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)])
        .unwrap_or_else(|_| panic!("startup test limits must qualify"));
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    (limits, plan)
}

fn reset_startup_test_observations() {
    let _ = take_injected_start_evidence_build_failure();
    let _ = take_injected_start_evidence_build_panic();
    let _ = take_injected_start_fallback_account_anchor_loss();
    let _ = take_start_evidence_build_calls();
    let _ = take_startup_failure_ledger_snapshot();
}

fn empty_plan() -> RetainedAllocationPlan {
    RetainedAllocationPlan::new(0, 0).unwrap()
}

fn retain_scalar_success(
    assembly: &mut TestPublication<u64, u64>,
) -> (
    CanonicalRetainedSlot<u64, CanonicalRootTag>,
    OutcomeEvidenceRetainedSlot<u64, EvidenceRootTag>,
) {
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical scalar must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence scalar must retain"));
    (canonical, evidence)
}

type PublishedScalarSource = (
    PublishedCanonicalOutput<u64>,
    CanonicalRetainedSlot<u64, CanonicalRootTag>,
    PublishedOutcomeEvidenceOutput<u64>,
    OutcomeEvidenceRetainedSlot<u64, EvidenceRootTag>,
);

fn publish_scalar_source(account: &PhysicalStorageAccount) -> PublishedScalarSource {
    let operation = test_operation_on_account_with_control(
        account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("source publication must open"));
    let (canonical_slot, evidence_slot) = retain_scalar_success(&mut assembly);
    let validated = match assembly.validate_success(
        canonical_slot,
        evidence_slot,
        |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        },
    ) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("source publication must validate"),
    };
    let published = handoff_for_test(
        validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("source publication must prepare")),
    );
    match published {
        PublishedRetainedPublicationParts::Success {
            canonical,
            outcome_evidence,
            ledger_evidence,
        } => {
            drop(ledger_evidence);
            (canonical, canonical_slot, outcome_evidence, evidence_slot)
        }
        _ => panic!("source publication must succeed"),
    }
}

#[test]
fn block_table_success_freezes_dual_batches_and_splits_owners() {
    let factory = account_factory(1);
    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(3, 2));
    let mut assembly: TestPublication<TestCanonicalBlock, TestEvidenceBlock> = operation
        .try_publication_assembly(publication_plan(2, 2))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let leaf = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Leaf(11), CanonicalLeafTag))
        })
        .unwrap_or_else(|_| panic!("canonical leaf must retain"));
    let canonical_root = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Root { leaf }, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical root must retain"));
    let detail = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Detail(7), EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("evidence detail must retain"));
    let evidence_root = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Root { detail }, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence root must retain"));

    assert_eq!(assembly.canonical_len(), 2);
    assert_eq!(assembly.outcome_evidence_len(), 2);
    assembly
        .try_commit_canonical(TestDimension::Bytes, 5)
        .unwrap_or_else(|_| panic!("canonical commitment must fit"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 6)
        .unwrap_or_else(|_| panic!("evidence commitment must fit"));

    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical_root, evidence_root| {
            let Some(TestCanonicalBlock::Root { leaf }) = session.canonical(canonical_root) else {
                return false;
            };
            if !matches!(session.canonical(*leaf), Some(TestCanonicalBlock::Leaf(11))) {
                return false;
            }
            // A DAG may reach the same slot more than once; only unique members are counted.
            if !matches!(session.canonical(*leaf), Some(TestCanonicalBlock::Leaf(11))) {
                return false;
            }
            let Some(TestEvidenceBlock::Root { detail }) = session.outcome_evidence(evidence_root)
            else {
                return false;
            };
            matches!(
                session.outcome_evidence(*detail),
                Some(TestEvidenceBlock::Detail(7))
            )
        },
    ) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("dual publication graph must validate"),
    };
    let prepared = validated
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("dual publication must prepare"));
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::Success {
            canonical,
            outcome_evidence,
            ledger_evidence,
        } => {
            assert!(matches!(canonical.root(), TestCanonicalBlock::Root { .. }));
            assert!(matches!(
                outcome_evidence.root(),
                TestEvidenceBlock::Root { .. }
            ));
            let snapshot = ledger_evidence
                .dimension(TestDimension::Bytes)
                .expect("published dimension evidence");
            assert_eq!(snapshot.committed_canonical_output(), 5);
            assert_eq!(snapshot.published_outcome_evidence(), 7);
        }
        _ => panic!("success must not publish a failure owner"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn inherited_blocks_use_fresh_slots_without_new_attribution_or_physical_charge() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (source_canonical, source_canonical_slot, source_evidence, source_evidence_slot) =
        publish_scalar_source(&account);
    let canonical_shares_before = source_canonical.aggregate().inner.table.as_ref().members[0]
        .allocation
        .share_call_count();
    let evidence_shares_before = source_evidence.aggregate().inner.table.as_ref().members[0]
        .allocation
        .share_call_count();

    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(3, 2),
        OperationControl::default(),
    );
    let plan = RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(writer_plan(2, 2), 3, 1),
        RetainedBlockTablePlan::new(writer_plan(1, 2), 3, 0),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(plan)
        .unwrap_or_else(|_| panic!("inherited publication must open"));
    let physical_before_shares = account.snapshot();

    let canonical_root = assembly
        .try_share_canonical(source_canonical.aggregate(), source_canonical_slot)
        .unwrap_or_else(|_| panic!("first canonical share must succeed"));
    let canonical_second = assembly
        .try_share_canonical(source_canonical.aggregate(), source_canonical_slot)
        .unwrap_or_else(|_| panic!("second canonical share must succeed"));
    let evidence_root = assembly
        .try_share_outcome_evidence(source_evidence.aggregate(), source_evidence_slot)
        .unwrap_or_else(|_| panic!("first evidence share must succeed"));
    let evidence_second = assembly
        .try_share_outcome_evidence(source_evidence.aggregate(), source_evidence_slot)
        .unwrap_or_else(|_| panic!("second evidence share must succeed"));
    let evidence_third = assembly
        .try_share_outcome_evidence(source_evidence.aggregate(), source_evidence_slot)
        .unwrap_or_else(|_| panic!("third evidence share must succeed"));

    assert_eq!(account.snapshot(), physical_before_shares);
    assert_eq!(assembly.canonical_len(), 2);
    assert_eq!(assembly.outcome_evidence_len(), 3);
    assert_eq!(
        assembly
            .canonical_batch
            .as_ref()
            .expect("canonical attribution batch")
            .attribution_len(),
        1
    );
    assert_eq!(
        assembly
            .evidence_batch
            .as_ref()
            .expect("evidence attribution batch")
            .attribution_len(),
        1
    );
    assert_eq!(
        source_canonical.aggregate().inner.table.as_ref().members[0]
            .allocation
            .share_call_count(),
        canonical_shares_before + 2
    );
    assert_eq!(
        source_evidence.aggregate().inner.table.as_ref().members[0]
            .allocation
            .share_call_count(),
        evidence_shares_before + 3
    );
    assert!(source_canonical.aggregate().get(canonical_root).is_none());
    assert!(source_evidence.aggregate().get(evidence_root).is_none());

    let canonical_new = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(13, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("one new canonical block must fit"));
    assert_eq!(assembly.canonical_len(), 3);
    assert_eq!(
        assembly
            .canonical_batch
            .as_ref()
            .expect("canonical attribution batch")
            .attribution_len(),
        2
    );
    assert!(
        account.snapshot().live_retained_bytes() > physical_before_shares.live_retained_bytes()
    );

    let validated = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical_root, evidence_root| {
            session.canonical(canonical_root) == Some(&11)
                && session.canonical(canonical_second) == Some(&11)
                && session.canonical(canonical_new) == Some(&13)
                && session.outcome_evidence(evidence_root) == Some(&7)
                && session.outcome_evidence(evidence_second) == Some(&7)
                && session.outcome_evidence(evidence_third) == Some(&7)
        },
    ) {
        RetainedSuccessValidation::Valid(validated) => validated,
        _ => panic!("inherited publication must validate"),
    };
    let published = handoff_for_test(
        validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("inherited publication must prepare")),
    );
    match &published {
        PublishedRetainedPublicationParts::Success {
            canonical,
            outcome_evidence,
            ..
        } => {
            assert_eq!(canonical.aggregate().get(canonical_root), Some(&11));
            assert!(canonical.aggregate().get(source_canonical_slot).is_none());
            assert_eq!(outcome_evidence.aggregate().get(evidence_root), Some(&7));
            assert!(
                outcome_evidence
                    .aggregate()
                    .get(source_evidence_slot)
                    .is_none()
            );
        }
        _ => panic!("inherited publication must succeed"),
    }
    drop(published);
    drop(source_canonical);
    drop(source_evidence);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn foreign_aggregate_is_rejected_before_member_access_or_reference_clone() {
    let factory = account_factory(2);
    let source_account = account(&factory, u64::MAX, 128);
    let target_account = account(&factory, u64::MAX, 128);
    let (source_canonical, source_slot, source_evidence, _) =
        publish_scalar_source(&source_account);
    let table_shares_before = source_canonical.aggregate().inner.table.share_call_count();
    let block_shares_before = source_canonical.aggregate().inner.table.as_ref().members[0]
        .allocation
        .share_call_count();
    let _ = take_test_member_lookups();

    let operation = test_operation_on_account_with_control(
        &target_account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("foreign-account target must open"));
    let result = assembly.try_share_canonical(source_canonical.aggregate(), source_slot);

    assert!(matches!(
        result,
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::ForeignAccount
        ))
    ));
    assert_eq!(take_test_member_lookups(), 0);
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(
        source_canonical.aggregate().inner.table.share_call_count(),
        table_shares_before
    );
    assert_eq!(
        source_canonical.aggregate().inner.table.as_ref().members[0]
            .allocation
            .share_call_count(),
        block_shares_before
    );
    drop(handoff_for_test(
        assembly
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("foreign rejection must preserve fallback")),
    ));
    drop(source_canonical);
    drop(source_evidence);
    drop(source_account);
    drop(target_account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn share_work_rejection_does_not_clone_or_mutate_the_target_table() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (source_canonical, source_slot, source_evidence, _) = publish_scalar_source(&account);
    let block_shares_before = source_canonical.aggregate().inner.table.as_ref().members[0]
        .allocation
        .share_call_count();
    let physical_before_operation = account.snapshot();
    let policy = DimensionPolicy::reservable_bytes(15, 15, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let start = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let operation =
        TestOperation::try_start(OperationControl::default(), limits, &account, start, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("exact share-work operation must start"));
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("exact share-work tables must open"));
    let physical_before_share = account.snapshot();

    let result = assembly.try_share_canonical(source_canonical.aggregate(), source_slot);

    assert!(matches!(
        result,
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Ledger(LedgerCause::LimitExceeded { .. })
        ))
    ));
    assert_eq!(assembly.canonical_len(), 0);
    assert_eq!(account.snapshot(), physical_before_share);
    assert!(
        physical_before_share.live_retained_bytes()
            > physical_before_operation.live_retained_bytes()
    );
    assert_eq!(
        source_canonical.aggregate().inner.table.as_ref().members[0]
            .allocation
            .share_call_count(),
        block_shares_before
    );
    drop(handoff_for_test(
        assembly
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("share-work rejection must preserve fallback")),
    ));
    drop(source_canonical);
    drop(source_evidence);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn invalid_publication_capacity_is_rejected_before_every_open_side_effect() {
    let factory = account_factory(1);
    let invalid_plans = [
        (
            "attribution capacity overflow",
            RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                writer_plan(usize::MAX, 1),
                1,
                0,
            )),
            false,
        ),
        (
            "commitment capacity overflow",
            RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                writer_plan(1, usize::MAX),
                1,
                0,
            )),
            false,
        ),
        (
            "member capacity overflow",
            RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                writer_plan(1, 1),
                usize::MAX,
                0,
            )),
            true,
        ),
        (
            "new block limit exceeds total blocks",
            RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(
                writer_plan(2, 1),
                1,
                2,
            )),
            false,
        ),
    ];

    for (index, (case, invalid_plan, expects_batch_capacity)) in
        invalid_plans.into_iter().enumerate()
    {
        let operation = test_operation(&factory, 64);
        let dimension_before = operation
            .coordinator
            .dimension_evidence(TestDimension::Bytes)
            .unwrap_or_else(|_| panic!("{case}: dimension evidence before open"));
        let physical_before = operation.account().snapshot();
        let account_shares_before = operation.account().share_call_count();
        let set_ids_before = test_retained_set_id_requests();
        let batch_allocations_before = crate::ledger::test_attribution_batch_allocation_entries();

        let error = match operation.try_publication_assembly::<u64, u64>(invalid_plan) {
            Ok(_) => panic!("{case}: invalid plan unexpectedly opened"),
            Err(error) => error,
        };
        assert!(
            if expects_batch_capacity {
                matches!(error.cause(), RetainedWriterError::BatchCapacity)
            } else {
                matches!(error.cause(), RetainedWriterError::InvalidPublicationPlan)
            },
            "{case}: unexpected preflight cause: {}",
            error.cause()
        );
        let operation = error
            .try_into_operation()
            .unwrap_or_else(|_| panic!("{case}: pure preflight must return the operation"));

        assert_eq!(test_retained_set_id_requests(), set_ids_before, "{case}");
        assert_eq!(
            crate::ledger::test_attribution_batch_allocation_entries(),
            batch_allocations_before,
            "{case}"
        );
        assert_eq!(
            operation.account().share_call_count(),
            account_shares_before,
            "{case}"
        );
        assert_eq!(operation.account().snapshot(), physical_before, "{case}");
        assert_eq!(
            operation
                .coordinator
                .dimension_evidence(TestDimension::Bytes)
                .unwrap_or_else(|_| panic!("{case}: dimension evidence after rejection")),
            dimension_before,
            "{case}"
        );

        if index == 2 {
            let assembly: TestPublication<u64, u64> = operation
                .try_publication_assembly(RetainedPublicationPlan::rich_failure(
                    RetainedBlockTablePlan::new(writer_plan(1, 1), 1, 0),
                ))
                .unwrap_or_else(|_| panic!("{case}: valid retry must open"));
            drop(handoff_for_test(
                assembly
                    .prepare_minimal_fallback()
                    .unwrap_or_else(|_| panic!("{case}: retry fallback must prepare")),
            ));
        } else {
            drop(handoff_for_test(
                operation
                    .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(70))
                    .unwrap_or_else(|_| panic!("direct fallback must prepare")),
            ));
        }
        assert_eq!(factory.snapshot().live_accounts(), 0, "{case}");
    }
}

#[test]
fn recoverable_open_failures_prepare_the_baseline_without_tables_or_roots() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let invalid_plan = RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 2),
        RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1),
    );
    let error = match operation.try_publication_assembly::<u64, u64>(invalid_plan) {
        Ok(_) => panic!("invalid total/new relation must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        RetainedWriterError::InvalidPublicationPlan
    ));
    let operation = error
        .try_into_operation()
        .unwrap_or_else(|_| panic!("invalid plan must return the untouched operation"));
    let published = handoff_for_test(
        operation
            .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(71))
            .unwrap_or_else(|_| panic!("direct fallback must prepare")),
    );
    match published {
        PublishedRetainedPublicationParts::MinimalFallback {
            fallback,
            ledger_evidence,
        } => {
            assert_eq!(fallback.cause(), PreparationFailureCode::domain(71));
            assert_eq!(*fallback.value(), 7);
            assert_eq!(
                ledger_evidence
                    .dimension(TestDimension::Bytes)
                    .expect("direct fallback ledger evidence")
                    .committed_canonical_output(),
                0
            );
        }
        _ => panic!("invalid plan must publish minimal fallback"),
    }

    let operation = test_operation(&factory, 64);
    fail_next_final_owner_allocation();
    let error = match operation.try_publication_assembly::<u64, u64>(publication_plan(1, 1)) {
        Ok(_) => panic!("injected table allocation failure must reject open"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
    ));
    let operation = error
        .try_into_operation()
        .unwrap_or_else(|_| panic!("verified allocation failure must return the operation"));
    let published = handoff_for_test(
        operation
            .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(72))
            .unwrap_or_else(|_| panic!("direct fallback must prepare")),
    );
    assert!(matches!(
        &published,
        PublishedRetainedPublicationParts::MinimalFallback { fallback, .. }
            if fallback.cause() == PreparationFailureCode::domain(72)
    ));
    drop(published);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn second_table_allocation_failure_releases_canonical_and_returns_operation() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    fail_final_owner_allocation_after(1);

    let error = match operation.try_publication_assembly::<u64, u64>(publication_plan(1, 1)) {
        Ok(_) => panic!("evidence table allocation failure must reject open"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
    ));
    let operation = error
        .try_into_operation()
        .unwrap_or_else(|_| panic!("clean canonical rollback must return the operation"));
    let published = handoff_for_test(
        operation
            .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(73))
            .unwrap_or_else(|_| panic!("recovered operation must prepare fallback")),
    );
    drop(published);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn second_table_allocation_failure_reports_canonical_storage_cleanup_failure() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    fail_final_owner_allocation_after(1);
    test_panic_final_owner_drop_after(0);

    let error = match operation.try_publication_assembly::<u64, u64>(publication_plan(1, 1)) {
        Ok(_) => panic!("combined evidence allocation and canonical drop failure must reject"),
        Err(error) => error,
    };
    assert!(matches!(
        error.cause(),
        RetainedWriterError::StorageDropUnverified
    ));
    assert!(matches!(
        error.try_into_operation(),
        Err(RetainedWriterError::StorageDropUnverified)
    ));
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn semantic_rejection_can_append_evidence_and_prepare_a_rich_failure() {
    let factory = account_factory(1);
    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(3, 2));
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 2))
        .unwrap_or_else(|_| panic!("semantic rejection publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);

    let mut failure = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |session, canonical, evidence| {
            let _ = session.canonical(canonical);
            let _ = session.outcome_evidence(evidence);
            false
        },
    ) {
        RetainedSuccessValidation::Rejected(failure) => failure,
        _ => panic!("semantic rejection must preserve evidence-building ownership"),
    };
    let detail = failure
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9, EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("bounded failure detail must retain"));
    failure
        .try_commit_outcome_evidence(TestDimension::Bytes, 3)
        .unwrap_or_else(|_| panic!("failure evidence commitment must fit"));
    let validated = match failure.validate_rich_failure(evidence_root, |session, root| {
        session.outcome_evidence(root) == Some(&7) && session.outcome_evidence(detail) == Some(&9)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("augmented failure evidence must validate"),
    };
    let published = handoff_for_test(
        validated
            .try_prepare(PreparationFailureCode::domain(91), final_progress())
            .unwrap_or_else(|_| panic!("rich failure must prepare")),
    );
    match published {
        PublishedRetainedPublicationParts::RichFailure {
            outcome_evidence,
            ledger_evidence,
        } => {
            assert_eq!(outcome_evidence.cause(), PreparationFailureCode::domain(91));
            assert_eq!(outcome_evidence.outcome_evidence().root(), &7);
            assert_eq!(
                outcome_evidence.outcome_evidence().aggregate().get(detail),
                Some(&9)
            );
            assert_eq!(
                ledger_evidence
                    .dimension(TestDimension::Bytes)
                    .expect("rich failure ledger evidence")
                    .committed_canonical_output(),
                0
            );
        }
        _ => panic!("semantic rejection must publish rich failure only"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn validation_panic_transitions_to_fallback_only() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("validation-panic publication must open"));
    let (canonical_root, evidence_root) = retain_scalar_success(&mut assembly);

    let fallback_only = match assembly.validate_success(
        canonical_root,
        evidence_root,
        |_session, _canonical, _evidence| panic!("validator panic"),
    ) {
        RetainedSuccessValidation::FallbackOnly(fallback) => fallback,
        _ => panic!("validator panic must expose only fallback"),
    };
    let published = handoff_for_test(
        fallback_only
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("validation panic fallback must prepare")),
    );
    match &published {
        PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
            assert_eq!(
                fallback.cause().class(),
                PreparationFailureClass::ProductGraph
            );
            assert_eq!(fallback.cause().code(), 5);
        }
        _ => panic!("validation panic must publish minimal fallback"),
    }
    drop(published);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn repeated_semantic_rejection_preserves_the_product_graph_fallback_cause() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("repeated-rejection publication must open"));
    let (canonical, evidence) = retain_scalar_success(&mut assembly);

    let failure =
        match assembly
            .validate_success(canonical, evidence, |_session, _canonical, _evidence| false)
        {
            RetainedSuccessValidation::Rejected(failure) => failure,
            _ => panic!("first semantic rejection must expose failure building"),
        };
    let failure = match failure.validate_rich_failure(evidence, |_session, _evidence| false) {
        RetainedFailureValidation::Rejected(failure) => failure,
        _ => panic!("second semantic rejection must preserve failure building"),
    };
    let published = handoff_for_test(
        failure
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("repeated rejection fallback must prepare")),
    );
    match &published {
        PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
            assert_eq!(
                fallback.cause().class(),
                PreparationFailureClass::ProductGraph
            );
            assert_eq!(fallback.cause().code(), 4);
        }
        _ => panic!("repeated semantic rejection must publish only fallback"),
    }
    drop(published);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn consuming_closure_validation_rejects_orphans_and_retagged_slots() {
    let factory = account_factory(1);
    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(3, 2));
    let mut assembly: TestPublication<TestCanonicalBlock, TestEvidenceBlock> = operation
        .try_publication_assembly(publication_plan(2, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let orphan = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Leaf(1), CanonicalLeafTag))
        })
        .unwrap_or_else(|_| panic!("orphan candidate must retain"));
    let root = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Root { leaf: orphan }, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("root must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Detail(9), EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("evidence must retain"));

    let rejected = match assembly.validate_success(root, evidence, |session, root, evidence| {
        session.canonical(root).is_some() && session.outcome_evidence(evidence).is_some()
    }) {
        RetainedSuccessValidation::Rejected(rejected) => rejected,
        _ => panic!("orphan graph must be rejected"),
    };
    let prepared = rejected
        .prepare_minimal_fallback()
        .unwrap_or_else(|_| panic!("orphan rejection must prepare minimal fallback"));
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
            assert_eq!(
                fallback.cause().class(),
                PreparationFailureClass::ProductGraph
            );
            assert_eq!(fallback.cause().code(), 4);
        }
        _ => panic!("orphan rejection must publish minimal fallback"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);

    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(3, 2));
    let mut assembly: TestPublication<TestCanonicalBlock, TestEvidenceBlock> = operation
        .try_publication_assembly(publication_plan(2, 1))
        .unwrap_or_else(|_| panic!("publication assembly must reopen"));
    let orphan = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Leaf(1), CanonicalLeafTag))
        })
        .unwrap_or_else(|_| panic!("orphan candidate must retain"));
    let root = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Root { leaf: orphan }, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("root must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Detail(9), EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("evidence must retain"));
    let retagged: CanonicalRetainedSlot<TestCanonicalBlock, CanonicalRootTag> =
        CanonicalRetainedSlot {
            identity: orphan.identity,
            marker: PhantomData,
        };
    let rejected = match assembly.validate_success(root, evidence, |session, root, evidence| {
        session.canonical(root).is_some()
            && session.canonical(retagged).is_some()
            && session.outcome_evidence(evidence).is_some()
    }) {
        RetainedSuccessValidation::Rejected(rejected) => rejected,
        _ => panic!("retagged graph must be rejected"),
    };
    let prepared = rejected
        .prepare_minimal_fallback()
        .unwrap_or_else(|_| panic!("retag rejection must prepare minimal fallback"));
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
            assert_eq!(
                fallback.cause().class(),
                PreparationFailureClass::ProductGraph
            );
            assert_eq!(fallback.cause().code(), 4);
        }
        _ => panic!("retag rejection must publish minimal fallback"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn block_limits_reject_before_running_the_builder_and_minimal_fallback_stays_available() {
    static BUILDER_RAN: AtomicBool = AtomicBool::new(false);

    let factory = account_factory(1);
    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(2, 2));
    let mut assembly: TestPublication<TestCanonicalBlock, TestEvidenceBlock> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let _ = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestCanonicalBlock::Leaf(1), CanonicalLeafTag))
        })
        .unwrap_or_else(|_| panic!("first block must fit"));
    BUILDER_RAN.store(false, Ordering::SeqCst);
    assert!(matches!(
        assembly.try_retain_canonical(empty_plan(), |emplacer| {
            BUILDER_RAN.store(true, Ordering::SeqCst);
            Ok(emplacer.place(TestCanonicalBlock::Leaf(2), CanonicalLeafTag))
        }),
        Err(RetainedPublicationError::CanonicalSlotLimit)
    ));
    assert!(!BUILDER_RAN.load(Ordering::SeqCst));
    let _ = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(TestEvidenceBlock::Detail(3), EvidenceDetailTag))
        })
        .unwrap_or_else(|_| panic!("evidence block must fit"));
    let prepared = assembly
        .prepare_minimal_fallback()
        .unwrap_or_else(|_| panic!("minimal fallback must remain available"));
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
            assert_eq!(*fallback.value(), 7);
        }
        _ => panic!("direct fallback must publish the minimal owner"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn partial_canonical_downgrades_to_rich_failure_after_storage_drops_first() {
    static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_OPERATION_LIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_PHYSICAL_BYTES: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_PHYSICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(2, 2));
    let mut assembly: TestPublication<LiveDropObservation, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let ledger = assembly
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .coordinator
        .retained_ledger();
    let observation = LiveDropObservation {
        ledger: ledger.test_probe(),
        account: assembly
            .account
            .as_ref()
            .expect("assembly retains account")
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("test account share must be admitted")),
        observed_active: &OBSERVED_ACTIVE,
        observed_operation_live: &OBSERVED_OPERATION_LIVE,
        observed_physical_bytes: &OBSERVED_PHYSICAL_BYTES,
        observed_physical_allocations: &OBSERVED_PHYSICAL_ALLOCATIONS,
    };
    let _partial = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(observation, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("partial canonical block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(5, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence root must retain"));
    OBSERVED_ACTIVE.store(0, Ordering::SeqCst);
    OBSERVED_OPERATION_LIVE.store(0, Ordering::SeqCst);
    OBSERVED_PHYSICAL_BYTES.store(0, Ordering::SeqCst);
    OBSERVED_PHYSICAL_ALLOCATIONS.store(0, Ordering::SeqCst);
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root) == Some(&5)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("partial canonical failure graph must validate"),
    };
    let prepared = validated
        .try_prepare(PreparationFailureCode::domain(9), final_progress())
        .unwrap_or_else(|_| panic!("partial canonical must downgrade to rich failure"));

    assert!(OBSERVED_ACTIVE.load(Ordering::SeqCst) > 0);
    assert!(OBSERVED_OPERATION_LIVE.load(Ordering::SeqCst) > 0);
    assert!(OBSERVED_PHYSICAL_BYTES.load(Ordering::SeqCst) > 0);
    assert!(OBSERVED_PHYSICAL_ALLOCATIONS.load(Ordering::SeqCst) > 0);
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::RichFailure {
            outcome_evidence, ..
        } => {
            assert_eq!(*outcome_evidence.outcome_evidence().root(), 5);
            assert_eq!(outcome_evidence.cause(), PreparationFailureCode::domain(9));
        }
        _ => panic!("partial canonical must not publish canonical output"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn failure_only_plan_rejects_canonical_construction_and_publishes_evidence() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1));
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(plan)
        .unwrap_or_else(|_| panic!("failure-only assembly must open"));
    assert!(matches!(
        assembly.try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        }),
        Err(RetainedPublicationError::WrongOutcomeMode)
    ));
    assert!(matches!(
        assembly.try_commit_canonical(TestDimension::Bytes, 1),
        Err(RetainedPublicationError::WrongOutcomeMode)
    ));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("failure evidence must retain"));
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root) == Some(&7)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("failure-only graph must validate"),
    };
    match handoff_for_test(
        validated
            .try_prepare(PreparationFailureCode::domain(3), final_progress())
            .unwrap_or_else(|_| panic!("failure-only publication must prepare")),
    ) {
        PublishedRetainedPublicationParts::RichFailure {
            outcome_evidence, ..
        } => assert_eq!(*outcome_evidence.outcome_evidence().root(), 7),
        _ => panic!("failure-only plan cannot publish canonical output"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn success_control_rejection_atomically_downgrades_to_rich_failure() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let control = OperationControl::default().with_progress_observer(
        |_event: crate::ProgressEvent<TestProgress, TestUnit>| crate::ProgressDecision::Cancel,
    );
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        control,
    );
    drop(account);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let (canonical, evidence) = retain_scalar_success(&mut assembly);

    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("control rejection candidate must validate"),
        };
    match handoff_for_test(
        validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("control rejection must produce a rich failure")),
    ) {
        PublishedRetainedPublicationParts::RichFailure {
            outcome_evidence, ..
        } => {
            assert_eq!(
                outcome_evidence.cause().class(),
                PreparationFailureClass::Control
            );
            assert_eq!(*outcome_evidence.outcome_evidence().root(), 7);
        }
        _ => panic!("control rejection must not publish canonical output"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn evidence_primary_rejection_uses_minimal_fallback_without_retry() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 4);
    let plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1));
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(plan)
        .unwrap_or_else(|_| panic!("failure-only assembly must open"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("failure evidence must retain"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root) == Some(&7)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("evidence rejection candidate must validate"),
    };
    match handoff_for_test(
        validated
            .try_prepare(PreparationFailureCode::domain(5), final_progress())
            .unwrap_or_else(|_| panic!("ledger rejection must retain minimal fallback")),
    ) {
        PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
            assert_eq!(*fallback.value(), 7);
            assert_eq!(fallback.cause().class(), PreparationFailureClass::Ledger);
        }
        _ => panic!("primary ledger rejection must not retry rich failure"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn success_primary_rejection_drops_both_tables_before_minimal_fallback() {
    static CANONICAL_DROPS: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);

    CANONICAL_DROPS.store(0, Ordering::SeqCst);
    EVIDENCE_DROPS.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let operation = test_operation(&factory, 4);
    let mut assembly: TestPublication<DropCounter, DropCounter> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("success rejection assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical drop counter must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence drop counter must retain"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));

    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence).is_some()
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("success rejection candidate must validate"),
        };
    let prepared = validated
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("success rejection must prepare minimal fallback"));
    assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 1);
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
            assert_eq!(fallback.cause().class(), PreparationFailureClass::Ledger);
        }
        _ => panic!("success ledger rejection must publish minimal fallback"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn primary_ordering_panic_drops_storage_while_attribution_remains_live() {
    static BLOCK_DROPS: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_LIVE: AtomicU64 = AtomicU64::new(0);

    let _ord_lock = PRIMARY_DIMENSION_ORD_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _reset = PrimaryDimensionOrdReset;
    PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);
    BLOCK_DROPS.store(0, Ordering::SeqCst);
    OBSERVED_ACTIVE.store(0, Ordering::SeqCst);
    OBSERVED_LIVE.store(0, Ordering::SeqCst);

    type PanicOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, PrimaryPanicDimension, u32>;
    type PanicPublication = RetainedPublicationAssembly<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        PrimaryPanicDimension,
        u32,
        PrimaryBindDropObservation,
        u64,
    >;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let dimension = PrimaryPanicDimension(7);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(dimension, policy)])
        .unwrap_or_else(|_| panic!("panic-order policy must be valid"));
    let writer = RetainedWriterPlan::new(
        TestPhase::Build,
        dimension,
        dimension,
        RetainedBatchLimits::new(2, 2),
    );
    let start = RetainedOperationStartPlan::new(
        writer,
        RetainedHandleSharePlan::new(dimension, dimension),
        empty_plan(),
        OutputCommitment::new(dimension, 1),
        OutputCommitment::new(dimension, 1),
    );
    let operation =
        PanicOperation::try_start(OperationControl::default(), limits, &account, start, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("panic-order operation must start"));
    let ledger = operation.coordinator.retained_ledger().test_probe();
    let deferred_ledger = StdArc::new(StdMutex::new(Some(ledger)));
    let table_writer = RetainedWriterPlan::new(
        TestPhase::Build,
        dimension,
        dimension,
        RetainedBatchLimits::new(2, 2),
    );
    let publication_plan = RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(table_writer, 1, 1),
        RetainedBlockTablePlan::new(table_writer, 1, 1),
    );
    let mut assembly: PanicPublication = operation
        .try_publication_assembly(publication_plan)
        .unwrap_or_else(|_| panic!("panic-order publication must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(
                PrimaryBindDropObservation {
                    ledger: StdArc::clone(&deferred_ledger),
                    drop_count: &BLOCK_DROPS,
                    observed_active: &OBSERVED_ACTIVE,
                    observed_live: &OBSERVED_LIVE,
                },
                CanonicalRootTag,
            ))
        })
        .unwrap_or_else(|_| panic!("panic-order canonical block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9_u64, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("panic-order evidence block must retain"));
    let (expected_active, expected_live) = {
        let ledger = lock_recovering_poison(&deferred_ledger);
        let ledger = ledger.as_ref().expect("panic-order ledger probe");
        (ledger.active_reservations(), ledger.current_live_bytes())
    };

    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence) == Some(&9)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("panic-order candidate must validate"),
        };
    PANIC_ON_PRIMARY_DIMENSION_ORD.store(true, Ordering::SeqCst);
    let result = validated.try_prepare(final_progress());
    PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);

    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::PrimaryPublicationBindingPanicked
    ));
    assert_eq!(BLOCK_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(OBSERVED_ACTIVE.load(Ordering::SeqCst), expected_active);
    assert_eq!(OBSERVED_LIVE.load(Ordering::SeqCst), expected_live);
    {
        let ledger = lock_recovering_poison(&deferred_ledger);
        let ledger = ledger.as_ref().expect("retained panic-order ledger probe");
        assert_eq!(ledger.active_reservations(), expected_active);
        assert_eq!(ledger.current_live_bytes(), expected_live);
    }
    assert!(account.snapshot().live_retained_bytes() > 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn cleanup_lattice_is_order_independent_and_preserves_error_precedence() {
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 16);
    let storage =
        RetainedStorageDropOutcome::StorageReleaseUnverified(record_account_poison_event(
            &account,
            AccountPoisonReason::RetainedStorageReleaseUnverified,
        ));
    let ownership = RetainedStorageDropOutcome::ForeignOwnershipFault(record_account_poison_event(
        &account,
        AccountPoisonReason::OwnershipGraphUnverified,
    ));
    let charge = RetainedStorageDropOutcome::ChargeReleaseUnverified(record_account_poison_event(
        &account,
        AccountPoisonReason::CounterInvariantViolated,
    ));

    for (left, right, expected) in [
        (storage, ownership, RetainedStorageDropSeverity::Storage),
        (storage, charge, RetainedStorageDropSeverity::Storage),
        (ownership, charge, RetainedStorageDropSeverity::Ownership),
        (
            charge,
            RetainedStorageDropOutcome::Clean,
            RetainedStorageDropSeverity::Charge,
        ),
    ] {
        assert_eq!(left.join(right).severity(), expected);
        assert_eq!(right.join(left).severity(), expected);
    }
    assert_eq!(
        RetainedStorageDropOutcome::MissingFaultAnchor
            .join(ownership)
            .witness()
            .map(|witness| witness.reason),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );
    assert_eq!(
        ownership
            .join(RetainedStorageDropOutcome::MissingFaultAnchor)
            .witness()
            .map(|witness| witness.reason),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );

    let cause: RetainedWriterError<TestDimension> = join_pre_attribution_cleanup_cause(
        RetainedStorageDropOutcome::Clean,
        RetainedWriterError::InvalidPublicationPlan,
    );
    assert!(matches!(cause, RetainedWriterError::InvalidPublicationPlan));
    let cause = join_pre_attribution_cleanup_cause(
        charge,
        RetainedWriterError::<TestDimension>::OwnershipUnverified,
    );
    assert!(matches!(cause, RetainedWriterError::OwnershipUnverified));
    let cause = join_pre_attribution_cleanup_cause(
        ownership,
        RetainedWriterError::<TestDimension>::StorageDropUnverified,
    );
    assert!(matches!(cause, RetainedWriterError::StorageDropUnverified));
    let cause = join_pre_attribution_cleanup_cause(
        storage,
        RetainedWriterError::<TestDimension>::OwnershipUnverified,
    );
    assert!(matches!(cause, RetainedWriterError::StorageDropUnverified));

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn primary_binding_panic_reports_later_storage_release_failure() {
    let _ord_lock = PRIMARY_DIMENSION_ORD_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _reset = PrimaryDimensionOrdReset;
    PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);

    type PanicOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, PrimaryPanicDimension, u32>;
    type PanicPublication = RetainedPublicationAssembly<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        PrimaryPanicDimension,
        u32,
        u64,
        u64,
    >;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let dimension = PrimaryPanicDimension(7);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(dimension, policy)])
        .unwrap_or_else(|_| panic!("combined-fault policy must be valid"));
    let writer = RetainedWriterPlan::new(
        TestPhase::Build,
        dimension,
        dimension,
        RetainedBatchLimits::new(2, 2),
    );
    let start = RetainedOperationStartPlan::new(
        writer,
        RetainedHandleSharePlan::new(dimension, dimension),
        empty_plan(),
        OutputCommitment::new(dimension, 1),
        OutputCommitment::new(dimension, 1),
    );
    let operation =
        PanicOperation::try_start(OperationControl::default(), limits, &account, start, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("combined-fault operation must start"));
    let publication_plan = RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(writer, 1, 1),
        RetainedBlockTablePlan::new(writer, 1, 1),
    );
    let mut assembly: PanicPublication = operation
        .try_publication_assembly(publication_plan)
        .unwrap_or_else(|_| panic!("combined-fault publication must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11_u64, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("combined-fault canonical block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(9_u64, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("combined-fault evidence block must retain"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&9)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("combined-fault candidate must validate"),
        };

    test_panic_final_owner_drop_after(0);
    PANIC_ON_PRIMARY_DIMENSION_ORD.store(true, Ordering::SeqCst);
    let result = validated.try_prepare(final_progress());
    PANIC_ON_PRIMARY_DIMENSION_ORD.store(false, Ordering::SeqCst);

    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
    ));
    let snapshot = account.snapshot();
    assert!(snapshot.poison_event_count() >= 2);
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::PrimaryPublicationBindingPanicked)
    );
    assert!(snapshot.live_retained_bytes() > 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn second_table_drop_panic_keeps_success_rejection_accounting_fail_closed() {
    static CANONICAL_DROPS: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);

    CANONICAL_DROPS.store(0, Ordering::SeqCst);
    EVIDENCE_DROPS.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let operation = test_operation(&factory, 4);
    let mut assembly: TestPublication<DropCounter, PanickingDropProbe> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("success rejection assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical drop counter must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("panicking evidence must retain"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
    let probe = assembly
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let before_active = probe.active_reservations();
    let before_live = probe.current_live_bytes();

    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence).is_some()
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("panicking-drop candidate must validate"),
        };
    let result = validated.try_prepare(final_progress());
    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
    ));
    assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(before_active - probe.active_reservations(), 3);
    assert!(probe.active_reservations() > 0);
    assert!(probe.current_live_bytes() > 0);
    assert!(probe.current_live_bytes() < before_live);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn first_table_drop_panic_still_releases_the_clean_evidence_table() {
    static CANONICAL_DROPS: AtomicU64 = AtomicU64::new(0);
    static EVIDENCE_DROPS: AtomicU64 = AtomicU64::new(0);

    CANONICAL_DROPS.store(0, Ordering::SeqCst);
    EVIDENCE_DROPS.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let operation = test_operation(&factory, 4);
    let mut assembly: TestPublication<PanickingDropProbe, DropCounter> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("success rejection assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&CANONICAL_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("panicking canonical must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(DropCounter(&EVIDENCE_DROPS), EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence drop counter must retain"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
    let probe = assembly
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let before_active = probe.active_reservations();
    let before_live = probe.current_live_bytes();

    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence).is_some()
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("panicking-drop candidate must validate"),
        };
    let result = validated.try_prepare(final_progress());
    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
    ));
    assert_eq!(CANONICAL_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(EVIDENCE_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(before_active - probe.active_reservations(), 3);
    assert!(probe.active_reservations() > 0);
    assert!(probe.current_live_bytes() > 0);
    assert!(probe.current_live_bytes() < before_live);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn table_payload_drop_panic_fails_closed_before_attribution_release() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<PanickingDropProbe, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    DROP_COUNT.store(0, Ordering::SeqCst);
    let _canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&DROP_COUNT), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("panicking block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence must retain"));
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root) == Some(&7)
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("table-drop failure candidate must validate"),
    };
    assert!(matches!(
        validated.try_prepare(PreparationFailureCode::domain(2), final_progress()),
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
    ));
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn baseline_drop_payload_before_releasing_live_ownership() {
    static EXPECTED_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static EXPECTED_OPERATION_LIVE: AtomicU64 = AtomicU64::new(0);
    static EXPECTED_PHYSICAL_BYTES: AtomicU64 = AtomicU64::new(0);
    static EXPECTED_PHYSICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_ACTIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_OPERATION_LIVE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_PHYSICAL_BYTES: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_PHYSICAL_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let deferred_ledger = StdArc::new(StdMutex::new(None));
    let fallback = DeferredLiveDropObservation {
        ledger: StdArc::clone(&deferred_ledger),
        account: account
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("test account share must be admitted")),
        observed_active: &OBSERVED_ACTIVE,
        observed_operation_live: &OBSERVED_OPERATION_LIVE,
        observed_physical_bytes: &OBSERVED_PHYSICAL_BYTES,
        observed_physical_allocations: &OBSERVED_PHYSICAL_ALLOCATIONS,
    };
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let operation: RetainedOperation<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TestDimension,
        DeferredLiveDropObservation,
    > = RetainedOperation::try_start(OperationControl::default(), limits, &account, plan, || {
        Ok(fallback)
    })
    .unwrap_or_else(|_| panic!("baseline observation operation must start"));
    let ledger = operation.coordinator.retained_ledger().test_probe();
    EXPECTED_ACTIVE.store(ledger.active_reservations(), Ordering::SeqCst);
    EXPECTED_OPERATION_LIVE.store(ledger.current_live_bytes(), Ordering::SeqCst);
    let physical = operation.account().snapshot();
    EXPECTED_PHYSICAL_BYTES.store(physical.live_retained_bytes(), Ordering::SeqCst);
    EXPECTED_PHYSICAL_ALLOCATIONS.store(physical.live_retained_allocations(), Ordering::SeqCst);
    *lock_recovering_poison(&deferred_ledger) = Some(ledger);

    drop(operation);
    assert_eq!(
        OBSERVED_ACTIVE.load(Ordering::SeqCst),
        EXPECTED_ACTIVE.load(Ordering::SeqCst)
    );
    assert_eq!(
        OBSERVED_OPERATION_LIVE.load(Ordering::SeqCst),
        EXPECTED_OPERATION_LIVE.load(Ordering::SeqCst)
    );
    assert_eq!(
        OBSERVED_PHYSICAL_BYTES.load(Ordering::SeqCst),
        EXPECTED_PHYSICAL_BYTES.load(Ordering::SeqCst)
    );
    assert_eq!(
        OBSERVED_PHYSICAL_ALLOCATIONS.load(Ordering::SeqCst),
        EXPECTED_PHYSICAL_ALLOCATIONS.load(Ordering::SeqCst)
    );
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn prepared_published_and_baseline_owners_contain_payload_drop_panics() {
    static PREPARED_DROPS: AtomicU64 = AtomicU64::new(0);
    static PUBLISHED_DROPS: AtomicU64 = AtomicU64::new(0);
    static BASELINE_DROPS: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    PREPARED_DROPS.store(0, Ordering::SeqCst);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<PanickingDropProbe, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("prepared drop assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&PREPARED_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("prepared drop block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("prepared drop evidence must retain"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("prepared-drop graph must validate"),
        };
    let prepared = validated
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("prepared drop publication must prepare"));
    assert!(catch_unwind(AssertUnwindSafe(|| drop(prepared))).is_ok());
    assert_eq!(PREPARED_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(factory.snapshot().live_accounts(), 1);

    let factory = account_factory(1);
    PUBLISHED_DROPS.store(0, Ordering::SeqCst);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<PanickingDropProbe, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("published drop assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&PUBLISHED_DROPS), CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("published drop block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("published drop evidence must retain"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("published-drop graph must validate"),
        };
    let published = handoff_for_test(
        validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("published drop publication must prepare")),
    );
    assert!(catch_unwind(AssertUnwindSafe(|| drop(published))).is_ok());
    assert_eq!(PUBLISHED_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(factory.snapshot().live_accounts(), 1);

    let factory = account_factory(1);
    BASELINE_DROPS.store(0, Ordering::SeqCst);
    let operation = test_operation_with_fallback(&factory, PanickingDropProbe(&BASELINE_DROPS));
    assert!(catch_unwind(AssertUnwindSafe(|| drop(operation))).is_ok());
    assert_eq!(BASELINE_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn prepared_and_published_drop_keep_physical_charge_and_handoff_is_a_pure_move() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_BYTES: AtomicU64 = AtomicU64::new(0);
    static OBSERVED_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    OBSERVED_BYTES.store(0, Ordering::SeqCst);
    OBSERVED_ALLOCATIONS.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let observed_account = operation
        .account()
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("test account share must be admitted"));
    let mut assembly: TestPublication<PhysicalDropObservation, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("prepared observation assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(
                PhysicalDropObservation {
                    account: observed_account,
                    drop_count: &DROP_COUNT,
                    observed_bytes: &OBSERVED_BYTES,
                    observed_allocations: &OBSERVED_ALLOCATIONS,
                },
                CanonicalRootTag,
            ))
        })
        .unwrap_or_else(|_| panic!("prepared observation block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("prepared observation evidence must retain"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("prepared observation graph must validate"),
        };
    let prepared = validated
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("prepared observation publication must prepare"));
    drop(prepared);
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert!(OBSERVED_BYTES.load(Ordering::SeqCst) > 0);
    assert!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst) > 0);
    assert_eq!(factory.snapshot().live_accounts(), 0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    OBSERVED_BYTES.store(0, Ordering::SeqCst);
    OBSERVED_ALLOCATIONS.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let observed_account = operation
        .account()
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("test account share must be admitted"));
    let mut assembly: TestPublication<PhysicalDropObservation, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("handoff observation assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(
                PhysicalDropObservation {
                    account: observed_account,
                    drop_count: &DROP_COUNT,
                    observed_bytes: &OBSERVED_BYTES,
                    observed_allocations: &OBSERVED_ALLOCATIONS,
                },
                CanonicalRootTag,
            ))
        })
        .unwrap_or_else(|_| panic!("handoff observation block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("handoff observation evidence must retain"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical).is_some() && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("handoff observation graph must validate"),
        };
    let prepared = validated
        .try_prepare(final_progress())
        .unwrap_or_else(|_| panic!("handoff observation publication must prepare"));
    let prepared = match prepared.into_outcome() {
        PreparedRetainedPublicationOutcome::Success(prepared) => prepared,
        _ => panic!("handoff observation must prepare success"),
    };
    let (canonical_identity, canonical_shares, evidence_identity, evidence_shares) = (
        prepared.canonical.inner.table.allocation_identity(),
        prepared.canonical.inner.table.share_call_count(),
        prepared.outcome_evidence.inner.table.allocation_identity(),
        prepared.outcome_evidence.inner.table.share_call_count(),
    );
    let ledger_allocation = prepared.evidence.allocation();
    let ledger_identity = ledger_allocation.allocation_identity();
    let ledger_shares = ledger_allocation.share_call_count();

    let published = prepared.handoff();
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(OBSERVED_BYTES.load(Ordering::SeqCst), 0);
    assert_eq!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst), 0);
    assert_eq!(
        published
            .canonical
            .aggregate
            .inner
            .table
            .allocation_identity(),
        canonical_identity
    );
    assert_eq!(
        published.canonical.aggregate.inner.table.share_call_count(),
        canonical_shares
    );
    assert_eq!(
        published
            .outcome_evidence
            .aggregate
            .inner
            .table
            .allocation_identity(),
        evidence_identity
    );
    assert_eq!(
        published
            .outcome_evidence
            .aggregate
            .inner
            .table
            .share_call_count(),
        evidence_shares
    );
    let ledger_allocation = published.ledger_evidence.evidence.allocation();
    assert_eq!(ledger_allocation.allocation_identity(), ledger_identity);
    assert_eq!(ledger_allocation.share_call_count(), ledger_shares);

    drop(published);
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert!(OBSERVED_BYTES.load(Ordering::SeqCst) > 0);
    assert!(OBSERVED_ALLOCATIONS.load(Ordering::SeqCst) > 0);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn success_preparation_contains_fallback_drop_panic() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    struct PanickingFallback;

    impl Drop for PanickingFallback {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(vec![0_u8; 8]);
        }
    }

    type PanicFallbackOperation = RetainedOperation<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TestDimension,
        PanickingFallback,
    >;
    type PanicFallbackPublication = RetainedPublicationAssembly<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TestDimension,
        PanickingFallback,
        u64,
        u64,
    >;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    DROP_COUNT.store(0, Ordering::SeqCst);
    let operation = PanicFallbackOperation::try_start(
        OperationControl::default(),
        limits,
        &account,
        plan,
        || Ok(PanickingFallback),
    )
    .unwrap_or_else(|_| panic!("panic fallback operation must start"));
    let mut assembly: PanicFallbackPublication = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("panic fallback assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(41, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical block must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence block must retain"));
    let probe = assembly
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let before_active = probe.active_reservations();
    let before_live = probe.current_live_bytes();
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&41)
                && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("fallback-panic candidate must validate"),
        };
    let result = catch_unwind(AssertUnwindSafe(|| validated.try_prepare(final_progress())))
        .expect("fallback destructor panic must be contained");
    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
    ));
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert!(before_active > 0);
    assert!(before_live > 0);
    assert!(probe.active_reservations() > 0);
    assert!(probe.active_reservations() <= before_active);
    assert!(probe.current_live_bytes() > 0);
    assert!(probe.current_live_bytes() <= before_live);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn rejected_primary_payload_drop_panic_fails_closed_before_token_release() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    struct PanickingEvidence;

    impl Drop for PanickingEvidence {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(vec![0_u8; 8]);
        }
    }

    let factory = account_factory(1);
    let operation = test_operation(&factory, 4);
    let plan =
        RetainedPublicationPlan::rich_failure(RetainedBlockTablePlan::new(writer_plan(2, 2), 1, 1));
    let mut assembly: TestPublication<u64, PanickingEvidence> = operation
        .try_publication_assembly(plan)
        .unwrap_or_else(|_| panic!("rejected primary assembly must open"));
    DROP_COUNT.store(0, Ordering::SeqCst);
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingEvidence, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("panicking evidence must retain"));
    assembly
        .try_commit_outcome_evidence(TestDimension::Bytes, 4)
        .unwrap_or_else(|_| panic!("commitment must fit the preallocated batch"));
    let validated = match assembly.validate_rich_failure(evidence, |session, root| {
        session.outcome_evidence(root).is_some()
    }) {
        RetainedFailureValidation::Valid(validated) => validated,
        _ => panic!("rejected-primary candidate must validate"),
    };
    let result = catch_unwind(AssertUnwindSafe(|| {
        validated.try_prepare(PreparationFailureCode::domain(5), final_progress())
    }))
    .expect("primary rejection payload panic must be contained");
    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
    ));
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn invalid_and_allocation_failed_open_preserve_operation_for_retry() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let invalid_plan = publication_plan(0, 1);
    let invalid = match operation.try_publication_assembly::<u64, u64>(invalid_plan) {
        Ok(_) => panic!("zero-slot table must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        invalid.cause(),
        RetainedWriterError::InvalidPublicationPlan
    ));
    let operation = match invalid.try_into_operation() {
        Ok(operation) => operation,
        Err(_) => panic!("invalid plan must preserve operation ownership"),
    };

    fail_next_final_owner_allocation();
    let allocation = match operation.try_publication_assembly::<u64, u64>(publication_plan(1, 1)) {
        Ok(_) => panic!("injected table allocation must fail"),
        Err(error) => error,
    };
    assert!(matches!(
        allocation.cause(),
        RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
    ));
    let operation = match allocation.try_into_operation() {
        Ok(operation) => operation,
        Err(_) => panic!("verified allocation failure must preserve operation ownership"),
    };
    let assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("retry must open a valid assembly"));
    drop(handoff_for_test(
        assembly
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("retry fallback must prepare")),
    ));
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn recoverable_block_allocation_failure_preserves_assembly_for_retry() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("retry assembly must open"));
    let probe = assembly
        .operation
        .as_ref()
        .expect("assembly retains operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let before_active = probe.active_reservations();
    let before_live = probe.current_live_bytes();
    let before_physical = assembly
        .account
        .as_ref()
        .expect("assembly retains account")
        .snapshot();

    fail_next_final_owner_allocation();
    assert!(matches!(
        assembly.try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        }),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::AllocationFailed)
        ))
    ));
    let after_physical = assembly
        .account
        .as_ref()
        .expect("assembly retains account after recoverable failure")
        .snapshot();
    assert_eq!(probe.active_reservations(), before_active);
    assert_eq!(probe.current_live_bytes(), before_live);
    assert_eq!(
        after_physical.live_retained_bytes(),
        before_physical.live_retained_bytes()
    );
    assert_eq!(
        after_physical.live_retained_allocations(),
        before_physical.live_retained_allocations()
    );

    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical retry must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence root must retain"));
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("retried assembly graph must validate"),
        };
    let published = handoff_for_test(
        validated
            .try_prepare(final_progress())
            .unwrap_or_else(|_| panic!("retried assembly must prepare")),
    );
    drop(published);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn closure_validation_panic_publishes_minimal_fallback() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let canonical = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(11, CanonicalRootTag))
        })
        .unwrap_or_else(|_| panic!("canonical root must retain"));
    let evidence = assembly
        .try_retain_outcome_evidence(empty_plan(), |emplacer| {
            Ok(emplacer.place(7, EvidenceRootTag))
        })
        .unwrap_or_else(|_| panic!("evidence root must retain"));

    let fallback_only =
        match assembly.validate_success(canonical, evidence, |_session, _canonical, _evidence| {
            std::panic::panic_any(vec![0_u8; 8])
        }) {
            RetainedSuccessValidation::FallbackOnly(fallback) => fallback,
            _ => panic!("validation panic must expose fallback-only state"),
        };
    let prepared = fallback_only
        .prepare_minimal_fallback()
        .unwrap_or_else(|_| panic!("validation panic must prepare minimal fallback"));
    match handoff_for_test(prepared) {
        PublishedRetainedPublicationParts::MinimalFallback { fallback, .. } => {
            assert_eq!(
                fallback.cause().class(),
                PreparationFailureClass::ProductGraph
            );
            assert_eq!(fallback.cause().code(), 5);
        }
        _ => panic!("validation panic must publish minimal fallback"),
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn explicit_limits_reject_zero_and_plans_reject_mismatched_nested_evidence() {
    assert_eq!(
        PhysicalStorageAccountFactoryLimits::new(0, 1),
        Err(PhysicalStorageLimitError::ZeroByteLimit)
    );
    assert_eq!(
        PhysicalStorageAccountLimits::new(1, 0),
        Err(PhysicalStorageLimitError::ZeroAllocationLimit)
    );
    assert_eq!(
        RetainedAllocationPlan::new(1, 0),
        Err(RetainedAllocationPlanError::MismatchedNestedFootprint {
            bytes: 1,
            allocations: 0,
        })
    );
}

#[test]
fn inline_ceiling_covers_the_complete_extracted_ownership_header() {
    let factory = account_factory(1);
    let account = account(&factory, 16_384, 8);
    let result = account.try_retain_with(empty_plan(), || {
        Ok::<_, RetainedBuildFailureCode>([0_u8; MAX_RETAINED_INLINE_BYTES])
    });

    assert!(matches!(
        result,
        Err(RetainedAllocationError::InlinePayloadTooLarge {
            bytes,
            limit: MAX_RETAINED_INLINE_BYTES,
        }) if bytes > MAX_RETAINED_INLINE_BYTES
    ));
}

#[test]
fn factory_enforces_exact_live_account_and_control_byte_limits() {
    let control_bytes =
        FinalOwnerAllocation::<AccountControlInner>::checked_layout_bytes().unwrap();
    let limits = PhysicalStorageAccountFactoryLimits::new(control_bytes, 1).unwrap();
    let factory = PhysicalStorageAccountFactory::try_new(limits).unwrap();
    let account_limits = PhysicalStorageAccountLimits::new(1_024, 8).unwrap();

    let first = factory.try_create_account(account_limits).unwrap();
    assert_eq!(factory.snapshot().live_accounts(), 1);
    assert_eq!(
        factory.snapshot().live_account_control_bytes(),
        control_bytes
    );
    assert!(matches!(
        factory.try_create_account(account_limits),
        Err(PhysicalStorageAccountCreateError::ControlByteLimitExceeded { .. })
            | Err(PhysicalStorageAccountCreateError::LiveAccountLimitExceeded { .. })
    ));

    drop(first);
    assert_eq!(factory.snapshot().live_accounts(), 0);
    assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
}

#[test]
fn account_control_allocation_failure_rolls_back_factory_admission() {
    let factory = account_factory(1);
    let account_limits = PhysicalStorageAccountLimits::new(1_024, 8).unwrap();

    fail_next_final_owner_allocation();
    assert_eq!(
        factory.try_create_account(account_limits).err(),
        Some(PhysicalStorageAccountCreateError::AllocationFailed)
    );
    assert_eq!(factory.snapshot().live_accounts(), 0);
    assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
}

#[test]
fn last_account_control_owner_releases_factory_admission_during_unwind() {
    let factory = account_factory(1);
    let account_limits = PhysicalStorageAccountLimits::new(1_024, 8).unwrap();

    let result = catch_unwind(AssertUnwindSafe(|| {
        let _account = factory.try_create_account(account_limits).unwrap();
        panic!("drop the final account control owner during unwind");
    }));

    assert!(result.is_err());
    assert_eq!(factory.snapshot().live_accounts(), 0);
    assert_eq!(factory.snapshot().live_account_control_bytes(), 0);
}

#[test]
fn concurrent_factory_admission_cannot_oversubscribe_one_account_slot() {
    let control_bytes =
        FinalOwnerAllocation::<AccountControlInner>::checked_layout_bytes().unwrap();
    let limits = PhysicalStorageAccountFactoryLimits::new(control_bytes, 1).unwrap();
    let factory = PhysicalStorageAccountFactory::try_new(limits).unwrap();
    let account_limits = PhysicalStorageAccountLimits::new(1_024, 8).unwrap();
    let start = StdArc::new(Barrier::new(3));
    let attempts_complete = StdArc::new(Barrier::new(3));
    let release_owner = StdArc::new(Barrier::new(2));
    let successes = StdArc::new(AtomicU64::new(0));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let factory = factory.clone();
        let start = StdArc::clone(&start);
        let attempts_complete = StdArc::clone(&attempts_complete);
        let release_owner = StdArc::clone(&release_owner);
        let successes = StdArc::clone(&successes);
        threads.push(thread::spawn(move || {
            start.wait();
            let account = factory.try_create_account(account_limits).ok();
            if account.is_some() {
                successes.fetch_add(1, Ordering::SeqCst);
            }
            attempts_complete.wait();
            if let Some(account) = account {
                release_owner.wait();
                drop(account);
            }
        }));
    }

    start.wait();
    attempts_complete.wait();
    assert_eq!(successes.load(Ordering::SeqCst), 1);
    release_owner.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn retained_admission_accepts_exact_limits_and_rejects_excess() {
    let outer = FinalOwnerAllocation::<
        RetainedFinalOwnerInner<RetainedPayload<u64>, LeafRetainedPolicy>,
    >::checked_layout_bytes()
    .unwrap();
    let factory = account_factory(1);
    let account = account(&factory, outer + 11, 2);
    let plan = RetainedAllocationPlan::new(11, 1).unwrap();

    let retained = account
        .try_retain_with(plan, || Ok::<_, RetainedBuildFailureCode>(7_u64))
        .unwrap();
    assert_eq!(retained.charged_bytes(), outer + 11);
    assert_eq!(retained.charged_allocations(), 2);
    assert_eq!(account.snapshot().live_retained_bytes(), outer + 11);
    assert!(matches!(
        account.try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(9_u64)
        }),
        Err(RetainedAllocationError::ByteLimitExceeded { .. })
            | Err(RetainedAllocationError::AllocationLimitExceeded { .. })
    ));

    drop(retained);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn builder_error_code_is_returned_after_charge_release() {
    let factory = account_factory(1);
    let account = account(&factory, 1_024, 8);

    let error = account
        .try_retain_with::<u64>(empty_plan(), || Err(BUILD_FAILED))
        .unwrap_err();
    assert_eq!(error, RetainedAllocationError::Build(BUILD_FAILED));
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn builder_local_storage_drops_before_its_physical_lease() {
    static OBSERVED_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

    struct BuilderLocalStorage {
        account: PhysicalStorageAccount,
        _bytes: Vec<u8>,
    }

    impl Drop for BuilderLocalStorage {
        fn drop(&mut self) {
            OBSERVED_LIVE_BYTES.store(
                self.account.snapshot().live_retained_bytes(),
                Ordering::SeqCst,
            );
        }
    }

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    OBSERVED_LIVE_BYTES.store(0, Ordering::SeqCst);
    let plan = RetainedAllocationPlan::new(64, 1).unwrap();
    let error = account
        .try_retain_with::<u64>(plan, || {
            let _storage = BuilderLocalStorage {
                account: account
                    .share_after_test_admission()
                    .unwrap_or_else(|_| panic!("test account share must be admitted")),
                _bytes: vec![0; 64],
            };
            Err(BUILD_FAILED)
        })
        .unwrap_err();

    assert_eq!(error, RetainedAllocationError::Build(BUILD_FAILED));
    assert!(OBSERVED_LIVE_BYTES.load(Ordering::SeqCst) > 64);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn every_builder_panic_payload_keeps_its_physical_charge_fail_closed() {
    let factory = account_factory(3);
    let borrowed_account = account(&factory, 4_096, 16);
    let before = borrowed_account.snapshot();

    let borrowed = borrowed_account
        .try_retain_with::<u64>(empty_plan(), || panic!("test panic"))
        .unwrap_err();
    assert_eq!(borrowed, RetainedAllocationError::BuilderPanicked);
    let after_borrowed = borrowed_account.snapshot();
    assert!(after_borrowed.live_retained_bytes() > before.live_retained_bytes());
    assert!(after_borrowed.live_retained_allocations() > before.live_retained_allocations());
    assert!(after_borrowed.poisoned());
    assert_eq!(after_borrowed.poison_event_count(), 1);
    assert!(after_borrowed.poison_event_count_exact());
    assert!(after_borrowed.poison_event_domain_valid());
    assert_eq!(
        after_borrowed
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedBuilderPanicked)
    );

    let later_builder_ran = AtomicBool::new(false);
    let later = borrowed_account
        .try_retain_with::<u64>(empty_plan(), || {
            later_builder_ran.store(true, Ordering::SeqCst);
            Ok(7)
        })
        .unwrap_err();
    assert!(matches!(later, RetainedAllocationError::AccountPoisoned(_)));
    assert!(!later_builder_ran.load(Ordering::SeqCst));
    assert_eq!(borrowed_account.snapshot(), after_borrowed);

    let owned_account = account(&factory, 4_096, 16);
    let owned_before = owned_account.snapshot();
    let owned = owned_account
        .try_retain_with::<u64>(empty_plan(), || {
            std::panic::panic_any(String::from("owned test panic"))
        })
        .unwrap_err();
    assert_eq!(owned, RetainedAllocationError::BuilderPanicked);
    let after_owned = owned_account.snapshot();
    assert!(after_owned.live_retained_bytes() > owned_before.live_retained_bytes());
    assert!(after_owned.live_retained_allocations() > owned_before.live_retained_allocations());

    let opaque_account = account(&factory, 4_096, 16);
    let opaque_before = opaque_account.snapshot();
    let plan = RetainedAllocationPlan::new(64, 1).unwrap();
    let opaque = opaque_account
        .try_retain_with::<u64>(plan, || std::panic::panic_any(vec![0_u8; 64]))
        .unwrap_err();
    assert_eq!(opaque, RetainedAllocationError::BuilderPanicked);
    let after_opaque = opaque_account.snapshot();
    assert!(after_opaque.live_retained_bytes() > opaque_before.live_retained_bytes() + 64);
    assert!(after_opaque.live_retained_allocations() > opaque_before.live_retained_allocations());
    drop(borrowed_account);
    drop(owned_account);
    drop(opaque_account);
    assert_eq!(factory.snapshot().live_accounts(), 3);
}

#[test]
fn shared_handles_observe_one_latched_first_event_and_exact_event_count() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let observer = account
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("observer share must be admitted before poison"));
    let first = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("first poison probe must retain"));
    let second = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("second poison probe must retain"));

    drop(first);
    let after_first = account.snapshot();
    assert_eq!(after_first, observer.snapshot());
    assert_eq!(after_first.poison_event_count(), 1);
    assert_eq!(
        after_first
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
    );

    drop(second);
    let after_second = account.snapshot();
    assert_eq!(after_second, observer.snapshot());
    assert_eq!(after_second.poison_event_count(), 2);
    assert!(after_second.poison_event_count_exact());
    assert!(after_second.poison_event_domain_valid());
    assert_eq!(
        after_second.first_poison_evidence(),
        after_first.first_poison_evidence()
    );
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 2);

    let before_rejected_share = account.snapshot();
    let rejected_share = account.share_after_test_admission().unwrap_err();
    assert!(matches!(
        rejected_share,
        RetainedAllocationError::AccountPoisoned(_)
    ));
    assert_eq!(account.snapshot(), before_rejected_share);
}

#[test]
fn poison_first_start_rejects_without_builder_or_account_side_effects() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 16);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("poison probe must retain"));
    let started = StdArc::new(Barrier::new(2));
    let poisoned = StdArc::new(Barrier::new(2));
    let thread_started = StdArc::clone(&started);
    let thread_poisoned = StdArc::clone(&poisoned);
    let poison_thread = thread::spawn(move || {
        thread_started.wait();
        drop(poisoner);
        thread_poisoned.wait();
    });
    started.wait();
    poisoned.wait();
    poison_thread
        .join()
        .unwrap_or_else(|_| panic!("poison thread must be contained"));

    let before = account.snapshot();
    let builder_ran = AtomicBool::new(false);
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let result =
        TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            builder_ran.store(true, Ordering::SeqCst);
            Ok(7)
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::AccountPoisoned(_)
        ))
    ));
    assert!(!builder_ran.load(Ordering::SeqCst));
    assert_eq!(account.snapshot(), before);
}

#[test]
fn admission_first_start_panic_after_external_poison_records_distinct_builder_fault() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, 8_192, 32);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("startup panic poison probe must retain"));
    let worker_account = account
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("startup worker account share must precede poison"));
    let shares_before_start = account.share_call_count();
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let builder_entered = StdArc::new(Barrier::new(2));
    let builder_continue = StdArc::new(Barrier::new(2));
    let worker_entered = StdArc::clone(&builder_entered);
    let worker_continue = StdArc::clone(&builder_continue);
    let worker = thread::spawn(move || {
        TestOperation::try_start(
            OperationControl::default(),
            limits,
            &worker_account,
            plan,
            || {
                worker_entered.wait();
                worker_continue.wait();
                std::panic::panic_any(vec![0_u8; 8]);
            },
        )
    });

    builder_entered.wait();
    drop(poisoner);
    let after_external_poison = account.snapshot();
    assert!(after_external_poison.poisoned());
    let first_evidence = after_external_poison.first_poison_evidence();
    builder_continue.wait();
    let result = worker
        .join()
        .unwrap_or_else(|_| panic!("startup builder panic must be contained"));
    let after_builder_panic = account.snapshot();

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
        ))
    ));
    assert_eq!(
        after_builder_panic.poison_event_count(),
        after_external_poison.poison_event_count() + 1
    );
    assert_eq!(after_builder_panic.first_poison_evidence(), first_evidence);
    assert_eq!(
        after_builder_panic.live_retained_bytes(),
        after_external_poison.live_retained_bytes()
    );
    assert_eq!(
        after_builder_panic.live_retained_allocations(),
        after_external_poison.live_retained_allocations()
    );
    assert_eq!(account.share_call_count(), shares_before_start + 4);
}

#[test]
fn admission_first_physical_retain_completes_then_later_retain_is_rejected() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 16);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("physical-race poison probe must retain"));
    let worker_account = account
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("worker account share must precede poison"));
    let builder_entered = StdArc::new(Barrier::new(2));
    let builder_continue = StdArc::new(Barrier::new(2));
    let worker_entered = StdArc::clone(&builder_entered);
    let worker_continue = StdArc::clone(&builder_continue);
    let worker = thread::spawn(move || {
        worker_account
            .try_retain_with(empty_plan(), || {
                worker_entered.wait();
                worker_continue.wait();
                Ok::<_, RetainedBuildFailureCode>(99_u64)
            })
            .unwrap_or_else(|_| panic!("admission-first retain must complete"))
    });

    builder_entered.wait();
    drop(poisoner);
    let after_poison = account.snapshot();
    assert!(after_poison.poisoned());
    builder_continue.wait();
    let retained = worker
        .join()
        .unwrap_or_else(|_| panic!("retaining worker must finish"));
    assert_eq!(retained.as_ref(), &99);
    assert_eq!(account.snapshot(), after_poison);

    let later_builder_ran = AtomicBool::new(false);
    let later = account
        .try_retain_with(empty_plan(), || {
            later_builder_ran.store(true, Ordering::SeqCst);
            Ok::<_, RetainedBuildFailureCode>(100_u64)
        })
        .unwrap_err();
    assert!(matches!(later, RetainedAllocationError::AccountPoisoned(_)));
    assert!(!later_builder_ran.load(Ordering::SeqCst));
    assert_eq!(account.snapshot(), after_poison);
    drop(retained);
}

#[test]
fn admission_first_builder_error_after_poison_retains_charge_without_rollback_clone() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 16);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("builder-error poison probe must retain"));
    let worker_account = account
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("worker account share must precede poison"));
    let shares_before_retain = account.share_call_count();
    let builder_entered = StdArc::new(Barrier::new(2));
    let builder_continue = StdArc::new(Barrier::new(2));
    let worker_entered = StdArc::clone(&builder_entered);
    let worker_continue = StdArc::clone(&builder_continue);
    let worker = thread::spawn(move || {
        worker_account
            .try_retain_with::<u64>(empty_plan(), || {
                worker_entered.wait();
                worker_continue.wait();
                Err(BUILD_FAILED)
            })
            .expect_err("admission-first builder error must not publish storage")
    });

    builder_entered.wait();
    drop(poisoner);
    let after_poison = account.snapshot();
    assert!(after_poison.poisoned());
    builder_continue.wait();
    let error = worker
        .join()
        .unwrap_or_else(|_| panic!("builder-error worker must finish"));

    assert!(matches!(
        error,
        RetainedAllocationError::AccountPoisonedWithRetainedCharge(_)
    ));
    assert_eq!(account.snapshot(), after_poison);
    assert_eq!(account.share_call_count(), shares_before_retain + 1);
    assert!(matches!(
        account.try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(1_u64)),
        Err(RetainedAllocationError::AccountPoisoned(_))
    ));
}

#[test]
fn admission_first_direct_fallback_ends_in_finalization_containment() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("post-start poison probe must retain"));
    let started = StdArc::new(Barrier::new(2));
    let poisoned = StdArc::new(Barrier::new(2));
    let thread_started = StdArc::clone(&started);
    let thread_poisoned = StdArc::clone(&poisoned);
    let poison_thread = thread::spawn(move || {
        thread_started.wait();
        drop(poisoner);
        thread_poisoned.wait();
    });
    started.wait();
    poisoned.wait();
    poison_thread
        .join()
        .unwrap_or_else(|_| panic!("poison thread must be contained"));
    let before_prepare = account.snapshot();
    let first_evidence = before_prepare.first_poison_evidence();
    let ledger = operation.coordinator.retained_ledger().test_probe();
    let active_before = ledger.active_reservations();
    let live_before = ledger.current_live_bytes();

    let result = operation.prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(91));
    let after_prepare = account.snapshot();
    assert!(matches!(
        result,
        Err(RetainedPublicationPrepareError::FinalizationContainment(cause))
            if cause.reason() == AccountPoisonReason::RetainedStorageReleaseUnverified
                && cause.account_poisoned().snapshot() == after_prepare
    ));
    assert_eq!(
        after_prepare.poison_event_count(),
        before_prepare.poison_event_count() + 2
    );
    assert!(after_prepare.poison_event_count_exact());
    assert!(after_prepare.poison_event_domain_valid());
    assert_eq!(after_prepare.first_poison_evidence(), first_evidence);
    assert_eq!(ledger.active_reservations(), active_before - 2);
    assert!(ledger.current_live_bytes() > 0);
    assert!(ledger.current_live_bytes() < live_before);
}

#[test]
fn poisoned_inherited_share_has_no_lookup_work_clone_or_target_side_effect() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (source, source_slot, source_evidence, _source_evidence_slot) =
        publish_scalar_source(&account);
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("target publication must open"));
    let source_allocation = source
        .aggregate
        .inner
        .table
        .as_ref()
        .member_allocation(source_slot.identity, source.aggregate.inner.set_id)
        .unwrap_or_else(|| panic!("source slot must resolve before poison"));
    let source_share_calls = source_allocation.share_call_count();
    let target_len = assembly.canonical_len();
    let ledger = assembly.test_ledger_probe();
    let active = ledger.active_reservations();
    let live = ledger.current_live_bytes();
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("share poison probe must retain"));
    drop(poisoner);
    let poisoned = account.snapshot();
    let _ = take_test_member_lookups();

    let result = assembly.try_share_canonical(source.aggregate(), source_slot);
    assert!(matches!(
        result,
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(_)
        ))
    ));
    assert_eq!(assembly.canonical_len(), target_len);
    assert_eq!(ledger.active_reservations(), active);
    assert_eq!(ledger.current_live_bytes(), live);
    assert_eq!(source_allocation.share_call_count(), source_share_calls);
    assert_eq!(take_test_member_lookups(), 0);
    assert_eq!(account.snapshot(), poisoned);
    drop(source_evidence);
}

#[test]
fn admission_first_inherited_share_completes_then_later_share_is_rejected() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (source, source_slot, source_evidence, _source_evidence_slot) =
        publish_scalar_source(&account);
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(3, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(2, 1))
        .unwrap_or_else(|_| panic!("admission-first share target must open"));
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("share-race poison probe must retain"));
    let target_account = assembly
        .account
        .as_ref()
        .unwrap_or_else(|| panic!("target retains account"));
    let share_admission = target_account
        .try_admit_account_action()
        .unwrap_or_else(|_| panic!("share action must be admitted before poison"));
    drop(poisoner);
    let poisoned = account.snapshot();
    let source_allocation = source
        .aggregate
        .inner
        .table
        .as_ref()
        .member_allocation(source_slot.identity, source.aggregate.inner.set_id)
        .unwrap_or_else(|| panic!("source slot must resolve"));
    let source_share_calls = source_allocation.share_call_count();
    let ledger = assembly
        .operation
        .as_ref()
        .unwrap_or_else(|| panic!("target retains operation"))
        .coordinator
        .retained_ledger();
    let identity = try_share_retained_block_after_admission::<
        TestOwner,
        TestPhase,
        TestDimension,
        u64,
        CanonicalRootTag,
    >(
        ledger,
        share_admission,
        assembly.handle_share_work,
        assembly
            .canonical_table
            .as_mut()
            .and_then(PolicyRetainedAllocation::table_mut_during_construction)
            .unwrap_or_else(|| panic!("target retains canonical table")),
        assembly.set_id,
        RetainedMembershipDestination::Canonical,
        0,
        1,
        source_allocation,
    )
    .unwrap_or_else(|_| panic!("admission-first share must complete"));
    assert_eq!(identity.index, 0);
    assert_eq!(assembly.canonical_len(), 1);
    assert_eq!(source_allocation.share_call_count(), source_share_calls + 1);
    assert_eq!(account.snapshot(), poisoned);

    let before_later = assembly.canonical_len();
    let later = assembly.try_share_canonical(source.aggregate(), source_slot);
    assert!(matches!(
        later,
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::AccountPoisoned(_)
        ))
    ));
    assert_eq!(assembly.canonical_len(), before_later);
    drop(source_evidence);
}

#[test]
fn prepared_handoff_is_a_pure_move_after_later_account_poison() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("handoff poison probe must retain"));
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let prepared = operation
        .prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(92))
        .unwrap_or_else(|_| panic!("healthy account must prepare fallback"));
    drop(poisoner);
    let before_handoff = account.snapshot();

    let published = handoff_for_test(prepared);
    assert_eq!(account.snapshot(), before_handoff);
    assert!(matches!(
        published,
        PublishedRetainedPublicationParts::MinimalFallback { .. }
    ));
}

#[test]
fn eligibility_first_capability_completes_preparation_after_later_account_poison() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("eligibility race poison probe must retain"));
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let eligibility_entered = StdArc::new(Barrier::new(2));
    let eligibility_resume = StdArc::new(Barrier::new(2));
    let worker_entered = StdArc::clone(&eligibility_entered);
    let worker_resume = StdArc::clone(&eligibility_resume);
    let worker = thread::spawn(move || {
        install_publication_eligibility_pause(worker_entered, worker_resume);
        operation.prepare_minimal_fallback::<u64, u64>(PreparationFailureCode::domain(93))
    });

    eligibility_entered.wait();
    drop(poisoner);
    let poisoned = account.snapshot();
    assert!(poisoned.poisoned());
    eligibility_resume.wait();

    let prepared = worker
        .join()
        .unwrap_or_else(|_| panic!("eligibility-first worker must remain contained"))
        .unwrap_or_else(|_| panic!("eligibility-first preparation must complete"));
    assert_eq!(account.snapshot(), poisoned);
    let published = handoff_for_test(prepared);
    assert!(matches!(
        published,
        PublishedRetainedPublicationParts::MinimalFallback { .. }
    ));
}

#[test]
fn primary_eligibility_first_capability_completes_after_later_account_poison() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    DROP_COUNT.store(0, Ordering::SeqCst);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let poisoner = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDropProbe(&DROP_COUNT))
        })
        .unwrap_or_else(|_| panic!("primary eligibility race poison probe must retain"));
    let operation = test_operation_on_account_with_control(
        &account,
        64,
        RetainedBatchLimits::new(2, 2),
        OperationControl::default(),
    );
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("primary eligibility publication must open"));
    let (canonical, evidence) = retain_scalar_success(&mut assembly);
    let validated =
        match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
            session.canonical(canonical) == Some(&11)
                && session.outcome_evidence(evidence) == Some(&7)
        }) {
            RetainedSuccessValidation::Valid(validated) => validated,
            _ => panic!("primary eligibility candidate must validate"),
        };
    let eligibility_entered = StdArc::new(Barrier::new(2));
    let eligibility_resume = StdArc::new(Barrier::new(2));
    let worker_entered = StdArc::clone(&eligibility_entered);
    let worker_resume = StdArc::clone(&eligibility_resume);
    let worker = thread::spawn(move || {
        install_publication_eligibility_pause(worker_entered, worker_resume);
        validated.try_prepare(final_progress())
    });

    eligibility_entered.wait();
    drop(poisoner);
    let poisoned = account.snapshot();
    assert!(poisoned.poisoned());
    eligibility_resume.wait();

    let prepared = worker
        .join()
        .unwrap_or_else(|_| panic!("primary eligibility worker must remain contained"))
        .unwrap_or_else(|_| panic!("primary eligibility preparation must complete"));
    assert_eq!(account.snapshot(), poisoned);
    let published = handoff_for_test(prepared);
    assert!(matches!(
        published,
        PublishedRetainedPublicationParts::Success { .. }
    ));
}

#[test]
fn release_underflow_latches_counter_invariant_without_saturating_event_count() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);

    let _ = account.release_retained_charge(AdmissionCharge::new(1, 1));
    let first = account.snapshot();
    assert!(first.poisoned());
    assert_eq!(first.poison_event_count(), 1);
    assert!(first.poison_event_count_exact());
    assert!(!first.poison_event_domain_valid());
    assert_eq!(
        first
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::CounterInvariantViolated)
    );

    let _ = account.release_retained_charge(AdmissionCharge::new(1, 1));
    let second = account.snapshot();
    assert_eq!(second.poison_event_count(), 2);
    assert!(second.poison_event_count_exact());
    assert!(!second.poison_event_domain_valid());
    assert_eq!(
        second.first_poison_evidence(),
        first.first_poison_evidence()
    );
}

#[test]
fn pending_single_rollback_underflow_records_one_in_domain_counter_fault() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let charge = AdmissionCharge::new(1, 1);
    let admission = account
        .try_admit_retained(charge, AdmittedRetainedShareWork { private: () })
        .unwrap_or_else(|_| panic!("single pending charge must admit"));
    {
        let mut state = lock_recovering_poison(&account.inner.value().linear);
        state.counters.live_bytes = 0;
        state.counters.live_allocations = 0;
    }

    let rollback =
        admission.rollback_with_fault(Some(AccountPoisonReason::OwnershipGraphUnverified));
    assert!(matches!(
        rollback,
        PendingRetainedRollback::ChargeReleaseUnverified(_)
    ));
    let snapshot = account.snapshot();
    assert_eq!(snapshot.poison_event_count(), 1);
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::CounterInvariantViolated)
    );
}

#[test]
fn pending_pair_rollback_underflow_records_one_in_domain_counter_fault() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let pair =
        RetainedAdmissionPairPlan::try_new(AdmissionCharge::new(1, 1), AdmissionCharge::new(1, 1))
            .unwrap_or_else(|_| panic!("pair charge must qualify"));
    let (admissions, _owners) = account
        .try_admit_start_retained_pair_after_action(
            account.try_admit_account_action().expect("account action"),
            pair,
            admitted_startup_share_work(&account),
        )
        .unwrap_or_else(|_| panic!("pending pair must admit"));
    {
        let mut state = lock_recovering_poison(&account.inner.value().linear);
        state.counters.live_bytes = 0;
        state.counters.live_allocations = 0;
    }

    let rollback =
        admissions.rollback_with_fault(Some(AccountPoisonReason::OwnershipGraphUnverified));
    assert!(matches!(
        rollback,
        PendingRetainedRollback::ChargeReleaseUnverified(_)
    ));
    let snapshot = account.snapshot();
    assert_eq!(snapshot.poison_event_count(), 1);
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::CounterInvariantViolated)
    );
}

#[test]
fn startup_admission_guard_underflow_records_one_counter_fault() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let pair =
        RetainedAdmissionPairPlan::try_new(AdmissionCharge::new(1, 1), AdmissionCharge::new(1, 1))
            .unwrap_or_else(|_| panic!("guard pair charge must qualify"));
    let (admissions, owners) = account
        .try_admit_start_retained_pair_after_action(
            account.try_admit_account_action().expect("account action"),
            pair,
            admitted_startup_share_work(&account),
        )
        .unwrap_or_else(|_| panic!("guard pair must admit"));
    let guard = StartupAdmissionGuard::new(&account, admissions, owners);
    {
        let mut state = lock_recovering_poison(&account.inner.value().linear);
        state.counters.live_bytes = 0;
        state.counters.live_allocations = 0;
    }

    drop(guard);
    let snapshot = account.snapshot();
    assert_eq!(snapshot.poison_event_count(), 1);
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::CounterInvariantViolated)
    );
}

#[test]
fn startup_admission_guard_unwind_records_exactly_one_outer_fault() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let pair =
        RetainedAdmissionPairPlan::try_new(AdmissionCharge::new(1, 1), AdmissionCharge::new(1, 1))
            .unwrap_or_else(|_| panic!("unwind pair charge must qualify"));
    let (admissions, owners) = account
        .try_admit_start_retained_pair_after_action(
            account.try_admit_account_action().expect("account action"),
            pair,
            admitted_startup_share_work(&account),
        )
        .unwrap_or_else(|_| panic!("unwind pair must admit"));

    let unwind = catch_unwind(AssertUnwindSafe(|| {
        let _guard = StartupAdmissionGuard::new(&account, admissions, owners);
        panic!("inject startup host unwind");
    }));
    assert!(unwind.is_err());
    let snapshot = account.snapshot();
    assert_eq!(snapshot.live_retained_bytes(), 0);
    assert_eq!(snapshot.live_retained_allocations(), 0);
    assert_eq!(snapshot.poison_event_count(), 1);
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::OwnershipGraphUnverified)
    );
}

#[test]
fn qualified_startup_capacity_failure_rolls_back_without_poisoning() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let pair =
        RetainedAdmissionPairPlan::try_new(AdmissionCharge::new(1, 1), AdmissionCharge::new(1, 1))
            .unwrap_or_else(|_| panic!("capacity pair charge must qualify"));
    let (admissions, owners) = account
        .try_admit_start_retained_pair_after_action(
            account.try_admit_account_action().expect("account action"),
            pair,
            admitted_startup_share_work(&account),
        )
        .unwrap_or_else(|_| panic!("capacity pair must admit"));
    let guard = StartupAdmissionGuard::new(&account, admissions, owners);

    let error = map_qualified_startup_batch_error::<TestDimension>(
        AttributionBatchCreateError::CapacityOverflow,
        |outer_reason| guard.rollback(outer_reason),
    );
    assert!(matches!(error, RetainedWriterError::BatchCapacity));
    let snapshot = account.snapshot();
    assert_eq!(snapshot.live_retained_bytes(), 0);
    assert_eq!(snapshot.live_retained_allocations(), 0);
    assert!(!snapshot.poisoned());
    assert_eq!(snapshot.poison_event_count(), 0);
}

#[test]
fn action_admission_counter_exhaustion_is_a_clean_non_poisoning_rejection() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    lock_recovering_poison(&account.inner.value().linear).admitted_account_actions = u64::MAX;
    let before = account.snapshot();

    let result = account.try_admit_account_action();
    assert!(matches!(
        result,
        Err(AccountActionAdmissionError::AccountingOverflow)
    ));
    assert_eq!(account.snapshot(), before);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn side_owner_escape_keeps_physical_and_operation_attribution_fail_closed() {
    let factory = account_factory(1);
    let operation = test_operation(&factory, 64);
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("side-owner publication must open"));
    let ledger = assembly
        .operation
        .as_ref()
        .expect("side-owner assembly retains operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let account = assembly
        .account
        .as_ref()
        .expect("side-owner assembly retains account")
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("test account share must be admitted"));
    let side_owner = StdArc::new(StdMutex::new(None));
    let escaped = StdArc::clone(&side_owner);
    let active_before = ledger.active_reservations();
    let live_before = ledger.current_live_bytes();
    let physical_before = account.snapshot();
    let plan = RetainedAllocationPlan::new(64, 1).unwrap();

    let result = assembly.try_retain_canonical::<CanonicalRootTag>(plan, move |_emplacer| {
        *lock_recovering_poison(&escaped) = Some(vec![0_u8; 64]);
        panic!("builder escaped side-owned storage")
    });

    assert!(matches!(
        result,
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
        ))
    ));
    assert!(lock_recovering_poison(&side_owner).is_some());
    assert_eq!(ledger.active_reservations(), active_before + 1);
    assert!(ledger.current_live_bytes() > live_before);
    let physical_after = account.snapshot();
    assert!(physical_after.live_retained_bytes() > physical_before.live_retained_bytes() + 64);
    assert!(
        physical_after.live_retained_allocations() > physical_before.live_retained_allocations()
    );
    let retained_live = ledger.current_live_bytes();
    drop(lock_recovering_poison(&side_owner).take());
    assert_eq!(ledger.current_live_bytes(), retained_live);
    drop(assembly);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn injected_shared_allocation_failure_does_not_run_the_builder() {
    static OBSERVED_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

    struct DropProbe {
        account: PhysicalStorageAccount,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            OBSERVED_LIVE_BYTES.store(
                self.account.snapshot().live_retained_bytes(),
                Ordering::SeqCst,
            );
        }
    }

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    OBSERVED_LIVE_BYTES.store(0, Ordering::SeqCst);
    fail_next_final_owner_allocation();
    let result = account.try_retain_with(empty_plan(), || {
        Ok::<_, RetainedBuildFailureCode>(DropProbe {
            account: account
                .share_after_test_admission()
                .unwrap_or_else(|_| panic!("test account share must be admitted")),
        })
    });

    assert!(matches!(
        result,
        Err(RetainedAllocationError::AllocationFailed)
    ));
    assert_eq!(OBSERVED_LIVE_BYTES.load(Ordering::SeqCst), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn allocation_failure_precedes_payload_construction_and_releases_the_lease() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    struct PanickingDrop;

    impl Drop for PanickingDrop {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(vec![0_u8; 8]);
        }
    }

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    DROP_COUNT.store(0, Ordering::SeqCst);
    fail_next_final_owner_allocation();
    let result = catch_unwind(AssertUnwindSafe(|| {
        account.try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDrop)
        })
    }));

    assert!(matches!(
        result,
        Ok(Err(RetainedAllocationError::AllocationFailed))
    ));
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn allocation_failure_rolls_back_block_attribution_before_build() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    let factory = account_factory(1);
    let operation = test_operation_with_writer_limits(&factory, 64, RetainedBatchLimits::new(2, 2));
    let mut assembly: TestPublication<PanickingDropProbe, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("publication assembly must open"));
    let probe = assembly
        .operation
        .as_ref()
        .expect("assembly retains the operation")
        .coordinator
        .retained_ledger()
        .test_probe();
    let before_active = probe.active_reservations();
    let before_live = probe.current_live_bytes();

    DROP_COUNT.store(0, Ordering::SeqCst);
    fail_next_final_owner_allocation();
    let error = assembly
        .try_retain_canonical(empty_plan(), |emplacer| {
            Ok(emplacer.place(PanickingDropProbe(&DROP_COUNT), CanonicalRootTag))
        })
        .expect_err("injected shared allocation must fail");

    assert!(matches!(
        error,
        RetainedPublicationError::Writer(RetainedWriterError::Physical(
            RetainedAllocationError::AllocationFailed
        ))
    ));
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
    assert_eq!(probe.active_reservations(), before_active);
    assert_eq!(probe.current_live_bytes(), before_live);
    drop(assembly);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn private_shares_charge_once_and_release_only_after_the_final_owner() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    let first = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(*b"payload!")
        })
        .unwrap();
    let charged = first.charged_bytes();
    assert_eq!(first.share_call_count(), 0);
    let second = first.share_after_test_admission();

    assert!(first.ptr_eq(&second));
    assert_eq!(first.share_call_count(), 1);
    assert_eq!(second.share_call_count(), 1);
    let third = second.share_after_test_admission();
    assert_eq!(first.share_call_count(), 2);
    assert_eq!(second.share_call_count(), 2);
    assert_eq!(third.share_call_count(), 2);
    assert_eq!(account.snapshot().live_retained_bytes(), charged);
    drop(first);
    assert_eq!(account.snapshot().live_retained_bytes(), charged);
    drop(second);
    assert_eq!(account.snapshot().live_retained_bytes(), charged);
    drop(third);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn concurrent_shared_drops_select_exactly_one_final_owner() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    struct DropCount;

    impl Drop for DropCount {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    DROP_COUNT.store(0, Ordering::SeqCst);
    let first = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(DropCount)
        })
        .unwrap();
    let second = first.share_after_test_admission();
    let third = first.share_after_test_admission();
    let start = StdArc::new(Barrier::new(4));
    let mut threads = Vec::new();
    for retained in [first, second, third] {
        let start = StdArc::clone(&start);
        threads.push(thread::spawn(move || {
            start.wait();
            drop(retained);
        }));
    }

    start.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
}

#[test]
fn final_shared_owner_contains_payload_drop_panic_and_retains_the_lease() {
    static DROP_COUNT: AtomicU64 = AtomicU64::new(0);

    struct PanickingDrop;

    impl Drop for PanickingDrop {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            std::panic::panic_any(vec![0_u8; 8]);
        }
    }

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    DROP_COUNT.store(0, Ordering::SeqCst);
    let first = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(PanickingDrop)
        })
        .unwrap();
    let charged = first.charged_bytes();
    let second = first.share_after_test_admission();
    drop(first);

    assert!(catch_unwind(AssertUnwindSafe(|| drop(second))).is_ok());
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
    assert_eq!(account.snapshot().live_retained_bytes(), charged);
    assert_eq!(account.snapshot().live_retained_allocations(), 1);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn payload_drop_barrier_keeps_headroom_unavailable_until_cleanup_finishes() {
    static ENTERED: Barrier = Barrier::new(2);
    static RELEASE: Barrier = Barrier::new(2);

    struct BlockingDrop;

    impl Drop for BlockingDrop {
        fn drop(&mut self) {
            ENTERED.wait();
            RELEASE.wait();
        }
    }

    let outer = FinalOwnerAllocation::<
        RetainedFinalOwnerInner<RetainedPayload<BlockingDrop>, LeafRetainedPolicy>,
    >::checked_layout_bytes()
    .unwrap();
    let factory = account_factory(1);
    let account = account(&factory, outer, 1);
    let retained = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(BlockingDrop)
        })
        .unwrap();
    let drop_thread = thread::spawn(move || drop(retained));

    ENTERED.wait();
    assert!(matches!(
        account.try_retain_with::<BlockingDrop>(empty_plan(), || {
            panic!("builder must not run before physical admission")
        }),
        Err(RetainedAllocationError::ByteLimitExceeded { .. })
            | Err(RetainedAllocationError::AllocationLimitExceeded { .. })
    ));
    RELEASE.wait();
    drop_thread.join().unwrap();
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    let replacement = account
        .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(()))
        .expect("headroom becomes available after payload cleanup and lease release");
    drop(replacement);
}

#[test]
fn payload_drop_observes_charge_and_factory_outlives_public_account_handle() {
    static OBSERVED_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

    struct DropProbe {
        account: PhysicalStorageAccount,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            OBSERVED_LIVE_BYTES.store(
                self.account.snapshot().live_retained_bytes(),
                Ordering::SeqCst,
            );
        }
    }

    let factory = account_factory(1);
    let account = account(&factory, 4_096, 8);
    OBSERVED_LIVE_BYTES.store(0, Ordering::SeqCst);
    let retained = account
        .try_retain_with(empty_plan(), || {
            Ok::<_, RetainedBuildFailureCode>(DropProbe {
                account: account
                    .share_after_test_admission()
                    .unwrap_or_else(|_| panic!("test account share must be admitted")),
            })
        })
        .unwrap();
    let charged = retained.charged_bytes();

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
    drop(retained);
    assert_eq!(OBSERVED_LIVE_BYTES.load(Ordering::SeqCst), charged);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn account_identity_is_opaque_and_foreign_accounts_do_not_match() {
    let factory = account_factory(2);
    let first_account = account(&factory, 4_096, 8);
    let second_account = account(&factory, 4_096, 8);
    let retained = first_account
        .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(5_u64))
        .unwrap();

    assert!(retained.belongs_to(&first_account));
    assert!(!retained.belongs_to(&second_account));
    let shared_first = first_account
        .share_after_test_admission()
        .unwrap_or_else(|_| panic!("test account share must be admitted"));
    assert!(first_account.is_same_account(&shared_first));
    assert!(!first_account.is_same_account(&second_account));
}

#[test]
fn foreign_account_cannot_authorize_retained_attribution_release() {
    let factory = account_factory(2);
    let first_account = account(&factory, 4_096, 8);
    let second_account = account(&factory, 4_096, 8);
    let retained = first_account
        .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(5_u64))
        .unwrap_or_else(|_| panic!("foreign-account probe allocation must retain"));
    let first_before = first_account.snapshot();

    assert!(matches!(
        drop_retained_allocation_before_attribution(retained, &second_account),
        RetainedStorageDropOutcome::ForeignOwnershipFault(_)
    ));
    assert_eq!(
        first_account.snapshot().live_retained_bytes(),
        first_before.live_retained_bytes()
    );
    assert_eq!(
        first_account.snapshot().live_retained_allocations(),
        first_before.live_retained_allocations()
    );
    assert_eq!(second_account.snapshot().live_retained_bytes(), 0);
    drop(first_account);
    drop(second_account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn concurrent_exact_limit_admission_is_linearizable() {
    let outer = FinalOwnerAllocation::<
        RetainedFinalOwnerInner<RetainedPayload<u64>, LeafRetainedPolicy>,
    >::checked_layout_bytes()
    .unwrap();
    let factory = account_factory(1);
    let account = account(&factory, outer, 1);
    let start = StdArc::new(Barrier::new(3));
    let attempts_complete = StdArc::new(Barrier::new(3));
    let release_owner = StdArc::new(Barrier::new(2));
    let successes = StdArc::new(AtomicU64::new(0));
    let mut threads = Vec::new();
    for value in [1_u64, 2_u64] {
        let account = account
            .share_after_test_admission()
            .unwrap_or_else(|_| panic!("test account share must be admitted"));
        let start = StdArc::clone(&start);
        let attempts_complete = StdArc::clone(&attempts_complete);
        let release_owner = StdArc::clone(&release_owner);
        let successes = StdArc::clone(&successes);
        threads.push(thread::spawn(move || {
            start.wait();
            let retained = account
                .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(value))
                .ok();
            if retained.is_some() {
                successes.fetch_add(1, Ordering::SeqCst);
            }
            attempts_complete.wait();
            if let Some(retained) = retained {
                release_owner.wait();
                drop(retained);
            }
        }));
    }

    start.wait();
    attempts_complete.wait();
    assert_eq!(successes.load(Ordering::SeqCst), 1);
    release_owner.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn final_owner_source_uses_only_the_qualified_extraction_path() {
    let non_final_owner_production = retained_non_final_owner_production_source();
    let final_owner = include_str!("final_owner.rs");
    let production = retained_production_source();

    for extraction in [
        "Arc::into_unique",
        "Arc::try_unique",
        "UniqueArc::into_inner",
    ] {
        assert!(
            !non_final_owner_production.contains(extraction),
            "final-owner extraction `{extraction}` escaped final_owner.rs"
        );
        assert!(
            final_owner.contains(extraction),
            "final_owner.rs lost required extraction `{extraction}`"
        );
    }
    assert_eq!(
        final_owner.matches("Arc::into_unique(shared)").count(),
        2,
        "ordinary release and final shared-owner drop are the only non-strict transitions"
    );
    assert_eq!(
        final_owner.matches("Arc::try_unique(shared)").count(),
        1,
        "strict pre-attribution cleanup has one qualified uniqueness transition"
    );
    assert_eq!(
        final_owner.matches("UniqueArc::into_inner(unique)").count(),
        3,
        "every qualified uniqueness transition extracts through the generic engine"
    );
    for forbidden in [
        "Arc::try_unwrap",
        "Arc::into_raw",
        "Arc::from_raw",
        "ArcBorrow",
        "ArcUnion",
        "OffsetArc",
        "ThinArc",
        "impl<T> Clone for RetainedAllocation",
        "impl Clone for PhysicalStorageAccount {",
    ] {
        assert!(
            !production.contains(forbidden),
            "forbidden final-owner bypass `{forbidden}` was introduced"
        );
    }
}

#[test]
fn generic_retained_authority_remains_crate_private() {
    let production = retained_production_source();
    let root_exports = include_str!("../lib.rs");
    let test_attribute = ["#[cfg(", "test)]"].concat();

    assert!(
        !root_exports.contains("pub use retained::*"),
        "the private retained module must never gain a glob re-export"
    );
    let export_prefix = "pub use retained::{";
    assert_eq!(root_exports.matches(export_prefix).count(), 1);
    let export_tail = root_exports
        .split_once(export_prefix)
        .expect("the crate root has one curated retained export block")
        .1;
    let export_body = export_tail
        .split_once("};")
        .expect("the curated retained export block is terminated")
        .0;
    let mut actual_exports: Vec<_> = export_body
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    let mut expected_exports = vec![
        "AccountPoisonEvidence",
        "AccountPoisonReason",
        "AccountPoisoned",
        "FinalizationContainmentError",
        "PhysicalStorageAccount",
        "PhysicalStorageAccountCreateError",
        "PhysicalStorageAccountFactory",
        "PhysicalStorageAccountFactoryCreateError",
        "PhysicalStorageAccountFactoryLimits",
        "PhysicalStorageAccountFactorySnapshot",
        "PhysicalStorageAccountLimits",
        "PhysicalStorageAccountSnapshot",
        "PhysicalStorageLimitError",
    ];
    actual_exports.sort_unstable();
    expected_exports.sort_unstable();
    assert_eq!(actual_exports, expected_exports);

    let mut public_declarations: Vec<_> = production
        .lines()
        .filter(|line| {
            line.starts_with("pub struct ")
                || line.starts_with("pub enum ")
                || line.starts_with("pub const ")
                || line.starts_with("pub type ")
                || line.starts_with("pub trait ")
        })
        .filter_map(|line| line.split_whitespace().nth(2))
        .map(|name| name.trim_end_matches(['{', ':']))
        .collect();
    public_declarations.sort_unstable();
    assert_eq!(public_declarations, expected_exports);

    assert!(
        production.contains(
            "type RetainedAllocation<T> = PolicyRetainedAllocation<T, LeafRetainedPolicy>;"
        )
    );
    assert!(!production.contains("pub(crate) type RetainedAllocation"));
    assert!(production.contains("\npub(super) struct RetainedPart<"));
    assert!(!production.contains("\npub(crate) struct RetainedPart<"));
    assert!(production.contains("\nenum RetainedClosureValidationError"));
    for forbidden in [
        "RetainedAllocation",
        "RetainedPart",
        "RetainedOperation",
        "RetainedAllocationPlan",
        "BlockEmplacer",
        "RetainedClosureValidationError",
        "RetainedSuccessClosureProof",
        "RetainedEvidenceClosureProof",
        "PublishedRetainedAggregate",
        "FinalOwnerAllocation",
        "AdmissionLease",
    ] {
        assert!(
            !actual_exports.contains(&forbidden),
            "raw retained authority `{forbidden}` escaped the crate root"
        );
    }

    {
        let method = "try_retain_with_admitted_handle";
        let needle = format!("fn {method}");
        let signatures: Vec<_> = production
            .lines()
            .filter(|line| line.contains(&needle))
            .collect();
        assert_eq!(
            signatures.len(),
            1,
            "raw physical mutator `{method}` must have one implementation"
        );
        assert!(
            signatures[0].trim_start().starts_with(&needle),
            "raw physical mutator `{method}` escaped the retained module: {}",
            signatures[0]
        );
    }
    assert!(
        !production.contains("fn share_after_admission"),
        "raw account sharing must not exist in production"
    );
    let count_test_only_wrappers = |candidate: &str| {
        candidate
            .lines()
            .zip(candidate.lines().skip(1))
            .filter(|(attribute, signature)| {
                attribute.trim_start() == test_attribute.as_str()
                    && *signature == "    fn try_retain_with<T>("
            })
            .count()
    };
    assert_eq!(
        count_test_only_wrappers(&production),
        1,
        "unadmitted physical retain wrapper must remain test-only"
    );
    let crlf_production = production.replace("\r\n", "\n").replace('\n', "\r\n");
    assert_eq!(
        count_test_only_wrappers(&crlf_production),
        1,
        "unadmitted physical retain wrapper gate must be line-ending independent"
    );
    for forbidden in [
        "fn try_prepare_success(",
        "fn try_prepare_rich_failure(",
        "pub struct RetainedSuccessClosureProof",
        "pub struct RetainedEvidenceClosureProof",
        "pub fn attach(",
        "pub fn detach(",
        "pub fn get_mut(",
        "pub fn into_block(",
        "pub fn replace(",
        "pub fn retag(",
        "pub fn retire(",
        "fn reset_poison(",
        "fn clear_poison(",
        "fn unpoison(",
        "fn rebrand_account(",
        "fn bypass_account_poison(",
        "RetainedAllocationVisitor",
    ] {
        assert!(
            !production.contains(forbidden),
            "raw retained authority `{forbidden}` escaped the deep module"
        );
    }

    let normalized = production.replace("\r\n", "\n");
    assert!(normalized.contains(
        "fn try_publication_eligibility(&self) -> Result<PublicationEligibility, AccountPoisoned>"
    ));
    assert_eq!(
        normalized.matches(".try_publication_eligibility()").count(),
        2
    );
    let eligible_method = |state_marker: &str| {
        normalized
            .split_once(state_marker)
            .unwrap_or_else(|| panic!("missing eligible state `{state_marker}`"))
            .1
            .split_once("fn into_prepared")
            .unwrap_or_else(|| panic!("missing consuming handoff for `{state_marker}`"))
            .1
            .split_once("\n    }\n}\n\nimpl")
            .unwrap_or_else(|| panic!("unterminated consuming handoff for `{state_marker}`"))
            .0
            .to_owned()
    };
    for eligible_tail in [
        eligible_method("struct EligibleTerminalFreezeGuard"),
        eligible_method("struct EligibleFallbackFreezeGuard"),
    ] {
        assert!(eligible_tail.contains("freeze_terminal_evidence("));
        assert!(eligible_tail.contains("physical.into_publication("));
        for forbidden in [
            "record_poison",
            "contain_",
            "catch_unwind",
            "try_retain",
            "try_discard",
            "drop_retained",
            "release_retained",
            "try_allocate",
            "assign_membership",
            "table_mut",
        ] {
            assert!(
                !eligible_tail.contains(forbidden),
                "post-eligibility tail regained fallible authority `{forbidden}`"
            );
        }
    }
    let freeze_end = format!("\n}}\n\n{test_attribute}");
    let freeze_tail = normalized
        .split_once("fn freeze_terminal_evidence")
        .expect("terminal evidence freeze seam exists")
        .1
        .split_once(&freeze_end)
        .expect("terminal evidence freeze seam is bounded")
        .0;
    assert!(freeze_tail.contains("_eligibility: PublicationEligibility"));
    for required in [
        "let snapshot = proof.freeze();",
        "drop(discarded_account);",
        "evidence.allocation().as_ref().freeze_from(&snapshot);",
        "drop(snapshot);",
    ] {
        assert!(
            freeze_tail.contains(required),
            "eligible freeze tail lost required pure step `{required}`"
        );
    }
    for forbidden in [
        "record_poison",
        "catch_unwind",
        "try_retain",
        "try_discard",
        "drop_retained",
        "release_retained",
        "try_allocate",
        "assign_membership",
        "table_mut",
    ] {
        assert!(
            !freeze_tail.contains(forbidden),
            "eligible freeze seam regained fallible authority `{forbidden}`"
        );
    }
}

#[test]
fn retained_handles_do_not_gain_unattributed_clone() {
    trait AmbiguousIfClone<Marker> {
        fn assert_not_clone() {}
    }

    impl<T: ?Sized> AmbiguousIfClone<()> for T {}
    impl<T: Clone> AmbiguousIfClone<u8> for T {}

    let _ = <PhysicalStorageAccount as AmbiguousIfClone<_>>::assert_not_clone;
    let _ = <RetainedAllocation<u64> as AmbiguousIfClone<_>>::assert_not_clone;
}

#[test]
fn public_handles_are_send_and_sync_for_thread_safe_payloads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PhysicalStorageAccountFactory>();
    assert_send_sync::<PhysicalStorageAccount>();
    assert_send_sync::<RetainedAllocation<u64>>();
}

#[test]
fn publication_evidence_precharges_the_capacity_envelope() {
    type EvidenceOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, u8, u32>;

    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let dimensions = (0_u8..17).map(|dimension| {
        (
            dimension,
            DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
                .with_published_outcome_evidence_limit(u64::MAX),
        )
    });
    let limits = LedgerLimits::new(u64::MAX, dimensions).unwrap();
    let capacity = CapacityPlan::for_type::<PublicationDimensionCell<u8>>(0, 0, 17).unwrap();
    let envelope_bytes = capacity.envelope_delta_bytes().unwrap();
    let plan = RetainedOperationStartPlan::new(
        RetainedWriterPlan::new(TestPhase::Build, 0, 0, RetainedBatchLimits::new(2, 2)),
        RetainedHandleSharePlan::new(0, 0),
        empty_plan(),
        OutputCommitment::new(0, 1),
        OutputCommitment::new(0, 1),
    );
    let operation =
        EvidenceOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("evidence operation must start"));
    assert_eq!(account.share_call_count(), 4);
    let allocation = operation.baseline.test_evidence_allocation();
    let outer_bytes = FinalOwnerAllocation::<
        RetainedFinalOwnerInner<RetainedPayload<PublicationEvidenceCells<u8>>, LeafRetainedPolicy>,
    >::checked_layout_bytes()
    .unwrap();

    assert_eq!(capacity.envelope_capacity(), 32);
    assert_eq!(allocation.charged_bytes(), outer_bytes + envelope_bytes);
    assert_eq!(allocation.charged_allocations(), 2);
    drop(operation);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn startup_share_qualification_rejection_preserves_the_borrowed_host_account() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 16);
    let policy = DimensionPolicy::reservable_bytes(7, 7, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let builder_called = AtomicBool::new(false);
    let result =
        TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            builder_called.store(true, Ordering::SeqCst);
            Ok(7_u32)
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Ledger(LedgerCause::LimitExceeded { .. })
        ))
    ));
    assert!(!builder_called.load(Ordering::SeqCst));
    assert_eq!(account.share_call_count(), 0);
    let retained = account
        .try_retain_with(empty_plan(), || Ok::<_, RetainedBuildFailureCode>(9_u32))
        .expect("borrowed host account remains usable after start rejection");
    drop(retained);
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn block_handle_clone_is_charged_before_builder_or_account_share() {
    let factory = account_factory(1);
    let account = account(&factory, 4_096, 16);
    let policy = DimensionPolicy::reservable_bytes(15, 15, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(u64::MAX);
    let limits = LedgerLimits::new(u64::MAX, [(TestDimension::Bytes, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer_plan(2, 2),
        share_work(),
        empty_plan(),
        OutputCommitment::new(TestDimension::Bytes, 1),
        OutputCommitment::new(TestDimension::Bytes, 1),
    );
    let operation =
        TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok(7_u32)
        })
        .unwrap_or_else(|_| panic!("exact pre-block handle budget must start"));
    let mut assembly: TestPublication<u64, u64> = operation
        .try_publication_assembly(publication_plan(1, 1))
        .unwrap_or_else(|_| panic!("exact pre-block handle budget must open tables"));
    let shares_before = account.share_call_count();
    let builder_called = AtomicBool::new(false);

    assert!(matches!(
        assembly.try_retain_canonical(empty_plan(), |emplacer| {
            builder_called.store(true, Ordering::SeqCst);
            Ok(emplacer.place(11, CanonicalRootTag))
        }),
        Err(RetainedPublicationError::Writer(
            RetainedWriterError::Ledger(LedgerCause::LimitExceeded { .. })
        ))
    ));
    assert!(!builder_called.load(Ordering::SeqCst));
    assert_eq!(account.share_call_count(), shares_before);
    drop(handoff_for_test(
        assembly
            .prepare_minimal_fallback()
            .unwrap_or_else(|_| panic!("handle rejection must preserve minimal fallback")),
    ));
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn startup_fallback_builder_error_returns_original_cause_and_restores_baseline() {
    reset_startup_test_observations();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let before = account.snapshot();
    let builder_called = AtomicBool::new(false);
    let (limits, plan) = startup_limits_and_plan();

    let result = TestOperation::try_start(
        OperationControl::default(),
        limits,
        &account,
        plan,
        || -> Result<u32, RetainedBuildFailureCode> {
            builder_called.store(true, Ordering::SeqCst);
            Err(BUILD_FAILED)
        },
    );

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        )) if cause == BUILD_FAILED
    ));
    assert!(builder_called.load(Ordering::SeqCst));
    assert_eq!(take_start_evidence_build_calls(), 0);
    assert_eq!(take_startup_failure_ledger_snapshot(), Some((0, 0)));
    let after = account.snapshot();
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations()
    );
    assert_eq!(after.poison_event_count(), before.poison_event_count());
    assert_eq!(account.share_call_count(), 4);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn startup_evidence_builder_error_drops_fallback_once_and_restores_baseline() {
    static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

    reset_startup_test_observations();
    FALLBACK_DROPS.store(0, Ordering::SeqCst);
    fail_next_start_evidence_build(BUILD_FAILED);
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let before = account.snapshot();
    let (limits, plan) = startup_limits_and_plan();
    type DropOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension, DropCounter>;

    let result =
        DropOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok::<DropCounter, RetainedBuildFailureCode>(DropCounter(&FALLBACK_DROPS))
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::Build(cause))
        )) if cause == BUILD_FAILED
    ));
    assert_eq!(take_start_evidence_build_calls(), 1);
    assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(take_startup_failure_ledger_snapshot(), Some((0, 0)));
    let after = account.snapshot();
    assert_eq!(after.live_retained_bytes(), before.live_retained_bytes());
    assert_eq!(
        after.live_retained_allocations(),
        before.live_retained_allocations()
    );
    assert_eq!(after.poison_event_count(), before.poison_event_count());
    assert_eq!(account.share_call_count(), 4);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn existing_poison_does_not_suppress_a_later_startup_ownership_fault() {
    reset_startup_test_observations();
    fail_next_start_evidence_build(BUILD_FAILED);
    lose_next_start_fallback_account_anchor();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (limits, plan) = startup_limits_and_plan();

    let result =
        TestOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            let _ = account.record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
            Ok::<u32, RetainedBuildFailureCode>(7)
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::OwnershipUnverified
        ))
    ));
    let snapshot = account.snapshot();
    assert_eq!(snapshot.poison_event_count(), 2);
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
    );

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn startup_evidence_builder_panic_releases_the_clean_fallback_sibling() {
    static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

    reset_startup_test_observations();
    FALLBACK_DROPS.store(0, Ordering::SeqCst);
    panic_next_start_evidence_build();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let before = account.snapshot();
    let evidence_capacity =
        CapacityPlan::for_type::<PublicationDimensionCell<TestDimension>>(0, 0, 1)
            .unwrap_or_else(|_| panic!("evidence panic capacity must qualify"));
    let nested_bytes = evidence_capacity
        .envelope_delta_bytes()
        .unwrap_or_else(|_| panic!("evidence panic envelope must fit"));
    let evidence_plan = RetainedAllocationPlan::new(nested_bytes, u64::from(nested_bytes != 0))
        .unwrap_or_else(|_| panic!("evidence panic plan must qualify"));
    let evidence_charge = account
        .checked_retained_charge::<PublicationEvidenceCells<TestDimension>>(evidence_plan)
        .unwrap_or_else(|_| panic!("evidence panic charge must qualify"));
    let (limits, plan) = startup_limits_and_plan();
    type DropOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension, DropCounter>;

    let result =
        DropOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok::<DropCounter, RetainedBuildFailureCode>(DropCounter(&FALLBACK_DROPS))
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
        ))
    ));
    assert_eq!(take_start_evidence_build_calls(), 1);
    assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
    let snapshot = account.snapshot();
    assert_eq!(
        snapshot.live_retained_bytes() - before.live_retained_bytes(),
        evidence_charge.bytes
    );
    assert_eq!(
        snapshot.live_retained_allocations() - before.live_retained_allocations(),
        evidence_charge.allocations
    );
    assert_eq!(snapshot.poison_event_count(), 1);
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedBuilderPanicked)
    );
    let (active, live) = take_startup_failure_ledger_snapshot()
        .unwrap_or_else(|| panic!("evidence panic must capture startup ledger state"));
    assert_eq!(active, 2);
    assert!(live > evidence_charge.bytes);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn startup_evidence_builder_panic_keeps_storage_release_as_r_and_records_builder_f() {
    static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

    reset_startup_test_observations();
    FALLBACK_DROPS.store(0, Ordering::SeqCst);
    panic_next_start_evidence_build();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (limits, plan) = startup_limits_and_plan();
    type PanicDropOperation = RetainedOperation<
        TestOwner,
        TestPhase,
        TestProgress,
        TestUnit,
        TestDimension,
        PanickingDropProbe,
    >;

    let result =
        PanicDropOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            Ok::<PanickingDropProbe, RetainedBuildFailureCode>(PanickingDropProbe(&FALLBACK_DROPS))
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::StorageDropUnverified
        ))
    ));
    assert_eq!(take_start_evidence_build_calls(), 1);
    assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
    let snapshot = account.snapshot();
    assert_eq!(snapshot.poison_event_count(), 2);
    assert!(snapshot.poison_event_count_exact());
    assert!(snapshot.poison_event_domain_valid());
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedStorageReleaseUnverified)
    );
    let (active, live) = take_startup_failure_ledger_snapshot()
        .unwrap_or_else(|| panic!("combined evidence panic must capture startup ledger state"));
    assert_eq!(active, 4);
    assert!(live > 0);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn startup_evidence_builder_panic_prefers_sibling_counter_f_without_builder_f() {
    static FALLBACK_DROPS: AtomicU64 = AtomicU64::new(0);

    reset_startup_test_observations();
    FALLBACK_DROPS.store(0, Ordering::SeqCst);
    panic_next_start_evidence_build();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (limits, plan) = startup_limits_and_plan();
    type DropOperation =
        RetainedOperation<TestOwner, TestPhase, TestProgress, TestUnit, TestDimension, DropCounter>;

    let result =
        DropOperation::try_start(OperationControl::default(), limits, &account, plan, || {
            let mut state = lock_recovering_poison(&account.inner.value().linear);
            state.counters.live_bytes = 0;
            state.counters.live_allocations = 0;
            drop(state);
            Ok::<DropCounter, RetainedBuildFailureCode>(DropCounter(&FALLBACK_DROPS))
        });

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::ChargeReleaseUnverified
        ))
    ));
    assert_eq!(take_start_evidence_build_calls(), 1);
    assert_eq!(FALLBACK_DROPS.load(Ordering::SeqCst), 1);
    let snapshot = account.snapshot();
    assert_eq!(snapshot.poison_event_count(), 1);
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::CounterInvariantViolated)
    );

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn startup_fallback_panic_contains_fallback_and_cleans_evidence() {
    reset_startup_test_observations();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let fallback_charge = account
        .checked_retained_charge::<u32>(empty_plan())
        .unwrap_or_else(|_| panic!("fallback charge must qualify"));
    let (limits, plan) = startup_limits_and_plan();

    let result = TestOperation::try_start(
        OperationControl::default(),
        limits,
        &account,
        plan,
        || -> Result<u32, RetainedBuildFailureCode> { panic!("inject fallback startup panic") },
    );

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
        ))
    ));
    assert_eq!(take_start_evidence_build_calls(), 0);
    let snapshot = account.snapshot();
    assert!(snapshot.poisoned());
    assert_eq!(snapshot.poison_event_count(), 1);
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::RetainedBuilderPanicked)
    );
    assert_eq!(snapshot.live_retained_bytes(), fallback_charge.bytes);
    assert_eq!(
        snapshot.live_retained_allocations(),
        fallback_charge.allocations
    );
    let (active, live) = take_startup_failure_ledger_snapshot()
        .unwrap_or_else(|| panic!("fallback panic must capture startup ledger state"));
    assert_eq!(active, 2);
    assert!(live > fallback_charge.bytes);
    assert_eq!(account.share_call_count(), 4);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn startup_fallback_panic_with_sibling_underflow_records_only_counter_fault() {
    reset_startup_test_observations();
    let factory = account_factory(1);
    let account = account(&factory, u64::MAX, 128);
    let (limits, plan) = startup_limits_and_plan();

    let result = TestOperation::try_start(
        OperationControl::default(),
        limits,
        &account,
        plan,
        || -> Result<u32, RetainedBuildFailureCode> {
            let mut state = lock_recovering_poison(&account.inner.value().linear);
            state.counters.live_bytes = 0;
            state.counters.live_allocations = 0;
            drop(state);
            panic!("inject fallback panic after counter corruption")
        },
    );

    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::ChargeReleaseUnverified
        ))
    ));
    assert_eq!(take_start_evidence_build_calls(), 0);
    let snapshot = account.snapshot();
    assert!(snapshot.poisoned());
    assert_eq!(snapshot.poison_event_count(), 1);
    assert_eq!(
        snapshot
            .first_poison_evidence()
            .map(AccountPoisonEvidence::reason),
        Some(AccountPoisonReason::CounterInvariantViolated)
    );
    assert_eq!(snapshot.live_retained_bytes(), 0);
    assert_eq!(snapshot.live_retained_allocations(), 0);
    let (active, live) = take_startup_failure_ledger_snapshot()
        .unwrap_or_else(|| panic!("underflow panic must capture startup ledger state"));
    assert_eq!(active, 4);
    assert!(live > 0);
    assert_eq!(account.share_call_count(), 4);

    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}

#[test]
fn single_value_bootstrap_and_primary_prepare_exclude_duplicate_tail_work() {
    let production = retained_production_source();
    assert!(!production.contains("set: fn("));
    assert!(!production.contains("AggregateSetterPanicked"));
    assert!(!production.contains("fn try_create_single_retained_part"));
    assert_eq!(production.matches(".try_prepare_typed_control(").count(), 1);
    assert_eq!(production.matches("ledger.try_bind_primary(").count(), 1);
    assert_eq!(
        production.matches("ledger.finish_bound_primary(").count(),
        1
    );
}
