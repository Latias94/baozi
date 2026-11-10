use super::config::{
    RetainedHandleSharePlan, RetainedWriterError, map_account_action_admission_error,
};
use crate::control::discard_untrusted_panic_payload;
use crate::ledger::{InternalWork, StartupAccountShareEnvelope, StartupAccountShareStep};
use crate::retained::account::AccountPoisonReason;
use crate::retained::allocation::{RetainedAllocation, RetainedSetId};
use crate::retained::physical::{
    FiniteStartupAccountOwners, PhysicalStorageAccount, PublicationOpenAdmissionError,
    StartupAccountOwners,
};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

mod borrowed;
use borrowed::admit_internal_share_charges;
pub(in crate::retained) use borrowed::{
    admit_internal_share, admit_internal_share_before_physical,
    try_admit_borrowed_retained_share_work,
};

pub(crate) struct RetainedShareWorkCompletionFailure<T, D> {
    owner: T,
    cause: crate::ledger::LedgerCause<D>,
}

impl<T, D> RetainedShareWorkCompletionFailure<T, D> {
    pub(crate) fn into_parts(self) -> (T, crate::ledger::LedgerCause<D>) {
        (self.owner, self.cause)
    }
}

/// Static completion adapter for the single physical clone/typed-handle implementation.
/// Each caller lends its existing numeric frame through the admitted borrowed work.
pub(crate) trait AdmittedShareWork<D>: Sized {
    fn complete_reference_clone(&mut self) -> Result<(), crate::ledger::LedgerCause<D>>;
    fn complete_typed_handle(self) -> Result<(), crate::ledger::LedgerCause<D>>;
    fn finish_partial(self) -> Result<(), crate::ledger::LedgerCause<D>>;

    fn try_share_account(
        mut self,
        account: &PhysicalStorageAccount,
    ) -> Result<PhysicalStorageAccount, RetainedShareWorkCompletionFailure<PhysicalStorageAccount, D>>
    {
        let inner = account.inner.share();
        if let Err(cause) = self.complete_reference_clone() {
            let cause = self.finish_partial().err().unwrap_or(cause);
            return Err(RetainedShareWorkCompletionFailure {
                owner: PhysicalStorageAccount { inner },
                cause,
            });
        }
        let owner = PhysicalStorageAccount { inner };
        if let Err(cause) = self.complete_typed_handle() {
            return Err(RetainedShareWorkCompletionFailure { owner, cause });
        }
        Ok(owner)
    }
}

/// Allocation handles remain private to retained storage, independently of the numeric adapter.
fn share_allocation_after_admitted_work<T, D>(
    mut work: impl AdmittedShareWork<D>,
    allocation: &RetainedAllocation<T>,
) -> Result<RetainedAllocation<T>, RetainedShareWorkCompletionFailure<RetainedAllocation<T>, D>> {
    let inner = allocation.inner.share();
    if let Err(cause) = work.complete_reference_clone() {
        let cause = work.finish_partial().err().unwrap_or(cause);
        return Err(RetainedShareWorkCompletionFailure {
            owner: RetainedAllocation {
                inner,
                membership: None,
            },
            cause,
        });
    }
    let owner = RetainedAllocation {
        inner,
        membership: None,
    };
    if let Err(cause) = work.complete_typed_handle() {
        return Err(RetainedShareWorkCompletionFailure { owner, cause });
    }
    Ok(owner)
}

pub(in crate::retained) fn try_open_publication_account<D: Copy + Ord + fmt::Debug>(
    work: &mut InternalWork<'_, D>,
    account: &PhysicalStorageAccount,
    plan: RetainedHandleSharePlan<D>,
) -> Result<(PhysicalStorageAccount, RetainedSetId), RetainedWriterError<D>> {
    let (admission, set_id) =
        account
            .try_admit_publication_open()
            .map_err(|cause| match cause {
                PublicationOpenAdmissionError::Account(cause) => {
                    map_account_action_admission_error(cause)
                }
                PublicationOpenAdmissionError::MembershipExhausted => {
                    RetainedWriterError::MembershipExhausted
                }
            })?;
    let admitted = admit_internal_share(work, plan)?;
    let account = admission
        .run(|account| admitted.try_share_account(account))
        .map_err(|failure| RetainedWriterError::Ledger(failure.into_parts().1))?;
    Ok((account, set_id))
}

pub(in crate::retained) fn try_admit_start_share_work<D: Copy + Ord + fmt::Debug>(
    work: &mut InternalWork<'_, D>,
    account: &PhysicalStorageAccount,
    account_shares: StartupAccountShareEnvelope<D>,
) -> Result<StartupAccountOwners, RetainedWriterError<D>> {
    let [
        fallback_account_step,
        evidence_account_step,
        fallback_lease_step,
        evidence_lease_step,
    ] = account_shares.into_steps();
    let fallback_account =
        try_share_startup_account_after_work(work, account, fallback_account_step)?;
    let evidence_account =
        try_share_startup_account_after_work(work, account, evidence_account_step)?;
    let fallback_lease_owner =
        try_share_startup_account_after_work(work, account, fallback_lease_step)?;
    let evidence_lease_owner =
        try_share_startup_account_after_work(work, account, evidence_lease_step)?;
    Ok(StartupAccountOwners {
        fallback_account,
        evidence_account,
        fallback_lease_owner,
        evidence_lease_owner,
    })
}

pub(in crate::retained) fn try_admit_finite_start_share_work<D: Copy + Ord + fmt::Debug>(
    work: &mut InternalWork<'_, D>,
    account: &PhysicalStorageAccount,
    account_shares: StartupAccountShareEnvelope<D, 3>,
) -> Result<FiniteStartupAccountOwners, RetainedWriterError<D>> {
    let [
        fallback_account_step,
        fallback_lease_step,
        terminal_account_step,
    ] = account_shares.into_steps();
    let fallback_account =
        try_share_startup_account_after_work(work, account, fallback_account_step)?;
    let fallback_lease_owner =
        try_share_startup_account_after_work(work, account, fallback_lease_step)?;
    let terminal_account =
        try_share_startup_account_after_work(work, account, terminal_account_step)?;
    Ok(FiniteStartupAccountOwners {
        fallback_account,
        fallback_lease_owner,
        terminal_account,
    })
}

fn try_share_startup_account_after_work<D: Copy + Ord + fmt::Debug>(
    work: &mut InternalWork<'_, D>,
    account: &PhysicalStorageAccount,
    step: StartupAccountShareStep<D>,
) -> Result<PhysicalStorageAccount, RetainedWriterError<D>> {
    match catch_unwind(AssertUnwindSafe(|| {
        admit_internal_share_charges(work, step.into_charges())?
            .try_share_account(account)
            .map_err(|failure| RetainedWriterError::Ledger(failure.into_parts().1))
    })) {
        Ok(result) => result,
        Err(payload) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            Err(RetainedWriterError::AccountPoisoned(poisoned))
        }
    }
}
