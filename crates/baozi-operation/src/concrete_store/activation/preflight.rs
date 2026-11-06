use super::*;

#[cfg(test)]
mod tests;

fn reject_semantic_entry<F, E>(
    body: ManuallyDrop<Option<F>>,
    first_fault: &mut Option<ActivationFirstFault>,
    cause: StoreRuntimeError,
) -> CanonicalSemanticWorkError<E> {
    let _ = preserve_first_fault::<()>(first_fault, Err(cause));
    if drop_optional_callback(body).is_err() {
        note_later_contract::<()>(
            first_fault,
            Err(StoreRuntimeError::ConstructionInvariantUnverified),
        );
    }
    CanonicalSemanticWorkError::Runtime(
        preserved_fault(first_fault).unwrap_or(StoreRuntimeError::ConstructionInvariantUnverified),
    )
}

fn canonical_build_work(units: Option<usize>) -> Result<NonZeroU64, StoreRuntimeError> {
    let units = units.ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    let units =
        u64::try_from(units).map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    NonZeroU64::new(units).ok_or(StoreRuntimeError::Plan(StorePlanError::EmptyPayload))
}

fn contain_dormant_activation<O, PP, U, L>(
    dormant: ScopedDormantActivation<'_, O, PP, U, L>,
) -> StoreActivationContainment
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match dormant.into_operation() {
        Ok(operation) => contain_operation(operation),
        Err(containment) => containment,
    }
}

impl<'inv, O, PP, U, L> RetainedStoreOperation<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Qualifies the exact possible failure-validation work once, before session activation.
    ///
    /// Only unused capacity from the supplied ordinary counter dimension can move. The operation
    /// preserves both total work limits, every attempted/consumed counter, the startup construction
    /// bound, and at least the startup member and validation guarantees. Evidence members are
    /// still individually qualified against their sealed plans when they are constructed.
    pub fn qualify_failure_validation_work(
        &mut self,
        ordinary_work_dimension: StoreDimensionKey,
        max_evidence_members: usize,
        validation_work_limit: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.raw
            .preflight_failure_validation_work()
            .map_err(|cause| map_writer_error(&cause))?;
        if self.raw.is_terminal_only() {
            return Err(StoreRuntimeError::FallbackOnly);
        }
        self.validate_dimension_key(ordinary_work_dimension)?;
        self.producer_host().poll()?;
        self.raw
            .qualify_failure_validation_work(
                ordinary_work_dimension,
                max_evidence_members,
                validation_work_limit,
            )
            .map_err(|cause| map_writer_error(&cause))
    }

    /// Moves the operation into the session-lifetime outcome owner before canonical activation.
    pub fn into_session_outcome_owner(
        self,
        policy: CanonicalActivationSessionPolicy,
        outcome_evidence_writer_limits: RetainedStoreBatchLimits,
        failure_requirements: CanonicalFailureRequirements,
    ) -> Result<RetainedStoreSessionOutcomeOwner<'inv, O, PP, U, L>, StoreActivationContainment>
    {
        let (canonical_phase, canonical_dimensions) =
            policy.canonical_writer.keys_for_publication_validation();
        let outcome_evidence_writer = policy
            .outcome_evidence_writer
            .bind(outcome_evidence_writer_limits);
        let (evidence_phase, evidence_dimensions) =
            outcome_evidence_writer.keys_for_publication_validation();
        let roles_are_valid = self
            .validate_writer_keys(canonical_phase, canonical_dimensions)
            .and_then(|()| self.validate_writer_keys(evidence_phase, evidence_dimensions))
            .and_then(|()| self.validate_activation_phase(policy.validation_phase))
            .and_then(|()| {
                self.validate_activation_dimensions([
                    policy.materialization_work_dimension,
                    policy.materialization_scratch_dimension,
                    policy.validation_work_dimension,
                    policy.validation_scratch_dimension,
                    policy.canonical_destination_dimension,
                    policy.outcome_evidence_destination_dimension,
                ])
            })
            .and_then(|()| {
                policy.semantic_bytes_dimension.map_or(Ok(()), |dimension| {
                    self.validate_activation_dimensions([dimension])
                })
            })
            .is_ok();
        if !roles_are_valid {
            return Err(contain_operation(self));
        }
        let failure = match policy
            .bind_scoped_failure(outcome_evidence_writer_limits, failure_requirements)
        {
            Ok(failure) => failure,
            Err(_) => return Err(contain_operation(self)),
        };
        let Some((max_evidence_members, construction_work_limit, validation_work_limit)) =
            self.raw.failure_terminal_work_limits()
        else {
            return Err(contain_operation(self));
        };
        let failure_work = FailureEvidenceWorkEnvelope {
            max_evidence_members,
            construction_work_limit,
            validation_work_limit,
        };
        let containment_anchor = self.raw.planning_containment_anchor();
        Ok(RetainedStoreSessionOutcomeOwner {
            dormant: ScopedDormantActivation {
                operation: Some(self),
                containment_anchor,
                contain_if_abandoned: false,
            },
            policy,
            scoped_failure: failure,
            failure_work: FailureEvidenceWorkBudget::new(failure_work),
        })
    }
}

impl<'inv, O, PP, U, L> RetainedStoreSessionOutcomeOwner<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Marks a session owner extracted into a host's transient activation state.
    ///
    /// Ordinary session drop is healthy cleanup. Once the host has removed this owner from its
    /// stable state slot, a scoped transaction must resolve it; abandonment is an ownership fault.
    pub fn into_activation_transition(mut self) -> Self {
        self.dormant.contain_if_abandoned = true;
        self
    }

    /// Lends the existing physical fault anchor to the host's consuming state transition.
    pub fn activation_containment_anchor(&self) -> StoreActivationContainmentAnchor {
        StoreActivationContainmentAnchor {
            raw: self.dormant.containment_anchor.share(),
        }
    }

    /// Lends the existing root with the ordinary loan's original entry checkpoint.
    /// Owned callback results leave after numeric cleanup; the caller's next explicit frontier
    /// or root entry observes any recorded failure, as with the original ordinary loan.
    pub fn with_root_execution<R, E: From<crate::concrete_store::StoreInvocationError>>(
        &mut self,
        boundary: &crate::concrete_store::StoreImporterBoundary,
        body: impl for<'op> FnOnce(
            &mut crate::concrete_store::StoreInvocation<'op, O, PP, U>,
        ) -> Result<R, E>,
    ) -> Result<R, E> {
        self.dormant
            .operation_mut()
            .with_root_execution(boundary, body)
    }

    /// Lends the existing Dormant root to the one executor; activation authority never escapes.
    pub fn with_parser_execution<R, E: From<crate::concrete_store::StoreInvocationError>>(
        &mut self,
        boundary: &crate::concrete_store::StoreImporterBoundary,
        body: impl for<'op> FnOnce(
            &mut crate::concrete_store::StoreInvocation<'op, O, PP, U>,
        ) -> Result<R, E>,
    ) -> Result<R, E> {
        self.dormant
            .operation_mut()
            .with_parser_execution(boundary, body)
    }

    /// Shared gate for replacement execution, legacy entry and both submission paths.
    pub fn check_execution_running(&self) -> Result<(), StoreRuntimeError> {
        self.dormant
            .operation()
            .raw
            .check_execution_running()
            .map_err(crate::concrete_store::execution::map_execution_error)
    }

    pub fn settle_importer_return(&self) {
        self.dormant.operation().raw.settle_importer_return();
    }

    /// Borrows producer work and accounting before canonical activation begins.
    pub fn producer_host(&mut self) -> RetainedStoreProducerHost<'_, O, PP, U> {
        self.dormant.operation_mut().producer_host()
    }

    /// Borrows the operation's control and reservation checkpoint before activation begins.
    pub fn control_checkpoint(&mut self) -> RetainedStoreControlCheckpoint<'_, O, PP, U> {
        self.dormant.operation_mut().control_checkpoint()
    }

    pub fn reserve_retained_storage(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
        allocations: u64,
    ) -> Result<RetainedStoreReservation<O>, StoreRuntimeError> {
        self.dormant
            .operation_mut()
            .reserve_retained_storage(phase, dimension, bytes, allocations)
    }

    /// Marks one qualified session phase complete without exposing the raw operation.
    pub fn try_complete_phase(&mut self, phase: StorePhaseKey) -> Result<(), StoreRuntimeError> {
        self.dormant.operation_mut().try_complete_phase(phase)
    }

    /// Resolves a session that never received a canonical activation request.
    #[allow(
        clippy::result_large_err,
        clippy::type_complexity,
        reason = "the HRTB transaction preserves the exact linear outcome owner"
    )]
    pub fn reject_without_activation<T, E, CanonicalKind, CanonicalTag>(
        self,
        activation: CanonicalActivationSlots<'inv, E>,
        cause: E,
        stable_code: u32,
    ) -> Result<
        CanonicalActivationOutcome<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
        StoreActivationContainment,
    > {
        let mut cause = Some(cause);
        self.run_plan_and_activate_scoped(None, activation, move |planning| {
            planning.reject(
                cause
                    .take()
                    .expect("a session can be rejected without activation exactly once"),
                stable_code,
            )
        })
    }

    /// Runs one scoped activation whose complete sealed plan carries its single-use request.
    ///
    /// The dormant authority remains inside this owner while semantic preflight runs. Only
    /// [`CanonicalPlanning::seal_activation`] can pair the request with matching success facts and
    /// make the owner-consuming activation transition reachable.
    #[allow(
        clippy::result_large_err,
        clippy::type_complexity,
        reason = "the HRTB transaction preserves the exact linear outcome owner"
    )]
    pub fn plan_and_activate_scoped<T, E, CanonicalKind, CanonicalTag>(
        self,
        output: CanonicalOutputSlot<'inv, T>,
        activation: CanonicalActivationSlots<'inv, E>,
        transaction: impl for<'scope> FnMut(
            CanonicalPlanning<'inv, 'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
        ) -> CanonicalScopedDisposition<'scope>,
    ) -> Result<
        CanonicalActivationOutcome<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
        StoreActivationContainment,
    > {
        self.run_plan_and_activate_scoped(Some(output), activation, transaction)
    }

    #[allow(
        clippy::result_large_err,
        clippy::type_complexity,
        reason = "the HRTB transaction preserves the exact linear outcome owner"
    )]
    fn run_plan_and_activate_scoped<T, E, CanonicalKind, CanonicalTag>(
        self,
        output: Option<CanonicalOutputSlot<'inv, T>>,
        activation: CanonicalActivationSlots<'inv, E>,
        transaction: impl for<'scope> FnMut(
            CanonicalPlanning<'inv, 'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
        ) -> CanonicalScopedDisposition<'scope>,
    ) -> Result<
        CanonicalActivationOutcome<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
        StoreActivationContainment,
    > {
        let Self {
            dormant,
            policy,
            scoped_failure,
            failure_work,
        } = self;
        let expected_request_identity = scoped_failure.request_identity;
        run_scoped_activation(
            dormant,
            BoundActivationRoles {
                policy,
                expected_request_identity,
            },
            scoped_failure,
            failure_work,
            output,
            activation,
            transaction,
        )
    }
}

#[allow(
    clippy::result_large_err,
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "the scoped driver keeps every linear owner inline across the complete activation transaction"
)]
fn run_scoped_activation<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>(
    dormant: ScopedDormantActivation<'inv, O, PP, U, L>,
    roles: BoundActivationRoles,
    scoped_failure: BoundScopedFailureRequest,
    failure_work: FailureEvidenceWorkBudget,
    output: Option<CanonicalOutputSlot<'inv, T>>,
    activation: CanonicalActivationSlots<'inv, E>,
    transaction: impl for<'scope> FnMut(
        CanonicalPlanning<'inv, 'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
    ) -> CanonicalScopedDisposition<'scope>,
) -> Result<
    CanonicalActivationOutcome<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
    StoreActivationContainment,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let planning_baseline = dormant.operation().raw.planning_baseline();
    let output_is_valid = output
        .as_ref()
        .is_none_or(|slot| slot.is_vacant_for(dormant.operation().raw.invocation_identity()))
        && activation.is_vacant_for(dormant.operation().raw.invocation_identity());
    let prior_failure = dormant.operation().activation_terminal_cause();
    let containment_anchor = dormant.containment_anchor.share();
    let mut transaction = ManuallyDrop::new(Some(transaction));
    let receipt_brand = ScopedReceiptBrand { _private: 0 };
    let mut driver = ScopedActivationDriver {
        output,
        never: Some(activation.never),
        rejection: Some(activation.rejection),
        state: ScopedActivationState::Dormant(dormant),
        containment_anchor,
        roles: Some(roles),
        failure: Some(scoped_failure),
        planning_baseline,
        failure_work,
        first_fault: None,
        semantic_state: ScopedSemanticState::NotStarted,
    };
    let result = catch_unwind(AssertUnwindSafe(|| {
        if !output_is_valid {
            return false;
        }
        // A selected terminal cannot lend ordinary execution. Preserve its exact code and
        // resolve the dormant owner before any semantic callback, as activation opening does.
        if let Some(failure_code) = prior_failure {
            if let ScopedActivationState::Dormant(dormant) = &mut driver.state {
                // Observe late control without replacing the already selected terminal code.
                dormant.operation_mut().raw.poll_terminal_control();
            }
            driver.install_never_activated_failure(
                StoreRuntimeError::SuccessOutcomeRejected,
                failure_code,
            );
            return true;
        }
        let transaction = transaction
            .as_mut()
            .expect("the scoped activation transaction remains owned until controlled drop");
        let receipt = transaction(CanonicalPlanning {
            driver: &mut driver,
            receipt_brand: &receipt_brand,
        });
        std::ptr::eq(receipt.brand, &receipt_brand)
    }));
    let transaction_drop_failed = drop_optional_callback(transaction).is_err();
    match result {
        Ok(true) if !transaction_drop_failed => driver.finish(),
        Ok(_) => Err(driver.contain_current()),
        Err(payload) => {
            discard_untrusted_panic_payload(payload);
            Err(driver.contain_current())
        }
    }
}

fn contain_scoped_pending<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>(
    pending: ScopedPendingActivation<'_, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
) -> StoreActivationContainment
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    match pending {
        ScopedPendingActivation::Activated(activated) => {
            let ActivatedCanonicalStoreSession {
                canonical_root: _,
                owner,
                failure_work: _,
                validation_phase: _,
                validation_work_dimension: _,
                validation_scratch_dimension: _,
                canonical_destination_dimension: _,
                outcome_evidence_destination_dimension: _,
                validation_work: _,
            } = activated;
            let (output, assembly) = owner.into_parts();
            let _ = drop_scoped_value(output);
            StoreActivationContainment {
                cause: assembly.contain_activation_fault(),
            }
        }
        ScopedPendingActivation::Rejected(rejected) => {
            let RejectedCanonicalStoreSession { owner } = rejected;
            let UnrootedFailureEvidence {
                owner,
                domain: _,
                failure_code: _,
                failure_work: _,
                outcome_evidence_destination_dimension: _,
            } = owner;
            let (cause, raw) = owner.into_parts();
            let _ = drop_scoped_value(cause);
            StoreActivationContainment {
                cause: raw.contain_activation_fault(),
            }
        }
        ScopedPendingActivation::NeverActivated(never) => {
            let NeverActivatedStoreSession {
                owner,
                failure_code: _,
                outcome_evidence_plan: _,
                failure_work: _,
                outcome_evidence_destination_dimension: _,
            } = never;
            let (cause, authority) = owner.into_parts();
            let _ = drop_scoped_value(cause);
            match authority {
                NeverActivatedAuthority::Operation(operation)
                | NeverActivatedAuthority::FallbackOnly(operation) => contain_operation(operation),
            }
        }
    }
}

impl<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
    ScopedActivationDriver<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn receipt<'scope>(brand: &'scope ScopedReceiptBrand) -> CanonicalScopedDisposition<'scope> {
        CanonicalScopedDisposition {
            brand,
            invariant: PhantomData,
        }
    }

    fn poll_final_planning_control(&mut self) {
        let ScopedActivationDriver {
            state, first_fault, ..
        } = self;
        let result = match state {
            ScopedActivationState::Dormant(dormant) => {
                dormant.operation_mut().control_checkpoint().poll()
            }
            ScopedActivationState::Transitioning
            | ScopedActivationState::Pending(_)
            | ScopedActivationState::Contained(_)
            | ScopedActivationState::Finished => {
                Err(StoreRuntimeError::ConstructionInvariantUnverified)
            }
        };
        let _ = preserve_first_fault(first_fault, result);
    }

    fn planning_is_quiescent(&self, operation: &RetainedStoreOperation<'inv, O, PP, U, L>) -> bool {
        !matches!(self.semantic_state, ScopedSemanticState::Active)
            && operation
                .raw
                .planning_matches_baseline(self.planning_baseline)
    }

    fn assembly_is_quiescent(
        &self,
        assembly: &RetainedStorePublicationAssembly<'inv, O, PP, U, L>,
        publication_storage_baseline: RetainedPlanningBaseline,
    ) -> bool {
        !matches!(self.semantic_state, ScopedSemanticState::Active)
            && assembly
                .raw
                .planning_matches_baseline(publication_storage_baseline)
    }

    fn install_pending(
        &mut self,
        pending: ScopedPendingActivation<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
    ) {
        self.state = ScopedActivationState::Pending(pending);
        self.roles.take();
    }

    fn contain_current(&mut self) -> StoreActivationContainment {
        self.roles.take();
        self.failure.take();
        let state = std::mem::replace(&mut self.state, ScopedActivationState::Transitioning);
        let containment = match state {
            ScopedActivationState::Dormant(dormant) => contain_dormant_activation(dormant),
            ScopedActivationState::Transitioning => StoreActivationContainment {
                cause: self.containment_anchor.contain(),
            },
            ScopedActivationState::Pending(pending) => contain_scoped_pending(pending),
            ScopedActivationState::Contained(containment) => containment,
            ScopedActivationState::Finished => StoreActivationContainment {
                cause: self.containment_anchor.contain(),
            },
        };
        self.state = ScopedActivationState::Contained(containment);
        containment
    }

    #[allow(
        clippy::type_complexity,
        reason = "the scoped driver returns the complete inline linear terminal owner without allocation"
    )]
    fn finish(
        &mut self,
    ) -> Result<
        CanonicalActivationOutcome<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
        StoreActivationContainment,
    > {
        let state = std::mem::replace(&mut self.state, ScopedActivationState::Transitioning);
        match state {
            ScopedActivationState::Pending(ScopedPendingActivation::Activated(state)) => {
                self.state = ScopedActivationState::Finished;
                Ok(CanonicalActivationOutcome::Activated(state))
            }
            ScopedActivationState::Pending(ScopedPendingActivation::Rejected(state)) => {
                self.state = ScopedActivationState::Finished;
                Ok(CanonicalActivationOutcome::Rejected(state))
            }
            ScopedActivationState::Pending(ScopedPendingActivation::NeverActivated(state)) => {
                self.state = ScopedActivationState::Finished;
                Ok(CanonicalActivationOutcome::NeverActivated(state))
            }
            ScopedActivationState::Contained(containment) => {
                self.state = ScopedActivationState::Contained(containment);
                Err(containment)
            }
            ScopedActivationState::Dormant(dormant) => {
                let containment = contain_dormant_activation(dormant);
                self.state = ScopedActivationState::Contained(containment);
                Err(containment)
            }
            ScopedActivationState::Transitioning => {
                let containment = StoreActivationContainment {
                    cause: self.containment_anchor.contain(),
                };
                self.state = ScopedActivationState::Contained(containment);
                Err(containment)
            }
            ScopedActivationState::Finished => {
                let containment = StoreActivationContainment {
                    cause: self.containment_anchor.contain(),
                };
                self.state = ScopedActivationState::Contained(containment);
                Err(containment)
            }
        }
    }

    fn bind_success(
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
        };
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

    fn install_never_activated_runtime(&mut self, fault: ActivationFirstFault) {
        let Some(failure_code) = fault.recoverable_code() else {
            let _ = self.contain_current();
            return;
        };
        self.install_never_activated_failure(fault.cause, failure_code);
    }

    fn install_never_activated_failure(
        &mut self,
        cause: StoreRuntimeError,
        failure_code: PreparationFailureCode,
    ) {
        let state = std::mem::replace(&mut self.state, ScopedActivationState::Transitioning);
        let ScopedActivationState::Dormant(dormant) = state else {
            self.state = state;
            let _ = self.contain_current();
            return;
        };
        if !self.planning_is_quiescent(dormant.operation()) {
            self.state = ScopedActivationState::Dormant(dormant);
            let _ = self.contain_current();
            return;
        }
        let Some(failure) = self.failure.take() else {
            self.state = ScopedActivationState::Dormant(dormant);
            let _ = self.contain_current();
            return;
        };
        let operation = match dormant.into_operation() {
            Ok(operation) => operation,
            Err(containment) => {
                self.state = ScopedActivationState::Contained(containment);
                return;
            }
        };
        let slot = self
            .never
            .take()
            .expect("never-activation installs its cause once");
        let pending = NeverActivatedStoreSession::runtime(
            operation,
            slot,
            &failure,
            self.failure_work,
            cause,
            failure_code,
            false,
        );
        self.install_never_activated(pending);
    }

    fn install_never_activated(
        &mut self,
        pending: Result<
            NeverActivatedStoreSession<'inv, O, PP, U, E, L>,
            StoreActivationContainment,
        >,
    ) {
        match pending {
            Ok(pending) => self.install_pending(ScopedPendingActivation::NeverActivated(pending)),
            Err(containment) => self.state = ScopedActivationState::Contained(containment),
        }
    }
}

impl<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L> Drop
    for ScopedActivationDriver<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if !matches!(
            self.state,
            ScopedActivationState::Contained(_) | ScopedActivationState::Finished
        ) {
            let _ = self.contain_current();
        }
    }
}

impl<'inv, 'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
    CanonicalPlanning<'inv, 'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Runs one semantic pass while the operation owns its unique numeric loan and cleanup.
    ///
    /// No caller-declared total crosses this boundary. Each [`CanonicalSemanticWorkSession::run_unit`]
    /// call records its own attempted unit before the action and its consumed unit only after the
    /// action returns normally.
    /// Plans may retain input borrows and outer-scoped reservations. The numeric session cannot
    /// escape this callback, and sealing or activation occurs only after it has ended.
    #[doc = include_str!("preflight/semantic_contracts.md")]
    pub fn with_semantic_work<R, Rejection>(
        &mut self,
        body: impl for<'work> FnMut(
            &mut CanonicalSemanticWorkSession<'work, 'scope, O, PP, U, L>,
        ) -> Result<R, Rejection>,
    ) -> Result<(R, CanonicalSemanticCompletion<'scope>), CanonicalSemanticWorkError<Rejection>>
    {
        let mut body = ManuallyDrop::new(Some(body));
        let receipt_brand = self.receipt_brand;
        let ScopedActivationDriver {
            state,
            roles,
            first_fault,
            semantic_state,
            ..
        } = self.driver;
        if !matches!(semantic_state, ScopedSemanticState::NotStarted) {
            *semantic_state = ScopedSemanticState::Rejected;
            return Err(reject_semantic_entry(
                body,
                first_fault,
                StoreRuntimeError::ConstructionInvariantUnverified,
            ));
        }
        if let Some(cause) = preserved_fault(first_fault) {
            *semantic_state = ScopedSemanticState::Rejected;
            return Err(reject_semantic_entry(body, first_fault, cause));
        }
        let Some(roles) = roles.as_ref() else {
            *semantic_state = ScopedSemanticState::Rejected;
            return Err(reject_semantic_entry(
                body,
                first_fault,
                StoreRuntimeError::ConstructionInvariantUnverified,
            ));
        };
        let (phase, work_dimension, scratch_dimension) = roles.authoring_keys();
        let operation = match state {
            ScopedActivationState::Dormant(dormant) => dormant.operation_mut(),
            _ => {
                *semantic_state = ScopedSemanticState::Rejected;
                return Err(reject_semantic_entry(
                    body,
                    first_fault,
                    StoreRuntimeError::ConstructionInvariantUnverified,
                ));
            }
        };
        let mut context = match operation.invocation_context(None) {
            Ok(context) => context,
            Err(cause) => {
                *semantic_state = ScopedSemanticState::Rejected;
                return Err(reject_semantic_entry(body, first_fault, cause));
            }
        };
        if let Err(cause) = context.ordinary_storage().checkpoint() {
            *semantic_state = ScopedSemanticState::Rejected;
            return Err(reject_semantic_entry(body, first_fault, cause));
        }
        let work_group = match context.bind_work([work_dimension]) {
            Ok(group) => group,
            Err(cause) => {
                *semantic_state = ScopedSemanticState::Rejected;
                return Err(reject_semantic_entry(body, first_fault, cause));
            }
        };
        *semantic_state = ScopedSemanticState::Active;
        let mut semantic = CanonicalSemanticWorkSession {
            context,
            work_group,
            copy_frame: None,
            first_fault,
            phase,
            scratch_dimension,
            state: semantic_state,
            receipt_brand,
            completed_any: false,
            resolved: false,
            lineage: PhantomData,
            scope: PhantomData,
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            body.as_mut()
                .expect("the semantic callback remains retained until controlled cleanup")(
                &mut semantic,
            )
        }));
        let result = match result {
            Ok(result) => result,
            Err(payload) => {
                let _ = semantic.close_copy_units();
                note_later_contract::<()>(
                    semantic.first_fault,
                    Err(StoreRuntimeError::ConstructionInvariantUnverified),
                );
                if drop_optional_callback(body).is_err() {
                    note_later_contract::<()>(
                        semantic.first_fault,
                        Err(StoreRuntimeError::ConstructionInvariantUnverified),
                    );
                }
                drop(semantic);
                std::panic::resume_unwind(payload);
            }
        };
        // Captures are disposed before the final semantic checkpoint and completion issuance.
        let _ = semantic.close_copy_units();
        if drop_optional_callback(body).is_err() {
            note_later_contract::<()>(
                semantic.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
            if drop_scoped_value(result).is_err() {
                note_later_contract::<()>(
                    semantic.first_fault,
                    Err(StoreRuntimeError::ConstructionInvariantUnverified),
                );
            }
            let _ = semantic.finish_rejected();
            return Err(CanonicalSemanticWorkError::Runtime(
                preserved_fault(semantic.first_fault)
                    .unwrap_or(StoreRuntimeError::ConstructionInvariantUnverified),
            ));
        }
        match result {
            Ok(value) => match semantic.finish_exact() {
                Ok(completion) => Ok((value, completion)),
                Err(cause) => {
                    if drop_scoped_value(value).is_err() {
                        note_later_contract::<()>(
                            semantic.first_fault,
                            Err(StoreRuntimeError::ConstructionInvariantUnverified),
                        );
                    }
                    Err(CanonicalSemanticWorkError::Runtime(cause))
                }
            },
            Err(rejection) => {
                // The caller keeps its domain owner; planning.reject consults the first fault.
                let _ = semantic.finish_rejected();
                Err(CanonicalSemanticWorkError::Rejected(rejection))
            }
        }
    }

    /// Seals one opaque activation plan after exact semantic work and role binding succeed.
    pub fn seal_activation<D>(
        self,
        description: D,
        completion: CanonicalSemanticCompletion<'scope>,
    ) -> CanonicalActivationSeal<
        'inv,
        'scope,
        O,
        PP,
        U,
        T,
        E,
        CanonicalKind,
        CanonicalTag,
        L,
        D::Execution,
    >
    where
        D: CanonicalActivationDescription,
    {
        let plan = description.seal_activation(CanonicalActivationPlanSealer {
            receipt_brand: self.receipt_brand,
            invariant: PhantomData,
        });
        if !std::ptr::eq(completion.receipt_brand, self.receipt_brand) {
            let _ = preserve_first_fault::<()>(
                &mut self.driver.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
        let (activation_request, requirements, plan, plan_brand) = plan.into_parts();
        if !std::ptr::eq(plan_brand, self.receipt_brand) {
            let _ = preserve_first_fault::<()>(
                &mut self.driver.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
        match self.driver.bind_success(activation_request, requirements) {
            Ok(request) => CanonicalActivationSeal::Sealed(CanonicalActivationPermit {
                driver: Some(self.driver),
                request: Some(request),
                plan: Some(plan),
                receipt_brand: self.receipt_brand,
            }),
            Err(()) => CanonicalActivationSeal::Unsealed(CanonicalActivationSealFailure {
                driver: Some(self.driver),
                plan: Some(plan),
                receipt_brand: self.receipt_brand,
            }),
        }
    }

    /// Records one verified pre-activation rejection without opening canonical storage.
    pub fn reject(self, cause: E, stable_code: u32) -> CanonicalScopedDisposition<'scope> {
        let rejection = VerifiedRejection {
            cause,
            failure_code: PreparationFailureCode::domain(stable_code),
        };
        self.driver.poll_final_planning_control();
        if let Some(fault) = self.driver.first_fault.take() {
            if drop_verified_rejection(rejection).is_err()
                || fault.contract_seen
                || fault.recoverable_code().is_none()
            {
                let _ = self.driver.contain_current();
            } else {
                self.driver.install_never_activated_runtime(fault);
            }
        } else {
            let state =
                std::mem::replace(&mut self.driver.state, ScopedActivationState::Transitioning);
            if let ScopedActivationState::Dormant(dormant) = state {
                if !self.driver.planning_is_quiescent(dormant.operation()) {
                    self.driver.state = ScopedActivationState::Dormant(dormant);
                    let _ = drop_verified_rejection(rejection);
                    let _ = self.driver.contain_current();
                } else if let Some(failure) = self.driver.failure.take() {
                    match dormant.into_operation() {
                        Ok(operation) => {
                            let slot = self
                                .driver
                                .never
                                .take()
                                .expect("requested never-activation installs its cause once");
                            let pending = NeverActivatedStoreSession::requested(
                                operation,
                                slot,
                                &failure,
                                self.driver.failure_work,
                                rejection,
                            );
                            self.driver.install_never_activated(pending);
                        }
                        Err(containment) => {
                            let _ = drop_verified_rejection(rejection);
                            self.driver.state = ScopedActivationState::Contained(containment);
                        }
                    }
                } else {
                    self.driver.state = ScopedActivationState::Dormant(dormant);
                    let _ = drop_verified_rejection(rejection);
                    let _ = self.driver.contain_current();
                }
            } else {
                self.driver.state = state;
                let _ = drop_verified_rejection(rejection);
                let _ = self.driver.contain_current();
            }
        }
        ScopedActivationDriver::<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>::receipt(
            self.receipt_brand,
        )
    }
}

struct ScopedActivationInputs<P, M> {
    plan: Option<P>,
    materialize: Option<M>,
}

impl<P, M> ScopedActivationInputs<P, M> {
    fn new(plan: P, materialize: M) -> Self {
        Self {
            plan: Some(plan),
            materialize: Some(materialize),
        }
    }

    fn plan_and_materializer_mut(&mut self) -> (&P, &mut M) {
        let Self { plan, materialize } = self;
        (
            plan.as_ref()
                .expect("the activation inputs retain their core plan until controlled drop"),
            materialize
                .as_mut()
                .expect("the scoped materializer remains owned until controlled drop"),
        )
    }

    fn drop_remaining(&mut self) -> Result<(), ()> {
        let plan_drop_failed = self
            .plan
            .take()
            .is_some_and(|plan| drop_scoped_value(plan).is_err());
        let materializer_drop_failed = self
            .materialize
            .take()
            .is_some_and(|materialize| drop_scoped_value(materialize).is_err());
        if plan_drop_failed || materializer_drop_failed {
            Err(())
        } else {
            Ok(())
        }
    }
}

impl<P, M> Drop for ScopedActivationInputs<P, M> {
    fn drop(&mut self) {
        let _ = self.drop_remaining();
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "the transition keeps the operation or assembly owner inline to prevent ownerless unwind without allocation"
)]
enum ScopedTransitionOwner<'inv, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Operation(RetainedStoreOperation<'inv, O, PP, U, L>),
    Assembly(RetainedStorePublicationAssembly<'inv, O, PP, U, L>),
}

struct ScopedOwnerTransition<'inv, 'driver, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    driver:
        &'driver mut ScopedActivationDriver<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
    owner: Option<ScopedTransitionOwner<'inv, O, PP, U, L>>,
}

impl<'inv, 'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L, P>
    CanonicalActivationSealFailure<'inv, 'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L, P>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Releases the paired core plan before resolving the seal failure.
    pub fn finish(mut self) -> CanonicalScopedDisposition<'scope> {
        let receipt_brand = self.receipt_brand;
        let plan_drop_failed = self
            .plan
            .take()
            .is_some_and(|plan| drop_scoped_value(plan).is_err());
        let driver = self
            .driver
            .take()
            .expect("an unsealed activation owns its scoped activation driver");
        if plan_drop_failed {
            let _ = driver.contain_current();
        } else if let Some(fault) = driver.first_fault.take() {
            if fault.contract_seen || fault.recoverable_code().is_none() {
                let _ = driver.contain_current();
            } else {
                driver.install_never_activated_runtime(fault);
            }
        } else {
            let _ = driver.contain_current();
        }
        ScopedActivationDriver::<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>::receipt(
            receipt_brand,
        )
    }
}

impl<'inv, 'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L, P> Drop
    for CanonicalActivationSealFailure<
        'inv,
        'scope,
        O,
        PP,
        U,
        T,
        E,
        CanonicalKind,
        CanonicalTag,
        L,
        P,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        let plan_drop_failed = self
            .plan
            .take()
            .is_some_and(|plan| drop_scoped_value(plan).is_err());
        if plan_drop_failed && let Some(driver) = self.driver.as_mut() {
            let _ = preserve_first_fault::<()>(
                &mut driver.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
        // Releasing the borrow leaves unresolved failure classification to the outer scoped
        // transaction boundary, which contains the operation after the paired plan has unwound.
        self.driver.take();
    }
}

impl<'inv, 'driver, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
    ScopedOwnerTransition<'inv, 'driver, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn begin(
        driver: &'driver mut ScopedActivationDriver<
            'inv,
            O,
            PP,
            U,
            T,
            E,
            CanonicalKind,
            CanonicalTag,
            L,
        >,
    ) -> Self {
        let state = std::mem::replace(&mut driver.state, ScopedActivationState::Transitioning);
        let ScopedActivationState::Dormant(dormant) = state else {
            driver.state = state;
            let _ = driver.contain_current();
            return Self {
                driver,
                owner: None,
            };
        };
        let owner = match dormant.into_operation() {
            Ok(operation) => Some(ScopedTransitionOwner::Operation(operation)),
            Err(containment) => {
                driver.state = ScopedActivationState::Contained(containment);
                None
            }
        };
        Self { driver, owner }
    }

    fn take_operation(&mut self) -> Option<RetainedStoreOperation<'inv, O, PP, U, L>> {
        match self.owner.take() {
            Some(ScopedTransitionOwner::Operation(operation)) => Some(operation),
            Some(owner @ ScopedTransitionOwner::Assembly(_)) => {
                self.owner = Some(owner);
                None
            }
            None => None,
        }
    }

    fn put_operation(&mut self, operation: RetainedStoreOperation<'inv, O, PP, U, L>) {
        self.owner = Some(ScopedTransitionOwner::Operation(operation));
    }

    fn operation_is_quiescent(&self) -> bool {
        match self.owner.as_ref() {
            Some(ScopedTransitionOwner::Operation(operation)) => {
                self.driver.planning_is_quiescent(operation)
            }
            Some(ScopedTransitionOwner::Assembly(_)) | None => false,
        }
    }

    fn prepare_activation(
        &mut self,
        request: &BoundScopedSuccessRequest,
        _outcome_evidence_destination_dimension: StoreDimensionKey,
    ) -> bool {
        let Some(ScopedTransitionOwner::Operation(operation)) = self.owner.as_mut() else {
            let _ = preserve_first_fault::<()>(
                &mut self.driver.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
            return false;
        };

        let original_baseline = self.driver.planning_baseline;
        let prepared = (|| {
            if request.semantic_bytes != 0
                && let Some(dimension) = request.semantic_bytes_dimension
            {
                operation
                    .producer_host()
                    .charge_observed_batch([(dimension, request.semantic_bytes)])?;
            }
            let before = operation.raw.planning_baseline();
            operation.rebind_activation_canonical_assembly(request.canonical_plan)?;
            original_baseline
                .checked_apply_rebind_delta(before, operation.raw.planning_baseline())
                .ok_or(StoreRuntimeError::ConstructionInvariantUnverified)
        })();
        match prepared {
            Ok(baseline) => self.driver.planning_baseline = baseline,
            Err(cause) => {
                let _ = preserve_first_fault::<()>(&mut self.driver.first_fault, Err(cause));
            }
        }
        self.driver.first_fault.is_none()
    }

    fn put_assembly(&mut self, assembly: RetainedStorePublicationAssembly<'inv, O, PP, U, L>) {
        self.owner = Some(ScopedTransitionOwner::Assembly(assembly));
    }

    fn assembly_mut(&mut self) -> Option<&mut RetainedStorePublicationAssembly<'inv, O, PP, U, L>> {
        match self.owner.as_mut() {
            Some(ScopedTransitionOwner::Assembly(assembly)) => Some(assembly),
            _ => None,
        }
    }

    fn assembly_is_quiescent(
        &self,
        publication_storage_baseline: RetainedPlanningBaseline,
    ) -> bool {
        match self.owner.as_ref() {
            Some(ScopedTransitionOwner::Assembly(assembly)) => self
                .driver
                .assembly_is_quiescent(assembly, publication_storage_baseline),
            Some(ScopedTransitionOwner::Operation(_)) | None => false,
        }
    }

    fn take_assembly(&mut self) -> Option<RetainedStorePublicationAssembly<'inv, O, PP, U, L>> {
        match self.owner.take() {
            Some(ScopedTransitionOwner::Assembly(assembly)) => Some(assembly),
            Some(owner @ ScopedTransitionOwner::Operation(_)) => {
                self.owner = Some(owner);
                None
            }
            None => None,
        }
    }

    fn install(
        mut self,
        pending: ScopedPendingActivation<'inv, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
    ) {
        self.owner.take();
        self.driver.install_pending(pending);
    }

    fn contain(mut self) {
        self.contain_in_place();
    }

    fn contain_in_place(&mut self) {
        let containment = match self.owner.take() {
            Some(ScopedTransitionOwner::Operation(operation)) => contain_operation(operation),
            Some(ScopedTransitionOwner::Assembly(assembly)) => StoreActivationContainment {
                cause: assembly.contain_activation_fault(),
            },
            None => self.driver.contain_current(),
        };
        self.driver.state = ScopedActivationState::Contained(containment);
    }
    /// Opens tables while the transition continues to own every success and failure branch.
    fn open_publication<P, M>(
        &mut self,
        request: &BoundScopedSuccessRequest,
        failure: &BoundScopedFailureRequest,
        inputs: &mut ScopedActivationInputs<P, M>,
    ) -> bool {
        let publication = RetainedStorePublicationPlan::success(
            request.canonical_plan,
            failure.outcome_evidence_plan,
        );
        let Some(operation) = self.take_operation() else {
            let _ = inputs.drop_remaining();
            return false;
        };
        match super::try_open_activation(operation, publication) {
            Ok(assembly) => {
                self.put_assembly(assembly);
                true
            }
            Err(ActivationOpenFailure::DeferredContainment { operation }) => {
                self.put_operation(operation);
                let _ = inputs.drop_remaining();
                self.contain_in_place();
                false
            }
            Err(ActivationOpenFailure::Recoverable {
                operation,
                cause,
                failure_code,
                fallback_only,
            }) => {
                self.put_operation(operation);
                if inputs.drop_remaining().is_err() || !self.operation_is_quiescent() {
                    self.contain_in_place();
                } else {
                    let operation = self
                        .take_operation()
                        .expect("recoverable activation retains its operation owner");
                    let slot = self
                        .driver
                        .never
                        .take()
                        .expect("recoverable open installs its cause once");
                    let pending = NeverActivatedStoreSession::runtime(
                        operation,
                        slot,
                        failure,
                        self.driver.failure_work,
                        cause,
                        failure_code,
                        fallback_only,
                    );
                    self.driver.install_never_activated(pending);
                }
                false
            }
        }
    }

    /// Resolves the private materialization after its caller-owned inputs have been destroyed.
    /// Keeping terminal-owner transitions here avoids retaining their large temporary slots
    /// across the materializer and publication-table open calls on a normal host stack.
    fn finish_materialization(
        mut self,
        request: BoundScopedSuccessRequest,
        failure: BoundScopedFailureRequest,
        materialized: CanonicalMaterializationState<T, E, CanonicalKind, CanonicalTag>,
    ) {
        let failure_work = self.driver.failure_work;
        let outcome_evidence_destination_dimension = failure.outcome_evidence_destination_dimension;
        let BoundScopedSuccessRequest {
            validation_phase,
            validation_work_dimension,
            validation_scratch_dimension,
            canonical_destination_dimension,
            validation_work,
            expected_materialized_blocks,
            ..
        } = request;
        let planning_baseline = self.driver.planning_baseline;
        let publication_storage_baseline = self
            .assembly_mut()
            .and_then(|assembly| assembly.raw.publication_storage_baseline(planning_baseline));
        let Some(publication_storage_baseline) = publication_storage_baseline else {
            let _ = drop_materialization(materialized);
            self.contain();
            return;
        };

        if !self.assembly_is_quiescent(publication_storage_baseline) {
            let _ = drop_materialization(materialized);
            self.contain();
            return;
        }

        if self
            .assembly_mut()
            .expect("materialization retains its assembly")
            .is_fallback_only()
        {
            let _ = drop_materialization(materialized);
            self.contain();
            return;
        }

        let first_fault = self.driver.first_fault.take();
        match (first_fault, materialized) {
            (Some(fault), CanonicalMaterializationState::Activated { root: _, output }) => {
                let _ = drop_scoped_value(output);
                let _fault = fault;
                self.contain();
            }
            (Some(fault), CanonicalMaterializationState::Rejected(rejection)) => {
                if fault.contract_seen {
                    let _ = drop_verified_rejection(rejection);
                    self.contain();
                } else if let Some(failure_code) = fault.recoverable_code() {
                    if drop_verified_rejection(rejection).is_err() {
                        self.contain();
                    } else {
                        let assembly = self
                            .take_assembly()
                            .expect("runtime rejection retains its assembly");
                        match finish_rejected_session(
                            assembly,
                            self.driver.rejection.take().expect(
                                "runtime materialization rejection installs its cause once",
                            ),
                            failure_work,
                            outcome_evidence_destination_dimension,
                            StoreCanonicalRejection::Runtime(fault.cause),
                            failure_code,
                        ) {
                            Ok(rejected) => {
                                self.install(ScopedPendingActivation::Rejected(rejected));
                            }
                            Err(containment) => {
                                self.driver.state = ScopedActivationState::Contained(containment);
                            }
                        }
                    }
                } else {
                    let _ = drop_verified_rejection(rejection);
                    self.contain();
                }
            }
            (None, CanonicalMaterializationState::Activated { root, output }) => {
                if self
                    .assembly_mut()
                    .expect("activated materialization retains its assembly")
                    .canonical_len()
                    != expected_materialized_blocks
                {
                    let _ = drop_scoped_value(output);
                    self.contain();
                } else {
                    let assembly = self
                        .take_assembly()
                        .expect("activated materialization consumes its assembly once");
                    let Some(slot) = self.driver.output.take() else {
                        let _ = drop_scoped_value(output);
                        self.owner = Some(ScopedTransitionOwner::Assembly(assembly));
                        self.contain();
                        return;
                    };
                    let output = match slot.place(output) {
                        Ok(output) => output,
                        Err(output) => {
                            let _ = drop_scoped_value(output);
                            self.owner = Some(ScopedTransitionOwner::Assembly(assembly));
                            self.contain();
                            return;
                        }
                    };
                    self.install(ScopedPendingActivation::Activated(
                        ActivatedCanonicalStoreSession {
                            canonical_root: root,
                            owner: PayloadOwner::new(output, assembly),
                            failure_work,
                            validation_phase,
                            validation_work_dimension,
                            validation_scratch_dimension,
                            canonical_destination_dimension,
                            outcome_evidence_destination_dimension,
                            validation_work,
                        },
                    ));
                }
            }
            (None, CanonicalMaterializationState::Rejected(rejection)) => {
                let assembly = self
                    .take_assembly()
                    .expect("requested rejection consumes its assembly once");
                match finish_rejected_session(
                    assembly,
                    self.driver
                        .rejection
                        .take()
                        .expect("requested materialization rejection installs its cause once"),
                    failure_work,
                    outcome_evidence_destination_dimension,
                    StoreCanonicalRejection::Requested(rejection.cause),
                    rejection.failure_code,
                ) {
                    Ok(rejected) => {
                        self.install(ScopedPendingActivation::Rejected(rejected));
                    }
                    Err(containment) => {
                        self.driver.state = ScopedActivationState::Contained(containment);
                    }
                }
            }
        }
    }
}

impl<'inv, 'driver, O, PP, U, T, E, CanonicalKind, CanonicalTag, L> Drop
    for ScopedOwnerTransition<'inv, 'driver, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        let containment = match self.owner.take() {
            Some(ScopedTransitionOwner::Operation(operation)) => Some(contain_operation(operation)),
            Some(ScopedTransitionOwner::Assembly(assembly)) => Some(StoreActivationContainment {
                cause: assembly.contain_activation_fault(),
            }),
            None if matches!(&self.driver.state, ScopedActivationState::Transitioning) => {
                Some(self.driver.contain_current())
            }
            None => None,
        };
        if let Some(containment) = containment {
            self.driver.state = ScopedActivationState::Contained(containment);
        }
    }
}

impl<'inv, 'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L, P>
    CanonicalActivationPermit<'inv, 'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L, P>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Retains one core-owned execution plan behind the owner-consuming activation transition.
    #[allow(
        clippy::result_large_err,
        clippy::type_complexity,
        reason = "the activation call preserves the scoped scratch owner and exact terminal state"
    )]
    pub fn activate(
        mut self,
        materialize: impl for<'lane> FnMut(
            CanonicalLane<'inv, 'lane, O, PP, U, L>,
            &P,
        ) -> CanonicalMaterialization<
            'lane,
            T,
            E,
            CanonicalKind,
            CanonicalTag,
        >,
    ) -> CanonicalScopedDisposition<'scope> {
        let receipt_brand = self.receipt_brand;
        let plan = self
            .plan
            .take()
            .expect("an activation permit owns its sealed core plan");
        let inputs = ScopedActivationInputs::new(plan, materialize);
        let driver = self
            .driver
            .take()
            .expect("an activation permit owns its scoped activation driver");
        let request = self
            .request
            .take()
            .expect("an activation permit owns its bound activation request");
        let mut transition = ScopedOwnerTransition::begin(driver);
        // Rebinding immediately after the transition makes every later unwind drop the complete
        // plan, its reservations, and the materializer before final ownership-graph containment.
        let mut inputs = inputs;
        let failure_work = transition.driver.failure_work;
        let failure = transition
            .driver
            .failure
            .take()
            .expect("sealed activation consumes the failure shape once");
        let outcome_evidence_destination_dimension = failure.outcome_evidence_destination_dimension;

        if !transition.prepare_activation(&request, outcome_evidence_destination_dimension) {
            if inputs.drop_remaining().is_err() || !transition.operation_is_quiescent() {
                transition.contain();
            } else if let Some(fault) = transition.driver.first_fault.take() {
                if fault.contract_seen {
                    transition.contain();
                } else if let Some(failure_code) = fault.recoverable_code() {
                    let operation = transition
                        .take_operation()
                        .expect("pre-activation failure retains its operation owner");
                    let slot = transition
                        .driver
                        .never
                        .take()
                        .expect("pre-activation failure installs its cause once");
                    match NeverActivatedStoreSession::runtime(
                        operation,
                        slot,
                        &failure,
                        failure_work,
                        fault.cause,
                        failure_code,
                        false,
                    ) {
                        Ok(pending) => {
                            transition.install(ScopedPendingActivation::NeverActivated(pending))
                        }
                        Err(containment) => {
                            transition.driver.state = ScopedActivationState::Contained(containment)
                        }
                    }
                } else {
                    transition.contain();
                }
            } else {
                transition.contain();
            }
            return ScopedActivationDriver::<
                O,
                PP,
                U,
                T,
                E,
                CanonicalKind,
                CanonicalTag,
                L,
            >::receipt(receipt_brand);
        }

        if !transition.open_publication(&request, &failure, &mut inputs) {
            return ScopedActivationDriver::<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>::receipt(
                receipt_brand,
            );
        }
        let (materialization_phase, _) = request.canonical_plan.keys_for_publication_validation();
        let materialization_work_dimension = request.materialization_work_dimension;
        let materialization_scratch_dimension = request.materialization_scratch_dimension;

        let materialized = catch_unwind(AssertUnwindSafe(|| {
            let ScopedOwnerTransition { driver, owner } = &mut transition;
            let assembly = match owner.as_mut() {
                Some(ScopedTransitionOwner::Assembly(assembly)) => assembly,
                _ => panic!("materialization requires one transition-owned assembly"),
            };
            let lane = CanonicalLane {
                assembly,
                first_fault: &mut driver.first_fault,
                pending_rejection: false,
                materialization_phase,
                materialization_work_dimension,
                materialization_scratch_dimension,
            };
            let (plan, materialize) = inputs.plan_and_materializer_mut();
            materialize(lane, plan).into_state()
        }));
        let inputs_drop_failed = inputs.drop_remaining().is_err();
        let materialized = match materialized {
            Ok(materialized) if !inputs_drop_failed => materialized,
            Ok(materialized) => {
                let _ = drop_materialization(materialized);
                transition.contain();
                return ScopedActivationDriver::<
                    O,
                    PP,
                    U,
                    T,
                    E,
                    CanonicalKind,
                    CanonicalTag,
                    L,
                >::receipt(receipt_brand);
            }
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                transition.contain();
                return ScopedActivationDriver::<
                    O,
                    PP,
                    U,
                    T,
                    E,
                    CanonicalKind,
                    CanonicalTag,
                    L,
                >::receipt(receipt_brand);
            }
        };

        transition.finish_materialization(request, failure, materialized);
        ScopedActivationDriver::<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>::receipt(
            receipt_brand,
        )
    }
}

impl<'inv, 'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L, P> Drop
    for CanonicalActivationPermit<'inv, 'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L, P>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        let plan_drop_failed = self
            .plan
            .take()
            .is_some_and(|plan| drop_scoped_value(plan).is_err());
        if plan_drop_failed && let Some(driver) = self.driver.as_mut() {
            let _ = preserve_first_fault::<()>(
                &mut driver.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
        self.request.take();
        // Releasing the exclusive borrow lets the outer `plan_and_activate_scoped` boundary
        // perform fail-closed containment after the paired plan and metered scratch are gone.
        self.driver.take();
    }
}

impl<'borrow, 'activation, O, PP, U> CanonicalControlCheckpoint<'borrow, 'activation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn poll(&mut self) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        preserve_first_fault(self.first_fault, self.raw.poll())
    }

    pub fn reserve(
        &mut self,
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
    ) -> Result<CanonicalReservation<'activation, O>, StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            note_later_contract(self.first_fault, self.raw.validate_phase(phase));
            note_later_contract(self.first_fault, self.raw.validate_dimension(dimension));
            return Err(cause);
        }
        preserve_first_fault(self.first_fault, self.raw.reserve(phase, dimension, bytes)).map(
            |raw| CanonicalReservation {
                raw,
                scope: PhantomData,
            },
        )
    }

    pub fn resize(
        &mut self,
        reservation: &mut CanonicalReservation<'activation, O>,
        bytes: u64,
    ) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        preserve_first_fault(
            self.first_fault,
            self.raw.resize(&mut reservation.raw, bytes),
        )
    }

    pub fn validate_live_owner(
        &mut self,
        reservation: &CanonicalReservation<'activation, O>,
    ) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        preserve_first_fault(
            self.first_fault,
            self.raw.validate_live_owner(&reservation.raw),
        )
    }
}

struct SemanticCopyGuard<'a> {
    first_fault: &'a mut Option<ActivationFirstFault>,
    armed: bool,
}

impl Drop for SemanticCopyGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            note_later_contract::<()>(
                self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
    }
}

impl<'borrow, 'scope, O, PP, U, L> CanonicalSemanticWorkSession<'borrow, 'scope, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn note_contract_if_untrusted(&mut self) {
        if self.first_fault.is_none() {
            let _ = preserve_first_fault::<()>(
                self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
    }

    fn poll_now(&mut self) -> Result<(), StoreRuntimeError> {
        self.close_copy_units()?;
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        preserve_first_fault(
            self.first_fault,
            self.context.ordinary_storage().checkpoint(),
        )
    }

    /// Polls when the invocation's bounded action checkpoint has been reached.
    pub fn poll_if_due(&mut self) -> Result<(), StoreRuntimeError> {
        self.close_copy_units()?;
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        preserve_first_fault(self.first_fault, self.context.before_metered_action(1))
    }

    fn finish_checkpoint(&mut self) -> Result<(), StoreRuntimeError> {
        let poll = self.poll_now();
        if let Err(cause) = self.context.check_work_quiescent() {
            let _ = preserve_first_fault::<()>(self.first_fault, Err(cause));
            note_later_contract::<()>(
                self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
        poll?;
        preserved_fault(self.first_fault).map_or(Ok(()), Err)
    }

    fn finish_action_callback<F, R>(
        first_fault: &mut Option<ActivationFirstFault>,
        action: ManuallyDrop<Option<F>>,
        result: Result<R, StoreRuntimeError>,
    ) -> Result<R, StoreRuntimeError> {
        let result = preserve_first_fault(first_fault, result);
        if drop_optional_callback(action).is_err() {
            note_later_contract::<()>(
                first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
            if let Ok(value) = result {
                let _ = drop_scoped_value(value);
            }
            return Err(preserved_fault(first_fault)
                .expect("callback drop failure preserves a semantic contract fault"));
        }
        result
    }

    /// Runs one semantic action after bounded control polling and records it only on normal return.
    pub fn run_unit<R>(&mut self, action: impl FnMut() -> R) -> Result<R, StoreRuntimeError> {
        let mut action = ManuallyDrop::new(Some(action));
        if let Err(cause) = self.poll_if_due() {
            return Self::finish_action_callback(self.first_fault, action, Err(cause));
        }
        let start = self.context.admit_action(self.work_group, [1]);
        let mut observed = match preserve_first_fault(self.first_fault, start) {
            Ok(observed) => observed,
            Err(cause) => {
                return Self::finish_action_callback(self.first_fault, action, Err(cause));
            }
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            action
                .as_mut()
                .expect("the semantic action remains owned until controlled drop")()
        }));
        let result = match result {
            Ok(result) => result,
            Err(payload) => {
                note_later_contract::<()>(
                    self.first_fault,
                    Err(StoreRuntimeError::ConstructionInvariantUnverified),
                );
                let _ = observed.finish_partial();
                if drop_optional_callback(action).is_err() {
                    note_later_contract::<()>(
                        self.first_fault,
                        Err(StoreRuntimeError::ConstructionInvariantUnverified),
                    );
                }
                std::panic::resume_unwind(payload);
            }
        };
        if let Err(cause) = observed.complete([1]).map_err(StoreRuntimeError::Ledger) {
            let _ = preserve_first_fault::<()>(self.first_fault, Err(cause.clone()));
            if observed.finish_partial().is_err() {
                note_later_contract::<()>(
                    self.first_fault,
                    Err(StoreRuntimeError::ConstructionInvariantUnverified),
                );
            }
            let result = preserve_runtime_fault_then_drop_result(self.first_fault, cause, result);
            return Self::finish_action_callback(self.first_fault, action, result);
        }
        if let Err(cause) = observed.finish().map_err(StoreRuntimeError::Ledger) {
            let result = preserve_runtime_fault_then_drop_result(self.first_fault, cause, result);
            return Self::finish_action_callback(self.first_fault, action, result);
        }
        self.completed_any = true;
        if let Err(cause) = self.context.completed_metered_action(1) {
            let result = preserve_runtime_fault_then_drop_result(self.first_fault, cause, result);
            return Self::finish_action_callback(self.first_fault, action, result);
        }
        Self::finish_action_callback(self.first_fault, action, Ok(result))
    }

    /// Runs one scalar check without an owned callback or result destructor.
    ///
    /// Every call admits its real unit before execution and completes it only on normal return,
    /// including a returned domain error. Consecutive checks reuse one zero-remaining frame;
    /// storage, general callbacks, polls and finish close that frame before transferring control.
    /// A caught panic still rejects the semantic pass and retains its exact completed prefix.
    ///
    /// Owned captures use [`Self::run_unit`] and its controlled disposal:
    ///
    /// ```compile_fail
    /// use baozi_operation::{OperationOwner, ProgressPhase, ProgressUnit};
    /// use baozi_operation::__retained_store_integration::activation::CanonicalSemanticWorkSession;
    /// fn owned<O: OperationOwner, P: ProgressPhase, U: ProgressUnit>(
    ///     semantic: &mut CanonicalSemanticWorkSession<'_, '_, O, P, U>,
    /// ) {
    ///     let value = String::from("owned capture");
    ///     let _ = semantic.run_copy_unit(move || value.len());
    /// }
    /// ```
    pub fn run_copy_unit<R: Copy>(
        &mut self,
        action: impl FnOnce() -> R + Copy,
    ) -> Result<R, StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        let mut guard = SemanticCopyGuard {
            first_fault: self.first_fault,
            armed: true,
        };
        let result = self
            .context
            .run_copy_unit(self.work_group, &mut self.copy_frame, action);
        guard.armed = false;
        let result = preserve_first_fault(guard.first_fault, result);
        if result.is_ok() {
            self.completed_any = true;
        }
        result
    }

    fn close_copy_units(&mut self) -> Result<(), StoreRuntimeError> {
        let result = preserve_first_fault(
            self.first_fault,
            self.context
                .close_copy_units(self.work_group, &mut self.copy_frame),
        );
        preserved_fault(self.first_fault).map_or(result, Err)
    }

    /// Reserves metered construction scratch in the policy-owned authoring role.
    pub fn reserve(
        &mut self,
        bytes: u64,
    ) -> Result<CanonicalReservation<'scope, O>, StoreRuntimeError> {
        self.close_copy_units()?;
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        preserve_first_fault(
            self.first_fault,
            self.context
                .ordinary_storage()
                .reserve(self.phase, self.scratch_dimension, bytes),
        )
        .map(|raw| CanonicalReservation {
            raw,
            scope: PhantomData,
        })
    }

    /// Resizes one scoped reservation while retaining it unchanged on rejection.
    pub fn resize(
        &mut self,
        reservation: &mut CanonicalReservation<'scope, O>,
        bytes: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.poll_now()?;
        preserve_first_fault(
            self.first_fault,
            self.context
                .ordinary_storage()
                .resize(&mut reservation.raw, bytes),
        )
    }

    /// Proves that one scoped reservation belongs to this operation.
    pub fn validate_live_owner(
        &mut self,
        reservation: &CanonicalReservation<'scope, O>,
    ) -> Result<(), StoreRuntimeError> {
        self.poll_now()?;
        preserve_first_fault(
            self.first_fault,
            self.context.validate_live_owner(&reservation.raw),
        )
    }

    /// Completes one nonempty action-derived semantic prefix.
    fn finish_exact(&mut self) -> Result<CanonicalSemanticCompletion<'scope>, StoreRuntimeError> {
        let receipt_brand = self.receipt_brand;
        let result = self.finish_checkpoint();
        if let Err(cause) = result {
            *self.state = ScopedSemanticState::Rejected;
            self.resolved = true;
            return Err(cause);
        }
        self.resolved = true;
        if self.completed_any {
            *self.state = ScopedSemanticState::Complete;
            Ok(CanonicalSemanticCompletion {
                receipt_brand,
                invariant: PhantomData,
            })
        } else {
            *self.state = ScopedSemanticState::Rejected;
            self.note_contract_if_untrusted();
            Err(preserved_fault(self.first_fault)
                .unwrap_or(StoreRuntimeError::ConstructionInvariantUnverified))
        }
    }

    /// Resolves a verified semantic rejection without requiring a success completion token.
    fn finish_rejected(&mut self) -> Result<(), StoreRuntimeError> {
        let result = self.finish_checkpoint();
        *self.state = ScopedSemanticState::Rejected;
        self.resolved = true;
        result
    }
}

impl<O, PP, U, L> Drop for CanonicalSemanticWorkSession<'_, '_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        let _ = self.close_copy_units();
        if self.resolved {
            return;
        }
        self.note_contract_if_untrusted();
        *self.state = ScopedSemanticState::Rejected;
        self.resolved = true;
    }
}

impl<'borrow, 'activation, O, PP, U>
    CanonicalMaterializationCheckpoint<'borrow, 'activation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn poll(&mut self) -> Result<(), StoreRuntimeError> {
        self.inner.poll()
    }

    pub fn reserve(
        &mut self,
        bytes: u64,
    ) -> Result<CanonicalReservation<'activation, O>, StoreRuntimeError> {
        self.inner.reserve(self.phase, self.dimension, bytes)
    }

    pub fn resize(
        &mut self,
        reservation: &mut CanonicalReservation<'activation, O>,
        bytes: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.inner.resize(reservation, bytes)
    }

    pub fn validate_live_owner(
        &mut self,
        reservation: &CanonicalReservation<'activation, O>,
    ) -> Result<(), StoreRuntimeError> {
        self.inner.validate_live_owner(reservation)
    }
}

impl<'inv, 'activation, O, PP, U, L> CanonicalLane<'inv, 'activation, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn len(&self) -> usize {
        self.assembly.canonical_len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[cfg(test)]
    pub(in crate::concrete_store) fn test_dimension_evidence(
        &self,
        dimension: StoreDimensionKey,
    ) -> Option<crate::DimensionSnapshot> {
        self.assembly.raw.test_dimension_evidence(dimension)
    }

    pub fn control_checkpoint<'borrow>(
        &'borrow mut self,
    ) -> Result<CanonicalMaterializationCheckpoint<'borrow, 'activation, O, PP, U>, StoreRuntimeError>
    {
        if let Some(cause) = self.entry_failure() {
            return Err(cause);
        }
        match self.assembly.try_control_checkpoint() {
            Ok(raw) => Ok(CanonicalMaterializationCheckpoint {
                inner: CanonicalControlCheckpoint {
                    raw,
                    first_fault: self.first_fault,
                    activation: PhantomData,
                },
                phase: self.materialization_phase,
                dimension: self.materialization_scratch_dimension,
            }),
            Err(cause) => preserve_first_fault(self.first_fault, Err(cause)),
        }
    }

    pub fn try_retain_sequence<T, Tag>(
        &mut self,
        plan: SequencePlan<T>,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut CanonicalBuildControl<'_, '_, 'control, O, PP, U, SequenceWriter<T>>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        if let Some(cause) = self.entry_failure() {
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let work = match canonical_build_work(Some(plan.element_count())) {
            Ok(work) => work,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(cause)),
                );
            }
        };
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        let result = try_retain_activation_canonical_writer(
            self.assembly,
            allocation,
            self.materialization_work_dimension,
            work,
            tag,
            || <T as super::super::sealed::StoredScalar>::new_writer(plan.capacity, plan.len),
            |control| {
                build
                    .as_mut()
                    .expect("the canonical sequence builder is executed at most once")(
                    control
                )
            },
            SequenceWriter::finish,
        );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn try_retain_bytes<Tag>(
        &mut self,
        plan: BytesPlan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut CanonicalBuildControl<'_, '_, 'control, O, PP, U, BytesWriter>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalBytesRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        if let Some(cause) = self.entry_failure() {
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let work = match canonical_build_work(Some(plan.bytes())) {
            Ok(work) => work,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(cause)),
                );
            }
        };
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        let result = try_retain_activation_canonical_writer(
            self.assembly,
            allocation,
            self.materialization_work_dimension,
            work,
            tag,
            || BytesWriter::new(plan),
            |control| {
                build
                    .as_mut()
                    .expect("the canonical bytes builder is executed at most once")(
                    control
                )
            },
            BytesWriter::finish,
        );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn try_retain_utf8<Tag>(
        &mut self,
        plan: Utf8Plan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut CanonicalBuildControl<'_, '_, 'control, O, PP, U, Utf8Writer>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalUtf8Ref<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        if let Some(cause) = self.entry_failure() {
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let work = match canonical_build_work(Some(plan.bytes())) {
            Ok(work) => work,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(cause)),
                );
            }
        };
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        let result = try_retain_activation_canonical_writer(
            self.assembly,
            allocation,
            self.materialization_work_dimension,
            work,
            tag,
            || Utf8Writer::new(plan),
            |control| {
                build
                    .as_mut()
                    .expect("the canonical UTF-8 builder is executed at most once")(
                    control
                )
            },
            Utf8Writer::finish,
        );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn try_retain_records<Tag>(
        &mut self,
        plan: RecordPlan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut CanonicalBuildControl<'_, '_, 'control, O, PP, U, CanonicalRecordWriter>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        if let Some(cause) = self.entry_failure() {
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let work = match canonical_build_work(plan.rows().checked_mul(plan.fields())) {
            Ok(work) => work,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(cause)),
                );
            }
        };
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        let result = try_retain_activation_canonical_writer(
            self.assembly,
            allocation,
            self.materialization_work_dimension,
            work,
            tag,
            || CanonicalRecordWriter::new(plan),
            |control| {
                build
                    .as_mut()
                    .expect("the canonical record builder is executed at most once")(
                    control
                )
            },
            CanonicalRecordWriter::finish,
        );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn complete<T, E, Kind, Tag>(
        mut self,
        root: CanonicalStoreRef<Kind, Tag>,
        output: T,
    ) -> CanonicalMaterialization<'activation, T, E, Kind, Tag>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let _ = self.entry_failure();
        if !self.assembly.contains_activation_canonical_root(root) {
            let _ = preserve_first_fault::<()>(
                self.first_fault,
                Err(StoreRuntimeError::InvalidSourceSlot),
            );
        }
        CanonicalMaterialization {
            state: CanonicalMaterializationState::Activated { root, output },
            lane: PhantomData,
        }
    }

    pub fn reject<T, E, Kind, Tag>(
        self,
        cause: E,
        stable_code: u32,
    ) -> CanonicalMaterialization<'activation, T, E, Kind, Tag> {
        CanonicalMaterialization {
            state: CanonicalMaterializationState::Rejected(VerifiedRejection {
                cause,
                failure_code: PreparationFailureCode::domain(stable_code),
            }),
            lane: PhantomData,
        }
    }
}

impl<'inv, 'activation, O, PP, U>
    CanonicalLane<'inv, 'activation, O, PP, U, CanonicalInheritanceSource>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Binds the exact canonical source selected when the derived operation started.
    pub fn try_bind_canonical_source<SourceRootKind, SourceRootTag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
    ) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = self.entry_failure() {
            note_later_contract(
                self.first_fault,
                self.assembly
                    .require_canonical_source_from(source.share_source()),
            );
            return Err(cause);
        }
        preserve_first_fault(
            self.first_fault,
            self.assembly
                .try_bind_canonical_source(source, self.materialization_work_dimension),
        )
    }

    /// Shares one immutable leaf from the exact bound source into the derived candidate.
    pub fn try_share_canonical<SourceRootKind, SourceRootTag, SlotKind, SlotTag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalStoreRef<SlotKind, SlotTag>,
    ) -> Result<CanonicalStoreRef<SlotKind, SlotTag>, StoreRuntimeError>
    where
        SlotKind: ShareableStoreKind,
        SlotTag: Copy + Send + Sync + 'static,
    {
        if let Some(cause) = self.entry_failure() {
            note_later_contract(
                self.first_fault,
                self.assembly
                    .validate_canonical_share_source_from(source.share_source(), source_ref),
            );
            return Err(cause);
        }
        preserve_first_fault(
            self.first_fault,
            self.assembly.try_share_canonical(source, source_ref),
        )
    }

    /// Replaces one exact-source scalar sequence through destination-plan-derived work control.
    pub fn try_rewrite_sequence<SourceRootKind, SourceRootTag, T, Tag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalSequenceRef<T, Tag>,
        plan: SequencePlan<T>,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut CanonicalBuildControl<'_, '_, 'control, O, PP, U, SequenceWriter<T>>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalSequenceRef<T, Tag>, StoreRuntimeError>
    where
        T: StoredScalar,
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        let failure = self.entry_failure();
        let mut validate_source = || {
            self.assembly
                .require_canonical_source_from(source.share_source())?;
            super::super::read_canonical_source_sequence(source.share_source(), source_ref)
                .map(|_| ())
                .ok_or(StoreRuntimeError::InvalidSourceSlot)
        };
        if let Some(cause) = failure {
            note_later_contract(self.first_fault, validate_source());
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        if let Err(cause) = validate_source() {
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let work = match canonical_build_work(Some(plan.element_count())) {
            Ok(work) => work,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(cause)),
                );
            }
        };
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        let result = try_rewrite_activation_canonical_writer(
            self.assembly,
            source,
            source_ref,
            allocation,
            self.materialization_work_dimension,
            work,
            tag,
            || <T as super::super::sealed::StoredScalar>::new_writer(plan.capacity, plan.len),
            |control| {
                build
                    .as_mut()
                    .expect("the canonical sequence rewrite is executed at most once")(
                    control
                )
            },
            SequenceWriter::finish,
        );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    /// Replaces one exact-source record block with destination-authored rows.
    pub fn try_rewrite_records<SourceRootKind, SourceRootTag, Tag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalRecordRef<Tag>,
        plan: RecordPlan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut CanonicalBuildControl<'_, '_, 'control, O, PP, U, CanonicalRecordWriter>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let mut build = ManuallyDrop::new(Some(build));
        let failure = self.entry_failure();
        let mut validate_source = || {
            self.assembly
                .require_canonical_source_from(source.share_source())?;
            super::super::read_canonical_source_records(source.share_source(), source_ref)
                .map(|_| ())
                .ok_or(StoreRuntimeError::InvalidSourceSlot)
        };
        if let Some(cause) = failure {
            note_later_contract(self.first_fault, validate_source());
            note_later_contract(
                self.first_fault,
                plan.allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        if let Err(cause) = validate_source() {
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let work = match canonical_build_work(plan.rows().checked_mul(plan.fields())) {
            Ok(work) => work,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(cause)),
                );
            }
        };
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        let result = try_rewrite_activation_canonical_writer(
            self.assembly,
            source,
            source_ref,
            allocation,
            self.materialization_work_dimension,
            work,
            tag,
            || CanonicalRecordWriter::new(plan),
            |control| {
                build
                    .as_mut()
                    .expect("the canonical record rewrite is executed at most once")(
                    control
                )
            },
            CanonicalRecordWriter::finish,
        );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    /// Copies one exact-source record block while mapping every canonical reference.
    pub fn try_copy_mapped_records<SourceRootKind, SourceRootTag, Tag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalRecordRef<Tag>,
        plan: RecordPlan,
        tag: Tag,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        self.try_copy_mapped_records_with_append(source, source_ref, plan, plan, tag, |_writer| {
            Ok(())
        })
    }

    /// Copies a mapped exact-source prefix and appends rows inside one destination-plan permit.
    #[allow(
        clippy::too_many_arguments,
        reason = "both exact record plans remain explicit while work is derived only from the destination plan"
    )]
    pub fn try_copy_mapped_records_with_append<SourceRootKind, SourceRootTag, Tag>(
        &mut self,
        source: &PublishedCanonicalStore<SourceRootKind, SourceRootTag>,
        source_ref: CanonicalRecordRef<Tag>,
        source_plan: RecordPlan,
        destination_plan: RecordPlan,
        tag: Tag,
        append: impl for<'control> FnMut(
            &mut CanonicalBuildControl<'_, '_, 'control, O, PP, U, CanonicalRecordWriter>,
        ) -> Result<(), StoreControlledError>,
    ) -> Result<CanonicalRecordRef<Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
    {
        let mut append = ManuallyDrop::new(Some(append));
        let failure = self.entry_failure();
        let mut validate_source = || {
            self.assembly
                .require_canonical_source_from(source.share_source())?;
            let source_records =
                super::super::read_canonical_source_records(source.share_source(), source_ref)
                    .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
            if source_records.rows() != source_plan.rows()
                || source_records.fields() != source_plan.fields()
                || source_plan.fields() != destination_plan.fields()
                || source_plan.rows() > destination_plan.rows()
            {
                return Err(StoreRuntimeError::InvalidSourceSlot);
            }
            Ok(())
        };
        if let Some(cause) = failure {
            note_later_contract(self.first_fault, validate_source());
            note_later_contract(
                self.first_fault,
                destination_plan
                    .allocation_plan()
                    .map(|_| ())
                    .map_err(StoreRuntimeError::Plan),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(append, Err(cause)),
            );
        }
        if let Err(cause) = validate_source() {
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(append, Err(cause)),
            );
        }
        let work = match canonical_build_work(
            destination_plan
                .rows()
                .checked_mul(destination_plan.fields()),
        ) {
            Ok(work) => work,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(append, Err(cause)),
                );
            }
        };
        let allocation = match destination_plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(append, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        let result = try_copy_activation_canonical_records(
            self.assembly,
            source,
            source_ref,
            source_plan,
            destination_plan,
            allocation,
            self.materialization_work_dimension,
            work,
            tag,
            |control| {
                append
                    .as_mut()
                    .expect("the mapped record append is executed at most once")(
                    control
                )
            },
        );
        preserve_first_fault(self.first_fault, finish_scoped_callback(append, result))
    }
}
