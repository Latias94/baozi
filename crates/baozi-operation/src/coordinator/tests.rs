use super::*;
use crate::ledger::finite::{FiniteLedger, FiniteLedgerLimits};
use crate::ledger::{test_evidence_allocation_guard, test_fail_next_evidence_allocation};
use crate::{DimensionPolicy, ledger::WorkLane};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
struct Owner;

impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Final,
}

impl ProgressPhase for Phase {
    fn order(self) -> u32 {
        0
    }

    fn stable_id(self) -> &'static str {
        "final"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Unit {
    Work,
}

impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        "work"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Dimension {
    Ordinary,
    Protected,
}

#[test]
fn final_gate_remains_sealed_for_protected_recovery_and_internal_share_nesting() {
    let events = Arc::new(AtomicUsize::new(0));
    let event_calls = Arc::clone(&events);
    let control =
        OperationControl::default().with_progress_observer(move |_: ProgressEvent<Phase, Unit>| {
            event_calls.fetch_add(1, Ordering::SeqCst);
            ProgressDecision::Continue
        });
    let limits = LedgerLimits::new(
        0,
        [
            (Dimension::Ordinary, DimensionPolicy::counter(32, 32, 0)),
            (Dimension::Protected, DimensionPolicy::counter(32, 32, 0)),
        ],
    )
    .expect("finite ordinary and protected limits");
    let mut coordinator = OperationCoordinator::<Owner, Phase, Phase, Unit, Dimension>::try_new(
        control,
        limits,
        &[Phase::Final],
    )
    .expect("complete phase catalog");
    coordinator
        .retained_ledger_mut()
        .view()
        .configure_protected_work(Dimension::Protected)
        .expect("protected role is configured before sealing");
    assert_eq!(
        coordinator
            .retained_ledger_mut()
            .charge_attempted(Dimension::Protected, 0),
        Err(LedgerCause::UnknownDimension {
            dimension: Dimension::Protected
        })
    );

    coordinator
        .try_prepare_typed_control(FinalProgress::new(Phase::Final, Unit::Work, 1))
        .expect("accepted final control seals ordinary work");
    coordinator
        .try_prepare_typed_control(FinalProgress::new(Phase::Final, Unit::Work, 2))
        .expect("a spent final gate returns its original result");
    assert_eq!(events.load(Ordering::SeqCst), 1);
    assert_eq!(
        coordinator.reported_progress(Phase::Final, Unit::Work),
        Some((1, Some(1)))
    );
    assert_eq!(
        coordinator
            .retained_ledger_mut()
            .charge_attempted(Dimension::Ordinary, 1),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    coordinator
        .retained_ledger_mut()
        .view()
        .activate_protected_work(Dimension::Protected)
        .expect("a later generation rejection can still activate its finite grant");

    let (producer_layout, mut producer) = {
        let (mut internal, _) = coordinator.retained_ledger_mut().split_internal();
        let layout = internal.bind_protected([Dimension::Protected]).unwrap();
        let producer = layout
            .admit_observed(
                &mut internal.reborrow().unwrap().work,
                [4],
                WorkLane::Protected,
            )
            .expect("one protected producer envelope");
        (layout, producer)
    };
    assert!(matches!(
        coordinator
            .retained_ledger_mut()
            .split_internal()
            .0
            .bind_share([Dimension::Ordinary]),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    let (share_layout, mut share) = {
        let (mut internal, _) = coordinator.retained_ledger_mut().split_internal();
        let layout = internal
            .bind_share([Dimension::Protected, Dimension::Protected])
            .unwrap();
        let share = layout
            .admit_observed(
                &mut internal.reborrow().unwrap().work,
                [1, 1],
                WorkLane::Internal,
            )
            .expect("protected internal clone-and-handle work coexists with its producer");
        (layout, share)
    };
    {
        let (mut internal, _) = coordinator.retained_ledger_mut().split_internal();
        producer
            .complete_batch_with_outcome(
                &producer_layout,
                &mut internal.reborrow().unwrap().work,
                [2],
            )
            .into_result()
            .expect("first physical producer prefix");
        share
            .complete_batch_with_outcome(
                &share_layout,
                &mut internal.reborrow().unwrap().work,
                [1, 0],
            )
            .into_result()
            .expect("reference clone completed");
        share
            .complete_batch_with_outcome(
                &share_layout,
                &mut internal.reborrow().unwrap().work,
                [0, 1],
            )
            .into_result()
            .expect("retained handle completed");
        assert_eq!(share.remaining, [0, 0]);
        internal
            .resolve(share_layout, &mut share, false)
            .expect("internal share explicitly closes its envelope");
        producer
            .complete_batch_with_outcome(
                &producer_layout,
                &mut internal.reborrow().unwrap().work,
                [2],
            )
            .into_result()
            .expect("remaining physical producer prefix");
    }
    assert_eq!(
        coordinator.retained_ledger_mut().close_work(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    assert_eq!(producer.remaining, [0]);
    coordinator
        .retained_ledger_mut()
        .split_internal()
        .0
        .resolve(producer_layout, &mut producer, false)
        .expect("a completed producer must still explicitly finish");
    coordinator
        .retained_ledger_mut()
        .close_work()
        .expect("all protected ownership is resolved");
    assert_eq!(
        coordinator
            .retained_ledger_mut()
            .view()
            .activate_protected_work(Dimension::Protected),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert!(matches!(
        coordinator
            .retained_ledger_mut()
            .split_internal()
            .0
            .bind_share([Dimension::Protected]),
        Err(LedgerCause::PublicationAlreadyCompleted)
    ));
    let expected_administration = coordinator.known_administrative_bytes().unwrap();
    let snapshot = coordinator
        .finish()
        .expect("closed work produces final evidence");
    let protected = snapshot
        .dimension(Dimension::Protected)
        .expect("protected work evidence");
    assert_eq!((protected.attempted(), protected.consumed()), (6, 6));
    assert_eq!(
        snapshot.current_administrative_bytes(),
        expected_administration
    );
    assert_eq!(events.load(Ordering::SeqCst), 1);
}

#[test]
fn exclusive_progress_retains_capacity_and_releases_it_after_the_actual_parser_closes() {
    let limits = LedgerLimits::new(
        0,
        [(Dimension::Ordinary, DimensionPolicy::counter(2, 2, 0))],
    )
    .unwrap();
    let mut coordinator = OperationCoordinator::<Owner, Phase, Phase, Unit, Dimension>::try_new(
        OperationControl::default(),
        limits,
        &[Phase::Final],
    )
    .unwrap();
    let mut physical = [0_u8; 2];
    {
        let (mut poll, mut progress, ledger) = coordinator.split_exclusive_progress_and_ledger();
        let (mut work, mut storage) = ledger.view().into_exclusive().unwrap();
        let layout = work.bind([Dimension::Ordinary]).unwrap();
        let mut frame = layout
            .begin_parser(
                &mut work.work,
                [2],
                [None],
                crate::ledger::ParserChannel::Child,
            )
            .unwrap();
        physical[0] = 17;
        frame.complete(&layout, &mut work.work, [1]).unwrap();
        progress
            .report_progress(
                &mut poll,
                &mut storage,
                Phase::Final,
                Unit::Work,
                1,
                Some(2),
            )
            .unwrap();
        assert_eq!(work.counters(Dimension::Ordinary).unwrap(), (2, 1, 0));
        assert_eq!(frame.remaining, [1]);
        assert_eq!(progress.progress.len(), 1);
        let ProgressRows::Dynamic {
            rows,
            retained_bytes,
        } = &progress.progress
        else {
            panic!("dynamic progress borrows the actual vector and credit");
        };
        assert_eq!(
            **retained_bytes,
            u64::try_from(rows.capacity() * std::mem::size_of::<((Phase, Unit), ProgressState)>())
                .unwrap()
        );
        assert!(**retained_bytes > 0);
        physical[1] = 29;
        frame.complete(&layout, &mut work.work, [1]).unwrap();
        frame.resolve(&mut work.work).unwrap();
        work.check_quiescent().unwrap();
    }
    assert_eq!(physical, [17, 29]);
    assert_eq!(
        coordinator.reported_progress(Phase::Final, Unit::Work),
        Some((1, Some(2)))
    );
    let retained = coordinator.progress_evidence_bytes;
    let expected_administration = coordinator.known_administrative_bytes().unwrap();
    let snapshot = coordinator.finish().unwrap();
    assert_eq!(
        snapshot.current_administrative_bytes(),
        expected_administration
    );
    assert!(snapshot.peak_administrative_bytes() >= retained);
    let work = snapshot.dimension(Dimension::Ordinary).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (2, 2));
}

#[test]
fn exclusive_progress_capacity_denial_uses_the_same_budget_and_terminal_owner() {
    let limits = LedgerLimits::new(0, [])
        .unwrap()
        .try_with_evidence_byte_limit(0)
        .unwrap();
    let mut coordinator = OperationCoordinator::<Owner, Phase, Phase, Unit, Dimension>::try_new(
        OperationControl::default(),
        limits,
        &[],
    )
    .unwrap();
    let cause = {
        let (mut poll, mut progress, ledger) = coordinator.split_exclusive_progress_and_ledger();
        let (work, mut storage) = ledger.view().into_exclusive().unwrap();
        let cause = progress
            .report_progress(&mut poll, &mut storage, Phase::Final, Unit::Work, 1, None)
            .unwrap_err();
        assert!(matches!(
            &cause,
            ControlCause::ProgressStorage {
                cause: ProgressStorageError::Evidence {
                    cause: LedgerEvidenceError::LimitExceeded {
                        purpose: crate::LedgerEvidencePurpose::Progress,
                        limit: 0,
                        ..
                    }
                }
            }
        ));
        assert_eq!(progress.progress.len(), 0);
        let ProgressRows::Dynamic {
            rows,
            retained_bytes,
        } = &progress.progress
        else {
            panic!("dynamic progress borrows the actual vector and credit");
        };
        assert!(rows.is_empty());
        assert_eq!(rows.capacity(), 0);
        assert_eq!(**retained_bytes, 0);
        assert_eq!(poll.poll(), Err(cause.clone()));
        work.check_quiescent().unwrap();
        cause
    };
    let expected_administration = coordinator.known_administrative_bytes().unwrap();
    let (terminal, snapshot) = coordinator.finish().unwrap_err().into_parts();
    assert_eq!(terminal, crate::PublicationCause::Control(cause));
    assert_eq!(
        snapshot.current_administrative_bytes(),
        expected_administration
    );
}

#[test]
fn terminal_capture_rejects_progress_credit_that_disagrees_with_its_actual_buffer() {
    for overcounted in [false, true] {
        let limits = LedgerLimits::new(0, []).unwrap();
        let mut coordinator =
            OperationCoordinator::<Owner, Phase, Phase, Unit, Dimension>::try_new(
                OperationControl::default(),
                limits,
                &[],
            )
            .unwrap();
        coordinator
            .report_progress(Phase::Final, Unit::Work, 1, None)
            .unwrap();
        let actual = crate::ledger::capacity_bytes(&coordinator.progress).unwrap();
        assert_eq!(coordinator.progress_evidence_bytes, actual);
        let expected_administration = coordinator.known_administrative_bytes().unwrap();
        coordinator.progress_evidence_bytes = if overcounted { actual + 1 } else { actual - 1 };
        let (snapshot, cause) = coordinator.take_terminal_snapshot();
        assert_eq!(cause, Some(LedgerCause::FinalSnapshotShapeMismatch));
        assert_eq!(
            snapshot.current_administrative_bytes(),
            expected_administration
        );
        assert!(!snapshot.has_exact_administrative_coverage());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ReportPhase {
    Prepare,
    Finish,
}

impl ProgressPhase for ReportPhase {
    fn order(self) -> u32 {
        match self {
            Self::Prepare => 0,
            Self::Finish => 1,
        }
    }

    fn stable_id(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Finish => "finish",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ReportUnit {
    First,
    Second,
    Third,
    Fourth,
    Fifth,
}

impl ProgressUnit for ReportUnit {
    fn stable_id(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Second => "second",
            Self::Third => "third",
            Self::Fourth => "fourth",
            Self::Fifth => "fifth",
        }
    }
}

fn finite_progress_ledger() -> FiniteLedger<Owner, u8, u8> {
    FiniteLedger::try_new(
        FiniteLedgerLimits::try_new(
            0,
            0,
            [6, 2, 5, 1, 4, 3].map(|dimension| (dimension, DimensionPolicy::counter(16, 16, 0))),
        )
        .unwrap(),
        [3, 1, 2],
    )
    .unwrap()
}

#[test]
fn finite_progress_owns_four_slots_without_consuming_allocation_or_work_credit() {
    let _guard = test_evidence_allocation_guard();
    let mut ledger = finite_progress_ledger();
    let mut dynamic = OperationCoordinator::<Owner, u8, ReportPhase, ReportUnit, u8>::try_new(
        OperationControl::default(),
        LedgerLimits::new(0, []).unwrap(),
        &[],
    )
    .unwrap();
    let mut control = OperationControl::default();
    let mut terminal = None;
    let mut fixed = FiniteProgress4::new();
    let mut delivery = ProgressDeliveryState::default();
    assert_eq!(fixed.len, 0);
    assert!(fixed.rows.iter().all(Option::is_none));
    assert_eq!(ledger.known_administrative_bytes(0), Some(0));
    test_fail_next_evidence_allocation();
    let cause = {
        let mut poll = OperationPollCheckpoint {
            control: &mut control,
            terminal: &mut terminal,
            allow_preexisting_terminal: false,
        };
        let mut fields = OperationProgressFields {
            progress: ProgressRows::Finite(&mut fixed),
            delivery: &mut delivery,
        };
        let (mut work, mut storage) = ledger.view().into_internal();
        let layout = work.bind_ordinary([1]).unwrap();
        let mut frame = layout
            .admit_observed(&mut work.reborrow().unwrap().work, [2], WorkLane::Internal)
            .unwrap();
        frame
            .complete_batch_with_outcome(&layout, &mut work.reborrow().unwrap().work, [1])
            .into_result()
            .unwrap();
        for unit in [
            ReportUnit::Third,
            ReportUnit::First,
            ReportUnit::Fourth,
            ReportUnit::Second,
        ] {
            fields
                .report_progress(
                    &mut poll,
                    &mut storage,
                    ReportPhase::Prepare,
                    unit,
                    1,
                    Some(3),
                )
                .unwrap();
        }
        assert_eq!(fields.progress.len(), 4);
        fields
            .report_progress(
                &mut poll,
                &mut storage,
                ReportPhase::Prepare,
                ReportUnit::Second,
                2,
                Some(3),
            )
            .unwrap();
        assert_eq!(fields.progress.len(), 4);
        assert_eq!(fields.progress.get(1).unwrap().completed, 2);
        let expected = ControlCause::ProgressStorage {
            cause: ProgressStorageError::StateLimitExceeded { value: 5, limit: 4 },
        };
        assert_eq!(
            fields.report_progress(
                &mut poll,
                &mut storage,
                ReportPhase::Prepare,
                ReportUnit::Fifth,
                1,
                Some(3),
            ),
            Err(expected.clone())
        );
        assert_eq!(fields.progress.len(), 4);
        assert_eq!(
            fields
                .progress
                .find((ReportPhase::Prepare, ReportUnit::Fifth)),
            Err(4)
        );
        assert_eq!(poll.poll(), Err(expected.clone()));
        assert_eq!(work.reborrow().unwrap().counters(1).unwrap(), (2, 1, 1));
        assert_eq!(frame.remaining, [1]);
        frame
            .complete_batch_with_outcome(&layout, &mut work.reborrow().unwrap().work, [1])
            .into_result()
            .unwrap();
        work.resolve(layout, &mut frame, false).unwrap();
        work.reborrow().unwrap().check_quiescent().unwrap();
        expected
    };
    assert_eq!(terminal, Some(cause));
    assert_eq!(fixed.len, 4);
    assert!(fixed.rows.iter().all(Option::is_some));
    assert_eq!(delivery.next_sequence, 0);
    assert_eq!(ledger.known_administrative_bytes(0), Some(0));
    let allocation = dynamic
        .report_progress(ReportPhase::Prepare, ReportUnit::First, 1, None)
        .unwrap_err();
    assert!(matches!(
        allocation,
        ControlCause::ProgressStorage {
            cause: ProgressStorageError::Evidence {
                cause: LedgerEvidenceError::AllocationFailed {
                    purpose: crate::LedgerEvidencePurpose::Progress,
                    requested_bytes,
                }
            }
        } if requested_bytes > 0
    ));
    assert!(dynamic.progress.is_empty());
    assert_eq!(dynamic.progress.capacity(), 0);
    assert_eq!(dynamic.progress_evidence_bytes, 0);
    assert_eq!(dynamic.known_administrative_bytes(), Some(0));
}

#[test]
fn finite_progress_honors_two_state_limit_and_updates_the_same_key_at_capacity() {
    let mut ledger = finite_progress_ledger();
    let mut control = OperationControl::default()
        .with_progress_limits(crate::ProgressDeliveryLimits::default().with_max_states(2));
    let mut terminal = None;
    let mut fixed = FiniteProgress4::new();
    let mut delivery = ProgressDeliveryState::default();
    {
        let mut poll = OperationPollCheckpoint {
            control: &mut control,
            terminal: &mut terminal,
            allow_preexisting_terminal: false,
        };
        let mut fields = OperationProgressFields {
            progress: ProgressRows::Finite(&mut fixed),
            delivery: &mut delivery,
        };
        let (_work, mut storage) = ledger.view().into_internal();
        for unit in [ReportUnit::Second, ReportUnit::First] {
            fields
                .report_progress(
                    &mut poll,
                    &mut storage,
                    ReportPhase::Prepare,
                    unit,
                    1,
                    Some(4),
                )
                .unwrap();
        }
        fields
            .report_progress(
                &mut poll,
                &mut storage,
                ReportPhase::Prepare,
                ReportUnit::First,
                2,
                Some(4),
            )
            .unwrap();
        assert_eq!(fields.progress.len(), 2);
        assert_eq!(fields.progress.get(0).unwrap().completed, 2);
        let cause = ControlCause::ProgressStorage {
            cause: ProgressStorageError::StateLimitExceeded { value: 3, limit: 2 },
        };
        assert_eq!(
            fields.report_progress(
                &mut poll,
                &mut storage,
                ReportPhase::Prepare,
                ReportUnit::Third,
                1,
                Some(4),
            ),
            Err(cause.clone())
        );
        assert_eq!(
            fields.report_progress(
                &mut poll,
                &mut storage,
                ReportPhase::Prepare,
                ReportUnit::First,
                0,
                None,
            ),
            Err(cause.clone())
        );
        assert_eq!(poll.recorded_control_cause(), Some(cause));
    }
    let moved = fixed;
    assert_eq!(moved.len, 2);
    assert!(moved.rows[..2].iter().all(Option::is_some));
    assert!(moved.rows[2..].iter().all(Option::is_none));
    assert_eq!(moved.rows[0].unwrap().1.completed, 2);
    assert_eq!(ledger.known_administrative_bytes(0), Some(0));
}

#[test]
fn finite_progress_sorts_actual_rows_and_delivers_in_report_order() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&events);
    let mut control = OperationControl::default().with_progress_observer(move |event| {
        observed.lock().unwrap().push(event);
        ProgressDecision::Continue
    });
    let mut ledger = finite_progress_ledger();
    let mut terminal = None;
    let mut fixed = FiniteProgress4::new();
    let mut delivery = ProgressDeliveryState::default();
    let reports = [
        (ReportUnit::Third, 3),
        (ReportUnit::First, 1),
        (ReportUnit::Fourth, 4),
        (ReportUnit::Second, 2),
        (ReportUnit::Third, 7),
    ];
    {
        let mut poll = OperationPollCheckpoint {
            control: &mut control,
            terminal: &mut terminal,
            allow_preexisting_terminal: false,
        };
        let mut fields = OperationProgressFields {
            progress: ProgressRows::Finite(&mut fixed),
            delivery: &mut delivery,
        };
        let (_work, mut storage) = ledger.view().into_internal();
        for (unit, completed) in reports {
            fields
                .report_progress(
                    &mut poll,
                    &mut storage,
                    ReportPhase::Prepare,
                    unit,
                    completed,
                    Some(10),
                )
                .unwrap();
        }
        for (position, (unit, completed)) in [
            (ReportUnit::First, 1),
            (ReportUnit::Second, 2),
            (ReportUnit::Third, 7),
            (ReportUnit::Fourth, 4),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                fields.progress.find((ReportPhase::Prepare, unit)),
                Ok(position)
            );
            let state = fields.progress.get(position).unwrap();
            assert_eq!(state.completed, completed);
            assert_eq!(state.total, Some(10));
            assert_eq!(state.last_delivered, Some(completed));
        }
        assert_eq!(fields.progress.len(), 4);
    }
    assert_eq!(terminal, None);
    assert_eq!(delivery.delivered_events, 5);
    assert_eq!(delivery.next_sequence, 5);
    let events = events.lock().unwrap();
    assert_eq!(events.len(), reports.len());
    for (sequence, ((unit, completed), event)) in reports.into_iter().zip(events.iter()).enumerate()
    {
        assert_eq!(event.phase(), ReportPhase::Prepare);
        assert_eq!(event.unit(), unit);
        assert_eq!(event.completed(), completed);
        assert_eq!(event.total(), Some(10));
        assert_eq!(event.sequence(), u64::try_from(sequence).unwrap());
    }
    assert_eq!(ledger.known_administrative_bytes(0), Some(0));
}

#[test]
fn finite_progress_preserves_coalescing_delivery_limits_and_unconditional_final_delivery() {
    let event_bytes = estimated_event_bytes(ReportPhase::Prepare, ReportUnit::First);
    for limits in [
        crate::ProgressDeliveryLimits::new(1, usize::MAX, 5),
        crate::ProgressDeliveryLimits::new(usize::MAX, event_bytes, 5),
    ] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let mut control = OperationControl::default()
            .with_progress_limits(limits)
            .with_progress_observer(move |event| {
                observed.lock().unwrap().push(event);
                ProgressDecision::Continue
            });
        let mut ledger = finite_progress_ledger();
        let mut terminal = None;
        let mut fixed = FiniteProgress4::new();
        let mut delivery = ProgressDeliveryState::default();
        {
            let mut poll = OperationPollCheckpoint {
                control: &mut control,
                terminal: &mut terminal,
                allow_preexisting_terminal: false,
            };
            let mut fields = OperationProgressFields {
                progress: ProgressRows::Finite(&mut fixed),
                delivery: &mut delivery,
            };
            let (_work, mut storage) = ledger.view().into_internal();
            for completed in [1, 1, 2, 6, 10] {
                fields
                    .report_progress(
                        &mut poll,
                        &mut storage,
                        ReportPhase::Prepare,
                        ReportUnit::First,
                        completed,
                        Some(10),
                    )
                    .unwrap();
                if completed == 2 {
                    assert_eq!(events.lock().unwrap().len(), 1);
                    assert_eq!(fields.progress.get(0).unwrap().completed, 2);
                }
            }
            assert_eq!(fields.progress.get(0).unwrap().completed, 10);
            assert_eq!(fields.progress.get(0).unwrap().last_delivered, Some(1));
            fields
                .report_progress_with_delivery(
                    &mut poll,
                    &mut storage,
                    ReportPhase::Prepare,
                    ReportUnit::First,
                    10,
                    Some(10),
                    ProgressDelivery::Final,
                )
                .unwrap();
            assert_eq!(fields.progress.get(0).unwrap().last_delivered, Some(10));
        }
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!((events[0].completed(), events[0].sequence()), (1, 0));
        assert_eq!((events[1].completed(), events[1].sequence()), (10, 1));
        assert_eq!(delivery.delivered_events, 1);
        assert_eq!(delivery.delivered_bytes, event_bytes);
        assert_eq!(delivery.next_sequence, 2);
        assert_eq!(terminal, None);
        assert_eq!(ledger.known_administrative_bytes(0), Some(0));
    }
}

#[test]
fn finite_progress_rejects_monotonicity_violations_before_mutating_its_rows() {
    for (phase, completed, total, violation) in [
        (
            ReportPhase::Finish,
            1,
            Some(5),
            ProgressViolation::CompletedRegressed,
        ),
        (
            ReportPhase::Finish,
            2,
            Some(4),
            ProgressViolation::TotalRegressed,
        ),
        (
            ReportPhase::Finish,
            2,
            None,
            ProgressViolation::TotalRegressed,
        ),
        (
            ReportPhase::Finish,
            6,
            Some(5),
            ProgressViolation::CompletedExceededTotal,
        ),
        (
            ReportPhase::Prepare,
            3,
            Some(5),
            ProgressViolation::PhaseRegressed,
        ),
    ] {
        let mut ledger = finite_progress_ledger();
        let mut control = OperationControl::default();
        let mut terminal = None;
        let mut fixed = FiniteProgress4::new();
        let mut delivery = ProgressDeliveryState::default();
        let cause = ControlCause::InvalidProgress { violation };
        {
            let mut poll = OperationPollCheckpoint {
                control: &mut control,
                terminal: &mut terminal,
                allow_preexisting_terminal: false,
            };
            let mut fields = OperationProgressFields {
                progress: ProgressRows::Finite(&mut fixed),
                delivery: &mut delivery,
            };
            let (_work, mut storage) = ledger.view().into_internal();
            fields
                .report_progress(
                    &mut poll,
                    &mut storage,
                    ReportPhase::Finish,
                    ReportUnit::First,
                    2,
                    Some(5),
                )
                .unwrap();
            assert_eq!(
                fields.report_progress(
                    &mut poll,
                    &mut storage,
                    phase,
                    ReportUnit::First,
                    completed,
                    total,
                ),
                Err(cause.clone())
            );
            assert_eq!(fields.progress.len(), 1);
            let state = fields.progress.get(0).unwrap();
            assert_eq!(state.completed, 2);
            assert_eq!(state.total, Some(5));
            assert_eq!(state.last_delivered, None);
            assert_eq!(poll.poll(), Err(cause.clone()));
        }
        assert_eq!(terminal, Some(cause));
        assert_eq!(delivery.last_phase_order, Some(ReportPhase::Finish.order()));
        assert_eq!(ledger.known_administrative_bytes(0), Some(0));
    }
}

#[test]
fn finite_progress_contains_observer_panics_and_disposes_the_observer_once() {
    struct PanickingObserver {
        calls: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
        panic_on_drop: bool,
    }

    impl crate::ProgressObserver<ReportPhase, ReportUnit> for PanickingObserver {
        fn observe(&mut self, _: ProgressEvent<ReportPhase, ReportUnit>) -> ProgressDecision {
            self.calls.fetch_add(1, Ordering::SeqCst);
            panic!("finite progress observer failure");
        }
    }

    impl Drop for PanickingObserver {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
            if self.panic_on_drop {
                panic!("finite progress observer disposal failure");
            }
        }
    }

    for panic_on_drop in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let mut control = OperationControl::default().with_progress_observer(PanickingObserver {
            calls: Arc::clone(&calls),
            drops: Arc::clone(&drops),
            panic_on_drop,
        });
        let mut ledger = finite_progress_ledger();
        let mut terminal = None;
        let mut fixed = FiniteProgress4::new();
        let mut delivery = ProgressDeliveryState::default();
        {
            let mut poll = OperationPollCheckpoint {
                control: &mut control,
                terminal: &mut terminal,
                allow_preexisting_terminal: false,
            };
            let mut fields = OperationProgressFields {
                progress: ProgressRows::Finite(&mut fixed),
                delivery: &mut delivery,
            };
            let (_work, mut storage) = ledger.view().into_internal();
            assert_eq!(
                fields.report_progress(
                    &mut poll,
                    &mut storage,
                    ReportPhase::Prepare,
                    ReportUnit::First,
                    1,
                    Some(2),
                ),
                Err(ControlCause::ObserverPanicked)
            );
            assert_eq!(fields.progress.len(), 1);
            let state = fields.progress.get(0).unwrap();
            assert_eq!(state.completed, 1);
            assert_eq!(state.total, Some(2));
            assert_eq!(state.last_delivered, None);
            assert_eq!(poll.poll(), Err(ControlCause::ObserverPanicked));
            assert_eq!(
                fields.report_progress(
                    &mut poll,
                    &mut storage,
                    ReportPhase::Prepare,
                    ReportUnit::Second,
                    3,
                    Some(2),
                ),
                Err(ControlCause::ObserverPanicked)
            );
        }
        assert_eq!(terminal, Some(ControlCause::ObserverPanicked));
        assert!(control.observer.is_none());
        control.dispose_extensions().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(delivery.delivered_events, 0);
        assert_eq!(delivery.delivered_bytes, 0);
        assert_eq!(delivery.next_sequence, 0);
        assert_eq!(ledger.known_administrative_bytes(0), Some(0));
    }
}

#[test]
fn finite_progress_polls_control_before_validation_and_preserves_the_first_cause() {
    for deadline in [false, true] {
        let cancellation = crate::CancellationToken::new();
        let mut control = OperationControl::default()
            .with_cancellation(cancellation.clone())
            .with_progress_limits(crate::ProgressDeliveryLimits::default().with_max_states(1));
        let mut ledger = finite_progress_ledger();
        let mut terminal = None;
        let mut fixed = FiniteProgress4::new();
        let mut delivery = ProgressDeliveryState::default();
        let cause = if deadline {
            ControlCause::DeadlineExceeded
        } else {
            ControlCause::Cancelled {
                origin: CancellationOrigin::Signal,
            }
        };
        {
            let mut poll = OperationPollCheckpoint {
                control: &mut control,
                terminal: &mut terminal,
                allow_preexisting_terminal: false,
            };
            let mut fields = OperationProgressFields {
                progress: ProgressRows::Finite(&mut fixed),
                delivery: &mut delivery,
            };
            let (_work, mut storage) = ledger.view().into_internal();
            fields
                .report_progress(
                    &mut poll,
                    &mut storage,
                    ReportPhase::Finish,
                    ReportUnit::First,
                    1,
                    Some(2),
                )
                .unwrap();
            if deadline {
                poll.control.deadline = Some(Instant::now());
            } else {
                cancellation.cancel();
            }
            assert_eq!(
                fields.report_progress(
                    &mut poll,
                    &mut storage,
                    ReportPhase::Prepare,
                    ReportUnit::Second,
                    3,
                    Some(2),
                ),
                Err(cause.clone())
            );
            assert_eq!(fields.progress.len(), 1);
            assert_eq!(fields.progress.get(0).unwrap().completed, 1);
            cancellation.cancel();
            assert_eq!(poll.poll(), Err(cause.clone()));
            assert_eq!(poll.recorded_control_cause(), Some(cause.clone()));
        }
        assert_eq!(terminal, Some(cause));
        assert_eq!(fixed.len, 1);
        assert!(fixed.rows[1..].iter().all(Option::is_none));
        assert_eq!(delivery.last_phase_order, Some(ReportPhase::Finish.order()));
        assert_eq!(ledger.known_administrative_bytes(0), Some(0));
    }
}

#[test]
fn finite_final_progress_observer_cancellation_bypasses_transient_limits() {
    let cancellation = crate::CancellationToken::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let mut control = OperationControl::default()
        .with_cancellation(cancellation.clone())
        .with_progress_limits(crate::ProgressDeliveryLimits::new(0, 0, u64::MAX))
        .with_progress_observer(move |_: ProgressEvent<ReportPhase, ReportUnit>| {
            observed.fetch_add(1, Ordering::SeqCst);
            ProgressDecision::Cancel
        });
    let mut ledger = finite_progress_ledger();
    let mut terminal = None;
    let mut fixed = FiniteProgress4::new();
    let mut delivery = ProgressDeliveryState::default();
    let cause = ControlCause::Cancelled {
        origin: CancellationOrigin::Observer,
    };
    {
        let mut poll = OperationPollCheckpoint {
            control: &mut control,
            terminal: &mut terminal,
            allow_preexisting_terminal: false,
        };
        let mut fields = OperationProgressFields {
            progress: ProgressRows::Finite(&mut fixed),
            delivery: &mut delivery,
        };
        let (_work, mut storage) = ledger.view().into_internal();
        fields
            .report_progress(
                &mut poll,
                &mut storage,
                ReportPhase::Finish,
                ReportUnit::First,
                1,
                Some(1),
            )
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            fields.report_progress_with_delivery(
                &mut poll,
                &mut storage,
                ReportPhase::Finish,
                ReportUnit::First,
                1,
                Some(1),
                ProgressDelivery::Final,
            ),
            Err(cause.clone())
        );
        assert_eq!(fields.progress.get(0).unwrap().last_delivered, Some(1));
        cancellation.cancel();
        assert_eq!(poll.poll(), Err(cause.clone()));
    }
    assert_eq!(terminal, Some(cause));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(delivery.delivered_events, 0);
    assert_eq!(delivery.delivered_bytes, 0);
    assert_eq!(delivery.next_sequence, 1);
    control.dispose_extensions().unwrap();
    assert_eq!(ledger.known_administrative_bytes(0), Some(0));
}

#[test]
fn finite_final_progress_keeps_both_polls_and_observes_callback_cancellation() {
    struct CountedCancellation {
        token: crate::CancellationToken,
        polls: Arc<AtomicUsize>,
    }

    impl crate::CancellationSignal for CountedCancellation {
        fn is_cancelled(&self) -> bool {
            self.polls.fetch_add(1, Ordering::SeqCst);
            self.token.is_cancelled()
        }
    }

    let cancellation = crate::CancellationToken::new();
    let polls = Arc::new(AtomicUsize::new(0));
    let from_observer = cancellation.clone();
    let mut control = OperationControl::default()
        .with_cancellation(CountedCancellation {
            token: cancellation,
            polls: Arc::clone(&polls),
        })
        .with_progress_limits(crate::ProgressDeliveryLimits::new(0, 0, u64::MAX))
        .with_progress_observer(move |_: ProgressEvent<ReportPhase, ReportUnit>| {
            from_observer.cancel();
            ProgressDecision::Continue
        });
    let mut ledger = finite_progress_ledger();
    let mut terminal = None;
    let mut fixed = FiniteProgress4::new();
    let mut delivery = ProgressDeliveryState::default();
    let cause = ControlCause::Cancelled {
        origin: CancellationOrigin::Signal,
    };
    {
        let mut poll = OperationPollCheckpoint {
            control: &mut control,
            terminal: &mut terminal,
            allow_preexisting_terminal: false,
        };
        let mut fields = OperationProgressFields {
            progress: ProgressRows::Finite(&mut fixed),
            delivery: &mut delivery,
        };
        let (_work, mut storage) = ledger.view().into_internal();
        assert_eq!(
            fields
                .report_progress_with_delivery(
                    &mut poll,
                    &mut storage,
                    ReportPhase::Finish,
                    ReportUnit::First,
                    1,
                    Some(1),
                    ProgressDelivery::Final,
                )
                .and_then(|()| poll.poll()),
            Err(cause.clone())
        );
        assert_eq!(fields.progress.get(0).unwrap().last_delivered, Some(1));
        assert_eq!(polls.load(Ordering::SeqCst), 2);
        assert_eq!(poll.poll(), Err(cause.clone()));
        assert_eq!(polls.load(Ordering::SeqCst), 2);
    }
    assert_eq!(terminal, Some(cause));
    assert_eq!(delivery.next_sequence, 1);
    assert_eq!(ledger.known_administrative_bytes(0), Some(0));
}

#[test]
fn dynamic_progress_retains_its_actual_fifth_row_and_capacity_credit() {
    let mut coordinator = OperationCoordinator::<Owner, u8, ReportPhase, ReportUnit, u8>::try_new(
        OperationControl::default(),
        LedgerLimits::new(0, []).unwrap(),
        &[],
    )
    .unwrap();
    for unit in [
        ReportUnit::Fifth,
        ReportUnit::Second,
        ReportUnit::Fourth,
        ReportUnit::First,
        ReportUnit::Third,
    ] {
        coordinator
            .report_progress(ReportPhase::Prepare, unit, 1, Some(2))
            .unwrap();
    }
    assert_eq!(coordinator.progress.len(), 5);
    assert!(coordinator.progress.capacity() >= 5);
    let actual_bytes = crate::ledger::capacity_bytes(&coordinator.progress).unwrap();
    assert_eq!(coordinator.progress_evidence_bytes, actual_bytes);
    assert_eq!(coordinator.known_administrative_bytes(), Some(actual_bytes));
    for unit in [
        ReportUnit::First,
        ReportUnit::Second,
        ReportUnit::Third,
        ReportUnit::Fourth,
        ReportUnit::Fifth,
    ] {
        assert_eq!(
            coordinator.reported_progress(ReportPhase::Prepare, unit),
            Some((1, Some(2)))
        );
    }
    let snapshot = coordinator.finish().unwrap();
    assert_eq!(snapshot.current_administrative_bytes(), actual_bytes);
    assert!(snapshot.has_exact_administrative_coverage());
}

type ViewDynamicCoordinator = DynamicOperationCoordinator<Owner, u8, ReportPhase, ReportUnit, u8>;
type ViewFiniteCoordinator =
    finite::FiniteOperationCoordinator<Owner, u8, ReportPhase, ReportUnit, u8>;

fn coordinator_view_owners(
    dynamic_control: OperationControl<ReportPhase, ReportUnit>,
    finite_control: OperationControl<ReportPhase, ReportUnit>,
) -> (ViewDynamicCoordinator, ViewFiniteCoordinator) {
    let policies = [6, 2, 5, 1, 4, 3]
        .map(|dimension| (dimension, DimensionPolicy::reservable_bytes(32, 32, 0, 64)));
    (
        ViewDynamicCoordinator::try_new(
            dynamic_control,
            LedgerLimits::new(64, policies).unwrap(),
            &[3, 1, 2],
        )
        .unwrap(),
        ViewFiniteCoordinator::try_new(
            finite_control,
            FiniteLedgerLimits::try_new(64, 16_384, policies).unwrap(),
            [3, 1, 2],
        )
        .unwrap(),
    )
}

struct ViewCancellation {
    token: crate::CancellationToken,
    polls: Arc<AtomicUsize>,
}

impl crate::CancellationSignal for ViewCancellation {
    fn is_cancelled(&self) -> bool {
        self.polls.fetch_add(1, Ordering::SeqCst);
        self.token.is_cancelled()
    }
}

#[test]
fn coordinator_views_split_the_actual_progress_poll_work_and_storage_owners() {
    let polls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let control = || {
        let observed = Arc::clone(&events);
        OperationControl::default()
            .with_cancellation(ViewCancellation {
                token: crate::CancellationToken::new(),
                polls: Arc::clone(&polls),
            })
            .with_progress_observer(move |event: ProgressEvent<ReportPhase, ReportUnit>| {
                observed
                    .lock()
                    .unwrap()
                    .push((event.completed(), event.sequence()));
                ProgressDecision::Continue
            })
    };
    let (mut dynamic, mut finite) = coordinator_view_owners(control(), control());
    let reservations = [dynamic.view(), finite.view()].map(|mut view| {
        let entry_polls = polls.load(Ordering::SeqCst);
        view.poll().unwrap();
        assert_eq!(view.recorded_control_cause(), None);
        assert_eq!(view.ledger_read().dimension_count(), 6);
        view.reborrow()
            .report_progress(ReportPhase::Prepare, ReportUnit::First, 1, Some(2))
            .unwrap();
        let (mut poll, mut progress, ledger) = view.into_exclusive_progress_and_ledger();
        let (mut work, mut storage) = ledger.into_exclusive().unwrap();
        let layout = work.bind([1]).unwrap();
        let mut frame = layout
            .begin_parser(
                &mut work.work,
                [2],
                [None],
                crate::ledger::ParserChannel::Child,
            )
            .unwrap();
        frame.complete(&layout, &mut work.work, [1]).unwrap();
        let mut reservation = storage.reserve(1, 1, 7).unwrap();
        storage.resize(&mut reservation, 9).unwrap();
        poll.reborrow().poll().unwrap();
        progress
            .report_progress(
                &mut poll,
                &mut storage,
                ReportPhase::Prepare,
                ReportUnit::First,
                2,
                Some(2),
            )
            .unwrap();
        assert_eq!(work.counters(1).unwrap(), (2, 1, 0));
        assert_eq!(frame.remaining, [1]);
        assert_eq!(progress.progress.len(), 1);
        assert_eq!(storage.planning_state().current_live_bytes(), 9);
        frame.complete(&layout, &mut work.work, [1]).unwrap();
        frame.resolve(&mut work.work).unwrap();
        work.check_quiescent().unwrap();
        assert_eq!(polls.load(Ordering::SeqCst), entry_polls + 4);
        reservation
    });
    dynamic.validate_live_reservation(&reservations[0]).unwrap();
    assert_eq!(
        dynamic.validate_live_reservation(&reservations[1]),
        Err(LedgerCause::OwnerMismatch)
    );
    {
        let (_, storage) = finite.retained_ledger_mut().view().into_internal();
        storage.validate_live_owner(&reservations[1]).unwrap();
        assert_eq!(
            storage.validate_live_owner(&reservations[0]),
            Err(LedgerCause::OwnerMismatch)
        );
    }
    for view in [dynamic.view(), finite.view()] {
        let evidence = view.ledger_read().dimension_evidence(1).unwrap();
        assert_eq!((evidence.attempted(), evidence.consumed()), (2, 2));
        assert_eq!(evidence.current_live_bytes(), 9);
    }
    drop(reservations);
    for view in [dynamic.view(), finite.view()] {
        let evidence = view.ledger_read().dimension_evidence(1).unwrap();
        assert_eq!(evidence.current_live_bytes(), 0);
        assert_eq!(evidence.peak_live_bytes(), 9);
    }
    assert_eq!(
        dynamic.reported_progress(ReportPhase::Prepare, ReportUnit::First),
        Some((2, Some(2)))
    );
    assert_eq!(
        finite.reported_progress(ReportPhase::Prepare, ReportUnit::First),
        Some((2, Some(2)))
    );
    assert_eq!(
        dynamic.progress_evidence_bytes,
        crate::ledger::capacity_bytes(&dynamic.progress).unwrap()
    );
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [(1, 0), (2, 1), (1, 0), (2, 1)]
    );
}

#[test]
fn consuming_coordinator_views_only_exempt_preexisting_terminal_causes() {
    let cause = ControlCause::Cancelled {
        origin: CancellationOrigin::Signal,
    };
    for preexisting in [false, true] {
        let polls = Arc::new(AtomicUsize::new(0));
        let tokens = [
            crate::CancellationToken::new(),
            crate::CancellationToken::new(),
        ];
        let control = |token: &crate::CancellationToken| {
            OperationControl::default().with_cancellation(ViewCancellation {
                token: token.clone(),
                polls: Arc::clone(&polls),
            })
        };
        let (mut dynamic, mut finite) =
            coordinator_view_owners(control(&tokens[0]), control(&tokens[1]));
        for (mut view, token) in [dynamic.view(), finite.view()].into_iter().zip(tokens) {
            let entry_polls = polls.load(Ordering::SeqCst);
            if preexisting {
                token.cancel();
                assert_eq!(view.poll(), Err(cause.clone()));
            }
            {
                let (mut poll, ledger) = view.reborrow().into_poll_and_ledger();
                assert_eq!(
                    poll.poll(),
                    if preexisting {
                        Err(cause.clone())
                    } else {
                        Ok(())
                    }
                );
                ledger.check_work_quiescent().unwrap();
            }
            let (mut poll, ledger) = view.into_terminal_poll_and_ledger();
            token.cancel();
            let expected = if preexisting {
                Ok(())
            } else {
                Err(cause.clone())
            };
            assert_eq!(poll.poll(), expected);
            assert_eq!(poll.poll(), expected);
            assert_eq!(poll.recorded_control_cause(), Some(cause.clone()));
            ledger.check_work_quiescent().unwrap();
            assert_eq!(
                polls.load(Ordering::SeqCst),
                entry_polls + if preexisting { 1 } else { 2 }
            );
        }
        assert_eq!(dynamic.recorded_control_cause(), Some(cause.clone()));
        assert_eq!(finite.recorded_control_cause(), Some(cause.clone()));
    }
}

#[test]
fn coordinator_view_final_control_keeps_both_polls_and_never_replays_delivery() {
    for cancel in [false, true] {
        let polls = Arc::new(AtomicUsize::new(0));
        let events = Arc::new(AtomicUsize::new(0));
        let control = || {
            let token = crate::CancellationToken::new();
            let observer_token = token.clone();
            let observed = Arc::clone(&events);
            OperationControl::default()
                .with_cancellation(ViewCancellation {
                    token,
                    polls: Arc::clone(&polls),
                })
                .with_progress_observer(move |_: ProgressEvent<ReportPhase, ReportUnit>| {
                    observed.fetch_add(1, Ordering::SeqCst);
                    if cancel {
                        observer_token.cancel();
                    }
                    ProgressDecision::Continue
                })
        };
        let (mut dynamic, mut finite) = coordinator_view_owners(control(), control());
        for mut view in [dynamic.view(), finite.view()] {
            let entry_polls = polls.load(Ordering::SeqCst);
            let entry_events = events.load(Ordering::SeqCst);
            let cause = cancel.then_some(ControlCause::Cancelled {
                origin: CancellationOrigin::Signal,
            });
            let expected = cause
                .clone()
                .map_or(Ok(()), |cause| Err(PublicationCause::Control(cause)));
            assert_eq!(
                view.try_prepare_typed_control(FinalProgress::new(
                    ReportPhase::Finish,
                    ReportUnit::First,
                    2
                )),
                expected
            );
            assert_eq!(view.recorded_control_cause(), cause);
            assert_eq!(
                view.ledger_mut().ensure_work_open(),
                Err(LedgerCause::PublicationAlreadyPending)
            );
            view.close_for_terminal_extraction().unwrap();
            assert_eq!(
                view.ledger_mut().ensure_work_open(),
                Err(LedgerCause::PublicationAlreadyCompleted)
            );
            assert_eq!(
                view.try_prepare_typed_control(FinalProgress::new(
                    ReportPhase::Prepare,
                    ReportUnit::Second,
                    9
                )),
                expected
            );
            assert_eq!(polls.load(Ordering::SeqCst), entry_polls + 2);
            assert_eq!(events.load(Ordering::SeqCst), entry_events + 1);
        }
        assert_eq!(
            dynamic.reported_progress(ReportPhase::Finish, ReportUnit::First),
            Some((2, Some(2)))
        );
        assert_eq!(
            finite.reported_progress(ReportPhase::Finish, ReportUnit::First),
            Some((2, Some(2)))
        );
        assert_eq!(
            dynamic.reported_progress(ReportPhase::Prepare, ReportUnit::Second),
            None
        );
        assert_eq!(
            finite.reported_progress(ReportPhase::Prepare, ReportUnit::Second),
            None
        );
    }
}
