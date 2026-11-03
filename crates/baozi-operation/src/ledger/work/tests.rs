use std::cmp::Ordering;
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::kernel::check_effective;
use super::*;

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

fn counters<const N: usize>(owner: &LocalWork<u8>) -> [(u64, u64, u64); N] {
    let state = owner.borrow_state().unwrap();
    assert_eq!(state.rows.len(), N);
    std::array::from_fn(|index| {
        let row = state.rows[index].state;
        (row.attempted, row.consumed, row.pending_consumed)
    })
}

fn group<const N: usize>(owner: &LocalWork<u8>, dimensions: [u8; N]) -> BoundWorkGroup<'_, u8, N> {
    owner.bind(dimensions, WorkGrant::Ordinary).unwrap()
}

#[test]
fn construction_transfers_existing_buffers_and_preserves_capacities() {
    let mut catalog = Vec::with_capacity(7);
    catalog.push(1_u8);
    let mut rows = Vec::with_capacity(5);
    rows.push(WorkEntry {
        dimension: 1,
        policy: WorkPolicy {
            attempted_limit: 10,
            consumed_limit: 10,
        },
        state: WorkDimensionState::default(),
    });
    let catalog_pointer = catalog.as_ptr();
    let rows_pointer = rows.as_ptr();
    let catalog_capacity = catalog.capacity();
    let rows_capacity = rows.capacity();
    let work = LocalWork::new(catalog, rows);
    assert_eq!(work.catalogue().as_ptr(), catalog_pointer);
    assert_eq!(work.catalog.capacity(), catalog_capacity);
    let state = work.borrow_state().unwrap();
    assert_eq!(state.rows.as_ptr(), rows_pointer);
    assert_eq!(state.rows.capacity(), rows_capacity);
}

#[test]
fn consuming_owner_preserves_exact_rows_and_outstanding_ownership() {
    let work = owner([(1, 20, 20)]);
    let rows_pointer = work.borrow_state().unwrap().rows.as_ptr();
    let mut permit = group(&work, [1])
        .admit_observed([5], WorkLane::Producer)
        .unwrap();
    permit.complete(0, 2).unwrap();
    std::mem::forget(permit);
    let (_catalogue, state) = work.into_state();
    assert_eq!(state.rows.as_ptr(), rows_pointer);
    assert_eq!(
        state.rows[0].state,
        WorkDimensionState {
            attempted: 5,
            consumed: 2,
            pending_consumed: 3,
        }
    );
    assert_eq!(state.header.open_envelopes, 1);
    assert!(state.header.producer_open);
}

#[test]
fn consuming_owner_retains_prior_borrow_fault_without_replacing_numeric_state() {
    let work = owner([(1, 20, 20)]);
    group(&work, [1]).charge_observed_ordered([3]).unwrap();
    let guard = work.borrow_state_mut().unwrap();
    assert!(matches!(
        work.borrow_state(),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    ));
    drop(guard);
    let (_catalogue, state) = work.into_state();
    assert!(state.header.integrity_fault);
    assert_eq!(
        state.rows[0].state,
        WorkDimensionState {
            attempted: 3,
            consumed: 3,
            pending_consumed: 0,
        }
    );
}

#[test]
fn consuming_owner_detects_forgotten_exclusive_guard_and_keeps_last_counters() {
    let work = owner([(1, 20, 20)]);
    group(&work, [1]).charge_observed_ordered([4]).unwrap();
    std::mem::forget(work.borrow_state_mut().unwrap());
    assert!(!work.borrow_fault.get());
    let (_catalogue, state) = work.into_state();
    assert!(state.header.integrity_fault);
    assert_eq!(
        state.rows[0].state,
        WorkDimensionState {
            attempted: 4,
            consumed: 4,
            pending_consumed: 0,
        }
    );
}

#[test]
fn binding_validates_all_members_in_caller_order_before_mode() {
    let work = owner([(1, 10, 10)]);
    work.borrow_state_mut().unwrap().header.mode = WorkMode::OrdinarySealed;
    assert!(matches!(
        work.bind([3, 2], WorkGrant::Ordinary),
        Err(LedgerCause::UnknownDimension { dimension: 3 })
    ));
    assert!(matches!(
        work.bind([1], WorkGrant::Ordinary),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    assert_eq!(counters(&work), [(0, 0, 0)]);
}

#[test]
fn attempted_stops_in_caller_order_and_keeps_representable_limit_failure() {
    let work = owner([(1, 10, 10), (2, 5, 5)]);
    let roles = group(&work, [2, 1, 2, 1]);
    assert_eq!(
        roles.charge_attempted_in_order([2, 4, 4, 3]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 2,
            value: 6,
            limit: 5,
        })
    );
    assert_eq!(counters(&work), [(4, 0, 0), (6, 0, 0)]);
}

#[test]
fn attempted_overflow_keeps_last_valid_value_and_earlier_entries() {
    let work = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    work.borrow_state_mut().unwrap().rows[1].state.attempted = u64::MAX - 1;
    assert_eq!(
        group(&work, [1, 2]).charge_attempted_in_order([3, 2]),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 2
        })
    );
    assert_eq!(counters(&work), [(3, 0, 0), (u64::MAX - 1, 0, 0)]);
}

#[test]
fn direct_completion_sorts_bound_positions_and_commits_duplicates_atomically() {
    let work = owner([(1, 20, 1), (2, 20, 0)]);
    let roles = group(&work, [2, 1, 2]);
    roles.charge_attempted_in_order([3, 4, 2]).unwrap();
    assert_eq!(
        roles.consume_direct_atomic([2, 2, 1]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 2,
            limit: 1,
        })
    );
    assert_eq!(counters(&work), [(4, 0, 0), (5, 0, 0)]);
    {
        let mut state = work.borrow_state_mut().unwrap();
        state.rows[0].policy.consumed_limit = 20;
        state.rows[1].policy.consumed_limit = 20;
    }
    roles.consume_direct_atomic([2, 2, 1]).unwrap();
    assert_eq!(counters(&work), [(4, 2, 0), (5, 3, 0)]);
}

#[test]
fn direct_completion_accounts_for_other_permits_pending_capacity() {
    let work = owner([(1, 10, 10)]);
    let roles = group(&work, [1]);
    let mut permit = roles.admit_observed([4], WorkLane::Producer).unwrap();
    roles.charge_attempted_in_order([2]).unwrap();
    assert_eq!(
        roles.consume_direct_atomic([3]),
        Err(LedgerCause::ConsumedExceedsAttempted {
            dimension: 1,
            consumed: 7,
            attempted: 6,
        })
    );
    assert_eq!(counters(&work), [(6, 0, 4)]);
    roles.consume_direct_atomic([2]).unwrap();
    permit.complete(0, 4).unwrap();
    assert_eq!(counters(&work), [(6, 6, 0)]);
    permit.finish().unwrap();
    work.check_quiescent().unwrap();
}

#[test]
fn direct_overflow_in_later_dimension_leaves_entire_consumed_group_unchanged() {
    let work = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    {
        let mut state = work.borrow_state_mut().unwrap();
        state.rows[0].state.attempted = 2;
        state.rows[1].state = WorkDimensionState {
            attempted: u64::MAX,
            consumed: u64::MAX,
            pending_consumed: 0,
        };
    }
    assert_eq!(
        group(&work, [2, 1]).consume_direct_atomic([1, 2]),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 2
        })
    );
    assert_eq!(counters(&work), [(2, 0, 0), (u64::MAX, u64::MAX, 0)]);
}

#[test]
fn direct_and_preflight_preserve_earlier_group_limit_before_later_duplicate_overflow() {
    let work = owner([(1, 1, 1), (2, u64::MAX, u64::MAX)]);
    {
        let mut state = work.borrow_state_mut().unwrap();
        state.rows[0].state.attempted = 2;
        state.rows[1].state.attempted = u64::MAX;
    }
    let roles = group(&work, [2, 1, 2]);
    assert_eq!(
        roles.consume_direct_atomic([u64::MAX, 2, 1]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 2,
            limit: 1,
        })
    );
    let caller_first = group(&work, [1, 2, 2]);
    assert_eq!(
        caller_first.preflight_repeated_observed([1, u64::MAX, 1], NonZeroU64::new(1).unwrap()),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 1,
            value: 3,
            limit: 1,
        })
    );
    assert_eq!(counters(&work), [(2, 0, 0), (u64::MAX, 0, 0)]);
}

#[test]
fn ordered_observation_keeps_prior_pairs_and_the_failing_physical_prefix() {
    let work = owner([(1, 10, 10), (2, 10, 2)]);
    assert_eq!(
        group(&work, [1, 2, 1]).charge_observed_ordered([3, 4, 5]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 2,
            value: 4,
            limit: 2
        })
    );
    assert_eq!(counters(&work), [(3, 3, 0), (4, 4, 0)]);
}

#[test]
fn ordered_observation_stops_before_consumed_on_attempted_limit_failure() {
    let work = owner([(1, 10, 10), (2, 2, 2)]);
    assert_eq!(
        group(&work, [1, 2]).charge_observed_ordered([3, 4]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 2,
            value: 4,
            limit: 2
        })
    );
    assert_eq!(counters(&work), [(3, 3, 0), (4, 0, 0)]);
}

#[test]
fn consumed_observation_uses_caller_order_without_debiting_attempts() {
    let work = owner([(1, 10, 10), (2, 10, 10)]);
    group(&work, [1, 2])
        .charge_attempted_in_order([3, 4])
        .unwrap();
    assert_eq!(
        group(&work, [2, 1, 2]).consume_observed_in_order([2, 2, 3]),
        Err(LedgerCause::ConsumedExceedsAttempted {
            dimension: 2,
            consumed: 5,
            attempted: 4
        })
    );
    assert_eq!(counters(&work), [(3, 2, 0), (4, 2, 0)]);
}

#[test]
fn consumed_observation_retains_physical_prefix_on_pending_projection_failure() {
    let work = owner([(1, 10, 10)]);
    let roles = group(&work, [1]);
    let permit = roles.admit_observed([4], WorkLane::Producer).unwrap();
    assert_eq!(
        roles.consume_observed_in_order([1]),
        Err(LedgerCause::ConsumedExceedsAttempted {
            dimension: 1,
            consumed: 5,
            attempted: 4
        })
    );
    assert_eq!(counters(&work), [(4, 1, 4)]);
    permit.finish_partial().unwrap();
    assert_eq!(counters(&work), [(4, 1, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn consumed_observation_retains_limit_failure_but_not_unrepresentable_increment() {
    let work = owner([(1, u64::MAX, 2)]);
    let roles = group(&work, [1]);
    roles.charge_attempted_in_order([u64::MAX]).unwrap();
    assert_eq!(
        roles.consume_observed_in_order([3]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 3,
            limit: 2,
        })
    );
    assert_eq!(counters(&work), [(u64::MAX, 3, 0)]);
    assert_eq!(
        roles.consume_observed_in_order([u64::MAX]),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 1
        })
    );
    assert_eq!(counters(&work), [(u64::MAX, 3, 0)]);
}

#[test]
fn consumed_observation_stores_representable_prefix_before_effective_overflow() {
    let work = owner([(1, u64::MAX, u64::MAX)]);
    {
        let mut state = work.borrow_state_mut().unwrap();
        state.rows[0].state = WorkDimensionState {
            attempted: u64::MAX,
            consumed: u64::MAX - 1,
            pending_consumed: 2,
        };
    }
    assert_eq!(
        group(&work, [1]).consume_observed_in_order([1]),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 1
        })
    );
    assert_eq!(counters(&work), [(u64::MAX, u64::MAX, 2)]);
}

#[test]
fn repeated_preflight_aggregates_duplicates_without_changing_evidence() {
    let work = owner([(1, 11, 11), (2, 20, 20)]);
    let roles = group(&work, [2, 1, 1]);
    let repeat = NonZeroU64::new(3).unwrap();
    roles
        .preflight_repeated_observed([2, 2, 1], repeat)
        .unwrap();
    assert_eq!(
        roles.preflight_repeated_observed([2, 3, 1], repeat),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 1,
            value: 12,
            limit: 11
        })
    );
    assert_eq!(
        roles.preflight_repeated_observed([u64::MAX, 0, 0], repeat),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 2
        })
    );
    assert_eq!(counters(&work), [(0, 0, 0), (0, 0, 0)]);
}

#[test]
fn repeated_preflight_uses_current_pending_and_current_policy() {
    let work = owner([(1, 20, 20)]);
    let roles = group(&work, [1]);
    let permit = roles.admit_observed([4], WorkLane::Internal).unwrap();
    work.borrow_state_mut().unwrap().rows[0]
        .policy
        .consumed_limit = 7;
    assert_eq!(
        roles.preflight_repeated_observed([2], NonZeroU64::new(2).unwrap()),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 8,
            limit: 7
        })
    );
    assert_eq!(counters(&work), [(4, 0, 4)]);
    permit.finish_partial().unwrap();
}

#[test]
fn admission_keeps_every_representable_attempt_and_prefers_attempted_failure() {
    let work = owner([(1, 20, 0), (2, 3, 3)]);
    assert!(matches!(
        group(&work, [1, 2, 2]).admit_observed([2, 3, 2], WorkLane::Producer),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 2,
            value: 5,
            limit: 3
        })
    ));
    assert_eq!(counters(&work), [(2, 0, 0), (5, 0, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn consumed_admission_failure_keeps_attempts_without_claiming_a_lane() {
    let work = owner([(1, 10, 2)]);
    assert!(matches!(
        group(&work, [1, 1]).admit_observed([2, 1], WorkLane::Producer),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 3,
            limit: 2
        })
    ));
    assert_eq!(counters(&work), [(3, 0, 0)]);
    assert!(!work.borrow_state().unwrap().header.producer_open);
    work.check_quiescent().unwrap();
}

#[test]
fn admission_attempted_and_duplicate_overflow_are_atomic() {
    let work = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    work.borrow_state_mut().unwrap().rows[1].state.attempted = u64::MAX;
    assert!(matches!(
        group(&work, [1, 2]).admit_observed([3, 1], WorkLane::Internal),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 2
        })
    ));
    assert!(matches!(
        group(&work, [1, 1]).admit_observed([u64::MAX, 1], WorkLane::Internal),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 1
        })
    ));
    assert_eq!(counters(&work), [(0, 0, 0), (u64::MAX, 0, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn admission_pending_overflow_does_not_commit_an_earlier_projection() {
    let work = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    work.borrow_state_mut().unwrap().rows[1]
        .state
        .pending_consumed = u64::MAX;
    assert!(matches!(
        group(&work, [1, 2]).admit_observed([3, 1], WorkLane::Internal),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 2
        })
    ));
    assert_eq!(counters(&work), [(0, 0, 0), (0, 0, u64::MAX)]);
}

#[test]
fn duplicate_batch_completion_and_partial_finish_preserve_exact_prefixes() {
    let work = owner([(1, 20, 20), (2, 20, 20)]);
    let mut permit = group(&work, [2, 1, 2])
        .admit_observed([3, 4, 2], WorkLane::Producer)
        .unwrap();
    assert_eq!(counters(&work), [(4, 0, 4), (5, 0, 5)]);
    permit.complete_batch([1, 2, 1]).unwrap();
    assert_eq!(permit.remaining(), [2, 2, 1]);
    assert_eq!(counters(&work), [(4, 2, 2), (5, 2, 3)]);
    permit.finish_partial().unwrap();
    assert_eq!(counters(&work), [(4, 2, 0), (5, 2, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn completion_overrun_and_bad_index_do_not_advance_any_prefix() {
    let work = owner([(1, 20, 20)]);
    let mut permit = group(&work, [1, 1])
        .admit_observed([2, 3], WorkLane::Producer)
        .unwrap();
    assert!(matches!(
        permit.complete_batch_with_outcome([1, 4]),
        LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit)
    ));
    assert!(matches!(
        permit.complete_with_outcome(2, 0),
        LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit)
    ));
    assert_eq!(permit.remaining(), [2, 3]);
    assert_eq!(counters(&work), [(5, 0, 5)]);
    permit.complete_batch([2, 3]).unwrap();
    permit.finish().unwrap();
    work.check_quiescent().unwrap();
}

#[test]
fn internal_permits_compete_with_producer_and_retain_applied_limit_failures() {
    let work = owner([(1, 20, 20)]);
    let roles = group(&work, [1]);
    let mut producer = roles.admit_observed([4], WorkLane::Producer).unwrap();
    let mut internal = roles.admit_observed([2], WorkLane::Internal).unwrap();
    assert!(matches!(
        roles.admit_observed([1], WorkLane::Producer),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    roles.charge_attempted_in_order([2]).unwrap();
    roles.consume_direct_atomic([2]).unwrap();
    work.borrow_state_mut().unwrap().rows[0]
        .policy
        .consumed_limit = 5;
    assert!(matches!(
        producer.complete_with_outcome(0, 2),
        LinearWorkCompletion::CompletedWithLedgerFailure(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 8,
            limit: 5,
        })
    ));
    assert_eq!(producer.remaining(), [2]);
    assert_eq!(counters(&work), [(8, 4, 4)]);
    producer.finish_partial().unwrap();
    assert!(matches!(
        internal.complete_with_outcome(0, 1),
        LinearWorkCompletion::CompletedWithLedgerFailure(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 6,
            limit: 5,
        })
    ));
    assert_eq!(internal.remaining(), [1]);
    internal.finish_partial().unwrap();
    assert_eq!(counters(&work), [(8, 5, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn grouped_applied_failure_advances_every_duplicate_and_dimension_together() {
    let work = owner([(1, 20, 20), (2, 20, 20)]);
    let mut permit = group(&work, [2, 1, 2])
        .admit_observed([3, 4, 2], WorkLane::Internal)
        .unwrap();
    work.borrow_state_mut().unwrap().rows[0]
        .policy
        .consumed_limit = 3;
    assert!(matches!(
        permit.complete_batch_with_outcome([1, 2, 1]),
        LinearWorkCompletion::CompletedWithLedgerFailure(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 4,
            limit: 3,
        })
    ));
    assert_eq!(permit.remaining(), [2, 2, 1]);
    assert_eq!(counters(&work), [(4, 2, 2), (5, 2, 3)]);
    permit.finish_partial().unwrap();
    assert_eq!(counters(&work), [(4, 2, 0), (5, 2, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn corrupt_unrepresentable_group_completion_changes_no_counter_or_remaining() {
    let work = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    let mut permit = group(&work, [1, 2])
        .admit_observed([2, 1], WorkLane::Internal)
        .unwrap();
    {
        let mut state = work.borrow_state_mut().unwrap();
        state.rows[1].state.consumed = u64::MAX;
        state.rows[1].state.attempted = u64::MAX;
    }
    assert!(matches!(
        permit.complete_batch_with_outcome([1, 1]),
        LinearWorkCompletion::NotCompleted(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 2,
        })
    ));
    assert_eq!(permit.remaining(), [2, 1]);
    assert_eq!(counters(&work), [(2, 0, 2), (u64::MAX, u64::MAX, 1)]);
    permit.finish_partial().unwrap();
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
}

#[test]
fn fully_completed_permit_is_open_until_finish_and_forgetting_is_detected() {
    let work = owner([(1, 10, 10)]);
    let mut permit = group(&work, [1])
        .admit_observed([0], WorkLane::Producer)
        .unwrap();
    for _ in 0..3 {
        permit.replenish_one().unwrap();
        permit.complete(0, 1).unwrap();
    }
    assert_eq!(permit.remaining(), [0]);
    assert_eq!(counters(&work), [(3, 3, 0)]);
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    std::mem::forget(permit);
    assert!(work.borrow_state().unwrap().header.producer_open);
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
}

#[test]
fn zero_and_empty_envelopes_still_own_persistent_channels() {
    let work = owner([(1, 0, 0)]);
    let zero = group(&work, [1])
        .admit_observed([0], WorkLane::Producer)
        .unwrap();
    let empty = group(&work, [])
        .admit_observed([], WorkLane::Internal)
        .unwrap();
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 2 })
    );
    empty.finish().unwrap();
    zero.finish().unwrap();
    work.check_quiescent().unwrap();
    let forgotten = group(&work, [1])
        .admit_observed([0], WorkLane::Producer)
        .unwrap();
    std::mem::forget(forgotten);
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
}

#[test]
fn replenishment_keeps_one_envelope_and_records_each_immediate_prefix() {
    let work = owner([(10, 8, 8), (90, 8, 8)]);
    let roles = group(&work, [90]);
    let mut permit = roles.admit_observed([0], WorkLane::Producer).unwrap();
    for completed in [1, 2, 3] {
        permit.replenish_one().unwrap();
        assert_eq!(permit.remaining(), [1]);
        assert_eq!(counters(&work), [(0, 0, 0), (completed, completed - 1, 1)]);
        assert_eq!(
            permit.replenish_one(),
            Err(LedgerCause::InactiveLinearWorkPermit)
        );
        permit.complete(0, 1).unwrap();
        assert_eq!(permit.remaining(), [0]);
        assert_eq!(counters(&work), [(0, 0, 0), (completed, completed, 0)]);
        let state = work.borrow_state().unwrap();
        assert_eq!(state.header.open_envelopes, 1);
        assert!(state.header.producer_open);
    }
    assert!(matches!(
        roles.admit_observed([0], WorkLane::Producer),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    assert!(matches!(
        roles.begin_parser_child([0], [None]),
        Err(ParserWorkError::Ledger(
            LedgerCause::InactiveLinearWorkPermit
        ))
    ));
    permit.finish().unwrap();
    work.check_quiescent().unwrap();
    assert_eq!(counters(&work), [(0, 0, 0), (3, 3, 0)]);
}

#[test]
fn rejected_refill_and_fresh_admission_keep_the_same_attempted_prefix() {
    for (attempted, consumed, class) in [
        (1, 20, CounterClass::Attempted),
        (20, 1, CounterClass::Consumed),
        (1, 1, CounterClass::Attempted),
    ] {
        let expected = LedgerCause::LimitExceeded {
            class,
            dimension: 90,
            value: 2,
            limit: 1,
        };
        let fresh = owner([(90, attempted, consumed)]);
        let fresh_group = group(&fresh, [90]);
        let mut first = fresh_group.admit_observed([1], WorkLane::Producer).unwrap();
        first.complete(0, 1).unwrap();
        first.finish().unwrap();
        assert!(matches!(
            fresh_group.admit_observed([1], WorkLane::Producer),
            Err(cause) if cause == expected
        ));
        assert_eq!(counters(&fresh), [(2, 1, 0)]);
        fresh.check_quiescent().unwrap();

        let sequence = owner([(90, attempted, consumed)]);
        let mut permit = group(&sequence, [90])
            .admit_observed([1], WorkLane::Producer)
            .unwrap();
        permit.complete(0, 1).unwrap();
        assert_eq!(permit.replenish_one(), Err(expected));
        assert_eq!(permit.remaining(), [0]);
        assert_eq!(counters(&sequence), [(2, 1, 0)]);
        assert_eq!(
            sequence.check_quiescent(),
            Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        );
        permit.finish_partial().unwrap();
        sequence.check_quiescent().unwrap();
    }
}

#[test]
fn refill_overflow_keeps_the_last_representable_prefix_without_pending_credit() {
    let work = owner([(90, u64::MAX, u64::MAX)]);
    let mut permit = group(&work, [90])
        .admit_observed([0], WorkLane::Producer)
        .unwrap();
    work.borrow_state_mut().unwrap().rows[0].state = WorkDimensionState {
        attempted: u64::MAX,
        consumed: u64::MAX,
        pending_consumed: 0,
    };
    assert_eq!(
        permit.replenish_one(),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 90,
        })
    );
    assert_eq!(permit.remaining(), [0]);
    assert_eq!(counters(&work), [(u64::MAX, u64::MAX, 0)]);
    permit.finish_partial().unwrap();
    work.check_quiescent().unwrap();
}

#[test]
fn refill_counts_internal_pending_and_current_policy_without_refunding_attempts() {
    let work = owner([(90, 20, 20)]);
    let roles = group(&work, [90]);
    let mut producer = roles.admit_observed([1], WorkLane::Producer).unwrap();
    producer.complete(0, 1).unwrap();
    let internal = roles.admit_observed([2], WorkLane::Internal).unwrap();
    work.borrow_state_mut().unwrap().rows[0]
        .policy
        .consumed_limit = 3;
    assert_eq!(
        producer.replenish_one(),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 90,
            value: 4,
            limit: 3,
        })
    );
    assert_eq!(producer.remaining(), [0]);
    assert_eq!(counters(&work), [(4, 1, 2)]);
    producer.finish_partial().unwrap();
    assert_eq!(counters(&work), [(4, 1, 2)]);
    internal.finish_partial().unwrap();
    assert_eq!(counters(&work), [(4, 1, 0)]);
    work.check_quiescent().unwrap();
}

#[test]
fn refill_rechecks_revoked_mode_and_protected_position_even_at_zero_remaining() {
    for mode in [
        WorkMode::OrdinarySealed,
        WorkMode::Protected,
        WorkMode::Closed,
    ] {
        let work = owner([(10, 20, 20), (90, 20, 20)]);
        let mut permit = group(&work, [90])
            .admit_observed([0], WorkLane::Producer)
            .unwrap();
        work.borrow_state_mut().unwrap().header.mode = mode;
        let expected = if mode == WorkMode::Closed {
            LedgerCause::PublicationAlreadyCompleted
        } else {
            LedgerCause::PublicationAlreadyPending
        };
        assert_eq!(permit.replenish_one(), Err(expected));
        assert_eq!(counters(&work), [(0, 0, 0); 2]);
        permit.finish_partial().unwrap();
        work.check_quiescent().unwrap();
    }
    let work = owner([(10, 20, 20), (90, 20, 20)]);
    let mut permit = group(&work, [90])
        .admit_observed([0], WorkLane::Producer)
        .unwrap();
    work.borrow_state_mut().unwrap().header.protected_dimension = Some(1);
    assert_eq!(
        permit.replenish_one(),
        Err(LedgerCause::UnknownDimension { dimension: 90 })
    );
    assert_eq!(counters(&work), [(0, 0, 0); 2]);
    permit.finish_partial().unwrap();
    work.check_quiescent().unwrap();
}

#[test]
fn internal_protected_and_faulted_permits_cannot_replenish() {
    let work = owner([(90, 20, 20)]);
    let mut internal = group(&work, [90])
        .admit_observed([0], WorkLane::Internal)
        .unwrap();
    assert_eq!(
        internal.replenish_one(),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    internal.finish().unwrap();
    {
        let mut state = work.borrow_state_mut().unwrap();
        state.header.mode = WorkMode::Protected;
        state.header.protected_dimension = Some(0);
    }
    let mut protected = work
        .bind([90], WorkGrant::Protected)
        .unwrap()
        .admit_observed([0], WorkLane::Protected)
        .unwrap();
    assert_eq!(
        protected.replenish_one(),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    protected.finish().unwrap();
    assert_eq!(counters(&work), [(0, 0, 0)]);

    let work = owner([(90, 20, 20)]);
    let mut permit = group(&work, [90])
        .admit_observed([1], WorkLane::Producer)
        .unwrap();
    work.borrow_state_mut().unwrap().rows[0]
        .policy
        .consumed_limit = 0;
    assert!(matches!(
        permit.complete_with_outcome(0, 1),
        LinearWorkCompletion::CompletedWithLedgerFailure(LedgerCause::LimitExceeded { .. })
    ));
    assert_eq!(permit.remaining(), [0]);
    assert_eq!(
        permit.replenish_one(),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(counters(&work), [(1, 1, 0)]);
    permit.finish_partial().unwrap();
    work.check_quiescent().unwrap();
}

#[test]
fn abandoned_drop_releases_only_pending_and_persistently_faults_even_after_full_completion() {
    let work = owner([(1, 10, 10)]);
    let mut permit = group(&work, [1, 1])
        .admit_observed([2, 3], WorkLane::Producer)
        .unwrap();
    permit.complete_batch([1, 2]).unwrap();
    drop(permit);
    assert_eq!(counters(&work), [(5, 3, 0)]);
    assert_eq!(work.borrow_state().unwrap().header.open_envelopes, 0);
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );

    let work = owner([(1, 10, 10)]);
    let mut permit = group(&work, [1])
        .admit_observed([2], WorkLane::Internal)
        .unwrap();
    permit.complete(0, 2).unwrap();
    drop(permit);
    assert_eq!(counters(&work), [(2, 2, 0)]);
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
}

#[test]
fn incomplete_finish_is_an_abandoned_envelope() {
    let work = owner([(1, 10, 10)]);
    let permit = group(&work, [1])
        .admit_observed([3], WorkLane::Producer)
        .unwrap();
    assert_eq!(permit.finish(), Err(LedgerCause::InactiveLinearWorkPermit));
    assert_eq!(counters(&work), [(3, 0, 0)]);
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
}

#[test]
fn drop_during_borrow_conflict_neither_panics_nor_invents_pending_release() {
    let work = owner([(1, 10, 10)]);
    let permit = group(&work, [1])
        .admit_observed([3], WorkLane::Producer)
        .unwrap();
    let state = work.borrow_state().unwrap();
    drop(permit);
    assert_eq!(state.header.open_envelopes, 1);
    assert!(state.header.producer_open);
    assert_eq!(state.rows[0].state.pending_consumed, 3);
    drop(state);
    assert_eq!(counters(&work), [(3, 0, 3)]);
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
}

#[test]
fn numeric_borrow_conflict_latches_fail_closed_after_guard_release() {
    let work = owner([(1, 10, 10)]);
    let roles = group(&work, [1]);
    let state = work.borrow_state_mut().unwrap();
    assert_eq!(
        roles.charge_attempted_in_order([2]),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    drop(state);
    assert_eq!(
        roles.charge_attempted_in_order([2]),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    assert_eq!(counters(&work), [(0, 0, 0)]);
}

#[test]
fn interim_integrity_allows_pending_work_but_retains_borrow_faults() {
    let work = owner([(1, 10, 10)]);
    let permit = group(&work, [1])
        .admit_observed([3], WorkLane::Producer)
        .unwrap();
    work.validate_integrity().unwrap();
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    let state = work.borrow_state_mut().unwrap();
    assert_eq!(
        work.validate_integrity(),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    drop(state);
    permit.finish_partial().unwrap();
    assert_eq!(counters(&work), [(3, 0, 0)]);
    assert_eq!(
        work.validate_integrity(),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
}

#[test]
fn modes_invalidate_old_groups_and_protected_work_needs_private_grant_and_lane() {
    let work = owner([(1, 20, 20)]);
    let ordinary = group(&work, [1]);
    assert!(matches!(
        work.bind([1], WorkGrant::Protected),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    work.check_quiescent().unwrap();
    work.borrow_state_mut().unwrap().header.mode = WorkMode::OrdinarySealed;
    assert_eq!(
        ordinary.charge_attempted_in_order([0]),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert_eq!(
        ordinary.consume_direct_atomic([0]),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert_eq!(
        ordinary.charge_observed_ordered([0]),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert_eq!(
        ordinary.consume_observed_in_order([0]),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert_eq!(
        ordinary.preflight_repeated_observed([0], NonZeroU64::new(1).unwrap()),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    assert!(matches!(
        ordinary.admit_observed([0], WorkLane::Producer),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    {
        let mut state = work.borrow_state_mut().unwrap();
        state.header.protected_dimension = Some(0);
        state.header.mode = WorkMode::Protected;
    }
    assert_eq!(
        ordinary.charge_attempted_in_order([1]),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    let protected = work.bind([1], WorkGrant::Protected).unwrap();
    assert!(matches!(
        protected.admit_observed([1], WorkLane::Producer),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    let mut permit = protected.admit_observed([2], WorkLane::Protected).unwrap();
    assert!(matches!(
        protected.admit_observed([0], WorkLane::Protected),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    let internal = protected.admit_observed([1], WorkLane::Internal).unwrap();
    internal.finish_partial().unwrap();
    permit.complete(0, 2).unwrap();
    permit.finish().unwrap();
    work.check_quiescent().unwrap();
    work.borrow_state_mut().unwrap().header.mode = WorkMode::Closed;
    assert_eq!(
        protected.charge_attempted_in_order([0]),
        Err(LedgerCause::PublicationAlreadyCompleted)
    );
    assert_eq!(counters(&work), [(3, 2, 0)]);
}

#[test]
fn parser_channels_and_orphan_pending_also_prevent_quiescence() {
    let work = owner([(1, 10, 10)]);
    let roles = group(&work, [1]);
    work.borrow_state_mut().unwrap().header.parser_parent_open = true;
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    assert!(matches!(
        roles.admit_observed([0], WorkLane::Producer),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    let internal = roles.admit_observed([1], WorkLane::Internal).unwrap();
    internal.finish_partial().unwrap();
    {
        let mut state = work.borrow_state_mut().unwrap();
        state.header.parser_parent_open = false;
        state.header.parser_child_open = true;
    }
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    {
        let mut state = work.borrow_state_mut().unwrap();
        state.header.parser_child_open = false;
        state.rows[0].state.pending_consumed = 1;
    }
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
}

#[test]
fn bound_capability_carries_its_actual_owner() {
    let first = owner([(1, 10, 10)]);
    let second = owner([(1, 1, 1)]);
    let roles = group(&first, [1]);
    roles.charge_observed_ordered([3]).unwrap();
    assert_eq!(counters(&first), [(3, 3, 0)]);
    assert_eq!(counters(&second), [(0, 0, 0)]);
    assert!(std::ptr::eq(roles.owner, &first));
    // Applying `roles` to `second` has no API: the owner is stored inside the capability.
}

#[test]
fn shape_mismatch_and_open_count_overflow_are_contained_without_numeric_changes() {
    let malformed = LocalWork::new(vec![1_u8], Vec::new());
    assert_eq!(
        malformed.check_quiescent(),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    assert!(matches!(
        malformed.bind([1], WorkGrant::Ordinary),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    ));

    let work = owner([(1, 10, 10)]);
    let roles = group(&work, [1]);
    work.borrow_state_mut().unwrap().header.open_envelopes = u64::MAX;
    assert!(matches!(
        roles.admit_observed([2], WorkLane::Internal),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    ));
    assert_eq!(counters(&work), [(0, 0, 0)]);
}

#[test]
fn work_validation_preserves_effective_attempted_then_consumed_precedence() {
    let policy = WorkPolicy {
        attempted_limit: 5,
        consumed_limit: 4,
    };
    assert_eq!(
        validate_work_state(
            1_u8,
            policy,
            WorkDimensionState {
                attempted: 6,
                consumed: 5,
                pending_consumed: 2,
            }
        ),
        Err(LedgerCause::ConsumedExceedsAttempted {
            dimension: 1,
            consumed: 7,
            attempted: 6
        })
    );
    assert_eq!(
        validate_work_state(
            1_u8,
            policy,
            WorkDimensionState {
                attempted: 6,
                consumed: 5,
                pending_consumed: 0,
            }
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 1,
            value: 6,
            limit: 5,
        })
    );
    assert_eq!(
        validate_work_state(
            1_u8,
            policy,
            WorkDimensionState {
                attempted: 5,
                consumed: 3,
                pending_consumed: 2,
            }
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 5,
            limit: 4,
        })
    );
    assert_eq!(
        validate_work_state(
            1_u8,
            policy,
            WorkDimensionState {
                attempted: u64::MAX,
                consumed: u64::MAX,
                pending_consumed: 1,
            }
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 1
        })
    );
}

#[test]
fn successful_numeric_helpers_never_resolve_diagnostic_keys() {
    let calls = Cell::new(0);
    let dimension = || {
        calls.set(calls.get() + 1);
        37_u8
    };
    assert_eq!(add_attempted(2, 3, dimension), Ok(5));
    assert_eq!(add_consumed(2, 3, dimension), Ok(5));
    assert_eq!(
        check_limit(CounterClass::Attempted, dimension, 5, 5),
        Ok(())
    );
    assert_eq!(check_effective(dimension, 2, 3, 5, 5), Ok(()));
    let mut row = WorkEntry {
        dimension: 37,
        policy: WorkPolicy {
            attempted_limit: 5,
            consumed_limit: 5,
        },
        state: WorkDimensionState::default(),
    };
    assert_eq!(charge_attempted_entry(&mut row, dimension, 2), Ok(()));
    assert_eq!(consume_observed_entry(&mut row, dimension, 2), Ok(()));
    assert_eq!(
        row.state,
        WorkDimensionState {
            attempted: 2,
            consumed: 2,
            pending_consumed: 0,
        }
    );
    assert_eq!(calls.get(), 0);
}

#[test]
fn numeric_helper_errors_resolve_their_exact_key_once() {
    let calls = Cell::new(0);
    let dimension = || {
        calls.set(calls.get() + 1);
        37_u8
    };
    assert_eq!(
        add_attempted(u64::MAX, 1, dimension),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 37,
        })
    );
    assert_eq!(calls.replace(0), 1);
    assert_eq!(
        add_consumed(u64::MAX, 1, dimension),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 37,
        })
    );
    assert_eq!(calls.replace(0), 1);
    assert_eq!(
        check_limit(CounterClass::Attempted, dimension, 6, 5),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 37,
            value: 6,
            limit: 5,
        })
    );
    assert_eq!(calls.replace(0), 1);
    assert_eq!(
        check_effective(dimension, u64::MAX, 1, u64::MAX, u64::MAX),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 37,
        })
    );
    assert_eq!(calls.replace(0), 1);
    assert_eq!(
        check_effective(dimension, 2, 3, 4, 0),
        Err(LedgerCause::ConsumedExceedsAttempted {
            dimension: 37,
            consumed: 5,
            attempted: 4,
        })
    );
    assert_eq!(calls.replace(0), 1);
    assert_eq!(
        check_effective(dimension, 2, 3, 5, 4),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 37,
            value: 5,
            limit: 4,
        })
    );
    assert_eq!(calls.replace(0), 1);
    let mut row = WorkEntry {
        dimension: 37,
        policy: WorkPolicy {
            attempted_limit: 1,
            consumed_limit: 1,
        },
        state: WorkDimensionState::default(),
    };
    assert_eq!(
        charge_attempted_entry(&mut row, dimension, 2),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 37,
            value: 2,
            limit: 1,
        })
    );
    assert_eq!(calls.replace(0), 1);
    assert_eq!(row.state.attempted, 2);
    assert_eq!(
        consume_observed_entry(&mut row, dimension, 2),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 37,
            value: 2,
            limit: 1,
        })
    );
    assert_eq!(calls.replace(0), 1);
    assert_eq!(row.state.consumed, 2);
    assert_eq!(
        consume_observed_entry(&mut row, dimension, 1),
        Err(LedgerCause::ConsumedExceedsAttempted {
            dimension: 37,
            consumed: 3,
            attempted: 2,
        })
    );
    assert_eq!(calls.get(), 1);
    assert_eq!(row.state.consumed, 2);
}

#[test]
fn duplicate_bound_lanes_recheck_noncontiguous_protected_keys_without_work() {
    let work = owner([(10, 20, 20), (30, 20, 20), (90, 20, 20)]);
    let roles = group(&work, [30, 90, 30, 10]);
    work.borrow_state_mut().unwrap().header.protected_dimension = Some(2);
    assert_eq!(
        roles.charge_attempted_in_order([0; 4]),
        Err(LedgerCause::UnknownDimension { dimension: 90 })
    );
    assert_eq!(
        roles.preflight_repeated_observed([0; 4], NonZeroU64::new(1).unwrap()),
        Err(LedgerCause::UnknownDimension { dimension: 90 })
    );
    assert_eq!(counters(&work), [(0, 0, 0); 3]);
    work.check_quiescent().unwrap();
}

#[test]
fn bound_numeric_errors_name_registered_keys_and_binding_errors_keep_caller_keys() {
    #[derive(Debug, Clone, Copy)]
    struct DiagnosticKey {
        id: u8,
        label: &'static str,
    }

    impl PartialEq for DiagnosticKey {
        fn eq(&self, other: &Self) -> bool {
            self.id == other.id
        }
    }

    impl Eq for DiagnosticKey {}

    impl PartialOrd for DiagnosticKey {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for DiagnosticKey {
        fn cmp(&self, other: &Self) -> Ordering {
            self.id.cmp(&other.id)
        }
    }

    let registered = DiagnosticKey {
        id: 90,
        label: "registered",
    };
    let caller = DiagnosticKey {
        id: 90,
        label: "caller",
    };
    let work = LocalWork::new(
        vec![registered],
        vec![WorkEntry {
            dimension: registered,
            policy: WorkPolicy {
                attempted_limit: 1,
                consumed_limit: 1,
            },
            state: WorkDimensionState::default(),
        }],
    );
    let roles = work.bind([caller], WorkGrant::Ordinary).unwrap();
    let Err(LedgerCause::LimitExceeded {
        class: CounterClass::Attempted,
        dimension,
        value: 2,
        limit: 1,
    }) = roles.charge_attempted_in_order([2])
    else {
        panic!("expected the bound attempted limit failure");
    };
    assert_eq!(dimension.id, registered.id);
    assert_eq!(dimension.label, registered.label);

    work.borrow_state_mut().unwrap().header.protected_dimension = Some(0);
    let Err(LedgerCause::UnknownDimension { dimension }) = roles.charge_attempted_in_order([0])
    else {
        panic!("expected the revoked bound role");
    };
    assert_eq!(dimension.label, registered.label);
    let Err(LedgerCause::UnknownDimension { dimension }) = work.bind([caller], WorkGrant::Ordinary)
    else {
        panic!("expected the protected role to reject binding");
    };
    assert_eq!(dimension.label, caller.label);

    let missing = DiagnosticKey {
        id: 30,
        label: "missing caller",
    };
    let Err(LedgerCause::UnknownDimension { dimension }) =
        work.bind([missing], WorkGrant::Ordinary)
    else {
        panic!("expected the absent caller key");
    };
    assert_eq!(dimension.id, missing.id);
    assert_eq!(dimension.label, missing.label);
}

#[test]
fn owner_and_numeric_interfaces_accept_keys_without_debug() {
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    struct OpaqueKey(u8);

    let work = LocalWork::new(
        vec![OpaqueKey(1)],
        vec![WorkEntry {
            dimension: OpaqueKey(1),
            policy: WorkPolicy {
                attempted_limit: 10,
                consumed_limit: 10,
            },
            state: WorkDimensionState::default(),
        }],
    );
    assert_eq!(work.catalogue().len(), 1);
    assert!(work.borrow_state().is_ok());
    assert!(work.borrow_state_mut().is_ok());
    let Ok(roles) = work.bind([OpaqueKey(1)], WorkGrant::Ordinary) else {
        panic!("valid opaque key was rejected");
    };
    assert!(roles.charge_observed_ordered([1]).is_ok());
    let Ok(mut permit) = roles.admit_observed([1], WorkLane::Producer) else {
        panic!("valid opaque work envelope was rejected");
    };
    assert!(permit.complete(0, 1).is_ok());
    assert!(permit.finish().is_ok());
    assert!(work.check_quiescent().is_ok());
    assert!(!work.into_state().1.header.integrity_fault);
}

#[derive(Debug, Clone, Copy)]
struct ComparingKey(u8);

thread_local! {
    static PANIC_COMPARISON: Cell<bool> = const { Cell::new(false) };
    static COMPARISONS: Cell<u64> = const { Cell::new(0) };
    static COMPARISON_OWNER: LocalWork<ComparingKey> = LocalWork::new(
        vec![ComparingKey(1), ComparingKey(2)],
        vec![1, 2].into_iter().map(|value| WorkEntry {
            dimension: ComparingKey(value),
            policy: WorkPolicy { attempted_limit: 20, consumed_limit: 20 },
            state: WorkDimensionState::default(),
        }).collect(),
    );
}

impl PartialEq for ComparingKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for ComparingKey {}

impl PartialOrd for ComparingKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ComparingKey {
    fn cmp(&self, other: &Self) -> Ordering {
        COMPARISONS.with(|count| count.set(count.get() + 1));
        COMPARISON_OWNER.with(|owner| {
            assert!(
                owner.state.try_borrow_mut().is_ok(),
                "comparison ran under a work borrow"
            );
        });
        PANIC_COMPARISON.with(|panic| assert!(!panic.get(), "injected comparator panic"));
        self.0.cmp(&other.0)
    }
}

#[test]
fn comparator_panic_is_outside_borrow_and_bound_numeric_paths_never_compare() {
    COMPARISON_OWNER.with(|work| {
        PANIC_COMPARISON.with(|panic| panic.set(true));
        let panicked = catch_unwind(AssertUnwindSafe(|| {
            let _ = work.bind([ComparingKey(2), ComparingKey(1)], WorkGrant::Ordinary);
        }));
        assert!(panicked.is_err());
        assert!(!work.borrow_fault.get());
        {
            let state = work.borrow_state().unwrap();
            assert_eq!(state.rows[0].state, WorkDimensionState::default());
            assert_eq!(state.rows[1].state, WorkDimensionState::default());
        }
        PANIC_COMPARISON.with(|panic| panic.set(false));
        let roles = work
            .bind(
                [ComparingKey(2), ComparingKey(1), ComparingKey(2)],
                WorkGrant::Ordinary,
            )
            .unwrap();
        let comparisons = COMPARISONS.with(Cell::get);
        PANIC_COMPARISON.with(|panic| panic.set(true));
        roles.charge_attempted_in_order([1, 1, 1]).unwrap();
        roles.consume_direct_atomic([1, 1, 1]).unwrap();
        roles.charge_observed_ordered([1, 1, 1]).unwrap();
        roles.consume_observed_in_order([0, 0, 0]).unwrap();
        roles
            .preflight_repeated_observed([1, 1, 1], NonZeroU64::new(2).unwrap())
            .unwrap();
        let mut permit = roles.admit_observed([2, 2, 2], WorkLane::Internal).unwrap();
        permit.complete_batch([1, 1, 1]).unwrap();
        permit.finish_partial().unwrap();
        work.check_quiescent().unwrap();
        assert_eq!(COMPARISONS.with(Cell::get), comparisons);
        let state = work.borrow_state().unwrap();
        assert_eq!(
            state.rows[0].state,
            WorkDimensionState {
                attempted: 4,
                consumed: 3,
                pending_consumed: 0
            }
        );
        assert_eq!(
            state.rows[1].state,
            WorkDimensionState {
                attempted: 8,
                consumed: 6,
                pending_consumed: 0
            }
        );
        PANIC_COMPARISON.with(|panic| panic.set(false));
    });
}
