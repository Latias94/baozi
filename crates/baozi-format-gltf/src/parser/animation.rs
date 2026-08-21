use super::{
    AccessorContract, AccessorOwner, BufferData, ContentError, PARSER_POLL_INTERVAL, Result,
    authoring_u32, finish_gltf_operation, gltf_parse_error, safe_gltf, to_u64, try_vec_capacity,
    try_vec_filled, validate_accessor,
};
use baozi_import::authoring::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationExtrapolationAuthoring, AnimationInterpolationAuthoring, AnimationTargetAuthoring,
    AnimationTimeDomainAuthoring, AuthoringRange, NodeLocalId, Presence, Quat,
};
use baozi_import::{
    DiagnosticCode, ImportStorageClass, ImportWorkClass, ImportWorkScope, MeteredVec, ParseScope,
    SourceScopeEntityKind,
};
use gltf::animation::util::ReadOutputs;
use gltf::json;
use gltf::json::validation::Checked;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeAnimationProperty {
    Translation,
    Rotation,
    Scale,
}

impl NodeAnimationProperty {
    const fn target_slot(self) -> usize {
        match self {
            Self::Translation => 0,
            Self::Rotation => 1,
            Self::Scale => 2,
        }
    }

    const fn value_width(self) -> usize {
        match self {
            Self::Translation | Self::Scale => 3,
            Self::Rotation => 4,
        }
    }

    const fn target(self, node: NodeLocalId) -> AnimationTargetAuthoring {
        match self {
            Self::Translation => AnimationTargetAuthoring::NodeTranslation(node),
            Self::Rotation => AnimationTargetAuthoring::NodeRotation(node),
            Self::Scale => AnimationTargetAuthoring::NodeScale(node),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AnimationClipContract {
    source_index: usize,
    materialization_offset: usize,
    materialization_count: usize,
    retained_channel_count: usize,
}

#[derive(Debug, Clone)]
struct AnimationChannelContract<'document> {
    source: gltf::animation::Channel<'document>,
    source_index: usize,
    animation_index: usize,
    channel_index: usize,
    source_node_index: usize,
    property: NodeAnimationProperty,
    interpolation: AnimationInterpolationAuthoring,
    key_count: usize,
    output_count: usize,
}

#[derive(Debug, Clone)]
struct AnimationDurationContract<'document> {
    source: gltf::animation::Channel<'document>,
    animation_index: usize,
    channel_index: usize,
    key_count: usize,
}

#[derive(Debug, Clone)]
enum AnimationMaterializationContract<'document> {
    Retained(AnimationChannelContract<'document>),
    DurationOnly(AnimationDurationContract<'document>),
}

pub(super) struct AnimationContracts<'scope, 'document> {
    clips: MeteredVec<'scope, AnimationClipContract>,
    materialization_channels: MeteredVec<'scope, AnimationMaterializationContract<'document>>,
    pub(super) selected_animations: MeteredVec<'scope, bool>,
    pub(super) source_channels: MeteredVec<'scope, SourceChannelDisposition>,
    retained_channel_count: usize,
    total_cells: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceChannelDisposition {
    Retained,
    ScopeOmitted,
    SemanticLoss,
}

impl SourceChannelDisposition {
    pub(super) const fn is_retained(self) -> bool {
        matches!(self, Self::Retained)
    }
}

#[derive(Debug, Clone, Copy)]
struct AnimationClipStorage {
    source_index: usize,
    duration: f64,
    channel_offset: usize,
    channel_count: usize,
}

struct AnimationChannelStorage<'scope> {
    source_index: usize,
    target: AnimationTargetAuthoring,
    interpolation: AnimationInterpolationAuthoring,
    times: MeteredVec<'scope, f64>,
    values: MeteredVec<'scope, f32>,
    in_tangents: MeteredVec<'scope, f32>,
    out_tangents: MeteredVec<'scope, f32>,
}

pub(super) struct AnimationStorage<'scope> {
    clips: MeteredVec<'scope, AnimationClipStorage>,
    channels: MeteredVec<'scope, AnimationChannelStorage<'scope>>,
}

pub(super) fn preflight<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    gltf: &'document gltf::Gltf,
    selected_nodes: &[bool],
) -> Result<AnimationContracts<'scope, 'document>> {
    safe_gltf(ctx, "glTF animation preflight", || {
        preflight_inner(ctx, gltf, selected_nodes)
    })?
}

fn preflight_inner<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    gltf: &'document gltf::Gltf,
    selected_nodes: &[bool],
) -> Result<AnimationContracts<'scope, 'document>> {
    let source_animations = &gltf.as_json().animations;
    let source_channel_count = source_animations
        .iter()
        .try_fold(0usize, |total, animation| {
            total.checked_add(animation.channels.len()).ok_or_else(|| {
                ContentError::from(ctx.allocation_failure("gltf-animation-channel-count"))
            })
        })?;
    let node_target_count = selected_nodes
        .len()
        .checked_mul(3)
        .ok_or_else(|| ctx.allocation_failure("gltf-animation-target-count"))?;
    let mut seen_targets = try_vec_filled(
        ctx,
        ImportStorageClass::FormatNormalization,
        node_target_count,
        usize::MAX,
        "gltf-animation-seen-targets",
    )?;

    ctx.with_work(
        ImportWorkClass::WorkUnits,
        to_u64(source_channel_count),
        |work| -> Result<_> {
            let mut clips = try_vec_capacity(
                ctx,
                ImportStorageClass::FormatNormalization,
                source_animations.len(),
                "gltf-animation-contract-clips",
            )?;
            let mut materialization_channels = try_vec_capacity(
                ctx,
                ImportStorageClass::FormatNormalization,
                source_channel_count,
                "gltf-animation-materialization-channels",
            )?;
            let mut selected_animations = try_vec_capacity(
                ctx,
                ImportStorageClass::FormatNormalization,
                source_animations.len(),
                "gltf-animation-selection",
            )?;
            let mut source_channels = try_vec_capacity(
                ctx,
                ImportStorageClass::FormatNormalization,
                source_channel_count,
                "gltf-animation-channel-selection",
            )?;
            let mut total_cells = 0usize;
            let mut retained_channel_count = 0usize;
            let mut visited_channels = 0usize;

            for (animation_index, animation) in gltf.animations().enumerate() {
                let source_animation = source_animations.get(animation_index).ok_or_else(|| {
                    gltf_parse_error(
                        ctx,
                        format!("glTF animation {animation_index} is out of range"),
                    )
                })?;
                let materialization_offset = materialization_channels.len();
                let source_channel_offset = source_channels.len();
                let mut runtime_channel_count = 0usize;
                let mut animation_selected = false;
                let mut animation_retained_channel_count = 0usize;
                let mut animation_cells = 0usize;

                for (channel_index, runtime_channel) in animation.channels().enumerate() {
                    if visited_channels.is_multiple_of(PARSER_POLL_INTERVAL) {
                        work.poll()?;
                    }
                    visited_channels = visited_channels.checked_add(1).ok_or_else(|| {
                        ctx.allocation_failure("gltf-animation-channel-progress")
                    })?;
                    runtime_channel_count = runtime_channel_count.checked_add(1).ok_or_else(|| {
                        ctx.allocation_failure("gltf-animation-runtime-channel-count")
                    })?;
                    let source_channel = source_animation.channels.get(channel_index).ok_or_else(|| {
                        gltf_parse_error(
                            ctx,
                            format!(
                                "glTF animation {animation_index} channel {channel_index} is out of range"
                            ),
                        )
                    })?;
                    let source_node_index = source_channel.target.node.value();
                    let Some(selected) = selected_nodes.get(source_node_index).copied() else {
                        return Err(gltf_parse_error(
                            ctx,
                            format!(
                                "glTF animation {animation_index} channel {channel_index} targets missing node {source_node_index}"
                            ),
                        ));
                    };
                    let property = checked_property(
                        ctx,
                        animation_index,
                        channel_index,
                        source_channel.target.path.as_ref(),
                    )?;
                    let sampler = source_animation
                        .samplers
                        .get(source_channel.sampler.value())
                        .ok_or_else(|| {
                            gltf_parse_error(
                                ctx,
                                format!(
                                    "glTF animation {animation_index} channel {channel_index} sampler reference is out of range"
                                ),
                            )
                        })?;
                    let owner = AccessorOwner::Animation {
                        animation_index,
                        channel_index,
                    };
                    let input = validate_accessor(
                        ctx,
                        gltf,
                        sampler.input.value(),
                        owner,
                        "input",
                        AccessorContract::vertex(
                            &[json::accessor::ComponentType::F32],
                            &[json::accessor::Type::Scalar],
                        ),
                    )?;
                    if input.count == 0 {
                        return Err(gltf_parse_error(
                            ctx,
                            format!(
                                "glTF animation {animation_index} channel {channel_index} input accessor is empty"
                            ),
                        ));
                    }
                    if !selected {
                        source_channels
                            .try_push_reserved(SourceChannelDisposition::ScopeOmitted)?;
                        animation_cells = animation_cells
                            .checked_add(input.count)
                            .ok_or_else(|| {
                                ctx.allocation_failure("gltf-animation-duration-cells")
                            })?;
                        materialization_channels.try_push_reserved(
                            AnimationMaterializationContract::DurationOnly(
                                AnimationDurationContract {
                                    source: runtime_channel,
                                    animation_index,
                                    channel_index,
                                    key_count: input.count,
                                },
                            ),
                        )?;
                        work.complete([1])?;
                        continue;
                    }
                    let Some(property) = property else {
                        source_channels
                            .try_push_reserved(SourceChannelDisposition::SemanticLoss)?;
                        animation_cells = animation_cells
                            .checked_add(input.count)
                            .ok_or_else(|| {
                                ctx.allocation_failure("gltf-animation-duration-cells")
                            })?;
                        materialization_channels.try_push_reserved(
                            AnimationMaterializationContract::DurationOnly(
                                AnimationDurationContract {
                                    source: runtime_channel,
                                    animation_index,
                                    channel_index,
                                    key_count: input.count,
                                },
                            ),
                        )?;
                        work.complete([1])?;
                        continue;
                    };

                    let source_node = gltf
                        .as_json()
                        .nodes
                        .get(source_node_index)
                        .ok_or_else(|| {
                            gltf_parse_error(
                                ctx,
                                format!(
                                    "glTF animation {animation_index} channel {channel_index} targets missing node {source_node_index}"
                                ),
                            )
                        })?;
                    if source_node.matrix.is_some() {
                        return Err(gltf_parse_error(
                            ctx,
                            format!(
                                "glTF animation {animation_index} channel {channel_index} targets matrix-authored node {source_node_index}"
                            ),
                        ));
                    }

                    let seen_index = source_node_index
                        .checked_mul(3)
                        .and_then(|base| base.checked_add(property.target_slot()))
                        .ok_or_else(|| ctx.allocation_failure("gltf-animation-target-index"))?;
                    let seen = seen_targets.as_mut_slice().get_mut(seen_index).ok_or_else(|| {
                        ContentError::from(super::GltfFailure::internal(
                            "animation target membership is incomplete",
                        ))
                    })?;
                    if *seen == animation_index {
                        return Err(gltf_parse_error(
                            ctx,
                            format!(
                                "glTF animation {animation_index} has a duplicate animation target at channel {channel_index}"
                            ),
                        ));
                    }
                    *seen = animation_index;

                    let interpolation = checked_interpolation(
                        ctx,
                        animation_index,
                        channel_index,
                        sampler.interpolation.as_ref(),
                    )?;
                    if interpolation == AnimationInterpolationAuthoring::CubicSpline
                        && input.count < 2
                    {
                        return Err(gltf_parse_error(
                            ctx,
                            format!(
                                "glTF animation {animation_index} channel {channel_index} cubic spline requires at least two keys"
                            ),
                        ));
                    }
                    let output = validate_accessor(
                        ctx,
                        gltf,
                        sampler.output.value(),
                        owner,
                        "output",
                        output_contract(property),
                    )?;
                    let output_multiplier = if interpolation
                        == AnimationInterpolationAuthoring::CubicSpline
                    {
                        3
                    } else {
                        1
                    };
                    let expected_output_count = input
                        .count
                        .checked_mul(output_multiplier)
                        .ok_or_else(|| ctx.allocation_failure("gltf-animation-output-count"))?;
                    if output.count != expected_output_count {
                        return Err(gltf_parse_error(
                            ctx,
                            format!(
                                "glTF animation {animation_index} channel {channel_index} output accessor count {} does not match expected {expected_output_count}",
                                output.count
                            ),
                        ));
                    }
                    let output_cells = output
                        .count
                        .checked_mul(property.value_width())
                        .ok_or_else(|| ctx.allocation_failure("gltf-animation-output-cells"))?;
                    let cell_count = input
                        .count
                        .checked_add(output_cells)
                        .ok_or_else(|| ctx.allocation_failure("gltf-animation-channel-cells"))?;
                    animation_cells = animation_cells
                        .checked_add(cell_count)
                        .ok_or_else(|| ctx.allocation_failure("gltf-animation-total-cells"))?;
                    materialization_channels.try_push_reserved(
                        AnimationMaterializationContract::Retained(AnimationChannelContract {
                            source: runtime_channel,
                            source_index: visited_channels - 1,
                            animation_index,
                            channel_index,
                            source_node_index,
                            property,
                            interpolation,
                            key_count: input.count,
                            output_count: output.count,
                        }),
                    )?;
                    source_channels.try_push_reserved(SourceChannelDisposition::Retained)?;
                    animation_selected = true;
                    animation_retained_channel_count = animation_retained_channel_count
                        .checked_add(1)
                        .ok_or_else(|| {
                            ctx.allocation_failure("gltf-animation-retained-channel-count")
                        })?;
                    work.complete([1])?;
                }

                if runtime_channel_count != source_animation.channels.len() {
                    return Err(gltf_parse_error(
                        ctx,
                        format!(
                            "glTF animation {animation_index} produced {runtime_channel_count} channels, expected {}",
                            source_animation.channels.len()
                        ),
                    ));
                }
                let materialization_count = materialization_channels
                    .len()
                    .checked_sub(materialization_offset)
                    .ok_or_else(|| {
                        ctx.allocation_failure("gltf-animation-materialization-range")
                    })?;
                if animation_selected {
                    total_cells = total_cells
                        .checked_add(animation_cells)
                        .ok_or_else(|| ctx.allocation_failure("gltf-animation-total-cells"))?;
                    retained_channel_count = retained_channel_count
                        .checked_add(animation_retained_channel_count)
                        .ok_or_else(|| {
                            ctx.allocation_failure("gltf-animation-retained-channel-count")
                        })?;
                    clips.try_push_reserved(AnimationClipContract {
                        source_index: animation_index,
                        materialization_offset,
                        materialization_count,
                        retained_channel_count: animation_retained_channel_count,
                    })?;
                } else {
                    materialization_channels.truncate(materialization_offset);
                }
                let source_channel_end = source_channel_offset
                    .checked_add(runtime_channel_count)
                    .ok_or_else(|| ctx.allocation_failure("gltf-animation-channel-range"))?;
                record_channel_losses(
                    ctx,
                    animation_index,
                    source_channels
                        .as_slice()
                        .get(source_channel_offset..source_channel_end)
                        .ok_or_else(|| {
                            ContentError::from(super::GltfFailure::internal(
                                "animation channel disposition range is invalid",
                            ))
                        })?,
                    animation_selected,
                )?;
                selected_animations.try_push_reserved(animation_selected)?;
            }
            if visited_channels != source_channel_count {
                return Err(gltf_parse_error(
                    ctx,
                    format!(
                        "glTF animation iterator produced {visited_channels} channels, expected {source_channel_count}"
                    ),
                ));
            }
            Ok(AnimationContracts {
                clips,
                materialization_channels,
                selected_animations,
                source_channels,
                retained_channel_count,
                total_cells,
            })
        },
    )
}

fn record_channel_losses(
    ctx: &ParseScope<'_>,
    animation_index: usize,
    dispositions: &[SourceChannelDisposition],
    animation_selected: bool,
) -> Result<()> {
    let mut object_path = ctx.metered_string(
        ImportStorageClass::FormatNormalization,
        "gltf-animation-loss-object-path",
    );
    object_path.try_reserve(48)?;
    for (channel_index, disposition) in dispositions.iter().copied().enumerate() {
        if channel_index.is_multiple_of(super::PARSER_POLL_INTERVAL) {
            ctx.poll()?;
        }
        let (code, reason) = match disposition {
            SourceChannelDisposition::Retained => continue,
            SourceChannelDisposition::ScopeOmitted if !animation_selected => continue,
            SourceChannelDisposition::ScopeOmitted => (
                "gltf.animation_channel_outside_scene_omitted",
                "targets a node outside the selected scene",
            ),
            SourceChannelDisposition::SemanticLoss => (
                "gltf.animation_morph_weights_ignored",
                "uses morph-weight animation, which is not imported yet",
            ),
        };
        object_path.truncate(0);
        object_path.try_write_fmt(format_args!(
            "/animations/{animation_index}/channels/{channel_index}"
        ))?;
        let code = DiagnosticCode::new(code).map_err(|_| {
            ContentError::from(super::GltfFailure::internal(
                "static glTF animation-loss code is invalid",
            ))
        })?;
        ctx.record_source_semantic_loss(
            code,
            SourceScopeEntityKind::AnimationChannel,
            object_path.as_str(),
            |details| {
                details.unsigned("animation_index", to_u64(animation_index));
                details.unsigned("channel_index", to_u64(channel_index));
            },
            format_args!("glTF animation {animation_index} channel {channel_index} {reason}"),
        )?;
    }
    Ok(())
}

fn checked_property(
    ctx: &ParseScope<'_>,
    animation_index: usize,
    channel_index: usize,
    property: Checked<&json::animation::Property>,
) -> Result<Option<NodeAnimationProperty>> {
    match property {
        Checked::Valid(json::animation::Property::Translation) => {
            Ok(Some(NodeAnimationProperty::Translation))
        }
        Checked::Valid(json::animation::Property::Rotation) => {
            Ok(Some(NodeAnimationProperty::Rotation))
        }
        Checked::Valid(json::animation::Property::Scale) => Ok(Some(NodeAnimationProperty::Scale)),
        Checked::Valid(json::animation::Property::MorphTargetWeights) => Ok(None),
        Checked::Invalid => Err(gltf_parse_error(
            ctx,
            format!(
                "glTF animation {animation_index} channel {channel_index} has an invalid target path"
            ),
        )),
    }
}

fn checked_interpolation(
    ctx: &ParseScope<'_>,
    animation_index: usize,
    channel_index: usize,
    interpolation: Checked<&json::animation::Interpolation>,
) -> Result<AnimationInterpolationAuthoring> {
    match interpolation {
        Checked::Valid(json::animation::Interpolation::Step) => {
            Ok(AnimationInterpolationAuthoring::Step)
        }
        Checked::Valid(json::animation::Interpolation::Linear) => {
            Ok(AnimationInterpolationAuthoring::Linear)
        }
        Checked::Valid(json::animation::Interpolation::CubicSpline) => {
            Ok(AnimationInterpolationAuthoring::CubicSpline)
        }
        Checked::Invalid => Err(gltf_parse_error(
            ctx,
            format!(
                "glTF animation {animation_index} channel {channel_index} has an invalid interpolation"
            ),
        )),
    }
}

fn output_contract(property: NodeAnimationProperty) -> AccessorContract {
    match property {
        NodeAnimationProperty::Translation | NodeAnimationProperty::Scale => {
            AccessorContract::vertex(
                &[json::accessor::ComponentType::F32],
                &[json::accessor::Type::Vec3],
            )
        }
        NodeAnimationProperty::Rotation => AccessorContract::vertex(
            &[
                json::accessor::ComponentType::I8,
                json::accessor::ComponentType::U8,
                json::accessor::ComponentType::I16,
                json::accessor::ComponentType::U16,
                json::accessor::ComponentType::F32,
            ],
            &[json::accessor::Type::Vec4],
        )
        .require_normalized_integer(),
    }
}

pub(super) fn build_storage<'scope>(
    ctx: &'scope ParseScope<'_>,
    buffers: &[Option<BufferData<'_>>],
    node_ids: &[Option<NodeLocalId>],
    contracts: &AnimationContracts<'_, '_>,
) -> Result<AnimationStorage<'scope>> {
    ctx.with_work_group(
        [
            (ImportWorkClass::WorkUnits, to_u64(contracts.total_cells)),
            (
                ImportWorkClass::AnimationCells,
                to_u64(contracts.total_cells),
            ),
        ],
        |work| {
            let mut clips = try_vec_capacity(
                ctx,
                ImportStorageClass::SceneConstruction,
                contracts.clips.len(),
                "gltf-animation-storage-clips",
            )?;
            let mut channels = try_vec_capacity(
                ctx,
                ImportStorageClass::SceneConstruction,
                contracts.retained_channel_count,
                "gltf-animation-storage-channels",
            )?;

            for clip in contracts.clips.as_slice() {
                work.poll()?;
                let materialization_channels = checked_contract_range(ctx, contracts, *clip)?;
                let channel_offset = channels.len();
                let mut duration = 0.0_f64;
                for contract in materialization_channels {
                    match contract {
                        AnimationMaterializationContract::Retained(contract) => {
                            let (channel, channel_duration) =
                                read_channel(ctx, buffers, node_ids, contract, work)?;
                            duration = duration.max(channel_duration);
                            channels.try_push_reserved(channel)?;
                        }
                        AnimationMaterializationContract::DurationOnly(contract) => {
                            duration =
                                duration.max(read_duration_channel(ctx, buffers, contract, work)?);
                        }
                    }
                }
                clips.try_push_reserved(AnimationClipStorage {
                    source_index: clip.source_index,
                    duration,
                    channel_offset,
                    channel_count: clip.retained_channel_count,
                })?;
            }
            if channels.len() != contracts.retained_channel_count {
                return Err(ContentError::from(super::GltfFailure::internal(
                    "animation storage did not materialize every admitted channel",
                )));
            }
            Ok(AnimationStorage { clips, channels })
        },
    )
}

fn checked_contract_range<'a>(
    ctx: &ParseScope<'_>,
    contracts: &'a AnimationContracts<'_, '_>,
    clip: AnimationClipContract,
) -> Result<&'a [AnimationMaterializationContract<'a>]> {
    let end = clip
        .materialization_offset
        .checked_add(clip.materialization_count)
        .ok_or_else(|| ctx.allocation_failure("gltf-animation-contract-range"))?;
    contracts
        .materialization_channels
        .as_slice()
        .get(clip.materialization_offset..end)
        .ok_or_else(|| {
            ContentError::from(super::GltfFailure::internal(
                "animation clip channel range is invalid",
            ))
        })
}

fn read_duration_channel(
    ctx: &ParseScope<'_>,
    buffers: &[Option<BufferData<'_>>],
    contract: &AnimationDurationContract<'_>,
    work: &mut ImportWorkScope<'_, '_, 2>,
) -> Result<f64> {
    let reader = safe_gltf(ctx, "glTF animation duration reader construction", || {
        contract.source.reader(|buffer| {
            buffers
                .get(buffer.index())
                .and_then(Option::as_ref)
                .map(BufferData::as_slice)
        })
    })?;
    let mut input = safe_gltf(ctx, "glTF animation duration input reader", || {
        reader.read_inputs()
    })?
    .ok_or_else(|| {
        channel_error_at(
            ctx,
            contract.animation_index,
            contract.channel_index,
            "input accessor could not be read",
        )
    })?;
    let mut previous = None;
    for key in 0..contract.key_count {
        if key.is_multiple_of(PARSER_POLL_INTERVAL) {
            work.poll()?;
        }
        let time = next_gltf_value("glTF animation duration input iteration", &mut input)?
            .ok_or_else(|| {
                channel_error_at(
                    ctx,
                    contract.animation_index,
                    contract.channel_index,
                    "input accessor ended early",
                )
            })?;
        let time = validate_animation_time(
            ctx,
            contract.animation_index,
            contract.channel_index,
            time,
            previous,
        )?;
        previous = Some(time);
        work.complete([1, 1])?;
    }
    if next_gltf_value("glTF animation duration input iteration", &mut input)?.is_some() {
        return Err(channel_error_at(
            ctx,
            contract.animation_index,
            contract.channel_index,
            "input accessor produced too many values",
        ));
    }
    Ok(previous.unwrap_or(0.0))
}

fn read_channel<'scope>(
    ctx: &'scope ParseScope<'_>,
    buffers: &[Option<BufferData<'_>>],
    node_ids: &[Option<NodeLocalId>],
    contract: &AnimationChannelContract<'_>,
    work: &mut ImportWorkScope<'_, '_, 2>,
) -> Result<(AnimationChannelStorage<'scope>, f64)> {
    let node = node_ids
        .get(contract.source_node_index)
        .copied()
        .flatten()
        .ok_or_else(|| {
            ContentError::from(super::GltfFailure::internal(
                "selected animation target has no canonical node identity",
            ))
        })?;
    let reader = safe_gltf(ctx, "glTF animation reader construction", || {
        contract.source.reader(|buffer| {
            buffers
                .get(buffer.index())
                .and_then(Option::as_ref)
                .map(BufferData::as_slice)
        })
    })?;
    let input = safe_gltf(ctx, "glTF animation input reader", || reader.read_inputs())?
        .ok_or_else(|| channel_error(ctx, contract, "input accessor could not be read"))?;
    let (times, duration) = read_times(ctx, contract, input, work)?;

    let scalar_count = contract
        .key_count
        .checked_mul(contract.property.value_width())
        .ok_or_else(|| ctx.allocation_failure("gltf-animation-value-count"))?;
    let mut values = try_vec_capacity(
        ctx,
        ImportStorageClass::SceneConstruction,
        scalar_count,
        "gltf-animation-values",
    )?;
    let tangent_count = if contract.interpolation == AnimationInterpolationAuthoring::CubicSpline {
        scalar_count
    } else {
        0
    };
    let mut in_tangents = try_vec_capacity(
        ctx,
        ImportStorageClass::SceneConstruction,
        tangent_count,
        "gltf-animation-in-tangents",
    )?;
    let mut out_tangents = try_vec_capacity(
        ctx,
        ImportStorageClass::SceneConstruction,
        tangent_count,
        "gltf-animation-out-tangents",
    )?;

    let output = safe_gltf(ctx, "glTF animation output reader", || {
        reader.read_outputs()
    })?
    .ok_or_else(|| channel_error(ctx, contract, "output accessor could not be read"))?;
    match (contract.property, output) {
        (NodeAnimationProperty::Translation, ReadOutputs::Translations(output)) => {
            read_output_vectors(
                ctx,
                contract,
                output,
                &mut values,
                &mut in_tangents,
                &mut out_tangents,
                work,
            )?;
        }
        (NodeAnimationProperty::Rotation, ReadOutputs::Rotations(output)) => {
            read_output_vectors(
                ctx,
                contract,
                output.into_f32(),
                &mut values,
                &mut in_tangents,
                &mut out_tangents,
                work,
            )?;
        }
        (NodeAnimationProperty::Scale, ReadOutputs::Scales(output)) => {
            read_output_vectors(
                ctx,
                contract,
                output,
                &mut values,
                &mut in_tangents,
                &mut out_tangents,
                work,
            )?;
        }
        _ => {
            return Err(ContentError::from(super::GltfFailure::internal(
                "animation output reader did not match its admitted target",
            )));
        }
    }
    if times.len() != contract.key_count
        || values.len() != scalar_count
        || in_tangents.len() != tangent_count
        || out_tangents.len() != tangent_count
    {
        return Err(ContentError::from(super::GltfFailure::internal(
            "animation payload lengths changed after materialization",
        )));
    }
    Ok((
        AnimationChannelStorage {
            source_index: contract.source_index,
            target: contract.property.target(node),
            interpolation: contract.interpolation,
            times,
            values,
            in_tangents,
            out_tangents,
        },
        duration,
    ))
}

fn read_times<'scope>(
    ctx: &'scope ParseScope<'_>,
    contract: &AnimationChannelContract<'_>,
    mut input: impl Iterator<Item = f32>,
    work: &mut ImportWorkScope<'_, '_, 2>,
) -> Result<(MeteredVec<'scope, f64>, f64)> {
    let mut times = try_vec_capacity(
        ctx,
        ImportStorageClass::SceneConstruction,
        contract.key_count,
        "gltf-animation-times",
    )?;
    let mut previous = None;
    for key in 0..contract.key_count {
        if key.is_multiple_of(PARSER_POLL_INTERVAL) {
            work.poll()?;
        }
        let time = next_gltf_value("glTF animation input iteration", &mut input)?
            .ok_or_else(|| channel_error(ctx, contract, "input accessor ended early"))?;
        let time = validate_animation_time(
            ctx,
            contract.animation_index,
            contract.channel_index,
            time,
            previous,
        )?;
        previous = Some(time);
        times.try_push_reserved(time)?;
        work.complete([1, 1])?;
    }
    if next_gltf_value("glTF animation input iteration", &mut input)?.is_some() {
        return Err(channel_error(
            ctx,
            contract,
            "input accessor produced too many values",
        ));
    }
    Ok((times, previous.unwrap_or(0.0)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CubicPart {
    InTangent,
    Value,
    OutTangent,
}

fn read_output_vectors<const N: usize>(
    ctx: &ParseScope<'_>,
    contract: &AnimationChannelContract<'_>,
    mut output: impl Iterator<Item = [f32; N]>,
    values: &mut MeteredVec<'_, f32>,
    in_tangents: &mut MeteredVec<'_, f32>,
    out_tangents: &mut MeteredVec<'_, f32>,
    work: &mut ImportWorkScope<'_, '_, 2>,
) -> Result<()> {
    if N != contract.property.value_width() {
        return Err(ContentError::from(super::GltfFailure::internal(
            "animation reader value width changed after preflight",
        )));
    }
    for ordinal in 0..contract.output_count {
        if ordinal.is_multiple_of(PARSER_POLL_INTERVAL) {
            work.poll()?;
        }
        let sample = next_gltf_value("glTF animation output iteration", &mut output)?
            .ok_or_else(|| channel_error(ctx, contract, "output accessor ended early"))?;
        let part = if contract.interpolation == AnimationInterpolationAuthoring::CubicSpline {
            match ordinal % 3 {
                0 => CubicPart::InTangent,
                1 => CubicPart::Value,
                _ => CubicPart::OutTangent,
            }
        } else {
            CubicPart::Value
        };
        if sample.iter().any(|value| !value.is_finite()) {
            let reason = match part {
                CubicPart::InTangent => "contains a non-finite animation in tangent",
                CubicPart::Value => "contains a non-finite animation value",
                CubicPart::OutTangent => "contains a non-finite animation out tangent",
            };
            return Err(channel_error(ctx, contract, reason));
        }
        if part == CubicPart::Value && contract.property == NodeAnimationProperty::Rotation {
            let rotation = Quat::from_xyzw(sample[0], sample[1], sample[2], sample[3]);
            if !rotation.is_unit() {
                return Err(channel_error(
                    ctx,
                    contract,
                    "contains a non-unit animation rotation value",
                ));
            }
        }
        match part {
            CubicPart::InTangent => in_tangents.try_extend_from_slice_reserved(&sample)?,
            CubicPart::Value => values.try_extend_from_slice_reserved(&sample)?,
            CubicPart::OutTangent => out_tangents.try_extend_from_slice_reserved(&sample)?,
        }
        let cells = to_u64(N);
        work.complete([cells, cells])?;
    }
    if next_gltf_value("glTF animation output iteration", &mut output)?.is_some() {
        return Err(channel_error(
            ctx,
            contract,
            "output accessor produced too many values",
        ));
    }
    Ok(())
}

fn next_gltf_value<T>(
    operation: &'static str,
    values: &mut impl Iterator<Item = T>,
) -> Result<Option<T>> {
    finish_gltf_operation(operation, catch_unwind(AssertUnwindSafe(|| values.next())))
}

fn channel_error(
    ctx: &ParseScope<'_>,
    contract: &AnimationChannelContract<'_>,
    reason: &'static str,
) -> ContentError {
    channel_error_at(
        ctx,
        contract.animation_index,
        contract.channel_index,
        reason,
    )
}

fn validate_animation_time(
    ctx: &ParseScope<'_>,
    animation_index: usize,
    channel_index: usize,
    time: f32,
    previous: Option<f64>,
) -> Result<f64> {
    if !time.is_finite() {
        return Err(channel_error_at(
            ctx,
            animation_index,
            channel_index,
            "contains a non-finite animation time",
        ));
    }
    let time = f64::from(time);
    if time < 0.0 {
        return Err(channel_error_at(
            ctx,
            animation_index,
            channel_index,
            "contains a negative animation time",
        ));
    }
    if previous.is_some_and(|previous| previous >= time) {
        return Err(channel_error_at(
            ctx,
            animation_index,
            channel_index,
            "animation times are not strictly increasing",
        ));
    }
    Ok(time)
}

fn channel_error_at(
    ctx: &ParseScope<'_>,
    animation_index: usize,
    channel_index: usize,
    reason: &'static str,
) -> ContentError {
    gltf_parse_error(
        ctx,
        format!("glTF animation {animation_index} channel {channel_index} {reason}"),
    )
}

impl AnimationStorage<'_> {
    pub(super) fn clip_count(&self) -> usize {
        self.clips.len()
    }

    pub(super) fn channel_count(&self) -> usize {
        self.channels.len()
    }

    pub(super) fn with_authoring_batch<T>(
        &self,
        ctx: &ParseScope<'_>,
        action: impl FnOnce(AnimationAuthoringBatch<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut clips = try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            self.clips.len(),
            "gltf-animation-authoring-clips",
        )?;
        let mut channels = try_vec_capacity(
            ctx,
            ImportStorageClass::SceneConstruction,
            self.channels.len(),
            "gltf-animation-authoring-channels",
        )?;
        for clip in self.clips.as_slice() {
            let end = clip
                .channel_offset
                .checked_add(clip.channel_count)
                .ok_or_else(|| ctx.allocation_failure("gltf-animation-storage-range"))?;
            let source_channels = self
                .channels
                .as_slice()
                .get(clip.channel_offset..end)
                .ok_or_else(|| {
                    ContentError::from(super::GltfFailure::internal(
                        "animation storage clip range is invalid",
                    ))
                })?;
            let authoring_offset = channels.len();
            for channel in source_channels {
                let (in_tangents, out_tangents) =
                    if channel.interpolation == AnimationInterpolationAuthoring::CubicSpline {
                        (
                            Presence::Present(channel.in_tangents.as_slice()),
                            Presence::Present(channel.out_tangents.as_slice()),
                        )
                    } else {
                        (Presence::Absent, Presence::Absent)
                    };
                channels.try_push_reserved(AnimationChannelAuthoring::new(
                    authoring_u32(
                        ctx,
                        channel.source_index,
                        "gltf-animation-source-channel-index",
                    )?,
                    channel.target,
                    channel.interpolation,
                    AnimationExtrapolationAuthoring::SourceDefault,
                    AnimationExtrapolationAuthoring::SourceDefault,
                    channel.times.as_slice(),
                    channel.values.as_slice(),
                    in_tangents,
                    out_tangents,
                ))?;
            }
            clips.try_push_reserved(AnimationClipAuthoring::new(
                authoring_u32(ctx, clip.source_index, "gltf-animation-source-clip-index")?,
                clip.duration,
                AnimationTimeDomainAuthoring::Seconds,
                AuthoringRange::new(
                    authoring_u32(ctx, authoring_offset, "gltf-animation-channel-offset")?,
                    authoring_u32(ctx, source_channels.len(), "gltf-animation-channel-count")?,
                ),
            ))?;
        }
        action(AnimationAuthoringBatch::new(
            clips.as_slice(),
            channels.as_slice(),
        ))
    }
}
