use super::*;

fn canonical_build_work(units: Option<usize>) -> Result<NonZeroU64, StoreRuntimeError> {
    let units = units.ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    let units =
        u64::try_from(units).map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    NonZeroU64::new(units).ok_or(StoreRuntimeError::Plan(StorePlanError::EmptyPayload))
}

impl<O, PP, U, L> RetainedStoreOperation<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Moves the operation into the single scoped planning-and-activation transaction.
    pub fn into_scoped_session_outcome_owner(
        self,
        policy: CanonicalActivationPolicy,
        failure_requirements: CanonicalFailureRequirements,
    ) -> Result<ScopedRetainedStoreSessionOutcomeOwner<O, PP, U, L>, StoreActivationContainment>
    {
        let (canonical_phase, canonical_dimensions) =
            policy.canonical_writer.keys_for_publication_validation();
        let (evidence_phase, evidence_dimensions) = policy
            .outcome_evidence_writer
            .keys_for_publication_validation();
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
            .is_ok();
        if !roles_are_valid {
            return Err(contain_operation(self));
        }
        let failure = match policy.bind_scoped_failure(failure_requirements) {
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
        let planning_baseline = self.raw.planning_baseline();
        let containment_anchor = self.raw.planning_containment_anchor();
        Ok(ScopedRetainedStoreSessionOutcomeOwner {
            dormant: ScopedDormantActivation {
                operation: Some(self),
                containment_anchor,
                armed: true,
            },
            roles: BoundActivationRoles { policy },
            scoped_failure: failure,
            planning_baseline,
            failure_work: FailureEvidenceWorkBudget::new(failure_work),
        })
    }
}

impl<O, PP, U, L> ScopedRetainedStoreSessionOutcomeOwner<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Runs one higher-ranked planning transaction whose core plan owns all scoped scratch.
    #[allow(
        clippy::result_large_err,
        clippy::type_complexity,
        reason = "the HRTB transaction preserves the exact linear outcome owner"
    )]
    pub fn plan_and_activate_scoped<T, E, CanonicalKind, CanonicalTag>(
        self,
        transaction: impl for<'scope> FnMut(
            CanonicalPlanning<'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
        ) -> CanonicalScopedDisposition<'scope>,
    ) -> Result<
        CanonicalActivationOutcome<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
        StoreActivationContainment,
    > {
        let Self {
            dormant,
            roles,
            scoped_failure,
            planning_baseline,
            failure_work,
        } = self;
        let (operation, containment_anchor) = dormant.into_parts()?;
        let mut transaction = ManuallyDrop::new(Some(transaction));
        let receipt_brand = ScopedReceiptBrand { _private: 0 };
        let mut driver = ScopedActivationDriver {
            state: ScopedActivationState::Planning(operation),
            containment_anchor,
            roles: Some(roles),
            failure: Some(scoped_failure),
            planning_baseline,
            failure_work,
            first_fault: None,
            semantic_state: ScopedSemanticState::NotStarted,
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
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
}

fn contain_scoped_pending<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>(
    pending: ScopedPendingActivation<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
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
                output,
                failure_work: _,
                validation_phase: _,
                validation_work_dimension: _,
                validation_scratch_dimension: _,
                canonical_destination_dimension: _,
                outcome_evidence_destination_dimension: _,
                validation_work: _,
                assembly,
            } = activated;
            let _ = drop_scoped_value(output);
            StoreActivationContainment {
                cause: assembly.contain_activation_fault(),
            }
        }
        ScopedPendingActivation::Rejected(rejected) => {
            let RejectedCanonicalStoreSession { owner } = rejected;
            let UnrootedFailureEvidence {
                cause,
                domain: _,
                failure_code: _,
                failure_work: _,
                outcome_evidence_destination_dimension: _,
                raw,
            } = owner;
            let _ = drop_scoped_value(cause);
            StoreActivationContainment {
                cause: raw.contain_activation_fault(),
            }
        }
        ScopedPendingActivation::NeverActivated(never) => {
            let NeverActivatedStoreSession {
                cause,
                failure_code: _,
                outcome_evidence_plan: _,
                failure_work: _,
                outcome_evidence_destination_dimension: _,
                authority,
            } = never;
            let _ = drop_scoped_value(cause);
            match authority {
                NeverActivatedAuthority::Operation(operation)
                | NeverActivatedAuthority::FallbackOnly(operation) => contain_operation(operation),
            }
        }
    }
}

impl<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
    ScopedActivationDriver<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
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

    fn planning_operation_mut(&mut self) -> Option<&mut RetainedStoreOperation<O, PP, U, L>> {
        match &mut self.state {
            ScopedActivationState::Planning(operation) => Some(operation),
            ScopedActivationState::Transitioning
            | ScopedActivationState::Pending(_)
            | ScopedActivationState::Contained(_)
            | ScopedActivationState::Finished => None,
        }
    }

    fn poll_final_planning_control(&mut self) {
        let ScopedActivationDriver {
            state, first_fault, ..
        } = self;
        let result = match state {
            ScopedActivationState::Planning(operation) => operation.control_checkpoint().poll(),
            ScopedActivationState::Transitioning
            | ScopedActivationState::Pending(_)
            | ScopedActivationState::Contained(_)
            | ScopedActivationState::Finished => {
                Err(StoreRuntimeError::ConstructionInvariantUnverified)
            }
        };
        let _ = preserve_first_fault(first_fault, result);
    }

    fn planning_is_quiescent(&self, operation: &RetainedStoreOperation<O, PP, U, L>) -> bool {
        !matches!(self.semantic_state, ScopedSemanticState::Active)
            && operation
                .raw
                .planning_matches_baseline(self.planning_baseline)
    }

    fn assembly_is_quiescent(
        &self,
        assembly: &RetainedStorePublicationAssembly<O, PP, U, L>,
        publication_storage_baseline: RetainedPlanningBaseline,
    ) -> bool {
        !matches!(self.semantic_state, ScopedSemanticState::Active)
            && assembly
                .raw
                .planning_matches_baseline(publication_storage_baseline)
    }

    fn install_pending(
        &mut self,
        pending: ScopedPendingActivation<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
    ) {
        self.state = ScopedActivationState::Pending(pending);
        self.roles.take();
    }

    fn contain_current(&mut self) -> StoreActivationContainment {
        self.roles.take();
        self.failure.take();
        let state = std::mem::replace(&mut self.state, ScopedActivationState::Transitioning);
        let containment = match state {
            ScopedActivationState::Planning(operation) => contain_operation(operation),
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
        reason = "the temporary U4e migration driver returns the complete inline linear terminal owner without allocation and is deleted at core cutover"
    )]
    fn finish(
        &mut self,
    ) -> Result<
        CanonicalActivationOutcome<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
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
            ScopedActivationState::Planning(operation) => {
                let containment = contain_operation(operation);
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
        requirements: CanonicalSuccessRequirements,
    ) -> Result<BoundScopedSuccessRequest, ()> {
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
        let request = match roles.policy.bind_scoped_success(requirements) {
            Ok(request) => request,
            Err(_) => {
                let _ = preserve_first_fault::<()>(
                    &mut self.first_fault,
                    Err(StoreRuntimeError::ConstructionInvariantUnverified),
                );
                return Err(());
            }
        };
        let Some(failure) = self.failure.as_ref() else {
            let _ = preserve_first_fault::<()>(
                &mut self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
            return Err(());
        };
        let outcome_evidence_destination_dimension = failure.outcome_evidence_destination_dimension;
        let semantic_bytes_dimension = roles.policy.semantic_bytes_dimension;
        let (phase, dimensions) = request.canonical_plan.keys_for_publication_validation();
        let original_baseline = self.planning_baseline;
        let Some(operation) = self.planning_operation_mut() else {
            let _ = preserve_first_fault::<()>(
                &mut self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
            return Err(());
        };
        let writer = operation.validate_writer_keys(phase, dimensions);
        let activation_dimensions = operation.validate_activation_dimensions([
            request.materialization_work_dimension,
            request.materialization_scratch_dimension,
            request.validation_work_dimension,
            request.validation_scratch_dimension,
            request.canonical_destination_dimension,
            outcome_evidence_destination_dimension,
        ]);
        let semantic_dimension = semantic_bytes_dimension.map_or(Ok(()), |dimension| {
            operation.validate_activation_dimensions([dimension])
        });
        let semantic_bytes = if writer.is_ok()
            && activation_dimensions.is_ok()
            && semantic_dimension.is_ok()
            && request.semantic_bytes != 0
        {
            semantic_bytes_dimension.map_or(Ok(()), |dimension| {
                operation
                    .producer_host()
                    .charge_observed_batch([(dimension, request.semantic_bytes)])
            })
        } else {
            Ok(())
        };
        let (canonical_assembly, rebound_baseline) = if writer.is_ok()
            && activation_dimensions.is_ok()
            && semantic_dimension.is_ok()
            && semantic_bytes.is_ok()
        {
            let before = operation.raw.planning_baseline();
            let canonical_assembly =
                operation.rebind_activation_canonical_assembly(request.canonical_plan);
            let rebound_baseline = canonical_assembly.as_ref().ok().and_then(|_| {
                original_baseline
                    .checked_apply_rebind_delta(before, operation.raw.planning_baseline())
            });
            (Some(canonical_assembly), rebound_baseline)
        } else {
            (None, None)
        };
        let _ = preserve_first_fault(&mut self.first_fault, writer);
        let _ = preserve_first_fault(&mut self.first_fault, activation_dimensions);
        let _ = preserve_first_fault(&mut self.first_fault, semantic_dimension);
        let _ = preserve_first_fault(&mut self.first_fault, semantic_bytes);
        if let Some(canonical_assembly) = canonical_assembly {
            let succeeded = canonical_assembly.is_ok();
            let _ = preserve_first_fault(&mut self.first_fault, canonical_assembly);
            if succeeded && rebound_baseline.is_none() {
                let _ = preserve_first_fault::<()>(
                    &mut self.first_fault,
                    Err(StoreRuntimeError::ConstructionInvariantUnverified),
                );
            }
        }
        if let Some(baseline) = rebound_baseline {
            self.planning_baseline = baseline;
        }
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
        let state = std::mem::replace(&mut self.state, ScopedActivationState::Transitioning);
        let ScopedActivationState::Planning(operation) = state else {
            self.state = state;
            let _ = self.contain_current();
            return;
        };
        if !self.planning_is_quiescent(&operation) {
            self.state = ScopedActivationState::Planning(operation);
            let _ = self.contain_current();
            return;
        }
        let Some(failure) = self.failure.take() else {
            self.state = ScopedActivationState::Planning(operation);
            let _ = self.contain_current();
            return;
        };
        self.install_pending(ScopedPendingActivation::NeverActivated(
            NeverActivatedStoreSession::runtime(
                operation,
                failure.outcome_evidence_plan,
                self.failure_work,
                failure.outcome_evidence_destination_dimension,
                fault.cause,
                failure_code,
                false,
            ),
        ));
    }
}

impl<O, PP, U, T, E, CanonicalKind, CanonicalTag, L> Drop
    for ScopedActivationDriver<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
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

impl<'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
    CanonicalPlanning<'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Starts one semantic-work session whose actions are admitted by operation as they execute.
    ///
    /// No caller-declared total crosses this boundary. Each [`CanonicalSemanticWorkSession::run_unit`]
    /// call records its own attempted unit before the action and its consumed unit only after the
    /// action returns normally.
    pub fn begin_semantic_work<'borrow>(
        &'borrow mut self,
    ) -> Result<CanonicalSemanticWorkSession<'borrow, 'scope, O, PP, U, L>, StoreRuntimeError> {
        let receipt_brand = self.receipt_brand;
        let ScopedActivationDriver {
            state,
            roles,
            first_fault,
            semantic_state,
            ..
        } = self.driver;
        if !matches!(semantic_state, ScopedSemanticState::NotStarted) {
            return preserve_first_fault(
                first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
        let Some(roles) = roles.as_ref() else {
            return preserve_first_fault(
                first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        };
        let (phase, work_dimension, scratch_dimension) = roles.policy.authoring_keys();
        let operation = match state {
            ScopedActivationState::Planning(operation) => operation,
            _ => {
                return preserve_first_fault(
                    first_fault,
                    Err(StoreRuntimeError::ConstructionInvariantUnverified),
                );
            }
        };
        *semantic_state = ScopedSemanticState::Active;
        Ok(CanonicalSemanticWorkSession {
            operation,
            first_fault,
            phase,
            work_dimension,
            scratch_dimension,
            state: semantic_state,
            receipt_brand,
            work_batch: work_batch::CompletionPrefixBatch::new(),
            completed_any: false,
            resolved: false,
            scope: PhantomData,
        })
    }

    /// Seals one payload-free activation permit after exact semantic work and role binding succeed.
    pub fn seal_activation(
        self,
        requirements: CanonicalSuccessRequirements,
        completion: CanonicalSemanticCompletion<'scope>,
    ) -> CanonicalActivationSeal<'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L> {
        if !std::ptr::eq(completion.receipt_brand, self.receipt_brand) {
            let _ = preserve_first_fault::<()>(
                &mut self.driver.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
        match self.driver.bind_success(requirements) {
            Ok(request) => CanonicalActivationSeal::Sealed(CanonicalActivationPermit {
                driver: Some(self.driver),
                request: Some(request),
                receipt_brand: self.receipt_brand,
            }),
            Err(()) => CanonicalActivationSeal::Unsealed(CanonicalActivationSealFailure {
                driver: Some(self.driver),
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
            if let ScopedActivationState::Planning(operation) = state {
                if !self.driver.planning_is_quiescent(&operation) {
                    self.driver.state = ScopedActivationState::Planning(operation);
                    let _ = drop_verified_rejection(rejection);
                    let _ = self.driver.contain_current();
                } else if let Some(failure) = self.driver.failure.take() {
                    self.driver
                        .install_pending(ScopedPendingActivation::NeverActivated(
                            NeverActivatedStoreSession::requested(
                                operation,
                                failure.outcome_evidence_plan,
                                self.driver.failure_work,
                                failure.outcome_evidence_destination_dimension,
                                rejection,
                            ),
                        ));
                } else {
                    self.driver.state = ScopedActivationState::Planning(operation);
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
    reason = "the temporary U4e transition keeps the operation or assembly owner inline to prevent ownerless unwind without allocation and is deleted at core cutover"
)]
enum ScopedTransitionOwner<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Operation(RetainedStoreOperation<O, PP, U, L>),
    Assembly(RetainedStorePublicationAssembly<O, PP, U, L>),
}

struct ScopedOwnerTransition<'driver, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    driver: &'driver mut ScopedActivationDriver<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
    owner: Option<ScopedTransitionOwner<O, PP, U, L>>,
}

impl<'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
    CanonicalActivationSealFailure<'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Resolves the seal failure after the caller has released its complete core-owned plan.
    pub fn finish_after_plan_drop(mut self) -> CanonicalScopedDisposition<'scope> {
        let receipt_brand = self.receipt_brand;
        let driver = self
            .driver
            .take()
            .expect("an unsealed activation owns its scoped activation driver");
        if let Some(fault) = driver.first_fault.take() {
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

impl<O, PP, U, T, E, CanonicalKind, CanonicalTag, L> Drop
    for CanonicalActivationSealFailure<'_, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        // Releasing only the borrow leaves unresolved failure classification to the outer scoped
        // transaction boundary, which contains the operation after caller-owned plans unwind.
        self.driver.take();
    }
}

impl<'driver, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
    ScopedOwnerTransition<'driver, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn begin(
        driver: &'driver mut ScopedActivationDriver<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
    ) -> Self {
        let state = std::mem::replace(&mut driver.state, ScopedActivationState::Transitioning);
        let ScopedActivationState::Planning(operation) = state else {
            driver.state = state;
            let _ = driver.contain_current();
            return Self {
                driver,
                owner: None,
            };
        };
        Self {
            driver,
            owner: Some(ScopedTransitionOwner::Operation(operation)),
        }
    }

    fn take_operation(&mut self) -> Option<RetainedStoreOperation<O, PP, U, L>> {
        match self.owner.take() {
            Some(ScopedTransitionOwner::Operation(operation)) => Some(operation),
            Some(owner @ ScopedTransitionOwner::Assembly(_)) => {
                self.owner = Some(owner);
                None
            }
            None => None,
        }
    }

    fn put_operation(&mut self, operation: RetainedStoreOperation<O, PP, U, L>) {
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

    fn put_assembly(&mut self, assembly: RetainedStorePublicationAssembly<O, PP, U, L>) {
        self.owner = Some(ScopedTransitionOwner::Assembly(assembly));
    }

    fn assembly_mut(&mut self) -> Option<&mut RetainedStorePublicationAssembly<O, PP, U, L>> {
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

    fn take_assembly(&mut self) -> Option<RetainedStorePublicationAssembly<O, PP, U, L>> {
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
        pending: ScopedPendingActivation<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
    ) {
        self.owner.take();
        self.driver.install_pending(pending);
    }

    fn contain(mut self) {
        let containment = match self.owner.take() {
            Some(ScopedTransitionOwner::Operation(operation)) => contain_operation(operation),
            Some(ScopedTransitionOwner::Assembly(assembly)) => StoreActivationContainment {
                cause: assembly.contain_activation_fault(),
            },
            None => self.driver.contain_current(),
        };
        self.driver.state = ScopedActivationState::Contained(containment);
    }
}

impl<O, PP, U, T, E, CanonicalKind, CanonicalTag, L> Drop
    for ScopedOwnerTransition<'_, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
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

impl<'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
    CanonicalActivationPermit<'scope, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
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
    pub fn activate<P>(
        mut self,
        plan: P,
        materialize: impl for<'lane> FnMut(
            CanonicalLane<'lane, O, PP, U, L>,
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
        let BoundScopedSuccessRequest {
            canonical_plan,
            materialization_work_dimension,
            materialization_scratch_dimension,
            validation_phase,
            validation_work_dimension,
            validation_scratch_dimension,
            canonical_destination_dimension,
            validation_work,
            expected_materialized_blocks,
            semantic_bytes: _,
        } = request;
        let (materialization_phase, _) = canonical_plan.keys_for_publication_validation();
        let outcome_evidence_destination_dimension = failure.outcome_evidence_destination_dimension;
        let publication_plan =
            RetainedStorePublicationPlan::success(canonical_plan, failure.outcome_evidence_plan);
        let Some(operation) = transition.take_operation() else {
            let _ = inputs.drop_remaining();
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
        };
        let opened = super::try_open_activation(operation, publication_plan);
        let assembly = match opened {
            Ok(assembly) => assembly,
            Err(ActivationOpenFailure::DeferredContainment { operation }) => {
                transition.put_operation(operation);
                let _ = inputs.drop_remaining();
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
            Err(ActivationOpenFailure::Recoverable {
                operation,
                cause,
                failure_code,
                fallback_only,
            }) => {
                transition.put_operation(operation);
                if inputs.drop_remaining().is_err() || !transition.operation_is_quiescent() {
                    transition.contain();
                } else {
                    let operation = transition
                        .take_operation()
                        .expect("recoverable activation retains its operation owner");
                    transition.install(ScopedPendingActivation::NeverActivated(
                        NeverActivatedStoreSession::runtime(
                            operation,
                            failure.outcome_evidence_plan,
                            failure_work,
                            outcome_evidence_destination_dimension,
                            cause,
                            failure_code,
                            fallback_only,
                        ),
                    ));
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
        };
        transition.put_assembly(assembly);

        let materialized = catch_unwind(AssertUnwindSafe(|| {
            let ScopedOwnerTransition { driver, owner } = &mut transition;
            let assembly = match owner.as_mut() {
                Some(ScopedTransitionOwner::Assembly(assembly)) => assembly,
                _ => panic!("materialization requires one transition-owned assembly"),
            };
            let lane = CanonicalLane {
                assembly,
                first_fault: &mut driver.first_fault,
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

        let planning_baseline = transition.driver.planning_baseline;
        let publication_storage_baseline = transition
            .assembly_mut()
            .and_then(|assembly| assembly.raw.publication_storage_baseline(planning_baseline));
        let Some(publication_storage_baseline) = publication_storage_baseline else {
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
        };

        if !transition.assembly_is_quiescent(publication_storage_baseline) {
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

        if transition
            .assembly_mut()
            .expect("materialization retains its assembly")
            .is_fallback_only()
        {
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

        let first_fault = transition.driver.first_fault.take();
        match (first_fault, materialized) {
            (Some(fault), CanonicalMaterializationState::Activated { root: _, output }) => {
                let _ = drop_scoped_value(output);
                let _fault = fault;
                transition.contain();
            }
            (Some(fault), CanonicalMaterializationState::Rejected(rejection)) => {
                if fault.contract_seen {
                    let _ = drop_verified_rejection(rejection);
                    transition.contain();
                } else if let Some(failure_code) = fault.recoverable_code() {
                    if drop_verified_rejection(rejection).is_err() {
                        transition.contain();
                    } else {
                        let assembly = transition
                            .take_assembly()
                            .expect("runtime rejection retains its assembly");
                        match finish_rejected_session(
                            assembly,
                            failure_work,
                            outcome_evidence_destination_dimension,
                            StoreCanonicalRejection::Runtime(fault.cause),
                            failure_code,
                        ) {
                            Ok(rejected) => {
                                transition.install(ScopedPendingActivation::Rejected(rejected));
                            }
                            Err(containment) => {
                                transition.driver.state =
                                    ScopedActivationState::Contained(containment);
                            }
                        }
                    }
                } else {
                    let _ = drop_verified_rejection(rejection);
                    transition.contain();
                }
            }
            (None, CanonicalMaterializationState::Activated { root, output }) => {
                if transition
                    .assembly_mut()
                    .expect("activated materialization retains its assembly")
                    .canonical_len()
                    != expected_materialized_blocks
                {
                    let _ = drop_scoped_value(output);
                    transition.contain();
                } else {
                    let assembly = transition
                        .take_assembly()
                        .expect("activated materialization consumes its assembly once");
                    transition.install(ScopedPendingActivation::Activated(
                        ActivatedCanonicalStoreSession {
                            canonical_root: root,
                            output,
                            failure_work,
                            validation_phase,
                            validation_work_dimension,
                            validation_scratch_dimension,
                            canonical_destination_dimension,
                            outcome_evidence_destination_dimension,
                            validation_work,
                            assembly,
                        },
                    ));
                }
            }
            (None, CanonicalMaterializationState::Rejected(rejection)) => {
                let assembly = transition
                    .take_assembly()
                    .expect("requested rejection consumes its assembly once");
                match finish_rejected_session(
                    assembly,
                    failure_work,
                    outcome_evidence_destination_dimension,
                    StoreCanonicalRejection::Requested(rejection.cause),
                    rejection.failure_code,
                ) {
                    Ok(rejected) => {
                        transition.install(ScopedPendingActivation::Rejected(rejected));
                    }
                    Err(containment) => {
                        transition.driver.state = ScopedActivationState::Contained(containment);
                    }
                }
            }
        }
        ScopedActivationDriver::<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>::receipt(
            receipt_brand,
        )
    }
}

impl<O, PP, U, T, E, CanonicalKind, CanonicalTag, L> Drop
    for CanonicalActivationPermit<'_, O, PP, U, T, E, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        self.request.take();
        // The permit contains no core payload. Releasing only this exclusive borrow lets the
        // transaction callback destroy its private plan and metered scratch before the outer
        // `plan_and_activate_scoped` boundary performs fail-closed containment.
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
        let mut checkpoint = CanonicalControlCheckpoint {
            raw: self.operation.control_checkpoint(),
            first_fault: &mut *self.first_fault,
            activation: PhantomData,
        };
        checkpoint.poll()?;
        self.work_batch.note_poll();
        Ok(())
    }

    /// Flushes and polls when the bounded completion checkpoint has been reached.
    pub fn poll_if_due(&mut self) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        if self.work_batch.poll_due() {
            self.poll_now()?;
        }
        Ok(())
    }

    /// Runs one semantic action after bounded control polling and records it only on normal return.
    pub fn run_unit<R>(&mut self, action: impl FnMut() -> R) -> Result<R, StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        if self.work_batch.poll_due() {
            // Observed-work admission performs the matching control poll immediately before the
            // action, so it also satisfies this bounded checkpoint without a duplicate poll.
            self.work_batch.note_poll();
        }
        let mut host = self.operation.producer_host();
        let start = host.try_begin_observed_work([(self.work_dimension, 1)]);
        let mut observed = preserve_first_fault(self.first_fault, start)?;
        let mut action = ManuallyDrop::new(Some(action));
        let result = catch_unwind(AssertUnwindSafe(|| {
            action
                .as_mut()
                .expect("the semantic action remains owned until controlled drop")()
        }));
        let action_drop_failed = drop_optional_callback(action).is_err();
        if action_drop_failed && self.first_fault.is_none() {
            let _ = preserve_first_fault::<()>(
                self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
        let result = match result {
            Ok(result) => result,
            Err(payload) => {
                observed.finish_partial();
                self.note_contract_if_untrusted();
                std::panic::resume_unwind(payload);
            }
        };
        if let Err(cause) = observed.complete([1]) {
            observed.finish_partial();
            return preserve_runtime_fault_then_drop_result(self.first_fault, cause, result);
        }
        if let Err(cause) = observed.finish() {
            return preserve_runtime_fault_then_drop_result(self.first_fault, cause, result);
        }
        self.completed_any = true;
        if self.work_batch.note_observed_unit().is_err() {
            self.note_contract_if_untrusted();
            let _ = drop_scoped_value(result);
            return Err(StoreRuntimeError::ConstructionInvariantUnverified);
        }
        if action_drop_failed {
            let _ = drop_scoped_value(result);
            return Err(StoreRuntimeError::ConstructionInvariantUnverified);
        }
        Ok(result)
    }

    /// Reserves metered construction scratch in the policy-owned authoring role.
    pub fn reserve(
        &mut self,
        bytes: u64,
    ) -> Result<CanonicalReservation<'scope, O>, StoreRuntimeError> {
        self.poll_if_due()?;
        let phase = self.phase;
        let dimension = self.scratch_dimension;
        let mut checkpoint = CanonicalControlCheckpoint {
            raw: self.operation.control_checkpoint(),
            first_fault: &mut *self.first_fault,
            activation: PhantomData,
        };
        checkpoint.reserve(phase, dimension, bytes)
    }

    /// Resizes one scoped reservation while retaining it unchanged on rejection.
    pub fn resize(
        &mut self,
        reservation: &mut CanonicalReservation<'scope, O>,
        bytes: u64,
    ) -> Result<(), StoreRuntimeError> {
        self.poll_if_due()?;
        let mut checkpoint = CanonicalControlCheckpoint {
            raw: self.operation.control_checkpoint(),
            first_fault: &mut *self.first_fault,
            activation: PhantomData,
        };
        checkpoint.resize(reservation, bytes)
    }

    /// Proves that one scoped reservation belongs to this operation.
    pub fn validate_live_owner(
        &mut self,
        reservation: &CanonicalReservation<'scope, O>,
    ) -> Result<(), StoreRuntimeError> {
        self.poll_if_due()?;
        let mut checkpoint = CanonicalControlCheckpoint {
            raw: self.operation.control_checkpoint(),
            first_fault: &mut *self.first_fault,
            activation: PhantomData,
        };
        checkpoint.validate_live_owner(reservation)
    }

    /// Completes one nonempty action-derived semantic prefix.
    pub fn finish_exact(
        mut self,
    ) -> Result<CanonicalSemanticCompletion<'scope>, StoreRuntimeError> {
        let receipt_brand = self.receipt_brand;
        let result = self.poll_now();
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
    pub fn finish_rejected(mut self) -> Result<(), StoreRuntimeError> {
        let result = self.poll_now();
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

impl<'activation, O, PP, U, L> CanonicalLane<'activation, O, PP, U, L>
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
    pub(in crate::concrete_store) fn test_ledger_probe(
        &self,
    ) -> crate::ledger::LedgerTestProbe<StorePhaseKey, StoreDimensionKey> {
        self.assembly.raw.test_ledger_probe()
    }

    pub fn control_checkpoint<'borrow>(
        &'borrow mut self,
    ) -> Result<CanonicalMaterializationCheckpoint<'borrow, 'activation, O, PP, U>, StoreRuntimeError>
    {
        if let Some(cause) = preserved_fault(self.first_fault) {
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
        if let Some(cause) = preserved_fault(self.first_fault) {
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
        if let Some(cause) = preserved_fault(self.first_fault) {
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
        if let Some(cause) = preserved_fault(self.first_fault) {
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
        if let Some(cause) = preserved_fault(self.first_fault) {
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
        self,
        root: CanonicalStoreRef<Kind, Tag>,
        output: T,
    ) -> CanonicalMaterialization<'activation, T, E, Kind, Tag>
    where
        Tag: Copy + Send + Sync + 'static,
    {
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

impl<'activation, O, PP, U> CanonicalLane<'activation, O, PP, U, CanonicalInheritanceSource>
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
        if let Some(cause) = preserved_fault(self.first_fault) {
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
        if let Some(cause) = preserved_fault(self.first_fault) {
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
        let mut validate_source = || {
            self.assembly
                .require_canonical_source_from(source.share_source())?;
            super::super::read_canonical_source_sequence(source.share_source(), source_ref)
                .map(|_| ())
                .ok_or(StoreRuntimeError::InvalidSourceSlot)
        };
        if let Some(cause) = preserved_fault(self.first_fault) {
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
        let mut validate_source = || {
            self.assembly
                .require_canonical_source_from(source.share_source())?;
            super::super::read_canonical_source_records(source.share_source(), source_ref)
                .map(|_| ())
                .ok_or(StoreRuntimeError::InvalidSourceSlot)
        };
        if let Some(cause) = preserved_fault(self.first_fault) {
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
        if let Some(cause) = preserved_fault(self.first_fault) {
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
