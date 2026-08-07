use super::*;

impl<O, PP, U, T, CanonicalKind, CanonicalTag, L>
    ActivatedCanonicalStoreSession<O, PP, U, T, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn output(&self) -> &T {
        &self.output
    }

    pub fn canonical_len(&self) -> usize {
        self.assembly.canonical_len()
    }

    /// Consumes the candidate through the operation-owned evidence and validation seam.
    ///
    /// The caller never receives the writable assembly. The validation-work dimension, canonical
    /// commitment, and exact materialized shape come from the consumed activation plan, so none can
    /// be replaced at this transition. Evidence is built in a one-shot lane, then the existing
    /// controlled validator produces either an opaque candidate or an eager-cleaned failure owner.
    /// Any invalid root, ignored control fault, panic, or uncertain cleanup is quarantined instead
    /// of becoming a fallback result.
    #[allow(clippy::type_complexity, clippy::result_large_err)]
    pub fn validate_centrally<EvidenceKind, EvidenceTag, Proof, E>(
        self,
        build_evidence: impl for<'lane> FnMut(
            &mut CanonicalOutcomeEvidenceLane<'lane, O, PP, U, L>,
            &T,
        )
            -> OutcomeEvidenceDecision<EvidenceKind, EvidenceTag>,
        validate: impl for<'session, 'storage, 'control> FnMut(
            &mut StoreValidationSession<'session, 'storage>,
            CanonicalStoreRef<CanonicalKind, CanonicalTag>,
            OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
            &T,
            &mut RetainedStoreWorkControl<'control, O, PP, U>,
        ) -> Result<
            CanonicalValidationDecision<Proof, E>,
            StoreWorkError,
        >,
    ) -> Result<
        CanonicalCentralValidationOutcome<
            O,
            PP,
            U,
            T,
            Proof,
            E,
            CanonicalKind,
            CanonicalTag,
            EvidenceKind,
            EvidenceTag,
        >,
        StoreActivationContainment,
    >
    where
        CanonicalTag: Copy + Send + Sync + 'static,
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        let mut build_evidence = ManuallyDrop::new(Some(build_evidence));
        let mut validate = ManuallyDrop::new(Some(validate));
        let Self {
            assembly,
            canonical_root,
            output,
            failure_work,
            intent,
        } = self;
        let output = ManuallyDrop::new(output);
        let mut assembly = assembly;
        let mut failure_work = failure_work;
        let work_dimension = intent.validation_work_dimension;
        if let Err(cause) =
            assembly.try_commit_canonical(intent.commitment_dimension, intent.commitment_amount)
        {
            let classification = classify_activation_fault(&cause);
            let build_drop_failed = drop_optional_callback(build_evidence).is_err();
            let validate_drop_failed = drop_optional_callback(validate).is_err();
            if build_drop_failed || validate_drop_failed {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                return Err(StoreActivationContainment {
                    cause: assembly.contain_activation_fault(),
                });
            }
            if drop_scoped_value(ManuallyDrop::into_inner(output)).is_err() {
                let _ = drop_scoped_value(cause);
                return Err(StoreActivationContainment {
                    cause: assembly.contain_activation_fault(),
                });
            }
            return match classification {
                ActivationFaultClassification::Contract => {
                    let _ = drop_scoped_value(cause);
                    Err(StoreActivationContainment {
                        cause: assembly.contain_activation_fault(),
                    })
                }
                ActivationFaultClassification::Recoverable(failure_code) => {
                    prepare_assembly_minimal(assembly, cause, failure_code)
                        .map(CanonicalCentralValidationOutcome::Fallback)
                }
            };
        }
        let mut evidence_fault = None;
        let evidence = catch_unwind(AssertUnwindSafe(|| {
            let mut lane = CanonicalOutcomeEvidenceLane {
                assembly: &mut assembly,
                first_fault: &mut evidence_fault,
                failure_work: &mut failure_work,
                work_dimension,
            };
            let build_evidence = build_evidence
                .as_mut()
                .expect("the outcome-evidence callback is executed at most once");
            build_evidence(&mut lane, &*output)
        }));
        let build_drop_failed = drop_optional_callback(build_evidence).is_err();
        let evidence = match evidence {
            Ok(evidence) if !build_drop_failed => evidence,
            Ok(evidence) => {
                let _ = drop_scoped_value(evidence);
                let _ = drop_optional_callback(validate);
                let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                return Err(StoreActivationContainment {
                    cause: assembly.contain_activation_fault(),
                });
            }
            Err(payload) => {
                discard_untrusted_panic_payload(payload);
                let _ = drop_optional_callback(validate);
                let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                return Err(StoreActivationContainment {
                    cause: assembly.contain_activation_fault(),
                });
            }
        };
        let evidence_root = match (evidence_fault, evidence) {
            (Some(fault), OutcomeEvidenceDecision::Abort) if !fault.contract_seen => {
                let validate_drop_failed = drop_optional_callback(validate).is_err();
                let output_drop_failed =
                    drop_scoped_value(ManuallyDrop::into_inner(output)).is_err();
                if validate_drop_failed || output_drop_failed {
                    return Err(StoreActivationContainment {
                        cause: assembly.contain_activation_fault(),
                    });
                }
                let Some(failure_code) = fault.recoverable_code() else {
                    return Err(StoreActivationContainment {
                        cause: assembly.contain_activation_fault(),
                    });
                };
                return prepare_assembly_minimal(assembly, fault.cause, failure_code)
                    .map(CanonicalCentralValidationOutcome::Fallback);
            }
            (Some(_), _) | (None, OutcomeEvidenceDecision::Abort) => {
                let _ = drop_optional_callback(validate);
                let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                return Err(StoreActivationContainment {
                    cause: assembly.contain_activation_fault(),
                });
            }
            (None, OutcomeEvidenceDecision::Complete(root)) => root,
        };
        if assembly.is_fallback_only() {
            let _ = drop_optional_callback(validate);
            let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
            return Err(StoreActivationContainment {
                cause: assembly.contain_activation_fault(),
            });
        }

        let mut decision = None;
        let validation = assembly.validate_success_controlled(
            work_dimension,
            canonical_root,
            evidence_root,
            |session, canonical, evidence, control| {
                let validate = validate
                    .as_mut()
                    .expect("the central validation callback is executed at most once");
                let result = validate(session, canonical, evidence, &*output, control);
                match result {
                    Ok(CanonicalValidationDecision::Accept(proof)) => {
                        decision = Some(CanonicalValidationDecision::Accept(proof));
                        Ok(true)
                    }
                    Ok(CanonicalValidationDecision::Reject { cause, stable_code }) => {
                        decision = Some(CanonicalValidationDecision::Reject { cause, stable_code });
                        Ok(false)
                    }
                    Err(cause) => Err(cause),
                }
            },
        );

        if drop_optional_callback(validate).is_err() {
            let _ = drop_validation_decision(decision);
            let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
            let cause = match validation {
                StoreControlledSuccessValidation::InvalidWorkDimension { assembly, cause: _ } => {
                    assembly.contain_activation_fault()
                }
                StoreControlledSuccessValidation::FallbackOnly(fallback) => {
                    fallback.contain_activation_fault()
                }
                StoreControlledSuccessValidation::Valid(validated) => {
                    validated.contain_activation_fault()
                }
                StoreControlledSuccessValidation::Rejected(failure)
                | StoreControlledSuccessValidation::Interrupted { failure, cause: _ } => {
                    failure.contain_activation_fault()
                }
            };
            return Err(StoreActivationContainment { cause });
        }

        match validation {
            StoreControlledSuccessValidation::InvalidWorkDimension { assembly, cause: _ } => {
                let _ = drop_validation_decision(decision);
                let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                Err(StoreActivationContainment {
                    cause: assembly.contain_activation_fault(),
                })
            }
            StoreControlledSuccessValidation::FallbackOnly(fallback) => {
                let containment_required = fallback.activation_containment_required();
                let decision_drop_failed = drop_validation_decision(decision).is_err();
                let output_drop_failed =
                    drop_scoped_value(ManuallyDrop::into_inner(output)).is_err();
                if containment_required || decision_drop_failed || output_drop_failed {
                    return Err(StoreActivationContainment {
                        cause: fallback.contain_activation_fault(),
                    });
                }
                prepare_product_graph_fallback(fallback)
                    .map(CanonicalCentralValidationOutcome::Fallback)
            }
            StoreControlledSuccessValidation::Valid(validated) => match decision {
                Some(CanonicalValidationDecision::Accept(proof)) => Ok(
                    CanonicalCentralValidationOutcome::Candidate(CanonicalCandidateStoreSession {
                        _validated: validated,
                        proof,
                        output: ManuallyDrop::into_inner(output),
                        _failure_work: failure_work,
                    }),
                ),
                decision => {
                    let _ = drop_validation_decision(decision);
                    let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                    Err(StoreActivationContainment {
                        cause: validated.contain_activation_fault(),
                    })
                }
            },
            StoreControlledSuccessValidation::Rejected(failure) => match decision {
                Some(CanonicalValidationDecision::Reject { cause, stable_code }) => {
                    let failure =
                        failure.with_forced_cause(PreparationFailureCode::domain(stable_code));
                    match failure.try_discard_canonical_for_activation() {
                        Ok(failure) => {
                            if drop_scoped_value(ManuallyDrop::into_inner(output)).is_err() {
                                let _ = drop_scoped_value(cause);
                                return Err(StoreActivationContainment {
                                    cause: failure.contain_activation_fault(),
                                });
                            }
                            let root = RootedFailureEvidence {
                                failure,
                                cause: StoreCanonicalRejection::Requested(cause),
                                failure_work,
                            };
                            Ok(CanonicalCentralValidationOutcome::Rejected(
                                CentrallyRejectedCanonicalStoreSession { root },
                            ))
                        }
                        Err(containment) => {
                            let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                            let _ = drop_scoped_value(cause);
                            Err(StoreActivationContainment { cause: containment })
                        }
                    }
                }
                decision => {
                    let _ = drop_validation_decision(decision);
                    let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                    Err(StoreActivationContainment {
                        cause: failure.contain_activation_fault(),
                    })
                }
            },
            StoreControlledSuccessValidation::Interrupted { failure, cause } => {
                if drop_validation_decision(decision).is_err() {
                    let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                    return Err(StoreActivationContainment {
                        cause: failure.contain_activation_fault(),
                    });
                }
                match failure.try_discard_canonical_for_activation() {
                    Ok(failure) => {
                        if drop_scoped_value(ManuallyDrop::into_inner(output)).is_err() {
                            return Err(StoreActivationContainment {
                                cause: failure.contain_activation_fault(),
                            });
                        }
                        Ok(CanonicalCentralValidationOutcome::Interrupted(
                            InterruptedCanonicalStoreSession {
                                root: RootedFailureEvidence {
                                    failure,
                                    cause,
                                    failure_work,
                                },
                            },
                        ))
                    }
                    Err(containment) => {
                        let _ = drop_scoped_value(ManuallyDrop::into_inner(output));
                        Err(StoreActivationContainment { cause: containment })
                    }
                }
            }
        }
    }
}

impl<Proof, E> CanonicalValidationDecision<Proof, E> {
    pub const fn accept(proof: Proof) -> Self {
        Self::Accept(proof)
    }

    pub const fn reject(cause: E, stable_code: u32) -> Self {
        Self::Reject { cause, stable_code }
    }
}

impl<O, PP, U, T, Proof, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    CanonicalCandidateStoreSession<
        O,
        PP,
        U,
        T,
        Proof,
        CanonicalKind,
        CanonicalTag,
        EvidenceKind,
        EvidenceTag,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn output(&self) -> &T {
        &self.output
    }

    pub const fn proof(&self) -> &Proof {
        &self.proof
    }
}

impl<O, PP, U, E, EvidenceKind, EvidenceTag>
    CentrallyRejectedCanonicalStoreSession<O, PP, U, E, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreCanonicalRejection<E> {
        self.root.cause()
    }

    pub const fn canonical_len(&self) -> usize {
        0
    }

    pub fn validate_rich_failure(
        self,
        validate: impl for<'session, 'storage, 'control> FnMut(
            &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
            FailureEvidenceMember<'session, EvidenceKind, EvidenceTag>,
        ) -> bool,
    ) -> Result<
        CanonicalRejectionFailureValidation<O, PP, U, E, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.root.validate_rich_failure(validate)
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<StoreCanonicalRejection<E>>, StorePrepareError> {
        self.root.prepare_minimal_fallback()
    }
}

impl<O, PP, U, EvidenceKind, EvidenceTag>
    InterruptedCanonicalStoreSession<O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreWorkError {
        self.root.cause()
    }

    pub fn validate_rich_failure(
        self,
        validate: impl for<'session, 'storage, 'control> FnMut(
            &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
            FailureEvidenceMember<'session, EvidenceKind, EvidenceTag>,
        ) -> bool,
    ) -> Result<
        RootedFailureValidation<O, PP, U, StoreWorkError, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.root.validate_rich_failure(validate)
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<StoreWorkError>, StorePrepareError> {
        self.root.prepare_minimal_fallback()
    }
}

struct CanonicalEvidenceMemberBuild<Tag, CreateWriter, FinishWriter> {
    allocation: crate::retained::RetainedAllocationPlan,
    construction_work: u64,
    action_work: u64,
    tag: Tag,
    missing_builder: &'static str,
    create_writer: CreateWriter,
    finish_writer: FinishWriter,
}

fn recover_canonical_evidence_build<T>(
    result: Result<T, EvidenceBuildCallbackError>,
    controlled_failure: &mut Option<StoreRuntimeError>,
) -> Result<T, crate::retained::RetainedBuildFailureCode> {
    match result {
        Ok(value) => Ok(value),
        Err(EvidenceBuildCallbackError::Build(cause)) => Err(cause.into()),
        Err(EvidenceBuildCallbackError::Runtime(cause)) => {
            *controlled_failure = Some(cause);
            Err(super::failure_evidence::failure_work_build_sentinel().into())
        }
    }
}

impl<O, PP, U, L> CanonicalOutcomeEvidenceLane<'_, O, PP, U, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn qualify_failure_validation_member(
        &mut self,
        amount: u64,
    ) -> Result<FailureEvidenceWorkBudget, StoreRuntimeError> {
        match (*self.failure_work).with_existing_member_validation(amount) {
            Ok(qualified) => Ok(qualified),
            Err(cause) => {
                self.assembly
                    .reject_failure_terminal_work_envelope(FAILURE_WORK_ENVELOPE_EXHAUSTED_CODE);
                Err(cause)
            }
        }
    }

    fn qualify_evidence_member(
        &mut self,
        allocation: Result<crate::retained::RetainedAllocationPlan, StorePlanError>,
        units: usize,
    ) -> Result<
        (
            crate::retained::RetainedAllocationPlan,
            u64,
            FailureEvidenceWorkBudget,
        ),
        StoreRuntimeError,
    > {
        let allocation = allocation.map_err(StoreRuntimeError::Plan)?;
        let construction_work = u64::try_from(units)
            .map_err(|_| StoreRuntimeError::Plan(StorePlanError::SizeOverflow))?;
        let qualified = self.qualify_failure_validation_member(construction_work)?;
        Ok((allocation, construction_work, qualified))
    }

    fn try_retain_evidence_member<Kind, W, Tag, Build, CreateWriter, FinishWriter>(
        &mut self,
        member: CanonicalEvidenceMemberBuild<Tag, CreateWriter, FinishWriter>,
        build: &mut ManuallyDrop<Option<Build>>,
    ) -> Result<OutcomeEvidenceStoreRef<Kind, Tag>, StoreRuntimeError>
    where
        Tag: Copy + Send + Sync + 'static,
        Build: for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, W>,
        ) -> Result<(), StoreBuildError>,
        CreateWriter: FnOnce() -> Result<W, StoreBuildError>,
        FinishWriter: FnOnce(W) -> Result<StoredBlock, StoreBuildError>,
    {
        let CanonicalEvidenceMemberBuild {
            allocation,
            construction_work,
            action_work,
            tag,
            missing_builder,
            create_writer,
            finish_writer,
        } = member;
        let mut controlled_failure = None;
        let result = self
            .assembly
            .try_retain_success_outcome_evidence_controlled(
                self.work_dimension,
                allocation,
                |emplacer, raw_control| {
                    let mut control = recover_canonical_evidence_build(
                        OutcomeEvidenceBuildControl::try_start_ordinary(
                            raw_control,
                            construction_work,
                            action_work,
                            create_writer,
                        ),
                        &mut controlled_failure,
                    )?;
                    let built = build.as_mut().expect(missing_builder)(&mut control);
                    let block = recover_canonical_evidence_build(
                        control.finish(built, finish_writer),
                        &mut controlled_failure,
                    )?;
                    Ok(emplacer.place(block, tag))
                },
            );
        controlled_failure.map_or(result, Err)
    }

    pub fn try_retain_sequence<T, Tag>(
        &mut self,
        plan: SequencePlan<T>,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, SequenceWriter<T>>,
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
        let (allocation, construction_work, qualified) =
            match self.qualify_evidence_member(plan.allocation_plan(), plan.element_count()) {
                Ok(qualified) => qualified,
                Err(cause) => {
                    return preserve_first_fault(
                        self.first_fault,
                        finish_scoped_callback(build, Err(cause)),
                    );
                }
            };
        let result = self.try_retain_evidence_member::<ScalarSequence<T>, _, _, _, _, _>(
            CanonicalEvidenceMemberBuild {
                allocation,
                construction_work,
                action_work: 1,
                tag,
                missing_builder: "the outcome-evidence sequence builder is executed at most once",
                create_writer: || {
                    <T as super::super::sealed::StoredScalar>::new_writer(plan.capacity, plan.len)
                },
                finish_writer: SequenceWriter::finish,
            },
            &mut build,
        );
        let result = finish_scoped_callback(build, result);
        if result.is_ok() {
            *self.failure_work = qualified;
        }
        preserve_first_fault(self.first_fault, result)
    }

    pub fn try_retain_bytes<Tag>(
        &mut self,
        plan: BytesPlan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, BytesWriter>,
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
        let (allocation, construction_work, qualified) =
            match self.qualify_evidence_member(plan.allocation_plan(), plan.bytes()) {
                Ok(qualified) => qualified,
                Err(cause) => {
                    return preserve_first_fault(
                        self.first_fault,
                        finish_scoped_callback(build, Err(cause)),
                    );
                }
            };
        let result = self.try_retain_evidence_member::<ByteBlock, _, _, _, _, _>(
            CanonicalEvidenceMemberBuild {
                allocation,
                construction_work,
                action_work: 1,
                tag,
                missing_builder: "the outcome-evidence bytes builder is executed at most once",
                create_writer: || BytesWriter::new(plan),
                finish_writer: BytesWriter::finish,
            },
            &mut build,
        );
        let result = finish_scoped_callback(build, result);
        if result.is_ok() {
            *self.failure_work = qualified;
        }
        preserve_first_fault(self.first_fault, result)
    }

    pub fn try_retain_utf8<Tag>(
        &mut self,
        plan: Utf8Plan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, Utf8Writer>,
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
        let (allocation, construction_work, qualified) =
            match self.qualify_evidence_member(plan.allocation_plan(), plan.bytes()) {
                Ok(qualified) => qualified,
                Err(cause) => {
                    return preserve_first_fault(
                        self.first_fault,
                        finish_scoped_callback(build, Err(cause)),
                    );
                }
            };
        let result = self.try_retain_evidence_member::<Utf8Block, _, _, _, _, _>(
            CanonicalEvidenceMemberBuild {
                allocation,
                construction_work,
                action_work: 1,
                tag,
                missing_builder: "the outcome-evidence UTF-8 builder is executed at most once",
                create_writer: || Utf8Writer::new(plan),
                finish_writer: Utf8Writer::finish,
            },
            &mut build,
        );
        let result = finish_scoped_callback(build, result);
        if result.is_ok() {
            *self.failure_work = qualified;
        }
        preserve_first_fault(self.first_fault, result)
    }

    pub fn try_retain_records<Tag>(
        &mut self,
        plan: RecordPlan,
        tag: Tag,
        build: impl for<'control> FnMut(
            &mut OutcomeEvidenceBuildControl<'control, O, PP, U, OutcomeEvidenceRecordWriter>,
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
        let cells = match plan.rows().checked_mul(plan.fields()) {
            Some(cells) => cells,
            None => {
                return preserve_first_fault(
                    self.first_fault,
                    finish_scoped_callback(
                        build,
                        Err(StoreRuntimeError::Plan(StorePlanError::SizeOverflow)),
                    ),
                );
            }
        };
        let action_work =
            u64::try_from(plan.fields()).expect("the fixed record field limit fits in u64");
        let (allocation, construction_work, qualified) =
            match self.qualify_evidence_member(plan.allocation_plan(), cells) {
                Ok(qualified) => qualified,
                Err(cause) => {
                    return preserve_first_fault(
                        self.first_fault,
                        finish_scoped_callback(build, Err(cause)),
                    );
                }
            };
        let result = self.try_retain_evidence_member::<RecordBlock, _, _, _, _, _>(
            CanonicalEvidenceMemberBuild {
                allocation,
                construction_work,
                action_work,
                tag,
                missing_builder: "the outcome-evidence record builder is executed at most once",
                create_writer: || OutcomeEvidenceRecordWriter::new(plan),
                finish_writer: OutcomeEvidenceRecordWriter::finish,
            },
            &mut build,
        );
        let result = finish_scoped_callback(build, result);
        if result.is_ok() {
            *self.failure_work = qualified;
        }
        preserve_first_fault(self.first_fault, result)
    }

    pub fn commit(
        &mut self,
        dimension: StoreDimensionKey,
        amount: u64,
    ) -> Result<(), StoreRuntimeError> {
        if let Some(cause) = preserved_fault(self.first_fault) {
            note_later_contract(
                self.first_fault,
                validate_dimension_contract(&self.assembly.domain, dimension),
            );
            return Err(cause);
        }
        let result = self.assembly.try_commit_outcome_evidence(dimension, amount);
        preserve_first_fault(self.first_fault, result)
    }

    pub const fn complete<Kind, Tag>(
        &mut self,
        root: OutcomeEvidenceStoreRef<Kind, Tag>,
    ) -> OutcomeEvidenceDecision<Kind, Tag> {
        OutcomeEvidenceDecision::Complete(root)
    }

    pub const fn abort<Kind, Tag>(&mut self) -> OutcomeEvidenceDecision<Kind, Tag> {
        OutcomeEvidenceDecision::Abort
    }
}
