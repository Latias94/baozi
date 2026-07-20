use super::config::{
    RetainedHandleSharePlan, RetainedWriterError, map_account_action_admission_error,
};
use crate::control::discard_untrusted_panic_payload;
use crate::ledger::{Ledger, StartupAccountShareEnvelope, StartupAccountShareStep};
use crate::owner::OperationOwner;
use crate::retained::account::AccountPoisonReason;
use crate::retained::allocation::RetainedSetId;
use crate::retained::physical::{
    AdmittedRetainedShareWork, PendingAccountActionAdmission, PhysicalStorageAccount,
    PublicationOpenAdmissionError, StartupAccountOwners,
};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

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
    Ok(admission.run(|account| admitted.consume_account(account)))
}

pub(in crate::retained) fn try_admit_retained_share_work<O, P, D>(
    ledger: &Ledger<O, P, D>,
    work: RetainedHandleSharePlan<D>,
) -> Result<AdmittedRetainedShareWork, RetainedWriterError<D>>
where
    O: OperationOwner,
    P: Copy + Ord,
    D: Copy + Ord + fmt::Debug + Send + Sync + 'static,
{
    ledger
        .charge_observed_batch(&work.charges())
        .map(|()| AdmittedRetainedShareWork { private: () })
        .map_err(RetainedWriterError::Ledger)
}

pub(in crate::retained) fn try_admit_retained_share_work_before_physical<O, P, D>(
    ledger: &Ledger<O, P, D>,
    account: &PhysicalStorageAccount,
    work: RetainedHandleSharePlan<D>,
) -> Result<AdmittedRetainedShareWork, RetainedWriterError<D>>
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
        ledger.charge_observed_batch(&step.into_charges())
    })) {
        Ok(Ok(())) => Ok(PhysicalStorageAccount {
            inner: account.inner.share(),
        }),
        Ok(Err(cause)) => Err(RetainedWriterError::Ledger(cause)),
        Err(payload) => {
            let poisoned = account.record_poison(AccountPoisonReason::OwnershipGraphUnverified);
            discard_untrusted_panic_payload(payload);
            Err(RetainedWriterError::AccountPoisoned(poisoned))
        }
    }
}
