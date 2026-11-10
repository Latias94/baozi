//! Single-part retained construction reuses the existing side allocation/cleanup state machine.

use super::*;
use crate::ledger::FiniteFallbackBatchBaseline;
use crate::retained::ownership::FiniteFallbackPublicationBaseline;

/// Original dirty startup owners, detached from the entry account borrow.
///
/// Normal failure transfers this value to the outer invocation error. Only
/// abandonment runs the existing fail-closed containment destructors.
pub(crate) struct FiniteStartupRetainedResidue<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pending: Option<ContainedStartupRetainedSide<O, P, D>>,
    built: Option<RetainedPart<F, OutcomeEvidenceDestination, O, P, D>>,
    baseline: Option<FiniteFallbackPublicationBaseline<O, P, D, F>>,
    canonical_assembly: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    evidence_assembly: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    terminal_account: Option<PhysicalStorageAccount>,
}

impl<O, P, D, F> FiniteStartupRetainedResidue<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(in crate::retained) fn from_baseline(
        baseline: FiniteFallbackPublicationBaseline<O, P, D, F>,
    ) -> Self {
        Self {
            pending: None,
            built: None,
            baseline: Some(baseline),
            canonical_assembly: None,
            evidence_assembly: None,
            terminal_account: None,
        }
    }

    pub(crate) fn account(&self) -> Option<&PhysicalStorageAccount> {
        self.terminal_account
            .as_ref()
            .or_else(|| {
                self.baseline
                    .as_ref()
                    .and_then(|baseline| baseline.first_account())
            })
            .or_else(|| {
                self.pending
                    .as_ref()
                    .and_then(|pending| pending.account.as_ref())
            })
    }
}

impl<O, P, D, F> Drop for FiniteStartupRetainedResidue<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    fn drop(&mut self) {
        if let Some(mut baseline) = self.baseline.take() {
            baseline.record_forgotten_physical_edges();
            baseline.fail_closed_after_recorded_poison();
        }
        if let Some(mut built) = self.built.take() {
            built.contain_after_recorded_poison();
        }
        drop(self.pending.take());
        std::mem::forget(self.canonical_assembly.take());
        std::mem::forget(self.evidence_assembly.take());
        drop(self.terminal_account.take());
    }
}

pub(crate) struct FiniteStartupRetainedBuildFailure<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(crate) cause: RetainedWriterError<D>,
    pub(crate) residue: Option<FiniteStartupRetainedResidue<O, P, D, F>>,
}

type FiniteStartupRetainedBuildResult<O, P, D, F> = Result<
    FiniteFallbackPublicationBaseline<O, P, D, F>,
    FiniteStartupRetainedBuildFailure<O, P, D, F>,
>;

impl<O, P, D, F> fmt::Debug for FiniteStartupRetainedBuildFailure<O, P, D, F>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FiniteStartupRetainedBuildFailure")
            .field("cause", &self.cause)
            .field("retains_original_owners", &self.residue.is_some())
            .finish()
    }
}

pub(in crate::retained) struct FiniteStartupRetainedPartGuard<'account, O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
{
    account: &'account PhysicalStorageAccount,
    fallback: StartupRetainedSideState<'account, O, P, D, F>,
    terminal_account: Option<PhysicalStorageAccount>,
    canonical_assembly: Option<AttributionReservationBatch<O, P, D, CanonicalOutput>>,
    evidence_assembly: Option<AttributionReservationBatch<O, P, D, OutcomeEvidenceDestination>>,
    allocation_plan: RetainedAllocationPlan,
}

impl<'account, O, P, D, F> FiniteStartupRetainedPartGuard<'account, O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
{
    pub(in crate::retained) fn from_ready(
        batches: FiniteFallbackBatchBaseline<O, P, D>,
        fallback_account: PhysicalStorageAccount,
        pending: PendingRetainedAdmission<'account>,
        terminal_account: PhysicalStorageAccount,
        allocation_plan: RetainedAllocationPlan,
    ) -> Self {
        let account = pending.account;
        let (canonical_assembly, evidence_assembly, fallback_batch) = batches.into_parts();
        Self {
            account,
            fallback: StartupRetainedSideState::Pending {
                account: fallback_account,
                admission: pending,
                batch: fallback_batch,
            },
            terminal_account: Some(terminal_account),
            canonical_assembly: Some(canonical_assembly),
            evidence_assembly: Some(evidence_assembly),
            allocation_plan,
        }
    }

    fn retire_assembly_and_account(&mut self) {
        drop(self.canonical_assembly.take());
        drop(self.evidence_assembly.take());
        drop(self.terminal_account.take());
    }

    fn cleanup_pending(&mut self, fault: Option<AccountPoisonReason>) -> PendingRetainedRollback {
        self.fallback.drop_unarmed_before_charge_resolution();
        let rollback = if let Some(charge) = self.fallback.pending_admission().map(|a| a.charge) {
            let rollback = self.fallback.rollback_pending_side(fault);
            if matches!(
                rollback,
                PendingRetainedRollback::ChargeReleaseUnverified(_)
                    | PendingRetainedRollback::ChargeRetainedAfterPoison(_)
            ) {
                self.fallback.contain_pending_charge(charge);
            }
            rollback
        } else {
            PendingRetainedRollback::Clean
        };
        drop(std::mem::replace(
            &mut self.fallback,
            StartupRetainedSideState::Empty,
        ));
        self.retire_assembly_and_account();
        rollback
    }

    fn take_residue(&mut self) -> FiniteStartupRetainedResidue<O, P, D, F> {
        let (pending, built) = self.fallback.take_owned_failure();
        // These still-empty assembly buffers have no physical member duty. Their
        // actual backing dies before its numerical cover, as on the original path.
        drop(self.canonical_assembly.take());
        drop(self.evidence_assembly.take());
        FiniteStartupRetainedResidue {
            pending,
            built,
            baseline: None,
            canonical_assembly: None,
            evidence_assembly: None,
            terminal_account: self.terminal_account.take(),
        }
    }

    fn reject(
        mut self,
        failure: StartupRetainedSideFailure<D>,
    ) -> FiniteStartupRetainedBuildFailure<O, P, D, F> {
        self.fallback.drop_unarmed_before_charge_resolution();
        let rollback = if failure.retain_attribution {
            let poisoned = self.account.record_poison(
                failure
                    .fault
                    .unwrap_or(AccountPoisonReason::OwnershipGraphUnverified),
            );
            if let Some(charge) = self.fallback.pending_admission().map(|a| a.charge) {
                self.fallback.contain_pending_charge(charge);
            }
            PendingRetainedRollback::FaultRecorded(poisoned)
        } else {
            let rollback = self.fallback.rollback_pending_side(failure.fault);
            if matches!(
                rollback,
                PendingRetainedRollback::ChargeReleaseUnverified(_)
                    | PendingRetainedRollback::ChargeRetainedAfterPoison(_)
            ) && let Some(charge) = self.fallback.pending_admission().map(|a| a.charge)
            {
                self.fallback.contain_pending_charge(charge);
            }
            rollback
        };
        let retains_owners = failure.retain_attribution
            || matches!(
                rollback,
                PendingRetainedRollback::ChargeReleaseUnverified(_)
                    | PendingRetainedRollback::ChargeRetainedAfterPoison(_)
            );
        let residue = if retains_owners {
            Some(self.take_residue())
        } else {
            drop(std::mem::replace(
                &mut self.fallback,
                StartupRetainedSideState::Empty,
            ));
            self.retire_assembly_and_account();
            None
        };
        FiniteStartupRetainedBuildFailure {
            cause: startup_rejection_error(failure, rollback),
            residue,
        }
    }

    fn reject_uninvoked<Builder>(
        mut self,
        failure: StartupRetainedSideFailure<D>,
        builder: Builder,
    ) -> FiniteStartupRetainedBuildFailure<O, P, D, F> {
        self.fallback.drop_unarmed_before_charge_resolution();
        match dispose_uninvoked_startup_builders(builder, ()) {
            Ok(()) => self.reject(failure),
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                let cause = if failure.fault.is_some() {
                    RetainedWriterError::OwnershipUnverified
                } else {
                    RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked)
                };
                let mut rejected = self.reject(StartupRetainedSideFailure {
                    cause,
                    fault: Some(
                        failure
                            .fault
                            .unwrap_or(AccountPoisonReason::RetainedBuilderPanicked),
                    ),
                    retain_attribution: true,
                });
                if failure.fault.is_none() {
                    rejected.cause =
                        RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked);
                }
                rejected
            }
        }
    }

    pub(in crate::retained) fn try_build(
        mut self,
        build_fallback: impl FnOnce() -> Result<F, RetainedBuildFailureCode>,
    ) -> FiniteStartupRetainedBuildResult<O, P, D, F> {
        if let Err(failure) = self
            .fallback
            .try_preflight(StartupRetainedSide::Fallback, self.allocation_plan)
        {
            return Err(self.reject_uninvoked(failure, build_fallback));
        }
        if let Err(failure) = self.fallback.try_prepare(StartupRetainedSide::Fallback) {
            return Err(self.reject_uninvoked(failure, build_fallback));
        }
        let ready = self
            .fallback
            .take_ready()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let value = match catch_unwind(AssertUnwindSafe(build_fallback)) {
            Ok(Ok(value)) => value,
            Ok(Err(cause)) => {
                self.fallback = StartupRetainedSideState::Ready(ready);
                return Err(self.reject(StartupRetainedSideFailure::clean(
                    RetainedWriterError::Physical(RetainedAllocationError::Build(cause)),
                )));
            }
            Err(payload) => {
                self.fallback = StartupRetainedSideState::Ready(ready);
                discard_untrusted_panic_payload(payload);
                let mut rejected = self.reject(StartupRetainedSideFailure {
                    cause: RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked),
                    fault: Some(AccountPoisonReason::RetainedBuilderPanicked),
                    retain_attribution: true,
                });
                rejected.cause =
                    RetainedWriterError::Physical(RetainedAllocationError::BuilderPanicked);
                return Err(rejected);
            }
        };
        let fallback = ready.commit(value);
        let canonical = self
            .canonical_assembly
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let evidence = self
            .evidence_assembly
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        let account = self
            .terminal_account
            .take()
            .unwrap_or_else(|| unreachable_final_owner_state());
        FiniteFallbackPublicationBaseline::try_from_parts_owned(
            fallback, canonical, evidence, account,
        )
        .map_err(|(cause, fallback, canonical, evidence, account)| {
            FiniteStartupRetainedBuildFailure {
                cause,
                residue: Some(FiniteStartupRetainedResidue {
                    pending: None,
                    built: Some(fallback),
                    baseline: None,
                    canonical_assembly: Some(canonical),
                    evidence_assembly: Some(evidence),
                    terminal_account: Some(account),
                }),
            }
        })
    }
}

impl<O, P, D, F> Drop for FiniteStartupRetainedPartGuard<'_, O, P, D, F>
where
    O: OperationOwner,
    P: TerminalEvidenceKey,
    D: TerminalEvidenceKey + fmt::Debug,
    F: Send + Sync + 'static,
{
    fn drop(&mut self) {
        let _ = self.cleanup_pending(None);
    }
}
