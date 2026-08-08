use super::config::{
    RetainedHandleSharePlan, RetainedWriterError, map_account_action_admission_error,
};
use crate::control::discard_untrusted_panic_payload;
use crate::ledger::{
    Ledger, LinearWorkBatchPermit, StartupAccountShareEnvelope, StartupAccountShareStep,
};
use crate::owner::OperationOwner;
use crate::retained::account::AccountPoisonReason;
use crate::retained::allocation::{RetainedAllocation, RetainedSetId};
use crate::retained::physical::{
    PendingAccountActionAdmission, PhysicalStorageAccount, PublicationOpenAdmissionError,
    StartupAccountOwners,
};
use std::fmt;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Linear completion authority for one admitted clone-and-handle work pair.
///
/// Admission records both attempted units and reserves their complete future consumed envelope
/// under one ledger lock. The two consumed units are recorded only after the reference-count clone
/// and its typed retained handle, respectively, have actually been constructed.
#[must_use = "admitted retained share work must complete a physical share or remain attempted-only"]
pub(in crate::retained) struct RetainedShareWorkPermit<'ledger, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    work: LinearWorkBatchPermit<O, P, D, 2>,
    charges: [(D, u64); 2],
    _ledger: PhantomData<&'ledger Ledger<O, P, D>>,
}

pub(in crate::retained) struct RetainedShareWorkCompletionFailure<T, D> {
    owner: T,
    cause: crate::ledger::LedgerCause<D>,
}

impl<T, D> RetainedShareWorkCompletionFailure<T, D> {
    pub(in crate::retained) fn into_parts(self) -> (T, crate::ledger::LedgerCause<D>) {
        (self.owner, self.cause)
    }
}

impl<O, P, D> RetainedShareWorkPermit<'_, O, P, D>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug,
{
    fn complete_reference_clone(&mut self) -> Result<(), crate::ledger::LedgerCause<D>> {
        self.work.complete(0, self.charges[0].1)
    }

    fn complete_typed_handle(mut self) -> Result<(), crate::ledger::LedgerCause<D>> {
        self.work.complete(1, self.charges[1].1)
    }

    pub(in crate::retained) fn try_share_account(
        mut self,
        account: &PhysicalStorageAccount,
    ) -> Result<PhysicalStorageAccount, RetainedShareWorkCompletionFailure<PhysicalStorageAccount, D>>
    {
        let inner = account.inner.share();
        if let Err(cause) = self.complete_reference_clone() {
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

    pub(in crate::retained) fn try_share_allocation<T>(
        mut self,
        allocation: &RetainedAllocation<T>,
    ) -> Result<RetainedAllocation<T>, RetainedShareWorkCompletionFailure<RetainedAllocation<T>, D>>
    {
        let inner = allocation.inner.share();
        if let Err(cause) = self.complete_reference_clone() {
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
        if let Err(cause) = self.complete_typed_handle() {
            return Err(RetainedShareWorkCompletionFailure { owner, cause });
        }
        Ok(owner)
    }
}

#[allow(
    dead_code,
    reason = "crate-private until the U4 canonical consumers switch atomically"
)]
pub(in crate::retained) fn try_share_account_after_work<O, P, D>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    work: RetainedHandleSharePlan<D>,
) -> Result<PhysicalStorageAccount, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    let admission = account
        .try_admit_account_action()
        .map_err(map_account_action_admission_error)?;
    share_account_after_admitted_work(admission, ledger, work)
}

pub(in crate::retained) fn try_open_publication_account<O, P, D>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    work: RetainedHandleSharePlan<D>,
) -> Result<(PhysicalStorageAccount, RetainedSetId), RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
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
    let account = share_account_after_admitted_work(admission, ledger, work)?;
    Ok((account, set_id))
}

pub(in crate::retained) fn share_account_after_admitted_work<O, P, D>(
    admission: PendingAccountActionAdmission<'_>,
    ledger: &Ledger<O, P, D>,
    work: RetainedHandleSharePlan<D>,
) -> Result<PhysicalStorageAccount, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    let admitted = try_admit_retained_share_work(ledger, work)?;
    admission
        .run(|account| admitted.try_share_account(account))
        .map_err(|failure| RetainedWriterError::Ledger(failure.into_parts().1))
}

pub(in crate::retained) fn try_admit_retained_share_work<O, P, D>(
    ledger: &Ledger<O, P, D>,
    work: RetainedHandleSharePlan<D>,
) -> Result<RetainedShareWorkPermit<'_, O, P, D>, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    let charges = work.charges();
    ledger
        .try_admit_linear_work_batch(&charges)
        .map(|work| RetainedShareWorkPermit {
            work,
            charges,
            _ledger: PhantomData,
        })
        .map_err(RetainedWriterError::Ledger)
}

pub(in crate::retained) fn try_admit_retained_share_work_before_physical<'ledger, O, P, D>(
    ledger: &'ledger Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    work: RetainedHandleSharePlan<D>,
) -> Result<RetainedShareWorkPermit<'ledger, O, P, D>, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    match catch_unwind(AssertUnwindSafe(|| {
        try_admit_retained_share_work(ledger, work)
    })) {
        Ok(result) => result,
        Err(payload) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            Err(RetainedWriterError::AccountPoisoned(poisoned))
        }
    }
}

pub(in crate::retained) fn try_admit_start_share_work<O, P, D>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    account_shares: StartupAccountShareEnvelope<D>,
) -> Result<StartupAccountOwners, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    let [
        fallback_account_step,
        evidence_account_step,
        fallback_lease_step,
        evidence_lease_step,
    ] = account_shares.into_steps();
    let fallback_account =
        try_share_startup_account_after_work(ledger, account, fallback_account_step)?;
    let evidence_account =
        try_share_startup_account_after_work(ledger, account, evidence_account_step)?;
    let fallback_lease_owner =
        try_share_startup_account_after_work(ledger, account, fallback_lease_step)?;
    let evidence_lease_owner =
        try_share_startup_account_after_work(ledger, account, evidence_lease_step)?;
    Ok(StartupAccountOwners {
        fallback_account,
        evidence_account,
        fallback_lease_owner,
        evidence_lease_owner,
    })
}

pub(in crate::retained) fn try_share_startup_account_after_work<O, P, D>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    step: StartupAccountShareStep<D>,
) -> Result<PhysicalStorageAccount, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    match catch_unwind(AssertUnwindSafe(|| {
        let charges = step.into_charges();
        ledger
            .try_admit_linear_work_batch(&charges)
            .map(|work| RetainedShareWorkPermit {
                work,
                charges,
                _ledger: PhantomData,
            })
            .map_err(RetainedWriterError::Ledger)?
            .try_share_account(account)
            .map_err(|failure| RetainedWriterError::Ledger(failure.into_parts().1))
    })) {
        Ok(Ok(account)) => Ok(account),
        Ok(Err(cause)) => Err(cause),
        Err(payload) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            Err(RetainedWriterError::AccountPoisoned(poisoned))
        }
    }
}
