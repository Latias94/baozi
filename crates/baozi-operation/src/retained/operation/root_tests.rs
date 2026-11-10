use super::*;
use crate::ledger::OutputCommitment;
use crate::retained::{
    CanonicalRetainedSlot, OutcomeEvidenceRetainedSlot, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits, RetainedBlockTablePlan,
    RetainedClosureValidationError, RetainedFailureValidation, RetainedSuccessFreezeDecision,
    RetainedSuccessJointCommitFormation, RetainedSuccessValidation, TerminalDisposition,
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
fn dynamic_startup_disposes_an_uninvoked_capture_before_charge_refund() {
    struct Capture {
        account: Arc<PhysicalStorageAccount>,
        observations: Arc<Mutex<Vec<(u64, u64)>>>,
        panics: bool,
    }
    impl Drop for Capture {
        fn drop(&mut self) {
            let physical = self.account.snapshot();
            self.observations.lock().unwrap().push((
                physical.live_retained_bytes(),
                physical.live_retained_allocations(),
            ));
            assert!(!self.panics, "injected dynamic startup capture panic");
        }
    }
    for panics in [false, true] {
        let account = Arc::new(account());
        let observations = Arc::new(Mutex::new(Vec::new()));
        let captured = Capture {
            account: Arc::clone(&account),
            observations: Arc::clone(&observations),
            panics,
        };
        let mut root = Root::new();
        let (limits, plan) = limits_and_plan();
        super::super::fail_next_final_owner_allocation();
        let result = Permission::try_start(
            &mut root,
            OperationControl::default(),
            limits,
            &account,
            plan,
            move || {
                drop(captured);
                panic!("a denied constructor cannot invoke its builder")
            },
        );
        assert!(result.is_err());
        drop(result);
        let observed = observations.lock().unwrap();
        assert_eq!(observed.len(), 1);
        assert!(observed[0].0 > 0);
        // Fallback owns one allocation; the dynamic evidence owns its header and two buffers.
        assert_eq!(observed[0].1, 4);
        assert_eq!(account.snapshot().poisoned(), panics);
        if panics {
            assert_eq!(account.snapshot().live_retained_bytes(), observed[0].0);
            assert_eq!(account.snapshot().live_retained_allocations(), 4);
        } else {
            assert_released(&account);
        }
        drop(root);
    }
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
    assert!(!retained.metadata.ordinary_work_permit_outstanding);
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(account.snapshot().live_retained_allocations(), live);

    drop(root);
    assert_eq!(*drops.lock().unwrap(), ["fallback"]);
    assert_released(&account);
}

#[test]
fn forgotten_terminal_classification_keeps_the_actual_armed_state_and_physical_graph() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let mut permission = start(&mut root, &account, &drops);
    permission
        .loan
        .common
        .view()
        .coordinator
        .report_progress(Phase, Unit, 1, None)
        .unwrap();
    let identity = permission.loan().invocation_identity();
    let table = RetainedBlockTablePlan::new(writer(), 1, 1);
    let assembly = permission
        .try_publication_assembly(RetainedPublicationPlan::success(table, table))
        .unwrap_or_else(|_| panic!("real terminal tables must open"));
    let ledger = assembly.loan().test_ledger_probe();
    let numeric_before = (ledger.current_live_bytes(), ledger.active_reservations());
    let administrative_before = ledger.current_administrative_bytes();
    let physical_before = account.snapshot();
    let classification = assembly
        .into_loan()
        .try_classify_terminal_preparation(super::super::RetainedTerminalRequest::MinimalFallback {
            requested_cause: PreparationFailureCode::domain(41),
        })
        .unwrap_or_else(|_| panic!("the actual fallback must classify before freeze"));
    std::mem::forget(classification);

    assert_eq!(root.invocation_identity(), identity);
    assert!(root.common.as_ref().unwrap().lifecycle.terminal_outstanding);
    assert!(root.terminal.as_ref().unwrap().graph.fallback.is_some());
    assert_eq!(
        root.common
            .as_ref()
            .unwrap()
            .coordinator
            .known_administrative_bytes(),
        Some(administrative_before)
    );
    root.cleanup_owned_state();
    assert!(root.common.is_none());
    assert!(root.must_retain_domain());
    assert!(ledger.is_available());
    assert_eq!(
        (ledger.current_live_bytes(), ledger.active_reservations()),
        numeric_before
    );
    assert_eq!(ledger.current_administrative_bytes(), administrative_before);
    let contained = account.snapshot();
    assert!(contained.poisoned());
    assert_eq!(
        contained.live_retained_bytes(),
        physical_before.live_retained_bytes()
    );
    assert_eq!(
        contained.live_retained_allocations(),
        physical_before.live_retained_allocations()
    );
    assert!(drops.lock().unwrap().is_empty());
    root.cleanup_owned_state();
    drop(root);
    assert_eq!(account.snapshot(), contained);
}

#[test]
fn forgotten_pre_freeze_classifications_keep_real_payload_batches_until_root_cleanup() {
    for (rich_failure, explicit_cleanup) in
        [(false, false), (false, true), (true, false), (true, true)]
    {
        let account = account();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let mut root = Root::new();
        let mut permission = start(&mut root, &account, &drops);
        permission
            .loan
            .common
            .view()
            .coordinator
            .report_progress(Phase, Unit, 1, None)
            .unwrap();
        let identity = permission.loan().invocation_identity();
        let table = RetainedBlockTablePlan::new(writer(), 1, 1);
        let mut assembly = permission
            .try_publication_assembly(RetainedPublicationPlan::success(table, table))
            .unwrap_or_else(|_| panic!("real terminal tables must open"));
        let _canonical = assembly
            .loan_mut()
            .try_retain_canonical(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
                Ok(emplacer.place(
                    DropProbe {
                        label: "canonical",
                        drops: Arc::clone(&drops),
                        attribution: None,
                    },
                    (),
                ))
            })
            .unwrap_or_else(|_| panic!("real canonical payload must retain"));
        let evidence = assembly
            .loan_mut()
            .try_retain_outcome_evidence(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
                Ok(emplacer.place(
                    DropProbe {
                        label: "evidence",
                        drops: Arc::clone(&drops),
                        attribution: None,
                    },
                    (),
                ))
            })
            .unwrap_or_else(|_| panic!("real outcome payload must retain"));
        assert_eq!(assembly.loan().canonical_len(), 1);
        assert_eq!(assembly.loan().outcome_evidence_len(), 1);
        assert!(assembly.loan().test_storage_graph_is_exact());
        let requested_cause = PreparationFailureCode::domain(41);
        let request = if rich_failure {
            let proof = assembly
                .loan_mut()
                .try_validate_rich_failure(evidence, |session, evidence| {
                    session
                        .outcome_evidence(evidence)
                        .is_some_and(|block| block.label == "evidence")
                })
                .unwrap_or_else(|_| panic!("the actual evidence closure must validate"));
            super::super::RetainedTerminalRequest::RichFailure {
                proof,
                requested_cause,
                final_progress: FinalProgress::new(Phase, Unit, 1),
            }
        } else {
            super::super::RetainedTerminalRequest::MinimalFallback { requested_cause }
        };
        let ledger = assembly.loan().test_ledger_probe();
        let numeric_before = (
            ledger.current_live_bytes(),
            ledger.active_reservations(),
            ledger.current_administrative_bytes(),
        );
        let physical_before = account.snapshot();
        let shares_before = account.share_call_count();
        let payload_owners_before = Arc::strong_count(&drops);
        assert!(numeric_before.0 > 0);
        assert!(numeric_before.1 > 0);
        assert!(numeric_before.2 > 0);
        assert!(!physical_before.poisoned());
        assert!(physical_before.live_retained_bytes() > 0);
        assert!(physical_before.live_retained_allocations() > 0);
        let classification = assembly
            .into_loan()
            .try_classify_terminal_preparation(request)
            .unwrap_or_else(|_| panic!("the real payload graph must classify before freeze"));
        if rich_failure {
            assert!(matches!(
                &classification.state,
                super::super::TerminalPreFreezeTransition::Rich { cause, .. }
                    if *cause == requested_cause
            ));
        } else {
            assert!(matches!(
                &classification.state,
                super::super::TerminalPreFreezeTransition::Fallback { cause, .. }
                    if *cause == requested_cause
            ));
        }
        assert!(!classification.state.test_evidence_is_frozen());
        std::mem::forget(classification);

        assert_eq!(root.invocation_identity(), identity);
        let common = root.common.as_ref().unwrap();
        assert!(common.lifecycle.terminal_outstanding);
        assert_eq!(
            common.coordinator.known_administrative_bytes(),
            Some(numeric_before.2)
        );
        assert!(
            root.retained
                .as_ref()
                .unwrap()
                .baseline
                .with_batches(|_| ())
                .is_some()
        );
        let fields = root.assembly.as_ref().unwrap();
        assert!(fields.canonical_batch.is_some());
        assert!(fields.evidence_batch.is_some());
        let physical = root.terminal.as_ref().unwrap();
        assert!(physical.graph.fallback.is_some());
        assert!(physical.graph.canonical_table.is_some());
        assert!(physical.graph.evidence_table.is_some());
        assert!(
            physical
                .graph
                .account
                .as_ref()
                .unwrap()
                .is_same_account(&account)
        );
        assert!(physical.evidence.is_some());
        assert!(ledger.is_available());
        assert!(!ledger.is_terminal_qualified());
        assert_eq!(
            (
                ledger.current_live_bytes(),
                ledger.active_reservations(),
                ledger.current_administrative_bytes(),
            ),
            numeric_before
        );
        assert_eq!(account.snapshot(), physical_before);
        assert_eq!(account.share_call_count(), shares_before);
        assert_eq!(Arc::strong_count(&drops), payload_owners_before);
        assert!(drops.lock().unwrap().is_empty());

        if explicit_cleanup {
            root.cleanup_owned_state();
            assert!(root.common.is_none());
            assert!(root.retained.is_none());
            assert!(root.assembly.is_none());
            assert!(root.terminal.is_none());
            assert!(root.must_retain_domain());
            let contained = account.snapshot();
            assert!(contained.poisoned());
            root.cleanup_owned_state();
            assert_eq!(account.snapshot(), contained);
        }
        drop(root);

        let contained = account.snapshot();
        assert!(contained.poisoned());
        assert_eq!(
            contained.live_retained_bytes(),
            physical_before.live_retained_bytes()
        );
        assert_eq!(
            contained.live_retained_allocations(),
            physical_before.live_retained_allocations()
        );
        assert!(ledger.is_available());
        assert_eq!(
            (
                ledger.current_live_bytes(),
                ledger.active_reservations(),
                ledger.current_administrative_bytes(),
            ),
            numeric_before
        );
        assert_eq!(account.share_call_count(), shares_before);
        assert_eq!(Arc::strong_count(&drops), payload_owners_before);
        assert!(drops.lock().unwrap().is_empty());
    }
}

#[test]
#[expect(
    clippy::forget_non_drop,
    reason = "the caller forgets actual borrowed permissions before root-owned batches are transferred"
)]
fn forgotten_early_terminal_loan_quarantines_root_owned_baseline_and_assembly_batches() {
    for with_assembly in [false, true] {
        let account = account();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let mut root = Root::new();
        let mut permission = start(&mut root, &account, &drops);
        let ledger = permission.loan.common.test_ledger_mut().test_probe();
        if with_assembly {
            let table = RetainedBlockTablePlan::new(writer(), 1, 1);
            let mut assembly = permission
                .try_publication_assembly(RetainedPublicationPlan::success(table, table))
                .unwrap_or_else(|_| panic!("real root-owned assembly must open"));
            let _canonical = assembly
                .loan_mut()
                .try_retain_canonical(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
                    Ok(emplacer.place(
                        DropProbe {
                            label: "canonical",
                            drops: Arc::clone(&drops),
                            attribution: None,
                        },
                        (),
                    ))
                })
                .unwrap_or_else(|_| panic!("real canonical payload must retain"));
            let _evidence = assembly
                .loan_mut()
                .try_retain_outcome_evidence(
                    RetainedAllocationPlan::new(0, 0).unwrap(),
                    |emplacer| {
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
                .unwrap_or_else(|_| panic!("real outcome payload must retain"));
            std::mem::forget(assembly);
        } else {
            std::mem::forget(permission);
        }
        let numeric_before = (
            ledger.current_live_bytes(),
            ledger.active_reservations(),
            ledger.current_administrative_bytes(),
        );
        let physical_before = account.snapshot();
        assert!(numeric_before.1 > 0);
        assert!(physical_before.live_retained_allocations() > 0);
        assert!(
            root.retained
                .as_ref()
                .unwrap()
                .baseline
                .with_batches(|_| ())
                .is_some()
        );
        std::mem::forget(root.terminal_loan_for_test());
        assert!(root.common.as_ref().unwrap().lifecycle.terminal_outstanding);

        root.cleanup_owned_state();

        assert!(root.common.is_none());
        assert!(root.retained.is_none());
        assert!(root.assembly.is_none());
        assert!(root.has_started());
        assert!(root.must_retain_domain());
        assert!(ledger.is_available());
        assert_eq!(
            (
                ledger.current_live_bytes(),
                ledger.active_reservations(),
                ledger.current_administrative_bytes(),
            ),
            numeric_before
        );
        let contained = account.snapshot();
        assert!(contained.poisoned());
        assert_eq!(
            contained.live_retained_bytes(),
            physical_before.live_retained_bytes()
        );
        assert_eq!(
            contained.live_retained_allocations(),
            physical_before.live_retained_allocations()
        );
        assert!(drops.lock().unwrap().is_empty());
        root.cleanup_owned_state();
        assert_eq!(
            (
                ledger.current_live_bytes(),
                ledger.active_reservations(),
                ledger.current_administrative_bytes(),
            ),
            numeric_before
        );
        drop(root);
        assert_eq!(account.snapshot(), contained);
        assert_eq!(
            (
                ledger.current_live_bytes(),
                ledger.active_reservations(),
                ledger.current_administrative_bytes(),
            ),
            numeric_before
        );
        assert!(drops.lock().unwrap().is_empty());
    }
}

#[test]
#[expect(
    clippy::forget_non_drop,
    reason = "the test transfers original physical owners after forgetting their short permission"
)]
fn terminal_physical_graph_quarantines_real_payloads_without_numerical_evidence() {
    for with_tables in [false, true] {
        let account = account();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let mut root = Root::new();
        let mut permission = start(&mut root, &account, &drops);
        let ledger = permission.loan.common.test_ledger_mut().test_probe();
        if with_tables {
            let table = RetainedBlockTablePlan::new(writer(), 1, 1);
            let mut assembly = permission
                .try_publication_assembly(RetainedPublicationPlan::success(table, table))
                .unwrap_or_else(|_| panic!("real terminal tables must open"));
            let _canonical = assembly
                .loan_mut()
                .try_retain_canonical(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
                    Ok(emplacer.place(
                        DropProbe {
                            label: "canonical",
                            drops: Arc::clone(&drops),
                            attribution: None,
                        },
                        (),
                    ))
                })
                .unwrap_or_else(|_| panic!("real canonical payload must retain"));
            let _evidence = assembly
                .loan_mut()
                .try_retain_outcome_evidence(
                    RetainedAllocationPlan::new(0, 0).unwrap(),
                    |emplacer| {
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
                .unwrap_or_else(|_| panic!("real outcome payload must retain"));
            std::mem::forget(assembly);
        } else {
            std::mem::forget(permission);
        }
        let (fallback, numerical_evidence, batches) =
            root.retained.as_mut().unwrap().baseline.take_parts();
        let (canonical_table, evidence_table, graph_account) = root
            .assembly
            .as_mut()
            .map(|fields| {
                (
                    fields.canonical_table.take(),
                    fields.evidence_table.take(),
                    fields.account.take(),
                )
            })
            .unwrap_or((None, None, None));
        let graph = super::super::terminal_physical::TerminalPhysicalGraph {
            fallback: Some(fallback),
            canonical_table,
            evidence_table,
            account: graph_account,
        };
        let physical_before = account.snapshot();
        let numeric_before = (
            ledger.current_live_bytes(),
            ledger.active_reservations(),
            ledger.current_administrative_bytes(),
        );
        let shares_before = account.share_call_count();

        drop(graph);

        let contained = account.snapshot();
        assert!(contained.poisoned());
        assert_eq!(
            contained.live_retained_bytes(),
            physical_before.live_retained_bytes()
        );
        assert_eq!(
            contained.live_retained_allocations(),
            physical_before.live_retained_allocations()
        );
        assert_eq!(account.share_call_count(), shares_before);
        assert_eq!(
            (
                ledger.current_live_bytes(),
                ledger.active_reservations(),
                ledger.current_administrative_bytes(),
            ),
            numeric_before
        );
        assert!(drops.lock().unwrap().is_empty());

        // The fixture's dynamic numerical part and batch cover are separate real owners.
        // Retain them after the physical quarantine; a finite graph has no numerical part.
        std::mem::forget(numerical_evidence);
        std::mem::forget(batches);
        std::mem::forget(root.terminal_loan_for_test());
        root.cleanup_owned_state();
        drop(root);
        assert!(ledger.is_available());
        assert_eq!(
            (
                ledger.current_live_bytes(),
                ledger.active_reservations(),
                ledger.current_administrative_bytes(),
            ),
            numeric_before
        );
        assert_eq!(account.snapshot(), contained);
        assert!(drops.lock().unwrap().is_empty());
    }
}

#[test]
fn consuming_pre_detach_containment_retains_real_common_after_physical_quarantine() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let permission = start(&mut root, &account, &drops);
    let table = RetainedBlockTablePlan::new(writer(), 1, 1);
    let assembly = permission
        .try_publication_assembly(RetainedPublicationPlan::success(table, table))
        .unwrap_or_else(|_| panic!("real terminal tables must open"));
    let ledger = assembly.loan().test_ledger_probe();
    let numeric_before = (
        ledger.current_live_bytes(),
        ledger.active_reservations(),
        ledger.current_administrative_bytes(),
    );
    let physical_before = account.snapshot();
    let shares_before = account.share_call_count();
    let classification = assembly
        .into_loan()
        .try_classify_terminal_preparation(super::super::RetainedTerminalRequest::MinimalFallback {
            requested_cause: PreparationFailureCode::domain(41),
        })
        .unwrap_or_else(|_| panic!("the actual fallback must classify before freeze"));
    let _ = account.record_poison(AccountPoisonReason::RetainedStorageReleaseUnverified);
    let first = account.snapshot().first_poison_evidence();
    let cause = classification.contain_activation_fault();
    assert_eq!(
        cause.reason(),
        AccountPoisonReason::RetainedStorageReleaseUnverified
    );
    assert!(root.common.is_some());
    assert!(root.common.as_ref().unwrap().lifecycle.terminal_outstanding);
    assert!(
        root.retained
            .as_ref()
            .unwrap()
            .baseline
            .with_batches(|_| ())
            .is_some()
    );
    let fields = root.assembly.as_ref().unwrap();
    assert!(fields.canonical_batch.is_some());
    assert!(fields.evidence_batch.is_some());
    assert!(root.must_retain_domain());
    assert!(ledger.is_available());
    assert_eq!(
        (
            ledger.current_live_bytes(),
            ledger.active_reservations(),
            ledger.current_administrative_bytes(),
        ),
        numeric_before
    );
    let contained = account.snapshot();
    assert_eq!(contained.first_poison_evidence(), first);
    assert_eq!(
        contained.live_retained_bytes(),
        physical_before.live_retained_bytes()
    );
    assert_eq!(
        contained.live_retained_allocations(),
        physical_before.live_retained_allocations()
    );
    assert_eq!(account.share_call_count(), shares_before);
    assert!(drops.lock().unwrap().is_empty());
    for _ in 0..2 {
        root.cleanup_owned_state();
        assert!(root.common.is_none());
        assert!(root.retained.is_none());
        assert!(root.assembly.is_none());
        assert!(root.terminal.is_none());
        assert!(root.must_retain_domain());
        assert_eq!(account.snapshot(), contained);
        assert!(ledger.is_available());
        assert_eq!(
            (
                ledger.current_live_bytes(),
                ledger.active_reservations(),
                ledger.current_administrative_bytes(),
            ),
            numeric_before
        );
        assert_eq!(account.share_call_count(), shares_before);
        assert!(drops.lock().unwrap().is_empty());
    }
    drop(root);
    assert_eq!(account.snapshot(), contained);
    assert!(ledger.is_available());
    assert_eq!(
        (
            ledger.current_live_bytes(),
            ledger.active_reservations(),
            ledger.current_administrative_bytes(),
        ),
        numeric_before
    );
    assert_eq!(account.share_call_count(), shares_before);
    assert!(drops.lock().unwrap().is_empty());
}

#[test]
fn successful_detach_consumes_the_actual_terminal_slot_once() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let permission = start(&mut root, &account, &drops);
    let prepared = permission
        .prepare_evidence_only_success(FinalProgress::new(Phase, Unit, 1))
        .unwrap_or_else(|_| panic!("the real read-only terminal must detach"));
    assert!(matches!(
        prepared,
        PreparedRetainedEvidenceOnlyOutcome::Success(_)
    ));
    assert!(root.common.is_none() && root.terminal.is_none());
    root.cleanup_owned_state();
    assert!(root.common.is_none() && root.terminal.is_none());
    assert_eq!(*drops.lock().unwrap(), ["fallback"]);
    drop(prepared);
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
    let canonical_ledger = assembly.loan().test_ledger_probe();
    let _canonical = assembly
        .loan_mut()
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
    let evidence_ledger = assembly.loan().test_ledger_probe();
    let _evidence = assembly
        .loan_mut()
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
    let ledger = assembly.loan().test_ledger_probe();
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
        common.lifecycle.execution_state.get(),
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
    let ledger = root
        .common
        .as_mut()
        .unwrap()
        .coordinator
        .retained_ledger_mut();
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
        common.lifecycle.execution_state.get(),
        StoreExecutionState::KnownError
    ));
    assert_eq!(
        common.lifecycle.sticky_terminal_cause.get(),
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
    assert_eq!(permission.loan().invocation_identity(), identity);
    let ledger = permission.loan.common.test_ledger();
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
            common.lifecycle.execution_state.get(),
            StoreExecutionState::KnownError
        );
        assert!(common.lifecycle.sticky_terminal_cause.get().is_some());
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
    assert!(!evidence.contained);
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

struct UnattachedControlProbe(Arc<Mutex<Vec<&'static str>>>);

impl crate::CancellationSignal for UnattachedControlProbe {
    fn is_cancelled(&self) -> bool {
        self.0.lock().unwrap().push("poll");
        false
    }
}

impl Drop for UnattachedControlProbe {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("control");
    }
}

#[test]
fn unattached_phase_order_panic_keeps_the_actual_root_armed_without_replaying_extensions() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct PanickingOrderPhase;

    impl ProgressPhase for PanickingOrderPhase {
        fn order(self) -> u32 {
            panic!("injected unattached phase order panic");
        }

        fn stable_id(self) -> &'static str {
            "unattached-order-panic"
        }
    }

    type PanicRoot =
        InvocationRoot<Owner, u8, PanickingOrderPhase, Unit, u8, DropProbe, DropProbe, DropProbe>;
    type PanicPermission<'inv> = RetainedOperation<
        'inv,
        Owner,
        u8,
        PanickingOrderPhase,
        Unit,
        u8,
        DropProbe,
        DropProbe,
        DropProbe,
    >;

    for explicit_cleanup in [false, true] {
        let account = account();
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let control = OperationControl::default()
            .with_cancellation(UnattachedControlProbe(Arc::clone(&events)))
            .with_progress_observer(move |_: crate::ProgressEvent<PanickingOrderPhase, Unit>| {
                observed.lock().unwrap().push("observer");
                crate::ProgressDecision::Continue
            });
        let (limits, plan) = limits_and_plan();
        let mut root = PanicRoot::new();
        let mut binding = PanicPermission::bind_with_phase_catalog(
            &mut root,
            control,
            (limits, &[1]),
            &account,
            plan,
        )
        .unwrap_or_else(|_| panic!("bind the actual invocation before phase code runs"));
        let identity = binding.invocation_identity();
        let scratch = {
            let mut parts = binding
                .exclusive_parts()
                .unwrap_or_else(|_| panic!("actual work precedes terminal preparation"));
            let work = parts.work.bind([1]).unwrap();
            parts.work.charge_observed(work, [3]).unwrap();
            parts.ordinary.storage.reserve(1, 1, 7).unwrap()
        };
        let pending = binding
            .defer_unattached(None)
            .unwrap_or_else(|_| panic!("defer with the original admission"));
        let ledger = root.common.as_ref().unwrap().coordinator.retained_ledger();
        let work_before = ledger.dimension_evidence(1).unwrap();
        let probe = ledger.test_shared_probe();
        assert_eq!((work_before.attempted(), work_before.consumed()), (3, 3));
        assert_eq!(
            (probe.current_live_bytes(), probe.active_reservations()),
            (7, 1)
        );
        assert!(events.lock().unwrap().is_empty());
        assert!(!account.snapshot().poisoned());
        assert_eq!(
            super::super::account::lock_recovering_poison(&account.inner.value().linear)
                .admitted_account_actions,
            1
        );

        let panic = match catch_unwind(AssertUnwindSafe(|| {
            let _ = root.prepare_unattached_terminal(
                pending,
                FinalProgress::new(PanickingOrderPhase, Unit, 1),
            );
        })) {
            Err(payload) => payload,
            Ok(()) => panic!("the owner's phase order must unwind through production preparation"),
        };
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"injected unattached phase order panic")
        );
        assert_eq!(root.invocation_identity(), identity);
        assert!(root.retained.is_none());
        assert!(root.assembly.is_none());
        let common = root.common.as_mut().unwrap();
        assert!(common.lifecycle.terminal_outstanding);
        assert_eq!(
            common
                .coordinator
                .retained_ledger()
                .dimension_evidence(1)
                .unwrap(),
            work_before
        );
        let administrative_bytes = probe.current_administrative_bytes();
        assert!(administrative_bytes > 0);
        assert_eq!(
            common.coordinator.known_administrative_bytes(),
            Some(administrative_bytes)
        );
        let poisoned = account.snapshot();
        assert!(poisoned.poisoned());
        assert_eq!(poisoned.poison_event_count(), 1);
        assert!(poisoned.poison_event_count_exact());
        assert!(poisoned.poison_event_domain_valid());
        let first = poisoned.first_poison_evidence().unwrap();
        assert_eq!(
            first.reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
        assert_eq!(first.event_ordinal(), 1);
        assert_eq!(*events.lock().unwrap(), ["poll"]);

        assert!(
            common
                .view()
                .prepare_unattached_terminal(
                    &account,
                    FinalProgress::new(PanickingOrderPhase, Unit, 2),
                )
                .is_err()
        );
        assert_eq!(account.snapshot(), poisoned);
        assert_eq!(*events.lock().unwrap(), ["poll"]);
        if explicit_cleanup {
            root.cleanup_owned_state();
            assert!(root.common.is_none());
            assert!(root.must_retain_domain());
        }
        drop(root);
        assert!(probe.is_available());
        assert_eq!(
            (
                probe.current_live_bytes(),
                probe.active_reservations(),
                probe.current_administrative_bytes(),
            ),
            (7, 1, administrative_bytes)
        );
        assert_eq!(account.snapshot(), poisoned);
        assert_eq!(account.share_call_count(), 0);
        assert_eq!(
            super::super::account::lock_recovering_poison(&account.inner.value().linear)
                .admitted_account_actions,
            1
        );
        assert_eq!(*events.lock().unwrap(), ["poll"]);
        std::mem::forget(scratch);
    }
}

#[test]
fn unattached_foreign_stamp_returns_the_same_owner_and_preserves_its_original_seal() {
    let original_account = account();
    let foreign_account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut original_root = Root::new();
    let mut foreign_root = Root::new();
    let (limits, plan) = limits_and_plan();
    let mut original_binding = Permission::bind_with_phase_catalog(
        &mut original_root,
        OperationControl::default().with_cancellation(ControlDropProbe(Arc::clone(&drops))),
        (limits, &[1]),
        &original_account,
        plan,
    )
    .unwrap_or_else(|_| panic!("bind the original owner"));
    {
        let mut parts = original_binding
            .exclusive_parts()
            .unwrap_or_else(|_| panic!("original prework"));
        let work = parts.work.bind([1]).unwrap();
        parts.work.charge_observed(work, [3]).unwrap();
        drop(parts.ordinary.storage.reserve(1, 1, 7).unwrap());
    }
    let _original_pending = original_binding
        .defer_unattached(None)
        .unwrap_or_else(|_| panic!("retain the original ticket"));
    let (limits, plan) = limits_and_plan();
    let mut foreign_binding = Permission::bind_with_phase_catalog(
        &mut foreign_root,
        OperationControl::default().with_cancellation(ControlDropProbe(Arc::clone(&drops))),
        (limits, &[1]),
        &foreign_account,
        plan,
    )
    .unwrap_or_else(|_| panic!("bind a distinct actual owner"));
    let foreign_scratch = {
        let mut parts = foreign_binding
            .exclusive_parts()
            .unwrap_or_else(|_| panic!("foreign prework"));
        let work = parts.work.bind([1]).unwrap();
        parts.work.charge_observed(work, [5]).unwrap();
        parts.ordinary.storage.reserve(1, 1, 11).unwrap()
    };
    let _foreign_pending = foreign_binding
        .defer_unattached(None)
        .unwrap_or_else(|_| panic!("retain the distinct owner's ticket"));
    let original_ledger = original_root
        .common
        .as_ref()
        .unwrap()
        .coordinator
        .retained_ledger();
    let foreign_ledger = foreign_root
        .common
        .as_ref()
        .unwrap()
        .coordinator
        .retained_ledger();
    let original_identity = original_ledger.as_read().identity();
    let foreign_identity = foreign_ledger.as_read().identity();
    assert_ne!(original_identity, foreign_identity);
    let original_probe = original_ledger.test_shared_probe();
    let foreign_probe = foreign_ledger.test_shared_probe();
    let original_stamp = original_root
        .common
        .as_mut()
        .unwrap()
        .view()
        .prepare_unattached_terminal(&original_account, FinalProgress::new(Phase, Unit, 3))
        .unwrap_or_else(|()| panic!("prepare only through the shared production kernel"));
    let foreign_stamp = foreign_root
        .common
        .as_mut()
        .unwrap()
        .view()
        .prepare_unattached_terminal(&foreign_account, FinalProgress::new(Phase, Unit, 5))
        .unwrap_or_else(|()| panic!("the distinct actual owner prepares its own seal"));
    let original_physical = original_account.snapshot();
    let foreign_physical = foreign_account.snapshot();
    let original_work = original_root
        .common
        .as_ref()
        .unwrap()
        .coordinator
        .retained_ledger()
        .dimension_evidence(1)
        .unwrap();
    let original_administration = original_probe.current_administrative_bytes();
    let foreign_administration = foreign_probe.current_administrative_bytes();
    assert_eq!(*drops.lock().unwrap(), ["control", "control"]);

    let common = original_root.common.take().unwrap();
    let common = match common.finish_unattached(foreign_stamp, &original_account) {
        Err(common) => common,
        Ok(_) => panic!("a foreign stamp cannot consume the original owner's seal"),
    };
    assert_eq!(
        common.coordinator.retained_ledger().as_read().identity(),
        original_identity
    );
    assert_eq!(
        common
            .coordinator
            .retained_ledger()
            .dimension_evidence(1)
            .unwrap(),
        original_work
    );
    assert_eq!(
        (original_work.attempted(), original_work.consumed()),
        (3, 3)
    );
    assert_eq!(original_work.peak_live_bytes(), 7);
    assert_eq!(
        common.coordinator.reported_progress(Phase, Unit),
        Some((3, Some(3)))
    );
    assert!(common.lifecycle.terminal_outstanding);
    assert_eq!(
        common.coordinator.known_administrative_bytes(),
        Some(original_administration)
    );
    assert!(original_probe.is_available());
    assert_eq!(
        (
            original_probe.current_live_bytes(),
            original_probe.active_reservations()
        ),
        (0, 0)
    );
    let foreign_common = foreign_root.common.as_ref().unwrap();
    assert_eq!(
        foreign_common
            .coordinator
            .retained_ledger()
            .as_read()
            .identity(),
        foreign_identity
    );
    let foreign_work = foreign_common
        .coordinator
        .retained_ledger()
        .dimension_evidence(1)
        .unwrap();
    assert_eq!((foreign_work.attempted(), foreign_work.consumed()), (5, 5));
    assert_eq!(foreign_work.current_live_bytes(), 11);
    assert_eq!(
        foreign_common.coordinator.reported_progress(Phase, Unit),
        Some((5, Some(5)))
    );
    assert_eq!(original_account.snapshot(), original_physical);
    assert_eq!(foreign_account.snapshot(), foreign_physical);
    assert_eq!(*drops.lock().unwrap(), ["control", "control"]);

    let evidence = common
        .finish_unattached(original_stamp, &original_account)
        .unwrap_or_else(|_| {
            panic!("the original seal still consumes its actual owner exactly once")
        });
    assert_eq!(evidence.cause, None);
    assert!(!evidence.contained);
    assert_eq!(evidence.snapshot.dimension(1), Some(&original_work));
    assert_eq!(evidence.snapshot.current_live_bytes(), 0);
    assert_eq!(evidence.snapshot.active_reservations(), 0);
    assert!(!original_probe.is_available());
    assert!(foreign_common.lifecycle.terminal_outstanding);
    foreign_root.cleanup_owned_state();
    assert!(foreign_root.common.is_none());
    assert!(foreign_root.must_retain_domain());
    drop(evidence);
    drop(original_root);
    drop(foreign_root);
    assert!(foreign_probe.is_available());
    assert_eq!(
        (
            foreign_probe.current_live_bytes(),
            foreign_probe.active_reservations(),
            foreign_probe.current_administrative_bytes(),
        ),
        (11, 1, foreign_administration)
    );
    for (account, before) in [
        (&original_account, original_physical),
        (&foreign_account, foreign_physical),
    ] {
        assert_eq!(account.snapshot(), before);
        assert_eq!(account.share_call_count(), 0);
        assert_eq!(
            super::super::account::lock_recovering_poison(&account.inner.value().linear)
                .admitted_account_actions,
            1
        );
    }
    assert_eq!(*drops.lock().unwrap(), ["control", "control"]);
    std::mem::forget(foreign_scratch);
}

#[test]
fn unattached_repeated_preparation_preserves_the_first_seal_and_rejects_another_roots_ticket() {
    let account = account();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let mut other_root = Root::new();
    let observed = Arc::clone(&events);
    let control = OperationControl::default()
        .with_cancellation(UnattachedControlProbe(Arc::clone(&events)))
        .with_progress_observer(move |_: crate::ProgressEvent<Phase, Unit>| {
            observed.lock().unwrap().push("observer");
            crate::ProgressDecision::Continue
        });
    let (limits, plan) = limits_and_plan();
    let binding =
        Permission::bind_with_phase_catalog(&mut root, control, (limits, &[1]), &account, plan)
            .unwrap_or_else(|_| panic!("bind the original owner once"));
    let pending = binding
        .defer_unattached(None)
        .unwrap_or_else(|_| panic!("retain the actual ticket"));
    let (limits, plan) = limits_and_plan();
    let other_binding = Permission::bind_with_phase_catalog(
        &mut other_root,
        OperationControl::default(),
        (limits, &[1]),
        &account,
        plan,
    )
    .unwrap_or_else(|_| panic!("the other root has its own original admission"));
    let other_pending = other_binding
        .defer_unattached(None)
        .unwrap_or_else(|_| panic!("retain the other ticket"));
    let stamp = root
        .common
        .as_mut()
        .unwrap()
        .view()
        .prepare_unattached_terminal(&account, FinalProgress::new(Phase, Unit, 1))
        .unwrap_or_else(|()| panic!("prepare the first actual seal"));
    let events_before = events.lock().unwrap().clone();
    assert_eq!(
        events_before
            .iter()
            .filter(|event| **event == "observer")
            .count(),
        1
    );
    assert_eq!(
        events_before
            .iter()
            .filter(|event| **event == "control")
            .count(),
        1
    );
    assert!(events_before.contains(&"poll"));
    let physical_before = account.snapshot();
    let common = root.common.as_mut().unwrap();
    assert!(common.lifecycle.terminal_outstanding);
    assert!(
        common
            .view()
            .prepare_unattached_terminal(&account, FinalProgress::new(Phase, Unit, 9))
            .is_err()
    );
    assert_eq!(
        common.coordinator.reported_progress(Phase, Unit),
        Some((1, Some(1)))
    );
    let pending =
        match other_root.prepare_unattached_terminal(pending, FinalProgress::new(Phase, Unit, 9)) {
            Err(pending) => pending,
            Ok(_) => panic!("another root cannot replace the original admission"),
        };
    assert!(
        !other_root
            .common
            .as_ref()
            .unwrap()
            .lifecycle
            .terminal_outstanding
    );
    let pending =
        match root.prepare_unattached_terminal(pending, FinalProgress::new(Phase, Unit, 9)) {
            Err(pending) => pending,
            Ok(_) => panic!("the existing seal refuses a second terminal preparation"),
        };
    assert_eq!(*events.lock().unwrap(), events_before);
    assert_eq!(account.snapshot(), physical_before);
    assert_eq!(
        super::super::account::lock_recovering_poison(&account.inner.value().linear)
            .admitted_account_actions,
        2
    );

    let evidence = root
        .common
        .take()
        .unwrap()
        .finish_unattached(stamp, &account)
        .unwrap_or_else(|_| panic!("the first seal survives every rejected replacement"));
    assert_eq!(evidence.cause, None);
    assert!(!evidence.contained);
    assert_eq!(evidence.snapshot.current_live_bytes(), 0);
    assert_eq!(evidence.snapshot.active_reservations(), 0);
    assert!(root.common.is_none());
    let _pending =
        match root.prepare_unattached_terminal(pending, FinalProgress::new(Phase, Unit, 9)) {
            Err(pending) => pending,
            Ok(_) => panic!("a consumed actual owner cannot be captured again"),
        };
    assert_eq!(*events.lock().unwrap(), events_before);
    let other_evidence = other_root
        .prepare_unattached_terminal(other_pending, FinalProgress::new(Phase, Unit, 2))
        .unwrap_or_else(|_| panic!("the other root can still finish under its own ticket"));
    assert_eq!(other_evidence.cause, None);
    assert!(!other_evidence.contained);
    assert!(other_root.common.is_none());
    drop(evidence);
    drop(other_evidence);
    drop(root);
    drop(other_root);
    assert_eq!(*events.lock().unwrap(), events_before);
    assert_eq!(account.snapshot(), physical_before);
    assert_eq!(account.share_call_count(), 0);
    assert_eq!(
        super::super::account::lock_recovering_poison(&account.inner.value().linear)
            .admitted_account_actions,
        2
    );
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
    assert!(evidence.contained);
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
    let quarantined = account.snapshot();
    assert!(quarantined.poisoned());
    assert_eq!(quarantined.live_retained_bytes(), 0);
    assert_eq!(quarantined.live_retained_allocations(), 0);
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
    let identity = permission.loan().invocation_identity();
    let first_cause = PreparationFailureCode::product_graph(0x524f_4f54);
    permission.loan_mut().record_terminal_cause(first_cause);
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
    assert_eq!(permission.loan().invocation_identity(), identity);
    assert_eq!(permission.loan().sticky_terminal_cause(), Some(first_cause));
    assert!(permission.loan().is_terminal_only());
    assert!(permission.loan().account().is_same_account(&account));
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
    let canonical_ledger = assembly.loan().test_ledger_probe();
    let _canonical = assembly
        .loan_mut()
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
    let evidence_ledger = assembly.loan().test_ledger_probe();
    let _evidence = assembly
        .loan_mut()
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
    let ledger = assembly.loan().test_ledger_probe();
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
    assert_eq!(assembly.loan().canonical_len(), 1);
    assert_eq!(assembly.loan().outcome_evidence_len(), 1);
    assert!(assembly.loan().test_storage_graph_is_exact());
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
    let work_before = failure.assembly.loan().test_dimension_evidence(2).unwrap();
    assert!(failure.defer_until_body_cleanup().is_ok());
    assert!(root.resume_deferred_assembly().is_none());
    assert!(root.resume_deferred_fallback().is_none());

    let mut failure = root
        .resume_deferred_failure()
        .unwrap_or_else(|| panic!("the actual failure stage must remain pending"));
    assert_eq!(failure.fallback_cause, fallback_cause);
    assert_eq!(
        failure.assembly.loan().test_dimension_evidence(2),
        Some(work_before)
    );
    assert_eq!(
        failure
            .assembly
            .loan
            .operation
            .as_ref()
            .unwrap()
            .sticky_terminal_cause(),
        Some(first_cause)
    );
    let evidence = failure
        .borrow_ordinary()
        .loan_mut()
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
    assert!(!fallback.activation_containment_required);
    assert!(fallback.defer_until_body_cleanup().is_ok());
    assert!(root.resume_deferred_failure().is_none());
    assert!(root.resume_deferred_success().is_none());

    let fallback = root
        .resume_deferred_fallback()
        .unwrap_or_else(|| panic!("the actual fallback stage must remain pending"));
    assert_eq!(fallback.cause, panic_cause);
    assert!(!fallback.activation_containment_required);
    assert_eq!(
        fallback
            .assembly
            .loan
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AttachedOrderPanicPhase {
    Body,
    Terminal,
}

impl ProgressPhase for AttachedOrderPanicPhase {
    fn order(self) -> u32 {
        match self {
            Self::Body => 0,
            Self::Terminal => panic!("injected attached terminal phase order panic"),
        }
    }

    fn stable_id(self) -> &'static str {
        "attached-order-panic"
    }
}

type AttachedOrderPanicRoot =
    InvocationRoot<Owner, u8, AttachedOrderPanicPhase, Unit, u8, DropProbe, DropProbe, DropProbe>;
type AttachedOrderPanicPermission<'inv> = RetainedOperation<
    'inv,
    Owner,
    u8,
    AttachedOrderPanicPhase,
    Unit,
    u8,
    DropProbe,
    DropProbe,
    DropProbe,
>;

fn start_attached_order_panic<'inv>(
    root: &'inv mut AttachedOrderPanicRoot,
    account: &PhysicalStorageAccount,
    drops: &Arc<Mutex<Vec<&'static str>>>,
    events: &Arc<Mutex<Vec<&'static str>>>,
) -> AttachedOrderPanicPermission<'inv> {
    let observer_owner = DropProbe {
        label: "observer",
        drops: Arc::clone(drops),
        attribution: None,
    };
    let observed = Arc::clone(events);
    let control = OperationControl::default()
        .with_cancellation(UnattachedControlProbe(Arc::clone(events)))
        .with_progress_observer(
            move |_: crate::ProgressEvent<AttachedOrderPanicPhase, Unit>| {
                let _owner = &observer_owner;
                observed.lock().unwrap().push("observer");
                crate::ProgressDecision::Continue
            },
        );
    let (limits, plan) = limits_and_plan();
    let mut permission = RetainedOperation::try_start(root, control, limits, account, plan, || {
        Ok(DropProbe {
            label: "fallback",
            drops: Arc::clone(drops),
            attribution: None,
        })
    })
    .unwrap_or_else(|_| panic!("the actual fallback and numerical evidence must attach"));
    permission
        .loan
        .common
        .view()
        .coordinator
        .report_progress(AttachedOrderPanicPhase::Body, Unit, 1, None)
        .unwrap();
    permission
}

#[test]
fn evidence_only_phase_order_panic_keeps_the_actual_attached_root_and_accounting() {
    for explicit_cleanup in [false, true] {
        let account = account();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut root = AttachedOrderPanicRoot::new();
        let permission = start_attached_order_panic(&mut root, &account, &drops, &events);
        let identity = permission.loan().invocation_identity();
        assert!(!permission.loan.common.lifecycle().terminal_outstanding);
        let ledger = permission.loan.common.test_ledger();
        let work_before = ledger.dimension_evidence(1).unwrap();
        let probe = ledger.test_shared_probe();
        let numeric_before = (
            probe.current_live_bytes(),
            probe.active_reservations(),
            probe.current_administrative_bytes(),
        );
        let physical_before = account.snapshot();
        let shares_before = account.share_call_count();
        let owners_before = (Arc::strong_count(&drops), Arc::strong_count(&events));
        let mut expected_events = events.lock().unwrap().clone();
        assert_eq!(
            expected_events
                .iter()
                .filter(|event| **event == "observer")
                .count(),
            1
        );
        expected_events.push("poll");
        assert!(numeric_before.0 > 0 && numeric_before.1 > 0 && numeric_before.2 > 0);
        assert!(!physical_before.poisoned());
        assert!(physical_before.live_retained_bytes() > 0);
        assert!(physical_before.live_retained_allocations() > 0);
        assert!(drops.lock().unwrap().is_empty());

        let panic = match catch_unwind(AssertUnwindSafe(|| {
            let _ = permission.prepare_evidence_only_success(FinalProgress::new(
                AttachedOrderPanicPhase::Terminal,
                Unit,
                1,
            ));
        })) {
            Err(payload) => payload,
            Ok(()) => panic!("the terminal phase order must run on the real evidence-only rail"),
        };
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"injected attached terminal phase order panic")
        );
        assert_eq!(root.invocation_identity(), identity);
        let common = root.common.as_ref().unwrap();
        assert!(common.lifecycle.terminal_outstanding);
        assert_eq!(
            common
                .coordinator
                .retained_ledger()
                .dimension_evidence(1)
                .unwrap(),
            work_before
        );
        assert_eq!(
            common.coordinator.known_administrative_bytes(),
            Some(numeric_before.2)
        );
        assert!(
            root.retained
                .as_ref()
                .unwrap()
                .baseline
                .with_batches(|_| ())
                .is_some()
        );
        assert_eq!(account.snapshot(), physical_before);
        assert_eq!(*events.lock().unwrap(), expected_events);
        assert!(drops.lock().unwrap().is_empty());

        if explicit_cleanup {
            root.cleanup_owned_state();
            assert!(root.common.is_none());
            assert!(root.must_retain_domain());
            let contained = account.snapshot();
            assert!(contained.poisoned());
            root.cleanup_owned_state();
            assert_eq!(account.snapshot(), contained);
        }
        drop(root);

        let contained = account.snapshot();
        assert!(contained.poisoned());
        let first = contained.first_poison_evidence().unwrap();
        assert_eq!(
            first.reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
        assert_eq!(first.event_ordinal(), 1);
        assert_eq!(
            contained.live_retained_bytes(),
            physical_before.live_retained_bytes()
        );
        assert_eq!(
            contained.live_retained_allocations(),
            physical_before.live_retained_allocations()
        );
        assert!(probe.is_available());
        assert_eq!(
            (
                probe.current_live_bytes(),
                probe.active_reservations(),
                probe.current_administrative_bytes()
            ),
            numeric_before
        );
        assert_eq!(account.share_call_count(), shares_before);
        assert_eq!(
            (Arc::strong_count(&drops), Arc::strong_count(&events)),
            owners_before
        );
        assert_eq!(*events.lock().unwrap(), expected_events);
        assert!(drops.lock().unwrap().is_empty());
    }
}

#[test]
fn freeze_permit_phase_order_panic_keeps_the_real_validated_tables_and_control_owners() {
    for explicit_cleanup in [false, true] {
        let account = account();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut root = AttachedOrderPanicRoot::new();
        let permission = start_attached_order_panic(&mut root, &account, &drops, &events);
        let identity = permission.loan().invocation_identity();
        let table = RetainedBlockTablePlan::new(writer(), 1, 1);
        let mut assembly = permission
            .try_publication_assembly(RetainedPublicationPlan::success(table, table))
            .unwrap_or_else(|_| panic!("the real success tables must open"));
        let canonical = assembly
            .loan_mut()
            .try_retain_canonical(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
                Ok(emplacer.place(
                    DropProbe {
                        label: "canonical",
                        drops: Arc::clone(&drops),
                        attribution: None,
                    },
                    (),
                ))
            })
            .unwrap_or_else(|_| panic!("the actual canonical member must retain"));
        let evidence = assembly
            .loan_mut()
            .try_retain_outcome_evidence(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
                Ok(emplacer.place(
                    DropProbe {
                        label: "evidence",
                        drops: Arc::clone(&drops),
                        attribution: None,
                    },
                    (),
                ))
            })
            .unwrap_or_else(|_| panic!("the actual outcome member must retain"));
        assert!(assembly.loan().test_storage_graph_is_exact());
        let probe = assembly.loan().test_ledger_probe();
        let numeric_before = (
            probe.current_live_bytes(),
            probe.active_reservations(),
            probe.current_administrative_bytes(),
        );
        let validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session
                    .canonical(canonical)
                    .is_some_and(|value| value.label == "canonical")
                    && session
                        .outcome_evidence(evidence)
                        .is_some_and(|value| value.label == "evidence")
            }) {
                super::super::RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("the exact real closure must validate without fabricated permissions"),
            };
        assert_eq!(validated.proof.canonical_member_count(), 1);
        assert_eq!(validated.proof.outcome_evidence_member_count(), 1);
        let physical_before = account.snapshot();
        let shares_before = account.share_call_count();
        let owners_before = (Arc::strong_count(&drops), Arc::strong_count(&events));
        let mut expected_events = events.lock().unwrap().clone();
        expected_events.push("poll");
        assert!(numeric_before.0 > 0 && numeric_before.1 > 0 && numeric_before.2 > 0);
        assert!(!physical_before.poisoned());
        assert!(physical_before.live_retained_bytes() > 0);
        assert!(physical_before.live_retained_allocations() > 0);
        assert!(drops.lock().unwrap().is_empty());

        let panic = match catch_unwind(AssertUnwindSafe(|| {
            let mut validated = validated;
            let _ = validated.assembly.loan_mut().try_issue_freeze_decision(
                &validated.proof,
                1,
                1,
                FinalProgress::new(AttachedOrderPanicPhase::Terminal, Unit, 1),
            );
        })) {
            Err(payload) => payload,
            Ok(()) => {
                panic!("the terminal phase order must run after genuine freeze qualification")
            }
        };
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"injected attached terminal phase order panic")
        );
        assert_eq!(root.invocation_identity(), identity);
        let common = root.common.as_ref().unwrap();
        assert!(common.lifecycle.terminal_outstanding);
        assert_eq!(
            common.coordinator.known_administrative_bytes(),
            Some(numeric_before.2)
        );
        assert_eq!(
            account.snapshot().live_retained_bytes(),
            physical_before.live_retained_bytes()
        );
        assert_eq!(
            account.snapshot().live_retained_allocations(),
            physical_before.live_retained_allocations()
        );
        assert_eq!(*events.lock().unwrap(), expected_events);
        assert!(drops.lock().unwrap().is_empty());
        let first_before_cleanup = account.snapshot().first_poison_evidence();

        if explicit_cleanup {
            root.cleanup_owned_state();
            assert!(root.common.is_none());
            assert!(root.must_retain_domain());
            let contained = account.snapshot();
            assert!(contained.poisoned());
            root.cleanup_owned_state();
            assert_eq!(account.snapshot(), contained);
        }
        drop(root);

        let contained = account.snapshot();
        assert!(contained.poisoned());
        let first = contained.first_poison_evidence().unwrap();
        assert_eq!(
            first.reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
        assert_eq!(first.event_ordinal(), 1);
        if let Some(original) = first_before_cleanup {
            assert_eq!(first, original);
        }
        assert_eq!(
            contained.live_retained_bytes(),
            physical_before.live_retained_bytes()
        );
        assert_eq!(
            contained.live_retained_allocations(),
            physical_before.live_retained_allocations()
        );
        assert!(probe.is_available());
        assert_eq!(
            (
                probe.current_live_bytes(),
                probe.active_reservations(),
                probe.current_administrative_bytes()
            ),
            numeric_before
        );
        assert_eq!(account.share_call_count(), shares_before);
        assert_eq!(
            (Arc::strong_count(&drops), Arc::strong_count(&events)),
            owners_before
        );
        assert_eq!(*events.lock().unwrap(), expected_events);
        assert!(drops.lock().unwrap().is_empty());
    }
}

#[test]
fn minimal_terminal_stamp_ends_loans_before_consuming_the_original_root_once() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let mut permission = start(&mut root, &account, &drops);
    permission
        .loan
        .common
        .view()
        .coordinator
        .report_progress(Phase, Unit, 1, None)
        .unwrap();
    {
        let mut parts = permission
            .loan_mut()
            .exclusive_parts()
            .unwrap_or_else(|_| panic!("the original attached operation must permit work"));
        let work = parts.work.bind([1]).unwrap();
        parts.work.charge_observed(work, [3]).unwrap();
    }
    let identity = permission.loan().invocation_identity();
    let work_before = permission
        .loan
        .common
        .test_ledger()
        .dimension_evidence(1)
        .unwrap();
    let probe = permission.loan.common.test_ledger().test_shared_probe();
    let batch_owner = permission
        .loan
        .retained
        .baseline
        .test_batch_identity()
        .unwrap();
    let physical_before = account.snapshot();
    assert!(permission.defer_until_body_cleanup().is_ok());

    let requested = PreparationFailureCode::domain(61);
    let stamp = root
        .prepare_minimal_terminal(requested)
        .unwrap_or_else(|_| panic!("the real deferred fallback must prepare"))
        .expect("the original operation must yield its stamp");
    assert_eq!(root.invocation_identity(), identity);
    let common = root.common.as_ref().unwrap();
    assert!(common.lifecycle.terminal_outstanding);
    let work_ready = common
        .coordinator
        .retained_ledger()
        .dimension_evidence(1)
        .unwrap();
    assert_eq!(work_ready, work_before);
    let administration_ready = common.coordinator.known_administrative_bytes().unwrap();
    assert_eq!(administration_ready, probe.current_administrative_bytes());
    assert_eq!(
        root.retained
            .as_ref()
            .unwrap()
            .baseline
            .with_batches(|batches| std::ptr::from_ref(batches).cast::<()>()),
        Some(batch_owner)
    );
    let physical = root.terminal.as_ref().unwrap();
    assert!(physical.graph.fallback.is_some());
    assert!(physical.evidence.is_some());
    assert!(physical.qualification.is_some());
    assert!(matches!(
        physical.disposition,
        Some(TerminalDisposition::MinimalFallback { cause }) if cause == requested
    ));
    assert_eq!(account.snapshot(), physical_before);
    assert!(probe.is_available());
    assert!(drops.lock().unwrap().is_empty());

    let prepared = root
        .try_finish_minimal_terminal(stamp)
        .unwrap_or_else(|_| panic!("the original stamp must match its actual owner"))
        .unwrap_or_else(|_| panic!("the healthy original graph must detach"));
    assert!(root.common.is_none());
    assert!(root.terminal.is_none());
    assert!(!probe.is_available());
    assert!(matches!(root.prepare_minimal_terminal(requested), Ok(None)));
    let PreparedRetainedPublicationOutcome::MinimalFallback(prepared) = prepared.into_outcome()
    else {
        panic!("minimal preparation must retain the original fallback");
    };
    let published = prepared.handoff();
    assert_eq!(published.fallback.value().label, "fallback");
    assert!(published.fallback.account().is_same_account(&account));
    let work = published.ledger_evidence.dimension(1).unwrap();
    assert_eq!(
        (work.attempted(), work.consumed()),
        (work_before.attempted(), work_before.consumed())
    );
    assert_eq!(
        published.ledger_evidence.terminal_cause(),
        Some(super::super::TerminalEvidenceCause::Failure(requested))
    );
    let header = published.ledger_evidence.header().unwrap();
    assert_eq!(header.current_live_bytes(), 0);
    assert_eq!(header.active_reservations(), 0);
    assert_eq!(header.current_administrative_bytes(), administration_ready);
    assert!(account.snapshot().live_retained_bytes() > 0);
    drop(published);
    assert_eq!(*drops.lock().unwrap(), ["fallback"]);
    assert_released(&account);
    root.cleanup_owned_state();
    // The raw root cannot acknowledge the store layer's separate domain transfer.
    assert!(root.must_retain_domain());
    drop(root);
    assert_released(&account);
}

#[test]
fn dynamic_root_rejects_finite_evidence_before_installing_or_taking_an_original_owner() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let permission = start(&mut root, &account, &drops);
    let mut prepared = permission
        .into_loan()
        .prepare_minimal_fallback_deferred(PreparationFailureCode::domain(62))
        .unwrap_or_else(|_| panic!("the original dynamic graph must qualify"));
    prepared.qualification = super::super::terminal_loan::TerminalEvidenceQualification::Finite;
    let physical_before = account.snapshot();
    let identity = root.invocation_identity();
    let probe = root
        .common
        .as_ref()
        .unwrap()
        .coordinator
        .retained_ledger()
        .test_shared_probe();
    let numeric_before = (
        probe.current_live_bytes(),
        probe.active_reservations(),
        probe.current_administrative_bytes(),
    );
    let caught = catch_unwind(AssertUnwindSafe(|| {
        let _ = root.install_terminal_preparation(prepared);
    }));
    assert!(caught.is_err());
    assert_eq!(root.invocation_identity(), identity);
    assert_eq!(account.snapshot(), physical_before);
    let physical = root.terminal.as_ref().unwrap();
    assert!(physical.qualification.is_none() && physical.disposition.is_none());
    assert!(physical.graph.fallback.is_some() && physical.evidence.is_some());
    assert!(root.common.as_ref().unwrap().lifecycle.terminal_outstanding);
    assert_eq!(
        (
            probe.current_live_bytes(),
            probe.active_reservations(),
            probe.current_administrative_bytes()
        ),
        numeric_before
    );
    assert!(drops.lock().unwrap().is_empty());
    root.cleanup_owned_state();
    assert!(root.must_retain_domain());
    assert!(account.snapshot().poisoned());
    assert_eq!(
        account.snapshot().live_retained_bytes(),
        physical_before.live_retained_bytes()
    );
    assert_eq!(
        (
            probe.current_live_bytes(),
            probe.active_reservations(),
            probe.current_administrative_bytes()
        ),
        numeric_before
    );
    assert!(drops.lock().unwrap().is_empty());
}

#[test]
fn dynamic_finalizer_rejects_a_foreign_finite_common_loan_before_preparation_or_owner_effects() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let mut finite = Some(
        FiniteOperationInvocation::<Owner, u8, Phase, Unit, u8>::try_new(
            OperationControl::default(),
            crate::ledger::finite::FiniteLedgerLimits::try_new(
                u64::MAX,
                u64::MAX,
                [1_u8, 2, 3, 4, 5, 6]
                    .map(|dimension| (dimension, DimensionPolicy::counter(u64::MAX, u64::MAX, 0))),
            )
            .unwrap(),
            [1, 2, 3],
            CanonicalLineageAuthority::fresh(),
        )
        .unwrap(),
    );
    let mut permission = start(&mut root, &account, &drops);
    let original_slot = permission.loan().invocation_identity();
    let original_batch = permission.loan.retained.baseline.test_batch_identity();
    let physical_before = account.snapshot();
    permission.loan.common.common = InvocationSlot::Finite(&mut finite);
    let caught = catch_unwind(AssertUnwindSafe(|| {
        let _ = permission.prepare_minimal_fallback(PreparationFailureCode::domain(63));
    }));
    assert!(caught.is_err());
    assert_eq!(root.invocation_identity(), original_slot);
    assert!(root.terminal.is_none() && root.common.is_some());
    assert!(!root.common.as_ref().unwrap().lifecycle.terminal_outstanding);
    assert!(!finite.as_ref().unwrap().lifecycle.terminal_outstanding);
    assert_eq!(
        root.retained
            .as_ref()
            .unwrap()
            .baseline
            .with_batches(|batches| std::ptr::from_ref(batches).cast::<()>()),
        original_batch
    );
    assert_eq!(account.snapshot(), physical_before);
    assert!(drops.lock().unwrap().is_empty());
    root.cleanup_owned_state();
    assert_eq!(*drops.lock().unwrap(), ["fallback"]);
    assert_released(&account);
}

#[test]
fn minimal_terminal_foreign_stamp_leaves_both_original_owners_ready() {
    let accounts = [account(), account()];
    let drops = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut roots = [Root::new(), Root::new()];
    for (root, account) in roots.iter_mut().zip(&accounts) {
        let (limits, plan) = limits_and_plan();
        let permission = Permission::try_start(
            root,
            OperationControl::default()
                .with_cancellation(UnattachedControlProbe(Arc::clone(&events))),
            limits,
            account,
            plan,
            || {
                Ok(DropProbe {
                    label: "fallback",
                    drops: Arc::clone(&drops),
                    attribution: None,
                })
            },
        )
        .unwrap_or_else(|_| panic!("each real root must admit its own fallback"));
        assert!(permission.defer_until_body_cleanup().is_ok());
    }
    let requested = PreparationFailureCode::domain(62);
    let first_stamp = roots[0]
        .prepare_minimal_terminal(requested)
        .unwrap_or_else(|_| panic!("the first real fallback must prepare"))
        .unwrap();
    let second_stamp = roots[1]
        .prepare_minimal_terminal(requested)
        .unwrap_or_else(|_| panic!("the second real fallback must prepare"))
        .unwrap();
    let identities = roots.each_ref().map(Root::invocation_identity);
    let physical = accounts.each_ref().map(PhysicalStorageAccount::snapshot);
    let shares = accounts
        .each_ref()
        .map(PhysicalStorageAccount::share_call_count);
    let work = roots.each_ref().map(|root| {
        root.common
            .as_ref()
            .unwrap()
            .coordinator
            .retained_ledger()
            .dimension_evidence(1)
            .unwrap()
    });
    let batch_owners = roots.each_ref().map(|root| {
        root.retained
            .as_ref()
            .unwrap()
            .baseline
            .with_batches(|batches| std::ptr::from_ref(batches).cast::<()>())
            .unwrap()
    });
    let events_ready = events.lock().unwrap().clone();
    let second_stamp = match roots[0].try_finish_minimal_terminal(second_stamp) {
        Err(stamp) => stamp,
        Ok(_) => panic!("another actual root's stamp cannot consume this owner"),
    };
    for (index, root) in roots.iter().enumerate() {
        assert_eq!(root.invocation_identity(), identities[index]);
        let common = root.common.as_ref().unwrap();
        assert!(common.lifecycle.terminal_outstanding);
        assert_eq!(
            common.coordinator.retained_ledger().dimension_evidence(1),
            Ok(work[index])
        );
        assert_eq!(
            root.retained
                .as_ref()
                .unwrap()
                .baseline
                .with_batches(|batches| std::ptr::from_ref(batches).cast::<()>()),
            Some(batch_owners[index])
        );
        let terminal = root.terminal.as_ref().unwrap();
        assert!(terminal.graph.fallback.is_some() && terminal.evidence.is_some());
        assert!(terminal.qualification.is_some());
        assert!(matches!(
            terminal.disposition,
            Some(TerminalDisposition::MinimalFallback { cause }) if cause == requested
        ));
        assert_eq!(accounts[index].snapshot(), physical[index]);
        assert_eq!(accounts[index].share_call_count(), shares[index]);
        assert_eq!(
            super::super::account::lock_recovering_poison(&accounts[index].inner.value().linear)
                .admitted_account_actions,
            1
        );
    }
    assert_eq!(*events.lock().unwrap(), events_ready);
    assert!(drops.lock().unwrap().is_empty());
    for (root, stamp) in roots.iter_mut().zip([first_stamp, second_stamp]) {
        let prepared = root
            .try_finish_minimal_terminal(stamp)
            .unwrap_or_else(|_| panic!("rejection must preserve each original stamp"))
            .unwrap_or_else(|_| panic!("each unchanged graph must still detach"));
        assert!(root.common.is_none());
        assert!(root.terminal.is_none());
        drop(prepared);
    }
    drop(roots);
    assert_eq!(*events.lock().unwrap(), events_ready);
    assert_eq!(*drops.lock().unwrap(), ["fallback", "fallback"]);
    for (index, account) in accounts.iter().enumerate() {
        assert_released(account);
        assert_eq!(account.share_call_count(), shares[index]);
        assert_eq!(
            super::super::account::lock_recovering_poison(&account.inner.value().linear)
                .admitted_account_actions,
            1
        );
    }
}

#[test]
#[expect(
    clippy::forget_non_drop,
    reason = "the adversarial caller forgets the actual non-copy terminal stamp"
)]
fn forgotten_minimal_terminal_stamp_quarantines_the_actual_graph_without_refund_or_replay() {
    for explicit_cleanup in [false, true] {
        let account = account();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut root = Root::new();
        let (limits, plan) = limits_and_plan();
        let observer_owner = DropProbe {
            label: "observer",
            drops: Arc::clone(&drops),
            attribution: None,
        };
        let observed = Arc::clone(&events);
        let control = OperationControl::default()
            .with_cancellation(UnattachedControlProbe(Arc::clone(&events)))
            .with_progress_observer(move |_: crate::ProgressEvent<Phase, Unit>| {
                let _owner = &observer_owner;
                observed.lock().unwrap().push("observer");
                crate::ProgressDecision::Continue
            });
        let mut permission =
            Permission::try_start(&mut root, control, limits, &account, plan, || {
                Ok(DropProbe {
                    label: "fallback",
                    drops: Arc::clone(&drops),
                    attribution: None,
                })
            })
            .unwrap_or_else(|_| panic!("real original control and fallback must attach"));
        permission
            .loan
            .common
            .view()
            .coordinator
            .report_progress(Phase, Unit, 1, None)
            .unwrap();
        let identity = permission.loan().invocation_identity();
        let probe = permission.loan.common.test_ledger().test_shared_probe();
        assert!(permission.defer_until_body_cleanup().is_ok());
        let stamp = root
            .prepare_minimal_terminal(PreparationFailureCode::domain(63))
            .unwrap_or_else(|_| panic!("the actual fallback must reach a sealed decision"))
            .unwrap();
        let physical = account.snapshot();
        let numeric = (
            probe.current_live_bytes(),
            probe.active_reservations(),
            probe.current_administrative_bytes(),
        );
        let shares = account.share_call_count();
        let owners = (Arc::strong_count(&drops), Arc::strong_count(&events));
        let events_ready = events.lock().unwrap().clone();
        assert!(events_ready.contains(&"observer"));
        assert_eq!(
            events_ready
                .iter()
                .filter(|event| **event == "control")
                .count(),
            1
        );
        assert_eq!(*drops.lock().unwrap(), ["observer"]);
        assert!(numeric.0 > 0 && numeric.1 > 0 && numeric.2 > 0);
        assert!(!physical.poisoned());
        assert!(physical.live_retained_bytes() > 0 && physical.live_retained_allocations() > 0);
        std::mem::forget(stamp);

        assert_eq!(root.invocation_identity(), identity);
        assert!(root.common.as_ref().unwrap().lifecycle.terminal_outstanding);
        assert!(
            root.retained
                .as_ref()
                .unwrap()
                .baseline
                .with_batches(|_| ())
                .is_some()
        );
        let terminal = root.terminal.as_ref().unwrap();
        assert!(terminal.graph.fallback.is_some() && terminal.evidence.is_some());
        assert!(terminal.qualification.is_some());
        assert!(matches!(
            terminal.disposition,
            Some(TerminalDisposition::MinimalFallback { cause })
                if cause == PreparationFailureCode::domain(63)
        ));
        assert_eq!(account.snapshot(), physical);
        if explicit_cleanup {
            root.cleanup_owned_state();
            assert!(root.common.is_none());
            assert!(root.must_retain_domain());
            let contained = account.snapshot();
            root.cleanup_owned_state();
            assert_eq!(account.snapshot(), contained);
        }
        drop(root);

        let contained = account.snapshot();
        assert!(contained.poisoned());
        let first = contained.first_poison_evidence().unwrap();
        assert_eq!(
            first.reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
        assert_eq!(first.event_ordinal(), 1);
        assert_eq!(
            contained.live_retained_bytes(),
            physical.live_retained_bytes()
        );
        assert_eq!(
            contained.live_retained_allocations(),
            physical.live_retained_allocations()
        );
        assert!(probe.is_available());
        assert_eq!(
            (
                probe.current_live_bytes(),
                probe.active_reservations(),
                probe.current_administrative_bytes()
            ),
            numeric
        );
        assert_eq!(account.share_call_count(), shares);
        assert_eq!(
            super::super::account::lock_recovering_poison(&account.inner.value().linear)
                .admitted_account_actions,
            1
        );
        assert_eq!(
            (Arc::strong_count(&drops), Arc::strong_count(&events)),
            owners
        );
        assert_eq!(*events.lock().unwrap(), events_ready);
        assert_eq!(*drops.lock().unwrap(), ["observer"]);
    }
}

fn start_deferred_terminal_regression<'inv>(
    root: &'inv mut Root,
    account: &PhysicalStorageAccount,
    drops: &Arc<Mutex<Vec<&'static str>>>,
    events: &Arc<Mutex<Vec<&'static str>>>,
    cancel_final_progress: bool,
) -> Permission<'inv> {
    let observer_owner = DropProbe {
        label: "observer",
        drops: Arc::clone(drops),
        attribution: None,
    };
    let observed = Arc::clone(events);
    let control = OperationControl::default()
        .with_cancellation(UnattachedControlProbe(Arc::clone(events)))
        .with_progress_observer(move |event: crate::ProgressEvent<Phase, Unit>| {
            let _owner = &observer_owner;
            observed.lock().unwrap().push("observer");
            if cancel_final_progress && event.completed() == 2 {
                crate::ProgressDecision::Cancel
            } else {
                crate::ProgressDecision::Continue
            }
        });
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
    let plan = plan.with_failure_terminal_work(failure_work);
    let mut permission = Permission::try_start(root, control, limits, account, plan, || {
        Ok(DropProbe {
            label: "fallback",
            drops: Arc::clone(drops),
            attribution: None,
        })
    })
    .unwrap_or_else(|_| panic!("the real terminal regression root must start"));
    permission
        .loan
        .common
        .view()
        .coordinator
        .report_progress(Phase, Unit, 1, None)
        .unwrap();
    {
        let mut parts = permission
            .loan_mut()
            .exclusive_parts()
            .unwrap_or_else(|_| panic!("the actual operation must admit completed work"));
        let work = parts.work.bind([1]).unwrap();
        parts.work.charge_observed(work, [3]).unwrap();
    }
    permission
}

fn retain_terminal_regression_payloads(
    assembly: &mut RetainedPublicationAssembly<
        '_,
        Owner,
        u8,
        Phase,
        Unit,
        u8,
        DropProbe,
        DropProbe,
        DropProbe,
    >,
    drops: &Arc<Mutex<Vec<&'static str>>>,
) -> (
    CanonicalRetainedSlot<DropProbe, ()>,
    OutcomeEvidenceRetainedSlot<DropProbe, ()>,
) {
    let canonical = assembly
        .loan_mut()
        .try_retain_canonical(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
            Ok(emplacer.place(
                DropProbe {
                    label: "canonical",
                    drops: Arc::clone(drops),
                    attribution: None,
                },
                (),
            ))
        })
        .unwrap_or_else(|_| panic!("the original canonical member must retain"));
    let evidence = assembly
        .loan_mut()
        .try_retain_outcome_evidence(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
            Ok(emplacer.place(
                DropProbe {
                    label: "evidence",
                    drops: Arc::clone(drops),
                    attribution: None,
                },
                (),
            ))
        })
        .unwrap_or_else(|_| panic!("the original outcome member must retain"));
    assert!(assembly.loan().test_storage_graph_is_exact());
    (canonical, evidence)
}

#[test]
fn evidence_only_ready_keeps_original_numerical_owners_until_concrete_root_consumption() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let permission =
        start_deferred_terminal_regression(&mut root, &account, &drops, &events, false);
    let identity = permission.loan().invocation_identity();
    let work_before = permission
        .loan
        .common
        .test_ledger()
        .dimension_evidence(1)
        .unwrap();
    let probe = permission.loan.common.test_ledger().test_shared_probe();
    let batch_owner = permission
        .loan
        .retained
        .baseline
        .test_batch_identity()
        .unwrap();
    assert!(permission.defer_until_body_cleanup().is_ok());

    let stamp = root
        .prepare_evidence_only_terminal(FinalProgress::new(Phase, Unit, 2))
        .unwrap_or_else(|_| panic!("the real evidence-only success must prepare"))
        .expect("the original operation must yield its decision stamp");
    assert_eq!(root.invocation_identity(), identity);
    let common = root.common.as_ref().unwrap();
    assert!(common.lifecycle.terminal_outstanding);
    let work_ready = common
        .coordinator
        .retained_ledger()
        .dimension_evidence(1)
        .unwrap();
    assert_eq!(
        (work_ready.attempted(), work_ready.consumed()),
        (work_before.attempted(), work_before.consumed())
    );
    let administration = common.coordinator.known_administrative_bytes().unwrap();
    assert_eq!(administration, probe.current_administrative_bytes());
    assert_eq!(
        root.retained
            .as_ref()
            .unwrap()
            .baseline
            .with_batches(|batches| std::ptr::from_ref(batches).cast::<()>()),
        Some(batch_owner)
    );
    let physical = root.terminal.as_ref().unwrap();
    assert!(
        physical
            .graph
            .fallback
            .as_ref()
            .is_some_and(super::super::DetachedRetainedPart::is_empty)
    );
    assert!(physical.evidence.is_some() && physical.qualification.is_some());
    assert!(matches!(
        physical.disposition,
        Some(TerminalDisposition::EvidenceOnlySuccess)
    ));
    assert!(probe.is_available() && probe.is_terminal_qualified());
    assert_eq!(*drops.lock().unwrap(), ["observer", "fallback"]);
    let shares = account.share_call_count();
    let events_ready = events.lock().unwrap().clone();
    assert_eq!(
        events_ready
            .iter()
            .filter(|event| **event == "observer")
            .count(),
        2
    );
    assert_eq!(
        events_ready
            .iter()
            .filter(|event| **event == "control")
            .count(),
        1
    );

    let prepared = root
        .try_finish_evidence_only_terminal(stamp)
        .unwrap_or_else(|_| panic!("the original stamp must consume its original owner"))
        .unwrap_or_else(|_| panic!("the qualified evidence graph must detach"));
    let PreparedRetainedEvidenceOnlyOutcome::Success(prepared) = prepared else {
        panic!("healthy evidence-only preparation must retain success");
    };
    let published = prepared.handoff();
    let work = published.ledger_evidence.dimension(1).unwrap();
    assert_eq!(
        (work.attempted(), work.consumed()),
        (work_before.attempted(), work_before.consumed())
    );
    let header = published.ledger_evidence.header().unwrap();
    assert_eq!(
        header.terminal_cause(),
        super::super::TerminalEvidenceCause::Success
    );
    assert_eq!(
        (header.current_live_bytes(), header.active_reservations()),
        (0, 0)
    );
    assert_eq!(header.current_administrative_bytes(), administration);
    assert!(root.common.is_none() && root.terminal.is_none());
    assert!(!probe.is_available());
    assert_eq!(account.share_call_count(), shares);
    assert_eq!(
        super::super::account::lock_recovering_poison(&account.inner.value().linear)
            .admitted_account_actions,
        1
    );
    assert_eq!(*events.lock().unwrap(), events_ready);
    root.cleanup_owned_state();
    assert!(root.must_retain_domain());
    drop(root);
    assert!(account.snapshot().live_retained_bytes() > 0);
    drop(published);
    assert_eq!(*drops.lock().unwrap(), ["observer", "fallback"]);
    assert_released(&account);
}

#[test]
fn evidence_only_final_cancellation_seals_minimal_failure_without_control_replay() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let permission = start_deferred_terminal_regression(&mut root, &account, &drops, &events, true);
    let identity = permission.loan().invocation_identity();
    let work_before = permission
        .loan
        .common
        .test_ledger()
        .dimension_evidence(1)
        .unwrap();
    let probe = permission.loan.common.test_ledger().test_shared_probe();
    let batch_owner = permission
        .loan
        .retained
        .baseline
        .test_batch_identity()
        .unwrap();
    let physical_before = account.snapshot();
    assert!(permission.defer_until_body_cleanup().is_ok());
    let stamp = root
        .prepare_evidence_only_terminal(FinalProgress::new(Phase, Unit, 2))
        .unwrap_or_else(|_| panic!("final cancellation must retain a prepared fallback"))
        .unwrap();
    let expected = PreparationFailureCode::from_control(&ControlCause::Cancelled {
        origin: crate::CancellationOrigin::Observer,
    });
    assert_eq!(root.invocation_identity(), identity);
    let common = root.common.as_ref().unwrap();
    assert!(common.lifecycle.terminal_outstanding);
    assert_eq!(
        common.coordinator.retained_ledger().dimension_evidence(1),
        Ok(work_before)
    );
    let administration = common.coordinator.known_administrative_bytes().unwrap();
    assert_eq!(administration, probe.current_administrative_bytes());
    assert_eq!(
        root.retained
            .as_ref()
            .unwrap()
            .baseline
            .with_batches(|batches| std::ptr::from_ref(batches).cast::<()>()),
        Some(batch_owner)
    );
    let physical = root.terminal.as_ref().unwrap();
    assert!(physical.graph.fallback.is_some() && physical.evidence.is_some());
    assert!(physical.qualification.is_some());
    assert!(matches!(physical.disposition,
        Some(TerminalDisposition::MinimalFallback { cause }) if cause == expected));
    assert_eq!(account.snapshot(), physical_before);
    assert!(probe.is_available() && probe.is_terminal_qualified());
    assert_eq!(*drops.lock().unwrap(), ["observer"]);
    let events_ready = events.lock().unwrap().clone();
    let shares = account.share_call_count();
    assert_eq!(
        events_ready
            .iter()
            .filter(|event| **event == "observer")
            .count(),
        2
    );
    assert_eq!(
        events_ready
            .iter()
            .filter(|event| **event == "control")
            .count(),
        1
    );

    let prepared = root
        .try_finish_evidence_only_terminal(stamp)
        .unwrap_or_else(|_| panic!("the cancelled operation retains its original stamp"))
        .unwrap_or_else(|_| panic!("the real cancellation fallback must detach"));
    let PreparedRetainedEvidenceOnlyOutcome::MinimalFallback(prepared) = prepared else {
        panic!("observer cancellation cannot publish evidence-only success");
    };
    let published = prepared.handoff();
    assert_eq!(published.fallback.value().label, "fallback");
    assert!(published.fallback.account().is_same_account(&account));
    let work = published.ledger_evidence.dimension(1).unwrap();
    assert_eq!(
        (work.attempted(), work.consumed()),
        (work_before.attempted(), work_before.consumed())
    );
    let header = published.ledger_evidence.header().unwrap();
    assert_eq!(
        header.terminal_cause(),
        super::super::TerminalEvidenceCause::Failure(expected)
    );
    assert_eq!(
        (header.current_live_bytes(), header.active_reservations()),
        (0, 0)
    );
    assert_eq!(header.current_administrative_bytes(), administration);
    assert!(root.common.is_none() && root.terminal.is_none());
    assert!(!probe.is_available());
    assert!(matches!(
        root.prepare_evidence_only_terminal(FinalProgress::new(Phase, Unit, 2)),
        Ok(None)
    ));
    assert_eq!(*events.lock().unwrap(), events_ready);
    assert_eq!(account.share_call_count(), shares);
    assert_eq!(
        super::super::account::lock_recovering_poison(&account.inner.value().linear)
            .admitted_account_actions,
        1
    );
    root.cleanup_owned_state();
    assert!(root.must_retain_domain());
    drop(root);
    assert_eq!(*drops.lock().unwrap(), ["observer"]);
    drop(published);
    assert_eq!(*drops.lock().unwrap(), ["observer", "fallback"]);
    assert_released(&account);
}

#[test]
fn validated_changed_success_ready_preserves_original_tables_set_and_keys_until_root_finish() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let permission =
        start_deferred_terminal_regression(&mut root, &account, &drops, &events, false);
    let identity = permission.loan().invocation_identity();
    let batch_owner = permission
        .loan
        .retained
        .baseline
        .test_batch_identity()
        .unwrap();
    let table = RetainedBlockTablePlan::new(writer(), 1, 1);
    let mut assembly = permission
        .try_publication_assembly(RetainedPublicationPlan::success(table, table))
        .unwrap_or_else(|_| panic!("the actual success tables must open"));
    let (canonical, evidence) = retain_terminal_regression_payloads(&mut assembly, &drops);
    let set_id = assembly.loan.fields.set_id;
    let tables = (
        std::ptr::from_ref(
            assembly
                .loan
                .fields
                .canonical_table
                .as_ref()
                .unwrap()
                .as_ref(),
        ),
        std::ptr::from_ref(
            assembly
                .loan
                .fields
                .evidence_table
                .as_ref()
                .unwrap()
                .as_ref(),
        ),
    );
    let probe = assembly.loan().test_ledger_probe();
    let work_before = assembly.loan().test_dimension_evidence(1).unwrap();
    let (prepared, control_cause) = {
        let mut validated =
            match assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                session
                    .canonical(canonical)
                    .is_some_and(|value| value.label == "canonical")
                    && session
                        .outcome_evidence(evidence)
                        .is_some_and(|value| value.label == "evidence")
            }) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("the genuine original roots must validate"),
            };
        match validated
            .assembly
            .loan_mut()
            .try_issue_freeze_decision(&validated.proof, 1, 1, FinalProgress::new(Phase, Unit, 2))
            .unwrap_or_else(|_| panic!("actual exact destination qualification must succeed"))
        {
            RetainedSuccessFreezeDecision::Ready => {}
            _ => panic!("the validated original candidate must receive a real freeze permit"),
        }
        let crate::retained::ValidatedRetainedSuccess { assembly, proof } = validated;
        let classification = assembly
            .into_loan()
            .try_classify_terminal_preparation(
                crate::retained::RetainedTerminalRequest::FrozenSuccess { proof },
            )
            .unwrap_or_else(|_| panic!("the actual frozen success must classify"));
        assert!(classification.is_success());
        classification.finish_deferred().unwrap_or_else(|_| {
            panic!("success qualification must leave its owners in the actual root")
        })
    };
    assert_eq!(control_cause, None);
    let stamp = root.install_terminal_preparation(prepared);
    assert_eq!(root.invocation_identity(), identity);
    let common = root.common.as_ref().unwrap();
    assert!(common.lifecycle.terminal_outstanding);
    let work_ready = common
        .coordinator
        .retained_ledger()
        .dimension_evidence(1)
        .unwrap();
    assert_eq!(
        (work_ready.attempted(), work_ready.consumed()),
        (work_before.attempted(), work_before.consumed())
    );
    let administration = common.coordinator.known_administrative_bytes().unwrap();
    assert_eq!(administration, probe.current_administrative_bytes());
    assert_eq!(
        root.retained
            .as_ref()
            .unwrap()
            .baseline
            .with_batches(|batches| std::ptr::from_ref(batches).cast::<()>()),
        Some(batch_owner)
    );
    let fields = root.assembly.as_ref().unwrap();
    assert!(fields.canonical_batch.is_some() && fields.evidence_batch.is_some());
    let physical = root.terminal.as_ref().unwrap();
    assert!(
        physical
            .graph
            .fallback
            .as_ref()
            .is_some_and(super::super::DetachedRetainedPart::is_empty)
    );
    assert_eq!(
        std::ptr::from_ref(physical.graph.canonical_table.as_ref().unwrap().as_ref()),
        tables.0
    );
    assert_eq!(
        std::ptr::from_ref(physical.graph.evidence_table.as_ref().unwrap().as_ref()),
        tables.1
    );
    assert!(physical.evidence.is_some() && physical.qualification.is_some());
    assert!(
        physical
            .graph
            .account
            .as_ref()
            .unwrap()
            .is_same_account(&account)
    );
    assert!(matches!(physical.disposition,
        Some(TerminalDisposition::Success { canonical_root, evidence_root, set_id: actual_set })
            if canonical_root == canonical.identity() && evidence_root == evidence.identity()
                && actual_set == set_id));
    assert!(probe.is_available() && probe.is_terminal_qualified());
    let live_ready = (probe.current_live_bytes(), probe.active_reservations());
    assert!(live_ready.0 > 0 && live_ready.1 > 0);
    assert_eq!(*drops.lock().unwrap(), ["observer", "fallback"]);
    let shares = account.share_call_count();
    let events_ready = events.lock().unwrap().clone();
    let admitted_ready =
        super::super::account::lock_recovering_poison(&account.inner.value().linear)
            .admitted_account_actions;
    assert_eq!(admitted_ready, 6);

    let formation = root
        .try_finish_publication_terminal(stamp)
        .unwrap_or_else(|_| panic!("the original success stamp must match"))
        .unwrap_or_else(|_| panic!("the real qualified graph must detach"));
    let RetainedSuccessJointCommitFormation::Intent(intent) = formation else {
        panic!("the qualified unchanged decision must form its success intent");
    };
    // Detached live cover remains actual until the account-bound joint commit.
    assert_eq!(intent.test_current_live_bytes(), live_ready.0);
    assert_eq!(intent.test_active_reservations(), live_ready.1);
    assert!(intent.test_cover_matches_state());
    let published = intent
        .qualify_and_commit_frozen_exact()
        .unwrap_or_else(|_| panic!("the original account must jointly commit the exact success"))
        .handoff();
    assert_eq!(
        published
            .canonical
            .aggregate()
            .get(canonical)
            .unwrap()
            .label,
        "canonical"
    );
    assert_eq!(
        published
            .outcome_evidence
            .aggregate()
            .get(evidence)
            .unwrap()
            .label,
        "evidence"
    );
    assert_eq!(published.canonical.aggregate().set_id(), set_id);
    assert_eq!(published.canonical.root, canonical.identity());
    assert_eq!(published.outcome_evidence.root, evidence.identity());
    let work = published.ledger_evidence.dimension(1).unwrap();
    assert_eq!(
        (work.attempted(), work.consumed()),
        (work_before.attempted(), work_before.consumed())
    );
    assert_eq!(work.committed_canonical_output(), 1);
    // This fixture uses the same dimension for one domain member and one ledger record.
    assert_eq!(work.published_outcome_evidence(), 2);
    let header = published.ledger_evidence.header().unwrap();
    assert_eq!(
        header.terminal_cause(),
        super::super::TerminalEvidenceCause::Success
    );
    assert_eq!(
        (header.current_live_bytes(), header.active_reservations()),
        (0, 0)
    );
    assert_eq!(header.current_administrative_bytes(), administration);
    assert!(root.common.is_none() && root.terminal.is_none());
    assert!(!probe.is_available());
    assert_eq!(account.share_call_count(), shares);
    assert_eq!(
        super::super::account::lock_recovering_poison(&account.inner.value().linear)
            .admitted_account_actions,
        admitted_ready
    );
    assert_eq!(*events.lock().unwrap(), events_ready);
    root.cleanup_owned_state();
    assert!(root.must_retain_domain());
    drop(root);
    assert_eq!(*drops.lock().unwrap(), ["observer", "fallback"]);
    drop(published);
    assert_eq!(
        *drops.lock().unwrap(),
        ["observer", "fallback", "canonical", "evidence"]
    );
    assert_released(&account);
}

#[test]
fn validated_rich_failure_discards_real_canonical_table_before_ready_and_preserves_owned_evidence()
{
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new();
    let permission =
        start_deferred_terminal_regression(&mut root, &account, &drops, &events, false);
    let identity = permission.loan().invocation_identity();
    let batch_owner = permission
        .loan
        .retained
        .baseline
        .test_batch_identity()
        .unwrap();
    let table = RetainedBlockTablePlan::new(writer(), 1, 1);
    let mut assembly = permission
        .try_publication_assembly(RetainedPublicationPlan::success(table, table))
        .unwrap_or_else(|_| panic!("the real candidate tables must open before rejection"));
    let (_, evidence) = retain_terminal_regression_payloads(&mut assembly, &drops);
    let set_id = assembly.loan.fields.set_id;
    let evidence_table = std::ptr::from_ref(
        assembly
            .loan
            .fields
            .evidence_table
            .as_ref()
            .unwrap()
            .as_ref(),
    );
    let probe = assembly.loan().test_ledger_probe();
    let work_before = assembly.loan().test_dimension_evidence(1).unwrap();
    let physical_before = account.snapshot();
    let requested = PreparationFailureCode::domain(64);
    let (prepared, control_cause) = {
        let mut validated = match assembly.validate_rich_failure(evidence, |session, evidence| {
            session
                .outcome_evidence(evidence)
                .is_some_and(|value| value.label == "evidence")
        }) {
            RetainedFailureValidation::Valid(validated) => validated,
            _ => panic!("the real evidence closure must validate"),
        };
        validated
            .assembly
            .loan_mut()
            .try_stage_exact_outcome_evidence(&validated.proof, 1)
            .unwrap_or_else(|_| {
                panic!("the real evidence member must receive its exact commitment")
            });
        let super::super::ValidatedRetainedFailure { assembly, proof } = validated;
        let classification = assembly
            .into_loan()
            .try_classify_terminal_preparation(super::super::RetainedTerminalRequest::RichFailure {
                proof,
                requested_cause: requested,
                final_progress: FinalProgress::new(Phase, Unit, 2),
            })
            .unwrap_or_else(|_| {
                panic!("the actual rejected candidate must classify as rich failure")
            });
        assert!(matches!(&classification.state,
        super::super::TerminalPreFreezeTransition::Rich { cause, .. } if *cause == requested));
        classification.finish_deferred().unwrap_or_else(|_| {
            panic!("real table discard and qualification must finish before Ready")
        })
    };
    assert_eq!(control_cause, None);
    let stamp = root.install_terminal_preparation(prepared);
    assert_eq!(root.invocation_identity(), identity);
    let common = root.common.as_ref().unwrap();
    assert!(common.lifecycle.terminal_outstanding);
    let work_ready = common
        .coordinator
        .retained_ledger()
        .dimension_evidence(1)
        .unwrap();
    assert_eq!(
        (work_ready.attempted(), work_ready.consumed()),
        (work_before.attempted(), work_before.consumed())
    );
    let administration = common.coordinator.known_administrative_bytes().unwrap();
    assert_eq!(administration, probe.current_administrative_bytes());
    assert_eq!(
        root.retained
            .as_ref()
            .unwrap()
            .baseline
            .with_batches(|batches| std::ptr::from_ref(batches).cast::<()>()),
        Some(batch_owner)
    );
    let fields = root.assembly.as_ref().unwrap();
    assert!(fields.canonical_batch.is_some() && fields.evidence_batch.is_some());
    let physical = root.terminal.as_ref().unwrap();
    assert!(
        physical
            .graph
            .fallback
            .as_ref()
            .is_some_and(super::super::DetachedRetainedPart::is_empty)
            && physical.graph.canonical_table.is_none()
    );
    assert_eq!(
        std::ptr::from_ref(physical.graph.evidence_table.as_ref().unwrap().as_ref()),
        evidence_table
    );
    assert!(
        physical
            .graph
            .account
            .as_ref()
            .unwrap()
            .is_same_account(&account)
    );
    assert!(physical.evidence.is_some() && physical.qualification.is_some());
    assert!(matches!(physical.disposition,
        Some(TerminalDisposition::RichFailure { evidence_root, set_id: actual_set, cause })
            if evidence_root == evidence.identity() && actual_set == set_id && cause == requested));
    assert!(probe.is_available() && probe.is_terminal_qualified());
    assert_eq!(
        *drops.lock().unwrap(),
        ["observer", "canonical", "fallback"]
    );
    assert!(account.snapshot().live_retained_bytes() < physical_before.live_retained_bytes());
    assert!(
        account.snapshot().live_retained_allocations()
            < physical_before.live_retained_allocations()
    );
    let events_ready = events.lock().unwrap().clone();
    let admitted_ready =
        super::super::account::lock_recovering_poison(&account.inner.value().linear)
            .admitted_account_actions;
    assert_eq!(admitted_ready, 6);
    let shares = account.share_call_count();

    let formation = root
        .try_finish_publication_terminal(stamp)
        .unwrap_or_else(|_| panic!("the original failure stamp must match"))
        .unwrap_or_else(|_| panic!("the actual qualified failure graph must detach"));
    let RetainedSuccessJointCommitFormation::PreparedFailure(prepared) = formation else {
        panic!("a rich-failure decision cannot produce a success intent");
    };
    let PreparedRetainedPublicationOutcome::RichFailure(prepared) = prepared.into_outcome() else {
        panic!("the actual rich failure must preserve its owned evidence output");
    };
    let published = prepared.handoff();
    assert!(
        published
            .outcome_evidence
            .account()
            .is_same_account(&account)
    );
    let output = published.outcome_evidence.outcome_evidence();
    assert_eq!(output.root, evidence.identity());
    assert_eq!(output.aggregate().get(evidence).unwrap().label, "evidence");
    let work = published.ledger_evidence.dimension(1).unwrap();
    assert_eq!(
        (work.attempted(), work.consumed()),
        (work_before.attempted(), work_before.consumed())
    );
    assert_eq!(work.committed_canonical_output(), 0);
    // This fixture uses the same dimension for one domain member and one ledger record.
    assert_eq!(work.published_outcome_evidence(), 2);
    let header = published.ledger_evidence.header().unwrap();
    assert_eq!(
        header.terminal_cause(),
        super::super::TerminalEvidenceCause::Failure(requested)
    );
    assert_eq!(
        (header.current_live_bytes(), header.active_reservations()),
        (0, 0)
    );
    assert_eq!(header.current_administrative_bytes(), administration);
    assert!(root.common.is_none() && root.terminal.is_none());
    assert!(!probe.is_available());
    assert_eq!(account.share_call_count(), shares);
    assert_eq!(
        super::super::account::lock_recovering_poison(&account.inner.value().linear)
            .admitted_account_actions,
        admitted_ready
    );
    assert_eq!(*events.lock().unwrap(), events_ready);
    root.cleanup_owned_state();
    assert!(root.must_retain_domain());
    drop(root);
    assert_eq!(
        *drops.lock().unwrap(),
        ["observer", "canonical", "fallback"]
    );
    drop(published);
    assert_eq!(
        *drops.lock().unwrap(),
        ["observer", "canonical", "fallback", "evidence"]
    );
    assert_released(&account);
}

#[test]
fn actual_terminal_fields_quarantine_physical_and_batch_slots_on_detach_unwind() {
    for after_canonical_detach in [false, true] {
        let account = account();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut root = Root::new();
        let permission =
            start_deferred_terminal_regression(&mut root, &account, &drops, &events, false);
        let table = RetainedBlockTablePlan::new(writer(), 1, 1);
        let mut assembly = permission
            .try_publication_assembly(RetainedPublicationPlan::success(table, table))
            .unwrap_or_else(|_| panic!("the actual table graph must open"));
        let (canonical, evidence) = retain_terminal_regression_payloads(&mut assembly, &drops);
        let probe = assembly.loan().test_ledger_probe();
        // Complete the actual phase so capture uses its positioned fact.
        assembly
            .loan
            .operation
            .as_mut()
            .unwrap()
            .try_complete_phase(writer().phase)
            .unwrap();
        let (deferred, control_cause) = {
            let validated = match assembly.validate_success(
                canonical,
                evidence,
                |session, canonical, evidence| {
                    session.canonical(canonical).is_some()
                        && session.outcome_evidence(evidence).is_some()
                },
            ) {
                RetainedSuccessValidation::Valid(validated) => validated,
                _ => panic!("the original retained roots must validate"),
            };
            let crate::retained::ValidatedRetainedSuccess { assembly, proof } = validated;
            assembly
                .into_loan()
                .try_classify_terminal_preparation(
                    crate::retained::RetainedTerminalRequest::Success {
                        proof,
                        final_progress: FinalProgress::new(Phase, Unit, 2),
                    },
                )
                .unwrap_or_else(|_| panic!("actual success must classify"))
                .finish_deferred()
                .unwrap_or_else(|_| panic!("actual fields must reach Ready"))
        };
        assert_eq!(control_cause, None);
        let stamp = root.install_terminal_preparation(deferred);
        let physical_before = account.snapshot();
        let numeric_before = (
            probe.current_live_bytes(),
            probe.active_reservations(),
            probe.current_administrative_bytes(),
        );
        assert!(numeric_before.0 > 0 && numeric_before.1 > 0 && numeric_before.2 > 0);
        assert!(
            root.retained
                .as_ref()
                .unwrap()
                .baseline
                .with_batches(|_| ())
                .is_some()
        );
        let assembly = root.assembly.as_ref().unwrap();
        assert!(assembly.canonical_batch.is_some() && assembly.evidence_batch.is_some());
        let common = root.common.as_mut().unwrap();
        if after_canonical_detach {
            common.test_panic_after_next_canonical_detach();
        } else {
            common
                .coordinator
                .retained_ledger_mut()
                .test_corrupt_first_terminal_phase_position();
        }
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = root.try_finish_publication_terminal(stamp);
        }));
        assert!(caught.is_err());
        assert!(root.must_retain_domain());
        assert!(root.common.is_none() && root.terminal.is_none());
        assert!(
            root.retained
                .as_ref()
                .unwrap()
                .baseline
                .with_batches(|_| ())
                .is_none()
        );
        assert!(
            root.retained
                .as_ref()
                .unwrap()
                .metadata
                .deferred_stage
                .is_none()
        );
        let assembly = root.assembly.as_ref().unwrap();
        assert!(assembly.is_empty());
        let contained = account.snapshot();
        assert!(contained.poisoned());
        assert_eq!(
            contained.first_poison_evidence().unwrap().reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
        assert_eq!(
            contained.live_retained_bytes(),
            physical_before.live_retained_bytes()
        );
        assert_eq!(
            contained.live_retained_allocations(),
            physical_before.live_retained_allocations()
        );
        assert!(probe.is_available());
        assert_eq!(
            (
                probe.current_live_bytes(),
                probe.active_reservations(),
                probe.current_administrative_bytes()
            ),
            numeric_before
        );
        assert_eq!(*drops.lock().unwrap(), ["observer", "fallback"]);
        let events_before = events.lock().unwrap().clone();
        root.cleanup_owned_state();
        root.cleanup_owned_state();
        assert!(root.must_retain_domain());
        assert_eq!(account.snapshot(), contained);
        assert_eq!(*events.lock().unwrap(), events_before);
        assert_eq!(*drops.lock().unwrap(), ["observer", "fallback"]);
        drop(root);
        assert_eq!(account.snapshot(), contained);
        assert!(probe.is_available());
        assert_eq!(
            (
                probe.current_live_bytes(),
                probe.active_reservations(),
                probe.current_administrative_bytes()
            ),
            numeric_before
        );
        assert_eq!(*events.lock().unwrap(), events_before);
        assert_eq!(*drops.lock().unwrap(), ["observer", "fallback"]);
    }
}
