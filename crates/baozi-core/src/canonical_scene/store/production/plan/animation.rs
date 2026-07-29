#![allow(
    clippy::result_large_err,
    reason = "animation authoring failures retain bounded control evidence inline without a failure-path allocation"
)]

use std::error::Error;
use std::fmt;
use std::ops::Range;

use baozi_operation::{
    CapacityPlan, CapacityPlanError, OperationOwner, ProgressPhase, ProgressUnit,
    RetainedStoreReservation,
};
use thiserror::Error;

use crate::entity::AuthoredTransformKind;
use crate::math::Quat;
use crate::{
    SCENE_VALIDATION_POLL_INTERVAL, SceneReservationKind, SceneValidationControl,
    SceneValidationLimits, SceneValidationWork,
};

use super::super::control::{OperationAuthoringControl, OperationAuthoringControlError};
use super::super::input::animation::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationExtrapolationAuthoring, AnimationInterpolationAuthoring, AnimationTargetAuthoring,
    AnimationTimeDomainAuthoring,
};
use super::super::input::geometry::AuthoringRange;
use super::super::input::{NodeLocalId, Presence};
use super::ValidatedNodeBatch;
use super::deformation::ValidatedDeformationBatch;

const LOGICAL_U8_BYTES: u64 = 1;
const LOGICAL_U32_BYTES: u64 = 4;
const LOGICAL_U64_BYTES: u64 = 8;
const LOGICAL_F32_BYTES: u64 = 4;
const LOGICAL_F64_BYTES: u64 = 8;
const LOGICAL_REFERENCE_BYTES: u64 = 4;

const ANIMATION_DIRECTORY_SCALAR_BYTES: u64 =
    (3 * LOGICAL_U32_BYTES) + (4 * LOGICAL_U64_BYTES) + (12 * LOGICAL_U8_BYTES);
const ANIMATION_CLIP_BASE_BYTES: u64 =
    (3 * LOGICAL_U32_BYTES) + LOGICAL_F64_BYTES + (5 * LOGICAL_U8_BYTES);
const ANIMATION_CHANNEL_BASE_BYTES: u64 = (12 * LOGICAL_U32_BYTES)
    + (6 * LOGICAL_U8_BYTES)
    + (3 * LOGICAL_REFERENCE_BYTES)
    + (2 * LOGICAL_U8_BYTES);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum AnimationAuthoringLimit {
    Entities,
    Edges,
    References,
    SemanticCells,
    CanonicalBytes,
    Work,
    ScratchBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum AnimationRateKind {
    Frames,
    Ticks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum AnimationSamplePart {
    Values,
    InTangents,
    OutTangents,
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum AnimationAuthoringSemanticError {
    #[error("animation authoring count for {collection} exceeds the u32 canonical domain")]
    CountDomainTooLarge { collection: &'static str },
    #[error("{limit:?} value {actual} exceeds limit {maximum}")]
    LimitExceeded {
        limit: AnimationAuthoringLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("animation authoring accounting overflowed for {dimension}")]
    AccountingOverflow { dimension: &'static str },
    #[error(
        "animation clip {clip} channel range starts at {actual}; canonical owner coverage expected {expected}"
    )]
    ClipChannelRangeOrder {
        clip: usize,
        expected: u32,
        actual: u32,
    },
    #[error("animation clip {clip} channel range exceeds channel count {channel_count}")]
    ClipChannelRangeOutOfBounds { clip: usize, channel_count: usize },
    #[error("flat animation channel table has {unowned} rows without exactly one clip owner")]
    UnownedChannels { unowned: usize },
    #[error("animation clip {clip} duration must be finite and non-negative")]
    InvalidDuration { clip: usize },
    #[error("animation clip {clip} {kind:?} rate must be finite and positive")]
    InvalidRate {
        clip: usize,
        kind: AnimationRateKind,
    },
    #[error("animation clip {clip} channel {channel} has no authored key time")]
    EmptyChannel { clip: usize, channel: usize },
    #[error("animation clip {clip} channel {channel} key time {key} is not finite")]
    NonFiniteTime {
        clip: usize,
        channel: usize,
        key: usize,
    },
    #[error(
        "animation clip {clip} channel {channel} key times {previous_key} and {key} are not strictly increasing"
    )]
    TimesNotStrictlyIncreasing {
        clip: usize,
        channel: usize,
        previous_key: usize,
        key: usize,
    },
    #[error(
        "animation clip {clip} channel {channel} targets node {node}, but only {node_count} authored nodes exist"
    )]
    NodeTargetOutOfRange {
        clip: usize,
        channel: usize,
        node: u32,
        node_count: usize,
    },
    #[error(
        "animation clip {clip} channel {channel} targets matrix-authored node {node} through a TRS channel"
    )]
    MatrixAuthoredNodeTarget {
        clip: usize,
        channel: usize,
        node: u32,
    },
    #[error(
        "animation clip {clip} channel {channel} targets mesh instance {mesh_instance}, but only {mesh_instance_count} instances exist"
    )]
    MeshInstanceTargetOutOfRange {
        clip: usize,
        channel: usize,
        mesh_instance: u32,
        mesh_instance_count: usize,
    },
    #[error(
        "animation clip {clip} channel {channel} targets mesh instance {mesh_instance} without logical morph targets"
    )]
    MorphTargetWidthZero {
        clip: usize,
        channel: usize,
        mesh_instance: u32,
    },
    #[error(
        "animation clip {clip} channel {channel} has {actual} value scalars; expected {expected}"
    )]
    ValueShape {
        clip: usize,
        channel: usize,
        expected: u64,
        actual: usize,
    },
    #[error("animation clip {clip} channel {channel} requires {part:?} for cubic interpolation")]
    MissingTangents {
        clip: usize,
        channel: usize,
        part: AnimationSamplePart,
    },
    #[error(
        "animation clip {clip} channel {channel} supplies {part:?} for non-cubic interpolation"
    )]
    UnexpectedTangents {
        clip: usize,
        channel: usize,
        part: AnimationSamplePart,
    },
    #[error(
        "animation clip {clip} channel {channel} has {actual} {part:?} scalars; expected {expected}"
    )]
    TangentShape {
        clip: usize,
        channel: usize,
        part: AnimationSamplePart,
        expected: u64,
        actual: usize,
    },
    #[error(
        "animation clip {clip} channel {channel} {part:?} sample {sample}, component {component} is not finite"
    )]
    NonFiniteSample {
        clip: usize,
        channel: usize,
        part: AnimationSamplePart,
        sample: usize,
        component: usize,
    },
    #[error(
        "animation clip {clip} channel {channel} rotation sample {sample} is not a unit quaternion"
    )]
    NonUnitQuaternion {
        clip: usize,
        channel: usize,
        sample: usize,
    },
    #[error(
        "animation clip {clip} channel {duplicate_channel} duplicates channel {first_channel} target {target:?}"
    )]
    DuplicateTarget {
        clip: usize,
        target: AnimationTargetAuthoring,
        first_channel: usize,
        duplicate_channel: usize,
    },
    #[error("failed to plan animation scratch capacity for {resource}")]
    ScratchCapacity {
        resource: &'static str,
        #[source]
        cause: CapacityPlanError,
    },
    #[error("failed to allocate animation scratch storage for {resource}")]
    ScratchAllocation { resource: &'static str },
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum AnimationAuthoringFailure<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Semantic(#[from] AnimationAuthoringSemanticError),
    #[error("animation authoring validation control failed")]
    Control(#[source] E),
}

/// Cross-domain facts required to validate animation targets.
///
/// Mesh-instance morph widths are projected from an already validated
/// deformation batch, so animation validation cannot observe unchecked
/// geometry-to-morph associations.
pub(in crate::canonical_scene::store) struct AnimationTargetCatalog<'view, 'input, NR, DR> {
    nodes: &'view ValidatedNodeBatch<'input, NR>,
    deformation: &'view ValidatedDeformationBatch<'input, DR>,
}

impl<'view, 'input, NR, DR> AnimationTargetCatalog<'view, 'input, NR, DR> {
    pub(in crate::canonical_scene::store) const fn new(
        nodes: &'view ValidatedNodeBatch<'input, NR>,
        deformation: &'view ValidatedDeformationBatch<'input, DR>,
    ) -> Self {
        Self { nodes, deformation }
    }

    pub(in crate::canonical_scene::store) fn authored_node_count(&self) -> usize {
        self.nodes.input().nodes().len()
    }

    pub(in crate::canonical_scene::store) fn authored_node(
        &self,
        node: usize,
    ) -> Option<super::super::input::NodeAuthoringNode> {
        self.nodes.input().nodes().get(node).copied()
    }

    pub(in crate::canonical_scene::store) fn canonical_node_index(
        &self,
        node: NodeLocalId,
    ) -> Option<u32> {
        self.nodes
            .canonical_index_by_local()
            .get(usize::try_from(node.index()).ok()?)
            .copied()
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_count(&self) -> usize {
        self.deformation.input().mesh_instances().len()
    }

    pub(in crate::canonical_scene::store) fn mesh_instance_morph_width(
        &self,
        mesh_instance: usize,
    ) -> Option<u32> {
        let logical_mesh = usize::try_from(
            self.deformation
                .mesh_instance(mesh_instance)?
                .logical_mesh(),
        )
        .ok()?;
        self.deformation
            .logical_morph(logical_mesh)
            .map(|morph| morph.target_count())
    }

    fn seen_target_count(&self, channel_count: usize) -> Option<usize> {
        if channel_count == 0 {
            return Some(0);
        }
        self.authored_node_count()
            .checked_mul(3)?
            .checked_add(self.mesh_instance_count())
    }
}

#[derive(Debug)]
struct ValidatedAnimationClipPlan {
    input_index: usize,
    channels: AuthoringRange,
}

#[derive(Debug)]
struct ValidatedAnimationChannelPlan {
    input_index: usize,
    canonical_target_index: u32,
    value_width: u32,
    time_offset: u32,
    time_count: u32,
    value_offset: u32,
    value_count: u32,
    in_tangent_offset: Presence<u32>,
    in_tangent_count: u32,
    out_tangent_offset: Presence<u32>,
    out_tangent_count: u32,
}

#[derive(Debug, Clone, Copy)]
struct SeenAnimationTarget {
    clip: u32,
    channel: u32,
}

impl SeenAnimationTarget {
    const UNUSED: Self = Self {
        clip: u32::MAX,
        channel: u32::MAX,
    };
}

pub(in crate::canonical_scene::store) struct ValidatedAnimationClip<'batch, 'input> {
    input: AnimationClipAuthoring,
    plan: &'batch ValidatedAnimationClipPlan,
    channels: &'batch [ValidatedAnimationChannelPlan],
    input_channels: &'input [AnimationChannelAuthoring<'input>],
}

impl<'batch, 'input> ValidatedAnimationClip<'batch, 'input> {
    pub(in crate::canonical_scene::store) const fn input_index(&self) -> usize {
        self.plan.input_index
    }

    pub(in crate::canonical_scene::store) const fn duration(&self) -> f64 {
        self.input.duration()
    }

    pub(in crate::canonical_scene::store) const fn time_domain(
        &self,
    ) -> AnimationTimeDomainAuthoring {
        self.input.time_domain()
    }

    pub(in crate::canonical_scene::store) const fn channel_range(&self) -> AuthoringRange {
        self.plan.channels
    }

    pub(in crate::canonical_scene::store) fn channels(
        &self,
    ) -> impl ExactSizeIterator<Item = ValidatedAnimationChannel<'_, 'input>> + '_ {
        self.channels
            .iter()
            .zip(self.input_channels)
            .map(|(plan, input)| ValidatedAnimationChannel { input, plan })
    }
}

pub(in crate::canonical_scene::store) struct ValidatedAnimationChannel<'batch, 'input> {
    input: &'batch AnimationChannelAuthoring<'input>,
    plan: &'batch ValidatedAnimationChannelPlan,
}

impl<'input> ValidatedAnimationChannel<'_, 'input> {
    pub(in crate::canonical_scene::store) const fn input_index(&self) -> usize {
        self.plan.input_index
    }

    pub(in crate::canonical_scene::store) const fn target(&self) -> AnimationTargetAuthoring {
        self.input.target()
    }

    pub(in crate::canonical_scene::store) const fn interpolation(
        &self,
    ) -> AnimationInterpolationAuthoring {
        self.input.interpolation()
    }

    pub(in crate::canonical_scene::store) const fn pre_extrapolation(
        &self,
    ) -> AnimationExtrapolationAuthoring {
        self.input.pre_extrapolation()
    }

    pub(in crate::canonical_scene::store) const fn post_extrapolation(
        &self,
    ) -> AnimationExtrapolationAuthoring {
        self.input.post_extrapolation()
    }

    pub(in crate::canonical_scene::store) const fn times(&self) -> &'input [f64] {
        self.input.times()
    }

    pub(in crate::canonical_scene::store) const fn values(&self) -> &'input [f32] {
        self.input.values()
    }

    pub(in crate::canonical_scene::store) const fn in_tangents(&self) -> Presence<&'input [f32]> {
        self.input.in_tangents()
    }

    pub(in crate::canonical_scene::store) const fn out_tangents(&self) -> Presence<&'input [f32]> {
        self.input.out_tangents()
    }

    pub(in crate::canonical_scene::store) const fn value_width(&self) -> u32 {
        self.plan.value_width
    }

    pub(in crate::canonical_scene::store) const fn canonical_target_index(&self) -> u32 {
        self.plan.canonical_target_index
    }

    pub(in crate::canonical_scene::store) const fn time_offset(&self) -> u32 {
        self.plan.time_offset
    }

    pub(in crate::canonical_scene::store) const fn time_count(&self) -> u32 {
        self.plan.time_count
    }

    pub(in crate::canonical_scene::store) const fn value_offset(&self) -> u32 {
        self.plan.value_offset
    }

    pub(in crate::canonical_scene::store) const fn value_count(&self) -> u32 {
        self.plan.value_count
    }

    pub(in crate::canonical_scene::store) const fn in_tangent_offset(&self) -> Presence<u32> {
        self.plan.in_tangent_offset
    }

    pub(in crate::canonical_scene::store) const fn in_tangent_count(&self) -> u32 {
        self.plan.in_tangent_count
    }

    pub(in crate::canonical_scene::store) const fn out_tangent_offset(&self) -> Presence<u32> {
        self.plan.out_tangent_offset
    }

    pub(in crate::canonical_scene::store) const fn out_tangent_count(&self) -> u32 {
        self.plan.out_tangent_count
    }
}

#[must_use = "dropping the validated animation batch releases its compiled-plan reservation"]
pub(in crate::canonical_scene::store) struct ValidatedAnimationBatch<'input, R> {
    input: AnimationAuthoringBatch<'input>,
    clip_count: u32,
    channel_count: u32,
    time_scalar_count: u64,
    value_scalar_count: u64,
    in_tangent_scalar_count: u64,
    out_tangent_scalar_count: u64,
    payload_block_count: u32,
    entity_count: u64,
    edge_count: u64,
    reference_count: u64,
    semantic_cell_count: u64,
    canonical_bytes: u64,
    validation_work: u64,
    scratch_bytes: u64,
    clip_plans: Vec<ValidatedAnimationClipPlan>,
    channel_plans: Vec<ValidatedAnimationChannelPlan>,
    // Every compiled-plan allocation above must drop before its accounting token.
    reservation: R,
}

impl<'input, R> ValidatedAnimationBatch<'input, R> {
    pub(in crate::canonical_scene::store) const fn input(&self) -> AnimationAuthoringBatch<'input> {
        self.input
    }

    pub(in crate::canonical_scene::store) const fn clip_count(&self) -> u32 {
        self.clip_count
    }

    pub(in crate::canonical_scene::store) const fn channel_count(&self) -> u32 {
        self.channel_count
    }

    pub(in crate::canonical_scene::store) const fn time_scalar_count(&self) -> u64 {
        self.time_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn value_scalar_count(&self) -> u64 {
        self.value_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn in_tangent_scalar_count(&self) -> u64 {
        self.in_tangent_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn out_tangent_scalar_count(&self) -> u64 {
        self.out_tangent_scalar_count
    }

    pub(in crate::canonical_scene::store) const fn payload_block_count(&self) -> u32 {
        self.payload_block_count
    }

    pub(in crate::canonical_scene::store) const fn entity_count(&self) -> u64 {
        self.entity_count
    }

    pub(in crate::canonical_scene::store) const fn edge_count(&self) -> u64 {
        self.edge_count
    }

    pub(in crate::canonical_scene::store) const fn reference_count(&self) -> u64 {
        self.reference_count
    }

    pub(in crate::canonical_scene::store) const fn semantic_cell_count(&self) -> u64 {
        self.semantic_cell_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_bytes(&self) -> u64 {
        self.canonical_bytes
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.validation_work
    }

    pub(in crate::canonical_scene::store) const fn scratch_bytes(&self) -> u64 {
        self.scratch_bytes
    }

    pub(in crate::canonical_scene::store) fn clip(
        &self,
        index: usize,
    ) -> Option<ValidatedAnimationClip<'_, 'input>> {
        let plan = self.clip_plans.get(index)?;
        let input = *self.input.clips().get(plan.input_index)?;
        let range = checked_range(plan.channels, self.channel_plans.len()).ok()?;
        Some(ValidatedAnimationClip {
            input,
            plan,
            channels: self.channel_plans.get(range.clone())?,
            input_channels: self.input.channels().get(range)?,
        })
    }

    pub(in crate::canonical_scene::store) fn channel(
        &self,
        index: usize,
    ) -> Option<ValidatedAnimationChannel<'_, 'input>> {
        let plan = self.channel_plans.get(index)?;
        let input = self.input.channels().get(plan.input_index)?;
        Some(ValidatedAnimationChannel { input, plan })
    }
}

impl<R> fmt::Debug for ValidatedAnimationBatch<'_, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedAnimationBatch")
            .field("clip_count", &self.clip_count)
            .field("channel_count", &self.channel_count)
            .field("time_scalar_count", &self.time_scalar_count)
            .field("value_scalar_count", &self.value_scalar_count)
            .field("in_tangent_scalar_count", &self.in_tangent_scalar_count)
            .field("out_tangent_scalar_count", &self.out_tangent_scalar_count)
            .field("payload_block_count", &self.payload_block_count)
            .field("entity_count", &self.entity_count)
            .field("edge_count", &self.edge_count)
            .field("reference_count", &self.reference_count)
            .field("semantic_cell_count", &self.semantic_cell_count)
            .field("canonical_bytes", &self.canonical_bytes)
            .field("validation_work", &self.validation_work)
            .field("scratch_bytes", &self.scratch_bytes)
            .field("reservation", &"<held>")
            .finish()
    }
}

struct AnimationValidationStorage<R> {
    clip_plans: Vec<ValidatedAnimationClipPlan>,
    channel_plans: Vec<ValidatedAnimationChannelPlan>,
    seen_targets: Vec<SeenAnimationTarget>,
    // Every allocation above must drop before its accounting token.
    reservation: R,
}

type AnimationValidationResult<T, C> =
    Result<T, AnimationAuthoringFailure<<C as SceneValidationControl>::Error>>;

struct AnimationValidationContext<'control, C: SceneValidationControl> {
    control: &'control mut C,
    limits: SceneValidationLimits,
    work: u64,
    work_since_poll: u64,
    edges: u64,
    references: u64,
    semantic_cells: u64,
    time_scalars: u64,
    value_scalars: u64,
    in_tangent_scalars: u64,
    out_tangent_scalars: u64,
    canonical_bytes: u64,
}

impl<'control, C: SceneValidationControl> AnimationValidationContext<'control, C> {
    const fn new(limits: SceneValidationLimits, control: &'control mut C) -> Self {
        Self {
            control,
            limits,
            work: 0,
            work_since_poll: 0,
            edges: 0,
            references: 0,
            semantic_cells: 0,
            time_scalars: 0,
            value_scalars: 0,
            in_tangent_scalars: 0,
            out_tangent_scalars: 0,
            canonical_bytes: 0,
        }
    }

    fn poll_now(&mut self) -> AnimationValidationResult<(), C> {
        self.control
            .poll()
            .map_err(AnimationAuthoringFailure::Control)?;
        self.work_since_poll = 0;
        Ok(())
    }

    fn finish(&mut self) -> AnimationValidationResult<(), C> {
        self.poll_now()
    }

    fn work(&mut self, kind: SceneValidationWork, amount: u64) -> AnimationValidationResult<(), C> {
        let final_work = self.work.checked_add(amount).ok_or(
            AnimationAuthoringSemanticError::AccountingOverflow {
                dimension: "validation work",
            },
        )?;
        enforce_limit::<C::Error>(
            AnimationAuthoringLimit::Work,
            final_work,
            self.limits.max_work(),
        )?;

        let mut remaining = amount;
        while remaining > 0 {
            let until_poll = SCENE_VALIDATION_POLL_INTERVAL - self.work_since_poll;
            let chunk = remaining.min(until_poll);
            self.control
                .charge_work(kind, chunk)
                .map_err(AnimationAuthoringFailure::Control)?;
            self.work = self.work.checked_add(chunk).ok_or(
                AnimationAuthoringSemanticError::AccountingOverflow {
                    dimension: "validation work",
                },
            )?;
            self.work_since_poll += chunk;
            remaining -= chunk;
            if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
                self.poll_now()?;
            }
        }
        Ok(())
    }

    fn add_relation(&mut self) -> AnimationValidationResult<(), C> {
        let final_edges = self
            .edges
            .checked_add(1)
            .ok_or(AnimationAuthoringSemanticError::AccountingOverflow { dimension: "edges" })?;
        let final_references = self.references.checked_add(1).ok_or(
            AnimationAuthoringSemanticError::AccountingOverflow {
                dimension: "references",
            },
        )?;
        enforce_limit::<C::Error>(
            AnimationAuthoringLimit::Edges,
            final_edges,
            self.limits.max_edges(),
        )?;
        enforce_limit::<C::Error>(
            AnimationAuthoringLimit::References,
            final_references,
            self.limits.max_references(),
        )?;
        self.edges = final_edges;
        self.references = final_references;
        self.work(SceneValidationWork::GraphEdge, 1)?;
        self.work(SceneValidationWork::Reference, 1)
    }

    fn set_structural_bytes(&mut self, bytes: u64) -> AnimationValidationResult<(), C> {
        enforce_limit::<C::Error>(
            AnimationAuthoringLimit::CanonicalBytes,
            bytes,
            self.limits.max_canonical_bytes(),
        )?;
        self.canonical_bytes = bytes;
        Ok(())
    }

    fn add_structural_bytes(&mut self, bytes: u64) -> AnimationValidationResult<(), C> {
        let final_bytes = self.canonical_bytes.checked_add(bytes).ok_or(
            AnimationAuthoringSemanticError::AccountingOverflow {
                dimension: "canonical animation structural footprint",
            },
        )?;
        enforce_limit::<C::Error>(
            AnimationAuthoringLimit::CanonicalBytes,
            final_bytes,
            self.limits.max_canonical_bytes(),
        )?;
        self.canonical_bytes = final_bytes;
        Ok(())
    }

    fn add_payload(
        &mut self,
        part: AnimationPayloadPart,
        count: u64,
    ) -> AnimationValidationResult<(), C> {
        let final_cells = self.semantic_cells.checked_add(count).ok_or(
            AnimationAuthoringSemanticError::AccountingOverflow {
                dimension: "semantic cells",
            },
        )?;
        enforce_limit::<C::Error>(
            AnimationAuthoringLimit::SemanticCells,
            final_cells,
            self.limits.max_payload_cells(),
        )?;
        let width = match part {
            AnimationPayloadPart::Times => LOGICAL_F64_BYTES,
            AnimationPayloadPart::Values
            | AnimationPayloadPart::InTangents
            | AnimationPayloadPart::OutTangents => LOGICAL_F32_BYTES,
        };
        let bytes = checked_bytes(count, width, "canonical animation payload")?;
        let counter = match part {
            AnimationPayloadPart::Times => &mut self.time_scalars,
            AnimationPayloadPart::Values => &mut self.value_scalars,
            AnimationPayloadPart::InTangents => &mut self.in_tangent_scalars,
            AnimationPayloadPart::OutTangents => &mut self.out_tangent_scalars,
        };
        let reference_growth = if *counter == 0 && count != 0 { 3 } else { 0 };
        let final_canonical_bytes = self
            .canonical_bytes
            .checked_add(reference_growth)
            .and_then(|total| total.checked_add(bytes))
            .ok_or(AnimationAuthoringSemanticError::AccountingOverflow {
                dimension: "canonical animation logical footprint",
            })?;
        enforce_limit::<C::Error>(
            AnimationAuthoringLimit::CanonicalBytes,
            final_canonical_bytes,
            self.limits.max_canonical_bytes(),
        )?;
        let final_count = counter.checked_add(count).ok_or(
            AnimationAuthoringSemanticError::AccountingOverflow {
                dimension: "animation scalar count",
            },
        )?;
        let _ = scalar_offset(final_count, "animation scalar count")?;
        *counter = final_count;
        self.semantic_cells = final_cells;
        self.canonical_bytes = final_canonical_bytes;
        Ok(())
    }

    fn scratch_plan<T>(
        &self,
        count: usize,
        resource: &'static str,
    ) -> Result<CapacityPlan, AnimationAuthoringSemanticError> {
        CapacityPlan::for_type::<T>(0, 0, count)
            .map_err(|cause| AnimationAuthoringSemanticError::ScratchCapacity { resource, cause })
    }

    fn reserve_scratch(&mut self, bytes: u64) -> AnimationValidationResult<C::Reservation, C> {
        enforce_limit::<C::Error>(
            AnimationAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        let reservation = self
            .control
            .reserve(SceneReservationKind::ValidationScratch, bytes)
            .map_err(AnimationAuthoringFailure::Control)?;
        self.control
            .validate_reservation(&reservation, SceneReservationKind::ValidationScratch)
            .map_err(AnimationAuthoringFailure::Control)?;
        Ok(reservation)
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut C::Reservation,
        bytes: u64,
    ) -> AnimationValidationResult<(), C> {
        enforce_limit::<C::Error>(
            AnimationAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        self.control
            .resize_reservation(reservation, SceneReservationKind::ValidationScratch, bytes)
            .map_err(AnimationAuthoringFailure::Control)
    }

    fn allocate_scratch<T>(
        &self,
        plan: CapacityPlan,
        resource: &'static str,
    ) -> Result<(Vec<T>, u64), AnimationAuthoringSemanticError> {
        plan.verify_preflight(0, 0).map_err(|cause| {
            AnimationAuthoringSemanticError::ScratchCapacity { resource, cause }
        })?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| AnimationAuthoringSemanticError::ScratchAllocation { resource })?;
        let actual_bytes = plan
            .finish(values.len(), values.capacity())
            .map_err(|cause| AnimationAuthoringSemanticError::ScratchCapacity {
                resource,
                cause,
            })?;
        Ok((values, actual_bytes))
    }
}

#[derive(Clone, Copy)]
enum AnimationPayloadPart {
    Times,
    Values,
    InTangents,
    OutTangents,
}

pub(in crate::canonical_scene::store) fn animation_shape_scratch_envelope(
    input: AnimationAuthoringBatch<'_>,
    authored_node_count: usize,
    mesh_instance_count: usize,
) -> Result<u64, AnimationAuthoringSemanticError> {
    let clip_plan = CapacityPlan::for_type::<ValidatedAnimationClipPlan>(0, 0, input.clips().len())
        .map_err(|cause| AnimationAuthoringSemanticError::ScratchCapacity {
            resource: "validated animation clip plans",
            cause,
        })?;
    let channel_plan =
        CapacityPlan::for_type::<ValidatedAnimationChannelPlan>(0, 0, input.channels().len())
            .map_err(|cause| AnimationAuthoringSemanticError::ScratchCapacity {
                resource: "validated animation channel plans",
                cause,
            })?;
    let seen_target_count = seen_target_count(
        authored_node_count,
        mesh_instance_count,
        input.channels().len(),
    )?;
    let seen_target_plan = CapacityPlan::for_type::<SeenAnimationTarget>(0, 0, seen_target_count)
        .map_err(|cause| AnimationAuthoringSemanticError::ScratchCapacity {
        resource: "animation seen-target index",
        cause,
    })?;
    checked_sum_bytes(
        [
            scratch_envelope(clip_plan, "validated animation clip plans")?,
            scratch_envelope(channel_plan, "validated animation channel plans")?,
            scratch_envelope(seen_target_plan, "animation seen-target index")?,
        ],
        "animation shape scratch envelope",
    )
}

pub(in crate::canonical_scene::store) fn validate_animation_batch<'input, NR, DR, O, PP, U>(
    input: AnimationAuthoringBatch<'input>,
    targets: &AnimationTargetCatalog<'_, '_, NR, DR>,
    limits: SceneValidationLimits,
    control: &mut OperationAuthoringControl<'_, O, PP, U>,
) -> Result<
    ValidatedAnimationBatch<'input, RetainedStoreReservation<O>>,
    AnimationAuthoringFailure<OperationAuthoringControlError>,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    validate_animation_batch_inner(input, targets, limits, control)
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn validate_animation_batch_with_control<'input, NR, DR, C>(
    input: AnimationAuthoringBatch<'input>,
    targets: &AnimationTargetCatalog<'_, '_, NR, DR>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedAnimationBatch<'input, C::Reservation>, AnimationAuthoringFailure<C::Error>>
where
    C: SceneValidationControl,
{
    validate_animation_batch_inner(input, targets, limits, control)
}

fn validate_animation_batch_inner<'input, NR, DR, C>(
    input: AnimationAuthoringBatch<'input>,
    targets: &AnimationTargetCatalog<'_, '_, NR, DR>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedAnimationBatch<'input, C::Reservation>, AnimationAuthoringFailure<C::Error>>
where
    C: SceneValidationControl,
{
    let mut context = AnimationValidationContext::new(limits, control);
    context.poll_now()?;

    let clip_count = collection_count(input.clips().len(), "animation clips")?;
    let channel_count = collection_count(input.channels().len(), "animation channels")?;
    let entity_count = u64::from(clip_count)
        .checked_add(u64::from(channel_count))
        .ok_or(AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "animation entities",
        })?;
    enforce_limit::<C::Error>(
        AnimationAuthoringLimit::Entities,
        entity_count,
        limits.max_entities(),
    )?;
    context.set_structural_bytes(animation_structural_bytes(clip_count, channel_count)?)?;

    let clip_plan = context.scratch_plan::<ValidatedAnimationClipPlan>(
        input.clips().len(),
        "validated animation clip plans",
    )?;
    let channel_plan = context.scratch_plan::<ValidatedAnimationChannelPlan>(
        input.channels().len(),
        "validated animation channel plans",
    )?;
    let seen_target_count = targets.seen_target_count(input.channels().len()).ok_or(
        AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "animation seen-target index",
        },
    )?;
    let seen_target_plan = context
        .scratch_plan::<SeenAnimationTarget>(seen_target_count, "animation seen-target index")?;
    context.work(
        SceneValidationWork::Traversal,
        usize_to_u64(seen_target_count, "animation seen-target initialization")?,
    )?;
    let scratch_envelope = checked_sum_bytes(
        [
            scratch_envelope(clip_plan, "validated animation clip plans")?,
            scratch_envelope(channel_plan, "validated animation channel plans")?,
            scratch_envelope(seen_target_plan, "animation seen-target index")?,
        ],
        "animation scratch envelope",
    )?;
    let mut reservation = context.reserve_scratch(scratch_envelope)?;
    let (clip_plans, clip_plan_bytes) =
        context.allocate_scratch(clip_plan, "validated animation clip plans")?;
    let (channel_plans, channel_plan_bytes) =
        context.allocate_scratch(channel_plan, "validated animation channel plans")?;
    let (mut seen_targets, seen_target_bytes) =
        context.allocate_scratch(seen_target_plan, "animation seen-target index")?;
    for _ in 0..seen_target_count {
        seen_targets.push(SeenAnimationTarget::UNUSED);
    }
    let scratch_actual = checked_sum_bytes(
        [clip_plan_bytes, channel_plan_bytes, seen_target_bytes],
        "animation scratch allocation",
    )?;
    context.resize_scratch(&mut reservation, scratch_actual)?;
    let mut storage = AnimationValidationStorage {
        clip_plans,
        channel_plans,
        seen_targets,
        reservation,
    };

    let mut expected_channel_offset = 0_u32;
    for (clip_index, clip) in input.clips().iter().copied().enumerate() {
        context.work(SceneValidationWork::Entity, 1)?;
        validate_clip_header(clip_index, clip)?;
        if animation_rate_present(clip.time_domain()) {
            context.add_structural_bytes(LOGICAL_F64_BYTES - LOGICAL_U8_BYTES)?;
        }
        let channel_range = clip.channels();
        if channel_range.count() != 0 {
            context.add_structural_bytes(LOGICAL_REFERENCE_BYTES - LOGICAL_U8_BYTES)?;
        }
        if channel_range.offset() != expected_channel_offset {
            return Err(AnimationAuthoringSemanticError::ClipChannelRangeOrder {
                clip: clip_index,
                expected: expected_channel_offset,
                actual: channel_range.offset(),
            }
            .into());
        }
        let range = checked_range(channel_range, input.channels().len()).map_err(|_| {
            AnimationAuthoringSemanticError::ClipChannelRangeOutOfBounds {
                clip: clip_index,
                channel_count: input.channels().len(),
            }
        })?;
        let first_channel = range.start;
        for channel_index in range.clone() {
            context.work(SceneValidationWork::Entity, 1)?;
            let channel = *input.channels().get(channel_index).ok_or(
                AnimationAuthoringSemanticError::AccountingOverflow {
                    dimension: "animation channel lookup",
                },
            )?;
            let target = validate_target(
                clip_index,
                channel_index - first_channel,
                channel.target(),
                targets,
            )?;
            validate_unique_target(
                clip_index,
                channel_index - first_channel,
                channel.target(),
                target.seen_index,
                &mut storage.seen_targets,
                &mut context,
            )?;
            context.add_relation()?;
            context.add_relation()?;
            let plan = validate_channel(
                clip_index,
                channel_index - first_channel,
                channel_index,
                channel,
                target,
                &mut context,
            )?;
            storage.channel_plans.push(plan);
        }
        expected_channel_offset = channel_range
            .offset()
            .checked_add(channel_range.count())
            .ok_or(
                AnimationAuthoringSemanticError::ClipChannelRangeOutOfBounds {
                    clip: clip_index,
                    channel_count: input.channels().len(),
                },
            )?;
        storage.clip_plans.push(ValidatedAnimationClipPlan {
            input_index: clip_index,
            channels: channel_range,
        });
    }
    if expected_channel_offset != channel_count {
        let unowned = usize::try_from(channel_count - expected_channel_offset).map_err(|_| {
            AnimationAuthoringSemanticError::AccountingOverflow {
                dimension: "unowned animation channels",
            }
        })?;
        return Err(AnimationAuthoringSemanticError::UnownedChannels { unowned }.into());
    }

    let payload_block_count = [
        context.time_scalars,
        context.value_scalars,
        context.in_tangent_scalars,
        context.out_tangent_scalars,
    ]
    .into_iter()
    .filter(|count| *count != 0)
    .count();
    let payload_block_count = u32::try_from(payload_block_count).map_err(|_| {
        AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "animation payload block count",
        }
    })?;
    let AnimationValidationStorage {
        clip_plans,
        channel_plans,
        seen_targets,
        mut reservation,
    } = storage;
    drop(seen_targets);
    let retained_scratch_bytes = checked_sum_bytes(
        [clip_plan_bytes, channel_plan_bytes],
        "retained animation scratch allocation",
    )?;
    context.resize_scratch(&mut reservation, retained_scratch_bytes)?;
    let validated = ValidatedAnimationBatch {
        input,
        clip_count,
        channel_count,
        time_scalar_count: context.time_scalars,
        value_scalar_count: context.value_scalars,
        in_tangent_scalar_count: context.in_tangent_scalars,
        out_tangent_scalar_count: context.out_tangent_scalars,
        payload_block_count,
        entity_count,
        edge_count: context.edges,
        reference_count: context.references,
        semantic_cell_count: context.semantic_cells,
        canonical_bytes: context.canonical_bytes,
        validation_work: context.work,
        scratch_bytes: retained_scratch_bytes,
        clip_plans,
        channel_plans,
        reservation,
    };
    context.finish()?;
    Ok(validated)
}

fn validate_clip_header(
    clip_index: usize,
    clip: AnimationClipAuthoring,
) -> Result<(), AnimationAuthoringSemanticError> {
    if !clip.duration().is_finite() || clip.duration() < 0.0 {
        return Err(AnimationAuthoringSemanticError::InvalidDuration { clip: clip_index });
    }
    match clip.time_domain() {
        AnimationTimeDomainAuthoring::Frames { rate } => {
            validate_rate(clip_index, AnimationRateKind::Frames, rate)
        }
        AnimationTimeDomainAuthoring::Ticks { rate: Some(rate) } => {
            validate_rate(clip_index, AnimationRateKind::Ticks, rate)
        }
        AnimationTimeDomainAuthoring::Seconds
        | AnimationTimeDomainAuthoring::Ticks { rate: None } => Ok(()),
    }
}

fn validate_rate(
    clip: usize,
    kind: AnimationRateKind,
    rate: f64,
) -> Result<(), AnimationAuthoringSemanticError> {
    if rate.is_finite() && rate > 0.0 {
        Ok(())
    } else {
        Err(AnimationAuthoringSemanticError::InvalidRate { clip, kind })
    }
}

const fn animation_rate_present(domain: AnimationTimeDomainAuthoring) -> bool {
    matches!(
        domain,
        AnimationTimeDomainAuthoring::Frames { .. }
            | AnimationTimeDomainAuthoring::Ticks { rate: Some(_) }
    )
}

fn validate_unique_target<C: SceneValidationControl>(
    clip: usize,
    channel: usize,
    target: AnimationTargetAuthoring,
    seen_index: usize,
    seen_targets: &mut [SeenAnimationTarget],
    context: &mut AnimationValidationContext<'_, C>,
) -> AnimationValidationResult<(), C> {
    context.work(SceneValidationWork::Traversal, 1)?;
    let entry = seen_targets.get_mut(seen_index).ok_or(
        AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "animation seen-target lookup",
        },
    )?;
    let clip_u32 =
        u32::try_from(clip).map_err(|_| AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "animation clip identity",
        })?;
    let channel_u32 = u32::try_from(channel).map_err(|_| {
        AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "animation channel identity",
        }
    })?;
    if entry.clip == clip_u32 {
        return Err(AnimationAuthoringSemanticError::DuplicateTarget {
            clip,
            target,
            first_channel: usize::try_from(entry.channel).map_err(|_| {
                AnimationAuthoringSemanticError::AccountingOverflow {
                    dimension: "first duplicate animation channel",
                }
            })?,
            duplicate_channel: channel,
        }
        .into());
    }
    *entry = SeenAnimationTarget {
        clip: clip_u32,
        channel: channel_u32,
    };
    Ok(())
}

#[derive(Clone, Copy)]
struct ValidatedAnimationTarget {
    canonical_index: u32,
    value_width: u32,
    seen_index: usize,
}

fn validate_target<NR, DR>(
    clip: usize,
    channel: usize,
    target: AnimationTargetAuthoring,
    catalog: &AnimationTargetCatalog<'_, '_, NR, DR>,
) -> Result<ValidatedAnimationTarget, AnimationAuthoringSemanticError> {
    match target {
        AnimationTargetAuthoring::NodeTranslation(node) => {
            validate_node_target(clip, channel, node, 0, 3, catalog)
        }
        AnimationTargetAuthoring::NodeRotation(node) => {
            validate_node_target(clip, channel, node, 1, 4, catalog)
        }
        AnimationTargetAuthoring::NodeScale(node) => {
            validate_node_target(clip, channel, node, 2, 3, catalog)
        }
        AnimationTargetAuthoring::MeshInstanceMorphWeights(mesh_instance) => {
            let mesh_instance_index = usize::try_from(mesh_instance).map_err(|_| {
                AnimationAuthoringSemanticError::MeshInstanceTargetOutOfRange {
                    clip,
                    channel,
                    mesh_instance,
                    mesh_instance_count: catalog.mesh_instance_count(),
                }
            })?;
            if mesh_instance_index >= catalog.mesh_instance_count() {
                return Err(
                    AnimationAuthoringSemanticError::MeshInstanceTargetOutOfRange {
                        clip,
                        channel,
                        mesh_instance,
                        mesh_instance_count: catalog.mesh_instance_count(),
                    },
                );
            }
            let width = catalog
                .mesh_instance_morph_width(mesh_instance_index)
                .ok_or(AnimationAuthoringSemanticError::AccountingOverflow {
                    dimension: "validated mesh-instance morph width lookup",
                })?;
            if width == 0 {
                return Err(AnimationAuthoringSemanticError::MorphTargetWidthZero {
                    clip,
                    channel,
                    mesh_instance,
                });
            }
            let node_target_count = catalog.authored_node_count().checked_mul(3).ok_or(
                AnimationAuthoringSemanticError::AccountingOverflow {
                    dimension: "animation node target count",
                },
            )?;
            let seen_index = node_target_count.checked_add(mesh_instance_index).ok_or(
                AnimationAuthoringSemanticError::AccountingOverflow {
                    dimension: "animation morph target index",
                },
            )?;
            Ok(ValidatedAnimationTarget {
                canonical_index: mesh_instance,
                value_width: width,
                seen_index,
            })
        }
    }
}

fn validate_node_target<NR, DR>(
    clip: usize,
    channel: usize,
    node: NodeLocalId,
    semantic: usize,
    value_width: u32,
    catalog: &AnimationTargetCatalog<'_, '_, NR, DR>,
) -> Result<ValidatedAnimationTarget, AnimationAuthoringSemanticError> {
    let node_index = usize::try_from(node.index()).map_err(|_| {
        AnimationAuthoringSemanticError::NodeTargetOutOfRange {
            clip,
            channel,
            node: node.index(),
            node_count: catalog.authored_node_count(),
        }
    })?;
    let authored_node = catalog.authored_node(node_index).ok_or(
        AnimationAuthoringSemanticError::NodeTargetOutOfRange {
            clip,
            channel,
            node: node.index(),
            node_count: catalog.authored_node_count(),
        },
    )?;
    if authored_node.authored_transform().kind() == AuthoredTransformKind::Matrix {
        return Err(AnimationAuthoringSemanticError::MatrixAuthoredNodeTarget {
            clip,
            channel,
            node: node.index(),
        });
    }
    let canonical_index = catalog.canonical_node_index(node).ok_or(
        AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "validated animation node target mapping",
        },
    )?;
    let seen_index = node_index
        .checked_mul(3)
        .and_then(|base| base.checked_add(semantic))
        .ok_or(AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "animation node target index",
        })?;
    Ok(ValidatedAnimationTarget {
        canonical_index,
        value_width,
        seen_index,
    })
}

fn validate_channel<C: SceneValidationControl>(
    clip: usize,
    channel: usize,
    input_index: usize,
    input: AnimationChannelAuthoring<'_>,
    target: ValidatedAnimationTarget,
    context: &mut AnimationValidationContext<'_, C>,
) -> AnimationValidationResult<ValidatedAnimationChannelPlan, C> {
    let value_width = target.value_width;
    let times = input.times();
    if times.is_empty() {
        return Err(AnimationAuthoringSemanticError::EmptyChannel { clip, channel }.into());
    }
    let time_count = collection_count(times.len(), "animation key times")?;
    let time_offset = scalar_offset(context.time_scalars, "animation time offset")?;
    context.add_payload(AnimationPayloadPart::Times, u64::from(time_count))?;
    let mut previous = None;
    for (key, &time) in times.iter().enumerate() {
        context.work(SceneValidationWork::PayloadCell, 1)?;
        if !time.is_finite() {
            return Err(
                AnimationAuthoringSemanticError::NonFiniteTime { clip, channel, key }.into(),
            );
        }
        if previous.is_some_and(|previous_time| previous_time >= time) {
            return Err(
                AnimationAuthoringSemanticError::TimesNotStrictlyIncreasing {
                    clip,
                    channel,
                    previous_key: key - 1,
                    key,
                }
                .into(),
            );
        }
        previous = Some(time);
    }

    let expected_scalars = u64::from(time_count)
        .checked_mul(u64::from(value_width))
        .ok_or(AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "animation sample scalar count",
        })?;
    if usize_to_u64(input.values().len(), "animation value scalar count")? != expected_scalars {
        return Err(AnimationAuthoringSemanticError::ValueShape {
            clip,
            channel,
            expected: expected_scalars,
            actual: input.values().len(),
        }
        .into());
    }
    let value_count = collection_count(input.values().len(), "animation value scalars")?;
    let value_offset = scalar_offset(context.value_scalars, "animation value offset")?;
    context.add_payload(AnimationPayloadPart::Values, u64::from(value_count))?;
    validate_f32_samples(
        clip,
        channel,
        AnimationSamplePart::Values,
        input.values(),
        value_width,
        context,
    )?;
    if matches!(input.target(), AnimationTargetAuthoring::NodeRotation(_)) {
        validate_rotation_values(clip, channel, input.values(), context)?;
    }

    let (in_tangent_offset, in_tangent_count) = validate_tangent_part(
        clip,
        channel,
        AnimationSamplePart::InTangents,
        input.interpolation(),
        input.in_tangents(),
        value_width,
        expected_scalars,
        AnimationPayloadPart::InTangents,
        context,
    )?;
    let (out_tangent_offset, out_tangent_count) = validate_tangent_part(
        clip,
        channel,
        AnimationSamplePart::OutTangents,
        input.interpolation(),
        input.out_tangents(),
        value_width,
        expected_scalars,
        AnimationPayloadPart::OutTangents,
        context,
    )?;
    if matches!(in_tangent_offset, Presence::Present(_)) {
        context.add_structural_bytes(LOGICAL_REFERENCE_BYTES - LOGICAL_U8_BYTES)?;
    }
    if matches!(out_tangent_offset, Presence::Present(_)) {
        context.add_structural_bytes(LOGICAL_REFERENCE_BYTES - LOGICAL_U8_BYTES)?;
    }

    Ok(ValidatedAnimationChannelPlan {
        input_index,
        canonical_target_index: target.canonical_index,
        value_width,
        time_offset,
        time_count,
        value_offset,
        value_count,
        in_tangent_offset,
        in_tangent_count,
        out_tangent_offset,
        out_tangent_count,
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_tangent_part<C: SceneValidationControl>(
    clip: usize,
    channel: usize,
    part: AnimationSamplePart,
    interpolation: AnimationInterpolationAuthoring,
    payload: Presence<&[f32]>,
    value_width: u32,
    expected_scalars: u64,
    payload_part: AnimationPayloadPart,
    context: &mut AnimationValidationContext<'_, C>,
) -> AnimationValidationResult<(Presence<u32>, u32), C> {
    match (interpolation, payload) {
        (AnimationInterpolationAuthoring::CubicSpline, Presence::Absent) => {
            Err(AnimationAuthoringSemanticError::MissingTangents {
                clip,
                channel,
                part,
            }
            .into())
        }
        (
            AnimationInterpolationAuthoring::Step | AnimationInterpolationAuthoring::Linear,
            Presence::Present(_),
        ) => Err(AnimationAuthoringSemanticError::UnexpectedTangents {
            clip,
            channel,
            part,
        }
        .into()),
        (
            AnimationInterpolationAuthoring::Step | AnimationInterpolationAuthoring::Linear,
            Presence::Absent,
        ) => Ok((Presence::Absent, 0)),
        (AnimationInterpolationAuthoring::CubicSpline, Presence::Present(values)) => {
            if usize_to_u64(values.len(), "animation tangent scalar count")? != expected_scalars {
                return Err(AnimationAuthoringSemanticError::TangentShape {
                    clip,
                    channel,
                    part,
                    expected: expected_scalars,
                    actual: values.len(),
                }
                .into());
            }
            let count = collection_count(values.len(), "animation tangent scalars")?;
            let current = match payload_part {
                AnimationPayloadPart::InTangents => context.in_tangent_scalars,
                AnimationPayloadPart::OutTangents => context.out_tangent_scalars,
                AnimationPayloadPart::Times | AnimationPayloadPart::Values => {
                    return Err(AnimationAuthoringSemanticError::AccountingOverflow {
                        dimension: "animation tangent payload kind",
                    }
                    .into());
                }
            };
            let offset = scalar_offset(current, "animation tangent offset")?;
            context.add_payload(payload_part, u64::from(count))?;
            validate_f32_samples(clip, channel, part, values, value_width, context)?;
            Ok((Presence::Present(offset), count))
        }
    }
}

fn validate_f32_samples<C: SceneValidationControl>(
    clip: usize,
    channel: usize,
    part: AnimationSamplePart,
    values: &[f32],
    value_width: u32,
    context: &mut AnimationValidationContext<'_, C>,
) -> AnimationValidationResult<(), C> {
    let width = usize::try_from(value_width).map_err(|_| {
        AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "animation sample width",
        }
    })?;
    for (scalar, value) in values.iter().enumerate() {
        context.work(SceneValidationWork::PayloadCell, 1)?;
        if !value.is_finite() {
            return Err(AnimationAuthoringSemanticError::NonFiniteSample {
                clip,
                channel,
                part,
                sample: scalar / width,
                component: scalar % width,
            }
            .into());
        }
    }
    Ok(())
}

fn validate_rotation_values<C: SceneValidationControl>(
    clip: usize,
    channel: usize,
    values: &[f32],
    context: &mut AnimationValidationContext<'_, C>,
) -> AnimationValidationResult<(), C> {
    for (sample, components) in values.chunks_exact(4).enumerate() {
        context.work(SceneValidationWork::Traversal, 1)?;
        let rotation = Quat::from_xyzw(components[0], components[1], components[2], components[3]);
        if !rotation.is_unit() {
            return Err(AnimationAuthoringSemanticError::NonUnitQuaternion {
                clip,
                channel,
                sample,
            }
            .into());
        }
    }
    Ok(())
}

fn animation_structural_bytes(
    clip_count: u32,
    channel_count: u32,
) -> Result<u64, AnimationAuthoringSemanticError> {
    let directory_bytes = checked_sum_bytes(
        [
            ANIMATION_DIRECTORY_SCALAR_BYTES,
            if clip_count == 0 {
                0
            } else {
                LOGICAL_REFERENCE_BYTES - LOGICAL_U8_BYTES
            },
            if channel_count == 0 {
                0
            } else {
                LOGICAL_REFERENCE_BYTES - LOGICAL_U8_BYTES
            },
        ],
        "canonical animation directory footprint",
    )?;
    checked_sum_bytes(
        [
            directory_bytes,
            checked_bytes(
                u64::from(clip_count),
                ANIMATION_CLIP_BASE_BYTES,
                "canonical animation clip records",
            )?,
            checked_bytes(
                u64::from(channel_count),
                ANIMATION_CHANNEL_BASE_BYTES,
                "canonical animation channel records",
            )?,
        ],
        "canonical animation structural footprint",
    )
}

fn seen_target_count(
    authored_node_count: usize,
    mesh_instance_count: usize,
    channel_count: usize,
) -> Result<usize, AnimationAuthoringSemanticError> {
    if channel_count == 0 {
        return Ok(0);
    }
    authored_node_count
        .checked_mul(3)
        .and_then(|count| count.checked_add(mesh_instance_count))
        .ok_or(AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "animation seen-target count",
        })
}

fn enforce_limit<E>(
    limit: AnimationAuthoringLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), AnimationAuthoringFailure<E>>
where
    E: Error + Send + Sync + 'static,
{
    if actual <= maximum {
        Ok(())
    } else {
        Err(AnimationAuthoringSemanticError::LimitExceeded {
            limit,
            actual,
            maximum,
        }
        .into())
    }
}

fn checked_bytes(
    count: u64,
    width: u64,
    dimension: &'static str,
) -> Result<u64, AnimationAuthoringSemanticError> {
    count
        .checked_mul(width)
        .ok_or(AnimationAuthoringSemanticError::AccountingOverflow { dimension })
}

fn scratch_envelope(
    plan: CapacityPlan,
    resource: &'static str,
) -> Result<u64, AnimationAuthoringSemanticError> {
    plan.envelope_delta_bytes()
        .map_err(|cause| AnimationAuthoringSemanticError::ScratchCapacity { resource, cause })
}

fn checked_sum_bytes<const N: usize>(
    values: [u64; N],
    dimension: &'static str,
) -> Result<u64, AnimationAuthoringSemanticError> {
    values.into_iter().try_fold(0_u64, |total, value| {
        total
            .checked_add(value)
            .ok_or(AnimationAuthoringSemanticError::AccountingOverflow { dimension })
    })
}

fn checked_range(
    range: AuthoringRange,
    table_len: usize,
) -> Result<Range<usize>, AnimationAuthoringSemanticError> {
    let start = usize::try_from(range.offset()).map_err(|_| {
        AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "animation authoring range offset",
        }
    })?;
    let count = usize::try_from(range.count()).map_err(|_| {
        AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "animation authoring range count",
        }
    })?;
    let end =
        start
            .checked_add(count)
            .ok_or(AnimationAuthoringSemanticError::AccountingOverflow {
                dimension: "animation authoring range end",
            })?;
    if end > table_len {
        return Err(AnimationAuthoringSemanticError::AccountingOverflow {
            dimension: "validated animation authoring range",
        });
    }
    Ok(start..end)
}

fn collection_count(
    count: usize,
    collection: &'static str,
) -> Result<u32, AnimationAuthoringSemanticError> {
    u32::try_from(count)
        .map_err(|_| AnimationAuthoringSemanticError::CountDomainTooLarge { collection })
}

fn scalar_offset(
    count: u64,
    dimension: &'static str,
) -> Result<u32, AnimationAuthoringSemanticError> {
    u32::try_from(count)
        .map_err(|_| AnimationAuthoringSemanticError::AccountingOverflow { dimension })
}

fn usize_to_u64(
    value: usize,
    dimension: &'static str,
) -> Result<u64, AnimationAuthoringSemanticError> {
    u64::try_from(value)
        .map_err(|_| AnimationAuthoringSemanticError::AccountingOverflow { dimension })
}
