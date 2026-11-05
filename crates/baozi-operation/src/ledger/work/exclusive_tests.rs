//! Literal oracles for the actual kernel, exercised over fixed rows and the dynamic owner.
use super::*;

fn row(dimension: u8, attempted: u64, consumed: u64) -> WorkEntry<u8> {
    WorkEntry {
        dimension,
        policy: WorkPolicy {
            attempted_limit: attempted,
            consumed_limit: consumed,
        },
        state: WorkDimensionState::default(),
    }
}

fn header() -> WorkHeader {
    WorkHeader {
        mode: WorkMode::Ordinary,
        open_envelopes: 0,
        producer_open: false,
        parser_parent_open: false,
        parser_child_open: false,
        protected_open: false,
        protected_dimension: None,
        integrity_fault: false,
    }
}

fn counters(view: &WorkView<'_, u8>) -> [(u64, u64, u64); 2] {
    std::array::from_fn(|index| {
        let state = view.rows[index].state;
        (state.attempted, state.consumed, state.pending_consumed)
    })
}

fn duplicate_internal_trace(view: &mut WorkView<'_, u8>) {
    let catalog = view.catalog;
    let layout = WorkLayout::bind(catalog, &[2, 1, 2], WorkGrant::Ordinary).unwrap();
    let mut producer = layout
        .admit_observed(view, [2, 1, 3], WorkLane::Producer)
        .unwrap();
    assert_eq!(counters(view), [(1, 0, 1), (5, 0, 5)]);
    assert!(matches!(
        producer.complete_batch_with_outcome(&layout, view, [1, 1, 2]),
        LinearWorkCompletion::Completed
    ));
    assert_eq!(counters(view), [(1, 1, 0), (5, 3, 2)]);
    let internal = WorkLayout::bind(catalog, &[2], WorkGrant::Ordinary).unwrap();
    let mut share = internal
        .admit_observed(view, [2], WorkLane::Internal)
        .unwrap();
    assert_eq!(counters(view), [(1, 1, 0), (7, 3, 4)]);
    assert_eq!(view.header.open_envelopes, 2);
    assert!(matches!(
        share.complete_batch_with_outcome(&internal, view, [1]),
        LinearWorkCompletion::Completed
    ));
    share.resolve(&internal, view, false).unwrap();
    assert_eq!(counters(view), [(1, 1, 0), (7, 4, 2)]);
    producer.resolve(&layout, view, false).unwrap();
    assert_eq!(counters(view), [(1, 1, 0), (7, 4, 0)]);
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn fixed_and_dynamic_rows_run_the_same_literal_duplicate_internal_trace() {
    let mut fixed = [row(1, 10, 10), row(2, 10, 10)];
    let mut fixed_header = header();
    duplicate_internal_trace(&mut WorkView {
        catalog: &[1, 2],
        rows: &mut fixed,
        header: &mut fixed_header,
    });
    let mut dynamic = LocalWork::new(vec![1, 2], vec![row(1, 10, 10), row(2, 10, 10)]);
    let (_, mut view) = dynamic.exclusive().unwrap();
    duplicate_internal_trace(&mut view);
}

#[test]
fn denied_admission_commits_all_representable_attempts_with_attempted_precedence() {
    let mut rows = [row(1, 2, 2), row(2, 10, 0)];
    let mut header = header();
    let mut view = WorkView {
        catalog: &[1, 2],
        rows: &mut rows,
        header: &mut header,
    };
    let layout = WorkLayout::bind(view.catalog, &[1, 2], WorkGrant::Ordinary).unwrap();
    assert!(matches!(
        layout.admit_observed(&mut view, [3, 1], WorkLane::Producer),
        Err(LedgerCause::LimitExceeded {
            class: CounterClass::Attempted,
            dimension: 1,
            value: 3,
            limit: 2,
        })
    ));
    assert_eq!(counters(&view), [(3, 0, 0), (1, 0, 0)]);
    assert_eq!(view.header.open_envelopes, 0);
}

#[test]
fn overflow_projects_all_rows_before_committing_any_attempt() {
    let mut rows = [row(1, u64::MAX, u64::MAX), row(2, u64::MAX, u64::MAX)];
    rows[0].state.attempted = 1;
    rows[1].state.attempted = u64::MAX;
    let mut header = header();
    let mut view = WorkView {
        catalog: &[1, 2],
        rows: &mut rows,
        header: &mut header,
    };
    let layout = WorkLayout::bind(view.catalog, &[1, 2], WorkGrant::Ordinary).unwrap();
    assert!(matches!(
        layout.admit_observed(&mut view, [1, 1], WorkLane::Producer),
        Err(LedgerCause::ArithmeticOverflow {
            class: CounterClass::Attempted,
            dimension: 2,
        })
    ));
    assert_eq!(counters(&view), [(1, 0, 0), (u64::MAX, 0, 0)]);
}

#[test]
fn completion_categories_keep_the_physical_prefix_and_resolve_only_the_suffix() {
    let mut rows = [row(1, 8, 8), row(2, 8, 8)];
    let mut header = header();
    let mut view = WorkView {
        catalog: &[1, 2],
        rows: &mut rows,
        header: &mut header,
    };
    let layout = WorkLayout::bind(view.catalog, &[1], WorkGrant::Ordinary).unwrap();
    let mut frame = layout
        .admit_observed(&mut view, [4], WorkLane::Producer)
        .unwrap();
    assert!(matches!(
        frame.complete_batch_with_outcome(&layout, &mut view, [5]),
        LinearWorkCompletion::NotCompleted(LedgerCause::InactiveLinearWorkPermit)
    ));
    assert_eq!(counters(&view), [(4, 0, 4), (0, 0, 0)]);
    view.rows[0].policy.consumed_limit = 2;
    assert!(matches!(
        frame.complete_batch_with_outcome(&layout, &mut view, [1]),
        LinearWorkCompletion::CompletedWithLedgerFailure(LedgerCause::LimitExceeded {
            class: CounterClass::Consumed,
            dimension: 1,
            value: 4,
            limit: 2,
        })
    ));
    assert_eq!(counters(&view), [(4, 1, 3), (0, 0, 0)]);
    assert_eq!(frame.remaining, [3]);
    frame.resolve(&layout, &mut view, false).unwrap();
    assert_eq!(counters(&view), [(4, 1, 0), (0, 0, 0)]);
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn zero_and_full_completion_still_own_a_frame_until_host_resolution() {
    for amount in [0, 1] {
        let mut owner = LocalWork::new(vec![1], vec![row(1, 8, 8)]);
        let (catalog, mut view) = owner.exclusive().unwrap();
        let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
        let mut frame = layout
            .admit_observed(&mut view, [amount], WorkLane::Producer)
            .unwrap();
        assert!(matches!(
            frame.complete_batch_with_outcome(&layout, &mut view, [amount]),
            LinearWorkCompletion::Completed
        ));
        assert!(matches!(
            kernel::check_quiescent(view.rows, view.header),
            Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        ));
        frame.resolve(&layout, &mut view, false).unwrap();
        kernel::check_quiescent(view.rows, view.header).unwrap();
    }
}

#[test]
fn exclusive_entry_does_not_recover_forgotten_legacy_ownership_or_borrows() {
    for amount in [0, 1] {
        let mut owner = LocalWork::new(vec![1], vec![row(1, 8, 8)]);
        let mut permit = owner
            .bind([1], WorkGrant::Ordinary)
            .unwrap()
            .admit_observed([amount], WorkLane::Producer)
            .unwrap();
        permit.complete(0, amount).unwrap();
        std::mem::forget(permit);
        assert!(matches!(
            owner.exclusive(),
            Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        ));
    }
    for shared in [false, true] {
        let mut owner = LocalWork::new(vec![1], vec![row(1, 8, 8)]);
        if shared {
            std::mem::forget(owner.borrow_state().unwrap());
        } else {
            std::mem::forget(owner.borrow_state_mut().unwrap());
        }
        assert!(matches!(
            owner.exclusive(),
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        ));
        assert!(owner.borrow_fault.get());
        assert!(owner.into_state().1.header.integrity_fault);
    }
    // Terminal composition also rejects a leaked shared Ref without relying on prior entry.
    let owner = LocalWork::new(vec![1], vec![row(1, 8, 8)]);
    std::mem::forget(owner.borrow_state().unwrap());
    assert!(owner.into_state().1.header.integrity_fault);
}

#[test]
fn internal_reborrow_preserves_its_enclosing_frame_and_external_quiescence_gate() {
    let mut owner = LocalWork::new(vec![1], vec![row(1, 8, 8)]);
    let (layout, mut enclosing) = {
        let (catalog, mut view) = owner.exclusive().unwrap();
        let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
        let enclosing = layout
            .admit_observed(&mut view, [3], WorkLane::Internal)
            .unwrap();
        (layout, enclosing)
    };
    assert!(matches!(
        owner.exclusive(),
        Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
    ));
    {
        let (_, mut view) = owner.reborrow().unwrap();
        let mut share = layout
            .admit_observed(&mut view, [2], WorkLane::Internal)
            .unwrap();
        assert_eq!(view.header.open_envelopes, 2);
        assert!(matches!(
            share.complete_batch_with_outcome(&layout, &mut view, [2]),
            LinearWorkCompletion::Completed
        ));
        share.resolve(&layout, &mut view, false).unwrap();
        assert_eq!(view.header.open_envelopes, 1);
        assert_eq!(view.rows[0].state.pending_consumed, 3);
        assert!(matches!(
            enclosing.complete_batch_with_outcome(&layout, &mut view, [1]),
            LinearWorkCompletion::Completed
        ));
        enclosing.resolve(&layout, &mut view, false).unwrap();
    }
    let (_, view) = owner.exclusive().unwrap();
    let state = view.rows[0].state;
    assert_eq!(
        (state.attempted, state.consumed, state.pending_consumed),
        (5, 3, 0)
    );
}

#[test]
fn internal_reborrow_never_recovers_forgotten_dynamic_guards() {
    for shared in [false, true] {
        let mut owner = LocalWork::new(vec![1], vec![row(1, 8, 8)]);
        if shared {
            std::mem::forget(owner.borrow_state().unwrap());
        } else {
            std::mem::forget(owner.borrow_state_mut().unwrap());
        }
        assert!(matches!(
            owner.reborrow(),
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        ));
        assert!(owner.borrow_fault.get());
        assert!(owner.into_state().1.header.integrity_fault);
    }
}

// Equal ordering identities intentionally have different diagnostic labels. These are the R2
// registered/caller values, with a third row label to detect projection from mutable rows.
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
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DiagnosticKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

fn diagnostic_work() -> LocalWork<DiagnosticKey> {
    LocalWork::new(
        [1, 37, 90]
            .map(|id| DiagnosticKey {
                id,
                label: "registered",
            })
            .to_vec(),
        [1, 37, 90]
            .map(|id| WorkEntry {
                dimension: DiagnosticKey {
                    id,
                    label: "row-only",
                },
                policy: WorkPolicy {
                    attempted_limit: 8,
                    consumed_limit: 8,
                },
                state: WorkDimensionState::default(),
            })
            .into_iter()
            .collect(),
    )
}

#[test]
fn borrowed_layout_binding_and_revalidation_keep_the_r2_diagnostic_sources() {
    let mut owner = diagnostic_work();
    let (catalog, view) = owner.exclusive().unwrap();
    let layout = {
        let supplied = [DiagnosticKey {
            id: 90,
            label: "caller",
        }];
        WorkLayout::bind(catalog, &supplied, WorkGrant::Ordinary).unwrap()
    };
    layout
        .validate_role_header(view.catalog, view.header)
        .unwrap();

    let missing = [DiagnosticKey {
        id: 30,
        label: "missing caller",
    }];
    let Err(LedgerCause::UnknownDimension { dimension }) =
        WorkLayout::bind(catalog, &missing, WorkGrant::Ordinary)
    else {
        panic!("expected the absent supplied key");
    };
    assert_eq!(dimension.id, 30);
    assert_eq!(dimension.label, "missing caller");

    view.header.protected_dimension = Some(2);
    let supplied = [DiagnosticKey {
        id: 90,
        label: "caller",
    }];
    let Err(LedgerCause::UnknownDimension { dimension }) =
        layout.validate_binding(view.header, &supplied)
    else {
        panic!("expected the supplied key on protected binding rejection");
    };
    assert_eq!(dimension.id, 90);
    assert_eq!(dimension.label, "caller");
    let Err(LedgerCause::UnknownDimension { dimension }) =
        layout.validate_role_header(view.catalog, view.header)
    else {
        panic!("expected the registered key on later role revocation");
    };
    assert_eq!(dimension.id, 90);
    assert_eq!(dimension.label, "registered");
    for row in view.rows.iter() {
        assert_eq!(row.state, WorkDimensionState::default());
    }
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn borrowed_duplicate_layout_retains_the_physical_prefix_and_registered_error_key() {
    let mut owner = diagnostic_work();
    let (catalog, mut view) = owner.exclusive().unwrap();
    let caller = DiagnosticKey {
        id: 90,
        label: "caller",
    };
    let first = DiagnosticKey {
        id: 1,
        label: "first caller",
    };
    let layout = WorkLayout::bind(catalog, &[caller, first, caller], WorkGrant::Ordinary).unwrap();
    let mut frame = layout
        .admit_observed(&mut view, [2, 1, 3], WorkLane::Producer)
        .unwrap();
    assert!(matches!(
        frame.complete_batch_with_outcome(&layout, &mut view, [1, 1, 2]),
        LinearWorkCompletion::Completed
    ));

    view.rows[2].policy.consumed_limit = 4;
    let LinearWorkCompletion::CompletedWithLedgerFailure(LedgerCause::LimitExceeded {
        class: CounterClass::Consumed,
        dimension,
        value: 5,
        limit: 4,
    }) = frame.complete_batch_with_outcome(&layout, &mut view, [0, 0, 1])
    else {
        panic!("expected the applied physical prefix with its registered diagnostic");
    };
    assert_eq!(dimension.id, 90);
    assert_eq!(dimension.label, "registered");
    assert_eq!(frame.remaining, [1, 0, 0]);
    let state = view.rows[2].state;
    assert_eq!(
        (state.attempted, state.consumed, state.pending_consumed),
        (5, 4, 1)
    );
    frame.resolve(&layout, &mut view, false).unwrap();
    let first = view.rows[0].state;
    assert_eq!(
        (first.attempted, first.consumed, first.pending_consumed),
        (1, 1, 0)
    );
    assert_eq!(view.rows[1].state, WorkDimensionState::default());
    let state = view.rows[2].state;
    assert_eq!(
        (state.attempted, state.consumed, state.pending_consumed),
        (5, 4, 0)
    );
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn borrowed_parser_layout_keeps_only_the_verified_prefix_and_registered_error_key() {
    let mut owner = diagnostic_work();
    let (catalog, mut view) = owner.exclusive().unwrap();
    let layout = {
        let supplied = [DiagnosticKey {
            id: 90,
            label: "caller",
        }];
        WorkLayout::bind(catalog, &supplied, WorkGrant::Ordinary).unwrap()
    };
    let mut frame = layout
        .begin_parser(&mut view, [2], [None], ParserChannel::Child)
        .unwrap();
    frame.complete(&layout, &mut view, [1]).unwrap();
    view.rows[2].policy.consumed_limit = 1;
    let Err(LedgerCause::LimitExceeded {
        class: CounterClass::Consumed,
        dimension,
        value: 2,
        limit: 1,
    }) = frame.complete(&layout, &mut view, [1])
    else {
        panic!("expected the uncompleted parser piece with its registered diagnostic");
    };
    assert_eq!(dimension.id, 90);
    assert_eq!(dimension.label, "registered");
    assert_eq!(frame.remaining, [1]);
    frame.resolve(&mut view).unwrap();
    let state = view.rows[2].state;
    assert_eq!(
        (state.attempted, state.consumed, state.pending_consumed),
        (2, 1, 0)
    );
    assert!(!view.header.parser_child_open);
    kernel::check_quiescent(view.rows, view.header).unwrap();
}

#[test]
fn registered_diagnostic_error_outlives_the_borrowed_catalog_owner() {
    let failure = {
        let mut owner = diagnostic_work();
        let (_, mut view) = owner.exclusive().unwrap();
        let supplied = [DiagnosticKey {
            id: 90,
            label: "caller",
        }];
        let layout = WorkLayout::bind(view.catalog, &supplied, WorkGrant::Ordinary).unwrap();
        let Err(cause) = layout.charge_attempted_in_order(&mut view, [9]) else {
            panic!("expected the representable denied attempt");
        };
        let state = view.rows[2].state;
        assert_eq!(
            (state.attempted, state.consumed, state.pending_consumed),
            (9, 0, 0)
        );
        cause
    };
    let LedgerCause::LimitExceeded {
        class: CounterClass::Attempted,
        dimension,
        value: 9,
        limit: 8,
    } = failure
    else {
        panic!("expected an owned registered-key error after the catalog owner was dropped");
    };
    assert_eq!(dimension.id, 90);
    assert_eq!(dimension.label, "registered");
}

#[test]
fn internal_split_preserves_the_original_catalog_rows_and_deferred_faults() {
    for mode in [
        WorkMode::Ordinary,
        WorkMode::OrdinarySealed,
        WorkMode::Protected,
        WorkMode::Closed,
    ] {
        let mut owner = LocalWork::new(vec![1, 2], vec![row(1, 8, 8), row(2, 8, 8)]);
        let catalog = owner.catalog.as_ptr();
        let rows = {
            let mut state = owner.borrow_state_mut().unwrap();
            state.header.mode = mode;
            state.header.open_envelopes = 1;
            state.header.integrity_fault = true;
            state.rows[0].state = WorkDimensionState {
                attempted: 9,
                consumed: 4,
                pending_consumed: 3,
            };
            state.rows.as_ptr()
        };
        let mut internal = owner.split_internal();
        assert_eq!(internal.catalog.as_ptr(), catalog);
        assert_eq!(internal.state.rows.as_ptr(), rows);
        assert_eq!(internal.state.header.mode, mode);
        assert_eq!(internal.state.header.open_envelopes, 1);
        assert_eq!(internal.state.rows[0].state.attempted, 9);
        assert_eq!(internal.state.rows[0].state.consumed, 4);
        assert_eq!(internal.state.rows[0].state.pending_consumed, 3);
        assert!(!internal.borrow_fault.get());
        assert!(matches!(
            internal.bind_ordinary([9]),
            Err(LedgerCause::UnknownDimension { dimension: 9 })
        ));
        assert!(matches!(
            internal.reborrow(),
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        ));
        assert_eq!(internal.state.rows[0].state.attempted, 9);
    }
}

#[test]
fn internal_binding_matches_local_key_integrity_mode_and_role_precedence() {
    for mode in [
        WorkMode::Ordinary,
        WorkMode::OrdinarySealed,
        WorkMode::Protected,
        WorkMode::Closed,
    ] {
        for integrity_fault in [false, true] {
            for grant in [WorkGrant::Ordinary, WorkGrant::Protected] {
                for dimensions in [[1], [2], [9]] {
                    let mut owner = LocalWork::new(vec![1, 2], vec![row(1, 8, 8), row(2, 8, 8)]);
                    {
                        let mut state = owner.borrow_state_mut().unwrap();
                        state.header.mode = mode;
                        state.header.protected_dimension = Some(1);
                        state.header.integrity_fault = integrity_fault;
                        state.rows[0].state.attempted = 9;
                    }
                    let expected = owner.bind(dimensions, grant).map(|_| ());
                    let internal = owner.split_internal();
                    let actual = match grant {
                        WorkGrant::Ordinary => internal.bind_ordinary(dimensions),
                        WorkGrant::Protected => internal.bind_protected(dimensions),
                    };
                    assert_eq!(actual.map(|_| ()), expected);
                    let share_grant = match mode {
                        WorkMode::Protected => WorkGrant::Protected,
                        _ => WorkGrant::Ordinary,
                    };
                    if grant == share_grant {
                        assert_eq!(internal.bind_share(dimensions).map(|_| ()), expected);
                    }
                    assert_eq!(internal.state.rows[0].state.attempted, 9);
                    assert_eq!(internal.state.rows[1].state, WorkDimensionState::default());
                    assert_eq!(internal.state.header.open_envelopes, 0);
                    assert!(!internal.borrow_fault.get());
                }
            }
        }
    }
}

#[test]
fn internal_split_preserves_forgotten_shared_and_mutable_borrow_precedence() {
    for shared in [false, true] {
        let mut owner = LocalWork::new(vec![1], vec![row(1, 8, 8)]);
        if shared {
            std::mem::forget(owner.borrow_state().unwrap());
        } else {
            std::mem::forget(owner.borrow_state_mut().unwrap());
        }
        {
            let mut root = owner.split_internal();
            let mut internal = root.loan();
            assert!(!internal.borrow_fault.get());
            assert!(matches!(
                internal.bind_ordinary([9]),
                Err(LedgerCause::UnknownDimension { dimension: 9 })
            ));
            assert!(matches!(
                internal.bind_protected([9]),
                Err(LedgerCause::UnknownDimension { dimension: 9 })
            ));
            assert!(!internal.borrow_fault.get());
            if shared {
                internal.bind_ordinary([1]).unwrap();
                internal.bind_share([1]).unwrap();
                assert!(matches!(
                    internal.bind_protected([1]),
                    Err(LedgerCause::InactiveLinearWorkPermit)
                ));
                assert!(!internal.borrow_fault.get());
            } else {
                assert!(matches!(
                    internal.bind_share([9]),
                    Err(LedgerCause::FinalSnapshotShapeMismatch)
                ));
                assert!(internal.borrow_fault.get());
            }
            assert!(matches!(
                internal.reborrow(),
                Err(LedgerCause::FinalSnapshotShapeMismatch)
            ));
            assert!(internal.borrow_fault.get());
            assert!(matches!(
                internal.bind_ordinary([1]),
                Err(LedgerCause::FinalSnapshotShapeMismatch)
            ));
            assert_eq!(internal.state.rows[0].state, WorkDimensionState::default());
        }
        assert!(owner.into_state().1.header.integrity_fault);
    }
}

#[test]
fn internal_root_reborrows_active_internal_and_producer_frames_without_external_entry() {
    for lane in [WorkLane::Internal, WorkLane::Producer] {
        let mut owner = LocalWork::new(vec![1, 2], vec![row(1, 8, 8), row(2, 8, 8)]);
        let (layout, mut enclosing) = {
            let (catalog, mut view) = owner.exclusive().unwrap();
            let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
            let frame = layout.admit_observed(&mut view, [3], lane).unwrap();
            (layout, frame)
        };
        assert!(matches!(
            owner.exclusive(),
            Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        ));
        {
            let mut internal = owner.split_internal();
            let share_layout = internal.bind_share([1]).unwrap();
            let mut loan = internal.loan();
            let mut work = loan.reborrow().unwrap();
            {
                let mut permission = work.reborrow();
                permission.validate_layout(share_layout).unwrap();
                let mut share = share_layout
                    .admit_observed(&mut permission.work, [2], WorkLane::Internal)
                    .unwrap();
                assert!(matches!(
                    share.complete_batch_with_outcome(&share_layout, &mut permission.work, [2]),
                    LinearWorkCompletion::Completed
                ));
                share
                    .resolve(&share_layout, &mut permission.work, false)
                    .unwrap();
            }
            assert_eq!(work.work.header.open_envelopes, 1);
            assert!(matches!(
                enclosing.complete_batch_with_outcome(&layout, &mut work.work, [1]),
                LinearWorkCompletion::Completed
            ));
            enclosing.resolve(&layout, &mut work.work, false).unwrap();
        }
        let (_, view) = owner.exclusive().unwrap();
        assert_eq!(counters(&view), [(5, 3, 0), (0, 0, 0)]);
    }
}

#[test]
fn internal_root_reborrows_active_parser_channels_without_external_entry() {
    for channel in [ParserChannel::Parent, ParserChannel::Child] {
        let mut owner = LocalWork::new(vec![1, 2], vec![row(1, 8, 8), row(2, 8, 8)]);
        let (layout, mut parser) = {
            let (catalog, mut view) = owner.exclusive().unwrap();
            let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
            let parser = layout
                .begin_parser(&mut view, [2], [None], channel)
                .unwrap();
            (layout, parser)
        };
        assert!(matches!(
            owner.exclusive(),
            Err(LedgerCause::OutstandingLinearWorkPermits { count: 1 })
        ));
        {
            let mut internal = owner.split_internal();
            let share_layout = internal.bind_share([1]).unwrap();
            let mut work = internal.reborrow().unwrap();
            let mut share = share_layout
                .admit_observed(&mut work.work, [1], WorkLane::Internal)
                .unwrap();
            assert!(matches!(
                share.complete_batch_with_outcome(&share_layout, &mut work.work, [1]),
                LinearWorkCompletion::Completed
            ));
            share.resolve(&share_layout, &mut work.work, false).unwrap();
            parser.complete(&layout, &mut work.work, [1]).unwrap();
            parser.resolve(&mut work.work).unwrap();
        }
        let (_, view) = owner.exclusive().unwrap();
        assert_eq!(counters(&view), [(3, 2, 0), (0, 0, 0)]);
    }
}

#[test]
fn internal_cleanup_releases_pending_suffix_after_header_or_borrow_faults() {
    for lane in [WorkLane::Internal, WorkLane::Producer, WorkLane::Protected] {
        for borrow_fault in [false, true] {
            let mut owner = LocalWork::new(vec![1, 2], vec![row(1, 8, 8), row(2, 8, 8)]);
            let (layout, mut frame) = {
                let (catalog, mut view) = owner.exclusive().unwrap();
                let grant = if lane == WorkLane::Protected {
                    view.header.mode = WorkMode::Protected;
                    view.header.protected_dimension = Some(0);
                    WorkGrant::Protected
                } else {
                    WorkGrant::Ordinary
                };
                let layout = WorkLayout::bind(catalog, &[1], grant).unwrap();
                let mut frame = layout.admit_observed(&mut view, [3], lane).unwrap();
                assert!(matches!(
                    frame.complete_batch_with_outcome(&layout, &mut view, [1]),
                    LinearWorkCompletion::Completed
                ));
                view.header.integrity_fault = !borrow_fault;
                frame.faulted = true;
                (layout, frame)
            };
            owner.borrow_fault.set(borrow_fault);
            {
                let mut internal = owner.split_internal();
                assert!(matches!(
                    internal.reborrow(),
                    Err(LedgerCause::FinalSnapshotShapeMismatch)
                ));
                internal.loan().resolve(layout, &mut frame, false).unwrap();
                assert!(!frame.active);
                assert_eq!(internal.state.rows[0].state.attempted, 3);
                assert_eq!(internal.state.rows[0].state.consumed, 1);
                assert_eq!(internal.state.rows[0].state.pending_consumed, 0);
                assert_eq!(internal.state.header.open_envelopes, 0);
                assert!(!internal.state.header.producer_open);
                assert!(!internal.state.header.protected_open);
                assert_eq!(
                    internal.resolve(layout, &mut frame, false),
                    Err(LedgerCause::InactiveLinearWorkPermit)
                );
            }
            let (_, state) = owner.into_state();
            assert!(state.header.integrity_fault);
            assert_eq!(state.rows[0].state.pending_consumed, 0);
        }
    }
}

#[test]
fn internal_cleanup_never_recovers_forgotten_guards_or_invalid_row_shape() {
    for shared in [false, true] {
        let mut owner = LocalWork::new(vec![1], vec![row(1, 8, 8)]);
        let (layout, mut frame) = {
            let (catalog, mut view) = owner.exclusive().unwrap();
            let layout = WorkLayout::bind(catalog, &[1], WorkGrant::Ordinary).unwrap();
            let frame = layout
                .admit_observed(&mut view, [2], WorkLane::Internal)
                .unwrap();
            (layout, frame)
        };
        if shared {
            std::mem::forget(owner.borrow_state().unwrap());
        } else {
            std::mem::forget(owner.borrow_state_mut().unwrap());
        }
        let mut internal = owner.split_internal();
        assert!(!internal.borrow_fault.get());
        assert_eq!(
            internal.loan().resolve(layout, &mut frame, false),
            Err(LedgerCause::FinalSnapshotShapeMismatch)
        );
        assert!(internal.borrow_fault.get());
        assert!(frame.active);
        assert_eq!(internal.state.rows[0].state.pending_consumed, 2);
    }

    let mut owner = LocalWork::new(vec![1], vec![row(1, 8, 8)]);
    let mut internal = owner.split_internal();
    let layout = internal.bind_ordinary([1]).unwrap();
    let mut frame = {
        let mut work = internal.reborrow().unwrap();
        layout
            .admit_observed(&mut work.work, [2], WorkLane::Internal)
            .unwrap()
    };
    internal.state.rows.clear();
    assert_eq!(
        internal.resolve(layout, &mut frame, false),
        Err(LedgerCause::FinalSnapshotShapeMismatch)
    );
    assert!(!frame.active);
    assert!(internal.state.header.integrity_fault);
}
