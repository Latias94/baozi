use baozi_operation::__retained_store_integration::activation::{
    CanonicalLane, CanonicalRecordBuildRow,
};
use baozi_operation::{
    OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, RecordPlan, ScalarSequence,
    SequencePlan, StoreBuildError, StoreControlledError, StorePlanError, StoreRuntimeError,
    StoredScalar, Utf8Plan,
};

use crate::canonical_scene::store::production::{Presence, ValidatedAnimationBatch};
use crate::canonical_scene::store::schema::animation::{
    ANIMATION_CHANNEL_FIELD_COUNT, ANIMATION_CLIP_FIELD_COUNT, ANIMATION_DIRECTORY_FIELD_COUNT,
    ANIMATION_SCHEMA_REVISION, ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS,
    ANIMATION_TARGET_NODE_ROTATION, ANIMATION_TARGET_NODE_SCALE, ANIMATION_TARGET_NODE_TRANSLATION,
    AnimationChannelTableSchema, AnimationClipNameSchema, AnimationClipTableSchema,
    AnimationDirectorySchema, AnimationInTangentsSchema, AnimationOutTangentsSchema,
    AnimationTimesSchema, AnimationValuesSchema, encode_animation_extrapolation,
    encode_animation_interpolation, encode_animation_target_kind, encode_animation_time_domain,
};
use crate::canonical_scene::store::schema::deformation::DeformationMeshInstanceTableSchema;
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, NodeSceneNodeTableSchema, PRESENCE_ABSENT, PRESENCE_PRESENT,
};
use crate::canonical_scene::store::validation::AnimationMeteredCounts;

use super::{
    checked_validation_work_sum, optional_record_validation_work,
    optional_sequence_validation_work, optional_utf8_validation_work, record_validation_work,
};

pub(super) struct AnimationMaterializationPlans {
    clips: Option<RecordPlan>,
    channels: Option<RecordPlan>,
    times: Option<SequencePlan<f64>>,
    values: Option<SequencePlan<f32>>,
    in_tangents: Option<SequencePlan<f32>>,
    out_tangents: Option<SequencePlan<f32>>,
    clip_names: Option<Utf8Plan>,
    directory: RecordPlan,
    clip_count: u32,
    clip_name_byte_count: u32,
    channel_count: u32,
    time_scalar_count: u64,
    value_scalar_count: u64,
    in_tangent_scalar_count: u64,
    out_tangent_scalar_count: u64,
    table_block_count: usize,
    payload_block_count: u32,
    canonical_block_count: usize,
}

impl AnimationMaterializationPlans {
    pub(super) fn new<R>(batch: &ValidatedAnimationBatch<'_, R>) -> Result<Self, StorePlanError> {
        let clip_count = batch.clip_count();
        let clip_name_byte_count = batch.clip_name_byte_count();
        let channel_count = batch.channel_count();
        let time_scalar_count = batch.time_scalar_count();
        let value_scalar_count = batch.value_scalar_count();
        let in_tangent_scalar_count = batch.in_tangent_scalar_count();
        let out_tangent_scalar_count = batch.out_tangent_scalar_count();

        let clips = record_plan_if_nonempty(clip_count, ANIMATION_CLIP_FIELD_COUNT)?;
        let channels = record_plan_if_nonempty(channel_count, ANIMATION_CHANNEL_FIELD_COUNT)?;
        let times = sequence_plan_if_nonempty(time_scalar_count)?;
        let values = sequence_plan_if_nonempty(value_scalar_count)?;
        let in_tangents = sequence_plan_if_nonempty(in_tangent_scalar_count)?;
        let out_tangents = sequence_plan_if_nonempty(out_tangent_scalar_count)?;
        let clip_names = if clip_name_byte_count == 0 {
            None
        } else {
            Some(Utf8Plan::new(
                usize::try_from(clip_name_byte_count).map_err(|_| StorePlanError::SizeOverflow)?,
            )?)
        };

        let payload_block_count = [
            times.is_some(),
            values.is_some(),
            in_tangents.is_some(),
            out_tangents.is_some(),
            clip_names.is_some(),
        ]
        .into_iter()
        .try_fold(0_u32, |count, present| {
            count.checked_add(u32::from(present))
        })
        .ok_or(StorePlanError::SizeOverflow)?;
        let invalid_empty_shape = channel_count == 0
            && (time_scalar_count != 0
                || value_scalar_count != 0
                || in_tangent_scalar_count != 0
                || out_tangent_scalar_count != 0);
        let invalid_nonempty_shape = channel_count != 0
            && (clip_count == 0 || time_scalar_count == 0 || value_scalar_count == 0);
        if payload_block_count != batch.payload_block_count()
            || invalid_empty_shape
            || invalid_nonempty_shape
            || in_tangent_scalar_count != out_tangent_scalar_count
        {
            return Err(StorePlanError::InvalidBlockTable);
        }

        let table_block_count = [clips.is_some(), channels.is_some()]
            .into_iter()
            .filter(|present| *present)
            .count();
        let canonical_block_count = 1_usize
            .checked_add(table_block_count)
            .and_then(|count| {
                usize::try_from(payload_block_count)
                    .ok()
                    .and_then(|payloads| count.checked_add(payloads))
            })
            .ok_or(StorePlanError::SizeOverflow)?;

        Ok(Self {
            clips,
            channels,
            times,
            values,
            in_tangents,
            out_tangents,
            clip_names,
            directory: RecordPlan::new(1, ANIMATION_DIRECTORY_FIELD_COUNT)?,
            clip_count,
            clip_name_byte_count,
            channel_count,
            time_scalar_count,
            value_scalar_count,
            in_tangent_scalar_count,
            out_tangent_scalar_count,
            table_block_count,
            payload_block_count,
            canonical_block_count,
        })
    }

    pub(super) const fn canonical_block_count(&self) -> usize {
        self.canonical_block_count
    }

    pub(super) const fn table_block_count(&self) -> usize {
        self.table_block_count
    }

    pub(super) const fn payload_block_count(&self) -> u32 {
        self.payload_block_count
    }

    pub(super) const fn clip_count(&self) -> u32 {
        self.clip_count
    }

    pub(super) const fn channel_count(&self) -> u32 {
        self.channel_count
    }

    pub(super) const fn time_scalar_count(&self) -> u64 {
        self.time_scalar_count
    }

    pub(super) const fn value_scalar_count(&self) -> u64 {
        self.value_scalar_count
    }

    pub(super) const fn in_tangent_scalar_count(&self) -> u64 {
        self.in_tangent_scalar_count
    }

    pub(super) const fn out_tangent_scalar_count(&self) -> u64 {
        self.out_tangent_scalar_count
    }

    pub(super) const fn metered_counts(&self) -> AnimationMeteredCounts {
        AnimationMeteredCounts::new(
            self.clip_count,
            self.channel_count,
            self.time_scalar_count,
            self.value_scalar_count,
            self.in_tangent_scalar_count,
            self.out_tangent_scalar_count,
            self.payload_block_count,
            self.clip_name_byte_count,
        )
    }

    pub(super) fn validation_work_units(&self) -> Result<u64, StorePlanError> {
        checked_validation_work_sum([
            optional_record_validation_work(self.clips)?,
            optional_record_validation_work(self.channels)?,
            optional_sequence_validation_work(self.times)?,
            optional_sequence_validation_work(self.values)?,
            optional_sequence_validation_work(self.in_tangents)?,
            optional_sequence_validation_work(self.out_tangents)?,
            optional_utf8_validation_work(self.clip_names)?,
            record_validation_work(self.directory)?,
        ])
    }

    fn matches_batch<R>(&self, batch: &ValidatedAnimationBatch<'_, R>) -> bool {
        self.clip_count == batch.clip_count()
            && self.clip_name_byte_count == batch.clip_name_byte_count()
            && self.channel_count == batch.channel_count()
            && self.time_scalar_count == batch.time_scalar_count()
            && self.value_scalar_count == batch.value_scalar_count()
            && self.in_tangent_scalar_count == batch.in_tangent_scalar_count()
            && self.out_tangent_scalar_count == batch.out_tangent_scalar_count()
            && self.payload_block_count == batch.payload_block_count()
    }
}

#[derive(Clone, Copy)]
pub(super) struct AnimationTargetTableRefs {
    nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
    mesh_instances: Option<CanonicalBlockRef<DeformationMeshInstanceTableSchema>>,
}

impl AnimationTargetTableRefs {
    pub(super) const fn new(
        nodes: CanonicalBlockRef<NodeSceneNodeTableSchema>,
        mesh_instances: Option<CanonicalBlockRef<DeformationMeshInstanceTableSchema>>,
    ) -> Self {
        Self {
            nodes,
            mesh_instances,
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct RetainedAnimationRefs {
    pub(in crate::canonical_scene::store) directory: CanonicalBlockRef<AnimationDirectorySchema>,
    pub(in crate::canonical_scene::store) clips:
        Option<CanonicalBlockRef<AnimationClipTableSchema>>,
    pub(in crate::canonical_scene::store) channels:
        Option<CanonicalBlockRef<AnimationChannelTableSchema>>,
    pub(in crate::canonical_scene::store) payloads: AnimationPayloadRefs,
}

impl RetainedAnimationRefs {
    pub(super) const fn directory(self) -> CanonicalBlockRef<AnimationDirectorySchema> {
        self.directory
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct AnimationPayloadRefs {
    pub(in crate::canonical_scene::store) clip_names:
        Option<CanonicalBlockRef<AnimationClipNameSchema>>,
    pub(in crate::canonical_scene::store) times: Option<CanonicalBlockRef<AnimationTimesSchema>>,
    pub(in crate::canonical_scene::store) values: Option<CanonicalBlockRef<AnimationValuesSchema>>,
    pub(in crate::canonical_scene::store) in_tangents:
        Option<CanonicalBlockRef<AnimationInTangentsSchema>>,
    pub(in crate::canonical_scene::store) out_tangents:
        Option<CanonicalBlockRef<AnimationOutTangentsSchema>>,
}

#[derive(Clone, Copy)]
enum AnimationF32PayloadPart {
    Values,
    InTangents,
    OutTangents,
}

pub(super) fn retain_animation<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &AnimationMaterializationPlans,
    batch: &ValidatedAnimationBatch<'_, R>,
    targets: AnimationTargetTableRefs,
    failure: StoreBuildError,
) -> Result<RetainedAnimationRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if !plans.matches_batch(batch) {
        return Err(StoreRuntimeError::Build(failure));
    }

    let times = match plans.times {
        Some(plan) => Some(retain_animation_times(lane, plan, plans, batch, failure)?),
        None => None,
    };
    let values = match plans.values {
        Some(plan) => Some(retain_animation_f32_payload(
            lane,
            plan,
            AnimationValuesSchema,
            AnimationF32PayloadPart::Values,
            plans,
            batch,
            failure,
        )?),
        None => None,
    };
    let in_tangents = match plans.in_tangents {
        Some(plan) => Some(retain_animation_f32_payload(
            lane,
            plan,
            AnimationInTangentsSchema,
            AnimationF32PayloadPart::InTangents,
            plans,
            batch,
            failure,
        )?),
        None => None,
    };
    let out_tangents = match plans.out_tangents {
        Some(plan) => Some(retain_animation_f32_payload(
            lane,
            plan,
            AnimationOutTangentsSchema,
            AnimationF32PayloadPart::OutTangents,
            plans,
            batch,
            failure,
        )?),
        None => None,
    };
    let clip_names = match plans.clip_names {
        Some(plan) => Some(CanonicalBlockRef::from_store(lane.try_retain_utf8(
            plan,
            AnimationClipNameSchema,
            |control| {
                let mut offset = 0_u32;
                for index in 0..usize::try_from(plans.clip_count).map_err(|_| failure)? {
                    let clip = batch.clip(index).ok_or(failure)?;
                    if clip.name_offset() != offset {
                        return Err(failure.into());
                    }
                    if let Presence::Present(name) = clip.name() {
                        control.write_str(name)?;
                    }
                    offset = offset.checked_add(clip.name_count()).ok_or(failure)?;
                }
                if offset != plans.clip_name_byte_count {
                    return Err(failure.into());
                }
                Ok(())
            },
        )?)),
        None => None,
    };
    let payloads = AnimationPayloadRefs {
        clip_names,
        times,
        values,
        in_tangents,
        out_tangents,
    };

    let channels = match plans.channels {
        Some(plan) => Some(retain_animation_channels(
            lane, plan, plans, batch, payloads, targets, failure,
        )?),
        None => None,
    };
    let clips = match plans.clips {
        Some(plan) => Some(retain_animation_clips(
            lane,
            plan,
            plans,
            batch,
            channels,
            payloads.clip_names,
            failure,
        )?),
        None => None,
    };
    let directory = retain_animation_directory(lane, plans, clips, channels, payloads, failure)?;
    Ok(RetainedAnimationRefs {
        directory,
        clips,
        channels,
        payloads,
    })
}

fn retain_animation_times<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<f64>,
    plans: &AnimationMaterializationPlans,
    batch: &ValidatedAnimationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AnimationTimesSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_sequence(plan, AnimationTimesSchema, |control| {
        let channel_count = usize::try_from(plans.channel_count).map_err(|_| failure)?;
        let mut scalar_offset = 0_u64;
        for channel_index in 0..channel_count {
            let channel = batch.channel(channel_index).ok_or(failure)?;
            let values = channel.times();
            let count = u32::try_from(values.len()).map_err(|_| failure)?;
            let expected_offset = u32::try_from(scalar_offset).map_err(|_| failure)?;
            if values.is_empty()
                || channel.time_offset() != expected_offset
                || channel.time_count() != count
            {
                return Err(failure.into());
            }
            control.extend_from_slice(values)?;
            scalar_offset = scalar_offset.checked_add(u64::from(count)).ok_or(failure)?;
        }
        if scalar_offset != plans.time_scalar_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_animation_f32_payload<'lane, O, PP, U, L, R, Tag>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: SequencePlan<f32>,
    tag: Tag,
    part: AnimationF32PayloadPart,
    plans: &AnimationMaterializationPlans,
    batch: &ValidatedAnimationBatch<'_, R>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<Tag>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<f32>>,
{
    let expected_total = match part {
        AnimationF32PayloadPart::Values => plans.value_scalar_count,
        AnimationF32PayloadPart::InTangents => plans.in_tangent_scalar_count,
        AnimationF32PayloadPart::OutTangents => plans.out_tangent_scalar_count,
    };
    lane.try_retain_sequence(plan, tag, |control| {
        let channel_count = usize::try_from(plans.channel_count).map_err(|_| failure)?;
        let mut scalar_offset = 0_u64;
        for channel_index in 0..channel_count {
            let channel = batch.channel(channel_index).ok_or(failure)?;
            let (authored_offset, planned_count, values) = match part {
                AnimationF32PayloadPart::Values => (
                    Presence::Present(channel.value_offset()),
                    channel.value_count(),
                    Presence::Present(channel.values()),
                ),
                AnimationF32PayloadPart::InTangents => (
                    channel.in_tangent_offset(),
                    channel.in_tangent_count(),
                    channel.in_tangents(),
                ),
                AnimationF32PayloadPart::OutTangents => (
                    channel.out_tangent_offset(),
                    channel.out_tangent_count(),
                    channel.out_tangents(),
                ),
            };
            match (authored_offset, values) {
                (Presence::Absent, Presence::Absent) if planned_count == 0 => {}
                (Presence::Present(offset), Presence::Present(values)) => {
                    let count = u32::try_from(values.len()).map_err(|_| failure)?;
                    let expected_offset = u32::try_from(scalar_offset).map_err(|_| failure)?;
                    if values.is_empty() || offset != expected_offset || planned_count != count {
                        return Err(failure.into());
                    }
                    control.extend_from_slice(values)?;
                    scalar_offset = scalar_offset.checked_add(u64::from(count)).ok_or(failure)?;
                }
                _ => return Err(failure.into()),
            }
        }
        if scalar_offset != expected_total {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_animation_channels<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    plans: &AnimationMaterializationPlans,
    batch: &ValidatedAnimationBatch<'_, R>,
    payloads: AnimationPayloadRefs,
    targets: AnimationTargetTableRefs,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AnimationChannelTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, AnimationChannelTableSchema, |control| {
        let times = payloads.times.ok_or(failure)?;
        let values = payloads.values.ok_or(failure)?;
        let channel_count = usize::try_from(plans.channel_count).map_err(|_| failure)?;
        for channel_index in 0..channel_count {
            let channel = batch.channel(channel_index).ok_or(failure)?;
            let expected_value_count = channel
                .time_count()
                .checked_mul(channel.value_width())
                .ok_or(failure)?;
            if channel.time_count() == 0
                || channel.value_width() == 0
                || channel.value_count() != expected_value_count
            {
                return Err(failure.into());
            }
            let target_kind = encode_animation_target_kind(channel.target());
            control.write_row(|row| {
                row.write_scalar(channel.source_index())?;
                row.write_scalar(target_kind)?;
                row.write_scalar(channel.canonical_target_index())?;
                write_animation_target_ref(row, target_kind, targets, failure)?;
                row.write_scalar(encode_animation_interpolation(channel.interpolation()))?;
                row.write_scalar(encode_animation_extrapolation(channel.pre_extrapolation()))?;
                row.write_scalar(encode_animation_extrapolation(channel.post_extrapolation()))?;
                row.write_scalar(channel.time_count())?;
                row.write_scalar(channel.value_width())?;
                row.write_scalar(channel.time_offset())?;
                row.write_scalar(channel.time_count())?;
                row.write_sequence_ref(times.into_store())?;
                row.write_scalar(channel.value_offset())?;
                row.write_scalar(channel.value_count())?;
                row.write_sequence_ref(values.into_store())?;
                write_optional_channel_sequence_ref(
                    row,
                    channel.in_tangent_offset(),
                    channel.in_tangent_count(),
                    channel.value_count(),
                    payloads.in_tangents,
                    failure,
                )?;
                write_optional_channel_sequence_ref(
                    row,
                    channel.out_tangent_offset(),
                    channel.out_tangent_count(),
                    channel.value_count(),
                    payloads.out_tangents,
                    failure,
                )
            })?;
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_animation_clips<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plan: RecordPlan,
    plans: &AnimationMaterializationPlans,
    batch: &ValidatedAnimationBatch<'_, R>,
    channels: Option<CanonicalBlockRef<AnimationChannelTableSchema>>,
    names: Option<CanonicalBlockRef<AnimationClipNameSchema>>,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AnimationClipTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plan, AnimationClipTableSchema, |control| {
        let clip_count = usize::try_from(plans.clip_count).map_err(|_| failure)?;
        let mut channel_offset = 0_u32;
        for clip_index in 0..clip_count {
            let clip = batch.clip(clip_index).ok_or(failure)?;
            let range = clip.channel_range();
            if range.offset() != channel_offset {
                return Err(failure.into());
            }
            channel_offset = channel_offset.checked_add(range.count()).ok_or(failure)?;
            let (time_domain, rate) = encode_animation_time_domain(clip.time_domain());
            control.write_row(|row| {
                row.write_scalar(clip.source_index())?;
                row.write_scalar(clip.duration())?;
                row.write_scalar(time_domain)?;
                write_optional_rate(row, rate)?;
                row.write_scalar(range.offset())?;
                row.write_scalar(range.count())?;
                write_optional_record_presence(row, range.count() != 0, channels, failure)?;
                row.write_scalar(if matches!(clip.name(), Presence::Present(_)) {
                    PRESENCE_PRESENT
                } else {
                    PRESENCE_ABSENT
                })?;
                row.write_scalar(clip.name_offset())?;
                row.write_scalar(clip.name_count())?;
                if clip.name_count() == 0 {
                    row.write_scalar(0_u8)
                } else {
                    row.write_utf8_ref(names.ok_or(failure)?.into_store())
                }
            })?;
        }
        if channel_offset != plans.channel_count {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_animation_directory<'lane, O, PP, U, L>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &AnimationMaterializationPlans,
    clips: Option<CanonicalBlockRef<AnimationClipTableSchema>>,
    channels: Option<CanonicalBlockRef<AnimationChannelTableSchema>>,
    payloads: AnimationPayloadRefs,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<AnimationDirectorySchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    lane.try_retain_records(plans.directory, AnimationDirectorySchema, |control| {
        control.write_row(|row| {
            row.write_scalar(ANIMATION_SCHEMA_REVISION)?;
            write_optional_record_group(row, plans.clip_count, clips, failure)?;
            write_optional_record_group(row, plans.channel_count, channels, failure)?;
            write_optional_sequence_group::<f64, _>(
                row,
                plans.time_scalar_count,
                payloads.times,
                failure,
            )?;
            write_optional_sequence_group::<f32, _>(
                row,
                plans.value_scalar_count,
                payloads.values,
                failure,
            )?;
            write_optional_sequence_group::<f32, _>(
                row,
                plans.in_tangent_scalar_count,
                payloads.in_tangents,
                failure,
            )?;
            write_optional_sequence_group::<f32, _>(
                row,
                plans.out_tangent_scalar_count,
                payloads.out_tangents,
                failure,
            )?;
            row.write_scalar(plans.clip_name_byte_count)?;
            row.write_scalar(if payloads.clip_names.is_some() {
                PRESENCE_PRESENT
            } else {
                PRESENCE_ABSENT
            })?;
            match payloads.clip_names {
                Some(names) => row.write_utf8_ref(names.into_store()),
                None => row.write_scalar(0_u8),
            }
        })
    })
    .map(CanonicalBlockRef::from_store)
}

fn write_animation_target_ref(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    target_kind: u8,
    targets: AnimationTargetTableRefs,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError> {
    match target_kind {
        ANIMATION_TARGET_NODE_TRANSLATION
        | ANIMATION_TARGET_NODE_ROTATION
        | ANIMATION_TARGET_NODE_SCALE => row.write_record_ref(targets.nodes.into_store()),
        ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS => {
            row.write_record_ref(targets.mesh_instances.ok_or(failure)?.into_store())
        }
        _ => Err(failure.into()),
    }
}

fn write_optional_rate(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    rate: Presence<f64>,
) -> Result<(), StoreControlledError> {
    match rate {
        Presence::Absent => {
            row.write_scalar(PRESENCE_ABSENT)?;
            row.write_scalar(0_u8)
        }
        Presence::Present(rate) => {
            row.write_scalar(PRESENCE_PRESENT)?;
            row.write_scalar(rate)
        }
    }
}

fn write_optional_record_presence<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    present: bool,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    if present {
        row.write_scalar(PRESENCE_PRESENT)?;
        row.write_record_ref(reference.ok_or(failure)?.into_store())
    } else {
        row.write_scalar(PRESENCE_ABSENT)?;
        row.write_scalar(0_u8)
    }
}

fn write_optional_channel_sequence_ref<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    offset: Presence<u32>,
    count: u32,
    expected_present_count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<f32>>,
{
    match offset {
        Presence::Absent if count == 0 => {
            row.write_scalar(PRESENCE_ABSENT)?;
            row.write_scalar(0_u32)?;
            row.write_scalar(0_u32)?;
            row.write_scalar(0_u8)
        }
        Presence::Present(offset) if count != 0 && count == expected_present_count => {
            row.write_scalar(PRESENCE_PRESENT)?;
            row.write_scalar(offset)?;
            row.write_scalar(count)?;
            row.write_sequence_ref(reference.ok_or(failure)?.into_store())
        }
        Presence::Absent | Presence::Present(_) => Err(failure.into()),
    }
}

fn write_optional_record_group<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    row.write_scalar(count)?;
    if count == 0 && reference.is_some() {
        return Err(failure.into());
    }
    write_optional_record_presence(row, count != 0, reference, failure)
}

fn write_optional_sequence_group<T, Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u64,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    row.write_scalar(count)?;
    if count == 0 {
        if reference.is_some() {
            return Err(failure.into());
        }
        row.write_scalar(PRESENCE_ABSENT)?;
        row.write_scalar(0_u8)
    } else {
        row.write_scalar(PRESENCE_PRESENT)?;
        row.write_sequence_ref(reference.ok_or(failure)?.into_store())
    }
}

fn record_plan_if_nonempty(
    count: u32,
    fields: usize,
) -> Result<Option<RecordPlan>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(RecordPlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
            fields,
        )?))
    }
}

fn sequence_plan_if_nonempty<T: StoredScalar>(
    count: u64,
) -> Result<Option<SequencePlan<T>>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(SequencePlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}
