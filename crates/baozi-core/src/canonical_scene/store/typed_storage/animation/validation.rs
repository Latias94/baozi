//! Central animation validation over borrowed typed rows and compact payloads.
//!
//! The caller validates node, geometry, and deformation owners first. Only duplicate-target marks
//! need mutable scratch; no clip/channel descriptor or scalar projection is materialized.
//! Finite key times preserve their authored domain independently of the clip's nonnegative
//! source duration, without cropping or rebasing keys.

use crate::canonical_scene::store::typed_storage::deformation::DeformationStorage;
use crate::canonical_scene::store::typed_storage::validation::TypedStorageValidationControl;
use crate::canonical_scene::store::typed_storage::{GeometryStorage, NodeStorage, StoredTransform};
use crate::entity::{MAX_ENTITY_NAME_BYTES, validate_entity_name_str};
use crate::math::Quat;

use super::*;

#[cfg(test)]
mod tests;

/// Initialized shape in AnimationStoragePlan row/payload order. Pools are counted in scalars,
/// not key tuples. Counts neither inspect rows nor include unused allocator capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct AnimationMeteredCounts {
    pub(in crate::canonical_scene::store) clip_count: u32,
    pub(in crate::canonical_scene::store) channel_count: u32,
    pub(in crate::canonical_scene::store) time_scalar_count: u64,
    pub(in crate::canonical_scene::store) value_scalar_count: u64,
    pub(in crate::canonical_scene::store) in_tangent_scalar_count: u64,
    pub(in crate::canonical_scene::store) out_tangent_scalar_count: u64,
    pub(in crate::canonical_scene::store) clip_name_byte_count: u64,
}

impl AnimationMeteredCounts {
    /// Rows: clips, channels. Pools: times, values, in tangents, out tangents, name bytes.
    pub(in crate::canonical_scene::store) fn try_new(
        rows: [u64; 2],
        payloads: [u64; 5],
    ) -> Option<Self> {
        for count in rows.into_iter().chain(payloads) {
            usize::try_from(count).ok()?;
        }
        if payloads[2] != payloads[3]
            || (rows[1] == 0 && payloads[..4] != [0; 4])
            || (rows[1] != 0 && (rows[0] == 0 || payloads[0] == 0 || payloads[1] == 0))
        {
            return None;
        }
        Some(Self {
            clip_count: u32::try_from(rows[0]).ok()?,
            channel_count: u32::try_from(rows[1]).ok()?,
            time_scalar_count: payloads[0],
            value_scalar_count: payloads[1],
            in_tangent_scalar_count: payloads[2],
            out_tangent_scalar_count: payloads[3],
            clip_name_byte_count: payloads[4],
        })
    }

    /// O(1): only the two row lengths and five pool lengths are observed.
    pub(in crate::canonical_scene::store) fn for_storage(
        storage: &AnimationStorage,
    ) -> Option<Self> {
        Self::try_new(
            [storage.clips.len() as u64, storage.channels.len() as u64],
            [
                storage.times.len() as u64,
                storage.values.len() as u64,
                storage.in_tangents.len() as u64,
                storage.out_tangents.len() as u64,
                storage.names.len() as u64,
            ],
        )
    }

    /// Exact successful actions, excluding separately charged target-mark initialization.
    /// Shape and completion take two actions. Every clip, channel, scalar, and name byte takes
    /// one action. A rotation's fourth value-scalar action checks its bounded xyzw tuple.
    pub(in crate::canonical_scene::store) fn validation_work(self) -> Option<u64> {
        [
            2,
            u64::from(self.clip_count),
            u64::from(self.channel_count),
            self.time_scalar_count,
            self.value_scalar_count,
            self.in_tangent_scalar_count,
            self.out_tangent_scalar_count,
            self.clip_name_byte_count,
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct AnimationValidationScratchRequirements {
    pub(in crate::canonical_scene::store) target_marks: usize,
}

impl AnimationValidationScratchRequirements {
    /// canonical_nodes includes the synthetic root, which has no target slot. Channel-free
    /// storage requires no marks, independently of the size of the Scene's other domains.
    pub(in crate::canonical_scene::store) fn try_new(
        counts: AnimationMeteredCounts,
        canonical_nodes: usize,
        mesh_instances: usize,
    ) -> Option<Self> {
        let target_marks = if counts.channel_count == 0 {
            0
        } else {
            canonical_nodes
                .checked_sub(1)?
                .checked_mul(3)?
                .checked_add(mesh_instances)?
        };
        Some(Self { target_marks })
    }

    pub(in crate::canonical_scene::store) fn initialization_work_units(self) -> Option<u64> {
        u64::try_from(self.target_marks).ok()
    }
}

/// The central scratch owner initializes every mark to zero once before validate.
pub(in crate::canonical_scene::store) struct AnimationValidationScratch<'scratch> {
    pub(in crate::canonical_scene::store) target_marks: &'scratch mut [u32],
}

impl<'scratch> AnimationValidationScratch<'scratch> {
    pub(in crate::canonical_scene::store) const fn new(target_marks: &'scratch mut [u32]) -> Self {
        Self { target_marks }
    }

    #[cfg(test)]
    fn initialize<C: TypedStorageValidationControl>(
        &mut self,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        for mark in self.target_marks.iter_mut() {
            if control.run_unit(|| *mark = 0)? != Some(()) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl AnimationStorage {
    pub(in crate::canonical_scene::store) fn validate<C: TypedStorageValidationControl>(
        &self,
        control: &mut C,
        counts: AnimationMeteredCounts,
        nodes: &NodeStorage,
        geometry: &GeometryStorage,
        deformation: &DeformationStorage,
        scratch: &mut AnimationValidationScratch<'_>,
    ) -> Result<bool, C::Error> {
        if control.run_unit(|| {
            AnimationMeteredCounts::for_storage(self) == Some(counts)
                && !nodes.nodes.is_empty()
                && deformation.logical_morphs.len() == geometry.logical_meshes.len()
                && deformation.primitives.len() == geometry.primitives.len()
                && deformation.mesh_instances.len() == geometry.mesh_instances.len()
                && AnimationValidationScratchRequirements::try_new(
                    counts,
                    nodes.nodes.len(),
                    geometry.mesh_instances.len(),
                )
                .is_some_and(|requirements| requirements.target_marks == scratch.target_marks.len())
        })? != Some(true)
        {
            return Ok(false);
        }
        let mut cursors = Cursors::default();
        let mut previous_clip = None;
        let mut previous_channel = None;
        for (clip_index, clip) in self.clips.iter().enumerate() {
            let Some((channels, name)) = run(control, || {
                (previous_clip.is_none_or(|previous| previous < clip.source_index)
                    && clip.duration.is_finite()
                    && clip.duration >= 0.0
                    && valid_time_domain(clip.time_domain))
                .then_some(())?;
                previous_clip = Some(clip.source_index);
                let name = match clip.name {
                    None => &[][..],
                    Some(range) => {
                        (range.0.len() <= MAX_ENTITY_NAME_BYTES).then_some(())?;
                        let bytes = consume(range.0, &self.names, &mut cursors.names)?;
                        validate_entity_name_str(std::str::from_utf8(bytes).ok()?).ok()?;
                        bytes
                    }
                };
                let channels = consume(clip.channels, &self.channels, &mut cursors.channels)?;
                Some((channels, name))
            })?
            else {
                return Ok(false);
            };
            for &byte in name {
                if control.run_unit(|| byte)? != Some(byte) {
                    return Ok(false);
                }
            }
            // clip_count is u32; the last possible zero-based ordinal is u32::MAX - 1.
            let marker = (clip_index as u32) + 1;
            for channel in channels {
                let Some((times, values, input, output)) = run(control, || {
                    previous_channel
                        .is_none_or(|previous| previous < channel.source_index)
                        .then_some(())?;
                    previous_channel = Some(channel.source_index);
                    let (width, mark_index) =
                        target_shape(channel.target, nodes, geometry, deformation)?;
                    let mark = scratch.target_marks.get_mut(mark_index)?;
                    (*mark != marker).then_some(())?;
                    *mark = marker;
                    let times = consume(channel.times, &self.times, &mut cursors.times)?;
                    let values = consume(channel.values, &self.values, &mut cursors.values)?;
                    (!times.is_empty() && times.len().checked_mul(width) == Some(values.len()))
                        .then_some(())?;
                    let (input, output) = match (
                        channel.interpolation,
                        channel.in_tangents,
                        channel.out_tangents,
                    ) {
                        (AnimationInterpolation::CubicSpline, Some(input), Some(output)) => {
                            let input = consume(input, &self.in_tangents, &mut cursors.input)?;
                            let output = consume(output, &self.out_tangents, &mut cursors.output)?;
                            (input.len() == values.len() && output.len() == values.len())
                                .then_some(())?;
                            (input, output)
                        }
                        (
                            AnimationInterpolation::Step | AnimationInterpolation::Linear,
                            None,
                            None,
                        ) => (&[][..], &[][..]),
                        _ => return None,
                    };
                    Some((times, values, input, output))
                })?
                else {
                    return Ok(false);
                };
                let mut previous_time = None;
                for &time in times {
                    if control.run_unit(|| {
                        let valid = time.is_finite()
                            && previous_time.is_none_or(|previous| previous < time);
                        previous_time = Some(time);
                        valid
                    })? != Some(true)
                    {
                        return Ok(false);
                    }
                }
                let rotation = matches!(channel.target, StoredAnimationTarget::NodeRotation(_));
                let mut tuple = [0.0_f32; 4];
                for (scalar, &value) in values.iter().enumerate() {
                    if control.run_unit(|| {
                        if !value.is_finite() {
                            return false;
                        }
                        if rotation {
                            tuple[scalar % 4] = value;
                            if scalar % 4 == 3 {
                                return Quat::from_xyzw(tuple[0], tuple[1], tuple[2], tuple[3])
                                    .is_unit();
                            }
                        }
                        true
                    })? != Some(true)
                    {
                        return Ok(false);
                    }
                }
                for &value in input.iter().chain(output) {
                    if control.run_unit(|| value.is_finite())? != Some(true) {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(control.run_unit(|| cursors.complete(self))? == Some(true))
    }
}

#[derive(Default)]
struct Cursors {
    channels: usize,
    names: usize,
    times: usize,
    values: usize,
    input: usize,
    output: usize,
}

impl Cursors {
    fn complete(&self, storage: &AnimationStorage) -> bool {
        self.channels == storage.channels.len()
            && self.names == storage.names.len()
            && self.times == storage.times.len()
            && self.values == storage.values.len()
            && self.input == storage.in_tangents.len()
            && self.output == storage.out_tangents.len()
    }
}

fn valid_time_domain(domain: AnimationTimeDomain) -> bool {
    match domain {
        AnimationTimeDomain::Seconds | AnimationTimeDomain::Ticks { rate: None } => true,
        AnimationTimeDomain::Frames { rate } | AnimationTimeDomain::Ticks { rate: Some(rate) } => {
            rate.is_finite() && rate > 0.0
        }
    }
}

/// Resolves a closed target against actual typed owners without scanning preceding rows.
fn target_shape(
    target: StoredAnimationTarget,
    nodes: &NodeStorage,
    geometry: &GeometryStorage,
    deformation: &DeformationStorage,
) -> Option<(usize, usize)> {
    let node = match target {
        StoredAnimationTarget::NodeTranslation(node) => Some((node, 0, 3)),
        StoredAnimationTarget::NodeRotation(node) => Some((node, 1, 4)),
        StoredAnimationTarget::NodeScale(node) => Some((node, 2, 3)),
        StoredAnimationTarget::MeshInstanceMorphWeights(_) => None,
    };
    if let Some((node, semantic, width)) = node {
        let index = usize::try_from(node.0).ok()?;
        let authored = index.checked_sub(1)?;
        matches!(
            nodes.nodes.get(index)?.transform,
            StoredTransform::Trs { .. }
        )
        .then_some(())?;
        return Some((width, authored.checked_mul(3)?.checked_add(semantic)?));
    }
    let StoredAnimationTarget::MeshInstanceMorphWeights(instance) = target else {
        return None;
    };
    let index = usize::try_from(instance.0).ok()?;
    deformation.mesh_instances.get(index)?;
    let logical_mesh = usize::try_from(geometry.mesh_instances.get(index)?.logical_mesh.0).ok()?;
    geometry.logical_meshes.get(logical_mesh)?;
    let logical = deformation.logical_morphs.get(logical_mesh)?;
    let definitions = logical.definitions.get(&deformation.morph_definitions)?;
    let width = definitions.len();
    (width != 0 && u32::try_from(width).is_ok()).then_some(())?;
    let mark = nodes
        .nodes
        .len()
        .checked_sub(1)?
        .checked_mul(3)?
        .checked_add(index)?;
    Some((width, mark))
}

fn consume<'storage, T>(
    range: TableRange<T>,
    pool: &'storage [T],
    cursor: &mut usize,
) -> Option<&'storage [T]> {
    (range.start() == *cursor).then_some(())?;
    let values = range.get(pool)?;
    *cursor = cursor.checked_add(values.len())?;
    Some(values)
}

fn run<C: TypedStorageValidationControl, R>(
    control: &mut C,
    action: impl FnOnce() -> Option<R>,
) -> Result<Option<R>, C::Error> {
    Ok(control.run_unit(action)?.flatten())
}
