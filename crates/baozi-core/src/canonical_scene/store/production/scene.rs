use thiserror::Error;

use crate::SceneValidationLimits;

use super::input::SceneAuthoringBatch;
use super::plan::NodeAuthoringSemanticError;
use super::plan::geometry::{GeometryAuthoringSemanticError, geometry_shape_scratch_envelope};
use super::plan::node_shape_scratch_envelope;
use super::{ValidatedGeometryBatch, ValidatedNodeBatch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct SceneInputPreflight {
    terminal_attribution_slots: usize,
}

impl SceneInputPreflight {
    pub(in crate::canonical_scene::store) fn try_new(
        input: SceneAuthoringBatch<'_>,
        limits: SceneValidationLimits,
    ) -> Result<Self, SceneInputPreflightError> {
        let authored_nodes = input.nodes().nodes().len();
        let maximum_u32_count = usize::try_from(u32::MAX).unwrap_or(usize::MAX);
        let maximum_authored_nodes = maximum_u32_count.saturating_sub(1);
        if authored_nodes > maximum_authored_nodes {
            return Err(SceneInputPreflightError::CountDomain {
                collection: "authored nodes",
                count: authored_nodes,
                maximum: maximum_authored_nodes,
            });
        }
        let geometry = input.geometry();
        for (collection, count) in [
            ("logical meshes", geometry.logical_meshes().len()),
            ("primitives", geometry.primitives().len()),
            ("attributes", geometry.attributes().len()),
            ("mesh instances", geometry.mesh_instances().len()),
        ] {
            if u32::try_from(count).is_err() {
                return Err(SceneInputPreflightError::CountDomain {
                    collection,
                    count,
                    maximum: maximum_u32_count,
                });
            }
        }

        let canonical_nodes = usize_to_u64(authored_nodes, "canonical node count")?
            .checked_add(1)
            .ok_or(SceneInputPreflightError::AccountingOverflow {
                dimension: "canonical node count",
            })?;
        require_limit("nodes", canonical_nodes, limits.max_nodes())?;
        let geometry_entities = [
            u64::from(geometry.material_count()),
            usize_to_u64(geometry.logical_meshes().len(), "logical mesh count")?,
            usize_to_u64(geometry.primitives().len(), "primitive count")?,
            usize_to_u64(geometry.attributes().len(), "attribute count")?,
            usize_to_u64(geometry.mesh_instances().len(), "mesh instance count")?,
        ]
        .into_iter()
        .try_fold(0_u64, |total, count| total.checked_add(count))
        .ok_or(SceneInputPreflightError::AccountingOverflow {
            dimension: "geometry entity count",
        })?;
        let entities = canonical_nodes.checked_add(geometry_entities).ok_or(
            SceneInputPreflightError::AccountingOverflow {
                dimension: "Scene entity count",
            },
        )?;
        require_limit("entities", entities, limits.max_entities())?;

        let node_scratch = node_shape_scratch_envelope(input.nodes())?;
        let geometry_scratch = geometry_shape_scratch_envelope(geometry)?;
        let geometry_stage_peak = node_scratch
            .retained_bytes()
            .checked_add(geometry_scratch)
            .ok_or(SceneInputPreflightError::AccountingOverflow {
                dimension: "validation scratch envelope",
            })?;
        let scratch = node_scratch.peak_bytes().max(geometry_stage_peak);
        require_limit("scratch bytes", scratch, limits.max_scratch_bytes())?;

        let terminal_attribution_slots = terminal_attribution_slots_for_shape(
            authored_nodes,
            usize::try_from(geometry.material_count()).map_err(|_| {
                SceneInputPreflightError::AccountingOverflow {
                    dimension: "material count",
                }
            })?,
            geometry.logical_meshes().len(),
            geometry.primitives().len(),
            geometry.attributes().len(),
            geometry.mesh_instances().len(),
        )?;
        Ok(Self {
            terminal_attribution_slots,
        })
    }

    pub(in crate::canonical_scene::store) const fn terminal_attribution_slots(self) -> usize {
        self.terminal_attribution_slots
    }
}

pub(super) fn terminal_attribution_slots_for_shape(
    authored_nodes: usize,
    material_count: usize,
    logical_mesh_count: usize,
    primitive_count: usize,
    attribute_count: usize,
    mesh_instance_count: usize,
) -> Result<usize, SceneInputPreflightError> {
    let node_leaf_upper = usize::from(authored_nodes != 0).checked_mul(2).ok_or(
        SceneInputPreflightError::AccountingOverflow {
            dimension: "node payload block upper bound",
        },
    )?;
    let table_upper = [
        material_count,
        logical_mesh_count,
        primitive_count,
        attribute_count,
        mesh_instance_count,
    ]
    .into_iter()
    .try_fold(0_usize, |count, rows| {
        count.checked_add(usize::from(rows != 0))
    })
    .ok_or(SceneInputPreflightError::AccountingOverflow {
        dimension: "geometry table block upper bound",
    })?;
    let primitive_leaf_upper =
        primitive_count
            .checked_mul(2)
            .ok_or(SceneInputPreflightError::AccountingOverflow {
                dimension: "primitive payload block upper bound",
            })?;
    let attribute_leaf_upper =
        attribute_count
            .checked_mul(3)
            .ok_or(SceneInputPreflightError::AccountingOverflow {
                dimension: "attribute payload block upper bound",
            })?;
    3_usize
        .checked_add(node_leaf_upper)
        .and_then(|count| count.checked_add(table_upper))
        .and_then(|count| count.checked_add(primitive_leaf_upper))
        .and_then(|count| count.checked_add(attribute_leaf_upper))
        .and_then(|count| count.checked_add(1))
        .ok_or(SceneInputPreflightError::AccountingOverflow {
            dimension: "terminal attribution slots",
        })
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum SceneInputPreflightError {
    #[error(
        "Scene input collection {collection} count {count} exceeds canonical maximum {maximum}"
    )]
    CountDomain {
        collection: &'static str,
        count: usize,
        maximum: usize,
    },
    #[error("Scene input preflight accounting overflowed for {dimension}")]
    AccountingOverflow { dimension: &'static str },
    #[error("Scene input preflight {limit} {actual} exceeds configured maximum {maximum}")]
    LimitExceeded {
        limit: &'static str,
        actual: u64,
        maximum: u64,
    },
    #[error("node-domain scratch capacity preflight failed")]
    NodeScratch(#[from] NodeAuthoringSemanticError),
    #[error("geometry-domain scratch capacity preflight failed")]
    GeometryScratch(#[from] GeometryAuthoringSemanticError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum SceneAuthoringAggregateLimit {
    Entities,
    Edges,
    References,
    SemanticCells,
    Work,
    CanonicalBytes,
    ScratchBytes,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum SceneAuthoringAggregateError {
    #[error("canonical Scene aggregate accounting overflowed for {dimension}")]
    AccountingOverflow { dimension: &'static str },
    #[error(
        "canonical Scene aggregate {limit:?} count {actual} exceeds configured maximum {maximum}"
    )]
    LimitExceeded {
        limit: SceneAuthoringAggregateLimit,
        actual: u64,
        maximum: u64,
    },
}

pub(in crate::canonical_scene::store) fn geometry_limits_after_nodes<R>(
    nodes: &ValidatedNodeBatch<'_, R>,
    limits: SceneValidationLimits,
) -> Result<SceneValidationLimits, SceneAuthoringAggregateError> {
    let canonical_non_geometry_bytes = nodes.canonical_non_geometry_bytes().ok_or(
        SceneAuthoringAggregateError::AccountingOverflow {
            dimension: "node-domain canonical bytes",
        },
    )?;
    Ok(limits
        .with_max_entities(remaining(
            SceneAuthoringAggregateLimit::Entities,
            limits.max_entities(),
            nodes.canonical_node_count(),
        )?)
        .with_max_edges(remaining(
            SceneAuthoringAggregateLimit::Edges,
            limits.max_edges(),
            nodes.reference_count(),
        )?)
        .with_max_references(remaining(
            SceneAuthoringAggregateLimit::References,
            limits.max_references(),
            nodes.reference_count(),
        )?)
        .with_max_payload_cells(remaining(
            SceneAuthoringAggregateLimit::SemanticCells,
            limits.max_payload_cells(),
            nodes.semantic_cell_count(),
        )?)
        .with_max_work(remaining(
            SceneAuthoringAggregateLimit::Work,
            limits.max_work(),
            nodes.validation_work(),
        )?)
        .with_max_canonical_bytes(remaining(
            SceneAuthoringAggregateLimit::CanonicalBytes,
            limits.max_canonical_bytes(),
            canonical_non_geometry_bytes,
        )?)
        .with_max_scratch_bytes(remaining(
            SceneAuthoringAggregateLimit::ScratchBytes,
            limits.max_scratch_bytes(),
            nodes.scratch_bytes(),
        )?))
}

pub(in crate::canonical_scene::store) fn validate_scene_aggregate<NR, GR>(
    nodes: &ValidatedNodeBatch<'_, NR>,
    geometry: &ValidatedGeometryBatch<'_, GR>,
    limits: SceneValidationLimits,
) -> Result<(), SceneAuthoringAggregateError> {
    let node_canonical_bytes = nodes.canonical_non_geometry_bytes().ok_or(
        SceneAuthoringAggregateError::AccountingOverflow {
            dimension: "node-domain canonical bytes",
        },
    )?;
    require_sum(
        SceneAuthoringAggregateLimit::Entities,
        nodes.canonical_node_count(),
        geometry.entity_count(),
        limits.max_entities(),
    )?;
    require_sum(
        SceneAuthoringAggregateLimit::Edges,
        nodes.reference_count(),
        geometry.edge_count(),
        limits.max_edges(),
    )?;
    require_sum(
        SceneAuthoringAggregateLimit::References,
        nodes.reference_count(),
        geometry.reference_count(),
        limits.max_references(),
    )?;
    require_sum(
        SceneAuthoringAggregateLimit::SemanticCells,
        nodes.semantic_cell_count(),
        geometry.semantic_cell_count(),
        limits.max_payload_cells(),
    )?;
    require_sum(
        SceneAuthoringAggregateLimit::Work,
        nodes.validation_work(),
        geometry.validation_work(),
        limits.max_work(),
    )?;
    require_sum(
        SceneAuthoringAggregateLimit::CanonicalBytes,
        node_canonical_bytes,
        geometry.canonical_bytes(),
        limits.max_canonical_bytes(),
    )?;
    require_sum(
        SceneAuthoringAggregateLimit::ScratchBytes,
        nodes.scratch_bytes(),
        geometry.scratch_bytes(),
        limits.max_scratch_bytes(),
    )
}

fn require_limit(
    limit: &'static str,
    actual: u64,
    maximum: u64,
) -> Result<(), SceneInputPreflightError> {
    if actual <= maximum {
        Ok(())
    } else {
        Err(SceneInputPreflightError::LimitExceeded {
            limit,
            actual,
            maximum,
        })
    }
}

fn usize_to_u64(value: usize, dimension: &'static str) -> Result<u64, SceneInputPreflightError> {
    u64::try_from(value).map_err(|_| SceneInputPreflightError::AccountingOverflow { dimension })
}

fn remaining(
    limit: SceneAuthoringAggregateLimit,
    maximum: u64,
    consumed: u64,
) -> Result<u64, SceneAuthoringAggregateError> {
    maximum
        .checked_sub(consumed)
        .ok_or(SceneAuthoringAggregateError::LimitExceeded {
            limit,
            actual: consumed,
            maximum,
        })
}

fn require_sum(
    limit: SceneAuthoringAggregateLimit,
    first: u64,
    second: u64,
    maximum: u64,
) -> Result<(), SceneAuthoringAggregateError> {
    let actual =
        first
            .checked_add(second)
            .ok_or(SceneAuthoringAggregateError::AccountingOverflow {
                dimension: "canonical Scene aggregate",
            })?;
    if actual <= maximum {
        Ok(())
    } else {
        Err(SceneAuthoringAggregateError::LimitExceeded {
            limit,
            actual,
            maximum,
        })
    }
}
