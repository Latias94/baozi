#![forbid(unsafe_code)]

//! Direct, controlled operations on immutable validated Scenes.

mod direct_error;
mod error;
mod normals;
mod operation;
mod precision;
mod prepare;
pub use prepare::{
    GeometryPreparationReport, NormalOrientation, PostProcessBudgets, PreparationCause,
    PreparationFailure, PreparedPrimitiveReport, TangentPolicy, TriangleLowering,
    TrianglePreparationOptions, prepare_triangles,
};
mod remap;
mod report;
mod scene_edit;
mod space;
mod tangents;
mod triangulate;
mod triangulation;
mod vertices;

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod test_fixtures;

pub use baozi_core::SceneGenerationId;
pub use precision::GeometryNumericIssue;

pub use direct_error::{TriangulationCause, TriangulationFailure};
pub use error::{
    PostProcessCancellationOrigin, PostProcessControlCause, PostProcessRunError,
    PostProcessRunFailure,
};
pub use operation::{
    PostProcessCancellationToken, PostProcessControl, PostProcessResourceEvidence,
};
pub use remap::{
    PrimitiveCorner, PrimitiveFace, PrimitiveVertex, TriangulationRemap, TriangulationRemapError,
};
pub use report::{
    PostProcessOperation, PostProcessReport, PostProcessRunDisposition, PostProcessRunOutcome,
};
pub use triangulate::triangulate_primitive;
pub use triangulation::PolygonError as TriangulationIssue;

pub use normals::{
    ExistingNormals, NormalCause, NormalFailure, NormalGenerationReport, NormalIssue, NormalMode,
    NormalOptions, generate_normals,
};
pub use space::{
    SpaceConversionCause, SpaceConversionFailure, SpaceConversionIssue, SpaceConversionOptions,
    SpaceConversionReport, convert_scene_space,
};
pub use tangents::{
    ExistingTangents, TangentCause, TangentFailure, TangentGenerationReport, TangentIssue,
    TangentOptions, generate_tangents,
};
pub use vertices::{
    ListAttribute, ListVertex, ListVertices, VertexAttribute, VertexLayoutError, list_vertices,
    triangle_vertices,
};
