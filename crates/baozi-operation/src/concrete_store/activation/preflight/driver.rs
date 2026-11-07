//! Shared activation algorithms borrow actual operation fields and original Host payload slots.

use super::*;

/// Runs the one callback/capture boundary while a concrete outer owner retains actual fields.
pub(super) fn run_scoped_driver<'host, 'op, O, PP, U, T, E, CK, CT, L, Transaction>(
    mut driver: ScopedActivationDriver<'host, 'op, O, PP, U, T, E, CK, CT, L>,
    output_is_valid: bool,
    prior_failure: Option<PreparationFailureCode>,
    mut transaction: ManuallyDrop<Option<Transaction>>,
) -> bool
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Transaction: for<'scope> FnMut(
        CanonicalPlanning<'host, 'op, 'scope, O, PP, U, T, E, CK, CT, L>,
    ) -> CanonicalScopedDisposition<'scope>,
{
    let receipt_brand = ScopedReceiptBrand { _private: 0 };
    let result = catch_unwind(AssertUnwindSafe(|| {
        if !output_is_valid {
            return false;
        }
        if let Some(failure_code) = prior_failure {
            if let ScopedActivationState::Dormant { operation, .. } = &mut driver.state {
                operation.poll_terminal_control();
            }
            driver.install_never_activated_failure(
                StoreRuntimeError::SuccessOutcomeRejected,
                failure_code,
                false,
            );
            return true;
        }
        let transaction = transaction
            .as_mut()
            .expect("the transaction remains owned until controlled drop");
        let receipt = transaction(CanonicalPlanning {
            driver: &mut driver,
            receipt_brand: &receipt_brand,
        });
        std::ptr::eq(receipt.brand, &receipt_brand)
    }));
    let transaction_drop_failed = drop_optional_callback(transaction).is_err();
    match result {
        Ok(valid) => valid && !transaction_drop_failed,
        Err(payload) => {
            discard_untrusted_panic_payload(payload);
            false
        }
    }
}

impl<'host, 'op, O, PP, U, T, E, CK, CT, L>
    ScopedActivationDriver<'host, 'op, O, PP, U, T, E, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fn receipt<'scope>(
        brand: &'scope ScopedReceiptBrand,
    ) -> CanonicalScopedDisposition<'scope> {
        CanonicalScopedDisposition {
            brand,
            invariant: PhantomData,
        }
    }

    pub(super) fn poll_final_planning_control(&mut self) {
        let result = match &mut self.state {
            ScopedActivationState::Dormant { operation, .. } => {
                operation.control_checkpoint().poll()
            }
            _ => Err(StoreRuntimeError::ConstructionInvariantUnverified),
        };
        let _ = preserve_first_fault(&mut self.first_fault, result);
    }

    fn operation_is_quiescent(&self) -> bool {
        !matches!(self.semantic_state, ScopedSemanticState::Active)
            && matches!(&self.state, ScopedActivationState::Dormant { operation, .. }
                if operation.planning_matches_baseline(self.planning_baseline))
    }

    pub(super) fn contain_current(&mut self) {
        if matches!(
            self.pending.as_ref(),
            Some(ScopedPendingDecision::Contain { recorded: Some(_) })
        ) {
            return;
        }
        if let Some(pending) = self.pending.take() {
            drop_pending_decision(pending);
        }
        self.roles.take();
        self.failure.take();
        let _ = self.lineage.take().map(drop_scoped_value);
        let recorded = match std::mem::replace(&mut self.state, ScopedActivationState::Resolved) {
            ScopedActivationState::Dormant { operation, .. } => Some(StoreActivationContainment {
                cause: operation.contain_fault(),
            }),
            ScopedActivationState::Opened(mut assembly) => Some(StoreActivationContainment {
                cause: assembly.contain_fault(),
            }),
            ScopedActivationState::Transitioning | ScopedActivationState::Resolved => None,
        };
        *self.pending = Some(ScopedPendingDecision::Contain { recorded });
    }

    fn recorded_containment(&mut self, cause: FinalizationContainmentError) {
        if let Some(pending) = self.pending.take() {
            drop_pending_decision(pending);
        }
        let _ = self.lineage.take().map(drop_scoped_value);
        self.state = ScopedActivationState::Resolved;
        self.roles.take();
        self.failure.take();
        *self.pending = Some(ScopedPendingDecision::Contain {
            recorded: Some(StoreActivationContainment { cause }),
        });
    }

    pub(super) fn bind_success(
        &mut self,
        activation_request: CanonicalActivationRequest,
        requirements: CanonicalSuccessRequirements,
    ) -> Result<BoundScopedSuccessRequest, ()> {
        if self.failure.is_none() {
            let _ = preserve_first_fault::<()>(
                &mut self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
            return Err(());
        }
        self.poll_final_planning_control();
        let semantic_complete = matches!(self.semantic_state, ScopedSemanticState::Complete);
        if !semantic_complete {
            let _ = preserve_first_fault::<()>(
                &mut self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
        let Some(roles) = self.roles.as_ref() else {
            let _ = preserve_first_fault::<()>(
                &mut self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
            return Err(());
        };
        let request = match roles.bind_scoped_success(activation_request, requirements) {
            Ok(request) => request,
            Err(_) => {
                let _ = preserve_first_fault::<()>(
                    &mut self.first_fault,
                    Err(StoreRuntimeError::ConstructionInvariantUnverified),
                );
                return Err(());
            }
        };
        self.roles.take();
        if !semantic_complete || self.first_fault.is_some() {
            Err(())
        } else {
            Ok(request)
        }
    }

    pub(super) fn install_never_activated_runtime(&mut self, fault: ActivationFirstFault) {
        let Some(failure_code) = fault.recoverable_code() else {
            self.contain_current();
            return;
        };
        self.install_never_activated_failure(fault.cause, failure_code, false);
    }

    pub(super) fn install_never_activated_failure(
        &mut self,
        cause: StoreRuntimeError,
        failure_code: PreparationFailureCode,
        fallback_only: bool,
    ) {
        self.install_never_value(
            StoreNeverActivatedCause::Runtime(cause),
            failure_code,
            fallback_only,
        );
    }

    pub(super) fn install_never_requested(&mut self, rejection: VerifiedRejection<E>) {
        self.install_never_value(
            StoreNeverActivatedCause::Requested(rejection.cause),
            rejection.failure_code,
            false,
        );
    }

    fn install_never_value(
        &mut self,
        value: StoreNeverActivatedCause<E>,
        failure_code: PreparationFailureCode,
        fallback_only: bool,
    ) {
        if !self.operation_is_quiescent() || self.failure.take().is_none() {
            let _ = drop_scoped_value(value);
            self.contain_current();
            return;
        }
        let ScopedActivationState::Dormant { operation, .. } = &mut self.state else {
            let _ = drop_scoped_value(value);
            self.contain_current();
            return;
        };
        let Some(slot) = self.never.take() else {
            let _ = drop_scoped_value(value);
            self.contain_current();
            return;
        };
        if !slot.is_vacant_for(operation.invocation_identity()) {
            let _ = drop_scoped_value(value);
            self.contain_current();
            return;
        }
        let cause = match slot.place(value) {
            Ok(cause) => cause,
            Err(value) => {
                let _ = drop_scoped_value(value);
                self.contain_current();
                return;
            }
        };
        operation.latch_activation_terminal_cause(failure_code);
        *self.pending = Some(ScopedPendingDecision::NeverActivated {
            cause,
            failure_code,
            fallback_only,
        });
        self.state = ScopedActivationState::Resolved;
        self.roles.take();
    }

    pub(super) fn prepare_activation(&mut self, request: &BoundScopedSuccessRequest) -> bool {
        let ScopedActivationState::Dormant { operation, .. } = &mut self.state else {
            let _ = preserve_first_fault::<()>(
                &mut self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
            return false;
        };
        let original_baseline = self.planning_baseline;
        let prepared = (|| {
            if request.semantic_bytes != 0
                && let Some(dimension) = request.semantic_bytes_dimension
            {
                operation
                    .producer_host()
                    .charge_observed_batch([(dimension, request.semantic_bytes)])?;
            }
            let before = operation.planning_baseline();
            operation.rebind_activation_canonical_assembly(request.canonical_plan)?;
            original_baseline
                .checked_apply_rebind_delta(before, operation.planning_baseline())
                .ok_or(StoreRuntimeError::ConstructionInvariantUnverified)
        })();
        match prepared {
            Ok(baseline) => self.planning_baseline = baseline,
            Err(cause) => {
                let _ = preserve_first_fault::<()>(&mut self.first_fault, Err(cause));
            }
        }
        self.first_fault.is_none()
    }

    pub(super) fn finish_pre_activation_failure<P, M>(
        &mut self,
        inputs: &mut ScopedActivationInputs<P, M>,
    ) {
        if inputs.drop_remaining().is_err() || !self.operation_is_quiescent() {
            self.contain_current();
        } else if let Some(fault) = self.first_fault.take() {
            self.install_never_activated_runtime(fault);
        } else {
            self.contain_current();
        }
    }

    pub(super) fn open_publication<P, M>(
        &mut self,
        request: &BoundScopedSuccessRequest,
        inputs: &mut ScopedActivationInputs<P, M>,
    ) -> bool {
        let Some(failure) = self.failure else {
            let _ = inputs.drop_remaining();
            self.contain_current();
            return false;
        };
        let plan = RetainedStorePublicationPlan::success(
            request.canonical_plan,
            failure.outcome_evidence_plan,
        );
        let state = std::mem::replace(&mut self.state, ScopedActivationState::Transitioning);
        let ScopedActivationState::Dormant {
            operation,
            publication_state,
        } = state
        else {
            self.state = state;
            let _ = inputs.drop_remaining();
            self.contain_current();
            return false;
        };
        let pre_open = catch_unwind(AssertUnwindSafe(|| {
            #[cfg(test)]
            inject_scoped_transition_panic(ScopedTransitionPanicPoint::TryOpenActivation);
            operation.validate_publication_plan_keys(&plan)
        }));
        match pre_open {
            Ok(Ok(())) => {}
            result => {
                if let Err(payload) = result {
                    discard_untrusted_panic_payload(payload);
                }
                self.state = ScopedActivationState::Dormant {
                    operation,
                    publication_state,
                };
                let _ = inputs.drop_remaining();
                self.contain_current();
                return false;
            }
        }
        if let Some(failure_code) = operation.activation_terminal_cause() {
            self.state = ScopedActivationState::Dormant {
                operation,
                publication_state,
            };
            if inputs.drop_remaining().is_err() || !self.operation_is_quiescent() {
                self.contain_current();
            } else {
                self.install_never_activated_failure(
                    StoreRuntimeError::SuccessOutcomeRejected,
                    failure_code,
                    false,
                );
            }
            return false;
        }
        match operation.open_publication_loan(plan, publication_state) {
            StorePublicationOpenDecision::Opened(assembly) => {
                let Some(stamp) = assembly.raw.opened_stamp() else {
                    self.state = ScopedActivationState::Opened(assembly);
                    let _ = inputs.drop_remaining();
                    self.contain_current();
                    return false;
                };
                *self.opened = Some(StorePublicationOpenDisposition::Opened(stamp));
                self.state = ScopedActivationState::Opened(assembly);
                true
            }
            StorePublicationOpenDecision::Failed {
                mut operation,
                state,
                failure,
            } => {
                let cause = map_writer_error(failure.cause());
                let code = if failure.is_fail_closed() {
                    None
                } else {
                    match classify_activation_fault(&cause) {
                        ActivationFaultClassification::Recoverable(code) => Some(code),
                        ActivationFaultClassification::Contract => None,
                    }
                };
                let fallback_only = failure.is_fallback_only();
                if let Some(code) = code {
                    operation.latch_activation_terminal_cause(code);
                }
                *self.opened = Some(StorePublicationOpenDisposition::Failed(failure));
                self.state = ScopedActivationState::Dormant {
                    operation,
                    publication_state: state,
                };
                if inputs.drop_remaining().is_err() || !self.operation_is_quiescent() {
                    self.contain_current();
                } else if let Some(code) = code {
                    self.install_never_activated_failure(cause, code, fallback_only);
                } else {
                    self.contain_current();
                }
                false
            }
        }
    }

    pub(super) fn finish_materialization(
        &mut self,
        request: BoundScopedSuccessRequest,
        materialized: CanonicalMaterializationState<T, E, CK, CT>,
    ) {
        let ScopedActivationState::Opened(assembly) = &self.state else {
            let _ = drop_materialization(materialized);
            self.contain_current();
            return;
        };
        let baseline = assembly
            .raw
            .loan()
            .publication_storage_baseline(self.planning_baseline);
        let quiescent = !matches!(self.semantic_state, ScopedSemanticState::Active)
            && baseline
                .is_some_and(|baseline| assembly.raw.loan().planning_matches_baseline(baseline));
        if !quiescent || assembly.is_fallback_only() {
            let _ = drop_materialization(materialized);
            self.contain_current();
            return;
        }
        match (self.first_fault.take(), materialized) {
            (Some(_fault), CanonicalMaterializationState::Activated { output, .. }) => {
                let _ = drop_scoped_value(output);
                self.contain_current();
            }
            (Some(fault), CanonicalMaterializationState::Rejected(rejection)) => {
                let dropped = drop_verified_rejection(rejection).is_ok();
                if !dropped || fault.contract_seen {
                    self.contain_current();
                } else if let Some(code) = fault.recoverable_code() {
                    self.finish_rejection(StoreCanonicalRejection::Runtime(fault.cause), code);
                } else {
                    self.contain_current();
                }
            }
            (None, CanonicalMaterializationState::Activated { root, output }) => {
                if assembly.canonical_len() != request.expected_materialized_blocks {
                    let _ = drop_scoped_value(output);
                    self.contain_current();
                    return;
                }
                let Some(slot) = self.output.take() else {
                    let _ = drop_scoped_value(output);
                    self.contain_current();
                    return;
                };
                let output = match slot.place(output) {
                    Ok(output) => output,
                    Err(output) => {
                        let _ = drop_scoped_value(output);
                        self.contain_current();
                        return;
                    }
                };
                *self.pending = Some(ScopedPendingDecision::Activated {
                    output,
                    root,
                    validation: request.into_validation(),
                });
                self.state = ScopedActivationState::Resolved;
            }
            (None, CanonicalMaterializationState::Rejected(rejection)) => {
                self.finish_rejection(
                    StoreCanonicalRejection::Requested(rejection.cause),
                    rejection.failure_code,
                );
            }
        }
    }

    fn finish_rejection(
        &mut self,
        value: StoreCanonicalRejection<E>,
        failure_code: PreparationFailureCode,
    ) {
        #[cfg(test)]
        inject_scoped_transition_panic(ScopedTransitionPanicPoint::FinishRejectedSession);
        let Some(slot) = self.rejection.take() else {
            let _ = drop_scoped_value(value);
            self.contain_current();
            return;
        };
        let cause = match slot.place(value) {
            Ok(cause) => cause,
            Err(value) => {
                let _ = drop_scoped_value(value);
                self.contain_current();
                return;
            }
        };
        *self.pending = Some(ScopedPendingDecision::Rejected {
            cause,
            failure_code,
        });
        let result = match &mut self.state {
            ScopedActivationState::Opened(assembly) => assembly.prepare_rejection(failure_code),
            _ => {
                self.contain_current();
                return;
            }
        };
        if let Err(cause) = result {
            self.recorded_containment(cause);
            return;
        }
        if self
            .lineage
            .take()
            .is_none_or(|lineage| drop_scoped_value(lineage).is_err())
        {
            self.contain_current();
            return;
        }
        self.state = ScopedActivationState::Resolved;
    }
}

/// Unresolved activation unwinds contain through its current actual-field loan after input cleanup.
pub(super) struct ScopedDriverTransition<'driver, 'host, 'op, O, PP, U, T, E, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) driver: &'driver mut ScopedActivationDriver<'host, 'op, O, PP, U, T, E, CK, CT, L>,
}

impl<O, PP, U, T, E, CK, CT, L> Drop
    for ScopedDriverTransition<'_, '_, '_, O, PP, U, T, E, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if !matches!(self.driver.state, ScopedActivationState::Resolved) {
            self.driver.contain_current();
        }
    }
}
