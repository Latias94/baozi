//! Triangle UV Derivative v1 tangent generation using the concrete Scene edit transaction.

mod error;
pub(crate) mod input;
pub(crate) mod kernel;
mod options;
mod report;
mod wide;

#[cfg(test)]
mod runtime_tests;

pub use error::{TangentCause, TangentFailure};
pub use kernel::TangentIssue;
pub use options::{ExistingTangents, TangentOptions};
pub use report::TangentGenerationReport;

pub(crate) const CONVENTIONS: baozi_core::TangentConventions = baozi_core::TangentConventions {
    basis: baozi_core::TangentBasisConvention::TriangleUvDerivative,
    encoding: baozi_core::TangentHandednessEncoding::SignInW,
};

use crate::operation::PostProcessOwner;
use crate::operation::runtime::{REJECTION_CODE, final_progress, plan_runtime};
use crate::scene_edit::{PreparedEdit, execute_changed};
use crate::{
    PostProcessControl, PostProcessReport, PostProcessResourceEvidence, PostProcessRunError,
    PostProcessRunFailure, PostProcessRunOutcome,
};
use baozi_core::__scene_integration::integration::{
    SceneEditBootstrap, SceneEditEvidence, try_start_scene_edit_operation,
};
use baozi_core::{PrimitiveId, ValidatedScene};
use baozi_operation::__retained_store_integration::authority::PreparedStoreEvidenceOnlyOutcome;

/// Generates one dense Corner F32x4 TANGENT on a TriangleList primitive.
///
/// Triangle UV Derivative v1 computes each triangle's UV derivatives, orthogonalizes against
/// the explicitly selected Normal at each Corner, and stores SignInW. It does not weld or average
/// across triangles and does not claim MikkTSpace compatibility. Normal/UV domains and UV set
/// are explicit; the selected Normal is F32x3 and UV is F32x2. No UV flip, texture transform baking
/// or automatic normal generation occurs.
/// Existing Tangents are preserved by default. Replacement of authored values requires exactly
/// one stream; either policy generates a missing Tangent.
/// Degenerate geometry/UV/frame and morph deltas on the generated target reject atomically.
/// Relative frame conditioning must exceed `64 * f32::EPSILON`. W records analytic UV-frame
/// orientation; final F32 rounding does not guarantee a positive bitangent dot product against
/// an arbitrarily ill-conditioned, unprojected V derivative.
/// Ordinary failures return the original immutable Scene with unchanged generation and values.
#[allow(
    clippy::result_large_err,
    reason = "failure returns the original Scene and bounded terminal evidence"
)]
pub fn generate_tangents(
    input: ValidatedScene,
    target: PrimitiveId,
    options: TangentOptions,
    control: PostProcessControl,
) -> Result<PostProcessRunOutcome, PostProcessRunFailure> {
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.postprocess.tangents").entered();
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
                TangentFailure::new(TangentCause::Start { source }),
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
                let report = PostProcessReport::tangents(
                    target,
                    options,
                    input.generation(),
                    resources,
                    None,
                );
                Ok(PostProcessRunOutcome::new(input, report))
            }
            Ok(PreparedStoreEvidenceOnlyOutcome::MinimalFallback(prepared)) => Err(failure(
                input,
                fallback(prepared, TangentCause::Publication),
            )),
            Err(source) => Err(failure(
                input,
                TangentFailure::new(TangentCause::Preparation { source }),
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
                TangentFailure::new(TangentCause::Containment { source }),
            ));
        }
    };
    match execute_changed(
        &input,
        owner,
        admission,
        PreparedEdit::Tangents {
            target,
            options,
            values,
        },
    ) {
        Ok((scene, SceneEditEvidence::Tangents(evidence), resources)) => {
            let tangent_report = TangentGenerationReport::new(evidence);
            let report = PostProcessReport::tangents(
                target,
                options,
                scene.generation(),
                resources,
                Some(tangent_report),
            );
            Ok(PostProcessRunOutcome::new(scene, report))
        }
        Ok(_) => Err(failure(
            input,
            TangentFailure::new(TangentCause::Publication),
        )),
        Err(cause) => Err(failure(input, cause.into())),
    }
}

fn failure(input: ValidatedScene, failure: TangentFailure) -> PostProcessRunFailure {
    PostProcessRunFailure::new(input, PostProcessRunError::Tangents { failure })
}

fn reject(
    operation: crate::operation::runtime::EditOperation,
    cause: TangentCause,
) -> TangentFailure {
    match operation.prepare_minimal_fallback(REJECTION_CODE) {
        Ok(prepared) => fallback(prepared, cause),
        Err(source) => TangentFailure::new(TangentCause::Preparation { source }),
    }
}

fn fallback(
    prepared: baozi_operation::__retained_store_integration::authority::PreparedStoreFallback,
    cause: TangentCause,
) -> TangentFailure {
    let published = prepared.handoff();
    let cause = match published.ledger_evidence.header().terminal_cause() {
        baozi_operation::StoreTerminalCause::Failure(failure)
            if failure.class() == baozi_operation::StorePreparationFailureClass::Control =>
        {
            match cause {
                cause @ TangentCause::Control { .. } => cause,
                _ => TangentCause::FinalControl,
            }
        }
        _ => cause,
    };
    TangentFailure::terminal(
        cause,
        PostProcessResourceEvidence::from_store(published.ledger_evidence),
        crate::direct_error::FailureOwner::Fallback {
            _owner: published.fallback,
        },
    )
}
