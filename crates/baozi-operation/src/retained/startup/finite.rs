use super::{
    PendingOperationStartAdmission, ReadyStartupGraphSplitAuthority,
    ReadyStartupPhysicalSplitAuthority, StartupAccountOwnerRole, StartupCrossPlanePlanError,
    StartupPhysicalNoMutation, try_commit_startup_physical_charge, validate_startup_owner,
};
use crate::OperationOwner;
use crate::control::discard_untrusted_panic_payload;
use crate::ledger::{
    AttributionBatchCreateError, FiniteStartupAttributionGraph, FiniteStartupLedgerPlanParts,
    LedgerView,
};
use crate::retained::account::{AccountPoisonReason, AccountPoisoned, AdmissionCharge};
use crate::retained::allocation::{
    AccountPoisonEventWitness, RetainedAllocationPlan, record_account_poison_event,
};
use crate::retained::evidence::TerminalEvidenceKey;
use crate::retained::operation_start::{
    FiniteStartupRetainedPartGuard, RetainedWriterError, try_admit_finite_start_share_work,
};
use crate::retained::physical::{
    FiniteStartupAccountOwners, PendingRetainedAdmission, PhysicalStorageAccount,
};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Matching ledger and physical requests for the single real retained fallback.
#[must_use = "the finite startup plan must cross both accounting planes"]
pub(in crate::retained) struct FiniteStartupCrossPlanePlan<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    parts: FiniteStartupLedgerPlanParts<O, P, D>,
    charge: AdmissionCharge,
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord> FiniteStartupCrossPlanePlan<O, P, D> {
    pub(in crate::retained) fn try_new(
        parts: FiniteStartupLedgerPlanParts<O, P, D>,
        charge: AdmissionCharge,
    ) -> Result<Self, StartupCrossPlanePlanError> {
        let qualified = parts.fallback_attribution.bytes();
        if qualified != charge.bytes {
            return Err(StartupCrossPlanePlanError::FallbackByteMismatch {
                qualified,
                physical: charge.bytes,
            });
        }
        Ok(Self { parts, charge })
    }
}

struct FiniteStartupCrossPlaneOwnership<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    graph: FiniteStartupAttributionGraph<O, P, D>,
    owners: FiniteStartupAccountOwners,
    charge: AdmissionCharge,
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord> FiniteStartupCrossPlaneOwnership<O, P, D> {
    fn contain_after_recorded_fault(self) {
        let Self {
            graph,
            owners,
            charge: _,
        } = self;
        std::mem::forget(owners);
        std::mem::forget(graph);
    }
}

/// Both admitted planes, ready for the sole consuming retained construction handoff.
#[must_use = "finite Ready startup must enter retained construction or be contained"]
pub(in crate::retained) struct ReadyFiniteStartupCrossPlane<'account, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    account: &'account PhysicalStorageAccount,
    ownership: Option<FiniteStartupCrossPlaneOwnership<O, P, D>>,
}

impl<'account, O: OperationOwner, P: Copy + Ord, D: Copy + Ord>
    ReadyFiniteStartupCrossPlane<'account, O, P, D>
{
    /// Moves existing owners without allocating, sharing, locking, or admitting more work.
    pub(in crate::retained) fn into_retained_guard<F>(
        mut self,
        fallback_plan: RetainedAllocationPlan,
    ) -> FiniteStartupRetainedPartGuard<'account, O, P, D, F>
    where
        P: TerminalEvidenceKey,
        D: TerminalEvidenceKey + fmt::Debug,
        F: Send + Sync + 'static,
    {
        let FiniteStartupCrossPlaneOwnership {
            graph,
            owners,
            charge,
        } = self
            .ownership
            .take()
            .expect("finite Ready startup owns its complete graph");
        let batches = graph.into_runtime_baseline(ReadyStartupGraphSplitAuthority { private: () });
        let FiniteStartupAccountOwners {
            fallback_account,
            fallback_lease_owner,
            terminal_account,
        } = owners;
        let pending = PendingRetainedAdmission::from_precommitted_startup_single(
            self.account,
            charge,
            fallback_lease_owner,
            ReadyStartupPhysicalSplitAuthority { private: () },
        );
        FiniteStartupRetainedPartGuard::from_ready(
            batches,
            fallback_account,
            pending,
            terminal_account,
            fallback_plan,
        )
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord> Drop
    for ReadyFiniteStartupCrossPlane<'_, O, P, D>
{
    fn drop(&mut self) {
        if let Some(ownership) = self.ownership.take() {
            let _ = record_account_poison_event(
                &ownership.owners.fallback_account,
                AccountPoisonReason::OwnershipGraphUnverified,
            );
            ownership.contain_after_recorded_fault();
        }
    }
}

/// Owns graph storage, all three account owners, and the possibly committed charge.
///
/// The numeric work and storage roots remain in the outer invocation.
pub(crate) struct FiniteStartupCrossPlaneContainment<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    ownership: Option<FiniteStartupCrossPlaneOwnership<O, P, D>>,
    poison: AccountPoisonEventWitness,
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord> FiniteStartupCrossPlaneContainment<O, P, D> {
    pub(in crate::retained) fn poisoned(&self) -> Option<AccountPoisoned> {
        self.ownership.as_ref().map(|_| self.poison.poisoned)
    }
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord> Drop
    for FiniteStartupCrossPlaneContainment<O, P, D>
{
    fn drop(&mut self) {
        if let Some(ownership) = self.ownership.take() {
            ownership.contain_after_recorded_fault();
        }
    }
}

/// Finite failures keep their actual optional ownership beside the typed cause.
/// The cause remains small; uncertainty never needs an allocation on the failure path.
pub(in crate::retained) struct FiniteStartupCrossPlaneAdmissionFailure<O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord,
{
    pub(in crate::retained) cause: FiniteStartupAdmissionCause<D>,
    pub(in crate::retained) containment: Option<FiniteStartupCrossPlaneContainment<O, P, D>>,
}

pub(in crate::retained) enum FiniteStartupAdmissionCause<D> {
    LedgerAdmission(AttributionBatchCreateError<D>),
    OwnerShare(RetainedWriterError<D>),
    PhysicalNoMutation(StartupPhysicalNoMutation),
    Uncertain,
}

impl<O: OperationOwner, P: Copy + Ord, D: Copy + Ord>
    FiniteStartupCrossPlaneAdmissionFailure<O, P, D>
{
    fn clean(cause: FiniteStartupAdmissionCause<D>) -> Self {
        Self {
            cause,
            containment: None,
        }
    }
}

impl<'account> PendingOperationStartAdmission<'account> {
    pub(in crate::retained) fn try_prepare_finite_startup_cross_plane<O, P, D>(
        self,
        ledger: &mut LedgerView<'_, O, P, D>,
        plan: FiniteStartupCrossPlanePlan<O, P, D>,
    ) -> Result<
        ReadyFiniteStartupCrossPlane<'account, O, P, D>,
        FiniteStartupCrossPlaneAdmissionFailure<O, P, D>,
    >
    where
        O: OperationOwner,
        P: Copy + Ord,
        D: Copy + Ord + fmt::Debug,
    {
        let FiniteStartupCrossPlanePlan { parts, charge } = plan;
        let (graph, account_shares) = ledger
            .try_bind_finite_startup_attribution_graph(parts)
            .map_err(|cause| {
                FiniteStartupCrossPlaneAdmissionFailure::clean(
                    FiniteStartupAdmissionCause::LedgerAdmission(cause),
                )
            })?;
        if !graph.belongs_to_view(&ledger.as_read()) {
            return Err(FiniteStartupCrossPlaneAdmissionFailure::clean(
                FiniteStartupAdmissionCause::OwnerShare(RetainedWriterError::OwnershipUnverified),
            ));
        }
        let account = self.account;
        let owners = {
            let (mut work, _storage) = ledger.reborrow().into_internal();
            try_admit_finite_start_share_work(&mut work, account, account_shares).map_err(
                |cause| {
                    FiniteStartupCrossPlaneAdmissionFailure::clean(
                        FiniteStartupAdmissionCause::OwnerShare(cause),
                    )
                },
            )?
        };
        let ownership = FiniteStartupCrossPlaneOwnership {
            graph,
            owners,
            charge,
        };
        let transition = catch_unwind(AssertUnwindSafe(|| {
            try_commit_finite_startup_physical(account, &self, &ownership.owners, charge)
        }));
        match transition {
            Ok(Ok(())) => Ok(ReadyFiniteStartupCrossPlane {
                account,
                ownership: Some(ownership),
            }),
            Ok(Err(error)) => {
                drop(ownership);
                Err(FiniteStartupCrossPlaneAdmissionFailure::clean(
                    FiniteStartupAdmissionCause::PhysicalNoMutation(error),
                ))
            }
            Err(payload) => {
                let poison = record_account_poison_event(
                    account,
                    AccountPoisonReason::OwnershipGraphUnverified,
                );
                discard_untrusted_panic_payload(payload);
                Err(FiniteStartupCrossPlaneAdmissionFailure {
                    cause: FiniteStartupAdmissionCause::Uncertain,
                    containment: Some(FiniteStartupCrossPlaneContainment {
                        ownership: Some(ownership),
                        poison,
                    }),
                })
            }
        }
    }
}

fn try_commit_finite_startup_physical(
    account: &PhysicalStorageAccount,
    admission: &PendingOperationStartAdmission<'_>,
    owners: &FiniteStartupAccountOwners,
    charge: AdmissionCharge,
) -> Result<(), StartupPhysicalNoMutation> {
    if !account.is_same_account(admission.account) {
        return Err(StartupPhysicalNoMutation::ForeignStartAdmission);
    }
    validate_startup_owner(
        account,
        &owners.fallback_account,
        StartupAccountOwnerRole::FallbackWitness,
    )?;
    validate_startup_owner(
        account,
        &owners.fallback_lease_owner,
        StartupAccountOwnerRole::FallbackLease,
    )?;
    validate_startup_owner(
        account,
        &owners.terminal_account,
        StartupAccountOwnerRole::TerminalWitness,
    )?;
    try_commit_startup_physical_charge(account, charge, 1)
}

#[cfg(test)]
mod tests;
