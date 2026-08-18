#![forbid(unsafe_code)]

//! Baozi post-process pipeline contracts.

mod error;
mod operation;
mod pipeline;

pub use baozi_core::SceneGenerationId;

pub use error::{
    PostProcessCancellationOrigin, PostProcessCompileFailure, PostProcessControlCause,
    PostProcessDiagnostic, PostProcessError, PostProcessExecutionFailure, PostProcessFailure,
    PostProcessFinalizationError, PostProcessInitializationError, PostProcessLoss,
    PostProcessReport, PostProcessRunDisposition, PostProcessRunError, PostProcessRunOutcome,
    PostProcessStepOutcome,
};
pub use operation::{
    PostProcessCancellationToken, PostProcessControl, PostProcessResourceEvidence,
};
pub use pipeline::{
    CompiledPostProcessPlan, CompiledPostProcessRun, CompiledPostProcessStep, PostProcessRequests,
};
