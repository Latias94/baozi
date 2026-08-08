use super::*;
use crate::ledger::{
    AttributionBatchPlan, OutcomeEvidenceDestination, OutputCommitment,
    StartupAccountShareEnvelope, StartupAttributionBatchPlans, StartupRetainedAttribution,
    test_phase_catalog_construction_entries, test_startup_graph_allocation_entries,
    test_startup_graph_ledger_release_locks, test_startup_graph_storage_releases,
};
use crate::retained::{
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits, TerminalEvidenceKeySeal,
};
use crate::{DimensionPolicy, LedgerLimits};

#[derive(Debug)]
struct Owner;

impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Build,
}

impl TerminalEvidenceKeySeal for Phase {}

impl TerminalEvidenceKey for Phase {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Dimension {
    Bytes,
    Records,
}

impl TerminalEvidenceKeySeal for Dimension {}

impl TerminalEvidenceKey for Dimension {}

fn factory() -> PhysicalStorageAccountFactory {
    PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1)
            .unwrap_or_else(|error| panic!("test factory limits: {error}")),
    )
    .unwrap_or_else(|error| panic!("test factory: {error}"))
}

fn account(
    factory: &PhysicalStorageAccountFactory,
    max_bytes: u64,
    max_allocations: u64,
) -> PhysicalStorageAccount {
    factory
        .try_create_account(
            PhysicalStorageAccountLimits::new(max_bytes, max_allocations)
                .unwrap_or_else(|error| panic!("test account limits: {error}")),
        )
        .unwrap_or_else(|error| panic!("test account: {error}"))
}

const fn charge(bytes: u64, allocations: u64) -> AdmissionCharge {
    AdmissionCharge::new(bytes, allocations)
}

fn physical_manifest(
    fallback: AdmissionCharge,
    evidence: AdmissionCharge,
) -> StartupPhysicalManifest {
    StartupPhysicalManifest::try_new(fallback, evidence)
        .unwrap_or_else(|error| panic!("test physical manifest: {error:?}"))
}

fn qualified_ledger_plan(
    fallback_bytes: u64,
    evidence_bytes: u64,
) -> StartupLedgerPlan<Owner, Phase, Dimension> {
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
            ),
            (
                Dimension::Records,
                DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                    .with_published_outcome_evidence_limit(u64::MAX),
            ),
        ],
    )
    .unwrap_or_else(|error| panic!("test ledger limits: {error}"));
    let batches = StartupAttributionBatchPlans::new(
        AttributionBatchPlan::try_new(Phase::Build, Dimension::Bytes, 2, 1)
            .unwrap_or_else(|error| panic!("canonical assembly plan: {error:?}")),
        AttributionBatchPlan::try_new(Phase::Build, Dimension::Bytes, 2, 2)
            .unwrap_or_else(|error| panic!("evidence assembly plan: {error:?}")),
        AttributionBatchPlan::try_new(Phase::Build, Dimension::Bytes, 1, 1)
            .unwrap_or_else(|error| panic!("fallback plan: {error:?}")),
        AttributionBatchPlan::try_new(Phase::Build, Dimension::Bytes, 1, 1)
            .unwrap_or_else(|error| panic!("evidence plan: {error:?}")),
    );

    limits
        .try_qualify_startup(
            batches,
            [
                StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, fallback_bytes),
                StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, evidence_bytes),
            ],
            StartupAccountShareEnvelope::four_identical([
                (Dimension::Records, 1),
                (Dimension::Records, 2),
            ]),
            [
                OutputCommitment::<Dimension, OutcomeEvidenceDestination>::new(
                    Dimension::Records,
                    4,
                ),
                OutputCommitment::new(Dimension::Records, 6),
            ],
        )
        .unwrap_or_else(|error| panic!("test startup qualification: {error}"))
}

fn cross_plane_plan() -> StartupCrossPlanePlan<Owner, Phase, Dimension> {
    StartupCrossPlanePlan::try_new(
        qualified_ledger_plan(2, 3),
        physical_manifest(charge(2, 1), charge(3, 1)),
    )
    .unwrap_or_else(|error| panic!("test cross-plane plan: {error:?}"))
}

fn ledger_observation(ledger: &Ledger<Owner, Phase, Dimension>) -> (u64, u64, u64, u64, u64) {
    let probe = ledger.test_probe();
    let records = ledger
        .dimension_evidence(Dimension::Records)
        .unwrap_or_else(|error| panic!("records evidence exists: {error}"));
    (
        probe.current_live_bytes(),
        probe.active_reservations(),
        records.attempted(),
        records.consumed(),
        records.published_outcome_evidence(),
    )
}

fn expect_uncertain(
    result: Result<
        ReadyStartupCrossPlane<Owner, Phase, Dimension>,
        StartupCrossPlaneAdmissionFailure<Owner, Phase, Dimension>,
    >,
) -> CrossPlaneAdmissionContainment<Owner, Phase, Dimension> {
    match result {
        Err(StartupCrossPlaneAdmissionFailure::Uncertain(containment)) => containment,
        Err(StartupCrossPlaneAdmissionFailure::OperationStart(_)) => {
            panic!("injected physical panic reached the operation-start gate")
        }
        Err(StartupCrossPlaneAdmissionFailure::LedgerInitialization(_)) => {
            panic!("injected physical panic reached ledger initialization")
        }
        Err(StartupCrossPlaneAdmissionFailure::TerminalEvidenceCatalogInvariant) => {
            panic!("injected physical panic reached terminal evidence catalog qualification")
        }
        Err(StartupCrossPlaneAdmissionFailure::LedgerAdmission(_)) => {
            panic!("injected physical panic reached ledger admission")
        }
        Err(StartupCrossPlaneAdmissionFailure::OwnerShare(_)) => {
            panic!("injected physical panic reached startup owner sharing")
        }
        Err(StartupCrossPlaneAdmissionFailure::PhysicalNoMutation(_)) => {
            panic!("injected physical panic became a typed rejection")
        }
        Ok(ready) => {
            std::mem::forget(ready);
            panic!("injected physical panic reported success")
        }
    }
}

#[test]
fn aggregate_plan_rejects_cross_source_bytes_before_any_side_effect() {
    let phase_constructions = test_phase_catalog_construction_entries();
    let graph_allocations = test_startup_graph_allocation_entries();

    assert!(matches!(
        StartupCrossPlanePlan::try_new(
            qualified_ledger_plan(2, 3),
            physical_manifest(charge(1, 1), charge(3, 1)),
        ),
        Err(StartupCrossPlanePlanError::FallbackByteMismatch {
            qualified: 2,
            physical: 1,
        })
    ));
    assert!(matches!(
        StartupCrossPlanePlan::try_new(
            qualified_ledger_plan(2, 3),
            physical_manifest(charge(2, 1), charge(4, 1)),
        ),
        Err(StartupCrossPlanePlanError::EvidenceByteMismatch {
            qualified: 3,
            physical: 4,
        })
    ));
    assert_eq!(
        test_phase_catalog_construction_entries(),
        phase_constructions
    );
    assert_eq!(test_startup_graph_allocation_entries(), graph_allocations);
}

#[test]
fn poisoned_operation_gate_precedes_ledger_and_owner_work() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
    let allocations = test_startup_graph_allocation_entries();
    let phase_constructions = test_phase_catalog_construction_entries();
    let shares = account.share_call_count();

    let result = account.try_prepare_startup_cross_plane(&[Phase::Build], cross_plane_plan());

    assert!(matches!(
        result,
        Err(StartupCrossPlaneAdmissionFailure::OperationStart(
            OperationStartAdmissionError::AccountPoisoned(_)
        ))
    ));
    assert_eq!(test_startup_graph_allocation_entries(), allocations);
    assert_eq!(
        test_phase_catalog_construction_entries(),
        phase_constructions
    );
    assert_eq!(account.share_call_count(), shares);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
}

#[test]
fn ledger_rejection_precedes_all_four_owner_shares_and_physical_admission() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    reset_startup_lock_probe();
    let shares = account.share_call_count();

    let result = account.try_prepare_startup_cross_plane(&[], cross_plane_plan());

    assert!(matches!(
        result,
        Err(StartupCrossPlaneAdmissionFailure::LedgerAdmission(_))
    ));
    assert_eq!(account.share_call_count(), shares);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(
        startup_lock_probe_snapshot(),
        StartupLockProbeSnapshot {
            operation_gate_calls: 1,
            physical_transition_calls: 0,
            current_depth: 0,
            max_depth: 1,
        }
    );
}

#[test]
fn ready_pair_charges_four_shares_exactly_once() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let shares = account.share_call_count();
    let allocations = test_startup_graph_allocation_entries();

    let ready = account
        .try_prepare_startup_cross_plane(&[Phase::Build], cross_plane_plan())
        .unwrap_or_else(|_| panic!("cross-plane startup must be ready"));

    assert_eq!(account.share_call_count(), shares + 4);
    assert_eq!(test_startup_graph_allocation_entries(), allocations + 8);
    let ledger = &ready
        .ownership
        .as_ref()
        .unwrap_or_else(|| panic!("ready pair owns the complete graph"))
        .ledger_graph
        .ledger;
    let records = ledger
        .dimension_evidence(Dimension::Records)
        .unwrap_or_else(|error| panic!("records evidence exists: {error}"));
    assert_eq!(records.attempted(), 12);
    assert_eq!(records.consumed(), 12);
    assert_eq!(account.snapshot().live_retained_bytes(), 5);
    assert_eq!(account.snapshot().live_retained_allocations(), 2);
    std::mem::forget(ready);
}

#[test]
fn aggregate_plan_preserves_each_checked_physical_allocation_count() {
    let factory = factory();
    let account = account(&factory, 5, 5);
    let plan = StartupCrossPlanePlan::try_new(
        qualified_ledger_plan(2, 3),
        physical_manifest(charge(2, 2), charge(3, 3)),
    )
    .unwrap_or_else(|error| panic!("allocation-qualified cross-plane plan: {error:?}"));

    let ready = account
        .try_prepare_startup_cross_plane(&[Phase::Build], plan)
        .unwrap_or_else(|_| panic!("cross-plane startup must preserve allocation counts"));

    assert_eq!(account.snapshot().live_retained_bytes(), 5);
    assert_eq!(account.snapshot().live_retained_allocations(), 5);
    std::mem::forget(ready);
}

#[test]
fn physical_no_mutation_drops_the_complete_graph_normally() {
    let factory = factory();
    let account = account(&factory, 4, 2);
    let releases = test_startup_graph_storage_releases();
    let release_locks = test_startup_graph_ledger_release_locks();

    let result = account.try_prepare_startup_cross_plane(&[Phase::Build], cross_plane_plan());

    assert!(matches!(
        result,
        Err(StartupCrossPlaneAdmissionFailure::PhysicalNoMutation(
            StartupPhysicalNoMutation::ByteLimitExceeded {
                requested: 5,
                limit: 4,
            }
        ))
    ));
    assert_eq!(test_startup_graph_storage_releases(), releases + 8);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks + 1);
    assert_eq!(account.share_call_count(), 4);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert!(!account.snapshot().poisoned());
    assert_eq!(account.snapshot().poison_event_count(), 0);
}

#[test]
fn precommit_panic_contains_the_graph_without_fabricating_a_refund() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let releases = test_startup_graph_storage_releases();
    let release_locks = test_startup_graph_ledger_release_locks();
    set_startup_transition_panic(StartupTransitionPanicPoint::BeforeCommit);

    let containment = expect_uncertain(
        account.try_prepare_startup_cross_plane(&[Phase::Build], cross_plane_plan()),
    );
    assert_eq!(account.snapshot().poison_event_count(), 1);
    drop(containment);

    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
    assert_eq!(account.share_call_count(), 4);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert!(account.snapshot().poisoned());
    assert_eq!(account.snapshot().poison_event_count(), 1);
}

#[test]
fn postcommit_panic_contains_the_graph_and_complete_physical_charge() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let releases = test_startup_graph_storage_releases();
    let release_locks = test_startup_graph_ledger_release_locks();
    set_startup_transition_panic(StartupTransitionPanicPoint::AfterCommit);

    let containment = expect_uncertain(
        account.try_prepare_startup_cross_plane(&[Phase::Build], cross_plane_plan()),
    );
    assert_eq!(account.snapshot().poison_event_count(), 1);
    drop(containment);

    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
    assert_eq!(account.share_call_count(), 4);
    assert_eq!(account.snapshot().live_retained_bytes(), 5);
    assert_eq!(account.snapshot().live_retained_allocations(), 2);
    assert!(account.snapshot().poisoned());
    assert_eq!(account.snapshot().poison_event_count(), 1);
}

#[test]
fn ready_drop_poison_contains_both_accounting_planes() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let releases = test_startup_graph_storage_releases();
    let release_locks = test_startup_graph_ledger_release_locks();
    let ready = account
        .try_prepare_startup_cross_plane(&[Phase::Build], cross_plane_plan())
        .unwrap_or_else(|_| panic!("cross-plane startup must be ready"));

    drop(ready);

    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
    assert_eq!(account.snapshot().live_retained_bytes(), 5);
    assert!(account.snapshot().poisoned());
    assert_eq!(account.snapshot().poison_event_count(), 1);
}

#[test]
fn ready_split_is_state_only_and_runtime_drop_remains_fail_closed() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let ready = account
        .try_prepare_startup_cross_plane(&[Phase::Build], cross_plane_plan())
        .unwrap_or_else(|_| panic!("cross-plane startup must be ready"));
    reset_startup_lock_probe();
    let allocations = test_startup_graph_allocation_entries();
    let releases = test_startup_graph_storage_releases();
    let release_locks = test_startup_graph_ledger_release_locks();
    let shares = account.share_call_count();
    let account_before = account.snapshot();
    let ledger_before = ledger_observation(
        &ready
            .ownership
            .as_ref()
            .unwrap_or_else(|| panic!("ready pair owns its ledger"))
            .ledger_graph
            .ledger,
    );

    let (started, baseline, guard) = ready.split();

    assert_eq!(
        startup_lock_probe_snapshot(),
        StartupLockProbeSnapshot::default()
    );
    assert_eq!(test_startup_graph_allocation_entries(), allocations);
    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
    assert_eq!(account.share_call_count(), shares);
    assert_eq!(account.snapshot(), account_before);
    assert_eq!(ledger_observation(&started.ledger), ledger_before);
    drop(baseline);
    drop(guard);
    drop(started);
    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
    assert_eq!(account.snapshot().live_retained_bytes(), 5);
    assert!(account.snapshot().poisoned());
    assert_eq!(account.snapshot().poison_event_count(), 1);
}

#[test]
fn cross_plane_authority_and_products_remain_private_and_indivisible() {
    let source = include_str!("../startup.rs");
    let ledger_root = include_str!("../../ledger.rs");
    let crate_root = include_str!("../../lib.rs");

    assert!(source.contains("struct StartupGraphPreholdAuthority<"));
    assert!(source.contains("manifest: StartupPhysicalManifest"));
    assert!(source.contains("plan: StartupCrossPlanePlan<O, P, D>"));
    assert!(!source.contains("parts: StartupLedgerPlanParts"));
    assert!(!source.contains("fn prehold_startup_account_owners"));
    for forbidden in [
        "ManuallyDrop",
        "unsafe",
        "dyn ",
        "visitor",
        "attach",
        "fn accept(",
        "fn into_parts",
        "charge_observed_batch",
    ] {
        assert!(
            !source.contains(forbidden),
            "cross-plane rail regained forbidden seam `{forbidden}`"
        );
    }
    assert!(ledger_root.contains("StartupAttributionGraph"));
    for private in [
        "StartupAttributionGraph",
        "StartupCrossPlanePlan",
        "ReadyStartupCrossPlane",
        "CrossPlaneAdmissionContainment",
    ] {
        assert!(
            !crate_root.contains(private),
            "crate root exposed private startup type `{private}`"
        );
    }

    assert!(!source.contains("PromotedStartupCrossPlane"));
    assert_eq!(source.matches("pub(super) fn split(").count(), 1);
    assert_eq!(source.matches("StartedLedger { ledger }").count(), 1);
    let ready_split = source
        .split("pub(super) fn split(")
        .nth(1)
        .unwrap_or_else(|| panic!("Ready-only split exists"))
        .split("impl<O, P, D> Drop for ReadyStartupCrossPlane")
        .next()
        .unwrap_or_else(|| panic!("Ready-only split boundary exists"));
    for forbidden in [
        "lock",
        "share",
        "try_",
        "Arc",
        "UniqueArc",
        "Box",
        "Vec",
        "String",
        "clone",
        "Result",
        "?",
    ] {
        assert!(
            !ready_split.contains(forbidden),
            "Ready-only split regained fallible or effectful operation `{forbidden}`"
        );
    }

    let physical_transition = source
        .split("fn try_commit_physical")
        .nth(1)
        .unwrap_or_else(|| panic!("graph-authorized physical transition exists"))
        .split("let owners")
        .next()
        .unwrap_or_else(|| panic!("physical transition signature boundary exists"));
    assert!(!physical_transition.contains("StartupPhysicalManifest"));
}
