//! Explicit, transactional conversion of Scene coordinate and unit declarations.

mod error;
mod report;
#[cfg(test)]
mod tests;

pub use baozi_core::scene::integration::SceneSpaceConversionIssue as SpaceConversionIssue;
pub use error::{SpaceConversionCause, SpaceConversionFailure};
pub use report::SpaceConversionReport;

use baozi_core::scene::integration::{
    SceneEditBootstrap, SceneEditEvidence, resolve_scene_space_conversion,
    try_start_scene_edit_operation,
};
use baozi_core::{TargetSpace, ValidatedScene};
use baozi_operation::__retained_store_integration::authority::PreparedStoreEvidenceOnlyOutcome;

use crate::operation::PostProcessOwner;
use crate::operation::runtime::{REJECTION_CODE, final_progress, plan_runtime, run_unit};
use crate::scene_edit::{PreparedEdit, execute_changed};
use crate::{
    PostProcessControl, PostProcessReport, PostProcessResourceEvidence, PostProcessRunError,
    PostProcessRunFailure, PostProcessRunOutcome,
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
                SpaceConversionFailure::new(SpaceConversionCause::Start { source }),
            ));
        }
    };
    let resolved = match run_unit(&mut operation, keys, || {
        resolve_scene_space_conversion(&input, options.target, options.source_assumptions)
    }) {
        Ok(Ok(resolved)) => resolved,
        Ok(Err(source)) => {
            return Err(failure(
                input,
                reject(operation, SpaceConversionCause::Source { source }),
            ));
        }
        Err(source) => return Err(failure(input, reject(operation, source.into()))),
    };
    if resolved.is_unchanged() {
        return match operation.prepare_evidence_only_success(final_progress()) {
            Ok(PreparedStoreEvidenceOnlyOutcome::Success(prepared)) => {
                let resources =
                    PostProcessResourceEvidence::from_store(prepared.handoff().ledger_evidence);
                let report = PostProcessReport::space_conversion(
                    input.generation(),
                    options,
                    input.generation(),
                    resources,
                    None,
                );
                Ok(PostProcessRunOutcome::new(input, report))
            }
            Ok(PreparedStoreEvidenceOnlyOutcome::MinimalFallback(prepared)) => Err(failure(
                input,
                publish_fallback(prepared, SpaceConversionCause::Publication),
            )),
            Err(source) => Err(failure(
                input,
                SpaceConversionFailure::new(SpaceConversionCause::Preparation { source }),
            )),
        };
    }
    let owner = match operation.into_session_outcome_owner(
        keys.session_policy(),
        writer_limits,
        failure_requirements,
    ) {
        Ok(owner) => owner,
        Err(source) => {
            return Err(failure(
                input,
                SpaceConversionFailure::new(SpaceConversionCause::Containment { source }),
            ));
        }
    };
    let source_generation = input.generation();
    match execute_changed(&input, owner, admission, PreparedEdit::Space { resolved }) {
        Ok((scene, SceneEditEvidence::SpaceConversion(evidence), resources)) => {
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
        Err(cause) => Err(failure(input, cause.into())),
    }
}

fn failure(input: ValidatedScene, failure: SpaceConversionFailure) -> PostProcessRunFailure {
    PostProcessRunFailure::new(input, PostProcessRunError::SpaceConversion { failure })
}

fn reject(
    operation: crate::operation::runtime::EditOperation,
    cause: SpaceConversionCause,
) -> SpaceConversionFailure {
    match operation.prepare_minimal_fallback(REJECTION_CODE) {
        Ok(prepared) => publish_fallback(prepared, cause),
        Err(source) => SpaceConversionFailure::new(SpaceConversionCause::Preparation { source }),
    }
}

fn publish_fallback(
    prepared: baozi_operation::__retained_store_integration::authority::PreparedStoreFallback,
    cause: SpaceConversionCause,
) -> SpaceConversionFailure {
    let published = prepared.handoff();
    let cause = match published.ledger_evidence.header().terminal_cause() {
        baozi_operation::StoreTerminalCause::Failure(failure)
            if failure.class() == baozi_operation::StorePreparationFailureClass::Control =>
        {
            match cause {
                cause @ SpaceConversionCause::Control { .. } => cause,
                _ => SpaceConversionCause::FinalControl,
            }
        }
        _ => cause,
    };
    SpaceConversionFailure::terminal(
        cause,
        PostProcessResourceEvidence::from_store(published.ledger_evidence),
        crate::direct_error::FailureOwner::Fallback {
            _owner: published.fallback,
        },
    )
}
