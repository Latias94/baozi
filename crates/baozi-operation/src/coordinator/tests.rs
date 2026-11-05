use super::*;
use crate::{DimensionPolicy, ledger::WorkLane};
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
        .ledger
        .configure_protected_work(Dimension::Protected)
        .expect("protected role is configured before sealing");
    assert_eq!(
        coordinator.ledger.charge_attempted(Dimension::Protected, 0),
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
        coordinator.ledger.charge_attempted(Dimension::Ordinary, 1),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    coordinator
        .ledger
        .activate_protected_work(Dimension::Protected)
        .expect("a later generation rejection can still activate its finite grant");

    let (producer_layout, mut producer) = {
        let (mut internal, _) = coordinator.ledger.split_internal();
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
            .ledger
            .split_internal()
            .0
            .bind_share([Dimension::Ordinary]),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    let (share_layout, mut share) = {
        let (mut internal, _) = coordinator.ledger.split_internal();
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
        let (mut internal, _) = coordinator.ledger.split_internal();
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
        coordinator.ledger.close_work(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    assert_eq!(producer.remaining, [0]);
    coordinator
        .ledger
        .split_internal()
        .0
        .resolve(producer_layout, &mut producer, false)
        .expect("a completed producer must still explicitly finish");
    coordinator
        .ledger
        .close_work()
        .expect("all protected ownership is resolved");
    assert_eq!(
        coordinator
            .ledger
            .activate_protected_work(Dimension::Protected),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert!(matches!(
        coordinator
            .ledger
            .split_internal()
            .0
            .bind_share([Dimension::Protected]),
        Err(LedgerCause::PublicationAlreadyCompleted)
    ));
    let snapshot = coordinator
        .finish()
        .expect("closed work produces final evidence");
    let protected = snapshot
        .dimension(Dimension::Protected)
        .expect("protected work evidence");
    assert_eq!((protected.attempted(), protected.consumed()), (6, 6));
    assert_eq!(snapshot.current_evidence_bytes(), 0);
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
        let (mut poll, progress, ledger) = coordinator.split_exclusive_progress_and_ledger();
        let (mut work, storage) = ledger.exclusive().unwrap();
        let mut progress = progress.bind(storage.progress_evidence());
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
            .report_progress(&mut poll, Phase::Final, Unit::Work, 1, Some(2))
            .unwrap();
        assert_eq!(work.counters(Dimension::Ordinary).unwrap(), (2, 1, 0));
        assert_eq!(frame.remaining, [1]);
        assert_eq!(progress.progress.len(), 1);
        assert_eq!(
            *progress.progress_evidence_bytes,
            u64::try_from(
                progress.progress.capacity()
                    * std::mem::size_of::<((Phase, Unit), ProgressState)>()
            )
            .unwrap()
        );
        assert!(*progress.progress_evidence_bytes > 0);
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
    let snapshot = coordinator.finish().unwrap();
    assert_eq!(snapshot.current_evidence_bytes(), 0);
    assert!(snapshot.peak_evidence_bytes() >= retained);
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
        let (mut poll, progress, ledger) = coordinator.split_exclusive_progress_and_ledger();
        let (work, storage) = ledger.exclusive().unwrap();
        let mut progress = progress.bind(storage.progress_evidence());
        let cause = progress
            .report_progress(&mut poll, Phase::Final, Unit::Work, 1, None)
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
        assert!(progress.progress.is_empty());
        assert_eq!(*progress.progress_evidence_bytes, 0);
        assert_eq!(poll.poll(), Err(cause.clone()));
        work.check_quiescent().unwrap();
        cause
    };
    let (terminal, snapshot) = coordinator.finish().unwrap_err().into_parts();
    assert_eq!(terminal, crate::PublicationCause::Control(cause));
    assert_eq!(snapshot.current_evidence_bytes(), 0);
}
