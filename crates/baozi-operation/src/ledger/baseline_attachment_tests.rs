use super::*;

#[derive(Debug)]
struct Owner;

impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Startup,
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Dimension {
    Bytes,
    Work,
    Failure,
    Unknown,
}

fn ledger() -> Ledger<Owner, Phase, Dimension> {
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, 64, u64::MAX)
                    .with_published_outcome_evidence_limit(64),
            ),
            (Dimension::Work, DimensionPolicy::counter(64, 64, 0)),
            (Dimension::Failure, DimensionPolicy::counter(8, 8, 0)),
        ],
    )
    .expect("actual attachment policy");
    Ledger::try_new(limits, &[Phase::Startup, Phase::Body]).expect("actual attachment ledger")
}

fn baseline(
    ledger: &mut Ledger<Owner, Phase, Dimension>,
) -> FallbackBatchBaseline<Owner, Phase, Dimension> {
    let mut canonical = ledger
        .try_attribution_batch::<CanonicalOutput>(Phase::Startup, Dimension::Bytes, 0, 1)
        .expect("canonical assembly storage");
    let evidence_assembly = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Startup, Dimension::Bytes, 2, 2)
        .expect("evidence assembly storage");
    let mut fallback = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Startup, Dimension::Bytes, 1, 1)
        .expect("fallback batch storage");
    let mut evidence = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Startup, Dimension::Bytes, 1, 1)
        .expect("ledger evidence batch storage");
    for (batch, bytes, commitment) in [(&mut fallback, 7, 3), (&mut evidence, 11, 4)] {
        let attribution = ledger
            .reserve_operation_attribution::<OutcomeEvidenceDestination>(
                Phase::Startup,
                Dimension::Bytes,
                bytes,
            )
            .expect("actual retained attribution");
        assert!(batch.try_push_attribution(attribution).is_ok());
        assert!(
            ledger
                .try_push_commitment(batch, OutputCommitment::new(Dimension::Bytes, commitment))
                .is_ok()
        );
    }
    assert!(
        ledger
            .try_push_commitment(&mut canonical, OutputCommitment::new(Dimension::Bytes, 5))
            .is_ok()
    );
    FallbackBatchBaseline::new_unbound(canonical, evidence_assembly, fallback, evidence)
}

#[test]
fn attachment_preserves_live_scratch_and_preexisting_work_but_strict_binding_rejects_residue() {
    let mut ledger = ledger();
    ledger.charge_attempted(Dimension::Work, 4).unwrap();
    ledger.charge_consumed(Dimension::Work, 2).unwrap();
    let scratch = ledger
        .reserve(Phase::Body, Dimension::Bytes, 17)
        .expect("preexisting live scratch");
    let mut baseline = baseline(&mut ledger);
    let before = ledger.try_snapshot().unwrap();

    ledger.try_attach_fallback_baseline(&mut baseline).unwrap();

    assert_eq!(ledger.try_snapshot().unwrap(), before);
    ledger.validate_live_reservation(&scratch).unwrap();
    assert_eq!(baseline.fallback.attribution_len(), 1);
    assert_eq!(baseline.evidence.attribution_len(), 1);
    assert_eq!(baseline.evidence_assembly.attribution_len(), 0);
    assert_eq!(
        ledger.try_bind_fallback_baseline(&mut baseline),
        Err(LedgerCause::ActiveReservationsRemain {
            count: before.active_reservations(),
        })
    );
    assert_eq!(ledger.try_snapshot().unwrap(), before);

    drop(scratch);
    ledger.try_bind_fallback_baseline(&mut baseline).unwrap();
    drop(baseline);
    let finished = ledger
        .finish()
        .expect("real storage owners release all charges");
    let work = finished.dimension(Dimension::Work).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (4, 2));
}

#[test]
fn attachment_preserves_pending_linear_work_until_explicit_resolution() {
    let mut ledger = ledger();
    let (layout, mut linear) = {
        let (mut work, layout) = ledger.bind_internal_work([Dimension::Work]).unwrap();
        let frame = layout
            .admit_observed(&mut work.work, [3], WorkLane::Internal)
            .unwrap();
        (layout, frame)
    };
    let mut baseline = baseline(&mut ledger);
    let before = ledger.try_snapshot().unwrap();

    ledger.try_attach_fallback_baseline(&mut baseline).unwrap();

    assert_eq!(ledger.try_snapshot().unwrap(), before);
    assert_eq!(linear.remaining, [3]);
    {
        let work = &ledger.work;
        assert_eq!(work.rows[1].state.pending_consumed, 3);
        assert_eq!(work.header.open_envelopes, 1);
        assert!(!work.header.parser_parent_open);
        assert!(!work.header.parser_child_open);
        assert_eq!(work.header.protected_dimension, None);
    }
    for rejected in [
        ledger.try_bind_fallback_baseline(&mut baseline),
        ledger.preflight_configure_protected_work(Dimension::Failure),
        ledger.configure_protected_work(Dimension::Failure),
    ] {
        assert_eq!(
            rejected,
            Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        );
    }
    assert_eq!(ledger.try_snapshot().unwrap(), before);

    {
        let (mut work, _) = ledger.split_internal();
        linear
            .complete_batch_with_outcome(&layout, &mut work.reborrow().unwrap().work, [3])
            .into_result()
            .unwrap();
        work.resolve(layout, &mut linear, false).unwrap();
    }
    ledger.try_bind_fallback_baseline(&mut baseline).unwrap();
    ledger
        .preflight_configure_protected_work(Dimension::Failure)
        .unwrap();
    ledger.configure_protected_work(Dimension::Failure).unwrap();
}

#[test]
fn attachment_preserves_forgotten_kernel_channels_and_pending_internal_work() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger);
    {
        let (mut work, _storage) = ledger.exclusive().unwrap();
        let layout = work.bind([Dimension::Work]).unwrap();
        let linear = layout
            .admit_observed(&mut work.work, [3], WorkLane::Internal)
            .unwrap();
        let parent = layout
            .begin_parser(&mut work.work, [2], [None], ParserChannel::Parent)
            .unwrap();
        let child = layout
            .begin_parser(&mut work.work, [1], [None], ParserChannel::Child)
            .unwrap();
        assert_eq!(linear.remaining, [3]);
        assert_eq!(parent.remaining, [2]);
        assert_eq!(child.remaining, [1]);
        assert_eq!(work.counters(Dimension::Work).unwrap(), (6, 0, 3));
        let _unresolved_frames = std::mem::ManuallyDrop::new((linear, parent, child));
    }
    let before = ledger.try_snapshot().unwrap();

    ledger.try_attach_fallback_baseline(&mut baseline).unwrap();

    assert_eq!(ledger.try_snapshot().unwrap(), before);
    {
        let work = &ledger.work;
        assert_eq!(work.rows[1].state.pending_consumed, 3);
        assert_eq!(work.header.open_envelopes, 1);
        assert!(work.header.parser_parent_open);
        assert!(work.header.parser_child_open);
        assert_eq!(work.header.protected_dimension, None);
    }
    for rejected in [
        ledger.try_bind_fallback_baseline(&mut baseline),
        ledger.preflight_configure_protected_work(Dimension::Failure),
        ledger.configure_protected_work(Dimension::Failure),
    ] {
        assert_eq!(
            rejected,
            Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        );
    }
    assert_eq!(ledger.try_snapshot().unwrap(), before);
}

#[test]
fn attachment_allows_an_unrelated_zero_byte_capability_but_strict_binding_still_counts_it() {
    let mut ledger = ledger();
    let extra = ledger.reserve(Phase::Body, Dimension::Bytes, 0).unwrap();
    let mut baseline = baseline(&mut ledger);
    let before = ledger.try_snapshot().unwrap();

    ledger.try_attach_fallback_baseline(&mut baseline).unwrap();
    assert_eq!(ledger.try_snapshot().unwrap(), before);
    assert_eq!(
        ledger.try_bind_fallback_baseline(&mut baseline),
        Err(LedgerCause::ActiveReservationsRemain {
            count: before.active_reservations(),
        })
    );

    drop(extra);
    ledger.try_bind_fallback_baseline(&mut baseline).unwrap();
}

#[test]
fn attachment_rejects_a_foreign_baseline_without_mutation_and_preserves_it_for_its_owner() {
    let mut target = ledger();
    let mut foreign = ledger();
    let mut baseline = baseline(&mut foreign);
    let target_before = target.try_snapshot().unwrap();
    let foreign_before = foreign.try_snapshot().unwrap();

    assert_eq!(
        target.try_attach_fallback_baseline(&mut baseline),
        Err(LedgerCause::OwnerMismatch)
    );

    assert_eq!(target.try_snapshot().unwrap(), target_before);
    assert_eq!(foreign.try_snapshot().unwrap(), foreign_before);
    foreign.try_bind_fallback_baseline(&mut baseline).unwrap();
}

#[test]
fn attachment_rechecks_current_limits_for_canonical_and_aggregated_evidence_commitments() {
    for (class, canonical_limit, evidence_limit, value, limit) in [
        (CounterClass::Committed, 4, 64, 5, 4),
        (CounterClass::PublishedOutcomeEvidence, 64, 6, 7, 6),
    ] {
        let mut ledger = ledger();
        let mut baseline = baseline(&mut ledger);
        ledger.try_attach_fallback_baseline(&mut baseline).unwrap();
        ledger
            .tighten_dimension_policy(
                Dimension::Bytes,
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, canonical_limit, u64::MAX)
                    .with_published_outcome_evidence_limit(evidence_limit),
            )
            .unwrap();
        let before = ledger.try_snapshot().unwrap();

        assert_eq!(
            ledger.try_attach_fallback_baseline(&mut baseline),
            Err(LedgerCause::LimitExceeded {
                class,
                dimension: Dimension::Bytes,
                value,
                limit,
            })
        );
        assert_eq!(ledger.try_snapshot().unwrap(), before);
    }
}

#[test]
fn attachment_rejects_an_assembly_that_cannot_absorb_the_actual_sources() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger);
    let undersized = ledger
        .try_attribution_batch::<OutcomeEvidenceDestination>(Phase::Startup, Dimension::Bytes, 1, 1)
        .unwrap();
    drop(std::mem::replace(
        &mut baseline.evidence_assembly,
        undersized,
    ));
    let before = ledger.try_snapshot().unwrap();

    assert_eq!(
        ledger.try_attach_fallback_baseline(&mut baseline),
        Err(LedgerCause::ActiveReservationsRemain { count: u64::MAX })
    );
    assert_eq!(ledger.try_snapshot().unwrap(), before);
}

#[test]
fn attachment_rejects_out_of_bounds_attribution_positions_without_releasing_the_token() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger);
    let before = ledger.try_snapshot().unwrap();
    let previous = baseline.fallback.attributions[0].core.phase_position;
    baseline.fallback.attributions[0].core.phase_position = usize::MAX;

    let rejected = ledger.try_attach_fallback_baseline(&mut baseline);

    baseline.fallback.attributions[0].core.phase_position = previous;
    assert_eq!(rejected, Err(LedgerCause::FinalSnapshotShapeMismatch));
    assert_eq!(ledger.try_snapshot().unwrap(), before);
    ledger.try_bind_fallback_baseline(&mut baseline).unwrap();
}

#[test]
fn attachment_rejects_out_of_bounds_capacity_cover_positions_without_releasing_storage() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger);
    let before = ledger.try_snapshot().unwrap();
    let cover = baseline.canonical_assembly.capacity_cover.as_mut().unwrap();
    let previous = cover.core.dimension_position;
    cover.core.dimension_position = usize::MAX;

    let rejected = ledger.try_attach_fallback_baseline(&mut baseline);

    baseline
        .canonical_assembly
        .capacity_cover
        .as_mut()
        .unwrap()
        .core
        .dimension_position = previous;
    assert_eq!(rejected, Err(LedgerCause::FinalSnapshotShapeMismatch));
    assert_eq!(ledger.try_snapshot().unwrap(), before);
    ledger.try_bind_fallback_baseline(&mut baseline).unwrap();
}

#[test]
fn attachment_rejects_out_of_bounds_commitment_positions_without_consuming_the_batch() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger);
    let before = ledger.try_snapshot().unwrap();
    let previous = baseline.evidence.commitments[0].dimension_position;
    baseline.evidence.commitments[0].dimension_position = usize::MAX;

    let rejected = ledger.try_attach_fallback_baseline(&mut baseline);

    baseline.evidence.commitments[0].dimension_position = previous;
    assert_eq!(rejected, Err(LedgerCause::FinalSnapshotShapeMismatch));
    assert_eq!(ledger.try_snapshot().unwrap(), before);
    ledger.try_bind_fallback_baseline(&mut baseline).unwrap();
}

#[test]
fn attachment_requires_open_publication_while_strict_binding_accepts_closed_work() {
    let mut ledger = ledger();
    let mut baseline = baseline(&mut ledger);
    ledger.seal_ordinary_work().unwrap();
    let before = ledger.try_snapshot().unwrap();

    assert_eq!(
        ledger.try_attach_fallback_baseline(&mut baseline),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert_eq!(ledger.try_snapshot().unwrap(), before);

    ledger.close_work().unwrap();
    assert_eq!(
        ledger.try_attach_fallback_baseline(&mut baseline),
        Err(LedgerCause::PublicationAlreadyCompleted)
    );
    ledger.try_bind_fallback_baseline(&mut baseline).unwrap();
}

#[test]
fn protected_work_preflight_is_read_only_and_configuration_remains_single_use() {
    let mut ledger = ledger();
    ledger.charge_attempted(Dimension::Work, 4).unwrap();
    ledger.charge_consumed(Dimension::Work, 2).unwrap();
    let before = ledger.try_snapshot().unwrap();

    ledger
        .preflight_configure_protected_work(Dimension::Failure)
        .unwrap();
    ledger
        .preflight_configure_protected_work(Dimension::Failure)
        .unwrap();

    assert_eq!(ledger.try_snapshot().unwrap(), before);
    assert_eq!(ledger.work.header.protected_dimension, None);
    ledger.configure_protected_work(Dimension::Failure).unwrap();
    assert_eq!(ledger.work.header.protected_dimension, Some(2));
    assert_eq!(
        ledger.preflight_configure_protected_work(Dimension::Failure),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(
        ledger.configure_protected_work(Dimension::Failure),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(
        ledger.preflight_configure_protected_work(Dimension::Unknown),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Unknown,
        })
    );
    assert_eq!(ledger.try_snapshot().unwrap(), before);
    assert_eq!(ledger.work.header.protected_dimension, Some(2));
}

#[test]
fn protected_work_preflight_and_configuration_reject_sealed_and_closed_modes() {
    let mut ledger = ledger();
    ledger.seal_ordinary_work().unwrap();
    for _ in 0..2 {
        assert_eq!(
            ledger.preflight_configure_protected_work(Dimension::Failure),
            Err(LedgerCause::InactiveLinearWorkPermit)
        );
        assert_eq!(
            ledger.configure_protected_work(Dimension::Failure),
            Err(LedgerCause::InactiveLinearWorkPermit)
        );
        assert_eq!(ledger.work.header.protected_dimension, None);
        ledger.close_work().unwrap();
    }
}

fn assert_dormant_failure_pool(ledger: &Ledger<Owner, Phase, Dimension>) {
    let work = &ledger.work;
    assert_eq!(work.header.mode, WorkMode::Ordinary);
    assert_eq!(work.header.protected_dimension, Some(2));
    assert_eq!(work.header.open_envelopes, 0);
    assert!(!work.header.protected_open);
    assert!(matches!(
        work.bind_layout(
            &ledger.work_catalog,
            [Dimension::Failure],
            WorkGrant::Ordinary,
        ),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Failure
        })
    ));
}

#[test]
fn dormant_attachment_capacity_is_exact_and_uses_the_current_tightened_policy() {
    let mut ledger = ledger();
    ledger.configure_protected_work(Dimension::Failure).unwrap();
    let charges = [(Dimension::Failure, 1), (Dimension::Failure, 1)];
    let repetitions = NonZeroU64::new(4).unwrap();
    let before = ledger.try_snapshot().unwrap();
    ledger
        .preflight_protected_work_attachment(Dimension::Failure, &charges, repetitions)
        .unwrap();
    assert_eq!(ledger.try_snapshot().unwrap(), before);
    assert_dormant_failure_pool(&ledger);

    ledger
        .tighten_dimension_policy(Dimension::Failure, DimensionPolicy::counter(7, 7, 0))
        .unwrap();
    let tightened = ledger.try_snapshot().unwrap();
    assert!(matches!(
        ledger.preflight_protected_work_attachment(Dimension::Failure, &charges, repetitions),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: Dimension::Failure,
            value: 8,
            limit: 7,
        })
    ));
    assert_eq!(ledger.try_snapshot().unwrap(), tightened);
    assert_dormant_failure_pool(&ledger);
}

#[test]
fn dormant_attachment_overflow_rejects_without_granting_or_charging_failure_work() {
    let mut ledger = ledger();
    ledger.configure_protected_work(Dimension::Failure).unwrap();
    let before = ledger.try_snapshot().unwrap();
    assert!(matches!(
        ledger.preflight_protected_work_attachment(
            Dimension::Failure,
            &[(Dimension::Failure, u64::MAX)],
            NonZeroU64::new(2).unwrap(),
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: Dimension::Failure,
        })
    ));
    assert_eq!(ledger.try_snapshot().unwrap(), before);
    assert_dormant_failure_pool(&ledger);
}
