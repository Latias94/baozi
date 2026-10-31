//! Direct triangulation; postprocess owns the runtime, core owns the Scene edit and publication.

pub(crate) mod input;

use baozi_core::{
    __scene_integration::authoring::OptionsDigest,
    __scene_integration::integration::{
        SceneEditBootstrap, SceneEditEvidence, try_start_scene_edit_operation,
    },
    PrimitiveId, ValidatedScene,
};
use baozi_operation::__retained_store_integration::authority::{
    PreparedStoreEvidenceOnlyOutcome, PreparedStoreFallback,
};
use baozi_operation::{
    PublishedStoreFallbackParts, StorePreparationFailureClass, StoreTerminalCause,
};

use crate::direct_error::FailureOwner;
use crate::operation::PostProcessOwner;
use crate::operation::runtime::{EditOperation, REJECTION_CODE, final_progress, plan_runtime};
use crate::{
    PostProcessControl, PostProcessReport, PostProcessResourceEvidence, PostProcessRunError,
    PostProcessRunFailure, PostProcessRunOutcome, TriangulationCause, TriangulationFailure,
    TriangulationRemap,
};

// SHA-256 of the UTF-8 bytes `baozi.triangulate_primitive/v1;source-fields=preserve`.
// The operation has no caller parameters; this fixed value identifies its explicit preservation rule.
pub(crate) const OPTIONS: OptionsDigest = OptionsDigest::sha256([
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
    match crate::scene_edit::execute_changed(
        &input,
        owner,
        admission,
        crate::scene_edit::PreparedEdit::Triangulation { target, origins },
    ) {
        Ok((scene, SceneEditEvidence::Triangulation(evidence), resources)) => {
            let report = PostProcessReport::triangulation(
                target,
                scene.generation(),
                resources,
                Some(TriangulationRemap::new(evidence)),
            );
            Ok(PostProcessRunOutcome::new(scene, report))
        }
        Ok(_) => Err(failure(
            input,
            TriangulationFailure::new(TriangulationCause::Publication),
        )),
        Err(cause) => Err(failure(input, cause.into())),
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

#[cfg(test)]
mod tests;
