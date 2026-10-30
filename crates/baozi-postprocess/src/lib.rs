#![forbid(unsafe_code)]

//! Direct, controlled operations on immutable validated Scenes.

mod direct_error;
mod error;
mod normals;
mod operation;
mod remap;
mod report;
mod scene_edit;
mod space;
mod triangulate;
mod triangulation;

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod test_fixtures;

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

pub use normals::{
    ExistingNormals, NormalCause, NormalFailure, NormalGenerationReport, NormalIssue, NormalMode,
    NormalOptions, TangentHandling, generate_normals,
};
pub use space::{
    SpaceConversionCause, SpaceConversionFailure, SpaceConversionIssue, SpaceConversionOptions,
    SpaceConversionReport, convert_scene_space,
};
