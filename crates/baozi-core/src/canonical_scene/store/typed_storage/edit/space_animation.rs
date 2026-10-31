//! Source-bound spatial conversion of retained animation curves.
//!
//! The source is centrally validated. Shape and work use only seven lengths; numeric
//! preflight visits each channel and each F32 scalar under control. A scalar action
//! may recompute one bounded TRS tuple, never a whole curve or morph-weight tuple.
//! The host admits both tables and all five pools before `materialize`.
//!
//! The complete Scene plan retains this source binding through publication and checks
//! correspondence for rebuilt and shared owners. Profile selection uses
//! `channel_changes` after numeric preflight. Each complete
//! selection pass costs channels.len() + values.len() + in_tangents.len() +
//! out_tangents.len(), computed in O(1) and admitted separately for lineage/state
//! passes. Pure winding/declaration changes bypass selection entirely.

use baozi_operation::{CapacityPlan, StorePlanError};

use crate::SceneAnimationSamplePart;
use crate::animation::{AnimationInterpolation, AnimationTimeDomain};
use crate::math::Quat;
use crate::space_conversion::{NumericConversionError, SpaceConversion};

use super::super::TableRange;
use super::super::animation::build::AnimationStoragePlan;
use super::super::animation::{
    AnimationStorage, StoredAnimationChannel, StoredAnimationClip, StoredAnimationTarget,
};
use super::super::build::{
    BuildResult, TypedStorageBuildControl, TypedStorageBuildError, allocate, append, finish, push,
};
use super::super::validation::TypedStorageValidationControl;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(in crate::canonical_scene::store) enum AnimationSpacePlanError<E> {
    Control(E),
    Numeric {
        /// Dense channel ordinal, independent of its authored source_index.
        channel: usize,
        part: SceneAnimationSamplePart,
        /// Zero-based key ordinal within this channel and stream.
        key: usize,
        source: NumericConversionError,
    },
    Plan(StorePlanError),
    Invariant,
}

/// Fixed-size recipe borrowing the exact immutable source checked by preflight.
/// No converted rows, scalar cache, or input-sized mapping are retained.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct AnimationSpacePlan<'a> {
    source: &'a AnimationStorage,
    conversion: SpaceConversion,
    work: u64,
    changes_storage: bool,
}

impl<'a> AnimationSpacePlan<'a> {
    /// Ordinary numeric failures precede retained admission. Successful preflight
    /// costs one action per channel plus one per value/in/out scalar, allocating nothing.
    pub(in crate::canonical_scene::store) fn try_new<C: TypedStorageBuildControl>(
        source: &'a AnimationStorage,
        conversion: SpaceConversion,
        control: &mut C,
    ) -> Result<Self, AnimationSpacePlanError<C::Error>> {
        let shape = storage_shape(source).map_err(AnimationSpacePlanError::Plan)?;
        let work = shape.work().map_err(AnimationSpacePlanError::Plan)?;
        let mut cursors = [0; 4];
        let mut changes_storage = false;
        for (index, &channel) in source.channels.iter().enumerate() {
            let streams = control
                .run_unit(|| channel_streams(source, channel, &mut cursors))
                .map_err(AnimationSpacePlanError::Control)?
                .ok_or(AnimationSpacePlanError::Invariant)?;
            for (part, values) in streams.parts() {
                for scalar in 0..values.len() {
                    let changed = control
                        .run_unit(|| {
                            convert_scalar(conversion, channel.target, values, scalar)
                                .map(|expected| expected.to_bits() != values[scalar].to_bits())
                        })
                        .map_err(AnimationSpacePlanError::Control)?
                        .map_err(|source| AnimationSpacePlanError::Numeric {
                            channel: index,
                            part,
                            key: scalar / streams.width,
                            source,
                        })?;
                    changes_storage |= changed;
                }
            }
        }
        if !complete(source, cursors) {
            return Err(AnimationSpacePlanError::Invariant);
        }
        Ok(Self {
            source,
            conversion,
            work,
            changes_storage,
        })
    }

    /// Order: clips, channels, times, values, in tangents, out tangents, names.
    pub(in crate::canonical_scene::store) fn capacities(
        self,
    ) -> Result<[CapacityPlan; 7], StorePlanError> {
        storage_shape(self.source)?.capacities()
    }

    /// One action per initialized row/scalar and one string action per name byte.
    pub(in crate::canonical_scene::store) const fn work(self) -> u64 {
        self.work
    }

    pub(in crate::canonical_scene::store) const fn correspondence_work(self) -> u64 {
        self.work
    }

    pub(in crate::canonical_scene::store) fn name_bytes(self) -> usize {
        self.source.names.len()
    }

    /// Actual scalar-bit changes, including signed zeros, not just a nonidentity Q/s.
    pub(in crate::canonical_scene::store) const fn changes_storage(self) -> bool {
        self.changes_storage
    }

    pub(in crate::canonical_scene::store) fn materialize<C: TypedStorageBuildControl>(
        self,
        control: &mut C,
    ) -> BuildResult<AnimationStorage, C::Error> {
        let capacities = self
            .capacities()
            .map_err(|_| TypedStorageBuildError::Invariant)?;
        let [clips, channels, times, values, input, output, names] = capacities;
        let mut actual = AnimationStorage {
            clips: allocate(clips)?,
            channels: allocate(channels)?,
            times: allocate(times)?,
            values: allocate(values)?,
            in_tangents: allocate(input)?,
            out_tangents: allocate(output)?,
            names: allocate(names)?,
        };
        for &clip in &self.source.clips {
            push(&mut actual.clips, clips, clip, control)?;
        }
        append(&mut actual.times, times, &self.source.times, control)?;
        let mut cursors = [0; 4];
        for &channel in &self.source.channels {
            if actual.channels.len() >= channels.desired_capacity()
                || actual.channels.len() == actual.channels.capacity()
            {
                return Err(TypedStorageBuildError::Invariant);
            }
            let streams = control
                .run_unit(|| -> BuildResult<_, C::Error> {
                    let streams = channel_streams(self.source, channel, &mut cursors)
                        .ok_or(TypedStorageBuildError::Invariant)?;
                    actual.channels.push(channel);
                    Ok(streams)
                })
                .map_err(TypedStorageBuildError::Control)??;
            for (source, target, capacity) in [
                (streams.values, &mut actual.values, values),
                (streams.input, &mut actual.in_tangents, input),
                (streams.output, &mut actual.out_tangents, output),
            ] {
                for scalar in 0..source.len() {
                    if target.len() >= capacity.desired_capacity()
                        || target.len() == target.capacity()
                    {
                        return Err(TypedStorageBuildError::Invariant);
                    }
                    control
                        .run_unit(|| -> BuildResult<(), C::Error> {
                            let value =
                                convert_scalar(self.conversion, channel.target, source, scalar)
                                    .map_err(|_| TypedStorageBuildError::Invariant)?;
                            target.push(value);
                            Ok(())
                        })
                        .map_err(TypedStorageBuildError::Control)??;
                }
            }
        }
        for &byte in &self.source.names {
            if actual.names.len() >= names.desired_capacity()
                || actual.names.len() == actual.names.capacity()
            {
                return Err(TypedStorageBuildError::Invariant);
            }
            control
                .run_string_unit(|| actual.names.push(byte))
                .map_err(TypedStorageBuildError::Control)?;
        }
        if !complete(self.source, cursors) {
            return Err(TypedStorageBuildError::Invariant);
        }
        finish(&actual.clips, clips)?;
        finish(&actual.channels, channels)?;
        finish(&actual.times, times)?;
        finish(&actual.values, values)?;
        finish(&actual.in_tangents, input)?;
        finish(&actual.out_tangents, output)?;
        finish(&actual.names, names)?;
        Ok(actual)
    }

    /// Proves every row field, range/presence, and scalar/byte against this exact
    /// source. Duration, rate, and key times use bit equality; no time bounds apply.
    pub(in crate::canonical_scene::store) fn validate_correspondence<
        C: TypedStorageValidationControl,
    >(
        self,
        actual: &AnimationStorage,
        control: &mut C,
    ) -> Result<bool, C::Error> {
        if lengths(self.source) != lengths(actual) {
            return Ok(false);
        }
        for (source, actual) in self.source.clips.iter().zip(&actual.clips) {
            if control.run_unit(|| same_clip(*source, *actual))? != Some(true) {
                return Ok(false);
            }
        }
        for (&source, &actual) in self.source.times.iter().zip(&actual.times) {
            if control.run_unit(|| source.to_bits() == actual.to_bits())? != Some(true) {
                return Ok(false);
            }
        }
        let mut source_cursors = [0; 4];
        let mut actual_cursors = [0; 4];
        for (&source, &row) in self.source.channels.iter().zip(&actual.channels) {
            let Some(Some((source_streams, actual_streams))) = control.run_unit(|| {
                same_channel(source, row).then_some(())?;
                Some((
                    channel_streams(self.source, source, &mut source_cursors)?,
                    channel_streams(actual, row, &mut actual_cursors)?,
                ))
            })?
            else {
                return Ok(false);
            };
            for (source_values, actual_values) in [
                (source_streams.values, actual_streams.values),
                (source_streams.input, actual_streams.input),
                (source_streams.output, actual_streams.output),
            ] {
                for (scalar, &value) in actual_values.iter().enumerate() {
                    if control.run_unit(|| {
                        convert_scalar(self.conversion, source.target, source_values, scalar)
                            .is_ok_and(|expected| expected.to_bits() == value.to_bits())
                    })? != Some(true)
                    {
                        return Ok(false);
                    }
                }
            }
        }
        for (&source, &actual) in self.source.names.iter().zip(&actual.names) {
            if control.run_unit(|| source == actual)? != Some(true) {
                return Ok(false);
            }
        }
        Ok(complete(self.source, source_cursors) && complete(actual, actual_cursors))
    }
}

/// Selects one changed animation owner after numeric preflight, without a cache.
/// One bounded descriptor action is followed by one action for every values/in/out
/// scalar, including unchanged morph weights. Finding a change never stops the scan.
/// Missing or invalid ranges and unadmitted numeric failures return false. Production
/// callers select only validated owners after complete numeric preflight.
/// Control failures retain their exact completed prefix.
pub(in crate::canonical_scene::store) fn channel_changes<C: TypedStorageBuildControl>(
    source: &AnimationStorage,
    channel: usize,
    conversion: SpaceConversion,
    control: &mut C,
) -> Result<bool, C::Error> {
    let Some((target, streams)) = control.run_unit(|| {
        let row = *source.channels.get(channel)?;
        // Random access needs only this row's starts, never preceding rows.
        let mut cursors = [
            row.times.start(),
            row.values.start(),
            row.in_tangents.map_or(0, TableRange::start),
            row.out_tangents.map_or(0, TableRange::start),
        ];
        Some((row.target, channel_streams(source, row, &mut cursors)?))
    })?
    else {
        return Ok(false);
    };
    let mut valid = true;
    let mut changed = false;
    for (_, values) in streams.parts() {
        for scalar in 0..values.len() {
            let scalar_changed = control.run_unit(|| {
                convert_scalar(conversion, target, values, scalar)
                    .ok()
                    .map(|expected| expected.to_bits() != values[scalar].to_bits())
            })?;
            valid &= scalar_changed.is_some();
            changed |= scalar_changed.unwrap_or(false);
        }
    }
    Ok(valid && changed)
}

fn lengths(source: &AnimationStorage) -> [usize; 7] {
    [
        source.clips.len(),
        source.channels.len(),
        source.times.len(),
        source.values.len(),
        source.in_tangents.len(),
        source.out_tangents.len(),
        source.names.len(),
    ]
}

fn storage_shape(source: &AnimationStorage) -> Result<AnimationStoragePlan, StorePlanError> {
    let [clips, channels, times, values, input, output, names] = lengths(source).map(|n| n as u64);
    AnimationStoragePlan::for_shape([clips, channels], [times, values, input, output, names])
}

struct ChannelStreams<'a> {
    width: usize,
    values: &'a [f32],
    input: &'a [f32],
    output: &'a [f32],
}

impl<'a> ChannelStreams<'a> {
    fn parts(&self) -> [(SceneAnimationSamplePart, &'a [f32]); 3] {
        [
            (SceneAnimationSamplePart::Values, self.values),
            (SceneAnimationSamplePart::InTangents, self.input),
            (SceneAnimationSamplePart::OutTangents, self.output),
        ]
    }
}

/// O(1) range/presence checks on a centrally valid channel; no sample/time scan.
fn channel_streams<'a>(
    source: &'a AnimationStorage,
    channel: StoredAnimationChannel,
    cursors: &mut [usize; 4],
) -> Option<ChannelStreams<'a>> {
    let width = usize::try_from(channel.value_width()?).ok()?;
    consume(channel.times, &source.times, &mut cursors[0])?;
    let values = consume(channel.values, &source.values, &mut cursors[1])?;
    let (input, output) = match (
        channel.interpolation,
        channel.in_tangents,
        channel.out_tangents,
    ) {
        (AnimationInterpolation::CubicSpline, Some(input), Some(output)) => {
            let input = consume(input, &source.in_tangents, &mut cursors[2])?;
            let output = consume(output, &source.out_tangents, &mut cursors[3])?;
            (input.len() == values.len() && output.len() == values.len()).then_some(())?;
            (input, output)
        }
        (AnimationInterpolation::Step | AnimationInterpolation::Linear, None, None) => {
            (&[][..], &[][..])
        }
        _ => return None,
    };
    Some(ChannelStreams {
        width,
        values,
        input,
        output,
    })
}

fn consume<'a, T>(range: TableRange<T>, values: &'a [T], cursor: &mut usize) -> Option<&'a [T]> {
    (range.start() == *cursor).then_some(())?;
    let values = range.get(values)?;
    *cursor = cursor.checked_add(values.len())?;
    Some(values)
}

fn complete(source: &AnimationStorage, cursors: [usize; 4]) -> bool {
    cursors
        == [
            source.times.len(),
            source.values.len(),
            source.in_tangents.len(),
            source.out_tangents.len(),
        ]
}

/// Every caller supplies a shape-checked stream and an in-bounds scalar ordinal.
/// Morph width may be arbitrary: only this scalar is read for a morph channel.
fn convert_scalar(
    conversion: SpaceConversion,
    target: StoredAnimationTarget,
    values: &[f32],
    scalar: usize,
) -> Result<f32, NumericConversionError> {
    match target {
        StoredAnimationTarget::NodeTranslation(_) | StoredAnimationTarget::NodeScale(_) => {
            let start = scalar / 3 * 3;
            let tuple = [values[start], values[start + 1], values[start + 2]];
            let converted = if matches!(target, StoredAnimationTarget::NodeTranslation(_)) {
                conversion.point(tuple)?
            } else {
                conversion.scale(tuple)?
            };
            Ok(converted[scalar % 3])
        }
        StoredAnimationTarget::NodeRotation(_) => {
            let start = scalar / 4 * 4;
            // rotation() checks finiteness only and applies (det(Q)Q xyz, w).
            // It is the same linear map for xyzw samples and arbitrary Vec4
            // derivatives, including zero/nonunit derivatives and q/-q keys.
            let converted = conversion.rotation(Quat::from_xyzw(
                values[start],
                values[start + 1],
                values[start + 2],
                values[start + 3],
            ))?;
            Ok([converted.x, converted.y, converted.z, converted.w][scalar % 4])
        }
        StoredAnimationTarget::MeshInstanceMorphWeights(_) => {
            let value = values[scalar];
            if value.is_finite() {
                Ok(value)
            } else {
                Err(NumericConversionError::NonFiniteOutput)
            }
        }
    }
}

fn same_clip(source: StoredAnimationClip, actual: StoredAnimationClip) -> bool {
    source.source_index == actual.source_index
        && source.name == actual.name
        && source.duration.to_bits() == actual.duration.to_bits()
        && same_time_domain(source.time_domain, actual.time_domain)
        && source.channels == actual.channels
}

fn same_time_domain(source: AnimationTimeDomain, actual: AnimationTimeDomain) -> bool {
    match (source, actual) {
        (AnimationTimeDomain::Seconds, AnimationTimeDomain::Seconds) => true,
        (
            AnimationTimeDomain::Frames { rate: source },
            AnimationTimeDomain::Frames { rate: actual },
        ) => source.to_bits() == actual.to_bits(),
        (
            AnimationTimeDomain::Ticks { rate: source },
            AnimationTimeDomain::Ticks { rate: actual },
        ) => source.map(f64::to_bits) == actual.map(f64::to_bits),
        _ => false,
    }
}

fn same_channel(source: StoredAnimationChannel, actual: StoredAnimationChannel) -> bool {
    source.source_index == actual.source_index
        && source.target == actual.target
        && source.interpolation == actual.interpolation
        && source.pre_extrapolation == actual.pre_extrapolation
        && source.post_extrapolation == actual.post_extrapolation
        && source.times == actual.times
        && source.values == actual.values
        && source.in_tangents == actual.in_tangents
        && source.out_tangents == actual.out_tangents
}
