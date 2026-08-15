use super::*;

fn canonical_build_work(units: Option<usize>) -> Result<NonZeroU64, StoreRuntimeError> {
    let units = units.ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    let units =
        u64::try_from(units).map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    NonZeroU64::new(units).ok_or(StoreRuntimeError::Plan(StorePlanError::EmptyPayload))
}

/// Owns the semantic-preflight permit while completed units accumulate in bounded chunks.
///
/// The guard is deliberately local to preflight. It keeps the raw permit alive until the exact
/// completion prefix has been flushed, including when an inspector unwinds or the caller drops
/// the preflight callback. No producer-provided work amount crosses this boundary.
struct SemanticWorkBatch<'borrow, 'operation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    permit: Option<CanonicalWorkPermit<'borrow, 'operation, O, PP, U>>,
    work_batch: work_batch::CompletionPrefixBatch,
}

impl<'borrow, 'operation, O, PP, U> SemanticWorkBatch<'borrow, 'operation, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn new(permit: CanonicalWorkPermit<'borrow, 'operation, O, PP, U>) -> Self {
        Self {
            permit: Some(permit),
            work_batch: work_batch::CompletionPrefixBatch::new(),
        }
    }

    fn fail(&mut self) -> StoreRuntimeError {
        if let Some(permit) = self.permit.as_mut() {
            permit.record_product_graph_failure(ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE);
        }
        if let Some(mut permit) = self.permit.take() {
            permit.resolve_partial();
        }
        StoreRuntimeError::ConstructionInvariantUnverified
    }

    fn flush(&mut self) -> Result<(), StoreRuntimeError> {
        let pending = self.work_batch.unflushed_completed();
        if pending == 0 {
            return Ok(());
        }
        let remaining = self
            .permit
            .as_ref()
            .expect("a live semantic batch retains its permit")
            .remaining_units();
        if pending > remaining {
            let _ = self.work_batch.take_completed();
            return Err(self.fail());
        }
        let completed = self
            .work_batch
            .take_completed()
            .expect("a nonzero semantic completion prefix remains flushable");
        self.permit
            .as_mut()
            .expect("a live semantic batch retains its permit")
            .complete_units(completed)
            .map_err(StoreWorkError::into_runtime)
    }

    fn poll_now(&mut self) -> Result<(), StoreRuntimeError> {
        self.flush()?;
        let permit = self
            .permit
            .as_mut()
            .expect("a live semantic batch retains its permit");
        permit.poll().map_err(StoreWorkError::into_runtime)?;
        self.work_batch.note_poll();
        Ok(())
    }

    fn complete_unit(&mut self) -> Result<(), StoreRuntimeError> {
        self.work_batch
            .note_completed_unit()
            .map_err(|_| self.fail())
    }

    fn finish_exact(mut self) -> Result<(), StoreRuntimeError> {
        self.poll_now()?;
        let remaining = self
            .permit
            .as_ref()
            .expect("a live semantic batch retains its permit")
            .remaining_units();
        if remaining != 0 {
            return Err(self.fail());
        }
        let permit = self
            .permit
            .take()
            .expect("a successful semantic batch retains its permit");
        if permit.finish_exact() {
            Ok(())
        } else {
            Err(StoreRuntimeError::ConstructionInvariantUnverified)
        }
    }
}

impl<O, PP, U> Drop for SemanticWorkBatch<'_, '_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _ = self.flush();
            if let Some(mut permit) = self.permit.take() {
                permit.resolve_partial();
            }
        }));
        if let Err(payload) = result {
            discard_untrusted_panic_payload(payload);
        }
    }
}

impl<O, PP, U> RetainedStoreOperation<O, PP, U, FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Moves the operation into the session-owned dormant activation seam.
    pub fn into_session_outcome_owner(
        self,
        policy: CanonicalActivationPolicy,
    ) -> (
        RetainedStoreSessionOutcomeOwner<O, PP, U, FreshStoreLineage>,
        CanonicalActivationRequestSealer,
    ) {
        let brand = self.domain.activation_brand();
        let (max_evidence_members, construction_work_limit, validation_work_limit) = self
            .raw
            .failure_terminal_work_limits()
            .expect("every concrete retained-store operation owns terminal failure work");
        let failure_work = FailureEvidenceWorkEnvelope {
            max_evidence_members,
            construction_work_limit,
            validation_work_limit,
        };
        (
            RetainedStoreSessionOutcomeOwner {
                operation: Some(self),
                roles: Some(BoundActivationRoles { policy, brand }),
                failure_work: FailureEvidenceWorkBudget::new(failure_work),
            },
            CanonicalActivationRequestSealer { brand },
        )
    }

    /// Moves the operation into the single scoped planning-and-activation transaction.
    pub fn into_scoped_session_outcome_owner(
        self,
        policy: CanonicalActivationPolicy,
        failure_requirements: CanonicalFailureRequirements,
    ) -> Result<
        ScopedRetainedStoreSessionOutcomeOwner<O, PP, U, FreshStoreLineage>,
        StoreActivationContainment,
    > {
        let brand = self.domain.activation_brand();
        let (canonical_phase, canonical_dimensions) =
            policy.canonical_writer.keys_for_publication_validation();
        let (evidence_phase, evidence_dimensions) = policy
            .outcome_evidence_writer
            .keys_for_publication_validation();
        let roles_are_valid = self
            .validate_writer_keys(canonical_phase, canonical_dimensions)
            .and_then(|()| self.validate_writer_keys(evidence_phase, evidence_dimensions))
            .and_then(|()| {
                self.validate_activation_dimensions([
                    policy.materialization_work_dimension,
                    policy.validation_work_dimension,
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
            roles: BoundActivationRoles { policy, brand },
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
        transaction: impl for<'scope> FnOnce(
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
                .take()
                .expect("the scoped activation transaction executes at most once");
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

impl<O, PP, U, L> RetainedStoreSessionOutcomeOwner<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Runs authority-free preflight and, only after it seals a plan, opens one scoped lane.
    ///
    /// An explicit preflight rejection is `NeverActivated`. A rollback-proved physical-open
    /// failure is also `NeverActivated`, with its exact store cause. Once the lane is exposed, a
    /// typed rejection eagerly destroys canonical storage before returning `Rejected`. Contract
    /// faults, ignored first failures, panics, and uncertain cleanup return containment instead of
    /// a publishable disposition.
    #[allow(
        clippy::result_large_err,
        clippy::type_complexity,
        reason = "the scoped transition preserves every linear owner and exact typed cause"
    )]
    // U4e migration: remove this two-callback rail in the same commit that switches the core
    // tracer to `plan_and_activate_scoped`.
    pub fn plan_and_activate<T, E, CanonicalKind, CanonicalTag>(
        mut self,
        request: SealedCanonicalActivationRequest,
        preflight: impl for<'scope> FnMut(
            CanonicalPreflight<'scope, O, PP, U, L>,
        ) -> CanonicalPreflightDecision<'scope, E>,
        materialize: impl for<'lane> FnMut(
            CanonicalLane<'lane, O, PP, U, L>,
        ) -> CanonicalMaterialization<
            'lane,
            T,
            E,
            CanonicalKind,
            CanonicalTag,
        >,
    ) -> Result<
        CanonicalActivationOutcome<O, PP, U, T, E, CanonicalKind, CanonicalTag, L>,
        StoreActivationContainment,
    > {
        let mut preflight = ManuallyDrop::new(Some(preflight));
        let mut materialize = ManuallyDrop::new(Some(materialize));
        let mut operation = self
            .operation
            .take()
            .expect("the session outcome owner begins activation at most once");
        let roles = self
            .roles
            .take()
            .expect("the session outcome owner consumes its activation roles at most once");
        let request = match roles.bind(request) {
            Ok(request) => request,
            Err(_) => {
                let _ = drop_optional_callback(preflight);
                let _ = drop_optional_callback(materialize);
                return Err(contain_operation(operation));
            }
        };
        let failure_work = self.failure_work;
        let mut first_fault = None;
        let mut semantic_work_complete = request.semantic_preflight_work == 0;
        let preflight_result = catch_unwind(AssertUnwindSafe(|| {
            let preflight = preflight
                .as_mut()
                .expect("the preflight callback is executed at most once");
            preflight(CanonicalPreflight {
                operation: &mut operation,
                request: &request,
                first_fault: &mut first_fault,
                semantic_work_complete: &mut semantic_work_complete,
            })
            .into_state()
        }));
        let preflight_drop_failed = drop_optional_callback(preflight).is_err();
        let preflight = match preflight_result {
            Ok(preflight) if !preflight_drop_failed => preflight,
            Ok(preflight) => {
                let _ = drop_scoped_value(preflight);
                let _ = drop_optional_callback(materialize);
                return Err(contain_operation(operation));
            }
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                let materialize_drop_failed = drop_optional_callback(materialize).is_err();
                if preflight_drop_failed || materialize_drop_failed {
                    return Err(contain_operation(operation));
                }
                let (cause, failure_code) = match first_fault {
                    Some(fault) if fault.contract_seen => return Err(contain_operation(operation)),
                    Some(fault) => {
                        let Some(failure_code) = fault.recoverable_code() else {
                            return Err(contain_operation(operation));
                        };
                        (fault.cause, failure_code)
                    }
                    None => (
                        activation_product_graph_cause(),
                        PreparationFailureCode::product_graph(
                            ACTIVATION_PRODUCT_GRAPH_FAILURE_CODE,
                        ),
                    ),
                };
                return prepare_operation_minimal(operation, failure_code, cause)
                    .map(CanonicalActivationOutcome::Fallback);
            }
        };

        match preflight {
            CanonicalPreflightState::Rejected(rejection) => {
                let outcome_evidence_plan = request.outcome_evidence_plan;
                if drop_optional_callback(materialize).is_err() {
                    let _ = drop_verified_rejection(rejection);
                    return Err(contain_operation(operation));
                }
                if let Some(fault) = first_fault {
                    if fault.contract_seen {
                        let _ = drop_verified_rejection(rejection);
                        return Err(contain_operation(operation));
                    }
                    let Some(failure_code) = fault.recoverable_code() else {
                        let _ = drop_verified_rejection(rejection);
                        return Err(contain_operation(operation));
                    };
                    if drop_verified_rejection(rejection).is_err() {
                        return Err(contain_operation(operation));
                    }
                    return Ok(CanonicalActivationOutcome::NeverActivated(
                        NeverActivatedStoreSession::runtime(
                            operation,
                            outcome_evidence_plan,
                            failure_work,
                            request.outcome_evidence_destination_dimension,
                            fault.cause,
                            failure_code,
                            false,
                        ),
                    ));
                }
                Ok(CanonicalActivationOutcome::NeverActivated(
                    NeverActivatedStoreSession::requested(
                        operation,
                        outcome_evidence_plan,
                        failure_work,
                        request.outcome_evidence_destination_dimension,
                        rejection,
                    ),
                ))
            }
            CanonicalPreflightState::Activate => {
                let BoundActivationRequest {
                    canonical_plan,
                    outcome_evidence_plan,
                    materialization_work_dimension,
                    validation_work_dimension,
                    canonical_destination_dimension,
                    outcome_evidence_destination_dimension,
                    semantic_preflight_work: _,
                    validation_work,
                    expected_materialized_blocks,
                } = request;
                if first_fault.is_some() {
                    let _ = drop_optional_callback(materialize);
                    return Err(contain_operation(operation));
                }
                let publication_plan =
                    RetainedStorePublicationPlan::success(canonical_plan, outcome_evidence_plan);
                let mut assembly = match try_open_activation(operation, publication_plan) {
                    Ok(assembly) => assembly,
                    Err(ActivationOpenFailure::DeferredContainment { operation }) => {
                        let _ = drop_optional_callback(materialize);
                        return Err(contain_operation(operation));
                    }
                    Err(ActivationOpenFailure::Recoverable {
                        operation,
                        cause,
                        failure_code,
                        fallback_only,
                    }) => {
                        if drop_optional_callback(materialize).is_err() {
                            return Err(contain_operation(operation));
                        }
                        return Ok(CanonicalActivationOutcome::NeverActivated(
                            NeverActivatedStoreSession::runtime(
                                operation,
                                outcome_evidence_plan,
                                failure_work,
                                outcome_evidence_destination_dimension,
                                cause,
                                failure_code,
                                fallback_only,
                            ),
                        ));
                    }
                };

                let mut first_fault = None;
                let materialized = catch_unwind(AssertUnwindSafe(|| {
                    let lane = CanonicalLane {
                        assembly: &mut assembly,
                        first_fault: &mut first_fault,
                        materialization_work_dimension,
                    };
                    let materialize = materialize
                        .as_mut()
                        .expect("the materialization callback is executed at most once");
                    materialize(lane).into_state()
                }));
                let materialize_drop_failed = drop_optional_callback(materialize).is_err();
                let materialized = match materialized {
                    Ok(materialized) if !materialize_drop_failed => materialized,
                    Ok(materialized) => {
                        let _ = drop_materialization(materialized);
                        return Err(StoreActivationContainment {
                            cause: assembly.contain_activation_fault(),
                        });
                    }
                    Err(payload) => {
                        discard_untrusted_panic_payload(payload);
                        return Err(StoreActivationContainment {
                            cause: assembly.contain_activation_fault(),
                        });
                    }
                };

                if assembly.is_fallback_only() {
                    match materialized {
                        CanonicalMaterializationState::Activated { root: _, output } => {
                            let _ = drop_scoped_value(output);
                        }
                        CanonicalMaterializationState::Rejected(rejection) => {
                            let _ = drop_verified_rejection(rejection);
                        }
                    }
                    return Err(StoreActivationContainment {
                        cause: assembly.contain_activation_fault(),
                    });
                }

                match (first_fault, materialized) {
                    (Some(fault), CanonicalMaterializationState::Activated { root: _, output }) => {
                        let _ = drop_scoped_value(output);
                        let _fault = fault;
                        Err(StoreActivationContainment {
                            cause: assembly.contain_activation_fault(),
                        })
                    }
                    (Some(fault), CanonicalMaterializationState::Rejected(rejection)) => {
                        if fault.contract_seen {
                            let _ = drop_verified_rejection(rejection);
                            return Err(StoreActivationContainment {
                                cause: assembly.contain_activation_fault(),
                            });
                        }
                        let Some(failure_code) = fault.recoverable_code() else {
                            let _ = drop_verified_rejection(rejection);
                            return Err(StoreActivationContainment {
                                cause: assembly.contain_activation_fault(),
                            });
                        };
                        if drop_verified_rejection(rejection).is_err() {
                            return Err(StoreActivationContainment {
                                cause: assembly.contain_activation_fault(),
                            });
                        }
                        finish_rejected(
                            assembly,
                            failure_work,
                            outcome_evidence_destination_dimension,
                            StoreCanonicalRejection::Runtime(fault.cause),
                            failure_code,
                        )
                    }
                    (None, CanonicalMaterializationState::Activated { root, output }) => {
                        if assembly.canonical_len() != expected_materialized_blocks {
                            let _ = drop_scoped_value(output);
                            return Err(StoreActivationContainment {
                                cause: assembly.contain_activation_fault(),
                            });
                        }
                        Ok(CanonicalActivationOutcome::Activated(
                            ActivatedCanonicalStoreSession {
                                assembly,
                                canonical_root: root,
                                output,
                                failure_work,
                                validation_work_dimension,
                                canonical_destination_dimension,
                                outcome_evidence_destination_dimension,
                                validation_work,
                            },
                        ))
                    }
                    (None, CanonicalMaterializationState::Rejected(rejection)) => finish_rejected(
                        assembly,
                        failure_work,
                        outcome_evidence_destination_dimension,
                        StoreCanonicalRejection::Requested(rejection.cause),
                        rejection.failure_code,
                    ),
                }
            }
        }
    }
}

impl<O, PP, U, L> Drop for RetainedStoreSessionOutcomeOwner<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if let Some(operation) = self.operation.take() {
            let _ = contain_operation(operation);
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
                validation_work_dimension: _,
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
        completed_semantic_work: NonZeroU64,
    ) -> Result<BoundScopedSuccessRequest, ()> {
        self.poll_final_planning_control();
        let semantic_complete = matches!(self.semantic_state, ScopedSemanticState::Complete);
        let semantic_work_matches = requirements.semantic_preflight_work == completed_semantic_work;
        if !semantic_complete || !semantic_work_matches {
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
        let (phase, dimensions) = request.canonical_plan.keys_for_publication_validation();
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
            request.validation_work_dimension,
            request.canonical_destination_dimension,
            outcome_evidence_destination_dimension,
        ]);
        let _ = preserve_first_fault(&mut self.first_fault, writer);
        let _ = preserve_first_fault(&mut self.first_fault, activation_dimensions);
        if !semantic_complete || !semantic_work_matches || self.first_fault.is_some() {
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
    /// Admits one exact semantic-work envelope before any producer action executes.
    pub fn begin_semantic_work<'borrow>(
        &'borrow mut self,
        attempted_units: NonZeroU64,
    ) -> Result<CanonicalSemanticWorkSession<'borrow, 'scope, O, PP, U>, StoreRuntimeError> {
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
        let (phase, dimension) = roles.policy.authoring_keys();
        let amount = attempted_units;
        let operation = match state {
            ScopedActivationState::Planning(operation) => operation,
            _ => {
                return preserve_first_fault(
                    first_fault,
                    Err(StoreRuntimeError::ConstructionInvariantUnverified),
                );
            }
        };
        let control = operation.activation_work_control(dimension);
        let control = preserve_first_fault(first_fault, control)?;
        let RetainedStoreWorkControl { raw, sticky: _ } = control;
        let permit = match raw.try_begin(amount) {
            Ok(permit) => permit,
            Err(RetainedWorkStartError::Control(cause)) => {
                return preserve_first_fault(first_fault, Err(StoreRuntimeError::Control(cause)));
            }
            Err(RetainedWorkStartError::Ledger(cause)) => {
                return preserve_first_fault(first_fault, Err(StoreRuntimeError::Ledger(cause)));
            }
            Err(RetainedWorkStartError::Outstanding) => {
                return preserve_first_fault(
                    first_fault,
                    Err(StoreRuntimeError::ConstructionInvariantUnverified),
                );
            }
        };
        *semantic_state = ScopedSemanticState::Active;
        Ok(CanonicalSemanticWorkSession {
            permit: Some(permit),
            first_fault,
            phase,
            state: semantic_state,
            receipt_brand,
            admitted_units: amount,
            work_batch: work_batch::CompletionPrefixBatch::new(),
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
        match self
            .driver
            .bind_success(requirements, completion.completed_units)
        {
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

    fn take_plan(&mut self) -> P {
        self.plan
            .take()
            .expect("the activation inputs move their core plan at most once")
    }

    fn take_materializer(&mut self) -> M {
        self.materialize
            .take()
            .expect("the scoped materializer executes at most once")
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
    /// Moves one core-owned execution plan behind the owner-consuming activation transition.
    #[allow(
        clippy::result_large_err,
        clippy::type_complexity,
        reason = "the activation call preserves the scoped scratch owner and exact terminal state"
    )]
    pub fn activate<P>(
        mut self,
        plan: P,
        materialize: impl for<'lane> FnOnce(
            CanonicalLane<'lane, O, PP, U, L>,
            P,
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
            validation_work_dimension,
            canonical_destination_dimension,
            validation_work,
            expected_materialized_blocks,
        } = request;
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
                materialization_work_dimension,
            };
            let plan = inputs.take_plan();
            let materialize = inputs.take_materializer();
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
                            validation_work_dimension,
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

impl<O, PP, U, L> RetainedStoreOperation<O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn activation_work_control(
        &mut self,
        dimension: StoreDimensionKey,
    ) -> Result<RetainedStoreWorkControl<'_, O, PP, U>, StoreRuntimeError> {
        self.validate_activation_dimensions([dimension; 4])?;
        let (raw, _) = self.raw.split_work_control(dimension);
        Ok(RetainedStoreWorkControl { raw, sticky: None })
    }
}

impl<'activation, O, PP, U, L> CanonicalPreflight<'activation, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Borrows the pre-activation control and work capability with first-fault preservation.
    pub fn control_checkpoint<'borrow>(
        &'borrow mut self,
    ) -> CanonicalControlCheckpoint<'borrow, 'activation, O, PP, U> {
        CanonicalControlCheckpoint {
            raw: self.operation.control_checkpoint(),
            first_fault: self.first_fault,
            activation: PhantomData,
        }
    }

    /// Runs every unit in the sealed semantic-preflight envelope exactly once.
    ///
    /// The full attempted envelope is admitted before the first inspector call. Each unit becomes
    /// consumed only after its inspector returns normally, and control is polled at bounded
    /// completion checkpoints. Returning `false` rejects the semantics but does not short-circuit
    /// the remaining evidence.
    pub fn inspect_semantic_work(
        &mut self,
        mut inspect: impl FnMut(u64) -> bool,
    ) -> Result<bool, StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        if *self.semantic_work_complete || self.request.semantic_preflight_work == 0 {
            return preserve_first_fault(
                self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
        let amount = NonZeroU64::new(self.request.semantic_preflight_work)
            .expect("zero semantic work is handled before admission");
        let dimension = self.request.materialization_work_dimension;
        let control = self.operation.activation_work_control(dimension);
        let mut control = preserve_first_fault(self.first_fault, control)?;
        let permit = try_begin_canonical_work(&mut control, amount);
        let permit = match permit {
            Ok(permit) => permit,
            Err(cause) => {
                return preserve_first_fault(self.first_fault, Err(cause.into_runtime()));
            }
        };
        let mut work = SemanticWorkBatch::new(permit);
        let mut accepted = true;
        for index in 0..amount.get() {
            if work.work_batch.poll_due()
                && let Err(cause) = work.poll_now()
            {
                return preserve_first_fault(self.first_fault, Err(cause));
            }
            accepted &= inspect(index);
            if let Err(cause) = work.complete_unit() {
                return preserve_first_fault(self.first_fault, Err(cause));
            }
        }
        if let Err(cause) = work.finish_exact() {
            return preserve_first_fault(self.first_fault, Err(cause));
        }
        *self.semantic_work_complete = true;
        Ok(accepted)
    }

    /// Accepts the session-owned activation shape after complete authority-free core preflight.
    pub fn seal<E>(self) -> CanonicalPreflightDecision<'activation, E> {
        if !*self.semantic_work_complete {
            let _ = preserve_first_fault::<()>(
                self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
        }
        if self.first_fault.is_none() {
            let (phase, dimensions) = self
                .request
                .canonical_plan
                .keys_for_publication_validation();
            let result = self.operation.validate_writer_keys(phase, dimensions);
            let _ = preserve_first_fault(self.first_fault, result);
        }
        if self.first_fault.is_none() {
            let (phase, dimensions) = self
                .request
                .outcome_evidence_plan
                .keys_for_publication_validation();
            let result = self.operation.validate_writer_keys(phase, dimensions);
            let _ = preserve_first_fault(self.first_fault, result);
        }
        if self.first_fault.is_none() {
            let result = self.operation.validate_activation_dimensions([
                self.request.materialization_work_dimension,
                self.request.validation_work_dimension,
                self.request.canonical_destination_dimension,
                self.request.outcome_evidence_destination_dimension,
            ]);
            let _ = preserve_first_fault(self.first_fault, result);
        }
        CanonicalPreflightDecision {
            state: CanonicalPreflightState::Activate,
            scope: PhantomData,
        }
    }

    /// Records one explicit, verified pre-activation rejection and its terminal classification.
    pub fn reject<E>(
        self,
        cause: E,
        stable_code: u32,
    ) -> CanonicalPreflightDecision<'activation, E> {
        CanonicalPreflightDecision {
            state: CanonicalPreflightState::Rejected(VerifiedRejection {
                cause,
                failure_code: PreparationFailureCode::domain(stable_code),
            }),
            scope: PhantomData,
        }
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

impl<'borrow, 'scope, O, PP, U> CanonicalSemanticWorkSession<'borrow, 'scope, O, PP, U>
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

    fn flush(&mut self) -> Result<(), StoreRuntimeError> {
        let Some(completed) = self.work_batch.take_completed() else {
            return Ok(());
        };
        let Some(permit) = self.permit.as_mut() else {
            self.note_contract_if_untrusted();
            return Err(StoreRuntimeError::ConstructionInvariantUnverified);
        };
        let result = match permit.complete_units(completed) {
            Ok(()) => Ok(()),
            Err(RetainedWorkConsumeError::Ledger(cause)) => Err(StoreRuntimeError::Ledger(cause)),
            Err(RetainedWorkConsumeError::ExceedsAttempted) => {
                Err(StoreRuntimeError::ConstructionInvariantUnverified)
            }
        };
        preserve_first_fault(self.first_fault, result)
    }

    fn poll_now(&mut self) -> Result<(), StoreRuntimeError> {
        self.flush()?;
        if let Some(permit) = self.permit.as_mut() {
            let result = permit.poll().map_err(StoreRuntimeError::Control);
            preserve_first_fault(self.first_fault, result)?;
        }
        self.work_batch.note_poll();
        Ok(())
    }

    /// Runs one semantic action after bounded control polling and records it only on normal return.
    pub fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        if self.work_batch.poll_due() {
            self.poll_now()?;
        }
        if self
            .permit
            .as_ref()
            .map_or(0, |permit| permit.remaining_units())
            <= self.work_batch.unflushed_completed()
        {
            self.note_contract_if_untrusted();
            return Err(StoreRuntimeError::ConstructionInvariantUnverified);
        }
        let result = action();
        if self.work_batch.note_completed_unit().is_err() {
            self.note_contract_if_untrusted();
            return Err(StoreRuntimeError::ConstructionInvariantUnverified);
        }
        Ok(result)
    }

    /// Reserves metered construction scratch in the policy-owned authoring role.
    pub fn reserve(
        &mut self,
        bytes: u64,
    ) -> Result<CanonicalReservation<'scope, O>, StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        let result = self
            .permit
            .as_mut()
            .ok_or(StoreRuntimeError::ConstructionInvariantUnverified)
            .and_then(|permit| {
                permit
                    .reserve(self.phase, bytes)
                    .map_err(StoreRuntimeError::Ledger)
            });
        preserve_first_fault(self.first_fault, result).map(|raw| CanonicalReservation {
            raw: RetainedStoreReservation { raw },
            scope: PhantomData,
        })
    }

    /// Resizes one scoped reservation while retaining it unchanged on rejection.
    pub fn resize(
        &mut self,
        reservation: &mut CanonicalReservation<'scope, O>,
        bytes: u64,
    ) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        let result = self
            .permit
            .as_mut()
            .ok_or(StoreRuntimeError::ConstructionInvariantUnverified)
            .and_then(|permit| {
                permit
                    .resize(&mut reservation.raw.raw, bytes)
                    .map_err(StoreRuntimeError::Ledger)
            });
        preserve_first_fault(self.first_fault, result)
    }

    /// Proves that one scoped reservation belongs to this operation.
    pub fn validate_live_owner(
        &mut self,
        reservation: &CanonicalReservation<'scope, O>,
    ) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        let result = self
            .permit
            .as_mut()
            .ok_or(StoreRuntimeError::ConstructionInvariantUnverified)
            .and_then(|permit| {
                permit
                    .validate_live_owner(&reservation.raw.raw)
                    .map_err(StoreRuntimeError::Ledger)
            });
        preserve_first_fault(self.first_fault, result)
    }

    /// Completes the exact admitted envelope; missing work is a contract only without an earlier
    /// trusted runtime fault.
    pub fn finish_exact(
        mut self,
    ) -> Result<CanonicalSemanticCompletion<'scope>, StoreRuntimeError> {
        let receipt_brand = self.receipt_brand;
        let completed_units = self.admitted_units;
        let result = self.poll_now();
        if let Err(cause) = result {
            if let Some(permit) = self.permit.take() {
                permit.finish_partial();
            }
            *self.state = ScopedSemanticState::Rejected;
            self.resolved = true;
            return Err(cause);
        }
        let complete = match self.permit.take() {
            Some(permit) if permit.remaining_units() == 0 => permit.complete().is_ok(),
            Some(permit) => {
                permit.finish_partial();
                false
            }
            None => false,
        };
        self.resolved = true;
        if complete {
            *self.state = ScopedSemanticState::Complete;
            Ok(CanonicalSemanticCompletion {
                receipt_brand,
                completed_units,
                invariant: PhantomData,
            })
        } else {
            *self.state = ScopedSemanticState::Rejected;
            self.note_contract_if_untrusted();
            Err(preserved_fault(self.first_fault)
                .unwrap_or(StoreRuntimeError::ConstructionInvariantUnverified))
        }
    }

    /// Resolves a verified semantic rejection without turning the incomplete suffix into a
    /// contract fault.
    pub fn finish_rejected(mut self) -> Result<(), StoreRuntimeError> {
        let result = self.poll_now();
        if let Some(permit) = self.permit.take() {
            permit.finish_partial();
        }
        *self.state = ScopedSemanticState::Rejected;
        self.resolved = true;
        result
    }
}

impl<O, PP, U> Drop for CanonicalSemanticWorkSession<'_, '_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if self.resolved {
            return;
        }
        let _ = self.flush();
        if let Some(permit) = self.permit.take() {
            permit.finish_partial();
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
        phase: StorePhaseKey,
        dimension: StoreDimensionKey,
        bytes: u64,
    ) -> Result<CanonicalReservation<'activation, O>, StoreRuntimeError> {
        self.inner.reserve(phase, dimension, bytes)
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
