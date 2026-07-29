//! Private canonical Scene animation schema.

use baozi_operation::{RecordBlock, ScalarSequence};

use super::CanonicalSchema;
use crate::canonical_scene::store::production::{
    AnimationExtrapolationAuthoring, AnimationInterpolationAuthoring, AnimationTargetAuthoring,
    AnimationTimeDomainAuthoring, Presence,
};

pub(in super::super) const ANIMATION_SCHEMA_REVISION: u32 = 1;
pub(in super::super) const ANIMATION_DIRECTORY_FIELD_COUNT: usize = 19;
pub(in super::super) const ANIMATION_CLIP_FIELD_COUNT: usize = 9;
pub(in super::super) const ANIMATION_CHANNEL_FIELD_COUNT: usize = 23;

pub(in super::super) const ANIMATION_TIME_DOMAIN_SECONDS: u8 = 0;
pub(in super::super) const ANIMATION_TIME_DOMAIN_FRAMES: u8 = 1;
pub(in super::super) const ANIMATION_TIME_DOMAIN_TICKS: u8 = 2;

pub(in super::super) const ANIMATION_INTERPOLATION_STEP: u8 = 0;
pub(in super::super) const ANIMATION_INTERPOLATION_LINEAR: u8 = 1;
pub(in super::super) const ANIMATION_INTERPOLATION_CUBIC_SPLINE: u8 = 2;

pub(in super::super) const ANIMATION_EXTRAPOLATION_SOURCE_DEFAULT: u8 = 0;
pub(in super::super) const ANIMATION_EXTRAPOLATION_CONSTANT: u8 = 1;
pub(in super::super) const ANIMATION_EXTRAPOLATION_LINEAR: u8 = 2;
pub(in super::super) const ANIMATION_EXTRAPOLATION_REPEAT: u8 = 3;

pub(in super::super) const ANIMATION_TARGET_NODE_TRANSLATION: u8 = 0;
pub(in super::super) const ANIMATION_TARGET_NODE_ROTATION: u8 = 1;
pub(in super::super) const ANIMATION_TARGET_NODE_SCALE: u8 = 2;
pub(in super::super) const ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS: u8 = 3;

pub(in super::super) const fn encode_animation_time_domain(
    domain: AnimationTimeDomainAuthoring,
) -> (u8, Presence<f64>) {
    match domain {
        AnimationTimeDomainAuthoring::Seconds => (ANIMATION_TIME_DOMAIN_SECONDS, Presence::Absent),
        AnimationTimeDomainAuthoring::Frames { rate } => {
            (ANIMATION_TIME_DOMAIN_FRAMES, Presence::Present(rate))
        }
        AnimationTimeDomainAuthoring::Ticks { rate: None } => {
            (ANIMATION_TIME_DOMAIN_TICKS, Presence::Absent)
        }
        AnimationTimeDomainAuthoring::Ticks { rate: Some(rate) } => {
            (ANIMATION_TIME_DOMAIN_TICKS, Presence::Present(rate))
        }
    }
}

pub(in super::super) const fn encode_animation_interpolation(
    interpolation: AnimationInterpolationAuthoring,
) -> u8 {
    match interpolation {
        AnimationInterpolationAuthoring::Step => ANIMATION_INTERPOLATION_STEP,
        AnimationInterpolationAuthoring::Linear => ANIMATION_INTERPOLATION_LINEAR,
        AnimationInterpolationAuthoring::CubicSpline => ANIMATION_INTERPOLATION_CUBIC_SPLINE,
    }
}

pub(in super::super) const fn encode_animation_extrapolation(
    extrapolation: AnimationExtrapolationAuthoring,
) -> u8 {
    match extrapolation {
        AnimationExtrapolationAuthoring::SourceDefault => ANIMATION_EXTRAPOLATION_SOURCE_DEFAULT,
        AnimationExtrapolationAuthoring::Constant => ANIMATION_EXTRAPOLATION_CONSTANT,
        AnimationExtrapolationAuthoring::Linear => ANIMATION_EXTRAPOLATION_LINEAR,
        AnimationExtrapolationAuthoring::Repeat => ANIMATION_EXTRAPOLATION_REPEAT,
    }
}

pub(in super::super) const fn encode_animation_target_kind(target: AnimationTargetAuthoring) -> u8 {
    match target {
        AnimationTargetAuthoring::NodeTranslation(_) => ANIMATION_TARGET_NODE_TRANSLATION,
        AnimationTargetAuthoring::NodeRotation(_) => ANIMATION_TARGET_NODE_ROTATION,
        AnimationTargetAuthoring::NodeScale(_) => ANIMATION_TARGET_NODE_SCALE,
        AnimationTargetAuthoring::MeshInstanceMorphWeights(_) => {
            ANIMATION_TARGET_MESH_INSTANCE_MORPH_WEIGHTS
        }
    }
}

pub(in super::super) mod animation_directory_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const CLIP_COUNT: usize = 1;
    pub(crate) const CLIPS_PRESENCE: usize = 2;
    pub(crate) const CLIPS: usize = 3;
    pub(crate) const CHANNEL_COUNT: usize = 4;
    pub(crate) const CHANNELS_PRESENCE: usize = 5;
    pub(crate) const CHANNELS: usize = 6;
    pub(crate) const TIME_SCALAR_COUNT: usize = 7;
    pub(crate) const TIMES_PRESENCE: usize = 8;
    pub(crate) const TIMES: usize = 9;
    pub(crate) const VALUE_SCALAR_COUNT: usize = 10;
    pub(crate) const VALUES_PRESENCE: usize = 11;
    pub(crate) const VALUES: usize = 12;
    pub(crate) const IN_TANGENT_SCALAR_COUNT: usize = 13;
    pub(crate) const IN_TANGENTS_PRESENCE: usize = 14;
    pub(crate) const IN_TANGENTS: usize = 15;
    pub(crate) const OUT_TANGENT_SCALAR_COUNT: usize = 16;
    pub(crate) const OUT_TANGENTS_PRESENCE: usize = 17;
    pub(crate) const OUT_TANGENTS: usize = 18;
}

pub(in super::super) mod animation_clip_field {
    pub(crate) const CLIP: usize = 0;
    pub(crate) const DURATION: usize = 1;
    pub(crate) const TIME_DOMAIN: usize = 2;
    pub(crate) const RATE_PRESENCE: usize = 3;
    pub(crate) const RATE: usize = 4;
    pub(crate) const CHANNEL_OFFSET: usize = 5;
    pub(crate) const CHANNEL_COUNT: usize = 6;
    pub(crate) const CHANNELS_PRESENCE: usize = 7;
    pub(crate) const CHANNELS: usize = 8;
}

pub(in super::super) mod animation_channel_field {
    pub(crate) const CHANNEL: usize = 0;
    pub(crate) const TARGET_KIND: usize = 1;
    pub(crate) const TARGET_INDEX: usize = 2;
    pub(crate) const TARGETS: usize = 3;
    pub(crate) const INTERPOLATION: usize = 4;
    pub(crate) const PRE_EXTRAPOLATION: usize = 5;
    pub(crate) const POST_EXTRAPOLATION: usize = 6;
    pub(crate) const KEY_COUNT: usize = 7;
    pub(crate) const VALUE_WIDTH: usize = 8;
    pub(crate) const TIME_OFFSET: usize = 9;
    pub(crate) const TIME_COUNT: usize = 10;
    pub(crate) const TIMES: usize = 11;
    pub(crate) const VALUE_OFFSET: usize = 12;
    pub(crate) const VALUE_COUNT: usize = 13;
    pub(crate) const VALUES: usize = 14;
    pub(crate) const IN_TANGENTS_PRESENCE: usize = 15;
    pub(crate) const IN_TANGENT_OFFSET: usize = 16;
    pub(crate) const IN_TANGENT_COUNT: usize = 17;
    pub(crate) const IN_TANGENTS: usize = 18;
    pub(crate) const OUT_TANGENTS_PRESENCE: usize = 19;
    pub(crate) const OUT_TANGENT_OFFSET: usize = 20;
    pub(crate) const OUT_TANGENT_COUNT: usize = 21;
    pub(crate) const OUT_TANGENTS: usize = 22;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AnimationDirectorySchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AnimationClipTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AnimationChannelTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AnimationTimesSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AnimationValuesSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AnimationInTangentsSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct AnimationOutTangentsSchema;

impl CanonicalSchema for AnimationDirectorySchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for AnimationClipTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for AnimationChannelTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for AnimationTimesSchema {
    type StoreKind = ScalarSequence<f64>;
}

impl CanonicalSchema for AnimationValuesSchema {
    type StoreKind = ScalarSequence<f32>;
}

impl CanonicalSchema for AnimationInTangentsSchema {
    type StoreKind = ScalarSequence<f32>;
}

impl CanonicalSchema for AnimationOutTangentsSchema {
    type StoreKind = ScalarSequence<f32>;
}
