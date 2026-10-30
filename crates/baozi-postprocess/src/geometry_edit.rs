//! Shared publication transaction for the closed set of concrete geometry operations.

use crate::direct_error::FailureOwner;
use crate::operation::runtime::{REJECTION_CODE, final_progress};
use crate::operation::{PostProcessOwner, PostProcessProgressPhase, PostProcessProgressUnit};
use crate::{PostProcessControlCause, PostProcessResourceEvidence};
use baozi_core::scene::integration::{
    SceneGeometryEditAdmission, SceneGeometryEditEvidence, SceneGeometryEditRejection,
    SceneNormalReplacement, SceneTriangulationReplacement, consume_scene_edit_freeze_permit,
};
use baozi_core::{PrimitiveId, SceneValidationLimits, ValidatedScene};
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalCentralValidationOutcome,
    CanonicalFreezePermitIssue, CanonicalValidationProgress, StoreCanonicalRejection,
    StoreNeverActivatedCause,
};
use baozi_operation::__retained_store_integration::authority::{
    PreparedStoreFailure, PreparedStoreFallback, PreparedStoreSuccessOutcome,
};
use baozi_operation::{
    PublishedStoreFallbackParts, RecordBlock, StorePreparationFailureClass, StoreRuntimeError,
    StoreTerminalCause,
};

pub(crate) enum PreparedGeometry {
    Triangulation {
        target: PrimitiveId,
        origins: crate::triangulate::input::Origins,
    },
    Normals {
        target: PrimitiveId,
        options: crate::NormalOptions,
        values: crate::normals::input::GeneratedNormals,
    },
}

impl PreparedGeometry {
    fn transaction<'a>(
        &'a self,
        input: &'a ValidatedScene,
        admission: SceneGeometryEditAdmission,
    ) -> baozi_core::scene::integration::SceneGeometryEditTransaction<'a> {
        match self {
            Self::Triangulation { target, origins } => admission.into_transaction(
                input,
                SceneTriangulationReplacement::new(
                    *target,
                    origins.triangles.values(),
                    origins.face_offsets.values(),
                    crate::triangulate::OPTIONS,
                ),
                SceneValidationLimits::default(),
            ),
            Self::Normals {
                target,
                options,
                values,
            } => admission.into_normal_transaction(
                input,
                SceneNormalReplacement::new(
                    *target,
                    values.values(),
                    values.had_normal(),
                    options.tangents == crate::TangentHandling::Remove,
                    options.mode == crate::NormalMode::Smooth,
                    options.digest(),
                ),
                SceneValidationLimits::default(),
            ),
        }
    }
}

#[derive(Debug)]
pub(crate) enum EditCause {
    Runtime {
        source: StoreRuntimeError,
    },
    Control {
        cause: PostProcessControlCause,
    },
    FinalControl,
    CoreRejected,
    CentralValidation,
    Publication,
    Containment {
        source:
            baozi_operation::__retained_store_integration::activation::StoreActivationContainment,
    },
    Preparation {
        source: baozi_operation::StorePrepareError,
    },
}

impl EditCause {
    fn control(cause: baozi_operation::ControlCause) -> Self {
        Self::Control {
            cause: crate::error::project_control_cause(cause),
        }
    }
}
impl From<StoreRuntimeError> for EditCause {
    fn from(source: StoreRuntimeError) -> Self {
        match source {
            StoreRuntimeError::Control(cause) => Self::control(cause),
            source => Self::Runtime { source },
        }
    }
}

pub(crate) struct EditFailure {
    pub(crate) cause: EditCause,
    pub(crate) resources: Option<PostProcessResourceEvidence>,
    pub(crate) owner: FailureOwner,
}
impl EditFailure {
    fn new(cause: EditCause) -> Self {
        Self {
            cause,
            resources: None,
            owner: FailureOwner::None,
        }
    }
    fn terminal(
        cause: EditCause,
        resources: PostProcessResourceEvidence,
        owner: FailureOwner,
    ) -> Self {
        Self {
            cause,
            resources: Some(resources),
            owner,
        }
    }
}

#[allow(
    clippy::result_large_err,
    reason = "all exact terminal owners remain inline"
)]
pub(crate) fn execute_changed(
    input: &ValidatedScene,
    owner: baozi_operation::__retained_store_integration::activation::RetainedStoreSessionOutcomeOwner<
        PostProcessOwner, PostProcessProgressPhase, PostProcessProgressUnit,
        baozi_operation::__retained_store_integration::authority::CanonicalInheritanceSource,
    >,
    admission: SceneGeometryEditAdmission,
    data: PreparedGeometry,
) -> Result<
    (
        ValidatedScene,
        SceneGeometryEditEvidence,
        PostProcessResourceEvidence,
    ),
    EditFailure,
> {
    let mut transaction = data.transaction(input, admission);
    let activated = owner
        .plan_and_activate_scoped(|mut planning| {
            let ready = match planning.begin_semantic_work() {
                Ok(mut semantic) => match transaction.preflight(&mut semantic) {
                    Ok(plan) => match semantic.finish_exact() {
                        Ok(completion) => Ok((plan, completion)),
                        Err(cause) => {
                            drop(plan);
                            Err(SceneGeometryEditRejection::Runtime(cause))
                        }
                    },
                    Err(cause) => {
                        let _ = semantic.finish_rejected();
                        Err(cause)
                    }
                },
                Err(cause) => Err(SceneGeometryEditRejection::Runtime(cause)),
            };
            let (plan, completion) = match ready {
                Ok(ready) => ready,
                Err(cause) => return planning.reject(cause, REJECTION_CODE),
            };
            match planning.seal_activation(plan, completion) {
                CanonicalActivationSeal::Sealed(permit) => {
                    permit.activate(|lane, plan| plan.materialize(lane))
                }
                CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
            }
        })
        .map_err(|source| EditFailure::new(EditCause::Containment { source }))?;
    let postflight = transaction.finish();
    let activated = match activated {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(rejected) => {
            let cause = core_rejection(rejected.cause());
            drop(data);
            return Err(resolve_fallback(rejected.prepare_minimal_fallback(), cause));
        }
        CanonicalActivationOutcome::NeverActivated(rejected) => {
            let cause = match rejected.cause() {
                StoreNeverActivatedCause::Runtime(cause) => cause.clone().into(),
                StoreNeverActivatedCause::Requested(cause) => core_requested(cause),
            };
            drop(data);
            return Err(resolve_fallback(rejected.prepare_minimal_fallback(), cause));
        }
        CanonicalActivationOutcome::Fallback(fallback) => {
            let cause = fallback.cause().clone().into();
            let (prepared, _) = fallback.into_parts();
            drop(data);
            return Err(publish_fallback(prepared, cause));
        }
    };
    let central = activated
        .validate_centrally(
            CanonicalValidationProgress::new(
                PostProcessProgressPhase::Validation,
                PostProcessProgressUnit::WorkItems,
            ),
            |lane, output| postflight.build_evidence(lane, output),
            |planning, output| postflight.validate_candidate(planning, output),
        )
        .map_err(|source| EditFailure::new(EditCause::Containment { source }))?;
    // Retained origins in central evidence now own the complete remap. Temporary leases end here.
    drop(data);
    let candidate = match central {
        CanonicalCentralValidationOutcome::Candidate(candidate) => candidate,
        CanonicalCentralValidationOutcome::Rejected(rejected) => {
            let cause = core_rejection(rejected.cause());
            return Err(resolve_fallback(rejected.prepare_minimal_fallback(), cause));
        }
        CanonicalCentralValidationOutcome::Interrupted(interrupted) => {
            let cause = interrupted
                .cause()
                .control_cause()
                .cloned()
                .map_or(EditCause::CentralValidation, EditCause::control);
            return Err(resolve_fallback(
                interrupted.prepare_minimal_fallback(),
                cause,
            ));
        }
        CanonicalCentralValidationOutcome::Fallback(fallback) => {
            let cause = fallback.cause().clone().into();
            return Err(resolve_fallback(fallback.prepare_minimal_fallback(), cause));
        }
    };
    let freeze = candidate
        .try_issue_freeze_permit(final_progress())
        .map_err(|source| EditFailure::new(EditCause::Containment { source }))?;
    let permit = match freeze {
        CanonicalFreezePermitIssue::Ready(permit) => permit,
        CanonicalFreezePermitIssue::Interrupted(rooted) => {
            let cause = rooted
                .cause()
                .control_cause()
                .cloned()
                .map_or(EditCause::Publication, EditCause::control);
            return Err(resolve_fallback(rooted.prepare_minimal_fallback(), cause));
        }
        CanonicalFreezePermitIssue::DestinationRejected(rooted) => {
            let cause = rooted
                .cause()
                .control_cause()
                .cloned()
                .map_or(EditCause::Publication, EditCause::control);
            return Err(resolve_fallback(rooted.prepare_minimal_fallback(), cause));
        }
    };
    consume_scene_edit_freeze_permit(
        permit,
        |rejected| {
            Err(resolve_fallback(
                rejected.prepare_minimal_fallback(),
                EditCause::Publication,
            ))
        },
        |token, joint, seal| match joint
            .prepare(seal)
            .map_err(|source| EditFailure::new(EditCause::Containment { source }))?
        {
            PreparedStoreSuccessOutcome::Success(store) => {
                let published = token
                    .handoff(store)
                    .map_err(|_| EditFailure::new(EditCause::Publication))?;
                let (scene, evidence, ledger) = published.into_parts();
                let resources = PostProcessResourceEvidence::from_store(ledger);
                Ok((scene, evidence, resources))
            }
            PreparedStoreSuccessOutcome::RichFailure(store) => {
                drop(token);
                Err(publish_rich_failure(store, EditCause::Publication))
            }
            PreparedStoreSuccessOutcome::MinimalFallback(store) => {
                drop(token);
                Err(publish_fallback(store, EditCause::Publication))
            }
        },
    )
    .map_err(|source| EditFailure::new(EditCause::Containment { source }))?
}

fn core_requested(cause: &SceneGeometryEditRejection) -> EditCause {
    match cause {
        SceneGeometryEditRejection::Runtime(cause) => cause.clone().into(),
        _ => EditCause::CoreRejected,
    }
}

fn core_rejection(cause: &StoreCanonicalRejection<SceneGeometryEditRejection>) -> EditCause {
    match cause {
        StoreCanonicalRejection::Runtime(cause) => cause.clone().into(),
        StoreCanonicalRejection::Requested(cause) => core_requested(cause),
    }
}

fn resolve_fallback<C>(
    result: Result<
        baozi_operation::__retained_store_integration::activation::VerifiedMinimalFallback<C>,
        baozi_operation::StorePrepareError,
    >,
    cause: EditCause,
) -> EditFailure {
    match result {
        Ok(fallback) => {
            let (prepared, _) = fallback.into_parts();
            publish_fallback(prepared, cause)
        }
        Err(source) => EditFailure::new(EditCause::Preparation { source }),
    }
}

fn terminal_cause(cause: EditCause, terminal: StoreTerminalCause) -> EditCause {
    match terminal {
        StoreTerminalCause::Failure(failure)
            if failure.class() == StorePreparationFailureClass::Control =>
        {
            match cause {
                cause @ EditCause::Control { .. } => cause,
                _ => EditCause::FinalControl,
            }
        }
        _ => cause,
    }
}

fn publish_fallback(prepared: PreparedStoreFallback, cause: EditCause) -> EditFailure {
    let PublishedStoreFallbackParts {
        fallback,
        ledger_evidence,
    } = prepared.handoff();
    let cause = terminal_cause(cause, ledger_evidence.header().terminal_cause());
    EditFailure::terminal(
        cause,
        PostProcessResourceEvidence::from_store(ledger_evidence),
        FailureOwner::Fallback { _owner: fallback },
    )
}

fn publish_rich_failure(
    prepared: PreparedStoreFailure<
        RecordBlock,
        baozi_core::scene::integration::SceneGeometryEditEvidenceTag,
    >,
    cause: EditCause,
) -> EditFailure {
    let published = prepared.handoff();
    let cause = terminal_cause(cause, published.ledger_evidence.header().terminal_cause());
    EditFailure::terminal(
        cause,
        PostProcessResourceEvidence::from_store(published.ledger_evidence),
        FailureOwner::Rich {
            _owner: published.outcome_evidence,
        },
    )
}
