#![forbid(unsafe_code)]

//! Baozi post-process pipeline contracts.

mod error;
mod operation;
pub mod pipeline;
pub mod preset;

pub use error::{
    PostProcessError, PostProcessFailure, PostProcessFinalizationError,
    PostProcessInitializationError, PostProcessPlanError, PostProcessReport,
    PostProcessRunDisposition, PostProcessRunError, PostProcessRunOutcome, PostProcessStepIssue,
    Result,
};
pub use operation::{
    PostProcessControl, PostProcessDimension, PostProcessLedgerPhase, PostProcessLedgerSnapshot,
    PostProcessLimits, PostProcessProgressPhase, PostProcessProgressUnit,
};
pub use pipeline::{PostProcessPipeline, PostProcessStep};
pub use preset::PostProcessPreset;
