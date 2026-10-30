use thiserror::Error;

use crate::SceneValidationLimits;
use crate::canonical_scene::store::schema::{
    SCENE_DIRECTORY_BLOCK_COUNT, SCENE_DIRECTORY_CANONICAL_BYTES,
};

use super::input::{Presence, SceneAuthoringBatch};
use super::{
    ValidatedAnimationBatch, ValidatedAppearanceBatch, ValidatedDeformationBatch,
    ValidatedGeometryBatch, ValidatedNodeBatch, ValidatedProfileBatch, ValidatedSpatialBatch,
};

/// Checks top-level count domains and limits, then returns the terminal attribution capacity.
///
/// A fresh Scene has only its directory record plus one terminal evidence record in the store.
/// Typed domain owners use independent retention admission and never add record-store slots.
/// Nested shape, scratch, references, and semantics remain the responsibility of the controlled
/// domain validators. Only top-level lengths are read here, with no input-sized traversal.
pub(in crate::canonical_scene::store) fn scene_input_terminal_attribution_slots(
    input: SceneAuthoringBatch<'_>,
    limits: SceneValidationLimits,
) -> Result<usize, SceneInputEnvelopeError> {
    let shape = SceneTopLevelShape::from_input(input);
    shape.require_top_level_count_domains()?;

    let canonical_nodes = shape.canonical_node_count()?;
    require_envelope_limit("nodes", canonical_nodes, limits.max_nodes())?;
    require_envelope_limit(
        "entity lower bound",
        shape.entity_lower_bound(canonical_nodes)?,
        limits.max_entities(),
    )?;
    require_envelope_limit(
        "metadata entries",
        usize_to_u64(shape.metadata_entries, "metadata entry count")?,
        limits.max_metadata_entries(),
    )?;

    Ok(SCENE_DIRECTORY_BLOCK_COUNT as usize + 1)
}

#[derive(Debug, Clone, Copy)]
struct SceneTopLevelShape {
    cameras: usize,
    lights: usize,
    roots: usize,
    nodes: usize,
    images: usize,
    samplers: usize,
    textures: usize,
    material_bindings: usize,
    materials: usize,
    logical_meshes: usize,
    primitives: usize,
    attributes: usize,
    mesh_instances: usize,
    skins: usize,
    morph_target_definitions: usize,
    logical_morphs: usize,
    primitive_deformations: usize,
    mesh_instance_deformations: usize,
    animation_clips: usize,
    animation_channels: usize,
    resources: usize,
    source_identities: usize,
    provenance_nodes: usize,
    owner_provenance: usize,
    metadata_entries: usize,
}

impl SceneTopLevelShape {
    fn from_input(input: SceneAuthoringBatch<'_>) -> Self {
        let spatial = input.spatial();
        let nodes = input.nodes();
        let appearance = input.appearance();
        let geometry = input.geometry();
        let deformation = input.deformation();
        let animations = input.animations();
        let profile = input.profile();
        let lineage = profile.lineage();
        Self {
            cameras: spatial.cameras().len(),
            lights: spatial.lights().len(),
            roots: match nodes.roots() {
                Presence::Absent => 0,
                Presence::Present(roots) => roots.len(),
            },
            nodes: nodes.nodes().len(),
            images: appearance.images().len(),
            samplers: appearance.samplers().len(),
            textures: appearance.textures().len(),
            material_bindings: appearance.bindings().len(),
            materials: appearance.materials().len(),
            logical_meshes: geometry.logical_meshes().len(),
            primitives: geometry.primitives().len(),
            attributes: geometry.attributes().len(),
            mesh_instances: geometry.mesh_instances().len(),
            skins: deformation.skins().len(),
            morph_target_definitions: deformation.morph_target_definitions().len(),
            logical_morphs: deformation.logical_meshes().len(),
            primitive_deformations: deformation.primitives().len(),
            mesh_instance_deformations: deformation.mesh_instances().len(),
            animation_clips: animations.clips().len(),
            animation_channels: animations.channels().len(),
            resources: lineage.resources().len(),
            source_identities: lineage.source_identities().len(),
            provenance_nodes: lineage.provenance_nodes().len(),
            owner_provenance: lineage.owner_provenance().len(),
            metadata_entries: profile.metadata().entries().len(),
        }
    }

    fn require_top_level_count_domains(self) -> Result<(), SceneInputEnvelopeError> {
        let maximum = u64::from(u32::MAX);
        require_count_domain("authored nodes", self.nodes, maximum.saturating_sub(1))?;
        for (collection, count) in [
            ("cameras", self.cameras),
            ("lights", self.lights),
            ("authored roots", self.roots),
            ("images", self.images),
            ("samplers", self.samplers),
            ("textures", self.textures),
            ("material bindings", self.material_bindings),
            ("materials", self.materials),
            ("logical meshes", self.logical_meshes),
            ("primitives", self.primitives),
            ("attributes", self.attributes),
            ("mesh instances", self.mesh_instances),
            ("skins", self.skins),
            ("morph target definitions", self.morph_target_definitions),
            ("logical morphs", self.logical_morphs),
            ("primitive deformation", self.primitive_deformations),
            ("mesh instance deformation", self.mesh_instance_deformations),
            ("animation clips", self.animation_clips),
            ("animation channels", self.animation_channels),
            ("observed resources", self.resources),
            ("source identities", self.source_identities),
            ("provenance nodes", self.provenance_nodes),
            ("owner provenance rows", self.owner_provenance),
            ("metadata entries", self.metadata_entries),
        ] {
            require_count_domain(collection, count, maximum)?;
        }
        Ok(())
    }

    fn canonical_node_count(self) -> Result<u64, SceneInputEnvelopeError> {
        usize_to_u64(self.nodes, "canonical node count")?
            .checked_add(1)
            .ok_or(SceneInputEnvelopeError::AccountingOverflow {
                dimension: "canonical node count",
            })
    }

    fn entity_lower_bound(self, canonical_nodes: u64) -> Result<u64, SceneInputEnvelopeError> {
        [
            canonical_nodes,
            usize_to_u64(self.cameras, "spatial entity lower bound")?,
            usize_to_u64(self.lights, "spatial entity lower bound")?,
            usize_to_u64(self.images, "appearance entity lower bound")?,
            usize_to_u64(self.samplers, "appearance entity lower bound")?,
            usize_to_u64(self.textures, "appearance entity lower bound")?,
            usize_to_u64(self.material_bindings, "appearance entity lower bound")?,
            usize_to_u64(self.materials, "appearance entity lower bound")?,
            usize_to_u64(self.logical_meshes, "geometry entity lower bound")?,
            usize_to_u64(self.primitives, "geometry entity lower bound")?,
            usize_to_u64(self.attributes, "geometry entity lower bound")?,
            usize_to_u64(self.mesh_instances, "geometry entity lower bound")?,
            usize_to_u64(self.skins, "deformation entity lower bound")?,
            usize_to_u64(
                self.morph_target_definitions,
                "deformation entity lower bound",
            )?,
            usize_to_u64(self.animation_clips, "animation entity lower bound")?,
            usize_to_u64(self.animation_channels, "animation entity lower bound")?,
            usize_to_u64(self.resources, "profile entity lower bound")?,
            usize_to_u64(self.source_identities, "profile entity lower bound")?,
            usize_to_u64(self.provenance_nodes, "profile entity lower bound")?,
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
        .ok_or(SceneInputEnvelopeError::AccountingOverflow {
            dimension: "entity lower bound",
        })
    }
}

#[derive(Debug, Error)]
pub(crate) enum SceneInputEnvelopeError {
    #[error(
        "Scene input collection {collection} count {count} exceeds canonical maximum {maximum}"
    )]
    CountDomain {
        collection: &'static str,
        count: u64,
        maximum: u64,
    },
    #[error("Scene input envelope accounting overflowed for {dimension}")]
    AccountingOverflow { dimension: &'static str },
    #[error("Scene input envelope {limit} {actual} exceeds configured maximum {maximum}")]
    LimitExceeded {
        limit: &'static str,
        actual: u64,
        maximum: u64,
    },
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

pub(in crate::canonical_scene::store) fn node_limits_after_spatial<R>(
    spatial: &ValidatedSpatialBatch<'_, R>,
    limits: SceneValidationLimits,
) -> Result<SceneValidationLimits, SceneAuthoringAggregateError> {
    let remaining_limits = remaining_limits_after(spatial_metrics(spatial), limits)?;
    // The root record belongs to Scene, independently of every typed domain.
    Ok(remaining_limits.with_max_canonical_bytes(remaining(
        SceneAuthoringAggregateLimit::CanonicalBytes,
        remaining_limits.max_canonical_bytes(),
        SCENE_DIRECTORY_CANONICAL_BYTES,
    )?))
}

pub(in crate::canonical_scene::store) fn appearance_limits_after_nodes<R>(
    nodes: &ValidatedNodeBatch<'_, R>,
    limits: SceneValidationLimits,
) -> Result<SceneValidationLimits, SceneAuthoringAggregateError> {
    remaining_limits_after(node_metrics(nodes), limits)
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

pub(in crate::canonical_scene::store) fn profile_limits_after_animation<R>(
    animation: &ValidatedAnimationBatch<'_, R>,
    remaining_limits: SceneValidationLimits,
) -> Result<SceneValidationLimits, SceneAuthoringAggregateError> {
    remaining_limits_after(animation_metrics(animation), remaining_limits)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::canonical_scene::store) fn validate_scene_aggregate<SR, NR, AR, GR, DR, ANR, PR>(
    spatial: &ValidatedSpatialBatch<'_, SR>,
    nodes: &ValidatedNodeBatch<'_, NR>,
    appearance: &ValidatedAppearanceBatch<'_, AR>,
    geometry: &ValidatedGeometryBatch<'_, GR>,
    deformation: &ValidatedDeformationBatch<'_, DR>,
    animation: &ValidatedAnimationBatch<'_, ANR>,
    profile: &ValidatedProfileBatch<'_, PR>,
    limits: SceneValidationLimits,
) -> Result<(), SceneAuthoringAggregateError> {
    let metrics = [
        spatial_metrics(spatial),
        node_metrics(nodes),
        appearance_metrics(appearance),
        geometry_metrics(geometry),
        deformation_metrics(deformation),
        animation_metrics(animation),
        profile_metrics(profile),
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
        metrics
            .into_iter()
            .map(|domain| domain.canonical_bytes)
            .chain(std::iter::once(SCENE_DIRECTORY_CANONICAL_BYTES)),
        limits.max_canonical_bytes(),
    )?;
    require_total(
        SceneAuthoringAggregateLimit::ScratchBytes,
        metrics.map(|domain| domain.scratch_bytes),
        limits.max_scratch_bytes(),
    )
}

const fn spatial_metrics<R>(spatial: &ValidatedSpatialBatch<'_, R>) -> SceneDomainMetrics {
    SceneDomainMetrics {
        entities: spatial.entity_count(),
        edges: spatial.edge_count(),
        references: spatial.reference_count(),
        semantic_cells: spatial.semantic_cell_count(),
        work: spatial.validation_work(),
        canonical_bytes: spatial.canonical_bytes(),
        scratch_bytes: spatial.scratch_bytes(),
    }
}

const fn node_metrics<R>(nodes: &ValidatedNodeBatch<'_, R>) -> SceneDomainMetrics {
    SceneDomainMetrics {
        entities: nodes.canonical_node_count(),
        edges: nodes.reference_count(),
        references: nodes.reference_count(),
        semantic_cells: nodes.semantic_cell_count(),
        work: nodes.validation_work(),
        canonical_bytes: nodes.canonical_bytes(),
        scratch_bytes: nodes.scratch_bytes(),
    }
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

const fn profile_metrics<R>(profile: &ValidatedProfileBatch<'_, R>) -> SceneDomainMetrics {
    SceneDomainMetrics {
        entities: profile.entity_count(),
        edges: profile.edge_count(),
        references: profile.reference_count(),
        semantic_cells: profile.payload_cell_count(),
        work: profile.validation_work(),
        canonical_bytes: profile.canonical_bytes(),
        scratch_bytes: profile.scratch_bytes(),
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

fn require_envelope_limit(
    limit: &'static str,
    actual: u64,
    maximum: u64,
) -> Result<(), SceneInputEnvelopeError> {
    if actual <= maximum {
        Ok(())
    } else {
        Err(SceneInputEnvelopeError::LimitExceeded {
            limit,
            actual,
            maximum,
        })
    }
}

fn require_count_domain(
    collection: &'static str,
    count: usize,
    maximum: u64,
) -> Result<(), SceneInputEnvelopeError> {
    let count = usize_to_u64(count, "input collection count")?;
    if count <= maximum {
        Ok(())
    } else {
        Err(SceneInputEnvelopeError::CountDomain {
            collection,
            count,
            maximum,
        })
    }
}

fn usize_to_u64(value: usize, dimension: &'static str) -> Result<u64, SceneInputEnvelopeError> {
    u64::try_from(value).map_err(|_| SceneInputEnvelopeError::AccountingOverflow { dimension })
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

fn require_total(
    limit: SceneAuthoringAggregateLimit,
    domain_values: impl IntoIterator<Item = u64>,
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
