use super::finite_attached::FiniteTerminalRootFields;
use super::*;
use crate::ledger::finite::FiniteLedgerSnapshot;
use crate::ledger::{
    AttributionBatchPlan, FiniteStartupLedgerPlanParts, LedgerTestProbe,
    OperationAttributionReservation, OutputCommitment, StartupAccountShareEnvelope,
    StartupRetainedAttribution, TypedFreezeDetachErrorKind,
};
use crate::retained::ownership::FiniteFallbackPublicationBaseline;
use crate::retained::physical::FiniteAccountBoundJointCommit;
use crate::retained::startup::FiniteStartupCrossPlanePlan;
use crate::retained::terminal_finite::{
    FiniteRetainedOutputPayload, FiniteRetainedPublicationPayload,
    PreparedFiniteRetainedPublication,
};
use crate::retained::{
    CanonicalRetainedSlot, DeferredAttachedTerminal, FiniteTerminalPhysicalState,
    OutcomeEvidenceRetainedSlot, PhysicalStorageAccountFactory,
    PhysicalStorageAccountFactoryLimits, PhysicalStorageAccountLimits, RetainedBlockTablePlan,
    RetainedClosureValidation, RetainedFailureTerminalWorkPlan, RetainedPublicationAssemblyLoan,
    RetainedPublicationPayload, RetainedPublicationPlan, RetainedTerminalRequest,
    TerminalDisposition, TerminalEvidenceCause,
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
        "finite-physical-terminal"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Unit;
impl ProgressUnit for Unit {
    fn stable_id(self) -> &'static str {
        "items"
    }
}

struct Payload {
    label: &'static str,
    drops: Arc<Mutex<Vec<&'static str>>>,
    drop_probe: Option<PayloadDropProbe>,
}

struct PayloadDropProbe {
    probe: LedgerTestProbe<u8, u8>,
    observations: Arc<Mutex<Vec<PayloadDropObservation>>>,
}

#[derive(Debug, PartialEq, Eq)]
struct PayloadDropObservation {
    label: &'static str,
    probe_available: bool,
    numerical_cover: (u64, u64, u64),
}

impl Drop for Payload {
    fn drop(&mut self) {
        if let Some(observation) = &self.drop_probe {
            observation
                .observations
                .lock()
                .unwrap()
                .push(PayloadDropObservation {
                    label: self.label,
                    probe_available: observation.probe.is_available(),
                    numerical_cover: numerical_cover(&observation.probe),
                });
        }
        self.drops.lock().unwrap().push(self.label);
    }
}

type Invocation = FiniteOperationInvocation<Owner, u8, Phase, Unit, u8>;
type Baseline = FiniteFallbackPublicationBaseline<Owner, u8, u8, Payload>;
type Physical = FiniteTerminalPhysicalState<Payload, Payload, Payload>;
type Commit = FiniteAccountBoundJointCommit<Physical, Owner, u8, u8>;
type Prepared = PreparedFiniteRetainedPublication<Payload, Payload, Payload, u8, u8>;

const FALLBACK_UNITS: u64 = 4;
const REQUESTED_CAUSE: PreparationFailureCode = PreparationFailureCode::domain(71);

#[derive(Clone, Copy)]
enum Rail {
    EvidenceOnly,
    Minimal,
}

/// The fixture owns the same original slots lent to preparation and concrete extraction.
struct Root {
    terminal: Option<Physical>,
    assembly: crate::retained::PublicationAssemblySlot<Owner, u8, u8, Payload, Payload>,
    baseline: Option<Baseline>,
    metadata: RetainedOperationMetadata<u8, u8, Payload, Payload>,
    common: Option<Invocation>,
    contained: bool,
}

impl Root {
    fn new(account: &PhysicalStorageAccount, drops: &Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self::with_failure_work(account, drops, false)
    }

    fn with_failure_work(
        account: &PhysicalStorageAccount,
        drops: &Arc<Mutex<Vec<&'static str>>>,
        protected: bool,
    ) -> Self {
        let failure_plan =
            protected.then(|| RetainedFailureTerminalWorkPlan::with_evidence_work(5, 1, 0, 0));
        let requirement = failure_plan.map(|plan| plan.requirement().unwrap());
        let policies = [1_u8, 2, 3, 4, 5, 6].map(|dimension| {
            let policy = if dimension == 5 && protected {
                let limit = requirement.as_ref().unwrap().observed_work_limit();
                DimensionPolicy::counter(limit, limit, 0)
            } else if dimension == 6 {
                DimensionPolicy::counter(u64::MAX, u64::MAX, u64::MAX)
                    .with_published_outcome_evidence_limit(u64::MAX)
            } else {
                DimensionPolicy::reservable_bytes(u64::MAX, u64::MAX, u64::MAX, u64::MAX)
            };
            (dimension, policy)
        });
        let mut common = Invocation::try_new(
            OperationControl::default(),
            FiniteLedgerLimits::try_new(u64::MAX, u64::MAX, policies).unwrap(),
            [3, 1, 2],
            CanonicalLineageAuthority::fresh(),
        )
        .unwrap();
        if let Some(plan) = failure_plan {
            common
                .terminal_ledger_mut()
                .view()
                .configure_protected_work(plan.dimension())
                .unwrap();
            common
                .terminal_ledger_mut()
                .as_read()
                .preflight_protected_work_attachment(
                    plan.dimension(),
                    &plan.share_work().charges(),
                    requirement.as_ref().unwrap().maximum_share_actions(),
                )
                .unwrap();
        }
        let allocation = RetainedAllocationPlan::new(0, 0).unwrap();
        let charge = account
            .checked_retained_charge::<Payload>(allocation)
            .unwrap();
        let parts = FiniteStartupLedgerPlanParts {
            canonical_assembly: AttributionBatchPlan::try_new(1, 1, 2, 1).unwrap(),
            evidence_assembly: AttributionBatchPlan::try_new(2, 2, 2, 1).unwrap(),
            fallback: AttributionBatchPlan::try_new(3, 3, 1, 1).unwrap(),
            fallback_attribution: StartupRetainedAttribution::new(3, 4, charge.bytes),
            account_shares: StartupAccountShareEnvelope::three_identical([(6, 1), (6, 2)]),
            fallback_commitment: OutputCommitment::new(6, FALLBACK_UNITS),
        };
        let plan = FiniteStartupCrossPlanePlan::try_new(parts, charge).unwrap();
        let ready = account
            .try_admit_operation_start()
            .unwrap()
            .try_prepare_finite_startup_cross_plane(&mut common.terminal_ledger_mut().view(), plan)
            .unwrap_or_else(|_| panic!("the original finite startup must cross both planes"));
        let baseline = ready
            .into_retained_guard::<Payload>(allocation)
            .try_build(|| {
                Ok(Payload {
                    label: "fallback",
                    drops: Arc::clone(drops),
                    drop_probe: None,
                })
            })
            .unwrap()
            .try_attach(common.coordinator.retained_ledger().as_read())
            .unwrap();
        assert!(baseline.account().is_same_account(account));
        assert_eq!(account.share_call_count(), 3);
        assert_eq!(
            baseline.with_batches(|batches| batches.as_read().numerical_evidence.is_none()),
            Some(true)
        );
        let raw = common.terminal_ledger_mut();
        let work_dimension = if protected { 6 } else { 5 };
        raw.view().charge_attempted(work_dimension, 3).unwrap();
        raw.view().charge_consumed(work_dimension, 2).unwrap();
        raw.view().try_complete_phase(1).unwrap();
        let writer = RetainedWriterPlan::new(1, 1, 1, RetainedBatchLimits::new(2, 1));
        Self {
            common: Some(common),
            baseline: Some(baseline),
            metadata: RetainedOperationMetadata {
                deferred_stage: None,
                terminal_writer: writer,
                canonical_assembly_limits: writer.limits,
                terminal_account_share_work: RetainedHandleSharePlan::new(6, 6),
                failure_terminal_work: failure_plan
                    .map(FailureTerminalWorkCapability::from_qualified),
                failure_terminal_work_dimension: failure_plan.map(|plan| plan.dimension()),
                failure_work_permit_outstanding: false,
                ordinary_work_permit_outstanding: false,
                ordinary_work_permit_partially_resolved: false,
            },
            terminal: None,
            assembly: None,
            contained: false,
        }
    }

    fn loan(
        &mut self,
    ) -> RetainedOperationLoan<'_, Owner, u8, Phase, Unit, u8, Payload, Payload, Payload> {
        RetainedOperationLoan {
            retained: RetainedOperationFieldsLoan {
                baseline: FallbackPublicationBaselineLoan::Finite(self.baseline.as_mut().unwrap()),
                metadata: &mut self.metadata,
            },
            terminal: TerminalPhysicalSlot::Finite(&mut self.terminal),
            assembly: Some(&mut self.assembly),
            common: InvocationLoan {
                common: InvocationSlot::Finite(&mut self.common),
                contained: &mut self.contained,
                not_send_or_sync: PhantomData,
            },
        }
    }

    fn probe(&self) -> LedgerTestProbe<u8, u8> {
        self.common
            .as_ref()
            .unwrap()
            .coordinator
            .retained_ledger()
            .test_shared_probe()
            .expect("the real startup batches have promoted their original finite ledger")
    }

    fn administration(&self) -> u64 {
        self.common
            .as_ref()
            .unwrap()
            .known_administrative_bytes()
            .unwrap()
    }

    fn prepare(&mut self, rail: Rail) -> DeferredAttachedTerminal {
        let prepared = match rail {
            Rail::EvidenceOnly => self
                .loan()
                .prepare_evidence_only_deferred(FinalProgress::new(Phase, Unit, 2)),
            Rail::Minimal => self
                .loan()
                .prepare_minimal_fallback_deferred(REQUESTED_CAUSE),
        };
        let prepared = prepared
            .unwrap_or_else(|_| panic!("the actual selected ordinary loan must reach Ready"));
        let common = self.common.as_ref().unwrap();
        let identity = common.coordinator.retained_ledger().as_read().identity();
        assert!(common.lifecycle.terminal_outstanding);
        assert!(
            common
                .lifecycle
                .has_attached_stamp(&prepared.stamp, identity)
        );
        assert!(prepared.qualification.is_finite());
        assert!(self.terminal.as_ref().unwrap().disposition.is_none());
        prepared
    }

    fn fields(
        &mut self,
    ) -> FiniteTerminalRootFields<'_, Owner, u8, Phase, Unit, u8, Payload, Payload, Payload> {
        FiniteTerminalRootFields {
            common: &mut self.common,
            baseline: self.baseline.as_mut().unwrap(),
            terminal: &mut self.terminal,
            assembly: self.assembly.as_mut(),
            contained: &mut self.contained,
        }
    }

    fn install(&mut self, prepared: DeferredAttachedTerminal) -> AttachedTerminalStamp {
        self.fields()
            .install_terminal_preparation(prepared)
            .unwrap_or_else(|_| panic!("the original finite Ready decision must install once"))
    }

    fn detach(&mut self, stamp: AttachedTerminalStamp) -> Commit {
        self.fields()
            .try_detach_terminal(stamp)
            .unwrap_or_else(|_| panic!("the actual finite root must accept its original stamp"))
            .unwrap_or_else(|_| panic!("the actual physical and numerical owners must detach"))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let outstanding = self
            .common
            .as_ref()
            .is_some_and(|common| common.lifecycle.terminal_outstanding);
        if self.contained || outstanding || self.terminal.is_some() {
            // Quarantine physical owners before retaining the original numerical owners.
            if let Some(physical) = self.terminal.take()
                && !physical.graph.is_empty()
            {
                let _ = physical.contain_new_fault(AccountPoisonReason::OwnershipGraphUnverified);
            }
            if let Some(fields) = self.assembly.as_mut() {
                crate::retained::terminal_physical::TerminalPhysicalSlots::<
                    u8,
                    u8,
                    Payload,
                    Payload,
                    Payload,
                > {
                    fallback: None,
                    evidence: None,
                    canonical_table: Some(&mut fields.canonical_table),
                    evidence_table: Some(&mut fields.evidence_table),
                    account: Some(&mut fields.account),
                }
                .quarantine_unresolved();
            }
            if let Some(mut baseline) = self.baseline.take() {
                baseline.record_forgotten_physical_edges();
                baseline.fail_closed_after_recorded_poison();
            }
            if let Some(fields) = self.assembly.as_mut() {
                fields.retain_batches_fail_closed();
            }
            std::mem::forget(self.common.take());
        } else {
            if self
                .assembly
                .as_ref()
                .is_some_and(|fields| !fields.is_empty())
            {
                let RetainedOperationLoan {
                    retained,
                    terminal,
                    common,
                    assembly,
                } = self.loan();
                let fields = assembly.unwrap().as_mut().unwrap();
                drop(RetainedPublicationAssemblyLoan {
                    fields: fields.into(),
                    operation: Some(RetainedOperationLoan {
                        retained,
                        terminal,
                        common,
                        assembly: None,
                    }),
                });
            }
            drop(self.baseline.take());
            if let Some(common) = self.common.as_mut() {
                common.view().close_terminal_owners();
            }
        }
    }
}

fn account() -> PhysicalStorageAccount {
    account_with_allocation_limit(1)
}

fn account_with_allocation_limit(allocations: u64) -> PhysicalStorageAccount {
    let factory = PhysicalStorageAccountFactory::try_new(
        PhysicalStorageAccountFactoryLimits::new(u64::MAX, 1).unwrap(),
    )
    .unwrap();
    factory
        .try_create_account(PhysicalStorageAccountLimits::new(u64::MAX, allocations).unwrap())
        .unwrap()
}

fn admitted_actions(account: &PhysicalStorageAccount) -> u64 {
    crate::retained::account::lock_recovering_poison(&account.inner.value().linear)
        .admitted_account_actions
}

fn numerical_cover(probe: &LedgerTestProbe<u8, u8>) -> (u64, u64, u64) {
    (
        probe.current_live_bytes(),
        probe.active_reservations(),
        probe.current_administrative_bytes(),
    )
}

fn physical_cover(account: &PhysicalStorageAccount) -> (u64, u64) {
    let snapshot = account.snapshot();
    (
        snapshot.live_retained_bytes(),
        snapshot.live_retained_allocations(),
    )
}

fn assert_fixed_facts(
    facts: &FiniteLedgerSnapshot<u8, u8>,
    administration: u64,
    fallback_units: u64,
) {
    assert_eq!(facts.dimensions().len(), 6);
    assert_eq!(facts.phases().len(), 3);
    assert_eq!(facts.last_completed_phase(), Some(1));
    assert_eq!(
        (facts.current_live_bytes(), facts.active_reservations()),
        (0, 0)
    );
    assert_eq!(facts.current_administrative_bytes(), administration);
    assert_eq!(facts.covered_administrative_bytes(), Some(administration));
    let work = facts.dimension(5).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (3, 2));
    assert_eq!(work.committed_canonical_output(), 0);
    assert_eq!(
        facts.dimension(6).unwrap().published_outcome_evidence(),
        fallback_units
    );
}

#[test]
fn selected_finite_ordinary_work_and_rebind_preserve_original_owners_through_terminal_commit() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new(&account, &drops);
    let original_common = std::ptr::from_ref(&root.common).cast();
    let original_metadata = std::ptr::from_ref(&root.metadata);
    let original_batches = root
        .baseline
        .as_ref()
        .unwrap()
        .with_batches(|batch| std::ptr::from_ref(batch).cast::<()>());
    let physical = physical_cover(&account);
    let shares = account.share_call_count();
    {
        let mut loan = root.loan();
        loan.assert_backend();
        assert_eq!(loan.invocation_identity(), original_common);
        assert_eq!(
            std::ptr::from_ref(&*loan.retained.metadata),
            original_metadata
        );
        assert_eq!(
            loan.retained.baseline.test_batch_identity(),
            original_batches
        );
        {
            let mut parts = loan.exclusive_parts().unwrap_or_else(|_| {
                panic!("the actual finite retained loan must admit ordinary work")
            });
            let work = parts.work.bind([5]).unwrap();
            parts.work.charge_observed(work, [1]).unwrap();
        }
        let writer = RetainedWriterPlan::new(1, 1, 1, RetainedBatchLimits::new(3, 1));
        loan.try_rebind_canonical_assembly(writer).unwrap();
        assert_eq!(
            loan.retained.metadata.canonical_assembly_limits,
            writer.limits
        );
        assert!(
            loan.retained
                .baseline
                .batch_read()
                .unwrap()
                .numerical_evidence
                .is_none()
        );
        assert_eq!(
            loan.retained.baseline.test_batch_identity(),
            original_batches
        );
    }
    assert_eq!(
        std::ptr::from_ref(&root.common).cast::<()>(),
        original_common
    );
    assert_eq!(std::ptr::from_ref(&root.metadata), original_metadata);
    assert_eq!(physical_cover(&account), physical);
    assert_eq!(account.share_call_count(), shares);
    let probe = root.probe();
    let numerical = numerical_cover(&probe);
    let [first, second, third, fourth]: [OperationAttributionReservation<
        Owner,
        u8,
        u8,
        CanonicalOutput,
    >; 4] = {
        let (work, mut storage) = root
            .common
            .as_mut()
            .unwrap()
            .terminal_ledger_mut()
            .view()
            .into_exclusive()
            .unwrap();
        [(); 4].map(|()| {
            work.reserve_operation_attribution(&mut storage, 1, 1, 0)
                .unwrap()
        })
    };
    assert_eq!(
        numerical_cover(&probe),
        (numerical.0, numerical.1 + 4, numerical.2)
    );
    {
        let batches = root.baseline.as_mut().unwrap().terminal_batches().unwrap();
        let batch = batches.canonical_assembly;
        assert_eq!(batch.attribution_len(), 0);
        assert!(batch.try_push_attribution(first).is_ok());
        assert!(batch.try_push_attribution(second).is_ok());
        assert!(batch.try_push_attribution(third).is_ok());
        assert_eq!(batch.attribution_len(), 3);
        let fourth = match batch.try_push_attribution(fourth) {
            Err(fourth) => fourth,
            Ok(()) => panic!("the actual rebound batch must return the fourth original token"),
        };
        assert_eq!(fourth.bytes(), 0);
        drop(fourth);
        assert_eq!(
            numerical_cover(&probe),
            (numerical.0, numerical.1 + 3, numerical.2)
        );
        // These zero-byte reservations never acquired physical storage.
        for _ in 0..3 {
            assert!(batch.finish_last_attribution_after_storage_rollback(true));
        }
        assert_eq!(batch.attribution_len(), 0);
    }
    assert_eq!(numerical_cover(&probe), numerical);
    assert_eq!(physical_cover(&account), physical);
    assert_eq!(account.share_call_count(), shares);
    let administration = root.administration();
    let prepared = root.prepare(Rail::Minimal);
    let stamp = root.install(prepared);
    let prepared = root.detach(stamp).try_into_prepared().unwrap();
    let work = prepared.facts.dimension(5).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (4, 3));
    assert_eq!(
        prepared
            .facts
            .dimension(6)
            .unwrap()
            .published_outcome_evidence(),
        FALLBACK_UNITS
    );
    assert_eq!(
        prepared.facts.covered_administrative_bytes(),
        Some(administration)
    );
    drop(root);
    assert!(!account.snapshot().poisoned());
    drop(prepared);
    assert_eq!(*drops.lock().unwrap(), ["fallback"]);
    assert_eq!(physical_cover(&account), (0, 0));
}

#[test]
fn dropping_untouched_finite_root_destroys_fallback_and_releases_original_cover() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let root = Root::new(&account, &drops);
    let probe = root.probe();
    let numerical = numerical_cover(&probe);
    assert!(numerical.0 > 0 && numerical.1 > 0 && numerical.2 > 0);
    assert!(physical_cover(&account).0 > 0);
    assert!(!root.common.as_ref().unwrap().lifecycle.terminal_outstanding);
    assert!(root.terminal.is_none() && root.assembly.is_none());
    drop(root);
    assert_eq!(*drops.lock().unwrap(), ["fallback"]);
    assert_eq!(physical_cover(&account), (0, 0));
    assert!(!probe.is_available());
    assert_eq!(numerical_cover(&probe), (0, 0, 0));
    assert!(!account.snapshot().poisoned());
}

#[test]
fn forgotten_finite_assembly_drops_members_before_cover_and_populated_ready_retains_owners() {
    for (ready, poisoned) in [(false, false), (false, true), (true, false), (true, true)] {
        let account = account_with_allocation_limit(128);
        let drops = Arc::new(Mutex::new(Vec::new()));
        let observations = Arc::new(Mutex::new(Vec::new()));
        let mut root = Root::with_failure_work(&account, &drops, true);
        let probe = root.probe();
        let writer = RetainedWriterPlan::new(2, 2, 2, RetainedBatchLimits::new(2, 1));
        let table = RetainedBlockTablePlan::new(writer, 1, 1);
        let mut assembly = root
            .loan()
            .try_publication_assembly(RetainedPublicationPlan::success(table, table))
            .unwrap_or_else(|error| panic!("the real finite tables must open: {:?}", error.cause));
        let canonical_probe = assembly.test_ledger_probe();
        let canonical: CanonicalRetainedSlot<Payload, ()> = assembly
            .try_retain_canonical(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
                Ok(emplacer.place(
                    Payload {
                        label: "canonical",
                        drops: Arc::clone(&drops),
                        drop_probe: Some(PayloadDropProbe {
                            probe: canonical_probe,
                            observations: Arc::clone(&observations),
                        }),
                    },
                    (),
                ))
            })
            .unwrap_or_else(|_| panic!("the original canonical member must retain"));
        let evidence_probe = assembly.test_ledger_probe();
        let evidence: OutcomeEvidenceRetainedSlot<Payload, ()> = assembly
            .try_retain_outcome_evidence(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
                Ok(emplacer.place(
                    Payload {
                        label: "evidence",
                        drops: Arc::clone(&drops),
                        drop_probe: Some(PayloadDropProbe {
                            probe: evidence_probe,
                            observations: Arc::clone(&observations),
                        }),
                    },
                    (),
                ))
            })
            .unwrap_or_else(|_| panic!("the original evidence member must retain"));
        assert_eq!(canonical.index_in_set(), 0);
        assert_eq!(evidence.identity().index, 0);
        assert!(assembly.test_storage_graph_is_exact());
        if ready {
            assembly.try_commit_canonical(6, 1).unwrap();
            assembly.try_commit_outcome_evidence(6, 1).unwrap();
            let RetainedClosureValidation::Valid(proof) =
                assembly.validate_success(canonical, evidence, |session, canonical, evidence| {
                    session.canonical(canonical).is_some()
                        && session.outcome_evidence(evidence).is_some()
                })
            else {
                panic!("the populated finite assembly must validate before abandonment");
            };
            let (prepared, control) = assembly
                .try_classify_terminal_preparation(RetainedTerminalRequest::Success {
                    proof,
                    final_progress: FinalProgress::new(Phase, Unit, 2),
                })
                .unwrap_or_else(|_| panic!("the populated finite assembly must classify"))
                .finish_deferred()
                .unwrap_or_else(|_| panic!("the populated finite assembly must reach Ready"));
            assert_eq!(control, None);
            let _stamp = root.install(prepared);
            assert_eq!(*drops.lock().unwrap(), ["fallback"]);
        } else {
            std::mem::forget(assembly);
            assert!(drops.lock().unwrap().is_empty());
        }
        let numerical = numerical_cover(&probe);
        let physical = physical_cover(&account);
        assert!(numerical.0 > 0 && numerical.1 > 0 && numerical.2 > 0);
        assert!(physical.0 > 0 && physical.1 > 1);
        let first_poison = if poisoned {
            let reason = AccountPoisonReason::RetainedStorageReleaseUnverified;
            let _ = account.record_poison(reason);
            let first = account.snapshot().first_poison_evidence().unwrap();
            assert_eq!(first.reason(), reason);
            Some(first)
        } else {
            None
        };
        assert_eq!(
            root.common.as_ref().unwrap().lifecycle.terminal_outstanding,
            ready
        );
        assert_eq!(root.terminal.is_some(), ready);
        drop(root);
        if ready {
            assert_eq!(*drops.lock().unwrap(), ["fallback"]);
            assert!(observations.lock().unwrap().is_empty());
            assert_eq!(physical_cover(&account), physical);
            assert!(probe.is_available());
            assert_eq!(numerical_cover(&probe), numerical);
            assert!(account.snapshot().poisoned());
            if poisoned {
                assert_eq!(account.snapshot().first_poison_evidence(), first_poison);
            } else {
                assert_eq!(
                    account.snapshot().first_poison_evidence().unwrap().reason(),
                    AccountPoisonReason::OwnershipGraphUnverified
                );
            }
        } else {
            assert_eq!(
                *drops.lock().unwrap(),
                ["canonical", "evidence", "fallback"]
            );
            assert_eq!(
                *observations.lock().unwrap(),
                [
                    PayloadDropObservation {
                        label: "canonical",
                        probe_available: true,
                        numerical_cover: numerical,
                    },
                    PayloadDropObservation {
                        label: "evidence",
                        probe_available: true,
                        numerical_cover: numerical,
                    },
                ]
            );
            assert_eq!(physical_cover(&account), (0, 0));
            assert!(!probe.is_available());
            assert_eq!(numerical_cover(&probe), (0, 0, 0));
            assert_eq!(account.snapshot().poisoned(), poisoned);
            assert_eq!(account.snapshot().first_poison_evidence(), first_poison);
        }
    }
}

#[test]
fn actual_finite_changed_and_rich_tables_commit_nonzero_primary_facts_without_dynamic_evidence() {
    for rich in [false, true] {
        let account = account_with_allocation_limit(128);
        let drops = Arc::new(Mutex::new(Vec::new()));
        let member_drops = Arc::new(Mutex::new(Vec::new()));
        let mut root = Root::with_failure_work(&account, &drops, true);
        let probe = root.probe();
        let (prepared, canonical, evidence) = {
            let writer = RetainedWriterPlan::new(2, 2, 2, RetainedBatchLimits::new(2, 1));
            let table = RetainedBlockTablePlan::new(writer, 1, 1);
            let mut assembly = root
                .loan()
                .try_publication_assembly(RetainedPublicationPlan::success(table, table))
                .unwrap_or_else(|error| {
                    panic!(
                        "the actual finite ordinary loan must open real publication tables: {:?}",
                        error.cause
                    )
                });
            let canonical: CanonicalRetainedSlot<Payload, ()> = assembly
                .try_retain_canonical(RetainedAllocationPlan::new(0, 0).unwrap(), |emplacer| {
                    Ok(emplacer.place(
                        Payload {
                            label: "canonical",
                            drops: Arc::clone(&member_drops),
                            drop_probe: None,
                        },
                        (),
                    ))
                })
                .unwrap_or_else(|_| panic!("the actual canonical allocation must retain"));
            let evidence: OutcomeEvidenceRetainedSlot<Payload, ()> = assembly
                .try_retain_outcome_evidence(
                    RetainedAllocationPlan::new(0, 0).unwrap(),
                    |emplacer| {
                        Ok(emplacer.place(
                            Payload {
                                label: "evidence",
                                drops: Arc::clone(&member_drops),
                                drop_probe: None,
                            },
                            (),
                        ))
                    },
                )
                .unwrap_or_else(|_| panic!("the actual evidence allocation must retain"));
            assembly.try_commit_canonical(6, 1).unwrap();
            assembly.try_commit_outcome_evidence(6, 1).unwrap();
            let request = if rich {
                let RetainedClosureValidation::Valid(proof) = assembly
                    .validate_rich_failure(evidence, |session, evidence| {
                        session.outcome_evidence(evidence).is_some()
                    })
                else {
                    panic!(
                        "the actual finite evidence closure must validate under original protected work"
                    );
                };
                RetainedTerminalRequest::RichFailure {
                    proof,
                    requested_cause: REQUESTED_CAUSE,
                    final_progress: FinalProgress::new(Phase, Unit, 2),
                }
            } else {
                let RetainedClosureValidation::Valid(proof) = assembly.validate_success(
                    canonical,
                    evidence,
                    |session, canonical, evidence| {
                        session.canonical(canonical).is_some()
                            && session.outcome_evidence(evidence).is_some()
                    },
                ) else {
                    panic!("the actual finite success closure must validate");
                };
                RetainedTerminalRequest::Success {
                    proof,
                    final_progress: FinalProgress::new(Phase, Unit, 2),
                }
            };
            let (prepared, control) = assembly
                .try_classify_terminal_preparation(request)
                .unwrap_or_else(|_| panic!("the actual finite tables must classify"))
                .finish_deferred()
                .unwrap_or_else(|_| panic!("the actual finite tables must reach Ready"));
            assert_eq!(control, None);
            (prepared, canonical, evidence)
        };
        assert_eq!(*drops.lock().unwrap(), ["fallback"]);
        assert!(probe.is_available() && probe.is_terminal_qualified());
        assert!(
            root.terminal
                .as_ref()
                .unwrap()
                .graph
                .account
                .as_ref()
                .unwrap()
                .is_same_account(&account)
        );
        let shares = account.share_call_count();
        let actions = admitted_actions(&account);
        let administration = root.administration();
        let stamp = root.install(prepared);
        let prepared = root.detach(stamp).try_into_prepared().unwrap();
        assert_eq!(prepared.facts.current_live_bytes(), 0);
        assert_eq!(prepared.facts.active_reservations(), 0);
        assert_eq!(
            prepared.facts.covered_administrative_bytes(),
            Some(administration)
        );
        let output = prepared.facts.dimension(6).unwrap();
        assert_eq!(output.committed_canonical_output(), u64::from(!rich));
        assert_eq!(output.published_outcome_evidence(), 1);
        match &prepared.payload {
            FiniteRetainedPublicationPayload::Publication(
                RetainedPublicationPayload::Success {
                    canonical: published,
                    outcome_evidence,
                    canonical_root,
                    outcome_evidence_root,
                    account: output_account,
                },
            ) if !rich => {
                assert_eq!(
                    published.get(canonical).map(|payload| payload.label),
                    Some("canonical")
                );
                assert_eq!(
                    outcome_evidence.get(evidence).map(|payload| payload.label),
                    Some("evidence")
                );
                assert_eq!(*canonical_root, canonical.identity());
                assert_eq!(*outcome_evidence_root, evidence.identity());
                assert!(output_account.is_same_account(&account));
                assert_eq!(prepared.cause, TerminalEvidenceCause::Success);
            }
            FiniteRetainedPublicationPayload::Publication(
                RetainedPublicationPayload::RichFailure {
                    outcome_evidence,
                    outcome_evidence_root,
                    account: output_account,
                },
            ) if rich => {
                assert_eq!(
                    outcome_evidence.get(evidence).map(|payload| payload.label),
                    Some("evidence")
                );
                assert_eq!(*outcome_evidence_root, evidence.identity());
                assert!(output_account.is_same_account(&account));
                assert_eq!(
                    prepared.cause,
                    TerminalEvidenceCause::Failure(REQUESTED_CAUSE)
                );
            }
            _ => panic!("the finite output must retain its exact qualified rail"),
        }
        assert!(!probe.is_available());
        assert_eq!(
            (account.share_call_count(), admitted_actions(&account)),
            (shares, actions)
        );
        drop(root);
        assert!(!account.snapshot().poisoned());
        let primary = *output;
        let output = prepared.into_output();
        assert_eq!(output.facts.dimension(6), Some(&primary));
        match &output.payload {
            FiniteRetainedOutputPayload::Success {
                canonical,
                outcome_evidence,
            } if !rich => {
                assert_eq!(canonical.root().label, "canonical");
                assert_eq!(outcome_evidence.root().label, "evidence");
                assert!(canonical.account().is_same_account(&account));
            }
            FiniteRetainedOutputPayload::RichFailure(evidence) if rich => {
                assert_eq!(evidence.outcome_evidence().root().label, "evidence");
                assert!(evidence.account().is_same_account(&account));
            }
            _ => panic!("pure handoff must retain its original qualified primary rail"),
        }
        assert_eq!(
            (account.share_call_count(), admitted_actions(&account)),
            (shares, actions)
        );
        drop(output);
        assert_eq!(*member_drops.lock().unwrap(), ["canonical", "evidence"]);
        assert_eq!(physical_cover(&account), (0, 0));
        assert!(!account.snapshot().poisoned());
    }
}

#[test]
fn evidence_only_ready_discards_real_fallback_but_keeps_original_independent_account_through_commit()
 {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new(&account, &drops);
    let probe = root.probe();
    let administration = root.administration();
    let shares = account.share_call_count();
    let actions = admitted_actions(&account);
    assert!(physical_cover(&account).0 > 0);
    let prepared = root.prepare(Rail::EvidenceOnly);
    assert_eq!(*drops.lock().unwrap(), ["fallback"]);
    assert_eq!(physical_cover(&account), (0, 0));
    assert!(
        root.terminal
            .as_ref()
            .unwrap()
            .graph
            .account
            .as_ref()
            .unwrap()
            .is_same_account(&account)
    );
    assert!(probe.is_available() && probe.is_terminal_qualified());
    let stamp = root.install(prepared);
    let prepared = root.detach(stamp).try_into_prepared().unwrap();
    assert!(matches!(
        &prepared.payload,
        FiniteRetainedPublicationPayload::EvidenceOnlySuccess
    ));
    assert_eq!(prepared.cause, TerminalEvidenceCause::Success);
    assert!(
        prepared
            .terminal_account
            .as_ref()
            .unwrap()
            .is_same_account(&account)
    );
    assert_fixed_facts(&prepared.facts, administration, 0);
    assert!(root.common.is_none() && root.terminal.is_none());
    assert!(!probe.is_available());
    drop(root);
    assert!(!account.snapshot().poisoned());
    assert_eq!(
        (account.share_call_count(), admitted_actions(&account)),
        (shares, actions)
    );
    let output = prepared.into_output();
    assert_fixed_facts(&output.facts, administration, 0);
    let FiniteRetainedOutputPayload::EvidenceOnlySuccess { terminal_account } = &output.payload
    else {
        panic!("pure handoff must retain its original evidence-only rail");
    };
    assert!(terminal_account.is_same_account(&account));
    assert_eq!(output.cause, TerminalEvidenceCause::Success);
    assert_eq!(
        (account.share_call_count(), admitted_actions(&account)),
        (shares, actions)
    );
    drop(output);
    assert_eq!(*drops.lock().unwrap(), ["fallback"]);
    assert_eq!(physical_cover(&account), (0, 0));
    assert!(!account.snapshot().poisoned());
}

#[test]
fn minimal_ready_commits_nonzero_fallback_and_moves_same_payload_and_independent_account() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new(&account, &drops);
    let payload = std::ptr::from_ref(
        root.baseline
            .as_ref()
            .unwrap()
            .test_fallback_allocation()
            .as_ref(),
    );
    let probe = root.probe();
    let administration = root.administration();
    let physical = physical_cover(&account);
    let shares = account.share_call_count();
    let actions = admitted_actions(&account);
    let prepared = root.prepare(Rail::Minimal);
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(physical_cover(&account), physical);
    assert!(probe.is_available() && probe.is_terminal_qualified());
    let cover = numerical_cover(&probe);
    assert!(cover.0 > 0 && cover.1 > 0 && cover.2 > 0);
    let stamp = root.install(prepared);
    let guard = root.detach(stamp);
    assert!(root.common.is_none() && root.terminal.is_none());
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(physical_cover(&account), physical);
    let prepared = guard.try_into_prepared().unwrap();
    let FiniteRetainedPublicationPayload::Publication(
        RetainedPublicationPayload::MinimalFallback { part },
    ) = &prepared.payload
    else {
        panic!("the qualified minimal graph must publish its original fallback");
    };
    assert_eq!(std::ptr::from_ref(part.allocation().as_ref()), payload);
    assert!(part.account().is_same_account(&account));
    assert!(
        prepared
            .terminal_account
            .as_ref()
            .unwrap()
            .is_same_account(&account)
    );
    assert_eq!(
        prepared.cause,
        TerminalEvidenceCause::Failure(REQUESTED_CAUSE)
    );
    assert_fixed_facts(&prepared.facts, administration, FALLBACK_UNITS);
    assert!(!probe.is_available());
    drop(root);
    assert_eq!(
        (account.share_call_count(), admitted_actions(&account)),
        (shares, actions)
    );
    assert_eq!(physical_cover(&account), physical);
    assert!(!account.snapshot().poisoned());
    let output = prepared.into_output();
    assert_fixed_facts(&output.facts, administration, FALLBACK_UNITS);
    let FiniteRetainedOutputPayload::MinimalFallback {
        fallback,
        terminal_account,
    } = &output.payload
    else {
        panic!("pure handoff must retain its original minimal rail");
    };
    assert_eq!(std::ptr::from_ref(fallback.value()), payload);
    assert!(fallback.account().is_same_account(&account));
    assert!(terminal_account.is_same_account(&account));
    assert_eq!(
        output.cause,
        TerminalEvidenceCause::Failure(REQUESTED_CAUSE)
    );
    assert_eq!(
        (account.share_call_count(), admitted_actions(&account)),
        (shares, actions)
    );
    drop(output);
    assert_eq!(*drops.lock().unwrap(), ["fallback"]);
    assert_eq!(physical_cover(&account), (0, 0));
    assert!(!account.snapshot().poisoned());
}

#[test]
fn poison_after_ready_or_after_detach_rejects_facts_and_retains_original_fallback_charge() {
    for poison_before_detach in [true, false] {
        let account = account();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let mut root = Root::new(&account, &drops);
        let prepared = root.prepare(Rail::Minimal);
        let stamp = root.install(prepared);
        let probe = root.probe();
        let numerical = numerical_cover(&probe);
        let administration = root.administration();
        let physical = physical_cover(&account);
        let shares = account.share_call_count();
        let actions = admitted_actions(&account);
        let reason = AccountPoisonReason::RetainedStorageReleaseUnverified;
        if poison_before_detach {
            let _ = account.record_poison(reason);
        }
        let guard = root.detach(stamp);
        if !poison_before_detach {
            let _ = account.record_poison(reason);
        }
        let result = guard.try_into_prepared();
        assert!(
            result.is_err(),
            "poisoned eligibility must not return finite facts or payload"
        );
        let mut error = result.err().unwrap();
        assert_eq!(error.cause.reason(), reason);
        let cover = error.proof.test_cover_observation().unwrap();
        assert_eq!((cover.0, cover.1), (numerical.0, numerical.1));
        // Exclusive recovery retires the original shared mutex shell before returning its cover.
        let retired_shell = crate::ledger::finite::FiniteDetachedTypedFreezeProof::<Owner, u8, u8>::test_shared_shell_bytes();
        assert_eq!(cover.2.checked_add(retired_shell), Some(numerical.2));
        assert_fixed_facts(
            error.proof.terminal_projection(),
            administration,
            FALLBACK_UNITS,
        );
        let projection = *error.proof.terminal_projection();
        assert!(root.contained && root.common.is_none() && root.terminal.is_none());
        assert!(!probe.is_available());
        assert!(drops.lock().unwrap().is_empty());
        assert_eq!(physical_cover(&account), physical);
        drop(root);
        let mut moved_error = error;
        assert_eq!(moved_error.cause.reason(), reason);
        assert_eq!(moved_error.proof.test_cover_observation(), Some(cover));
        assert_eq!(moved_error.proof.terminal_projection(), &projection);
        let mut proof = moved_error.proof;
        assert_eq!(proof.test_cover_observation(), Some(cover));
        assert_eq!(proof.terminal_projection(), &projection);
        assert!(drops.lock().unwrap().is_empty());
        assert_eq!(physical_cover(&account), physical);
        assert_eq!(
            (account.share_call_count(), admitted_actions(&account)),
            (shares, actions)
        );
        assert!(account.snapshot().poisoned());
        assert_eq!(
            account.snapshot().first_poison_evidence().unwrap().reason(),
            reason
        );
        drop(proof);
        assert!(drops.lock().unwrap().is_empty());
        assert_eq!(physical_cover(&account), physical);
        assert_eq!(
            (account.share_call_count(), admitted_actions(&account)),
            (shares, actions)
        );
        assert_eq!(
            account.snapshot().first_poison_evidence().unwrap().reason(),
            reason
        );
    }
}

#[test]
fn invalid_finite_ready_shape_returns_original_owned_commit_proof_after_physical_containment() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new(&account, &drops);
    let prepared = root.prepare(Rail::Minimal);
    let stamp = root.install(prepared);
    let administration = root.administration();
    let probe = root.probe();
    let numerical = numerical_cover(&probe);
    let physical = physical_cover(&account);
    let actions = admitted_actions(&account);
    let shares = account.share_call_count();
    // Corrupt only the installed physical disposition after genuine minimal-rail qualification.
    // Evidence-only success cannot own the still-live original fallback.
    root.terminal.as_mut().unwrap().disposition = Some(TerminalDisposition::EvidenceOnlySuccess);
    let result = root.detach(stamp).try_into_prepared();
    let mut error = result
        .err()
        .expect("an invalid physical Ready shape must not publish finite facts");
    assert_eq!(
        error.cause.reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    let cover = error.proof.test_cover_observation().unwrap();
    assert_eq!((cover.0, cover.1), (numerical.0, numerical.1));
    let retired_shell = crate::ledger::finite::FiniteDetachedTypedFreezeProof::<Owner, u8, u8>::test_shared_shell_bytes();
    assert_eq!(cover.2.checked_add(retired_shell), Some(numerical.2));
    assert_fixed_facts(
        error.proof.terminal_projection(),
        administration,
        FALLBACK_UNITS,
    );
    let projection = *error.proof.terminal_projection();
    assert!(root.contained && root.common.is_none() && root.terminal.is_none());
    assert!(!probe.is_available());
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(physical_cover(&account), physical);
    drop(root);
    let mut proof = error.proof;
    assert_eq!(proof.test_cover_observation(), Some(cover));
    assert_eq!(proof.terminal_projection(), &projection);
    assert_eq!(physical_cover(&account), physical);
    assert!(drops.lock().unwrap().is_empty());
    drop(proof);
    assert_eq!(physical_cover(&account), physical);
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(
        (account.share_call_count(), admitted_actions(&account)),
        (shares, actions)
    );
    assert_eq!(
        account.snapshot().first_poison_evidence().unwrap().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
}

#[test]
fn finite_root_extra_shared_owner_returns_original_owned_detach_residue_after_physical_containment()
{
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new(&account, &drops);
    let prepared = root.prepare(Rail::Minimal);
    let stamp = root.install(prepared);
    let probe = root.probe();
    let numerical = numerical_cover(&probe);
    let physical = physical_cover(&account);
    let shares = account.share_call_count();
    let actions = admitted_actions(&account);
    let original = root.common.as_ref().unwrap().coordinator.retained_ledger();
    let identity = original.as_read().identity();
    let work = original.as_read().dimension_evidence(5).unwrap();
    assert_eq!((work.attempted(), work.consumed()), (3, 2));
    let shared = original.test_shared_owner().unwrap();
    let error = root
        .fields()
        .try_detach_terminal(stamp)
        .unwrap_or_else(|_| panic!("the original Ready stamp must be accepted"))
        .err()
        .expect("the original extra Arc must refuse exclusive numerical extraction");
    assert!(root.contained && root.common.is_none() && root.terminal.is_none());
    assert!(account.snapshot().poisoned());
    assert_eq!(
        error.cause.reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
    assert_eq!(
        error.residue.kind(),
        TypedFreezeDetachErrorKind::SharedLedgerOwner
    );
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(physical_cover(&account), physical);
    assert!(probe.is_available());
    assert_eq!(numerical_cover(&probe), numerical);
    let retained = error
        .residue
        .test_invocation()
        .coordinator
        .retained_ledger();
    assert_eq!(retained.as_read().identity(), identity);
    assert_eq!(retained.as_read().dimension_evidence(5).unwrap(), work);
    let retained_shared = retained.test_shared_owner().unwrap();
    assert!(Arc::ptr_eq(&shared, &retained_shared));
    drop(retained_shared);
    drop(root);
    let moved_error = error;
    let retained = moved_error
        .residue
        .test_invocation()
        .coordinator
        .retained_ledger();
    assert_eq!(retained.as_read().identity(), identity);
    assert_eq!(retained.as_read().dimension_evidence(5).unwrap(), work);
    assert_eq!(numerical_cover(&probe), numerical);
    assert_eq!(physical_cover(&account), physical);
    drop(moved_error);
    assert!(probe.is_available());
    assert_eq!(numerical_cover(&probe), numerical);
    assert_eq!(physical_cover(&account), physical);
    drop(shared);
    assert!(probe.is_available());
    assert_eq!(numerical_cover(&probe), numerical);
    assert_eq!(physical_cover(&account), physical);
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(
        (account.share_call_count(), admitted_actions(&account)),
        (shares, actions)
    );
    assert_eq!(
        account.snapshot().first_poison_evidence().unwrap().reason(),
        AccountPoisonReason::OwnershipGraphUnverified
    );
}

#[test]
fn abandoning_final_guard_quarantines_original_account_even_after_fallback_was_discarded() {
    for rail in [Rail::EvidenceOnly, Rail::Minimal] {
        let account = account();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let mut root = Root::new(&account, &drops);
        let prepared = root.prepare(rail);
        let stamp = root.install(prepared);
        let guard = root.detach(stamp);
        let physical = physical_cover(&account);
        let dropped = drops.lock().unwrap().clone();
        let shares = account.share_call_count();
        let actions = admitted_actions(&account);
        drop(guard);
        drop(root);
        assert_eq!(*drops.lock().unwrap(), dropped);
        assert_eq!(physical_cover(&account), physical);
        assert_eq!(
            (account.share_call_count(), admitted_actions(&account)),
            (shares, actions)
        );
        assert!(account.snapshot().poisoned());
        assert_eq!(
            account.snapshot().first_poison_evidence().unwrap().reason(),
            AccountPoisonReason::OwnershipGraphUnverified
        );
    }
}

#[test]
fn foreign_preparation_and_stamp_are_returned_without_taking_or_poisoning_actual_owners() {
    let first_account = account();
    let second_account = account();
    let first_drops = Arc::new(Mutex::new(Vec::new()));
    let second_drops = Arc::new(Mutex::new(Vec::new()));
    let mut first = Root::new(&first_account, &first_drops);
    let mut second = Root::new(&second_account, &second_drops);
    let first_prepared = first.prepare(Rail::Minimal);
    let second_prepared = second.prepare(Rail::Minimal);
    let first_probe = first.probe();
    let second_probe = second.probe();
    let first_cover = numerical_cover(&first_probe);
    let second_cover = numerical_cover(&second_probe);
    let first_physical = physical_cover(&first_account);
    let second_physical = physical_cover(&second_account);
    let first_batch = first
        .baseline
        .as_ref()
        .unwrap()
        .with_batches(std::ptr::from_ref);
    let second_batch = second
        .baseline
        .as_ref()
        .unwrap()
        .with_batches(std::ptr::from_ref);
    let first_prepared = second
        .fields()
        .install_terminal_preparation(first_prepared)
        .err()
        .expect("a foreign actual finite preparation must be returned unchanged");
    assert!(first.terminal.as_ref().unwrap().disposition.is_none());
    assert!(second.terminal.as_ref().unwrap().disposition.is_none());
    let first_stamp = first.install(first_prepared);
    let second_stamp = second.install(second_prepared);
    let second_stamp = first
        .fields()
        .try_detach_terminal(second_stamp)
        .err()
        .expect("a foreign actual finite stamp must be returned before extraction");
    assert!(first.common.is_some() && second.common.is_some());
    assert!(!first.contained && !second.contained);
    assert_eq!(numerical_cover(&first_probe), first_cover);
    assert_eq!(numerical_cover(&second_probe), second_cover);
    assert_eq!(
        first
            .baseline
            .as_ref()
            .unwrap()
            .with_batches(std::ptr::from_ref),
        first_batch
    );
    assert_eq!(
        second
            .baseline
            .as_ref()
            .unwrap()
            .with_batches(std::ptr::from_ref),
        second_batch
    );
    assert_eq!(physical_cover(&first_account), first_physical);
    assert_eq!(physical_cover(&second_account), second_physical);
    assert!(!first_account.snapshot().poisoned() && !second_account.snapshot().poisoned());
    assert!(first_drops.lock().unwrap().is_empty() && second_drops.lock().unwrap().is_empty());
    let first = first.detach(first_stamp).try_into_prepared().unwrap();
    let second = second.detach(second_stamp).try_into_prepared().unwrap();
    assert_eq!(
        first
            .facts
            .dimension(6)
            .unwrap()
            .published_outcome_evidence(),
        FALLBACK_UNITS
    );
    assert_eq!(
        second
            .facts
            .dimension(6)
            .unwrap()
            .published_outcome_evidence(),
        FALLBACK_UNITS
    );
    drop(first);
    drop(second);
    assert_eq!(*first_drops.lock().unwrap(), ["fallback"]);
    assert_eq!(*second_drops.lock().unwrap(), ["fallback"]);
    assert_eq!(physical_cover(&first_account), (0, 0));
    assert_eq!(physical_cover(&second_account), (0, 0));
    assert!(!first_account.snapshot().poisoned() && !second_account.snapshot().poisoned());
}

#[test]
fn abandoning_ready_quarantines_physical_graph_before_retaining_actual_startup_batches() {
    let account = account();
    let drops = Arc::new(Mutex::new(Vec::new()));
    let mut root = Root::new(&account, &drops);
    let prepared = root.prepare(Rail::Minimal);
    let _stamp = root.install(prepared);
    let probe = root.probe();
    let numerical = numerical_cover(&probe);
    let physical = physical_cover(&account);
    drop(root);
    assert!(account.snapshot().poisoned());
    assert!(drops.lock().unwrap().is_empty());
    assert_eq!(physical_cover(&account), physical);
    assert!(probe.is_available());
    assert_eq!(numerical_cover(&probe), numerical);
}

#[test]
fn capture_and_partial_detach_unwind_quarantine_actual_fallback_before_retaining_original_cover() {
    for partial_detach in [false, true] {
        let account = account();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let mut root = Root::new(&account, &drops);
        let prepared = root.prepare(Rail::Minimal);
        let stamp = root.install(prepared);
        let probe = root.probe();
        let numerical = numerical_cover(&probe);
        let physical = physical_cover(&account);
        if partial_detach {
            root.common
                .as_mut()
                .unwrap()
                .test_panic_after_next_canonical_detach();
        } else {
            root.common
                .as_mut()
                .unwrap()
                .terminal_ledger_mut()
                .test_corrupt_storage_phase_position();
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            root.fields().try_detach_terminal(stamp)
        }));
        assert!(result.is_err());
        assert!(root.contained && root.common.is_none() && root.terminal.is_none());
        assert!(account.snapshot().poisoned());
        assert!(drops.lock().unwrap().is_empty());
        assert_eq!(physical_cover(&account), physical);
        assert!(probe.is_available());
        assert_eq!(numerical_cover(&probe), numerical);
        drop(root);
        assert!(drops.lock().unwrap().is_empty());
        assert_eq!(physical_cover(&account), physical);
        assert!(probe.is_available());
        assert_eq!(numerical_cover(&probe), numerical);
    }
}
