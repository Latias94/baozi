use super::*;

impl CanonicalFailureTerminalWorkPlan {
    /// Derives the complete protected requirement inside the caller's attempted/consumed totals.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        dimension: StoreDimensionKey,
        max_evidence_members: usize,
        construction_work_limit: u64,
        validation_work_limit: u64,
        total_attempted_work_limit: u64,
        total_consumed_work_limit: u64,
    ) -> Result<Self, StorePlanError> {
        let terminal_work = RetainedStoreFailureTerminalWorkPlan::with_evidence_work(
            dimension,
            max_evidence_members,
            construction_work_limit,
            validation_work_limit,
            total_attempted_work_limit,
            total_consumed_work_limit,
        )?;
        Ok(Self { terminal_work })
    }

    pub const fn requirement(&self) -> &RetainedStoreFailureTerminalWorkRequirement {
        self.terminal_work.requirement()
    }

    pub const fn ordinary_attempted_work_limit(&self) -> u64 {
        self.terminal_work.ordinary_attempted_work_limit()
    }

    pub const fn ordinary_consumed_work_limit(&self) -> u64 {
        self.terminal_work.ordinary_consumed_work_limit()
    }

    pub fn into_store_plan(self) -> RetainedStoreFailureTerminalWorkPlan {
        self.terminal_work
    }
}

impl<O, PP, U> FailureEvidenceBuildControl<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn try_start<'control>(
        raw: RawFailureEvidenceWorkControl<'control, O, PP, U>,
        construction_work: u64,
    ) -> Result<FailureEvidenceBuildControl<'control, O, PP, U>, StoreRuntimeError> {
        let amount =
            NonZeroU64::new(construction_work).ok_or_else(failure_work_envelope_exhausted)?;
        let permit = raw.try_begin(amount).map_err(|cause| match cause {
            RetainedFailureWorkStartError::Control(cause) => StoreRuntimeError::Control(cause),
            RetainedFailureWorkStartError::Ledger(cause) => StoreRuntimeError::Ledger(cause),
            RetainedFailureWorkStartError::Outstanding => failure_work_envelope_exhausted(),
        })?;
        Ok(FailureEvidenceBuildControl {
            permit,
            sticky: None,
        })
    }

    /// Polls cancellation and deadline state without exposing the protected dimension.
    pub fn poll(&mut self) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = self.sticky.clone() {
            return Err(cause);
        }
        let result = self.permit.poll().map_err(StoreRuntimeError::Control);
        if let Err(cause) = &result {
            self.sticky.get_or_insert_with(|| cause.clone());
        }
        result
    }

    fn finish<T>(self, result: Result<T, StoreBuildError>) -> Result<T, FailureBuildCallbackError> {
        if let Some(cause) = self.sticky {
            return Err(FailureBuildCallbackError::Runtime(cause));
        }
        match result {
            Ok(value) => self
                .permit
                .complete()
                .map(|_control| value)
                .map_err(|cause| {
                    FailureBuildCallbackError::Runtime(StoreRuntimeError::Ledger(cause))
                }),
            Err(cause) => Err(FailureBuildCallbackError::Build(cause)),
        }
    }
}

impl<O, PP, U> FailureEvidenceValidationControl<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    /// Polls cancellation and charges one bounded validation chunk before work begins.
    pub fn try_begin_chunk(&mut self, amount: u64) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = self.sticky.clone() {
            return Err(cause);
        }
        if let Err(cause) = self.raw.poll().map_err(StoreRuntimeError::Control) {
            self.sticky = Some(cause.clone());
            return Err(cause);
        }
        if self.budget.reserve_validation(amount).is_err() {
            self.raw
                .record_product_graph_failure(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
            let cause = failure_work_envelope_exhausted();
            self.sticky = Some(cause.clone());
            return Err(cause);
        }
        let result = self
            .raw
            .charge_observed(amount)
            .map_err(StoreRuntimeError::Ledger);
        if result.is_ok() {
            self.began_chunk = true;
        }
        if let Err(cause) = &result {
            self.sticky.get_or_insert_with(|| cause.clone());
        }
        result
    }

    fn finish(mut self, accepted: bool) -> Result<bool, StoreRuntimeError> {
        match self.sticky {
            Some(cause) => Err(cause),
            None if self.began_chunk => Ok(accepted),
            None => {
                self.raw
                    .record_product_graph_failure(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
                Err(failure_work_envelope_exhausted())
            }
        }
    }
}

impl<O, PP, U, E> RejectedCanonicalStoreSession<O, PP, U, E>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreCanonicalRejection<E> {
        &self.owner.cause
    }

    pub const fn canonical_len(&self) -> usize {
        0
    }

    pub fn outcome_evidence_len(&self) -> usize {
        self.owner.raw.outcome_evidence_len()
    }

    pub fn build_failure_evidence<EvidenceKind, EvidenceTag>(
        self,
        build: impl for<'lane> FnMut(
            &mut FailureEvidenceLane<'lane, O, PP, U>,
        ) -> FailureEvidenceDecision<EvidenceKind, EvidenceTag>,
    ) -> Result<
        CanonicalRejectionEvidenceOutcome<O, PP, U, E, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.owner.build(build)
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<StoreCanonicalRejection<E>>, StorePrepareError> {
        self.owner.prepare_minimal_fallback()
    }
}

impl<O, PP, U> FailureEvidenceLane<'_, O, PP, U>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub fn try_retain_sequence<T, Tag>(
        &mut self,
        plan: SequencePlan<T>,
        construction_work: u64,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut SequenceWriter<T>,
            &mut FailureEvidenceBuildControl<'control, O, PP, U>,
        ) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceSequenceRef<T, Tag>, StoreRuntimeError>
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
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        if let Err(cause) = self.failure_work.reserve_construction(construction_work) {
            self.raw
                .reject_failure_terminal_work_envelope(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let mut controlled_failure = None;
        let result = self
            .raw
            .try_retain_outcome_evidence_controlled(allocation, |emplacer, raw_control| {
                let mut control =
                    match FailureEvidenceBuildControl::try_start(raw_control, construction_work) {
                        Ok(control) => control,
                        Err(cause) => {
                            controlled_failure = Some(cause);
                            return Err(StoreBuildError::domain(
                                FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE,
                            )
                            .expect("the failure-work envelope code is inside the domain range")
                            .into());
                        }
                    };
                let writer =
                    <T as super::super::sealed::StoredScalar>::new_writer(plan.capacity, plan.len)?;
                let block = finish_scoped_writer(
                    writer,
                    |writer| {
                        build.as_mut().expect(
                            "the failure-evidence sequence builder is executed at most once",
                        )(writer, &mut control)
                    },
                    SequenceWriter::finish,
                );
                let block = control.finish(block);
                let block = match block {
                    Ok(block) => block,
                    Err(FailureBuildCallbackError::Build(cause)) => return Err(cause.into()),
                    Err(FailureBuildCallbackError::Runtime(cause)) => {
                        controlled_failure = Some(cause);
                        return Err(
                            StoreBuildError::domain(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE)
                                .expect("the failure-work envelope code is inside the domain range")
                                .into(),
                        );
                    }
                };
                Ok(emplacer.place(block, tag))
            })
            .map(|raw| OutcomeEvidenceStoreRef {
                raw,
                marker: std::marker::PhantomData,
            })
            .map_err(|cause| map_publication_error(&cause));
        let result = controlled_failure.map_or(result, Err);
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn try_retain_bytes<Tag>(
        &mut self,
        plan: BytesPlan,
        construction_work: u64,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut BytesWriter,
            &mut FailureEvidenceBuildControl<'control, O, PP, U>,
        ) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceBytesRef<Tag>, StoreRuntimeError>
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
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        if let Err(cause) = self.failure_work.reserve_construction(construction_work) {
            self.raw
                .reject_failure_terminal_work_envelope(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let mut controlled_failure = None;
        let result = self
            .raw
            .try_retain_outcome_evidence_controlled(allocation, |emplacer, raw_control| {
                let mut control =
                    match FailureEvidenceBuildControl::try_start(raw_control, construction_work) {
                        Ok(control) => control,
                        Err(cause) => {
                            controlled_failure = Some(cause);
                            return Err(StoreBuildError::domain(
                                FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE,
                            )
                            .expect("the failure-work envelope code is inside the domain range")
                            .into());
                        }
                    };
                let writer = BytesWriter::new(plan)?;
                let block =
                    finish_scoped_writer(
                        writer,
                        |writer| {
                            build.as_mut().expect(
                                "the failure-evidence bytes builder is executed at most once",
                            )(writer, &mut control)
                        },
                        BytesWriter::finish,
                    );
                let block = control.finish(block);
                let block = match block {
                    Ok(block) => block,
                    Err(FailureBuildCallbackError::Build(cause)) => return Err(cause.into()),
                    Err(FailureBuildCallbackError::Runtime(cause)) => {
                        controlled_failure = Some(cause);
                        return Err(
                            StoreBuildError::domain(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE)
                                .expect("the failure-work envelope code is inside the domain range")
                                .into(),
                        );
                    }
                };
                Ok(emplacer.place(block, tag))
            })
            .map(|raw| OutcomeEvidenceStoreRef {
                raw,
                marker: std::marker::PhantomData,
            })
            .map_err(|cause| map_publication_error(&cause));
        let result = controlled_failure.map_or(result, Err);
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn try_retain_utf8<Tag>(
        &mut self,
        plan: Utf8Plan,
        construction_work: u64,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut Utf8Writer,
            &mut FailureEvidenceBuildControl<'control, O, PP, U>,
        ) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceUtf8Ref<Tag>, StoreRuntimeError>
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
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        if let Err(cause) = self.failure_work.reserve_construction(construction_work) {
            self.raw
                .reject_failure_terminal_work_envelope(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let mut controlled_failure = None;
        let result = self
            .raw
            .try_retain_outcome_evidence_controlled(allocation, |emplacer, raw_control| {
                let mut control =
                    match FailureEvidenceBuildControl::try_start(raw_control, construction_work) {
                        Ok(control) => control,
                        Err(cause) => {
                            controlled_failure = Some(cause);
                            return Err(StoreBuildError::domain(
                                FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE,
                            )
                            .expect("the failure-work envelope code is inside the domain range")
                            .into());
                        }
                    };
                let writer = Utf8Writer::new(plan)?;
                let block =
                    finish_scoped_writer(
                        writer,
                        |writer| {
                            build.as_mut().expect(
                                "the failure-evidence UTF-8 builder is executed at most once",
                            )(writer, &mut control)
                        },
                        Utf8Writer::finish,
                    );
                let block = control.finish(block);
                let block = match block {
                    Ok(block) => block,
                    Err(FailureBuildCallbackError::Build(cause)) => return Err(cause.into()),
                    Err(FailureBuildCallbackError::Runtime(cause)) => {
                        controlled_failure = Some(cause);
                        return Err(
                            StoreBuildError::domain(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE)
                                .expect("the failure-work envelope code is inside the domain range")
                                .into(),
                        );
                    }
                };
                Ok(emplacer.place(block, tag))
            })
            .map(|raw| OutcomeEvidenceStoreRef {
                raw,
                marker: std::marker::PhantomData,
            })
            .map_err(|cause| map_publication_error(&cause));
        let result = controlled_failure.map_or(result, Err);
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn try_retain_records<Tag>(
        &mut self,
        plan: RecordPlan,
        construction_work: u64,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceRecordWriter,
            &mut FailureEvidenceBuildControl<'control, O, PP, U>,
        ) -> Result<(), StoreBuildError>,
    ) -> Result<OutcomeEvidenceRecordRef<Tag>, StoreRuntimeError>
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
        let allocation = match plan.allocation_plan() {
            Ok(allocation) => allocation,
            Err(cause) => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(build, Err(StoreRuntimeError::Plan(cause))),
                );
            }
        };
        if let Err(cause) = self.failure_work.reserve_construction(construction_work) {
            self.raw
                .reject_failure_terminal_work_envelope(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
            return preserve_first_fault(
                self.first_fault,
                finish_scoped_callback(build, Err(cause)),
            );
        }
        let mut controlled_failure = None;
        let result = self
            .raw
            .try_retain_outcome_evidence_controlled(allocation, |emplacer, raw_control| {
                let mut control =
                    match FailureEvidenceBuildControl::try_start(raw_control, construction_work) {
                        Ok(control) => control,
                        Err(cause) => {
                            controlled_failure = Some(cause);
                            return Err(StoreBuildError::domain(
                                FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE,
                            )
                            .expect("the failure-work envelope code is inside the domain range")
                            .into());
                        }
                    };
                let writer = OutcomeEvidenceRecordWriter::new(plan)?;
                let block =
                    finish_scoped_writer(
                        writer,
                        |writer| {
                            build.as_mut().expect(
                                "the failure-evidence record builder is executed at most once",
                            )(writer, &mut control)
                        },
                        OutcomeEvidenceRecordWriter::finish,
                    );
                let block = control.finish(block);
                let block = match block {
                    Ok(block) => block,
                    Err(FailureBuildCallbackError::Build(cause)) => return Err(cause.into()),
                    Err(FailureBuildCallbackError::Runtime(cause)) => {
                        controlled_failure = Some(cause);
                        return Err(
                            StoreBuildError::domain(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE)
                                .expect("the failure-work envelope code is inside the domain range")
                                .into(),
                        );
                    }
                };
                Ok(emplacer.place(block, tag))
            })
            .map(|raw| OutcomeEvidenceStoreRef {
                raw,
                marker: std::marker::PhantomData,
            })
            .map_err(|cause| map_publication_error(&cause));
        let result = controlled_failure.map_or(result, Err);
        preserve_first_fault(self.first_fault, finish_scoped_callback(build, result))
    }

    pub fn commit(
        &mut self,
        dimension: StoreDimensionKey,
        amount: u64,
    ) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            note_later_contract(
                self.first_fault,
                validate_dimension_contract(self.domain, dimension),
            );
            return Err(cause);
        }
        let result = if !self.domain.owns_dimension(dimension) {
            Err(StoreRuntimeError::ForeignDimensionDomain { dimension })
        } else {
            self.raw
                .try_commit_outcome_evidence(dimension, amount)
                .map_err(|cause| map_publication_error(&cause))
        };
        preserve_first_fault(self.first_fault, result)
    }

    pub const fn complete<Kind, Tag>(
        &mut self,
        root: OutcomeEvidenceStoreRef<Kind, Tag>,
    ) -> FailureEvidenceDecision<Kind, Tag> {
        FailureEvidenceDecision::Complete(root)
    }

    pub const fn abort<Kind, Tag>(&mut self) -> FailureEvidenceDecision<Kind, Tag> {
        FailureEvidenceDecision::Abort
    }
}

impl<O, PP, U, Cause, EvidenceKind, EvidenceTag>
    RootedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &Cause {
        &self.cause
    }

    pub fn outcome_evidence_len(&self) -> usize {
        self.failure.outcome_evidence_len()
    }

    pub fn validate_rich_failure(
        self,
        validate: impl for<'session, 'storage, 'control> FnMut(
            &mut StoreEvidenceValidationSession<'session, 'storage>,
            OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
            &mut FailureEvidenceValidationControl<'control, O, PP, U>,
        ) -> bool,
    ) -> Result<
        RootedFailureValidation<O, PP, U, Cause, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let mut validate = ManuallyDrop::new(Some(validate));
        let Self {
            failure,
            cause,
            mut failure_work,
        } = self;
        let cause = ManuallyDrop::new(cause);
        let evidence_root = failure.evidence_root();
        let mut controlled_failure = None;
        let validation = failure.validate_rich_failure_controlled(
            evidence_root,
            |session, root, raw_control| {
                let mut control = FailureEvidenceValidationControl {
                    raw: raw_control,
                    budget: &mut failure_work,
                    sticky: None,
                    began_chunk: false,
                };
                let validate = validate
                    .as_mut()
                    .expect("the rich-failure validation callback is executed at most once");
                let accepted = validate(session, root, &mut control);
                match control.finish(accepted) {
                    Ok(accepted) => accepted,
                    Err(cause) => {
                        controlled_failure = Some(cause);
                        false
                    }
                }
            },
        );
        if drop_optional_callback(validate).is_err() {
            let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
            let cause = match validation {
                StoreFailureValidation::Valid(validated) => validated.contain_activation_fault(),
                StoreFailureValidation::Rejected(failure) => failure.contain_activation_fault(),
                StoreFailureValidation::FallbackOnly(fallback) => {
                    fallback.contain_activation_fault()
                }
            };
            return Err(StoreActivationContainment { cause });
        }
        if let Some(runtime_cause) = controlled_failure {
            if drop_scoped_value(ManuallyDrop::into_inner(cause)).is_err() {
                let cause = match validation {
                    StoreFailureValidation::Valid(validated) => {
                        validated.contain_activation_fault()
                    }
                    StoreFailureValidation::Rejected(failure) => failure.contain_activation_fault(),
                    StoreFailureValidation::FallbackOnly(fallback) => {
                        fallback.contain_activation_fault()
                    }
                };
                return Err(StoreActivationContainment { cause });
            }
            let prepared = match validation {
                StoreFailureValidation::Valid(validated) => validated.prepare_minimal_fallback(),
                StoreFailureValidation::Rejected(failure) => failure.prepare_minimal_fallback(),
                StoreFailureValidation::FallbackOnly(fallback) => {
                    fallback.prepare_minimal_fallback()
                }
            };
            return match prepared {
                Ok(prepared) => Ok(RootedFailureValidation::Fallback(VerifiedMinimalFallback {
                    prepared,
                    cause: runtime_cause,
                })),
                Err(StorePrepareError::FinalizationContainment(cause)) => {
                    Err(StoreActivationContainment { cause })
                }
            };
        }
        match validation {
            StoreFailureValidation::Valid(validated) => {
                Ok(RootedFailureValidation::Valid(ValidatedFailureEvidence {
                    validated,
                    cause: ManuallyDrop::into_inner(cause),
                }))
            }
            StoreFailureValidation::Rejected(failure) => {
                Ok(RootedFailureValidation::Rejected(RejectedFailureEvidence {
                    failure,
                    cause: ManuallyDrop::into_inner(cause),
                }))
            }
            StoreFailureValidation::FallbackOnly(fallback) => {
                let containment_required = fallback.activation_containment_required();
                if drop_scoped_value(ManuallyDrop::into_inner(cause)).is_err() {
                    return Err(StoreActivationContainment {
                        cause: fallback.contain_activation_fault(),
                    });
                }
                if containment_required {
                    return Err(StoreActivationContainment {
                        cause: fallback.contain_activation_fault(),
                    });
                }
                prepare_product_graph_fallback(fallback).map(RootedFailureValidation::Fallback)
            }
        }
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<Cause>, StorePrepareError> {
        let Self {
            failure,
            cause,
            failure_work: _,
        } = self;
        let cause = ManuallyDrop::new(cause);
        let prepared = failure.prepare_minimal_fallback();
        match prepared {
            Ok(prepared) => Ok(VerifiedMinimalFallback {
                prepared,
                cause: ManuallyDrop::into_inner(cause),
            }),
            Err(error) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                Err(error)
            }
        }
    }
}

impl<O, PP, U, Cause, EvidenceKind, EvidenceTag>
    ValidatedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &Cause {
        &self.cause
    }

    pub fn try_prepare(
        self,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<PreparedFailureEvidence<Cause, EvidenceKind, EvidenceTag>, StorePrepareError> {
        let Self { validated, cause } = self;
        let cause = ManuallyDrop::new(cause);
        let prepared = validated.try_prepare(0, final_progress);
        match prepared {
            Ok(prepared) => Ok(PreparedFailureEvidence {
                prepared,
                cause: ManuallyDrop::into_inner(cause),
            }),
            Err(error) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                Err(error)
            }
        }
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<Cause>, StorePrepareError> {
        let Self { validated, cause } = self;
        let cause = ManuallyDrop::new(cause);
        let prepared = validated.prepare_minimal_fallback();
        match prepared {
            Ok(prepared) => Ok(VerifiedMinimalFallback {
                prepared,
                cause: ManuallyDrop::into_inner(cause),
            }),
            Err(error) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                Err(error)
            }
        }
    }
}

impl<Cause, EvidenceKind, EvidenceTag> PreparedFailureEvidence<Cause, EvidenceKind, EvidenceTag> {
    pub const fn cause(&self) -> &Cause {
        &self.cause
    }

    pub fn into_parts(
        self,
    ) -> (
        PreparedStoreFailureOutcome<EvidenceKind, EvidenceTag>,
        Cause,
    ) {
        (self.prepared, self.cause)
    }
}

impl<O, PP, U, Cause, EvidenceKind, EvidenceTag>
    RejectedFailureEvidence<O, PP, U, Cause, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &Cause {
        &self.cause
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<Cause>, StorePrepareError> {
        let Self { failure, cause } = self;
        let cause = ManuallyDrop::new(cause);
        match failure.prepare_minimal_fallback() {
            Ok(prepared) => Ok(VerifiedMinimalFallback {
                prepared,
                cause: ManuallyDrop::into_inner(cause),
            }),
            Err(error) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                Err(error)
            }
        }
    }
}

impl<Cause> VerifiedMinimalFallback<Cause> {
    pub const fn cause(&self) -> &Cause {
        &self.cause
    }

    pub fn into_parts(self) -> (PreparedStoreFallback, Cause) {
        (self.prepared, self.cause)
    }
}

impl<O, PP, U, Cause> UnrootedFailureEvidence<O, PP, U, Cause>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn prepare_minimal_fallback(self) -> Result<VerifiedMinimalFallback<Cause>, StorePrepareError> {
        let Self {
            raw,
            domain,
            failure_code: _,
            cause,
            failure_work: _,
        } = self;
        let cause = ManuallyDrop::new(cause);
        match raw
            .prepare_minimal_fallback()
            .map(|prepared| classify_prepared_fallback(prepared, domain))
            .map_err(StorePrepareError::from_raw)
        {
            Ok(prepared) => Ok(VerifiedMinimalFallback {
                prepared,
                cause: ManuallyDrop::into_inner(cause),
            }),
            Err(error) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                Err(error)
            }
        }
    }

    #[allow(clippy::type_complexity, clippy::result_large_err)]
    pub fn build<EvidenceKind, EvidenceTag>(
        self,
        build: impl for<'lane> FnMut(
            &mut FailureEvidenceLane<'lane, O, PP, U>,
        ) -> FailureEvidenceDecision<EvidenceKind, EvidenceTag>,
    ) -> Result<
        FailureEvidenceOutcome<O, PP, U, Cause, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let Self {
            raw,
            domain,
            failure_code,
            cause,
            mut failure_work,
        } = self;
        let cause = ManuallyDrop::new(cause);
        let mut raw = raw;
        let mut build = ManuallyDrop::new(Some(build));
        let mut first_fault = None;
        let decision = catch_unwind(AssertUnwindSafe(|| {
            let mut lane = FailureEvidenceLane {
                raw: &mut raw,
                domain: &domain,
                first_fault: &mut first_fault,
                failure_work: &mut failure_work,
            };
            let build = build
                .as_mut()
                .expect("the failure-evidence callback is executed at most once");
            build(&mut lane)
        }));
        let build_drop_failed = drop_optional_callback(build).is_err();
        let decision = match decision {
            Ok(decision) if !build_drop_failed => decision,
            Ok(decision) => {
                let _ = drop_scoped_value(decision);
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                return Err(StoreActivationContainment {
                    cause: raw.contain_activation_fault(),
                });
            }
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                return Err(StoreActivationContainment {
                    cause: raw.contain_activation_fault(),
                });
            }
        };
        if let Some(fault) = first_fault {
            match decision {
                FailureEvidenceDecision::Abort if !fault.contract_seen => {
                    let Some(runtime_failure_code) = fault.recoverable_code() else {
                        let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                        return Err(StoreActivationContainment {
                            cause: raw.contain_activation_fault(),
                        });
                    };
                    let prepared = raw
                        .with_fallback_cause(runtime_failure_code)
                        .prepare_minimal_fallback()
                        .map(|prepared| classify_prepared_fallback(prepared, domain));
                    return match prepared {
                        Ok(prepared) => Ok(FailureEvidenceOutcome::MinimalFallback(
                            VerifiedMinimalFallback {
                                prepared,
                                cause: ManuallyDrop::into_inner(cause),
                            },
                        )),
                        Err(error) => {
                            let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                            Err(StoreActivationContainment {
                                cause: map_raw_prepare_error(error),
                            })
                        }
                    };
                }
                FailureEvidenceDecision::Complete(_) | FailureEvidenceDecision::Abort => {
                    let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                    return Err(StoreActivationContainment {
                        cause: raw.contain_activation_fault(),
                    });
                }
            }
        }
        let evidence_root = match decision {
            FailureEvidenceDecision::Complete(root) => root,
            FailureEvidenceDecision::Abort => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                return Err(StoreActivationContainment {
                    cause: raw.contain_activation_fault(),
                });
            }
        };
        Ok(FailureEvidenceOutcome::Rooted(RootedFailureEvidence {
            failure: StoreFailureAssembly {
                raw,
                fallback_only: false,
                forced_cause: Some(failure_code),
                evidence_root,
                domain,
            },
            cause: ManuallyDrop::into_inner(cause),
            failure_work,
        }))
    }
}
