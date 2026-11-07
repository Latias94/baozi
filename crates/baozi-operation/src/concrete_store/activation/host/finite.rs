//! The finite host keeps its concrete store and every original payload Option until exit.

use super::super::payload_owner::drop_captured_value;
use super::*;
use crate::concrete_store::finite::{
    FiniteBoundStoreOperation, FiniteStoreExitCause, FiniteStoreExitError,
    FiniteStoreFinalizeCause, FiniteStoreInvocationRoot, bind_finite_store,
};
use crate::concrete_store::{
    CanonicalInheritanceSource, FiniteRetainedStoreOperation, FiniteRetainedStoreStartPlan,
    FiniteStoreFacts, FiniteStoreLedgerPlan, FreshStoreLineage,
    PreparedFiniteStoreEvidenceOnlyOutcome, PreparedFiniteStoreFallback, PublishedCanonicalStore,
    RetainedStoreStartError, StoreActionExecution, StoreDimensionKey, StoreInvocation,
    StoreInvocationError,
};
use crate::retained::operation::finite_root::{
    FiniteDeferredUnattachedBinding, FiniteEntryAdmission,
};
use crate::{FinalProgress, OperationControl, PhysicalStorageAccount};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};

type RootTypeMarker<CK, CT, EK, ET> = PhantomData<fn() -> (CK, CT, EK, ET)>;
type FiniteHostLoan<'host, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> =
    &'host mut FiniteCanonicalInvocationHost<O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET>;
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

pub(in crate::concrete_store) enum FiniteDisposition {
    Unattached(FiniteDeferredUnattachedBinding),
    EvidenceOnly,
    Minimal { code: u32 },
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
    pub(in crate::concrete_store) pending: Option<FiniteDisposition>,
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
    value: &'call mut Option<FiniteDisposition>,
    identity: *const (),
    _root_types: RootTypeMarker<CK, CT, EK, ET>,
}

impl<CK, CT, EK, ET> FiniteCanonicalDispositionSlot<'_, CK, CT, EK, ET> {
    fn install(
        self,
        identity: *const (),
        disposition: FiniteDisposition,
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
    ) -> Result<FiniteRetainedStoreOperation<'call, O, PP, U, L>, RetainedStoreStartError> {
        self.entry
            .take()
            .ok_or(RetainedStoreStartError::Runtime(
                StoreRuntimeError::FallbackOnly,
            ))?
            .attach()
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

impl<'inv, O, PP, U, L> FiniteRetainedStoreOperation<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn request_unchanged<CK, CT, EK, ET>(
        mut self,
        disposition: FiniteCanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
    ) -> Result<(), StoreRuntimeError> {
        let identity = self.invocation_identity();
        self.defer_body(None)?;
        disposition.install(identity, FiniteDisposition::EvidenceOnly)
    }

    pub fn request_minimal_fallback<CK, CT, EK, ET>(
        mut self,
        disposition: FiniteCanonicalDispositionSlot<'inv, CK, CT, EK, ET>,
        code: u32,
    ) -> Result<(), StoreRuntimeError> {
        let identity = self.invocation_identity();
        self.defer_body(Some(crate::retained::PreparationFailureCode::domain(code)))?;
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

    pub fn run_bound_body<'account>(
        &mut self,
        control: OperationControl<PP, U>,
        ledger: FiniteStoreLedgerPlan,
        account: &'account PhysicalStorageAccount,
        plan: FiniteRetainedStoreStartPlan,
        body: impl for<'call, 'entry> FnMut(
            FiniteCanonicalBoundBodyParts<'call, 'entry, 'account, O, PP, U, T, Proof, EA, EV>,
            FiniteCanonicalDispositionSlot<'call, CK, CT, EK, ET>,
        ),
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
        ),
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
        ),
    ) -> BoundFinishResult<'_, 'account, O, PP, U, T, Proof, EA, EV, EG, CK, CT, EK, ET> {
        let (account, source, lineage) = source;
        let mut body = Some(body);
        let mut control = Some(control);
        let mut inputs = Some((ledger, plan, lineage));
        let mut admission = FiniteEntryAdmission::new(account);
        let mut start_error = None;
        let execution = if self.body_entered || self.store.is_none() {
            None
        } else {
            self.body_entered = true;
            Some(catch_unwind(AssertUnwindSafe(|| {
                let store = self
                    .store
                    .as_mut()
                    .expect("the original finite root is installed");
                let identity = store.invocation_identity();
                let (ledger, plan, lineage) = inputs.take().expect("the finite entry binds once");
                let bound = bind_finite_store(
                    store,
                    &mut admission,
                    &mut control,
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
                body.as_mut().expect("the finite callback is invoked once")(
                    CanonicalInvocationParts {
                        startup: FiniteCanonicalBoundBodyStart { entry: &mut entry },
                        output: CanonicalOutputSlot {
                            value: &mut self.output,
                            order: &self.order,
                            identity,
                        },
                        proof: CanonicalProofSlot {
                            value: &mut self.proof,
                            order: &self.order,
                            identity,
                            fallback: Some(CanonicalCauseSlot {
                                value: &mut self.central_fallback,
                                order: &self.order,
                                identity,
                            }),
                        },
                        activation: CanonicalActivationSlots {
                            never: CanonicalCauseSlot {
                                value: &mut self.never,
                                order: &self.order,
                                identity,
                            },
                            rejection: CanonicalCauseSlot {
                                value: &mut self.activation_rejected,
                                order: &self.order,
                                identity,
                            },
                        },
                        validation: CanonicalValidationSlots {
                            rejection: CanonicalCauseSlot {
                                value: &mut self.validation_rejected,
                                order: &self.order,
                                identity,
                            },
                            interruption: CanonicalCauseSlot {
                                value: &mut self.interrupted,
                                order: &self.order,
                                identity,
                            },
                        },
                    },
                    FiniteCanonicalDispositionSlot {
                        value: &mut self.pending,
                        identity,
                        _root_types: PhantomData,
                    },
                );
            })))
        };
        let admitted = execution.is_some();
        let body_panic = execution.and_then(Result::err);
        if body_panic.is_some() || !admitted || !self.has_valid_disposition() {
            self.record_payload_failure();
        }
        let mut capture_panic = catch_unwind(AssertUnwindSafe(|| drop(body.take()))).err();
        drop_captured_value(&mut control, &mut capture_panic, || {
            self.record_payload_failure()
        });
        let mut rejected_domain = None;
        if let Some((ledger, plan, lineage)) = inputs.take() {
            rejected_domain = Some(ledger.into_parts().0);
            let mut plan = Some(plan);
            let mut lineage = Some(lineage);
            drop_captured_value(&mut plan, &mut capture_panic, || {
                self.record_payload_failure()
            });
            drop_captured_value(&mut lineage, &mut capture_panic, || {
                self.record_payload_failure()
            });
        }
        if capture_panic.is_some() {
            self.record_payload_failure();
        }
        if body_panic.is_some() || capture_panic.is_some() {
            let cleanup_panic = self.clear_owned_values();
            // Retire every actual root only after all callback and payload disposal catches.
            drop(self.store.take());
            resume_first([body_panic, capture_panic, cleanup_panic]);
        }
        let start_error = start_error.or_else(|| {
            self.store
                .as_ref()
                .and_then(FiniteStoreInvocationRoot::startup_error)
        });
        if let Some(cause) = start_error {
            let panic = self.clear_owned_values();
            let error = self
                .take_error(FiniteStoreExitCause::Start(cause))
                .with_rejected_domain(rejected_domain);
            if let Some(payload) = panic {
                drop(error);
                std::panic::resume_unwind(payload);
            }
            return Err(error);
        }
        if !admitted || !self.has_valid_disposition() {
            let panic = self.clear_owned_values();
            let cause = FiniteStoreExitCause::Body(
                driver::CanonicalInvocationBodyError::invalid_disposition(),
            );
            let error = self.take_error(cause).with_rejected_domain(rejected_domain);
            if let Some(payload) = panic {
                drop(error);
                std::panic::resume_unwind(payload);
            }
            return Err(error);
        }
        Ok(FiniteCanonicalBoundInvocationFinish {
            host: self,
            admission,
        })
    }

    fn has_valid_disposition(&self) -> bool {
        match self.pending {
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
            None => false,
        }
    }

    pub(in crate::concrete_store) fn record_payload_failure(&mut self) {
        if let Some(store) = &mut self.store {
            store.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
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
        self.central_fallback = None;
        self.failure_fallback = None;
        self.interrupted = None;
        self.freeze_interrupted = None;
        let mut first = None;
        let store = &mut self.store;
        let mut record = || {
            if let Some(store) = store {
                store.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
            }
        };
        drop_captured_value(&mut self.never, &mut first, &mut record);
        drop_captured_value(&mut self.activation_rejected, &mut first, &mut record);
        drop_captured_value(&mut self.validation_rejected, &mut first, &mut record);
        drop_captured_value(&mut self.generation_rejected, &mut first, &mut record);
        match self.order.get() {
            PayloadOrder::ProofBeforeOutput => {
                drop_captured_value(&mut self.proof, &mut first, &mut record);
                drop_captured_value(&mut self.output, &mut first, &mut record);
            }
            PayloadOrder::OutputBeforeProof => {
                drop_captured_value(&mut self.output, &mut first, &mut record);
                drop_captured_value(&mut self.proof, &mut first, &mut record);
            }
        }
        first
    }
}

fn resume_first(panics: [Option<Box<dyn std::any::Any + Send>>; 3]) -> ! {
    let mut first = None;
    for payload in panics.into_iter().flatten() {
        if first.is_none() {
            first = Some(payload);
        } else {
            discard_untrusted_panic_payload(payload);
        }
    }
    std::panic::resume_unwind(first.expect("an original finite transition panic exists"));
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
