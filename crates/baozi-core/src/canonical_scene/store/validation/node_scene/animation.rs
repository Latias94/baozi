use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, CanonicalSequenceRef, RecordBlock, ScalarSequence,
    StoreValidationSession, StoredScalar,
};

use crate::canonical_scene::store::production::{Presence, ValidatedAnimationBatch};
use crate::canonical_scene::store::schema::animation::{
    ANIMATION_CHANNEL_FIELD_COUNT, ANIMATION_CLIP_FIELD_COUNT, ANIMATION_DIRECTORY_FIELD_COUNT,
    ANIMATION_SCHEMA_REVISION, ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS,
    ANIMATION_TARGET_NODE_ROTATION, ANIMATION_TARGET_NODE_SCALE, ANIMATION_TARGET_NODE_TRANSLATION,
    AnimationChannelTableSchema, AnimationClipTableSchema, AnimationDirectorySchema,
    AnimationInTangentsSchema, AnimationOutTangentsSchema, AnimationTimesSchema,
    AnimationValuesSchema, animation_channel_field, animation_clip_field,
    animation_directory_field, encode_animation_extrapolation, encode_animation_interpolation,
    encode_animation_target_kind, encode_animation_time_domain,
};
use crate::canonical_scene::store::schema::deformation::{
    DEFORMATION_MESH_INSTANCE_FIELD_COUNT, DeformationMeshInstanceTableSchema,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, NODE_SCENE_NODE_FIELD_COUNT, NodeSceneNodeTableSchema,
    PRESENCE_ABSENT, PRESENCE_PRESENT,
};

use super::super::require_scalar;

#[derive(Clone, Copy)]
pub(super) struct AnimationValidatedRefs {
    pub(super) clips: Option<CanonicalBlockRef<AnimationClipTableSchema>>,
    pub(super) channels: Option<CanonicalBlockRef<AnimationChannelTableSchema>>,
    pub(super) times: Option<CanonicalBlockRef<AnimationTimesSchema>>,
    pub(super) values: Option<CanonicalBlockRef<AnimationValuesSchema>>,
    pub(super) in_tangents: Option<CanonicalBlockRef<AnimationInTangentsSchema>>,
    pub(super) out_tangents: Option<CanonicalBlockRef<AnimationOutTangentsSchema>>,
}

pub(super) fn validate_animation_graph<R>(
    session: &mut StoreValidationSession<'_, '_>,
    animation_ref: CanonicalRecordRef<AnimationDirectorySchema>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    deformation_mesh_instances: Option<CanonicalRecordRef<DeformationMeshInstanceTableSchema>>,
    expected: &ValidatedAnimationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<AnimationValidatedRefs> {
    observe_work(1)?;
    let directory = session.canonical_records(animation_ref)?;
    if directory.rows() != 1 || directory.fields() != ANIMATION_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, animation_directory_field::SCHEMA_REVISION),
        ANIMATION_SCHEMA_REVISION,
    )?;
    let invalid_empty_shape = expected.channel_count() == 0
        && (expected.time_scalar_count() != 0
            || expected.value_scalar_count() != 0
            || expected.in_tangent_scalar_count() != 0
            || expected.out_tangent_scalar_count() != 0);
    let invalid_nonempty_shape = expected.channel_count() != 0
        && (expected.clip_count() == 0
            || expected.time_scalar_count() == 0
            || expected.value_scalar_count() == 0);
    if invalid_empty_shape
        || invalid_nonempty_shape
        || expected.in_tangent_scalar_count() != expected.out_tangent_scalar_count()
    {
        return None;
    }

    let nodes = session.canonical_records(nodes_ref)?;
    if nodes.rows() == 0 || nodes.fields() != NODE_SCENE_NODE_FIELD_COUNT {
        return None;
    }
    let node_rows = nodes.rows();
    let deformation_mesh_instance_rows = match deformation_mesh_instances {
        Some(reference) => {
            let table = session.canonical_records(reference)?;
            if table.fields() != DEFORMATION_MESH_INSTANCE_FIELD_COUNT || table.rows() == 0 {
                return None;
            }
            Some(table.rows())
        }
        None => None,
    };

    let clips = optional_record_ref::<AnimationClipTableSchema>(
        session,
        directory,
        animation_directory_field::CLIP_COUNT,
        animation_directory_field::CLIPS_PRESENCE,
        animation_directory_field::CLIPS,
        expected.clip_count(),
        ANIMATION_CLIP_FIELD_COUNT,
    )?;
    let channels = optional_record_ref::<AnimationChannelTableSchema>(
        session,
        directory,
        animation_directory_field::CHANNEL_COUNT,
        animation_directory_field::CHANNELS_PRESENCE,
        animation_directory_field::CHANNELS,
        expected.channel_count(),
        ANIMATION_CHANNEL_FIELD_COUNT,
    )?;
    let times = optional_sequence_ref::<f64, AnimationTimesSchema>(
        session,
        directory,
        animation_directory_field::TIME_SCALAR_COUNT,
        animation_directory_field::TIMES_PRESENCE,
        animation_directory_field::TIMES,
        expected.time_scalar_count(),
    )?;
    let values = optional_sequence_ref::<f32, AnimationValuesSchema>(
        session,
        directory,
        animation_directory_field::VALUE_SCALAR_COUNT,
        animation_directory_field::VALUES_PRESENCE,
        animation_directory_field::VALUES,
        expected.value_scalar_count(),
    )?;
    let in_tangents = optional_sequence_ref::<f32, AnimationInTangentsSchema>(
        session,
        directory,
        animation_directory_field::IN_TANGENT_SCALAR_COUNT,
        animation_directory_field::IN_TANGENTS_PRESENCE,
        animation_directory_field::IN_TANGENTS,
        expected.in_tangent_scalar_count(),
    )?;
    let out_tangents = optional_sequence_ref::<f32, AnimationOutTangentsSchema>(
        session,
        directory,
        animation_directory_field::OUT_TANGENT_SCALAR_COUNT,
        animation_directory_field::OUT_TANGENTS_PRESENCE,
        animation_directory_field::OUT_TANGENTS,
        expected.out_tangent_scalar_count(),
    )?;

    let payload_block_count = [
        times.is_some(),
        values.is_some(),
        in_tangents.is_some(),
        out_tangents.is_some(),
    ]
    .into_iter()
    .try_fold(0_u32, |count, present| {
        count.checked_add(u32::from(present))
    })?;
    if payload_block_count != expected.payload_block_count() {
        return None;
    }

    validate_channel_table(
        session,
        channels,
        times,
        values,
        in_tangents,
        out_tangents,
        nodes_ref,
        node_rows,
        deformation_mesh_instances,
        deformation_mesh_instance_rows,
        expected,
        observe_work,
    )?;
    validate_clip_table(session, clips, channels, expected, observe_work)?;

    Some(AnimationValidatedRefs {
        clips: clips.map(CanonicalBlockRef::from_store),
        channels: channels.map(CanonicalBlockRef::from_store),
        times: times.map(CanonicalBlockRef::from_store),
        values: values.map(CanonicalBlockRef::from_store),
        in_tangents: in_tangents.map(CanonicalBlockRef::from_store),
        out_tangents: out_tangents.map(CanonicalBlockRef::from_store),
    })
}

#[allow(clippy::too_many_arguments)]
fn optional_record_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    presence_field: usize,
    reference_field: usize,
    expected_count: u32,
    expected_fields: usize,
) -> Option<Option<CanonicalRecordRef<Tag>>>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    require_scalar(directory.scalar::<u32>(0, count_field), expected_count)?;
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, presence_field), PRESENCE_ABSENT)?;
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        require_scalar(directory.scalar::<u8>(0, presence_field), PRESENCE_PRESENT)?;
        let reference = directory.record_ref::<Tag>(0, reference_field)?;
        let table = session.canonical_records(reference)?;
        if table.rows() != usize::try_from(expected_count).ok()?
            || table.fields() != expected_fields
        {
            return None;
        }
        Some(Some(reference))
    }
}

#[allow(clippy::too_many_arguments)]
fn optional_sequence_ref<T, Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    presence_field: usize,
    reference_field: usize,
    expected_count: u64,
) -> Option<Option<CanonicalSequenceRef<T, Tag>>>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    require_scalar(directory.scalar::<u64>(0, count_field), expected_count)?;
    if expected_count == 0 {
        require_scalar(directory.scalar::<u8>(0, presence_field), PRESENCE_ABSENT)?;
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some(None)
    } else {
        require_scalar(directory.scalar::<u8>(0, presence_field), PRESENCE_PRESENT)?;
        let reference = directory.sequence_ref::<T, Tag>(0, reference_field)?;
        let sequence = session.canonical_sequence(reference)?;
        if sequence.len() != usize::try_from(expected_count).ok()? {
            return None;
        }
        Some(Some(reference))
    }
}

fn validate_clip_table<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<AnimationClipTableSchema>>,
    channel_reference: Option<CanonicalRecordRef<AnimationChannelTableSchema>>,
    expected: &ValidatedAnimationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.clip_count() == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
    let mut channel_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        let clip = expected.clip(row)?;
        let clip_id = u32::try_from(row).ok()?;
        if u32::try_from(clip.input_index()).ok()? != clip_id {
            return None;
        }
        require_scalar(
            table.scalar::<u32>(row, animation_clip_field::CLIP),
            clip_id,
        )?;
        require_float64_bits(
            table.scalar::<f64>(row, animation_clip_field::DURATION),
            clip.duration(),
        )?;
        let (time_domain, rate) = encode_animation_time_domain(clip.time_domain());
        require_scalar(
            table.scalar::<u8>(row, animation_clip_field::TIME_DOMAIN),
            time_domain,
        )?;
        validate_optional_rate(table, row, rate)?;

        let range = clip.channel_range();
        if range.offset() != channel_cursor {
            return None;
        }
        require_scalar(
            table.scalar::<u32>(row, animation_clip_field::CHANNEL_OFFSET),
            range.offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, animation_clip_field::CHANNEL_COUNT),
            range.count(),
        )?;
        validate_optional_record_presence(
            table,
            row,
            animation_clip_field::CHANNELS_PRESENCE,
            animation_clip_field::CHANNELS,
            range.count() != 0,
            channel_reference,
        )?;
        channel_cursor = channel_cursor.checked_add(range.count())?;
    }
    (channel_cursor == expected.channel_count()).then_some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_channel_table<R>(
    session: &mut StoreValidationSession<'_, '_>,
    reference: Option<CanonicalRecordRef<AnimationChannelTableSchema>>,
    times: Option<CanonicalSequenceRef<f64, AnimationTimesSchema>>,
    values: Option<CanonicalSequenceRef<f32, AnimationValuesSchema>>,
    in_tangents: Option<CanonicalSequenceRef<f32, AnimationInTangentsSchema>>,
    out_tangents: Option<CanonicalSequenceRef<f32, AnimationOutTangentsSchema>>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    node_rows: usize,
    deformation_mesh_instances: Option<CanonicalRecordRef<DeformationMeshInstanceTableSchema>>,
    deformation_mesh_instance_rows: Option<usize>,
    expected: &ValidatedAnimationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        return (expected.channel_count() == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
    let times_ref = times?;
    let values_ref = values?;
    let stored_times = session.canonical_sequence(times_ref)?.as_slice();
    let stored_values = session.canonical_sequence(values_ref)?.as_slice();
    let stored_in_tangents = match in_tangents {
        Some(reference) => Some(session.canonical_sequence(reference)?.as_slice()),
        None => None,
    };
    let stored_out_tangents = match out_tangents {
        Some(reference) => Some(session.canonical_sequence(reference)?.as_slice()),
        None => None,
    };

    let mut time_cursor = 0_u64;
    let mut value_cursor = 0_u64;
    let mut in_tangent_cursor = 0_u64;
    let mut out_tangent_cursor = 0_u64;
    for row in 0..table.rows() {
        observe_work(1)?;
        let channel = expected.channel(row)?;
        let channel_id = u32::try_from(row).ok()?;
        if u32::try_from(channel.input_index()).ok()? != channel_id {
            return None;
        }
        require_scalar(
            table.scalar::<u32>(row, animation_channel_field::CHANNEL),
            channel_id,
        )?;

        let target_kind = encode_animation_target_kind(channel.target());
        require_scalar(
            table.scalar::<u8>(row, animation_channel_field::TARGET_KIND),
            target_kind,
        )?;
        require_scalar(
            table.scalar::<u32>(row, animation_channel_field::TARGET_INDEX),
            channel.canonical_target_index(),
        )?;
        validate_target_ref(
            table,
            row,
            target_kind,
            channel.canonical_target_index(),
            nodes_ref,
            node_rows,
            deformation_mesh_instances,
            deformation_mesh_instance_rows,
        )?;

        require_scalar(
            table.scalar::<u8>(row, animation_channel_field::INTERPOLATION),
            encode_animation_interpolation(channel.interpolation()),
        )?;
        require_scalar(
            table.scalar::<u8>(row, animation_channel_field::PRE_EXTRAPOLATION),
            encode_animation_extrapolation(channel.pre_extrapolation()),
        )?;
        require_scalar(
            table.scalar::<u8>(row, animation_channel_field::POST_EXTRAPOLATION),
            encode_animation_extrapolation(channel.post_extrapolation()),
        )?;

        let time_count = u32::try_from(channel.times().len()).ok()?;
        let value_count = u32::try_from(channel.values().len()).ok()?;
        let expected_value_count = time_count.checked_mul(channel.value_width())?;
        if time_count == 0
            || channel.value_width() == 0
            || channel.time_count() != time_count
            || channel.value_count() != value_count
            || value_count != expected_value_count
            || u64::from(channel.time_offset()) != time_cursor
            || u64::from(channel.value_offset()) != value_cursor
        {
            return None;
        }
        require_scalar(
            table.scalar::<u32>(row, animation_channel_field::KEY_COUNT),
            time_count,
        )?;
        require_scalar(
            table.scalar::<u32>(row, animation_channel_field::VALUE_WIDTH),
            channel.value_width(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, animation_channel_field::TIME_OFFSET),
            channel.time_offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, animation_channel_field::TIME_COUNT),
            time_count,
        )?;
        require_scalar(
            table.sequence_ref::<f64, AnimationTimesSchema>(row, animation_channel_field::TIMES),
            times_ref,
        )?;
        require_float64_range_bits(
            stored_times,
            channel.time_offset(),
            time_count,
            channel.times(),
            observe_work,
        )?;
        time_cursor = time_cursor.checked_add(u64::from(time_count))?;

        require_scalar(
            table.scalar::<u32>(row, animation_channel_field::VALUE_OFFSET),
            channel.value_offset(),
        )?;
        require_scalar(
            table.scalar::<u32>(row, animation_channel_field::VALUE_COUNT),
            value_count,
        )?;
        require_scalar(
            table.sequence_ref::<f32, AnimationValuesSchema>(row, animation_channel_field::VALUES),
            values_ref,
        )?;
        require_float32_range_bits(
            stored_values,
            channel.value_offset(),
            value_count,
            channel.values(),
            observe_work,
        )?;
        value_cursor = value_cursor.checked_add(u64::from(value_count))?;

        validate_optional_tangent::<AnimationInTangentsSchema>(
            table,
            row,
            animation_channel_field::IN_TANGENTS_PRESENCE,
            animation_channel_field::IN_TANGENT_OFFSET,
            animation_channel_field::IN_TANGENT_COUNT,
            animation_channel_field::IN_TANGENTS,
            channel.in_tangent_offset(),
            channel.in_tangent_count(),
            channel.in_tangents(),
            in_tangents,
            stored_in_tangents,
            &mut in_tangent_cursor,
            value_count,
            observe_work,
        )?;
        validate_optional_tangent::<AnimationOutTangentsSchema>(
            table,
            row,
            animation_channel_field::OUT_TANGENTS_PRESENCE,
            animation_channel_field::OUT_TANGENT_OFFSET,
            animation_channel_field::OUT_TANGENT_COUNT,
            animation_channel_field::OUT_TANGENTS,
            channel.out_tangent_offset(),
            channel.out_tangent_count(),
            channel.out_tangents(),
            out_tangents,
            stored_out_tangents,
            &mut out_tangent_cursor,
            value_count,
            observe_work,
        )?;
    }

    (time_cursor == expected.time_scalar_count()
        && value_cursor == expected.value_scalar_count()
        && in_tangent_cursor == expected.in_tangent_scalar_count()
        && out_tangent_cursor == expected.out_tangent_scalar_count())
    .then_some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_target_ref(
    table: CanonicalRecordView<'_>,
    row: usize,
    target_kind: u8,
    target_index: u32,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    node_rows: usize,
    deformation_mesh_instances: Option<CanonicalRecordRef<DeformationMeshInstanceTableSchema>>,
    deformation_mesh_instance_rows: Option<usize>,
) -> Option<()> {
    let target_index = usize::try_from(target_index).ok()?;
    match target_kind {
        ANIMATION_TARGET_NODE_TRANSLATION
        | ANIMATION_TARGET_NODE_ROTATION
        | ANIMATION_TARGET_NODE_SCALE => {
            if target_index == 0 || target_index >= node_rows {
                return None;
            }
            require_scalar(
                table.record_ref::<NodeSceneNodeTableSchema>(row, animation_channel_field::TARGETS),
                nodes_ref,
            )
        }
        ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS => {
            if target_index >= deformation_mesh_instance_rows? {
                return None;
            }
            require_scalar(
                table.record_ref::<DeformationMeshInstanceTableSchema>(
                    row,
                    animation_channel_field::TARGETS,
                ),
                deformation_mesh_instances?,
            )
        }
        _ => None,
    }
}

fn validate_optional_rate(
    table: CanonicalRecordView<'_>,
    row: usize,
    expected: Presence<f64>,
) -> Option<()> {
    match expected {
        Presence::Absent => {
            require_scalar(
                table.scalar::<u8>(row, animation_clip_field::RATE_PRESENCE),
                PRESENCE_ABSENT,
            )?;
            require_scalar(table.scalar::<u8>(row, animation_clip_field::RATE), 0_u8)
        }
        Presence::Present(rate) => {
            require_scalar(
                table.scalar::<u8>(row, animation_clip_field::RATE_PRESENCE),
                PRESENCE_PRESENT,
            )?;
            require_float64_bits(table.scalar::<f64>(row, animation_clip_field::RATE), rate)
        }
    }
}

fn validate_optional_record_presence<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    presence_field: usize,
    reference_field: usize,
    expected_present: bool,
    expected_reference: Option<CanonicalRecordRef<Tag>>,
) -> Option<()>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    if expected_present {
        require_scalar(table.scalar::<u8>(row, presence_field), PRESENCE_PRESENT)?;
        require_scalar(
            table.record_ref::<Tag>(row, reference_field),
            expected_reference?,
        )
    } else {
        require_scalar(table.scalar::<u8>(row, presence_field), PRESENCE_ABSENT)?;
        require_scalar(table.scalar::<u8>(row, reference_field), 0_u8)
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_optional_tangent<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    presence_field: usize,
    offset_field: usize,
    count_field: usize,
    reference_field: usize,
    expected_offset: Presence<u32>,
    expected_count: u32,
    expected_values: Presence<&[f32]>,
    expected_reference: Option<CanonicalSequenceRef<f32, Tag>>,
    stored_values: Option<&[f32]>,
    cursor: &mut u64,
    expected_present_count: u32,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<f32>>,
{
    match (expected_offset, expected_values) {
        (Presence::Absent, Presence::Absent) if expected_count == 0 => {
            require_scalar(table.scalar::<u8>(row, presence_field), PRESENCE_ABSENT)?;
            require_scalar(table.scalar::<u32>(row, offset_field), 0_u32)?;
            require_scalar(table.scalar::<u32>(row, count_field), 0_u32)?;
            require_scalar(table.scalar::<u8>(row, reference_field), 0_u8)
        }
        (Presence::Present(offset), Presence::Present(values))
            if expected_count != 0 && expected_count == expected_present_count =>
        {
            if u64::from(offset) != *cursor || u32::try_from(values.len()).ok()? != expected_count {
                return None;
            }
            require_scalar(table.scalar::<u8>(row, presence_field), PRESENCE_PRESENT)?;
            require_scalar(table.scalar::<u32>(row, offset_field), offset)?;
            require_scalar(table.scalar::<u32>(row, count_field), expected_count)?;
            require_scalar(
                table.sequence_ref::<f32, Tag>(row, reference_field),
                expected_reference?,
            )?;
            require_float32_range_bits(
                stored_values?,
                offset,
                expected_count,
                values,
                observe_work,
            )?;
            *cursor = (*cursor).checked_add(u64::from(expected_count))?;
            Some(())
        }
        _ => None,
    }
}

fn require_float64_range_bits(
    stored: &[f64],
    offset: u32,
    count: u32,
    expected: &[f64],
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let range = checked_range(offset, count, stored.len())?;
    let actual = stored.get(range)?;
    if actual.len() != expected.len() {
        return None;
    }
    for (&actual, &expected) in actual.iter().zip(expected) {
        observe_work(1)?;
        require_scalar(Some(actual.to_bits()), expected.to_bits())?;
    }
    Some(())
}

fn require_float32_range_bits(
    stored: &[f32],
    offset: u32,
    count: u32,
    expected: &[f32],
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let range = checked_range(offset, count, stored.len())?;
    let actual = stored.get(range)?;
    if actual.len() != expected.len() {
        return None;
    }
    for (&actual, &expected) in actual.iter().zip(expected) {
        observe_work(1)?;
        require_scalar(Some(actual.to_bits()), expected.to_bits())?;
    }
    Some(())
}

fn require_float64_bits(actual: Option<f64>, expected: f64) -> Option<()> {
    require_scalar(actual.map(f64::to_bits), expected.to_bits())
}

fn checked_range(offset: u32, count: u32, len: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(offset).ok()?;
    let count = usize::try_from(count).ok()?;
    let end = start.checked_add(count)?;
    (end <= len).then_some(start..end)
}
