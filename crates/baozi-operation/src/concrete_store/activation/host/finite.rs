//! The finite host keeps its concrete store and every original payload Option until exit.

#[cfg(test)]
mod activation_tests;
mod completion;
#[cfg(test)]
mod final_failure_tests;
pub use completion::{
    FiniteCanonicalInvocationCompletion, FiniteCanonicalInvocationOutcome,
    FiniteCanonicalOperationCompletion,
};

use super::super::payload_owner::drop_captured_value;
use super::super::{
    CanonicalActivationSessionPolicy, CanonicalFailureRequirements, FiniteSessionOutcomeOwner,
};
use super::*;
use crate::concrete_store::finite::{
    FiniteBoundStoreOperation, FiniteStoreExitCause, FiniteStoreExitError,
    FiniteStoreFinalizeCause, FiniteStoreInvocationRoot, bind_finite_store,
};
use crate::concrete_store::{
    CanonicalInheritanceSource, FiniteRetainedStoreOperation, FiniteRetainedStoreStartPlan,
    FiniteStoreFacts, FiniteStoreLedgerPlan, FreshStoreLineage,
    PreparedFiniteStoreEvidenceOnlyOutcome, PreparedFiniteStoreFallback, PublishedCanonicalStore,
    RetainedStoreReservation, RetainedStoreStartError, StoreActionExecution, StoreDimensionKey,
    StoreInvocation, StoreInvocationError,
};
use crate::retained::operation::finite_root::{
    FiniteDeferredUnattachedBinding, FiniteEntryAdmission,
};
use crate::{
    FinalProgress, OperationControl, PhysicalStorageAccount, RetainedStoreBatchLimits,
    StorePhaseKey,
};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};

type RootTypeMarker<CK, CT, EK, ET> = PhantomData<fn() -> (CK, CT, EK, ET)>;
type FiniteBodySessionResult<'inv, O, PP, U, L> =
    Result<FiniteSessionOutcomeOwner<'inv, O, PP, U, L>, FiniteStoreExitError<O, PP, U>>;
type FiniteHostLoan<'host, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> =
    &'host mut FiniteCanonicalInvocationHost<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>;
type FiniteHostLoanSlot<'host, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> =
    Option<FiniteHostLoan<'host, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>>;

type BoundFinishResult<'host, 'account, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> = Result<
    FiniteCanonicalBoundInvocationFinish<
        'host,
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
    FiniteStoreExitError<O, PP, U>,
>;
type FiniteBodySource<'account, L> = (
    &'account PhysicalStorageAccount,
    Option<&'account crate::retained::PublishedCanonicalOutput<crate::concrete_store::StoredBlock>>,
    L,
);

pub(in crate::concrete_store::activation) enum FiniteDisposition<CK, CT, EK, ET> {
    Unattached(FiniteDeferredUnattachedBinding),
    EvidenceOnly,
    Minimal { code: u32 },
    Candidate(driver::PendingCandidate<CK, CT, EK, ET>),
    NeverActivated(driver::PendingNeverActivated),
    Rejected(driver::PendingRejected),
    CentralRejected(driver::PendingRootedFailure<EK, ET>),
    Interrupted(driver::PendingRootedFailure<EK, ET>),
    Fallback(crate::concrete_store::deferred::DeferredStoreFallback),
}

/// Concrete fixed invocation owner; a spent host never installs another startup root.
pub struct FiniteCanonicalInvocationHost<
    O,
    PP,
    U,
    T,
    Proof,
    EA = (),
    EV = (),
    EG = (),
    CK = (),
    CT = (),
    EK = (),
    ET = (),
> where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store) never: Option<StoreNeverActivatedCause<EA>>,
    pub(in crate::concrete_store) activation_rejected: Option<StoreCanonicalRejection<EA>>,
    pub(in crate::concrete_store) validation_rejected: Option<StoreCanonicalRejection<EV>>,
    pub(in crate::concrete_store) interrupted: Option<StoreWorkError>,
    pub(in crate::concrete_store) freeze_interrupted: Option<CanonicalFreezeInterruption>,
    pub(in crate::concrete_store) generation_rejected: Option<StoreCanonicalRejection<EG>>,
    pub(in crate::concrete_store) failure_fallback: Option<StoreRuntimeError>,
    pub(in crate::concrete_store) central_fallback: Option<StoreRuntimeError>,
    pub(in crate::concrete_store) output: Option<T>,
    pub(in crate::concrete_store) proof: Option<Proof>,
    pub(in crate::concrete_store::activation) pending: Option<FiniteDisposition<CK, CT, EK, ET>>,
    _root_types: RootTypeMarker<CK, CT, EK, ET>,
    pub(in crate::concrete_store) body_entered: bool,
    pub(in crate::concrete_store::activation) order: Cell<PayloadOrder>,
    pub(in crate::concrete_store) store: Option<FiniteStoreInvocationRoot<O, PP, U>>,
}

pub type FiniteCanonicalBoundBodyParts<
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
    L = FreshStoreLineage,
> = CanonicalInvocationParts<
    'call,
    FiniteCanonicalBoundBodyStart<'call, 'entry, 'account, O, PP, U, L>,
    T,
    Proof,
    EA,
    EV,
>;

/// A forgotten permission cannot forget the stack entry's original admission.
pub struct FiniteCanonicalBoundBodyStart<'call, 'entry, 'account, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    entry: &'call mut Option<FiniteBoundStoreOperation<'entry, 'account, O, PP, U, L>>,
}

pub struct FiniteCanonicalDispositionSlot<'call, CK, CT, EK, ET> {
    value: &'call mut Option<FiniteDisposition<CK, CT, EK, ET>>,
    identity: *const (),
    _root_types: RootTypeMarker<CK, CT, EK, ET>,
}

impl<CK, CT, EK, ET> FiniteCanonicalDispositionSlot<'_, CK, CT, EK, ET> {
    pub(in crate::concrete_store::activation) fn is_vacant_for(&self, identity: *const ()) -> bool {
        self.identity == identity && self.value.is_none()
    }

    pub(in crate::concrete_store::activation) fn install_pending(
        self,
        disposition: FiniteDisposition<CK, CT, EK, ET>,
    ) {
        *self.value = Some(disposition);
    }

    pub(in crate::concrete_store::activation) fn install(
        self,
        identity: *const (),
        disposition: FiniteDisposition<CK, CT, EK, ET>,
    ) -> Result<(), StoreRuntimeError> {
        if self.identity != identity || self.value.is_some() {
            return Err(StoreRuntimeError::OwnershipUnverified);
        }
        *self.value = Some(disposition);
        Ok(())
    }
}

impl<'call, O, PP, U, L> FiniteCanonicalBoundBodyStart<'call, '_, '_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn with_exclusive_execution<R, E: From<StoreInvocationError>>(
        &mut self,
        body: impl for<'op> FnOnce(&mut StoreInvocation<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        self.entry
            .as_mut()
            .ok_or(StoreInvocationError::from(StoreRuntimeError::FallbackOnly))?
            .with_exclusive_execution(body)
    }

    pub fn with_action_execution<R, E: From<StoreRuntimeError>>(
        &mut self,
        dimension: StoreDimensionKey,
        body: impl for<'op> FnOnce(&mut StoreActionExecution<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        self.entry
            .as_mut()
            .ok_or(StoreRuntimeError::FallbackOnly)?
            .with_action_execution(dimension, body)
    }

    pub fn attach(
        self,
    ) -> Result<FiniteCanonicalBodyOperation<'call, O, PP, U, L>, RetainedStoreStartError> {
        self.entry
            .take()
            .ok_or(RetainedStoreStartError::Runtime(
                StoreRuntimeError::FallbackOnly,
            ))?
            .attach()
            .map(|operation| FiniteCanonicalBodyOperation { operation })
    }

    pub fn request_unchanged<CK, CT, EK, ET>(
        self,
        disposition: FiniteCanonicalDispositionSlot<'call, CK, CT, EK, ET>,
    ) -> Result<(), StoreRuntimeError> {
        self.request_unattached(disposition, None)
    }

    pub fn request_failure<CK, CT, EK, ET>(
        self,
        disposition: FiniteCanonicalDispositionSlot<'call, CK, CT, EK, ET>,
        code: u32,
    ) -> Result<(), StoreRuntimeError> {
        self.request_unattached(
            disposition,
            Some(crate::retained::PreparationFailureCode::domain(code)),
        )
    }

    fn request_unattached<CK, CT, EK, ET>(
        self,
        disposition: FiniteCanonicalDispositionSlot<'call, CK, CT, EK, ET>,
        requested: Option<crate::retained::PreparationFailureCode>,
    ) -> Result<(), StoreRuntimeError> {
        let entry = self.entry.take().ok_or(StoreRuntimeError::FallbackOnly)?;
        let identity = entry.invocation_identity();
        let binding = entry.defer_unattached(requested)?;
        disposition.install(identity, FiniteDisposition::Unattached(binding))
    }
}

/// The original finite operation with only bounded body capabilities.
///
/// Final preparation belongs to the outer host after the body and its captures are disposed.
/// The actual finite startup permission attaches this body operation:
///
/// ```
/// use baozi_operation::__retained_store_integration::activation::{
///     FiniteCanonicalBodyOperation, FiniteCanonicalBoundBodyStart,
///     FiniteCanonicalDispositionSlot,
/// };
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit, StoreRuntimeError};
/// fn request<'call, 'entry, 'account, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     start: FiniteCanonicalBoundBodyStart<'call, 'entry, 'account, O, P, U>,
///     disposition: FiniteCanonicalDispositionSlot<'call, (), (), (), ()>,
/// ) -> Result<(), StoreRuntimeError> {
///     let operation: FiniteCanonicalBodyOperation<'call, O, P, U> = start.attach().unwrap();
///     operation.request_unchanged(disposition)
/// }
/// ```
///
/// Attaching inside the body cannot open publication assembly:
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::FiniteCanonicalBoundBodyStart;
/// use baozi_operation::__retained_store_integration::authority::RetainedStorePublicationPlan;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn premature<'call, 'entry, 'account, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     start: FiniteCanonicalBoundBodyStart<'call, 'entry, 'account, O, P, U>,
///     plan: RetainedStorePublicationPlan,
/// ) {
///     let operation = start.attach().unwrap();
///     let _ = operation.try_publication_assembly(plan);
/// }
/// ```
///
/// Nor can it prepare an evidence-only success or minimal fallback:
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::FiniteCanonicalBoundBodyStart;
/// use baozi_operation::{FinalProgress, OperationOwner, ProgressPhase, ProgressUnit};
/// fn premature<'call, 'entry, 'account, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     start: FiniteCanonicalBoundBodyStart<'call, 'entry, 'account, O, P, U>,
///     progress: FinalProgress<P, U>,
/// ) {
///     let operation = start.attach().unwrap();
///     let _ = operation.prepare_evidence_only_success(progress);
/// }
/// ```
///
/// ```compile_fail,E0599
/// use baozi_operation::__retained_store_integration::activation::FiniteCanonicalBoundBodyStart;
/// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
/// fn premature<'call, 'entry, 'account, O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
///     start: FiniteCanonicalBoundBodyStart<'call, 'entry, 'account, O, P, U>,
/// ) {
///     let operation = start.attach().unwrap();
///     let _ = operation.prepare_minimal_fallback(1);
/// }
/// ```
#[must_use = "body execution must request a disposition or leave cleanup to its armed host"]
pub struct FiniteCanonicalBodyOperation<'inv, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: FiniteRetainedStoreOperation<'inv, O, PP, U, L>,
}

impl<'inv, O, PP, U, L> FiniteCanonicalBodyOperation<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn with_exclusive_execution<R, E: From<StoreInvocationError>>(
        &mut self,
        body: impl for<'op> FnOnce(&mut StoreInvocation<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        self.operation.with_exclusive_execution(body)
    }

    pub fn with_action_execution<R, E: From<StoreRuntimeError>>(
        &mut self,
        dimension: StoreDimensionKey,
        body: impl for<'op> FnOnce(&mut StoreActionExecution<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        self.operation.with_action_execution(dimension, body)
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

    pub fn try_complete_phase(&mut self, phase: StorePhaseKey) -> Result<(), StoreRuntimeError> {
        self.operation.try_complete_phase(phase)
    }

    pub fn physical_storage_snapshot(&mut self) -> crate::PhysicalStorageAccountSnapshot {
        self.operation.physical_storage_snapshot()
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
    ) -> FiniteBodySessionResult<'inv, O, PP, U, L> {
        self.operation.into_session_outcome_owner(
            policy,
            outcome_evidence_writer_limits,
            failure_requirements,
        )
    }

    pub fn request_unchanged<CK, CT, EK, ET>(
        mut self,
        disposition: FiniteCanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
    ) -> Result<(), StoreRuntimeError> {
        let identity = self.operation.invocation_identity();
        self.operation.defer_body(None)?;
        disposition.install(identity, FiniteDisposition::EvidenceOnly)
    }

    pub fn request_minimal_fallback<CK, CT, EK, ET>(
        mut self,
        disposition: FiniteCanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
        code: u32,
    ) -> Result<(), StoreRuntimeError> {
        let identity = self.operation.invocation_identity();
        self.operation
            .defer_body(Some(crate::retained::PreparationFailureCode::domain(code)))?;
        disposition.install(identity, FiniteDisposition::Minimal { code })
    }
}

impl<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
    FiniteCanonicalInvocationHost<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn new() -> Self {
        Self {
            never: None,
            activation_rejected: None,
            validation_rejected: None,
            interrupted: None,
            freeze_interrupted: None,
            generation_rejected: None,
            failure_fallback: None,
            central_fallback: None,
            output: None,
            proof: None,
            pending: None,
            body_entered: false,
            order: Cell::new(PayloadOrder::ProofBeforeOutput),
            _root_types: PhantomData,
            store: Some(FiniteStoreInvocationRoot::new()),
        }
    }

    /// Runs one body, preserving any returned owned error through capture and payload cleanup.
    ///
    /// Existing owned session errors can be propagated with `?`. A returned error prevents
    /// every finish, including a request already recorded by the same body. Returning an error
    /// alone does not record another failure or poison an otherwise healthy owning root.
    pub fn run_bound_body<'account>(
        &mut self,
        control: OperationControl<PP, U>,
        ledger: FiniteStoreLedgerPlan,
        account: &'account PhysicalStorageAccount,
        plan: FiniteRetainedStoreStartPlan,
        body: impl for<'call, 'entry> FnMut(
            FiniteCanonicalBoundBodyParts<'call, 'entry, 'account, O, PP, U, T, Proof, EA, EV>,
            FiniteCanonicalDispositionSlot<'call, CK, CT, EK, ET>,
        ) -> Result<(), FiniteStoreExitError<O, PP, U>>,
    ) -> BoundFinishResult<'_, 'account, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> {
        self.run_bound_body_with(
            control,
            ledger,
            (account, None, FreshStoreLineage { _private: () }),
            plan,
            body,
        )
    }

    pub fn run_bound_derived_body<'account, SK, ST>(
        &mut self,
        control: OperationControl<PP, U>,
        ledger: FiniteStoreLedgerPlan,
        source: &'account PublishedCanonicalStore<SK, ST>,
        plan: FiniteRetainedStoreStartPlan,
        body: impl for<'call, 'entry> FnMut(
            FiniteCanonicalBoundBodyParts<
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
            FiniteCanonicalDispositionSlot<'call, CK, CT, EK, ET>,
        ) -> Result<(), FiniteStoreExitError<O, PP, U>>,
    ) -> BoundFinishResult<'_, 'account, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> {
        let source = source.share_source();
        self.run_bound_body_with(
            control,
            ledger,
            (
                source.account(),
                Some(source),
                CanonicalInheritanceSource { _private: () },
            ),
            plan,
            body,
        )
    }

    fn run_bound_body_with<'account, L>(
        &mut self,
        control: OperationControl<PP, U>,
        ledger: FiniteStoreLedgerPlan,
        source: FiniteBodySource<'account, L>,
        plan: FiniteRetainedStoreStartPlan,
        body: impl for<'call, 'entry> FnMut(
            FiniteCanonicalBoundBodyParts<'call, 'entry, 'account, O, PP, U, T, Proof, EA, EV, L>,
            FiniteCanonicalDispositionSlot<'call, CK, CT, EK, ET>,
        ) -> Result<(), FiniteStoreExitError<O, PP, U>>,
    ) -> BoundFinishResult<'_, 'account, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> {
        let (account, source, lineage) = source;
        let admitted = !self.body_entered && self.store.is_some();
        if admitted {
            self.body_entered = true;
        }
        let mut exit = FiniteBodyExitGuard {
            host: Some(self),
            body: Some(body),
            control: Some(control),
            inputs: Some((ledger, plan, lineage)),
            admission: Some(FiniteEntryAdmission::new(account)),
            returned_error: None,
            cleanup_fault: false,
            first_panic: None,
        };
        let mut start_error = None;
        let execution = admitted.then(|| {
            catch_unwind(AssertUnwindSafe(|| {
                let FiniteBodyExitGuard {
                    host,
                    body,
                    control,
                    inputs,
                    admission,
                    returned_error,
                    ..
                } = &mut exit;
                let host = host
                    .as_deref_mut()
                    .expect("the original finite Host loan remains armed");
                let store = host
                    .store
                    .as_mut()
                    .expect("the original finite root is installed");
                let identity = store.invocation_identity();
                let (ledger, plan, lineage) = inputs.take().expect("the finite entry binds once");
                let bound = bind_finite_store(
                    store,
                    admission
                        .as_mut()
                        .expect("the original finite admission remains armed"),
                    control,
                    ledger,
                    source,
                    plan,
                    lineage,
                );
                let entry = match bound {
                    Ok(entry) => entry,
                    Err(cause) => {
                        start_error = Some(cause);
                        return;
                    }
                };
                let mut entry = Some(entry);
                let result = body.as_mut().expect("the finite callback is invoked once")(
                    CanonicalInvocationParts {
                        startup: FiniteCanonicalBoundBodyStart { entry: &mut entry },
                        output: CanonicalOutputSlot {
                            value: &mut host.output,
                            order: &host.order,
                            identity,
                        },
                        proof: CanonicalProofSlot {
                            value: &mut host.proof,
                            order: &host.order,
                            identity,
                            fallback: Some(CanonicalCauseSlot {
                                value: &mut host.central_fallback,
                                order: &host.order,
                                identity,
                            }),
                        },
                        activation: CanonicalActivationSlots {
                            never: CanonicalCauseSlot {
                                value: &mut host.never,
                                order: &host.order,
                                identity,
                            },
                            rejection: CanonicalCauseSlot {
                                value: &mut host.activation_rejected,
                                order: &host.order,
                                identity,
                            },
                        },
                        validation: CanonicalValidationSlots {
                            rejection: CanonicalCauseSlot {
                                value: &mut host.validation_rejected,
                                order: &host.order,
                                identity,
                            },
                            interruption: CanonicalCauseSlot {
                                value: &mut host.interrupted,
                                order: &host.order,
                                identity,
                            },
                        },
                    },
                    FiniteCanonicalDispositionSlot {
                        value: &mut host.pending,
                        identity,
                        _root_types: PhantomData,
                    },
                );
                // Own the error before the original local entry is disposed.
                if let Err(error) = result {
                    *returned_error = Some(error);
                }
            }))
        });
        if let Some(payload) = execution.and_then(Result::err) {
            exit.record_fault();
            exit.merge_panic(Some(payload));
        }
        // A returned owned error is already the selected failure, regardless of its disposition.
        if exit.returned_error.is_none()
            && (!admitted
                || !exit
                    .host
                    .as_ref()
                    .expect("the Host loan remains armed")
                    .has_valid_disposition())
        {
            exit.host
                .as_deref_mut()
                .expect("the Host loan remains armed")
                .record_payload_failure();
        }
        let rejected_domain = exit.dispose_captures();
        if exit.first_panic.is_some() {
            exit.cleanup_failed_body();
            exit.resume_panic();
        }
        if exit.returned_error.is_some() {
            return Err(exit.take_returned_error());
        }
        let start_error = start_error.or_else(|| {
            exit.host
                .as_ref()
                .and_then(|host| host.store.as_ref())
                .and_then(FiniteStoreInvocationRoot::startup_error)
        });
        if let Some(cause) = start_error {
            exit.returned_error = Some(
                exit.host
                    .as_deref_mut()
                    .expect("the Host loan remains armed")
                    .take_error(FiniteStoreExitCause::Start(cause))
                    .with_rejected_domain(rejected_domain),
            );
            return Err(exit.take_returned_error());
        }
        if !admitted
            || !exit
                .host
                .as_ref()
                .expect("the Host loan remains armed")
                .has_valid_disposition()
        {
            exit.cleanup_fault = true;
            exit.returned_error = Some(
                exit.host
                    .as_deref_mut()
                    .expect("the Host loan remains armed")
                    .take_error(FiniteStoreExitCause::Body(
                        driver::CanonicalInvocationBodyError::invalid_disposition(),
                    ))
                    .with_rejected_domain(rejected_domain),
            );
            return Err(exit.take_returned_error());
        }
        exit.take_finish()
    }

    fn has_valid_disposition(&self) -> bool {
        match self.pending.as_ref() {
            Some(
                FiniteDisposition::Unattached(_)
                | FiniteDisposition::EvidenceOnly
                | FiniteDisposition::Minimal { .. },
            ) => {
                self.output.is_none()
                    && self.proof.is_none()
                    && self.never.is_none()
                    && self.activation_rejected.is_none()
                    && self.validation_rejected.is_none()
                    && self.interrupted.is_none()
                    && self.central_fallback.is_none()
            }
            Some(FiniteDisposition::Candidate(_)) => self.output.is_some() && self.proof.is_some(),
            Some(FiniteDisposition::NeverActivated(_)) => self.never.is_some(),
            Some(FiniteDisposition::Rejected(_)) => self.activation_rejected.is_some(),
            Some(FiniteDisposition::CentralRejected(_)) => self.validation_rejected.is_some(),
            Some(FiniteDisposition::Interrupted(_)) => self.interrupted.is_some(),
            Some(FiniteDisposition::Fallback(_)) => self.central_fallback.is_some(),
            None => false,
        }
    }

    pub(in crate::concrete_store) fn record_payload_failure(&mut self) {
        if let Some(store) = &mut self.store {
            store.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
        }
    }

    fn contain_payload_failure(&mut self) {
        if let Some(store) = self.store.as_mut() {
            store.contain_payload_failure();
        }
    }

    pub(in crate::concrete_store) fn take_error(
        &mut self,
        cause: FiniteStoreExitCause,
    ) -> FiniteStoreExitError<O, PP, U> {
        match self.store.take() {
            Some(store) => store.into_error(cause),
            None => FiniteStoreExitError::spent(cause),
        }
    }

    pub(in crate::concrete_store) fn clear_owned_values(
        &mut self,
    ) -> Option<Box<dyn std::any::Any + Send>> {
        self.pending = None;
        let store = &mut self.store;
        FinitePayloadFields {
            never: &mut self.never,
            activation_rejected: &mut self.activation_rejected,
            validation_rejected: &mut self.validation_rejected,
            interrupted: &mut self.interrupted,
            freeze_interrupted: &mut self.freeze_interrupted,
            generation_rejected: &mut self.generation_rejected,
            failure_fallback: &mut self.failure_fallback,
            central_fallback: &mut self.central_fallback,
            output: &mut self.output,
            proof: &mut self.proof,
            order: &self.order,
        }
        .clear(|| {
            if let Some(store) = store {
                store.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
            }
        })
    }
}

/// The original error outlives entry teardown, captures, payloads and both owning roots.
struct FiniteBodyExitGuard<
    'host,
    'account,
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    T,
    Proof,
    EA,
    EV,
    EG,
    CK,
    CT,
    EK,
    ET,
    L,
    B,
> {
    host: FiniteHostLoanSlot<'host, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>,
    body: Option<B>,
    control: Option<OperationControl<PP, U>>,
    inputs: Option<(FiniteStoreLedgerPlan, FiniteRetainedStoreStartPlan, L)>,
    admission: Option<FiniteEntryAdmission<'account>>,
    returned_error: Option<FiniteStoreExitError<O, PP, U>>,
    cleanup_fault: bool,
    first_panic: Option<Box<dyn std::any::Any + Send>>,
}

impl<'host, 'account, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET, L, B>
    FiniteBodyExitGuard<'host, 'account, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET, L, B>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn record_owners(
        root: Option<&mut FiniteStoreInvocationRoot<O, PP, U>>,
        error: Option<&mut FiniteStoreExitError<O, PP, U>>,
        fault: &mut bool,
    ) {
        *fault = true;
        if let Some(root) = root {
            root.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
        }
        if let Some(error) = error {
            error.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
        }
    }

    fn record_fault(&mut self) {
        Self::record_owners(
            self.host
                .as_deref_mut()
                .and_then(|host| host.store.as_mut()),
            self.returned_error.as_mut(),
            &mut self.cleanup_fault,
        );
    }

    fn merge_panic(&mut self, panic: Option<Box<dyn std::any::Any + Send>>) {
        if let Some(payload) = panic {
            if self.first_panic.is_some() {
                discard_untrusted_panic_payload(payload);
            } else {
                self.first_panic = Some(payload);
            }
        }
    }

    fn dispose_captures(&mut self) -> Option<crate::concrete_store::StoreKeyDomain> {
        let Self {
            host,
            body,
            control,
            inputs,
            returned_error,
            cleanup_fault,
            first_panic,
            ..
        } = self;
        let mut record = || {
            Self::record_owners(
                host.as_deref_mut().and_then(|host| host.store.as_mut()),
                returned_error.as_mut(),
                cleanup_fault,
            )
        };
        drop_captured_value(body, first_panic, &mut record);
        drop_captured_value(control, first_panic, &mut record);
        let mut rejected_domain = None;
        if let Some((ledger, plan, lineage)) = inputs.take() {
            rejected_domain = Some(ledger.into_parts().0);
            let mut plan = Some(plan);
            let mut lineage = Some(lineage);
            drop_captured_value(&mut plan, first_panic, &mut record);
            drop_captured_value(&mut lineage, first_panic, &mut record);
        }
        rejected_domain
    }

    fn clear_payloads(&mut self) {
        let Self {
            host,
            returned_error,
            cleanup_fault,
            ..
        } = self;
        let Some(host) = host.as_deref_mut() else {
            return;
        };
        host.pending = None;
        let store = &mut host.store;
        let panic = FinitePayloadFields {
            never: &mut host.never,
            activation_rejected: &mut host.activation_rejected,
            validation_rejected: &mut host.validation_rejected,
            interrupted: &mut host.interrupted,
            freeze_interrupted: &mut host.freeze_interrupted,
            generation_rejected: &mut host.generation_rejected,
            failure_fallback: &mut host.failure_fallback,
            central_fallback: &mut host.central_fallback,
            output: &mut host.output,
            proof: &mut host.proof,
            order: &host.order,
        }
        .clear(|| Self::record_owners(store.as_mut(), returned_error.as_mut(), cleanup_fault));
        self.merge_panic(panic);
    }

    fn contain_error(&mut self) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
            if let Some(error) = self.returned_error.as_mut() {
                error.contain_payload_failure();
            }
        })) {
            self.record_fault();
            self.merge_panic(Some(payload));
        }
    }

    fn contain_faults(&mut self) {
        if !self.cleanup_fault {
            return;
        }
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
            if let Some(host) = self.host.as_deref_mut() {
                host.contain_payload_failure();
            }
        })) {
            self.record_fault();
            self.merge_panic(Some(payload));
        }
        self.contain_error();
    }

    fn cleanup_failed_body(&mut self) {
        self.dispose_captures();
        self.clear_payloads();
        // Admission owns only fixed metadata and references; clearing it invokes no destructor.
        self.admission = None;
        let error_containment_attempted = self.cleanup_fault;
        self.contain_faults();
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
            if let Some(host) = self.host.as_deref_mut() {
                drop(host.store.take());
            }
        })) {
            self.record_fault();
            self.merge_panic(Some(payload));
            if !error_containment_attempted {
                self.contain_error();
            }
        }
    }

    fn retire_error(&mut self) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(self.returned_error.take()))) {
            self.record_fault();
            self.merge_panic(Some(payload));
        }
    }

    fn resume_panic(&mut self) -> ! {
        self.retire_error();
        let payload = self
            .first_panic
            .take()
            .expect("the original finite body panic is retained");
        self.host.take();
        std::panic::resume_unwind(payload);
    }

    fn take_returned_error(&mut self) -> FiniteStoreExitError<O, PP, U> {
        self.cleanup_failed_body();
        if self.first_panic.is_some() {
            self.resume_panic();
        }
        let error = self
            .returned_error
            .take()
            .expect("the exact finite body error is retained");
        self.host.take();
        error
    }

    fn take_finish(
        &mut self,
    ) -> BoundFinishResult<'host, 'account, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> {
        Ok(FiniteCanonicalBoundInvocationFinish {
            host: self
                .host
                .take()
                .expect("the original Host loan transfers once"),
            admission: self
                .admission
                .take()
                .expect("the original admission transfers once"),
        })
    }
}

impl<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET, L, B> Drop
    for FiniteBodyExitGuard<'_, '_, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET, L, B>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if self.host.is_none() {
            return;
        }
        let already_unwinding = std::thread::panicking();
        if already_unwinding {
            self.record_fault();
        }
        self.cleanup_failed_body();
        self.retire_error();
        self.host.take();
        if let Some(payload) = self.first_panic.take() {
            if already_unwinding {
                discard_untrusted_panic_payload(payload);
            } else {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

/// Short views keep cleanup on the actual Host Options while a restored store loan is live.
struct FinitePayloadFields<'host, T, Proof, EA, EV, EG> {
    never: &'host mut Option<StoreNeverActivatedCause<EA>>,
    activation_rejected: &'host mut Option<StoreCanonicalRejection<EA>>,
    validation_rejected: &'host mut Option<StoreCanonicalRejection<EV>>,
    interrupted: &'host mut Option<StoreWorkError>,
    freeze_interrupted: &'host mut Option<CanonicalFreezeInterruption>,
    generation_rejected: &'host mut Option<StoreCanonicalRejection<EG>>,
    failure_fallback: &'host mut Option<StoreRuntimeError>,
    central_fallback: &'host mut Option<StoreRuntimeError>,
    output: &'host mut Option<T>,
    proof: &'host mut Option<Proof>,
    order: &'host Cell<PayloadOrder>,
}

impl<T, Proof, EA, EV, EG> FinitePayloadFields<'_, T, Proof, EA, EV, EG> {
    fn clear(self, mut record: impl FnMut()) -> Option<Box<dyn std::any::Any + Send>> {
        *self.central_fallback = None;
        *self.failure_fallback = None;
        *self.interrupted = None;
        *self.freeze_interrupted = None;
        let mut first = None;
        drop_captured_value(self.never, &mut first, &mut record);
        drop_captured_value(self.activation_rejected, &mut first, &mut record);
        drop_captured_value(self.validation_rejected, &mut first, &mut record);
        drop_captured_value(self.generation_rejected, &mut first, &mut record);
        match self.order.get() {
            PayloadOrder::ProofBeforeOutput => {
                drop_captured_value(self.proof, &mut first, &mut record);
                drop_captured_value(self.output, &mut first, &mut record);
            }
            PayloadOrder::OutputBeforeProof => {
                drop_captured_value(self.output, &mut first, &mut record);
                drop_captured_value(self.proof, &mut first, &mut record);
            }
        }
        first
    }
}

impl<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> Default
    for FiniteCanonicalInvocationHost<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> Drop
    for FiniteCanonicalInvocationHost<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        let unwinding = std::thread::panicking();
        let panic = self.clear_owned_values();
        if panic.is_some() {
            self.record_payload_failure();
            self.contain_payload_failure();
        }
        drop(self.store.take());
        if let Some(payload) = panic {
            if unwinding {
                discard_untrusted_panic_payload(payload);
            } else {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

pub struct FiniteCanonicalBoundInvocationFinish<
    'host,
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
    pub(in crate::concrete_store) host:
        FiniteHostLoan<'host, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>,
    pub(in crate::concrete_store) admission: FiniteEntryAdmission<'account>,
}

pub enum FiniteCanonicalOperationPublication {
    EvidenceOnly(PreparedFiniteStoreEvidenceOnlyOutcome),
    MinimalFallback(PreparedFiniteStoreFallback),
}

impl<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
    FiniteCanonicalBoundInvocationFinish<'_, '_, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn finish_unattached(
        mut self,
        progress: FinalProgress<PP, U>,
    ) -> Result<FiniteStoreFacts, FiniteStoreExitError<O, PP, U>> {
        let Some(FiniteDisposition::Unattached(binding)) = self.host.pending.take() else {
            self.host.record_payload_failure();
            let panic = self.host.clear_owned_values();
            self.host.contain_payload_failure();
            let error = self.host.take_error(FiniteStoreExitCause::Body(
                driver::CanonicalInvocationBodyError::invalid_disposition(),
            ));
            if let Some(payload) = panic {
                drop(error);
                std::panic::resume_unwind(payload);
            }
            return Err(error);
        };
        let result = self
            .host
            .store
            .as_mut()
            .expect("unattached finite finish retains its root")
            .finish_unattached(binding, &mut self.admission, progress);
        match result {
            Ok(facts) => {
                drop(self.host.store.take());
                Ok(facts)
            }
            Err(failure) => {
                let cause = FiniteStoreExitCause::Finalize(FiniteStoreFinalizeCause::Unattached);
                Err(self.host.take_error(cause).with_unattached(failure))
            }
        }
    }

    pub fn finish_attached(
        self,
        progress: FinalProgress<PP, U>,
    ) -> Result<FiniteCanonicalOperationPublication, FiniteStoreExitError<O, PP, U>> {
        let request = self.host.pending.take();
        let store = self
            .host
            .store
            .as_mut()
            .expect("attached finite finish retains its original root");
        let result = match request {
            Some(FiniteDisposition::EvidenceOnly) => store
                .fields()
                .prepare_evidence_only(progress)
                .map(FiniteCanonicalOperationPublication::EvidenceOnly),
            Some(FiniteDisposition::Minimal { code }) => store
                .fields()
                .prepare_minimal(code)
                .map(FiniteCanonicalOperationPublication::MinimalFallback),
            _ => {
                self.host.record_payload_failure();
                let panic = self.host.clear_owned_values();
                self.host.contain_payload_failure();
                let error = self.host.take_error(FiniteStoreExitCause::Body(
                    driver::CanonicalInvocationBodyError::invalid_disposition(),
                ));
                if let Some(payload) = panic {
                    drop(error);
                    std::panic::resume_unwind(payload);
                }
                Err(error)
            }
        };
        drop(self.host.store.take());
        result
    }
}
