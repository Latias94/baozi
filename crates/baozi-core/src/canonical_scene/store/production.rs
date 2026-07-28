//! Operation-bound node authoring for canonical store construction.

mod control;
mod input;
mod plan;
mod scene;

#[cfg(test)]
mod tests;

pub(super) use control::{OperationAuthoringControl, OperationAuthoringControlError};
#[cfg(test)]
pub(super) use input::geometry::GeometryAuthoringBatch;
#[cfg(test)]
pub(super) use input::geometry::{
    AuthoringRange, GeometryAttributeAuthoring, GeometryLogicalMeshAuthoring,
    GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring, LogicalMeshLocalId, MaterialLocalId,
};
pub(super) use input::geometry::{GeometryAttributePayloadAuthoring, GeometryTopologyAuthoring};
#[cfg(test)]
pub(super) use input::{NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent};
pub(super) use input::{Presence, SceneAuthoringBatch};
pub(super) use plan::geometry::{
    GeometryAttributePayloadKind, GeometryAttributeScalarKind, GeometryAuthoringFailure,
    ValidatedGeometryBatch, ValidatedGeometryPrimitive, validate_geometry_batch,
};
pub(super) use plan::{NodeAuthoringFailure, ValidatedNodeBatch, validate_node_batch};
#[cfg(test)]
pub(super) use plan::{NodeAuthoringLimit, NodeAuthoringSemanticError};
pub(super) use scene::{
    SceneAuthoringAggregateError, SceneInputPreflight, SceneInputPreflightError,
    geometry_limits_after_nodes, validate_scene_aggregate,
};
