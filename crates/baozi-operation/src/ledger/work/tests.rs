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
    row_counters(&state.rows)
}

fn row_counters<const N: usize>(rows: &[WorkEntry<u8>]) -> [(u64, u64, u64); N] {
    assert_eq!(rows.len(), N);
    std::array::from_fn(|index| {
        let row = rows[index].state;
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
    let mut work = owner([(1, 20, 20)]);
    let rows_pointer = work.borrow_state().unwrap().rows.as_ptr();
    {
        let (catalog, mut view) = work.exclusive().unwrap();
        let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
        let mut frame = layout
            .admit_observed(&mut view, [5], WorkLane::Producer)
            .unwrap();
        assert!(matches!(
            frame.complete_batch_with_outcome(&layout, &mut view, [2]),
            LinearWorkCompletion::Completed
        ));
        assert_eq!(frame.remaining, [3]);
        assert!(frame.active);
    }
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
fn direct_completion_accounts_for_other_frames_pending_capacity() {
    let mut work = owner([(1, 10, 10)]);
    let (layout, mut frame) = {
        let (catalog, mut view) = work.exclusive().unwrap();
        let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
        let frame = layout
            .admit_observed(&mut view, [4], WorkLane::Producer)
            .unwrap();
        (layout, frame)
    };
    let roles = group(&work, [1]);
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
    {
        let (_, mut view) = work.reborrow().unwrap();
        assert!(matches!(
            frame.complete_batch_with_outcome(&layout, &mut view, [4]),
            LinearWorkCompletion::Completed
        ));
        assert_eq!(row_counters(view.rows), [(6, 6, 0)]);
        frame.resolve(&layout, &mut view, false).unwrap();
    }
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
    let mut work = owner([(1, 10, 10)]);
    let (layout, mut frame) = {
        let (catalog, mut view) = work.exclusive().unwrap();
        let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
        let frame = layout
            .admit_observed(&mut view, [4], WorkLane::Producer)
            .unwrap();
        (layout, frame)
    };
    let roles = group(&work, [1]);
    assert_eq!(
        roles.consume_observed_in_order([1]),
        Err(LedgerCause::ConsumedExceedsAttempted {
            dimension: 1,
            consumed: 5,
            attempted: 4
        })
    );
    assert_eq!(counters(&work), [(4, 1, 4)]);
    work.split_internal()
        .resolve(layout, &mut frame, false)
        .unwrap();
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
    let mut work = owner([(1, 20, 20)]);
    let (layout, mut frame) = {
        let (catalog, mut view) = work.exclusive().unwrap();
        let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
        let frame = layout
            .admit_observed(&mut view, [4], WorkLane::Internal)
            .unwrap();
        (layout, frame)
    };
    let roles = group(&work, [1]);
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
    work.split_internal()
        .resolve(layout, &mut frame, false)
        .unwrap();
}

#[test]
fn admission_keeps_every_representable_attempt_and_prefers_attempted_failure() {
    let mut work = owner([(1, 20, 0), (2, 3, 3)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[1, 2, 2], WorkGrant::Ordinary).unwrap();
    assert!(matches!(
        layout.admit_observed(&mut view, [2, 3, 2], WorkLane::Producer),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 2,
            value: 5,
            limit: 3
        })
    ));
    assert_eq!(row_counters(view.rows), [(2, 0, 0), (5, 0, 0)]);
    assert_eq!(view.header.open_envelopes, 0);
    assert!(!view.header.producer_open);
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn consumed_admission_failure_keeps_attempts_without_claiming_a_lane() {
    let mut work = owner([(1, 10, 2)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[1, 1], WorkGrant::Ordinary).unwrap();
    assert!(matches!(
        layout.admit_observed(&mut view, [2, 1], WorkLane::Producer),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 3,
            limit: 2
        })
    ));
    assert_eq!(row_counters(view.rows), [(3, 0, 0)]);
    assert_eq!(view.header.open_envelopes, 0);
    assert!(!view.header.producer_open);
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn admission_attempted_and_duplicate_overflow_are_atomic() {
    let mut work = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let distinct = WorkLayout::bind(catalog, &[1, 2], WorkGrant::Ordinary).unwrap();
    let duplicate = WorkLayout::bind(catalog, &[1, 1], WorkGrant::Ordinary).unwrap();
    view.rows[1].state.attempted = u64::MAX;
    assert!(matches!(
        distinct.admit_observed(&mut view, [3, 1], WorkLane::Internal),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 2
        })
    ));
    assert!(matches!(
        duplicate.admit_observed(&mut view, [u64::MAX, 1], WorkLane::Internal),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 1
        })
    ));
    assert_eq!(row_counters(view.rows), [(0, 0, 0), (u64::MAX, 0, 0)]);
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn admission_pending_overflow_does_not_commit_an_earlier_projection() {
    let mut work = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[1, 2], WorkGrant::Ordinary).unwrap();
    view.rows[1].state.pending_consumed = u64::MAX;
    assert!(matches!(
        layout.admit_observed(&mut view, [3, 1], WorkLane::Internal),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 2
        })
    ));
    assert_eq!(row_counters(view.rows), [(0, 0, 0), (0, 0, u64::MAX)]);
    assert_eq!(view.header.open_envelopes, 0);
}

#[test]
fn duplicate_batch_completion_and_resolution_preserve_exact_prefixes() {
    let mut work = owner([(1, 20, 20), (2, 20, 20)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[2, 1, 2], WorkGrant::Ordinary).unwrap();
    let mut frame = layout
        .admit_observed(&mut view, [3, 4, 2], WorkLane::Producer)
        .unwrap();
    assert_eq!(row_counters(view.rows), [(4, 0, 4), (5, 0, 5)]);
    assert!(matches!(
        frame.complete_batch_with_outcome(&layout, &mut view, [1, 2, 1]),
        LinearWorkCompletion::Completed
    ));
    assert_eq!(frame.remaining, [2, 2, 1]);
    assert_eq!(row_counters(view.rows), [(4, 2, 2), (5, 2, 3)]);
    frame.resolve(&layout, &mut view, false).unwrap();
    assert!(!frame.active);
    assert_eq!(row_counters(view.rows), [(4, 2, 0), (5, 2, 0)]);
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn completion_overrun_and_inactive_frame_do_not_advance_any_prefix() {
    let mut work = owner([(1, 20, 20)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[1, 1], WorkGrant::Ordinary).unwrap();
    let mut frame = layout
        .admit_observed(&mut view, [2, 3], WorkLane::Producer)
        .unwrap();
    assert!(matches!(
        frame.complete_batch_with_outcome(&layout, &mut view, [1, 4]),
        LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit)
    ));
    assert_eq!(frame.remaining, [2, 3]);
    assert!(frame.active);
    assert!(!frame.faulted);
    assert_eq!(row_counters(view.rows), [(5, 0, 5)]);
    assert!(matches!(
        frame.complete_batch_with_outcome(&layout, &mut view, [2, 3]),
        LinearWorkCompletion::Completed
    ));
    frame.resolve(&layout, &mut view, false).unwrap();
    assert!(matches!(
        frame.complete_batch_with_outcome(&layout, &mut view, [0, 0]),
        LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit)
    ));
    assert_eq!(
        frame.resolve(&layout, &mut view, false),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(frame.remaining, [0, 0]);
    assert_eq!(row_counters(view.rows), [(5, 5, 0)]);
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn internal_frames_compete_with_producer_and_retain_applied_limit_failures() {
    let mut work = owner([(1, 20, 20)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
    let mut producer = layout
        .admit_observed(&mut view, [4], WorkLane::Producer)
        .unwrap();
    let mut internal = layout
        .admit_observed(&mut view, [2], WorkLane::Internal)
        .unwrap();
    assert!(matches!(
        layout.admit_observed(&mut view, [1], WorkLane::Producer),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    assert_eq!(view.header.open_envelopes, 2);
    layout.charge_attempted_in_order(&mut view, [2]).unwrap();
    layout.consume_direct_atomic(&mut view, [2]).unwrap();
    view.rows[0].policy.consumed_limit = 5;
    assert!(matches!(
        producer.complete_batch_with_outcome(&layout, &mut view, [2]),
        LinearWorkCompletion::CompletedWithLedgerFailure(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 8,
            limit: 5,
        })
    ));
    assert_eq!(producer.remaining, [2]);
    assert!(producer.faulted);
    assert_eq!(row_counters(view.rows), [(8, 4, 4)]);
    assert!(matches!(
        producer.complete_batch_with_outcome(&layout, &mut view, [1]),
        LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit)
    ));
    assert_eq!(producer.remaining, [2]);
    assert_eq!(row_counters(view.rows), [(8, 4, 4)]);
    producer.resolve(&layout, &mut view, false).unwrap();
    assert!(matches!(
        internal.complete_batch_with_outcome(&layout, &mut view, [1]),
        LinearWorkCompletion::CompletedWithLedgerFailure(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 6,
            limit: 5,
        })
    ));
    assert_eq!(internal.remaining, [1]);
    internal.resolve(&layout, &mut view, false).unwrap();
    assert_eq!(row_counters(view.rows), [(8, 5, 0)]);
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn grouped_applied_failure_advances_every_duplicate_and_dimension_together() {
    let mut work = owner([(1, 20, 20), (2, 20, 20)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[2, 1, 2], WorkGrant::Ordinary).unwrap();
    let mut frame = layout
        .admit_observed(&mut view, [3, 4, 2], WorkLane::Internal)
        .unwrap();
    view.rows[0].policy.consumed_limit = 3;
    assert!(matches!(
        frame.complete_batch_with_outcome(&layout, &mut view, [1, 2, 1]),
        LinearWorkCompletion::CompletedWithLedgerFailure(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 4,
            limit: 3,
        })
    ));
    assert_eq!(frame.remaining, [2, 2, 1]);
    assert!(frame.faulted);
    assert_eq!(row_counters(view.rows), [(4, 2, 2), (5, 2, 3)]);
    frame.resolve(&layout, &mut view, false).unwrap();
    assert_eq!(row_counters(view.rows), [(4, 2, 0), (5, 2, 0)]);
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn corrupt_unrepresentable_group_completion_changes_no_counter_or_remaining() {
    let mut work = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[1, 2], WorkGrant::Ordinary).unwrap();
    let mut frame = layout
        .admit_observed(&mut view, [2, 1], WorkLane::Internal)
        .unwrap();
    view.rows[1].state.consumed = u64::MAX;
    view.rows[1].state.attempted = u64::MAX;
    assert!(matches!(
        frame.complete_batch_with_outcome(&layout, &mut view, [1, 1]),
        LinearWorkCompletion::NotCompleted(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 2,
        })
    ));
    assert_eq!(frame.remaining, [2, 1]);
    assert!(frame.faulted);
    assert_eq!(
        row_counters(view.rows),
        [(2, 0, 2), (u64::MAX, u64::MAX, 1)]
    );
    frame.resolve(&layout, &mut view, false).unwrap();
    assert_eq!(
        row_counters(view.rows),
        [(2, 0, 0), (u64::MAX, u64::MAX, 0)]
    );
    assert_eq!(
        kernel::check_quiescent(view.rows, view.header),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
}

#[test]
fn fully_completed_unresolved_frame_keeps_ownership_after_metadata_leaves_scope() {
    let mut work = owner([(1, 10, 10)]);
    {
        let (catalog, mut view) = work.exclusive().unwrap();
        let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
        let mut frame = layout
            .admit_observed(&mut view, [0], WorkLane::Producer)
            .unwrap();
        for _ in 0..3 {
            frame.replenish_one(&layout, &mut view).unwrap();
            assert!(matches!(
                frame.complete_batch_with_outcome(&layout, &mut view, [1]),
                LinearWorkCompletion::Completed
            ));
        }
        assert_eq!(frame.remaining, [0]);
        assert!(frame.active);
        assert_eq!(row_counters(view.rows), [(3, 3, 0)]);
        assert_eq!(
            kernel::check_quiescent(view.rows, view.header),
            Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        );
    }
    assert_eq!(counters(&work), [(3, 3, 0)]);
    assert!(work.borrow_state().unwrap().header.producer_open);
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
}

#[test]
fn zero_and_empty_envelopes_still_own_persistent_channels() {
    let mut work = owner([(1, 0, 0)]);
    {
        let (catalog, mut view) = work.exclusive().unwrap();
        let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
        let empty_layout = WorkLayout::bind(catalog, &[], WorkGrant::Ordinary).unwrap();
        let mut zero = layout
            .admit_observed(&mut view, [0], WorkLane::Producer)
            .unwrap();
        let mut empty = empty_layout
            .admit_observed(&mut view, [], WorkLane::Internal)
            .unwrap();
        assert_eq!(zero.remaining, [0]);
        assert_eq!(empty.remaining, []);
        assert_eq!(
            kernel::check_quiescent(view.rows, view.header),
            Err(LedgerCause::OutstandingLinearWorkPermits { count: 2 })
        );
        empty.resolve(&empty_layout, &mut view, false).unwrap();
        zero.resolve(&layout, &mut view, false).unwrap();
        kernel::check_quiescent(view.rows, view.header).unwrap();
        let unresolved = layout
            .admit_observed(&mut view, [0], WorkLane::Producer)
            .unwrap();
        assert!(unresolved.active);
    }
    assert_eq!(counters(&work), [(0, 0, 0)]);
    assert_eq!(
        work.check_quiescent(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
}

#[test]
fn replenishment_keeps_one_envelope_and_records_each_immediate_prefix() {
    let mut work = owner([(10, 8, 8), (90, 8, 8)]);
    {
        let (_, mut view) = work.exclusive().unwrap();
        let layout = WorkLayout::bind(view.catalog, &[90], WorkGrant::Ordinary).unwrap();
        let mut frame = layout
            .admit_observed(&mut view, [0], WorkLane::Producer)
            .unwrap();
        for completed in [1, 2, 3] {
            frame.replenish_one(&layout, &mut view).unwrap();
            assert_eq!(frame.remaining, [1]);
            let row = view.rows[1].state;
            assert_eq!(
                (row.attempted, row.consumed, row.pending_consumed),
                (completed, completed - 1, 1)
            );
            assert_eq!(
                frame.replenish_one(&layout, &mut view),
                Err(LedgerCause::InactiveLinearWorkPermit)
            );
            assert!(matches!(
                frame.complete_batch_with_outcome(&layout, &mut view, [1]),
                LinearWorkCompletion::Completed
            ));
            assert_eq!(frame.remaining, [0]);
            let row = view.rows[1].state;
            assert_eq!(
                (row.attempted, row.consumed, row.pending_consumed),
                (completed, completed, 0)
            );
            assert_eq!(view.rows[0].state, WorkDimensionState::default());
            assert_eq!(view.header.open_envelopes, 1);
            assert!(view.header.producer_open);
        }
        assert!(matches!(
            layout.admit_observed(&mut view, [0], WorkLane::Producer),
            Err(LedgerCause::InactiveLinearWorkPermit)
        ));
        assert!(matches!(
            layout.begin_parser(&mut view, [0], [None], ParserChannel::Child),
            Err(ParserWorkError::Ledger(
                LedgerCause::InactiveLinearWorkPermit
            ))
        ));
        frame.resolve(&layout, &mut view, false).unwrap();
    }
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
        let mut fresh = owner([(90, attempted, consumed)]);
        let (catalog, mut fresh_view) = fresh.exclusive().unwrap();
        let fresh_layout = WorkLayout::bind(catalog, &[90], WorkGrant::Ordinary).unwrap();
        let mut first = fresh_layout
            .admit_observed(&mut fresh_view, [1], WorkLane::Producer)
            .unwrap();
        assert!(matches!(
            first.complete_batch_with_outcome(&fresh_layout, &mut fresh_view, [1]),
            LinearWorkCompletion::Completed
        ));
        first
            .resolve(&fresh_layout, &mut fresh_view, false)
            .unwrap();
        assert!(matches!(
            fresh_layout.admit_observed(&mut fresh_view, [1], WorkLane::Producer),
            Err(cause) if cause == expected
        ));
        assert_eq!(row_counters(fresh_view.rows), [(2, 1, 0)]);
        kernel::check_quiescent(fresh_view.rows, fresh_view.header).unwrap();

        let mut sequence = owner([(90, attempted, consumed)]);
        let (catalog, mut sequence_view) = sequence.exclusive().unwrap();
        let sequence_layout = WorkLayout::bind(catalog, &[90], WorkGrant::Ordinary).unwrap();
        let mut frame = sequence_layout
            .admit_observed(&mut sequence_view, [1], WorkLane::Producer)
            .unwrap();
        assert!(matches!(
            frame.complete_batch_with_outcome(&sequence_layout, &mut sequence_view, [1]),
            LinearWorkCompletion::Completed
        ));
        assert_eq!(
            frame.replenish_one(&sequence_layout, &mut sequence_view),
            Err(expected)
        );
        assert_eq!(frame.remaining, [0]);
        assert_eq!(row_counters(sequence_view.rows), [(2, 1, 0)]);
        assert_eq!(
            kernel::check_quiescent(sequence_view.rows, sequence_view.header),
            Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        );
        frame
            .resolve(&sequence_layout, &mut sequence_view, false)
            .unwrap();
        kernel::check_quiescent(sequence_view.rows, sequence_view.header).unwrap();
    }
}

#[test]
fn refill_overflow_keeps_the_last_representable_prefix_without_pending_credit() {
    let mut work = owner([(90, u64::MAX, u64::MAX)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[90], WorkGrant::Ordinary).unwrap();
    let mut frame = layout
        .admit_observed(&mut view, [0], WorkLane::Producer)
        .unwrap();
    view.rows[0].state = WorkDimensionState {
        attempted: u64::MAX,
        consumed: u64::MAX,
        pending_consumed: 0,
    };
    assert_eq!(
        frame.replenish_one(&layout, &mut view),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 90,
        })
    );
    assert_eq!(frame.remaining, [0]);
    assert_eq!(row_counters(view.rows), [(u64::MAX, u64::MAX, 0)]);
    frame.resolve(&layout, &mut view, false).unwrap();
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn refill_counts_internal_pending_and_current_policy_without_refunding_attempts() {
    let mut work = owner([(90, 20, 20)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[90], WorkGrant::Ordinary).unwrap();
    let mut producer = layout
        .admit_observed(&mut view, [1], WorkLane::Producer)
        .unwrap();
    assert!(matches!(
        producer.complete_batch_with_outcome(&layout, &mut view, [1]),
        LinearWorkCompletion::Completed
    ));
    let mut internal = layout
        .admit_observed(&mut view, [2], WorkLane::Internal)
        .unwrap();
    view.rows[0].policy.consumed_limit = 3;
    assert_eq!(
        producer.replenish_one(&layout, &mut view),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 90,
            value: 4,
            limit: 3,
        })
    );
    assert_eq!(producer.remaining, [0]);
    assert_eq!(row_counters(view.rows), [(4, 1, 2)]);
    producer.resolve(&layout, &mut view, false).unwrap();
    assert_eq!(row_counters(view.rows), [(4, 1, 2)]);
    assert_eq!(view.header.open_envelopes, 1);
    internal.resolve(&layout, &mut view, false).unwrap();
    assert_eq!(row_counters(view.rows), [(4, 1, 0)]);
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn layout_validation_rechecks_revoked_mode_and_protected_position_without_work() {
    for mode in [
        WorkMode::OrdinarySealed,
        WorkMode::Protected,
        WorkMode::Closed,
    ] {
        let mut work = owner([(10, 20, 20), (90, 20, 20)]);
        let (catalog, mut view) = work.exclusive().unwrap();
        let layout = WorkLayout::bind(catalog, &[90], WorkGrant::Ordinary).unwrap();
        let mut frame = layout
            .admit_observed(&mut view, [0], WorkLane::Producer)
            .unwrap();
        view.header.mode = mode;
        let expected = if mode == WorkMode::Closed {
            LedgerCause::PublicationAlreadyCompleted
        } else {
            LedgerCause::PublicationAlreadyPending
        };
        assert_eq!(layout.validate_active(&view), Err(expected));
        assert_eq!(frame.remaining, [0]);
        assert_eq!(row_counters(view.rows), [(0, 0, 0); 2]);
        frame.resolve(&layout, &mut view, false).unwrap();
        kernel::check_quiescent(view.rows, view.header).unwrap();
    }
    let mut work = owner([(10, 20, 20), (90, 20, 20)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[90], WorkGrant::Ordinary).unwrap();
    let mut frame = layout
        .admit_observed(&mut view, [0], WorkLane::Producer)
        .unwrap();
    view.header.protected_dimension = Some(1);
    assert_eq!(
        layout.validate_active(&view),
        Err(LedgerCause::UnknownDimension { dimension: 90 })
    );
    assert_eq!(frame.remaining, [0]);
    assert_eq!(row_counters(view.rows), [(0, 0, 0); 2]);
    frame.resolve(&layout, &mut view, false).unwrap();
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn internal_protected_and_faulted_frames_cannot_replenish() {
    let mut work = owner([(90, 20, 20)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let ordinary_layout = WorkLayout::bind(catalog, &[90], WorkGrant::Ordinary).unwrap();
    let mut internal = ordinary_layout
        .admit_observed(&mut view, [0], WorkLane::Internal)
        .unwrap();
    assert_eq!(
        internal.replenish_one(&ordinary_layout, &mut view),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    internal
        .resolve(&ordinary_layout, &mut view, false)
        .unwrap();
    view.header.mode = WorkMode::Protected;
    view.header.protected_dimension = Some(0);
    let protected_layout = WorkLayout::bind(catalog, &[90], WorkGrant::Protected).unwrap();
    protected_layout.validate_active(&view).unwrap();
    let mut protected = protected_layout
        .admit_observed(&mut view, [0], WorkLane::Protected)
        .unwrap();
    assert_eq!(
        protected.replenish_one(&protected_layout, &mut view),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    protected
        .resolve(&protected_layout, &mut view, false)
        .unwrap();
    assert_eq!(row_counters(view.rows), [(0, 0, 0)]);

    let mut work = owner([(90, 20, 20)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[90], WorkGrant::Ordinary).unwrap();
    let mut frame = layout
        .admit_observed(&mut view, [1], WorkLane::Producer)
        .unwrap();
    view.rows[0].policy.consumed_limit = 0;
    assert!(matches!(
        frame.complete_batch_with_outcome(&layout, &mut view, [1]),
        LinearWorkCompletion::CompletedWithLedgerFailure(LedgerCause::LimitExceeded { .. })
    ));
    assert_eq!(frame.remaining, [0]);
    assert!(frame.faulted);
    assert_eq!(
        frame.replenish_one(&layout, &mut view),
        Err(LedgerCause::InactiveLinearWorkPermit)
    );
    assert_eq!(row_counters(view.rows), [(1, 1, 0)]);
    frame.resolve(&layout, &mut view, false).unwrap();
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn abandoned_resolution_releases_only_pending_and_faults_even_after_full_completion() {
    let mut work = owner([(1, 10, 10)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[1, 1], WorkGrant::Ordinary).unwrap();
    let mut frame = layout
        .admit_observed(&mut view, [2, 3], WorkLane::Producer)
        .unwrap();
    assert!(matches!(
        frame.complete_batch_with_outcome(&layout, &mut view, [1, 2]),
        LinearWorkCompletion::Completed
    ));
    frame.resolve(&layout, &mut view, true).unwrap();
    assert!(!frame.active);
    assert_eq!(row_counters(view.rows), [(5, 3, 0)]);
    assert_eq!(view.header.open_envelopes, 0);
    assert!(!view.header.producer_open);
    assert_eq!(
        kernel::check_quiescent(view.rows, view.header),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );

    let mut work = owner([(1, 10, 10)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
    let mut frame = layout
        .admit_observed(&mut view, [2], WorkLane::Internal)
        .unwrap();
    assert!(matches!(
        frame.complete_batch_with_outcome(&layout, &mut view, [2]),
        LinearWorkCompletion::Completed
    ));
    frame.resolve(&layout, &mut view, true).unwrap();
    assert_eq!(row_counters(view.rows), [(2, 2, 0)]);
    assert_eq!(view.header.open_envelopes, 0);
    assert_eq!(
        kernel::check_quiescent(view.rows, view.header),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
}

#[test]
fn internal_resolution_preserves_pending_when_cold_observation_guards_are_forgotten() {
    for shared in [false, true] {
        let mut work = owner([(1, 10, 10)]);
        let (layout, mut frame) = {
            let (catalog, mut view) = work.exclusive().unwrap();
            let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
            let frame = layout
                .admit_observed(&mut view, [3], WorkLane::Producer)
                .unwrap();
            (layout, frame)
        };
        if shared {
            std::mem::forget(work.borrow_state().unwrap());
        } else {
            std::mem::forget(work.borrow_state_mut().unwrap());
        }
        {
            let mut internal = work.split_internal();
            assert_eq!(
                internal.resolve(layout, &mut frame, true),
                Err(LedgerCause::FinalSnapshotShapeMismatch)
            );
            assert!(frame.active);
            assert_eq!(frame.remaining, [3]);
            assert!(internal.borrow_fault.get());
            assert_eq!(internal.state.header.open_envelopes, 1);
            assert!(internal.state.header.producer_open);
            assert_eq!(row_counters(&internal.state.rows), [(3, 0, 3)]);
        }
        let (_, state) = work.into_state();
        assert!(state.header.integrity_fault);
        assert_eq!(state.header.open_envelopes, 1);
        assert_eq!(row_counters(&state.rows), [(3, 0, 3)]);
    }
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
    let mut work = owner([(1, 10, 10)]);
    let (layout, mut frame) = {
        let (catalog, mut view) = work.exclusive().unwrap();
        let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
        let frame = layout
            .admit_observed(&mut view, [3], WorkLane::Producer)
            .unwrap();
        (layout, frame)
    };
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
    work.split_internal()
        .resolve(layout, &mut frame, false)
        .unwrap();
    assert!(!frame.active);
    assert_eq!(counters(&work), [(3, 0, 0)]);
    assert_eq!(
        work.validate_integrity(),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
}

#[test]
fn modes_invalidate_old_groups_and_protected_work_needs_private_grant_and_lane() {
    let mut work = owner([(1, 20, 20)]);
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
    {
        let mut state = work.borrow_state_mut().unwrap();
        state.header.protected_dimension = Some(0);
        state.header.mode = WorkMode::Protected;
    }
    assert_eq!(
        ordinary.charge_attempted_in_order([1]),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    let protected_layout = work.bind_layout([1], WorkGrant::Protected).unwrap();
    {
        let (_, mut view) = work.reborrow().unwrap();
        protected_layout.validate_active(&view).unwrap();
        assert!(matches!(
            protected_layout.admit_observed(&mut view, [1], WorkLane::Producer),
            Err(LedgerCause::InactiveLinearWorkPermit)
        ));
        let mut frame = protected_layout
            .admit_observed(&mut view, [2], WorkLane::Protected)
            .unwrap();
        assert!(matches!(
            protected_layout.admit_observed(&mut view, [0], WorkLane::Protected),
            Err(LedgerCause::InactiveLinearWorkPermit)
        ));
        let mut internal = protected_layout
            .admit_observed(&mut view, [1], WorkLane::Internal)
            .unwrap();
        internal
            .resolve(&protected_layout, &mut view, false)
            .unwrap();
        assert!(matches!(
            frame.complete_batch_with_outcome(&protected_layout, &mut view, [2]),
            LinearWorkCompletion::Completed
        ));
        frame.resolve(&protected_layout, &mut view, false).unwrap();
    }
    work.check_quiescent().unwrap();
    let protected = work.bind([1], WorkGrant::Protected).unwrap();
    work.borrow_state_mut().unwrap().header.mode = WorkMode::Closed;
    assert_eq!(
        protected.charge_attempted_in_order([0]),
        Err(LedgerCause::PublicationAlreadyCompleted)
    );
    assert_eq!(counters(&work), [(3, 2, 0)]);
}

#[test]
fn parser_channels_and_orphan_pending_also_prevent_quiescence() {
    let mut work = owner([(1, 10, 10)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
    view.header.parser_parent_open = true;
    assert_eq!(
        kernel::check_quiescent(view.rows, view.header),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    assert!(matches!(
        layout.admit_observed(&mut view, [0], WorkLane::Producer),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    let mut internal = layout
        .admit_observed(&mut view, [1], WorkLane::Internal)
        .unwrap();
    internal.resolve(&layout, &mut view, false).unwrap();
    assert!(view.header.parser_parent_open);
    view.header.parser_parent_open = false;
    view.header.parser_child_open = true;
    assert_eq!(
        kernel::check_quiescent(view.rows, view.header),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    view.header.parser_child_open = false;
    view.rows[0].state.pending_consumed = 1;
    assert_eq!(
        kernel::check_quiescent(view.rows, view.header),
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

    let mut work = owner([(1, 10, 10)]);
    let (catalog, mut view) = work.exclusive().unwrap();
    let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
    view.header.open_envelopes = u64::MAX;
    assert!(matches!(
        layout.admit_observed(&mut view, [2], WorkLane::Internal),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    ));
    assert_eq!(row_counters(view.rows), [(0, 0, 0)]);
    assert_eq!(view.header.open_envelopes, u64::MAX);
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

    let mut work = LocalWork::new(
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
    {
        let Ok((catalog, mut view)) = work.exclusive() else {
            panic!("valid opaque work owner was rejected");
        };
        let Ok(layout) = WorkLayout::bind(catalog, &[OpaqueKey(1)], WorkGrant::Ordinary) else {
            panic!("valid opaque work layout was rejected");
        };
        let Ok(mut frame) = layout.admit_observed(&mut view, [1], WorkLane::Producer) else {
            panic!("valid opaque work envelope was rejected");
        };
        assert!(matches!(
            frame.complete_batch_with_outcome(&layout, &mut view, [1]),
            LinearWorkCompletion::Completed
        ));
        assert!(frame.resolve(&layout, &mut view, false).is_ok());
    }
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
        {
            let mut state = work.borrow_state_mut().unwrap();
            let mut view = state.view(work.catalogue());
            let layout = roles.layout;
            let mut frame = layout
                .admit_observed(&mut view, [2, 2, 2], WorkLane::Internal)
                .unwrap();
            assert!(matches!(
                frame.complete_batch_with_outcome(&layout, &mut view, [1, 1, 1]),
                LinearWorkCompletion::Completed
            ));
            frame.resolve(&layout, &mut view, false).unwrap();
        }
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
