use baozi_operation::__retained_store_integration::activation::{
    CanonicalValidationMember, CanonicalValidationSession,
};
use baozi_operation::{
    OperationOwner, ProgressPhase, ProgressUnit, RecordBlock, ScalarSequence, StoreWorkError,
    Utf8Block,
};

use crate::canonical_scene::store::schema::animation::{
    ANIMATION_CHANNEL_FIELD_COUNT, ANIMATION_CLIP_FIELD_COUNT, ANIMATION_DIRECTORY_FIELD_COUNT,
    ANIMATION_EXTRAPOLATION_CONSTANT, ANIMATION_EXTRAPOLATION_LINEAR,
    ANIMATION_EXTRAPOLATION_REPEAT, ANIMATION_EXTRAPOLATION_SOURCE_DEFAULT,
    ANIMATION_INTERPOLATION_CUBIC_SPLINE, ANIMATION_INTERPOLATION_LINEAR,
    ANIMATION_INTERPOLATION_STEP, ANIMATION_SCHEMA_REVISION,
    ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS, ANIMATION_TARGET_NODE_ROTATION,
    ANIMATION_TARGET_NODE_SCALE, ANIMATION_TARGET_NODE_TRANSLATION, ANIMATION_TIME_DOMAIN_FRAMES,
    ANIMATION_TIME_DOMAIN_SECONDS, ANIMATION_TIME_DOMAIN_TICKS, AnimationChannelTableSchema,
    AnimationClipNameSchema, AnimationClipTableSchema, AnimationDirectorySchema,
    AnimationInTangentsSchema, AnimationOutTangentsSchema, AnimationTimesSchema,
    AnimationValuesSchema, animation_channel_field, animation_clip_field,
    animation_directory_field,
};
use crate::canonical_scene::store::schema::deformation::{
    DeformationLogicalMorphTableSchema, DeformationMeshInstanceTableSchema,
};
use crate::canonical_scene::store::schema::{CanonicalBlockRef, PRESENCE_ABSENT, PRESENCE_PRESENT};
use crate::canonical_scene::store::typed_storage::{
    NodeStorage, StoredMeshInstance, StoredTransform,
};
use crate::entity::{MAX_ENTITY_NAME_BYTES, validate_entity_name_str};
use crate::math::Quat;

use super::deformation::DeformationLogicalMorphValidationProjection;

#[derive(Clone, Copy)]
pub(super) struct AnimationValidatedRefs {
    pub(super) clip_names: Option<CanonicalBlockRef<AnimationClipNameSchema>>,
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

/// Validation-scoped animation members retained for same-session cross-domain checks.
#[derive(Clone, Copy)]
pub(super) struct AnimationValidatedMembers<'session> {
    pub(super) clip_names:
        Option<CanonicalValidationMember<'session, Utf8Block, AnimationClipNameSchema>>,
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
    pub(in crate::canonical_scene::store) clip_name_byte_count: u32,
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
        clip_name_byte_count: u32,
    ) -> Self {
        Self {
            clip_name_byte_count,
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
    name_count: u32,
}

impl AnimationClipValidationDescriptor {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        channel_end: 0,
        name_count: 0,
    };
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
    nodes: &'projection NodeStorage,
    node_count: u32,
    deformation_mesh_instances: Option<
        CanonicalValidationMember<'session, RecordBlock, DeformationMeshInstanceTableSchema>,
    >,
    deformation_mesh_instance_count: u32,
    geometry_mesh_instance_count: u32,
    logical_morphs: Option<
        CanonicalValidationMember<'session, RecordBlock, DeformationLogicalMorphTableSchema>,
    >,
    logical_morph_count: u32,
    geometry_mesh_instance_projections: &'projection [StoredMeshInstance],
    logical_morph_projections: &'projection [DeformationLogicalMorphValidationProjection],
}

impl<'session, 'projection> AnimationMeteredDependencies<'session, 'projection> {
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        nodes: &'projection NodeStorage,
        node_count: u32,
        deformation_mesh_instances: Option<
            CanonicalValidationMember<'session, RecordBlock, DeformationMeshInstanceTableSchema>,
        >,
        deformation_mesh_instance_count: u32,
        geometry_mesh_instance_count: u32,
        logical_morphs: Option<
            CanonicalValidationMember<'session, RecordBlock, DeformationLogicalMorphTableSchema>,
        >,
        logical_morph_count: u32,
        geometry_mesh_instance_projections: &'projection [StoredMeshInstance],
        logical_morph_projections: &'projection [DeformationLogicalMorphValidationProjection],
    ) -> Self {
        Self {
            nodes,
            node_count,
            deformation_mesh_instances,
            deformation_mesh_instance_count,
            geometry_mesh_instance_count,
            logical_morphs,
            logical_morph_count,
            geometry_mesh_instance_projections,
            logical_morph_projections,
        }
    }
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
        || !validate_animation_clip_names_metered(
            session,
            members.clip_names,
            counts.clip_name_byte_count,
            clip_descriptors,
        )?
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
        members.clip_names.is_some(),
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
            .is_ok_and(|count| dependencies.nodes.nodes.len() == count)
        && dependencies.deformation_mesh_instances.is_some()
            == (dependencies.deformation_mesh_instance_count != 0)
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
    let mut clip_names = None;
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

            let clip_name_byte_count = row.scalar(
                animation_directory_field::CLIP_NAME_BYTE_COUNT,
                |value: u32| value,
            )?;
            let clip_names_presence = row.scalar(
                animation_directory_field::CLIP_NAMES_PRESENCE,
                |value: u8| value,
            )?;
            let clip_names_slot_valid = if counts.clip_name_byte_count == 0 {
                row.scalar(animation_directory_field::CLIP_NAMES, |value: u8| {
                    value == 0
                })? == Some(true)
            } else {
                clip_names = row.utf8_ref::<AnimationClipNameSchema, _>(
                    animation_directory_field::CLIP_NAMES,
                    |value| value,
                )?;
                clip_names.is_some()
            };

            Ok(schema_revision == Some(ANIMATION_SCHEMA_REVISION)
                && clip_name_byte_count == Some(counts.clip_name_byte_count)
                && clip_names_presence
                    == Some(if counts.clip_name_byte_count == 0 {
                        PRESENCE_ABSENT
                    } else {
                        PRESENCE_PRESENT
                    })
                && clip_names_slot_valid
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
        clip_names,
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
    let mut previous_source_channel_index = None;
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
                    row.scalar(animation_channel_field::TARGET_CONTEXT, |value: u32| {
                        value as usize == dependencies.nodes.nodes.len()
                    })? == Some(true)
                }
                Some(ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS) => {
                    row.record_ref::<DeformationMeshInstanceTableSchema, _>(
                        animation_channel_field::TARGET_CONTEXT,
                        |value| Some(value) == dependencies.deformation_mesh_instances,
                    )? == Some(true)
                }
                _ => {
                    row.scalar(animation_channel_field::TARGET_CONTEXT, |value: u8| {
                        value == 0
                    })? == Some(true)
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
            let Some(source_channel_index) = channel else {
                return Ok(false);
            };
            if previous_source_channel_index
                .is_some_and(|previous| source_channel_index <= previous)
            {
                return Ok(false);
            }
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

            let valid = target_reference_matches
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
            previous_source_channel_index = Some(source_channel_index);
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
                || !matches!(
                    dependencies.nodes.nodes.get(target_index)?.transform,
                    StoredTransform::Trs { .. }
                )
            {
                return None;
            }
            Some(3)
        }
        ANIMATION_TARGET_NODE_ROTATION => {
            if target_index == 0
                || target_index >= usize::try_from(dependencies.node_count).ok()?
                || !matches!(
                    dependencies.nodes.nodes.get(target_index)?.transform,
                    StoredTransform::Trs { .. }
                )
            {
                return None;
            }
            Some(4)
        }
        ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS => {
            if dependencies.deformation_mesh_instances.is_none()
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
                    .logical_mesh
                    .0,
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
    let mut name_cursor = 0_u32;
    let mut previous_source_clip_index = None;
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
            let name_presence =
                row.scalar(animation_clip_field::NAME_PRESENCE, |value: u8| value)?;
            let name_offset = row.scalar(animation_clip_field::NAME_OFFSET, |value: u32| value)?;
            let name_count = row.scalar(animation_clip_field::NAME_COUNT, |value: u32| value)?;
            let name_reference_matches = if name_count.is_some_and(|count| count != 0) {
                row.utf8_ref::<AnimationClipNameSchema, _>(animation_clip_field::NAMES, |value| {
                    Some(value) == members.clip_names
                })? == Some(true)
            } else {
                row.scalar(animation_clip_field::NAMES, |value: u8| value == 0)? == Some(true)
            };
            let Some(name_count) = name_count else {
                return Ok(false);
            };
            let Some(name_end) = name_cursor.checked_add(name_count) else {
                return Ok(false);
            };
            let name_valid = name_offset == Some(name_cursor)
                && name_end <= counts.clip_name_byte_count
                && usize::try_from(name_count).is_ok_and(|count| count <= MAX_ENTITY_NAME_BYTES)
                && matches!(
                    (name_presence, name_count),
                    (Some(PRESENCE_ABSENT), 0) | (Some(PRESENCE_PRESENT), _)
                )
                && name_reference_matches;

            let Some(source_clip_index) = clip else {
                return Ok(false);
            };
            if previous_source_clip_index.is_some_and(|previous| source_clip_index <= previous) {
                return Ok(false);
            }
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
            let valid = duration.is_finite()
                && name_valid
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
            *descriptor = AnimationClipValidationDescriptor {
                channel_end,
                name_count,
            };
            name_cursor = name_end;
            channel_cursor = channel_end;
            previous_source_clip_index = Some(source_clip_index);
            Ok(true)
        },
    )?;
    Ok(complete
        && channel_cursor == counts.channel_count
        && name_cursor == counts.clip_name_byte_count)
}

fn validate_animation_clip_names_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    names: Option<CanonicalValidationMember<'session, Utf8Block, AnimationClipNameSchema>>,
    expected_len: u32,
    descriptors: &[AnimationClipValidationDescriptor],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(names) = names else {
        return Ok(expected_len == 0 && descriptors.iter().all(|clip| clip.name_count == 0));
    };
    let Ok(expected_len) = usize::try_from(expected_len) else {
        return Ok(false);
    };
    let mut clip_index = 0;
    let mut segment = [0_u8; MAX_ENTITY_NAME_BYTES];
    let mut used = 0;
    let complete = session.visit_canonical_utf8_bytes(names, expected_len, |_index, byte| {
        while descriptors
            .get(clip_index)
            .is_some_and(|clip| clip.name_count == 0)
        {
            clip_index += 1;
        }
        let Some(clip) = descriptors.get(clip_index) else {
            return false;
        };
        let Ok(count) = usize::try_from(clip.name_count) else {
            return false;
        };
        if used >= count || count > segment.len() {
            return false;
        }
        segment[used] = byte;
        used += 1;
        if used != count {
            return true;
        }
        let valid = std::str::from_utf8(&segment[..used])
            .is_ok_and(|name| validate_entity_name_str(name).is_ok());
        used = 0;
        clip_index += 1;
        valid
    })?;
    Ok(complete
        && used == 0
        && descriptors[clip_index..]
            .iter()
            .all(|clip| clip.name_count == 0))
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

fn checked_range(offset: u32, count: u32, len: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(offset).ok()?;
    let count = usize::try_from(count).ok()?;
    let end = start.checked_add(count)?;
    (end <= len).then_some(start..end)
}
