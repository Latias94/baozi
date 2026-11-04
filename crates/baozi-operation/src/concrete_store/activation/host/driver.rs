//! A retained body ends before its outer host lends final-only terminal authority.

use super::*;
use crate::concrete_store::activation::payload_owner::PayloadOwner;
use crate::concrete_store::activation::{
    CanonicalCandidateStoreSession, CanonicalFinal, CentrallyRejectedCanonicalStoreSession,
    FailureEvidenceWorkBudget, InterruptedCanonicalStoreSession, NeverActivatedAuthority,
    NeverActivatedOwner, NeverActivatedStoreSession, PendingMinimalFallback,
    RejectedCanonicalStoreSession, RootedFailureEvidence, UnrootedFailureEvidence,
    ZeroCommitDestinationFacts,
};
use crate::concrete_store::deferred::{
    DeferredStoreFailure, DeferredStoreFallback, DeferredStoreOperation,
    DeferredUnrootedStoreFailure,
};
use crate::concrete_store::{
    BoundStoreOperation, CanonicalInheritanceSource, PublishedCanonicalStore,
    RetainedStoreStartError, RetainedStoreStartPlan, StoreLedgerPlan,
};
use crate::concrete_store::{
    DeferredStoreSuccess, PreparedStoreEvidenceOnlyOutcome, PreparedStoreFallback,
    RetainedStoreBlockTablePlan, RetainedStoreOperation, StoreDimensionKey, StorePrepareError,
    TerminalStoreLineage,
};
use crate::{FinalProgress, retained::PreparationFailureCode};
use crate::{OperationControl, PhysicalStorageAccount};

#[derive(Debug)]
pub enum CanonicalBoundBodyError {
    Start(RetainedStoreStartError),
    Body(CanonicalInvocationBodyError),
}

impl CanonicalBoundBodyError {
    pub fn containment(&self) -> Option<&super::super::StoreActivationContainment> {
        match self {
            Self::Start(_) => None,
            Self::Body(cause) => cause.containment(),
        }
    }
}

impl std::fmt::Display for CanonicalBoundBodyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start(cause) => cause.fmt(formatter),
            Self::Body(cause) => cause.fmt(formatter),
        }
    }
}

impl std::error::Error for CanonicalBoundBodyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Start(cause) => Some(cause),
            Self::Body(cause) => Some(cause),
        }
    }
}

pub(in crate::concrete_store::activation) struct PendingCandidate<CK, CT, EK, ET> {
    pub(in crate::concrete_store::activation) store: DeferredStoreSuccess<CK, CT, EK, ET>,
    pub(in crate::concrete_store::activation) destinations: ZeroCommitDestinationFacts,
    pub(in crate::concrete_store::activation) failure_work: FailureEvidenceWorkBudget,
}

pub(in crate::concrete_store::activation) enum OperationDisposition {
    Unchanged,
    MinimalFallback { code: u32 },
}

pub(in crate::concrete_store::activation) struct PendingOperation {
    pub(in crate::concrete_store::activation) store: DeferredStoreOperation,
    pub(in crate::concrete_store::activation) request: OperationDisposition,
}

pub(in crate::concrete_store::activation) struct PendingNeverActivated {
    pub(in crate::concrete_store::activation) store: DeferredStoreOperation,
    pub(in crate::concrete_store::activation) fallback_only: bool,
    pub(in crate::concrete_store::activation) failure_code: PreparationFailureCode,
    pub(in crate::concrete_store::activation) outcome_evidence_plan: RetainedStoreBlockTablePlan,
    pub(in crate::concrete_store::activation) failure_work: FailureEvidenceWorkBudget,
    pub(in crate::concrete_store::activation) destination: StoreDimensionKey,
}

pub(in crate::concrete_store::activation) struct PendingRejected {
    pub(in crate::concrete_store::activation) store: DeferredUnrootedStoreFailure,
    pub(in crate::concrete_store::activation) failure_code: PreparationFailureCode,
    pub(in crate::concrete_store::activation) failure_work: FailureEvidenceWorkBudget,
    pub(in crate::concrete_store::activation) destination: StoreDimensionKey,
}

pub(in crate::concrete_store::activation) struct PendingRootedFailure<EK, ET> {
    pub(in crate::concrete_store::activation) store: DeferredStoreFailure<EK, ET>,
    pub(in crate::concrete_store::activation) failure_work: FailureEvidenceWorkBudget,
    pub(in crate::concrete_store::activation) destination: StoreDimensionKey,
}

pub(in crate::concrete_store::activation) enum PendingDisposition<CK, CT, EK, ET> {
    Unattached,
    Operation(PendingOperation),
    Candidate(PendingCandidate<CK, CT, EK, ET>),
    NeverActivated(PendingNeverActivated),
    Rejected(PendingRejected),
    CentralRejected(PendingRootedFailure<EK, ET>),
    Interrupted(PendingRootedFailure<EK, ET>),
    Fallback(DeferredStoreFallback),
}

/// A single disposition request for this invocation's exact typed canonical roots.
pub struct CanonicalDispositionSlot<'inv, CK, CT, EK, ET> {
    pub(super) value: &'inv mut Option<PendingDisposition<CK, CT, EK, ET>>,
    pub(super) identity: *const (),
}

impl<CK, CT, EK, ET> CanonicalDispositionSlot<'_, CK, CT, EK, ET> {
    pub(in crate::concrete_store::activation) fn is_vacant_for(&self, identity: *const ()) -> bool {
        self.identity == identity && self.value.is_none()
    }

    pub(in crate::concrete_store::activation) fn install(
        self,
        pending: PendingDisposition<CK, CT, EK, ET>,
    ) {
        *self.value = Some(pending);
    }
}

/// The body left no valid disposition, or the invocation had already admitted a body.
#[derive(Debug)]
pub struct CanonicalInvocationBodyError {
    containment: Option<super::super::StoreActivationContainment>,
}

impl CanonicalInvocationBodyError {
    pub const fn containment(&self) -> Option<&super::super::StoreActivationContainment> {
        self.containment.as_ref()
    }
}

impl std::fmt::Display for CanonicalInvocationBodyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("canonical invocation body did not leave one valid disposition")
    }
}

impl std::error::Error for CanonicalInvocationBodyError {}

/// Final-only authority issued after the retained body and all its captures are gone.
pub struct CanonicalInvocationFinish<'inv, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    #[allow(
        clippy::type_complexity,
        reason = "finish borrows the one host with its distinct output, proof, causes and root brands"
    )]
    host: &'inv mut CanonicalInvocationHost<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>,
}

/// Final authority after the bound body and its captures have gone, retaining its original account.
///
/// `'inv` exclusively borrows the host; the independent `'account` loan preserves the original
/// account for [`Self::finish_unattached`]. Only a successful [`Self::into_attached`] conversion
/// releases the external account loan while retaining the host loan.
///
/// Consume the genuine finish before replacing or dropping its host:
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartPlan;
/// use baozi_operation::{
///     FinalProgress, OperationControl, OperationOwner, PhysicalStorageAccount,
///     ProgressPhase, ProgressUnit, StoreLedgerPlan,
/// };
/// fn finish_then_reuse<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
///     account: &PhysicalStorageAccount, plan: RetainedStoreStartPlan,
///     progress: FinalProgress<P, U>,
/// ) {
///     let mut host = CanonicalInvocationHost::<O, P, U, (), ()>::new();
///     let finish = host.run_bound_body(control, ledger, account, plan, |parts, disposition| {
///         parts.startup.request_unchanged(disposition).unwrap();
///     }).unwrap();
///     let _ = finish.finish_unattached(progress);
///     let previous = std::mem::replace(&mut host, CanonicalInvocationHost::new());
///     drop(previous);
///     drop(host);
/// }
/// ```
///
/// Dropping the host before the later terminal call would move a borrowed owner (`E0505`):
///
/// ```compile_fail,E0505
/// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartPlan;
/// use baozi_operation::{
///     FinalProgress, OperationControl, OperationOwner, PhysicalStorageAccount,
///     ProgressPhase, ProgressUnit, StoreLedgerPlan,
/// };
/// fn drop_host_early<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
///     account: &PhysicalStorageAccount, plan: RetainedStoreStartPlan,
///     progress: FinalProgress<P, U>,
/// ) {
///     let mut host = CanonicalInvocationHost::<O, P, U, (), ()>::new();
///     let finish = host.run_bound_body(control, ledger, account, plan, |parts, disposition| {
///         parts.startup.request_unchanged(disposition).unwrap();
///     }).unwrap();
///     drop(host);
///     let _ = finish.finish_unattached(progress);
/// }
/// ```
///
/// Replacing that host also requires a second mutable loan while the finish still uses it (`E0499`):
///
/// ```compile_fail,E0499
/// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartPlan;
/// use baozi_operation::{
///     FinalProgress, OperationControl, OperationOwner, PhysicalStorageAccount,
///     ProgressPhase, ProgressUnit, StoreLedgerPlan,
/// };
/// fn replace_host_early<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
///     account: &PhysicalStorageAccount, plan: RetainedStoreStartPlan,
///     progress: FinalProgress<P, U>,
/// ) {
///     let mut host = CanonicalInvocationHost::<O, P, U, (), ()>::new();
///     let finish = host.run_bound_body(control, ledger, account, plan, |parts, disposition| {
///         parts.startup.request_unchanged(disposition).unwrap();
///     }).unwrap();
///     let previous = std::mem::replace(&mut host, CanonicalInvocationHost::new());
///     let _ = finish.finish_unattached(progress);
///     drop(previous);
/// }
/// ```
///
/// A scoped worker may run alongside completion on the invocation's own thread:
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartPlan;
/// use baozi_operation::{
///     FinalProgress, OperationControl, OperationOwner, PhysicalStorageAccount,
///     ProgressPhase, ProgressUnit, StoreLedgerPlan,
/// };
/// fn finish_on_invocation_thread<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     host: &mut CanonicalInvocationHost<O, P, U, (), ()>,
///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
///     account: &PhysicalStorageAccount, plan: RetainedStoreStartPlan,
///     progress: FinalProgress<P, U>,
/// ) {
///     let finish = host.run_bound_body(control, ledger, account, plan, |parts, disposition| {
///         parts.startup.request_unchanged(disposition).unwrap();
///     }).unwrap();
///     std::thread::scope(|scope| {
///         let worker = scope.spawn(|| ());
///         let _ = finish.finish_unattached(progress);
///         worker.join().unwrap();
///     });
/// }
/// ```
///
/// Moving the finish into that worker fails because the invocation permission is not `Send`
/// (`E0277`), even though a scoped thread needs no `'static` borrow:
///
/// ```compile_fail,E0277
/// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartPlan;
/// use baozi_operation::{
///     FinalProgress, OperationControl, OperationOwner, PhysicalStorageAccount,
///     ProgressPhase, ProgressUnit, StoreLedgerPlan,
/// };
/// fn move_finish_to_worker<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     host: &mut CanonicalInvocationHost<O, P, U, (), ()>,
///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
///     account: &PhysicalStorageAccount, plan: RetainedStoreStartPlan,
///     progress: FinalProgress<P, U>,
/// ) {
///     let finish = host.run_bound_body(control, ledger, account, plan, |parts, disposition| {
///         parts.startup.request_unchanged(disposition).unwrap();
///     }).unwrap();
///     std::thread::scope(|scope| {
///         scope.spawn(move || {
///             let _ = finish.finish_unattached(progress);
///         });
///     });
/// }
/// ```
pub struct CanonicalBoundInvocationFinish<
    'inv,
    'account,
    O,
    PP,
    U,
    T,
    Proof,
    EA,
    EV,
    EG,
    CK,
    CT,
    EK,
    ET,
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    finish: CanonicalInvocationFinish<'inv, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>,
    unattached: Option<crate::concrete_store::deferred::DeferredUnattachedStore<'account>>,
}

impl<'inv, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
    CanonicalBoundInvocationFinish<'inv, '_, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Ends the original account loan when this body actually installed a retained operation.
    ///
    /// `Ok` retains only the host loan. `Err` returns the bound finish with its original account
    /// loan intact. The independent lifetimes allow a local account to die while an attached
    /// finish is returned for the caller's longer host loan:
    ///
    /// ```
    /// use baozi_operation::__retained_store_integration::activation::{
    ///     CanonicalInvocationFinish, CanonicalInvocationHost,
    /// };
    /// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartPlan;
    /// use baozi_operation::{
    ///     OperationControl, OperationOwner, PhysicalStorageAccount, ProgressPhase,
    ///     ProgressUnit, StoreLedgerPlan,
    /// };
    /// fn attach<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     host: &'inv mut CanonicalInvocationHost<O, P, U, (), ()>,
    ///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
    ///     account: PhysicalStorageAccount, plan: RetainedStoreStartPlan,
    /// ) -> Option<CanonicalInvocationFinish<
    ///     'inv, O, P, U, (), (), (), (), (), (), (), (), (),
    /// >> {
    ///     let finish = host.run_bound_body(control, ledger, &account, plan, |parts, disposition| {
    ///         if let Ok(operation) = parts.startup.attach() {
    ///             operation.request_unchanged(disposition).unwrap();
    ///         }
    ///     }).ok()?;
    ///     let attached = finish.into_attached().ok()?;
    ///     drop(account);
    ///     Some(attached)
    /// }
    /// ```
    ///
    /// The runtime attachment alone does not shorten the bound finish's account loan. Dropping
    /// that account before consuming the conversion still moves a borrowed owner (`E0505`):
    ///
    /// ```compile_fail,E0505
    /// use baozi_operation::__retained_store_integration::activation::{
    ///     CanonicalInvocationFinish, CanonicalInvocationHost,
    /// };
    /// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartPlan;
    /// use baozi_operation::{
    ///     OperationControl, OperationOwner, PhysicalStorageAccount, ProgressPhase,
    ///     ProgressUnit, StoreLedgerPlan,
    /// };
    /// fn drop_account_before_conversion<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     host: &'inv mut CanonicalInvocationHost<O, P, U, (), ()>,
    ///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
    ///     account: PhysicalStorageAccount, plan: RetainedStoreStartPlan,
    /// ) -> Option<CanonicalInvocationFinish<
    ///     'inv, O, P, U, (), (), (), (), (), (), (), (), (),
    /// >> {
    ///     let finish = host.run_bound_body(control, ledger, &account, plan, |parts, disposition| {
    ///         if let Ok(operation) = parts.startup.attach() {
    ///             operation.request_unchanged(disposition).unwrap();
    ///         }
    ///     }).ok()?;
    ///     drop(account);
    ///     let attached = finish.into_attached().ok()?;
    ///     Some(attached)
    /// }
    /// ```
    #[allow(
        clippy::result_large_err,
        clippy::type_complexity,
        reason = "a mismatched terminal keeps its actual host and original account permission"
    )]
    pub fn into_attached(
        self,
    ) -> Result<CanonicalInvocationFinish<'inv, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>, Self>
    {
        if self.unattached.is_some() {
            Err(self)
        } else {
            Ok(self.finish)
        }
    }

    /// Applies the original account/control gates and transfers its existing snapshot buffers.
    ///
    /// Consume the finish inside the original account's scope. The returned evidence no longer
    /// borrows that account:
    ///
    /// ```
    /// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
    /// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartPlan;
    /// use baozi_operation::{
    ///     FinalProgress, OperationControl, OperationOwner, PhysicalStorageAccount,
    ///     ProgressPhase, ProgressUnit, StoreLedgerEvidence, StoreLedgerPlan, StoreRuntimeError,
    /// };
    /// fn finish_before_account_dies<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     host: &mut CanonicalInvocationHost<O, P, U, (), ()>,
    ///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
    ///     account: PhysicalStorageAccount, plan: RetainedStoreStartPlan,
    ///     progress: FinalProgress<P, U>,
    /// ) -> Result<StoreLedgerEvidence, StoreRuntimeError> {
    ///     let evidence = {
    ///         let original_account = account;
    ///         let finish = host.run_bound_body(
    ///             control, ledger, &original_account, plan, |parts, disposition| {
    ///                 parts.startup.request_unchanged(disposition).unwrap();
    ///             },
    ///         ).unwrap();
    ///         finish.finish_unattached(progress)
    ///     };
    ///     evidence
    /// }
    /// ```
    ///
    /// Letting the original account leave scope before that same terminal call is rejected
    /// (`E0597`); the finish cannot substitute another account:
    ///
    /// ```compile_fail,E0597
    /// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
    /// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartPlan;
    /// use baozi_operation::{
    ///     FinalProgress, OperationControl, OperationOwner, PhysicalStorageAccount,
    ///     ProgressPhase, ProgressUnit, StoreLedgerEvidence, StoreLedgerPlan, StoreRuntimeError,
    /// };
    /// fn outlive_original_account<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     host: &mut CanonicalInvocationHost<O, P, U, (), ()>,
    ///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
    ///     account: PhysicalStorageAccount, plan: RetainedStoreStartPlan,
    ///     progress: FinalProgress<P, U>,
    /// ) -> Result<StoreLedgerEvidence, StoreRuntimeError> {
    ///     let finish = {
    ///         let original_account = account;
    ///         host.run_bound_body(control, ledger, &original_account, plan, |parts, disposition| {
    ///             parts.startup.request_unchanged(disposition).unwrap();
    ///         }).unwrap()
    ///     };
    ///     finish.finish_unattached(progress)
    /// }
    /// ```
    pub fn finish_unattached(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<crate::StoreLedgerEvidence, crate::StoreRuntimeError> {
        let Self { finish, unattached } = self;
        let pending = unattached.ok_or(crate::StoreRuntimeError::OwnershipUnverified)?;
        if !matches!(finish.host.pending, Some(PendingDisposition::Unattached)) {
            finish
                .host
                .store
                .record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
            return Err(crate::StoreRuntimeError::OwnershipUnverified);
        }
        finish.host.pending = None;
        finish
            .host
            .store
            .prepare_deferred_unattached(pending, final_progress)
    }
}

/// One actual host stage, restored only after body and capture cleanup.
#[allow(
    clippy::type_complexity,
    reason = "each final capability retains the exact invocation, payload, proof and root brands"
)]
pub enum CanonicalInvocationOutcome<'inv, O, PP, U, T, Proof, EA, EV, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Operation(CanonicalOperationCompletion<'inv, O, PP, U>),
    Candidate(
        CanonicalFinal<CanonicalCandidateStoreSession<'inv, O, PP, U, T, Proof, CK, CT, EK, ET>>,
    ),
    NeverActivated(
        CanonicalFinal<NeverActivatedStoreSession<'inv, O, PP, U, EA, TerminalStoreLineage>>,
    ),
    Rejected(CanonicalFinal<RejectedCanonicalStoreSession<'inv, O, PP, U, EA>>),
    CentralRejected(
        CanonicalFinal<CentrallyRejectedCanonicalStoreSession<'inv, O, PP, U, EV, EK, ET>>,
    ),
    Interrupted(CanonicalFinal<InterruptedCanonicalStoreSession<'inv, O, PP, U, EK, ET>>),
    Fallback(CanonicalFinal<PendingMinimalFallback<'inv, O, PP, U>>),
}

/// The exact outcome together with its final generation and fallback permissions.
pub struct CanonicalInvocationCompletion<'inv, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub outcome: CanonicalInvocationOutcome<'inv, O, PP, U, T, Proof, EA, EV, CK, CT, EK, ET>,
    pub generation: CanonicalGenerationSlots<'inv, EG>,
    pub failure_fallback: CanonicalCauseSlot<'inv, StoreRuntimeError>,
}

/// Started operation whose exact unchanged or fallback request is already sealed.
pub struct CanonicalOperationCompletion<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: RetainedStoreOperation<'inv, O, PP, U, TerminalStoreLineage>,
    request: OperationDisposition,
}

/// Owned terminal result for a body that required no canonical activation.
pub enum CanonicalOperationPublication {
    Unchanged(PreparedStoreEvidenceOnlyOutcome),
    MinimalFallback(PreparedStoreFallback),
}

impl<O, PP, U> CanonicalOperationCompletion<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn finish(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<CanonicalOperationPublication, StorePrepareError> {
        match self.request {
            OperationDisposition::Unchanged => self
                .operation
                .prepare_evidence_only_success(final_progress)
                .map(CanonicalOperationPublication::Unchanged),
            OperationDisposition::MinimalFallback { code } => self
                .operation
                .prepare_minimal_fallback(code)
                .map(CanonicalOperationPublication::MinimalFallback),
        }
    }
}

impl<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
    CanonicalInvocationHost<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Binds the fresh invocation before lending a short permission to the retained body.
    ///
    /// Prework uses the bound invocation before attachment:
    ///
    /// ```
    /// use baozi_operation::__retained_store_integration::activation::{
    ///     CanonicalInvocationHost, StoreInvocationError,
    /// };
    /// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartPlan;
    /// use baozi_operation::{
    ///     OperationControl, OperationOwner, PhysicalStorageAccount, ProgressPhase,
    ///     ProgressUnit, StoreLedgerPlan,
    /// };
    /// fn run<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     host: &mut CanonicalInvocationHost<O, P, U, (), ()>,
    ///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
    ///     account: &PhysicalStorageAccount, plan: RetainedStoreStartPlan,
    /// ) {
    ///     let _ = host.run_bound_body(control, ledger, account, plan, |mut parts, disposition| {
    ///         let work = parts.startup.with_exclusive_execution::<_, StoreInvocationError>(
    ///             |_| Ok(()),
    ///         );
    ///         if work.is_ok() {
    ///             if let Ok(operation) = parts.startup.attach() {
    ///                 let _ = operation.request_unchanged(disposition);
    ///             }
    ///         }
    ///     });
    /// }
    /// ```
    ///
    /// The short startup permission cannot escape the body:
    ///
    /// ```compile_fail,E0521
    /// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
    /// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartPlan;
    /// use baozi_operation::{
    ///     OperationControl, OperationOwner, PhysicalStorageAccount, ProgressPhase,
    ///     ProgressUnit, StoreLedgerPlan,
    /// };
    /// fn escape<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     host: &mut CanonicalInvocationHost<O, P, U, (), ()>,
    ///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
    ///     account: &PhysicalStorageAccount, plan: RetainedStoreStartPlan,
    /// ) {
    ///     let mut escaped = None;
    ///     let _ = host.run_bound_body(control, ledger, account, plan, |parts, _| {
    ///         escaped = Some(parts.startup);
    ///     });
    ///     drop(escaped);
    /// }
    /// ```
    ///
    /// Attachment does not extend the operation beyond its body:
    ///
    /// ```compile_fail,E0521
    /// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
    /// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartPlan;
    /// use baozi_operation::{
    ///     OperationControl, OperationOwner, PhysicalStorageAccount, ProgressPhase,
    ///     ProgressUnit, StoreLedgerPlan,
    /// };
    /// fn escape<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     host: &mut CanonicalInvocationHost<O, P, U, (), ()>,
    ///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
    ///     account: &PhysicalStorageAccount, plan: RetainedStoreStartPlan,
    /// ) {
    ///     let mut escaped = None;
    ///     let _ = host.run_bound_body(control, ledger, account, plan, |parts, _| {
    ///         escaped = parts.startup.attach().ok();
    ///     });
    ///     drop(escaped);
    /// }
    /// ```
    #[allow(
        clippy::type_complexity,
        clippy::result_large_err,
        reason = "the canonical entry preserves exact typed startup/body causes without allocating"
    )]
    pub fn run_bound_body<'account>(
        &mut self,
        control: OperationControl<PP, U>,
        ledger: StoreLedgerPlan,
        account: &'account PhysicalStorageAccount,
        plan: RetainedStoreStartPlan,
        body: impl for<'call, 'entry> FnMut(
            CanonicalBoundBodyParts<'call, 'entry, 'account, O, PP, U, T, Proof, EA, EV>,
            CanonicalDispositionSlot<'call, CK, CT, EK, ET>,
        ),
    ) -> Result<
        CanonicalBoundInvocationFinish<
            '_,
            'account,
            O,
            PP,
            U,
            T,
            Proof,
            EA,
            EV,
            EG,
            CK,
            CT,
            EK,
            ET,
        >,
        CanonicalBoundBodyError,
    > {
        self.run_bound_body_with(
            control,
            ledger,
            plan,
            |start, control, ledger, plan| {
                RetainedStoreOperation::bind(start.into_start(), control, ledger, account, plan)
            },
            body,
        )
    }

    /// Binds one exact source generation and its account before client prework.
    #[allow(
        clippy::type_complexity,
        clippy::result_large_err,
        reason = "the canonical entry retains exact source/startup/body types until outer cleanup"
    )]
    pub fn run_bound_derived_body<'account, SourceKind, SourceTag>(
        &mut self,
        control: OperationControl<PP, U>,
        ledger: StoreLedgerPlan,
        source: &'account PublishedCanonicalStore<SourceKind, SourceTag>,
        plan: RetainedStoreStartPlan,
        body: impl for<'call, 'entry> FnMut(
            CanonicalBoundBodyParts<
                'call,
                'entry,
                'account,
                O,
                PP,
                U,
                T,
                Proof,
                EA,
                EV,
                CanonicalInheritanceSource,
            >,
            CanonicalDispositionSlot<'call, CK, CT, EK, ET>,
        ),
    ) -> Result<
        CanonicalBoundInvocationFinish<
            '_,
            'account,
            O,
            PP,
            U,
            T,
            Proof,
            EA,
            EV,
            EG,
            CK,
            CT,
            EK,
            ET,
        >,
        CanonicalBoundBodyError,
    > {
        self.run_bound_body_with(
            control,
            ledger,
            plan,
            |start, control, ledger, plan| {
                RetainedStoreOperation::bind_derived(
                    start.into_start(),
                    control,
                    ledger,
                    source,
                    plan,
                )
            },
            body,
        )
    }

    #[allow(
        clippy::type_complexity,
        clippy::too_many_arguments,
        clippy::result_large_err,
        reason = "one retained callback driver owns both typed binding and body captures"
    )]
    fn run_bound_body_with<'account, L>(
        &mut self,
        control: OperationControl<PP, U>,
        ledger: StoreLedgerPlan,
        plan: RetainedStoreStartPlan,
        mut bind: impl for<'entry> FnMut(
            CanonicalBodyStart<'entry, O, PP, U>,
            OperationControl<PP, U>,
            StoreLedgerPlan,
            RetainedStoreStartPlan,
        ) -> Result<
            BoundStoreOperation<'entry, 'account, O, PP, U, L>,
            RetainedStoreStartError,
        >,
        mut body: impl for<'call, 'entry> FnMut(
            CanonicalBoundBodyParts<'call, 'entry, 'account, O, PP, U, T, Proof, EA, EV, L>,
            CanonicalDispositionSlot<'call, CK, CT, EK, ET>,
        ),
    ) -> Result<
        CanonicalBoundInvocationFinish<
            '_,
            'account,
            O,
            PP,
            U,
            T,
            Proof,
            EA,
            EV,
            EG,
            CK,
            CT,
            EK,
            ET,
        >,
        CanonicalBoundBodyError,
    > {
        let mut startup = Some((control, ledger, plan));
        let mut start_error = None;
        let start_error_slot = &mut start_error;
        let mut unattached = None;
        let unattached_slot = &mut unattached;
        let result = self.run_body(move |parts, disposition| {
            let CanonicalBodyParts {
                startup: start,
                output,
                proof,
                activation,
                validation,
            } = parts;
            let (control, ledger, plan) = startup.take().expect("the canonical entry binds once");
            let entry = match bind(start, control, ledger, plan) {
                Ok(entry) => entry,
                Err(cause) => {
                    *start_error_slot = Some(cause);
                    return;
                }
            };
            let mut entry = Some(entry);
            body(
                CanonicalInvocationParts {
                    startup: CanonicalBoundBodyStart {
                        entry: &mut entry,
                        unattached: unattached_slot,
                    },
                    output,
                    proof,
                    activation,
                    validation,
                },
                disposition,
            );
        });
        if let Some(cause) = start_error {
            drop(result);
            return Err(CanonicalBoundBodyError::Start(cause));
        }
        result
            .map(|finish| CanonicalBoundInvocationFinish { finish, unattached })
            .map_err(CanonicalBoundBodyError::Body)
    }

    fn has_valid_disposition(&self) -> bool {
        match self.pending.as_ref() {
            Some(PendingDisposition::Unattached) => {
                self.output.is_none()
                    && self.proof.is_none()
                    && self.never.is_none()
                    && self.activation_rejected.is_none()
                    && self.validation_rejected.is_none()
                    && self.interrupted.is_none()
                    && self.central_fallback.is_none()
            }
            Some(PendingDisposition::Operation(_)) => self.output.is_none() && self.proof.is_none(),
            Some(PendingDisposition::Candidate(_)) => self.output.is_some() && self.proof.is_some(),
            Some(PendingDisposition::NeverActivated(_)) => self.never.is_some(),
            Some(PendingDisposition::Rejected(_)) => self.activation_rejected.is_some(),
            Some(PendingDisposition::CentralRejected(_)) => self.validation_rejected.is_some(),
            Some(PendingDisposition::Interrupted(_)) => self.interrupted.is_some(),
            Some(PendingDisposition::Fallback(_)) => self.central_fallback.is_some(),
            None => false,
        }
    }

    /// Invokes a retained body once and disposes its captures before issuing finish authority.
    ///
    /// The body records one actual disposition and returns no mapped result. A body or capture
    /// unwind preserves the original panic while actual host values and storage are resolved
    /// under separate boundaries. Neither captures nor output/proof values need to be static.
    ///
    /// A short body permission cannot be saved outside its invocation:
    ///
    /// ```compile_fail,E0521
    /// use baozi_operation::__retained_store_integration::activation::CanonicalInvocationHost;
    /// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
    /// fn escape<O: OperationOwner, P: ProgressPhase, U: ProgressUnit, T, Proof>(
    ///     host: &mut CanonicalInvocationHost<O, P, U, T, Proof>,
    /// ) {
    ///     let mut escaped = None;
    ///     let _ = host.run_body(|parts, _disposition| escaped = Some(parts));
    ///     drop(escaped);
    /// }
    /// ```
    #[allow(
        clippy::type_complexity,
        reason = "the exact typed finish prevents erasing or substituting the invocation's roots and causes"
    )]
    pub fn run_body(
        &mut self,
        body: impl for<'call> FnMut(
            CanonicalBodyParts<'call, O, PP, U, T, Proof, EA, EV>,
            CanonicalDispositionSlot<'call, CK, CT, EK, ET>,
        ),
    ) -> Result<
        CanonicalInvocationFinish<'_, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>,
        CanonicalInvocationBodyError,
    > {
        let mut body = Some(body);
        let execution = if self.body_entered {
            None
        } else {
            self.body_entered = true;
            Some(catch_unwind(AssertUnwindSafe(|| {
                let (parts, disposition) = self.parts_and_disposition();
                body.as_mut().expect("the retained callback is installed")(parts, disposition);
            })))
        };
        let admitted = execution.is_some();
        if execution.as_ref().is_some_and(Result::is_err)
            || !admitted
            || !self.has_valid_disposition()
        {
            self.store
                .record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
        }
        let captures = catch_unwind(AssertUnwindSafe(|| drop(body.take())));
        if captures.is_err() {
            self.store
                .record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
        }
        let body_panic = execution.and_then(Result::err);
        let capture_panic = captures.err();
        if body_panic.is_some() || capture_panic.is_some() {
            let (_, cleanup_panic) = self.fail_body();
            let mut first = None;
            for payload in [body_panic, capture_panic, cleanup_panic]
                .into_iter()
                .flatten()
            {
                if first.is_none() {
                    first = Some(payload);
                } else {
                    discard_untrusted_panic_payload(payload);
                }
            }
            std::panic::resume_unwind(first.expect("a body or capture panic was observed"));
        }
        if !admitted || !self.has_valid_disposition() {
            let (error, cleanup_panic) = self.fail_body();
            if let Some(payload) = cleanup_panic {
                std::panic::resume_unwind(payload);
            }
            return Err(error);
        }
        Ok(CanonicalInvocationFinish { host: self })
    }

    fn fail_body(
        &mut self,
    ) -> (
        CanonicalInvocationBodyError,
        Option<Box<dyn std::any::Any + Send>>,
    ) {
        self.store
            .record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
        let panic = self.clear_owned_values();
        let containment = self
            .store
            .contain_payload_failure()
            .map(|cause| super::super::StoreActivationContainment { cause });
        (CanonicalInvocationBodyError { containment }, panic)
    }
}

impl<'inv, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
    CanonicalInvocationFinish<'inv, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Admits the consumer's complete immutable schema against this exact live root.
    ///
    /// This reads catalog identities, not live accounting totals. The later typed terminal
    /// proof qualifies all owned covers before projecting their post-commit zero state.
    /// Rejection consumes the finish permission and cleans the actual host before containment.
    pub fn admit_terminal_catalog(
        self,
        phases: &[crate::StorePhaseSemanticId],
        dimensions: &[crate::StoreDimensionSemanticId],
    ) -> Result<Self, CanonicalInvocationBodyError> {
        if self.host.store.matches_terminal_catalog(phases, dimensions) {
            return Ok(self);
        }
        let already_unwinding = std::thread::panicking();
        let (error, cleanup_panic) = self.host.fail_body();
        if let Some(payload) = cleanup_panic {
            if already_unwinding {
                discard_untrusted_panic_payload(payload);
            } else {
                std::panic::resume_unwind(payload);
            }
        }
        Err(error)
    }

    pub fn resume(
        self,
    ) -> CanonicalInvocationCompletion<'inv, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> {
        let host = self.host;
        let identity = host.store.invocation_identity();
        let pending = host
            .pending
            .take()
            .expect("finish retains its recorded disposition");
        let outcome = match pending {
            PendingDisposition::Unattached => {
                unreachable!("unattached facts require the bound finish")
            }
            PendingDisposition::Operation(pending) => {
                CanonicalInvocationOutcome::Operation(CanonicalOperationCompletion {
                    operation: host
                        .store
                        .resume_deferred_operation(pending.store)
                        .expect("the same root retains the deferred started operation"),
                    request: pending.request,
                })
            }
            PendingDisposition::Candidate(pending) => CanonicalInvocationOutcome::Candidate(
                CanonicalFinal::new(CanonicalCandidateStoreSession {
                    owner: PayloadOwner::new(
                        PayloadLoan::new(&mut host.proof, &host.order, identity),
                        PayloadOwner::new(
                            PayloadLoan::new(&mut host.output, &host.order, identity),
                            host.store
                                .resume_deferred_success(pending.store)
                                .expect("the same root retains the deferred success proof"),
                        ),
                    ),
                    destinations: pending.destinations,
                    _failure_work: pending.failure_work,
                }),
            ),
            PendingDisposition::NeverActivated(pending) => {
                let operation = host
                    .store
                    .resume_deferred_operation(pending.store)
                    .expect("the same root retains the never-activated operation");
                CanonicalInvocationOutcome::NeverActivated(CanonicalFinal::new(
                    NeverActivatedStoreSession {
                        owner: NeverActivatedOwner {
                            cause: Some(PayloadLoan::new(&mut host.never, &host.order, identity)),
                            authority: Some(if pending.fallback_only {
                                NeverActivatedAuthority::FallbackOnly(operation)
                            } else {
                                NeverActivatedAuthority::Operation(operation)
                            }),
                        },
                        failure_code: pending.failure_code,
                        outcome_evidence_plan: pending.outcome_evidence_plan,
                        failure_work: pending.failure_work,
                        outcome_evidence_destination_dimension: pending.destination,
                    },
                ))
            }
            PendingDisposition::Rejected(pending) => {
                let (raw, domain) = host
                    .store
                    .resume_deferred_unrooted_failure(pending.store)
                    .expect("the same root retains the unrooted failure assembly");
                CanonicalInvocationOutcome::Rejected(CanonicalFinal::new(
                    RejectedCanonicalStoreSession {
                        owner: UnrootedFailureEvidence {
                            owner: PayloadOwner::new(
                                PayloadLoan::new(
                                    &mut host.activation_rejected,
                                    &host.order,
                                    identity,
                                ),
                                raw,
                            ),
                            domain,
                            failure_code: pending.failure_code,
                            failure_work: pending.failure_work,
                            outcome_evidence_destination_dimension: pending.destination,
                        },
                    },
                ))
            }
            PendingDisposition::CentralRejected(pending) => {
                CanonicalInvocationOutcome::CentralRejected(CanonicalFinal::new(
                    CentrallyRejectedCanonicalStoreSession {
                        root: RootedFailureEvidence {
                            owner: PayloadOwner::new(
                                PayloadLoan::new(
                                    &mut host.validation_rejected,
                                    &host.order,
                                    identity,
                                ),
                                host.store.resume_deferred_failure(pending.store).expect(
                                    "the same root retains the centrally rejected assembly",
                                ),
                            ),
                            failure_work: pending.failure_work,
                            outcome_evidence_destination_dimension: pending.destination,
                            _lifecycle: std::marker::PhantomData,
                        },
                    },
                ))
            }
            PendingDisposition::Interrupted(pending) => CanonicalInvocationOutcome::Interrupted(
                CanonicalFinal::new(InterruptedCanonicalStoreSession {
                    root: RootedFailureEvidence {
                        owner: PayloadOwner::new(
                            PayloadLoan::new(&mut host.interrupted, &host.order, identity),
                            host.store
                                .resume_deferred_failure(pending.store)
                                .expect("the same root retains the interrupted assembly"),
                        ),
                        failure_work: pending.failure_work,
                        outcome_evidence_destination_dimension: pending.destination,
                        _lifecycle: std::marker::PhantomData,
                    },
                }),
            ),
            PendingDisposition::Fallback(pending) => {
                CanonicalInvocationOutcome::Fallback(CanonicalFinal::new(PendingMinimalFallback {
                    owner: PayloadOwner::new(
                        PayloadLoan::new(&mut host.central_fallback, &host.order, identity),
                        host.store
                            .resume_deferred_fallback(pending)
                            .expect("the same root retains the exact fallback-only state"),
                    ),
                }))
            }
        };
        CanonicalInvocationCompletion {
            outcome,
            generation: CanonicalGenerationSlots {
                interruption: CanonicalCauseSlot {
                    value: &mut host.freeze_interrupted,
                    order: &host.order,
                    identity,
                },
                rejection: CanonicalCauseSlot {
                    value: &mut host.generation_rejected,
                    order: &host.order,
                    identity,
                },
            },
            failure_fallback: CanonicalCauseSlot {
                value: &mut host.failure_fallback,
                order: &host.order,
                identity,
            },
        }
    }
}
