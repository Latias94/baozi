use super::evidence::{
    fail_next_terminal_evidence_qualification, panic_next_terminal_evidence_qualification,
    panic_next_terminal_evidence_write,
};
use super::terminal::{
    cause_after_primary_ledger_rejection, panic_next_evidence_only_primary_bind,
};
use super::*;
use crate::CounterClass;
use crate::DimensionPolicy;
use crate::ledger::LedgerTestProbe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc as StdArc, Barrier, Mutex as StdMutex};
use std::thread;

const BUILD_FAILED: RetainedBuildFailureCode = RetainedBuildFailureCode::new(1);

impl TerminalEvidenceKeySeal for u8 {}

impl TerminalEvidenceKey for u8 {}

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
        include_str!("allocation.rs"),
        include_str!("block_table.rs"),
        include_str!("evidence.rs"),
        include_str!("final_owner.rs"),
        include_str!("operation.rs"),
        include_str!("operation_start/mod.rs"),
        include_str!("operation_start/config.rs"),
        include_str!("operation_start/share.rs"),
        include_str!("operation_start/pair.rs"),
        include_str!("ownership.rs"),
        include_str!("physical.rs"),
        include_str!("publication_contract.rs"),
        include_str!("range.rs"),
        include_str!("startup.rs"),
        include_str!("terminal.rs"),
        include_str!("terminal_failure.rs"),
        include_str!("terminal_output.rs"),
    ]
    .join("\n")
}

#[test]
fn allocation_module_remains_table_agnostic() {
    let source = include_str!("allocation.rs");
    for forbidden in [
        "RetainedBlockTable",
        "block_table",
        "TableRetainedPolicy",
        "RetainedTableAllocation",
    ] {
        assert!(
            !source.contains(forbidden),
            "allocation module must not depend on {forbidden}"
        );
    }
}

#[test]
fn retained_batch_range_uses_invariant_brands_and_no_table_internals() {
    let table = include_str!("block_table.rs");
    let range = include_str!("range.rs");

    assert!(table.contains("type InvariantRetainedBrand<Block, Tag>"));
    assert!(table.contains("PhantomData<fn(Block, Tag) -> (Block, Tag)>"));
    assert!(table.contains("marker: InvariantRetainedBrand<Block, Tag>"));
    assert!(range.contains("marker: InvariantRetainedBrand<Block, Tag>"));
    assert!(range.contains("QualifiedCanonicalRetainBatch"));
    assert!(range.contains("try_qualify_canonical_retain_batch"));
    assert!(range.contains("preflight_repeated_observed_batch"));
    for table_internal in [
        ".members",
        ".total_blocks",
        ".max_new_blocks",
        ".new_blocks",
        ".committed_members",
        ".committed_new_blocks",
        ".committed_derived_members",
        ".committed_new_block_bytes",
        ".qualified_capacity",
    ] {
        assert!(
            !range.contains(table_internal),
            "range orchestration copied private table invariant `{table_internal}`"
        );
    }
}

#[test]
fn retained_batch_rollback_has_one_storage_first_attribution_suffix_seam() {
    let table = include_str!("block_table.rs");
    let reservation = include_str!("../ledger/reservation.rs");
    let seam = "finish_last_attribution_after_storage_rollback";

    assert_eq!(table.matches(seam).count(), 1);
    assert_eq!(reservation.matches(seam).count(), 1);
    let rollback = table
        .split("pub(in crate::retained) fn try_rollback_prefix")
        .nth(1)
        .expect("retained batch rollback exists")
        .split("pub(in crate::retained) fn try_complete")
        .next()
        .expect("retained batch rollback boundary exists");
    let storage_drop = rollback
        .find("drop_retained_allocation_before_attribution")
        .expect("batch rollback releases retained storage");
    let attribution_finish = rollback
        .find(seam)
        .expect("batch rollback resolves attribution");
    assert!(
        storage_drop < attribution_finish,
        "batch rollback must release storage before attribution"
    );
    assert!(!reservation.contains("pop_attribution_for_storage_rollback"));
}

#[test]
fn retained_exactness_production_path_uses_only_incremental_proofs() {
    let table = include_str!("block_table.rs");
    let summary_exact = table
        .split("fn committed_summary_is_bounded")
        .nth(1)
        .expect("committed table summary check exists")
        .split("fn try_push_committed_member")
        .next()
        .expect("committed table summary boundary exists");
    let mapping_exact = table
        .split("fn canonical_source_mapping_is_exact")
        .nth(1)
        .expect("canonical source mapping exactness check exists")
        .split("pub(super) fn retained_block_table_is_exact")
        .next()
        .expect("canonical source mapping exactness boundary exists");
    let table_exact = table
        .split("pub(super) fn retained_block_table_is_exact")
        .nth(1)
        .expect("retained table exactness check exists")
        .split("fn audit_retained_block_table_is_exact")
        .next()
        .expect("test-only retained table audit boundary exists");

    for production_check in [summary_exact, mapping_exact, table_exact] {
        for scan in [".iter()", ".enumerate()", "for ("] {
            assert!(
                !production_check.contains(scan),
                "production exactness check must not contain structural scan `{scan}`"
            );
        }
    }
    for proof in [
        "root_membership",
        "committed_members",
        "committed_new_blocks",
        "committed_derived_members",
        "committed_new_block_bytes",
        "mapped_entries",
    ] {
        assert!(
            summary_exact.contains(proof)
                || mapping_exact.contains(proof)
                || table_exact.contains(proof),
            "production exactness must consume incremental proof `{proof}`"
        );
    }

    let (before_audit, audit) = table
        .split_once("fn audit_retained_block_table_is_exact")
        .expect("full retained table audit exists");
    assert!(before_audit.trim_end().ends_with("#[cfg(test)]"));
    assert!(audit.contains("members.iter().enumerate()"));
    assert!(audit.contains("for (source_index, destination_ordinal)"));
    assert!(audit.contains("mapping.entries"));

    assert_eq!(table.matches("self.members.push(").count(), 1);
    assert_eq!(table.matches("self.members.pop()").count(), 1);
    assert_eq!(table.matches("self.mapped_entries =").count(), 2);
    for counter in [
        "committed_members",
        "committed_new_blocks",
        "committed_derived_members",
        "committed_new_block_bytes",
    ] {
        assert_eq!(
            table.matches(&format!("self.{counter} =")).count(),
            2,
            "committed counter `{counter}` must change only through push/pop proof transitions"
        );
    }
}

fn retained_non_final_owner_production_source() -> String {
    [
        retained_root_production_source(),
        include_str!("account.rs"),
        include_str!("allocation.rs"),
        include_str!("block_table.rs"),
        include_str!("evidence.rs"),
        include_str!("operation.rs"),
        include_str!("operation_start/mod.rs"),
        include_str!("operation_start/config.rs"),
        include_str!("operation_start/share.rs"),
        include_str!("operation_start/pair.rs"),
        include_str!("ownership.rs"),
        include_str!("physical.rs"),
        include_str!("publication_contract.rs"),
        include_str!("range.rs"),
        include_str!("startup.rs"),
        include_str!("terminal.rs"),
        include_str!("terminal_failure.rs"),
        include_str!("terminal_output.rs"),
    ]
    .join("\n")
}

#[test]
fn retained_publication_has_one_terminal_ledger_evidence_rail() {
    let production = retained_production_source();

    assert!(
        production.contains("TerminalLedgerEvidence"),
        "the retained ownership graph must publish the complete terminal ledger evidence"
    );
    for removed in [
        "PublicationEvidenceCells",
        "PublicationDimensionCell",
        "PublishedDimensionSnapshot",
    ] {
        assert!(
            !production.contains(removed),
            "legacy terminal evidence rail `{removed}` still has production reachability"
        );
    }
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
    Unregistered,
}

impl TerminalEvidenceKeySeal for TestPhase {}

impl TerminalEvidenceKey for TestPhase {}

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
    FailureTerminalWork,
    #[cfg(feature = "_u4-activation")]
    Work,
    Unregistered,
}

impl TerminalEvidenceKeySeal for TestDimension {}

impl TerminalEvidenceKey for TestDimension {}

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

impl TerminalEvidenceKeySeal for PrimaryPanicDimension {}

impl TerminalEvidenceKey for PrimaryPanicDimension {}

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

enum PublishedRetainedPublicationParts<CanonicalBlock, EvidenceBlock, F, P, D> {
    Success {
        canonical: PublishedCanonicalOutput<CanonicalBlock>,
        outcome_evidence: PublishedOutcomeEvidenceOutput<EvidenceBlock>,
        ledger_evidence: PublishedLedgerEvidence<P, D>,
    },
    RichFailure {
        outcome_evidence: PublishedRichFailureOutput<EvidenceBlock>,
        ledger_evidence: PublishedLedgerEvidence<P, D>,
    },
    MinimalFallback {
        fallback: PublishedFallbackOutput<F>,
        ledger_evidence: PublishedLedgerEvidence<P, D>,
    },
}

fn handoff_for_test<CanonicalBlock, EvidenceBlock, F, P, D>(
    prepared: PreparedRetainedPublication<CanonicalBlock, EvidenceBlock, F, P, D>,
) -> PublishedRetainedPublicationParts<CanonicalBlock, EvidenceBlock, F, P, D>
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
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

fn published_failure_cause<P, D>(evidence: &PublishedLedgerEvidence<P, D>) -> PreparationFailureCode
where
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey,
{
    match evidence
        .terminal_cause()
        .unwrap_or_else(|| panic!("published ledger evidence must have a terminal cause"))
    {
        TerminalEvidenceCause::Failure(cause) => cause,
        TerminalEvidenceCause::Success => panic!("published failure must not report success"),
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

fn publication_plan_with_canonical_source_mapping(
    canonical_blocks: usize,
    evidence_blocks: usize,
    source_blocks: usize,
) -> RetainedPublicationPlan<TestPhase, TestDimension> {
    RetainedPublicationPlan::success(
        RetainedBlockTablePlan::new(
            writer_plan(canonical_blocks + 1, 2),
            canonical_blocks,
            canonical_blocks,
        )
        .with_canonical_source_mapping_capacity(source_blocks),
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

fn test_derived_operation_on_account_with_control<CanonicalBlock>(
    account: &PhysicalStorageAccount,
    source: &PublishedCanonicalOutput<CanonicalBlock>,
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
    RetainedOperation::try_start_derived_with_phase_catalog(
        control,
        limits,
        &[TestPhase::Build],
        account,
        source.aggregate(),
        plan,
        || Ok(7_u32),
    )
    .unwrap_or_else(|_| panic!("derived test operation bootstrap must succeed"))
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
    reset_startup_footprint_mismatches();
    reset_startup_payload_allocation_attempts();
    let _ = take_injected_start_evidence_build_failure();
    let _ = take_injected_start_evidence_build_panic();
    let _ = take_injected_start_evidence_structural_failure();
    let _ = take_injected_terminal_evidence_catalog_failure();
    let _ = take_injected_start_fallback_account_anchor_loss();
    let _ = take_start_evidence_build_calls();
    let _ = take_startup_failure_ledger_snapshot();
    reset_retained_final_owner_allocator_entries();
    let _ = take_startup_ready_final_owner_allocator_entries();
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

mod accounting;
mod architecture;
mod publication;
mod startup_operation;
