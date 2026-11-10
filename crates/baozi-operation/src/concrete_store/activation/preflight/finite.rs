//! Finite activation owners retain original fields across the shared scoped driver.

use super::super::payload_owner::PayloadValue;
use super::*;
use crate::concrete_store::finite::{
    FinitePublicationTransition, FiniteRetainedStoreOperation,
    FiniteRetainedStorePublicationAssembly, FiniteStoreExitCause, FiniteStoreExitError,
    FiniteStoreFailureAssembly, FiniteStoreFinalFields, FiniteStoreFinalizeCause,
    FiniteUnrootedStoreFailure,
};
use crate::concrete_store::{
    PreparedFiniteStoreFallback, StoreActionExecution, StoreInvocationError,
};

type FiniteActivationResult<'host, O, PP, U, T, E, CK, CT, L> = Result<
    FiniteCanonicalActivationOutcome<'host, O, PP, U, T, E, CK, CT, L>,
    FiniteStoreExitError<O, PP, U>,
>;
type FiniteSessionAdmissionResult<'host, O, PP, U, L> =
    Result<FiniteSessionOutcomeOwner<'host, O, PP, U, L>, FiniteStoreExitError<O, PP, U>>;
type ActivatedPayloadOwner<'host, O, PP, U, T, L> =
    PayloadOwner<PayloadLoan<'host, T>, FiniteRetainedStorePublicationAssembly<'host, O, PP, U, L>>;
type NeverActivatedPayloadOwner<'host, O, PP, U, E, L> = PayloadOwner<
    PayloadLoan<'host, StoreNeverActivatedCause<E>>,
    FiniteRetainedStoreOperation<'host, O, PP, U, L>,
>;

/// Session authority over the original finite operation before scoped activation.
#[must_use = "the finite session owner must resolve or activate its original operation"]
pub struct FiniteSessionOutcomeOwner<'host, O, PP, U, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    operation: Option<FiniteRetainedStoreOperation<'host, O, PP, U, L>>,
    policy: Option<CanonicalActivationSessionPolicy>,
    scoped_failure: Option<BoundScopedFailureRequest>,
    failure_work: FailureEvidenceWorkBudget,
    contain_if_abandoned: bool,
}

struct FiniteScopedOwnerTransition<'host, O, PP, U, T, E, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fields: Option<FiniteStoreFinalFields<'host, O, PP, U>>,
    lineage: Option<L>,
    publication_state: StorePublicationState,
    opened: Option<StorePublicationOpenDisposition>,
    pending: Option<ScopedPendingDecision<'host, T, E, CK, CT>>,
    failure: Option<BoundScopedFailureRequest>,
    failure_work: FailureEvidenceWorkBudget,
}

/// The concrete finite owner selected by one scoped activation transaction.
#[must_use = "the finite activation outcome retains its original storage and payload"]
pub enum FiniteCanonicalActivationOutcome<'host, O, PP, U, T, E, CK, CT, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Activated(FiniteActivatedSession<'host, O, PP, U, T, CK, CT, L>),
    NeverActivated(FiniteNeverActivatedSession<'host, O, PP, U, E, L>),
    Rejected(FiniteRejectedSession<'host, O, PP, U, E>),
}

/// An original output permission coupled to its bound finite publication assembly.
#[must_use = "the activated finite session retains the unpublished candidate"]
pub struct FiniteActivatedSession<'host, O, PP, U, T, CK, CT, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    canonical_root: CanonicalStoreRef<CK, CT>,
    owner: Option<ActivatedPayloadOwner<'host, O, PP, U, T, L>>,
    validation: ScopedActivationValidation,
    failure_work: FailureEvidenceWorkBudget,
    evidence_destination: StoreDimensionKey,
}

/// An original pre-activation cause permission coupled to the unopened finite operation.
#[must_use = "the never-activated finite session retains its original operation and cause"]
pub struct FiniteNeverActivatedSession<'host, O, PP, U, E, L = FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    owner: Option<NeverActivatedPayloadOwner<'host, O, PP, U, E, L>>,
    failure_code: PreparationFailureCode,
    fallback_only: bool,
    outcome_evidence_plan: RetainedStoreBlockTablePlan,
    failure_work: FailureEvidenceWorkBudget,
    evidence_destination: StoreDimensionKey,
}

/// An original rejection permission coupled to the already failure-only finite fields.
#[must_use = "the rejected finite session retains its original failure storage and cause"]
pub struct FiniteRejectedSession<'host, O, PP, U, E>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    owner: PayloadOwner<
        PayloadLoan<'host, StoreCanonicalRejection<E>>,
        FiniteUnrootedStoreFailure<'host, O, PP, U>,
    >,
    failure_code: PreparationFailureCode,
    failure_work: FailureEvidenceWorkBudget,
    evidence_destination: StoreDimensionKey,
}

impl<'host, O, PP, U, L> FiniteRetainedStoreOperation<'host, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Binds the common session preflight while preserving original finite owners on rejection.
    pub fn into_session_outcome_owner(
        mut self,
        policy: CanonicalActivationSessionPolicy,
        limits: RetainedStoreBatchLimits,
        requirements: CanonicalFailureRequirements,
    ) -> FiniteSessionAdmissionResult<'host, O, PP, U, L> {
        let preflight = self
            .ordinary_operation()
            .and_then(|operation| operation.preflight_session(&policy, limits, requirements));
        match preflight {
            Ok((scoped_failure, failure_work)) => Ok(FiniteSessionOutcomeOwner {
                operation: Some(self),
                policy: Some(policy),
                scoped_failure: Some(scoped_failure),
                failure_work,
                contain_if_abandoned: false,
            }),
            Err(_) => {
                let (mut fields, lineage) = self.into_activation_parts();
                let _ = drop_scoped_value(lineage);
                let cause = StoreActivationContainment {
                    cause: fields.contain_publication_fault(),
                };
                Err(fields.into_activation_error(cause))
            }
        }
    }
}

impl<'host, O, PP, U, L> FiniteSessionOutcomeOwner<'host, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Arms containment when this owner leaves its host's stable session slot.
    pub fn into_activation_transition(mut self) -> Self {
        self.contain_if_abandoned = true;
        self
    }

    fn operation_mut(&mut self) -> &mut FiniteRetainedStoreOperation<'host, O, PP, U, L> {
        self.operation
            .as_mut()
            .expect("the finite session retains its original operation until activation")
    }

    pub fn with_exclusive_execution<R, E: From<StoreInvocationError>>(
        &mut self,
        body: impl for<'op> FnOnce(&mut StoreInvocation<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        self.operation_mut().with_exclusive_execution(body)
    }

    pub fn with_action_execution<R, E: From<StoreRuntimeError>>(
        &mut self,
        dimension: StoreDimensionKey,
        body: impl for<'op> FnOnce(&mut StoreActionExecution<'op, O, PP, U>) -> Result<R, E>,
    ) -> Result<R, E> {
        self.operation_mut().with_action_execution(dimension, body)
    }

    pub fn reserve_retained_storage(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreRuntimeError> {
        self.operation_mut()
            .reserve_retained_storage(phase, dimension, bytes, allocations)
    }

    pub fn try_complete_phase(&mut self, phase: StorePhaseKey) -> Result<(), StoreRuntimeError> {
        self.operation_mut().try_complete_phase(phase)
    }

    pub fn reject_without_activation<T, E, CK, CT>(
        self,
        activation: CanonicalActivationSlots<'host, E>,
        cause: E,
        stable_code: u32,
    ) -> FiniteActivationResult<'host, O, PP, U, T, E, CK, CT, L> {
        let mut cause = Some(cause);
        self.run_plan_and_activate_scoped(None, activation, move |planning| {
            planning.reject(
                cause
                    .take()
                    .expect("a finite session is rejected without activation exactly once"),
                stable_code,
            )
        })
    }

    pub fn plan_and_activate_scoped<T, E, CK, CT>(
        self,
        output: CanonicalOutputSlot<'host, T>,
        activation: CanonicalActivationSlots<'host, E>,
        transaction: impl for<'op, 'scope> FnMut(
            CanonicalPlanning<'host, 'op, 'scope, O, PP, U, T, E, CK, CT, L>,
        ) -> CanonicalScopedDisposition<'scope>,
    ) -> FiniteActivationResult<'host, O, PP, U, T, E, CK, CT, L> {
        self.run_plan_and_activate_scoped(Some(output), activation, transaction)
    }

    fn run_plan_and_activate_scoped<T, E, CK, CT>(
        mut self,
        output: Option<CanonicalOutputSlot<'host, T>>,
        activation: CanonicalActivationSlots<'host, E>,
        transaction: impl for<'op, 'scope> FnMut(
            CanonicalPlanning<'host, 'op, 'scope, O, PP, U, T, E, CK, CT, L>,
        ) -> CanonicalScopedDisposition<'scope>,
    ) -> FiniteActivationResult<'host, O, PP, U, T, E, CK, CT, L> {
        let policy = self
            .policy
            .take()
            .expect("the finite session transfers its original policy once");
        let scoped_failure = self
            .scoped_failure
            .take()
            .expect("the finite session transfers its original failure request once");
        let expected_request_identity = scoped_failure.request_identity;
        let operation = self
            .operation
            .take()
            .expect("the finite session transfers its original operation once");
        let mut owner =
            FiniteScopedOwnerTransition::new(operation, scoped_failure, self.failure_work);
        let transaction = ManuallyDrop::new(Some(transaction));
        let valid = {
            let fields = owner.fields.as_mut().expect("original finite fields");
            match fields.operation_loan::<L>() {
                Ok(operation) => {
                    let planning_baseline = operation.planning_baseline();
                    let identity = operation.invocation_identity();
                    let prior_failure = operation.activation_terminal_cause();
                    let output_is_valid = output
                        .as_ref()
                        .is_none_or(|slot| slot.is_vacant_for(identity))
                        && activation.is_vacant_for(identity);
                    let driver = ScopedActivationDriver {
                        output,
                        never: Some(activation.never),
                        rejection: Some(activation.rejection),
                        state: ScopedActivationState::Dormant {
                            operation,
                            publication_state: &mut owner.publication_state,
                        },
                        lineage: &mut owner.lineage,
                        opened: &mut owner.opened,
                        pending: &mut owner.pending,
                        roles: Some(BoundActivationRoles {
                            policy,
                            expected_request_identity,
                        }),
                        failure: owner.failure.as_ref(),
                        planning_baseline,
                        first_fault: None,
                        semantic_state: ScopedSemanticState::NotStarted,
                    };
                    run_scoped_driver(driver, output_is_valid, prior_failure, transaction)
                }
                Err(_) => {
                    let _ = drop_optional_callback(transaction);
                    false
                }
            }
        };
        owner.finish(valid)
    }
}

impl<O, PP, U, L> Drop for FiniteSessionOutcomeOwner<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        let Some(operation) = self.operation.take() else {
            return;
        };
        let (mut fields, lineage) = operation.into_activation_parts();
        let lineage_drop_failed = drop_scoped_value(lineage).is_err();
        if self.contain_if_abandoned || lineage_drop_failed {
            let _ = fields.contain_publication_fault();
        }
    }
}

impl<'host, O, PP, U, T, E, CK, CT, L> FiniteScopedOwnerTransition<'host, O, PP, U, T, E, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn new(
        operation: FiniteRetainedStoreOperation<'host, O, PP, U, L>,
        failure: BoundScopedFailureRequest,
        failure_work: FailureEvidenceWorkBudget,
    ) -> Self {
        let (fields, lineage) = operation.into_activation_parts();
        Self {
            fields: Some(fields),
            lineage: Some(lineage),
            publication_state: StorePublicationState::default(),
            opened: None,
            pending: None,
            failure: Some(failure),
            failure_work,
        }
    }

    fn contain(&mut self) -> StoreActivationContainment {
        if let Some(ScopedPendingDecision::Contain {
            recorded: Some(cause),
        }) = self.pending.as_ref()
        {
            let cause = *cause;
            let _ = self.lineage.take().map(drop_scoped_value);
            return cause;
        }
        if let Some(pending) = self.pending.take() {
            drop_pending_decision(pending);
        }
        let _ = self.lineage.take().map(drop_scoped_value);
        let fields = self.fields.as_mut().expect("original finite fields");
        if let Some(opened) = self.opened.take() {
            let _ = fields.bind_scoped_open(opened);
        }
        let cause = StoreActivationContainment {
            cause: fields.contain_publication_fault(),
        };
        self.pending = Some(ScopedPendingDecision::Contain {
            recorded: Some(cause),
        });
        cause
    }

    fn into_error(mut self) -> FiniteStoreExitError<O, PP, U> {
        let cause = self.contain();
        self.fields
            .take()
            .expect("the finite transition transfers its original fields once")
            .into_activation_error(cause)
    }

    fn finish(mut self, valid: bool) -> FiniteActivationResult<'host, O, PP, U, T, E, CK, CT, L> {
        if !valid
            || self.failure.is_none()
            || matches!(
                self.pending,
                None | Some(ScopedPendingDecision::Contain { .. })
            )
        {
            return Err(self.into_error());
        }
        match self.opened.take() {
            Some(opened @ StorePublicationOpenDisposition::Opened(_)) => {
                if self
                    .fields
                    .as_mut()
                    .expect("original finite fields")
                    .bind_scoped_open(opened)
                    .is_err()
                {
                    return Err(self.into_error());
                }
                match self.pending.as_ref() {
                    Some(ScopedPendingDecision::Activated { .. }) => {
                        let Some(lineage) = self.lineage.take() else {
                            return Err(self.into_error());
                        };
                        let failure = self.failure.take().expect(
                            "the finite transition transfers its original failure request once",
                        );
                        let Some(ScopedPendingDecision::Activated {
                            output,
                            root,
                            validation,
                        }) = self.pending.take()
                        else {
                            unreachable!("the original activated branch remains selected")
                        };
                        let assembly = FiniteRetainedStorePublicationAssembly::from_bound_fields(
                            self.fields.take().expect("original finite fields"),
                            std::mem::take(&mut self.publication_state),
                            lineage,
                        );
                        Ok(FiniteCanonicalActivationOutcome::Activated(
                            FiniteActivatedSession {
                                canonical_root: root,
                                owner: Some(PayloadOwner::new(output, assembly)),
                                validation,
                                failure_work: self.failure_work,
                                evidence_destination: failure
                                    .outcome_evidence_destination_dimension,
                            },
                        ))
                    }
                    Some(ScopedPendingDecision::Rejected { failure_code, .. }) => {
                        if self.lineage.is_some() {
                            return Err(self.into_error());
                        }
                        let failure_code = *failure_code;
                        let failure = self.failure.take().expect(
                            "the finite transition transfers its original failure request once",
                        );
                        let storage = self
                            .fields
                            .take()
                            .expect("original finite fields")
                            .bind_unrooted_failure(
                                std::mem::take(&mut self.publication_state),
                                failure_code,
                            );
                        match storage {
                            Ok(storage) => {
                                let Some(ScopedPendingDecision::Rejected { cause, .. }) =
                                    self.pending.take()
                                else {
                                    unreachable!("the original rejected branch remains selected")
                                };
                                Ok(FiniteCanonicalActivationOutcome::Rejected(
                                    FiniteRejectedSession {
                                        owner: PayloadOwner::new(cause, storage),
                                        failure_code,
                                        failure_work: self.failure_work,
                                        evidence_destination: failure
                                            .outcome_evidence_destination_dimension,
                                    },
                                ))
                            }
                            Err(error) => {
                                if let Some(pending) = self.pending.take() {
                                    drop_pending_decision(pending);
                                }
                                Err(error)
                            }
                        }
                    }
                    _ => Err(self.into_error()),
                }
            }
            unopened => {
                let failed_fallback_only = match unopened {
                    Some(StorePublicationOpenDisposition::Failed(failure)) => {
                        let fail_closed = failure.is_fail_closed();
                        let fallback_only = failure.is_fallback_only();
                        if self
                            .fields
                            .as_mut()
                            .expect("original finite fields")
                            .bind_scoped_open(StorePublicationOpenDisposition::Failed(failure))
                            .is_err()
                            || fail_closed
                        {
                            return Err(self.into_error());
                        }
                        Some(fallback_only)
                    }
                    None => None,
                    Some(StorePublicationOpenDisposition::Opened(_)) => {
                        unreachable!("opened fields use the concrete opened binding")
                    }
                };
                let Some(ScopedPendingDecision::NeverActivated { fallback_only, .. }) =
                    self.pending.as_ref()
                else {
                    return Err(self.into_error());
                };
                if failed_fallback_only.is_some_and(|expected| expected != *fallback_only)
                    || self
                        .fields
                        .as_mut()
                        .expect("original finite fields")
                        .ensure_unopened_activation()
                        .is_err()
                {
                    return Err(self.into_error());
                }
                let Some(lineage) = self.lineage.take() else {
                    return Err(self.into_error());
                };
                let failure = self
                    .failure
                    .take()
                    .expect("the finite transition transfers its original failure request once");
                let Some(ScopedPendingDecision::NeverActivated {
                    cause,
                    failure_code,
                    fallback_only,
                }) = self.pending.take()
                else {
                    unreachable!("the original never-activated branch remains selected")
                };
                let operation = FiniteRetainedStoreOperation::from_activation_parts(
                    self.fields.take().expect("original finite fields"),
                    lineage,
                );
                Ok(FiniteCanonicalActivationOutcome::NeverActivated(
                    FiniteNeverActivatedSession {
                        owner: Some(PayloadOwner::new(cause, operation)),
                        failure_code,
                        fallback_only,
                        outcome_evidence_plan: failure.outcome_evidence_plan,
                        failure_work: self.failure_work,
                        evidence_destination: failure.outcome_evidence_destination_dimension,
                    },
                ))
            }
        }
    }
}

impl<O, PP, U, T, E, CK, CT, L> Drop for FiniteScopedOwnerTransition<'_, O, PP, U, T, E, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if self.fields.is_some() {
            let _ = self.contain();
        }
    }
}

impl<O, PP, U, T, CK, CT, L> FiniteActivatedSession<'_, O, PP, U, T, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn output(&self) -> &T {
        self.owner
            .as_ref()
            .expect("the activated finite session retains its original payload owner")
            .value()
            .value()
    }
}

impl<O, PP, U, T, CK, CT, L> Drop for FiniteActivatedSession<'_, O, PP, U, T, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        let Some(owner) = self.owner.take() else {
            return;
        };
        let already_unwinding = std::thread::panicking();
        let (output, assembly) = owner.into_parts();
        let payload_panic = catch_unwind(AssertUnwindSafe(|| drop(output))).err();
        let _ = assembly.retire_activation(payload_panic.is_some());
        if let Some(payload) = payload_panic {
            if already_unwinding {
                discard_untrusted_panic_payload(payload);
            } else {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

impl<O, PP, U, E, L> FiniteNeverActivatedSession<'_, O, PP, U, E, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreNeverActivatedCause<E> {
        self.owner
            .as_ref()
            .expect("the never-activated finite session retains its original payload owner")
            .value()
            .value()
    }
}

impl<O, PP, U, E, L> Drop for FiniteNeverActivatedSession<'_, O, PP, U, E, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        let Some(owner) = self.owner.take() else {
            return;
        };
        let already_unwinding = std::thread::panicking();
        let (cause, operation) = owner.into_parts();
        let payload_panic = catch_unwind(AssertUnwindSafe(|| drop(cause))).err();
        let _ = operation.retire_activation(payload_panic.is_some());
        if let Some(payload) = payload_panic {
            if already_unwinding {
                discard_untrusted_panic_payload(payload);
            } else {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

impl<O, PP, U, E> FiniteRejectedSession<'_, O, PP, U, E>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreCanonicalRejection<E> {
        self.owner.value().value()
    }
}

impl<'host, O, PP, U, T, CK, CT, L> FiniteActivatedSession<'host, O, PP, U, T, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store::activation) fn into_central_parts(
        mut self,
    ) -> super::super::canonical::finite::FiniteActivatedParts<'host, O, PP, U, T, CK, CT, L> {
        let (output, assembly) = self
            .owner
            .take()
            .expect("the activated finite owner transfers once")
            .into_parts();
        super::super::canonical::finite::FiniteActivatedParts {
            output,
            assembly,
            canonical_root: self.canonical_root,
            validation: self.validation,
            failure_work: self.failure_work,
            evidence_destination: self.evidence_destination,
        }
    }
}

impl<'host, O, PP, U, E, L> FiniteNeverActivatedSession<'host, O, PP, U, E, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Records the original operation and cause until the body's actual captures are disposed.
    pub fn request_finish<CK, CT, EK, ET>(
        mut self,
        disposition: FiniteCanonicalDispositionSlot<'host, CK, CT, EK, ET>,
    ) -> Result<(), FiniteStoreExitError<O, PP, U>> {
        let (cause, operation) = self
            .owner
            .take()
            .expect("the finite never-activated owner transfers once")
            .into_parts();
        let mut cause = PayloadValue::new(cause);
        if !disposition.is_vacant_for(
            cause
                .value
                .as_ref()
                .expect("original cause permission")
                .identity(),
        ) {
            let _ = cause.dispose();
            return Err(operation.into_request_error());
        }
        let store = match operation.defer_until_body_cleanup() {
            Ok(store) => store,
            Err(operation) => {
                let _ = cause.dispose();
                return Err(operation.into_request_error());
            }
        };
        disposition.install_pending(host::finite::FiniteDisposition::NeverActivated(
            host::driver::PendingNeverActivated {
                store,
                fallback_only: self.fallback_only,
                failure_code: self.failure_code,
                outcome_evidence_plan: self.outcome_evidence_plan,
                failure_work: self.failure_work,
                destination: self.evidence_destination,
            },
        ));
        cause
            .take()
            .expect("original cause permission")
            .retain_in_host();
        Ok(())
    }
}

impl<'host, O, PP, U, E>
    FiniteNeverActivatedSession<'host, O, PP, U, E, crate::concrete_store::TerminalStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(in crate::concrete_store::activation) fn from_deferred(
        cause: PayloadLoan<'host, StoreNeverActivatedCause<E>>,
        operation: FiniteRetainedStoreOperation<
            'host,
            O,
            PP,
            U,
            crate::concrete_store::TerminalStoreLineage,
        >,
        fallback_only: bool,
        failure_code: PreparationFailureCode,
        outcome_evidence_plan: RetainedStoreBlockTablePlan,
        failure_work: FailureEvidenceWorkBudget,
        evidence_destination: StoreDimensionKey,
    ) -> Self {
        Self {
            owner: Some(PayloadOwner::new(cause, operation)),
            fallback_only,
            failure_code,
            outcome_evidence_plan,
            failure_work,
            evidence_destination,
        }
    }
}

impl<'host, O, PP, U, E> FiniteRejectedSession<'host, O, PP, U, E>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Defers the already rejected graph without repeating canonical disposal.
    pub fn request_finish<CK, CT, EK, ET>(
        self,
        disposition: FiniteCanonicalDispositionSlot<'host, CK, CT, EK, ET>,
    ) -> Result<(), FiniteStoreExitError<O, PP, U>> {
        let Self {
            owner,
            failure_code,
            failure_work,
            evidence_destination,
        } = self;
        let (cause, failure) = owner.into_parts();
        let mut cause = PayloadValue::new(cause);
        if !disposition.is_vacant_for(
            cause
                .value
                .as_ref()
                .expect("original cause permission")
                .identity(),
        ) {
            let _ = cause.dispose();
            return Err(failure.into_request_error());
        }
        let store = match failure.defer_until_body_cleanup() {
            Ok(store) => store,
            Err(failure) => {
                let _ = cause.dispose();
                return Err(failure.into_request_error());
            }
        };
        disposition.install_pending(host::finite::FiniteDisposition::Rejected(
            host::driver::PendingRejected {
                store,
                failure_code,
                failure_work,
                destination: evidence_destination,
            },
        ));
        cause
            .take()
            .expect("original cause permission")
            .retain_in_host();
        Ok(())
    }

    pub(in crate::concrete_store::activation) fn from_deferred(
        cause: PayloadLoan<'host, StoreCanonicalRejection<E>>,
        failure: FiniteUnrootedStoreFailure<'host, O, PP, U>,
        failure_code: PreparationFailureCode,
        failure_work: FailureEvidenceWorkBudget,
        evidence_destination: StoreDimensionKey,
    ) -> Self {
        Self {
            owner: PayloadOwner::new(cause, failure),
            failure_code,
            failure_work,
            evidence_destination,
        }
    }
}

impl<O, PP, U, E, L> CanonicalFinal<FiniteNeverActivatedSession<'_, O, PP, U, E, L>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreNeverActivatedCause<E> {
        self.state.cause()
    }
}

impl<O, PP, U, E> CanonicalFinal<FiniteRejectedSession<'_, O, PP, U, E>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreCanonicalRejection<E> {
        self.state.cause()
    }
}

struct FiniteFailureBuild<'host, O, PP, U, Cause>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fields: Option<FiniteStoreFinalFields<'host, O, PP, U>>,
    cause: PayloadValue<PayloadLoan<'host, Cause>>,
    state: StorePublicationState,
    failure_code: PreparationFailureCode,
    failure_work: FailureEvidenceWorkBudget,
    evidence_destination: StoreDimensionKey,
}

impl<'host, O, PP, U, Cause> FiniteFailureBuild<'host, O, PP, U, Cause>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn into_error(mut self) -> FiniteStoreExitError<O, PP, U> {
        let _ = self.cause.dispose();
        let mut fields = self.fields.take().expect("original finite failure fields");
        let cause = fields.contain_publication_fault();
        fields.into_activation_error(StoreActivationContainment::from_cause(cause))
    }

    fn prepare_minimal_fallback(
        mut self,
        code: PreparationFailureCode,
    ) -> Result<
        VerifiedMinimalFallback<Cause, PreparedFiniteStoreFallback>,
        FiniteStoreExitError<O, PP, U>,
    > {
        let result = self
            .fields
            .take()
            .expect("original finite failure fields")
            .prepare_publication_fallback(code, None);
        match result {
            Ok(prepared) => Ok(VerifiedMinimalFallback::new(
                prepared,
                self.cause
                    .take()
                    .expect("original finite failure cause")
                    .into_owned(),
            )),
            Err(mut error) => {
                if self.cause.dispose().is_err() {
                    error.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
                    error.contain_payload_failure();
                }
                Err(error)
            }
        }
    }

    fn build<EK, ET>(
        mut self,
        build: impl for<'evidence, 'lane> FnMut(
            &mut FailureEvidenceLane<'evidence, 'lane, O, PP, U>,
        ) -> FailureEvidenceDecision<EK, ET>,
    ) -> FiniteFailureBuildResult<'host, O, PP, U, Cause, EK, ET>
    where
        ET: Copy + Send + Sync + 'static,
    {
        let mut build = PayloadValue::new(build);
        let evidence = self
            .fields
            .as_mut()
            .expect("original finite failure fields")
            .failure_decision_loan(&mut self.state);
        let result = run_failure_evidence_build(
            evidence,
            &mut self.failure_work,
            build
                .take()
                .expect("the original failure builder transfers once"),
        );
        let evidence_root = match result {
            Ok(root) => root,
            Err(Some(code)) => {
                self.fields
                    .as_mut()
                    .expect("original finite failure fields")
                    .failure_decision_loan(&mut self.state)
                    .publication_impl()
                    .raw
                    .loan_mut()
                    .record_terminal_cause(code);
                return self
                    .prepare_minimal_fallback(code)
                    .map(FailureEvidenceOutcome::MinimalFallback);
            }
            Err(None) => return Err(self.into_error()),
        };
        let storage = FiniteStoreFailureAssembly::from_bound_fields(
            self.fields.take().expect("original finite failure fields"),
            std::mem::take(&mut self.state),
            self.failure_code,
            Some(self.failure_code),
            evidence_root,
            None,
        );
        Ok(FailureEvidenceOutcome::Rooted(CanonicalFinal::new(
            RootedFailureEvidence {
                owner: PayloadOwner::new(
                    self.cause.take().expect("original finite failure cause"),
                    storage,
                ),
                failure_work: self.failure_work,
                outcome_evidence_destination_dimension: self.evidence_destination,
                _lifecycle: PhantomData::<OrdinaryFailureState>,
            },
        )))
    }
}

impl<O, PP, U, Cause> Drop for FiniteFailureBuild<'_, O, PP, U, Cause>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if let Some(fields) = self.fields.as_mut() {
            let _ = self.cause.dispose();
            let _ = fields.contain_publication_fault();
        }
    }
}

struct FiniteFailureOpen<'host, O, PP, U, Cause, L, Build>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    publication: Option<FinitePublicationTransition<'host, O, PP, U, L>>,
    opened: Option<StorePublicationOpenDisposition>,
    cause: PayloadValue<PayloadLoan<'host, Cause>>,
    build: PayloadValue<Build>,
}

impl<'host, O, PP, U, Cause, L, Build> FiniteFailureOpen<'host, O, PP, U, Cause, L, Build>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn new(
        owner: PayloadOwner<
            PayloadLoan<'host, Cause>,
            FiniteRetainedStoreOperation<'host, O, PP, U, L>,
        >,
        build: Option<Build>,
    ) -> Self {
        let (cause, operation) = owner.into_parts();
        let (fields, lineage) = operation.into_activation_parts();
        Self {
            publication: Some(FinitePublicationTransition::new(
                fields,
                StorePublicationState::default(),
                lineage,
            )),
            opened: None,
            cause: PayloadValue::new(cause),
            build: PayloadValue { value: build },
        }
    }

    fn into_error(mut self) -> FiniteStoreExitError<O, PP, U> {
        let _ = self.build.dispose();
        let _ = self.cause.dispose();
        let publication = self
            .publication
            .as_mut()
            .expect("original finite publication transition");
        let error = publication.contain();
        let (fields, _, original_error) = self
            .publication
            .take()
            .expect("original finite publication transition")
            .finish();
        fields.into_error(FiniteStoreExitCause::Finalize(
            FiniteStoreFinalizeCause::Publication(original_error.unwrap_or(error)),
        ))
    }

    fn clear_error(
        &mut self,
        mut error: FiniteStoreExitError<O, PP, U>,
    ) -> FiniteStoreExitError<O, PP, U> {
        let build_failed = self.build.dispose().is_err();
        let cause_failed = self.cause.dispose().is_err();
        if build_failed || cause_failed {
            error.record_payload_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
            error.contain_payload_failure();
        }
        error
    }

    fn open(
        &mut self,
        plan: RetainedStorePublicationPlan,
        failure_code: PreparationFailureCode,
    ) -> Result<bool, ()> {
        let publication = self
            .publication
            .as_mut()
            .expect("original finite publication transition");
        let opened = catch_unwind(AssertUnwindSafe(|| {
            let operation = publication
                .fields
                .as_mut()
                .expect("original finite operation fields")
                .operation_loan::<L>()?;
            operation.validate_publication_plan_keys(&plan)?;
            operation
                .open_publication_loan(plan, &mut publication.state)
                .into_disposition()
        }));
        self.opened = match opened {
            Ok(Ok(opened)) => Some(opened),
            Ok(Err(_)) => return Err(()),
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                return Err(());
            }
        };
        let failure = match self.opened.as_ref().expect("original open disposition") {
            StorePublicationOpenDisposition::Opened(_) => None,
            StorePublicationOpenDisposition::Failed(failure) => Some((
                failure.is_fail_closed(),
                classify_activation_fault(&map_writer_error(failure.cause())),
            )),
        };
        if let Err(opened) = publication
            .fields
            .as_mut()
            .expect("original finite operation fields")
            .bind_scoped_open(self.opened.take().expect("original open disposition"))
        {
            self.opened = Some(opened);
            return Err(());
        }
        if let Some((fail_closed, classification)) = failure {
            let ActivationFaultClassification::Recoverable(code) = classification else {
                return Err(());
            };
            if fail_closed {
                return Err(());
            }
            publication
                .fields
                .as_mut()
                .expect("original finite operation fields")
                .operation_loan::<L>()
                .map_err(|_| ())?
                .latch_activation_terminal_cause(code);
            return Ok(false);
        }
        if publication.state.fallback_cause.is_some()
            || publication.state.success_rejection.is_some()
        {
            return Err(());
        }
        let verification = publication
            .ordinary()
            .raw
            .loan_mut()
            .verify_failure_only(failure_code);
        if let Err(cause) = verification {
            publication.record_containment(StorePrepareError::FinalizationContainment(cause));
            return Err(());
        }
        if drop_scoped_value(publication.take_lineage()).is_err() {
            publication.contain();
            return Err(());
        }
        Ok(true)
    }

    fn prepare_minimal_fallback(
        mut self,
        code: PreparationFailureCode,
    ) -> Result<
        VerifiedMinimalFallback<Cause, PreparedFiniteStoreFallback>,
        FiniteStoreExitError<O, PP, U>,
    > {
        if self.build.dispose().is_err() || self.opened.is_some() {
            return Err(self.into_error());
        }
        let publication = self
            .publication
            .as_mut()
            .expect("original finite publication transition");
        let poll = catch_unwind(AssertUnwindSafe(|| {
            publication
                .fields
                .as_mut()
                .expect("original finite operation fields")
                .operation_loan::<L>()
                .map(|mut operation| operation.poll_terminal_control())
        }));
        match poll {
            Ok(Ok(())) => {}
            Ok(Err(_)) => return Err(self.into_error()),
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                return Err(self.into_error());
            }
        }
        let (fields, _, error) = self
            .publication
            .take()
            .expect("original finite publication transition")
            .finish();
        if let Some(error) = error {
            let error = fields.into_error(FiniteStoreExitCause::Finalize(
                FiniteStoreFinalizeCause::Publication(error),
            ));
            return Err(self.clear_error(error));
        }
        match fields.prepare_minimal_code(code) {
            Ok(prepared) => Ok(VerifiedMinimalFallback::new(
                prepared,
                self.cause
                    .take()
                    .expect("original finite failure cause")
                    .into_owned(),
            )),
            Err(error) => Err(self.clear_error(error)),
        }
    }
}

impl<O, PP, U, Cause, L, Build> Drop for FiniteFailureOpen<'_, O, PP, U, Cause, L, Build>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if let Some(publication) = self.publication.as_mut() {
            let _ = self.build.dispose();
            let _ = self.cause.dispose();
            publication.retire_lineage();
            publication.contain();
        }
    }
}

impl<'host, O, PP, U, E, L> CanonicalFinal<FiniteNeverActivatedSession<'host, O, PP, U, E, L>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn build_failure_evidence<EK, ET>(
        mut self,
        build: impl for<'evidence, 'lane> FnMut(
            &mut FailureEvidenceLane<'evidence, 'lane, O, PP, U>,
        ) -> FailureEvidenceDecision<EK, ET>,
    ) -> FiniteFailureBuildResult<'host, O, PP, U, StoreNeverActivatedCause<E>, EK, ET>
    where
        ET: Copy + Send + Sync + 'static,
    {
        let mut transition = FiniteFailureOpen::new(
            self.state
                .owner
                .take()
                .expect("the final finite session transfers its original owner once"),
            Some(build),
        );
        let failure_code = self.state.failure_code;
        if self.state.fallback_only {
            return transition
                .prepare_minimal_fallback(failure_code)
                .map(FailureEvidenceOutcome::MinimalFallback);
        }
        let plan = RetainedStorePublicationPlan::rich_failure(self.state.outcome_evidence_plan);
        match transition.open(plan, failure_code) {
            Ok(true) => {}
            Ok(false) => {
                return transition
                    .prepare_minimal_fallback(failure_code)
                    .map(FailureEvidenceOutcome::MinimalFallback);
            }
            Err(()) => return Err(transition.into_error()),
        }
        let (fields, state, error) = transition
            .publication
            .take()
            .expect("original finite publication transition")
            .finish();
        if let Some(error) = error {
            let error = fields.into_error(FiniteStoreExitCause::Finalize(
                FiniteStoreFinalizeCause::Publication(error),
            ));
            return Err(transition.clear_error(error));
        }
        FiniteFailureBuild {
            fields: Some(fields),
            cause: PayloadValue::new(
                transition
                    .cause
                    .take()
                    .expect("original finite failure cause"),
            ),
            state,
            failure_code,
            failure_work: self.state.failure_work,
            evidence_destination: self.state.evidence_destination,
        }
        .build(
            transition
                .build
                .take()
                .expect("the original failure builder transfers once"),
        )
    }

    pub fn prepare_minimal_fallback(
        mut self,
    ) -> Result<
        VerifiedMinimalFallback<StoreNeverActivatedCause<E>, PreparedFiniteStoreFallback>,
        FiniteStoreExitError<O, PP, U>,
    > {
        FiniteFailureOpen::<_, _, _, _, L, ()>::new(
            self.state
                .owner
                .take()
                .expect("the final finite session transfers its original owner once"),
            None,
        )
        .prepare_minimal_fallback(self.state.failure_code)
    }
}

impl<'host, O, PP, U, E> CanonicalFinal<FiniteRejectedSession<'host, O, PP, U, E>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn build_failure_evidence<EK, ET>(
        self,
        build: impl for<'evidence, 'lane> FnMut(
            &mut FailureEvidenceLane<'evidence, 'lane, O, PP, U>,
        ) -> FailureEvidenceDecision<EK, ET>,
    ) -> FiniteFailureBuildResult<'host, O, PP, U, StoreCanonicalRejection<E>, EK, ET>
    where
        ET: Copy + Send + Sync + 'static,
    {
        let FiniteRejectedSession {
            owner,
            failure_code: _,
            failure_work,
            evidence_destination,
        } = self.state;
        let (cause, failure) = owner.into_parts();
        let cause = PayloadValue::new(cause);
        let (fields, state, failure_code) = failure.into_failure_parts();
        FiniteFailureBuild {
            fields: Some(fields),
            cause,
            state,
            failure_code,
            failure_work,
            evidence_destination,
        }
        .build(build)
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<
        VerifiedMinimalFallback<StoreCanonicalRejection<E>, PreparedFiniteStoreFallback>,
        FiniteStoreExitError<O, PP, U>,
    > {
        let FiniteRejectedSession {
            owner,
            failure_code: _,
            failure_work,
            evidence_destination,
        } = self.state;
        let (cause, failure) = owner.into_parts();
        let cause = PayloadValue::new(cause);
        let (fields, state, failure_code) = failure.into_failure_parts();
        FiniteFailureBuild {
            fields: Some(fields),
            cause,
            state,
            failure_code,
            failure_work,
            evidence_destination,
        }
        .prepare_minimal_fallback(failure_code)
    }
}
