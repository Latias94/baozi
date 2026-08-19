use std::cell::{RefCell, RefMut};

use crate::SceneDraft;
use baozi_core::scene::integration::{
    SceneActivationOutput, SceneActivationRejection, SceneCanonicalRootTag, SceneConstruction,
    SceneEvidenceRecipe, SceneHandoffToken, SceneOutcomeEvidenceTag, SceneValidationProof,
    consume_scene_freeze_permit,
};
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalCandidateStoreSession,
    CanonicalCentralValidationOutcome, CanonicalFreezePermitIssue, CanonicalValidationProgress,
    CentrallyRejectedCanonicalStoreSession, FailureEvidenceOutcome, FrozenFailureState,
    InterruptedCanonicalStoreSession, NeverActivatedStoreSession, RejectedCanonicalStoreSession,
    RootedFailureValidation, StoreCanonicalRejection, StoreNeverActivatedCause,
    VerifiedMinimalFallback,
};
use baozi_operation::{
    ControlCause, FinalProgress, PreparedStoreFailure, PreparedStoreFailureOutcome,
    PreparedStoreFallback, PreparedStoreSuccess, PreparedStoreSuccessOutcome, RecordBlock,
    StoreRuntimeError,
};

use crate::ledger::{ImportOperationOwner, ImportProgressPhase, ImportProgressUnit};

use super::{
    ImportStoreActivationError, ImportStoreFailureCauseAuthority, ImportStoreFinalizationError,
    ImportStoreHostError, ImportStoreMinimalFallback, ImportStoreOutcomeOwner,
    ImportStoreRichFailure, ImportStoreSessionOwner, ImportStoreSessionState,
    ImportStoreSuccessReport, ImportStoreTerminalCause, ImportStoreTerminalClass,
    ImportStoreTerminalEvidence, ImportStoreTerminalOutcome, ImportStoreTerminalProjectionError,
};

pub(super) const IMPORT_SCENE_NOT_SUBMITTED_CODE: u32 = 0x494d_0002;

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

fn never_activated_control(
    cause: &StoreNeverActivatedCause<SceneActivationRejection>,
) -> Option<ControlCause> {
    match cause {
        StoreNeverActivatedCause::Runtime(cause) => runtime_control(cause),
        StoreNeverActivatedCause::Requested(_) => None,
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
    control_cause: Option<ControlCause>,
) -> Result<SceneDraft, ImportStoreActivationError> {
    if let Some(cause) = control_cause {
        Err(ImportStoreActivationError::Control(cause))
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
                let evidence = ImportStoreTerminalEvidence::try_from_ledger(&ledger_evidence)?;
                if evidence.cause() != ImportStoreTerminalCause::Success {
                    return Err(ImportStoreTerminalProjectionError::TerminalOwnerCauseMismatch);
                }
                Ok(ImportStoreTerminalOutcome::Success {
                    scene,
                    report: ImportStoreSuccessReport {
                        _outcome_evidence: outcome_evidence,
                        _ledger_evidence: ledger_evidence,
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
                    ImportStoreTerminalEvidence::try_from_ledger(&published.ledger_evidence)?;
                if evidence.cause() == ImportStoreTerminalCause::Success
                    || (control_cause.is_some()
                        && evidence.cause().class() != ImportStoreTerminalClass::Control)
                {
                    return Err(ImportStoreTerminalProjectionError::TerminalOwnerCauseMismatch);
                }
                Ok(ImportStoreTerminalOutcome::RichFailure {
                    failure: ImportStoreRichFailure {
                        _outcome_evidence: published.outcome_evidence,
                        _ledger_evidence: published.ledger_evidence,
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
                    ImportStoreTerminalEvidence::try_from_ledger(&published.ledger_evidence)?;
                if evidence.cause() == ImportStoreTerminalCause::Success
                    || (control_cause.is_some()
                        && evidence.cause().class() != ImportStoreTerminalClass::Control)
                {
                    return Err(ImportStoreTerminalProjectionError::TerminalOwnerCauseMismatch);
                }
                Ok(ImportStoreTerminalOutcome::MinimalFallback {
                    fallback: ImportStoreMinimalFallback {
                        _fallback: published.fallback,
                        _ledger_evidence: published.ledger_evidence,
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
            ImportStoreSessionState::Dormant(owner) => owner,
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
            ImportStoreSessionState::Pending(ImportPendingScene::PreparedFallback {
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
        &self,
        construction: SceneConstruction<'input>,
    ) -> Result<SceneDraft, ImportStoreActivationError> {
        let (owner, transition) = ImportStoreActivationTransition::begin(&self.state)
            .map_err(ImportStoreActivationError::StoreUnavailable)?;
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
                transition.contain();
                return Err(ImportStoreActivationError::Containment(cause));
            }
        };
        let postflight = transaction
            .take()
            .expect("the scoped Scene transaction retains its postflight recipe")
            .finish();
        let (pending, result) = match outcome {
            CanonicalActivationOutcome::Activated(activated) => {
                let output = *activated.output();
                let evidence = postflight.candidate_evidence(&output);
                let central = match activated.validate_centrally(
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
                ) {
                    Ok(outcome) => outcome,
                    Err(cause) => {
                        transition.contain();
                        return Err(ImportStoreActivationError::Containment(cause));
                    }
                };
                match central {
                    CanonicalCentralValidationOutcome::Candidate(owner) => {
                        let draft = owner.output().draft();
                        (ImportPendingScene::Candidate { owner, evidence }, Ok(draft))
                    }
                    CanonicalCentralValidationOutcome::Rejected(owner) => {
                        let control_cause = canonical_rejection_control(owner.cause());
                        let result = activation_failure_result(false, control_cause);
                        (
                            ImportPendingScene::CentralRejected { owner, evidence },
                            result,
                        )
                    }
                    CanonicalCentralValidationOutcome::Interrupted(owner) => {
                        let control_cause = owner.cause().control_cause().cloned();
                        let result = activation_failure_result(false, control_cause);
                        (
                            ImportPendingScene::CentralInterrupted { owner, evidence },
                            result,
                        )
                    }
                    CanonicalCentralValidationOutcome::Fallback(owner) => {
                        let control_cause = runtime_control(owner.cause());
                        let result = activation_failure_result(false, control_cause);
                        (
                            ImportPendingScene::PreparedFallback {
                                owner,
                                completed_boundary: Some(ImportStoreSceneBoundary::DraftBuilt),
                            },
                            result,
                        )
                    }
                }
            }
            CanonicalActivationOutcome::Rejected(owner) => {
                let authoring_rejected = canonical_rejection_is_authoring(owner.cause());
                let control_cause = canonical_rejection_control(owner.cause());
                let result = activation_failure_result(authoring_rejected, control_cause);
                (
                    ImportPendingScene::ActivationRejected {
                        owner,
                        evidence: postflight.failure_evidence(),
                    },
                    result,
                )
            }
            CanonicalActivationOutcome::NeverActivated(owner) => {
                let authoring_rejected = never_activated_is_authoring(owner.cause());
                let control_cause = never_activated_control(owner.cause());
                let result = activation_failure_result(authoring_rejected, control_cause);
                (
                    ImportPendingScene::NeverActivated {
                        owner,
                        evidence: postflight.failure_evidence(),
                    },
                    result,
                )
            }
            CanonicalActivationOutcome::Fallback(owner) => {
                let control_cause = runtime_control(owner.cause());
                let result = activation_failure_result(false, control_cause);
                (
                    ImportPendingScene::PreparedFallback {
                        owner,
                        completed_boundary: None,
                    },
                    result,
                )
            }
        };
        transition.install(pending);
        result
    }

    pub(super) fn prepare_scene(
        self,
        draft: Option<SceneDraft>,
    ) -> Result<ImportStorePreparedScene, ImportStoreActivationError> {
        let unsubmitted_evidence = self.unsubmitted_evidence;
        match self.state.into_inner() {
            ImportStoreSessionState::Pending(ImportPendingScene::Candidate { owner, evidence }) => {
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
                                prepare_frozen_failure(
                                    validation,
                                    control_cause,
                                    rejection_authority,
                                )
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
                                        cause_authority:
                                            ImportStoreFailureCauseAuthority::OperationTerminal,
                                    })
                                }
                                PreparedStoreSuccessOutcome::MinimalFallback(store) => {
                                    drop(token);
                                    Ok(ImportStorePreparedScene::MinimalFallback {
                                        store,
                                        control_cause: None,
                                        cause_authority:
                                            ImportStoreFailureCauseAuthority::OperationTerminal,
                                    })
                                }
                            },
                        )
                        .map_err(ImportStoreActivationError::Containment)?
                    }
                    CanonicalFreezePermitIssue::Interrupted(rooted) => {
                        let control_cause = rooted.cause().control_cause().cloned();
                        let validation = rooted
                            .validate_rich_failure(|session, root| {
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
                            .validate_rich_failure(|session, root| {
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
            ImportStoreSessionState::Pending(ImportPendingScene::ActivationRejected {
                owner,
                evidence,
            }) => {
                let control_cause = canonical_rejection_control(owner.cause());
                let outcome = owner
                    .build_failure_evidence(|lane| {
                        match lane.try_retain_records(
                            evidence.record_plan(),
                            SceneOutcomeEvidenceTag,
                            |writer| writer.write_row(|row| evidence.write_row(row)),
                        ) {
                            Ok(root) => lane.complete(root),
                            Err(_) => lane.abort(),
                        }
                    })
                    .map_err(ImportStoreActivationError::Containment)?;
                prepare_failure_outcome(
                    outcome,
                    evidence,
                    scene_final_progress(),
                    control_cause,
                    ImportStoreFailureCauseAuthority::OperationTerminal,
                )
            }
            ImportStoreSessionState::Pending(ImportPendingScene::NeverActivated {
                owner,
                evidence,
            }) => {
                let control_cause = never_activated_control(owner.cause());
                let outcome = owner
                    .build_failure_evidence(|lane| {
                        match lane.try_retain_records(
                            evidence.record_plan(),
                            SceneOutcomeEvidenceTag,
                            |writer| writer.write_row(|row| evidence.write_row(row)),
                        ) {
                            Ok(root) => lane.complete(root),
                            Err(_) => lane.abort(),
                        }
                    })
                    .map_err(ImportStoreActivationError::Containment)?;
                prepare_failure_outcome(
                    outcome,
                    evidence,
                    scene_final_progress(),
                    control_cause,
                    ImportStoreFailureCauseAuthority::OperationTerminal,
                )
            }
            ImportStoreSessionState::Pending(ImportPendingScene::CentralRejected {
                owner,
                evidence,
            }) => {
                let control_cause = canonical_rejection_control(owner.cause());
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
            ImportStoreSessionState::Pending(ImportPendingScene::CentralInterrupted {
                owner,
                evidence,
            }) => {
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
                                    |writer| {
                                        writer.write_row(|row| unsubmitted_evidence.write_row(row))
                                    },
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
                                    |writer| {
                                        writer.write_row(|row| unsubmitted_evidence.write_row(row))
                                    },
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
