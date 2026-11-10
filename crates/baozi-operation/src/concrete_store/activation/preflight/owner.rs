//! Actual dynamic final fields outlive every short activation permission.

use super::*;
use crate::concrete_store::{RawStoreOperation, StorePublicationState};

type ActivationResult<'host, O, PP, U, T, E, CK, CT, L> = Result<
    CanonicalActivationOutcome<'host, O, PP, U, T, E, CK, CT, L>,
    StoreActivationContainment,
>;

pub(super) struct ScopedOwnerTransition<'host, O, PP, U, T, E, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) raw: Option<RawStoreOperation<'host, O, PP, U>>,
    pub(super) domain: Option<StoreDomainLoan<'host>>,
    pub(super) lineage: Option<L>,
    pub(super) publication_state: StorePublicationState,
    pub(super) opened: Option<StorePublicationOpenDisposition>,
    pub(super) pending: Option<ScopedPendingDecision<'host, T, E, CK, CT>>,
    pub(super) failure: BoundScopedFailureRequest,
    pub(super) failure_work: FailureEvidenceWorkBudget,
    anchor: RetainedPlanningContainmentAnchor,
    finished: bool,
}

pub(super) fn drop_pending_decision<T, E, CK, CT>(
    pending: ScopedPendingDecision<'_, T, E, CK, CT>,
) {
    match pending {
        ScopedPendingDecision::Activated { output, .. } => {
            let _ = drop_scoped_value(output);
        }
        ScopedPendingDecision::Rejected { cause, .. } => {
            let _ = drop_scoped_value(cause);
        }
        ScopedPendingDecision::NeverActivated { cause, .. } => {
            let _ = drop_scoped_value(cause);
        }
        ScopedPendingDecision::Contain { .. } => {}
    }
}

impl<'host, O, PP, U, T, E, CK, CT, L> ScopedOwnerTransition<'host, O, PP, U, T, E, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    pub(super) fn new(
        dormant: ScopedDormantActivation<'host, O, PP, U, L>,
        failure: BoundScopedFailureRequest,
        failure_work: FailureEvidenceWorkBudget,
    ) -> Result<Self, StoreActivationContainment> {
        let anchor = dormant.containment_anchor.share();
        let RetainedStoreOperation {
            raw,
            domain,
            lineage,
        } = dormant.into_operation()?;
        Ok(Self {
            raw: Some(raw),
            domain: Some(domain),
            lineage: Some(lineage),
            publication_state: StorePublicationState::default(),
            opened: None,
            pending: None,
            failure,
            failure_work,
            anchor,
            finished: false,
        })
    }

    pub(super) fn contain(&mut self) -> StoreActivationContainment {
        if let Some(ScopedPendingDecision::Contain {
            recorded: Some(containment),
        }) = self.pending.as_ref()
        {
            let containment = *containment;
            self.pending.take();
            self.finished = true;
            return containment;
        }
        if let Some(pending) = self.pending.take() {
            drop_pending_decision(pending);
        }
        let cause = if let Some(raw) = self.raw.take() {
            let _ = self.lineage.take().map(drop_scoped_value);
            match self.opened.take() {
                Some(StorePublicationOpenDisposition::Opened(opened)) => {
                    match raw.bind_opened_assembly(opened) {
                        Ok(assembly) => assembly.contain_activation_fault(),
                        Err((raw, _)) => raw.contain_activation_fault(),
                    }
                }
                Some(StorePublicationOpenDisposition::Failed(_)) | None => {
                    raw.contain_activation_fault()
                }
            }
        } else {
            self.anchor.contain()
        };
        self.finished = true;
        StoreActivationContainment { cause }
    }

    pub(super) fn finish(
        mut self,
        valid: bool,
    ) -> ActivationResult<'host, O, PP, U, T, E, CK, CT, L> {
        if !valid
            || matches!(
                self.pending,
                None | Some(ScopedPendingDecision::Contain { .. })
            )
        {
            return Err(self.contain());
        }
        match self.opened.take() {
            Some(StorePublicationOpenDisposition::Opened(opened)) => {
                let raw = self
                    .raw
                    .take()
                    .expect("the original raw operation remains installed");
                let assembly = match raw.bind_opened_assembly(opened) {
                    Ok(assembly) => assembly,
                    Err((raw, _)) => {
                        self.raw = Some(raw);
                        return Err(self.contain());
                    }
                };
                match self
                    .pending
                    .take()
                    .expect("a valid transaction selected one pending branch")
                {
                    ScopedPendingDecision::Activated {
                        output,
                        root,
                        validation,
                    } => {
                        let Some(lineage) = self.lineage.take() else {
                            let _ = drop_scoped_value(output);
                            self.finished = true;
                            return Err(StoreActivationContainment {
                                cause: assembly.contain_activation_fault(),
                            });
                        };
                        let domain = self
                            .domain
                            .take()
                            .expect("the original domain remains owned");
                        let assembly = RetainedStorePublicationAssembly {
                            raw: assembly,
                            state: std::mem::take(&mut self.publication_state),
                            domain,
                            lineage,
                        };
                        self.finished = true;
                        Ok(CanonicalActivationOutcome::Activated(
                            ActivatedCanonicalStoreSession {
                                canonical_root: root,
                                owner: PayloadOwner::new(output, assembly),
                                failure_work: self.failure_work,
                                validation_phase: validation.phase,
                                validation_work_dimension: validation.work_dimension,
                                validation_scratch_dimension: validation.scratch_dimension,
                                canonical_destination_dimension: validation.canonical_destination,
                                outcome_evidence_destination_dimension: self
                                    .failure
                                    .outcome_evidence_destination_dimension,
                                validation_work: validation.work,
                            },
                        ))
                    }
                    ScopedPendingDecision::Rejected {
                        cause,
                        failure_code,
                    } => {
                        let cause = ManuallyDrop::new(cause);
                        let raw = match assembly.into_verified_failure_only(failure_code) {
                            Ok(raw) => raw,
                            Err(error) => {
                                let _ = drop_scoped_value(ManuallyDrop::into_inner(cause));
                                self.finished = true;
                                return Err(StoreActivationContainment { cause: error });
                            }
                        };
                        let domain = self
                            .domain
                            .take()
                            .expect("the original rejected domain remains owned");
                        self.finished = true;
                        Ok(CanonicalActivationOutcome::Rejected(
                            RejectedCanonicalStoreSession {
                                owner: UnrootedFailureEvidence {
                                    owner: PayloadOwner::new(ManuallyDrop::into_inner(cause), raw),
                                    domain,
                                    state: std::mem::take(&mut self.publication_state),
                                    failure_code,
                                    failure_work: self.failure_work,
                                    outcome_evidence_destination_dimension: self
                                        .failure
                                        .outcome_evidence_destination_dimension,
                                },
                            },
                        ))
                    }
                    pending => {
                        drop_pending_decision(pending);
                        let _ = self.lineage.take().map(drop_scoped_value);
                        self.finished = true;
                        Err(StoreActivationContainment {
                            cause: assembly.contain_activation_fault(),
                        })
                    }
                }
            }
            failure => {
                let raw = self
                    .raw
                    .take()
                    .expect("the original unopened operation remains installed");
                let raw = match failure {
                    Some(StorePublicationOpenDisposition::Failed(failure)) => {
                        match raw.bind_publication_open_failure(failure) {
                            Ok(error) => match error.into_activation_disposition() {
                                RetainedActivationOpenDisposition::Retry(raw)
                                | RetainedActivationOpenDisposition::FallbackOnly(raw) => raw,
                                RetainedActivationOpenDisposition::FailClosed(raw) => {
                                    self.raw = Some(raw);
                                    return Err(self.contain());
                                }
                            },
                            Err((raw, _)) => {
                                self.raw = Some(raw);
                                return Err(self.contain());
                            }
                        }
                    }
                    None => raw,
                    Some(StorePublicationOpenDisposition::Opened(_)) => {
                        unreachable!("opened fields use the other concrete binder")
                    }
                };
                let Some(lineage) = self.lineage.take() else {
                    self.raw = Some(raw);
                    return Err(self.contain());
                };
                let domain = self
                    .domain
                    .take()
                    .expect("the original unopened domain remains owned");
                let operation = RetainedStoreOperation {
                    raw,
                    domain,
                    lineage,
                };
                match self
                    .pending
                    .take()
                    .expect("the valid unopened branch remains selected")
                {
                    ScopedPendingDecision::NeverActivated {
                        cause,
                        failure_code,
                        fallback_only,
                    } => {
                        self.finished = true;
                        Ok(CanonicalActivationOutcome::NeverActivated(
                            NeverActivatedStoreSession {
                                owner: NeverActivatedOwner {
                                    cause: Some(cause),
                                    authority: Some(if fallback_only {
                                        NeverActivatedAuthority::FallbackOnly(operation)
                                    } else {
                                        NeverActivatedAuthority::Operation(operation)
                                    }),
                                },
                                failure_code,
                                outcome_evidence_plan: self.failure.outcome_evidence_plan,
                                failure_work: self.failure_work,
                                outcome_evidence_destination_dimension: self
                                    .failure
                                    .outcome_evidence_destination_dimension,
                            },
                        ))
                    }
                    pending => {
                        drop_pending_decision(pending);
                        self.finished = true;
                        Err(contain_operation(operation))
                    }
                }
            }
        }
    }
}

impl<O, PP, U, T, E, CK, CT, L> Drop for ScopedOwnerTransition<'_, O, PP, U, T, E, CK, CT, L>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.contain();
        }
    }
}
