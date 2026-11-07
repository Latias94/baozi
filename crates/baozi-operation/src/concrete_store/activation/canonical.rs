use super::super::{RawStoreAssembly, StoreFallbackState, bind_resolved_success_failure};
use super::central_driver::{
    CentralFailureFacts, CentralPendingDecision, CentralValidationFacts, CentralValidationRequest,
    drop_pending_central_decision, run_central_validation,
};
use super::final_state::CanonicalFinal;
use super::payload_owner::PayloadValue as CentralValue;
use super::*;
use crate::retained::{RetainedClosureValidation, RetainedSuccessValidation};

pub(super) mod finite;

type CentralValidationResult<
    'inv,
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
> = Result<
    CanonicalCentralValidationOutcome<
        'inv,
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
>;

// Keep mutually exclusive allocation-free terminal owners out of the active validator frame.
// Windows test threads use a 2 MiB stack, so these phase boundaries are part of the API's
// operational footprint rather than an optimization hint.
struct CentralValidationTransition<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    raw: Option<RawStoreAssembly<'host, O, PP, U>>,
    state: StorePublicationState,
    domain: Option<StoreDomainLoan<'host>>,
    lineage: Option<L>,
    output: Option<PayloadLoan<'host, T>>,
    pending: Option<CentralPendingDecision<'host, T, Proof, E, CK, CT, EK, ET>>,
}

impl<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET, L>
    CentralValidationTransition<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn new(
        output: PayloadLoan<'host, T>,
        assembly: RetainedStorePublicationAssembly<'host, O, PP, U, L>,
    ) -> Self {
        let RetainedStorePublicationAssembly {
            raw,
            state,
            domain,
            lineage,
        } = assembly;
        Self {
            raw: Some(raw),
            state,
            domain: Some(domain),
            lineage: Some(lineage),
            output: Some(output),
            pending: None,
        }
    }

    fn contain(&mut self) -> Option<StoreActivationContainment> {
        let recorded = match self.pending.take() {
            Some(CentralPendingDecision::Contain { recorded }) => recorded,
            pending => {
                let _ = drop_pending_central_decision(pending);
                None
            }
        };
        let _ = drop_scoped_value(self.output.take());
        let _ = drop_scoped_value(self.lineage.take());
        self.raw.take().map(|raw| match recorded {
            Some(recorded) => {
                drop(raw);
                recorded
            }
            None => StoreActivationContainment {
                cause: raw.contain_activation_fault(),
            },
        })
    }

    fn into_containment(mut self) -> StoreActivationContainment {
        self.contain()
            .expect("the central transition retains its original storage")
    }

    #[inline(never)]
    fn bind(
        mut self,
        failure_work: FailureEvidenceWorkBudget,
        evidence_destination: StoreDimensionKey,
    ) -> CentralValidationResult<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET> {
        if drop_scoped_value(self.lineage.take()).is_err() {
            return Err(self.into_containment());
        }
        match self.pending.as_ref() {
            Some(CentralPendingDecision::Candidate { .. }) => self.bind_candidate(failure_work),
            Some(CentralPendingDecision::Rejected { .. }) => {
                self.bind_rejected(failure_work, evidence_destination)
            }
            Some(CentralPendingDecision::Interrupted { .. }) => {
                self.bind_interrupted(failure_work, evidence_destination)
            }
            Some(CentralPendingDecision::Fallback { .. }) => self.bind_fallback(),
            Some(CentralPendingDecision::Contain { .. }) | None => Err(self.into_containment()),
        }
    }

    #[inline(never)]
    fn bind_candidate(
        mut self,
        failure_work: FailureEvidenceWorkBudget,
    ) -> CentralValidationResult<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET> {
        let Some(CentralPendingDecision::Candidate {
            output,
            proof,
            closure,
            canonical_root,
            evidence_root,
            destinations,
        }) = self.pending.take()
        else {
            return Err(self.into_containment());
        };
        let mut output = CentralValue::new(output);
        let mut proof = CentralValue::new(proof);
        let raw = self
            .raw
            .take()
            .expect("the central candidate retains its assembly");
        let raw = match raw.bind_resolved_success(RetainedClosureValidation::Valid(closure)) {
            RetainedSuccessValidation::Valid(raw) => raw,
            RetainedSuccessValidation::Rejected(raw) => {
                let _ = proof.dispose();
                let _ = output.dispose();
                return Err(StoreActivationContainment {
                    cause: raw.contain_activation_fault(),
                });
            }
            RetainedSuccessValidation::FallbackOnly(raw) => {
                let _ = proof.dispose();
                let _ = output.dispose();
                return Err(StoreActivationContainment {
                    cause: raw.contain_activation_fault(),
                });
            }
        };
        let storage = ValidatedStoreSuccess {
            raw,
            canonical_root,
            evidence_root,
            domain: self
                .domain
                .take()
                .expect("the candidate retains its original domain"),
        };
        let output = PayloadOwner::new(
            output
                .value
                .take()
                .expect("the candidate retains its output permission"),
            storage,
        );
        Ok(CanonicalCentralValidationOutcome::Candidate(
            CanonicalCandidateStoreSession {
                destinations,
                owner: PayloadOwner::new(
                    proof
                        .value
                        .take()
                        .expect("the candidate retains its proof permission"),
                    output,
                ),
                _failure_work: failure_work,
            },
        ))
    }

    #[inline(never)]
    fn bind_failure<EvidenceKind, EvidenceTag>(
        &mut self,
        failure: CentralFailureFacts<EvidenceKind, EvidenceTag>,
    ) -> StoreFailureAssembly<'host, O, PP, U, EvidenceKind, EvidenceTag> {
        StoreFailureAssembly {
            raw: bind_resolved_success_failure(
                self.raw
                    .take()
                    .expect("the central failure retains its original assembly"),
                failure.failure_cause,
            ),
            state: std::mem::take(&mut self.state),
            forced_cause: failure.forced_cause,
            evidence_root: failure.evidence_root,
            domain: self
                .domain
                .take()
                .expect("the failure retains its original domain"),
        }
    }

    #[inline(never)]
    fn bind_rejected(
        mut self,
        failure_work: FailureEvidenceWorkBudget,
        evidence_destination: StoreDimensionKey,
    ) -> CentralValidationResult<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET> {
        let Some(CentralPendingDecision::Rejected { cause, failure }) = self.pending.take() else {
            return Err(self.into_containment());
        };
        let mut cause = CentralValue::new(cause);
        let failure = self.bind_failure(failure);
        Ok(CanonicalCentralValidationOutcome::Rejected(
            CentrallyRejectedCanonicalStoreSession {
                root: RootedFailureEvidence {
                    owner: PayloadOwner::new(
                        cause
                            .value
                            .take()
                            .expect("the rejected candidate retains its cause"),
                        failure,
                    ),
                    failure_work,
                    outcome_evidence_destination_dimension: evidence_destination,
                    _lifecycle: PhantomData::<OrdinaryFailureState>,
                },
            },
        ))
    }

    #[inline(never)]
    fn bind_interrupted(
        mut self,
        failure_work: FailureEvidenceWorkBudget,
        evidence_destination: StoreDimensionKey,
    ) -> CentralValidationResult<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET> {
        let Some(CentralPendingDecision::Interrupted { cause, failure }) = self.pending.take()
        else {
            return Err(self.into_containment());
        };
        let mut cause = CentralValue::new(cause);
        let failure = self.bind_failure(failure);
        Ok(CanonicalCentralValidationOutcome::Interrupted(
            InterruptedCanonicalStoreSession {
                root: RootedFailureEvidence {
                    owner: PayloadOwner::new(
                        cause
                            .value
                            .take()
                            .expect("the interrupted candidate retains its cause"),
                        failure,
                    ),
                    failure_work,
                    outcome_evidence_destination_dimension: evidence_destination,
                    _lifecycle: PhantomData::<OrdinaryFailureState>,
                },
            },
        ))
    }

    #[inline(never)]
    fn bind_fallback(
        mut self,
    ) -> CentralValidationResult<'host, O, PP, U, T, Proof, E, CK, CT, EK, ET> {
        let Some(CentralPendingDecision::Fallback {
            cause,
            code,
            provenance,
        }) = self.pending.take()
        else {
            return Err(self.into_containment());
        };
        let mut cause = CentralValue::new(cause);
        let raw = self
            .raw
            .take()
            .expect("the central fallback retains its original assembly");
        let state = match provenance {
            super::super::ordinary::StoreSuccessFallbackProvenance::EarlyAssemblyLatch => {
                StoreFallbackState::Assembly { raw, cause: code }
            }
            super::super::ordinary::StoreSuccessFallbackProvenance::RawValidated {
                activation_containment_required,
            } => {
                match raw.bind_resolved_success(RetainedClosureValidation::FallbackOnly {
                    cause: code,
                    activation_containment_required,
                }) {
                    RetainedSuccessValidation::FallbackOnly(raw) => {
                        StoreFallbackState::Validated(raw)
                    }
                    RetainedSuccessValidation::Valid(raw) => {
                        let _ = cause.dispose();
                        return Err(StoreActivationContainment {
                            cause: raw.contain_activation_fault(),
                        });
                    }
                    RetainedSuccessValidation::Rejected(raw) => {
                        let _ = cause.dispose();
                        return Err(StoreActivationContainment {
                            cause: raw.contain_activation_fault(),
                        });
                    }
                }
            }
            super::super::ordinary::StoreSuccessFallbackProvenance::FailureUnverified => {
                StoreFallbackState::FailureAssembly(bind_resolved_success_failure(raw, code))
            }
        };
        let fallback = StoreFallbackOnly {
            state,
            domain: self
                .domain
                .take()
                .expect("the fallback retains its original domain"),
        };
        Ok(CanonicalCentralValidationOutcome::Fallback(
            PendingMinimalFallback {
                owner: PayloadOwner::new(
                    cause
                        .value
                        .take()
                        .expect("the central fallback retains its cause permission"),
                    fallback,
                ),
            },
        ))
    }
}

impl<O, PP, U, T, Proof, E, CK, CT, EK, ET, L> Drop
    for CentralValidationTransition<'_, O, PP, U, T, Proof, E, CK, CT, EK, ET, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        let _ = self.contain();
    }
}

impl<'inv, O, PP, U, T, CanonicalKind, CanonicalTag, L>
    ActivatedCanonicalStoreSession<'inv, O, PP, U, T, CanonicalKind, CanonicalTag, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn output(&self) -> &T {
        self.owner.value().value()
    }

    pub fn canonical_len(&self) -> usize {
        self.owner.storage().raw.loan().canonical_len()
    }

    /// Consumes the candidate through the operation-owned evidence and validation seam.
    ///
    /// The caller never receives the writable assembly. Work and destination roles come from the
    /// session owner while exact materialized shape comes from its matching keyless request.
    /// Evidence is built in a one-shot lane, then the controlled validator produces either an
    /// opaque zero-commit candidate or a failure owner. Recoverable fallback-only failures remain
    /// unprepared so callers can end borrowed input lifetimes and release charged scratch first.
    /// Any invalid root, ignored control fault, panic, or uncertain cleanup is quarantined instead
    /// of becoming a fallback result.
    #[inline(never)]
    #[allow(clippy::type_complexity, clippy::result_large_err)]
    pub fn validate_centrally<EvidenceKind, EvidenceTag, Proof, E>(
        self,
        proof_slot: CanonicalProofSlot<'inv, Proof>,
        validation_slots: CanonicalValidationSlots<'inv, E>,
        progress: CanonicalValidationProgress<PP, U>,
        build_evidence: impl for<'borrow, 'lane> FnMut(
            &mut CanonicalOutcomeEvidenceLane<'borrow, 'lane, O, PP, U, L>,
            &T,
        ) -> OutcomeEvidenceDecision<
            EvidenceKind,
            EvidenceTag,
        >,
        validate: impl for<'session, 'storage, 'scope, 'operation> FnMut(
            CanonicalValidationPlanning<
                'session,
                'storage,
                'scope,
                'operation,
                O,
                PP,
                U,
                CanonicalKind,
                CanonicalTag,
                EvidenceKind,
                EvidenceTag,
                Proof,
                E,
            >,
            &T,
        )
            -> Result<(), StoreWorkError>,
    ) -> Result<
        CanonicalCentralValidationOutcome<
            'inv,
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
        let Self {
            owner,
            canonical_root,
            failure_work,
            validation_phase,
            validation_work_dimension,
            validation_scratch_dimension,
            canonical_destination_dimension,
            outcome_evidence_destination_dimension,
            validation_work,
        } = self;
        let (output, assembly) = owner.into_parts();
        let mut transition = CentralValidationTransition::new(output, assembly);
        let mut failure_work = failure_work;
        let publication: StorePublicationLoan<'_, O, PP, U, L> = StorePublicationLoan {
            raw: transition
                .raw
                .as_mut()
                .expect("the central transition retains its assembly")
                .loan_mut()
                .borrow_ordinary(),
            domain: transition
                .domain
                .as_ref()
                .expect("the central transition retains its original domain"),
            state: &mut transition.state,
            lineage: PhantomData,
        };
        let completed = run_central_validation(
            publication,
            &mut transition.output,
            CentralValidationRequest {
                proof_slot,
                validation_slots,
                facts: CentralValidationFacts {
                    canonical_root,
                    validation: ScopedActivationValidation {
                        phase: validation_phase,
                        work_dimension: validation_work_dimension,
                        scratch_dimension: validation_scratch_dimension,
                        canonical_destination: canonical_destination_dimension,
                        work: validation_work,
                    },
                    evidence_destination: outcome_evidence_destination_dimension,
                },
                progress,
            },
            &mut failure_work,
            &mut transition.pending,
            build_evidence,
            validate,
        );
        if !completed {
            return Err(transition.into_containment());
        }
        transition.bind(failure_work, outcome_evidence_destination_dimension)
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

impl<'inv, O, PP, U, T, Proof, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    CanonicalCandidateStoreSession<
        'inv,
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
        self.owner.storage().value().value()
    }

    pub const fn proof(&self) -> &Proof {
        self.owner.value().value()
    }

    pub const fn canonical_member_count(&self) -> u64 {
        self.destinations.canonical_members.get()
    }

    pub const fn outcome_evidence_member_count(&self) -> u64 {
        self.destinations.outcome_evidence_members.get()
    }

    /// Leaves the actual candidate and proof in their host until the retained body is disposed.
    pub fn request_finish(
        self,
        disposition: CanonicalDispositionSlot<
            'inv,
            CanonicalKind,
            CanonicalTag,
            EvidenceKind,
            EvidenceTag,
        >,
    ) -> Result<(), StoreActivationContainment> {
        let Self {
            destinations,
            owner,
            _failure_work: failure_work,
        } = self;
        let (proof, output_owner) = owner.into_parts();
        let (output, store) = output_owner.into_parts();
        if !disposition.is_vacant_for(output.identity()) {
            let _ = drop_scoped_value(proof);
            let _ = drop_scoped_value(output);
            return Err(StoreActivationContainment {
                cause: store.contain_activation_fault(),
            });
        }
        let store = match store.defer_until_body_cleanup() {
            Ok(pending) => pending,
            Err(store) => {
                let _ = drop_scoped_value(proof);
                let _ = drop_scoped_value(output);
                return Err(StoreActivationContainment {
                    cause: store.contain_activation_fault(),
                });
            }
        };
        disposition.install(host::driver::PendingDisposition::Candidate(
            host::driver::PendingCandidate {
                store,
                destinations,
                failure_work,
            },
        ));
        proof.retain_in_host();
        output.retain_in_host();
        Ok(())
    }
}

impl<'inv, O, PP, U, T, Proof, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    CanonicalFinal<
        CanonicalCandidateStoreSession<
            'inv,
            O,
            PP,
            U,
            T,
            Proof,
            CanonicalKind,
            CanonicalTag,
            EvidenceKind,
            EvidenceTag,
        >,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn output(&self) -> &T {
        self.state.output()
    }

    pub const fn proof(&self) -> &Proof {
        self.state.proof()
    }

    pub const fn canonical_member_count(&self) -> u64 {
        self.state.canonical_member_count()
    }

    pub const fn outcome_evidence_member_count(&self) -> u64 {
        self.state.outcome_evidence_member_count()
    }

    /// Seals exact destinations and final operation progress before core reserves a generation.
    #[allow(clippy::type_complexity)]
    pub fn try_issue_freeze_permit(
        self,
        interruption_slot: CanonicalCauseSlot<'inv, CanonicalFreezeInterruption>,
        final_progress: FinalProgress<PP, U>,
    ) -> Result<
        CanonicalFreezePermitIssue<
            'inv,
            O,
            PP,
            U,
            T,
            Proof,
            CanonicalKind,
            CanonicalTag,
            EvidenceKind,
            EvidenceTag,
        >,
        StoreActivationContainment,
    > {
        let CanonicalCandidateStoreSession {
            destinations,
            owner,
            _failure_work: failure_work,
        } = self.state;
        let (proof, output_owner) = owner.into_parts();
        let (output, validated) = output_owner.into_parts();
        if !interruption_slot.is_vacant_for(output.identity()) {
            let _ = drop_scoped_value(proof);
            let _ = drop_scoped_value(output);
            return Err(StoreActivationContainment {
                cause: validated.contain_activation_fault(),
            });
        }
        let ZeroCommitDestinationFacts {
            _canonical_dimension: canonical_dimension,
            canonical_members,
            _outcome_evidence_dimension: outcome_evidence_dimension,
            outcome_evidence_members,
        } = destinations;
        let issued = match validated.try_issue_freeze_permit(
            canonical_dimension,
            outcome_evidence_dimension,
            final_progress,
        ) {
            Ok(issued) => issued,
            Err(cause) => {
                let _ = drop_scoped_value(proof);
                let _ = drop_scoped_value(output);
                return Err(StoreActivationContainment { cause });
            }
        };
        match issued {
            StoreSuccessFreezePermitIssue::Ready(store) => {
                output.freeze_order();
                Ok(CanonicalFreezePermitIssue::Ready(CanonicalFreezePermit {
                    owner: PayloadOwner::new(output, PayloadOwner::new(proof, store)),
                    failure_work,
                    destinations: ZeroCommitDestinationFacts {
                        _canonical_dimension: canonical_dimension,
                        canonical_members,
                        _outcome_evidence_dimension: outcome_evidence_dimension,
                        outcome_evidence_members,
                    },
                }))
            }
            StoreSuccessFreezePermitIssue::RichFailure {
                failure,
                cause,
                control_cause,
            } => {
                let proof_drop_failed = drop_scoped_value(proof).is_err();
                let output_drop_failed = drop_scoped_value(output).is_err();
                if proof_drop_failed || output_drop_failed {
                    return Err(StoreActivationContainment {
                        cause: failure.contain_activation_fault(),
                    });
                }
                let cause = match interruption_slot.place(
                    CanonicalFreezeInterruption::from_failure_code(cause, control_cause),
                ) {
                    Ok(cause) => cause,
                    Err(cause) => {
                        let _ = drop_scoped_value(cause);
                        return Err(StoreActivationContainment {
                            cause: failure.contain_activation_fault(),
                        });
                    }
                };
                Ok(CanonicalFreezePermitIssue::Interrupted(
                    RootedFailureEvidence {
                        owner: PayloadOwner::new(cause, failure),
                        failure_work,
                        outcome_evidence_destination_dimension: outcome_evidence_dimension,
                        _lifecycle: PhantomData::<FrozenFailureState>,
                    },
                ))
            }
            StoreSuccessFreezePermitIssue::DestinationRejected { failure, cause } => {
                let proof_drop_failed = drop_scoped_value(proof).is_err();
                let output_drop_failed = drop_scoped_value(output).is_err();
                if proof_drop_failed || output_drop_failed {
                    return Err(StoreActivationContainment {
                        cause: failure.contain_activation_fault(),
                    });
                }
                let cause = match interruption_slot
                    .place(CanonicalFreezeInterruption::from_failure_code(cause, None))
                {
                    Ok(cause) => cause,
                    Err(cause) => {
                        let _ = drop_scoped_value(cause);
                        return Err(StoreActivationContainment {
                            cause: failure.contain_activation_fault(),
                        });
                    }
                };
                Ok(CanonicalFreezePermitIssue::DestinationRejected(
                    RootedFailureEvidence {
                        owner: PayloadOwner::new(cause, failure),
                        failure_work,
                        outcome_evidence_destination_dimension: outcome_evidence_dimension,
                        _lifecycle: PhantomData::<OrdinaryFailureState>,
                    },
                ))
            }
        }
    }
}

impl CanonicalFreezeInterruption {
    fn from_failure_code(
        cause: PreparationFailureCode,
        control_cause: Option<ControlCause>,
    ) -> Self {
        let class = match cause.class() {
            PreparationFailureClass::Control => CanonicalFreezeInterruptionClass::Control,
            PreparationFailureClass::Ledger => CanonicalFreezeInterruptionClass::Ledger,
            PreparationFailureClass::Requested | PreparationFailureClass::ProductGraph => {
                CanonicalFreezeInterruptionClass::ProductGraph
            }
        };
        Self {
            class,
            stable_code: cause.code(),
            control_cause,
        }
    }

    pub const fn class(&self) -> CanonicalFreezeInterruptionClass {
        self.class
    }

    pub const fn stable_code(&self) -> u32 {
        self.stable_code
    }

    pub const fn control_cause(&self) -> Option<&ControlCause> {
        self.control_cause.as_ref()
    }
}

impl<'inv, O, PP, U, T, Proof, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    CanonicalFreezePermit<
        'inv,
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
    /// Borrows the exact candidate output frozen into this permit.
    ///
    /// Callers can validate descriptor pairing before generation admission without accepting a
    /// second, independently movable copy of the output.
    pub const fn output(&self) -> &T {
        self.owner.value().value()
    }

    /// Borrows the exact centrally accepted proof paired with this frozen output.
    pub const fn proof(&self) -> &Proof {
        self.owner.storage().value().value()
    }

    /// Contains an unexpected failure of the core-owned generation admission boundary.
    ///
    /// Ordinary generation exhaustion uses `reject_generation`; this consumes the complete
    /// frozen owner only after the generation source itself violated its execution contract.
    pub fn contain_generation_fault(self) -> StoreActivationContainment {
        let Self {
            owner,
            failure_work: _,
            destinations: _,
        } = self;
        let (output, proof_owner) = owner.into_parts();
        let (proof, store) = proof_owner.into_parts();
        let _ = drop_scoped_value(output);
        let _ = drop_scoped_value(proof);
        StoreActivationContainment {
            cause: store.contain_activation_fault(),
        }
    }

    /// Admits recovery before a generation source or result callback can run.
    ///
    /// Each denied-entry callback is disposed under its own unwind boundary while canonical
    /// values and their storage remain live.
    pub fn admit_generation_rejection<E, Generation, Reject, Prepare>(
        self,
        rejection_slot: &CanonicalCauseSlot<'inv, StoreCanonicalRejection<E>>,
        inputs: (Generation, Reject, Prepare),
    ) -> Result<(Self, (Generation, Reject, Prepare)), StoreActivationContainment> {
        if rejection_slot.is_vacant_for(self.owner.value().identity()) {
            Ok((self, inputs))
        } else {
            let (generation, reject, prepare) = inputs;
            let _ = drop_scoped_value(generation);
            let _ = drop_scoped_value(reject);
            let _ = drop_scoped_value(prepare);
            Err(self.contain_generation_fault())
        }
    }

    /// Checks core's proven roots before a generation source can execute.
    ///
    /// A denied source is disposed before its canonical output, proof and physical owner.
    pub fn admit_generation_pairing<Generation>(
        self,
        canonical_root: CanonicalStoreRef<CanonicalKind, CanonicalTag>,
        evidence_root: OutcomeEvidenceStoreRef<EvidenceKind, EvidenceTag>,
        generation: Generation,
    ) -> Result<(Self, Generation), StoreActivationContainment> {
        let store = self.owner.storage().storage();
        if store.canonical_root == canonical_root && store.evidence_root == evidence_root {
            Ok((self, generation))
        } else {
            let _ = drop_scoped_value(generation);
            Err(self.contain_generation_fault())
        }
    }

    pub fn destination_counters_are_zero(&self) -> bool {
        let canonical = self
            .owner
            .storage()
            .storage()
            .terminal_observation(self.destinations._canonical_dimension)
            .expect("the sealed canonical destination remains registered");
        let evidence = self
            .owner
            .storage()
            .storage()
            .terminal_observation(self.destinations._outcome_evidence_dimension)
            .expect("the sealed outcome-evidence destination remains registered");
        [canonical, evidence].into_iter().all(|snapshot| {
            snapshot.pending_publication() == 0
                && snapshot.committed() == 0
                && snapshot.committed_canonical_output() == 0
                && snapshot.published_outcome_evidence() == 0
        })
    }

    #[allow(
        clippy::type_complexity,
        reason = "The typestate result carries the frozen failure state and its exact cause brand."
    )]
    pub fn reject_generation<E>(
        self,
        rejection_slot: CanonicalCauseSlot<'inv, StoreCanonicalRejection<E>>,
        cause: E,
        stable_code: u32,
    ) -> Result<
        CanonicalFinal<
            CentrallyRejectedCanonicalStoreSession<
                'inv,
                O,
                PP,
                U,
                E,
                EvidenceKind,
                EvidenceTag,
                FrozenFailureState,
            >,
        >,
        StoreActivationContainment,
    > {
        let Self {
            owner,
            failure_work,
            destinations,
        } = self;
        let (output, proof_owner) = owner.into_parts();
        let (proof, store) = proof_owner.into_parts();
        let cause = ManuallyDrop::new(cause);
        if !rejection_slot.is_vacant_for(output.identity()) {
            let _ = drop_scoped_value(proof);
            let _ = drop_scoped_value(output);
            let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
            return Err(StoreActivationContainment {
                cause: store.contain_activation_fault(),
            });
        }
        let proof_drop_failed = drop_scoped_value(proof).is_err();
        let output_drop_failed = drop_scoped_value(output).is_err();
        if proof_drop_failed || output_drop_failed {
            let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
            return Err(StoreActivationContainment {
                cause: store.contain_activation_fault(),
            });
        }
        let failure = match store.reject_generation(stable_code) {
            Ok(failure) => failure,
            Err(containment) => {
                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                return Err(StoreActivationContainment { cause: containment });
            }
        };
        let cause = match rejection_slot.place(StoreCanonicalRejection::Requested(
            ManuallyDrop::into_inner(cause),
        )) {
            Ok(cause) => cause,
            Err(cause) => {
                let _ = drop_scoped_value(cause);
                return Err(StoreActivationContainment {
                    cause: failure.contain_activation_fault(),
                });
            }
        };
        Ok(CanonicalFinal::new(
            CentrallyRejectedCanonicalStoreSession {
                root: RootedFailureEvidence {
                    owner: PayloadOwner::new(cause, failure),
                    failure_work,
                    outcome_evidence_destination_dimension: destinations
                        ._outcome_evidence_dimension,
                    _lifecycle: PhantomData::<FrozenFailureState>,
                },
            },
        ))
    }

    /// Opens one unforgeable joint-commit scope.
    ///
    /// The higher-ranked callback prevents either branded capability from escaping. The joint
    /// retains the actual output and proof through preparation and returns them only with
    /// committed success. A preparation fault contains the original graph.
    #[allow(clippy::type_complexity)]
    pub fn bind_joint_commit<R>(
        self,
        bind: impl for<'joint> FnOnce(
            CanonicalJointCommit<
                'inv,
                'joint,
                O,
                PP,
                U,
                T,
                Proof,
                CanonicalKind,
                CanonicalTag,
                EvidenceKind,
                EvidenceTag,
            >,
            CanonicalJointSeal<'joint>,
        ) -> R,
    ) -> R {
        let Self {
            owner,
            failure_work,
            destinations,
        } = self;
        let (output, proof_owner) = owner.into_parts();
        let (proof, store) = proof_owner.into_parts();
        bind(
            CanonicalJointCommit {
                _failure_work: failure_work,
                _destinations: destinations,
                joint: PhantomData,
                owner: PayloadOwner::new(
                    output,
                    PayloadOwner::new(proof, store.into_contribution()),
                ),
            },
            CanonicalJointSeal { joint: PhantomData },
        )
    }
}

impl<'inv, 'joint, O, PP, U, T, Proof, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>
    CanonicalJointCommit<
        'inv,
        'joint,
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
    /// Consumes the operation contribution and the seal from the same binder scope.
    ///
    /// Returns the original output and proof only after the exact joint commit. A preparation
    /// fault contains the original graph and disposes both payloads under separate unwind guards.
    #[allow(clippy::type_complexity)]
    pub fn prepare(
        self,
        seal: CanonicalJointSeal<'joint>,
    ) -> Result<
        CanonicalJointSuccess<T, Proof, CanonicalKind, CanonicalTag, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    > {
        let Self {
            _failure_work: _,
            _destinations: _,
            joint: _,
            owner,
        } = self;
        let CanonicalJointSeal { joint: _ } = seal;
        let (output, proof_owner) = owner.into_parts();
        let (proof, store) = proof_owner.into_parts();
        let payloads = PayloadPair::new(output, proof);
        let store = match store.try_prepare() {
            Ok(store) => store,
            Err(cause) => {
                let _ = drop_scoped_value(payloads);
                return Err(StoreActivationContainment { cause });
            }
        };
        let (output, proof) = payloads.into_parts();
        Ok(CanonicalJointSuccess {
            output: output.into_owned(),
            proof: proof.into_owned(),
            store,
        })
    }
}

#[allow(
    private_bounds,
    reason = "public failure typestates are sealed by the crate-private lifecycle contract"
)]
impl<'inv, O, PP, U, E, EvidenceKind, EvidenceTag, State>
    CentrallyRejectedCanonicalStoreSession<'inv, O, PP, U, E, EvidenceKind, EvidenceTag, State>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    State: FailureLifecycleState,
{
    pub const fn cause(&self) -> &StoreCanonicalRejection<E> {
        self.root.cause()
    }

    pub const fn canonical_len(&self) -> usize {
        0
    }
}

#[allow(
    private_bounds,
    reason = "public failure typestates are sealed by the crate-private lifecycle contract"
)]
impl<'inv, O, PP, U, E, EvidenceKind, EvidenceTag, State>
    CanonicalFinal<
        CentrallyRejectedCanonicalStoreSession<'inv, O, PP, U, E, EvidenceKind, EvidenceTag, State>,
    >
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    State: FailureLifecycleState,
{
    pub const fn cause(&self) -> &StoreCanonicalRejection<E> {
        self.state.cause()
    }

    pub const fn canonical_len(&self) -> usize {
        self.state.canonical_len()
    }

    #[allow(
        clippy::type_complexity,
        reason = "The typestate result preserves the rejection cause and lifecycle state."
    )]
    pub fn validate_rich_failure(
        self,
        fallback_slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
        validate: impl for<'session, 'storage, 'control> FnMut(
            &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
            FailureEvidenceMember<'session, EvidenceKind, EvidenceTag>,
        ) -> bool,
    ) -> Result<
        CanonicalRejectionFailureValidation<'inv, O, PP, U, E, EvidenceKind, EvidenceTag, State>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.state
            .root
            .validate_rich_failure(fallback_slot, validate)
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<StoreCanonicalRejection<E>>, StorePrepareError> {
        self.state.root.prepare_minimal_fallback()
    }
}

impl<'inv, O, PP, U, EvidenceKind, EvidenceTag>
    InterruptedCanonicalStoreSession<'inv, O, PP, U, EvidenceKind, EvidenceTag>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreWorkError {
        self.root.cause()
    }
}

impl<'inv, O, PP, U, EvidenceKind, EvidenceTag>
    CanonicalFinal<InterruptedCanonicalStoreSession<'inv, O, PP, U, EvidenceKind, EvidenceTag>>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub const fn cause(&self) -> &StoreWorkError {
        self.state.cause()
    }

    pub fn validate_rich_failure(
        self,
        fallback_slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
        validate: impl for<'session, 'storage, 'control> FnMut(
            &mut FailureEvidenceValidationSession<'session, 'storage, 'control, O, PP, U>,
            FailureEvidenceMember<'session, EvidenceKind, EvidenceTag>,
        ) -> bool,
    ) -> Result<
        RootedFailureValidation<'inv, O, PP, U, StoreWorkError, EvidenceKind, EvidenceTag>,
        StoreActivationContainment,
    >
    where
        EvidenceTag: Copy + Send + Sync + 'static,
    {
        self.state
            .root
            .validate_rich_failure(fallback_slot, validate)
    }

    pub fn prepare_minimal_fallback(
        self,
    ) -> Result<VerifiedMinimalFallback<StoreWorkError>, StorePrepareError> {
        self.state.root.prepare_minimal_fallback()
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

impl<'op, 'lane, O, PP, U, L> CanonicalOutcomeEvidenceLane<'op, 'lane, O, PP, U, L>
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
                self.publication
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
            .publication
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
