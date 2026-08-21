use baozi_operation::{
    CanonicalRecordView, PublishedCanonicalStore, RecordBlock, ScalarSequence, SequenceView,
    StoredScalar,
};

use crate::canonical_scene::store::schema::animation::{
    ANIMATION_CHANNEL_FIELD_COUNT, ANIMATION_CLIP_FIELD_COUNT, ANIMATION_EXTRAPOLATION_CONSTANT,
    ANIMATION_EXTRAPOLATION_LINEAR, ANIMATION_EXTRAPOLATION_REPEAT,
    ANIMATION_EXTRAPOLATION_SOURCE_DEFAULT, ANIMATION_INTERPOLATION_CUBIC_SPLINE,
    ANIMATION_INTERPOLATION_LINEAR, ANIMATION_INTERPOLATION_STEP,
    ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS, ANIMATION_TARGET_NODE_ROTATION,
    ANIMATION_TARGET_NODE_SCALE, ANIMATION_TARGET_NODE_TRANSLATION, ANIMATION_TIME_DOMAIN_FRAMES,
    ANIMATION_TIME_DOMAIN_SECONDS, ANIMATION_TIME_DOMAIN_TICKS, AnimationInTangentsSchema,
    AnimationOutTangentsSchema, AnimationTimesSchema, AnimationValuesSchema,
    animation_channel_field, animation_clip_field,
};
use crate::canonical_scene::store::schema::deformation::DEFORMATION_MESH_INSTANCE_FIELD_COUNT;
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, PRESENCE_ABSENT, PRESENCE_PRESENT, SceneDirectorySchema,
};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;

use super::{NodeSceneSemanticResult, invariant, read_scalar, read_u32_as_usize};

type SceneCanonicalStore = PublishedCanonicalStore<RecordBlock, SceneDirectorySchema>;

/// Proof-bound projections of the animation tables and shared scalar payloads.
pub(super) struct NodeSceneAnimationTables<'scene> {
    clips: Option<CanonicalRecordView<'scene>>,
    channels: Option<CanonicalRecordView<'scene>>,
    deformation_mesh_instances: Option<CanonicalRecordView<'scene>>,
    times: Option<&'scene [f64]>,
    values: Option<&'scene [f32]>,
    in_tangents: Option<&'scene [f32]>,
    out_tangents: Option<&'scene [f32]>,
}

impl<'scene> NodeSceneAnimationTables<'scene> {
    pub(super) fn try_new(
        canonical: &'scene SceneCanonicalStore,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        let clips = resolve_table(
            canonical,
            proof.animation_clips(),
            proof.animation_clip_count(),
            ANIMATION_CLIP_FIELD_COUNT,
            "validated animation-clip table is unavailable or mistyped",
            "validated animation-clip table presence disagrees with validation proof",
            "validated animation-clip table layout disagrees with validation proof",
        )?;
        let channels = resolve_table(
            canonical,
            proof.animation_channels(),
            proof.animation_channel_count(),
            ANIMATION_CHANNEL_FIELD_COUNT,
            "validated animation-channel table is unavailable or mistyped",
            "validated animation-channel table presence disagrees with validation proof",
            "validated animation-channel table layout disagrees with validation proof",
        )?;
        let deformation_mesh_instances = resolve_optional_dependency_table(
            canonical,
            proof.deformation_mesh_instances(),
            DEFORMATION_MESH_INSTANCE_FIELD_COUNT,
            "validated deformation mesh-instance target table is unavailable or mistyped",
            "validated deformation mesh-instance target table has an invalid layout",
        )?;
        let times = resolve_sequence(
            canonical,
            proof.animation_times(),
            "validated animation-time payload is unavailable or mistyped",
        )?;
        let values = resolve_sequence(
            canonical,
            proof.animation_values(),
            "validated animation-value payload is unavailable or mistyped",
        )?;
        let in_tangents = resolve_sequence(
            canonical,
            proof.animation_in_tangents(),
            "validated animation in-tangent payload is unavailable or mistyped",
        )?;
        let out_tangents = resolve_sequence(
            canonical,
            proof.animation_out_tangents(),
            "validated animation out-tangent payload is unavailable or mistyped",
        )?;

        if proof.animation_channel_count() == 0 {
            if times.is_some()
                || values.is_some()
                || in_tangents.is_some()
                || out_tangents.is_some()
            {
                return Err(invariant(
                    "validated empty animation domain retains scalar payloads",
                ));
            }
        } else if times.is_none() || values.is_none() {
            return Err(invariant(
                "validated populated animation domain is missing required scalar payloads",
            ));
        }
        if payload_len(in_tangents) != payload_len(out_tangents) {
            return Err(invariant(
                "validated animation tangent payload lengths disagree",
            ));
        }

        Ok(Self {
            clips,
            channels,
            deformation_mesh_instances,
            times,
            values,
            in_tangents,
            out_tangents,
        })
    }

    pub(super) const fn into_cursor(
        self,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneAnimationCursor<'scene> {
        NodeSceneAnimationCursor {
            tables: self,
            proof,
            phase: AnimationCursorPhase::Clips,
            next_clip: 0,
            next_channel: 0,
            next_clip_channel_offset: 0,
            next_time_offset: 0,
            next_value_offset: 0,
            next_in_tangent_offset: 0,
            next_out_tangent_offset: 0,
        }
    }
}

fn resolve_table<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<Tag>>,
    expected_rows: usize,
    expected_fields: usize,
    unavailable_reason: &'static str,
    presence_reason: &'static str,
    layout_reason: &'static str,
) -> NodeSceneSemanticResult<Option<CanonicalRecordView<'scene>>>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    match (expected_rows, reference) {
        (0, None) => Ok(None),
        (0, Some(_)) | (_, None) => Err(invariant(presence_reason)),
        (_, Some(reference)) => {
            let table = canonical
                .records(reference.into_store())
                .ok_or_else(|| invariant(unavailable_reason))?;
            if table.rows() != expected_rows || table.fields() != expected_fields {
                return Err(invariant(layout_reason));
            }
            Ok(Some(table))
        }
    }
}

fn resolve_optional_dependency_table<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<Tag>>,
    expected_fields: usize,
    unavailable_reason: &'static str,
    layout_reason: &'static str,
) -> NodeSceneSemanticResult<Option<CanonicalRecordView<'scene>>>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    reference
        .map(|reference| {
            let table = canonical
                .records(reference.into_store())
                .ok_or_else(|| invariant(unavailable_reason))?;
            if table.rows() == 0 || table.fields() != expected_fields {
                return Err(invariant(layout_reason));
            }
            Ok(table)
        })
        .transpose()
}

fn resolve_sequence<'scene, T, Tag>(
    canonical: &'scene SceneCanonicalStore,
    reference: Option<CanonicalBlockRef<Tag>>,
    unavailable_reason: &'static str,
) -> NodeSceneSemanticResult<Option<&'scene [T]>>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    reference
        .map(|reference| {
            let sequence: SequenceView<'scene, T> = canonical
                .sequence(reference.into_store())
                .ok_or_else(|| invariant(unavailable_reason))?;
            if sequence.as_slice().is_empty() {
                return Err(invariant(
                    "validated present animation payload has no scalar values",
                ));
            }
            Ok(sequence.as_slice())
        })
        .transpose()
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AnimationCursorPhase {
    Clips,
    Channels,
}

/// Linear allocation-free traversal of every proof-qualified animation row.
#[must_use = "the animation cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneAnimationCursor<'scene> {
    tables: NodeSceneAnimationTables<'scene>,
    proof: NodeSceneValidationProof,
    phase: AnimationCursorPhase,
    next_clip: usize,
    next_channel: usize,
    next_clip_channel_offset: usize,
    next_time_offset: usize,
    next_value_offset: usize,
    next_in_tangent_offset: usize,
    next_out_tangent_offset: usize,
}

impl<'scene> NodeSceneAnimationCursor<'scene> {
    pub(in crate::canonical_scene::store) fn clip_count(&self) -> usize {
        self.proof.animation_clip_count()
    }

    pub(in crate::canonical_scene::store) fn channel_count(&self) -> usize {
        self.proof.animation_channel_count()
    }

    pub(in crate::canonical_scene::store) fn next_clip(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAnimationClip>> {
        self.enter_phase(AnimationCursorPhase::Clips)?;
        let Some(row) = next_row(
            self.next_clip,
            self.proof.animation_clip_count(),
            "animation-clip cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .clips
            .ok_or_else(|| invariant("validated animation-clip table view is absent"))?;
        let source_index = read_u32_as_usize(
            table,
            row,
            animation_clip_field::CLIP,
            "validated animation-clip source identity is unreadable",
        )?;

        let duration = read_scalar::<f64>(
            table,
            row,
            animation_clip_field::DURATION,
            "validated animation-clip duration is unreadable",
        )?;
        let time_domain_code = read_scalar::<u8>(
            table,
            row,
            animation_clip_field::TIME_DOMAIN,
            "validated animation-clip time domain is unreadable",
        )?;
        let rate = read_optional_rate(table, row)?;
        let time_domain = decode_time_domain(time_domain_code, rate)?;

        let channel_offset = read_u32_as_usize(
            table,
            row,
            animation_clip_field::CHANNEL_OFFSET,
            "validated animation-clip channel offset is unreadable",
        )?;
        let channel_count = read_u32_as_usize(
            table,
            row,
            animation_clip_field::CHANNEL_COUNT,
            "validated animation-clip channel count is unreadable",
        )?;
        if channel_offset != self.next_clip_channel_offset {
            return Err(invariant(
                "validated animation-clip channel ranges are not contiguous",
            ));
        }
        require_range(
            channel_offset,
            channel_count,
            self.proof.animation_channel_count(),
            "validated animation-clip channel range is outside channel space",
        )?;
        require_optional_record_reference(
            table,
            row,
            animation_clip_field::CHANNELS_PRESENCE,
            animation_clip_field::CHANNELS,
            channel_count != 0,
            self.proof.animation_channels(),
            "validated animation-clip channel presence is unreadable",
            "validated empty animation-clip channel range has a nonzero reference sentinel",
            "validated animation-clip channel-table reference is unreadable",
            "validated animation-clip channel-table reference disagrees with validation proof",
        )?;

        self.next_clip_channel_offset = channel_offset
            .checked_add(channel_count)
            .ok_or_else(|| invariant("animation-clip channel cursor overflows"))?;
        self.next_clip = advance_row(row, "animation-clip cursor index overflows")?;
        Ok(Some(NodeSceneAnimationClip {
            source_index,
            duration,
            time_domain,
            channel_offset,
            channel_count,
        }))
    }

    pub(in crate::canonical_scene::store) fn next_channel(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneAnimationChannel<'scene>>> {
        self.enter_phase(AnimationCursorPhase::Channels)?;
        let Some(row) = next_row(
            self.next_channel,
            self.proof.animation_channel_count(),
            "animation-channel cursor advanced beyond its proof-qualified count",
        )?
        else {
            return Ok(None);
        };
        let table = self
            .tables
            .channels
            .ok_or_else(|| invariant("validated animation-channel table view is absent"))?;
        let source_index = read_u32_as_usize(
            table,
            row,
            animation_channel_field::CHANNEL,
            "validated animation-channel source identity is unreadable",
        )?;

        let target = self.read_target(table, row)?;
        let interpolation = decode_interpolation(read_scalar::<u8>(
            table,
            row,
            animation_channel_field::INTERPOLATION,
            "validated animation-channel interpolation is unreadable",
        )?)?;
        let pre_extrapolation = decode_extrapolation(read_scalar::<u8>(
            table,
            row,
            animation_channel_field::PRE_EXTRAPOLATION,
            "validated animation-channel pre-extrapolation is unreadable",
        )?)?;
        let post_extrapolation = decode_extrapolation(read_scalar::<u8>(
            table,
            row,
            animation_channel_field::POST_EXTRAPOLATION,
            "validated animation-channel post-extrapolation is unreadable",
        )?)?;
        let key_count = read_u32_as_usize(
            table,
            row,
            animation_channel_field::KEY_COUNT,
            "validated animation-channel key count is unreadable",
        )?;
        let value_width = read_u32_as_usize(
            table,
            row,
            animation_channel_field::VALUE_WIDTH,
            "validated animation-channel value width is unreadable",
        )?;
        if key_count == 0 || value_width == 0 {
            return Err(invariant(
                "validated animation channel has an empty key or value dimension",
            ));
        }
        let expected_value_count = key_count
            .checked_mul(value_width)
            .ok_or_else(|| invariant("animation-channel value count overflows"))?;

        let times = read_required_payload::<f64, AnimationTimesSchema>(
            table,
            row,
            animation_channel_field::TIME_OFFSET,
            animation_channel_field::TIME_COUNT,
            animation_channel_field::TIMES,
            self.next_time_offset,
            key_count,
            self.tables.times,
            self.proof.animation_times(),
            "validated animation-time range is inconsistent",
            "validated animation-time payload reference is unreadable",
            "validated animation-time payload reference disagrees with validation proof",
        )?;
        let values = read_required_payload::<f32, AnimationValuesSchema>(
            table,
            row,
            animation_channel_field::VALUE_OFFSET,
            animation_channel_field::VALUE_COUNT,
            animation_channel_field::VALUES,
            self.next_value_offset,
            expected_value_count,
            self.tables.values,
            self.proof.animation_values(),
            "validated animation-value range is inconsistent",
            "validated animation-value payload reference is unreadable",
            "validated animation-value payload reference disagrees with validation proof",
        )?;
        let in_tangents = read_optional_tangent::<AnimationInTangentsSchema>(
            table,
            row,
            animation_channel_field::IN_TANGENTS_PRESENCE,
            animation_channel_field::IN_TANGENT_OFFSET,
            animation_channel_field::IN_TANGENT_COUNT,
            animation_channel_field::IN_TANGENTS,
            self.next_in_tangent_offset,
            expected_value_count,
            self.tables.in_tangents,
            self.proof.animation_in_tangents(),
            "validated animation in-tangent payload is inconsistent",
        )?;
        let out_tangents = read_optional_tangent::<AnimationOutTangentsSchema>(
            table,
            row,
            animation_channel_field::OUT_TANGENTS_PRESENCE,
            animation_channel_field::OUT_TANGENT_OFFSET,
            animation_channel_field::OUT_TANGENT_COUNT,
            animation_channel_field::OUT_TANGENTS,
            self.next_out_tangent_offset,
            expected_value_count,
            self.tables.out_tangents,
            self.proof.animation_out_tangents(),
            "validated animation out-tangent payload is inconsistent",
        )?;

        self.next_time_offset = self
            .next_time_offset
            .checked_add(key_count)
            .ok_or_else(|| invariant("animation-time cursor overflows"))?;
        self.next_value_offset = self
            .next_value_offset
            .checked_add(expected_value_count)
            .ok_or_else(|| invariant("animation-value cursor overflows"))?;
        if in_tangents.is_some() {
            self.next_in_tangent_offset = self
                .next_in_tangent_offset
                .checked_add(expected_value_count)
                .ok_or_else(|| invariant("animation in-tangent cursor overflows"))?;
        }
        if out_tangents.is_some() {
            self.next_out_tangent_offset = self
                .next_out_tangent_offset
                .checked_add(expected_value_count)
                .ok_or_else(|| invariant("animation out-tangent cursor overflows"))?;
        }
        self.next_channel = advance_row(row, "animation-channel cursor index overflows")?;
        Ok(Some(NodeSceneAnimationChannel {
            source_index,
            target,
            interpolation,
            pre_extrapolation,
            post_extrapolation,
            key_count,
            value_width,
            times,
            values,
            in_tangents,
            out_tangents,
        }))
    }

    pub(in crate::canonical_scene::store) fn finish(self) -> NodeSceneSemanticResult<()> {
        if self.next_clip != self.proof.animation_clip_count()
            || self.next_channel != self.proof.animation_channel_count()
            || self.next_clip_channel_offset != self.proof.animation_channel_count()
            || self.next_time_offset != payload_len(self.tables.times)
            || self.next_value_offset != payload_len(self.tables.values)
            || self.next_in_tangent_offset != payload_len(self.tables.in_tangents)
            || self.next_out_tangent_offset != payload_len(self.tables.out_tangents)
        {
            return Err(invariant(
                "animation cursor ended before all proof-qualified tables and payloads were exhausted",
            ));
        }
        Ok(())
    }

    fn enter_phase(&mut self, target: AnimationCursorPhase) -> NodeSceneSemanticResult<()> {
        if target < self.phase {
            return Err(invariant(
                "animation cursor cannot return to an earlier table phase",
            ));
        }
        if target >= AnimationCursorPhase::Channels {
            if self.next_clip != self.proof.animation_clip_count() {
                return Err(invariant(
                    "animation-clip table must be exhausted before channels",
                ));
            }
            if self.next_clip_channel_offset != self.proof.animation_channel_count() {
                return Err(invariant(
                    "animation clips do not partition the proof-qualified channel table",
                ));
            }
        }
        self.phase = target;
        Ok(())
    }

    fn read_target(
        &self,
        table: CanonicalRecordView<'_>,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneAnimationTarget> {
        let kind = read_scalar::<u8>(
            table,
            row,
            animation_channel_field::TARGET_KIND,
            "validated animation-channel target kind is unreadable",
        )?;
        let index = read_u32_as_usize(
            table,
            row,
            animation_channel_field::TARGET_INDEX,
            "validated animation-channel target index is unreadable",
        )?;
        match kind {
            ANIMATION_TARGET_NODE_TRANSLATION => {
                require_node_target(table, row, index, self.proof)?;
                Ok(NodeSceneAnimationTarget::NodeTranslation(index))
            }
            ANIMATION_TARGET_NODE_ROTATION => {
                require_node_target(table, row, index, self.proof)?;
                Ok(NodeSceneAnimationTarget::NodeRotation(index))
            }
            ANIMATION_TARGET_NODE_SCALE => {
                require_node_target(table, row, index, self.proof)?;
                Ok(NodeSceneAnimationTarget::NodeScale(index))
            }
            ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS => {
                let mesh_instances = self.tables.deformation_mesh_instances.ok_or_else(|| {
                    invariant(
                        "validated morph-weight target is missing its deformation mesh-instance table",
                    )
                })?;
                if index >= mesh_instances.rows() {
                    return Err(invariant(
                        "validated morph-weight target is outside deformation mesh-instance space",
                    ));
                }
                let reference = self.proof.deformation_mesh_instances().ok_or_else(|| {
                    invariant(
                        "validated morph-weight target has no deformation mesh-instance proof",
                    )
                })?;
                require_exact_record_reference(
                    table,
                    row,
                    animation_channel_field::TARGETS,
                    reference,
                    "validated morph-weight target reference is unreadable",
                    "validated morph-weight target reference disagrees with validation proof",
                )?;
                Ok(NodeSceneAnimationTarget::MeshInstanceMorphWeights(index))
            }
            _ => Err(invariant(
                "validated animation-channel target kind is unknown",
            )),
        }
    }
}

fn require_node_target(
    table: CanonicalRecordView<'_>,
    row: usize,
    index: usize,
    proof: NodeSceneValidationProof,
) -> NodeSceneSemanticResult<()> {
    if index == 0 || index > proof.source_node_count() {
        return Err(invariant(
            "validated animation-channel node target is outside canonical node space",
        ));
    }
    require_exact_record_reference(
        table,
        row,
        animation_channel_field::TARGETS,
        proof.nodes(),
        "validated animation-channel node target reference is unreadable",
        "validated animation-channel node target reference disagrees with validation proof",
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneAnimationClip {
    source_index: usize,
    duration: f64,
    time_domain: NodeSceneAnimationTimeDomain,
    channel_offset: usize,
    channel_count: usize,
}

impl NodeSceneAnimationClip {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn duration(self) -> f64 {
        self.duration
    }

    pub(in crate::canonical_scene::store) const fn time_domain(
        self,
    ) -> NodeSceneAnimationTimeDomain {
        self.time_domain
    }

    pub(in crate::canonical_scene::store) const fn channel_offset(self) -> usize {
        self.channel_offset
    }

    pub(in crate::canonical_scene::store) const fn channel_count(self) -> usize {
        self.channel_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) enum NodeSceneAnimationTimeDomain {
    Seconds,
    Frames { rate: f64 },
    Ticks { rate: Option<f64> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneAnimationTarget {
    NodeTranslation(usize),
    NodeRotation(usize),
    NodeScale(usize),
    MeshInstanceMorphWeights(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneAnimationInterpolation {
    Step,
    Linear,
    CubicSpline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneAnimationExtrapolation {
    SourceDefault,
    Constant,
    Linear,
    Repeat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneAnimationChannel<'scene> {
    source_index: usize,
    target: NodeSceneAnimationTarget,
    interpolation: NodeSceneAnimationInterpolation,
    pre_extrapolation: NodeSceneAnimationExtrapolation,
    post_extrapolation: NodeSceneAnimationExtrapolation,
    key_count: usize,
    value_width: usize,
    times: &'scene [f64],
    values: &'scene [f32],
    in_tangents: Option<&'scene [f32]>,
    out_tangents: Option<&'scene [f32]>,
}

impl<'scene> NodeSceneAnimationChannel<'scene> {
    pub(in crate::canonical_scene::store) const fn source_index(self) -> usize {
        self.source_index
    }

    pub(in crate::canonical_scene::store) const fn target(self) -> NodeSceneAnimationTarget {
        self.target
    }

    pub(in crate::canonical_scene::store) const fn interpolation(
        self,
    ) -> NodeSceneAnimationInterpolation {
        self.interpolation
    }

    pub(in crate::canonical_scene::store) const fn pre_extrapolation(
        self,
    ) -> NodeSceneAnimationExtrapolation {
        self.pre_extrapolation
    }

    pub(in crate::canonical_scene::store) const fn post_extrapolation(
        self,
    ) -> NodeSceneAnimationExtrapolation {
        self.post_extrapolation
    }

    pub(in crate::canonical_scene::store) const fn key_count(self) -> usize {
        self.key_count
    }

    pub(in crate::canonical_scene::store) const fn value_width(self) -> usize {
        self.value_width
    }

    pub(in crate::canonical_scene::store) const fn times(self) -> &'scene [f64] {
        self.times
    }

    pub(in crate::canonical_scene::store) const fn values(self) -> &'scene [f32] {
        self.values
    }

    pub(in crate::canonical_scene::store) const fn in_tangents(self) -> Option<&'scene [f32]> {
        self.in_tangents
    }

    pub(in crate::canonical_scene::store) const fn out_tangents(self) -> Option<&'scene [f32]> {
        self.out_tangents
    }
}

fn read_optional_rate(
    table: CanonicalRecordView<'_>,
    row: usize,
) -> NodeSceneSemanticResult<Option<f64>> {
    match read_scalar::<u8>(
        table,
        row,
        animation_clip_field::RATE_PRESENCE,
        "validated animation-clip rate presence is unreadable",
    )? {
        PRESENCE_ABSENT => {
            require_absent_cell(
                table,
                row,
                animation_clip_field::RATE,
                "validated absent animation-clip rate has a nonzero sentinel",
            )?;
            Ok(None)
        }
        PRESENCE_PRESENT => read_scalar::<f64>(
            table,
            row,
            animation_clip_field::RATE,
            "validated animation-clip rate is unreadable",
        )
        .map(Some),
        _ => Err(invariant(
            "validated animation-clip rate presence code is unknown",
        )),
    }
}

fn decode_time_domain(
    value: u8,
    rate: Option<f64>,
) -> NodeSceneSemanticResult<NodeSceneAnimationTimeDomain> {
    match (value, rate) {
        (ANIMATION_TIME_DOMAIN_SECONDS, None) => Ok(NodeSceneAnimationTimeDomain::Seconds),
        (ANIMATION_TIME_DOMAIN_FRAMES, Some(rate)) => {
            Ok(NodeSceneAnimationTimeDomain::Frames { rate })
        }
        (ANIMATION_TIME_DOMAIN_TICKS, rate) => Ok(NodeSceneAnimationTimeDomain::Ticks { rate }),
        (ANIMATION_TIME_DOMAIN_SECONDS, Some(_)) | (ANIMATION_TIME_DOMAIN_FRAMES, None) => Err(
            invariant("validated animation-clip rate disagrees with its time domain"),
        ),
        _ => Err(invariant(
            "validated animation-clip time-domain code is unknown",
        )),
    }
}

fn decode_interpolation(value: u8) -> NodeSceneSemanticResult<NodeSceneAnimationInterpolation> {
    match value {
        ANIMATION_INTERPOLATION_STEP => Ok(NodeSceneAnimationInterpolation::Step),
        ANIMATION_INTERPOLATION_LINEAR => Ok(NodeSceneAnimationInterpolation::Linear),
        ANIMATION_INTERPOLATION_CUBIC_SPLINE => Ok(NodeSceneAnimationInterpolation::CubicSpline),
        _ => Err(invariant(
            "validated animation-channel interpolation code is unknown",
        )),
    }
}

fn decode_extrapolation(value: u8) -> NodeSceneSemanticResult<NodeSceneAnimationExtrapolation> {
    match value {
        ANIMATION_EXTRAPOLATION_SOURCE_DEFAULT => {
            Ok(NodeSceneAnimationExtrapolation::SourceDefault)
        }
        ANIMATION_EXTRAPOLATION_CONSTANT => Ok(NodeSceneAnimationExtrapolation::Constant),
        ANIMATION_EXTRAPOLATION_LINEAR => Ok(NodeSceneAnimationExtrapolation::Linear),
        ANIMATION_EXTRAPOLATION_REPEAT => Ok(NodeSceneAnimationExtrapolation::Repeat),
        _ => Err(invariant(
            "validated animation-channel extrapolation code is unknown",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn read_required_payload<'scene, T, Tag>(
    table: CanonicalRecordView<'scene>,
    row: usize,
    offset_field: usize,
    count_field: usize,
    reference_field: usize,
    expected_offset: usize,
    expected_count: usize,
    payload: Option<&'scene [T]>,
    expected_reference: Option<CanonicalBlockRef<Tag>>,
    range_reason: &'static str,
    unreadable_reference_reason: &'static str,
    mismatch_reference_reason: &'static str,
) -> NodeSceneSemanticResult<&'scene [T]>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    let offset = read_u32_as_usize(table, row, offset_field, range_reason)?;
    let count = read_u32_as_usize(table, row, count_field, range_reason)?;
    if offset != expected_offset || count != expected_count {
        return Err(invariant(range_reason));
    }
    let expected_reference = expected_reference
        .ok_or_else(|| invariant("validated required animation payload proof is absent"))?;
    require_exact_sequence_reference::<T, Tag>(
        table,
        row,
        reference_field,
        expected_reference,
        unreadable_reference_reason,
        mismatch_reference_reason,
    )?;
    slice_range(
        payload.ok_or_else(|| invariant("validated required animation payload is absent"))?,
        offset,
        count,
        range_reason,
    )
}

#[allow(clippy::too_many_arguments)]
fn read_optional_tangent<'scene, Tag>(
    table: CanonicalRecordView<'scene>,
    row: usize,
    presence_field: usize,
    offset_field: usize,
    count_field: usize,
    reference_field: usize,
    expected_offset: usize,
    expected_count: usize,
    payload: Option<&'scene [f32]>,
    expected_reference: Option<CanonicalBlockRef<Tag>>,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<&'scene [f32]>>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<f32>>,
{
    match read_scalar::<u8>(table, row, presence_field, reason)? {
        PRESENCE_ABSENT => {
            if read_scalar::<u32>(table, row, offset_field, reason)? != 0
                || read_scalar::<u32>(table, row, count_field, reason)? != 0
            {
                return Err(invariant(reason));
            }
            require_absent_cell(table, row, reference_field, reason)?;
            Ok(None)
        }
        PRESENCE_PRESENT => {
            let offset = read_u32_as_usize(table, row, offset_field, reason)?;
            let count = read_u32_as_usize(table, row, count_field, reason)?;
            if offset != expected_offset || count != expected_count || count == 0 {
                return Err(invariant(reason));
            }
            let expected_reference = expected_reference
                .ok_or_else(|| invariant("validated animation tangent payload proof is absent"))?;
            require_exact_sequence_reference::<f32, Tag>(
                table,
                row,
                reference_field,
                expected_reference,
                reason,
                reason,
            )?;
            slice_range(
                payload
                    .ok_or_else(|| invariant("validated animation tangent payload is absent"))?,
                offset,
                count,
                reason,
            )
            .map(Some)
        }
        _ => Err(invariant(
            "validated animation tangent presence code is unknown",
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn require_optional_record_reference<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    presence_field: usize,
    reference_field: usize,
    expected_present: bool,
    expected_reference: Option<CanonicalBlockRef<Tag>>,
    presence_reason: &'static str,
    absent_reason: &'static str,
    unreadable_reason: &'static str,
    mismatch_reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    let presence = read_scalar::<u8>(table, row, presence_field, presence_reason)?;
    if expected_present {
        if presence != PRESENCE_PRESENT {
            return Err(invariant(presence_reason));
        }
        let reference = expected_reference
            .ok_or_else(|| invariant("validated required record-table proof is absent"))?;
        require_exact_record_reference(
            table,
            row,
            reference_field,
            reference,
            unreadable_reason,
            mismatch_reason,
        )
    } else {
        if presence != PRESENCE_ABSENT {
            return Err(invariant(presence_reason));
        }
        require_absent_cell(table, row, reference_field, absent_reason)
    }
}

fn require_exact_record_reference<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    expected: CanonicalBlockRef<Tag>,
    unreadable_reason: &'static str,
    mismatch_reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    let reference = table
        .record_ref::<Tag>(row, field)
        .ok_or_else(|| invariant(unreadable_reason))?;
    if CanonicalBlockRef::from_store(reference) != expected {
        return Err(invariant(mismatch_reason));
    }
    Ok(())
}

fn require_exact_sequence_reference<T, Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    expected: CanonicalBlockRef<Tag>,
    unreadable_reason: &'static str,
    mismatch_reason: &'static str,
) -> NodeSceneSemanticResult<()>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    let reference = table
        .sequence_ref::<T, Tag>(row, field)
        .ok_or_else(|| invariant(unreadable_reason))?;
    if CanonicalBlockRef::from_store(reference) != expected {
        return Err(invariant(mismatch_reason));
    }
    Ok(())
}

fn require_absent_cell(
    table: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    if table.scalar::<u8>(row, field) != Some(0) {
        return Err(invariant(reason));
    }
    Ok(())
}

fn require_range(
    offset: usize,
    count: usize,
    limit: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    if offset.checked_add(count).is_none_or(|end| end > limit) {
        return Err(invariant(reason));
    }
    Ok(())
}

fn slice_range<'scene, T>(
    values: &'scene [T],
    offset: usize,
    count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene [T]> {
    let end = offset.checked_add(count).ok_or_else(|| invariant(reason))?;
    values.get(offset..end).ok_or_else(|| invariant(reason))
}

fn payload_len<T>(payload: Option<&[T]>) -> usize {
    payload.map_or(0, <[T]>::len)
}

fn next_row(
    next: usize,
    expected: usize,
    overflow_reason: &'static str,
) -> NodeSceneSemanticResult<Option<usize>> {
    if next == expected {
        return Ok(None);
    }
    if next > expected {
        return Err(invariant(overflow_reason));
    }
    Ok(Some(next))
}

fn advance_row(row: usize, reason: &'static str) -> NodeSceneSemanticResult<usize> {
    row.checked_add(1).ok_or_else(|| invariant(reason))
}
