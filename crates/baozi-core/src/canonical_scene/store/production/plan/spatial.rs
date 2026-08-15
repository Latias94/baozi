#![allow(
    clippy::result_large_err,
    reason = "spatial authoring failures retain bounded control evidence inline without a failure-path allocation"
)]

use std::error::Error;
use std::fmt;

use baozi_operation::{CapacityPlan, CapacityPlanError};
use thiserror::Error;

use crate::entity::{
    CameraProjectionKind, LightEmissionError, LightKindCategory, validate_canonical_light_emission,
};
use crate::scene_space::SceneSpaceValidationError;
use crate::{SCENE_VALIDATION_POLL_INTERVAL, SceneValidationLimits, SceneValidationWork};

use super::super::control::CanonicalAuthoringControl;
use super::super::input::spatial::{CameraAuthoring, LightAuthoring, SpatialAuthoringBatch};
use super::super::input::{NodeAuthoringBatch, Presence};

const LOGICAL_U8_BYTES: u64 = 1;
const LOGICAL_U32_BYTES: u64 = 4;
const LOGICAL_F32_BYTES: u64 = 4;
const LOGICAL_F64_BYTES: u64 = 8;
const LOGICAL_REFERENCE_BYTES: u64 = 4;

// The fixed directory shape excludes optional unit/name payloads but includes
// every presence/count/version cell and collection reference slot.
const SPATIAL_DIRECTORY_FIXED_BYTES: u64 =
    (7 * LOGICAL_U32_BYTES) + (11 * LOGICAL_U8_BYTES) + (4 * LOGICAL_REFERENCE_BYTES);
const CAMERA_ROW_FIXED_BYTES: u64 =
    LOGICAL_U32_BYTES + LOGICAL_U8_BYTES + (2 * LOGICAL_U8_BYTES) + (2 * LOGICAL_F32_BYTES);
const LIGHT_ROW_FIXED_BYTES: u64 =
    LOGICAL_U32_BYTES + LOGICAL_U8_BYTES + (3 * LOGICAL_U8_BYTES) + (4 * LOGICAL_F32_BYTES);
const SYNTHETIC_NODE_ATTACHMENT_BYTES: u64 = 6 * LOGICAL_U8_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum SpatialAuthoringLimit {
    Entities,
    Edges,
    References,
    SemanticCells,
    CanonicalBytes,
    Work,
    ScratchBytes,
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum SpatialAuthoringSemanticError {
    #[error("spatial authoring count for {collection} exceeds the u32 canonical domain")]
    CountDomainTooLarge { collection: &'static str },
    #[error("{limit:?} value {actual} exceeds limit {maximum}")]
    LimitExceeded {
        limit: SpatialAuthoringLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("spatial authoring accounting overflowed for {dimension}")]
    AccountingOverflow { dimension: &'static str },
    #[error("SceneSpace is not canonical")]
    InvalidSceneSpace(#[source] SceneSpaceValidationError),
    #[error("camera {camera} has invalid retained projection state")]
    InvalidCameraProjection { camera: usize },
    #[error("light {light} has invalid retained emission")]
    InvalidLightEmission {
        light: usize,
        #[source]
        source: LightEmissionError,
    },
    #[error("node {node} camera {camera} exceeds camera count {camera_count}")]
    CameraOutOfRange {
        node: usize,
        camera: u32,
        camera_count: u32,
    },
    #[error("node {node} light {light} exceeds light count {light_count}")]
    LightOutOfRange {
        node: usize,
        light: u32,
        light_count: u32,
    },
    #[error("camera {camera} has no node placement")]
    OrphanCamera { camera: usize },
    #[error("light {light} has no node placement")]
    OrphanLight { light: usize },
    #[error("scratch capacity planning failed for {resource}: {cause}")]
    ScratchCapacity {
        resource: &'static str,
        #[source]
        cause: CapacityPlanError,
    },
    #[error("scratch allocation failed for {resource}")]
    ScratchAllocation { resource: &'static str },
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum SpatialAuthoringFailure<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Semantic(#[from] SpatialAuthoringSemanticError),
    #[error("spatial authoring validation control failed")]
    Control(#[source] E),
}

/// Operation-accounted spatial semantics paired with the exact authored inputs
/// that produced them.
#[must_use = "dropping the validated spatial batch releases its compiled-plan reservation"]
pub(in crate::canonical_scene::store) struct ValidatedSpatialBatch<'input, R> {
    compiled: SpatialValidationStorage<R>,
    input: SpatialValidationInput<'input>,
}

struct SpatialValidationInput<'input> {
    spatial: SpatialAuthoringBatch<'input>,
    nodes: NodeAuthoringBatch<'input>,
}

struct SpatialValidationStorage<R> {
    camera_count: u32,
    light_count: u32,
    entity_count: u64,
    edge_count: u64,
    reference_count: u64,
    semantic_cell_count: u64,
    canonical_bytes: u64,
    validation_work: u64,
    // Maximum transient scratch observed while validating this domain.
    peak_scratch_bytes: u64,
    // Scratch still physically live while this successful proof is held.
    retained_scratch_bytes: u64,
    tangent_basis_schema_name_bytes: u32,
    tangent_encoding_schema_name_bytes: u32,
    // The zero-sized reservation keeps this proof coupled to its operation owner.
    reservation: R,
}

impl<'input, R> ValidatedSpatialBatch<'input, R> {
    const fn new(
        spatial: SpatialAuthoringBatch<'input>,
        nodes: NodeAuthoringBatch<'input>,
        compiled: SpatialValidationStorage<R>,
    ) -> Self {
        Self {
            compiled,
            input: SpatialValidationInput { spatial, nodes },
        }
    }

    pub(in crate::canonical_scene::store) const fn input(&self) -> SpatialAuthoringBatch<'input> {
        self.input.spatial
    }

    pub(in crate::canonical_scene::store) const fn node_input(&self) -> NodeAuthoringBatch<'input> {
        self.input.nodes
    }

    pub(in crate::canonical_scene::store) const fn scene_space(
        &self,
    ) -> &crate::scene_space::SceneSpace {
        self.input.spatial.scene_space()
    }

    pub(in crate::canonical_scene::store) const fn camera_count(&self) -> u32 {
        self.compiled.camera_count
    }

    pub(in crate::canonical_scene::store) const fn light_count(&self) -> u32 {
        self.compiled.light_count
    }

    pub(in crate::canonical_scene::store) const fn entity_count(&self) -> u64 {
        self.compiled.entity_count
    }

    pub(in crate::canonical_scene::store) const fn edge_count(&self) -> u64 {
        self.compiled.edge_count
    }

    pub(in crate::canonical_scene::store) const fn reference_count(&self) -> u64 {
        self.compiled.reference_count
    }

    pub(in crate::canonical_scene::store) const fn semantic_cell_count(&self) -> u64 {
        self.compiled.semantic_cell_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_bytes(&self) -> u64 {
        self.compiled.canonical_bytes
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.compiled.validation_work
    }

    pub(in crate::canonical_scene::store) const fn scratch_bytes(&self) -> u64 {
        self.compiled.retained_scratch_bytes
    }

    /// Returns the independently admitted transient peak for conformance tests.
    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn peak_scratch_bytes(&self) -> u64 {
        self.compiled.peak_scratch_bytes
    }

    pub(in crate::canonical_scene::store) const fn tangent_basis_schema_name_bytes(&self) -> u32 {
        self.compiled.tangent_basis_schema_name_bytes
    }

    pub(in crate::canonical_scene::store) const fn tangent_encoding_schema_name_bytes(
        &self,
    ) -> u32 {
        self.compiled.tangent_encoding_schema_name_bytes
    }

    pub(in crate::canonical_scene::store) const fn payload_block_count(&self) -> u32 {
        let mut count = 0;
        if self.compiled.tangent_basis_schema_name_bytes != 0 {
            count += 1;
        }
        if self.compiled.tangent_encoding_schema_name_bytes != 0 {
            count += 1;
        }
        count
    }

    pub(in crate::canonical_scene::store) fn cameras(
        &self,
    ) -> impl ExactSizeIterator<Item = &CameraAuthoring> {
        self.input.spatial.cameras().iter()
    }

    pub(in crate::canonical_scene::store) fn lights(
        &self,
    ) -> impl ExactSizeIterator<Item = &LightAuthoring> {
        self.input.spatial.lights().iter()
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store::production) fn into_compiled_facts_for_test(
        self,
    ) -> (u32, u32, u64, u64, R) {
        let SpatialValidationStorage {
            camera_count,
            light_count,
            edge_count,
            reference_count,
            reservation,
            ..
        } = self.compiled;
        (
            camera_count,
            light_count,
            edge_count,
            reference_count,
            reservation,
        )
    }
}

impl<R> fmt::Debug for ValidatedSpatialBatch<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedSpatialBatch")
            .field("input", &self.input.spatial)
            .field("camera_count", &self.compiled.camera_count)
            .field("light_count", &self.compiled.light_count)
            .field("entity_count", &self.compiled.entity_count)
            .field("edge_count", &self.compiled.edge_count)
            .field("reference_count", &self.compiled.reference_count)
            .field("semantic_cell_count", &self.compiled.semantic_cell_count)
            .field("canonical_bytes", &self.compiled.canonical_bytes)
            .field("validation_work", &self.compiled.validation_work)
            .field("peak_scratch_bytes", &self.compiled.peak_scratch_bytes)
            .field(
                "retained_scratch_bytes",
                &self.compiled.retained_scratch_bytes,
            )
            .field("reservation", &"<held>")
            .finish()
    }
}

type SpatialValidationResult<T, C> =
    Result<T, SpatialAuthoringFailure<<C as CanonicalAuthoringControl>::Error>>;

struct SpatialValidationContext<'control, C: CanonicalAuthoringControl> {
    control: &'control mut C,
    limits: SceneValidationLimits,
    work: u64,
    work_since_poll: u64,
}

impl<'control, C: CanonicalAuthoringControl> SpatialValidationContext<'control, C> {
    const fn new(limits: SceneValidationLimits, control: &'control mut C) -> Self {
        Self {
            control,
            limits,
            work: 0,
            work_since_poll: 0,
        }
    }

    fn poll_now(&mut self) -> SpatialValidationResult<(), C> {
        self.control
            .poll()
            .map_err(SpatialAuthoringFailure::Control)?;
        self.work_since_poll = 0;
        Ok(())
    }

    fn finish(&mut self) -> SpatialValidationResult<(), C> {
        self.poll_now()
    }

    fn run_unit<R>(
        &mut self,
        kind: SceneValidationWork,
        action: impl FnOnce() -> R,
    ) -> SpatialValidationResult<R, C> {
        let final_work =
            self.work
                .checked_add(1)
                .ok_or(SpatialAuthoringSemanticError::AccountingOverflow {
                    dimension: "validation work",
                })?;
        enforce_limit::<C::Error>(
            SpatialAuthoringLimit::Work,
            final_work,
            self.limits.max_work(),
        )?;
        let result = self
            .control
            .run_unit(kind, action)
            .map_err(SpatialAuthoringFailure::Control)?;
        self.work = final_work;
        self.work_since_poll = self.work_since_poll.checked_add(1).ok_or(
            SpatialAuthoringSemanticError::AccountingOverflow {
                dimension: "validation work poll interval",
            },
        )?;
        if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
            self.poll_now()?;
        }
        Ok(result)
    }

    fn reserve_scratch(&mut self, bytes: u64) -> SpatialValidationResult<C::Reservation, C> {
        enforce_limit::<C::Error>(
            SpatialAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        let reservation = self
            .control
            .reserve_scratch(bytes)
            .map_err(SpatialAuthoringFailure::Control)?;
        self.control
            .validate_scratch(&reservation)
            .map_err(SpatialAuthoringFailure::Control)?;
        Ok(reservation)
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut C::Reservation,
        bytes: u64,
    ) -> SpatialValidationResult<(), C> {
        enforce_limit::<C::Error>(
            SpatialAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        self.control
            .resize_scratch(reservation, bytes)
            .map_err(SpatialAuthoringFailure::Control)
    }
}

pub(in crate::canonical_scene::store) fn validate_spatial_batch_with_control<'input, C>(
    input: SpatialAuthoringBatch<'input>,
    nodes: NodeAuthoringBatch<'input>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedSpatialBatch<'input, C::Reservation>, SpatialAuthoringFailure<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    validate_spatial_batch_inner(input, nodes, limits, control)
}

fn validate_spatial_batch_inner<'input, C>(
    input: SpatialAuthoringBatch<'input>,
    nodes: NodeAuthoringBatch<'input>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedSpatialBatch<'input, C::Reservation>, SpatialAuthoringFailure<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    let mut context = SpatialValidationContext::new(limits, control);
    context.poll_now()?;
    context.run_unit(SceneValidationWork::Traversal, || {
        input
            .scene_space()
            .validate()
            .map_err(SpatialAuthoringSemanticError::InvalidSceneSpace)
    })??;

    let camera_count = collection_count(input.cameras().len(), "cameras")?;
    let light_count = collection_count(input.lights().len(), "lights")?;
    let entity_count = u64::from(camera_count)
        .checked_add(u64::from(light_count))
        .ok_or(SpatialAuthoringSemanticError::AccountingOverflow {
            dimension: "spatial entities",
        })?;
    enforce_limit::<C::Error>(
        SpatialAuthoringLimit::Entities,
        entity_count,
        limits.max_entities(),
    )?;

    let camera_plan = scratch_plan::<u32>(input.cameras().len(), "camera placement counts")?;
    let light_plan = scratch_plan::<u32>(input.lights().len(), "light placement counts")?;
    let scratch_envelope = capacity_envelope(camera_plan, "camera placement counts")?
        .checked_add(capacity_envelope(light_plan, "light placement counts")?)
        .ok_or(SpatialAuthoringSemanticError::AccountingOverflow {
            dimension: "spatial placement scratch envelope",
        })?;
    let mut reservation = context.reserve_scratch(scratch_envelope)?;
    let (mut camera_placement_counts, camera_bytes) =
        allocate_scratch(camera_plan, "camera placement counts")?;
    let (mut light_placement_counts, light_bytes) =
        allocate_scratch(light_plan, "light placement counts")?;
    for _ in 0..input.cameras().len() {
        context.run_unit(SceneValidationWork::Traversal, || {
            camera_placement_counts.push(0_u32);
        })?;
    }
    for _ in 0..input.lights().len() {
        context.run_unit(SceneValidationWork::Traversal, || {
            light_placement_counts.push(0_u32);
        })?;
    }
    let allocated_scratch_bytes = camera_bytes.checked_add(light_bytes).ok_or(
        SpatialAuthoringSemanticError::AccountingOverflow {
            dimension: "spatial placement scratch",
        },
    )?;
    let peak_scratch_bytes = scratch_envelope.max(allocated_scratch_bytes);
    context.resize_scratch(&mut reservation, allocated_scratch_bytes)?;

    let mut semantic_cells = 9_u64;
    let directory_reference_bytes = [
        tangent_schema_reference_bytes(input.scene_space().tangent_basis.schema_id().is_some()),
        tangent_schema_reference_bytes(
            input
                .scene_space()
                .tangent_handedness_encoding
                .schema_id()
                .is_some(),
        ),
        tangent_schema_reference_bytes(camera_count != 0),
        tangent_schema_reference_bytes(light_count != 0),
    ]
    .into_iter()
    .try_fold(0_u64, u64::checked_add)
    .ok_or(SpatialAuthoringSemanticError::AccountingOverflow {
        dimension: "spatial directory references",
    })?;
    let mut canonical_bytes = SPATIAL_DIRECTORY_FIXED_BYTES
        .checked_sub(4 * LOGICAL_REFERENCE_BYTES)
        .and_then(|bytes| bytes.checked_add(directory_reference_bytes))
        .and_then(|bytes| bytes.checked_add(LOGICAL_REFERENCE_BYTES))
        .and_then(|bytes| bytes.checked_add(SYNTHETIC_NODE_ATTACHMENT_BYTES))
        .ok_or(SpatialAuthoringSemanticError::AccountingOverflow {
            dimension: "spatial fixed canonical footprint",
        })?;
    canonical_bytes = canonical_bytes
        .checked_add(optional_scalar_slot_bytes(
            input.scene_space().unit_scale_to_meters.is_some(),
            LOGICAL_F64_BYTES,
        ))
        .ok_or(SpatialAuthoringSemanticError::AccountingOverflow {
            dimension: "SceneSpace unit-scale slot",
        })?;
    if input.scene_space().unit_scale_to_meters.is_some() {
        semantic_cells = semantic_cells.checked_add(1).ok_or(
            SpatialAuthoringSemanticError::AccountingOverflow {
                dimension: "SceneSpace semantic cells",
            },
        )?;
    }

    let tangent_basis_schema_name_bytes = schema_name_bytes(
        input.scene_space().tangent_basis.schema_id(),
        "tangent-basis schema name",
    )?;
    let tangent_encoding_schema_name_bytes = schema_name_bytes(
        input.scene_space().tangent_handedness_encoding.schema_id(),
        "tangent-encoding schema name",
    )?;
    let schema_name_bytes = u64::from(tangent_basis_schema_name_bytes)
        .checked_add(u64::from(tangent_encoding_schema_name_bytes))
        .ok_or(SpatialAuthoringSemanticError::AccountingOverflow {
            dimension: "SceneSpace schema names",
        })?;
    semantic_cells = semantic_cells.checked_add(schema_name_bytes).ok_or(
        SpatialAuthoringSemanticError::AccountingOverflow {
            dimension: "SceneSpace schema-name semantic cells",
        },
    )?;
    canonical_bytes = canonical_bytes.checked_add(schema_name_bytes).ok_or(
        SpatialAuthoringSemanticError::AccountingOverflow {
            dimension: "SceneSpace schema-name bytes",
        },
    )?;
    // The fixed directory and schema extents are shape facts available before
    // payload traversal. Admit that envelope once; subsequent authored facts
    // enforce their incremental limits inside the action that observes them.
    enforce_limit::<C::Error>(
        SpatialAuthoringLimit::SemanticCells,
        semantic_cells,
        limits.max_payload_cells(),
    )?;
    enforce_limit::<C::Error>(
        SpatialAuthoringLimit::CanonicalBytes,
        canonical_bytes,
        limits.max_canonical_bytes(),
    )?;
    for byte in input
        .scene_space()
        .tangent_basis
        .schema_id()
        .into_iter()
        .flat_map(|schema| schema.name().as_str().as_bytes())
        .chain(
            input
                .scene_space()
                .tangent_handedness_encoding
                .schema_id()
                .into_iter()
                .flat_map(|schema| schema.name().as_str().as_bytes()),
        )
    {
        context.run_unit(SceneValidationWork::PayloadCell, || *byte)?;
    }

    for (camera, authored) in input.cameras().iter().enumerate() {
        context.run_unit(SceneValidationWork::Entity, || {
            let projection = authored.projection();
            let (optional_values, optional_slot_bytes) = match projection.kind() {
                CameraProjectionKind::Perspective => {
                    let Some((_fov, aspect, _near, far)) = projection.perspective_parameters()
                    else {
                        return Err(SpatialAuthoringSemanticError::InvalidCameraProjection {
                            camera,
                        });
                    };
                    (
                        u64::from(aspect.is_some()) + u64::from(far.is_some()),
                        optional_scalar_slot_bytes(aspect.is_some(), LOGICAL_F32_BYTES)
                            + optional_scalar_slot_bytes(far.is_some(), LOGICAL_F32_BYTES),
                    )
                }
                CameraProjectionKind::Orthographic => {
                    if projection.orthographic_parameters().is_none() {
                        return Err(SpatialAuthoringSemanticError::InvalidCameraProjection {
                            camera,
                        });
                    }
                    (2, 2 * LOGICAL_F32_BYTES)
                }
            };
            let final_semantic_cells = semantic_cells
                .checked_add(4_u64.checked_add(optional_values).ok_or(
                    SpatialAuthoringSemanticError::AccountingOverflow {
                        dimension: "camera semantic cells",
                    },
                )?)
                .ok_or(SpatialAuthoringSemanticError::AccountingOverflow {
                    dimension: "camera semantic cells",
                })?;
            let final_canonical_bytes = canonical_bytes
                .checked_add(CAMERA_ROW_FIXED_BYTES)
                .and_then(|bytes| bytes.checked_add(optional_slot_bytes))
                .ok_or(SpatialAuthoringSemanticError::AccountingOverflow {
                    dimension: "camera canonical footprint",
                })?;
            enforce_semantic_limit(
                SpatialAuthoringLimit::SemanticCells,
                final_semantic_cells,
                limits.max_payload_cells(),
            )?;
            enforce_semantic_limit(
                SpatialAuthoringLimit::CanonicalBytes,
                final_canonical_bytes,
                limits.max_canonical_bytes(),
            )?;
            semantic_cells = final_semantic_cells;
            canonical_bytes = final_canonical_bytes;
            Ok::<(), SpatialAuthoringSemanticError>(())
        })??;
    }

    for (light, authored) in input.lights().iter().enumerate() {
        context.run_unit(SceneValidationWork::Entity, || {
            let emission = authored.emission();
            validate_canonical_light_emission(emission.tint(), emission.intensity()).map_err(
                |source| SpatialAuthoringSemanticError::InvalidLightEmission { light, source },
            )?;
            let kind = authored.kind();
            let optional_values = match kind.category() {
                LightKindCategory::Directional => 0,
                LightKindCategory::Point => u64::from(kind.range().is_some()),
                LightKindCategory::Spot => 2 + u64::from(kind.range().is_some()),
            };
            let final_semantic_cells = semantic_cells
                .checked_add(6_u64.checked_add(optional_values).ok_or(
                    SpatialAuthoringSemanticError::AccountingOverflow {
                        dimension: "light semantic cells",
                    },
                )?)
                .ok_or(SpatialAuthoringSemanticError::AccountingOverflow {
                    dimension: "light semantic cells",
                })?;
            let optional_slot_bytes = match kind.category() {
                LightKindCategory::Directional => 3 * LOGICAL_U8_BYTES,
                LightKindCategory::Point => {
                    tangent_schema_reference_bytes(kind.range().is_some()) + (2 * LOGICAL_U8_BYTES)
                }
                LightKindCategory::Spot => {
                    tangent_schema_reference_bytes(kind.range().is_some()) + (2 * LOGICAL_F32_BYTES)
                }
            };
            let final_canonical_bytes = canonical_bytes
                .checked_add(LIGHT_ROW_FIXED_BYTES)
                .and_then(|bytes| bytes.checked_add(optional_slot_bytes))
                .ok_or(SpatialAuthoringSemanticError::AccountingOverflow {
                    dimension: "light canonical footprint",
                })?;
            enforce_semantic_limit(
                SpatialAuthoringLimit::SemanticCells,
                final_semantic_cells,
                limits.max_payload_cells(),
            )?;
            enforce_semantic_limit(
                SpatialAuthoringLimit::CanonicalBytes,
                final_canonical_bytes,
                limits.max_canonical_bytes(),
            )?;
            semantic_cells = final_semantic_cells;
            canonical_bytes = final_canonical_bytes;
            Ok::<(), SpatialAuthoringSemanticError>(())
        })??;
    }

    let mut edge_count = 0_u64;
    let mut reference_count = 0_u64;
    for (node, authored) in nodes.nodes().iter().enumerate() {
        context.run_unit(SceneValidationWork::Traversal, || {
            let final_canonical_bytes = canonical_bytes
                .checked_add(node_attachment_bytes(authored.camera()))
                .and_then(|bytes| bytes.checked_add(node_attachment_bytes(authored.light())))
                .ok_or(SpatialAuthoringSemanticError::AccountingOverflow {
                    dimension: "node spatial attachments",
                })?;
            let final_semantic_cells = semantic_cells.checked_add(2).ok_or(
                SpatialAuthoringSemanticError::AccountingOverflow {
                    dimension: "node attachment semantic cells",
                },
            )?;
            enforce_semantic_limit(
                SpatialAuthoringLimit::SemanticCells,
                final_semantic_cells,
                limits.max_payload_cells(),
            )?;
            enforce_semantic_limit(
                SpatialAuthoringLimit::CanonicalBytes,
                final_canonical_bytes,
                limits.max_canonical_bytes(),
            )?;
            semantic_cells = final_semantic_cells;
            canonical_bytes = final_canonical_bytes;
            Ok::<(), SpatialAuthoringSemanticError>(())
        })??;
        if let Presence::Present(camera_id) = authored.camera() {
            context.run_unit(SceneValidationWork::GraphEdge, || {
                let index = usize::try_from(camera_id.index()).map_err(|_| {
                    SpatialAuthoringSemanticError::CameraOutOfRange {
                        node,
                        camera: camera_id.index(),
                        camera_count,
                    }
                })?;
                let count = camera_placement_counts.get_mut(index).ok_or(
                    SpatialAuthoringSemanticError::CameraOutOfRange {
                        node,
                        camera: camera_id.index(),
                        camera_count,
                    },
                )?;
                let final_placement_count = (*count).checked_add(1).ok_or(
                    SpatialAuthoringSemanticError::AccountingOverflow {
                        dimension: "camera placements",
                    },
                )?;
                let final_edge_count = edge_count.checked_add(1).ok_or(
                    SpatialAuthoringSemanticError::AccountingOverflow {
                        dimension: "spatial edges",
                    },
                )?;
                enforce_semantic_limit(
                    SpatialAuthoringLimit::Edges,
                    final_edge_count,
                    limits.max_edges(),
                )?;
                *count = final_placement_count;
                edge_count = final_edge_count;
                Ok::<(), SpatialAuthoringSemanticError>(())
            })??;
            context.run_unit(SceneValidationWork::Reference, || {
                let final_reference_count = reference_count.checked_add(1).ok_or(
                    SpatialAuthoringSemanticError::AccountingOverflow {
                        dimension: "spatial references",
                    },
                )?;
                let final_semantic_cells = semantic_cells.checked_add(1).ok_or(
                    SpatialAuthoringSemanticError::AccountingOverflow {
                        dimension: "camera attachment semantic cells",
                    },
                )?;
                enforce_semantic_limit(
                    SpatialAuthoringLimit::References,
                    final_reference_count,
                    limits.max_references(),
                )?;
                enforce_semantic_limit(
                    SpatialAuthoringLimit::SemanticCells,
                    final_semantic_cells,
                    limits.max_payload_cells(),
                )?;
                reference_count = final_reference_count;
                semantic_cells = final_semantic_cells;
                Ok::<(), SpatialAuthoringSemanticError>(())
            })??;
        }
        if let Presence::Present(light_id) = authored.light() {
            context.run_unit(SceneValidationWork::GraphEdge, || {
                let index = usize::try_from(light_id.index()).map_err(|_| {
                    SpatialAuthoringSemanticError::LightOutOfRange {
                        node,
                        light: light_id.index(),
                        light_count,
                    }
                })?;
                let count = light_placement_counts.get_mut(index).ok_or(
                    SpatialAuthoringSemanticError::LightOutOfRange {
                        node,
                        light: light_id.index(),
                        light_count,
                    },
                )?;
                let final_placement_count = (*count).checked_add(1).ok_or(
                    SpatialAuthoringSemanticError::AccountingOverflow {
                        dimension: "light placements",
                    },
                )?;
                let final_edge_count = edge_count.checked_add(1).ok_or(
                    SpatialAuthoringSemanticError::AccountingOverflow {
                        dimension: "spatial edges",
                    },
                )?;
                enforce_semantic_limit(
                    SpatialAuthoringLimit::Edges,
                    final_edge_count,
                    limits.max_edges(),
                )?;
                *count = final_placement_count;
                edge_count = final_edge_count;
                Ok::<(), SpatialAuthoringSemanticError>(())
            })??;
            context.run_unit(SceneValidationWork::Reference, || {
                let final_reference_count = reference_count.checked_add(1).ok_or(
                    SpatialAuthoringSemanticError::AccountingOverflow {
                        dimension: "spatial references",
                    },
                )?;
                let final_semantic_cells = semantic_cells.checked_add(1).ok_or(
                    SpatialAuthoringSemanticError::AccountingOverflow {
                        dimension: "light attachment semantic cells",
                    },
                )?;
                enforce_semantic_limit(
                    SpatialAuthoringLimit::References,
                    final_reference_count,
                    limits.max_references(),
                )?;
                enforce_semantic_limit(
                    SpatialAuthoringLimit::SemanticCells,
                    final_semantic_cells,
                    limits.max_payload_cells(),
                )?;
                reference_count = final_reference_count;
                semantic_cells = final_semantic_cells;
                Ok::<(), SpatialAuthoringSemanticError>(())
            })??;
        }
    }

    for (camera, &placements) in camera_placement_counts.iter().enumerate() {
        context.run_unit(SceneValidationWork::Traversal, || {
            if placements == 0 {
                Err(SpatialAuthoringSemanticError::OrphanCamera { camera })
            } else {
                Ok(())
            }
        })??;
    }
    for (light, &placements) in light_placement_counts.iter().enumerate() {
        context.run_unit(SceneValidationWork::Traversal, || {
            if placements == 0 {
                Err(SpatialAuthoringSemanticError::OrphanLight { light })
            } else {
                Ok(())
            }
        })??;
    }

    // Placement counters are prepublication scratch. Central raw validation
    // independently derives the reverse relation from retained node rows.
    drop(camera_placement_counts);
    drop(light_placement_counts);
    let retained_scratch_bytes = 0;
    context.resize_scratch(&mut reservation, retained_scratch_bytes)?;

    context.finish()?;

    let compiled = SpatialValidationStorage {
        camera_count,
        light_count,
        entity_count,
        edge_count,
        reference_count,
        semantic_cell_count: semantic_cells,
        canonical_bytes,
        validation_work: context.work,
        peak_scratch_bytes,
        retained_scratch_bytes,
        tangent_basis_schema_name_bytes,
        tangent_encoding_schema_name_bytes,
        reservation,
    };
    Ok(ValidatedSpatialBatch::new(input, nodes, compiled))
}

fn collection_count(
    count: usize,
    collection: &'static str,
) -> Result<u32, SpatialAuthoringSemanticError> {
    u32::try_from(count)
        .map_err(|_| SpatialAuthoringSemanticError::CountDomainTooLarge { collection })
}

fn schema_name_bytes(
    schema: Option<&crate::extension::ExtensionSchemaId>,
    dimension: &'static str,
) -> Result<u32, SpatialAuthoringSemanticError> {
    match schema {
        Some(schema) => u32::try_from(schema.name().as_str().len())
            .map_err(|_| SpatialAuthoringSemanticError::AccountingOverflow { dimension }),
        None => Ok(0),
    }
}

fn scratch_plan<T>(
    count: usize,
    resource: &'static str,
) -> Result<CapacityPlan, SpatialAuthoringSemanticError> {
    CapacityPlan::for_type::<T>(0, 0, count)
        .map_err(|cause| SpatialAuthoringSemanticError::ScratchCapacity { resource, cause })
}

fn capacity_envelope(
    plan: CapacityPlan,
    resource: &'static str,
) -> Result<u64, SpatialAuthoringSemanticError> {
    plan.envelope_delta_bytes()
        .map_err(|cause| SpatialAuthoringSemanticError::ScratchCapacity { resource, cause })
}

fn allocate_scratch<T>(
    plan: CapacityPlan,
    resource: &'static str,
) -> Result<(Vec<T>, u64), SpatialAuthoringSemanticError> {
    plan.verify_preflight(0, 0)
        .map_err(|cause| SpatialAuthoringSemanticError::ScratchCapacity { resource, cause })?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| SpatialAuthoringSemanticError::ScratchAllocation { resource })?;
    let bytes = plan
        .finish(values.len(), values.capacity())
        .map_err(|cause| SpatialAuthoringSemanticError::ScratchCapacity { resource, cause })?;
    Ok((values, bytes))
}

fn node_attachment_bytes<T>(attachment: Presence<T>) -> u64 {
    match attachment {
        Presence::Absent => 3 * LOGICAL_U8_BYTES,
        Presence::Present(_) => LOGICAL_U8_BYTES + LOGICAL_U32_BYTES + LOGICAL_REFERENCE_BYTES,
    }
}

const fn tangent_schema_reference_bytes(present: bool) -> u64 {
    if present {
        LOGICAL_REFERENCE_BYTES
    } else {
        LOGICAL_U8_BYTES
    }
}

const fn optional_scalar_slot_bytes(present: bool, scalar_bytes: u64) -> u64 {
    if present {
        scalar_bytes
    } else {
        LOGICAL_U8_BYTES
    }
}

fn enforce_limit<E>(
    limit: SpatialAuthoringLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), SpatialAuthoringFailure<E>>
where
    E: Error + Send + Sync + 'static,
{
    enforce_semantic_limit(limit, actual, maximum).map_err(SpatialAuthoringFailure::Semantic)
}

fn enforce_semantic_limit(
    limit: SpatialAuthoringLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), SpatialAuthoringSemanticError> {
    if actual <= maximum {
        Ok(())
    } else {
        Err(SpatialAuthoringSemanticError::LimitExceeded {
            limit,
            actual,
            maximum,
        })
    }
}
