use super::*;

impl<O, PP, U> RetainedStoreOperation<O, PP, U, FreshStoreLineage>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Moves the operation into the session-owned dormant activation seam.
    pub fn into_session_outcome_owner(
        self,
        outcome_evidence_plan: RetainedStoreBlockTablePlan,
    ) -> RetainedStoreSessionOutcomeOwner<O, PP, U, FreshStoreLineage> {
        let (max_evidence_members, construction_work_limit, validation_work_limit) = self
            .raw
            .failure_terminal_work_limits()
            .expect("every concrete retained-store operation owns terminal failure work");
        let failure_work = FailureEvidenceWorkEnvelope {
            max_evidence_members,
            construction_work_limit,
            validation_work_limit,
        };
        RetainedStoreSessionOutcomeOwner {
            operation: Some(self),
            outcome_evidence_plan,
            failure_work: FailureEvidenceWorkBudget::new(failure_work),
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
    pub fn plan_and_activate<T, E>(
        mut self,
        preflight: impl for<'scope> FnMut(
            CanonicalPreflight<'scope, O, PP, U, L>,
        ) -> CanonicalPreflightDecision<E>,
        materialize: impl for<'lane> FnMut(
            &mut CanonicalLane<'lane, O, PP, U, L>,
        ) -> CanonicalMaterialization<T, E>,
    ) -> Result<CanonicalActivationOutcome<O, PP, U, T, E, L>, StoreActivationContainment> {
        let mut preflight = ManuallyDrop::new(Some(preflight));
        let mut materialize = ManuallyDrop::new(Some(materialize));
        let mut operation = self
            .operation
            .take()
            .expect("the session outcome owner begins activation at most once");
        let outcome_evidence_plan = self.outcome_evidence_plan;
        let failure_work = self.failure_work;
        let mut first_fault = None;
        let preflight_result = catch_unwind(AssertUnwindSafe(|| {
            let preflight = preflight
                .as_mut()
                .expect("the preflight callback is executed at most once");
            preflight(CanonicalPreflight {
                operation: &mut operation,
                first_fault: &mut first_fault,
            })
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

        match preflight.state {
            CanonicalPreflightState::Rejected(rejection) => {
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
                        rejection,
                    ),
                ))
            }
            CanonicalPreflightState::Activate {
                canonical_plan,
                intent: activation_intent,
            } => {
                if first_fault.is_some() {
                    let _ = drop_optional_callback(materialize);
                    return Err(contain_operation(operation));
                }
                let publication_plan =
                    RetainedStorePublicationPlan::success(canonical_plan, outcome_evidence_plan);
                let mut assembly = match try_open_activation(operation, publication_plan) {
                    Ok(assembly) => assembly,
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
                                cause,
                                failure_code,
                                fallback_only,
                            ),
                        ));
                    }
                    Err(ActivationOpenFailure::Contained(cause)) => {
                        let _ = drop_optional_callback(materialize);
                        return Err(StoreActivationContainment { cause });
                    }
                };

                let mut first_fault = None;
                let materialized = catch_unwind(AssertUnwindSafe(|| {
                    let mut lane = CanonicalLane {
                        assembly: &mut assembly,
                        first_fault: &mut first_fault,
                    };
                    let materialize = materialize
                        .as_mut()
                        .expect("the materialization callback is executed at most once");
                    materialize(&mut lane)
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
                    let _ = drop_materialization(materialized);
                    return Err(StoreActivationContainment {
                        cause: assembly.contain_activation_fault(),
                    });
                }

                match (first_fault, materialized.state) {
                    (Some(fault), CanonicalMaterializationState::Activated(output)) => {
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
                            StoreCanonicalRejection::Runtime(fault.cause),
                            failure_code,
                        )
                    }
                    (None, CanonicalMaterializationState::Activated(output)) => {
                        if assembly.canonical_len()
                            != activation_intent.expected_materialized_blocks
                        {
                            let _ = drop_scoped_value(output);
                            return Err(StoreActivationContainment {
                                cause: assembly.contain_activation_fault(),
                            });
                        }
                        Ok(CanonicalActivationOutcome::Activated(
                            ActivatedCanonicalStoreSession {
                                assembly,
                                output,
                                failure_work,
                                intent: activation_intent,
                            },
                        ))
                    }
                    (None, CanonicalMaterializationState::Rejected(rejection)) => finish_rejected(
                        assembly,
                        failure_work,
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

    /// Seals the mechanical table envelope after complete authority-free core preflight.
    pub fn seal<E>(
        self,
        activation_plan: CanonicalActivationPlan,
    ) -> CanonicalPreflightDecision<E> {
        let (canonical_plan, intent) = activation_plan.into_parts();
        if self.first_fault.is_none() {
            let (phase, dimensions) = canonical_plan.keys_for_publication_validation();
            let result = self.operation.validate_writer_keys(phase, dimensions);
            let _ = preserve_first_fault(self.first_fault, result);
        }
        if self.first_fault.is_none() {
            let result = self.operation.validate_activation_dimensions([
                intent.work_dimension,
                intent.commitment_dimension,
            ]);
            let _ = preserve_first_fault(self.first_fault, result);
        }
        CanonicalPreflightDecision {
            state: CanonicalPreflightState::Activate {
                canonical_plan,
                intent,
            },
        }
    }

    /// Records one explicit, verified pre-activation rejection and its terminal classification.
    pub fn reject<E>(self, cause: E, stable_code: u32) -> CanonicalPreflightDecision<E> {
        CanonicalPreflightDecision {
            state: CanonicalPreflightState::Rejected(VerifiedRejection {
                cause,
                failure_code: PreparationFailureCode::domain(stable_code),
            }),
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

    pub fn charge_observed(
        &mut self,
        dimension: StoreDimensionKey,
        amount: u64,
    ) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            note_later_contract(self.first_fault, self.raw.validate_dimension(dimension));
            return Err(cause);
        }
        preserve_first_fault(
            self.first_fault,
            self.raw.charge_observed(dimension, amount),
        )
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

    pub fn control_checkpoint<'borrow>(
        &'borrow mut self,
    ) -> Result<CanonicalControlCheckpoint<'borrow, 'activation, O, PP, U>, StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            return Err(cause);
        }
        match self.assembly.try_control_checkpoint() {
            Ok(raw) => Ok(CanonicalControlCheckpoint {
                raw,
                first_fault: self.first_fault,
                activation: PhantomData,
            }),
            Err(cause) => preserve_first_fault(self.first_fault, Err(cause)),
        }
    }

    pub fn try_retain_sequence<T, Tag>(
        &mut self,
        plan: SequencePlan<T>,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut SequenceWriter<T>,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
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
            note_later_contract(
                self.first_fault,
                validate_dimension_contract(&self.assembly.domain, work_dimension),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let result = self.assembly.try_retain_canonical_sequence_controlled(
            plan,
            work_dimension,
            tag,
            |writer, control| {
                build
                    .as_mut()
                    .expect("the canonical sequence builder is executed at most once")(
                    writer, control,
                )
            },
        );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn try_retain_bytes<Tag>(
        &mut self,
        plan: BytesPlan,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut BytesWriter,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
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
            note_later_contract(
                self.first_fault,
                validate_dimension_contract(&self.assembly.domain, work_dimension),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let result = self.assembly.try_retain_canonical_bytes_controlled(
            plan,
            work_dimension,
            tag,
            |writer, control| {
                build
                    .as_mut()
                    .expect("the canonical bytes builder is executed at most once")(
                    writer, control
                )
            },
        );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn try_retain_utf8<Tag>(
        &mut self,
        plan: Utf8Plan,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut Utf8Writer,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
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
            note_later_contract(
                self.first_fault,
                validate_dimension_contract(&self.assembly.domain, work_dimension),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let result = self.assembly.try_retain_canonical_utf8_controlled(
            plan,
            work_dimension,
            tag,
            |writer, control| {
                build
                    .as_mut()
                    .expect("the canonical UTF-8 builder is executed at most once")(
                    writer, control
                )
            },
        );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn try_retain_records<Tag>(
        &mut self,
        plan: RecordPlan,
        work_dimension: StoreDimensionKey,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut CanonicalRecordWriter,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
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
            note_later_contract(
                self.first_fault,
                validate_dimension_contract(&self.assembly.domain, work_dimension),
            );
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let result = self.assembly.try_retain_canonical_records_controlled(
            plan,
            work_dimension,
            tag,
            |writer, control| {
                build
                    .as_mut()
                    .expect("the canonical record builder is executed at most once")(
                    writer, control,
                )
            },
        );
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn accept<T, E>(&mut self, output: T) -> CanonicalMaterialization<T, E> {
        CanonicalMaterialization {
            state: CanonicalMaterializationState::Activated(output),
        }
    }

    pub fn reject<T, E>(&mut self, cause: E, stable_code: u32) -> CanonicalMaterialization<T, E> {
        CanonicalMaterialization {
            state: CanonicalMaterializationState::Rejected(VerifiedRejection {
                cause,
                failure_code: PreparationFailureCode::domain(stable_code),
            }),
        }
    }
}
