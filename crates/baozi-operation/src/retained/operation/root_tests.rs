use super::*;
use crate::ledger::OutputCommitment;
use crate::retained::{
    PhysicalStorageAccountFactory, PhysicalStorageAccountFactoryLimits,
    PhysicalStorageAccountLimits, RetainedBlockTablePlan, RetainedClosureValidationError,
    RetainedFailureValidation,
};
use std::sync::{Arc, Mutex};

struct Owner;
impl OperationOwner for Owner {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Phase;
impl ProgressPhase for Phase {
    fn order(self) -> u32 {
        0
    }

    fn stable_id(self) -> &'static str {
        "root-test"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Unit;
impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        "items"
    }
}

struct DropProbe {
    label: &'static str,
    drops: Arc<Mutex<Vec<&'static str>>>,
    attribution: Option<AttributionAtDrop>,
}

struct AttributionAtDrop {
    ledger: crate::ledger::LedgerTestProbe<u8, u8>,
    observations: Arc<Mutex<Vec<(u64, u64)>>>,
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        if let Some(attribution) = &self.attribution {
            attribution.observations.lock().unwrap().push((
                attribution.ledger.current_live_bytes(),
                attribution.ledger.active_reservations(),
            ));
        }
        self.drops.lock().unwrap().push(self.label);
    }
}

type Root = InvocationRoot<Owner, u8, Phase, Unit, u8, DropProbe, DropProbe, DropProbe>;
type Permission<'inv> =
    RetainedOperation<'inv, Owner, u8, Phase, Unit, u8, DropProbe, DropProbe, DropProbe>;

fn account() -> PhysicalStorageAccount {
    PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap()
    .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, 128).unwrap())
    .unwrap()
}

fn writer() -> RetainedWriterPlan<u8, u8> {
    RetainedWriterPlan::new(1, 1, 1, RetainedBatchLimits::new(2, 2))
}

fn limits_and_plan() -> (LedgerLimits<u8>, RetainedOperationStartPlan<u8, u8>) {
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(1, policy)]).unwrap();
    let plan = RetainedOperationStartPlan::new(
        writer(),
        RetainedHandleSharePlan::new(1, 1),
        RetainedAllocationPlan::new(0, 0).unwrap(),
        OutputCommitment::new(1, 1),
        OutputCommitment::new(1, 1),
    );
    (limits, plan)
}

fn start<'inv>(
    root: &'inv mut Root,
    account: &PhysicalStorageAccount,
    drops: &Arc<Mutex<Vec<&'static str>>>,
) -> Permission<'inv> {
    let (limits, plan) = limits_and_plan();
    Permission::try_start(
        root,
        OperationControl::default(),
        limits,
        account,
        plan,
        || {
            Ok(DropProbe {
                label: "fallback",
                drops: Arc::clone(drops),
                attribution: None,
            })
        },
    )
    .unwrap_or_else(|_| panic!("real retained startup must succeed"))
}

fn assert_released(account: &PhysicalStorageAccount) {
    let snapshot = account.snapshot();
    assert!(!snapshot.poisoned());
    assert_eq!(snapshot.live_retained_bytes(), 0);
    assert_eq!(snapshot.live_retained_allocations(), 0);
}

#[test]
#[expect(
    clippy::forget_non_drop,
    reason = "the adversarial caller forgets an intentionally inert permission"
)]
fn forgotten_operation_permission_keeps_real_baseline_until_root_drop() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let permission = start(&mut root, &account, &drops);
    let live = account.snapshot().live_retained_allocations();
    assert!(live > 0);
    std::mem::forget(permission);

    assert!(root.common.is_some());
    let retained = root.retained.as_ref().unwrap();
    assert!(retained.baseline.account().is_same_account(&account));
    assert!(!retained.ordinary_work_permit_outstanding);
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(account.snapshot().live_retained_allocations(), live);

    drop(root);
    assert_eq!(*drops.lock().unwrap(), ["fallback"]);
    assert_released(&account);
}

#[test]
fn forgotten_assembly_permission_keeps_tables_and_cleans_values_before_attribution() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let permission = start(&mut root, &account, &drops);
    let table = RetainedBlockTablePlan::new(writer(), 1, 1);
    let mut assembly = permission
        .try_publication_assembly(RetainedPublicationPlan::success(table, table))
        .unwrap_or_else(|_| panic!("real publication tables must open"));
    let attribution_at_drop = Arc::new(Mutex::new(Vec::new()));
    let canonical_ledger = assembly.test_ledger_probe();
    let _canonical = assembly
        .try_retain_canonical(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
            Ok(emplacer.place(
                DropProbe {
                    label: "canonical",
                    drops: Arc::clone(&drops),
                    attribution: Some(AttributionAtDrop {
                        ledger: canonical_ledger,
                        observations: Arc::clone(&attribution_at_drop),
                    }),
                },
                (),
            ))
        })
        .unwrap_or_else(|_| panic!("real canonical payload must retain"));
    let evidence_ledger = assembly.test_ledger_probe();
    let _evidence = assembly
        .try_retain_outcome_evidence(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
            Ok(emplacer.place(
                DropProbe {
                    label: "evidence",
                    drops: Arc::clone(&drops),
                    attribution: Some(AttributionAtDrop {
                        ledger: evidence_ledger,
                        observations: Arc::clone(&attribution_at_drop),
                    }),
                },
                (),
            ))
        })
        .unwrap_or_else(|_| panic!("real evidence payload must retain"));
    let ledger = assembly.test_ledger_probe();
    let expected_attribution = (ledger.current_live_bytes(), ledger.active_reservations());
    assert!(expected_attribution.0 > 0);
    assert!(expected_attribution.1 > 0);
    let live = account.snapshot().live_retained_allocations();
    std::mem::forget(assembly);

    let fields = root.assembly.as_ref().unwrap();
    assert!(fields.canonical_table.is_some());
    assert!(fields.evidence_table.is_some());
    assert!(fields.canonical_batch.is_some());
    assert!(fields.evidence_batch.is_some());
    assert!(root.common.is_some());
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(account.snapshot().live_retained_allocations(), live);

    drop(root);
    assert_eq!(
        *drops.lock().unwrap(),
        ["canonical", "evidence", "fallback"]
    );
    assert_eq!(
        *attribution_at_drop.lock().unwrap(),
        [expected_attribution, expected_attribution],
        "both payload destructors must run before either attribution batch releases"
    );
    assert_released(&account);
}

#[test]
fn containment_detaches_physical_baseline_and_keeps_stopped_common_in_root() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let permission = start(&mut root, &account, &drops);
    let live = account.snapshot().live_retained_bytes();
    let cause = permission.contain_activation_fault();

    assert_eq!(
        cause.reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    let common = root.common.as_ref().unwrap();
    assert_eq!(
        common.execution_state.get(),
        StoreExecutionState::KnownError
    );
    assert!(
        common
            .coordinator
            .retained_ledger()
            .ordinary_work_is_sealed()
    );
    assert!(
        root.retained
            .as_ref()
            .unwrap()
            .baseline
            .first_account()
            .is_none()
    );
    assert!(account.snapshot().poisoned());
    assert_eq!(account.snapshot().live_retained_bytes(), live);

    drop(root);
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(account.snapshot().live_retained_bytes(), live);
}

struct ControlDropProbe(Arc<Mutex<Vec<&'static str>>>);

impl crate::CancellationSignal for ControlDropProbe {
    fn is_cancelled(&self) -> bool {
        false
    }
}

impl Drop for ControlDropProbe {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("control");
    }
}

#[test]
fn rejected_startup_builder_keeps_actual_control_in_root_until_cleanup() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let (limits, plan) = limits_and_plan();
    let control =
        OperationControl::default().with_cancellation(ControlDropProbe(Arc::clone(&drops)));
    let rejected = Permission::try_start(&mut root, control, limits, &account, plan, || {
        Err(RetainedBuildFailureCode::new(1))
    });
    assert!(rejected.is_err());
    drop(rejected);

    assert!(
        root.common.is_some(),
        "the builder runs with common installed in the outer root"
    );
    assert!(
        root.retained.is_none(),
        "a rejected builder installs no baseline"
    );
    assert!(drops.lock().unwrap().is_empty());
    assert_released(&account);

    drop(root);
    assert_eq!(*drops.lock().unwrap(), ["control"]);
    assert_released(&account);
}

#[test]
fn physical_startup_denial_keeps_common_control_and_the_actual_share_prefix_in_root() {
    let account = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap()
    .try_create_account(PhysicalStorageAccountLimits::new(1, 128).unwrap())
    .unwrap();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let control =
        OperationControl::default().with_cancellation(ControlDropProbe(Arc::clone(&drops)));
    let mut root = Root::new();
    let (limits, plan) = limits_and_plan();
    let mut built = false;
    let rejected = Permission::try_start(&mut root, control, limits, &account, plan, || {
        built = true;
        Ok(DropProbe {
            label: "unexpected-fallback",
            drops: Arc::clone(&drops),
            attribution: None,
        })
    });
    assert!(matches!(
        rejected,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Physical(RetainedAllocationError::ByteLimitExceeded {
                limit: 1,
                ..
            })
        ))
    ));
    drop(rejected);

    assert!(!built);
    assert!(root.retained.is_none());
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(account.share_call_count(), 4);
    assert_released(&account);
    let ledger = root.common.as_ref().unwrap().coordinator.retained_ledger();
    let evidence = ledger.dimension_evidence(1).unwrap();
    assert_eq!((evidence.attempted(), evidence.consumed()), (8, 8));
    assert!(evidence.peak_live_bytes() > 0);
    let probe = ledger.test_probe();
    assert_eq!(probe.current_live_bytes(), 0);
    assert_eq!(probe.active_reservations(), 0);
    drop(root);
    assert_eq!(*drops.lock().unwrap(), ["control"]);
    assert!(!probe.is_available());
    assert_released(&account);
}

#[test]
fn attachment_uses_current_prework_limits_and_latches_rejection_in_the_same_root() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let control =
        OperationControl::default().with_cancellation(ControlDropProbe(Arc::clone(&drops)));
    let mut root = Root::new();
    let (_, plan) = limits_and_plan();
    let policy = DimensionPolicy::reservable_bytes(10, 10, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(u64::MAX, [(1, policy)]).unwrap();
    let mut binding =
        Permission::bind_with_phase_catalog(&mut root, control, (limits, &[1]), &account, plan)
            .unwrap_or_else(|_| panic!("binding owns policy before current-state attachment"));
    let identity = binding.root.invocation_identity();
    {
        let mut parts = binding
            .exclusive_parts()
            .unwrap_or_else(|_| panic!("bound prework"));
        let layout = parts.work.bind([1]).unwrap();
        parts.work.charge_observed(layout, [3]).unwrap();
    }
    let shares = account.share_call_count();
    let mut built = false;
    let result = binding.attach(|| {
        built = true;
        Ok(DropProbe {
            label: "unexpected-fallback",
            drops: Arc::clone(&drops),
            attribution: None,
        })
    });
    assert!(matches!(
        result,
        Err(RetainedOperationStartError::Writer(
            RetainedWriterError::Ledger(LedgerCause::LimitExceeded {
                value: 11,
                limit: 10,
                ..
            })
        ))
    ));
    drop(result);
    assert!(!built);
    assert_eq!(root.invocation_identity(), identity);
    assert!(root.retained.is_none());
    let common = root.common.as_ref().unwrap();
    assert!(matches!(
        common.execution_state.get(),
        StoreExecutionState::KnownError
    ));
    assert_eq!(
        common.sticky_terminal_cause.get(),
        Some(PreparationFailureCode::ledger())
    );
    let evidence = common
        .coordinator
        .retained_ledger()
        .dimension_evidence(1)
        .unwrap();
    assert_eq!((evidence.attempted(), evidence.consumed()), (3, 3));
    assert!(
        common
            .coordinator
            .retained_ledger()
            .ordinary_work_is_sealed()
    );
    assert_eq!(account.share_call_count(), shares);
    assert!(drops.lock().unwrap().is_empty());
    let actions = super::super::account::lock_recovering_poison(&account.inner.value().linear)
        .admitted_account_actions;
    assert_eq!(actions, 1);
    assert_released(&account);
    drop(root);
    assert_eq!(*drops.lock().unwrap(), ["control"]);
}

#[test]
fn attachment_preserves_actual_prework_scratch_and_one_start_admission() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let (limits, plan) = limits_and_plan();
    let mut root = Root::new();
    let mut binding = Permission::bind_with_phase_catalog(
        &mut root,
        OperationControl::default(),
        (limits, &[1]),
        &account,
        plan,
    )
    .unwrap_or_else(|_| panic!("bind actual invocation"));
    let identity = binding.root.invocation_identity();
    let scratch = {
        let mut parts = binding
            .exclusive_parts()
            .unwrap_or_else(|_| panic!("bound prework"));
        let layout = parts.work.bind([1]).unwrap();
        parts.work.charge_observed(layout, [3]).unwrap();
        parts.ordinary.storage.reserve(1, 1, 7).unwrap()
    };
    let before = binding
        .root
        .common
        .as_ref()
        .unwrap()
        .coordinator
        .retained_ledger()
        .dimension_evidence(1)
        .unwrap();
    assert_eq!(
        (
            before.attempted(),
            before.consumed(),
            before.current_live_bytes()
        ),
        (3, 3, 7)
    );
    let permission = binding
        .attach(|| {
            Ok(DropProbe {
                label: "fallback",
                drops: Arc::clone(&drops),
                attribution: None,
            })
        })
        .unwrap_or_else(|_| panic!("attachment permits real unrelated scratch"));
    assert_eq!(permission.invocation_identity(), identity);
    let ledger = permission.common.coordinator.retained_ledger();
    let evidence = ledger.dimension_evidence(1).unwrap();
    assert_eq!((evidence.attempted(), evidence.consumed()), (11, 11));
    assert!(evidence.current_live_bytes() > 7);
    assert!(evidence.peak_live_bytes() >= before.peak_live_bytes());
    ledger.validate_live_reservation(&scratch).unwrap();
    let actions = super::super::account::lock_recovering_poison(&account.inner.value().linear)
        .admitted_account_actions;
    assert_eq!(actions, 1);
    assert_eq!(account.share_call_count(), 4);
    drop(scratch);
    drop(permission);
    drop(root);
    assert_eq!(*drops.lock().unwrap(), ["fallback"]);
    assert_released(&account);
}

#[test]
fn fallback_rejection_and_panic_after_prework_keep_the_actual_root_and_completed_startup() {
    for panic in [false, true] {
        let account = account();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let control =
            OperationControl::default().with_cancellation(ControlDropProbe(Arc::clone(&drops)));
        let (limits, plan) = limits_and_plan();
        let mut root = Root::new();
        let mut binding =
            Permission::bind_with_phase_catalog(&mut root, control, (limits, &[1]), &account, plan)
                .unwrap_or_else(|_| panic!("bind before real prework"));
        let identity = binding.root.invocation_identity();
        let scratch = {
            let mut parts = binding
                .exclusive_parts()
                .unwrap_or_else(|_| panic!("the bound entry admits real prework"));
            let layout = parts.work.bind([1]).unwrap();
            parts.work.charge_observed(layout, [3]).unwrap();
            parts.ordinary.storage.reserve(1, 1, 7).unwrap()
        };
        let mut calls = 0;
        let rejected = binding.attach(|| {
            calls += 1;
            if panic {
                panic!("fallback builder after real prework");
            }
            Err(RetainedBuildFailureCode::new(41))
        });
        if panic {
            assert!(matches!(
                rejected,
                Err(RetainedOperationStartError::Writer(
                    RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
                ))
            ));
        } else {
            assert!(matches!(
                rejected,
                Err(RetainedOperationStartError::Writer(RetainedWriterError::Physical(
                    RetainedAllocationError::Build(cause)
                ))) if cause == RetainedBuildFailureCode::new(41)
            ));
        }
        drop(rejected);
        assert_eq!(calls, 1);
        assert_eq!(root.invocation_identity(), identity);
        assert!(root.retained.is_none());
        let common = root.common.as_ref().unwrap();
        assert_eq!(
            common.execution_state.get(),
            StoreExecutionState::KnownError
        );
        assert!(common.sticky_terminal_cause.get().is_some());
        let ledger = common.coordinator.retained_ledger();
        let evidence = ledger.dimension_evidence(1).unwrap();
        assert_eq!((evidence.attempted(), evidence.consumed()), (11, 11));
        assert!(ledger.ordinary_work_is_sealed());
        ledger.validate_live_reservation(&scratch).unwrap();
        assert_eq!(account.share_call_count(), 4);
        let actions = super::super::account::lock_recovering_poison(&account.inner.value().linear)
            .admitted_account_actions;
        assert_eq!(actions, 1);
        assert!(drops.lock().unwrap().is_empty());
        drop(scratch);
        drop(root);
        assert_eq!(*drops.lock().unwrap(), ["control"]);
        if panic {
            let snapshot = account.snapshot();
            assert!(snapshot.poisoned());
            assert!(snapshot.live_retained_allocations() > 0);
            assert_eq!(
                snapshot.first_poison_evidence().unwrap().reason(),
                AccountPoisonReason::RetainedBuilderPanicked
            );
        } else {
            assert_released(&account);
        }
    }
}

#[test]
fn unattached_terminal_uses_the_original_ticket_and_actual_prework_without_startup_shares() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let control =
        OperationControl::default().with_cancellation(ControlDropProbe(Arc::clone(&drops)));
    let (limits, plan) = limits_and_plan();
    let mut root = Root::new();
    let mut binding =
        Permission::bind_with_phase_catalog(&mut root, control, (limits, &[1]), &account, plan)
            .unwrap_or_else(|_| panic!("the original invocation binds once"));
    {
        let mut parts = binding
            .exclusive_parts()
            .unwrap_or_else(|_| panic!("actual prework"));
        let work = parts.work.bind([1]).unwrap();
        parts.work.charge_observed(work, [3]).unwrap();
        let scratch = parts.ordinary.storage.reserve(1, 1, 7).unwrap();
        drop(scratch);
    }
    let binding = binding
        .defer_unattached(None)
        .unwrap_or_else(|_| panic!("the request retains the original admission"));
    assert!(drops.lock().unwrap().is_empty());
    assert!(root.retained.is_none());
    assert!(
        root.common
            .as_ref()
            .unwrap()
            .coordinator
            .retained_ledger()
            .ordinary_work_is_sealed()
    );
    let actions = super::super::account::lock_recovering_poison(&account.inner.value().linear)
        .admitted_account_actions;
    assert_eq!(actions, 1);
    assert_eq!(account.share_call_count(), 0);
    let evidence = root
        .prepare_unattached_terminal(binding, FinalProgress::new(Phase, Unit, 1))
        .unwrap_or_else(|_| panic!("only the original root can finish"));
    assert_eq!(evidence.cause, None);
    let work = evidence.snapshot.dimension(1).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (3, 3));
    assert_eq!(work.peak_live_bytes(), 7);
    assert_eq!(evidence.snapshot.current_live_bytes(), 0);
    assert_eq!(evidence.snapshot.active_reservations(), 0);
    assert_eq!(account.share_call_count(), 0);
    assert!(root.common.is_none());
    assert_eq!(*drops.lock().unwrap(), ["control"]);
    drop(evidence);
    drop(root);
    assert_released(&account);
}

#[test]
fn unattached_failure_keeps_dirty_snapshot_and_records_the_actual_live_residue() {
    let account = account();
    let (limits, plan) = limits_and_plan();
    let mut root = Root::new();
    let mut binding = Permission::bind_with_phase_catalog(
        &mut root,
        OperationControl::default(),
        (limits, &[1]),
        &account,
        plan,
    )
    .unwrap_or_else(|_| panic!("bind before original prework"));
    let scratch = {
        let mut parts = binding
            .exclusive_parts()
            .unwrap_or_else(|_| panic!("actual scratch"));
        let work = parts.work.bind([1]).unwrap();
        parts.work.charge_observed(work, [2]).unwrap();
        parts.ordinary.storage.reserve(1, 1, 7).unwrap()
    };
    let requested = PreparationFailureCode::domain(41);
    let binding = binding
        .defer_unattached(Some(requested))
        .unwrap_or_else(|_| panic!("failure requests end the same numeric loan"));
    let evidence = root
        .prepare_unattached_terminal(binding, FinalProgress::new(Phase, Unit, 1))
        .unwrap_or_else(|_| panic!("the original root still owns failure facts"));
    assert_eq!(evidence.cause, Some(PreparationFailureCode::ledger()));
    assert_eq!(evidence.snapshot.current_live_bytes(), 7);
    assert_eq!(evidence.snapshot.active_reservations(), 1);
    let work = evidence.snapshot.dimension(1).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (2, 2));
    assert_eq!(work.current_live_bytes(), 7);
    drop(scratch);
    assert_eq!(evidence.snapshot.current_live_bytes(), 7);
    assert_eq!(account.share_call_count(), 0);
    drop(evidence);
    drop(root);
    assert_released(&account);
}

struct PoisonUnattachedAccountOnDrop(Arc<PhysicalStorageAccount>);

impl crate::CancellationSignal for PoisonUnattachedAccountOnDrop {
    fn is_cancelled(&self) -> bool {
        false
    }
}

impl Drop for PoisonUnattachedAccountOnDrop {
    fn drop(&mut self) {
        self.0
            .record_poison(AccountPoisonReason::OwnershipGraphUnverified);
    }
}

#[test]
fn unattached_terminal_rechecks_original_account_after_capture_and_control_disposal() {
    for control_disposal in [false, true] {
        let account = Arc::new(account());
        let poison = PoisonUnattachedAccountOnDrop(Arc::clone(&account));
        let (control, capture) = if control_disposal {
            (OperationControl::default().with_cancellation(poison), None)
        } else {
            (OperationControl::default(), Some(poison))
        };
        let (limits, plan) = limits_and_plan();
        let mut root = Root::new();
        let binding =
            Permission::bind_with_phase_catalog(&mut root, control, (limits, &[1]), &account, plan)
                .unwrap_or_else(|_| panic!("original account is healthy at binding"));
        let pending = binding
            .defer_unattached(None)
            .unwrap_or_else(|_| panic!("the original ticket reaches the outer finish"));
        drop(capture);
        let evidence = root
            .prepare_unattached_terminal(pending, FinalProgress::new(Phase, Unit, 1))
            .unwrap_or_else(|_| panic!("the same root records the account rejection"));
        assert_eq!(
            evidence.cause,
            Some(PreparationFailureCode::product_graph(0x5541_4343))
        );
        assert_eq!(evidence.snapshot.current_live_bytes(), 0);
        assert_eq!(evidence.snapshot.active_reservations(), 0);
        assert!(account.snapshot().poisoned());
        assert_eq!(account.share_call_count(), 0);
        let actions = super::super::account::lock_recovering_poison(&account.inner.value().linear)
            .admitted_account_actions;
        assert_eq!(actions, 1);
        drop(evidence);
        drop(root);
        assert_eq!(account.snapshot().live_retained_bytes(), 0);
        assert_eq!(account.snapshot().live_retained_allocations(), 0);
    }
}

#[test]
fn invocation_permission_is_neither_send_nor_sync() {
    trait AmbiguousIfSend<Marker> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    impl<T: Send> AmbiguousIfSend<u8> for T {}

    trait AmbiguousIfSync<Marker> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfSync<()> for T {}
    impl<T: Sync> AmbiguousIfSync<u8> for T {}

    let _ = <Permission<'static> as AmbiguousIfSend<_>>::check;
    let _ = <Permission<'static> as AmbiguousIfSync<_>>::check;

    fn independent_storage_is_send_and_sync<T: Send + Sync>() {}
    independent_storage_is_send_and_sync::<PhysicalStorageAccount>();
}

#[test]
fn deferred_operation_restores_the_same_ledger_after_missing_common_is_repaired() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let mut permission = start(&mut root, &account, &drops);
    let identity = permission.invocation_identity();
    let first_cause = PreparationFailureCode::product_graph(0x524f_4f54);
    permission.record_terminal_cause(first_cause);
    let live = account.snapshot().live_retained_allocations();
    assert!(permission.defer_until_body_cleanup().is_ok());

    assert!(root.resume_deferred_assembly().is_none());
    assert!(root.resume_deferred_success().is_none());
    let common = root.common.take();
    assert!(root.resume_deferred_operation().is_none());
    root.common = common;
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(account.snapshot().live_retained_allocations(), live);

    let permission = root
        .resume_deferred_operation()
        .unwrap_or_else(|| panic!("the actual operation stage must remain pending"));
    assert_eq!(permission.invocation_identity(), identity);
    assert_eq!(permission.sticky_terminal_cause(), Some(first_cause));
    assert!(permission.is_terminal_only());
    assert!(permission.account().is_same_account(&account));
    let Err(permission) = permission.defer_until_body_cleanup() else {
        panic!("an operation restored for finalization cannot admit another body disposition")
    };
    let table = RetainedBlockTablePlan::new(writer(), 1, 1);
    let denied = match permission
        .try_publication_assembly(RetainedPublicationPlan::success(table, table))
    {
        Ok(_) => panic!("terminal operation authority cannot reopen success-capable activation"),
        Err(denied) => denied,
    };
    assert!(matches!(
        denied.cause(),
        RetainedWriterError::InvalidPublicationPlan
    ));
    drop(denied);

    drop(root);
    assert_eq!(*drops.lock().unwrap(), ["fallback"]);
    assert_released(&account);
}

#[test]
fn deferred_assembly_keeps_payloads_and_survives_missing_attribution_on_restore() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let permission = start(&mut root, &account, &drops);
    let table = RetainedBlockTablePlan::new(writer(), 1, 1);
    let mut assembly = permission
        .try_publication_assembly(RetainedPublicationPlan::success(table, table))
        .unwrap_or_else(|_| panic!("real publication tables must open"));
    let observations = Arc::new(Mutex::new(Vec::new()));
    let canonical_ledger = assembly.test_ledger_probe();
    let _canonical = assembly
        .try_retain_canonical(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
            Ok(emplacer.place(
                DropProbe {
                    label: "canonical",
                    drops: Arc::clone(&drops),
                    attribution: Some(AttributionAtDrop {
                        ledger: canonical_ledger,
                        observations: Arc::clone(&observations),
                    }),
                },
                (),
            ))
        })
        .unwrap_or_else(|_| panic!("real canonical payload must retain"));
    let evidence_ledger = assembly.test_ledger_probe();
    let _evidence = assembly
        .try_retain_outcome_evidence(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
            Ok(emplacer.place(
                DropProbe {
                    label: "evidence",
                    drops: Arc::clone(&drops),
                    attribution: Some(AttributionAtDrop {
                        ledger: evidence_ledger,
                        observations: Arc::clone(&observations),
                    }),
                },
                (),
            ))
        })
        .unwrap_or_else(|_| panic!("real evidence payload must retain"));
    let ledger = assembly.test_ledger_probe();
    let expected_attribution = (ledger.current_live_bytes(), ledger.active_reservations());
    let live = account.snapshot().live_retained_allocations();
    assert!(assembly.defer_until_body_cleanup().is_ok());

    assert!(root.resume_deferred_operation().is_none());
    assert!(root.resume_deferred_failure().is_none());
    assert!(root.resume_deferred_success().is_none());
    let evidence_batch = root.assembly.as_mut().unwrap().evidence_batch.take();
    assert!(root.resume_deferred_assembly().is_none());
    root.assembly.as_mut().unwrap().evidence_batch = evidence_batch;
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(account.snapshot().live_retained_allocations(), live);
    assert_eq!(
        (ledger.current_live_bytes(), ledger.active_reservations()),
        expected_attribution
    );

    let assembly = root
        .resume_deferred_assembly()
        .unwrap_or_else(|| panic!("the actual assembly stage must remain pending"));
    assert_eq!(assembly.canonical_len(), 1);
    assert_eq!(assembly.outcome_evidence_len(), 1);
    assert!(assembly.test_storage_graph_is_exact());
    assert!(assembly.defer_until_body_cleanup().is_ok());

    drop(root);
    assert_eq!(
        *drops.lock().unwrap(),
        ["canonical", "evidence", "fallback"]
    );
    assert_eq!(
        *observations.lock().unwrap(),
        [expected_attribution, expected_attribution]
    );
    assert_released(&account);
}

#[test]
fn deferred_failure_and_fallback_preserve_protected_work_and_original_causes() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let failure_work = RetainedFailureTerminalWorkPlan::with_evidence_work(2, 1, 1, 0);
    let failure_limit = failure_work.requirement().unwrap().observed_work_limit();
    let policy = DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        .with_published_outcome_evidence_limit(64);
    let limits = LedgerLimits::new(
        u64::MAX,
        [
            (1, policy),
            (2, DimensionPolicy::counter(failure_limit, failure_limit, 0)),
        ],
    )
    .unwrap();
    let (_, plan) = limits_and_plan();
    let permission = Permission::try_start(
        &mut root,
        OperationControl::default(),
        limits,
        &account,
        plan.with_failure_terminal_work(failure_work),
        || {
            Ok(DropProbe {
                label: "fallback",
                drops: Arc::clone(&drops),
                attribution: None,
            })
        },
    )
    .unwrap_or_else(|_| panic!("the protected failure-work operation must start"));
    let table = RetainedBlockTablePlan::new(writer(), 1, 1);
    let first_cause = PreparationFailureCode::product_graph(0x5246_4149);
    let fallback_cause = PreparationFailureCode::requested();
    let failure = permission
        .try_publication_assembly(RetainedPublicationPlan::rich_failure(table))
        .unwrap_or_else(|_| panic!("the protected evidence table must open"))
        .into_verified_failure_only(first_cause)
        .unwrap_or_else(|_| panic!("the evidence-only graph must remain exact"))
        .with_fallback_cause(fallback_cause);
    let work_before = failure.assembly.test_dimension_evidence(2).unwrap();
    assert!(failure.defer_until_body_cleanup().is_ok());
    assert!(root.resume_deferred_assembly().is_none());
    assert!(root.resume_deferred_fallback().is_none());

    let mut failure = root
        .resume_deferred_failure()
        .unwrap_or_else(|| panic!("the actual failure stage must remain pending"));
    assert_eq!(failure.fallback_cause, fallback_cause);
    assert_eq!(
        failure.assembly.test_dimension_evidence(2),
        Some(work_before)
    );
    assert_eq!(
        failure
            .assembly
            .operation
            .as_ref()
            .unwrap()
            .sticky_terminal_cause(),
        Some(first_cause)
    );
    let evidence = failure
        .try_retain_outcome_evidence_controlled(
            RetainedAllocationPlan::new(0, 0).unwrap(),
            |emplacer, control| {
                let mut work = control
                    .try_begin(NonZeroU64::new(1).unwrap())
                    .unwrap_or_else(|_| panic!("protected work must survive deferral"));
                work.complete_units(NonZeroU64::new(1).unwrap())
                    .unwrap_or_else(|_| panic!("the admitted protected unit must complete"));
                let _control = work
                    .complete()
                    .unwrap_or_else(|_| panic!("work must resolve"));
                Ok(emplacer.place(
                    DropProbe {
                        label: "evidence",
                        drops: Arc::clone(&drops),
                        attribution: None,
                    },
                    (),
                ))
            },
        )
        .unwrap_or_else(|_| panic!("protected evidence construction must remain available"));
    let fallback = match failure.validate_rich_failure(evidence, |_, _| {
        panic!("injected raw failure validator panic")
    }) {
        RetainedFailureValidation::FallbackOnly(fallback) => fallback,
        _ => panic!("a validator panic must preserve the fallback owner"),
    };
    let panic_cause =
        PreparationFailureCode::from_closure_validation(RetainedClosureValidationError::Panicked);
    assert_eq!(fallback.cause, panic_cause);
    assert!(!fallback.activation_containment_required());
    assert!(fallback.defer_until_body_cleanup().is_ok());
    assert!(root.resume_deferred_failure().is_none());
    assert!(root.resume_deferred_success().is_none());

    let fallback = root
        .resume_deferred_fallback()
        .unwrap_or_else(|| panic!("the actual fallback stage must remain pending"));
    assert_eq!(fallback.cause, panic_cause);
    assert!(!fallback.activation_containment_required());
    assert_eq!(
        fallback
            .assembly
            .operation
            .as_ref()
            .unwrap()
            .sticky_terminal_cause(),
        Some(first_cause)
    );
    drop(fallback);
    assert_eq!(*drops.lock().unwrap(), ["evidence"]);
    drop(root);
    assert_eq!(*drops.lock().unwrap(), ["evidence", "fallback"]);
    assert_released(&account);
}
