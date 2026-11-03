//! Private host-boundary probes use an actual Ledger, not a shadow numeric implementation.
use super::*;
use crate::ledger::{Ledger, LedgerLimits};
use crate::retained::RetainedHandleSharePlan;
use crate::{
    OperationControl, OperationCoordinator, PhysicalStorageAccountFactory,
    PhysicalStorageAccountLimits,
};
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
    let (mut exclusive, storage) = owner.exclusive().unwrap();
    let layout = exclusive.bind([key]).unwrap();
    let frame = layout
        .admit_observed(&mut exclusive.work, [4], WorkLane::Producer)
        .unwrap();
    // The actual joint policy method installs the tighter limits before returning its current
    // pending-policy violation. This probes the same completion category as the legacy live alias.
    assert!(
        exclusive
            .tighten_policy(&storage, key, DimensionPolicy::counter(8, 2, 0))
            .is_err()
    );
    let physical = Cell::new(0);
    let failure = Cell::new(None);
    let execution_state = Cell::new(StoreExecutionState::Running);
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame,
        admitted: [4],
        failure: &failure,
        execution_state: &execution_state,
        boundary: None,
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
    let (mut exclusive, _storage) = owner.exclusive().unwrap();
    let layout = exclusive.bind([key]).unwrap();
    let frame = layout
        .admit_observed(&mut exclusive.work, [1], WorkLane::Producer)
        .unwrap();
    let failure = Cell::new(None);
    let execution_state = Cell::new(StoreExecutionState::Running);
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame,
        admitted: [1],
        failure: &failure,
        execution_state: &execution_state,
        boundary: None,
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
    let (mut exclusive, _storage) = owner.exclusive().unwrap();
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
    let execution_state = Cell::new(StoreExecutionState::Running);
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame,
        admitted: [1, 1],
        failure: &failure,
        execution_state: &execution_state,
        boundary: None,
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

#[test]
fn real_import_boundary_defers_only_unclassified_unwind_and_caught_return_is_sticky() {
    let (mut owner, key) = actual_ledger();
    let (mut exclusive, _storage) = owner.exclusive().unwrap();
    let layout = exclusive.bind([key]).unwrap();
    let failure = Cell::new(None);
    let execution_state = Cell::new(StoreExecutionState::Running);
    let boundary = StoreImporterBoundary::default();
    assert!(boundary.begin());
    let physical = Cell::new(0);
    let first = layout
        .admit_observed(&mut exclusive.work, [1], WorkLane::Producer)
        .unwrap();
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame: first,
        admitted: [1],
        failure: &failure,
        execution_state: &execution_state,
        boundary: Some(&boundary),
    };
    physical.set(1);
    host.complete([1]).unwrap();
    host.finish().unwrap();
    let frame = layout
        .admit_observed(&mut exclusive.work, [1], WorkLane::Producer)
        .unwrap();
    let unwind = catch_unwind(AssertUnwindSafe(|| {
        let _host = ActionHost {
            work: &mut exclusive.work,
            layout,
            frame,
            admitted: [1],
            failure: &failure,
            execution_state: &execution_state,
            boundary: Some(&boundary),
        };
        physical.set(2);
        panic!("actual numeric leaf");
    }));
    assert!(unwind.is_err());
    assert_eq!(physical.get(), 2);
    assert_eq!(exclusive.counters(key).unwrap(), (2, 1, 0));
    exclusive.check_quiescent().unwrap();
    assert_eq!(execution_state.get(), StoreExecutionState::UnwindPending);
    assert_eq!(
        failure.get(),
        None,
        "a real importer catch has not classified this unwind"
    );
    StoreExecutionState::settle_caught_unwind(&execution_state, &failure);
    assert_eq!(
        failure.get(),
        Some(PreparationFailureCode::product_graph(ABANDONED))
    );
    record(&failure, PreparationFailureCode::domain(41));
    assert_eq!(
        failure.get(),
        Some(PreparationFailureCode::product_graph(ABANDONED))
    );
}

#[test]
fn cleanup_fault_precedes_later_import_disposition_and_preserves_the_actual_prefix() {
    let (mut owner, key) = actual_ledger();
    let (mut exclusive, _storage) = owner.exclusive().unwrap();
    let layout = exclusive.bind([key]).unwrap();
    let failure = Cell::new(Some(PreparationFailureCode::ledger()));
    let state = Cell::new(StoreExecutionState::Running);
    let boundary = StoreImporterBoundary::default();
    assert!(boundary.begin());
    let frame = layout
        .admit_observed(&mut exclusive.work, [2], WorkLane::Producer)
        .unwrap();
    let mut host = ActionHost {
        work: &mut exclusive.work,
        layout,
        frame,
        admitted: [2],
        failure: &failure,
        execution_state: &state,
        boundary: Some(&boundary),
    };
    host.complete([1]).unwrap();
    drop(host);
    assert_eq!(exclusive.counters(key).unwrap(), (2, 1, 0));
    exclusive.check_quiescent().unwrap();
    boundary.finish_panicked();
    assert_eq!(failure.get(), Some(PreparationFailureCode::ledger()));
}

#[test]
fn failure_materializer_observes_active_frame_before_resolution() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    struct Progress;
    impl ProgressPhase for Progress {
        fn order(self) -> u32 {
            0
        }
        fn stable_id(self) -> &'static str {
            "materialize"
        }
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    struct Unit;
    impl ProgressUnit for Unit {
        fn stable_id(self) -> &'static str {
            "pieces"
        }
    }
    #[derive(Debug)]
    enum Failure {
        Runtime(StoreInvocationError),
        Owned(Rc<u8>),
    }
    impl From<StoreInvocationError> for Failure {
        fn from(cause: StoreInvocationError) -> Self {
            Self::Runtime(cause)
        }
    }

    let catalog = StoreSemanticCatalogId::new(*b"frame-order-test", NonZeroU32::MIN);
    let domain = StoreKeyDomain::try_new(catalog).unwrap();
    let phase = domain
        .try_admit_phase(crate::StorePhaseSemanticId::new(catalog, NonZeroU64::MIN))
        .unwrap();
    let [parent_key, child_key, internal_key] = [1, 2, 3].map(|local| {
        domain
            .try_admit_dimension(StoreDimensionSemanticId::new(
                catalog,
                NonZeroU64::new(local).unwrap(),
            ))
            .unwrap()
    });
    let limits = LedgerLimits::new(
        u64::MAX,
        [parent_key, child_key, internal_key].map(|key| (key, DimensionPolicy::counter(8, 8, 0))),
    )
    .unwrap();
    let mut coordinator = OperationCoordinator::<Owner, _, Progress, Unit, _>::try_new(
        OperationControl::default(),
        limits,
        &[phase],
    )
    .unwrap();
    let factory =
        PhysicalStorageAccountFactory::try_new_for_live_accounts(NonZeroU64::MIN).unwrap();
    let account = factory
        .try_create_account(PhysicalStorageAccountLimits::new(1 << 20, 8).unwrap())
        .unwrap();
    let failure = Cell::new(None);
    let debt = Cell::new(0);
    let execution_state = Cell::new(StoreExecutionState::Running);
    let share_work = RetainedHandleSharePlan::new(internal_key, internal_key);
    let (poll, ledger) = coordinator.split_exclusive_poll_and_ledger();
    let (work, storage) = ledger.exclusive().unwrap();
    let share_layout = work.bind(share_work.charges().map(|(key, _)| key)).unwrap();
    let mut context = StoreInvocation {
        parts: ExclusiveOperationParts {
            work,
            ordinary: crate::retained::ExecutionOrdinaryStorage {
                storage,
                poll,
                failure: &failure,
                debt: &debt,
                execution_state: &execution_state,
            },
            account: &account,
            share_work,
        },
        domain: &domain,
        share_layout,
        boundary: None,
        brand: PhantomData,
        local: PhantomData,
    };
    context.checkpoint().unwrap();
    let parent_layout = context.bind_group([parent_key]).unwrap();
    let child_layout = context.bind_group([child_key]).unwrap();
    let semantic_owner = Rc::new(41);
    let materialized = Cell::new(0);
    let mut physical = [0; 3];
    let result = context.parser_parent::<_, Failure, 1>(parent_layout, [1], [None], |parent| {
        assert!(parent.host.frame.active);
        assert_eq!(parent.host.frame.remaining, [1]);
        let result = parent.child::<_, Failure, 1>(child_layout, [2], [None], |child| {
            child.run_prefix([1], 7, || {
                physical[0] = 17;
                Ok::<_, Failure>(())
            })?;
            let result = child.run_prefix_with_failure(
                [1],
                11,
                || -> Result<(), Rc<u8>> {
                    physical[1] = 29;
                    Err(Rc::clone(&semantic_owner))
                },
                |host, cause| {
                    assert!(
                        host.host.frame.active,
                        "materialization must precede frame resolution"
                    );
                    assert_eq!(host.host.frame.remaining, [1]);
                    // Parser suffixes are frame metadata; they do not reserve pending_consumed.
                    assert_eq!(host.work_counters(child_key).unwrap(), (2, 1, 0));
                    assert_eq!(host.work_counters(parent_key).unwrap(), (1, 0, 0));
                    assert_eq!(host.work_counters(internal_key).unwrap(), (0, 0, 0));
                    materialized.set(materialized.get() + 1);
                    Failure::Owned(cause)
                },
            );
            assert!(
                !child.host.frame.active,
                "the failed frame resolves before returning"
            );
            assert_eq!(child.host.frame.remaining, [1]);
            result?;
            physical[2] = 1;
            Ok(())
        });
        assert!(parent.host.frame.active);
        assert_eq!(parent.host.frame.remaining, [1]);
        assert_eq!(parent.work_counters(parent_key).unwrap(), (1, 0, 0));
        assert_eq!(parent.work_counters(child_key).unwrap(), (2, 1, 0));
        result
    });
    context.parts.work.check_quiescent().unwrap();
    assert_eq!(context.work_counters(parent_key).unwrap(), (1, 0, 0));
    assert_eq!(context.work_counters(child_key).unwrap(), (2, 1, 0));
    assert_eq!(context.work_counters(internal_key).unwrap(), (0, 0, 0));
    match result {
        Err(Failure::Owned(identity)) => {
            assert!(Rc::ptr_eq(&identity, &semantic_owner));
            assert_eq!(*identity, 41);
        }
        Err(Failure::Runtime(cause)) => panic!("runtime cleanup replaced the owned error: {cause}"),
        Ok(()) => panic!("the failed physical piece cannot complete successfully"),
    }
    assert_eq!(physical, [17, 29, 0]);
    assert_eq!(materialized.get(), 1);
    assert_eq!(execution_state.get(), StoreExecutionState::KnownError);
    assert_eq!(failure.get(), None);
    assert_eq!(account.snapshot().live_retained_bytes(), 0);
    assert_eq!(account.snapshot().live_retained_allocations(), 0);
    assert!(!account.snapshot().poisoned());
}

#[test]
fn parser_completion_failure_after_real_write_resolves_suffix_without_abandonment() {
    let (mut owner, key) = actual_ledger();
    let (mut exclusive, storage) = owner.exclusive().unwrap();
    let layout = exclusive.bind([key]).unwrap();
    let frame = layout
        .begin_parser(&mut exclusive.work, [2], [None], ParserChannel::Child)
        .unwrap();
    // Tightening uses the actual owner before borrowing its action guard. No outside alias.
    exclusive
        .tighten_policy(&storage, key, DimensionPolicy::counter(8, 0, 0))
        .unwrap();
    let failure = Cell::new(None);
    let execution_state = Cell::new(StoreExecutionState::Running);
    let mut frame = frame;
    let mut host = ParserActionHost {
        work: &mut exclusive.work,
        frame: &mut frame,
        layout,
        failure: &failure,
        execution_state: &execution_state,
        boundary: None,
        armed: true,
    };
    let mut physical = [0_u8; 2];
    physical[0] = 37;
    assert!(matches!(
        host.complete([1]),
        Err(LedgerCause::LimitExceeded {
            class: crate::CounterClass::Consumed,
            value: 1,
            limit: 0,
            ..
        })
    ));
    assert!(!host.frame.active);
    assert_eq!(host.frame.remaining, [2]);
    drop(host);
    assert_eq!(execution_state.get(), StoreExecutionState::Running);
    assert_eq!(physical, [37, 0]);
    assert_eq!(exclusive.counters(key).unwrap(), (2, 0, 0));
    exclusive.check_quiescent().unwrap();
    assert_eq!(failure.get(), Some(PreparationFailureCode::ledger()));
}
