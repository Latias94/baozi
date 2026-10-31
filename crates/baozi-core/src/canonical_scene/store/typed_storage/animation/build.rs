//! Capacity-qualified construction inside the existing retained-storage admission.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::canonical_scene::store::production::{
    AnimationTargetAuthoring, Presence, ValidatedAnimationBatch,
};
use crate::canonical_scene::store::typed_storage::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, append, append_name,
    capacity, finish, push,
};

use super::*;

#[cfg(test)]
mod tests;

const CLIPS: usize = 0;
const CHANNELS: usize = 1;
const TIMES: usize = 2;
const VALUES: usize = 3;
const IN_TANGENTS: usize = 4;
const OUT_TANGENTS: usize = 5;
const NAMES: usize = 6;

/// Only initialized counts persist. Admission reconstructs seven checked capacity
/// envelopes; no capacity array or derived byte/work evidence is carried in the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct AnimationStoragePlan {
    rows: [u32; 2],
    payloads: [usize; 5],
}

impl AnimationStoragePlan {
    pub(in crate::canonical_scene::store) fn new<R>(
        batch: &ValidatedAnimationBatch<'_, R>,
    ) -> Result<Self, StorePlanError> {
        let plan = Self::for_shape(
            [
                u64::from(batch.clip_count()),
                u64::from(batch.channel_count()),
            ],
            [
                batch.time_scalar_count(),
                batch.value_scalar_count(),
                batch.in_tangent_scalar_count(),
                batch.out_tangent_scalar_count(),
                batch.clip_name_byte_count(),
            ],
        )?;
        let semantic_cells = plan.payload_counts()[..4]
            .iter()
            .try_fold(0_u64, |sum, &count| {
                sum.checked_add(count).ok_or(StorePlanError::SizeOverflow)
            })?;
        if semantic_cells != batch.semantic_cell_count()
            || plan.canonical_bytes()? != batch.canonical_bytes()
        {
            return Err(StorePlanError::InvalidBlockTable);
        }
        Ok(plan)
    }

    /// Rows: clips, channels. Pools: f64 times, f32 values, f32 in tangents,
    /// f32 out tangents, UTF-8 name bytes. Counts exclude spare allocator capacity.
    pub(in crate::canonical_scene::store) fn for_shape(
        rows: [u64; 2],
        payloads: [u64; 5],
    ) -> Result<Self, StorePlanError> {
        let mut checked_rows = [0; 2];
        for (output, count) in checked_rows.iter_mut().zip(rows) {
            *output = u32::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?;
        }
        let mut checked_payloads = [0; 5];
        for (output, count) in checked_payloads.iter_mut().zip(payloads) {
            *output = usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?;
        }
        let plan = Self {
            rows: checked_rows,
            payloads: checked_payloads,
        };
        plan.capacities()?;
        Ok(plan)
    }

    pub(in crate::canonical_scene::store) fn capacities(
        self,
    ) -> Result<[CapacityPlan; 7], StorePlanError> {
        let rows = self.row_counts();
        let payloads = self.payload_counts();
        Ok([
            capacity::<StoredAnimationClip>(rows[0])?,
            capacity::<StoredAnimationChannel>(rows[1])?,
            capacity::<f64>(payloads[0])?,
            capacity::<f32>(payloads[1])?,
            capacity::<f32>(payloads[2])?,
            capacity::<f32>(payloads[3])?,
            capacity::<u8>(payloads[4])?,
        ])
    }

    pub(in crate::canonical_scene::store) fn row_counts(self) -> [u64; 2] {
        self.rows.map(u64::from)
    }

    pub(in crate::canonical_scene::store) fn payload_counts(self) -> [u64; 5] {
        self.payloads.map(|count| count as u64)
    }

    pub(in crate::canonical_scene::store) const fn name_bytes(self) -> usize {
        self.payloads[4]
    }

    /// One action per initialized row/scalar, and one string action per name byte.
    pub(in crate::canonical_scene::store) fn work(self) -> Result<u64, StorePlanError> {
        self.row_counts()
            .into_iter()
            .chain(self.payload_counts())
            .try_fold(0_u64, |sum, count| {
                sum.checked_add(count).ok_or(StorePlanError::SizeOverflow)
            })
    }

    pub(in crate::canonical_scene::store) fn canonical_bytes(self) -> Result<u64, StorePlanError> {
        Self::canonical_bytes_for_shape(self.row_counts(), self.payload_counts())
    }

    /// Initialized target-ABI header, rows, and pools. Retained-owner infrastructure
    /// and spare capacity are excluded; optional rate and duration are inline once.
    pub(in crate::canonical_scene::store) fn canonical_bytes_for_shape(
        rows: [u64; 2],
        payloads: [u64; 5],
    ) -> Result<u64, StorePlanError> {
        let parts = [
            (rows[0], std::mem::size_of::<StoredAnimationClip>()),
            (rows[1], std::mem::size_of::<StoredAnimationChannel>()),
            (payloads[0], std::mem::size_of::<f64>()),
            (payloads[1], std::mem::size_of::<f32>()),
            (payloads[2], std::mem::size_of::<f32>()),
            (payloads[3], std::mem::size_of::<f32>()),
            (payloads[4], std::mem::size_of::<u8>()),
        ];
        let header = u64::try_from(std::mem::size_of::<AnimationStorage>())
            .map_err(|_| StorePlanError::SizeOverflow)?;
        parts.into_iter().try_fold(header, |sum, (count, width)| {
            let width = u64::try_from(width).map_err(|_| StorePlanError::SizeOverflow)?;
            count
                .checked_mul(width)
                .and_then(|bytes| sum.checked_add(bytes))
                .ok_or(StorePlanError::SizeOverflow)
        })
    }

    /// Admit the owner and all seven capacities before entry. Node target indices
    /// were resolved by the same validated target catalog, including the synthetic
    /// root offset; no second node batch or independently supplied mapping is accepted.
    pub(in crate::canonical_scene::store) fn build<R, C: TypedStorageBuildControl>(
        self,
        batch: &ValidatedAnimationBatch<'_, R>,
        mut control: C,
    ) -> BuildResult<AnimationStorage, C::Error> {
        if Self::new(batch).map_err(|_| TypedStorageBuildError::Invariant)? != self {
            return Err(TypedStorageBuildError::Invariant);
        }
        let capacities = self
            .capacities()
            .map_err(|_| TypedStorageBuildError::Invariant)?;
        let mut storage = AnimationStorage {
            clips: allocate(capacities[CLIPS])?,
            channels: allocate(capacities[CHANNELS])?,
            times: allocate(capacities[TIMES])?,
            values: allocate(capacities[VALUES])?,
            in_tangents: allocate(capacities[IN_TANGENTS])?,
            out_tangents: allocate(capacities[OUT_TANGENTS])?,
            names: allocate(capacities[NAMES])?,
        };
        for index in 0..batch.clip_count() as usize {
            let clip = batch.clip(index).ok_or(TypedStorageBuildError::Invariant)?;
            let range = clip.channel_range();
            if range.offset() as usize != storage.channels.len() {
                return Err(TypedStorageBuildError::Invariant);
            }
            let name = append_name(
                &mut storage.names,
                capacities[NAMES],
                clip.name(),
                &mut control,
            )?;
            let channel_start = storage.channels.len();
            for channel in clip.channels() {
                let canonical = channel.canonical_target_index();
                let target = match channel.target() {
                    AnimationTargetAuthoring::NodeTranslation(_) if canonical != 0 => {
                        StoredAnimationTarget::NodeTranslation(StoredNodeIndex(canonical))
                    }
                    AnimationTargetAuthoring::NodeRotation(_) if canonical != 0 => {
                        StoredAnimationTarget::NodeRotation(StoredNodeIndex(canonical))
                    }
                    AnimationTargetAuthoring::NodeScale(_) if canonical != 0 => {
                        StoredAnimationTarget::NodeScale(StoredNodeIndex(canonical))
                    }
                    AnimationTargetAuthoring::MeshInstanceMorphWeights(source)
                        if canonical == source =>
                    {
                        StoredAnimationTarget::MeshInstanceMorphWeights(StoredMeshInstanceIndex(
                            canonical,
                        ))
                    }
                    _ => return Err(TypedStorageBuildError::Invariant),
                };
                let times = append(
                    &mut storage.times,
                    capacities[TIMES],
                    channel.times(),
                    &mut control,
                )?;
                let values = append(
                    &mut storage.values,
                    capacities[VALUES],
                    channel.values(),
                    &mut control,
                )?;
                let in_tangents = append_optional(
                    &mut storage.in_tangents,
                    capacities[IN_TANGENTS],
                    channel.in_tangents(),
                    &mut control,
                )?;
                let out_tangents = append_optional(
                    &mut storage.out_tangents,
                    capacities[OUT_TANGENTS],
                    channel.out_tangents(),
                    &mut control,
                )?;
                let row = StoredAnimationChannel {
                    source_index: channel.source_index(),
                    target,
                    interpolation: channel.interpolation(),
                    pre_extrapolation: channel.pre_extrapolation(),
                    post_extrapolation: channel.post_extrapolation(),
                    times,
                    values,
                    in_tangents,
                    out_tangents,
                };
                if row.value_width() != Some(channel.value_width()) {
                    return Err(TypedStorageBuildError::Invariant);
                }
                match (row.interpolation, row.in_tangents, row.out_tangents) {
                    (AnimationInterpolation::CubicSpline, Some(input), Some(output))
                        if input.len() == values.len() && output.len() == values.len() => {}
                    (AnimationInterpolation::Step | AnimationInterpolation::Linear, None, None) => {
                    }
                    _ => return Err(TypedStorageBuildError::Invariant),
                }
                push(
                    &mut storage.channels,
                    capacities[CHANNELS],
                    row,
                    &mut control,
                )?;
            }
            let channel_count = storage
                .channels
                .len()
                .checked_sub(channel_start)
                .ok_or(TypedStorageBuildError::Invariant)?;
            if channel_count != range.count() as usize {
                return Err(TypedStorageBuildError::Invariant);
            }
            push(
                &mut storage.clips,
                capacities[CLIPS],
                StoredAnimationClip {
                    source_index: clip.source_index(),
                    name,
                    duration: clip.duration(),
                    time_domain: clip.time_domain(),
                    channels: TableRange::new(channel_start, channel_count),
                },
                &mut control,
            )?;
        }
        finish(&storage.clips, capacities[CLIPS])?;
        finish(&storage.channels, capacities[CHANNELS])?;
        finish(&storage.times, capacities[TIMES])?;
        finish(&storage.values, capacities[VALUES])?;
        finish(&storage.in_tangents, capacities[IN_TANGENTS])?;
        finish(&storage.out_tangents, capacities[OUT_TANGENTS])?;
        finish(&storage.names, capacities[NAMES])?;
        Ok(storage)
    }
}

fn append_optional<C: TypedStorageBuildControl>(
    values: &mut Vec<f32>,
    capacity: CapacityPlan,
    source: Presence<&[f32]>,
    control: &mut C,
) -> BuildResult<Option<TableRange<f32>>, C::Error> {
    match source {
        Presence::Absent => Ok(None),
        Presence::Present(source) => append(values, capacity, source, control).map(Some),
    }
}
