use std::cell::{RefCell, RefMut};

use crate::SceneDraft;
use baozi_core::scene::integration::{
    SceneActivationOutput, SceneActivationPostflight, SceneActivationRejection,
    SceneCanonicalRootTag, SceneConstruction, SceneEvidenceRecipe, SceneHandoffToken,
    SceneOutcomeEvidenceTag, SceneValidationProof, consume_scene_freeze_permit,
};
use baozi_operation::__retained_store_integration::activation::{
    ActivatedCanonicalStoreSession, CanonicalActivationOutcome, CanonicalActivationSeal,
    CanonicalCandidateStoreSession, CanonicalCentralValidationOutcome, CanonicalFreezePermitIssue,
    CanonicalValidationProgress, CentrallyRejectedCanonicalStoreSession, FailureEvidenceOutcome,
    FrozenFailureState, InterruptedCanonicalStoreSession, NeverActivatedStoreSession,
    PendingMinimalFallback, RejectedCanonicalStoreSession, RootedFailureValidation,
    StoreCanonicalRejection, StoreNeverActivatedCause, VerifiedMinimalFallback,
};
use baozi_operation::__retained_store_integration::authority::{
    PreparedStoreFailure, PreparedStoreFailureOutcome, PreparedStoreFallback, PreparedStoreSuccess,
    PreparedStoreSuccessOutcome,
};
use baozi_operation::{ControlCause, FinalProgress, RecordBlock, StoreRuntimeError};

use crate::ledger::{ImportOperationOwner, ImportProgressPhase, ImportProgressUnit};

use super::{
    ImportStoreActivationError, ImportStoreFailureCauseAuthority, ImportStoreFinalizationError,
    ImportStoreHostError, ImportStoreMinimalFallback, ImportStoreOutcomeOwner,
    ImportStoreRichFailure, ImportStoreSessionOwner, ImportStoreSessionState,
    ImportStoreSuccessReport, ImportStoreTerminalCause, ImportStoreTerminalClass,
    ImportStoreTerminalEvidence, ImportStoreTerminalOutcome, ImportStoreTerminalProjectionError,
};

pub(super) const IMPORT_SCENE_NOT_SUBMITTED_CODE: u32 = 0x494d_0002;

type ImportSceneActivationOutcome = CanonicalActivationOutcome<
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    SceneActivationOutput,
    SceneActivationRejection,
    RecordBlock,
    SceneCanonicalRootTag,
>;

type ImportSceneActivated = ActivatedCanonicalStoreSession<
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    SceneActivationOutput,
    RecordBlock,
    SceneCanonicalRootTag,
>;

type ImportSceneCentralOutcome = CanonicalCentralValidationOutcome<
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

pub(super) type ImportSceneCandidate = CanonicalCandidateStoreSession<
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
pub(super) type ImportSceneActivationRejected = RejectedCanonicalStoreSession<
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    SceneActivationRejection,
>;
pub(super) type ImportSceneNeverActivated = NeverActivatedStoreSession<
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    SceneActivationRejection,
>;
pub(super) type ImportSceneCentralRejected = CentrallyRejectedCanonicalStoreSession<
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    SceneActivationRejection,
    RecordBlock,
    SceneOutcomeEvidenceTag,
>;
pub(super) type ImportSceneCentralInterrupted = InterruptedCanonicalStoreSession<
    ImportOperationOwner,
    ImportProgressPhase,
    ImportProgressUnit,
    RecordBlock,
    SceneOutcomeEvidenceTag,
>;
pub(super) type ImportScenePreparedSuccess =
    PreparedStoreSuccess<RecordBlock, SceneCanonicalRootTag, RecordBlock, SceneOutcomeEvidenceTag>;
pub(super) type ImportScenePreparedFailure =
    PreparedStoreFailure<RecordBlock, SceneOutcomeEvidenceTag>;

#[allow(
    clippy::large_enum_variant,
    reason = "pending retained owners stay inline so descriptor rejection remains allocation-free"
)]
pub(super) enum ImportPendingScene {
    Candidate {
        owner: ImportSceneCandidate,
        evidence: SceneEvidenceRecipe,
    },
    ActivationRejected {
        owner: ImportSceneActivationRejected,
        evidence: SceneEvidenceRecipe,
    },
    NeverActivated {
        owner: ImportSceneNeverActivated,
        evidence: SceneEvidenceRecipe,
    },
    CentralRejected {
        owner: ImportSceneCentralRejected,
        evidence: SceneEvidenceRecipe,
    },
    CentralInterrupted {
        owner: ImportSceneCentralInterrupted,
        evidence: SceneEvidenceRecipe,
    },
    PreparedFallback {
        owner: VerifiedMinimalFallback<StoreRuntimeError>,
        completed_boundary: Option<ImportStoreSceneBoundary>,
    },
    DeferredFallback {
        owner:
            PendingMinimalFallback<ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>,
        completed_boundary: Option<ImportStoreSceneBoundary>,
    },
}

#[allow(
    clippy::large_enum_variant,
    reason = "terminal owners stay inline so the final fail-closed pivot cannot allocate or lose linear ownership"
)]
pub(super) enum ImportStorePreparedScene {
    Success {
        token: SceneHandoffToken,
        store: ImportScenePreparedSuccess,
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
) -> Result<SceneDraft, ImportStoreActivationError> {
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
    validation: RootedFailureValidation<
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
    validation: RootedFailureValidation<
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
fn prepare_failure_outcome<Cause>(
    outcome: FailureEvidenceOutcome<
        ImportOperationOwner,
        ImportProgressPhase,
        ImportProgressUnit,
        Cause,
        RecordBlock,
        SceneOutcomeEvidenceTag,
    >,
    evidence: SceneEvidenceRecipe,
    final_progress: FinalProgress<ImportProgressPhase, ImportProgressUnit>,
    control_cause: Option<ControlCause>,
    cause_authority: ImportStoreFailureCauseAuthority,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    match outcome {
        FailureEvidenceOutcome::Rooted(rooted) => {
            let validation = rooted
                .validate_rich_failure(|session, root| evidence.validate_failure(session, root))
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
    pub(super) fn handoff(
        self,
    ) -> Result<ImportStoreTerminalOutcome, ImportStoreTerminalProjectionError> {
        match self {
            Self::Success { token, store } => {
                let published = token
                    .handoff(store)
                    .map_err(|_| ImportStoreTerminalProjectionError::TerminalOwnerCauseMismatch)?;
                let (scene, outcome_evidence, ledger_evidence) = published.into_parts();
                let evidence = ImportStoreTerminalEvidence::try_from_ledger(ledger_evidence)?;
                if evidence.cause() != ImportStoreTerminalCause::Success {
                    return Err(ImportStoreTerminalProjectionError::TerminalOwnerCauseMismatch);
                }
                Ok(ImportStoreTerminalOutcome::Success {
                    scene,
                    report: ImportStoreSuccessReport {
                        _outcome_evidence: outcome_evidence,
                    },
                    evidence,
                })
            }
            Self::RichFailure {
                store,
                control_cause,
                cause_authority,
            } => {
                let published = store.handoff();
                let evidence =
                    ImportStoreTerminalEvidence::try_from_ledger(published.ledger_evidence)?;
                if evidence.cause() == ImportStoreTerminalCause::Success
                    || (control_cause.is_some()
                        && evidence.cause().class() != ImportStoreTerminalClass::Control)
                {
                    return Err(ImportStoreTerminalProjectionError::TerminalOwnerCauseMismatch);
                }
                Ok(ImportStoreTerminalOutcome::RichFailure {
                    failure: ImportStoreRichFailure {
                        _outcome_evidence: published.outcome_evidence,
                    },
                    evidence,
                    control_cause,
                    cause_authority,
                })
            }
            Self::MinimalFallback {
                store,
                control_cause,
                cause_authority,
            } => {
                let published = store.handoff();
                let evidence =
                    ImportStoreTerminalEvidence::try_from_ledger(published.ledger_evidence)?;
                if evidence.cause() == ImportStoreTerminalCause::Success
                    || (control_cause.is_some()
                        && evidence.cause().class() != ImportStoreTerminalClass::Control)
                {
                    return Err(ImportStoreTerminalProjectionError::TerminalOwnerCauseMismatch);
                }
                Ok(ImportStoreTerminalOutcome::MinimalFallback {
                    fallback: ImportStoreMinimalFallback {
                        _fallback: published.fallback,
                    },
                    evidence,
                    control_cause,
                    cause_authority,
                })
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
pub(super) struct ImportStoreActivationTransition<'session> {
    state: RefMut<'session, ImportStoreSessionState>,
    armed: bool,
}

impl<'session> ImportStoreActivationTransition<'session> {
    pub(super) fn begin(
        state: &'session RefCell<ImportStoreSessionState>,
    ) -> Result<(ImportStoreOutcomeOwner, Self), ImportStoreHostError> {
        let mut state = state
            .try_borrow_mut()
            .map_err(|_| ImportStoreHostError::ReentrantBorrow)?;
        let previous = std::mem::replace(&mut *state, ImportStoreSessionState::Transitioning);
        let owner = match previous {
            ImportStoreSessionState::Dormant(owner) => owner.into_activation_transition(),
            ImportStoreSessionState::Contained => {
                *state = ImportStoreSessionState::Contained;
                return Err(ImportStoreHostError::OperationContained);
            }
            previous @ (ImportStoreSessionState::Transitioning
            | ImportStoreSessionState::Pending(_)) => {
                *state = previous;
                return Err(ImportStoreHostError::OperationConsumed);
            }
        };
        Ok((owner, Self { state, armed: true }))
    }

    fn install(mut self, pending: ImportPendingScene) {
        debug_assert!(matches!(
            *self.state,
            ImportStoreSessionState::Transitioning
        ));
        *self.state = ImportStoreSessionState::Pending(pending);
        self.armed = false;
    }

    fn contain(mut self) {
        *self.state = ImportStoreSessionState::Contained;
        self.armed = false;
    }
}

impl Drop for ImportStoreActivationTransition<'_> {
    fn drop(&mut self) {
        if self.armed {
            *self.state = ImportStoreSessionState::Contained;
        }
    }
}

#[allow(clippy::result_large_err)]
fn prepare_activation_rejection(
    owner: ImportSceneActivationRejected,
    evidence: SceneEvidenceRecipe,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    let mut control_cause = scene_rejection_control(owner.cause());
    let cause_authority = validation_cause_authority(canonical_rejection_validation(owner.cause()));
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
fn prepare_never_activated_rejection(
    owner: ImportSceneNeverActivated,
    evidence: SceneEvidenceRecipe,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    let mut control_cause = never_activated_control(owner.cause());
    let cause_authority = validation_cause_authority(never_activated_validation(owner.cause()));
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
fn prepare_candidate(
    owner: ImportSceneCandidate,
    evidence: SceneEvidenceRecipe,
    draft: Option<SceneDraft>,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    match owner
        .try_issue_freeze_permit(scene_final_progress())
        .map_err(ImportStoreActivationError::Containment)?
    {
        CanonicalFreezePermitIssue::Ready(permit) => {
            // A missing descriptor only closes an already-failed import. Its terminal
            // evidence must not replace the earlier importer or scope cause. Once a
            // descriptor is supplied, pairing and generation failures belong to this
            // Scene transaction and remain operation-authoritative.
            let rejection_authority = if draft.is_none() {
                ImportStoreFailureCauseAuthority::OriginalImport
            } else {
                ImportStoreFailureCauseAuthority::OperationTerminal
            };
            consume_scene_freeze_permit(
                permit,
                draft,
                |rejected| {
                    let control_cause = canonical_rejection_control(rejected.cause());
                    let validation = rejected
                        .validate_rich_failure(|session, root| {
                            evidence.validate_failure(session, root)
                        })
                        .map_err(ImportStoreActivationError::Containment)?;
                    prepare_frozen_failure(validation, control_cause, rejection_authority)
                },
                |token, joint, seal| match joint
                    .prepare(seal)
                    .map_err(ImportStoreActivationError::Containment)?
                {
                    PreparedStoreSuccessOutcome::Success(store) => {
                        Ok(ImportStorePreparedScene::Success { token, store })
                    }
                    PreparedStoreSuccessOutcome::RichFailure(store) => {
                        drop(token);
                        Ok(ImportStorePreparedScene::RichFailure {
                            store,
                            control_cause: None,
                            cause_authority: ImportStoreFailureCauseAuthority::OperationTerminal,
                        })
                    }
                    PreparedStoreSuccessOutcome::MinimalFallback(store) => {
                        drop(token);
                        Ok(ImportStorePreparedScene::MinimalFallback {
                            store,
                            control_cause: None,
                            cause_authority: ImportStoreFailureCauseAuthority::OperationTerminal,
                        })
                    }
                },
            )
            .map_err(ImportStoreActivationError::Containment)?
        }
        CanonicalFreezePermitIssue::Interrupted(rooted) => {
            let control_cause = rooted.cause().control_cause().cloned();
            let validation = rooted
                .validate_rich_failure(|session, root| evidence.validate_failure(session, root))
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
                .validate_rich_failure(|session, root| evidence.validate_failure(session, root))
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

// These terminal dispositions are mutually exclusive with successful publication. Keeping each
// rejection's validation/preparation temporaries in its own frame leaves the successful caller
// with only its live owner, rather than storage for every possible failure continuation.
#[inline(never)]
#[allow(clippy::result_large_err)]
fn prepare_central_rejection(
    owner: ImportSceneCentralRejected,
    evidence: SceneEvidenceRecipe,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    let control_cause = scene_rejection_control(owner.cause());
    let cause_authority = validation_cause_authority(canonical_rejection_validation(owner.cause()));
    let validation = owner
        .validate_rich_failure(|session, root| evidence.validate_failure(session, root))
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
fn prepare_central_interruption(
    owner: ImportSceneCentralInterrupted,
    evidence: SceneEvidenceRecipe,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    let control_cause = owner.cause().control_cause().cloned();
    let validation = owner
        .validate_rich_failure(|session, root| evidence.validate_failure(session, root))
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
    owner: PendingMinimalFallback<ImportOperationOwner, ImportProgressPhase, ImportProgressUnit>,
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
fn prepare_unsubmitted(
    owner: ImportStoreOutcomeOwner,
    unsubmitted_evidence: SceneEvidenceRecipe,
) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
    let outcome = owner
        .reject_without_activation::<
            SceneActivationOutput,
            SceneActivationRejection,
            RecordBlock,
            SceneCanonicalRootTag,
        >(
            SceneActivationRejection::construction_invariant(),
            IMPORT_SCENE_NOT_SUBMITTED_CODE,
        )
        .map_err(ImportStoreActivationError::Containment)?;
    match outcome {
        CanonicalActivationOutcome::NeverActivated(owner) => {
            let control_cause = never_activated_control(owner.cause());
            let outcome = owner
                .build_failure_evidence(|lane| {
                    match lane.try_retain_records(
                        unsubmitted_evidence.record_plan(),
                        SceneOutcomeEvidenceTag,
                        |writer| writer.write_row(|row| unsubmitted_evidence.write_row(row)),
                    ) {
                        Ok(root) => lane.complete(root),
                        Err(_) => lane.abort(),
                    }
                })
                .map_err(ImportStoreActivationError::Containment)?;
            prepare_failure_outcome(
                outcome,
                unsubmitted_evidence,
                scene_final_progress(),
                control_cause,
                ImportStoreFailureCauseAuthority::OriginalImport,
            )
        }
        CanonicalActivationOutcome::Fallback(owner) => {
            let control_cause = runtime_control(owner.cause());
            let (store, _cause) = owner.into_parts();
            Ok(ImportStorePreparedScene::MinimalFallback {
                store,
                control_cause,
                cause_authority: ImportStoreFailureCauseAuthority::OriginalImport,
            })
        }
        CanonicalActivationOutcome::Rejected(owner) => {
            let control_cause = canonical_rejection_control(owner.cause());
            let outcome = owner
                .build_failure_evidence(|lane| {
                    match lane.try_retain_records(
                        unsubmitted_evidence.record_plan(),
                        SceneOutcomeEvidenceTag,
                        |writer| writer.write_row(|row| unsubmitted_evidence.write_row(row)),
                    ) {
                        Ok(root) => lane.complete(root),
                        Err(_) => lane.abort(),
                    }
                })
                .map_err(ImportStoreActivationError::Containment)?;
            prepare_failure_outcome(
                outcome,
                unsubmitted_evidence,
                scene_final_progress(),
                control_cause,
                ImportStoreFailureCauseAuthority::OriginalImport,
            )
        }
        CanonicalActivationOutcome::Activated(owner) => {
            drop(owner);
            Err(ImportStoreActivationError::StoreUnavailable(
                ImportStoreHostError::OperationContained,
            ))
        }
    }
}

#[allow(clippy::result_large_err)]
fn activate_scene(
    owner: ImportStoreOutcomeOwner,
    construction: SceneConstruction<'_>,
) -> Result<(ImportSceneActivationOutcome, SceneActivationPostflight), ImportStoreActivationError> {
    let mut transaction = Some(construction.into_transaction());
    let outcome = match owner.plan_and_activate_scoped(|mut planning| {
        let transaction = transaction
            .as_mut()
            .expect("the scoped Scene transaction executes at most once");
        let preflight = match planning.begin_semantic_work() {
            Ok(mut semantic) => match transaction.preflight(&mut semantic) {
                Ok(plan) => match semantic.finish_exact() {
                    Ok(completion) => Ok((plan, completion)),
                    Err(_cause) => {
                        drop(plan);
                        Err(SceneActivationRejection::construction_invariant())
                    }
                },
                Err(rejection) => {
                    let _ = semantic.finish_rejected();
                    Err(rejection)
                }
            },
            Err(_cause) => Err(SceneActivationRejection::construction_invariant()),
        };
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
fn validate_activated_scene(
    transition: ImportStoreActivationTransition<'_>,
    activated: ImportSceneActivated,
    postflight: SceneActivationPostflight,
) -> Result<SceneDraft, ImportStoreActivationError> {
    let evidence = postflight.candidate_evidence(activated.output());
    let central = activated
        .validate_centrally(
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
fn install_central_scene(
    transition: ImportStoreActivationTransition<'_>,
    central: ImportSceneCentralOutcome,
    evidence: SceneEvidenceRecipe,
) -> Result<SceneDraft, ImportStoreActivationError> {
    match central {
        CanonicalCentralValidationOutcome::Candidate(owner) => {
            let draft = owner.output().draft();
            transition.install(ImportPendingScene::Candidate { owner, evidence });
            Ok(draft)
        }
        CanonicalCentralValidationOutcome::Rejected(owner) => {
            let control_cause = scene_rejection_control(owner.cause());
            let detail = canonical_rejection_validation(owner.cause());
            transition.install(ImportPendingScene::CentralRejected { owner, evidence });
            activation_failure_result(false, detail, control_cause)
        }
        CanonicalCentralValidationOutcome::Interrupted(owner) => {
            let control_cause = owner.cause().control_cause().cloned();
            transition.install(ImportPendingScene::CentralInterrupted { owner, evidence });
            activation_failure_result(false, None, control_cause)
        }
        CanonicalCentralValidationOutcome::Fallback(owner) => {
            let control_cause = runtime_control(owner.cause());
            transition.install(ImportPendingScene::DeferredFallback {
                owner,
                completed_boundary: Some(ImportStoreSceneBoundary::DraftBuilt),
            });
            activation_failure_result(false, None, control_cause)
        }
    }
}

#[allow(clippy::result_large_err)]
fn validate_scene_activation(
    transition: ImportStoreActivationTransition<'_>,
    outcome: ImportSceneActivationOutcome,
    postflight: SceneActivationPostflight,
) -> Result<SceneDraft, ImportStoreActivationError> {
    match outcome {
        CanonicalActivationOutcome::Activated(activated) => {
            validate_activated_scene(transition, activated, postflight)
        }
        CanonicalActivationOutcome::Rejected(owner) => {
            let authoring_rejected = canonical_rejection_is_authoring(owner.cause());
            let detail = canonical_rejection_validation(owner.cause());
            let control_cause = scene_rejection_control(owner.cause());
            transition.install(ImportPendingScene::ActivationRejected {
                owner,
                evidence: postflight.failure_evidence(),
            });
            activation_failure_result(authoring_rejected, detail, control_cause)
        }
        CanonicalActivationOutcome::NeverActivated(owner) => {
            let authoring_rejected = never_activated_is_authoring(owner.cause());
            let detail = never_activated_validation(owner.cause());
            let control_cause = never_activated_control(owner.cause());
            transition.install(ImportPendingScene::NeverActivated {
                owner,
                evidence: postflight.failure_evidence(),
            });
            activation_failure_result(authoring_rejected, detail, control_cause)
        }
        CanonicalActivationOutcome::Fallback(owner) => {
            let control_cause = runtime_control(owner.cause());
            transition.install(ImportPendingScene::PreparedFallback {
                owner,
                completed_boundary: None,
            });
            activation_failure_result(false, None, control_cause)
        }
    }
}

#[allow(
    clippy::result_large_err,
    reason = "Scene activation and finalization errors preserve exact linear retained-store owners without boxing"
)]
impl ImportStoreSessionOwner {
    pub(crate) fn completed_scene_boundary(
        &self,
    ) -> Result<Option<ImportStoreSceneBoundary>, ImportStoreHostError> {
        let state = self
            .state
            .try_borrow()
            .map_err(|_| ImportStoreHostError::ReentrantBorrow)?;
        Ok(match &*state {
            ImportStoreSessionState::Pending(ImportPendingScene::Candidate { .. }) => {
                Some(ImportStoreSceneBoundary::Validated)
            }
            ImportStoreSessionState::Pending(
                ImportPendingScene::CentralRejected { .. }
                | ImportPendingScene::CentralInterrupted { .. },
            ) => Some(ImportStoreSceneBoundary::DraftBuilt),
            ImportStoreSessionState::Pending(
                ImportPendingScene::PreparedFallback {
                    completed_boundary, ..
                }
                | ImportPendingScene::DeferredFallback {
                    completed_boundary, ..
                },
            ) => *completed_boundary,
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
        &self,
        construction: SceneConstruction<'input>,
    ) -> Result<SceneDraft, ImportStoreActivationError> {
        let (owner, transition) = ImportStoreActivationTransition::begin(&self.state)
            .map_err(ImportStoreActivationError::StoreUnavailable)?;
        let (outcome, postflight) = match activate_scene(owner, construction) {
            Ok(activated) => activated,
            Err(cause) => {
                transition.contain();
                return Err(cause);
            }
        };
        validate_scene_activation(transition, outcome, postflight)
    }

    pub(super) fn prepare_scene(
        self,
        draft: Option<SceneDraft>,
    ) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
        let (state, unsubmitted_evidence) = self.into_terminal_state();
        match state {
            ImportStoreSessionState::Pending(ImportPendingScene::Candidate { owner, evidence }) => {
                prepare_candidate(owner, evidence, draft)
            }
            ImportStoreSessionState::Pending(ImportPendingScene::ActivationRejected {
                owner,
                evidence,
            }) => prepare_activation_rejection(owner, evidence),
            ImportStoreSessionState::Pending(ImportPendingScene::NeverActivated {
                owner,
                evidence,
            }) => prepare_never_activated_rejection(owner, evidence),
            ImportStoreSessionState::Pending(ImportPendingScene::CentralRejected {
                owner,
                evidence,
            }) => prepare_central_rejection(owner, evidence),
            ImportStoreSessionState::Pending(ImportPendingScene::CentralInterrupted {
                owner,
                evidence,
            }) => prepare_central_interruption(owner, evidence),
            ImportStoreSessionState::Pending(ImportPendingScene::DeferredFallback {
                owner,
                completed_boundary: _,
            }) => prepare_deferred_fallback(owner),
            ImportStoreSessionState::Pending(ImportPendingScene::PreparedFallback {
                owner,
                completed_boundary: _,
            }) => {
                let control_cause = runtime_control(owner.cause());
                let (store, _cause) = owner.into_parts();
                Ok(ImportStorePreparedScene::MinimalFallback {
                    store,
                    control_cause,
                    cause_authority: ImportStoreFailureCauseAuthority::OperationTerminal,
                })
            }
            ImportStoreSessionState::Dormant(owner) => {
                prepare_unsubmitted(owner, unsubmitted_evidence)
            }
            ImportStoreSessionState::Transitioning | ImportStoreSessionState::Contained => {
                Err(ImportStoreActivationError::StoreUnavailable(
                    ImportStoreHostError::OperationContained,
                ))
            }
        }
    }

    pub(crate) fn finish_scene(
        self,
        draft: Option<SceneDraft>,
    ) -> Result<ImportStoreTerminalOutcome, ImportStoreFinalizationError> {
        let prepared = self
            .prepare_scene(draft)
            .map_err(ImportStoreFinalizationError::Activation)?;
        prepared
            .handoff()
            .map_err(ImportStoreFinalizationError::Projection)
    }
}
