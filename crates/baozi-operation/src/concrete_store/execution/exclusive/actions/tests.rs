use super::*;
use crate::ledger::LedgerLimits;
use crate::{
    OperationControl, OperationCoordinator, PhysicalStorageAccountFactory,
    PhysicalStorageAccountLimits,
};
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct CountPolls(Arc<AtomicUsize>);
impl crate::CancellationSignal for CountPolls {
    fn is_cancelled(&self) -> bool {
        self.0.fetch_add(1, Ordering::Relaxed);
        false
    }
}

struct Owner;
impl OperationOwner for Owner {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Progress;
impl ProgressPhase for Progress {
    fn order(self) -> u32 {
        0
    }
    fn stable_id(self) -> &'static str {
        "actions"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Unit;
impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        "leaves"
    }
}

fn with_actual_host<R>(
    action: impl FnOnce(
        &mut StoreActionExecution<'_, Owner, Progress, Unit>,
        StoreDimensionKey,
        &Arc<AtomicUsize>,
    ) -> R,
) -> (R, Option<PreparationFailureCode>, (u64, u64, u64)) {
    let catalog = StoreSemanticCatalogId::new(*b"geometry-actions", NonZeroU32::MIN);
    let domain = StoreKeyDomain::try_new(catalog).unwrap();
    let phase = domain
        .try_admit_phase(crate::StorePhaseSemanticId::new(catalog, NonZeroU64::MIN))
        .unwrap();
    let key = domain
        .try_admit_dimension(StoreDimensionSemanticId::new(catalog, NonZeroU64::MIN))
        .unwrap();
    let polls = Arc::new(AtomicUsize::new(0));
    let mut coordinator = OperationCoordinator::<Owner, _, Progress, Unit, _>::try_new(
        OperationControl::default().with_cancellation(CountPolls(Arc::clone(&polls))),
        LedgerLimits::new(u64::MAX, [(key, DimensionPolicy::counter(32, 32, 0))]).unwrap(),
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
    let share_work = crate::retained::RetainedHandleSharePlan::new(key, key);
    let (poll, progress, ledger) = coordinator.split_exclusive_progress_and_ledger();
    let (work, storage) = ledger.view().into_exclusive().unwrap();
    let share_layout = work.bind(share_work.charges().map(|(key, _)| key)).unwrap();
    let layout = work.bind([key]).unwrap();
    let context = StoreInvocation {
        parts: ExclusiveOperationParts {
            work,
            progress,
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
    let mut host = StoreActionExecution {
        context,
        layout,
        pending: None,
        action_pending: None,
    };
    host.checkpoint().unwrap();
    let result = action(&mut host, key, &polls);
    drop(host);
    let (work, _storage) = coordinator
        .split_exclusive_progress_and_ledger()
        .2
        .view()
        .into_exclusive()
        .unwrap();
    let counters = work.counters(key).unwrap();
    work.check_quiescent().unwrap();
    (result, failure.get(), counters)
}

#[test]
fn a_physically_completed_unmatched_action_still_requires_ownership_resolution() {
    for physically_complete in [false, true] {
        let ((polls, before_close), failure, counters) = with_actual_host(|host, _key, polls| {
            host.begin_action(1).unwrap();
            if physically_complete {
                // Owner-level fault injection: clients cannot extract or mutate this frame.
                let frame = host.pending.as_mut().unwrap();
                frame
                    .complete_batch_with_outcome(
                        &host.layout,
                        &mut host.context.parts.work.work,
                        [1],
                    )
                    .into_result()
                    .unwrap();
            }
            let before_close = polls.load(Ordering::Relaxed);
            assert!(
                before_close != 0,
                "the fixture must instrument the actual poll owner"
            );
            assert_eq!(
                host.finish(true),
                Err(StoreRuntimeError::ObservedWorkIncomplete)
            );
            assert_eq!(polls.load(Ordering::Relaxed), before_close);
            (Arc::clone(polls), before_close)
        });
        assert_eq!(
            polls.load(Ordering::Relaxed),
            before_close,
            "unmatched-action Drop must not poll, including at zero remaining"
        );
        assert_eq!(counters, (1, u64::from(physically_complete), 0));
        assert_eq!(
            failure,
            Some(PreparationFailureCode::product_graph(
                ORDINARY_WORK_PERMIT_ABANDONED_CODE
            ))
        );
    }
}

#[test]
fn failed_completion_resolves_the_real_frame_before_returning_the_literal_prefix() {
    let (_, failure, counters) = with_actual_host(|host, key, _polls| {
        host.begin_action(118).unwrap();
        assert_eq!(host.action_pending, Some(118));
        let expected = StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
            dimension: key,
            class: crate::CounterClass::Consumed,
            value: 1,
            limit: 0,
        });
        assert_eq!(
            host.tighten_dimension_policy(key, DimensionPolicy::counter(32, 0, 0)),
            Err(expected.clone())
        );
        assert_eq!(host.complete_action(), Err(expected));
        assert!(host.pending.is_none());
        assert!(host.action_pending.is_none());
        host.context.parts.work.check_quiescent().unwrap();
    });
    assert_eq!(counters, (1, 1, 0));
    assert_eq!(failure, Some(PreparationFailureCode::ledger()));
}

#[test]
fn one_real_envelope_is_replenished_and_explicitly_closed_after_completed_leaves() {
    let (_, failure, counters) = with_actual_host(|host, _key, _polls| {
        for cost in [118, 35, 1] {
            host.begin_action(cost).unwrap();
            assert_eq!(host.action_pending, Some(cost));
            assert_eq!(host.pending.as_ref().unwrap().remaining, [1]);
            host.complete_action().unwrap();
            assert!(host.action_pending.is_none());
            assert_eq!(host.pending.as_ref().unwrap().remaining, [0]);
        }
        host.finish(true).unwrap();
        assert!(host.pending.is_none());
    });
    assert_eq!(counters, (3, 3, 0));
    assert!(failure.is_none());
}

#[test]
fn a_denied_refill_retains_the_idle_frame_without_capturing_another_action() {
    for class in [
        crate::CounterClass::Attempted,
        crate::CounterClass::Consumed,
    ] {
        let ((polls, before_close), failure, counters) = with_actual_host(|host, key, polls| {
            host.begin_action(118).unwrap();
            host.complete_action().unwrap();
            let attempted = if class == crate::CounterClass::Attempted {
                1
            } else {
                32
            };
            host.tighten_dimension_policy(key, DimensionPolicy::counter(attempted, 1, 0))
                .unwrap();
            let expected = StoreRuntimeError::Ledger(LedgerCause::LimitExceeded {
                dimension: key,
                class,
                value: 2,
                limit: 1,
            });
            assert_eq!(host.begin_action(35), Err(expected));
            assert!(host.action_pending.is_none());
            assert_eq!(host.pending.as_ref().unwrap().remaining, [0]);
            assert_eq!(host.work_counters(key).unwrap(), (2, 1, 0));
            let before_close = polls.load(Ordering::Relaxed);
            host.finish(false).unwrap();
            assert!(host.pending.is_none());
            host.context.parts.work.check_quiescent().unwrap();
            assert_eq!(polls.load(Ordering::Relaxed), before_close);
            (Arc::clone(polls), before_close)
        });
        assert_eq!(polls.load(Ordering::Relaxed), before_close);
        assert_eq!(counters, (2, 1, 0));
        assert_eq!(failure, Some(PreparationFailureCode::ledger()));
    }
}
