use super::super::semantic::{
    NodeSceneAnimationCursor, NodeSceneAnimationExtrapolation, NodeSceneAnimationInterpolation,
    NodeSceneAnimationTarget, NodeSceneAnimationTimeDomain,
};
use super::{
    CanonicalDigestEncoder, NodeSceneSnapshotError, PRESENCE_ABSENT, PRESENCE_PRESENT, PreviewSink,
    SnapshotWorkMeter, TAG_COLLECTION_TOTAL, TAG_PRESENCE, TAG_REFERENCE,
    begin_geometry_collection, write_metered_bytes,
};

const TAG_ANIMATION: u8 = 120;
const TAG_CLIPS: u8 = 121;
const TAG_CLIP: u8 = 122;
const TAG_DURATION_BITS: u8 = 123;
const TAG_TIME_DOMAIN: u8 = 124;
const TAG_RATE: u8 = 125;
const TAG_CHANNEL_OFFSET: u8 = 126;
const TAG_CHANNELS: u8 = 127;
const TAG_CHANNEL: u8 = 128;
const TAG_TARGET_KIND: u8 = 129;
const TAG_TARGET_REFERENCE: u8 = 130;
const TAG_INTERPOLATION: u8 = 131;
const TAG_PRE_EXTRAPOLATION: u8 = 132;
const TAG_POST_EXTRAPOLATION: u8 = 133;
const TAG_KEY_COUNT: u8 = 134;
const TAG_VALUE_WIDTH: u8 = 135;
const TAG_TIMES: u8 = 136;
const TAG_VALUES: u8 = 137;
const TAG_IN_TANGENTS: u8 = 138;
const TAG_OUT_TANGENTS: u8 = 139;
const TAG_F64_BITS: u8 = 140;
const TAG_F32_BITS: u8 = 141;
// Optional, length-delimited UTF-8 field after the fixed clip fields. Absence writes no
// extension; an authored empty name writes this tag and a zero u64 byte length.
// Existing unnamed clips and non-animation transcripts therefore keep their encoding.
const TAG_CLIP_NAME_BYTES: u8 = 223;

#[derive(Clone, Copy)]
pub(super) struct AnimationCollectionCounts {
    pub(super) clips: usize,
    pub(super) channels: usize,
}

#[derive(Clone, Copy, Default)]
pub(super) struct AnimationPayloadTotals {
    pub(super) time_scalars: u64,
    pub(super) value_scalars: u64,
    pub(super) in_tangent_scalars: u64,
    pub(super) out_tangent_scalars: u64,
}

pub(super) fn write_animation(
    cursor: &mut NodeSceneAnimationCursor<'_>,
    counts: AnimationCollectionCounts,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<AnimationPayloadTotals, NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(TAG_ANIMATION)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.push_str("animation(present)\n");
    write_clips(cursor, counts.clips, transcript, preview, work)?;
    let totals = write_channels(cursor, counts.channels, transcript, preview, work)?;
    preview.push_str("animation_end\n");
    Ok(totals)
}

fn write_clips(
    cursor: &mut NodeSceneAnimationCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_geometry_collection(transcript, preview, work, TAG_CLIPS, "clips", count)?;
    for _ in 0..count {
        work.observe()?;
        let clip = cursor
            .next_clip()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "animation-clip collection ended before declared total",
            })?;
        transcript.write_tag(TAG_CLIP)?;
        transcript.tagged_usize(TAG_REFERENCE, clip.source_index())?;
        transcript.tagged_u64(TAG_DURATION_BITS, clip.duration().to_bits())?;
        let (domain, rate) = encode_time_domain(clip.time_domain());
        transcript.tagged_u8(TAG_TIME_DOMAIN, domain)?;
        transcript.write_tag(TAG_RATE)?;
        match rate {
            Some(rate) => {
                transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
                transcript.tagged_u64(TAG_F64_BITS, rate.to_bits())?;
            }
            None => transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?,
        }
        transcript.tagged_usize(TAG_CHANNEL_OFFSET, clip.channel_offset())?;
        transcript.tagged_usize(TAG_COLLECTION_TOTAL, clip.channel_count())?;
        if let Some(name) = clip.name() {
            write_metered_bytes(transcript, work, TAG_CLIP_NAME_BYTES, name.as_bytes())?;
        }
        let render = preview.begin_geometry_item("clip", clip.source_index())?;
        preview.geometry_field_u64(render, "duration_bits", clip.duration().to_bits());
        preview.geometry_field_u32(render, "time_domain", u32::from(domain));
        preview.geometry_field_usize(render, "channel_offset", clip.channel_offset());
        preview.geometry_field_usize(render, "channel_count", clip.channel_count());
        if let Some(name) = clip.name() {
            preview.geometry_field_word(render, "name", name);
        }
        preview.end_geometry_item(render);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(())
}

fn write_channels(
    cursor: &mut NodeSceneAnimationCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<AnimationPayloadTotals, NodeSceneSnapshotError> {
    begin_geometry_collection(transcript, preview, work, TAG_CHANNELS, "channels", count)?;
    let mut totals = AnimationPayloadTotals::default();
    for _ in 0..count {
        work.observe()?;
        let channel = cursor
            .next_channel()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "animation-channel collection ended before declared total",
            })?;
        let (target_kind, target_reference) = encode_target(channel.target());
        let interpolation = encode_interpolation(channel.interpolation());
        let pre = encode_extrapolation(channel.pre_extrapolation());
        let post = encode_extrapolation(channel.post_extrapolation());
        transcript.write_tag(TAG_CHANNEL)?;
        transcript.tagged_usize(TAG_REFERENCE, channel.source_index())?;
        transcript.tagged_u8(TAG_TARGET_KIND, target_kind)?;
        transcript.tagged_usize(TAG_TARGET_REFERENCE, target_reference)?;
        transcript.tagged_u8(TAG_INTERPOLATION, interpolation)?;
        transcript.tagged_u8(TAG_PRE_EXTRAPOLATION, pre)?;
        transcript.tagged_u8(TAG_POST_EXTRAPOLATION, post)?;
        transcript.tagged_usize(TAG_KEY_COUNT, channel.key_count())?;
        transcript.tagged_usize(TAG_VALUE_WIDTH, channel.value_width())?;

        let render = preview.begin_geometry_item("channel", channel.source_index())?;
        preview.geometry_field_u32(render, "target_kind", u32::from(target_kind));
        preview.geometry_field_usize(render, "target", target_reference);
        preview.geometry_field_u32(render, "interpolation", u32::from(interpolation));
        preview.geometry_field_usize(render, "key_count", channel.key_count());
        preview.geometry_field_usize(render, "value_width", channel.value_width());

        write_f64_values(
            transcript,
            preview,
            work,
            TAG_TIMES,
            "times",
            channel.times(),
            render,
        )?;
        write_f32_values(
            transcript,
            preview,
            work,
            TAG_VALUES,
            "values",
            Some(channel.values()),
            render,
        )?;
        write_f32_values(
            transcript,
            preview,
            work,
            TAG_IN_TANGENTS,
            "in_tangents",
            channel.in_tangents(),
            render,
        )?;
        write_f32_values(
            transcript,
            preview,
            work,
            TAG_OUT_TANGENTS,
            "out_tangents",
            channel.out_tangents(),
            render,
        )?;
        preview.end_geometry_item(render);

        totals.time_scalars = add_scalars(
            totals.time_scalars,
            channel.times().len(),
            "animation times",
        )?;
        totals.value_scalars = add_scalars(
            totals.value_scalars,
            channel.values().len(),
            "animation values",
        )?;
        totals.in_tangent_scalars = add_scalars(
            totals.in_tangent_scalars,
            channel.in_tangents().map_or(0, <[f32]>::len),
            "animation in tangents",
        )?;
        totals.out_tangent_scalars = add_scalars(
            totals.out_tangent_scalars,
            channel.out_tangents().map_or(0, <[f32]>::len),
            "animation out tangents",
        )?;
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, count)?;
    Ok(totals)
}

fn write_f64_values(
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    tag: u8,
    name: &'static str,
    values: &[f64],
    render: bool,
) -> Result<(), NodeSceneSnapshotError> {
    transcript.write_tag(tag)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.begin_geometry_leaf(render, name, true, values.len());
    for (ordinal, value) in values.iter().copied().enumerate() {
        work.observe()?;
        let bits = value.to_bits();
        transcript.tagged_u64(TAG_F64_BITS, bits)?;
        preview.geometry_field_u64(render && ordinal == 0, "first_bits", bits);
    }
    transcript.tagged_usize(TAG_COLLECTION_TOTAL, values.len())?;
    preview.end_geometry_leaf(render);
    Ok(())
}

fn write_f32_values(
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    tag: u8,
    name: &'static str,
    values: Option<&[f32]>,
    render: bool,
) -> Result<(), NodeSceneSnapshotError> {
    transcript.write_tag(tag)?;
    match values {
        Some(values) => {
            transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
            preview.begin_geometry_leaf(render, name, true, values.len());
            for (ordinal, value) in values.iter().copied().enumerate() {
                work.observe()?;
                let bits = value.to_bits();
                transcript.tagged_u32(TAG_F32_BITS, bits)?;
                preview.geometry_f32_bits(render, ordinal, bits)?;
            }
            transcript.tagged_usize(TAG_COLLECTION_TOTAL, values.len())?;
            preview.end_geometry_leaf(render);
        }
        None => {
            transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?;
            transcript.tagged_usize(TAG_COLLECTION_TOTAL, 0)?;
            preview.begin_geometry_leaf(render, name, false, 0);
            preview.end_geometry_leaf(render);
        }
    }
    Ok(())
}

fn add_scalars(
    total: u64,
    count: usize,
    name: &'static str,
) -> Result<u64, NodeSceneSnapshotError> {
    total
        .checked_add(
            u64::try_from(count)
                .map_err(|_| NodeSceneSnapshotError::SemanticTotalOverflow { total: name })?,
        )
        .ok_or(NodeSceneSnapshotError::SemanticTotalOverflow { total: name })
}

const fn encode_time_domain(value: NodeSceneAnimationTimeDomain) -> (u8, Option<f64>) {
    match value {
        NodeSceneAnimationTimeDomain::Seconds => (0, None),
        NodeSceneAnimationTimeDomain::Frames { rate } => (1, Some(rate)),
        NodeSceneAnimationTimeDomain::Ticks { rate } => (2, rate),
    }
}

const fn encode_target(value: NodeSceneAnimationTarget) -> (u8, usize) {
    match value {
        NodeSceneAnimationTarget::NodeTranslation(index) => (0, index),
        NodeSceneAnimationTarget::NodeRotation(index) => (1, index),
        NodeSceneAnimationTarget::NodeScale(index) => (2, index),
        NodeSceneAnimationTarget::MeshInstanceMorphWeights(index) => (3, index),
    }
}

const fn encode_interpolation(value: NodeSceneAnimationInterpolation) -> u8 {
    match value {
        NodeSceneAnimationInterpolation::Step => 0,
        NodeSceneAnimationInterpolation::Linear => 1,
        NodeSceneAnimationInterpolation::CubicSpline => 2,
    }
}

const fn encode_extrapolation(value: NodeSceneAnimationExtrapolation) -> u8 {
    match value {
        NodeSceneAnimationExtrapolation::SourceDefault => 0,
        NodeSceneAnimationExtrapolation::Constant => 1,
        NodeSceneAnimationExtrapolation::Linear => 2,
        NodeSceneAnimationExtrapolation::Repeat => 3,
    }
}
