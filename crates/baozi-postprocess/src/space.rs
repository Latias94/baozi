//! Explicit, transactional conversion of Scene coordinate and unit declarations.

mod error;
mod report;
#[cfg(test)]
mod tests;

pub use baozi_core::__scene_integration::integration::SceneSpaceConversionIssue as SpaceConversionIssue;
pub use error::{SpaceConversionCause, SpaceConversionFailure};
pub use report::SpaceConversionReport;

use baozi_core::__scene_integration::integration::{
    SceneEditBootstrap, SceneEditEvidence, resolve_scene_space_conversion, run_scene_edit_body,
};
use baozi_core::{TargetSpace, ValidatedScene};
use baozi_operation::__retained_store_integration::activation::CanonicalBoundBodyError;

use crate::operation::runtime::{REJECTION_CODE, plan_runtime, run_unit};
use crate::scene_edit::{
    EditCanonicalSlots, EditCause, EditFailure, EditTerminal, PreparedEdit, validate_changed,
};
use crate::{
    PostProcessControl, PostProcessReport, PostProcessRunError, PostProcessRunFailure,
    PostProcessRunOutcome,
};

/// Spatial conventions requested for the complete Scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpaceConversionOptions {
    /// Complete spatial target. UV origin and tangent-generation conventions are preserved.
    pub target: TargetSpace,
    /// Explicit source facts used only where the Scene declares an unknown value. Conflicting
    /// known facts reject the operation. The report records which fields were actually filled.
    pub source_assumptions: Option<TargetSpace>,
}

impl Default for SpaceConversionOptions {
    fn default() -> Self {
        Self {
            target: TargetSpace::STANDARD,
            source_assumptions: None,
        }
    }
}

/// Converts a Scene with explicit source facts, retaining the immutable input on ordinary failure.
///
/// Positions use a signed-axis permutation and a positive uniform unit ratio. Nodes retain their
/// authored Matrix/TRS form. Existing normals retain their physical direction, tangents transport
/// their direction and handedness, and triangle winding changes together with Corner attributes.
/// Skin matrices use conjugation; morph deltas and animation values and cubic derivatives follow
/// their Position, direction, scale, or quaternion semantics without normalization.
/// Unhandled present spatial semantics return a typed capability error before publication.
/// This operation does not change UV origin, generate tangents, or infer missing source units.
#[allow(
    clippy::result_large_err,
    reason = "failure returns the original Scene and terminal evidence"
)]
pub fn convert_scene_space(
    input: ValidatedScene,
    options: SpaceConversionOptions,
    control: PostProcessControl,
) -> Result<PostProcessRunOutcome, PostProcessRunFailure> {
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.postprocess.convert_space").entered();
    let bootstrap = match SceneEditBootstrap::try_new() {
        Ok(value) => value,
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
    let mut invocation_host = crate::operation::runtime::EditHost::new();
    let source_generation = input.generation();
    let mut body_cause = None;
    let mut body_containment = None;
    let source = &input;
    let local_cause = &mut body_cause;
    let containment = &mut body_containment;
    let mut activation = Some((admission, failure_requirements));
    let body = run_scene_edit_body(
        &mut invocation_host,
        source,
        control.into_runtime(),
        ledger,
        start,
        move |parts, disposition| {
            let mut startup = parts.startup;
            let resolved = match startup.with_action_execution(keys.work, |runtime| {
                run_unit(runtime, || {
                    resolve_scene_space_conversion(
                        source,
                        options.target,
                        options.source_assumptions,
                    )
                })
            }) {
                Ok(Ok(resolved)) => resolved,
                Ok(Err(source)) => {
                    *local_cause = Some(SpaceConversionCause::Source { source });
                    if let Err(source) = startup.request_failure(disposition, REJECTION_CODE) {
                        *local_cause = Some(source.into());
                    }
                    return;
                }
                Err(source) => {
                    *local_cause = Some(source.into());
                    if let Err(source) = startup.request_failure(disposition, REJECTION_CODE) {
                        *local_cause = Some(source.into());
                    }
                    return;
                }
            };
            if resolved.is_unchanged() {
                if let Err(source) = startup.request_unchanged(disposition) {
                    *local_cause = Some(source.into());
                }
                return;
            }
            let operation = match startup.attach() {
                Ok(operation) => operation,
                Err(source) => {
                    *local_cause = Some(SpaceConversionCause::Start { source });
                    return;
                }
            };
            let (admission, failure_requirements) = activation
                .take()
                .expect("the retained edit body activates at most once");
            let owner = match operation.into_session_outcome_owner(
                keys.session_policy(),
                writer_limits,
                failure_requirements,
            ) {
                Ok(owner) => owner,
                Err(source) => {
                    *containment = Some(source);
                    return;
                }
            };
            *containment = validate_changed(
                source,
                owner,
                EditCanonicalSlots {
                    output: parts.output,
                    proof: parts.proof,
                    activation: parts.activation,
                    validation: parts.validation,
                },
                admission,
                PreparedEdit::Space { resolved },
                disposition,
            )
            .err();
        },
    );
    let result = match body {
        Ok(finish) => crate::scene_edit::finish_bound_edit(finish),
        Err(CanonicalBoundBodyError::Start(source)) => {
            return Err(failure(
                input,
                SpaceConversionFailure::new(SpaceConversionCause::Start { source }),
            ));
        }
        Err(CanonicalBoundBodyError::Body(error)) => {
            if let Some(source) = error.containment().copied().or(body_containment) {
                Err(EditFailure::new(EditCause::Containment { source }))
            } else {
                return Err(failure(
                    input,
                    SpaceConversionFailure::new(
                        body_cause.unwrap_or(SpaceConversionCause::Publication),
                    ),
                ));
            }
        }
    };
    match result {
        Ok(EditTerminal::Unchanged { resources }) => {
            let report = PostProcessReport::space_conversion(
                source_generation,
                options,
                input.generation(),
                resources,
                None,
            );
            Ok(PostProcessRunOutcome::new(input, report))
        }
        Ok(EditTerminal::Changed {
            scene,
            evidence: SceneEditEvidence::SpaceConversion(evidence),
            resources,
        }) => {
            let effects = SpaceConversionReport::new(evidence);
            let report = PostProcessReport::space_conversion(
                source_generation,
                options,
                scene.generation(),
                resources,
                Some(effects),
            );
            Ok(PostProcessRunOutcome::new(scene, report))
        }
        Ok(_) => Err(failure(
            input,
            SpaceConversionFailure::new(SpaceConversionCause::Publication),
        )),
        Err(error) => {
            let error = match body_cause {
                Some(cause) => local_failure(cause, error),
                None => error.into(),
            };
            Err(failure(input, error))
        }
    }
}

fn failure(input: ValidatedScene, failure: SpaceConversionFailure) -> PostProcessRunFailure {
    PostProcessRunFailure::new(input, PostProcessRunError::SpaceConversion { failure })
}

fn local_failure(cause: SpaceConversionCause, error: EditFailure) -> SpaceConversionFailure {
    let EditFailure {
        cause: terminal_cause,
        resources,
        owner,
    } = error;
    let cause = match terminal_cause {
        EditCause::Publication => cause,
        EditCause::FinalControl => match cause {
            cause @ SpaceConversionCause::Control { .. } => cause,
            _ => SpaceConversionCause::FinalControl,
        },
        cause => {
            return EditFailure {
                cause,
                resources,
                owner,
            }
            .into();
        }
    };
    match resources {
        Some(resources) => SpaceConversionFailure::terminal(cause, resources, owner),
        None => SpaceConversionFailure::new(cause),
    }
}
