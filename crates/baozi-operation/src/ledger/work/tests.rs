use std::cell::Cell;
use std::cmp::Ordering;
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::kernel::check_effective;
use super::*;

fn owner<const N: usize>(policies: [(u8, u64, u64); N]) -> (Vec<u8>, WorkState<u8>) {
    let catalog = policies
        .iter()
        .map(|(dimension, _, _)| *dimension)
        .collect();
    let state = WorkState::new(
        N,
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
    );
    (catalog, state)
}

fn counters<const N: usize>(state: &WorkState<u8>) -> [(u64, u64, u64); N] {
    row_counters(&state.rows)
}

fn row_counters<const N: usize>(rows: &[WorkEntry<u8>]) -> [(u64, u64, u64); N] {
    assert_eq!(rows.len(), N);
    std::array::from_fn(|index| {
        let row = rows[index].state;
        (row.attempted, row.consumed, row.pending_consumed)
    })
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
    let state = WorkState::new(catalog.len(), rows);
    let (catalog, state) = (catalog, state);
    assert_eq!(catalog.as_ptr(), catalog_pointer);
    assert_eq!(catalog.capacity(), catalog_capacity);
    assert_eq!(state.rows.as_ptr(), rows_pointer);
    assert_eq!(state.rows.capacity(), rows_capacity);
}

#[test]
fn moving_state_preserves_exact_rows_and_outstanding_ownership() {
    let (catalog, mut work) = owner([(1, 20, 20)]);
    let rows_pointer = work.rows.as_ptr();
    {
        let layout = work
            .bind_layout(&catalog, [1], WorkGrant::Ordinary)
            .unwrap();
        let mut view = work.view(&catalog);
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
    let state = work;
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
fn binding_validates_all_members_in_caller_order_before_mode() {
    let (catalog, mut work) = owner([(1, 10, 10)]);
    work.header.mode = WorkMode::OrdinarySealed;
    assert!(matches!(
        work.bind_layout(&catalog, [3, 2], WorkGrant::Ordinary),
        Err(LedgerCause::UnknownDimension { dimension: 3 })
    ));
    assert!(matches!(
        work.bind_layout(&catalog, [1], WorkGrant::Ordinary),
        Err(LedgerCause::PublicationAlreadyPending)
    ));
    assert_eq!(counters(&work), [(0, 0, 0)]);
}

#[test]
fn attempted_stops_in_caller_order_and_keeps_representable_limit_failure() {
    let (catalog, mut work) = owner([(1, 10, 10), (2, 5, 5)]);
    let roles = work
        .bind_layout(&catalog, [2, 1, 2, 1], WorkGrant::Ordinary)
        .unwrap();
    assert_eq!(
        roles.charge_attempted_in_order(&mut work.view(&catalog), [2, 4, 4, 3]),
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
    let (catalog, mut work) = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    work.rows[1].state.attempted = u64::MAX - 1;
    let roles = work
        .bind_layout(&catalog, [1, 2], WorkGrant::Ordinary)
        .unwrap();
    assert_eq!(
        roles.charge_attempted_in_order(&mut work.view(&catalog), [3, 2]),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 2
        })
    );
    assert_eq!(counters(&work), [(3, 0, 0), (u64::MAX - 1, 0, 0)]);
}

#[test]
fn direct_completion_sorts_bound_positions_and_commits_duplicates_atomically() {
    let (catalog, mut work) = owner([(1, 20, 1), (2, 20, 0)]);
    let roles = work
        .bind_layout(&catalog, [2, 1, 2], WorkGrant::Ordinary)
        .unwrap();
    roles
        .charge_attempted_in_order(&mut work.view(&catalog), [3, 4, 2])
        .unwrap();
    assert_eq!(
        roles.consume_direct_atomic(&mut work.view(&catalog), [2, 2, 1]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 2,
            limit: 1,
        })
    );
    assert_eq!(counters(&work), [(4, 0, 0), (5, 0, 0)]);
    work.rows[0].policy.consumed_limit = 20;
    work.rows[1].policy.consumed_limit = 20;
    roles
        .consume_direct_atomic(&mut work.view(&catalog), [2, 2, 1])
        .unwrap();
    assert_eq!(counters(&work), [(4, 2, 0), (5, 3, 0)]);
}

#[test]
fn direct_completion_accounts_for_other_frames_pending_capacity() {
    let (catalog, mut work) = owner([(1, 10, 10)]);
    let (layout, mut frame) = {
        let layout = work
            .bind_layout(&catalog, [1], WorkGrant::Ordinary)
            .unwrap();
        let mut view = work.view(&catalog);
        let frame = layout
            .admit_observed(&mut view, [4], WorkLane::Producer)
            .unwrap();
        (layout, frame)
    };
    let roles = work
        .bind_layout(&catalog, [1], WorkGrant::Ordinary)
        .unwrap();
    roles
        .charge_attempted_in_order(&mut work.view(&catalog), [2])
        .unwrap();
    assert_eq!(
        roles.consume_direct_atomic(&mut work.view(&catalog), [3]),
        Err(LedgerCause::ConsumedExceedsAttempted {
            dimension: 1,
            consumed: 7,
            attempted: 6,
        })
    );
    assert_eq!(counters(&work), [(6, 0, 4)]);
    roles
        .consume_direct_atomic(&mut work.view(&catalog), [2])
        .unwrap();
    {
        let mut view = work.view(&catalog);
        assert!(matches!(
            frame.complete_batch_with_outcome(&layout, &mut view, [4]),
            LinearWorkCompletion::Completed
        ));
        assert_eq!(row_counters(view.rows), [(6, 6, 0)]);
        frame.resolve(&layout, &mut view, false).unwrap();
    }
    work.check_quiescent(catalog.len()).unwrap();
}

#[test]
fn direct_overflow_in_later_dimension_leaves_entire_consumed_group_unchanged() {
    let (catalog, mut work) = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    work.rows[0].state.attempted = 2;
    work.rows[1].state = WorkDimensionState {
        attempted: u64::MAX,
        consumed: u64::MAX,
        pending_consumed: 0,
    };
    let roles = work
        .bind_layout(&catalog, [2, 1], WorkGrant::Ordinary)
        .unwrap();
    assert_eq!(
        roles.consume_direct_atomic(&mut work.view(&catalog), [1, 2]),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 2
        })
    );
    assert_eq!(counters(&work), [(2, 0, 0), (u64::MAX, u64::MAX, 0)]);
}

#[test]
fn direct_and_preflight_preserve_earlier_group_limit_before_later_duplicate_overflow() {
    let (catalog, mut work) = owner([(1, 1, 1), (2, u64::MAX, u64::MAX)]);
    work.rows[0].state.attempted = 2;
    work.rows[1].state.attempted = u64::MAX;
    let roles = work
        .bind_layout(&catalog, [2, 1, 2], WorkGrant::Ordinary)
        .unwrap();
    assert_eq!(
        roles.consume_direct_atomic(&mut work.view(&catalog), [u64::MAX, 2, 1]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 2,
            limit: 1,
        })
    );
    let caller_first = work
        .bind_layout(&catalog, [1, 2, 2], WorkGrant::Ordinary)
        .unwrap();
    assert_eq!(
        caller_first.preflight_repeated_observed(
            &catalog,
            &work.rows,
            &work.header,
            [1, u64::MAX, 1],
            NonZeroU64::new(1).unwrap(),
        ),
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
    let (catalog, mut work) = owner([(1, 10, 10), (2, 10, 2)]);
    let roles = work
        .bind_layout(&catalog, [1, 2, 1], WorkGrant::Ordinary)
        .unwrap();
    assert_eq!(
        roles.charge_observed_ordered(&mut work.view(&catalog), [3, 4, 5]),
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
    let (catalog, mut work) = owner([(1, 10, 10), (2, 2, 2)]);
    let roles = work
        .bind_layout(&catalog, [1, 2], WorkGrant::Ordinary)
        .unwrap();
    assert_eq!(
        roles.charge_observed_ordered(&mut work.view(&catalog), [3, 4]),
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
    let (catalog, mut work) = owner([(1, 10, 10), (2, 10, 10)]);
    let attempted = work
        .bind_layout(&catalog, [1, 2], WorkGrant::Ordinary)
        .unwrap();
    attempted
        .charge_attempted_in_order(&mut work.view(&catalog), [3, 4])
        .unwrap();
    let roles = work
        .bind_layout(&catalog, [2, 1, 2], WorkGrant::Ordinary)
        .unwrap();
    assert_eq!(
        roles.consume_observed_in_order(&mut work.view(&catalog), [2, 2, 3]),
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
    let (catalog, mut work) = owner([(1, 10, 10)]);
    let (layout, mut frame) = {
        let layout = work
            .bind_layout(&catalog, [1], WorkGrant::Ordinary)
            .unwrap();
        let mut view = work.view(&catalog);
        let frame = layout
            .admit_observed(&mut view, [4], WorkLane::Producer)
            .unwrap();
        (layout, frame)
    };
    let roles = work
        .bind_layout(&catalog, [1], WorkGrant::Ordinary)
        .unwrap();
    assert_eq!(
        roles.consume_observed_in_order(&mut work.view(&catalog), [1]),
        Err(LedgerCause::ConsumedExceedsAttempted {
            dimension: 1,
            consumed: 5,
            attempted: 4
        })
    );
    assert_eq!(counters(&work), [(4, 1, 4)]);
    InternalWork {
        catalog: &catalog,
        state: &mut work,
    }
    .resolve(layout, &mut frame, false)
    .unwrap();
    assert_eq!(counters(&work), [(4, 1, 0)]);
    work.check_quiescent(catalog.len()).unwrap();
}

#[test]
fn consumed_observation_retains_limit_failure_but_not_unrepresentable_increment() {
    let (catalog, mut work) = owner([(1, u64::MAX, 2)]);
    let roles = work
        .bind_layout(&catalog, [1], WorkGrant::Ordinary)
        .unwrap();
    roles
        .charge_attempted_in_order(&mut work.view(&catalog), [u64::MAX])
        .unwrap();
    assert_eq!(
        roles.consume_observed_in_order(&mut work.view(&catalog), [3]),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 3,
            limit: 2,
        })
    );
    assert_eq!(counters(&work), [(u64::MAX, 3, 0)]);
    assert_eq!(
        roles.consume_observed_in_order(&mut work.view(&catalog), [u64::MAX]),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 1
        })
    );
    assert_eq!(counters(&work), [(u64::MAX, 3, 0)]);
}

#[test]
fn consumed_observation_stores_representable_prefix_before_effective_overflow() {
    let (catalog, mut work) = owner([(1, u64::MAX, u64::MAX)]);
    work.rows[0].state = WorkDimensionState {
        attempted: u64::MAX,
        consumed: u64::MAX - 1,
        pending_consumed: 2,
    };
    let roles = work
        .bind_layout(&catalog, [1], WorkGrant::Ordinary)
        .unwrap();
    assert_eq!(
        roles.consume_observed_in_order(&mut work.view(&catalog), [1]),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Consumed,
            dimension: 1
        })
    );
    assert_eq!(counters(&work), [(u64::MAX, u64::MAX, 2)]);
}

#[test]
fn repeated_preflight_aggregates_duplicates_without_changing_evidence() {
    let (catalog, work) = owner([(1, 11, 11), (2, 20, 20)]);
    let roles = work
        .bind_layout(&catalog, [2, 1, 1], WorkGrant::Ordinary)
        .unwrap();
    let repeat = NonZeroU64::new(3).unwrap();
    roles
        .preflight_repeated_observed(&catalog, &work.rows, &work.header, [2, 2, 1], repeat)
        .unwrap();
    assert_eq!(
        roles.preflight_repeated_observed(&catalog, &work.rows, &work.header, [2, 3, 1], repeat),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 1,
            value: 12,
            limit: 11
        })
    );
    assert_eq!(
        roles.preflight_repeated_observed(
            &catalog,
            &work.rows,
            &work.header,
            [u64::MAX, 0, 0],
            repeat
        ),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 2
        })
    );
    assert_eq!(counters(&work), [(0, 0, 0), (0, 0, 0)]);
}

#[test]
fn repeated_preflight_uses_current_pending_and_current_policy() {
    let (catalog, mut work) = owner([(1, 20, 20)]);
    let (layout, mut frame) = {
        let layout = work
            .bind_layout(&catalog, [1], WorkGrant::Ordinary)
            .unwrap();
        let mut view = work.view(&catalog);
        let frame = layout
            .admit_observed(&mut view, [4], WorkLane::Internal)
            .unwrap();
        (layout, frame)
    };
    let roles = work
        .bind_layout(&catalog, [1], WorkGrant::Ordinary)
        .unwrap();
    work.rows[0].policy.consumed_limit = 7;
    assert_eq!(
        roles.preflight_repeated_observed(
            &catalog,
            &work.rows,
            &work.header,
            [2],
            NonZeroU64::new(2).unwrap(),
        ),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 8,
            limit: 7
        })
    );
    assert_eq!(counters(&work), [(4, 0, 4)]);
    InternalWork {
        catalog: &catalog,
        state: &mut work,
    }
    .resolve(layout, &mut frame, false)
    .unwrap();
}

#[test]
fn admission_keeps_every_representable_attempt_and_prefers_attempted_failure() {
    let (catalog, mut work) = owner([(1, 20, 0), (2, 3, 3)]);
    let layout = work
        .bind_layout(&catalog, [1, 2, 2], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(1, 10, 2)]);
    let layout = work
        .bind_layout(&catalog, [1, 1], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    let distinct = work
        .bind_layout(&catalog, [1, 2], WorkGrant::Ordinary)
        .unwrap();
    let duplicate = work
        .bind_layout(&catalog, [1, 1], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    let layout = work
        .bind_layout(&catalog, [1, 2], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(1, 20, 20), (2, 20, 20)]);
    let layout = work
        .bind_layout(&catalog, [2, 1, 2], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(1, 20, 20)]);
    let layout = work
        .bind_layout(&catalog, [1, 1], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(1, 20, 20)]);
    let layout = work
        .bind_layout(&catalog, [1], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(1, 20, 20), (2, 20, 20)]);
    let layout = work
        .bind_layout(&catalog, [2, 1, 2], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(1, u64::MAX, u64::MAX), (2, u64::MAX, u64::MAX)]);
    let layout = work
        .bind_layout(&catalog, [1, 2], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(1, 10, 10)]);
    {
        let layout = work
            .bind_layout(&catalog, [1], WorkGrant::Ordinary)
            .unwrap();
        let mut view = work.view(&catalog);
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
    assert!(work.header.producer_open);
    assert_eq!(
        work.check_quiescent(catalog.len()),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
}

#[test]
fn zero_and_empty_envelopes_still_own_persistent_channels() {
    let (catalog, mut work) = owner([(1, 0, 0)]);
    {
        let layout = work
            .bind_layout(&catalog, [1], WorkGrant::Ordinary)
            .unwrap();
        let empty_layout = work.bind_layout(&catalog, [], WorkGrant::Ordinary).unwrap();
        let mut view = work.view(&catalog);
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
        work.check_quiescent(catalog.len()),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
}

#[test]
fn replenishment_keeps_one_envelope_and_records_each_immediate_prefix() {
    let (catalog, mut work) = owner([(10, 8, 8), (90, 8, 8)]);
    {
        let layout = work
            .bind_layout(&catalog, [90], WorkGrant::Ordinary)
            .unwrap();
        let mut view = work.view(&catalog);
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
    work.check_quiescent(catalog.len()).unwrap();
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
        let (catalog, mut fresh) = owner([(90, attempted, consumed)]);
        let fresh_layout = fresh
            .bind_layout(&catalog, [90], WorkGrant::Ordinary)
            .unwrap();
        let mut fresh_view = fresh.view(&catalog);
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

        let (catalog, mut sequence) = owner([(90, attempted, consumed)]);
        let sequence_layout = sequence
            .bind_layout(&catalog, [90], WorkGrant::Ordinary)
            .unwrap();
        let mut sequence_view = sequence.view(&catalog);
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
    let (catalog, mut work) = owner([(90, u64::MAX, u64::MAX)]);
    let layout = work
        .bind_layout(&catalog, [90], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(90, 20, 20)]);
    let layout = work
        .bind_layout(&catalog, [90], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
        let (catalog, mut work) = owner([(10, 20, 20), (90, 20, 20)]);
        let layout = work
            .bind_layout(&catalog, [90], WorkGrant::Ordinary)
            .unwrap();
        let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(10, 20, 20), (90, 20, 20)]);
    let layout = work
        .bind_layout(&catalog, [90], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(90, 20, 20)]);
    let ordinary_layout = work
        .bind_layout(&catalog, [90], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let protected_layout = WorkLayout::bind(view.catalog, &[90], WorkGrant::Protected).unwrap();
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

    let (catalog, mut work) = owner([(90, 20, 20)]);
    let layout = work
        .bind_layout(&catalog, [90], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(1, 10, 10)]);
    let layout = work
        .bind_layout(&catalog, [1, 1], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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

    let (catalog, mut work) = owner([(1, 10, 10)]);
    let layout = work
        .bind_layout(&catalog, [1], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
fn interim_integrity_allows_pending_work_without_quiescence() {
    let (catalog, mut work) = owner([(1, 10, 10)]);
    let (layout, mut frame) = {
        let layout = work
            .bind_layout(&catalog, [1], WorkGrant::Ordinary)
            .unwrap();
        let mut view = work.view(&catalog);
        let frame = layout
            .admit_observed(&mut view, [3], WorkLane::Producer)
            .unwrap();
        (layout, frame)
    };
    work.check_integrity(catalog.len()).unwrap();
    assert_eq!(
        work.check_quiescent(catalog.len()),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    );
    InternalWork {
        catalog: &catalog,
        state: &mut work,
    }
    .resolve(layout, &mut frame, false)
    .unwrap();
    assert!(!frame.active);
    assert_eq!(counters(&work), [(3, 0, 0)]);
    work.check_integrity(catalog.len()).unwrap();
    work.check_quiescent(catalog.len()).unwrap();
}

#[test]
fn interim_integrity_retains_header_fault_after_pending_cleanup() {
    let (catalog, mut work) = owner([(1, 10, 10)]);
    let layout = work
        .bind_layout(&catalog, [1], WorkGrant::Ordinary)
        .unwrap();
    let mut frame = layout
        .admit_observed(&mut work.view(&catalog), [3], WorkLane::Producer)
        .unwrap();
    work.header.integrity_fault = true;
    assert_eq!(
        work.check_integrity(catalog.len()),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    InternalWork {
        catalog: &catalog,
        state: &mut work,
    }
    .resolve(layout, &mut frame, false)
    .unwrap();
    assert!(!frame.active);
    assert_eq!(counters(&work), [(3, 0, 0)]);
    assert_eq!(work.header.open_envelopes, 0);
    assert!(!work.header.producer_open);
    assert_eq!(
        work.check_integrity(catalog.len()),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
}

#[test]
fn modes_invalidate_old_layouts_and_protected_work_needs_private_grant_and_lane() {
    let (catalog, mut work) = owner([(1, 20, 20)]);
    let ordinary = work
        .bind_layout(&catalog, [1], WorkGrant::Ordinary)
        .unwrap();
    assert!(matches!(
        work.bind_layout(&catalog, [1], WorkGrant::Protected),
        Err(LedgerCause::InactiveLinearWorkPermit)
    ));
    work.check_quiescent(catalog.len()).unwrap();
    work.header.mode = WorkMode::OrdinarySealed;
    assert_eq!(
        ordinary.validate_active(&work.view(&catalog)),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    work.header.protected_dimension = Some(0);
    work.header.mode = WorkMode::Protected;
    assert_eq!(
        ordinary.validate_active(&work.view(&catalog)),
        Err(LedgerCause::PublicationAlreadyPending)
    );
    let protected_layout = work
        .bind_layout(&catalog, [1], WorkGrant::Protected)
        .unwrap();
    {
        let mut view = work.view(&catalog);
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
    work.check_quiescent(catalog.len()).unwrap();
    let protected = work
        .bind_layout(&catalog, [1], WorkGrant::Protected)
        .unwrap();
    work.header.mode = WorkMode::Closed;
    assert_eq!(
        protected.validate_active(&work.view(&catalog)),
        Err(LedgerCause::PublicationAlreadyCompleted)
    );
    assert_eq!(counters(&work), [(3, 2, 0)]);
}

#[test]
fn parser_channels_and_orphan_pending_also_prevent_quiescence() {
    let (catalog, mut work) = owner([(1, 10, 10)]);
    let layout = work
        .bind_layout(&catalog, [1], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
fn numeric_view_mutates_only_its_borrowed_state() {
    let (catalog, mut first) = owner([(1, 10, 10)]);
    let (_, second) = owner([(1, 1, 1)]);
    let roles = first
        .bind_layout(&catalog, [1], WorkGrant::Ordinary)
        .unwrap();
    {
        let mut view = first.view(&catalog);
        assert!(std::ptr::eq(view.catalog.as_ptr(), catalog.as_ptr()));
        roles.charge_observed_ordered(&mut view, [3]).unwrap();
    }
    assert_eq!(counters(&first), [(3, 3, 0)]);
    assert_eq!(counters(&second), [(0, 0, 0)]);
}

#[test]
fn shape_mismatch_and_open_count_overflow_are_contained_without_numeric_changes() {
    let catalog = vec![1_u8];
    let malformed = WorkState::new(catalog.len(), Vec::new());
    assert!(malformed.header.integrity_fault);
    assert_eq!(
        malformed.check_quiescent(catalog.len()),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    assert!(matches!(
        malformed.bind_layout(&catalog, [1], WorkGrant::Ordinary),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    ));

    let (catalog, mut work) = owner([(1, 10, 10)]);
    let layout = work
        .bind_layout(&catalog, [1], WorkGrant::Ordinary)
        .unwrap();
    let mut view = work.view(&catalog);
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
    let (catalog, mut work) = owner([(10, 20, 20), (30, 20, 20), (90, 20, 20)]);
    let roles = work
        .bind_layout(&catalog, [30, 90, 30, 10], WorkGrant::Ordinary)
        .unwrap();
    work.header.protected_dimension = Some(2);
    assert_eq!(
        roles.validate_active(&work.view(&catalog)),
        Err(LedgerCause::UnknownDimension { dimension: 90 })
    );
    assert_eq!(
        roles.preflight_repeated_observed(
            &catalog,
            &work.rows,
            &work.header,
            [0; 4],
            NonZeroU64::new(1).unwrap(),
        ),
        Err(LedgerCause::UnknownDimension { dimension: 90 })
    );
    assert_eq!(counters(&work), [(0, 0, 0); 3]);
    work.check_quiescent(catalog.len()).unwrap();
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
    let catalog = vec![registered];
    let mut work = WorkState::new(
        catalog.len(),
        vec![WorkEntry {
            dimension: registered,
            policy: WorkPolicy {
                attempted_limit: 1,
                consumed_limit: 1,
            },
            state: WorkDimensionState::default(),
        }],
    );
    let roles = work
        .bind_layout(&catalog, [caller], WorkGrant::Ordinary)
        .unwrap();
    let Err(LedgerCause::LimitExceeded {
        class: CounterClass::Attempted,
        dimension,
        value: 2,
        limit: 1,
    }) = roles.charge_attempted_in_order(&mut work.view(&catalog), [2])
    else {
        panic!("expected the bound attempted limit failure");
    };
    assert_eq!(dimension.id, registered.id);
    assert_eq!(dimension.label, registered.label);

    work.header.protected_dimension = Some(0);
    let Err(LedgerCause::UnknownDimension { dimension }) =
        roles.validate_active(&work.view(&catalog))
    else {
        panic!("expected the revoked bound role");
    };
    assert_eq!(dimension.label, registered.label);
    let Err(LedgerCause::UnknownDimension { dimension }) =
        work.bind_layout(&catalog, [caller], WorkGrant::Ordinary)
    else {
        panic!("expected the protected role to reject binding");
    };
    assert_eq!(dimension.label, caller.label);

    let missing = DiagnosticKey {
        id: 30,
        label: "missing caller",
    };
    let Err(LedgerCause::UnknownDimension { dimension }) =
        work.bind_layout(&catalog, [missing], WorkGrant::Ordinary)
    else {
        panic!("expected the absent caller key");
    };
    assert_eq!(dimension.id, missing.id);
    assert_eq!(dimension.label, missing.label);
}

#[test]
fn direct_state_and_numeric_interfaces_accept_keys_without_debug() {
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    struct OpaqueKey(u8);

    let catalog = vec![OpaqueKey(1)];
    let mut work = WorkState::new(
        catalog.len(),
        vec![WorkEntry {
            dimension: OpaqueKey(1),
            policy: WorkPolicy {
                attempted_limit: 10,
                consumed_limit: 10,
            },
            state: WorkDimensionState::default(),
        }],
    );
    assert_eq!(catalog.len(), 1);
    assert!(work.check_integrity(catalog.len()).is_ok());
    let Ok(roles) = work.bind_layout(&catalog, [OpaqueKey(1)], WorkGrant::Ordinary) else {
        panic!("valid opaque key was rejected");
    };
    assert!(
        roles
            .charge_observed_ordered(&mut work.view(&catalog), [1])
            .is_ok()
    );
    {
        let Ok(layout) = work.bind_layout(&catalog, [OpaqueKey(1)], WorkGrant::Ordinary) else {
            panic!("valid opaque work layout was rejected");
        };
        let mut view = work.view(&catalog);
        let Ok(mut frame) = layout.admit_observed(&mut view, [1], WorkLane::Producer) else {
            panic!("valid opaque work envelope was rejected");
        };
        assert!(matches!(
            frame.complete_batch_with_outcome(&layout, &mut view, [1]),
            LinearWorkCompletion::Completed
        ));
        assert!(frame.resolve(&layout, &mut view, false).is_ok());
    }
    assert!(work.check_quiescent(catalog.len()).is_ok());
    assert!(!work.header.integrity_fault);
}

#[derive(Debug, Clone, Copy)]
struct ComparingKey(u8);

thread_local! {
    static PANIC_COMPARISON: Cell<bool> = const { Cell::new(false) };
    static COMPARISONS: Cell<u64> = const { Cell::new(0) };
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
        PANIC_COMPARISON.with(|panic| assert!(!panic.get(), "injected comparator panic"));
        self.0.cmp(&other.0)
    }
}

#[test]
fn comparator_panic_leaves_state_unchanged_and_bound_numeric_paths_never_compare() {
    let catalog = vec![ComparingKey(1), ComparingKey(2)];
    let mut work = WorkState::new(
        catalog.len(),
        [1, 2]
            .map(|value| WorkEntry {
                dimension: ComparingKey(value),
                policy: WorkPolicy {
                    attempted_limit: 20,
                    consumed_limit: 20,
                },
                state: WorkDimensionState::default(),
            })
            .into_iter()
            .collect(),
    );
    PANIC_COMPARISON.with(|panic| panic.set(true));
    let panicked = catch_unwind(AssertUnwindSafe(|| {
        let _ = work.bind_layout(
            &catalog,
            [ComparingKey(2), ComparingKey(1)],
            WorkGrant::Ordinary,
        );
    }));
    PANIC_COMPARISON.with(|panic| panic.set(false));
    assert!(panicked.is_err());
    work.check_integrity(catalog.len()).unwrap();
    assert_eq!(work.rows[0].state, WorkDimensionState::default());
    assert_eq!(work.rows[1].state, WorkDimensionState::default());
    let roles = work
        .bind_layout(
            &catalog,
            [ComparingKey(2), ComparingKey(1), ComparingKey(2)],
            WorkGrant::Ordinary,
        )
        .unwrap();
    let comparisons = COMPARISONS.with(Cell::get);
    PANIC_COMPARISON.with(|panic| panic.set(true));
    {
        let mut view = work.view(&catalog);
        roles.validate_active(&view).unwrap();
        roles
            .charge_attempted_in_order(&mut view, [1, 1, 1])
            .unwrap();
        roles.consume_direct_atomic(&mut view, [1, 1, 1]).unwrap();
        roles.charge_observed_ordered(&mut view, [1, 1, 1]).unwrap();
        roles
            .consume_observed_in_order(&mut view, [0, 0, 0])
            .unwrap();
        roles
            .preflight_repeated_observed(
                view.catalog,
                view.rows,
                view.header,
                [1, 1, 1],
                NonZeroU64::new(2).unwrap(),
            )
            .unwrap();
        let mut frame = roles
            .admit_observed(&mut view, [2, 2, 2], WorkLane::Internal)
            .unwrap();
        assert!(matches!(
            frame.complete_batch_with_outcome(&roles, &mut view, [1, 1, 1]),
            LinearWorkCompletion::Completed
        ));
        frame.resolve(&roles, &mut view, false).unwrap();
    }
    work.check_quiescent(catalog.len()).unwrap();
    assert_eq!(COMPARISONS.with(Cell::get), comparisons);
    assert_eq!(
        work.rows[0].state,
        WorkDimensionState {
            attempted: 4,
            consumed: 3,
            pending_consumed: 0
        }
    );
    assert_eq!(
        work.rows[1].state,
        WorkDimensionState {
            attempted: 8,
            consumed: 6,
            pending_consumed: 0
        }
    );
    PANIC_COMPARISON.with(|panic| panic.set(false));
}
