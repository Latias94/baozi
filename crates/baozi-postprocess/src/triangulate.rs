//! Direct triangulation; postprocess owns the runtime, core owns the Scene edit and publication.

mod input;

use baozi_core::{
    PrimitiveId, SceneValidationLimits, ValidatedScene,
    scene::authoring::OptionsDigest,
    scene::integration::{
        SceneGeometryEditAdmission, SceneGeometryEditBootstrap, SceneGeometryEditRejection,
        SceneTriangulationReplacement, consume_scene_edit_freeze_permit,
        try_start_scene_edit_operation,
    },
};
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationOutcome, CanonicalActivationSeal, CanonicalCentralValidationOutcome,
    CanonicalFreezePermitIssue, CanonicalValidationProgress, StoreCanonicalRejection,
    StoreNeverActivatedCause,
};
use baozi_operation::__retained_store_integration::authority::{
    PreparedStoreEvidenceOnlyOutcome, PreparedStoreFailure, PreparedStoreFallback,
    PreparedStoreSuccessOutcome,
};
use baozi_operation::{
    PublishedStoreFallbackParts, RecordBlock, StorePreparationFailureClass, StoreTerminalCause,
};

use crate::direct_error::FailureOwner;
use crate::operation::runtime::{EditOperation, REJECTION_CODE, final_progress, plan_runtime};
use crate::operation::{PostProcessOwner, PostProcessProgressPhase, PostProcessProgressUnit};
use crate::{
    PostProcessControl, PostProcessReport, PostProcessResourceEvidence, PostProcessRunError,
    PostProcessRunFailure, PostProcessRunOutcome, TriangulationCause, TriangulationFailure,
    TriangulationRemap,
};

// SHA-256 of the UTF-8 bytes `baozi.triangulate_primitive/v1;source-fields=preserve`.
// The operation has no caller parameters; this fixed value identifies its explicit preservation rule.
const OPTIONS: OptionsDigest = OptionsDigest::sha256([
    0x82, 0x4c, 0xac, 0x97, 0x28, 0x91, 0xba, 0xe3, 0x59, 0x64, 0x5b, 0x14, 0x26, 0x6d, 0x48, 0x55,
    0x74, 0xfb, 0x24, 0xd1, 0x7a, 0xf1, 0x1d, 0x1f, 0x28, 0x58, 0x7e, 0x12, 0x2c, 0x89, 0x6f, 0xa0,
]);

/// Triangulates one polygon primitive through a same-account, centrally validated Scene edit.
///
/// Every face must be a simple planar ring without holes. Uncertain or malformed geometry is
/// rejected. Source winding, Vertex identity, Corner/Face values, scalar bits and sparse defaults
/// are preserved. Authored normals/tangents follow their domain origins; they are not recomputed.
/// Triangle lists and empty polygon primitives return the original Scene as `Unchanged`.
///
/// Any ordinary rejection, cancellation, startup failure or containment returns the original
/// immutable Scene in the error. The changed report owns the actual retained origin relation.
#[allow(
    clippy::result_large_err,
    reason = "failure returns the original linear Scene and bounded evidence"
)]
pub fn triangulate_primitive(
    input: ValidatedScene,
    target: PrimitiveId,
    control: PostProcessControl,
) -> Result<PostProcessRunOutcome, PostProcessRunFailure> {
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.postprocess.triangulate").entered();

    let bootstrap = match SceneGeometryEditBootstrap::try_new() {
        Ok(bootstrap) => bootstrap,
        Err(_) => {
            return Err(PostProcessRunFailure::new(
                input,
                PostProcessRunError::ResourcePolicyInitialization,
            ));
        }
    };
    let (writer_limits, failure_requirements, admission) = bootstrap.into_parts();
    let Some((keys, ledger, start)) = plan_runtime(writer_limits) else {
        return Err(PostProcessRunFailure::new(
            input,
            PostProcessRunError::ResourcePolicyInitialization,
        ));
    };
    // No target inspection, source sharing, scratch, or kernel work precedes this exact-input start.
    let mut operation = match try_start_scene_edit_operation::<PostProcessOwner, _, _>(
        &input,
        control.into_runtime(),
        ledger,
        start,
    ) {
        Ok(operation) => operation,
        Err(source) => {
            return Err(failure(
                input,
                TriangulationFailure::new(TriangulationCause::Start { source }),
            ));
        }
    };
    let source = match input::inspect(&input, target, &mut operation, keys) {
        Ok(source) => source,
        Err(cause) => return Err(failure(input, reject_operation(operation, cause))),
    };
    let Some(source) = source else {
        return match operation.prepare_evidence_only_success(final_progress()) {
            Ok(PreparedStoreEvidenceOnlyOutcome::Success(prepared)) => {
                let resources =
                    PostProcessResourceEvidence::from_store(prepared.handoff().ledger_evidence);
                let report =
                    PostProcessReport::triangulation(target, input.generation(), resources, None);
                Ok(PostProcessRunOutcome::new(input, report))
            }
            Ok(PreparedStoreEvidenceOnlyOutcome::MinimalFallback(prepared)) => Err(failure(
                input,
                publish_fallback(prepared, TriangulationCause::Publication),
            )),
            Err(source) => Err(failure(
                input,
                TriangulationFailure::new(TriangulationCause::Preparation { source }),
            )),
        };
    };
    let origins = match input::build_origins(source, &mut operation, keys) {
        Ok(origins) => origins,
        Err(cause) => return Err(failure(input, reject_operation(operation, cause))),
    };
    let owner = match operation.into_session_outcome_owner(
        keys.session_policy(),
        writer_limits,
        failure_requirements,
    ) {
        Ok(owner) => owner,
        Err(source) => {
            drop(origins);
            return Err(failure(
                input,
                TriangulationFailure::new(TriangulationCause::Containment { source }),
            ));
        }
    };
    match execute_changed(&input, target, owner, admission, origins) {
        Ok((scene, report)) => Ok(PostProcessRunOutcome::new(scene, report)),
        Err(cause) => Err(failure(input, cause)),
    }
}

#[allow(
    clippy::result_large_err,
    reason = "all exact terminal owners remain inline"
)]
fn execute_changed(
    input: &ValidatedScene,
    target: PrimitiveId,
    owner: baozi_operation::__retained_store_integration::activation::RetainedStoreSessionOutcomeOwner<
        PostProcessOwner, PostProcessProgressPhase, PostProcessProgressUnit,
        baozi_operation::__retained_store_integration::authority::CanonicalInheritanceSource,
    >,
    admission: SceneGeometryEditAdmission,
    origins: input::Origins,
) -> Result<(ValidatedScene, PostProcessReport), TriangulationFailure> {
    let replacement = SceneTriangulationReplacement::new(
        target,
        origins.triangles.values(),
        origins.face_offsets.values(),
        OPTIONS,
    );
    let mut transaction =
        admission.into_transaction(input, replacement, SceneValidationLimits::default());
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
        .map_err(|source| TriangulationFailure::new(TriangulationCause::Containment { source }))?;
    let postflight = transaction.finish();
    let activated = match activated {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(rejected) => {
            let cause = core_rejection(rejected.cause());
            drop(origins);
            return Err(resolve_fallback(rejected.prepare_minimal_fallback(), cause));
        }
        CanonicalActivationOutcome::NeverActivated(rejected) => {
            let cause = match rejected.cause() {
                StoreNeverActivatedCause::Runtime(cause) => cause.clone().into(),
                StoreNeverActivatedCause::Requested(cause) => core_requested(cause),
            };
            drop(origins);
            return Err(resolve_fallback(rejected.prepare_minimal_fallback(), cause));
        }
        CanonicalActivationOutcome::Fallback(fallback) => {
            let cause = fallback.cause().clone().into();
            let (prepared, _) = fallback.into_parts();
            drop(origins);
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
        .map_err(|source| TriangulationFailure::new(TriangulationCause::Containment { source }))?;
    let source_corners = origins.source_corners;
    let vertices = origins.vertices;
    // Retained origins in central evidence now own the complete remap. Temporary leases end here.
    drop(origins);
    let candidate = match central {
        CanonicalCentralValidationOutcome::Candidate(candidate) => candidate,
        CanonicalCentralValidationOutcome::Rejected(rejected) => {
            let cause = core_rejection(rejected.cause());
            return Err(resolve_fallback(rejected.prepare_minimal_fallback(), cause));
        }
        CanonicalCentralValidationOutcome::Interrupted(interrupted) => {
            let cause = interrupted.cause().control_cause().cloned().map_or(
                TriangulationCause::CentralValidation,
                TriangulationCause::control,
            );
            return Err(resolve_fallback(
                interrupted.prepare_minimal_fallback(),
                cause,
            ));
        }
        CanonicalCentralValidationOutcome::Fallback(fallback) => {
            let cause = fallback.cause().clone().into();
            let (prepared, _) = fallback.into_parts();
            return Err(publish_fallback(prepared, cause));
        }
    };
    let freeze = candidate
        .try_issue_freeze_permit(final_progress())
        .map_err(|source| TriangulationFailure::new(TriangulationCause::Containment { source }))?;
    let permit = match freeze {
        CanonicalFreezePermitIssue::Ready(permit) => permit,
        CanonicalFreezePermitIssue::Interrupted(rooted) => {
            let cause = rooted
                .cause()
                .control_cause()
                .cloned()
                .map_or(TriangulationCause::Publication, TriangulationCause::control);
            return Err(resolve_fallback(rooted.prepare_minimal_fallback(), cause));
        }
        CanonicalFreezePermitIssue::DestinationRejected(rooted) => {
            let cause = rooted
                .cause()
                .control_cause()
                .cloned()
                .map_or(TriangulationCause::Publication, TriangulationCause::control);
            return Err(resolve_fallback(rooted.prepare_minimal_fallback(), cause));
        }
    };
    consume_scene_edit_freeze_permit(
        permit,
        |rejected| {
            Err(resolve_fallback(
                rejected.prepare_minimal_fallback(),
                TriangulationCause::Publication,
            ))
        },
        |token, joint, seal| match joint.prepare(seal).map_err(|source| {
            TriangulationFailure::new(TriangulationCause::Containment { source })
        })? {
            PreparedStoreSuccessOutcome::Success(store) => {
                let published = token
                    .handoff(store)
                    .map_err(|_| TriangulationFailure::new(TriangulationCause::Publication))?;
                let (scene, evidence, ledger) = published.into_parts();
                let resources = PostProcessResourceEvidence::from_store(ledger);
                let remap = TriangulationRemap::new(evidence, source_corners, vertices);
                let report = PostProcessReport::triangulation(
                    target,
                    scene.generation(),
                    resources,
                    Some(remap),
                );
                Ok((scene, report))
            }
            PreparedStoreSuccessOutcome::RichFailure(store) => {
                drop(token);
                Err(publish_rich_failure(store, TriangulationCause::Publication))
            }
            PreparedStoreSuccessOutcome::MinimalFallback(store) => {
                drop(token);
                Err(publish_fallback(store, TriangulationCause::Publication))
            }
        },
    )
    .map_err(|source| TriangulationFailure::new(TriangulationCause::Containment { source }))?
}

fn core_requested(cause: &SceneGeometryEditRejection) -> TriangulationCause {
    match cause {
        SceneGeometryEditRejection::Runtime(cause) => cause.clone().into(),
        _ => TriangulationCause::CoreRejected,
    }
}

fn core_rejection(
    cause: &StoreCanonicalRejection<SceneGeometryEditRejection>,
) -> TriangulationCause {
    match cause {
        StoreCanonicalRejection::Runtime(cause) => cause.clone().into(),
        StoreCanonicalRejection::Requested(cause) => core_requested(cause),
    }
}

fn failure(input: ValidatedScene, failure: TriangulationFailure) -> PostProcessRunFailure {
    PostProcessRunFailure::new(input, PostProcessRunError::Triangulation { failure })
}

fn reject_operation(operation: EditOperation, cause: TriangulationCause) -> TriangulationFailure {
    match operation.prepare_minimal_fallback(REJECTION_CODE) {
        Ok(prepared) => publish_fallback(prepared, cause),
        Err(source) => TriangulationFailure::new(TriangulationCause::Preparation { source }),
    }
}

fn resolve_fallback<C>(
    result: Result<
        baozi_operation::__retained_store_integration::activation::VerifiedMinimalFallback<C>,
        baozi_operation::StorePrepareError,
    >,
    cause: TriangulationCause,
) -> TriangulationFailure {
    match result {
        Ok(fallback) => {
            let (prepared, _) = fallback.into_parts();
            publish_fallback(prepared, cause)
        }
        Err(source) => TriangulationFailure::new(TriangulationCause::Preparation { source }),
    }
}

fn terminal_cause(cause: TriangulationCause, terminal: StoreTerminalCause) -> TriangulationCause {
    match terminal {
        StoreTerminalCause::Failure(failure)
            if failure.class() == StorePreparationFailureClass::Control =>
        {
            match cause {
                cause @ TriangulationCause::Control { .. } => cause,
                _ => TriangulationCause::FinalControl,
            }
        }
        _ => cause,
    }
}

fn publish_fallback(
    prepared: PreparedStoreFallback,
    cause: TriangulationCause,
) -> TriangulationFailure {
    let PublishedStoreFallbackParts {
        fallback,
        ledger_evidence,
    } = prepared.handoff();
    let cause = terminal_cause(cause, ledger_evidence.header().terminal_cause());
    TriangulationFailure::terminal(
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
    cause: TriangulationCause,
) -> TriangulationFailure {
    let published = prepared.handoff();
    let cause = terminal_cause(cause, published.ledger_evidence.header().terminal_cause());
    TriangulationFailure::terminal(
        cause,
        PostProcessResourceEvidence::from_store(published.ledger_evidence),
        FailureOwner::Rich {
            _owner: published.outcome_evidence,
        },
    )
}

#[cfg(test)]
mod tests;
