use std::panic::{AssertUnwindSafe, catch_unwind};

use super::super::{LocalWork, WorkDimensionState, WorkEntry, WorkLane, WorkMode, WorkPolicy};
use super::*;
use crate::ledger::CounterClass;

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

fn group<const N: usize>(owner: &LocalWork<u8>, dimensions: [u8; N]) -> BoundWorkGroup<'_, u8, N> {
    owner.bind(dimensions, WorkGrant::Ordinary).unwrap()
}

fn counters<const N: usize>(owner: &LocalWork<u8>) -> [(u64, u64, u64); N] {
    let state = owner.borrow_state().unwrap();
    assert_eq!(state.rows.len(), N);
    std::array::from_fn(|index| {
        let row = state.rows[index].state;
        (row.attempted, row.consumed, row.pending_consumed)
    })
}

fn channels(owner: &LocalWork<u8>) -> (bool, bool, u64) {
    let state = owner.borrow_state().unwrap();
    (
        state.header.parser_parent_open,
        state.header.parser_child_open,
        state.header.open_envelopes,
    )
}

#[test]
fn entry_checks_each_item_after_its_attempt_and_before_later_lanes() {
    let work = owner([(1, 40, 40), (2, 5, 5)]);
    assert!(matches!(
        group(&work, [2, 1, 2, 1]).begin_parser_parent([2, 4, 4, 3], [None, Some(3), None, None],),
        Err(ParserWorkError::ItemLimit {
            index: 1,
            value: 4,
            limit: 3,
        })
    ));
    assert_eq!(counters(&work), [(4, 0, 0), (2, 0, 0)]);
    assert_eq!(channels(&work), (false, false, 0));
    work.check_quiescent().unwrap();
}

#[test]
fn attempted_limit_precedes_the_same_items_limit_and_retains_rejected_attempt() {
    let work = owner([(1, 40, 40), (2, 5, 5)]);
    assert!(matches!(
        group(&work, [2, 1]).begin_parser_child([6, 4], [Some(0), None]),
        Err(ParserWorkError::Ledger(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 2,
            value: 6,
            limit: 5,
        }))
    ));
    assert_eq!(counters(&work), [(0, 0, 0), (6, 0, 0)]);
    assert_eq!(channels(&work), (false, false, 0));
}

#[test]
fn later_child_entry_failure_retains_attempted_prefix_and_the_existing_parent() {
    let work = owner([(1, 20, 20), (2, 5, 5)]);
    let parent = group(&work, [1]).begin_parser_parent([3], [None]).unwrap();
    assert!(matches!(
        group(&work, [2, 1, 2, 1]).begin_parser_child([2, 4, 4, 3], [None; 4]),
        Err(ParserWorkError::Ledger(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 2,
            value: 6,
            limit: 5,
        }))
    ));
    assert_eq!(counters(&work), [(7, 0, 0), (6, 0, 0)]);
    assert_eq!(channels(&work), (true, false, 0));
    parent.finish_partial().unwrap();
    work.check_quiescent().unwrap();
}

#[test]
fn attempted_overflow_preserves_the_last_valid_row_and_earlier_lanes() {
    let work = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    work.borrow_state_mut().unwrap().rows[1].state.attempted = u64::MAX - 1;
    assert!(matches!(
        group(&work, [1, 2]).begin_parser_parent([3, 2], [None, Some(0)]),
        Err(ParserWorkError::Ledger(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 2,
        }))
    ));
    assert_eq!(counters(&work), [(3, 0, 0), (u64::MAX - 1, 0, 0)]);
    assert_eq!(channels(&work), (false, false, 0));
}

#[test]
fn item_limits_apply_to_each_original_lane_even_for_duplicate_rows() {
    let work = owner([(1, 20, 20)]);
    let mut scope = group(&work, [1, 1])
        .begin_parser_child([3, 4], [Some(3), Some(4)])
        .unwrap();
    assert_eq!(counters(&work), [(7, 0, 0)]);
    assert_eq!(channels(&work), (false, true, 0));
    scope.complete([2, 1]).unwrap();
    assert_eq!(scope.remaining(), [1, 3]);
    assert_eq!(counters(&work), [(7, 3, 0)]);
    scope.complete_all().unwrap();
    assert_eq!(counters(&work), [(7, 7, 0)]);
    scope.finish_exact().unwrap();
    work.check_quiescent().unwrap();
}

#[test]
fn failing_direct_group_preserves_every_counter_and_remaining_lane() {
    let work = owner([(1, 20, 1), (2, 20, 0)]);
    let mut scope = group(&work, [2, 1, 2])
        .begin_parser_child([3, 4, 2], [None; 3])
        .unwrap();
    assert_eq!(
        scope.complete([2, 2, 1]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 2,
            limit: 1,
        })
    );
    assert_eq!(scope.remaining(), [3, 4, 2]);
    assert_eq!(counters(&work), [(4, 0, 0), (5, 0, 0)]);
    scope.finish_partial().unwrap();
    work.check_quiescent().unwrap();
}

#[test]
fn completion_overrun_cannot_use_a_different_lanes_attempted_authority() {
    let work = owner([(1, 20, 20)]);
    let mut scope = group(&work, [1, 1])
        .begin_parser_child([1, 5], [None; 2])
        .unwrap();
    assert_eq!(
        scope.complete([2, 0]),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(scope.remaining(), [1, 5]);
    assert_eq!(counters(&work), [(6, 0, 0)]);
    scope.complete([1, 2]).unwrap();
    scope.finish_partial().unwrap();
    assert_eq!(counters(&work), [(6, 3, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn consumed_overflow_in_a_later_row_does_not_commit_an_earlier_row() {
    let work = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    let mut scope = group(&work, [2, 1])
        .begin_parser_child([2, 2], [None; 2])
        .unwrap();
    {
        let mut state = work.borrow_state_mut().unwrap();
        state.rows[1].state.attempted = u64::MAX;
        state.rows[1].state.consumed = u64::MAX;
    }
    assert_eq!(
        scope.complete([1, 1]),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 2,
        })
    );
    assert_eq!(scope.remaining(), [2, 2]);
    assert_eq!(counters(&work), [(2, 0, 0), (u64::MAX, u64::MAX, 0)]);
    scope.finish_partial().unwrap();
}

#[test]
fn direct_completion_accounts_for_internal_pending_after_policy_tightening() {
    let work = owner([(1, 20, 20)]);
    let roles = group(&work, [1]);
    let mut scope = roles.begin_parser_child([4], [None]).unwrap();
    let internal = roles.admit_observed([3], WorkLane::Internal).unwrap();
    work.borrow_state_mut().unwrap().rows[0]
        .policy
        .consumed_limit = 5;
    assert_eq!(
        scope.complete([3]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 6,
            limit: 5,
        })
    );
    assert_eq!(scope.remaining(), [4]);
    assert_eq!(counters(&work), [(7, 0, 3)]);
    scope.finish_partial().unwrap();
    assert_eq!(counters(&work), [(7, 0, 3)]);
    internal.finish_partial().unwrap();
    assert_eq!(counters(&work), [(7, 0, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn parent_sequential_children_and_internal_shares_use_the_same_rows() {
    let work = owner([(1, 40, 40)]);
    let roles = group(&work, [1]);
    let mut parent = roles.begin_parser_parent([3], [None]).unwrap();
    assert!(matches!(
        roles.begin_parser_parent([1], [None]),
        Err(ParserWorkError::Ledger(
            LedgerCause::InactiveLinearWorkPermit
        ))
    ));
    let mut child = roles.begin_parser_child([2], [None]).unwrap();
    assert!(matches!(
        roles.begin_parser_child([1], [None]),
        Err(ParserWorkError::Ledger(
            LedgerCause::InactiveLinearWorkPermit
        ))
    ));
    let mut internal = roles.admit_observed([2], WorkLane::Internal).unwrap();
    child.complete([1]).unwrap();
    internal.complete(0, 1).unwrap();
    assert_eq!(counters(&work), [(7, 2, 1)]);
    assert_eq!(channels(&work), (true, true, 1));
    internal.finish_partial().unwrap();
    child.complete_all().unwrap();
    child.finish_exact().unwrap();
    assert_eq!(channels(&work), (true, false, 0));
    let mut next = roles.begin_parser_child([1], [None]).unwrap();
    next.complete_all().unwrap();
    next.finish_exact().unwrap();
    parent.complete_all().unwrap();
    assert_eq!(channels(&work), (true, false, 0));
    parent.finish_exact().unwrap();
    assert_eq!(counters(&work), [(8, 7, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn parser_entry_rejects_an_active_producer_before_charging() {
    let work = owner([(1, 20, 20)]);
    let roles = group(&work, [1]);
    let producer = roles.admit_observed([2], WorkLane::Producer).unwrap();
    assert!(matches!(
        roles.begin_parser_parent([3], [None]),
        Err(ParserWorkError::Ledger(
            LedgerCause::InactiveLinearWorkPermit
        ))
    ));
    assert!(matches!(
        roles.begin_parser_child([3], [None]),
        Err(ParserWorkError::Ledger(
            LedgerCause::InactiveLinearWorkPermit
        ))
    ));
    assert_eq!(counters(&work), [(2, 0, 2)]);
    assert_eq!(channels(&work), (false, false, 1));
    producer.finish_partial().unwrap();
    work.check_quiescent().unwrap();
}

#[test]
fn standalone_unused_child_cannot_be_adopted_by_a_later_parent() {
    let work = owner([(1, 20, 20)]);
    let roles = group(&work, [1]);
    let child = roles.begin_parser_child([0], [None]).unwrap();
    assert!(matches!(
        roles.begin_parser_parent([0], [None]),
        Err(ParserWorkError::Ledger(
            LedgerCause::InactiveLinearWorkPermit
        ))
    ));
    assert!(matches!(
        roles.admit_observed([0], WorkLane::Producer),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    assert_eq!(counters(&work), [(0, 0, 0)]);
    assert_eq!(channels(&work), (false, true, 0));
    child.finish_exact().unwrap();
    roles
        .begin_parser_parent([0], [None])
        .unwrap()
        .finish_exact()
        .unwrap();
    work.check_quiescent().unwrap();
}

#[test]
fn parent_cannot_complete_or_replenish_while_even_an_unused_zero_child_is_open() {
    let work = owner([(1, 20, 20)]);
    let roles = group(&work, [1]);
    let mut parent = roles.begin_parser_parent([0], [None]).unwrap();
    let child = roles.begin_parser_child([0], [None]).unwrap();
    assert_eq!(
        parent.complete([0]),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(
        parent.complete_all(),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(
        parent.replenish(2),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(parent.remaining(), [0]);
    assert_eq!(counters(&work), [(0, 0, 0)]);
    assert!(!work.borrow_state().unwrap().header.integrity_fault);
    child.finish_exact().unwrap();
    parent.replenish(2).unwrap();
    parent.complete_all().unwrap();
    parent.finish_exact().unwrap();
    assert_eq!(counters(&work), [(2, 2, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn parent_resolution_with_an_open_child_faults_and_closes_only_the_parent() {
    for exact in [false, true] {
        let work = owner([(1, 20, 20)]);
        let roles = group(&work, [1]);
        let parent = roles.begin_parser_parent([0], [None]).unwrap();
        let mut child = roles.begin_parser_child([0], [None]).unwrap();
        let result = if exact {
            parent.finish_exact()
        } else {
            parent.finish_partial()
        };
        assert_eq!(result, Err(LedgerCause::InactiveLinearWorkPermit));
        assert_eq!(channels(&work), (false, true, 0));
        assert!(work.borrow_state().unwrap().header.integrity_fault);
        assert_eq!(
            child.complete([0]),
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        );
        assert_eq!(
            child.finish_partial(),
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        );
        assert_eq!(channels(&work), (false, false, 0));
        assert_eq!(
            work.check_quiescent(),
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        );
    }
}

#[test]
fn forgetting_a_fully_completed_child_still_blocks_its_parent() {
    for amount in [0, 2] {
        let work = owner([(1, 20, 20)]);
        let roles = group(&work, [1]);
        let mut parent = roles.begin_parser_parent([3], [None]).unwrap();
        let mut child = roles.begin_parser_child([amount], [None]).unwrap();
        child.complete_all().unwrap();
        std::mem::forget(child);
        assert_eq!(
            parent.complete_all(),
            Err(LedgerCause::InactiveLinearWorkPermit)
        );
        assert_eq!(parent.remaining(), [3]);
        assert_eq!(counters(&work), [(3 + amount, amount, 0)]);
        drop(parent);
        assert_eq!(channels(&work), (false, true, 0));
        assert!(work.borrow_state().unwrap().header.integrity_fault);
        assert!(matches!(
            roles.begin_parser_parent([0], [None]),
            Err(ParserWorkError::Ledger(
                LedgerCause::FinalSnapshotShapeMismatch
            ))
        ));
        assert_eq!(
            work.check_quiescent(),
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        );
    }
}

#[test]
fn normal_parser_drop_preserves_prefixes_without_an_abandoned_producer_fault() {
    for channel in [ParserChannel::Parent, ParserChannel::Child] {
        let work = owner([(1, 20, 20)]);
        let mut scope = group(&work, [1])
            .begin_parser([4], [None], channel)
            .unwrap();
        scope.complete([2]).unwrap();
        drop(scope);
        assert_eq!(counters(&work), [(4, 2, 0)]);
        assert_eq!(channels(&work), (false, false, 0));
        work.check_quiescent().unwrap();
    }
}

#[test]
fn child_drop_does_not_close_its_parent_or_lose_completed_prefixes() {
    let work = owner([(1, 20, 20)]);
    let roles = group(&work, [1]);
    let parent = roles.begin_parser_parent([3], [None]).unwrap();
    let mut child = roles.begin_parser_child([2], [None]).unwrap();
    child.complete([1]).unwrap();
    drop(child);
    assert_eq!(channels(&work), (true, false, 0));
    parent.finish_partial().unwrap();
    assert_eq!(counters(&work), [(5, 1, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn unwind_closes_child_then_parent_and_allows_a_subsequent_parent() {
    let work = owner([(1, 20, 20)]);
    let roles = group(&work, [1]);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _parent = roles.begin_parser_parent([3], [None]).unwrap();
        let mut child = roles.begin_parser_child([2], [None]).unwrap();
        child.complete([1]).unwrap();
        panic!("parser action failed");
    }));
    assert!(result.is_err());
    assert_eq!(counters(&work), [(5, 1, 0)]);
    assert_eq!(channels(&work), (false, false, 0));
    work.check_quiescent().unwrap();
    let mut parent = roles.begin_parser_parent([2], [None]).unwrap();
    parent.complete_all().unwrap();
    parent.finish_exact().unwrap();
    assert_eq!(counters(&work), [(7, 3, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn incomplete_exact_finish_closes_the_channel_and_returns_the_raw_contract_error() {
    let work = owner([(1, 20, 20)]);
    let mut scope = group(&work, [1]).begin_parser_child([4], [None]).unwrap();
    scope.complete([2]).unwrap();
    assert_eq!(
        scope.finish_exact(),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(channels(&work), (false, false, 0));
    assert_eq!(counters(&work), [(4, 2, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn forgetting_full_or_zero_parser_scopes_preserves_persistent_ownership() {
    for channel in [ParserChannel::Parent, ParserChannel::Child] {
        for amount in [0, 2] {
            let work = owner([(1, 20, 20)]);
            let roles = group(&work, [1]);
            let mut scope = roles.begin_parser([amount], [None], channel).unwrap();
            scope.complete_all().unwrap();
            assert_eq!(scope.remaining(), [0]);
            std::mem::forget(scope);
            assert!(channel.is_open(&work.borrow_state().unwrap().header));
            assert_eq!(work.borrow_state().unwrap().header.open_envelopes, 0);
            assert_eq!(counters(&work), [(amount, amount, 0)]);
            assert_eq!(
                work.check_quiescent(),
                Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
            );
            assert!(matches!(
                roles.admit_observed([0], WorkLane::Producer),
                Err(LedgerCause::InactiveLinearWorkPermit)
            ));
        }
    }
}

#[test]
fn empty_parser_groups_also_require_explicit_channel_resolution() {
    for channel in [ParserChannel::Parent, ParserChannel::Child] {
        let work = owner([(1, 0, 0)]);
        let roles = group(&work, []);
        let scope = roles.begin_parser([], [], channel).unwrap();
        assert_eq!(scope.remaining(), []);
        scope.finish_exact().unwrap();
        work.check_quiescent().unwrap();
        std::mem::forget(roles.begin_parser([], [], channel).unwrap());
        assert_eq!(
            work.check_quiescent(),
            Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        );
    }
}

#[test]
fn drop_borrow_conflict_latches_the_existing_fault_and_guesses_no_release() {
    let work = owner([(1, 20, 20)]);
    let roles = group(&work, [1]);
    let scope = roles.begin_parser_child([3], [None]).unwrap();
    let internal = roles.admit_observed([2], WorkLane::Internal).unwrap();
    let state = work.borrow_state_mut().unwrap();
    drop(scope);
    assert!(work.borrow_fault.get());
    assert!(state.header.parser_child_open);
    assert_eq!(state.header.open_envelopes, 1);
    assert_eq!(state.rows[0].state.attempted, 5);
    assert_eq!(state.rows[0].state.consumed, 0);
    assert_eq!(state.rows[0].state.pending_consumed, 2);
    drop(state);
    internal.finish_partial().unwrap();
    assert_eq!(channels(&work), (false, true, 0));
    assert_eq!(counters(&work), [(5, 0, 0)]);
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
}

#[test]
fn replenish_requires_the_prior_chunk_complete_and_preserves_exact_prefixes() {
    let work = owner([(1, 20, 20)]);
    let mut scope = group(&work, [1]).begin_parser_child([2], [None]).unwrap();
    scope.complete([1]).unwrap();
    assert_eq!(
        scope.replenish(3),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(scope.remaining(), [1]);
    assert_eq!(counters(&work), [(2, 1, 0)]);
    scope.complete_all().unwrap();
    scope.replenish(3).unwrap();
    assert_eq!(scope.remaining(), [3]);
    assert_eq!(counters(&work), [(5, 2, 0)]);
    scope.complete([1]).unwrap();
    scope.finish_partial().unwrap();
    assert_eq!(counters(&work), [(5, 3, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn any_original_item_limit_disallows_replenish_even_for_a_zero_next_chunk() {
    for (amount, limit) in [(0, 0), (2, 10)] {
        let work = owner([(1, 20, 20)]);
        let mut scope = group(&work, [1])
            .begin_parser_child([amount], [Some(limit)])
            .unwrap();
        scope.complete_all().unwrap();
        assert_eq!(
            scope.replenish(0),
            Err(LedgerCause::InactiveLinearWorkPermit)
        );
        assert_eq!(scope.remaining(), [0]);
        assert_eq!(counters(&work), [(amount, amount, 0)]);
        scope.finish_exact().unwrap();
        work.check_quiescent().unwrap();
    }
}

#[test]
fn rejected_replenish_keeps_representable_attempts_without_installing_a_new_chunk() {
    let work = owner([(1, 4, 4)]);
    let mut scope = group(&work, [1]).begin_parser_child([2], [None]).unwrap();
    scope.complete_all().unwrap();
    assert_eq!(
        scope.replenish(3),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 1,
            value: 5,
            limit: 4,
        })
    );
    assert_eq!(scope.remaining(), [0]);
    assert_eq!(counters(&work), [(5, 2, 0)]);
    assert_eq!(channels(&work), (false, true, 0));
    scope.finish_partial().unwrap();
}

#[test]
fn replenish_overflow_leaves_the_last_representable_counter_unchanged() {
    let work = owner([(1, u64::MAX, u64::MAX)]);
    let mut scope = group(&work, [1])
        .begin_parser_child([u64::MAX], [None])
        .unwrap();
    scope.complete_all().unwrap();
    assert_eq!(
        scope.replenish(1),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 1,
        })
    );
    assert_eq!(scope.remaining(), [0]);
    assert_eq!(counters(&work), [(u64::MAX, u64::MAX, 0)]);
    scope.finish_exact().unwrap();
    work.check_quiescent().unwrap();
}

#[test]
fn parser_methods_recheck_current_mode_even_for_zero_amounts() {
    for mode in [
        WorkMode::OrdinarySealed,
        WorkMode::Protected,
        WorkMode::Closed,
    ] {
        let work = owner([(1, 20, 20)]);
        let roles = group(&work, [1]);
        let mut scope = roles.begin_parser_child([0], [None]).unwrap();
        work.borrow_state_mut().unwrap().header.mode = mode;
        let expected = if mode == WorkMode::Closed {
            LedgerCause::PublicationAlreadyCompleted
        } else {
            LedgerCause::PublicationAlreadyPending
        };
        assert_eq!(scope.complete([0]), Err(expected.clone()));
        assert_eq!(scope.replenish(0), Err(expected.clone()));
        assert!(matches!(
            roles.begin_parser_parent([0], [None]),
            Err(ParserWorkError::Ledger(cause)) if cause == expected
        ));
        assert!(matches!(
            roles.begin_parser_child([0], [None]),
            Err(ParserWorkError::Ledger(cause)) if cause == expected
        ));
        assert_eq!(scope.finish_partial(), Err(expected));
        assert_eq!(channels(&work), (false, false, 0));
        assert_eq!(counters(&work), [(0, 0, 0)]);
        work.check_quiescent().unwrap();
    }
}

#[test]
fn incomplete_exact_finish_preserves_an_existing_mode_failure() {
    let work = owner([(1, 20, 20)]);
    let scope = group(&work, [1]).begin_parser_child([2], [None]).unwrap();
    work.borrow_state_mut().unwrap().header.mode = WorkMode::OrdinarySealed;
    assert_eq!(
        scope.finish_exact(),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert_eq!(channels(&work), (false, false, 0));
    assert_eq!(counters(&work), [(2, 0, 0)]);
}

#[test]
fn protected_grant_cannot_open_parser_channels() {
    let work = owner([(1, 20, 20)]);
    {
        let mut state = work.borrow_state_mut().unwrap();
        state.header.mode = WorkMode::Protected;
        state.header.protected_dimension = Some(0);
    }
    let roles = work.bind([1], WorkGrant::Protected).unwrap();
    assert!(matches!(
        roles.begin_parser_parent([0], [None]),
        Err(ParserWorkError::Ledger(
            LedgerCause::InactiveLinearWorkPermit
        ))
    ));
    assert!(matches!(
        roles.begin_parser_child([0], [None]),
        Err(ParserWorkError::Ledger(
            LedgerCause::InactiveLinearWorkPermit
        ))
    ));
    assert_eq!(channels(&work), (false, false, 0));
    assert_eq!(counters(&work), [(0, 0, 0)]);
}

#[test]
fn zero_completion_rechecks_bound_role_and_cleanup_preserves_its_cause() {
    let work = owner([(10, 20, 20), (30, 20, 20), (90, 20, 20)]);
    let mut scope = group(&work, [90]).begin_parser_child([0], [None]).unwrap();
    work.borrow_state_mut().unwrap().header.protected_dimension = Some(2);
    assert_eq!(
        scope.complete([0]),
        Err(LedgerCause::UnknownDimension { dimension: 90 })
    );
    assert_eq!(
        scope.replenish(0),
        Err(LedgerCause::UnknownDimension { dimension: 90 })
    );
    assert_eq!(
        scope.finish_partial(),
        Err(LedgerCause::UnknownDimension { dimension: 90 })
    );
    assert_eq!(channels(&work), (false, false, 0));
    assert_eq!(counters(&work), [(0, 0, 0); 3]);
}

#[test]
fn parser_numeric_methods_need_neither_debug_nor_ordering_on_dimension_keys() {
    #[derive(Clone, Copy)]
    struct Dimension(u8);

    let work = LocalWork::new(
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
    let roles = BoundWorkGroup {
        owner: &work,
        layout: super::super::WorkLayout {
            positions: [0],
            representatives: [0],
            completion_order: [0],
            grant: WorkGrant::Ordinary,
        },
    };
    let mut parent = match roles.begin_parser_parent([2], [None]) {
        Ok(scope) => scope,
        Err(_) => panic!("parent entry failed"),
    };
    let mut child = match roles.begin_parser_child([1], [None]) {
        Ok(scope) => scope,
        Err(_) => panic!("child entry failed"),
    };
    assert!(child.complete_all().is_ok());
    assert!(child.replenish(2).is_ok());
    assert!(child.complete([1]).is_ok());
    assert!(child.finish_partial().is_ok());
    assert!(parent.complete_all().is_ok());
    assert!(parent.finish_exact().is_ok());
    assert!(work.check_quiescent().is_ok());
    let state = match work.borrow_state() {
        Ok(state) => state,
        Err(_) => panic!("numeric observation failed"),
    };
    assert_eq!(state.rows[0].dimension.0, 1);
    assert_eq!(state.rows[0].state.attempted, 5);
    assert_eq!(state.rows[0].state.consumed, 4);
    assert_eq!(state.rows[0].state.pending_consumed, 0);
}
