use super::*;
use crate::DimensionPolicy;
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

    let mut producer = coordinator
        .ledger
        .try_admit_failure_work(Dimension::Protected, 4)
        .expect("one protected producer envelope");
    assert!(matches!(
        coordinator
            .ledger
            .try_admit_linear_work_batch(&[(Dimension::Ordinary, 0)]),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    let mut share = coordinator
        .ledger
        .try_admit_linear_work_batch(&[(Dimension::Protected, 1), (Dimension::Protected, 1)])
        .expect("protected internal clone-and-handle work coexists with its producer");
    producer
        .complete(0, 2)
        .expect("first physical producer prefix");
    share.complete(0, 1).expect("reference clone completed");
    share.complete(1, 1).expect("retained handle completed");
    share
        .finish()
        .expect("internal share explicitly closes its envelope");
    producer
        .complete(0, 2)
        .expect("remaining physical producer prefix");
    assert_eq!(
        coordinator.ledger.close_work(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    producer
        .finish()
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
            .try_admit_linear_work_batch(&[(Dimension::Protected, 0)]),
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
