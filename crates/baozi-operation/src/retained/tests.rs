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
        include_str!("allocation.rs"),
        include_str!("block_table.rs"),
        include_str!("evidence.rs"),
        include_str!("final_owner.rs"),
        include_str!("ownership.rs"),
        include_str!("startup.rs"),
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

fn retained_non_final_owner_production_source() -> String {
    [
        retained_root_production_source(),
        include_str!("account.rs"),
        include_str!("allocation.rs"),
        include_str!("block_table.rs"),
        include_str!("evidence.rs"),
        include_str!("ownership.rs"),
        include_str!("startup.rs"),
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

mod publication;

mod accounting;

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
            matches!(
                signatures[0].trim_start(),
                signature if signature.starts_with(&needle)
                    || signature.starts_with(&format!("pub(super) {needle}"))
            ),
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
                    && *signature == "    pub(super) fn try_retain_with<T>("
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
