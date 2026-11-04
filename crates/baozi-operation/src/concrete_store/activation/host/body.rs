//! Bounded body capabilities that cannot finalize their enclosing invocation.

use super::super::{
    CanonicalActivationSessionPolicy, CanonicalDispositionSlot, CanonicalFailureRequirements,
    RetainedStoreSessionOutcomeOwner, StoreActivationContainment,
};
use crate::concrete_store::{
    BoundStoreOperation, CanonicalInheritanceSource, FreshStoreLineage, PublishedCanonicalStore,
    RetainedStoreBatchLimits, RetainedStoreControlCheckpoint, RetainedStoreOperation,
    RetainedStoreProducerHost, RetainedStoreReservation, RetainedStoreStartError,
    RetainedStoreStartPlan, StoreActionExecution, StoreDimensionKey, StoreExecutionServices,
    StoreExecutionWork, StoreInvocationStart, StoreLedgerPlan, StorePhaseKey, StoreRuntimeError,
};
use crate::{
    OperationControl, OperationOwner, PhysicalStorageAccount, ProgressPhase, ProgressUnit,
};

/// Startup permission lent only inside the retained invocation body.
///
/// This permission has no conversion into a raw startup or physical invocation root.
///
/// A body starts the restricted operation:
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalBodyOperation, CanonicalBodyStart,
/// };
/// use baozi_operation::__retained_store_integration::authority::{
///     RetainedStoreStartError, RetainedStoreStartPlan,
/// };
/// use baozi_operation::{
///     OperationControl, OperationOwner, PhysicalStorageAccount, ProgressPhase, ProgressUnit,
///     StoreLedgerPlan,
/// };
/// fn start<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     permission: CanonicalBodyStart<'inv, O, P, U>,
///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
///     account: &PhysicalStorageAccount, plan: RetainedStoreStartPlan,
/// ) -> Result<CanonicalBodyOperation<'inv, O, P, U>, RetainedStoreStartError> {
///     CanonicalBodyOperation::try_start(permission, control, ledger, account, plan)
/// }
/// ```
///
/// The same permission cannot start the unrestricted ordinary operation:
///
/// ```compile_fail,E0277
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalBodyOperation, CanonicalBodyStart,
/// };
/// use baozi_operation::__retained_store_integration::authority::{
///     RetainedStoreOperation, RetainedStoreStartError, RetainedStoreStartPlan,
/// };
/// use baozi_operation::{
///     OperationControl, OperationOwner, PhysicalStorageAccount, ProgressPhase, ProgressUnit,
///     StoreLedgerPlan,
/// };
/// fn start<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     permission: CanonicalBodyStart<'inv, O, P, U>,
///     control: OperationControl<P, U>, ledger: StoreLedgerPlan,
///     account: &PhysicalStorageAccount, plan: RetainedStoreStartPlan,
/// ) -> Result<RetainedStoreOperation<'inv, O, P, U>, RetainedStoreStartError> {
///     RetainedStoreOperation::try_start(permission, control, ledger, account, plan)
/// }
/// ```
pub struct CanonicalBodyStart<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    start: StoreInvocationStart<'inv, O, PP, U>,
}

impl<'inv, O, PP, U> CanonicalBodyStart<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) const fn new(start: StoreInvocationStart<'inv, O, PP, U>) -> Self {
        Self { start }
    }

    pub(super) fn into_start(self) -> StoreInvocationStart<'inv, O, PP, U> {
        self.start
    }
}

/// Short permission to the binding frame retained by the canonical entry adapter.
/// It cannot replace any bound inputs or take ownership of the frame.
///
/// Attachment returns only the operation restricted to this body:
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalBodyOperation, CanonicalBoundBodyStart,
/// };
/// use baozi_operation::__retained_store_integration::authority::RetainedStoreStartError;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn attach<'call, 'entry, 'account, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     entry: CanonicalBoundBodyStart<'call, 'entry, 'account, O, P, U>,
/// ) -> Result<CanonicalBodyOperation<'call, O, P, U>, RetainedStoreStartError> {
///     entry.attach()
/// }
/// ```
///
/// The original account cannot be replaced:
///
/// ```compile_fail,E0061
/// use baozi_operation::__retained_store_integration::activation::CanonicalBoundBodyStart;
/// use baozi_operation::{OperationOwner, PhysicalStorageAccount, ProgressPhase, ProgressUnit};
/// fn replace<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     entry: CanonicalBoundBodyStart<'_, '_, '_, O, P, U>,
///     other: &PhysicalStorageAccount,
/// ) {
///     let _ = entry.attach(other);
/// }
/// ```
///
/// The view exposes no startup-root extractor:
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::CanonicalBoundBodyStart;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn extract<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     entry: CanonicalBoundBodyStart<'_, '_, '_, O, P, U>,
/// ) {
///     let _ = entry.into_start();
/// }
/// ```
///
/// The invocation permission cannot move to a scoped thread:
///
/// ```compile_fail,E0277
/// use baozi_operation::__retained_store_integration::activation::CanonicalBoundBodyStart;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn move_to_thread<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     entry: CanonicalBoundBodyStart<'_, '_, '_, O, P, U>,
/// ) {
///     std::thread::scope(|scope| {
///         scope.spawn(move || drop(entry));
///     });
/// }
/// ```
pub struct CanonicalBoundBodyStart<'call, 'entry, 'account, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) entry: &'call mut Option<BoundStoreOperation<'entry, 'account, O, PP, U, L>>,
    pub(super) unattached:
        &'call mut Option<crate::concrete_store::deferred::DeferredUnattachedStore<'account>>,
}

impl<'call, O, PP, U, L> CanonicalBoundBodyStart<'call, '_, '_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Requests direct terminal facts after the real body and its captures have been disposed.
    pub fn request_unchanged<CK, CT, EK, ET>(
        self,
        disposition: CanonicalDispositionSlot<'call, CK, CT, EK, ET>,
    ) -> Result<(), StoreRuntimeError> {
        self.request_unattached(disposition, None)
    }

    /// Retains the requested failure before handing final facts back to the outer entry.
    pub fn request_failure<CK, CT, EK, ET>(
        self,
        disposition: CanonicalDispositionSlot<'call, CK, CT, EK, ET>,
        code: u32,
    ) -> Result<(), StoreRuntimeError> {
        self.request_unattached(
            disposition,
            Some(crate::retained::PreparationFailureCode::domain(code)),
        )
    }

    fn request_unattached<CK, CT, EK, ET>(
        self,
        disposition: CanonicalDispositionSlot<'call, CK, CT, EK, ET>,
        requested: Option<crate::retained::PreparationFailureCode>,
    ) -> Result<(), StoreRuntimeError> {
        let entry = self.entry.take().ok_or(StoreRuntimeError::FallbackOnly)?;
        if self.unattached.is_some() || !disposition.is_vacant_for(entry.invocation_identity()) {
            let _ = entry.defer_unattached(Some(
                crate::retained::PreparationFailureCode::product_graph(
                    super::super::ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE,
                ),
            ));
            return Err(StoreRuntimeError::OwnershipUnverified);
        }
        *self.unattached = Some(entry.defer_unattached(requested)?);
        disposition.install(super::driver::PendingDisposition::Unattached);
        Ok(())
    }

    pub fn with_action_execution<R, E: From<StoreRuntimeError>>(
        &mut self,
        work_dimension: StoreDimensionKey,
        body: impl for<'op> FnOnce(&mut StoreActionExecution<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        self.entry
            .as_mut()
            .ok_or(StoreRuntimeError::FallbackOnly)?
            .with_action_execution(work_dimension, body)
    }

    pub fn with_exclusive_execution<R, E: From<crate::concrete_store::StoreInvocationError>>(
        &mut self,
        body: impl for<'op> FnOnce(
            &mut crate::concrete_store::activation::StoreInvocation<'op, O, PP, U>,
        ) -> Result<R, E>,
    ) -> Result<R, E> {
        self.entry
            .as_mut()
            .ok_or_else(|| {
                E::from(crate::concrete_store::StoreInvocationError::from(
                    StoreRuntimeError::FallbackOnly,
                ))
            })?
            .with_exclusive_execution(body)
    }

    pub fn attach(
        self,
    ) -> Result<CanonicalBodyOperation<'call, O, PP, U, L>, RetainedStoreStartError> {
        let entry = self
            .entry
            .take()
            .ok_or(RetainedStoreStartError::InvocationAlreadyStarted)?;
        entry
            .attach()
            .map(|operation| CanonicalBodyOperation { operation })
    }
}

/// The invocation's existing operation with only bounded body capabilities.
///
/// Final preparation requires a consuming disposition request and the outer host's finish.
///
/// A body may request its terminal disposition:
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalBodyOperation, CanonicalDispositionSlot, StoreActivationContainment,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn request<'inv, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     operation: CanonicalBodyOperation<'inv, O, P, U>,
///     disposition: CanonicalDispositionSlot<'inv, (), (), (), ()>,
/// ) -> Result<(), StoreActivationContainment> {
///     operation.request_unchanged(disposition)
/// }
/// ```
///
/// It cannot prepare a terminal while its body or captures are still live:
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::CanonicalBodyOperation;
/// use baozi_operation::{FinalProgress, OperationOwner, ProgressPhase, ProgressUnit};
/// fn premature<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     operation: CanonicalBodyOperation<'_, O, P, U>, progress: FinalProgress<P, U>,
/// ) {
///     let _ = operation.prepare_evidence_only_success(progress);
/// }
/// ```
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::CanonicalBodyOperation;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn premature<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     operation: CanonicalBodyOperation<'_, O, P, U>,
/// ) {
///     let _ = operation.prepare_minimal_fallback(1);
/// }
/// ```
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::CanonicalBodyOperation;
/// use baozi_operation::__retained_store_integration::authority::RetainedStorePublicationPlan;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn premature<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     operation: CanonicalBodyOperation<'_, O, P, U>, plan: RetainedStorePublicationPlan,
/// ) {
///     let _ = operation.try_publication_assembly(plan);
/// }
/// ```
///
/// A bounded startup adapter can run inside the body and request outside-body completion:
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalBodyOperation, CanonicalBodyStart, CanonicalInvocationHost,
///     CanonicalInvocationOutcome,
/// };
/// use baozi_operation::{FinalProgress, OperationOwner, ProgressPhase, ProgressUnit};
/// fn run<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     host: &mut CanonicalInvocationHost<O, P, U, (), ()>,
///     mut start: impl for<'call> FnMut(CanonicalBodyStart<'call, O, P, U>)
///         -> CanonicalBodyOperation<'call, O, P, U>,
///     progress: FinalProgress<P, U>,
/// ) {
///     let finish = host.run_body(|parts, disposition| {
///         start(parts.startup).request_unchanged(disposition).unwrap();
///     }).unwrap();
///     if let CanonicalInvocationOutcome::Operation(operation) = finish.resume().outcome {
///         let _ = operation.finish(progress);
///     }
/// }
/// ```
///
/// The same adapter cannot extend the operation's body lifetime:
///
/// ```compile_fail,E0521
/// use baozi_operation::__retained_store_integration::activation::{
///     CanonicalBodyOperation, CanonicalBodyStart, CanonicalInvocationHost,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn escape<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     host: &mut CanonicalInvocationHost<O, P, U, (), ()>,
///     mut start: impl for<'call> FnMut(CanonicalBodyStart<'call, O, P, U>)
///         -> CanonicalBodyOperation<'call, O, P, U>,
/// ) {
///     let mut escaped = None;
///     let _ = host.run_body(|parts, _| escaped = Some(start(parts.startup)));
///     drop(escaped);
/// }
/// ```
#[must_use = "body execution must request a disposition or leave cleanup to its armed host"]
pub struct CanonicalBodyOperation<'inv, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: RetainedStoreOperation<'inv, O, PP, U, L>,
}

impl<'inv, O, PP, U> CanonicalBodyOperation<'inv, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn try_start(
        start: CanonicalBodyStart<'inv, O, PP, U>,
        control: OperationControl<PP, U>,
        ledger_plan: StoreLedgerPlan,
        account: &PhysicalStorageAccount,
        plan: RetainedStoreStartPlan,
    ) -> Result<Self, RetainedStoreStartError> {
        RetainedStoreOperation::try_start(start.start, control, ledger_plan, account, plan)
            .map(|operation| Self { operation })
    }

    pub fn try_start_derived<SourceRootKind, SourceRootTag>(
        start: CanonicalBodyStart<'inv, O, PP, U>,
        control: OperationControl<PP, U>,
        ledger_plan: StoreLedgerPlan,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        plan: RetainedStoreStartPlan,
    ) -> Result<
        CanonicalBodyOperation<'inv, O, PP, U, CanonicalInheritanceSource>,
        RetainedStoreStartError,
    > {
        RetainedStoreOperation::try_start_derived(start.start, control, ledger_plan, source, plan)
            .map(|operation| CanonicalBodyOperation { operation })
    }
}

impl<'inv, O, PP, U, L> CanonicalBodyOperation<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    #[allow(clippy::type_complexity)]
    pub fn execution(
        &mut self,
    ) -> Result<
        (
            StoreExecutionWork<'_, O>,
            StoreExecutionServices<'_, O, PP, U>,
        ),
        StoreRuntimeError,
    > {
        self.operation.execution()
    }

    /// Runs bounded geometry events while the outer invocation retains all final authority.
    pub fn with_action_execution<R, E: From<StoreRuntimeError>>(
        &mut self,
        work_dimension: StoreDimensionKey,
        body: impl for<'op> FnOnce(&mut StoreActionExecution<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        self.operation.with_action_execution(work_dimension, body)
    }

    pub fn reserve_retained_storage(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreRuntimeError> {
        self.operation
            .reserve_retained_storage(phase, dimension, bytes, allocations)
    }

    pub fn control_checkpoint(&mut self) -> RetainedStoreControlCheckpoint<'_, O, PP, U> {
        self.operation.control_checkpoint()
    }

    pub fn producer_host(&mut self) -> RetainedStoreProducerHost<'_, O, PP, U> {
        self.operation.producer_host()
    }

    /// Copies physical resource facts without lending the account or its release authority.
    pub fn physical_storage_snapshot(&self) -> crate::PhysicalStorageAccountSnapshot {
        self.operation.account().snapshot()
    }

    pub fn try_complete_phase(&self, phase: StorePhaseKey) -> Result<(), StoreRuntimeError> {
        self.operation.try_complete_phase(phase)
    }

    pub fn qualify_failure_validation_work(
        &mut self,
        ordinary_work_dimension: StoreDimensionKey,
        max_evidence_members: usize,
        validation_work_limit: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.operation.qualify_failure_validation_work(
            ordinary_work_dimension,
            max_evidence_members,
            validation_work_limit,
        )
    }

    pub fn into_session_outcome_owner(
        self,
        policy: CanonicalActivationSessionPolicy,
        outcome_evidence_writer_limits: RetainedStoreBatchLimits,
        failure_requirements: CanonicalFailureRequirements,
    ) -> Result<RetainedStoreSessionOutcomeOwner<'inv, O, PP, U, L>, StoreActivationContainment>
    {
        self.operation.into_session_outcome_owner(
            policy,
            outcome_evidence_writer_limits,
            failure_requirements,
        )
    }

    pub fn request_unchanged<CK, CT, EK, ET>(
        self,
        disposition: CanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
    ) -> Result<(), StoreActivationContainment> {
        self.operation.request_unchanged(disposition)
    }

    pub fn request_minimal_fallback<CK, CT, EK, ET>(
        self,
        disposition: CanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
        code: u32,
    ) -> Result<(), StoreActivationContainment> {
        self.operation.request_minimal_fallback(disposition, code)
    }
}
