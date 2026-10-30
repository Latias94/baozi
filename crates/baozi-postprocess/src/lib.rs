#![forbid(unsafe_code)]

//! Direct, controlled geometry operations on immutable validated Scenes.

mod direct_error;
mod error;
mod operation;
mod remap;
mod report;
mod triangulate;
mod triangulation;

pub use baozi_core::SceneGenerationId;

pub use direct_error::{TriangulationCause, TriangulationFailure};
pub use error::{
    PostProcessCancellationOrigin, PostProcessControlCause, PostProcessRunError,
    PostProcessRunFailure,
};
pub use operation::{
    PostProcessCancellationToken, PostProcessControl, PostProcessResourceEvidence,
};
pub use remap::{PrimitiveCorner, PrimitiveFace, TriangulationRemap, TriangulationRemapError};
pub use report::{
    PostProcessOperation, PostProcessReport, PostProcessRunDisposition, PostProcessRunOutcome,
};
pub use triangulate::triangulate_primitive;
pub use triangulation::PolygonError as TriangulationIssue;
