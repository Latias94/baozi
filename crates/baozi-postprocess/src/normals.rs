//! Flat and smooth normal generation using the concrete Scene edit transaction.

mod error;
pub(crate) mod input;
mod kernel;
mod options;
mod report;

#[cfg(test)]
mod runtime_tests;

pub use error::{NormalCause, NormalFailure, NormalIssue};
pub use options::{ExistingNormals, NormalMode, NormalOptions, TangentHandling};
pub use report::NormalGenerationReport;

use crate::operation::PostProcessOwner;
use crate::operation::runtime::{REJECTION_CODE, final_progress, plan_runtime};
use crate::scene_edit::{PreparedEdit, execute_changed};
use crate::{
    PostProcessControl, PostProcessReport, PostProcessResourceEvidence, PostProcessRunError,
    PostProcessRunFailure, PostProcessRunOutcome,
};
use baozi_core::scene::integration::{
    SceneEditBootstrap, SceneEditEvidence, try_start_scene_edit_operation,
};
use baozi_core::{PrimitiveId, ValidatedScene};
use baozi_operation::__retained_store_integration::authority::PreparedStoreEvidenceOnlyOutcome;

/// Generates one Corner NORMAL field on a TriangleList primitive.
///
/// Oriented face crosses use `(b - a) × (c - a)` in the stored numeric basis; Scene front-face
/// conventions are not inferred. Smooth normals use shared Vertex identity
/// and Face smoothing groups with area weights; group zero is a hard edge. Coincident positions
/// are not welded and UV seams do not split a shared vertex's smoothing bucket. Degenerate faces
/// and cancelling smooth sums reject the operation. Existing authored normals are preserved by
/// default. Replacing them or removing invalidated tangents requires the corresponding option.
/// Ordinary failures return the exact immutable input Scene.
/// Multiple authored NORMAL or TANGENT domains are explicitly rejected when generation would
/// need to choose or merge them. The operation does not infer a metadata or field-selection policy.
#[allow(
    clippy::result_large_err,
    reason = "failure returns the original Scene and bounded terminal evidence"
)]
pub fn generate_normals(
    input: ValidatedScene,
    target: PrimitiveId,
    options: NormalOptions,
    control: PostProcessControl,
) -> Result<PostProcessRunOutcome, PostProcessRunFailure> {
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.postprocess.normals").entered();
    let bootstrap = match SceneEditBootstrap::try_new() {
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
                NormalFailure::new(NormalCause::Start { source }),
            ));
        }
    };
    let source = match input::inspect(&input, target, options, &mut operation, keys) {
        Ok(source) => source,
        Err(cause) => return Err(failure(input, reject(operation, cause.into()))),
    };
    let Some(source) = source else {
        return match operation.prepare_evidence_only_success(final_progress()) {
            Ok(PreparedStoreEvidenceOnlyOutcome::Success(prepared)) => {
                let resources =
                    PostProcessResourceEvidence::from_store(prepared.handoff().ledger_evidence);
                let report = PostProcessReport::normals(
                    target,
                    options,
                    input.generation(),
                    resources,
                    None,
                );
                Ok(PostProcessRunOutcome::new(input, report))
            }
            Ok(PreparedStoreEvidenceOnlyOutcome::MinimalFallback(prepared)) => {
                Err(failure(input, fallback(prepared, NormalCause::Publication)))
            }
            Err(source) => Err(failure(
                input,
                NormalFailure::new(NormalCause::Preparation { source }),
            )),
        };
    };
    let values = match input::generate(source, &mut operation, keys) {
        Ok(values) => values,
        Err(cause) => return Err(failure(input, reject(operation, cause.into()))),
    };
    let owner = match operation.into_session_outcome_owner(
        keys.session_policy(),
        writer_limits,
        failure_requirements,
    ) {
        Ok(owner) => owner,
        Err(source) => {
            return Err(failure(
                input,
                NormalFailure::new(NormalCause::Containment { source }),
            ));
        }
    };
    match execute_changed(
        &input,
        owner,
        admission,
        PreparedEdit::Normals {
            target,
            options,
            values,
        },
    ) {
        Ok((scene, SceneEditEvidence::Normals(evidence), resources)) => {
            let normal_report = NormalGenerationReport::new(evidence);
            let report = PostProcessReport::normals(
                target,
                options,
                scene.generation(),
                resources,
                Some(normal_report),
            );
            Ok(PostProcessRunOutcome::new(scene, report))
        }
        Ok(_) => Err(failure(input, NormalFailure::new(NormalCause::Publication))),
        Err(cause) => Err(failure(input, cause.into())),
    }
}

fn failure(input: ValidatedScene, failure: NormalFailure) -> PostProcessRunFailure {
    PostProcessRunFailure::new(input, PostProcessRunError::Normals { failure })
}

fn reject(
    operation: crate::operation::runtime::EditOperation,
    cause: NormalCause,
) -> NormalFailure {
    match operation.prepare_minimal_fallback(REJECTION_CODE) {
        Ok(prepared) => fallback(prepared, cause),
        Err(source) => NormalFailure::new(NormalCause::Preparation { source }),
    }
}

fn fallback(
    prepared: baozi_operation::__retained_store_integration::authority::PreparedStoreFallback,
    cause: NormalCause,
) -> NormalFailure {
    let published = prepared.handoff();
    let cause = match published.ledger_evidence.header().terminal_cause() {
        baozi_operation::StoreTerminalCause::Failure(failure)
            if failure.class() == baozi_operation::StorePreparationFailureClass::Control =>
        {
            match cause {
                cause @ NormalCause::Control { .. } => cause,
                _ => NormalCause::FinalControl,
            }
        }
        _ => cause,
    };
    NormalFailure::terminal(
        cause,
        PostProcessResourceEvidence::from_store(published.ledger_evidence),
        crate::direct_error::FailureOwner::Fallback {
            _owner: published.fallback,
        },
    )
}
