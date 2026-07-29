use thiserror::Error;

use crate::SceneValidationLimits;

use super::input::animation::AnimationAuthoringBatch;
use super::input::appearance::{AppearanceAuthoringBatch, ImagePayloadAuthoring};
use super::input::deformation::{DeformationAuthoringBatch, MorphDeltaPayloadAuthoring};
use super::input::geometry::{
    GeometryAttributePayloadAuthoring, GeometryAuthoringBatch, GeometryTopologyAuthoring,
};
use super::input::{Presence, SceneAuthoringBatch};
use super::plan::NodeAuthoringSemanticError;
use super::plan::animation::{AnimationAuthoringSemanticError, animation_shape_scratch_envelope};
use super::plan::appearance::{
    AppearanceAuthoringSemanticError, appearance_shape_scratch_envelope,
};
use super::plan::deformation::{
    DeformationAuthoringSemanticError, deformation_shape_scratch_envelope,
};
use super::plan::geometry::{GeometryAuthoringSemanticError, geometry_shape_scratch_envelope};
use super::plan::node_shape_scratch_envelope;
use super::{
    ValidatedAnimationBatch, ValidatedAppearanceBatch, ValidatedDeformationBatch,
    ValidatedGeometryBatch, ValidatedNodeBatch,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct SceneInputPreflight {
    terminal_attribution_slots: usize,
}

impl SceneInputPreflight {
    pub(in crate::canonical_scene::store) fn try_new(
        input: SceneAuthoringBatch<'_>,
        limits: SceneValidationLimits,
    ) -> Result<Self, SceneInputPreflightError> {
        let shape = preflight_input_shape(input)?;
        require_limit("nodes", shape.canonical_nodes, limits.max_nodes())?;
        require_limit("entities", shape.entities, limits.max_entities())?;

        let node_scratch = node_shape_scratch_envelope(input.nodes())?;
        let appearance_scratch = appearance_shape_scratch_envelope(input.appearance())?;
        let geometry_scratch = geometry_shape_scratch_envelope(input.geometry())?;
        let deformation_scratch = deformation_shape_scratch_envelope(input.deformation())?;
        let animation_scratch = animation_shape_scratch_envelope(
            input.animations(),
            input.nodes().nodes().len(),
            input.geometry().mesh_instances().len(),
        )?;

        let mut peak_scratch = node_scratch.peak_bytes();
        let mut retained_scratch = node_scratch.retained_bytes();
        retained_scratch = accumulated_scratch(
            retained_scratch,
            appearance_scratch,
            "appearance validation scratch envelope",
        )?;
        peak_scratch = peak_scratch.max(retained_scratch);
        retained_scratch = accumulated_scratch(
            retained_scratch,
            geometry_scratch,
            "geometry validation scratch envelope",
        )?;
        peak_scratch = peak_scratch.max(retained_scratch);
        let deformation_peak = accumulated_scratch(
            retained_scratch,
            deformation_scratch.peak_bytes(),
            "deformation validation scratch envelope",
        )?;
        peak_scratch = peak_scratch.max(deformation_peak);
        retained_scratch = accumulated_scratch(
            retained_scratch,
            deformation_scratch.retained_bytes(),
            "retained deformation validation scratch",
        )?;
        let animation_peak = accumulated_scratch(
            retained_scratch,
            animation_scratch,
            "animation validation scratch envelope",
        )?;
        peak_scratch = peak_scratch.max(animation_peak);
        require_limit("scratch bytes", peak_scratch, limits.max_scratch_bytes())?;

        Ok(Self {
            terminal_attribution_slots: shape.terminal_attribution_slots,
        })
    }

    pub(in crate::canonical_scene::store) const fn terminal_attribution_slots(self) -> usize {
        self.terminal_attribution_slots
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct DomainBlockUpperBounds {
    node: usize,
    appearance: usize,
    geometry: usize,
    deformation: usize,
    animation: usize,
}

#[derive(Debug, Clone, Copy)]
struct ScenePreflightShape {
    canonical_nodes: u64,
    entities: u64,
    terminal_attribution_slots: usize,
}

fn preflight_input_shape(
    input: SceneAuthoringBatch<'_>,
) -> Result<ScenePreflightShape, SceneInputPreflightError> {
    let maximum_u32_count = u64::from(u32::MAX);
    let authored_nodes = input.nodes().nodes().len();
    let maximum_authored_nodes = maximum_u32_count.saturating_sub(1);
    require_count_domain("authored nodes", authored_nodes, maximum_authored_nodes)?;
    if let Presence::Present(roots) = input.nodes().roots() {
        require_count_domain("authored roots", roots.len(), maximum_u32_count)?;
    }

    let canonical_nodes = usize_to_u64(authored_nodes, "canonical node count")?
        .checked_add(1)
        .ok_or(SceneInputPreflightError::AccountingOverflow {
            dimension: "canonical node count",
        })?;
    let mut entities = canonical_nodes;
    let mut blocks = DomainBlockUpperBounds {
        node: 1_usize
            .checked_add(usize::from(authored_nodes != 0).checked_mul(2).ok_or(
                SceneInputPreflightError::AccountingOverflow {
                    dimension: "node payload block upper bound",
                },
            )?)
            .ok_or(SceneInputPreflightError::AccountingOverflow {
                dimension: "node block upper bound",
            })?,
        ..DomainBlockUpperBounds::default()
    };

    let appearance = inspect_appearance_shape(input.appearance(), maximum_u32_count)?;
    entities = checked_entity_add(entities, appearance.entities, "appearance entities")?;
    blocks.appearance = appearance.block_upper_bound;

    let geometry = inspect_geometry_shape(input.geometry(), maximum_u32_count)?;
    entities = checked_entity_add(entities, geometry.entities, "geometry entities")?;
    blocks.geometry = geometry.block_upper_bound;

    let deformation = inspect_deformation_shape(input.deformation(), maximum_u32_count)?;
    entities = checked_entity_add(entities, deformation.entities, "deformation entities")?;
    blocks.deformation = deformation.block_upper_bound;

    let animation = inspect_animation_shape(input.animations(), maximum_u32_count)?;
    entities = checked_entity_add(entities, animation.entities, "animation entities")?;
    blocks.animation = animation.block_upper_bound;

    let canonical_block_upper_bound = [
        1_usize,
        blocks.node,
        blocks.appearance,
        blocks.geometry,
        blocks.deformation,
        blocks.animation,
    ]
    .into_iter()
    .try_fold(0_usize, usize::checked_add)
    .ok_or(SceneInputPreflightError::AccountingOverflow {
        dimension: "canonical block upper bound",
    })?;
    let terminal_attribution_slots = canonical_block_upper_bound.checked_add(1).ok_or(
        SceneInputPreflightError::AccountingOverflow {
            dimension: "terminal attribution slots",
        },
    )?;

    Ok(ScenePreflightShape {
        canonical_nodes,
        entities,
        terminal_attribution_slots,
    })
}

#[derive(Debug, Clone, Copy)]
struct DomainInputShape {
    entities: u64,
    block_upper_bound: usize,
}

fn inspect_appearance_shape(
    input: AppearanceAuthoringBatch<'_>,
    maximum: u64,
) -> Result<DomainInputShape, SceneInputPreflightError> {
    let collections = [
        ("images", input.images().len()),
        ("samplers", input.samplers().len()),
        ("textures", input.textures().len()),
        ("material bindings", input.bindings().len()),
        ("materials", input.materials().len()),
    ];
    require_collections(collections, maximum)?;
    let entities = sum_collection_entities(collections, "appearance entity count")?;

    let mut payload_bytes = 0_u64;
    let mut media_type_bytes = 0_u64;
    let mut has_media_type = false;
    for image in input.images() {
        match image.payload() {
            ImagePayloadAuthoring::Encoded { bytes, media_type } => {
                require_count_domain("encoded image payload bytes", bytes.len(), maximum)?;
                payload_bytes =
                    checked_len_add(payload_bytes, bytes.len(), "encoded image payload bytes")?;
                if let Presence::Present(media_type) = media_type {
                    has_media_type = true;
                    require_count_domain(
                        "image media type bytes",
                        media_type.as_str().len(),
                        maximum,
                    )?;
                    media_type_bytes = checked_len_add(
                        media_type_bytes,
                        media_type.as_str().len(),
                        "image media type bytes",
                    )?;
                }
            }
        }
    }
    require_aggregate_count_domain("encoded image payload bytes", payload_bytes, maximum)?;
    require_aggregate_count_domain("image media type bytes", media_type_bytes, maximum)?;

    let table_blocks = collections
        .into_iter()
        .try_fold(0_usize, |count, (_, rows)| {
            count.checked_add(usize::from(rows != 0))
        })
        .ok_or(SceneInputPreflightError::AccountingOverflow {
            dimension: "appearance table block upper bound",
        })?;
    let payload_blocks = usize::from(payload_bytes != 0)
        .checked_add(usize::from(has_media_type))
        .ok_or(SceneInputPreflightError::AccountingOverflow {
            dimension: "appearance payload block upper bound",
        })?;
    let block_upper_bound = 1_usize
        .checked_add(table_blocks)
        .and_then(|count| count.checked_add(payload_blocks))
        .ok_or(SceneInputPreflightError::AccountingOverflow {
            dimension: "appearance block upper bound",
        })?;
    Ok(DomainInputShape {
        entities,
        block_upper_bound,
    })
}

fn inspect_geometry_shape(
    input: GeometryAuthoringBatch<'_>,
    maximum: u64,
) -> Result<DomainInputShape, SceneInputPreflightError> {
    let collections = [
        ("logical meshes", input.logical_meshes().len()),
        ("primitives", input.primitives().len()),
        ("attributes", input.attributes().len()),
        ("mesh instances", input.mesh_instances().len()),
    ];
    require_collections(collections, maximum)?;
    let entities = sum_collection_entities(collections, "geometry entity count")?;

    for primitive in input.primitives() {
        if let GeometryTopologyAuthoring::Polygons(boundaries) = primitive.topology() {
            require_count_domain("polygon boundaries", boundaries.len(), maximum)?;
        }
        if let Presence::Present(indices) = primitive.indices() {
            require_count_domain("primitive indices", indices.len(), maximum)?;
        }
    }
    for attribute in input.attributes() {
        match attribute.payload() {
            GeometryAttributePayloadAuthoring::DenseF32 {
                source_values,
                source_indices,
                ..
            } => {
                require_count_domain("dense F32 attribute values", source_values.len(), maximum)?;
                if let Presence::Present(indices) = source_indices {
                    require_count_domain("dense F32 source indices", indices.len(), maximum)?;
                }
            }
            GeometryAttributePayloadAuthoring::DenseU32 { values, .. } => {
                require_count_domain("dense U32 attribute values", values.len(), maximum)?;
            }
            GeometryAttributePayloadAuthoring::SparseF32 {
                indices,
                values,
                default,
                ..
            } => {
                require_count_domain("sparse F32 attribute indices", indices.len(), maximum)?;
                require_count_domain("sparse F32 attribute values", values.len(), maximum)?;
                require_count_domain("sparse F32 attribute default", default.len(), maximum)?;
            }
            GeometryAttributePayloadAuthoring::SparseU32 {
                indices,
                values,
                default,
                ..
            } => {
                require_count_domain("sparse U32 attribute indices", indices.len(), maximum)?;
                require_count_domain("sparse U32 attribute values", values.len(), maximum)?;
                require_count_domain("sparse U32 attribute default", default.len(), maximum)?;
            }
        }
    }

    let table_blocks = collections
        .into_iter()
        .try_fold(0_usize, |count, (_, rows)| {
            count.checked_add(usize::from(rows != 0))
        })
        .ok_or(SceneInputPreflightError::AccountingOverflow {
            dimension: "geometry table block upper bound",
        })?;
    let primitive_payload_blocks = input.primitives().len().checked_mul(2).ok_or(
        SceneInputPreflightError::AccountingOverflow {
            dimension: "primitive payload block upper bound",
        },
    )?;
    let attribute_payload_blocks = input.attributes().len().checked_mul(3).ok_or(
        SceneInputPreflightError::AccountingOverflow {
            dimension: "attribute payload block upper bound",
        },
    )?;
    let block_upper_bound = 1_usize
        .checked_add(table_blocks)
        .and_then(|count| count.checked_add(primitive_payload_blocks))
        .and_then(|count| count.checked_add(attribute_payload_blocks))
        .ok_or(SceneInputPreflightError::AccountingOverflow {
            dimension: "geometry block upper bound",
        })?;
    Ok(DomainInputShape {
        entities,
        block_upper_bound,
    })
}

fn inspect_deformation_shape(
    input: DeformationAuthoringBatch<'_>,
    maximum: u64,
) -> Result<DomainInputShape, SceneInputPreflightError> {
    let collections = [
        ("skins", input.skins().len()),
        (
            "morph target definitions",
            input.morph_target_definitions().len(),
        ),
        ("logical morphs", input.logical_meshes().len()),
        ("primitive deformation", input.primitives().len()),
        ("mesh instance deformation", input.mesh_instances().len()),
    ];
    require_collections(collections, maximum)?;

    let mut morph_target_count = 0_u64;
    let mut morph_delta_count = 0_u64;
    let mut payload_presence = [false; 13];
    for skin in input.skins() {
        require_count_domain("skin joints", skin.joints().len(), maximum)?;
        require_count_domain(
            "skin inverse-bind matrices",
            skin.inverse_bind_matrices().len(),
            maximum,
        )?;
        payload_presence[0] |= !skin.joints().is_empty();
        payload_presence[1] |= !skin.inverse_bind_matrices().is_empty();
        payload_presence[2] |= skin.matrix_spaces().correction().is_some();
        payload_presence[3] |= matches!(skin.name(), Presence::Present(_));
    }
    for logical in input.logical_meshes() {
        require_count_domain(
            "logical morph default weights",
            logical.default_weights().len(),
            maximum,
        )?;
        payload_presence[8] |= !logical.default_weights().is_empty();
    }
    for definition in input.morph_target_definitions() {
        payload_presence[9] |= matches!(definition.name(), Presence::Present(_));
    }
    for primitive in input.primitives() {
        if let Presence::Present(rows) = primitive.influences() {
            require_count_domain(
                "influence row offsets",
                rows.vertex_offsets().len(),
                maximum,
            )?;
            require_count_domain("influences", rows.influences().len(), maximum)?;
            payload_presence[4] |= !rows.vertex_offsets().is_empty();
            if !rows.influences().is_empty() {
                payload_presence[5] = true;
                payload_presence[6] = true;
                payload_presence[7] = true;
            }
        }
        require_count_domain(
            "primitive morph targets",
            primitive.morph_targets().len(),
            maximum,
        )?;
        morph_target_count = checked_len_add(
            morph_target_count,
            primitive.morph_targets().len(),
            "morph target count",
        )?;
        for target in primitive.morph_targets() {
            require_count_domain("morph target deltas", target.deltas().len(), maximum)?;
            morph_delta_count = checked_len_add(
                morph_delta_count,
                target.deltas().len(),
                "morph delta count",
            )?;
            for delta in target.deltas() {
                match delta.payload() {
                    MorphDeltaPayloadAuthoring::Dense { values } => {
                        require_count_domain("morph delta values", values.len(), maximum)?;
                        payload_presence[10] |= !values.is_empty();
                    }
                    MorphDeltaPayloadAuthoring::Sparse {
                        indices, values, ..
                    } => {
                        require_count_domain("morph sparse indices", indices.len(), maximum)?;
                        require_count_domain("morph delta values", values.len(), maximum)?;
                        payload_presence[10] |= !values.is_empty();
                        payload_presence[11] |= !indices.is_empty();
                    }
                }
            }
        }
    }
    for instance in input.mesh_instances() {
        if let Presence::Present(weights) = instance.morph_weights() {
            require_count_domain("instance morph weights", weights.len(), maximum)?;
            payload_presence[12] |= !weights.is_empty();
        }
    }
    require_aggregate_count_domain("morph targets", morph_target_count, maximum)?;
    require_aggregate_count_domain("morph deltas", morph_delta_count, maximum)?;

    let entities = [
        usize_to_u64(input.skins().len(), "skin entity count")?,
        usize_to_u64(
            input.morph_target_definitions().len(),
            "morph definition entity count",
        )?,
        morph_target_count,
        morph_delta_count,
    ]
    .into_iter()
    .try_fold(0_u64, u64::checked_add)
    .ok_or(SceneInputPreflightError::AccountingOverflow {
        dimension: "deformation entity count",
    })?;
    let table_blocks = [
        !input.skins().is_empty(),
        !input.logical_meshes().is_empty(),
        !input.morph_target_definitions().is_empty(),
        !input.primitives().is_empty(),
        morph_target_count != 0,
        morph_delta_count != 0,
        !input.mesh_instances().is_empty(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    let payload_blocks = payload_presence
        .into_iter()
        .filter(|present| *present)
        .count();
    let block_upper_bound = 1_usize
        .checked_add(table_blocks)
        .and_then(|count| count.checked_add(payload_blocks))
        .ok_or(SceneInputPreflightError::AccountingOverflow {
            dimension: "deformation block upper bound",
        })?;
    Ok(DomainInputShape {
        entities,
        block_upper_bound,
    })
}

fn inspect_animation_shape(
    input: AnimationAuthoringBatch<'_>,
    maximum: u64,
) -> Result<DomainInputShape, SceneInputPreflightError> {
    let collections = [
        ("animation clips", input.clips().len()),
        ("animation channels", input.channels().len()),
    ];
    require_collections(collections, maximum)?;
    let entities = sum_collection_entities(collections, "animation entity count")?;

    let mut time_count = 0_u64;
    let mut value_count = 0_u64;
    let mut in_tangent_count = 0_u64;
    let mut out_tangent_count = 0_u64;
    for channel in input.channels() {
        require_count_domain("animation times", channel.times().len(), maximum)?;
        require_count_domain("animation values", channel.values().len(), maximum)?;
        time_count = checked_len_add(time_count, channel.times().len(), "animation times")?;
        value_count = checked_len_add(value_count, channel.values().len(), "animation values")?;
        if let Presence::Present(tangents) = channel.in_tangents() {
            require_count_domain("animation in-tangents", tangents.len(), maximum)?;
            in_tangent_count =
                checked_len_add(in_tangent_count, tangents.len(), "animation in-tangents")?;
        }
        if let Presence::Present(tangents) = channel.out_tangents() {
            require_count_domain("animation out-tangents", tangents.len(), maximum)?;
            out_tangent_count =
                checked_len_add(out_tangent_count, tangents.len(), "animation out-tangents")?;
        }
    }
    for (collection, count) in [
        ("animation times", time_count),
        ("animation values", value_count),
        ("animation in-tangents", in_tangent_count),
        ("animation out-tangents", out_tangent_count),
    ] {
        require_aggregate_count_domain(collection, count, maximum)?;
    }

    let table_blocks = usize::from(!input.clips().is_empty())
        .checked_add(usize::from(!input.channels().is_empty()))
        .ok_or(SceneInputPreflightError::AccountingOverflow {
            dimension: "animation table block upper bound",
        })?;
    let payload_blocks = [time_count, value_count, in_tangent_count, out_tangent_count]
        .into_iter()
        .filter(|count| *count != 0)
        .count();
    let block_upper_bound = 1_usize
        .checked_add(table_blocks)
        .and_then(|count| count.checked_add(payload_blocks))
        .ok_or(SceneInputPreflightError::AccountingOverflow {
            dimension: "animation block upper bound",
        })?;
    Ok(DomainInputShape {
        entities,
        block_upper_bound,
    })
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum SceneInputPreflightError {
    #[error(
        "Scene input collection {collection} count {count} exceeds canonical maximum {maximum}"
    )]
    CountDomain {
        collection: &'static str,
        count: u64,
        maximum: u64,
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
    #[error("appearance-domain scratch capacity preflight failed")]
    AppearanceScratch(#[from] AppearanceAuthoringSemanticError),
    #[error("geometry-domain scratch capacity preflight failed")]
    GeometryScratch(#[from] GeometryAuthoringSemanticError),
    #[error("deformation-domain scratch capacity preflight failed")]
    DeformationScratch(#[from] DeformationAuthoringSemanticError),
    #[error("animation-domain scratch capacity preflight failed")]
    AnimationScratch(#[from] AnimationAuthoringSemanticError),
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

#[derive(Debug, Clone, Copy)]
struct SceneDomainMetrics {
    entities: u64,
    edges: u64,
    references: u64,
    semantic_cells: u64,
    work: u64,
    canonical_bytes: u64,
    scratch_bytes: u64,
}

pub(in crate::canonical_scene::store) fn appearance_limits_after_nodes<R>(
    nodes: &ValidatedNodeBatch<'_, R>,
    limits: SceneValidationLimits,
) -> Result<SceneValidationLimits, SceneAuthoringAggregateError> {
    remaining_limits_after(node_metrics(nodes)?, limits)
}

pub(in crate::canonical_scene::store) fn geometry_limits_after_appearance<R>(
    appearance: &ValidatedAppearanceBatch<'_, R>,
    remaining_limits: SceneValidationLimits,
) -> Result<SceneValidationLimits, SceneAuthoringAggregateError> {
    remaining_limits_after(appearance_metrics(appearance), remaining_limits)
}

pub(in crate::canonical_scene::store) fn deformation_limits_after_geometry<R>(
    geometry: &ValidatedGeometryBatch<'_, R>,
    remaining_limits: SceneValidationLimits,
) -> Result<SceneValidationLimits, SceneAuthoringAggregateError> {
    remaining_limits_after(geometry_metrics(geometry), remaining_limits)
}

pub(in crate::canonical_scene::store) fn animation_limits_after_deformation<R>(
    deformation: &ValidatedDeformationBatch<'_, R>,
    remaining_limits: SceneValidationLimits,
) -> Result<SceneValidationLimits, SceneAuthoringAggregateError> {
    remaining_limits_after(deformation_metrics(deformation), remaining_limits)
}

pub(in crate::canonical_scene::store) fn validate_scene_aggregate<NR, AR, GR, DR, ANR>(
    nodes: &ValidatedNodeBatch<'_, NR>,
    appearance: &ValidatedAppearanceBatch<'_, AR>,
    geometry: &ValidatedGeometryBatch<'_, GR>,
    deformation: &ValidatedDeformationBatch<'_, DR>,
    animation: &ValidatedAnimationBatch<'_, ANR>,
    limits: SceneValidationLimits,
) -> Result<(), SceneAuthoringAggregateError> {
    let metrics = [
        node_metrics(nodes)?,
        appearance_metrics(appearance),
        geometry_metrics(geometry),
        deformation_metrics(deformation),
        animation_metrics(animation),
    ];
    require_total(
        SceneAuthoringAggregateLimit::Entities,
        metrics.map(|domain| domain.entities),
        limits.max_entities(),
    )?;
    require_total(
        SceneAuthoringAggregateLimit::Edges,
        metrics.map(|domain| domain.edges),
        limits.max_edges(),
    )?;
    require_total(
        SceneAuthoringAggregateLimit::References,
        metrics.map(|domain| domain.references),
        limits.max_references(),
    )?;
    require_total(
        SceneAuthoringAggregateLimit::SemanticCells,
        metrics.map(|domain| domain.semantic_cells),
        limits.max_payload_cells(),
    )?;
    require_total(
        SceneAuthoringAggregateLimit::Work,
        metrics.map(|domain| domain.work),
        limits.max_work(),
    )?;
    require_total(
        SceneAuthoringAggregateLimit::CanonicalBytes,
        metrics.map(|domain| domain.canonical_bytes),
        limits.max_canonical_bytes(),
    )?;
    require_total(
        SceneAuthoringAggregateLimit::ScratchBytes,
        metrics.map(|domain| domain.scratch_bytes),
        limits.max_scratch_bytes(),
    )
}

fn node_metrics<R>(
    nodes: &ValidatedNodeBatch<'_, R>,
) -> Result<SceneDomainMetrics, SceneAuthoringAggregateError> {
    let canonical_bytes = nodes.canonical_non_geometry_bytes().ok_or(
        SceneAuthoringAggregateError::AccountingOverflow {
            dimension: "node-domain canonical bytes",
        },
    )?;
    Ok(SceneDomainMetrics {
        entities: nodes.canonical_node_count(),
        edges: nodes.reference_count(),
        references: nodes.reference_count(),
        semantic_cells: nodes.semantic_cell_count(),
        work: nodes.validation_work(),
        canonical_bytes,
        scratch_bytes: nodes.scratch_bytes(),
    })
}

const fn appearance_metrics<R>(appearance: &ValidatedAppearanceBatch<'_, R>) -> SceneDomainMetrics {
    SceneDomainMetrics {
        entities: appearance.entity_count(),
        edges: appearance.edge_count(),
        references: appearance.reference_count(),
        semantic_cells: appearance.semantic_cell_count(),
        work: appearance.validation_work(),
        canonical_bytes: appearance.canonical_bytes(),
        scratch_bytes: appearance.scratch_bytes(),
    }
}

const fn geometry_metrics<R>(geometry: &ValidatedGeometryBatch<'_, R>) -> SceneDomainMetrics {
    SceneDomainMetrics {
        entities: geometry.entity_count(),
        edges: geometry.edge_count(),
        references: geometry.reference_count(),
        semantic_cells: geometry.semantic_cell_count(),
        work: geometry.validation_work(),
        canonical_bytes: geometry.canonical_bytes(),
        scratch_bytes: geometry.scratch_bytes(),
    }
}

const fn deformation_metrics<R>(
    deformation: &ValidatedDeformationBatch<'_, R>,
) -> SceneDomainMetrics {
    SceneDomainMetrics {
        entities: deformation.entity_count(),
        edges: deformation.edge_count(),
        references: deformation.reference_count(),
        semantic_cells: deformation.semantic_cell_count(),
        work: deformation.validation_work(),
        canonical_bytes: deformation.canonical_bytes(),
        scratch_bytes: deformation.scratch_bytes(),
    }
}

const fn animation_metrics<R>(animation: &ValidatedAnimationBatch<'_, R>) -> SceneDomainMetrics {
    SceneDomainMetrics {
        entities: animation.entity_count(),
        edges: animation.edge_count(),
        references: animation.reference_count(),
        semantic_cells: animation.semantic_cell_count(),
        work: animation.validation_work(),
        canonical_bytes: animation.canonical_bytes(),
        scratch_bytes: animation.scratch_bytes(),
    }
}

fn remaining_limits_after(
    consumed: SceneDomainMetrics,
    limits: SceneValidationLimits,
) -> Result<SceneValidationLimits, SceneAuthoringAggregateError> {
    Ok(limits
        .with_max_entities(remaining(
            SceneAuthoringAggregateLimit::Entities,
            limits.max_entities(),
            consumed.entities,
        )?)
        .with_max_edges(remaining(
            SceneAuthoringAggregateLimit::Edges,
            limits.max_edges(),
            consumed.edges,
        )?)
        .with_max_references(remaining(
            SceneAuthoringAggregateLimit::References,
            limits.max_references(),
            consumed.references,
        )?)
        .with_max_payload_cells(remaining(
            SceneAuthoringAggregateLimit::SemanticCells,
            limits.max_payload_cells(),
            consumed.semantic_cells,
        )?)
        .with_max_work(remaining(
            SceneAuthoringAggregateLimit::Work,
            limits.max_work(),
            consumed.work,
        )?)
        .with_max_canonical_bytes(remaining(
            SceneAuthoringAggregateLimit::CanonicalBytes,
            limits.max_canonical_bytes(),
            consumed.canonical_bytes,
        )?)
        .with_max_scratch_bytes(remaining(
            SceneAuthoringAggregateLimit::ScratchBytes,
            limits.max_scratch_bytes(),
            consumed.scratch_bytes,
        )?))
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

fn require_collections<const N: usize>(
    collections: [(&'static str, usize); N],
    maximum: u64,
) -> Result<(), SceneInputPreflightError> {
    for (collection, count) in collections {
        require_count_domain(collection, count, maximum)?;
    }
    Ok(())
}

fn require_count_domain(
    collection: &'static str,
    count: usize,
    maximum: u64,
) -> Result<(), SceneInputPreflightError> {
    require_aggregate_count_domain(
        collection,
        usize_to_u64(count, "input collection count")?,
        maximum,
    )
}

fn require_aggregate_count_domain(
    collection: &'static str,
    count: u64,
    maximum: u64,
) -> Result<(), SceneInputPreflightError> {
    if count <= maximum {
        Ok(())
    } else {
        Err(SceneInputPreflightError::CountDomain {
            collection,
            count,
            maximum,
        })
    }
}

fn sum_collection_entities<const N: usize>(
    collections: [(&'static str, usize); N],
    dimension: &'static str,
) -> Result<u64, SceneInputPreflightError> {
    let mut total = 0_u64;
    for (_, count) in collections {
        total = total
            .checked_add(usize_to_u64(count, dimension)?)
            .ok_or(SceneInputPreflightError::AccountingOverflow { dimension })?;
    }
    Ok(total)
}

fn checked_entity_add(
    entities: u64,
    domain_entities: u64,
    dimension: &'static str,
) -> Result<u64, SceneInputPreflightError> {
    entities
        .checked_add(domain_entities)
        .ok_or(SceneInputPreflightError::AccountingOverflow { dimension })
}

fn checked_len_add(
    total: u64,
    additional: usize,
    dimension: &'static str,
) -> Result<u64, SceneInputPreflightError> {
    total
        .checked_add(usize_to_u64(additional, dimension)?)
        .ok_or(SceneInputPreflightError::AccountingOverflow { dimension })
}

fn accumulated_scratch(
    retained: u64,
    stage: u64,
    dimension: &'static str,
) -> Result<u64, SceneInputPreflightError> {
    retained
        .checked_add(stage)
        .ok_or(SceneInputPreflightError::AccountingOverflow { dimension })
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

fn require_total<const N: usize>(
    limit: SceneAuthoringAggregateLimit,
    domain_values: [u64; N],
    maximum: u64,
) -> Result<(), SceneAuthoringAggregateError> {
    let actual = domain_values
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
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
