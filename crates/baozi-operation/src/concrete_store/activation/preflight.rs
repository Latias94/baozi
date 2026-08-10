use super::*;

fn canonical_build_work(units: Option<usize>) -> Result<NonZeroU64, StoreRuntimeError> {
    let units = units.ok_or(StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    let units =
        u64::try_from(units).map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
    NonZeroU64::new(units).ok_or(StoreRuntimeError::Plan(StorePlanError::EmptyPayload))
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
                    Err(ActivationOpenFailure::Contained(cause)) => {
                        let _ = drop_optional_callback(materialize);
                        return Err(StoreActivationContainment { cause });
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
    /// consumed only after its inspector returns normally, and control is polled before every unit.
    /// Returning `false` rejects the semantics but does not short-circuit the remaining evidence.
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
        let mut permit = match permit {
            Ok(permit) => permit,
            Err(cause) => {
                return preserve_first_fault(self.first_fault, Err(cause.into_runtime()));
            }
        };
        let mut accepted = true;
        for index in 0..amount.get() {
            if let Err(cause) = permit.poll() {
                return preserve_first_fault(self.first_fault, Err(cause.into_runtime()));
            }
            accepted &= inspect(index);
            if let Err(cause) = permit.complete_units(NonZeroU64::MIN) {
                return preserve_first_fault(self.first_fault, Err(cause.into_runtime()));
            }
        }
        if !permit.finish_exact() {
            return preserve_first_fault(
                self.first_fault,
                Err(StoreRuntimeError::ConstructionInvariantUnverified),
            );
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
