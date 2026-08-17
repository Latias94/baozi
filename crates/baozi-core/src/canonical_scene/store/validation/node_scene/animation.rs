use baozi_operation::__canonical_activation::{
    CanonicalValidationMember, CanonicalValidationSession,
};
use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, CanonicalSequenceRef, OperationOwner, ProgressPhase,
    ProgressUnit, RecordBlock, ScalarSequence, StoreValidationSession, StoreWorkError,
    StoredScalar,
};

use crate::canonical_scene::store::production::{Presence, ValidatedAnimationBatch};
use crate::canonical_scene::store::schema::animation::{
    ANIMATION_CHANNEL_FIELD_COUNT, ANIMATION_CLIP_FIELD_COUNT, ANIMATION_DIRECTORY_FIELD_COUNT,
    ANIMATION_EXTRAPOLATION_CONSTANT, ANIMATION_EXTRAPOLATION_LINEAR,
    ANIMATION_EXTRAPOLATION_REPEAT, ANIMATION_EXTRAPOLATION_SOURCE_DEFAULT,
    ANIMATION_INTERPOLATION_CUBIC_SPLINE, ANIMATION_INTERPOLATION_LINEAR,
    ANIMATION_INTERPOLATION_STEP, ANIMATION_SCHEMA_REVISION,
    ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS, ANIMATION_TARGET_NODE_ROTATION,
    ANIMATION_TARGET_NODE_SCALE, ANIMATION_TARGET_NODE_TRANSLATION, ANIMATION_TIME_DOMAIN_FRAMES,
    ANIMATION_TIME_DOMAIN_SECONDS, ANIMATION_TIME_DOMAIN_TICKS, AnimationChannelTableSchema,
    AnimationClipTableSchema, AnimationDirectorySchema, AnimationInTangentsSchema,
    AnimationOutTangentsSchema, AnimationTimesSchema, AnimationValuesSchema,
    animation_channel_field, animation_clip_field, animation_directory_field,
    encode_animation_extrapolation, encode_animation_interpolation, encode_animation_target_kind,
    encode_animation_time_domain,
};
use crate::canonical_scene::store::schema::deformation::{
    DEFORMATION_LOGICAL_MORPH_FIELD_COUNT, DEFORMATION_MESH_INSTANCE_FIELD_COUNT,
    DeformationLogicalMorphTableSchema, DeformationMeshInstanceTableSchema,
    deformation_logical_morph_field,
};
use crate::canonical_scene::store::schema::geometry::{
    GEOMETRY_MESH_INSTANCE_FIELD_COUNT, GeometryMeshInstanceTableSchema,
    geometry_mesh_instance_field,
};
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, NODE_SCENE_NODE_FIELD_COUNT, NODE_SCENE_TRANSFORM_TRS,
    NodeSceneNodeTableSchema, PRESENCE_ABSENT, PRESENCE_PRESENT, node_scene_node_field,
};
use crate::math::Quat;

use super::super::require_scalar;
use super::deformation::DeformationLogicalMorphValidationProjection;
use super::geometry::GeometryMeshInstanceValidationProjection;

#[derive(Clone, Copy)]
pub(super) struct AnimationValidatedRefs {
    pub(super) clips: Option<CanonicalBlockRef<AnimationClipTableSchema>>,
    pub(super) channels: Option<CanonicalBlockRef<AnimationChannelTableSchema>>,
    pub(super) times: Option<CanonicalBlockRef<AnimationTimesSchema>>,
    pub(super) values: Option<CanonicalBlockRef<AnimationValuesSchema>>,
    pub(super) in_tangents: Option<CanonicalBlockRef<AnimationInTangentsSchema>>,
    pub(super) out_tangents: Option<CanonicalBlockRef<AnimationOutTangentsSchema>>,
}

#[derive(Clone, Copy)]
pub(super) struct AnimationValidatedShape {
    pub(super) refs: AnimationValidatedRefs,
    pub(super) clip_count: u32,
    pub(super) channel_count: u32,
    pub(super) time_scalar_count: u64,
    pub(super) value_scalar_count: u64,
    pub(super) in_tangent_scalar_count: u64,
    pub(super) out_tangent_scalar_count: u64,
    pub(super) payload_block_count: u32,
}

#[derive(Clone, Copy)]
pub(super) struct AnimationCanonicalDependencies {
    deformation_mesh_instances: Option<CanonicalBlockRef<DeformationMeshInstanceTableSchema>>,
    geometry_mesh_instances: Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>>,
    logical_morphs: Option<CanonicalBlockRef<DeformationLogicalMorphTableSchema>>,
}

impl AnimationCanonicalDependencies {
    pub(super) const fn new(
        deformation_mesh_instances: Option<CanonicalBlockRef<DeformationMeshInstanceTableSchema>>,
        geometry_mesh_instances: Option<CanonicalBlockRef<GeometryMeshInstanceTableSchema>>,
        logical_morphs: Option<CanonicalBlockRef<DeformationLogicalMorphTableSchema>>,
    ) -> Self {
        Self {
            deformation_mesh_instances,
            geometry_mesh_instances,
            logical_morphs,
        }
    }
}

/// Validation-scoped animation members retained for same-session cross-domain checks.
#[derive(Clone, Copy)]
pub(super) struct AnimationValidatedMembers<'session> {
    pub(super) clips:
        Option<CanonicalValidationMember<'session, RecordBlock, AnimationClipTableSchema>>,
    pub(super) channels:
        Option<CanonicalValidationMember<'session, RecordBlock, AnimationChannelTableSchema>>,
    pub(super) times:
        Option<CanonicalValidationMember<'session, ScalarSequence<f64>, AnimationTimesSchema>>,
    pub(super) values:
        Option<CanonicalValidationMember<'session, ScalarSequence<f32>, AnimationValuesSchema>>,
    pub(super) in_tangents:
        Option<CanonicalValidationMember<'session, ScalarSequence<f32>, AnimationInTangentsSchema>>,
    pub(super) out_tangents: Option<
        CanonicalValidationMember<'session, ScalarSequence<f32>, AnimationOutTangentsSchema>,
    >,
}

/// Metered semantic shape of one validated canonical animation graph.
#[derive(Clone, Copy)]
pub(super) struct AnimationMeteredShape<'session> {
    pub(super) members: AnimationValidatedMembers<'session>,
    pub(super) clip_count: u32,
    pub(super) channel_count: u32,
    pub(super) time_scalar_count: u64,
    pub(super) value_scalar_count: u64,
    pub(super) in_tangent_scalar_count: u64,
    pub(super) out_tangent_scalar_count: u64,
    pub(super) payload_block_count: u32,
}

/// Aggregate animation shape sealed before physical materialization begins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct AnimationMeteredCounts {
    pub(in crate::canonical_scene::store) clip_count: u32,
    pub(in crate::canonical_scene::store) channel_count: u32,
    pub(in crate::canonical_scene::store) time_scalar_count: u64,
    pub(in crate::canonical_scene::store) value_scalar_count: u64,
    pub(in crate::canonical_scene::store) in_tangent_scalar_count: u64,
    pub(in crate::canonical_scene::store) out_tangent_scalar_count: u64,
    pub(in crate::canonical_scene::store) payload_block_count: u32,
}

impl AnimationMeteredCounts {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::canonical_scene::store) const fn new(
        clip_count: u32,
        channel_count: u32,
        time_scalar_count: u64,
        value_scalar_count: u64,
        in_tangent_scalar_count: u64,
        out_tangent_scalar_count: u64,
        payload_block_count: u32,
    ) -> Self {
        Self {
            clip_count,
            channel_count,
            time_scalar_count,
            value_scalar_count,
            in_tangent_scalar_count,
            out_tangent_scalar_count,
            payload_block_count,
        }
    }
}

/// One clip boundary retained only while the enclosing validation session is active.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct AnimationClipValidationDescriptor {
    channel_end: u32,
}

impl AnimationClipValidationDescriptor {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self { channel_end: 0 };
}

/// One canonical channel summary needed to validate the shared payload sequences exactly once.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct AnimationChannelValidationDescriptor {
    target_kind: u8,
    value_width: u32,
    time_end: u32,
    value_end: u32,
    in_tangent_end: u32,
    out_tangent_end: u32,
}

impl AnimationChannelValidationDescriptor {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        target_kind: 0,
        value_width: 0,
        time_end: 0,
        value_end: 0,
        in_tangent_end: 0,
        out_tangent_end: 0,
    };
}

/// Exact animation scratch capacity derived from sealed aggregate counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct AnimationValidationScratchRequirements {
    pub(in crate::canonical_scene::store) clip_descriptors: usize,
    pub(in crate::canonical_scene::store) channel_descriptors: usize,
    pub(in crate::canonical_scene::store) target_marks: usize,
}

impl AnimationValidationScratchRequirements {
    pub(in crate::canonical_scene::store) fn try_new(
        counts: AnimationMeteredCounts,
        node_count: u32,
        deformation_mesh_instance_count: u32,
    ) -> Option<Self> {
        Some(Self {
            clip_descriptors: usize::try_from(counts.clip_count).ok()?,
            channel_descriptors: usize::try_from(counts.channel_count).ok()?,
            target_marks: required_animation_target_marks(
                usize::try_from(node_count).ok()?,
                usize::try_from(deformation_mesh_instance_count).ok()?,
            )?,
        })
    }

    pub(in crate::canonical_scene::store) fn initialization_work_units(self) -> Option<u64> {
        [
            self.clip_descriptors,
            self.channel_descriptors,
            self.target_marks,
        ]
        .into_iter()
        .try_fold(0_u64, |total, count| {
            total.checked_add(u64::try_from(count).ok()?)
        })
    }
}

/// Caller-owned, reservation-backed animation scratch.
pub(in crate::canonical_scene::store) struct AnimationValidationScratch<'scratch> {
    clip_descriptors: &'scratch mut [AnimationClipValidationDescriptor],
    channel_descriptors: &'scratch mut [AnimationChannelValidationDescriptor],
    target_marks: &'scratch mut [u32],
}

impl<'scratch> AnimationValidationScratch<'scratch> {
    pub(in crate::canonical_scene::store) const fn new(
        clip_descriptors: &'scratch mut [AnimationClipValidationDescriptor],
        channel_descriptors: &'scratch mut [AnimationChannelValidationDescriptor],
        target_marks: &'scratch mut [u32],
    ) -> Self {
        Self {
            clip_descriptors,
            channel_descriptors,
            target_marks,
        }
    }
}

/// Already-validated members and semantic counts used by animation target checks.
///
/// These members are never reopened through a raw view. Animation only verifies that its target
/// references point at the exact validation-scoped members and that their sealed semantic counts
/// admit the requested target.
#[derive(Clone, Copy)]
pub(super) struct AnimationMeteredDependencies<'session, 'projection> {
    nodes: CanonicalValidationMember<'session, RecordBlock, NodeSceneNodeTableSchema>,
    node_count: u32,
    node_transform_kinds: &'projection [u8],
    deformation_mesh_instances: Option<
        CanonicalValidationMember<'session, RecordBlock, DeformationMeshInstanceTableSchema>,
    >,
    deformation_mesh_instance_count: u32,
    geometry_mesh_instances:
        Option<CanonicalValidationMember<'session, RecordBlock, GeometryMeshInstanceTableSchema>>,
    geometry_mesh_instance_count: u32,
    logical_morphs: Option<
        CanonicalValidationMember<'session, RecordBlock, DeformationLogicalMorphTableSchema>,
    >,
    logical_morph_count: u32,
    geometry_mesh_instance_projections: &'projection [GeometryMeshInstanceValidationProjection],
    logical_morph_projections: &'projection [DeformationLogicalMorphValidationProjection],
}

impl<'session, 'projection> AnimationMeteredDependencies<'session, 'projection> {
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        nodes: CanonicalValidationMember<'session, RecordBlock, NodeSceneNodeTableSchema>,
        node_count: u32,
        node_transform_kinds: &'projection [u8],
        deformation_mesh_instances: Option<
            CanonicalValidationMember<'session, RecordBlock, DeformationMeshInstanceTableSchema>,
        >,
        deformation_mesh_instance_count: u32,
        geometry_mesh_instances: Option<
            CanonicalValidationMember<'session, RecordBlock, GeometryMeshInstanceTableSchema>,
        >,
        geometry_mesh_instance_count: u32,
        logical_morphs: Option<
            CanonicalValidationMember<'session, RecordBlock, DeformationLogicalMorphTableSchema>,
        >,
        logical_morph_count: u32,
        geometry_mesh_instance_projections: &'projection [GeometryMeshInstanceValidationProjection],
        logical_morph_projections: &'projection [DeformationLogicalMorphValidationProjection],
    ) -> Self {
        Self {
            nodes,
            node_count,
            node_transform_kinds,
            deformation_mesh_instances,
            deformation_mesh_instance_count,
            geometry_mesh_instances,
            geometry_mesh_instance_count,
            logical_morphs,
            logical_morph_count,
            geometry_mesh_instance_projections,
            logical_morph_projections,
        }
    }
}

#[derive(Clone, Copy)]
struct AnimationCanonicalTargetViews<'storage> {
    geometry_mesh_instances: Option<CanonicalRecordView<'storage>>,
    logical_morphs: Option<CanonicalRecordView<'storage>>,
}

struct AnimationValidationExpectation<'batch, 'input, R> {
    fresh: Option<&'batch ValidatedAnimationBatch<'input, R>>,
}

pub(super) fn required_animation_target_marks(
    node_rows: usize,
    deformation_mesh_instance_rows: usize,
) -> Option<usize> {
    node_rows
        .checked_mul(3)?
        .checked_add(deformation_mesh_instance_rows)
}

/// Validates the canonical animation graph through the exact metered visitor surface.
///
/// Only aggregate shape sealed before materialization crosses this boundary. The validator derives
/// every range, payload constraint, and target relation from canonical storage itself.
#[allow(
    clippy::too_many_arguments,
    reason = "the animation validator keeps its scoped members, dependencies, scratch, and sealed semantic authority explicit"
)]
pub(super) fn validate_animation_graph_metered<
    'session,
    'storage,
    'control,
    'operation,
    'projection,
    'scratch,
    O,
    PP,
    U,
>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    animation: CanonicalValidationMember<'session, RecordBlock, AnimationDirectorySchema>,
    dependencies: AnimationMeteredDependencies<'session, 'projection>,
    counts: AnimationMeteredCounts,
    scratch: &mut AnimationValidationScratch<'scratch>,
) -> Result<Option<AnimationMeteredShape<'session>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let invalid_empty_shape = counts.channel_count == 0
        && (counts.time_scalar_count != 0
            || counts.value_scalar_count != 0
            || counts.in_tangent_scalar_count != 0
            || counts.out_tangent_scalar_count != 0);
    let invalid_nonempty_shape = counts.channel_count != 0
        && (counts.clip_count == 0
            || counts.time_scalar_count == 0
            || counts.value_scalar_count == 0);
    if invalid_empty_shape
        || invalid_nonempty_shape
        || counts.in_tangent_scalar_count != counts.out_tangent_scalar_count
        || !metered_animation_dependencies_are_coherent(dependencies)
    {
        return Ok(None);
    }

    let Some(requirements) = AnimationValidationScratchRequirements::try_new(
        counts,
        dependencies.node_count,
        dependencies.deformation_mesh_instance_count,
    ) else {
        return Ok(None);
    };
    let Some(clip_descriptors) = scratch
        .clip_descriptors
        .get_mut(..requirements.clip_descriptors)
    else {
        return Ok(None);
    };
    let Some(channel_descriptors) = scratch
        .channel_descriptors
        .get_mut(..requirements.channel_descriptors)
    else {
        return Ok(None);
    };
    let Some(target_marks) = scratch.target_marks.get_mut(..requirements.target_marks) else {
        return Ok(None);
    };

    let Some(members) = validate_animation_directory_metered(session, animation, counts)? else {
        return Ok(None);
    };
    if !validate_animation_clips_metered(session, members, counts, clip_descriptors)?
        || !validate_animation_channels_metered(
            session,
            members,
            dependencies,
            counts,
            clip_descriptors,
            channel_descriptors,
            target_marks,
        )?
        || !validate_animation_times_metered(
            session,
            members.times,
            counts.time_scalar_count,
            channel_descriptors,
        )?
        || !validate_animation_f32_payload_metered(
            session,
            members.values,
            counts.value_scalar_count,
            channel_descriptors,
            MeteredAnimationF32Part::Values,
        )?
        || !validate_animation_f32_payload_metered(
            session,
            members.in_tangents,
            counts.in_tangent_scalar_count,
            channel_descriptors,
            MeteredAnimationF32Part::InTangents,
        )?
        || !validate_animation_f32_payload_metered(
            session,
            members.out_tangents,
            counts.out_tangent_scalar_count,
            channel_descriptors,
            MeteredAnimationF32Part::OutTangents,
        )?
    {
        return Ok(None);
    }

    let payload_block_count = [
        members.times.is_some(),
        members.values.is_some(),
        members.in_tangents.is_some(),
        members.out_tangents.is_some(),
    ]
    .into_iter()
    .try_fold(0_u32, |count, present| {
        count.checked_add(u32::from(present))
    });
    let Some(payload_block_count) = payload_block_count else {
        return Ok(None);
    };
    if payload_block_count != counts.payload_block_count {
        return Ok(None);
    }

    Ok(Some(AnimationMeteredShape {
        members,
        clip_count: counts.clip_count,
        channel_count: counts.channel_count,
        time_scalar_count: counts.time_scalar_count,
        value_scalar_count: counts.value_scalar_count,
        in_tangent_scalar_count: counts.in_tangent_scalar_count,
        out_tangent_scalar_count: counts.out_tangent_scalar_count,
        payload_block_count,
    }))
}

fn metered_animation_dependencies_are_coherent(
    dependencies: AnimationMeteredDependencies<'_, '_>,
) -> bool {
    dependencies.node_count != 0
        && usize::try_from(dependencies.node_count)
            .is_ok_and(|count| dependencies.node_transform_kinds.len() == count)
        && dependencies.deformation_mesh_instances.is_some()
            == (dependencies.deformation_mesh_instance_count != 0)
        && dependencies.geometry_mesh_instances.is_some()
            == (dependencies.geometry_mesh_instance_count != 0)
        && dependencies.logical_morphs.is_some() == (dependencies.logical_morph_count != 0)
        && dependencies.deformation_mesh_instance_count == dependencies.geometry_mesh_instance_count
        && usize::try_from(dependencies.geometry_mesh_instance_count)
            .is_ok_and(|count| dependencies.geometry_mesh_instance_projections.len() == count)
        && usize::try_from(dependencies.logical_morph_count)
            .is_ok_and(|count| dependencies.logical_morph_projections.len() == count)
}

fn validate_animation_directory_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    animation: CanonicalValidationMember<'session, RecordBlock, AnimationDirectorySchema>,
    counts: AnimationMeteredCounts,
) -> Result<Option<AnimationValidatedMembers<'session>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mut clips = None;
    let mut channels = None;
    let mut times = None;
    let mut values = None;
    let mut in_tangents = None;
    let mut out_tangents = None;
    let complete = session.visit_canonical_records(
        animation,
        1,
        ANIMATION_DIRECTORY_FIELD_COUNT,
        |_row_index, row| {
            let schema_revision = row
                .scalar(animation_directory_field::SCHEMA_REVISION, |value: u32| {
                    value
                })?;
            let clip_count =
                row.scalar(animation_directory_field::CLIP_COUNT, |value: u32| value)?;
            let clips_presence =
                row.scalar(animation_directory_field::CLIPS_PRESENCE, |value: u8| value)?;
            let clips_slot_valid = if counts.clip_count == 0 {
                row.scalar(animation_directory_field::CLIPS, |value: u8| value == 0)? == Some(true)
            } else {
                clips = row.record_ref::<AnimationClipTableSchema, _>(
                    animation_directory_field::CLIPS,
                    |value| value,
                )?;
                clips.is_some()
            };

            let channel_count =
                row.scalar(animation_directory_field::CHANNEL_COUNT, |value: u32| value)?;
            let channels_presence = row
                .scalar(animation_directory_field::CHANNELS_PRESENCE, |value: u8| {
                    value
                })?;
            let channels_slot_valid = if counts.channel_count == 0 {
                row.scalar(animation_directory_field::CHANNELS, |value: u8| value == 0)?
                    == Some(true)
            } else {
                channels = row.record_ref::<AnimationChannelTableSchema, _>(
                    animation_directory_field::CHANNELS,
                    |value| value,
                )?;
                channels.is_some()
            };

            let time_scalar_count = row.scalar(
                animation_directory_field::TIME_SCALAR_COUNT,
                |value: u64| value,
            )?;
            let times_presence =
                row.scalar(animation_directory_field::TIMES_PRESENCE, |value: u8| value)?;
            let times_slot_valid = if counts.time_scalar_count == 0 {
                row.scalar(animation_directory_field::TIMES, |value: u8| value == 0)? == Some(true)
            } else {
                times = row.sequence_ref::<f64, AnimationTimesSchema, _>(
                    animation_directory_field::TIMES,
                    |value| value,
                )?;
                times.is_some()
            };

            let value_scalar_count = row.scalar(
                animation_directory_field::VALUE_SCALAR_COUNT,
                |value: u64| value,
            )?;
            let values_presence = row
                .scalar(animation_directory_field::VALUES_PRESENCE, |value: u8| {
                    value
                })?;
            let values_slot_valid = if counts.value_scalar_count == 0 {
                row.scalar(animation_directory_field::VALUES, |value: u8| value == 0)? == Some(true)
            } else {
                values = row.sequence_ref::<f32, AnimationValuesSchema, _>(
                    animation_directory_field::VALUES,
                    |value| value,
                )?;
                values.is_some()
            };

            let in_tangent_scalar_count = row.scalar(
                animation_directory_field::IN_TANGENT_SCALAR_COUNT,
                |value: u64| value,
            )?;
            let in_tangents_presence = row.scalar(
                animation_directory_field::IN_TANGENTS_PRESENCE,
                |value: u8| value,
            )?;
            let in_tangents_slot_valid = if counts.in_tangent_scalar_count == 0 {
                row.scalar(animation_directory_field::IN_TANGENTS, |value: u8| {
                    value == 0
                })? == Some(true)
            } else {
                in_tangents = row.sequence_ref::<f32, AnimationInTangentsSchema, _>(
                    animation_directory_field::IN_TANGENTS,
                    |value| value,
                )?;
                in_tangents.is_some()
            };

            let out_tangent_scalar_count = row.scalar(
                animation_directory_field::OUT_TANGENT_SCALAR_COUNT,
                |value: u64| value,
            )?;
            let out_tangents_presence = row.scalar(
                animation_directory_field::OUT_TANGENTS_PRESENCE,
                |value: u8| value,
            )?;
            let out_tangents_slot_valid = if counts.out_tangent_scalar_count == 0 {
                row.scalar(animation_directory_field::OUT_TANGENTS, |value: u8| {
                    value == 0
                })? == Some(true)
            } else {
                out_tangents = row.sequence_ref::<f32, AnimationOutTangentsSchema, _>(
                    animation_directory_field::OUT_TANGENTS,
                    |value| value,
                )?;
                out_tangents.is_some()
            };

            Ok(schema_revision == Some(ANIMATION_SCHEMA_REVISION)
                && clip_count == Some(counts.clip_count)
                && clips_presence
                    == Some(if counts.clip_count == 0 {
                        PRESENCE_ABSENT
                    } else {
                        PRESENCE_PRESENT
                    })
                && clips_slot_valid
                && channel_count == Some(counts.channel_count)
                && channels_presence
                    == Some(if counts.channel_count == 0 {
                        PRESENCE_ABSENT
                    } else {
                        PRESENCE_PRESENT
                    })
                && channels_slot_valid
                && time_scalar_count == Some(counts.time_scalar_count)
                && times_presence
                    == Some(if counts.time_scalar_count == 0 {
                        PRESENCE_ABSENT
                    } else {
                        PRESENCE_PRESENT
                    })
                && times_slot_valid
                && value_scalar_count == Some(counts.value_scalar_count)
                && values_presence
                    == Some(if counts.value_scalar_count == 0 {
                        PRESENCE_ABSENT
                    } else {
                        PRESENCE_PRESENT
                    })
                && values_slot_valid
                && in_tangent_scalar_count == Some(counts.in_tangent_scalar_count)
                && in_tangents_presence
                    == Some(if counts.in_tangent_scalar_count == 0 {
                        PRESENCE_ABSENT
                    } else {
                        PRESENCE_PRESENT
                    })
                && in_tangents_slot_valid
                && out_tangent_scalar_count == Some(counts.out_tangent_scalar_count)
                && out_tangents_presence
                    == Some(if counts.out_tangent_scalar_count == 0 {
                        PRESENCE_ABSENT
                    } else {
                        PRESENCE_PRESENT
                    })
                && out_tangents_slot_valid)
        },
    )?;
    if !complete {
        return Ok(None);
    }
    Ok(Some(AnimationValidatedMembers {
        clips,
        channels,
        times,
        values,
        in_tangents,
        out_tangents,
    }))
}

#[allow(
    clippy::too_many_arguments,
    reason = "each animation channel carries one explicit typed target and four independently bounded payload ranges"
)]
fn validate_animation_channels_metered<
    'session,
    'storage,
    'control,
    'operation,
    'projection,
    O,
    PP,
    U,
>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    members: AnimationValidatedMembers<'session>,
    dependencies: AnimationMeteredDependencies<'session, 'projection>,
    counts: AnimationMeteredCounts,
    clip_descriptors: &[AnimationClipValidationDescriptor],
    channel_descriptors: &mut [AnimationChannelValidationDescriptor],
    target_marks: &mut [u32],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(channels) = members.channels else {
        return Ok(counts.channel_count == 0);
    };
    let Ok(channel_rows) = usize::try_from(counts.channel_count) else {
        return Ok(false);
    };
    let Some(node_target_mark_count) = usize::try_from(dependencies.node_count)
        .ok()
        .and_then(|count| count.checked_mul(3))
    else {
        return Ok(false);
    };
    let mut clip_index = 0_usize;
    let mut time_cursor = 0_u32;
    let mut value_cursor = 0_u32;
    let mut in_tangent_cursor = 0_u32;
    let mut out_tangent_cursor = 0_u32;
    let complete = session.visit_canonical_records(
        channels,
        channel_rows,
        ANIMATION_CHANNEL_FIELD_COUNT,
        |row_index, row| {
            let channel = row.scalar(animation_channel_field::CHANNEL, |value: u32| value)?;
            let target_kind =
                row.scalar(animation_channel_field::TARGET_KIND, |value: u8| value)?;
            let target_index =
                row.scalar(animation_channel_field::TARGET_INDEX, |value: u32| value)?;
            let target_reference_matches = match target_kind {
                Some(
                    ANIMATION_TARGET_NODE_TRANSLATION
                    | ANIMATION_TARGET_NODE_ROTATION
                    | ANIMATION_TARGET_NODE_SCALE,
                ) => {
                    row.record_ref::<NodeSceneNodeTableSchema, _>(
                        animation_channel_field::TARGETS,
                        |value| value == dependencies.nodes,
                    )? == Some(true)
                }
                Some(ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS) => {
                    row.record_ref::<DeformationMeshInstanceTableSchema, _>(
                        animation_channel_field::TARGETS,
                        |value| Some(value) == dependencies.deformation_mesh_instances,
                    )? == Some(true)
                }
                _ => {
                    row.scalar(animation_channel_field::TARGETS, |value: u8| value == 0)?
                        == Some(true)
                }
            };
            let interpolation =
                row.scalar(animation_channel_field::INTERPOLATION, |value: u8| value)?;
            let pre_extrapolation = row
                .scalar(animation_channel_field::PRE_EXTRAPOLATION, |value: u8| {
                    value
                })?;
            let post_extrapolation = row
                .scalar(animation_channel_field::POST_EXTRAPOLATION, |value: u8| {
                    value
                })?;
            let key_count = row.scalar(animation_channel_field::KEY_COUNT, |value: u32| value)?;
            let value_width =
                row.scalar(animation_channel_field::VALUE_WIDTH, |value: u32| value)?;
            let time_offset =
                row.scalar(animation_channel_field::TIME_OFFSET, |value: u32| value)?;
            let time_count = row.scalar(animation_channel_field::TIME_COUNT, |value: u32| value)?;
            let times_reference_matches = row.sequence_ref::<f64, AnimationTimesSchema, _>(
                animation_channel_field::TIMES,
                |value| Some(value) == members.times,
            )? == Some(true);
            let value_offset =
                row.scalar(animation_channel_field::VALUE_OFFSET, |value: u32| value)?;
            let value_count =
                row.scalar(animation_channel_field::VALUE_COUNT, |value: u32| value)?;
            let values_reference_matches = row.sequence_ref::<f32, AnimationValuesSchema, _>(
                animation_channel_field::VALUES,
                |value| Some(value) == members.values,
            )? == Some(true);

            let in_tangents_presence = row.scalar(
                animation_channel_field::IN_TANGENTS_PRESENCE,
                |value: u8| value,
            )?;
            let in_tangent_offset = row
                .scalar(animation_channel_field::IN_TANGENT_OFFSET, |value: u32| {
                    value
                })?;
            let in_tangent_count = row
                .scalar(animation_channel_field::IN_TANGENT_COUNT, |value: u32| {
                    value
                })?;
            let in_tangents_reference_matches = if in_tangents_presence == Some(PRESENCE_PRESENT) {
                row.sequence_ref::<f32, AnimationInTangentsSchema, _>(
                    animation_channel_field::IN_TANGENTS,
                    |value| Some(value) == members.in_tangents,
                )? == Some(true)
            } else {
                row.scalar(animation_channel_field::IN_TANGENTS, |value: u8| value == 0)?
                    == Some(true)
            };

            let out_tangents_presence = row.scalar(
                animation_channel_field::OUT_TANGENTS_PRESENCE,
                |value: u8| value,
            )?;
            let out_tangent_offset = row
                .scalar(animation_channel_field::OUT_TANGENT_OFFSET, |value: u32| {
                    value
                })?;
            let out_tangent_count = row
                .scalar(animation_channel_field::OUT_TANGENT_COUNT, |value: u32| {
                    value
                })?;
            let out_tangents_reference_matches = if out_tangents_presence == Some(PRESENCE_PRESENT)
            {
                row.sequence_ref::<f32, AnimationOutTangentsSchema, _>(
                    animation_channel_field::OUT_TANGENTS,
                    |value| Some(value) == members.out_tangents,
                )? == Some(true)
            } else {
                row.scalar(animation_channel_field::OUT_TANGENTS, |value: u8| {
                    value == 0
                })? == Some(true)
            };

            let Some(row_id) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            let Some(target_kind) = target_kind else {
                return Ok(false);
            };
            let Some(target_index) = target_index else {
                return Ok(false);
            };
            let Some(expected_value_width) =
                metered_animation_target_width(dependencies, target_kind, target_index)
            else {
                return Ok(false);
            };
            let Some(key_count) = key_count else {
                return Ok(false);
            };
            let Some(value_width) = value_width else {
                return Ok(false);
            };
            let Some(time_offset) = time_offset else {
                return Ok(false);
            };
            let Some(time_count) = time_count else {
                return Ok(false);
            };
            let Some(value_offset) = value_offset else {
                return Ok(false);
            };
            let Some(value_count) = value_count else {
                return Ok(false);
            };
            let Some(interpolation) = interpolation else {
                return Ok(false);
            };
            let Some(pre_extrapolation) = pre_extrapolation else {
                return Ok(false);
            };
            let Some(post_extrapolation) = post_extrapolation else {
                return Ok(false);
            };
            let Some(expected_value_count) = key_count.checked_mul(value_width) else {
                return Ok(false);
            };
            let Some(time_end) = time_offset.checked_add(time_count) else {
                return Ok(false);
            };
            let Some(value_end) = value_offset.checked_add(value_count) else {
                return Ok(false);
            };
            let Some(in_tangent_end) = metered_tangent_end(
                interpolation,
                in_tangents_presence,
                in_tangent_offset,
                in_tangent_count,
                in_tangents_reference_matches,
                in_tangent_cursor,
                value_count,
            ) else {
                return Ok(false);
            };
            let Some(out_tangent_end) = metered_tangent_end(
                interpolation,
                out_tangents_presence,
                out_tangent_offset,
                out_tangent_count,
                out_tangents_reference_matches,
                out_tangent_cursor,
                value_count,
            ) else {
                return Ok(false);
            };
            let Some(clip_marker) =
                metered_animation_clip_marker(row_id, clip_descriptors, &mut clip_index)
            else {
                return Ok(false);
            };
            let Some(mark_index) = metered_animation_target_mark_index(
                target_kind,
                target_index,
                node_target_mark_count,
            ) else {
                return Ok(false);
            };
            let Some(mark) = target_marks.get_mut(mark_index) else {
                return Ok(false);
            };
            if *mark == clip_marker {
                return Ok(false);
            }

            let valid = channel == Some(row_id)
                && target_reference_matches
                && value_width == expected_value_width
                && valid_interpolation(interpolation)
                && valid_extrapolation(pre_extrapolation)
                && valid_extrapolation(post_extrapolation)
                && key_count != 0
                && time_offset == time_cursor
                && time_count == key_count
                && times_reference_matches
                && value_offset == value_cursor
                && value_count == expected_value_count
                && values_reference_matches;
            if !valid {
                return Ok(false);
            }

            *mark = clip_marker;
            let Some(descriptor) = channel_descriptors.get_mut(row_index) else {
                return Ok(false);
            };
            *descriptor = AnimationChannelValidationDescriptor {
                target_kind,
                value_width,
                time_end,
                value_end,
                in_tangent_end,
                out_tangent_end,
            };
            time_cursor = time_end;
            value_cursor = value_end;
            in_tangent_cursor = in_tangent_end;
            out_tangent_cursor = out_tangent_end;
            Ok(true)
        },
    )?;
    Ok(complete
        && u64::from(time_cursor) == counts.time_scalar_count
        && u64::from(value_cursor) == counts.value_scalar_count
        && u64::from(in_tangent_cursor) == counts.in_tangent_scalar_count
        && u64::from(out_tangent_cursor) == counts.out_tangent_scalar_count)
}

fn metered_animation_target_width(
    dependencies: AnimationMeteredDependencies<'_, '_>,
    target_kind: u8,
    target_index: u32,
) -> Option<u32> {
    let target_index = usize::try_from(target_index).ok()?;
    match target_kind {
        ANIMATION_TARGET_NODE_TRANSLATION | ANIMATION_TARGET_NODE_SCALE => {
            if target_index == 0
                || target_index >= usize::try_from(dependencies.node_count).ok()?
                || dependencies
                    .node_transform_kinds
                    .get(target_index)
                    .copied()?
                    != NODE_SCENE_TRANSFORM_TRS
            {
                return None;
            }
            Some(3)
        }
        ANIMATION_TARGET_NODE_ROTATION => {
            if target_index == 0
                || target_index >= usize::try_from(dependencies.node_count).ok()?
                || dependencies
                    .node_transform_kinds
                    .get(target_index)
                    .copied()?
                    != NODE_SCENE_TRANSFORM_TRS
            {
                return None;
            }
            Some(4)
        }
        ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS => {
            if dependencies.deformation_mesh_instances.is_none()
                || dependencies.geometry_mesh_instances.is_none()
                || dependencies.logical_morphs.is_none()
                || target_index
                    >= usize::try_from(dependencies.deformation_mesh_instance_count).ok()?
            {
                return None;
            }
            let logical_mesh = usize::try_from(
                dependencies
                    .geometry_mesh_instance_projections
                    .get(target_index)?
                    .logical_mesh,
            )
            .ok()?;
            let width = dependencies.logical_morph_projections.get(logical_mesh)?;
            let width = width.definition_count();
            (width != 0).then_some(width)
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn metered_tangent_end(
    interpolation: u8,
    presence: Option<u8>,
    offset: Option<u32>,
    count: Option<u32>,
    reference_matches: bool,
    cursor: u32,
    value_count: u32,
) -> Option<u32> {
    match presence? {
        PRESENCE_ABSENT
            if interpolation != ANIMATION_INTERPOLATION_CUBIC_SPLINE
                && offset == Some(0)
                && count == Some(0)
                && reference_matches =>
        {
            Some(cursor)
        }
        PRESENCE_PRESENT
            if interpolation == ANIMATION_INTERPOLATION_CUBIC_SPLINE
                && offset == Some(cursor)
                && count == Some(value_count)
                && value_count != 0
                && reference_matches =>
        {
            cursor.checked_add(value_count)
        }
        _ => None,
    }
}

fn metered_animation_clip_marker(
    channel_index: u32,
    descriptors: &[AnimationClipValidationDescriptor],
    clip_index: &mut usize,
) -> Option<u32> {
    loop {
        let descriptor = descriptors.get(*clip_index)?;
        if channel_index < descriptor.channel_end {
            return u32::try_from(*clip_index).ok()?.checked_add(1);
        }
        *clip_index = (*clip_index).checked_add(1)?;
    }
}

fn metered_animation_target_mark_index(
    target_kind: u8,
    target_index: u32,
    node_target_mark_count: usize,
) -> Option<usize> {
    let target_index = usize::try_from(target_index).ok()?;
    match target_kind {
        ANIMATION_TARGET_NODE_TRANSLATION => target_index.checked_mul(3),
        ANIMATION_TARGET_NODE_ROTATION => target_index.checked_mul(3)?.checked_add(1),
        ANIMATION_TARGET_NODE_SCALE => target_index.checked_mul(3)?.checked_add(2),
        ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS => {
            node_target_mark_count.checked_add(target_index)
        }
        _ => None,
    }
}

fn validate_animation_clips_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    members: AnimationValidatedMembers<'session>,
    counts: AnimationMeteredCounts,
    descriptors: &mut [AnimationClipValidationDescriptor],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(clips) = members.clips else {
        return Ok(counts.clip_count == 0);
    };
    let Ok(clip_rows) = usize::try_from(counts.clip_count) else {
        return Ok(false);
    };
    let mut channel_cursor = 0_u32;
    let complete = session.visit_canonical_records(
        clips,
        clip_rows,
        ANIMATION_CLIP_FIELD_COUNT,
        |row_index, row| {
            let clip = row.scalar(animation_clip_field::CLIP, |value: u32| value)?;
            let duration = row.scalar(animation_clip_field::DURATION, |value: f64| value)?;
            let time_domain = row.scalar(animation_clip_field::TIME_DOMAIN, |value: u8| value)?;
            let rate_presence =
                row.scalar(animation_clip_field::RATE_PRESENCE, |value: u8| value)?;
            let mut rate = None;
            let mut absent_rate_is_zero = None;
            if rate_presence == Some(PRESENCE_PRESENT) {
                rate = row.scalar(animation_clip_field::RATE, |value: f64| value)?;
            } else {
                absent_rate_is_zero =
                    row.scalar(animation_clip_field::RATE, |value: u8| value == 0)?;
            }
            let channel_offset =
                row.scalar(animation_clip_field::CHANNEL_OFFSET, |value: u32| value)?;
            let channel_count =
                row.scalar(animation_clip_field::CHANNEL_COUNT, |value: u32| value)?;
            let channels_presence =
                row.scalar(animation_clip_field::CHANNELS_PRESENCE, |value: u8| value)?;
            let channels_reference_matches = if channel_count.is_some_and(|count| count != 0) {
                row.record_ref::<AnimationChannelTableSchema, _>(
                    animation_clip_field::CHANNELS,
                    |value| Some(value) == members.channels,
                )? == Some(true)
            } else {
                row.scalar(animation_clip_field::CHANNELS, |value: u8| value == 0)? == Some(true)
            };

            let Some(row_id) = u32::try_from(row_index).ok() else {
                return Ok(false);
            };
            let Some(duration) = duration else {
                return Ok(false);
            };
            let Some(time_domain) = time_domain else {
                return Ok(false);
            };
            let Some(channel_offset) = channel_offset else {
                return Ok(false);
            };
            let Some(channel_count) = channel_count else {
                return Ok(false);
            };
            let Some(channel_end) = channel_offset.checked_add(channel_count) else {
                return Ok(false);
            };
            let rate_is_valid = match rate_presence {
                Some(PRESENCE_ABSENT) => {
                    absent_rate_is_zero == Some(true)
                        && matches!(
                            time_domain,
                            ANIMATION_TIME_DOMAIN_SECONDS | ANIMATION_TIME_DOMAIN_TICKS
                        )
                }
                Some(PRESENCE_PRESENT) => {
                    rate.is_some_and(|rate| rate.is_finite() && rate > 0.0)
                        && matches!(
                            time_domain,
                            ANIMATION_TIME_DOMAIN_FRAMES | ANIMATION_TIME_DOMAIN_TICKS
                        )
                }
                _ => false,
            };
            let collection_is_valid = if channel_count == 0 {
                channels_presence == Some(PRESENCE_ABSENT) && channels_reference_matches
            } else {
                channels_presence == Some(PRESENCE_PRESENT) && channels_reference_matches
            };
            let valid = clip == Some(row_id)
                && duration.is_finite()
                && duration >= 0.0
                && valid_time_domain(time_domain)
                && rate_is_valid
                && channel_offset == channel_cursor
                && channel_end <= counts.channel_count
                && collection_is_valid;
            if !valid {
                return Ok(false);
            }
            let Some(descriptor) = descriptors.get_mut(row_index) else {
                return Ok(false);
            };
            *descriptor = AnimationClipValidationDescriptor { channel_end };
            channel_cursor = channel_end;
            Ok(true)
        },
    )?;
    Ok(complete && channel_cursor == counts.channel_count)
}

fn validate_animation_times_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    times: Option<CanonicalValidationMember<'session, ScalarSequence<f64>, AnimationTimesSchema>>,
    expected_len: u64,
    descriptors: &[AnimationChannelValidationDescriptor],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Ok(expected_len) = usize::try_from(expected_len) else {
        return Ok(false);
    };
    let Some(times) = times else {
        return Ok(expected_len == 0);
    };
    let mut channel_index = 0_usize;
    let mut previous = None;
    session.visit_canonical_sequence(times, expected_len, |index, actual| {
        let Ok(index_u32) = u32::try_from(index) else {
            return false;
        };
        loop {
            let Some(descriptor) = descriptors.get(channel_index) else {
                return false;
            };
            if index_u32 < descriptor.time_end {
                if !actual.is_finite() || previous.is_some_and(|previous| previous >= actual) {
                    return false;
                }
                previous = Some(actual);
                return true;
            }
            channel_index = match channel_index.checked_add(1) {
                Some(value) => value,
                None => return false,
            };
            previous = None;
        }
    })
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum MeteredAnimationF32Part {
    Values,
    InTangents,
    OutTangents,
}

fn validate_animation_f32_payload_metered<'session, 'storage, 'control, 'operation, O, PP, U, Tag>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<CanonicalValidationMember<'session, ScalarSequence<f32>, Tag>>,
    expected_len: u64,
    descriptors: &[AnimationChannelValidationDescriptor],
    part: MeteredAnimationF32Part,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: Copy + Send + Sync + 'static,
{
    let Ok(expected_len) = usize::try_from(expected_len) else {
        return Ok(false);
    };
    let Some(member) = member else {
        return Ok(expected_len == 0);
    };
    let mut channel_index = 0_usize;
    let mut channel_start = 0_u32;
    let mut rotation = [0.0_f32; 4];
    session.visit_canonical_sequence(member, expected_len, |index, actual| {
        let Ok(index_u32) = u32::try_from(index) else {
            return false;
        };
        loop {
            let Some(descriptor) = descriptors.get(channel_index) else {
                return false;
            };
            let channel_end = match part {
                MeteredAnimationF32Part::Values => descriptor.value_end,
                MeteredAnimationF32Part::InTangents => descriptor.in_tangent_end,
                MeteredAnimationF32Part::OutTangents => descriptor.out_tangent_end,
            };
            if index_u32 < channel_end {
                if !actual.is_finite() {
                    return false;
                }
                if part == MeteredAnimationF32Part::Values
                    && descriptor.target_kind == ANIMATION_TARGET_NODE_ROTATION
                {
                    if descriptor.value_width != 4 {
                        return false;
                    }
                    let Some(local_index) = index_u32.checked_sub(channel_start) else {
                        return false;
                    };
                    let Some(component) = usize::try_from(local_index % 4).ok() else {
                        return false;
                    };
                    rotation[component] = actual;
                    if component == 3
                        && !Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3])
                            .is_unit()
                    {
                        return false;
                    }
                }
                return true;
            }
            channel_start = channel_end;
            channel_index = match channel_index.checked_add(1) {
                Some(value) => value,
                None => return false,
            };
            rotation = [0.0; 4];
        }
    })
}

pub(super) fn validate_animation_graph<R>(
    session: &mut StoreValidationSession<'_, '_>,
    animation_ref: CanonicalRecordRef<AnimationDirectorySchema>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    deformation_mesh_instances: Option<CanonicalRecordRef<DeformationMeshInstanceTableSchema>>,
    expected: &ValidatedAnimationBatch<'_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<AnimationValidatedRefs> {
    let mut expectation = AnimationValidationExpectation {
        fresh: Some(expected),
    };
    validate_animation_graph_inner(
        session,
        animation_ref,
        nodes_ref,
        deformation_mesh_instances,
        None,
        None,
        &mut expectation,
        observe_work,
    )
    .map(|shape| shape.refs)
}

/// Validates canonical animation storage without consulting a producer batch.
///
/// `target_marks` is caller-owned, metered scratch. Its usable prefix must be zero-initialized and
/// cover three target slots per canonical node plus one per deformation mesh instance.
pub(super) fn validate_animation_graph_structure(
    session: &mut StoreValidationSession<'_, '_>,
    animation_ref: CanonicalRecordRef<AnimationDirectorySchema>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    dependencies: AnimationCanonicalDependencies,
    target_marks: &mut [u32],
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<AnimationValidatedShape> {
    let mut expectation = AnimationValidationExpectation::<()> { fresh: None };
    validate_animation_graph_inner(
        session,
        animation_ref,
        nodes_ref,
        dependencies
            .deformation_mesh_instances
            .map(CanonicalBlockRef::into_store),
        Some(dependencies),
        Some(target_marks),
        &mut expectation,
        observe_work,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "the central validator keeps storage references, optional proof authorities, scratch, and work accounting explicit"
)]
fn validate_animation_graph_inner<'storage, R>(
    session: &mut StoreValidationSession<'_, 'storage>,
    animation_ref: CanonicalRecordRef<AnimationDirectorySchema>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    deformation_mesh_instances: Option<CanonicalRecordRef<DeformationMeshInstanceTableSchema>>,
    canonical_dependencies: Option<AnimationCanonicalDependencies>,
    target_marks: Option<&mut [u32]>,
    expectation: &mut AnimationValidationExpectation<'_, '_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<AnimationValidatedShape> {
    observe_work(1)?;
    let directory = session.canonical_records(animation_ref)?;
    if directory.rows() != 1 || directory.fields() != ANIMATION_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, animation_directory_field::SCHEMA_REVISION),
        ANIMATION_SCHEMA_REVISION,
    )?;
    if let Some(expected) = expectation.fresh {
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
    }

    let nodes = session.canonical_records(nodes_ref)?;
    if nodes.rows() == 0 || nodes.fields() != NODE_SCENE_NODE_FIELD_COUNT {
        return None;
    }
    let node_rows = nodes.rows();
    let deformation_mesh_instance_table = match deformation_mesh_instances {
        Some(reference) => {
            let table = session.canonical_records(reference)?;
            if table.fields() != DEFORMATION_MESH_INSTANCE_FIELD_COUNT || table.rows() == 0 {
                return None;
            }
            Some(table)
        }
        None => None,
    };
    let canonical_target_views = match canonical_dependencies {
        Some(dependencies) => Some(validate_canonical_target_views(
            session,
            dependencies,
            deformation_mesh_instance_table,
        )?),
        None => None,
    };
    let node_target_mark_count = match target_marks.as_deref() {
        Some(target_marks) => {
            let deformation_mesh_instance_rows =
                deformation_mesh_instance_table.map_or(0, CanonicalRecordView::rows);
            let target_mark_count =
                required_animation_target_marks(node_rows, deformation_mesh_instance_rows)?;
            if target_marks.len() < target_mark_count {
                return None;
            }
            target_mark_count.checked_sub(deformation_mesh_instance_rows)?
        }
        None => 0,
    };

    let (clip_count, clips) = optional_record_ref::<AnimationClipTableSchema>(
        session,
        directory,
        animation_directory_field::CLIP_COUNT,
        animation_directory_field::CLIPS_PRESENCE,
        animation_directory_field::CLIPS,
        expectation.fresh.map(ValidatedAnimationBatch::clip_count),
        ANIMATION_CLIP_FIELD_COUNT,
    )?;
    let (channel_count, channels) = optional_record_ref::<AnimationChannelTableSchema>(
        session,
        directory,
        animation_directory_field::CHANNEL_COUNT,
        animation_directory_field::CHANNELS_PRESENCE,
        animation_directory_field::CHANNELS,
        expectation
            .fresh
            .map(ValidatedAnimationBatch::channel_count),
        ANIMATION_CHANNEL_FIELD_COUNT,
    )?;
    let (time_scalar_count, times) = optional_sequence_ref::<f64, AnimationTimesSchema>(
        session,
        directory,
        animation_directory_field::TIME_SCALAR_COUNT,
        animation_directory_field::TIMES_PRESENCE,
        animation_directory_field::TIMES,
        expectation
            .fresh
            .map(ValidatedAnimationBatch::time_scalar_count),
    )?;
    let (value_scalar_count, values) = optional_sequence_ref::<f32, AnimationValuesSchema>(
        session,
        directory,
        animation_directory_field::VALUE_SCALAR_COUNT,
        animation_directory_field::VALUES_PRESENCE,
        animation_directory_field::VALUES,
        expectation
            .fresh
            .map(ValidatedAnimationBatch::value_scalar_count),
    )?;
    let (in_tangent_scalar_count, in_tangents) =
        optional_sequence_ref::<f32, AnimationInTangentsSchema>(
            session,
            directory,
            animation_directory_field::IN_TANGENT_SCALAR_COUNT,
            animation_directory_field::IN_TANGENTS_PRESENCE,
            animation_directory_field::IN_TANGENTS,
            expectation
                .fresh
                .map(ValidatedAnimationBatch::in_tangent_scalar_count),
        )?;
    let (out_tangent_scalar_count, out_tangents) =
        optional_sequence_ref::<f32, AnimationOutTangentsSchema>(
            session,
            directory,
            animation_directory_field::OUT_TANGENT_SCALAR_COUNT,
            animation_directory_field::OUT_TANGENTS_PRESENCE,
            animation_directory_field::OUT_TANGENTS,
            expectation
                .fresh
                .map(ValidatedAnimationBatch::out_tangent_scalar_count),
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
    if let Some(expected) = expectation.fresh
        && payload_block_count != expected.payload_block_count()
    {
        return None;
    }
    let invalid_empty_shape = channel_count == 0
        && (time_scalar_count != 0
            || value_scalar_count != 0
            || in_tangent_scalar_count != 0
            || out_tangent_scalar_count != 0);
    let invalid_nonempty_shape = channel_count != 0
        && (clip_count == 0 || time_scalar_count == 0 || value_scalar_count == 0);
    if invalid_empty_shape
        || invalid_nonempty_shape
        || in_tangent_scalar_count != out_tangent_scalar_count
    {
        return None;
    }

    let channel_table = validate_channel_table(
        session,
        channels,
        times,
        values,
        in_tangents,
        out_tangents,
        nodes_ref,
        node_rows,
        deformation_mesh_instances,
        deformation_mesh_instance_table,
        canonical_target_views,
        channel_count,
        time_scalar_count,
        value_scalar_count,
        in_tangent_scalar_count,
        out_tangent_scalar_count,
        nodes,
        expectation,
        observe_work,
    )?;
    validate_clip_table(
        session,
        clips,
        channels,
        channel_table,
        clip_count,
        channel_count,
        expectation.fresh,
        target_marks,
        node_target_mark_count,
        observe_work,
    )?;

    Some(AnimationValidatedShape {
        refs: AnimationValidatedRefs {
            clips: clips.map(CanonicalBlockRef::from_store),
            channels: channels.map(CanonicalBlockRef::from_store),
            times: times.map(CanonicalBlockRef::from_store),
            values: values.map(CanonicalBlockRef::from_store),
            in_tangents: in_tangents.map(CanonicalBlockRef::from_store),
            out_tangents: out_tangents.map(CanonicalBlockRef::from_store),
        },
        clip_count,
        channel_count,
        time_scalar_count,
        value_scalar_count,
        in_tangent_scalar_count,
        out_tangent_scalar_count,
        payload_block_count,
    })
}

fn validate_canonical_target_views<'storage>(
    session: &mut StoreValidationSession<'_, 'storage>,
    dependencies: AnimationCanonicalDependencies,
    deformation_mesh_instances: Option<CanonicalRecordView<'storage>>,
) -> Option<AnimationCanonicalTargetViews<'storage>> {
    let geometry_mesh_instances = match dependencies.geometry_mesh_instances {
        Some(reference) => {
            let table = session.canonical_records(reference.into_store())?;
            if table.rows() == 0 || table.fields() != GEOMETRY_MESH_INSTANCE_FIELD_COUNT {
                return None;
            }
            Some(table)
        }
        None => None,
    };
    match (deformation_mesh_instances, geometry_mesh_instances) {
        (Some(deformation), Some(geometry)) if deformation.rows() == geometry.rows() => {}
        (None, None) => {}
        _ => return None,
    }

    let logical_morphs = match dependencies.logical_morphs {
        Some(reference) => {
            let table = session.canonical_records(reference.into_store())?;
            if table.rows() == 0 || table.fields() != DEFORMATION_LOGICAL_MORPH_FIELD_COUNT {
                return None;
            }
            Some(table)
        }
        None => None,
    };
    Some(AnimationCanonicalTargetViews {
        geometry_mesh_instances,
        logical_morphs,
    })
}

#[allow(clippy::too_many_arguments)]
fn optional_record_ref<Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    presence_field: usize,
    reference_field: usize,
    expected_count: Option<u32>,
    expected_fields: usize,
) -> Option<(u32, Option<CanonicalRecordRef<Tag>>)>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    let count = directory.scalar::<u32>(0, count_field)?;
    if let Some(expected_count) = expected_count {
        require_scalar(Some(count), expected_count)?;
    }
    if count == 0 {
        require_scalar(directory.scalar::<u8>(0, presence_field), PRESENCE_ABSENT)?;
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some((count, None))
    } else {
        require_scalar(directory.scalar::<u8>(0, presence_field), PRESENCE_PRESENT)?;
        let reference = directory.record_ref::<Tag>(0, reference_field)?;
        let table = session.canonical_records(reference)?;
        if table.rows() != usize::try_from(count).ok()? || table.fields() != expected_fields {
            return None;
        }
        Some((count, Some(reference)))
    }
}

#[allow(clippy::too_many_arguments)]
fn optional_sequence_ref<T, Tag>(
    session: &mut StoreValidationSession<'_, '_>,
    directory: CanonicalRecordView<'_>,
    count_field: usize,
    presence_field: usize,
    reference_field: usize,
    expected_count: Option<u64>,
) -> Option<(u64, Option<CanonicalSequenceRef<T, Tag>>)>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    let count = directory.scalar::<u64>(0, count_field)?;
    if let Some(expected_count) = expected_count {
        require_scalar(Some(count), expected_count)?;
    }
    if count == 0 {
        require_scalar(directory.scalar::<u8>(0, presence_field), PRESENCE_ABSENT)?;
        require_scalar(directory.scalar::<u8>(0, reference_field), 0_u8)?;
        Some((count, None))
    } else {
        require_scalar(directory.scalar::<u8>(0, presence_field), PRESENCE_PRESENT)?;
        let reference = directory.sequence_ref::<T, Tag>(0, reference_field)?;
        let sequence = session.canonical_sequence(reference)?;
        if sequence.len() != usize::try_from(count).ok()? {
            return None;
        }
        Some((count, Some(reference)))
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_clip_table<'storage, R>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<AnimationClipTableSchema>>,
    channel_reference: Option<CanonicalRecordRef<AnimationChannelTableSchema>>,
    channel_table: Option<CanonicalRecordView<'storage>>,
    clip_count: u32,
    channel_count: u32,
    expected: Option<&ValidatedAnimationBatch<'_, R>>,
    mut target_marks: Option<&mut [u32]>,
    node_target_mark_count: usize,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let Some(reference) = reference else {
        if expected.is_some_and(|expected| expected.clip_count() != 0) {
            return None;
        }
        return (clip_count == 0).then_some(());
    };
    let table = session.canonical_records(reference)?;
    let mut channel_cursor = 0_u32;
    for row in 0..table.rows() {
        observe_work(1)?;
        let clip = match expected {
            Some(expected) => Some(expected.clip(row)?),
            None => None,
        };
        let clip_id = u32::try_from(row).ok()?;
        if let Some(clip) = clip.as_ref()
            && u32::try_from(clip.input_index()).ok()? != clip_id
        {
            return None;
        }
        require_scalar(
            table.scalar::<u32>(row, animation_clip_field::CLIP),
            clip_id,
        )?;
        let duration = table.scalar::<f64>(row, animation_clip_field::DURATION)?;
        if let Some(clip) = clip.as_ref() {
            require_float64_bits(Some(duration), clip.duration())?;
        }
        if !duration.is_finite() || duration < 0.0 {
            return None;
        }
        let time_domain = table.scalar::<u8>(row, animation_clip_field::TIME_DOMAIN)?;
        let expected_rate = match clip.as_ref() {
            Some(clip) => {
                let (expected_time_domain, rate) = encode_animation_time_domain(clip.time_domain());
                require_scalar(Some(time_domain), expected_time_domain)?;
                Some(rate)
            }
            None => None,
        };
        if !valid_time_domain(time_domain) {
            return None;
        }
        validate_optional_rate(table, row, time_domain, expected_rate)?;

        let expected_range = clip.as_ref().map(|clip| clip.channel_range());
        if expected_range.is_some_and(|range| range.offset() != channel_cursor) {
            return None;
        }
        let channel_offset = table.scalar::<u32>(row, animation_clip_field::CHANNEL_OFFSET)?;
        if let Some(range) = expected_range {
            require_scalar(Some(channel_offset), range.offset())?;
        }
        require_scalar(Some(channel_offset), channel_cursor)?;
        let channel_range_count = table.scalar::<u32>(row, animation_clip_field::CHANNEL_COUNT)?;
        if let Some(range) = expected_range {
            require_scalar(Some(channel_range_count), range.count())?;
        }
        let channel_end = channel_offset.checked_add(channel_range_count)?;
        if channel_end > channel_count {
            return None;
        }
        validate_optional_record_presence(
            table,
            row,
            animation_clip_field::CHANNELS_PRESENCE,
            animation_clip_field::CHANNELS,
            channel_range_count != 0,
            channel_reference,
        )?;
        if expected.is_none() && channel_range_count != 0 {
            validate_unique_clip_targets(
                channel_table?,
                channel_offset,
                channel_range_count,
                target_marks.as_deref_mut()?,
                node_target_mark_count,
                clip_id.checked_add(1)?,
                observe_work,
            )?;
        }
        channel_cursor = channel_end;
    }
    if expected.is_some_and(|expected| channel_cursor != expected.channel_count()) {
        return None;
    }
    (u32::try_from(table.rows()) == Ok(clip_count) && channel_cursor == channel_count).then_some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_channel_table<'storage, R>(
    session: &mut StoreValidationSession<'_, 'storage>,
    reference: Option<CanonicalRecordRef<AnimationChannelTableSchema>>,
    times: Option<CanonicalSequenceRef<f64, AnimationTimesSchema>>,
    values: Option<CanonicalSequenceRef<f32, AnimationValuesSchema>>,
    in_tangents: Option<CanonicalSequenceRef<f32, AnimationInTangentsSchema>>,
    out_tangents: Option<CanonicalSequenceRef<f32, AnimationOutTangentsSchema>>,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    node_rows: usize,
    deformation_mesh_instances: Option<CanonicalRecordRef<DeformationMeshInstanceTableSchema>>,
    deformation_mesh_instance_table: Option<CanonicalRecordView<'storage>>,
    canonical_target_views: Option<AnimationCanonicalTargetViews<'storage>>,
    channel_count: u32,
    time_scalar_count: u64,
    value_scalar_count: u64,
    in_tangent_scalar_count: u64,
    out_tangent_scalar_count: u64,
    nodes: CanonicalRecordView<'storage>,
    expectation: &mut AnimationValidationExpectation<'_, '_, R>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<CanonicalRecordView<'storage>>> {
    let Some(reference) = reference else {
        if expectation
            .fresh
            .is_some_and(|expected| expected.channel_count() != 0)
        {
            return None;
        }
        return (channel_count == 0).then_some(None);
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
        let channel = match expectation.fresh {
            Some(expected) => Some(expected.channel(row)?),
            None => None,
        };
        let channel_id = u32::try_from(row).ok()?;
        if channel
            .as_ref()
            .is_some_and(|channel| u32::try_from(channel.input_index()) != Ok(channel_id))
        {
            return None;
        }
        require_scalar(
            table.scalar::<u32>(row, animation_channel_field::CHANNEL),
            channel_id,
        )?;

        let target_kind = table.scalar::<u8>(row, animation_channel_field::TARGET_KIND)?;
        if let Some(channel) = channel.as_ref() {
            require_scalar(
                Some(target_kind),
                encode_animation_target_kind(channel.target()),
            )?;
        }
        let target_index = table.scalar::<u32>(row, animation_channel_field::TARGET_INDEX)?;
        if let Some(channel) = channel.as_ref() {
            require_scalar(Some(target_index), channel.canonical_target_index())?;
        }
        let target_value_width = validate_target_ref(
            table,
            row,
            target_kind,
            target_index,
            nodes_ref,
            nodes,
            node_rows,
            deformation_mesh_instances,
            deformation_mesh_instance_table,
            canonical_target_views,
            channel.as_ref().map(|channel| channel.value_width()),
        )?;

        let interpolation = table.scalar::<u8>(row, animation_channel_field::INTERPOLATION)?;
        if let Some(channel) = channel.as_ref() {
            require_scalar(
                Some(interpolation),
                encode_animation_interpolation(channel.interpolation()),
            )?;
        }
        if !valid_interpolation(interpolation) {
            return None;
        }
        let pre_extrapolation =
            table.scalar::<u8>(row, animation_channel_field::PRE_EXTRAPOLATION)?;
        if let Some(channel) = channel.as_ref() {
            require_scalar(
                Some(pre_extrapolation),
                encode_animation_extrapolation(channel.pre_extrapolation()),
            )?;
        }
        if !valid_extrapolation(pre_extrapolation) {
            return None;
        }
        let post_extrapolation =
            table.scalar::<u8>(row, animation_channel_field::POST_EXTRAPOLATION)?;
        if let Some(channel) = channel.as_ref() {
            require_scalar(
                Some(post_extrapolation),
                encode_animation_extrapolation(channel.post_extrapolation()),
            )?;
        }
        if !valid_extrapolation(post_extrapolation) {
            return None;
        }

        let expected_counts = match channel.as_ref() {
            Some(channel) => {
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
                Some((time_count, value_count))
            }
            None => None,
        };

        let key_count = table.scalar::<u32>(row, animation_channel_field::KEY_COUNT)?;
        if let Some((expected_time_count, _)) = expected_counts {
            require_scalar(Some(key_count), expected_time_count)?;
        }
        if key_count == 0 {
            return None;
        }
        let value_width = table.scalar::<u32>(row, animation_channel_field::VALUE_WIDTH)?;
        if let Some(channel) = channel.as_ref() {
            require_scalar(Some(value_width), channel.value_width())?;
        }
        require_scalar(Some(value_width), target_value_width)?;
        let time_offset = table.scalar::<u32>(row, animation_channel_field::TIME_OFFSET)?;
        if let Some(channel) = channel.as_ref() {
            require_scalar(Some(time_offset), channel.time_offset())?;
        }
        if u64::from(time_offset) != time_cursor {
            return None;
        }
        let time_count = table.scalar::<u32>(row, animation_channel_field::TIME_COUNT)?;
        if let Some((expected_time_count, _)) = expected_counts {
            require_scalar(Some(time_count), expected_time_count)?;
        }
        require_scalar(Some(time_count), key_count)?;
        require_scalar(
            table.sequence_ref::<f64, AnimationTimesSchema>(row, animation_channel_field::TIMES),
            times_ref,
        )?;
        validate_time_range(
            stored_times,
            time_offset,
            time_count,
            channel.as_ref().map(|channel| channel.times()),
            observe_work,
        )?;
        time_cursor = time_cursor.checked_add(u64::from(time_count))?;

        let value_offset = table.scalar::<u32>(row, animation_channel_field::VALUE_OFFSET)?;
        if let Some(channel) = channel.as_ref() {
            require_scalar(Some(value_offset), channel.value_offset())?;
        }
        if u64::from(value_offset) != value_cursor {
            return None;
        }
        let value_count = table.scalar::<u32>(row, animation_channel_field::VALUE_COUNT)?;
        if let Some((_, expected_value_count)) = expected_counts {
            require_scalar(Some(value_count), expected_value_count)?;
        }
        require_scalar(Some(value_count), key_count.checked_mul(value_width)?)?;
        require_scalar(
            table.sequence_ref::<f32, AnimationValuesSchema>(row, animation_channel_field::VALUES),
            values_ref,
        )?;
        let stored_value_range = validate_float32_range(
            stored_values,
            value_offset,
            value_count,
            channel.as_ref().map(|channel| channel.values()),
            observe_work,
        )?;
        if expectation.fresh.is_none() && target_kind == ANIMATION_TARGET_NODE_ROTATION {
            validate_rotation_values(stored_value_range, observe_work)?;
        }
        value_cursor = value_cursor.checked_add(u64::from(value_count))?;

        validate_optional_tangent::<AnimationInTangentsSchema>(
            table,
            row,
            animation_channel_field::IN_TANGENTS_PRESENCE,
            animation_channel_field::IN_TANGENT_OFFSET,
            animation_channel_field::IN_TANGENT_COUNT,
            animation_channel_field::IN_TANGENTS,
            interpolation,
            channel.as_ref().map(|channel| {
                (
                    channel.in_tangent_offset(),
                    channel.in_tangent_count(),
                    channel.in_tangents(),
                )
            }),
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
            interpolation,
            channel.as_ref().map(|channel| {
                (
                    channel.out_tangent_offset(),
                    channel.out_tangent_count(),
                    channel.out_tangents(),
                )
            }),
            out_tangents,
            stored_out_tangents,
            &mut out_tangent_cursor,
            value_count,
            observe_work,
        )?;
    }

    if expectation.fresh.is_some_and(|expected| {
        time_cursor != expected.time_scalar_count()
            || value_cursor != expected.value_scalar_count()
            || in_tangent_cursor != expected.in_tangent_scalar_count()
            || out_tangent_cursor != expected.out_tangent_scalar_count()
    }) {
        return None;
    }
    (u32::try_from(table.rows()) == Ok(channel_count)
        && time_cursor == time_scalar_count
        && value_cursor == value_scalar_count
        && in_tangent_cursor == in_tangent_scalar_count
        && out_tangent_cursor == out_tangent_scalar_count)
        .then_some(Some(table))
}

#[allow(clippy::too_many_arguments)]
fn validate_target_ref(
    table: CanonicalRecordView<'_>,
    row: usize,
    target_kind: u8,
    target_index: u32,
    nodes_ref: CanonicalRecordRef<NodeSceneNodeTableSchema>,
    nodes: CanonicalRecordView<'_>,
    node_rows: usize,
    deformation_mesh_instances: Option<CanonicalRecordRef<DeformationMeshInstanceTableSchema>>,
    deformation_mesh_instance_table: Option<CanonicalRecordView<'_>>,
    canonical_target_views: Option<AnimationCanonicalTargetViews<'_>>,
    fresh_value_width: Option<u32>,
) -> Option<u32> {
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
            )?;
            if let Some(fresh_value_width) = fresh_value_width {
                return Some(fresh_value_width);
            }
            require_scalar(
                nodes.scalar::<u8>(target_index, node_scene_node_field::TRANSFORM_KIND),
                NODE_SCENE_TRANSFORM_TRS,
            )?;
            Some(if target_kind == ANIMATION_TARGET_NODE_ROTATION {
                4
            } else {
                3
            })
        }
        ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS => {
            let deformation_mesh_instance_table = deformation_mesh_instance_table?;
            if target_index >= deformation_mesh_instance_table.rows() {
                return None;
            }
            require_scalar(
                table.record_ref::<DeformationMeshInstanceTableSchema>(
                    row,
                    animation_channel_field::TARGETS,
                ),
                deformation_mesh_instances?,
            )?;
            if let Some(fresh_value_width) = fresh_value_width {
                return Some(fresh_value_width);
            }
            let target_views = canonical_target_views?;
            let logical_mesh = target_views
                .geometry_mesh_instances?
                .scalar::<u32>(target_index, geometry_mesh_instance_field::LOGICAL_MESH)?;
            let logical_mesh = usize::try_from(logical_mesh).ok()?;
            let logical_morphs = target_views.logical_morphs?;
            if logical_mesh >= logical_morphs.rows() {
                return None;
            }
            let width = logical_morphs.scalar::<u32>(
                logical_mesh,
                deformation_logical_morph_field::DEFINITION_COUNT,
            )?;
            (width != 0).then_some(width)
        }
        _ => None,
    }
}

fn validate_optional_rate(
    table: CanonicalRecordView<'_>,
    row: usize,
    time_domain: u8,
    expected: Option<Presence<f64>>,
) -> Option<()> {
    let rate = match expected {
        Some(Presence::Absent) => {
            require_scalar(
                table.scalar::<u8>(row, animation_clip_field::RATE_PRESENCE),
                PRESENCE_ABSENT,
            )?;
            require_scalar(table.scalar::<u8>(row, animation_clip_field::RATE), 0_u8)?;
            None
        }
        Some(Presence::Present(expected_rate)) => {
            require_scalar(
                table.scalar::<u8>(row, animation_clip_field::RATE_PRESENCE),
                PRESENCE_PRESENT,
            )?;
            let rate = table.scalar::<f64>(row, animation_clip_field::RATE)?;
            require_float64_bits(Some(rate), expected_rate)?;
            Some(rate)
        }
        None => match table.scalar::<u8>(row, animation_clip_field::RATE_PRESENCE)? {
            PRESENCE_ABSENT => {
                require_scalar(table.scalar::<u8>(row, animation_clip_field::RATE), 0_u8)?;
                None
            }
            PRESENCE_PRESENT => Some(table.scalar::<f64>(row, animation_clip_field::RATE)?),
            _ => return None,
        },
    };
    if rate.is_some_and(|rate| !rate.is_finite() || rate <= 0.0) {
        return None;
    }
    match time_domain {
        ANIMATION_TIME_DOMAIN_SECONDS => rate.is_none().then_some(()),
        ANIMATION_TIME_DOMAIN_FRAMES => rate.is_some().then_some(()),
        ANIMATION_TIME_DOMAIN_TICKS => Some(()),
        _ => None,
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

type ExpectedOptionalTangent<'a> = (Presence<u32>, u32, Presence<&'a [f32]>);

#[allow(clippy::too_many_arguments)]
fn validate_optional_tangent<Tag>(
    table: CanonicalRecordView<'_>,
    row: usize,
    presence_field: usize,
    offset_field: usize,
    count_field: usize,
    reference_field: usize,
    interpolation: u8,
    expected: Option<ExpectedOptionalTangent<'_>>,
    expected_reference: Option<CanonicalSequenceRef<f32, Tag>>,
    stored_values: Option<&[f32]>,
    cursor: &mut u64,
    expected_present_count: u32,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<f32>>,
{
    if let Some((expected_offset, expected_count, expected_values)) = expected {
        return match (expected_offset, expected_values) {
            (Presence::Absent, Presence::Absent) if expected_count == 0 => {
                if interpolation == ANIMATION_INTERPOLATION_CUBIC_SPLINE {
                    return None;
                }
                require_scalar(table.scalar::<u8>(row, presence_field), PRESENCE_ABSENT)?;
                require_scalar(table.scalar::<u32>(row, offset_field), 0_u32)?;
                require_scalar(table.scalar::<u32>(row, count_field), 0_u32)?;
                require_scalar(table.scalar::<u8>(row, reference_field), 0_u8)
            }
            (Presence::Present(offset), Presence::Present(values))
                if expected_count != 0 && expected_count == expected_present_count =>
            {
                if interpolation != ANIMATION_INTERPOLATION_CUBIC_SPLINE
                    || u64::from(offset) != *cursor
                    || u32::try_from(values.len()).ok()? != expected_count
                {
                    return None;
                }
                require_scalar(table.scalar::<u8>(row, presence_field), PRESENCE_PRESENT)?;
                require_scalar(table.scalar::<u32>(row, offset_field), offset)?;
                require_scalar(table.scalar::<u32>(row, count_field), expected_count)?;
                require_scalar(
                    table.sequence_ref::<f32, Tag>(row, reference_field),
                    expected_reference?,
                )?;
                validate_float32_range(
                    stored_values?,
                    offset,
                    expected_count,
                    Some(values),
                    observe_work,
                )?;
                *cursor = (*cursor).checked_add(u64::from(expected_count))?;
                Some(())
            }
            _ => None,
        };
    }

    match table.scalar::<u8>(row, presence_field)? {
        PRESENCE_ABSENT if interpolation != ANIMATION_INTERPOLATION_CUBIC_SPLINE => {
            require_scalar(table.scalar::<u8>(row, presence_field), PRESENCE_ABSENT)?;
            require_scalar(table.scalar::<u32>(row, offset_field), 0_u32)?;
            require_scalar(table.scalar::<u32>(row, count_field), 0_u32)?;
            require_scalar(table.scalar::<u8>(row, reference_field), 0_u8)
        }
        PRESENCE_PRESENT if interpolation == ANIMATION_INTERPOLATION_CUBIC_SPLINE => {
            let offset = table.scalar::<u32>(row, offset_field)?;
            let count = table.scalar::<u32>(row, count_field)?;
            if count == 0 || count != expected_present_count || u64::from(offset) != *cursor {
                return None;
            }
            require_scalar(
                table.sequence_ref::<f32, Tag>(row, reference_field),
                expected_reference?,
            )?;
            validate_float32_range(stored_values?, offset, count, None, observe_work)?;
            *cursor = (*cursor).checked_add(u64::from(count))?;
            Some(())
        }
        _ => None,
    }
}

fn validate_time_range(
    stored: &[f64],
    offset: u32,
    count: u32,
    expected: Option<&[f64]>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let range = checked_range(offset, count, stored.len())?;
    let actual = stored.get(range)?;
    if expected.is_some_and(|expected| actual.len() != expected.len()) {
        return None;
    }
    let mut previous = None;
    for (index, &actual) in actual.iter().enumerate() {
        observe_work(1)?;
        if let Some(expected) = expected {
            require_scalar(Some(actual.to_bits()), expected.get(index)?.to_bits())?;
        }
        if !actual.is_finite() || previous.is_some_and(|previous| previous >= actual) {
            return None;
        }
        previous = Some(actual);
    }
    Some(())
}

fn validate_float32_range<'stored>(
    stored: &'stored [f32],
    offset: u32,
    count: u32,
    expected: Option<&[f32]>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<&'stored [f32]> {
    let range = checked_range(offset, count, stored.len())?;
    let actual = stored.get(range)?;
    if expected.is_some_and(|expected| actual.len() != expected.len()) {
        return None;
    }
    for (index, &actual) in actual.iter().enumerate() {
        observe_work(1)?;
        if let Some(expected) = expected {
            require_scalar(Some(actual.to_bits()), expected.get(index)?.to_bits())?;
        }
        if !actual.is_finite() {
            return None;
        }
    }
    Some(actual)
}

fn validate_rotation_values(
    values: &[f32],
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let mut rotations = values.chunks_exact(4);
    for components in &mut rotations {
        observe_work(1)?;
        let rotation = Quat::from_xyzw(components[0], components[1], components[2], components[3]);
        if !rotation.is_unit() {
            return None;
        }
    }
    rotations.remainder().is_empty().then_some(())
}

fn validate_unique_clip_targets(
    channels: CanonicalRecordView<'_>,
    offset: u32,
    count: u32,
    target_marks: &mut [u32],
    node_target_mark_count: usize,
    clip_marker: u32,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    for channel in checked_range(offset, count, channels.rows())? {
        observe_work(1)?;
        let target_kind = channels.scalar::<u8>(channel, animation_channel_field::TARGET_KIND)?;
        let target_index = usize::try_from(
            channels.scalar::<u32>(channel, animation_channel_field::TARGET_INDEX)?,
        )
        .ok()?;
        let mark_index = match target_kind {
            ANIMATION_TARGET_NODE_TRANSLATION => target_index.checked_mul(3)?,
            ANIMATION_TARGET_NODE_ROTATION => target_index.checked_mul(3)?.checked_add(1)?,
            ANIMATION_TARGET_NODE_SCALE => target_index.checked_mul(3)?.checked_add(2)?,
            ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS => {
                node_target_mark_count.checked_add(target_index)?
            }
            _ => return None,
        };
        let mark = target_marks.get_mut(mark_index)?;
        if *mark == clip_marker {
            return None;
        }
        *mark = clip_marker;
    }
    Some(())
}

const fn valid_time_domain(value: u8) -> bool {
    matches!(
        value,
        ANIMATION_TIME_DOMAIN_SECONDS | ANIMATION_TIME_DOMAIN_FRAMES | ANIMATION_TIME_DOMAIN_TICKS
    )
}

const fn valid_interpolation(value: u8) -> bool {
    matches!(
        value,
        ANIMATION_INTERPOLATION_STEP
            | ANIMATION_INTERPOLATION_LINEAR
            | ANIMATION_INTERPOLATION_CUBIC_SPLINE
    )
}

const fn valid_extrapolation(value: u8) -> bool {
    matches!(
        value,
        ANIMATION_EXTRAPOLATION_SOURCE_DEFAULT
            | ANIMATION_EXTRAPOLATION_CONSTANT
            | ANIMATION_EXTRAPOLATION_LINEAR
            | ANIMATION_EXTRAPOLATION_REPEAT
    )
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
