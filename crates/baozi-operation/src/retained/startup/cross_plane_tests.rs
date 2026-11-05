use super::*;
use crate::ledger::{
    AttributionBatchPlan, LedgerCause, OutcomeEvidenceDestination, OutputCommitment,
    StartupAccountShareEnvelope, StartupAttributionBatchPlans, StartupLedgerPlanParts,
    StartupRetainedAttribution, test_phase_catalog_construction_entries,
    test_startup_graph_allocation_entries, test_startup_graph_ledger_release_locks,
    test_startup_graph_storage_releases,
};
use crate::retained::{
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits, TerminalEvidenceKey, TerminalEvidenceKeySeal,
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
    Scratch,
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

fn ledger_request(
    fallback_bytes: u64,
    evidence_bytes: u64,
) -> (
    LedgerLimits<Dimension>,
    StartupLedgerPlanParts<Owner, Phase, Dimension>,
) {
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
            (
                Dimension::Scratch,
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
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

    (
        limits,
        StartupLedgerPlanParts {
            batches,
            retained_attributions: [
                StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, fallback_bytes),
                StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, evidence_bytes),
            ],
            account_shares: StartupAccountShareEnvelope::four_identical([
                (Dimension::Records, 1),
                (Dimension::Records, 2),
            ]),
            commitments: [
                OutputCommitment::<Dimension, OutcomeEvidenceDestination>::new(
                    Dimension::Records,
                    4,
                ),
                OutputCommitment::new(Dimension::Records, 6),
            ],
        },
    )
}

fn cross_plane_plan() -> (
    LedgerLimits<Dimension>,
    StartupCrossPlanePlan<Owner, Phase, Dimension>,
) {
    let (limits, parts) = ledger_request(2, 3);
    let plan = StartupCrossPlanePlan::try_new(parts, physical_manifest(charge(2, 1), charge(3, 1)))
        .unwrap_or_else(|error| panic!("test cross-plane plan: {error:?}"));
    (limits, plan)
}

fn start_ledger<'account>(
    account: &'account PhysicalStorageAccount,
    phases: &[Phase],
    limits: LedgerLimits<Dimension>,
) -> Result<
    (
        Ledger<Owner, Phase, Dimension>,
        PendingOperationStartAdmission<'account>,
    ),
    OperationStartAdmissionError,
> {
    let admission = account.try_admit_operation_start()?;
    let ledger = Ledger::try_new(limits, phases)
        .unwrap_or_else(|error| panic!("test startup ledger: {error}"));
    Ok((ledger, admission))
}

fn account_actions(account: &PhysicalStorageAccount) -> u64 {
    super::super::account::lock_recovering_poison(&account.inner.value().linear)
        .admitted_account_actions
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
        ReadyStartupCrossPlane<'_, Owner, Phase, Dimension>,
        StartupCrossPlaneAdmissionFailure<Owner, Phase, Dimension>,
    >,
) -> CrossPlaneAdmissionContainment<Owner, Phase, Dimension> {
    match result {
        Err(StartupCrossPlaneAdmissionFailure::Uncertain(containment)) => containment,
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
    let factory = factory();
    let account = account(&factory, 5, 2);
    let account_before = account.snapshot();
    let shares = account.share_call_count();
    let phase_constructions = test_phase_catalog_construction_entries();
    let graph_allocations = test_startup_graph_allocation_entries();

    assert!(matches!(
        StartupCrossPlanePlan::try_new(
            ledger_request(2, 3).1,
            physical_manifest(charge(1, 1), charge(3, 1)),
        ),
        Err(StartupCrossPlanePlanError::FallbackByteMismatch {
            qualified: 2,
            physical: 1,
        })
    ));
    assert!(matches!(
        StartupCrossPlanePlan::try_new(
            ledger_request(2, 3).1,
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
    assert_eq!(account.share_call_count(), shares);
    assert_eq!(account_actions(&account), 0);
    assert_eq!(account.snapshot(), account_before);
}

#[test]
fn initialized_ledger_preflight_rejects_before_graph_shares_or_account_admission() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let (limits, parts) = ledger_request(2, 3);
    let mut ledger = Ledger::<Owner, Phase, Dimension>::try_new(limits, &[Phase::Build])
        .unwrap_or_else(|error| panic!("existing ledger: {error}"));
    ledger
        .charge_attempted(Dimension::Records, u64::MAX)
        .unwrap_or_else(|error| panic!("existing attempted work: {error}"));
    let ledger_before = ledger_observation(&ledger);
    let account_before = account.snapshot();
    let phase_constructions = test_phase_catalog_construction_entries();
    let allocations = test_startup_graph_allocation_entries();
    let shares = account.share_call_count();

    assert!(matches!(
        ledger.preflight_startup_attribution_graph(&parts),
        Err(AttributionBatchCreateError::Ledger(
            LedgerCause::ArithmeticOverflow {
                class: crate::CounterClass::Attempted,
                dimension: Dimension::Records,
            }
        ))
    ));
    assert_eq!(ledger_observation(&ledger), ledger_before);
    assert_eq!(
        test_phase_catalog_construction_entries(),
        phase_constructions
    );
    assert_eq!(test_startup_graph_allocation_entries(), allocations);
    assert_eq!(account.share_call_count(), shares);
    assert_eq!(account_actions(&account), 0);
    assert_eq!(account.snapshot(), account_before);
}

#[test]
fn poisoned_operation_gate_precedes_ledger_and_owner_work() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let _ = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
    let allocations = test_startup_graph_allocation_entries();
    let phase_constructions = test_phase_catalog_construction_entries();
    let shares = account.share_call_count();
    let before = account.snapshot();
    let (limits, _plan) = cross_plane_plan();
    reset_startup_lock_probe();

    let result = start_ledger(&account, &[Phase::Build], limits);

    assert!(matches!(
        result,
        Err(OperationStartAdmissionError::AccountPoisoned(_))
    ));
    assert_eq!(test_startup_graph_allocation_entries(), allocations);
    assert_eq!(
        test_phase_catalog_construction_entries(),
        phase_constructions
    );
    assert_eq!(account.share_call_count(), shares);
    assert_eq!(account_actions(&account), 0);
    assert_eq!(account.snapshot(), before);
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
fn ledger_rejection_precedes_all_four_owner_shares_and_physical_admission() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    reset_startup_lock_probe();
    let shares = account.share_call_count();
    let allocations = test_startup_graph_allocation_entries();
    let (limits, plan) = cross_plane_plan();
    let (mut ledger, admission) = start_ledger(&account, &[], limits)
        .unwrap_or_else(|_| panic!("start must be admitted before ledger initialization"));

    let result = admission.try_prepare_startup_cross_plane(&mut ledger, plan);

    assert!(matches!(
        result,
        Err(StartupCrossPlaneAdmissionFailure::LedgerAdmission(
            AttributionBatchCreateError::Ledger(LedgerCause::UnknownPhase)
        ))
    ));
    assert_eq!(account.share_call_count(), shares);
    assert_eq!(account_actions(&account), 1);
    assert_eq!(test_startup_graph_allocation_entries(), allocations);
    assert_eq!(ledger_observation(&ledger), (0, 0, 0, 0, 0));
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
    let (limits, plan) = cross_plane_plan();
    let (mut ledger, admission) = start_ledger(&account, &[Phase::Build], limits)
        .unwrap_or_else(|_| panic!("start must be admitted"));

    let ready = admission
        .try_prepare_startup_cross_plane(&mut ledger, plan)
        .unwrap_or_else(|_| panic!("cross-plane startup must be ready"));

    assert_eq!(account.share_call_count(), shares + 4);
    assert_eq!(account_actions(&account), 1);
    assert_eq!(test_startup_graph_allocation_entries(), allocations + 8);
    let records = ledger
        .dimension_evidence(Dimension::Records)
        .unwrap_or_else(|error| panic!("records evidence exists: {error}"));
    assert_eq!(records.attempted(), 12);
    assert_eq!(records.consumed(), 12);
    assert_eq!(ledger.test_probe().active_reservations(), 6);
    assert!(ledger.test_probe().current_live_bytes() > 5);
    assert_eq!(account.snapshot().live_retained_bytes(), 5);
    assert_eq!(account.snapshot().live_retained_allocations(), 2);
    std::mem::forget(ready);
}

#[test]
fn aggregate_plan_preserves_each_checked_physical_allocation_count() {
    let factory = factory();
    let account = account(&factory, 5, 5);
    let (limits, parts) = ledger_request(2, 3);
    let plan = StartupCrossPlanePlan::try_new(parts, physical_manifest(charge(2, 2), charge(3, 3)))
        .unwrap_or_else(|error| panic!("allocation-qualified cross-plane plan: {error:?}"));
    let (mut ledger, admission) = start_ledger(&account, &[Phase::Build], limits)
        .unwrap_or_else(|_| panic!("start must be admitted"));

    let ready = admission
        .try_prepare_startup_cross_plane(&mut ledger, plan)
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
    let (limits, plan) = cross_plane_plan();
    let (mut ledger, admission) = start_ledger(&account, &[Phase::Build], limits)
        .unwrap_or_else(|_| panic!("start must be admitted"));

    let result = admission.try_prepare_startup_cross_plane(&mut ledger, plan);

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
    assert_eq!(account_actions(&account), 1);
    assert_eq!(ledger_observation(&ledger), (0, 0, 12, 12, 0));
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
    let (limits, plan) = cross_plane_plan();
    let (mut ledger, admission) = start_ledger(&account, &[Phase::Build], limits)
        .unwrap_or_else(|_| panic!("start must be admitted"));
    set_startup_transition_panic(StartupTransitionPanicPoint::BeforeCommit);

    let containment =
        expect_uncertain(admission.try_prepare_startup_cross_plane(&mut ledger, plan));
    let ledger_before = ledger_observation(&ledger);
    assert_eq!(ledger_before.1, 6);
    assert_eq!(
        (ledger_before.2, ledger_before.3, ledger_before.4),
        (12, 12, 0)
    );
    assert_eq!(account.snapshot().poison_event_count(), 1);
    drop(containment);

    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
    assert_eq!(account.share_call_count(), 4);
    assert_eq!(account_actions(&account), 1);
    assert_eq!(ledger_observation(&ledger), ledger_before);
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
    let (limits, plan) = cross_plane_plan();
    let (mut ledger, admission) = start_ledger(&account, &[Phase::Build], limits)
        .unwrap_or_else(|_| panic!("start must be admitted"));
    set_startup_transition_panic(StartupTransitionPanicPoint::AfterCommit);

    let containment =
        expect_uncertain(admission.try_prepare_startup_cross_plane(&mut ledger, plan));
    let ledger_before = ledger_observation(&ledger);
    assert_eq!(ledger_before.1, 6);
    assert_eq!(
        (ledger_before.2, ledger_before.3, ledger_before.4),
        (12, 12, 0)
    );
    assert_eq!(account.snapshot().poison_event_count(), 1);
    drop(containment);

    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
    assert_eq!(account.share_call_count(), 4);
    assert_eq!(account_actions(&account), 1);
    assert_eq!(ledger_observation(&ledger), ledger_before);
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
    let (limits, plan) = cross_plane_plan();
    let (mut ledger, admission) = start_ledger(&account, &[Phase::Build], limits)
        .unwrap_or_else(|_| panic!("start must be admitted"));
    let ready = admission
        .try_prepare_startup_cross_plane(&mut ledger, plan)
        .unwrap_or_else(|_| panic!("cross-plane startup must be ready"));
    let ledger_before = ledger_observation(&ledger);

    drop(ready);

    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
    assert_eq!(ledger_observation(&ledger), ledger_before);
    assert_eq!(account_actions(&account), 1);
    assert_eq!(account.share_call_count(), 4);
    assert_eq!(account.snapshot().live_retained_bytes(), 5);
    assert!(account.snapshot().poisoned());
    assert_eq!(account.snapshot().poison_event_count(), 1);
}

#[test]
fn ready_split_is_state_only_and_runtime_drop_remains_fail_closed() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let (limits, plan) = cross_plane_plan();
    let (mut ledger, admission) = start_ledger(&account, &[Phase::Build], limits)
        .unwrap_or_else(|_| panic!("start must be admitted"));
    let ready = admission
        .try_prepare_startup_cross_plane(&mut ledger, plan)
        .unwrap_or_else(|_| panic!("cross-plane startup must be ready"));
    reset_startup_lock_probe();
    let allocations = test_startup_graph_allocation_entries();
    let releases = test_startup_graph_storage_releases();
    let release_locks = test_startup_graph_ledger_release_locks();
    let shares = account.share_call_count();
    let account_before = account.snapshot();
    let ledger_before = ledger_observation(&ledger);

    let (baseline, guard) = ready.split();

    assert_eq!(
        startup_lock_probe_snapshot(),
        StartupLockProbeSnapshot::default()
    );
    assert_eq!(test_startup_graph_allocation_entries(), allocations);
    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
    assert_eq!(account.share_call_count(), shares);
    assert_eq!(account_actions(&account), 1);
    assert_eq!(account.snapshot(), account_before);
    assert_eq!(ledger_observation(&ledger), ledger_before);
    drop(baseline);
    drop(guard);
    assert_eq!(ledger_observation(&ledger), (0, 0, 12, 12, 0));
    drop(ledger);
    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
    assert_eq!(account.snapshot().live_retained_bytes(), 5);
    assert!(account.snapshot().poisoned());
    assert_eq!(account.snapshot().poison_event_count(), 1);
}

#[test]
fn ready_runtime_baseline_keeps_its_original_ledger_owner() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let (limits, plan) = cross_plane_plan();
    let (mut ledger, admission) = start_ledger(&account, &[Phase::Build], limits)
        .unwrap_or_else(|_| panic!("start must be admitted"));
    let (foreign_limits, _foreign_plan) = cross_plane_plan();
    let foreign = Ledger::<Owner, Phase, Dimension>::try_new(foreign_limits, &[Phase::Build])
        .unwrap_or_else(|error| panic!("foreign ledger: {error}"));
    let ready = admission
        .try_prepare_startup_cross_plane(&mut ledger, plan)
        .unwrap_or_else(|_| panic!("cross-plane startup must be ready"));
    let ledger_before = ledger_observation(&ledger);
    let foreign_before = ledger_observation(&foreign);
    let account_before = account.snapshot();
    let (mut baseline, guard) = ready.split();

    assert!(matches!(
        foreign.try_bind_fallback_baseline(&mut baseline),
        Err(LedgerCause::OwnerMismatch)
    ));
    ledger
        .try_bind_fallback_baseline(&mut baseline)
        .unwrap_or_else(|error| panic!("original ledger owns the runtime baseline: {error}"));

    assert_eq!(ledger_observation(&ledger), ledger_before);
    assert_eq!(ledger_observation(&foreign), foreign_before);
    assert_eq!(account.snapshot(), account_before);
    assert_eq!(account.share_call_count(), 4);
    assert_eq!(account_actions(&account), 1);
    drop(baseline);
    assert_eq!(ledger_observation(&ledger), (0, 0, 12, 12, 0));
    drop(guard);
    assert_eq!(account.snapshot().live_retained_bytes(), 5);
    assert_eq!(account.snapshot().live_retained_allocations(), 2);
    assert_eq!(account.snapshot().poison_event_count(), 1);
}

#[test]
fn existing_ledger_work_and_live_scratch_survive_startup_and_runtime_split() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let (limits, plan) = cross_plane_plan();
    let (mut ledger, admission) = start_ledger(&account, &[Phase::Build], limits)
        .unwrap_or_else(|_| panic!("start must be admitted"));
    ledger
        .charge_attempted(Dimension::Records, 7)
        .unwrap_or_else(|error| panic!("existing attempted work: {error}"));
    ledger
        .charge_consumed(Dimension::Records, 3)
        .unwrap_or_else(|error| panic!("existing consumed work: {error}"));
    let scratch = ledger
        .reserve(Phase::Build, Dimension::Scratch, 11)
        .unwrap_or_else(|error| panic!("existing live scratch: {error}"));
    assert_eq!(ledger_observation(&ledger), (11, 1, 7, 3, 0));

    let ready = admission
        .try_prepare_startup_cross_plane(&mut ledger, plan)
        .unwrap_or_else(|_| panic!("startup must use the existing ledger"));

    let ledger_after = ledger_observation(&ledger);
    assert!(ledger_after.0 > 16);
    assert_eq!(
        (
            ledger_after.1,
            ledger_after.2,
            ledger_after.3,
            ledger_after.4
        ),
        (7, 19, 15, 0)
    );
    assert_eq!(
        ledger
            .dimension_evidence(Dimension::Scratch)
            .unwrap_or_else(|error| panic!("scratch evidence: {error}"))
            .current_live_bytes(),
        11
    );
    assert_eq!(account.share_call_count(), 4);
    assert_eq!(account_actions(&account), 1);
    let (baseline, guard) = ready.split();
    assert_eq!(ledger_observation(&ledger), ledger_after);
    drop(baseline);
    assert_eq!(ledger_observation(&ledger), (11, 1, 19, 15, 0));
    drop(scratch);
    assert_eq!(ledger_observation(&ledger), (0, 0, 19, 15, 0));
    drop(guard);
    assert_eq!(account.snapshot().live_retained_bytes(), 5);
    assert_eq!(account.snapshot().live_retained_allocations(), 2);
    assert_eq!(account.snapshot().poison_event_count(), 1);
}

#[test]
fn foreign_ledger_prehold_rejects_before_work_shares_or_physical_mutation() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let (limits, plan) = cross_plane_plan();
    reset_startup_lock_probe();
    let (mut ledger, admission) = start_ledger(&account, &[Phase::Build], limits)
        .unwrap_or_else(|_| panic!("start must be admitted"));
    ledger
        .charge_observed_batch(&[(Dimension::Records, 7)])
        .unwrap_or_else(|error| panic!("existing source work: {error}"));
    let scratch = ledger
        .reserve(Phase::Build, Dimension::Scratch, 11)
        .unwrap_or_else(|error| panic!("existing source scratch: {error}"));
    let (foreign_limits, _foreign_plan) = cross_plane_plan();
    let mut foreign = Ledger::<Owner, Phase, Dimension>::try_new(foreign_limits, &[Phase::Build])
        .unwrap_or_else(|error| panic!("foreign ledger: {error}"));
    foreign
        .charge_observed_batch(&[(Dimension::Records, 9)])
        .unwrap_or_else(|error| panic!("existing foreign work: {error}"));
    let foreign_scratch = foreign
        .reserve(Phase::Build, Dimension::Scratch, 13)
        .unwrap_or_else(|error| panic!("existing foreign scratch: {error}"));
    let ledger_before = ledger_observation(&ledger);
    let foreign_before = ledger_observation(&foreign);
    let account_before = account.snapshot();
    let allocations = test_startup_graph_allocation_entries();
    let releases = test_startup_graph_storage_releases();
    let release_locks = test_startup_graph_ledger_release_locks();
    let StartupCrossPlanePlan {
        ledger: parts,
        physical: manifest,
    } = plan;
    let admitted = ledger
        .try_bind_startup_attribution_graph(parts)
        .unwrap_or_else(|error| panic!("source ledger graph: {error:?}"));
    assert!(admitted.graph.belongs_to(&ledger));
    assert!(!admitted.graph.belongs_to(&foreign));

    assert!(matches!(
        (StartupGraphPreholdAuthority { admitted, manifest })
            .try_commit_physical(admission, &mut foreign),
        Err(StartupCrossPlaneAdmissionFailure::OwnerShare(
            RetainedWriterError::OwnershipUnverified
        ))
    ));

    assert_eq!(ledger_observation(&ledger), ledger_before);
    assert_eq!(ledger_observation(&foreign), foreign_before);
    assert_eq!(account.snapshot(), account_before);
    assert_eq!(account.share_call_count(), 0);
    assert_eq!(account_actions(&account), 1);
    assert_eq!(test_startup_graph_allocation_entries(), allocations + 8);
    assert_eq!(test_startup_graph_storage_releases(), releases + 8);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks + 1);
    assert_eq!(
        startup_lock_probe_snapshot(),
        StartupLockProbeSnapshot {
            operation_gate_calls: 1,
            physical_transition_calls: 0,
            current_depth: 0,
            max_depth: 1,
        }
    );
    let probe = ledger.test_probe();
    let foreign_probe = foreign.test_probe();
    drop(scratch);
    drop(foreign_scratch);
    assert_eq!(ledger_observation(&ledger), (0, 0, 7, 7, 0));
    assert_eq!(ledger_observation(&foreign), (0, 0, 9, 9, 0));
    drop(ledger);
    drop(foreign);
    assert!(!probe.is_available());
    assert!(!foreign_probe.is_available());
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 0);
}

#[test]
fn forgetting_ready_keeps_the_numeric_host_separate_from_contained_storage() {
    let factory = factory();
    let account = account(&factory, 5, 2);
    let (limits, plan) = cross_plane_plan();
    let (mut ledger, admission) = start_ledger(&account, &[Phase::Build], limits)
        .unwrap_or_else(|_| panic!("start must be admitted"));
    ledger
        .charge_observed_batch(&[(Dimension::Records, 7)])
        .unwrap_or_else(|error| panic!("existing invocation work: {error}"));
    let scratch = ledger
        .reserve(Phase::Build, Dimension::Scratch, 11)
        .unwrap_or_else(|error| panic!("existing live scratch: {error}"));
    let releases = test_startup_graph_storage_releases();
    let release_locks = test_startup_graph_ledger_release_locks();
    let ready = admission
        .try_prepare_startup_cross_plane(&mut ledger, plan)
        .unwrap_or_else(|_| panic!("startup must use the existing numeric host"));
    let ledger_before = ledger_observation(&ledger);
    let account_before = account.snapshot();
    let probe = ledger.test_probe();

    std::mem::forget(ready);

    assert_eq!(ledger_observation(&ledger), ledger_before);
    ledger
        .charge_observed_batch(&[(Dimension::Records, 5)])
        .unwrap_or_else(|error| panic!("outer numeric host remains usable: {error}"));
    assert_eq!(ledger_observation(&ledger), (ledger_before.0, 7, 24, 24, 0));
    drop(scratch);
    assert_eq!(
        ledger_observation(&ledger),
        (ledger_before.0 - 11, 6, 24, 24, 0)
    );
    ledger
        .seal_ordinary_work()
        .unwrap_or_else(|error| panic!("outer invocation work can seal: {error}"));
    ledger
        .close_work()
        .unwrap_or_else(|error| panic!("outer invocation work can close: {error}"));
    drop(ledger);

    assert!(probe.is_available());
    assert_eq!(probe.current_live_bytes(), ledger_before.0 - 11);
    assert_eq!(probe.active_reservations(), 6);
    assert_eq!(test_startup_graph_storage_releases(), releases);
    assert_eq!(test_startup_graph_ledger_release_locks(), release_locks);
    assert_eq!(account.snapshot(), account_before);
    assert_eq!(account.snapshot().live_retained_bytes(), 5);
    assert_eq!(account.snapshot().live_retained_allocations(), 2);
    assert_eq!(account.share_call_count(), 4);
    assert_eq!(account_actions(&account), 1);
    assert!(!account.snapshot().poisoned());
    drop(account);
    assert_eq!(factory.snapshot().live_accounts(), 1);
}
