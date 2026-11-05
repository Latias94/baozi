use super::super::kernel::check_quiescent;
use super::super::{
    LocalWork, ParserChannel, WorkDimensionState, WorkEntry, WorkGrant, WorkLane, WorkLayout,
    WorkMode, WorkPolicy, WorkView,
};
use super::*;
use crate::ledger::{CounterClass, LinearWorkCompletion};

fn owner<const N: usize>(policies: [(u8, u64, u64); N]) -> LocalWork<u8> {
    LocalWork::new(
        policies
            .iter()
            .map(|(dimension, _, _)| *dimension)
            .collect(),
        policies
            .into_iter()
            .map(|(dimension, attempted_limit, consumed_limit)| WorkEntry {
                dimension,
                policy: WorkPolicy {
                    attempted_limit,
                    consumed_limit,
                },
                state: WorkDimensionState::default(),
            })
            .collect(),
    )
}

fn layout<const N: usize>(view: &WorkView<'_, u8>, dimensions: [u8; N]) -> WorkLayout<N> {
    let layout = WorkLayout::bind(view.catalog, &dimensions, WorkGrant::Ordinary).unwrap();
    layout.validate_active(view).unwrap();
    layout
}

fn counters<const N: usize>(view: &WorkView<'_, u8>) -> [(u64, u64, u64); N] {
    assert_eq!(view.rows.len(), N);
    std::array::from_fn(|index| {
        let row = view.rows[index].state;
        (row.attempted, row.consumed, row.pending_consumed)
    })
}

fn channels(view: &WorkView<'_, u8>) -> (bool, bool, u64) {
    (
        view.header.parser_parent_open,
        view.header.parser_child_open,
        view.header.open_envelopes,
    )
}

#[test]
fn entry_checks_each_item_after_its_attempt_and_before_later_lanes() {
    let mut work = owner([(1, 40, 40), (2, 5, 5)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [2, 1, 2, 1]);
    assert!(matches!(
        roles.begin_parser(
            &mut view,
            [2, 4, 4, 3],
            [None, Some(3), None, None],
            ParserChannel::Parent,
        ),
        Err(ParserWorkError::ItemLimit {
            index: 1,
            value: 4,
            limit: 3,
        })
    ));
    assert_eq!(counters(&view), [(4, 0, 0), (2, 0, 0)]);
    assert_eq!(channels(&view), (false, false, 0));
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn attempted_limit_precedes_the_same_items_limit_and_retains_rejected_attempt() {
    let mut work = owner([(1, 40, 40), (2, 5, 5)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [2, 1]);
    assert!(matches!(
        roles.begin_parser(&mut view, [6, 4], [Some(0), None], ParserChannel::Child),
        Err(ParserWorkError::Ledger(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 2,
            value: 6,
            limit: 5,
        }))
    ));
    assert_eq!(counters(&view), [(0, 0, 0), (6, 0, 0)]);
    assert_eq!(channels(&view), (false, false, 0));
}

#[test]
fn later_child_entry_failure_retains_attempted_prefix_and_the_existing_parent() {
    let mut work = owner([(1, 20, 20), (2, 5, 5)]);
    let (_, mut view) = work.exclusive().unwrap();
    let parent_layout = layout(&view, [1]);
    let mut parent = parent_layout
        .begin_parser(&mut view, [3], [None], ParserChannel::Parent)
        .unwrap();
    let child_layout = layout(&view, [2, 1, 2, 1]);
    assert!(matches!(
        child_layout.begin_parser(&mut view, [2, 4, 4, 3], [None; 4], ParserChannel::Child),
        Err(ParserWorkError::Ledger(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 2,
            value: 6,
            limit: 5,
        }))
    ));
    assert_eq!(counters(&view), [(7, 0, 0), (6, 0, 0)]);
    assert_eq!(channels(&view), (true, false, 0));
    parent.resolve(&mut view).unwrap();
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn attempted_overflow_preserves_the_last_valid_row_and_earlier_lanes() {
    let mut work = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    let (_, mut view) = work.exclusive().unwrap();
    view.rows[1].state.attempted = u64::MAX - 1;
    let roles = layout(&view, [1, 2]);
    assert!(matches!(
        roles.begin_parser(&mut view, [3, 2], [None, Some(0)], ParserChannel::Parent),
        Err(ParserWorkError::Ledger(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 2,
        }))
    ));
    assert_eq!(counters(&view), [(3, 0, 0), (u64::MAX - 1, 0, 0)]);
    assert_eq!(channels(&view), (false, false, 0));
}

#[test]
fn item_limits_apply_to_each_original_lane_even_for_duplicate_rows() {
    let mut work = owner([(1, 20, 20)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [1, 1]);
    let mut frame = roles
        .begin_parser(&mut view, [3, 4], [Some(3), Some(4)], ParserChannel::Child)
        .unwrap();
    assert_eq!(counters(&view), [(7, 0, 0)]);
    assert_eq!(channels(&view), (false, true, 0));
    frame.complete(&roles, &mut view, [2, 1]).unwrap();
    assert_eq!(frame.remaining, [1, 3]);
    assert_eq!(counters(&view), [(7, 3, 0)]);
    frame.complete(&roles, &mut view, [1, 3]).unwrap();
    assert_eq!(frame.remaining, [0, 0]);
    assert_eq!(counters(&view), [(7, 7, 0)]);
    frame.resolve(&mut view).unwrap();
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn failing_direct_group_preserves_every_counter_and_remaining_lane() {
    let mut work = owner([(1, 20, 1), (2, 20, 0)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [2, 1, 2]);
    let mut frame = roles
        .begin_parser(&mut view, [3, 4, 2], [None; 3], ParserChannel::Child)
        .unwrap();
    assert_eq!(
        frame.complete(&roles, &mut view, [2, 2, 1]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 2,
            limit: 1,
        })
    );
    assert_eq!(frame.remaining, [3, 4, 2]);
    assert_eq!(counters(&view), [(4, 0, 0), (5, 0, 0)]);
    frame.resolve(&mut view).unwrap();
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn completion_overrun_cannot_use_a_different_lanes_attempted_authority() {
    let mut work = owner([(1, 20, 20)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [1, 1]);
    let mut frame = roles
        .begin_parser(&mut view, [1, 5], [None; 2], ParserChannel::Child)
        .unwrap();
    assert_eq!(
        frame.complete(&roles, &mut view, [2, 0]),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(frame.remaining, [1, 5]);
    assert_eq!(counters(&view), [(6, 0, 0)]);
    frame.complete(&roles, &mut view, [1, 2]).unwrap();
    frame.resolve(&mut view).unwrap();
    assert_eq!(counters(&view), [(6, 3, 0)]);
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn consumed_overflow_in_a_later_row_does_not_commit_an_earlier_row() {
    let mut work = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [2, 1]);
    let mut frame = roles
        .begin_parser(&mut view, [2, 2], [None; 2], ParserChannel::Child)
        .unwrap();
    view.rows[1].state.attempted = u64::MAX;
    view.rows[1].state.consumed = u64::MAX;
    assert_eq!(
        frame.complete(&roles, &mut view, [1, 1]),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 2,
        })
    );
    assert_eq!(frame.remaining, [2, 2]);
    assert_eq!(counters(&view), [(2, 0, 0), (u64::MAX, u64::MAX, 0)]);
    frame.resolve(&mut view).unwrap();
}

#[test]
fn direct_completion_accounts_for_internal_pending_after_policy_tightening() {
    let mut work = owner([(1, 20, 20)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [1]);
    let mut frame = roles
        .begin_parser(&mut view, [4], [None], ParserChannel::Child)
        .unwrap();
    let mut internal = roles
        .admit_observed(&mut view, [3], WorkLane::Internal)
        .unwrap();
    view.rows[0].policy.consumed_limit = 5;
    assert_eq!(
        frame.complete(&roles, &mut view, [3]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 6,
            limit: 5,
        })
    );
    assert_eq!(frame.remaining, [4]);
    assert_eq!(counters(&view), [(7, 0, 3)]);
    frame.resolve(&mut view).unwrap();
    assert_eq!(counters(&view), [(7, 0, 3)]);
    internal.resolve(&roles, &mut view, false).unwrap();
    assert_eq!(counters(&view), [(7, 0, 0)]);
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn parent_sequential_children_and_internal_shares_use_the_same_rows() {
    let mut work = owner([(1, 40, 40)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [1]);
    let mut parent = roles
        .begin_parser(&mut view, [3], [None], ParserChannel::Parent)
        .unwrap();
    assert!(matches!(
        roles.begin_parser(&mut view, [1], [None], ParserChannel::Parent),
        Err(ParserWorkError::Ledger(
            LedgerCause::InactiveLinearWorkPermit
        ))
    ));
    let mut child = roles
        .begin_parser(&mut view, [2], [None], ParserChannel::Child)
        .unwrap();
    assert!(matches!(
        roles.begin_parser(&mut view, [1], [None], ParserChannel::Child),
        Err(ParserWorkError::Ledger(
            LedgerCause::InactiveLinearWorkPermit
        ))
    ));
    let mut internal = roles
        .admit_observed(&mut view, [2], WorkLane::Internal)
        .unwrap();
    child.complete(&roles, &mut view, [1]).unwrap();
    assert!(matches!(
        internal.complete_batch_with_outcome(&roles, &mut view, [1]),
        LinearWorkCompletion::Completed
    ));
    assert_eq!(counters(&view), [(7, 2, 1)]);
    assert_eq!(channels(&view), (true, true, 1));
    internal.resolve(&roles, &mut view, false).unwrap();
    child.complete(&roles, &mut view, [1]).unwrap();
    child.resolve(&mut view).unwrap();
    assert_eq!(channels(&view), (true, false, 0));
    let mut next = roles
        .begin_parser(&mut view, [1], [None], ParserChannel::Child)
        .unwrap();
    next.complete(&roles, &mut view, [1]).unwrap();
    next.resolve(&mut view).unwrap();
    parent.complete(&roles, &mut view, [3]).unwrap();
    assert_eq!(channels(&view), (true, false, 0));
    parent.resolve(&mut view).unwrap();
    assert_eq!(counters(&view), [(8, 7, 0)]);
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn parser_entry_rejects_an_active_producer_before_charging() {
    let mut work = owner([(1, 20, 20)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [1]);
    let mut producer = roles
        .admit_observed(&mut view, [2], WorkLane::Producer)
        .unwrap();
    for channel in [ParserChannel::Parent, ParserChannel::Child] {
        assert!(matches!(
            roles.begin_parser(&mut view, [3], [None], channel),
            Err(ParserWorkError::Ledger(
                LedgerCause::InactiveLinearWorkPermit
            ))
        ));
    }
    assert_eq!(counters(&view), [(2, 0, 2)]);
    assert_eq!(channels(&view), (false, false, 1));
    producer.resolve(&roles, &mut view, false).unwrap();
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn standalone_unused_child_cannot_be_adopted_by_a_later_parent() {
    let mut work = owner([(1, 20, 20)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [1]);
    let mut child = roles
        .begin_parser(&mut view, [0], [None], ParserChannel::Child)
        .unwrap();
    assert!(matches!(
        roles.begin_parser(&mut view, [0], [None], ParserChannel::Parent),
        Err(ParserWorkError::Ledger(
            LedgerCause::InactiveLinearWorkPermit
        ))
    ));
    assert!(matches!(
        roles.admit_observed(&mut view, [0], WorkLane::Producer),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    assert_eq!(counters(&view), [(0, 0, 0)]);
    assert_eq!(channels(&view), (false, true, 0));
    child.resolve(&mut view).unwrap();
    let mut parent = roles
        .begin_parser(&mut view, [0], [None], ParserChannel::Parent)
        .unwrap();
    parent.resolve(&mut view).unwrap();
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn parent_cannot_complete_or_replenish_while_even_an_unused_zero_child_is_open() {
    let mut work = owner([(1, 20, 20)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [1]);
    let mut parent = roles
        .begin_parser(&mut view, [0], [None], ParserChannel::Parent)
        .unwrap();
    let mut child = roles
        .begin_parser(&mut view, [0], [None], ParserChannel::Child)
        .unwrap();
    assert_eq!(
        parent.complete(&roles, &mut view, [0]),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(
        parent.replenish(&roles, &mut view, 2),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(parent.remaining, [0]);
    assert_eq!(counters(&view), [(0, 0, 0)]);
    assert!(!view.header.integrity_fault);
    child.resolve(&mut view).unwrap();
    parent.replenish(&roles, &mut view, 2).unwrap();
    parent.complete(&roles, &mut view, [2]).unwrap();
    parent.resolve(&mut view).unwrap();
    assert_eq!(counters(&view), [(2, 2, 0)]);
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn parent_resolution_with_an_open_child_faults_and_closes_only_the_parent() {
    let mut work = owner([(1, 20, 20)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [1]);
    let mut parent = roles
        .begin_parser(&mut view, [0], [None], ParserChannel::Parent)
        .unwrap();
    let mut child = roles
        .begin_parser(&mut view, [0], [None], ParserChannel::Child)
        .unwrap();
    assert_eq!(
        parent.resolve(&mut view),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(channels(&view), (false, true, 0));
    assert!(view.header.integrity_fault);
    assert_eq!(
        roles.validate_active(&view),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    child.resolve(&mut view).unwrap();
    assert_eq!(channels(&view), (false, false, 0));
    assert_eq!(
        check_quiescent(view.rows, view.header),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
}

#[test]
fn forgetting_a_fully_completed_child_still_blocks_its_parent() {
    for amount in [0, 2] {
        let mut work = owner([(1, 20, 20)]);
        let (_, mut view) = work.exclusive().unwrap();
        let roles = layout(&view, [1]);
        let mut parent = roles
            .begin_parser(&mut view, [3], [None], ParserChannel::Parent)
            .unwrap();
        let mut child = roles
            .begin_parser(&mut view, [amount], [None], ParserChannel::Child)
            .unwrap();
        child.complete(&roles, &mut view, [amount]).unwrap();
        let _unresolved_child = std::mem::ManuallyDrop::new(child);
        assert_eq!(
            parent.complete(&roles, &mut view, [3]),
            Err(LedgerCause::InactiveLinearWorkPermit)
        );
        assert_eq!(parent.remaining, [3]);
        assert_eq!(counters(&view), [(3 + amount, amount, 0)]);
        assert_eq!(
            parent.resolve(&mut view),
            Err(LedgerCause::InactiveLinearWorkPermit)
        );
        assert_eq!(channels(&view), (false, true, 0));
        assert!(view.header.integrity_fault);
        assert_eq!(
            roles.validate_active(&view),
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        );
        assert_eq!(
            check_quiescent(view.rows, view.header),
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        );
    }
}

#[test]
fn partial_resolution_preserves_prefixes_without_an_abandoned_producer_fault() {
    for channel in [ParserChannel::Parent, ParserChannel::Child] {
        let mut work = owner([(1, 20, 20)]);
        let (_, mut view) = work.exclusive().unwrap();
        let roles = layout(&view, [1]);
        let mut frame = roles.begin_parser(&mut view, [4], [None], channel).unwrap();
        frame.complete(&roles, &mut view, [2]).unwrap();
        frame.resolve(&mut view).unwrap();
        assert_eq!(frame.remaining, [2]);
        assert!(!frame.active);
        assert!(!view.header.integrity_fault);
        assert_eq!(counters(&view), [(4, 2, 0)]);
        assert_eq!(channels(&view), (false, false, 0));
        check_quiescent(view.rows, view.header).unwrap();
    }
}

#[test]
fn partial_child_resolution_does_not_close_its_parent_or_lose_completed_prefixes() {
    let mut work = owner([(1, 20, 20)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [1]);
    let mut parent = roles
        .begin_parser(&mut view, [3], [None], ParserChannel::Parent)
        .unwrap();
    let mut child = roles
        .begin_parser(&mut view, [2], [None], ParserChannel::Child)
        .unwrap();
    child.complete(&roles, &mut view, [1]).unwrap();
    child.resolve(&mut view).unwrap();
    assert_eq!(channels(&view), (true, false, 0));
    parent.resolve(&mut view).unwrap();
    assert_eq!(counters(&view), [(5, 1, 0)]);
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn forgetting_full_or_zero_parser_frames_preserves_persistent_ownership() {
    for channel in [ParserChannel::Parent, ParserChannel::Child] {
        for amount in [0, 2] {
            let mut work = owner([(1, 20, 20)]);
            let (_, mut view) = work.exclusive().unwrap();
            let roles = layout(&view, [1]);
            let mut frame = roles
                .begin_parser(&mut view, [amount], [None], channel)
                .unwrap();
            frame.complete(&roles, &mut view, [amount]).unwrap();
            assert_eq!(frame.remaining, [0]);
            let _unresolved_frame = std::mem::ManuallyDrop::new(frame);
            assert!(channel.is_open(view.header));
            assert_eq!(view.header.open_envelopes, 0);
            assert_eq!(counters(&view), [(amount, amount, 0)]);
            assert_eq!(
                check_quiescent(view.rows, view.header),
                Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
            );
            assert!(matches!(
                roles.admit_observed(&mut view, [0], WorkLane::Producer),
                Err(LedgerCause::InactiveLinearWorkPermit)
            ));
        }
    }
}

#[test]
fn empty_parser_groups_also_require_explicit_channel_resolution() {
    for channel in [ParserChannel::Parent, ParserChannel::Child] {
        let mut work = owner([(1, 0, 0)]);
        let (_, mut view) = work.exclusive().unwrap();
        let roles = layout(&view, []);
        let mut frame = roles.begin_parser(&mut view, [], [], channel).unwrap();
        assert_eq!(frame.remaining, []);
        frame.resolve(&mut view).unwrap();
        check_quiescent(view.rows, view.header).unwrap();
        let _unresolved_frame =
            std::mem::ManuallyDrop::new(roles.begin_parser(&mut view, [], [], channel).unwrap());
        assert_eq!(
            check_quiescent(view.rows, view.header),
            Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        );
    }
}

#[test]
fn replenish_requires_the_prior_chunk_complete_and_preserves_exact_prefixes() {
    let mut work = owner([(1, 20, 20)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [1]);
    let mut frame = roles
        .begin_parser(&mut view, [2], [None], ParserChannel::Child)
        .unwrap();
    frame.complete(&roles, &mut view, [1]).unwrap();
    assert_eq!(
        frame.replenish(&roles, &mut view, 3),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(frame.remaining, [1]);
    assert_eq!(counters(&view), [(2, 1, 0)]);
    frame.complete(&roles, &mut view, [1]).unwrap();
    frame.replenish(&roles, &mut view, 3).unwrap();
    assert_eq!(frame.remaining, [3]);
    assert_eq!(counters(&view), [(5, 2, 0)]);
    frame.complete(&roles, &mut view, [1]).unwrap();
    frame.resolve(&mut view).unwrap();
    assert_eq!(counters(&view), [(5, 3, 0)]);
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn any_original_item_limit_disallows_replenish_even_for_a_zero_next_chunk() {
    for (amount, limit) in [(0, 0), (2, 10)] {
        let mut work = owner([(1, 20, 20)]);
        let (_, mut view) = work.exclusive().unwrap();
        let roles = layout(&view, [1]);
        let mut frame = roles
            .begin_parser(&mut view, [amount], [Some(limit)], ParserChannel::Child)
            .unwrap();
        frame.complete(&roles, &mut view, [amount]).unwrap();
        assert_eq!(
            frame.replenish(&roles, &mut view, 0),
            Err(LedgerCause::InactiveLinearWorkPermit)
        );
        assert_eq!(frame.remaining, [0]);
        assert_eq!(counters(&view), [(amount, amount, 0)]);
        frame.resolve(&mut view).unwrap();
        check_quiescent(view.rows, view.header).unwrap();
    }
}

#[test]
fn rejected_replenish_keeps_representable_attempts_without_installing_a_new_chunk() {
    let mut work = owner([(1, 4, 4)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [1]);
    let mut frame = roles
        .begin_parser(&mut view, [2], [None], ParserChannel::Child)
        .unwrap();
    frame.complete(&roles, &mut view, [2]).unwrap();
    assert_eq!(
        frame.replenish(&roles, &mut view, 3),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 1,
            value: 5,
            limit: 4,
        })
    );
    assert_eq!(frame.remaining, [0]);
    assert_eq!(counters(&view), [(5, 2, 0)]);
    assert_eq!(channels(&view), (false, true, 0));
    frame.resolve(&mut view).unwrap();
}

#[test]
fn replenish_overflow_leaves_the_last_representable_counter_unchanged() {
    let mut work = owner([(1, u64::MAX, u64::MAX)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [1]);
    let mut frame = roles
        .begin_parser(&mut view, [u64::MAX], [None], ParserChannel::Child)
        .unwrap();
    frame.complete(&roles, &mut view, [u64::MAX]).unwrap();
    assert_eq!(
        frame.replenish(&roles, &mut view, 1),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 1,
        })
    );
    assert_eq!(frame.remaining, [0]);
    assert_eq!(counters(&view), [(u64::MAX, u64::MAX, 0)]);
    frame.resolve(&mut view).unwrap();
    check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn layout_validation_rejects_nonordinary_modes_before_zero_work() {
    for mode in [
        WorkMode::OrdinarySealed,
        WorkMode::Protected,
        WorkMode::Closed,
    ] {
        let mut work = owner([(1, 20, 20)]);
        let (_, mut view) = work.exclusive().unwrap();
        let roles = layout(&view, [1]);
        let mut frame = roles
            .begin_parser(&mut view, [0], [None], ParserChannel::Child)
            .unwrap();
        view.header.mode = mode;
        let expected = if mode == WorkMode::Closed {
            LedgerCause::PublicationAlreadyCompleted
        } else {
            LedgerCause::PublicationAlreadyPending
        };
        assert_eq!(roles.validate_active(&view), Err(expected));
        frame.resolve(&mut view).unwrap();
        assert_eq!(channels(&view), (false, false, 0));
        assert_eq!(counters(&view), [(0, 0, 0)]);
        check_quiescent(view.rows, view.header).unwrap();
    }
}

#[test]
fn protected_grant_cannot_open_parser_channels() {
    let mut work = owner([(1, 20, 20)]);
    let (_, mut view) = work.exclusive().unwrap();
    view.header.mode = WorkMode::Protected;
    view.header.protected_dimension = Some(0);
    let roles = WorkLayout::bind(view.catalog, &[1], WorkGrant::Protected).unwrap();
    roles.validate_active(&view).unwrap();
    for channel in [ParserChannel::Parent, ParserChannel::Child] {
        assert!(matches!(
            roles.begin_parser(&mut view, [0], [None], channel),
            Err(ParserWorkError::Ledger(
                LedgerCause::InactiveLinearWorkPermit
            ))
        ));
    }
    assert_eq!(channels(&view), (false, false, 0));
    assert_eq!(counters(&view), [(0, 0, 0)]);
}

#[test]
fn layout_validation_rechecks_the_bound_role_before_zero_work() {
    let mut work = owner([(10, 20, 20), (30, 20, 20), (90, 20, 20)]);
    let (_, mut view) = work.exclusive().unwrap();
    let roles = layout(&view, [90]);
    let mut frame = roles
        .begin_parser(&mut view, [0], [None], ParserChannel::Child)
        .unwrap();
    view.header.protected_dimension = Some(2);
    assert_eq!(
        roles.validate_active(&view),
        Err(LedgerCause::UnknownDimension { dimension: 90 })
    );
    frame.resolve(&mut view).unwrap();
    assert_eq!(channels(&view), (false, false, 0));
    assert_eq!(counters(&view), [(0, 0, 0); 3]);
}

#[test]
fn parser_numeric_methods_need_neither_debug_nor_ordering_on_dimension_keys() {
    #[derive(Clone, Copy)]
    struct Dimension(u8);

    let mut work = LocalWork::new(
        vec![Dimension(1)],
        vec![WorkEntry {
            dimension: Dimension(1),
            policy: WorkPolicy {
                attempted_limit: 20,
                consumed_limit: 20,
            },
            state: WorkDimensionState::default(),
        }],
    );
    let (_, mut view) = match work.exclusive() {
        Ok(parts) => parts,
        Err(_) => panic!("exclusive entry failed"),
    };
    let roles = WorkLayout {
        positions: [0],
        representatives: [0],
        completion_order: [0],
        grant: WorkGrant::Ordinary,
    };
    let mut parent = match roles.begin_parser(&mut view, [2], [None], ParserChannel::Parent) {
        Ok(frame) => frame,
        Err(_) => panic!("parent entry failed"),
    };
    let mut child = match roles.begin_parser(&mut view, [1], [None], ParserChannel::Child) {
        Ok(frame) => frame,
        Err(_) => panic!("child entry failed"),
    };
    assert!(child.complete(&roles, &mut view, [1]).is_ok());
    assert!(child.replenish(&roles, &mut view, 2).is_ok());
    assert!(child.complete(&roles, &mut view, [1]).is_ok());
    assert!(child.resolve(&mut view).is_ok());
    assert!(parent.complete(&roles, &mut view, [2]).is_ok());
    assert!(parent.resolve(&mut view).is_ok());
    assert!(check_quiescent(view.rows, view.header).is_ok());
    assert_eq!(view.rows[0].dimension.0, 1);
    assert_eq!(view.rows[0].state.attempted, 5);
    assert_eq!(view.rows[0].state.consumed, 4);
    assert_eq!(view.rows[0].state.pending_consumed, 0);
}
