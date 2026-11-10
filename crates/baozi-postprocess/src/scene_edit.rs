//! Shared publication transaction for the closed set of concrete Scene operations.

#![allow(
    clippy::result_large_err,
    reason = "exact terminal errors retain their bounded evidence owners without a rejection-path allocation"
)]

use crate::direct_error::FailureOwner;
use crate::operation::runtime::{
    EditBoundFinish, EditCompletion, EditDisposition, EditInvocationError, REJECTION_CODE,
    final_progress,
};
use crate::operation::{PostProcessOwner, PostProcessProgressPhase, PostProcessProgressUnit};
#[cfg(test)]
use crate::prepare::cleanup_tests::{self, Branch, Event};
use crate::{PostProcessControlCause, PostProcessResourceEvidence};
use baozi_core::__scene_integration::integration::{
    PreparedFiniteSceneEditPublication, ResolvedSceneSpaceConversion, SceneCanonicalRootTag,
    SceneEditAdmission, SceneEditEvidence, SceneEditEvidenceTag, SceneEditGenerationRejection,
    SceneEditOutput, SceneEditPostflight, SceneEditRejection, SceneEditTransaction,
    SceneEditValidationProof, SceneSpaceConversionIssue, consume_finite_scene_edit_freeze_permit,
};
use baozi_core::{SceneValidationLimits, ValidatedScene};
use baozi_operation::__retained_store_integration::activation::{
    CanonicalActivationSeal, CanonicalActivationSlots, CanonicalFinal, CanonicalGenerationSlots,
    CanonicalOutputSlot, CanonicalProofSlot, CanonicalSemanticWorkError,
    CanonicalValidationProgress, CanonicalValidationSlots, FiniteActivatedSession,
    FiniteCanonicalActivationOutcome, FiniteCanonicalCentralValidationOutcome,
    FiniteCanonicalFreezePermitIssue, FiniteCanonicalInvocationOutcome,
    FiniteCanonicalOperationPublication, FiniteCentralCandidate, FiniteSessionOutcomeOwner,
    StoreCanonicalRejection, StoreNeverActivatedCause,
};
use baozi_operation::__retained_store_integration::authority::{
    CanonicalInheritanceSource, PreparedFiniteStoreEvidenceOnlyOutcome, PreparedFiniteStoreFallback,
};
use baozi_operation::{
    PublishedFiniteStoreFallbackParts, RecordBlock, StorePreparationFailureClass,
    StoreRuntimeError, StoreTerminalCause,
};

#[allow(
    clippy::large_enum_variant,
    reason = "the selection owns its metered scratch inline and releases it before terminal preparation"
)]
pub(crate) enum PreparedEdit<'a> {
    Selection {
        replacements:
            &'a [baozi_core::__scene_integration::integration::SceneGeometryReplacement<'a>],
        mappings: baozi_core::__scene_integration::integration::SceneGeometryPlanStorage<
            PostProcessOwner,
        >,
        digest: baozi_core::__scene_integration::authoring::OptionsDigest,
        limits: SceneValidationLimits,
    },
    Space {
        resolved: ResolvedSceneSpaceConversion<'a>,
    },
}

impl PreparedEdit<'_> {
    fn transaction<'a>(
        &'a mut self,
        input: &'a ValidatedScene,
        admission: SceneEditAdmission,
    ) -> baozi_core::__scene_integration::integration::SceneEditTransaction<'a> {
        match self {
            Self::Selection {
                replacements,
                mappings,
                digest,
                limits,
            } => admission.into_geometry_transaction(
                input,
                replacements,
                mappings.scratch(),
                *digest,
                *limits,
            ),
            Self::Space { resolved } => {
                admission.into_space_transaction(*resolved, SceneValidationLimits::default())
            }
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
    PrecisionLoss {
        semantic: baozi_core::AttributeSemanticKind,
    },
    MorphDependency {
        semantic: baozi_core::MorphDeltaSemantic,
    },
    SpaceIssue {
        issue: SceneSpaceConversionIssue,
    },
    CentralValidation,
    Publication,
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

#[expect(
    clippy::large_enum_variant,
    reason = "semantic rejection retains its exact facts and physical owner inline; invocation ownership stays in the entry"
)]
pub(crate) enum EditFailure {
    Invocation,
    Semantic(EditSemanticFailure),
}

pub(crate) struct EditSemanticFailure {
    pub(crate) cause: EditCause,
    pub(crate) resources: Option<PostProcessResourceEvidence>,
    pub(crate) owner: FailureOwner,
}
impl EditFailure {
    // Finalization borrows this entry-owned slot only after Host body teardown has completed.
    pub(crate) fn invocation(
        error: EditInvocationError,
        destination: &mut Option<EditInvocationError>,
    ) -> Self {
        assert!(
            destination.is_none(),
            "one original invocation error per entry"
        );
        *destination = Some(error);
        Self::Invocation
    }

    fn terminal(
        cause: EditCause,
        resources: PostProcessResourceEvidence,
        owner: FailureOwner,
    ) -> Self {
        Self::Semantic(EditSemanticFailure {
            cause,
            resources: Some(resources),
            owner,
        })
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "terminal alternatives transfer their exact Scene and evidence owners without allocation"
)]
pub(crate) enum EditTerminal {
    Unchanged {
        resources: PostProcessResourceEvidence,
    },
    Changed {
        scene: ValidatedScene,
        evidence: SceneEditEvidence,
        resources: PostProcessResourceEvidence,
    },
}

type EditResult = Result<EditTerminal, EditFailure>;

// Construct an owning Operation result in this frame, then leave its error in the entry.
fn finish_step<T>(
    complete: impl FnOnce() -> Result<T, EditInvocationError>,
    invocation_error: &mut Option<EditInvocationError>,
) -> Result<T, EditFailure> {
    match complete() {
        Ok(value) => Ok(value),
        Err(error) => Err(EditFailure::invocation(error, invocation_error)),
    }
}

type EditOwner<'inv> = FiniteSessionOutcomeOwner<
    'inv,
    PostProcessOwner,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    CanonicalInheritanceSource,
>;
type EditActivation<'inv> = FiniteCanonicalActivationOutcome<
    'inv,
    PostProcessOwner,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    SceneEditOutput,
    SceneEditRejection,
    RecordBlock,
    SceneCanonicalRootTag,
    CanonicalInheritanceSource,
>;
type ActivatedEdit<'inv> = FiniteActivatedSession<
    'inv,
    PostProcessOwner,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    SceneEditOutput,
    RecordBlock,
    SceneCanonicalRootTag,
    CanonicalInheritanceSource,
>;
type CentralEdit<'inv> = FiniteCanonicalCentralValidationOutcome<
    'inv,
    PostProcessOwner,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    SceneEditOutput,
    SceneEditValidationProof,
    SceneEditRejection,
    RecordBlock,
    SceneCanonicalRootTag,
    RecordBlock,
    SceneEditEvidenceTag,
>;
type EditCandidate<'inv> = FiniteCentralCandidate<
    'inv,
    PostProcessOwner,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    SceneEditOutput,
    SceneEditValidationProof,
    RecordBlock,
    SceneCanonicalRootTag,
    RecordBlock,
    SceneEditEvidenceTag,
>;

pub(crate) struct EditCanonicalSlots<'inv> {
    pub(crate) output: CanonicalOutputSlot<'inv, SceneEditOutput>,
    pub(crate) proof: CanonicalProofSlot<'inv, SceneEditValidationProof>,
    pub(crate) activation: CanonicalActivationSlots<'inv, SceneEditRejection>,
    pub(crate) validation: CanonicalValidationSlots<'inv, SceneEditRejection>,
}

pub(crate) fn validate_changed<'inv>(
    input: &ValidatedScene,
    owner: EditOwner<'inv>,
    slots: EditCanonicalSlots<'inv>,
    admission: SceneEditAdmission,
    mut data: PreparedEdit<'_>,
    disposition: EditDisposition<'inv>,
) -> Result<(), EditInvocationError> {
    let mut transaction = data.transaction(input, admission);
    let activated = activate_edit(owner, slots.output, slots.activation, &mut transaction)?;
    let postflight = transaction.finish();
    let activated = match activated {
        FiniteCanonicalActivationOutcome::Activated(activated) => activated,
        FiniteCanonicalActivationOutcome::Rejected(rejected) => {
            drop(data);
            #[cfg(test)]
            cleanup_tests::record(Event::PreparedEditDropped);
            return rejected.request_finish(disposition);
        }
        FiniteCanonicalActivationOutcome::NeverActivated(rejected) => {
            drop(data);
            #[cfg(test)]
            cleanup_tests::record(Event::PreparedEditDropped);
            return rejected.request_finish(disposition);
        }
    };
    let central = validate_edit(activated, slots.proof, slots.validation, postflight)?;
    drop(data);
    #[cfg(test)]
    cleanup_tests::record(Event::PreparedEditDropped);
    central.request_finish(disposition)
}

pub(crate) fn finish_bound_edit(
    finish: EditBoundFinish<'_, '_>,
    did_attach: bool,
    invocation_error: &mut Option<EditInvocationError>,
) -> EditResult {
    if did_attach {
        let completion = finish_step(|| finish.resume(), invocation_error)?;
        return finish_edit(completion, invocation_error);
    }
    let facts = finish_step(
        || finish.finish_unattached(final_progress()),
        invocation_error,
    )?;
    let cause = facts.header().terminal_cause();
    let resources = PostProcessResourceEvidence::from_facts(facts);
    match cause {
        StoreTerminalCause::Success => Ok(EditTerminal::Unchanged { resources }),
        StoreTerminalCause::Failure(cause) => Err(EditFailure::terminal(
            if cause.class() == StorePreparationFailureClass::Control {
                EditCause::FinalControl
            } else {
                EditCause::Publication
            },
            resources,
            FailureOwner::None,
        )),
    }
}

fn finish_edit(
    completion: EditCompletion<'_>,
    invocation_error: &mut Option<EditInvocationError>,
) -> EditResult {
    match completion.outcome {
        FiniteCanonicalInvocationOutcome::Operation(operation) => {
            let publication = finish_step(|| operation.finish(final_progress()), invocation_error)?;
            match publication {
                FiniteCanonicalOperationPublication::EvidenceOnly(
                    PreparedFiniteStoreEvidenceOnlyOutcome::Success(prepared),
                ) => Ok(EditTerminal::Unchanged {
                    resources: {
                        let parts = prepared.handoff();
                        PostProcessResourceEvidence::from_evidence_only(
                            parts.facts,
                            parts.terminal_account,
                        )
                    },
                }),
                FiniteCanonicalOperationPublication::EvidenceOnly(
                    PreparedFiniteStoreEvidenceOnlyOutcome::MinimalFallback(prepared),
                )
                | FiniteCanonicalOperationPublication::MinimalFallback(prepared) => {
                    Err(publish_fallback(prepared, EditCause::Publication))
                }
            }
        }
        FiniteCanonicalInvocationOutcome::Candidate(candidate) => {
            #[cfg(test)]
            cleanup_tests::record(Event::Preparing(Branch::Candidate));
            publish_candidate(candidate, completion.generation, invocation_error)
        }
        FiniteCanonicalInvocationOutcome::Rejected(rejected) => {
            let cause = core_rejection(rejected.cause());
            #[cfg(test)]
            cleanup_tests::record(Event::Preparing(Branch::Rejected));
            Err(resolve_fallback(
                || rejected.prepare_minimal_fallback(),
                cause,
                invocation_error,
            ))
        }
        FiniteCanonicalInvocationOutcome::NeverActivated(rejected) => {
            let cause = match rejected.cause() {
                StoreNeverActivatedCause::Runtime(cause) => cause.clone().into(),
                StoreNeverActivatedCause::Requested(cause) => core_requested(cause),
            };
            #[cfg(test)]
            cleanup_tests::record(Event::Preparing(Branch::NeverActivated));
            Err(resolve_fallback(
                || rejected.prepare_minimal_fallback(),
                cause,
                invocation_error,
            ))
        }
        FiniteCanonicalInvocationOutcome::CentralRejected(rejected) => {
            let cause = core_rejection(rejected.cause());
            #[cfg(test)]
            cleanup_tests::record(Event::Preparing(Branch::CentralRejected));
            Err(resolve_fallback(
                || rejected.prepare_minimal_fallback(),
                cause,
                invocation_error,
            ))
        }
        FiniteCanonicalInvocationOutcome::Interrupted(interrupted) => {
            let cause = interrupted
                .cause()
                .control_cause()
                .cloned()
                .map_or(EditCause::CentralValidation, EditCause::control);
            #[cfg(test)]
            cleanup_tests::record(Event::Preparing(Branch::Interrupted));
            Err(resolve_fallback(
                || interrupted.prepare_minimal_fallback(),
                cause,
                invocation_error,
            ))
        }
        FiniteCanonicalInvocationOutcome::Fallback(fallback) => {
            let cause = fallback.cause().clone().into();
            #[cfg(test)]
            cleanup_tests::record(Event::Preparing(Branch::MinimalFallback));
            Err(resolve_fallback(
                || fallback.prepare_minimal_fallback(),
                cause,
                invocation_error,
            ))
        }
    }
}

#[inline(never)]
fn activate_edit<'inv>(
    owner: EditOwner<'inv>,
    output_slot: CanonicalOutputSlot<'inv, SceneEditOutput>,
    activation_slots: CanonicalActivationSlots<'inv, SceneEditRejection>,
    transaction: &mut SceneEditTransaction<'_>,
) -> Result<EditActivation<'inv>, EditInvocationError> {
    owner.plan_and_activate_scoped(output_slot, activation_slots, |mut planning| {
        let ready = planning
            .with_semantic_work(|semantic| transaction.preflight(semantic))
            .map_err(|cause| match cause {
                CanonicalSemanticWorkError::Runtime(cause) => SceneEditRejection::Runtime(cause),
                CanonicalSemanticWorkError::Rejected(cause) => cause,
            });
        let (plan, completion) = match ready {
            Ok(ready) => ready,
            Err(cause) => return planning.reject(cause, REJECTION_CODE),
        };
        #[cfg(test)]
        cleanup_tests::record(Event::Planned);
        match planning.seal_activation(plan, completion) {
            CanonicalActivationSeal::Sealed(permit) => permit.activate(|lane, plan| {
                #[cfg(test)]
                cleanup_tests::record(Event::Materializing);
                plan.materialize(lane)
            }),
            CanonicalActivationSeal::Unsealed(failure) => failure.finish(),
        }
    })
}

#[inline(never)]
fn validate_edit<'inv>(
    activated: ActivatedEdit<'inv>,
    proof_slot: CanonicalProofSlot<'inv, SceneEditValidationProof>,
    validation_slots: CanonicalValidationSlots<'inv, SceneEditRejection>,
    postflight: SceneEditPostflight<'_>,
) -> Result<CentralEdit<'inv>, EditInvocationError> {
    activated.validate_centrally(
        proof_slot,
        validation_slots,
        CanonicalValidationProgress::new(
            PostProcessProgressPhase::Validation,
            PostProcessProgressUnit::WorkItems,
        ),
        |lane, output| {
            #[cfg(test)]
            cleanup_tests::record(Event::BuildingEvidence);
            postflight.build_evidence(lane, output)
        },
        |planning, output| {
            #[cfg(test)]
            cleanup_tests::record(Event::Validating);
            postflight.validate_candidate(planning, output)
        },
    )
}

#[inline(never)]
fn publish_candidate<'inv>(
    candidate: CanonicalFinal<EditCandidate<'inv>>,
    generation_slots: CanonicalGenerationSlots<'inv, SceneEditGenerationRejection>,
    invocation_error: &mut Option<EditInvocationError>,
) -> EditResult {
    let freeze = finish_step(
        || candidate.try_issue_freeze_permit(generation_slots.interruption, final_progress()),
        invocation_error,
    )?;
    let permit = match freeze {
        FiniteCanonicalFreezePermitIssue::Ready(permit) => permit,
        FiniteCanonicalFreezePermitIssue::Interrupted(rooted) => {
            let cause = rooted
                .cause()
                .control_cause()
                .cloned()
                .map_or(EditCause::Publication, EditCause::control);
            return Err(resolve_fallback(
                || rooted.prepare_minimal_fallback(),
                cause,
                invocation_error,
            ));
        }
        FiniteCanonicalFreezePermitIssue::DestinationRejected(rooted) => {
            let cause = rooted
                .cause()
                .control_cause()
                .cloned()
                .map_or(EditCause::Publication, EditCause::control);
            return Err(resolve_fallback(
                || rooted.prepare_minimal_fallback(),
                cause,
                invocation_error,
            ));
        }
    };
    let prepared = finish_step(
        || consume_finite_scene_edit_freeze_permit(permit, generation_slots.rejection),
        invocation_error,
    )?;
    match prepared {
        PreparedFiniteSceneEditPublication::Success(prepared) => {
            let (scene, evidence, facts) = prepared.handoff().into_parts();
            #[cfg(test)]
            cleanup_tests::record(Event::Frozen(facts.header().terminal_cause()));
            let resources = PostProcessResourceEvidence::from_facts(facts);
            Ok(EditTerminal::Changed {
                scene,
                evidence,
                resources,
            })
        }
        PreparedFiniteSceneEditPublication::MinimalFallback { store } => {
            Err(publish_fallback(store, EditCause::Publication))
        }
    }
}

fn core_requested(cause: &SceneEditRejection) -> EditCause {
    match cause {
        SceneEditRejection::Runtime(cause) => cause.clone().into(),
        SceneEditRejection::PrecisionLoss { semantic } => EditCause::PrecisionLoss {
            semantic: *semantic,
        },
        SceneEditRejection::Space(issue) => EditCause::SpaceIssue { issue: *issue },
        SceneEditRejection::MorphDependency { semantic } => EditCause::MorphDependency {
            semantic: *semantic,
        },
        _ => EditCause::CoreRejected,
    }
}

fn core_rejection(cause: &StoreCanonicalRejection<SceneEditRejection>) -> EditCause {
    match cause {
        StoreCanonicalRejection::Runtime(cause) => cause.clone().into(),
        StoreCanonicalRejection::Requested(cause) => core_requested(cause),
    }
}

fn resolve_fallback<C>(
    prepare: impl FnOnce() -> Result<
        baozi_operation::__retained_store_integration::activation::VerifiedMinimalFallback<
            C,
            PreparedFiniteStoreFallback,
        >,
        EditInvocationError,
    >,
    cause: EditCause,
    invocation_error: &mut Option<EditInvocationError>,
) -> EditFailure {
    match prepare() {
        Ok(fallback) => {
            let (prepared, _) = fallback.into_parts();
            publish_fallback(prepared, cause)
        }
        Err(source) => EditFailure::invocation(source, invocation_error),
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

fn publish_fallback(prepared: PreparedFiniteStoreFallback, cause: EditCause) -> EditFailure {
    published_fallback(prepared.handoff(), cause)
}

fn published_fallback(
    published: PublishedFiniteStoreFallbackParts,
    cause: EditCause,
) -> EditFailure {
    let PublishedFiniteStoreFallbackParts {
        fallback,
        facts,
        terminal_account,
    } = published;
    #[cfg(test)]
    cleanup_tests::record(Event::Frozen(facts.header().terminal_cause()));
    let cause = match facts.original_freeze_control() {
        Some(control) => EditCause::control(control.clone()),
        None => terminal_cause(cause, facts.header().terminal_cause()),
    };
    EditFailure::terminal(
        cause,
        PostProcessResourceEvidence::from_facts(facts),
        FailureOwner::Fallback {
            _owner: fallback,
            _terminal_account: terminal_account,
        },
    )
}
