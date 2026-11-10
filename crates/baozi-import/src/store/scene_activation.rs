use baozi_core::__scene_integration::integration::{
    PreparedScenePublication, PreparedSceneSuccess, SceneActivationOutput,
    SceneActivationPostflight, SceneActivationRejection, SceneCanonicalRootTag, SceneConstruction,
    SceneEvidenceRecipe, SceneGenerationRejection, SceneOutcomeEvidenceTag, SceneParseOutcome,
    ScenePublicationError, SceneValidationProof, consume_scene_freeze_permit,
};
use baozi_operation::__retained_store_integration::activation::{
    ActivatedCanonicalStoreSession, CanonicalActivationOutcome, CanonicalActivationSeal,
    CanonicalActivationSlots, CanonicalCandidateStoreSession, CanonicalCauseSlot,
    CanonicalCentralValidationOutcome, CanonicalFinal, CanonicalFreezePermitIssue,
    CanonicalGenerationSlots, CanonicalInvocationOutcome, CanonicalOutputSlot, CanonicalProofSlot,
    CanonicalSemanticWorkError, CanonicalValidationProgress, CanonicalValidationSlots,
    CentrallyRejectedCanonicalStoreSession, DynamicFailureEvidenceOutcome,
    DynamicRootedFailureValidation, FailureEvidenceOutcome, FrozenFailureState,
    InterruptedCanonicalStoreSession, NeverActivatedStoreSession, PendingMinimalFallback,
    RejectedCanonicalStoreSession, RootedFailureValidation, StoreActivationContainmentAnchor,
    StoreCanonicalRejection, StoreNeverActivatedCause,
};
use baozi_operation::__retained_store_integration::authority::{
    FreshStoreLineage, PreparedStoreFailure, PreparedStoreFailureOutcome, PreparedStoreFallback,
};
use baozi_operation::{ControlCause, FinalProgress, RecordBlock, StoreRuntimeError};

use crate::ledger::{ImportOperationOwner, ImportProgressPhase, ImportProgressUnit};

use super::{
    ImportStoreActivationError, ImportStoreDisposition, ImportStoreFailureCauseAuthority,
    ImportStoreFinish, ImportStoreHostError, ImportStoreMinimalFallback, ImportStoreOutcomeOwner,
    ImportStoreRichFailure, ImportStoreSessionData, ImportStoreSessionOwner,
    ImportStoreSessionState, ImportStoreSuccessReport, ImportStoreTerminalEvidence,
    ImportStoreTerminalOutcome,
};

#[cfg(test)]
use super::ImportStoreTerminalClass;

#[cfg(test)]
mod transition_tests;

pub(super) const IMPORT_SCENE_NOT_SUBMITTED_CODE: u32 = 0x494d_0002;

pub(super) struct ImportSceneActivationSlots<'inv> {
    pub(super) output: CanonicalOutputSlot<'inv, SceneActivationOutput>,
    pub(super) proof: CanonicalProofSlot<'inv, SceneValidationProof>,
    pub(super) activation: CanonicalActivationSlots<'inv, SceneActivationRejection>,
    pub(super) validation: CanonicalValidationSlots<'inv, SceneActivationRejection>,
}

type ImportSceneActivationOutcome<'inv> = CanonicalActivationOutcome<
    'inv,
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    SceneActivationOutput,
    SceneActivationRejection,
    RecordBlock,
    SceneCanonicalRootTag,
>;

type ImportSceneActivated<'inv> = ActivatedCanonicalStoreSession<
    'inv,
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    SceneActivationOutput,
    RecordBlock,
    SceneCanonicalRootTag,
>;

type ImportSceneCentralOutcome<'inv> = CanonicalCentralValidationOutcome<
    'inv,
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    SceneActivationOutput,
    SceneValidationProof,
    SceneActivationRejection,
    RecordBlock,
    SceneCanonicalRootTag,
    RecordBlock,
    SceneOutcomeEvidenceTag,
>;

pub(super) type ImportSceneCandidate<'inv> = CanonicalCandidateStoreSession<
    'inv,
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    SceneActivationOutput,
    SceneValidationProof,
    RecordBlock,
    SceneCanonicalRootTag,
    RecordBlock,
    SceneOutcomeEvidenceTag,
>;
pub(super) type ImportSceneActivationRejected<'inv> = RejectedCanonicalStoreSession<
    'inv,
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    SceneActivationRejection,
>;
pub(super) type ImportSceneNeverActivated<'inv, L = FreshStoreLineage> = NeverActivatedStoreSession<
    'inv,
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    SceneActivationRejection,
    L,
>;
pub(super) type ImportSceneCentralRejected<'inv> = CentrallyRejectedCanonicalStoreSession<
    'inv,
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    SceneActivationRejection,
    RecordBlock,
    SceneOutcomeEvidenceTag,
>;
pub(super) type ImportSceneCentralInterrupted<'inv> = InterruptedCanonicalStoreSession<
    'inv,
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    RecordBlock,
    SceneOutcomeEvidenceTag,
>;
pub(super) type ImportScenePreparedFailure =
    PreparedStoreFailure<RecordBlock, SceneOutcomeEvidenceTag>;

#[allow(
    clippy::large_enum_variant,
    reason = "pending retained owners stay inline so descriptor rejection remains allocation-free"
)]
pub(super) enum ImportPendingScene<'inv> {
    Candidate {
        owner: ImportSceneCandidate<'inv>,
        evidence: SceneEvidenceRecipe,
    },
    ActivationRejected {
        owner: ImportSceneActivationRejected<'inv>,
        evidence: SceneEvidenceRecipe,
    },
    NeverActivated {
        owner: ImportSceneNeverActivated<'inv>,
        evidence: SceneEvidenceRecipe,
    },
    CentralRejected {
        owner: ImportSceneCentralRejected<'inv>,
        evidence: SceneEvidenceRecipe,
    },
    CentralInterrupted {
        owner: ImportSceneCentralInterrupted<'inv>,
        evidence: SceneEvidenceRecipe,
    },
    DeferredFallback {
        owner: PendingMinimalFallback<
            'inv,
            ImportOperationOwner,
            ImportProgressPhase,
            ImportProgressUnit,
        >,
        completed_boundary: Option<ImportStoreSceneBoundary>,
    },
}

pub(crate) struct ImportStoreFinishRequest {
    evidence: SceneEvidenceRecipe,
    outcome: SceneParseOutcome,
    unsubmitted: bool,
    _vocabulary: super::ImportStoreVocabulary,
    _completed_boundary: Option<ImportStoreSceneBoundary>,
    _importer_boundary:
        baozi_operation::__retained_store_integration::activation::StoreImporterBoundary,
}

#[allow(
    clippy::large_enum_variant,
    reason = "terminal owners stay inline so the final fail-closed pivot cannot allocate or lose linear ownership"
)]
pub(super) enum ImportStorePreparedScene {
    Success {
        publication: PreparedSceneSuccess,
    },
    RichFailure {
        store: ImportScenePreparedFailure,
        control_cause: Option<ControlCause>,
        cause_authority: ImportStoreFailureCauseAuthority,
    },
    MinimalFallback {
        store: PreparedStoreFallback,
        control_cause: Option<ControlCause>,
        cause_authority: ImportStoreFailureCauseAuthority,
    },
}

const fn scene_final_progress() -> FinalProgress<ImportProgressPhase, ImportProgressUnit> {
    FinalProgress::new(
        ImportProgressPhase::Finalize,
        ImportProgressUnit::CanonicalItems,
        1,
    )
}

fn runtime_control(cause: &StoreRuntimeError) -> Option<ControlCause> {
    match cause {
        StoreRuntimeError::Control(cause) => Some(cause.clone()),
        _ => None,
    }
}

fn canonical_rejection_control<E>(cause: &StoreCanonicalRejection<E>) -> Option<ControlCause> {
    match cause {
        StoreCanonicalRejection::Runtime(cause) => runtime_control(cause),
        StoreCanonicalRejection::Requested(_) => None,
    }
}

fn canonical_rejection_is_authoring(
    cause: &StoreCanonicalRejection<SceneActivationRejection>,
) -> bool {
    matches!(cause, StoreCanonicalRejection::Requested(cause) if cause.is_authoring_rejection())
}

fn scene_rejection_control(
    cause: &StoreCanonicalRejection<SceneActivationRejection>,
) -> Option<ControlCause> {
    canonical_rejection_control(cause).or_else(|| match cause {
        StoreCanonicalRejection::Requested(cause) => cause.control_cause().cloned(),
        StoreCanonicalRejection::Runtime(_) => None,
    })
}

fn canonical_rejection_validation(
    cause: &StoreCanonicalRejection<SceneActivationRejection>,
) -> Option<baozi_core::SceneValidationError> {
    match cause {
        StoreCanonicalRejection::Requested(cause) => cause.scene_validation_error(),
        StoreCanonicalRejection::Runtime(_) => None,
    }
}

fn never_activated_validation(
    cause: &StoreNeverActivatedCause<SceneActivationRejection>,
) -> Option<baozi_core::SceneValidationError> {
    match cause {
        StoreNeverActivatedCause::Requested(cause) => cause.scene_validation_error(),
        StoreNeverActivatedCause::Runtime(_) => None,
    }
}

fn validation_cause_authority(
    detail: Option<baozi_core::SceneValidationError>,
) -> ImportStoreFailureCauseAuthority {
    detail.map_or(
        ImportStoreFailureCauseAuthority::OperationTerminal,
        ImportStoreFailureCauseAuthority::SceneValidation,
    )
}

fn never_activated_control(
    cause: &StoreNeverActivatedCause<SceneActivationRejection>,
) -> Option<ControlCause> {
    match cause {
        StoreNeverActivatedCause::Runtime(cause) => runtime_control(cause),
        StoreNeverActivatedCause::Requested(cause) => cause.control_cause().cloned(),
    }
}

fn never_activated_is_authoring(
    cause: &StoreNeverActivatedCause<SceneActivationRejection>,
) -> bool {
    matches!(cause, StoreNeverActivatedCause::Requested(cause) if cause.is_authoring_rejection())
}

#[allow(
    clippy::result_large_err,
    reason = "activation errors preserve exact containment and preparation ownership without boxing"
)]
fn activation_failure_result(
    authoring_rejected: bool,
    validation_error: Option<baozi_core::SceneValidationError>,
    control_cause: Option<ControlCause>,
) -> Result<(), ImportStoreActivationError> {
    if let Some(cause) = control_cause {
        Err(ImportStoreActivationError::Control(cause))
    } else if let Some(detail) = validation_error {
        Err(ImportStoreActivationError::SceneValidation(detail))
    } else if authoring_rejected {
        Err(ImportStoreActivationError::AuthoringRejected)
    } else {
        Err(ImportStoreActivationError::MaterializationFailed)
    }
}

fn map_prepared_failure(
    prepared: PreparedStoreFailureOutcome<RecordBlock, SceneOutcomeEvidenceTag>,
    control_cause: Option<ControlCause>,
    cause_authority: ImportStoreFailureCauseAuthority,
) -> ImportStorePreparedScene {
    match prepared {
        PreparedStoreFailureOutcome::RichFailure(store) => ImportStorePreparedScene::RichFailure {
            store,
            control_cause,
            cause_authority,
        },
        PreparedStoreFailureOutcome::MinimalFallback(store) => {
            ImportStorePreparedScene::MinimalFallback {
                store,
                control_cause,
                cause_authority,
            }
        }
    }
}

#[allow(
    clippy::result_large_err,
    reason = "failure preparation preserves the complete linear retained-store error owner"
)]
fn prepare_ordinary_failure<Cause>(
    validation: DynamicRootedFailureValidation<
        '_,
        ImportOperationOwner,
        ImportProgressPhase,
        ImportProgressUnit,
        Cause,
        RecordBlock,
        SceneOutcomeEvidenceTag,
    >,
    final_progress: FinalProgress<ImportProgressPhase, ImportProgressUnit>,
    control_cause: Option<ControlCause>,
    cause_authority: ImportStoreFailureCauseAuthority,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    match validation {
        RootedFailureValidation::Valid(validated) => {
            let (prepared, _cause, final_control_cause) = validated
                .try_prepare(final_progress)
                .map_err(ImportStoreActivationError::Preparation)?
                .into_parts_with_control();
            Ok(map_prepared_failure(
                prepared,
                final_control_cause.or(control_cause),
                cause_authority,
            ))
        }
        RootedFailureValidation::Rejected(rejected) => {
            let (store, _cause) = rejected
                .prepare_minimal_fallback()
                .map_err(ImportStoreActivationError::Preparation)?
                .into_parts();
            Ok(ImportStorePreparedScene::MinimalFallback {
                store,
                control_cause,
                cause_authority,
            })
        }
        RootedFailureValidation::Fallback(fallback) => {
            let control_cause = runtime_control(fallback.cause()).or(control_cause);
            let (store, _cause) = fallback.into_parts();
            Ok(ImportStorePreparedScene::MinimalFallback {
                store,
                control_cause,
                cause_authority,
            })
        }
    }
}

#[allow(
    clippy::result_large_err,
    reason = "frozen failure preparation preserves the complete linear retained-store error owner"
)]
fn prepare_frozen_failure<Cause>(
    validation: DynamicRootedFailureValidation<
        '_,
        ImportOperationOwner,
        ImportProgressPhase,
        ImportProgressUnit,
        Cause,
        RecordBlock,
        SceneOutcomeEvidenceTag,
        FrozenFailureState,
    >,
    control_cause: Option<ControlCause>,
    cause_authority: ImportStoreFailureCauseAuthority,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    match validation {
        RootedFailureValidation::Valid(validated) => {
            let (prepared, _cause) = validated
                .try_prepare_frozen()
                .map_err(ImportStoreActivationError::Preparation)?
                .into_parts();
            Ok(map_prepared_failure(
                prepared,
                control_cause,
                cause_authority,
            ))
        }
        RootedFailureValidation::Rejected(rejected) => {
            let (store, _cause) = rejected
                .prepare_minimal_fallback()
                .map_err(ImportStoreActivationError::Preparation)?
                .into_parts();
            Ok(ImportStorePreparedScene::MinimalFallback {
                store,
                control_cause,
                cause_authority,
            })
        }
        RootedFailureValidation::Fallback(fallback) => {
            let control_cause = runtime_control(fallback.cause()).or(control_cause);
            let (store, _cause) = fallback.into_parts();
            Ok(ImportStorePreparedScene::MinimalFallback {
                store,
                control_cause,
                cause_authority,
            })
        }
    }
}

#[allow(
    clippy::result_large_err,
    reason = "failure outcomes preserve exact retained evidence and containment owners without boxing"
)]
fn prepare_failure_outcome<'inv, Cause>(
    outcome: DynamicFailureEvidenceOutcome<
        'inv,
        ImportOperationOwner,
        ImportProgressPhase,
        ImportProgressUnit,
        Cause,
        RecordBlock,
        SceneOutcomeEvidenceTag,
    >,
    failure_fallback_slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
    evidence: SceneEvidenceRecipe,
    final_progress: FinalProgress<ImportProgressPhase, ImportProgressUnit>,
    control_cause: Option<ControlCause>,
    cause_authority: ImportStoreFailureCauseAuthority,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => {
            let validation = rooted
                .validate_rich_failure(failure_fallback_slot, |session, root| {
                    evidence.validate_failure(session, root)
                })
                .map_err(ImportStoreActivationError::Containment)?;
            prepare_ordinary_failure(validation, final_progress, control_cause, cause_authority)
        }
        FailureEvidenceOutcome::MinimalFallback(fallback) => {
            let (store, _cause) = fallback.into_parts();
            Ok(ImportStorePreparedScene::MinimalFallback {
                store,
                control_cause,
                cause_authority,
            })
        }
    }
}

impl ImportStorePreparedScene {
    pub(super) fn handoff(self) -> ImportStoreTerminalOutcome {
        match self {
            Self::Success { publication } => {
                let published = publication.handoff();
                let (scene, outcome_evidence, ledger_evidence) = published.into_parts();
                let evidence = ImportStoreTerminalEvidence::from_admitted_ledger(ledger_evidence);
                ImportStoreTerminalOutcome::Success {
                    scene,
                    report: ImportStoreSuccessReport {
                        _outcome_evidence: outcome_evidence,
                    },
                    evidence,
                }
            }
            Self::RichFailure {
                store,
                control_cause,
                cause_authority,
            } => {
                let published = store.handoff();
                let evidence =
                    ImportStoreTerminalEvidence::from_admitted_ledger(published.ledger_evidence);
                // A terminal checkpoint may observe control after the sole operation owner
                // already selected a ledger failure. That observation is supplemental evidence.
                ImportStoreTerminalOutcome::RichFailure {
                    failure: ImportStoreRichFailure {
                        _outcome_evidence: published.outcome_evidence,
                    },
                    evidence,
                    control_cause,
                    cause_authority,
                }
            }
            Self::MinimalFallback {
                store,
                control_cause,
                cause_authority,
            } => {
                let published = store.handoff();
                let evidence =
                    ImportStoreTerminalEvidence::from_admitted_ledger(published.ledger_evidence);
                ImportStoreTerminalOutcome::MinimalFallback {
                    fallback: ImportStoreMinimalFallback {
                        _fallback: published.fallback,
                    },
                    evidence,
                    control_cause,
                    cause_authority,
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportStoreSceneBoundary {
    DraftBuilt,
    Validated,
}

#[must_use = "the Scene activation transition must install a pending owner or become contained"]
pub(super) struct ImportStoreActivationTransition<'session, 'inv> {
    state: &'session mut ImportStoreSessionState<'inv>,
    containment: StoreActivationContainmentAnchor,
    armed: bool,
}

#[allow(
    clippy::result_large_err,
    reason = "installation failures retain the complete inline pending owner in containment"
)]
impl<'session, 'inv> ImportStoreActivationTransition<'session, 'inv> {
    pub(super) fn begin(
        state: &'session mut ImportStoreSessionState<'inv>,
    ) -> Result<(ImportStoreOutcomeOwner<'inv>, Self), ImportStoreHostError> {
        let containment = match state {
            ImportStoreSessionState::Dormant(owner) => owner.activation_containment_anchor(),
            ImportStoreSessionState::Contained => {
                return Err(ImportStoreHostError::OperationContained);
            }
            _ => return Err(ImportStoreHostError::OperationConsumed),
        };
        let previous = std::mem::replace(state, ImportStoreSessionState::Transitioning);
        let transition = Self {
            state,
            containment,
            armed: true,
        };
        match previous {
            ImportStoreSessionState::Dormant(owner) => {
                Ok((owner.into_activation_transition(), transition))
            }
            unexpected => {
                let _ = transition.containment.retain(unexpected);
                Err(ImportStoreHostError::OperationContained)
            }
        }
    }

    fn install(
        mut self,
        pending: ImportPendingScene<'inv>,
    ) -> Result<(), ImportStoreActivationError> {
        if let Some(cause) = self.containment.existing_containment() {
            let _ = self.containment.retain(pending);
            self.contain();
            return Err(ImportStoreActivationError::Containment(cause));
        }
        if matches!(*self.state, ImportStoreSessionState::Transitioning) {
            *self.state = ImportStoreSessionState::Pending(pending);
            self.armed = false;
            return Ok(());
        }
        let cause = self.containment.retain(pending);
        self.contain();
        Err(ImportStoreActivationError::Containment(cause))
    }

    fn contain(mut self) {
        self.contain_in_place();
    }

    fn contain_in_place(&mut self) {
        let _ = self.containment.contain();
        if matches!(*self.state, ImportStoreSessionState::Transitioning) {
            *self.state = ImportStoreSessionState::Contained;
        }
        self.armed = false;
    }
}

impl Drop for ImportStoreActivationTransition<'_, '_> {
    fn drop(&mut self) {
        if self.armed {
            self.contain_in_place();
        }
    }
}

#[allow(clippy::result_large_err)]
fn prepare_activation_rejection<'inv>(
    owner: CanonicalFinal<ImportSceneActivationRejected<'inv>>,
    failure_fallback_slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
    evidence: SceneEvidenceRecipe,
    unsubmitted: bool,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    let mut control_cause = scene_rejection_control(owner.cause());
    let cause_authority = if unsubmitted {
        ImportStoreFailureCauseAuthority::OriginalImport
    } else {
        validation_cause_authority(canonical_rejection_validation(owner.cause()))
    };
    let outcome = owner
        .build_failure_evidence(|lane| {
            match lane.try_retain_records(
                evidence.record_plan(),
                SceneOutcomeEvidenceTag,
                |writer| writer.write_row(|row| evidence.write_row(row)),
            ) {
                Ok(root) => lane.complete(root),
                Err(cause) => {
                    control_cause = runtime_control(&cause).or(control_cause.take());
                    lane.abort()
                }
            }
        })
        .map_err(ImportStoreActivationError::Containment)?;
    prepare_failure_outcome(
        outcome,
        failure_fallback_slot,
        evidence,
        scene_final_progress(),
        control_cause,
        cause_authority,
    )
}

#[allow(
    clippy::result_large_err,
    reason = "terminal rejection preserves the linear failure owner without allocating"
)]
fn prepare_never_activated_rejection<'inv, L>(
    owner: CanonicalFinal<ImportSceneNeverActivated<'inv, L>>,
    failure_fallback_slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
    evidence: SceneEvidenceRecipe,
    unsubmitted: bool,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    let mut control_cause = never_activated_control(owner.cause());
    let cause_authority = if unsubmitted {
        ImportStoreFailureCauseAuthority::OriginalImport
    } else {
        validation_cause_authority(never_activated_validation(owner.cause()))
    };
    let outcome = owner
        .build_failure_evidence(|lane| {
            match lane.try_retain_records(
                evidence.record_plan(),
                SceneOutcomeEvidenceTag,
                |writer| writer.write_row(|row| evidence.write_row(row)),
            ) {
                Ok(root) => lane.complete(root),
                Err(cause) => {
                    control_cause = runtime_control(&cause).or(control_cause.take());
                    lane.abort()
                }
            }
        })
        .map_err(ImportStoreActivationError::Containment)?;
    prepare_failure_outcome(
        outcome,
        failure_fallback_slot,
        evidence,
        scene_final_progress(),
        control_cause,
        cause_authority,
    )
}

#[allow(
    clippy::result_large_err,
    reason = "terminal preparation preserves the linear failure owner without allocating"
)]
fn prepare_candidate<'inv>(
    owner: CanonicalFinal<ImportSceneCandidate<'inv>>,
    generation_slots: CanonicalGenerationSlots<'inv, SceneGenerationRejection>,
    failure_fallback_slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
    evidence: SceneEvidenceRecipe,
    outcome: SceneParseOutcome,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    match owner
        .try_issue_freeze_permit(generation_slots.interruption, scene_final_progress())
        .map_err(ImportStoreActivationError::Containment)?
    {
        CanonicalFreezePermitIssue::Ready(permit) => {
            let publication = consume_scene_freeze_permit(
                permit,
                generation_slots.rejection,
                failure_fallback_slot,
                evidence,
                outcome,
            )
            .map_err(|cause| match cause {
                ScenePublicationError::Containment(cause) => {
                    ImportStoreActivationError::Containment(cause)
                }
                ScenePublicationError::Preparation(cause) => {
                    ImportStoreActivationError::Preparation(cause)
                }
            })?;
            Ok(match publication {
                PreparedScenePublication::Success(publication) => {
                    ImportStorePreparedScene::Success { publication }
                }
                PreparedScenePublication::RichFailure {
                    store,
                    control_cause,
                    rejection,
                } => ImportStorePreparedScene::RichFailure {
                    store,
                    control_cause,
                    cause_authority: generation_cause_authority(rejection),
                },
                PreparedScenePublication::MinimalFallback {
                    store,
                    control_cause,
                    rejection,
                } => ImportStorePreparedScene::MinimalFallback {
                    store,
                    control_cause,
                    cause_authority: generation_cause_authority(rejection),
                },
            })
        }
        CanonicalFreezePermitIssue::Interrupted(rooted) => {
            let control_cause = rooted.cause().control_cause().cloned();
            let validation = rooted
                .validate_rich_failure(failure_fallback_slot, |session, root| {
                    evidence.validate_failure(session, root)
                })
                .map_err(ImportStoreActivationError::Containment)?;
            prepare_frozen_failure(
                validation,
                control_cause,
                ImportStoreFailureCauseAuthority::OperationTerminal,
            )
        }
        CanonicalFreezePermitIssue::DestinationRejected(rooted) => {
            let control_cause = rooted.cause().control_cause().cloned();
            let validation = rooted
                .validate_rich_failure(failure_fallback_slot, |session, root| {
                    evidence.validate_failure(session, root)
                })
                .map_err(ImportStoreActivationError::Containment)?;
            prepare_ordinary_failure(
                validation,
                scene_final_progress(),
                control_cause,
                ImportStoreFailureCauseAuthority::OperationTerminal,
            )
        }
    }
}

fn generation_cause_authority(
    rejection: Option<SceneGenerationRejection>,
) -> ImportStoreFailureCauseAuthority {
    if rejection.is_some_and(|cause| cause.is_parse_rejected()) {
        ImportStoreFailureCauseAuthority::OriginalImport
    } else {
        ImportStoreFailureCauseAuthority::OperationTerminal
    }
}

// These terminal dispositions are mutually exclusive with successful publication. Keeping each
// rejection's validation/preparation temporaries in its own frame leaves the successful caller
// with only its live owner, rather than storage for every possible failure continuation.
#[inline(never)]
#[allow(clippy::result_large_err)]
fn prepare_central_rejection<'inv>(
    owner: CanonicalFinal<ImportSceneCentralRejected<'inv>>,
    failure_fallback_slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
    evidence: SceneEvidenceRecipe,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    let control_cause = scene_rejection_control(owner.cause());
    let cause_authority = validation_cause_authority(canonical_rejection_validation(owner.cause()));
    let validation = owner
        .validate_rich_failure(failure_fallback_slot, |session, root| {
            evidence.validate_failure(session, root)
        })
        .map_err(ImportStoreActivationError::Containment)?;
    prepare_ordinary_failure(
        validation,
        scene_final_progress(),
        control_cause,
        cause_authority,
    )
}

#[inline(never)]
#[allow(clippy::result_large_err)]
fn prepare_central_interruption<'inv>(
    owner: CanonicalFinal<ImportSceneCentralInterrupted<'inv>>,
    failure_fallback_slot: CanonicalCauseSlot<'inv, StoreRuntimeError>,
    evidence: SceneEvidenceRecipe,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    let control_cause = owner.cause().control_cause().cloned();
    let validation = owner
        .validate_rich_failure(failure_fallback_slot, |session, root| {
            evidence.validate_failure(session, root)
        })
        .map_err(ImportStoreActivationError::Containment)?;
    prepare_ordinary_failure(
        validation,
        scene_final_progress(),
        control_cause,
        ImportStoreFailureCauseAuthority::OperationTerminal,
    )
}

#[inline(never)]
#[allow(clippy::result_large_err)]
fn prepare_deferred_fallback(
    owner: CanonicalFinal<
        PendingMinimalFallback<'_, ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>,
    >,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    let control_cause = runtime_control(owner.cause());
    let (store, _cause) = owner
        .prepare_minimal_fallback()
        .map_err(ImportStoreActivationError::Preparation)?
        .into_parts();
    Ok(ImportStorePreparedScene::MinimalFallback {
        store,
        control_cause,
        cause_authority: ImportStoreFailureCauseAuthority::OperationTerminal,
    })
}

#[allow(clippy::result_large_err)]
fn activate_scene<'inv>(
    owner: ImportStoreOutcomeOwner<'inv>,
    output_slot: CanonicalOutputSlot<'inv, SceneActivationOutput>,
    activation_slots: CanonicalActivationSlots<'inv, SceneActivationRejection>,
    construction: SceneConstruction<'_>,
) -> Result<
    (
        ImportSceneActivationOutcome<'inv>,
        SceneActivationPostflight,
    ),
    ImportStoreActivationError,
> {
    let mut transaction = Some(construction.into_transaction());
    let outcome =
        match owner.plan_and_activate_scoped(output_slot, activation_slots, |mut planning| {
            let transaction = transaction
                .as_mut()
                .expect("the scoped Scene transaction executes at most once");
            let preflight = planning
                .with_semantic_work(|semantic| transaction.preflight(semantic))
                .map_err(|error| match error {
                    CanonicalSemanticWorkError::Runtime(_cause) => {
                        SceneActivationRejection::construction_invariant()
                    }
                    CanonicalSemanticWorkError::Rejected(rejection) => rejection,
                });
            let (plan, completion) = match preflight {
                Ok(ready) => ready,
                Err(rejection) => {
                    let stable_code = rejection.stable_code();
                    return planning.reject(rejection, stable_code);
                }
            };
            match planning.seal_activation(plan, completion) {
                CanonicalActivationSeal::Sealed(permit) => {
                    permit.activate(|lane, plan| plan.materialize(lane))
                }
                CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
            }
        }) {
            Ok(outcome) => outcome,
            Err(cause) => {
                return Err(ImportStoreActivationError::Containment(cause));
            }
        };
    let postflight = transaction
        .take()
        .expect("the scoped Scene transaction retains its postflight recipe")
        .finish();
    Ok((outcome, postflight))
}

// Central validation and terminal installation use disjoint sets of large linear owners.
// Keep the terminal variants out of the validator's active stack frame.
#[allow(clippy::result_large_err)]
fn validate_activated_scene<'inv>(
    transition: ImportStoreActivationTransition<'_, 'inv>,
    activated: ImportSceneActivated<'inv>,
    proof_slot: CanonicalProofSlot<'inv, SceneValidationProof>,
    validation_slots: CanonicalValidationSlots<'inv, SceneActivationRejection>,
    postflight: SceneActivationPostflight,
) -> Result<(), ImportStoreActivationError> {
    let evidence = postflight.candidate_evidence(activated.output());
    let central = activated
        .validate_centrally(
            proof_slot,
            validation_slots,
            CanonicalValidationProgress::new(
                ImportProgressPhase::Validate,
                ImportProgressUnit::WorkUnits,
            ),
            |lane, _output| match lane.try_retain_records(
                evidence.record_plan(),
                SceneOutcomeEvidenceTag,
                |writer| writer.write_row(|row| evidence.write_row(row)),
            ) {
                Ok(root) => lane.complete(root),
                Err(_) => lane.abort(),
            },
            |planning, output| postflight.validate_candidate(planning, output),
        )
        .map_err(ImportStoreActivationError::Containment)?;
    install_central_scene(transition, central, evidence)
}

#[allow(clippy::result_large_err)]
fn install_central_scene<'inv>(
    transition: ImportStoreActivationTransition<'_, 'inv>,
    central: ImportSceneCentralOutcome<'inv>,
    evidence: SceneEvidenceRecipe,
) -> Result<(), ImportStoreActivationError> {
    match central {
        CanonicalCentralValidationOutcome::Candidate(owner) => {
            transition.install(ImportPendingScene::Candidate { owner, evidence })?;
            Ok(())
        }
        CanonicalCentralValidationOutcome::Rejected(owner) => {
            let control_cause = scene_rejection_control(owner.cause());
            let detail = canonical_rejection_validation(owner.cause());
            transition.install(ImportPendingScene::CentralRejected { owner, evidence })?;
            activation_failure_result(false, detail, control_cause)
        }
        CanonicalCentralValidationOutcome::Interrupted(owner) => {
            let control_cause = owner.cause().control_cause().cloned();
            transition.install(ImportPendingScene::CentralInterrupted { owner, evidence })?;
            activation_failure_result(false, None, control_cause)
        }
        CanonicalCentralValidationOutcome::Fallback(owner) => {
            let control_cause = runtime_control(owner.cause());
            transition.install(ImportPendingScene::DeferredFallback {
                owner,
                completed_boundary: Some(ImportStoreSceneBoundary::DraftBuilt),
            })?;
            activation_failure_result(false, None, control_cause)
        }
    }
}

#[allow(clippy::result_large_err)]
fn validate_scene_activation<'inv>(
    transition: ImportStoreActivationTransition<'_, 'inv>,
    outcome: ImportSceneActivationOutcome<'inv>,
    proof_slot: CanonicalProofSlot<'inv, SceneValidationProof>,
    validation_slots: CanonicalValidationSlots<'inv, SceneActivationRejection>,
    postflight: SceneActivationPostflight,
) -> Result<(), ImportStoreActivationError> {
    match outcome {
        CanonicalActivationOutcome::Activated(activated) => validate_activated_scene(
            transition,
            activated,
            proof_slot,
            validation_slots,
            postflight,
        ),
        CanonicalActivationOutcome::Rejected(owner) => {
            let authoring_rejected = canonical_rejection_is_authoring(owner.cause());
            let detail = canonical_rejection_validation(owner.cause());
            let control_cause = scene_rejection_control(owner.cause());
            transition.install(ImportPendingScene::ActivationRejected {
                owner,
                evidence: postflight.failure_evidence(),
            })?;
            activation_failure_result(authoring_rejected, detail, control_cause)
        }
        CanonicalActivationOutcome::NeverActivated(owner) => {
            let authoring_rejected = never_activated_is_authoring(owner.cause());
            let detail = never_activated_validation(owner.cause());
            let control_cause = never_activated_control(owner.cause());
            transition.install(ImportPendingScene::NeverActivated {
                owner,
                evidence: postflight.failure_evidence(),
            })?;
            activation_failure_result(authoring_rejected, detail, control_cause)
        }
    }
}

#[allow(
    clippy::result_large_err,
    reason = "Scene activation and finalization errors preserve exact linear retained-store owners without boxing"
)]
impl<'inv> ImportStoreSessionOwner<'inv> {
    pub(crate) fn completed_scene_boundary(
        &self,
    ) -> Result<Option<ImportStoreSceneBoundary>, ImportStoreHostError> {
        Ok(match &self.state {
            ImportStoreSessionState::Pending(ImportPendingScene::Candidate { .. }) => {
                Some(ImportStoreSceneBoundary::Validated)
            }
            ImportStoreSessionState::Pending(
                ImportPendingScene::CentralRejected { .. }
                | ImportPendingScene::CentralInterrupted { .. },
            ) => Some(ImportStoreSceneBoundary::DraftBuilt),
            ImportStoreSessionState::Pending(ImportPendingScene::DeferredFallback {
                completed_boundary,
                ..
            }) => *completed_boundary,
            ImportStoreSessionState::Dormant(_)
            | ImportStoreSessionState::Transitioning
            | ImportStoreSessionState::Pending(
                ImportPendingScene::ActivationRejected { .. }
                | ImportPendingScene::NeverActivated { .. },
            )
            | ImportStoreSessionState::Contained => None,
        })
    }

    pub(crate) fn submit_scene<'input>(
        &mut self,
        construction: SceneConstruction<'input>,
    ) -> Result<(), ImportStoreActivationError> {
        let data = &mut self.storage[0];
        let (owner, transition) = ImportStoreActivationTransition::begin(&mut data.state)
            .map_err(ImportStoreActivationError::StoreUnavailable)?;
        let Some(slots) = data.canonical_slots.take() else {
            transition.contain();
            return Err(ImportStoreActivationError::StoreUnavailable(
                ImportStoreHostError::OperationContained,
            ));
        };
        let (outcome, postflight) =
            match activate_scene(owner, slots.output, slots.activation, construction) {
                Ok(activated) => activated,
                Err(cause) => {
                    transition.contain();
                    return Err(cause);
                }
            };
        validate_scene_activation(
            transition,
            outcome,
            slots.proof,
            slots.validation,
            postflight,
        )
    }

    pub(crate) fn request_finish(
        self,
        disposition: ImportStoreDisposition<'inv>,
        outcome: SceneParseOutcome,
    ) -> Result<ImportStoreFinishRequest, ImportStoreActivationError> {
        let completed_boundary = self
            .completed_scene_boundary()
            .map_err(ImportStoreActivationError::StoreUnavailable)?;
        let ImportStoreSessionData {
            state,
            canonical_slots,
            vocabulary,
            unsubmitted_evidence,
            importer_boundary,
        } = self
            .into_body_state()
            .map_err(ImportStoreActivationError::StoreUnavailable)?;
        let (evidence, unsubmitted) = match state {
            ImportStoreSessionState::Pending(ImportPendingScene::Candidate { owner, evidence }) => {
                owner
                    .request_finish(disposition)
                    .map_err(ImportStoreActivationError::Containment)?;
                (evidence, false)
            }
            ImportStoreSessionState::Pending(ImportPendingScene::ActivationRejected {
                owner,
                evidence,
            }) => {
                owner
                    .request_finish(disposition)
                    .map_err(ImportStoreActivationError::Containment)?;
                (evidence, false)
            }
            ImportStoreSessionState::Pending(ImportPendingScene::NeverActivated {
                owner,
                evidence,
            }) => {
                owner
                    .request_finish(disposition)
                    .map_err(ImportStoreActivationError::Containment)?;
                (evidence, false)
            }
            ImportStoreSessionState::Pending(ImportPendingScene::CentralRejected {
                owner,
                evidence,
            }) => {
                owner
                    .request_finish(disposition)
                    .map_err(ImportStoreActivationError::Containment)?;
                (evidence, false)
            }
            ImportStoreSessionState::Pending(ImportPendingScene::CentralInterrupted {
                owner,
                evidence,
            }) => {
                owner
                    .request_finish(disposition)
                    .map_err(ImportStoreActivationError::Containment)?;
                (evidence, false)
            }
            ImportStoreSessionState::Pending(ImportPendingScene::DeferredFallback {
                owner,
                completed_boundary: _,
            }) => {
                owner
                    .request_finish(disposition)
                    .map_err(ImportStoreActivationError::Containment)?;
                (unsubmitted_evidence, false)
            }
            ImportStoreSessionState::Dormant(owner) => {
                let Some(slots) = canonical_slots else {
                    let cause = owner.activation_containment_anchor().retain(owner);
                    return Err(ImportStoreActivationError::Containment(cause));
                };
                let rejected = owner
                    .reject_without_activation::<
                        SceneActivationOutput,
                        SceneActivationRejection,
                        RecordBlock,
                        SceneCanonicalRootTag,
                    >(
                        slots.activation,
                        SceneActivationRejection::construction_invariant(),
                        IMPORT_SCENE_NOT_SUBMITTED_CODE,
                    )
                    .map_err(ImportStoreActivationError::Containment)?;
                match rejected {
                    CanonicalActivationOutcome::NeverActivated(owner) => owner
                        .request_finish(disposition)
                        .map_err(ImportStoreActivationError::Containment)?,
                    CanonicalActivationOutcome::Rejected(owner) => owner
                        .request_finish(disposition)
                        .map_err(ImportStoreActivationError::Containment)?,
                    CanonicalActivationOutcome::Activated(owner) => {
                        drop(owner);
                        return Err(ImportStoreActivationError::StoreUnavailable(
                            ImportStoreHostError::OperationContained,
                        ));
                    }
                }
                (unsubmitted_evidence, true)
            }
            ImportStoreSessionState::Transitioning | ImportStoreSessionState::Contained => {
                return Err(ImportStoreActivationError::StoreUnavailable(
                    ImportStoreHostError::OperationContained,
                ));
            }
        };
        Ok(ImportStoreFinishRequest {
            evidence,
            outcome,
            unsubmitted,
            _vocabulary: vocabulary,
            _completed_boundary: completed_boundary,
            _importer_boundary: importer_boundary,
        })
    }
}

#[allow(clippy::result_large_err)]
pub(super) fn prepare_scene(
    finish: ImportStoreFinish<'_>,
    request: ImportStoreFinishRequest,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    let finish = finish
        .admit_terminal_catalog(
            &super::import_terminal_phases(),
            &super::import_terminal_dimensions(),
        )
        .map_err(|_| {
            ImportStoreActivationError::StoreUnavailable(ImportStoreHostError::OperationContained)
        })?;
    let completion = finish.resume();
    match completion.outcome {
        CanonicalInvocationOutcome::Candidate(owner) => prepare_candidate(
            owner,
            completion.generation,
            completion.failure_fallback,
            request.evidence,
            request.outcome,
        ),
        CanonicalInvocationOutcome::Rejected(owner) => prepare_activation_rejection(
            owner,
            completion.failure_fallback,
            request.evidence,
            request.unsubmitted,
        ),
        CanonicalInvocationOutcome::NeverActivated(owner) => prepare_never_activated_rejection(
            owner,
            completion.failure_fallback,
            request.evidence,
            request.unsubmitted,
        ),
        CanonicalInvocationOutcome::CentralRejected(owner) => {
            prepare_central_rejection(owner, completion.failure_fallback, request.evidence)
        }
        CanonicalInvocationOutcome::Interrupted(owner) => {
            prepare_central_interruption(owner, completion.failure_fallback, request.evidence)
        }
        CanonicalInvocationOutcome::Fallback(owner) => prepare_deferred_fallback(owner),
        CanonicalInvocationOutcome::Operation(_) => Err(
            ImportStoreActivationError::StoreUnavailable(ImportStoreHostError::OperationContained),
        ),
    }
}

#[allow(clippy::result_large_err)]
pub(crate) fn finish_scene(
    finish: ImportStoreFinish<'_>,
    request: ImportStoreFinishRequest,
) -> Result<ImportStoreTerminalOutcome, ImportStoreActivationError> {
    prepare_scene(finish, request).map(ImportStorePreparedScene::handoff)
}
