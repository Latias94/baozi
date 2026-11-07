use super::*;
use crate::ledger::finite::{FiniteLedger, FiniteLedgerLimits};

#[derive(Debug)]
struct Owner;
impl OperationOwner for Owner {}

type TestLedger = FiniteLedger<Owner, u8, u8>;
type Baseline = FiniteFallbackBatchBaseline<Owner, u8, u8>;

fn policy(canonical: u64, evidence: u64) -> DimensionPolicy {
    DimensionPolicy::reservable_bytes(64, 64, canonical, 16_384)
        .with_published_outcome_evidence_limit(evidence)
}

fn ledger() -> TestLedger {
    FiniteLedger::try_new(
        FiniteLedgerLimits::try_new(
            16_384,
            16_384,
            [6, 2, 4, 1, 5, 3].map(|dimension| (dimension, policy(64, 64))),
        )
        .unwrap(),
        [3, 1, 2],
    )
    .unwrap()
}

fn batch<Destination: PublicationDestination>(
    ledger: &mut TestLedger,
    attributions: usize,
    commitments: usize,
) -> AttributionReservationBatch<Owner, u8, u8, Destination> {
    ledger
        .view()
        .try_attribution_batch_from_plan(
            AttributionBatchPlan::try_new(3, 1, attributions, commitments).unwrap(),
        )
        .unwrap()
}

fn attributed<Destination: PublicationDestination>(
    ledger: &mut TestLedger,
    bytes: u64,
    amount: u64,
) -> AttributionReservationBatch<Owner, u8, u8, Destination> {
    let mut batch = batch(ledger, 1, 1);
    let attribution = {
        let (work, mut storage) = ledger.view().into_exclusive().unwrap();
        work.reserve_operation_attribution::<Destination, _, _>(&mut storage, 3, 1, bytes)
            .unwrap()
    };
    assert!(batch.try_push_attribution(attribution).is_ok());
    assert!(
        ledger
            .as_read()
            .try_push_commitment(&mut batch, OutputCommitment::new(1, amount))
            .is_ok()
    );
    batch
}

fn baseline(ledger: &mut TestLedger, bytes: u64) -> Baseline {
    let mut canonical = batch(ledger, 1, 2);
    let evidence = batch(ledger, 1, 2);
    let fallback = attributed(ledger, bytes, 3);
    assert!(
        ledger
            .as_read()
            .try_push_commitment(&mut canonical, OutputCommitment::new(1, 5))
            .is_ok()
    );
    Baseline::new_unbound(canonical, evidence, fallback)
}

#[derive(Debug, PartialEq, Eq)]
struct Counters {
    dimensions: [DimensionState; 6],
    phases: [(u64, u64, u64); 3],
    totals: [u64; 8],
}

fn counters(ledger: &TestLedger) -> Counters {
    let selected = ledger.storage.read();
    let state = selected.view();
    Counters {
        dimensions: std::array::from_fn(|position| {
            DimensionState::join(
                ledger.work[position].state,
                state.dimensions[position].state,
            )
        }),
        phases: std::array::from_fn(|position| {
            let phase = state.phase_states[position];
            (
                phase.current_live_bytes,
                phase.peak_live_bytes,
                phase.retained_output_bytes,
            )
        }),
        totals: [
            state.current_live_bytes,
            state.peak_live_bytes,
            state.active_reservations,
            state.retained_output_bytes,
            state.retained_output_reservations,
            state.evidence_byte_limit,
            state.current_administrative_bytes,
            state.peak_administrative_bytes,
        ],
    }
}

#[test]
fn finite_attachment_keeps_the_real_startup_graph_and_counts_zero_byte_tokens() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger, 0);
    let (graph, _shares) = ledger
        .view()
        .try_bind_finite_startup_attribution_graph(FiniteStartupLedgerPlanParts {
            canonical_assembly: AttributionBatchPlan::try_new(3, 1, 0, 0).unwrap(),
            evidence_assembly: AttributionBatchPlan::try_new(3, 1, 1, 1).unwrap(),
            fallback: AttributionBatchPlan::try_new(3, 1, 1, 1).unwrap(),
            fallback_attribution: StartupRetainedAttribution::new(2, 3, 5),
            account_shares: StartupAccountShareEnvelope::three_identical([(2, 1), (2, 1)]),
            fallback_commitment: OutputCommitment::new(1, 3),
        })
        .unwrap();
    let zero = ledger.view().reserve(1, 1, 0).unwrap();
    assert!(graph.belongs_to_view(&ledger.as_read()));
    assert!(baseline.as_read().numerical_evidence.is_none());
    assert!(baseline.view().numerical_evidence.is_none());
    assert_eq!(baseline.as_read().fallback.attribution_len(), 1);
    assert_eq!(baseline.as_read().fallback.attributed_bytes(), Some(0));
    let before = counters(&ledger);
    ledger
        .as_read()
        .try_attach_fallback_rows(baseline.view())
        .unwrap();
    assert_eq!(
        ledger.as_read().try_bind_fallback_rows(baseline.view()),
        Err(LedgerCause::ActiveReservationsRemain {
            count: before.totals[2]
        })
    );
    assert_eq!(counters(&ledger), before);
    drop(graph);
    let before = counters(&ledger);
    ledger
        .as_read()
        .try_attach_fallback_rows(baseline.view())
        .unwrap();
    assert_eq!(
        ledger.as_read().try_bind_fallback_rows(baseline.view()),
        Err(LedgerCause::ActiveReservationsRemain {
            count: before.totals[2]
        })
    );
    assert_eq!(counters(&ledger), before);
    drop(zero);
    ledger
        .as_read()
        .try_bind_fallback_rows(baseline.view())
        .unwrap();
    drop(baseline);
    assert_eq!(ledger.as_read().terminal_residue_cause(), None);
}

#[test]
fn foreign_finite_baseline_rejection_preserves_both_real_owners() {
    let mut owner = ledger();
    let foreign = ledger();
    let mut baseline = baseline(&mut owner, 7);
    let before = (counters(&owner), counters(&foreign));
    for rejected in [
        foreign.as_read().try_attach_fallback_rows(baseline.view()),
        foreign.as_read().try_bind_fallback_rows(baseline.view()),
        foreign
            .as_read()
            .try_bind_primary_rows(baseline.view(), None, None, true),
        foreign
            .as_read()
            .try_bind_fallback_discard_rows(baseline.view(), None, None),
    ] {
        assert_eq!(rejected, Err(LedgerCause::OwnerMismatch));
    }
    assert_eq!((counters(&owner), counters(&foreign)), before);
    assert_eq!(
        baseline.as_read().fallback.attributions[0]
            .core
            .phase_position,
        0
    );
    assert_eq!(
        baseline.as_read().fallback.commitments[0].dimension_position,
        0
    );
    owner
        .as_read()
        .try_bind_fallback_rows(baseline.view())
        .unwrap();
}

#[test]
fn finite_attachment_and_exact_binding_recheck_current_limits() {
    for (class, canonical, evidence, value, limit) in [
        (CounterClass::Committed, 4, 64, 5, 4),
        (CounterClass::PublishedOutcomeEvidence, 64, 2, 3, 2),
    ] {
        let mut ledger = ledger();
        let mut baseline = baseline(&mut ledger, 7);
        ledger
            .as_read()
            .try_attach_fallback_rows(baseline.view())
            .unwrap();
        ledger
            .view()
            .tighten_dimension_policy(1, policy(canonical, evidence))
            .unwrap();
        let before = counters(&ledger);
        let expected = Err(LedgerCause::LimitExceeded {
            class,
            dimension: 1,
            value,
            limit,
        });
        assert_eq!(
            ledger.as_read().try_attach_fallback_rows(baseline.view()),
            expected
        );
        assert_eq!(
            ledger.as_read().try_bind_fallback_rows(baseline.view()),
            expected
        );
        assert_eq!(counters(&ledger), before);
        assert_eq!(
            baseline.as_read().canonical_assembly.commitments[0].dimension_position,
            0
        );
        assert_eq!(baseline.as_read().fallback.attribution_len(), 1);
        assert_eq!(
            baseline.as_read().fallback.commitments[0].dimension_position,
            0
        );
    }
}

#[test]
fn finite_binding_rejects_bound_position_faults_without_retiring_any_owner() {
    for fault in 0..3 {
        let mut ledger = ledger();
        let mut baseline = baseline(&mut ledger, 7);
        let before = counters(&ledger);
        let previous = match fault {
            0 => std::mem::replace(
                &mut baseline.view().fallback.attributions[0].core.phase_position,
                usize::MAX,
            ),
            1 => std::mem::replace(
                &mut baseline
                    .view()
                    .canonical_assembly
                    .capacity_cover
                    .as_mut()
                    .unwrap()
                    .core
                    .dimension_position,
                usize::MAX,
            ),
            _ => std::mem::replace(
                &mut baseline.view().fallback.commitments[0].dimension_position,
                usize::MAX,
            ),
        };
        let attached = ledger.as_read().try_attach_fallback_rows(baseline.view());
        let bound = ledger.as_read().try_bind_fallback_rows(baseline.view());
        match fault {
            0 => baseline.view().fallback.attributions[0].core.phase_position = previous,
            1 => {
                baseline
                    .view()
                    .canonical_assembly
                    .capacity_cover
                    .as_mut()
                    .unwrap()
                    .core
                    .dimension_position = previous
            }
            _ => baseline.view().fallback.commitments[0].dimension_position = previous,
        }
        assert_eq!(attached, Err(LedgerCause::FinalSnapshotShapeMismatch));
        assert_eq!(bound, Err(LedgerCause::FinalSnapshotShapeMismatch));
        assert_eq!(counters(&ledger), before);
        assert_eq!(baseline.as_read().fallback.attribution_len(), 1);
        ledger
            .as_read()
            .try_bind_fallback_rows(baseline.view())
            .unwrap();
    }
}

#[test]
fn finite_primary_binding_moves_only_selected_rows_and_preserves_storage_covers() {
    for publish_canonical in [false, true] {
        let mut ledger = ledger();
        let mut baseline = baseline(&mut ledger, 7);
        let mut canonical = attributed(&mut ledger, 13, 3);
        let mut evidence = attributed(&mut ledger, 17, 2);
        let covers = (
            canonical.account_capacity_bytes(),
            evidence.account_capacity_bytes(),
        );
        let before = counters(&ledger);
        ledger
            .as_read()
            .try_bind_primary_rows(
                baseline.view(),
                Some(&mut canonical),
                Some(&mut evidence),
                publish_canonical,
            )
            .unwrap();
        assert_eq!(counters(&ledger), before);
        assert_eq!(canonical.is_empty_storage_shell(), publish_canonical);
        assert!(evidence.is_empty_storage_shell());
        assert_eq!(
            (
                canonical.account_capacity_bytes(),
                evidence.account_capacity_bytes()
            ),
            covers
        );
        let rows = baseline.as_read();
        assert!(rows.numerical_evidence.is_none());
        assert_eq!(
            rows.canonical_assembly.attribution_len(),
            usize::from(publish_canonical)
        );
        assert_eq!(
            rows.canonical_assembly.commitment_len(),
            1 + usize::from(publish_canonical)
        );
        assert_eq!(rows.evidence_assembly.attribution_len(), 1);
        assert_eq!(rows.evidence_assembly.commitments[0].dimension_position, 0);
        assert_eq!(rows.fallback.attribution_len(), 1);
        drop((baseline, canonical, evidence));
        assert_eq!(ledger.as_read().terminal_residue_cause(), None);
    }
}

#[test]
fn finite_fallback_discards_check_real_tokens_before_absorbing_the_single_fallback() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger, 7);
    let mut canonical = attributed(&mut ledger, 13, 3);
    let mut evidence = attributed(&mut ledger, 17, 2);
    let backing = baseline.as_read().evidence_assembly.attributions.as_ptr();
    let fallback_cover = baseline.as_read().fallback.account_capacity_bytes();
    canonical.commitments[0].dimension_position = usize::MAX;
    let previous = std::mem::replace(
        &mut canonical
            .capacity_cover
            .as_mut()
            .unwrap()
            .core
            .phase_position,
        3,
    );
    let before = counters(&ledger);
    let rejected = ledger.as_read().try_bind_fallback_discard_rows(
        baseline.view(),
        Some(&mut canonical),
        Some(&mut evidence),
    );
    canonical
        .capacity_cover
        .as_mut()
        .unwrap()
        .core
        .phase_position = previous;
    assert_eq!(rejected, Err(LedgerCause::FinalSnapshotShapeMismatch));
    assert_eq!(counters(&ledger), before);
    assert_eq!(baseline.as_read().evidence_assembly.attribution_len(), 0);
    assert_eq!(baseline.as_read().fallback.attribution_len(), 1);
    ledger
        .as_read()
        .try_bind_fallback_discard_rows(baseline.view(), Some(&mut canonical), Some(&mut evidence))
        .unwrap();
    assert_eq!(counters(&ledger), before);
    let rows = baseline.as_read();
    assert!(rows.numerical_evidence.is_none());
    assert!(rows.fallback.is_empty_storage_shell());
    assert_eq!(rows.fallback.account_capacity_bytes(), fallback_cover);
    assert_eq!(rows.evidence_assembly.attributions.as_ptr(), backing);
    assert_eq!(rows.evidence_assembly.attribution_len(), 1);
    assert_eq!(rows.evidence_assembly.commitments[0].dimension_position, 0);
    assert_eq!(
        (canonical.attribution_len(), evidence.attribution_len()),
        (1, 1)
    );
    assert_eq!(canonical.commitments[0].dimension_position, usize::MAX);
    drop((baseline, canonical, evidence));
    assert_eq!(ledger.as_read().terminal_residue_cause(), None);
}
