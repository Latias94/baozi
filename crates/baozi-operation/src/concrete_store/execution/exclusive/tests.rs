//! Private host-boundary probes use an actual Ledger, not a shadow numeric implementation.
use super::*;
use crate::ledger::{Ledger, LedgerLimits};
use std::num::{NonZeroU32, NonZeroU64};

struct Owner;
impl OperationOwner for Owner {}

fn actual_ledger() -> (Ledger<Owner, u8, StoreDimensionKey>, StoreDimensionKey) {
    let catalog = StoreSemanticCatalogId::new(*b"baozi-test-store", NonZeroU32::new(1).unwrap());
    let domain = StoreKeyDomain::try_new(catalog).unwrap();
    let key = domain
        .try_admit_dimension(StoreDimensionSemanticId::new(
            catalog,
            NonZeroU64::new(1).unwrap(),
        ))
        .unwrap();
    let limits = LedgerLimits::new(u64::MAX, [(key, DimensionPolicy::counter(8, 8, 0))]).unwrap();
    (Ledger::try_new(limits, &[1]).unwrap(), key)
}

#[test]
fn host_completion_failure_cleans_the_suffix_before_any_result_is_disposed() {
    let (mut owner, key) = actual_ledger();
    let mut exclusive = owner.exclusive().unwrap();
    let layout = exclusive.bind([key]).unwrap();
    let frame = layout
        .admit_observed(&mut exclusive.work, [4], WorkLane::Producer)
        .unwrap();
    // The actual joint policy method installs the tighter limits before returning its current
    // pending-policy violation. This probes the same completion category as the legacy live alias.
    assert!(
        exclusive
            .tighten_policy(key, DimensionPolicy::counter(8, 2, 0))
            .is_err()
    );
    let physical = Cell::new(0);
    let failure = Cell::new(None);
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame,
        admitted: [4],
        failure: &failure,
    };
    physical.set(1);
    assert!(matches!(
        host.complete([1]),
        Err(LedgerCause::LimitExceeded {
            class: crate::CounterClass::Consumed,
            value: 4,
            limit: 2,
            ..
        })
    ));
    assert!(!host.frame.active);
    assert_eq!(host.frame.remaining, [3]);
    drop(host);
    assert_eq!(physical.get(), 1);
    assert_eq!(exclusive.counters(key).unwrap(), (4, 1, 0));
    exclusive.check_quiescent().unwrap();
    assert_eq!(failure.get(), Some(PreparationFailureCode::ledger()));
}

#[test]
fn unapplied_completion_failure_has_no_invented_consumption_and_no_later_abandonment() {
    let (mut owner, key) = actual_ledger();
    let mut exclusive = owner.exclusive().unwrap();
    let layout = exclusive.bind([key]).unwrap();
    let frame = layout
        .admit_observed(&mut exclusive.work, [1], WorkLane::Producer)
        .unwrap();
    let failure = Cell::new(None);
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame,
        admitted: [1],
        failure: &failure,
    };
    assert!(matches!(
        host.complete([2]),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    assert!(!host.frame.active);
    drop(host);
    assert_eq!(exclusive.counters(key).unwrap(), (1, 0, 0));
    exclusive.check_quiescent().unwrap();
    assert_eq!(failure.get(), Some(PreparationFailureCode::ledger()));
}

#[test]
fn an_internal_frame_does_not_claim_or_close_the_attempted_only_parser_channels() {
    let (mut owner, key) = actual_ledger();
    let mut exclusive = owner.exclusive().unwrap();
    let parent_layout = exclusive.bind([key]).unwrap();
    let mut parent = parent_layout
        .begin_parser(&mut exclusive.work, [2], [None], ParserChannel::Parent)
        .unwrap();
    let mut child = parent_layout
        .begin_parser(&mut exclusive.work, [1], [None], ParserChannel::Child)
        .unwrap();
    let layout = exclusive.bind([key, key]).unwrap();
    let frame = layout
        .admit_observed(&mut exclusive.work, [1, 1], WorkLane::Internal)
        .unwrap();
    let failure = Cell::new(None);
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame,
        admitted: [1, 1],
        failure: &failure,
    };
    host.complete([1, 0]).unwrap();
    host.complete([0, 1]).unwrap();
    host.finish().unwrap();
    child
        .complete(&parent_layout, &mut exclusive.work, [1])
        .unwrap();
    child.resolve(&mut exclusive.work).unwrap();
    parent
        .complete(&parent_layout, &mut exclusive.work, [2])
        .unwrap();
    parent.resolve(&mut exclusive.work).unwrap();
    assert_eq!(exclusive.counters(key).unwrap(), (5, 5, 0));
    exclusive.check_quiescent().unwrap();
    assert_eq!(failure.get(), None);
}
