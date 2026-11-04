//! Shared publication transaction for the closed set of concrete Scene operations.

#![allow(
    clippy::result_large_err,
    reason = "exact terminal errors retain their bounded evidence owners without a rejection-path allocation"
)]

use crate::direct_error::FailureOwner;
use crate::operation::runtime::{
    EditBoundFinish, EditDisposition, EditFinish, REJECTION_CODE, final_progress,
};
use crate::operation::{PostProcessOwner, PostProcessProgressPhase, PostProcessProgressUnit};
#[cfg(test)]
use crate::prepare::cleanup_tests::{self, Branch, Event};
use crate::{PostProcessControlCause, PostProcessResourceEvidence};
use baozi_core::__scene_integration::integration::{
    ResolvedSceneSpaceConversion, SceneCanonicalRootTag, SceneEditAdmission, SceneEditEvidence,
    SceneEditEvidenceTag, SceneEditGenerationRejection, SceneEditOutput, SceneEditPostflight,
    SceneEditPublication, SceneEditPublicationError, SceneEditRejection, SceneEditTransaction,
    SceneEditValidationProof, SceneSpaceConversionIssue, consume_scene_edit_freeze_permit,
};
use baozi_core::{SceneValidationLimits, ValidatedScene};
use baozi_operation::__retained_store_integration::activation::{
    ActivatedCanonicalStoreSession, CanonicalActivationOutcome, CanonicalActivationSeal,
    CanonicalActivationSlots, CanonicalCandidateStoreSession, CanonicalCentralValidationOutcome,
    CanonicalFinal, CanonicalFreezePermitIssue, CanonicalGenerationSlots,
    CanonicalInvocationOutcome, CanonicalOperationPublication, CanonicalOutputSlot,
    CanonicalProofSlot, CanonicalSemanticWorkError, CanonicalValidationProgress,
    CanonicalValidationSlots, RetainedStoreSessionOutcomeOwner, StoreActivationContainment,
    StoreCanonicalRejection, StoreNeverActivatedCause,
};
use baozi_operation::__retained_store_integration::authority::{
    CanonicalInheritanceSource, PreparedStoreEvidenceOnlyOutcome, PreparedStoreFallback,
};
use baozi_operation::{
    PublishedStoreFallbackParts, RecordBlock, StorePreparationFailureClass, StoreRuntimeError,
    StoreTerminalCause,
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
    pub(crate) fn new(cause: EditCause) -> Self {
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
type EditOwner<'inv> = RetainedStoreSessionOutcomeOwner<
    'inv,
    PostProcessOwner,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    CanonicalInheritanceSource,
>;
type EditActivation<'inv> = CanonicalActivationOutcome<
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
type ActivatedEdit<'inv> = ActivatedCanonicalStoreSession<
    'inv,
    PostProcessOwner,
    PostProcessProgressPhase,
    PostProcessProgressUnit,
    SceneEditOutput,
    RecordBlock,
    SceneCanonicalRootTag,
    CanonicalInheritanceSource,
>;
type CentralEdit<'inv> = CanonicalCentralValidationOutcome<
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
type EditCandidate<'inv> = CanonicalCandidateStoreSession<
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
) -> Result<(), StoreActivationContainment> {
    let mut transaction = data.transaction(input, admission);
    let activated = activate_edit(owner, slots.output, slots.activation, &mut transaction)?;
    let postflight = transaction.finish();
    let activated = match activated {
        CanonicalActivationOutcome::Activated(activated) => activated,
        CanonicalActivationOutcome::Rejected(rejected) => {
            drop(data);
            #[cfg(test)]
            cleanup_tests::record(Event::PreparedEditDropped);
            return rejected.request_finish(disposition);
        }
        CanonicalActivationOutcome::NeverActivated(rejected) => {
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

pub(crate) fn finish_bound_edit(finish: EditBoundFinish<'_, '_>) -> EditResult {
    match finish.into_attached() {
        Ok(finish) => finish_edit(finish),
        Err(finish) => {
            let evidence = finish
                .finish_unattached(final_progress())
                .map_err(|source| EditFailure::new(EditCause::Runtime { source }))?;
            let cause = evidence.header().terminal_cause();
            let resources = PostProcessResourceEvidence::from_store(evidence);
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
    }
}

pub(crate) fn finish_edit(finish: EditFinish<'_>) -> EditResult {
    let completion = finish.resume();
    match completion.outcome {
        CanonicalInvocationOutcome::Operation(operation) => {
            let publication = operation
                .finish(final_progress())
                .map_err(|source| EditFailure::new(EditCause::Preparation { source }))?;
            match publication {
                CanonicalOperationPublication::Unchanged(
                    PreparedStoreEvidenceOnlyOutcome::Success(prepared),
                ) => Ok(EditTerminal::Unchanged {
                    resources: PostProcessResourceEvidence::from_store(
                        prepared.handoff().ledger_evidence,
                    ),
                }),
                CanonicalOperationPublication::Unchanged(
                    PreparedStoreEvidenceOnlyOutcome::MinimalFallback(prepared),
                )
                | CanonicalOperationPublication::MinimalFallback(prepared) => {
                    Err(publish_fallback(prepared, EditCause::Publication))
                }
            }
        }
        CanonicalInvocationOutcome::Candidate(candidate) => {
            #[cfg(test)]
            cleanup_tests::record(Event::Preparing(Branch::Candidate));
            publish_candidate(candidate, completion.generation)
        }
        CanonicalInvocationOutcome::Rejected(rejected) => {
            let cause = core_rejection(rejected.cause());
            #[cfg(test)]
            cleanup_tests::record(Event::Preparing(Branch::Rejected));
            Err(resolve_fallback(rejected.prepare_minimal_fallback(), cause))
        }
        CanonicalInvocationOutcome::NeverActivated(rejected) => {
            let cause = match rejected.cause() {
                StoreNeverActivatedCause::Runtime(cause) => cause.clone().into(),
                StoreNeverActivatedCause::Requested(cause) => core_requested(cause),
            };
            #[cfg(test)]
            cleanup_tests::record(Event::Preparing(Branch::NeverActivated));
            Err(resolve_fallback(rejected.prepare_minimal_fallback(), cause))
        }
        CanonicalInvocationOutcome::CentralRejected(rejected) => {
            let cause = core_rejection(rejected.cause());
            #[cfg(test)]
            cleanup_tests::record(Event::Preparing(Branch::CentralRejected));
            Err(resolve_fallback(rejected.prepare_minimal_fallback(), cause))
        }
        CanonicalInvocationOutcome::Interrupted(interrupted) => {
            let cause = interrupted
                .cause()
                .control_cause()
                .cloned()
                .map_or(EditCause::CentralValidation, EditCause::control);
            #[cfg(test)]
            cleanup_tests::record(Event::Preparing(Branch::Interrupted));
            Err(resolve_fallback(
                interrupted.prepare_minimal_fallback(),
                cause,
            ))
        }
        CanonicalInvocationOutcome::Fallback(fallback) => {
            let cause = fallback.cause().clone().into();
            #[cfg(test)]
            cleanup_tests::record(Event::Preparing(Branch::MinimalFallback));
            Err(resolve_fallback(fallback.prepare_minimal_fallback(), cause))
        }
    }
}

#[inline(never)]
fn activate_edit<'inv>(
    owner: EditOwner<'inv>,
    output_slot: CanonicalOutputSlot<'inv, SceneEditOutput>,
    activation_slots: CanonicalActivationSlots<'inv, SceneEditRejection>,
    transaction: &mut SceneEditTransaction<'_>,
) -> Result<EditActivation<'inv>, StoreActivationContainment> {
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
) -> Result<CentralEdit<'inv>, StoreActivationContainment> {
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
) -> EditResult {
    let freeze = candidate
        .try_issue_freeze_permit(generation_slots.interruption, final_progress())
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
    let published =
        consume_scene_edit_freeze_permit(permit, generation_slots.rejection).map_err(|error| {
            EditFailure::new(match error {
                SceneEditPublicationError::Containment(source) => EditCause::Containment { source },
                SceneEditPublicationError::Preparation(source) => EditCause::Preparation { source },
            })
        })?;
    match published {
        SceneEditPublication::Success(published) => {
            let (scene, evidence, ledger) = published.into_parts();
            #[cfg(test)]
            cleanup_tests::record(Event::Frozen(ledger.header().terminal_cause()));
            let resources = PostProcessResourceEvidence::from_store(ledger);
            Ok(EditTerminal::Changed {
                scene,
                evidence,
                resources,
            })
        }
        SceneEditPublication::MinimalFallback { published } => {
            Err(published_fallback(published, EditCause::Publication))
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
    published_fallback(prepared.handoff(), cause)
}

fn published_fallback(published: PublishedStoreFallbackParts, cause: EditCause) -> EditFailure {
    let PublishedStoreFallbackParts {
        fallback,
        ledger_evidence,
    } = published;
    #[cfg(test)]
    cleanup_tests::record(Event::Frozen(ledger_evidence.header().terminal_cause()));
    let cause = terminal_cause(cause, ledger_evidence.header().terminal_cause());
    EditFailure::terminal(
        cause,
        PostProcessResourceEvidence::from_store(ledger_evidence),
        FailureOwner::Fallback { _owner: fallback },
    )
}
