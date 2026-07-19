use super::super::{
    AttributionBatchPlan, CounterClass, LedgerCause, LedgerLimits, OutcomeEvidenceDestination,
    OutputCommitment, test_attribution_batch_allocation_entries,
};
use super::*;
use crate::{DimensionPolicy, OperationOwner};

#[derive(Debug)]
struct Owner;

impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Build,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Dimension {
    Bytes,
    Records,
    Unknown,
}

#[test]
fn startup_qualification_has_one_production_module() {
    let ledger_root = include_str!("../../ledger.rs");
    let startup = include_str!("../startup.rs");

    assert!(ledger_root.contains("mod startup;"));
    assert!(ledger_root.contains("pub(crate) use startup::{"));
    assert!(!ledger_root.contains("pub(crate) struct StartupLedgerPlan<"));
    assert!(startup.contains("pub(crate) struct StartupLedgerPlan<"));
    assert!(startup.contains("pub(crate) fn try_qualify_startup<"));
}

fn startup_limits(
    operation_live: u64,
    dimension_live: u64,
    attempted: u64,
    consumed: u64,
    published_evidence: u64,
) -> LedgerLimits<Dimension> {
    LedgerLimits::new(
        operation_live,
        [
            (
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(attempted, consumed, u64::MAX, dimension_live)
                    .with_published_outcome_evidence_limit(published_evidence),
            ),
            (
                Dimension::Records,
                DimensionPolicy::counter(attempted, consumed, u64::MAX)
                    .with_published_outcome_evidence_limit(published_evidence),
            ),
        ],
    )
    .expect("startup test policy")
}

fn startup_batch_plans(
    capacity_dimension: Dimension,
) -> StartupAttributionBatchPlans<Owner, Phase, Dimension> {
    StartupAttributionBatchPlans::new(
        AttributionBatchPlan::try_new(Phase::Build, capacity_dimension, 2, 1)
            .expect("canonical assembly plan"),
        AttributionBatchPlan::try_new(Phase::Build, capacity_dimension, 2, 2)
            .expect("evidence assembly plan"),
        AttributionBatchPlan::try_new(Phase::Build, capacity_dimension, 1, 1)
            .expect("fallback plan"),
        AttributionBatchPlan::try_new(Phase::Build, capacity_dimension, 1, 1)
            .expect("evidence plan"),
    )
}

fn startup_batch_bytes(batches: &StartupAttributionBatchPlans<Owner, Phase, Dimension>) -> u64 {
    [
        batches.canonical_assembly.planned_bytes,
        batches.evidence_assembly.planned_bytes,
        batches.fallback.planned_bytes,
        batches.evidence.planned_bytes,
    ]
    .into_iter()
    .try_fold(0_u64, u64::checked_add)
    .expect("test batch envelope")
}

fn startup_attributions(
    first_dimension: Dimension,
    second_dimension: Dimension,
) -> [StartupRetainedAttribution<Phase, Dimension>; 2] {
    [
        StartupRetainedAttribution::new(Phase::Build, first_dimension, 3),
        StartupRetainedAttribution::new(Phase::Build, second_dimension, 5),
    ]
}

fn startup_share_charges() -> [(Dimension, u64); 2] {
    [(Dimension::Records, 1), (Dimension::Records, 2)]
}

fn startup_account_shares() -> StartupAccountShareEnvelope<Dimension> {
    StartupAccountShareEnvelope::four_identical(startup_share_charges())
}

fn startup_commitments(
    first_dimension: Dimension,
    second_dimension: Dimension,
) -> [OutputCommitment<Dimension, OutcomeEvidenceDestination>; 2] {
    [
        OutputCommitment::new(first_dimension, 4),
        OutputCommitment::new(second_dimension, 6),
    ]
}

#[test]
fn startup_qualification_preserves_the_exact_typed_envelope() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let batch_bytes = startup_batch_bytes(&batches);
    let retained_attributions = startup_attributions(Dimension::Bytes, Dimension::Bytes);
    let total_live = batch_bytes.checked_add(8).expect("test live envelope");
    let limits = startup_limits(total_live, total_live, 12, 12, 10);

    let qualified = limits
        .try_qualify_startup(
            batches,
            retained_attributions,
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        )
        .expect("the exact startup envelope must qualify");
    let (limits, parts) = qualified.into_parts();

    assert_eq!(startup_batch_bytes(&parts.batches), batch_bytes);
    assert_eq!(parts.retained_attributions, retained_attributions);
    let (phase, dimension, bytes) = parts.retained_attributions[0].into_parts();
    assert_eq!(
        (phase, dimension, bytes),
        (Phase::Build, Dimension::Bytes, 3)
    );
    let (_canonical, _evidence_assembly, _fallback, _evidence) = parts.batches.into_parts();
    for share in parts.account_shares.into_steps() {
        assert_eq!(share.into_charges(), startup_share_charges());
    }
    assert_eq!(parts.commitments[0].dimension(), Dimension::Records);
    assert_eq!(parts.commitments[0].amount(), 4);
    assert_eq!(parts.commitments[1].dimension(), Dimension::Records);
    assert_eq!(parts.commitments[1].amount(), 6);
    assert_eq!(limits.dimension_count(), 2);
}

#[test]
fn startup_live_and_observed_work_are_independent_on_the_same_dimension() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let total_live = startup_batch_bytes(&batches)
        .checked_add(8)
        .expect("test live envelope");
    let limits = startup_limits(total_live, total_live, 12, 12, 10);
    let account_shares =
        StartupAccountShareEnvelope::four_identical([(Dimension::Bytes, 1), (Dimension::Bytes, 2)]);

    assert!(
        limits
            .try_qualify_startup(
                batches,
                startup_attributions(Dimension::Bytes, Dimension::Bytes),
                account_shares,
                startup_commitments(Dimension::Bytes, Dimension::Bytes),
            )
            .is_ok(),
        "live reservations and observed work use independent ledger counter classes"
    );
}

#[test]
fn startup_qualification_rejects_unknown_dimensions_without_allocation() {
    let allocations_before = test_attribution_batch_allocation_entries();

    assert!(matches!(
        startup_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).try_qualify_startup(
            startup_batch_plans(Dimension::Unknown),
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    ));
    assert!(matches!(
        startup_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).try_qualify_startup(
            startup_batch_plans(Dimension::Bytes),
            startup_attributions(Dimension::Unknown, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    ));
    assert!(matches!(
        startup_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).try_qualify_startup(
            startup_batch_plans(Dimension::Bytes),
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            StartupAccountShareEnvelope::four_identical([
                (Dimension::Unknown, 1),
                (Dimension::Records, 2),
            ]),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    ));
    assert!(matches!(
        startup_limits(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).try_qualify_startup(
            startup_batch_plans(Dimension::Bytes),
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Unknown, Dimension::Records),
        ),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    ));

    assert_eq!(
        test_attribution_batch_allocation_entries(),
        allocations_before,
        "qualification rejection must not allocate a batch"
    );
}

#[test]
fn startup_qualification_checks_live_dimension_and_operation_limits() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let total_live = startup_batch_bytes(&batches)
        .checked_add(8)
        .expect("test live envelope");
    let dimension_limited = startup_limits(
        u64::MAX,
        total_live.checked_sub(1).expect("nonzero envelope"),
        12,
        12,
        10,
    );
    assert!(matches!(
        dimension_limited.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
            value,
            limit,
        }) if value == total_live && limit == total_live - 1
    ));

    let batches = startup_batch_plans(Dimension::Bytes);
    let operation_limited = startup_limits(
        total_live.checked_sub(1).expect("nonzero envelope"),
        total_live,
        12,
        12,
        10,
    );
    assert!(matches!(
        operation_limited.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
            value,
            limit,
        }) if value == total_live && limit == total_live - 1
    ));

    let non_reservable = startup_limits(u64::MAX, u64::MAX, 12, 12, 10);
    assert!(matches!(
        non_reservable.try_qualify_startup(
            startup_batch_plans(Dimension::Records),
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::DimensionNotReservable {
            dimension: Dimension::Records,
        })
    ));

    let live_overflow = startup_limits(u64::MAX, u64::MAX, 12, 12, 10);
    assert!(matches!(
        live_overflow.try_qualify_startup(
            startup_batch_plans(Dimension::Bytes),
            [
                StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, u64::MAX,),
                StartupRetainedAttribution::new(Phase::Build, Dimension::Bytes, 0),
            ],
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Live,
            dimension: Dimension::Bytes,
        })
    ));
}

#[test]
fn startup_qualification_aggregates_four_account_shares() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let total_live = startup_batch_bytes(&batches)
        .checked_add(8)
        .expect("test live envelope");
    let attempted_limited = startup_limits(total_live, total_live, 11, 11, 10);
    assert!(matches!(
        attempted_limited.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
            value: 12,
            limit: 11,
        })
    ));

    let batches = startup_batch_plans(Dimension::Bytes);
    let consumed_limited = startup_limits(total_live, total_live, 12, 11, 10);
    assert!(matches!(
        consumed_limited.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: Dimension::Records,
            value: 12,
            limit: 11,
        })
    ));

    let batches = startup_batch_plans(Dimension::Bytes);
    let share_overflow = startup_limits(total_live, total_live, u64::MAX, u64::MAX, 10);
    assert!(matches!(
        share_overflow.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            StartupAccountShareEnvelope::four_identical([
                (Dimension::Records, u64::MAX),
                (Dimension::Records, 0),
            ]),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: Dimension::Records,
        })
    ));
}

#[test]
fn startup_qualification_checked_sums_outcome_evidence_commitments() {
    let batches = startup_batch_plans(Dimension::Bytes);
    let total_live = startup_batch_bytes(&batches)
        .checked_add(8)
        .expect("test live envelope");
    let evidence_limited = startup_limits(total_live, total_live, 12, 12, 9);
    assert!(matches!(
        evidence_limited.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            startup_commitments(Dimension::Records, Dimension::Records),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::PublishedOutcomeEvidence,
            dimension: Dimension::Records,
            value: 10,
            limit: 9,
        })
    ));

    let batches = startup_batch_plans(Dimension::Bytes);
    let overflow_limits = startup_limits(total_live, total_live, 12, 12, u64::MAX);
    assert!(matches!(
        overflow_limits.try_qualify_startup(
            batches,
            startup_attributions(Dimension::Bytes, Dimension::Bytes),
            startup_account_shares(),
            [
                OutputCommitment::new(Dimension::Records, u64::MAX),
                OutputCommitment::new(Dimension::Records, 1),
            ],
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::PublishedOutcomeEvidence,
            dimension: Dimension::Records,
        })
    ));
}
