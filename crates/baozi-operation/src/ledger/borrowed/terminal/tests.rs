use super::*;
use crate::ledger::finite::{FiniteLedger, FiniteLedgerLimits};
use crate::ledger::storage::finite::FiniteStorage;

#[derive(Debug)]
struct Owner;

impl OperationOwner for Owner {}

type TestLedger = FiniteLedger<Owner, u8, u8>;

fn policy(committed: u64, evidence: u64) -> DimensionPolicy {
    DimensionPolicy::reservable_bytes(64, 64, committed, 16_384)
        .with_published_outcome_evidence_limit(evidence)
}

fn ledger() -> TestLedger {
    FiniteLedger::try_new(
        FiniteLedgerLimits::try_new(
            16_384,
            16_384,
            [1_u8, 2, 3, 4, 5, 6].map(|dimension| (dimension, policy(64, 64))),
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

fn attributed_batch<Destination: PublicationDestination>(
    ledger: &mut TestLedger,
    bytes: u64,
) -> AttributionReservationBatch<Owner, u8, u8, Destination> {
    let mut batch = batch(ledger, 1, 1);
    let attribution = {
        let (work, mut storage) = ledger.view().into_exclusive().unwrap();
        work.reserve_operation_attribution::<Destination, _, _>(&mut storage, 3, 1, bytes)
            .unwrap()
    };
    assert!(batch.try_push_attribution(attribution).is_ok());
    batch
}

fn empty_baseline(ledger: &mut TestLedger) -> FallbackBatchBaseline<Owner, u8, u8> {
    FallbackBatchBaseline::new_unbound(
        batch(ledger, 0, 0),
        batch(ledger, 0, 0),
        batch(ledger, 0, 0),
        batch(ledger, 0, 0),
    )
}

fn baseline(ledger: &mut TestLedger) -> FallbackBatchBaseline<Owner, u8, u8> {
    let mut canonical = batch(ledger, 1, 3);
    let evidence_assembly = batch(ledger, 2, 3);
    let mut fallback = attributed_batch(ledger, 7);
    let mut evidence = attributed_batch(ledger, 11);
    assert!(
        ledger
            .as_read()
            .try_push_commitment(&mut canonical, OutputCommitment::new(1, 5))
            .is_ok()
    );
    for (batch, amount) in [(&mut fallback, 3), (&mut evidence, 4)] {
        assert!(
            ledger
                .as_read()
                .try_push_commitment(batch, OutputCommitment::new(1, amount))
                .is_ok()
        );
    }
    FallbackBatchBaseline::new_unbound(canonical, evidence_assembly, fallback, evidence)
}

#[derive(Debug, PartialEq, Eq)]
struct Observation {
    dimensions: [DimensionState; 6],
    phases: [(u64, u64, u64); 3],
    totals: (u64, u64, u64, u64, u64),
    administration: (u64, u64),
    publication: PublicationState,
    completed: Option<CompletedPhase<u8>>,
    work_mode: WorkMode,
    open_envelopes: u64,
}

fn observe(ledger: &TestLedger) -> Observation {
    let selected = ledger.storage.read();
    let state = selected.view();
    Observation {
        dimensions: std::array::from_fn(|index| {
            DimensionState::join(ledger.work[index].state, state.dimensions[index].state)
        }),
        phases: std::array::from_fn(|index| {
            let phase = state.phase_states[index];
            (
                phase.current_live_bytes,
                phase.peak_live_bytes,
                phase.retained_output_bytes,
            )
        }),
        totals: (
            state.current_live_bytes,
            state.peak_live_bytes,
            state.active_reservations,
            state.retained_output_bytes,
            state.retained_output_reservations,
        ),
        administration: (
            state.current_administrative_bytes,
            state.peak_administrative_bytes,
        ),
        publication: state.publication,
        completed: state.last_completed_phase,
        work_mode: ledger.work_header.mode,
        open_envelopes: ledger.work_header.open_envelopes,
    }
}

#[test]
fn tokenless_terminal_bindings_use_the_actual_inline_and_promoted_identity() {
    for promote in [false, true] {
        let mut owner = ledger();
        let foreign = ledger();
        let mut baseline = empty_baseline(&mut owner);
        if promote {
            drop(owner.view().reserve(1, 1, 0).unwrap());
        }
        let before = observe(&owner);
        assert_eq!(owner.storage.shared().is_some(), promote);
        let read = owner.as_read();
        read.try_attach_fallback_rows(baseline.view()).unwrap();
        read.try_bind_fallback_rows(baseline.view()).unwrap();
        read.validate_typed_batch_pair(&baseline.fallback, &baseline.evidence)
            .unwrap();
        for rejected in [
            foreign.as_read().try_attach_fallback_rows(baseline.view()),
            foreign.as_read().try_bind_fallback_rows(baseline.view()),
            foreign
                .as_read()
                .try_bind_primary_rows(baseline.view(), None, None, true),
            foreign
                .as_read()
                .try_bind_fallback_discard_rows(baseline.view(), None, None),
            foreign
                .as_read()
                .validate_typed_batch_pair(&baseline.fallback, &baseline.evidence),
        ] {
            assert_eq!(rejected, Err(LedgerCause::OwnerMismatch));
        }
        read.try_bind_primary_rows(baseline.view(), None, None, true)
            .unwrap();
        read.try_bind_fallback_discard_rows(baseline.view(), None, None)
            .unwrap();
        assert_eq!(observe(&owner), before);
        assert_eq!(owner.storage.shared().is_some(), promote);
    }
}

#[test]
fn attachment_keeps_real_open_work_and_scratch_while_terminal_coverage_remains_exact() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger);
    let scratch = ledger.view().reserve(1, 1, 0).unwrap();
    let (layout, mut frame) = {
        let (mut work, _) = ledger.view().into_internal();
        let layout = work.bind_ordinary([2]).unwrap();
        let frame = layout
            .admit_observed(&mut work.reborrow().unwrap().work, [0], WorkLane::Internal)
            .unwrap();
        (layout, frame)
    };
    let before = observe(&ledger);
    ledger
        .as_read()
        .try_attach_fallback_rows(baseline.view())
        .unwrap();
    assert_eq!(observe(&ledger), before);
    assert_eq!(
        ledger.as_read().try_bind_fallback_rows(baseline.view()),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    ledger
        .view()
        .into_internal()
        .0
        .resolve(layout, &mut frame, false)
        .unwrap();
    let before = observe(&ledger);
    assert_eq!(
        ledger.as_read().try_bind_fallback_rows(baseline.view()),
        Err(LedgerCause::ActiveReservationsRemain {
            count: before.totals.2,
        })
    );
    assert_eq!(observe(&ledger), before);
    drop(scratch);
    ledger
        .as_read()
        .try_bind_fallback_rows(baseline.view())
        .unwrap();
    drop(baseline);
    assert_eq!(ledger.as_read().terminal_residue_cause(), None);
}

#[test]
fn terminal_baseline_distinguishes_live_mismatch_and_real_retained_output_cover() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger);
    let mut output = ledger.view().reserve(1, 1, 13).unwrap();
    output.retain_for_output().unwrap();
    ledger
        .as_read()
        .try_bind_fallback_rows(baseline.view())
        .unwrap();
    {
        let mut selected = ledger.storage.write();
        selected.view_mut().current_live_bytes += 1;
    }
    let before = observe(&ledger);
    let expected_live = baseline.canonical_assembly.live_bytes().unwrap()
        + baseline.evidence_assembly.live_bytes().unwrap()
        + baseline.fallback.live_bytes().unwrap()
        + baseline.evidence.live_bytes().unwrap();
    assert_eq!(
        ledger.as_read().try_bind_fallback_rows(baseline.view()),
        Err(LedgerCause::PublicationLiveMismatch {
            transaction_bytes: expected_live,
            ledger_bytes: before.totals.0,
        })
    );
    assert_eq!(observe(&ledger), before);
    ledger.storage.write().view_mut().current_live_bytes -= 1;
    ledger
        .as_read()
        .try_bind_fallback_rows(baseline.view())
        .unwrap();
    drop(output);
    drop(baseline);
    assert_eq!(ledger.as_read().terminal_residue_cause(), None);
}

#[test]
fn sealed_exact_pair_and_closed_primary_share_the_real_future_graph() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger);
    let mut canonical = attributed_batch(&mut ledger, 13);
    let mut evidence = attributed_batch(&mut ledger, 17);
    let canonical_cover = canonical.account_capacity_bytes();
    let evidence_cover = evidence.account_capacity_bytes();
    let shared = Arc::clone(ledger.storage.shared().unwrap());
    assert!(lock(&shared).dimension_snapshot_staging.is_empty());
    assert!(lock(&shared).phase_snapshot_staging.is_empty());
    ledger.view().seal_ordinary_work().unwrap();
    let before = observe(&ledger);
    assert!(
        ledger
            .as_read()
            .try_qualify_and_push_exact_primary_commitment_pair_rows(
                baseline.as_read(),
                &mut canonical,
                OutputCommitment::new(1, 3),
                &mut evidence,
                OutputCommitment::new(1, 2),
            )
            .is_ok()
    );
    assert_eq!(observe(&ledger), before);
    assert_eq!(canonical.commitments[0].dimension_position, 0);
    assert_eq!(evidence.commitments[0].dimension_position, 0);
    assert_eq!(
        ledger.as_read().try_bind_primary_rows(
            baseline.view(),
            Some(&mut canonical),
            Some(&mut evidence),
            true,
        ),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert_eq!(canonical.attribution_len(), 1);
    assert_eq!(evidence.attribution_len(), 1);
    assert_eq!(canonical.commitment_len(), 1);
    assert_eq!(evidence.commitment_len(), 1);
    assert_eq!(observe(&ledger), before);
    ledger.view().close_work().unwrap();
    let before = observe(&ledger);
    ledger
        .view()
        .as_read()
        .try_bind_primary_rows(
            baseline.view(),
            Some(&mut canonical),
            Some(&mut evidence),
            true,
        )
        .unwrap();
    assert_eq!(observe(&ledger), before);
    assert!(canonical.is_empty_storage_shell());
    assert!(evidence.is_empty_storage_shell());
    assert!(baseline.evidence.is_empty_storage_shell());
    assert_eq!(canonical.account_capacity_bytes(), canonical_cover);
    assert_eq!(evidence.account_capacity_bytes(), evidence_cover);
    assert_eq!(baseline.canonical_assembly.attribution_len(), 1);
    assert_eq!(baseline.canonical_assembly.commitment_len(), 2);
    assert_eq!(baseline.evidence_assembly.attribution_len(), 2);
    assert_eq!(baseline.evidence_assembly.commitment_len(), 2);
    assert_eq!(baseline.fallback.attribution_len(), 1);
    assert!(Arc::ptr_eq(ledger.storage.shared().unwrap(), &shared));
    drop((baseline, canonical, evidence));
    assert_eq!(ledger.as_read().terminal_residue_cause(), None);
}

#[test]
fn attachment_and_terminal_binding_recheck_current_canonical_and_evidence_limits() {
    for (class, canonical_limit, evidence_limit, value, limit) in [
        (CounterClass::Committed, 4, 64, 5, 4),
        (CounterClass::PublishedOutcomeEvidence, 64, 6, 7, 6),
    ] {
        let mut ledger = ledger();
        let mut baseline = baseline(&mut ledger);
        ledger
            .as_read()
            .try_attach_fallback_rows(baseline.view())
            .unwrap();
        ledger
            .view()
            .tighten_dimension_policy(1, policy(canonical_limit, evidence_limit))
            .unwrap();
        let before = observe(&ledger);
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
        assert_eq!(baseline.canonical_assembly.commitment_len(), 1);
        assert_eq!(baseline.fallback.commitment_len(), 1);
        assert_eq!(baseline.evidence.commitment_len(), 1);
        assert_eq!(observe(&ledger), before);
    }
}

#[test]
fn rejected_exact_pair_and_materialized_primary_have_the_same_current_limit_error() {
    for (class, canonical_limit, evidence_limit, value, limit) in [
        (CounterClass::Committed, 7, 64, 8, 7),
        (CounterClass::PublishedOutcomeEvidence, 64, 5, 6, 5),
    ] {
        let mut ledger = ledger();
        let mut baseline = baseline(&mut ledger);
        let mut canonical = attributed_batch(&mut ledger, 13);
        let mut evidence = attributed_batch(&mut ledger, 17);
        ledger
            .view()
            .tighten_dimension_policy(1, policy(canonical_limit, evidence_limit))
            .unwrap();
        let before = observe(&ledger);
        let cause = LedgerCause::LimitExceeded {
            class,
            dimension: 1,
            value,
            limit,
        };
        assert!(matches!(
            ledger
                .as_read()
                .try_qualify_and_push_exact_primary_commitment_pair_rows(
                    baseline.as_read(),
                    &mut canonical,
                    OutputCommitment::new(1, 3),
                    &mut evidence,
                    OutputCommitment::new(1, 2),
                ),
            Err(CommitmentPairPushError::Ledger(actual)) if actual == cause
        ));
        assert_eq!(canonical.commitment_len(), 0);
        assert_eq!(evidence.commitment_len(), 0);
        assert_eq!(observe(&ledger), before);
        assert!(
            ledger
                .as_read()
                .try_push_commitment(&mut canonical, OutputCommitment::new(1, 3))
                .is_ok()
        );
        assert!(
            ledger
                .as_read()
                .try_push_commitment(&mut evidence, OutputCommitment::new(1, 2))
                .is_ok()
        );
        assert_eq!(
            ledger.as_read().try_bind_primary_rows(
                baseline.view(),
                Some(&mut canonical),
                Some(&mut evidence),
                true,
            ),
            Err(cause)
        );
        assert_eq!(canonical.attribution_len(), 1);
        assert_eq!(evidence.attribution_len(), 1);
        assert_eq!(canonical.commitment_len(), 1);
        assert_eq!(evidence.commitment_len(), 1);
        assert_eq!(baseline.evidence.attribution_len(), 1);
        assert_eq!(baseline.evidence_assembly.attribution_len(), 0);
        assert_eq!(observe(&ledger), before);
    }
}

#[test]
fn exact_pair_and_batch_pair_check_overflow_before_current_limits() {
    for canonical_overflow in [false, true] {
        let mut ledger = ledger();
        let baseline = baseline(&mut ledger);
        let mut canonical = batch(&mut ledger, 0, 2);
        let mut evidence = batch(&mut ledger, 0, 2);
        let class = if canonical_overflow {
            assert!(
                ledger
                    .as_read()
                    .try_push_commitment(&mut canonical, OutputCommitment::new(1, u64::MAX))
                    .is_ok()
            );
            CounterClass::Committed
        } else {
            assert!(
                ledger
                    .as_read()
                    .try_push_commitment(&mut evidence, OutputCommitment::new(1, u64::MAX))
                    .is_ok()
            );
            CounterClass::PublishedOutcomeEvidence
        };
        ledger
            .view()
            .tighten_dimension_policy(1, policy(7, 7))
            .unwrap();
        let before = observe(&ledger);
        let cause = LedgerCause::ArithmeticOverflow {
            class,
            dimension: 1,
        };
        assert!(matches!(
            ledger
                .as_read()
                .try_qualify_and_push_exact_primary_commitment_pair_rows(
                    baseline.as_read(),
                    &mut canonical,
                    OutputCommitment::new(1, 1),
                    &mut evidence,
                    OutputCommitment::new(1, 1),
                ),
            Err(CommitmentPairPushError::Ledger(actual)) if actual == cause
        ));
        let pair = if canonical_overflow {
            ledger
                .as_read()
                .validate_typed_batch_pair(&baseline.canonical_assembly, &canonical)
        } else {
            ledger
                .as_read()
                .validate_typed_batch_pair(&baseline.evidence, &evidence)
        };
        assert_eq!(pair, Err(cause));
        assert_eq!(canonical.commitment_len(), usize::from(canonical_overflow));
        assert_eq!(evidence.commitment_len(), usize::from(!canonical_overflow));
        assert_eq!(observe(&ledger), before);
    }
}

#[test]
fn terminal_phase_faults_use_selected_inline_and_shared_rows_after_work_precedence() {
    for promote in [false, true] {
        let mut ledger = ledger();
        let mut baseline = empty_baseline(&mut ledger);
        if promote {
            drop(ledger.view().reserve(3, 1, 0).unwrap());
        }
        match &mut ledger.storage {
            FiniteStorage::Inline(Some(state)) => state.phases[1].state_position = 3,
            FiniteStorage::Shared(shared) => lock(shared).phases[1].state_position = 3,
            FiniteStorage::Inline(None) => panic!("actual finite storage remains installed"),
        }
        ledger.work_header.producer_open = true;
        let before = observe(&ledger);
        assert_eq!(
            ledger.as_read().try_attach_fallback_rows(baseline.view()),
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        );
        for rejected in [
            ledger.as_read().try_bind_fallback_rows(baseline.view()),
            ledger
                .as_read()
                .try_bind_primary_rows(baseline.view(), None, None, true),
            ledger
                .as_read()
                .try_bind_fallback_discard_rows(baseline.view(), None, None),
        ] {
            assert_eq!(
                rejected,
                Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
            );
        }
        assert_eq!(observe(&ledger), before);
        ledger.work_header.producer_open = false;
        for rejected in [
            ledger.as_read().try_bind_fallback_rows(baseline.view()),
            ledger
                .as_read()
                .try_bind_primary_rows(baseline.view(), None, None, true),
            ledger
                .as_read()
                .try_bind_fallback_discard_rows(baseline.view(), None, None),
        ] {
            assert_eq!(rejected, Err(LedgerCause::FinalSnapshotShapeMismatch));
        }
    }
}

#[test]
fn shared_fallback_discards_preflight_tokens_but_ignore_discarded_commitment_positions() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger);
    let mut canonical = attributed_batch(&mut ledger, 13);
    let mut evidence = attributed_batch(&mut ledger, 17);
    assert!(
        ledger
            .as_read()
            .try_push_commitment(&mut canonical, OutputCommitment::new(1, 3))
            .is_ok()
    );
    canonical.commitments[0].dimension_position = usize::MAX;
    let original_phase = canonical
        .capacity_cover
        .as_ref()
        .unwrap()
        .core
        .phase_position;
    canonical
        .capacity_cover
        .as_mut()
        .unwrap()
        .core
        .phase_position = 3;
    let before = observe(&ledger);
    assert_eq!(
        ledger.as_read().try_bind_fallback_discard_rows(
            baseline.view(),
            Some(&mut canonical),
            Some(&mut evidence),
        ),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    assert_eq!(baseline.evidence_assembly.attribution_len(), 0);
    assert_eq!(baseline.evidence.attribution_len(), 1);
    assert_eq!(baseline.fallback.attribution_len(), 1);
    assert_eq!(observe(&ledger), before);
    canonical
        .capacity_cover
        .as_mut()
        .unwrap()
        .core
        .phase_position = original_phase;
    ledger
        .as_read()
        .try_bind_fallback_discard_rows(baseline.view(), Some(&mut canonical), Some(&mut evidence))
        .unwrap();
    assert_eq!(observe(&ledger), before);
    assert!(baseline.evidence.is_empty_storage_shell());
    assert!(baseline.fallback.is_empty_storage_shell());
    assert_eq!(baseline.evidence_assembly.attribution_len(), 2);
    assert_eq!(baseline.evidence_assembly.commitment_len(), 2);
    assert_eq!(canonical.attribution_len(), 1);
    assert_eq!(canonical.commitments[0].dimension_position, usize::MAX);
    assert_eq!(evidence.attribution_len(), 1);
    drop((baseline, canonical, evidence));
    assert_eq!(ledger.as_read().terminal_residue_cause(), None);
}

#[test]
fn foreign_primary_sources_keep_each_entry_points_existing_integrity_precedence() {
    let mut target = ledger();
    let mut foreign = ledger();
    let mut baseline = baseline(&mut target);
    let mut canonical = attributed_batch(&mut foreign, 13);
    let mut evidence = attributed_batch(&mut target, 17);
    let target_before = observe(&target);
    let foreign_before = observe(&foreign);
    assert!(matches!(
        target
            .as_read()
            .try_qualify_and_push_exact_primary_commitment_pair_rows(
                baseline.as_read(),
                &mut canonical,
                OutputCommitment::new(1, 1),
                &mut evidence,
                OutputCommitment::new(1, 1),
            ),
        Err(CommitmentPairPushError::Ledger(LedgerCause::OwnerMismatch))
    ));
    target.work_header.integrity_fault = true;
    assert_eq!(
        target.as_read().try_bind_primary_rows(
            baseline.view(),
            Some(&mut canonical),
            Some(&mut evidence),
            true,
        ),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    assert_eq!(
        target.as_read().try_bind_fallback_discard_rows(
            baseline.view(),
            Some(&mut canonical),
            Some(&mut evidence),
        ),
        Err(LedgerCause::OwnerMismatch)
    );
    assert_eq!(
        target
            .as_read()
            .validate_typed_batch_pair(&baseline.canonical_assembly, &canonical),
        Err(LedgerCause::OwnerMismatch)
    );
    target.work_header.integrity_fault = false;
    assert_eq!(
        target.as_read().try_bind_primary_rows(
            baseline.view(),
            Some(&mut canonical),
            Some(&mut evidence),
            true,
        ),
        Err(LedgerCause::OwnerMismatch)
    );
    assert_eq!(canonical.attribution_len(), 1);
    assert_eq!(canonical.commitment_len(), 0);
    assert_eq!(evidence.attribution_len(), 1);
    assert_eq!(evidence.commitment_len(), 0);
    assert_eq!(observe(&target), target_before);
    assert_eq!(observe(&foreign), foreign_before);
}

#[test]
fn terminal_binding_keeps_publication_state_ahead_of_open_channel_errors() {
    for (publication, mode, cause) in [
        (
            PublicationState::Closing,
            WorkMode::OrdinarySealed,
            LedgerCause::PublicationAlreadyPending,
        ),
        (
            PublicationState::Pending,
            WorkMode::Closed,
            LedgerCause::PublicationAlreadyPending,
        ),
        (
            PublicationState::TerminalQualified,
            WorkMode::Closed,
            LedgerCause::PublicationAlreadyCompleted,
        ),
        (
            PublicationState::Published,
            WorkMode::Closed,
            LedgerCause::PublicationAlreadyCompleted,
        ),
    ] {
        let mut ledger = ledger();
        let mut baseline = empty_baseline(&mut ledger);
        ledger.storage.write().view_mut().publication = publication;
        ledger.work_header.mode = mode;
        ledger.work_header.producer_open = true;
        let before = observe(&ledger);
        for rejected in [
            ledger.as_read().try_bind_fallback_rows(baseline.view()),
            ledger
                .as_read()
                .try_bind_primary_rows(baseline.view(), None, None, true),
            ledger
                .as_read()
                .try_bind_fallback_discard_rows(baseline.view(), None, None),
        ] {
            assert_eq!(rejected, Err(cause.clone()));
        }
        assert_eq!(observe(&ledger), before);
        assert!(ledger.storage.shared().is_none());
    }
}

#[test]
fn primary_discard_keeps_canonical_owners_and_skips_their_commitment_positions() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger);
    let mut canonical = attributed_batch(&mut ledger, 13);
    let mut evidence = attributed_batch(&mut ledger, 17);
    assert!(
        ledger
            .as_read()
            .try_push_commitment(&mut canonical, OutputCommitment::new(1, u64::MAX))
            .is_ok()
    );
    canonical.commitments[0].dimension_position = usize::MAX;
    let before = observe(&ledger);
    ledger
        .as_read()
        .try_bind_primary_rows(
            baseline.view(),
            Some(&mut canonical),
            Some(&mut evidence),
            false,
        )
        .unwrap();
    assert_eq!(observe(&ledger), before);
    assert_eq!(canonical.attribution_len(), 1);
    assert_eq!(canonical.commitment_len(), 1);
    assert_eq!(canonical.commitments[0].dimension_position, usize::MAX);
    assert_eq!(baseline.canonical_assembly.attribution_len(), 0);
    assert_eq!(baseline.canonical_assembly.commitment_len(), 1);
    assert_eq!(baseline.evidence_assembly.attribution_len(), 2);
    assert_eq!(baseline.evidence_assembly.commitment_len(), 1);
    assert!(evidence.is_empty_storage_shell());
    assert!(baseline.evidence.is_empty_storage_shell());
    assert_eq!(baseline.fallback.attribution_len(), 1);
    drop((baseline, canonical, evidence));
    assert_eq!(ledger.as_read().terminal_residue_cause(), None);
}
