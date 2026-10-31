use super::{
    AccessorContract, AccessorOwner, BufferData, ContentError, MeshInstanceStorage,
    PARSER_POLL_INTERVAL, Result, accessor_provenance, authoring_u32, finish_gltf_operation,
    gltf_parse_error, provenance_work_bound, safe_gltf, to_u64, try_extend_provenance_unique,
    try_vec_capacity, try_vec_filled, validate_accessor,
};
use baozi_import::authoring::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationExtrapolation, AnimationInterpolation, AnimationTargetAuthoring, AnimationTimeDomain,
    AuthoringRange, NodeLocalId, Presence, Quat,
};
use baozi_import::{
    DiagnosticCode, ImportStorageClass, ImportWorkClass, ImportWorkScope, MeteredString,
    MeteredVec, ParseScope, SourceScopeEntityKind,
};
use gltf::animation::util::ReadOutputs;
use gltf::json;
use gltf::json::validation::Checked;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnimationProperty {
    Translation,
    Rotation,
    Scale,
    MorphWeights,
}

impl AnimationProperty {
    const fn target_slot(self) -> usize {
        match self {
            Self::Translation => 0,
            Self::Rotation => 1,
            Self::Scale => 2,
            Self::MorphWeights => 3,
        }
    }

    const fn accessor_width(self) -> usize {
        match self {
            Self::Translation | Self::Scale => 3,
            Self::Rotation => 4,
            Self::MorphWeights => 1,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AnimationClipContract<'document> {
    source_index: usize,
    name: Option<&'document str>,
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
    property: AnimationProperty,
    interpolation: AnimationInterpolation,
    key_count: usize,
    output_count: usize,
    value_width: usize,
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
    clips: MeteredVec<'scope, AnimationClipContract<'document>>,
    materialization_channels: MeteredVec<'scope, AnimationMaterializationContract<'document>>,
    pub(super) selected_animations: MeteredVec<'scope, bool>,
    pub(super) source_channels: MeteredVec<'scope, SourceChannelDisposition>,
    retained_channel_count: usize,
    total_cells: usize,
    total_name_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceChannelDisposition {
    Retained,
    ScopeOmitted,
}

impl SourceChannelDisposition {
    pub(super) const fn is_retained(self) -> bool {
        matches!(self, Self::Retained)
    }
}

struct AnimationClipStorage<'scope> {
    source_index: usize,
    name: Option<MeteredString<'scope>>,
    duration: f64,
    channel_offset: usize,
    channel_count: usize,
    provenance_offset: usize,
    provenance_count: usize,
}

struct AnimationChannelStorage<'scope> {
    source_index: usize,
    target: AnimationTargetAuthoring,
    interpolation: AnimationInterpolation,
    times: MeteredVec<'scope, f64>,
    values: MeteredVec<'scope, f32>,
    in_tangents: MeteredVec<'scope, f32>,
    out_tangents: MeteredVec<'scope, f32>,
    provenance_offset: usize,
    provenance_count: usize,
}

pub(super) struct AnimationStorage<'scope> {
    clips: MeteredVec<'scope, AnimationClipStorage<'scope>>,
    channels: MeteredVec<'scope, AnimationChannelStorage<'scope>>,
    clip_provenance: MeteredVec<'scope, u32>,
    channel_provenance: MeteredVec<'scope, u32>,
}

pub(super) fn preflight<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    gltf: &'document gltf::Document,
    selected_nodes: &[bool],
) -> Result<AnimationContracts<'scope, 'document>> {
    safe_gltf(ctx, "glTF animation preflight", || {
        preflight_inner(ctx, gltf, selected_nodes)
    })?
}

fn preflight_inner<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    gltf: &'document gltf::Document,
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
        .checked_mul(4)
        .ok_or_else(|| ctx.allocation_failure("gltf-animation-target-count"))?;
    let mut seen_targets = try_vec_filled(
        ctx,
        ImportStorageClass::FormatNormalization,
        node_target_count,
        usize::MAX,
        "gltf-animation-seen-targets",
    )?;

    let mut morph_widths = try_vec_filled(
        ctx,
        ImportStorageClass::FormatNormalization,
        gltf.as_json().meshes.len(),
        None,
        "gltf-animation-morph-widths",
    )?;
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
    let mut total_name_bytes = 0usize;
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
            ctx.charge_work(1)?;
            if visited_channels.is_multiple_of(PARSER_POLL_INTERVAL) {
                ctx.poll()?;
            }
            visited_channels = visited_channels
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-animation-channel-progress"))?;
            runtime_channel_count = runtime_channel_count
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-animation-runtime-channel-count"))?;
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
                source_channels.try_push_reserved(SourceChannelDisposition::ScopeOmitted)?;
                animation_cells = animation_cells
                    .checked_add(input.count)
                    .ok_or_else(|| ctx.allocation_failure("gltf-animation-duration-cells"))?;
                materialization_channels.try_push_reserved(
                    AnimationMaterializationContract::DurationOnly(AnimationDurationContract {
                        source: runtime_channel,
                        animation_index,
                        channel_index,
                        key_count: input.count,
                    }),
                )?;
                continue;
            }

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
            if source_node.matrix.is_some() && property != AnimationProperty::MorphWeights {
                return Err(gltf_parse_error(
                    ctx,
                    format!(
                        "glTF animation {animation_index} channel {channel_index} targets matrix-authored node {source_node_index}"
                    ),
                ));
            }

            let seen_index = source_node_index
                .checked_mul(4)
                .and_then(|base| base.checked_add(property.target_slot()))
                .ok_or_else(|| ctx.allocation_failure("gltf-animation-target-index"))?;
            let seen = seen_targets
                .as_mut_slice()
                .get_mut(seen_index)
                .ok_or_else(|| {
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

            let value_width = if property == AnimationProperty::MorphWeights {
                let mesh_index = source_node
                    .mesh
                    .ok_or_else(|| {
                        channel_error_at(
                            ctx,
                            animation_index,
                            channel_index,
                            "morph weights target a meshless node",
                        )
                    })?
                    .value();
                let mesh = gltf.as_json().meshes.get(mesh_index).ok_or_else(|| {
                    channel_error_at(
                        ctx,
                        animation_index,
                        channel_index,
                        "targets morph weights on a missing mesh",
                    )
                })?;
                let cached = morph_widths
                    .as_mut_slice()
                    .get_mut(mesh_index)
                    .ok_or_else(|| {
                        ContentError::from(super::GltfFailure::internal(
                            "animation morph width cache is incomplete",
                        ))
                    })?;
                let width = match *cached {
                    Some(width) => width,
                    None => {
                        let width = super::morph::target_count(ctx, mesh_index, mesh)?;
                        *cached = Some(width);
                        width
                    }
                };
                if width == 0 {
                    return Err(channel_error_at(
                        ctx,
                        animation_index,
                        channel_index,
                        "targets morph weights on a mesh without morph targets",
                    ));
                }
                width
            } else {
                property.accessor_width()
            };

            let interpolation = checked_interpolation(
                ctx,
                animation_index,
                channel_index,
                sampler.interpolation.as_ref(),
            )?;
            if interpolation == AnimationInterpolation::CubicSpline && input.count < 2 {
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
            let output_multiplier = if interpolation == AnimationInterpolation::CubicSpline {
                3
            } else {
                1
            };
            let expected_output_count = input
                .count
                .checked_mul(output_multiplier)
                .and_then(|count| {
                    count.checked_mul(if property == AnimationProperty::MorphWeights {
                        value_width
                    } else {
                        1
                    })
                })
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
                .checked_mul(property.accessor_width())
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
                    value_width,
                }),
            )?;
            source_channels.try_push_reserved(SourceChannelDisposition::Retained)?;
            animation_selected = true;
            animation_retained_channel_count = animation_retained_channel_count
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-animation-retained-channel-count"))?;
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
            .ok_or_else(|| ctx.allocation_failure("gltf-animation-materialization-range"))?;
        if animation_selected {
            total_name_bytes = total_name_bytes
                .checked_add(animation.name().map_or(0, str::len))
                .ok_or_else(|| ctx.allocation_failure("gltf-animation-name-bytes"))?;
            total_cells = total_cells
                .checked_add(animation_cells)
                .ok_or_else(|| ctx.allocation_failure("gltf-animation-total-cells"))?;
            retained_channel_count = retained_channel_count
                .checked_add(animation_retained_channel_count)
                .ok_or_else(|| ctx.allocation_failure("gltf-animation-retained-channel-count"))?;
            clips.try_push_reserved(AnimationClipContract {
                source_index: animation_index,
                name: animation.name(),
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
        total_name_bytes,
    })
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
) -> Result<AnimationProperty> {
    match property {
        Checked::Valid(json::animation::Property::Translation) => {
            Ok(AnimationProperty::Translation)
        }
        Checked::Valid(json::animation::Property::Rotation) => Ok(AnimationProperty::Rotation),
        Checked::Valid(json::animation::Property::Scale) => Ok(AnimationProperty::Scale),
        Checked::Valid(json::animation::Property::MorphTargetWeights) => {
            Ok(AnimationProperty::MorphWeights)
        }
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
) -> Result<AnimationInterpolation> {
    match interpolation {
        Checked::Valid(json::animation::Interpolation::Step) => Ok(AnimationInterpolation::Step),
        Checked::Valid(json::animation::Interpolation::Linear) => {
            Ok(AnimationInterpolation::Linear)
        }
        Checked::Valid(json::animation::Interpolation::CubicSpline) => {
            Ok(AnimationInterpolation::CubicSpline)
        }
        Checked::Invalid => Err(gltf_parse_error(
            ctx,
            format!(
                "glTF animation {animation_index} channel {channel_index} has an invalid interpolation"
            ),
        )),
    }
}

fn output_contract(property: AnimationProperty) -> AccessorContract {
    match property {
        AnimationProperty::Translation | AnimationProperty::Scale => AccessorContract::vertex(
            &[json::accessor::ComponentType::F32],
            &[json::accessor::Type::Vec3],
        ),
        // The glTF animation output profile permits float32 or normalized signed/unsigned
        // 8-/16-bit components for rotation and morph weights; weights remain SCALAR.
        AnimationProperty::Rotation | AnimationProperty::MorphWeights => AccessorContract::vertex(
            &[
                json::accessor::ComponentType::I8,
                json::accessor::ComponentType::U8,
                json::accessor::ComponentType::I16,
                json::accessor::ComponentType::U16,
                json::accessor::ComponentType::F32,
            ],
            if property == AnimationProperty::Rotation {
                &[json::accessor::Type::Vec4]
            } else {
                &[json::accessor::Type::Scalar]
            },
        )
        .require_normalized_integer(),
    }
}

pub(super) fn build_storage<'scope>(
    ctx: &'scope ParseScope<'_>,
    buffers: &[Option<BufferData<'_>>],
    node_ids: &[Option<NodeLocalId>],
    mesh_instances: &[MeshInstanceStorage],
    contracts: &AnimationContracts<'_, '_>,
) -> Result<AnimationStorage<'scope>> {
    let mut instance_ids = try_vec_filled(
        ctx,
        ImportStorageClass::FormatNormalization,
        node_ids.len(),
        None,
        "gltf-animation-mesh-instance-map",
    )?;
    for (ordinal, instance) in mesh_instances.iter().enumerate() {
        ctx.charge_work(1)?;
        let slot = instance_ids
            .as_mut_slice()
            .get_mut(instance.source_node_index)
            .ok_or_else(|| {
                ContentError::from(super::GltfFailure::internal(
                    "animation mesh instance source node is out of range",
                ))
            })?;
        if slot.is_some() {
            return Err(ContentError::from(super::GltfFailure::internal(
                "animation mesh instance source node is duplicated",
            )));
        }
        *slot = Some(authoring_u32(
            ctx,
            ordinal,
            "gltf-animation-mesh-instance-index",
        )?);
    }
    // Work scopes cannot nest. Prepay source collection/sort/dedup before sample decoding;
    // retained channel pairs need at most two comparisons and two writes each.
    let mut provenance_work = to_u64(contracts.retained_channel_count)
        .checked_mul(4)
        .ok_or_else(|| ctx.allocation_failure("gltf-animation-provenance-work"))?;
    let mut maximum_pending_provenance = 0;
    for clip in contracts.clips.as_slice() {
        ctx.charge_work(1)?;
        let references = clip
            .materialization_count
            .checked_add(clip.retained_channel_count)
            .ok_or_else(|| ctx.allocation_failure("gltf-animation-clip-provenance-count"))?;
        maximum_pending_provenance = maximum_pending_provenance.max(references);
        provenance_work = provenance_work
            .checked_add(provenance_work_bound(ctx, references)?)
            .ok_or_else(|| ctx.allocation_failure("gltf-animation-provenance-work"))?;
    }
    ctx.charge_work(provenance_work)?;
    ctx.with_work_group(
        [
            (ImportWorkClass::WorkUnits, to_u64(contracts.total_cells)),
            (
                ImportWorkClass::AnimationCells,
                to_u64(contracts.total_cells),
            ),
            (
                ImportWorkClass::CanonicalStringBytes,
                to_u64(contracts.total_name_bytes),
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
            let available_provenance_count = buffers
                .iter()
                .filter(|buffer| buffer.is_some())
                .count()
                .max(1);
            let maximum_channel_provenance = contracts
                .retained_channel_count
                .checked_mul(available_provenance_count.min(2))
                .ok_or_else(|| ctx.allocation_failure("gltf-animation-channel-provenance-count"))?;
            let maximum_clip_provenance = contracts.clips.as_slice().iter().try_fold(
                0usize,
                |count, clip| -> Result<usize> {
                    let reference_count = clip
                        .materialization_count
                        .checked_add(clip.retained_channel_count)
                        .ok_or_else(|| {
                            ctx.allocation_failure("gltf-animation-clip-provenance-count")
                        })?;
                    count
                        .checked_add(reference_count.min(available_provenance_count))
                        .ok_or_else(|| {
                            ctx.allocation_failure("gltf-animation-clip-provenance-count")
                                .into()
                        })
                },
            )?;
            let mut clip_provenance = try_vec_capacity(
                ctx,
                ImportStorageClass::FormatNormalization,
                maximum_clip_provenance,
                "gltf-animation-clip-provenance",
            )?;
            let mut channel_provenance = try_vec_capacity(
                ctx,
                ImportStorageClass::FormatNormalization,
                maximum_channel_provenance,
                "gltf-animation-channel-provenance",
            )?;
            let mut pending_provenance = try_vec_capacity(
                ctx,
                ImportStorageClass::FormatNormalization,
                maximum_pending_provenance,
                "gltf-animation-pending-provenance",
            )?;

            for clip in contracts.clips.as_slice() {
                work.poll()?;
                let materialization_channels = checked_contract_range(ctx, contracts, *clip)?;
                let channel_offset = channels.len();
                let provenance_offset = clip_provenance.len();
                let mut duration = 0.0_f64;
                for contract in materialization_channels {
                    match contract {
                        AnimationMaterializationContract::Retained(contract) => {
                            let (channel, channel_duration) = read_channel(
                                ctx,
                                buffers,
                                node_ids,
                                instance_ids.as_slice(),
                                contract,
                                work,
                                &mut channel_provenance,
                            )?;
                            duration = duration.max(channel_duration);
                            let channel_end = channel
                                .provenance_offset
                                .checked_add(channel.provenance_count)
                                .ok_or_else(|| {
                                    ctx.allocation_failure(
                                        "gltf-animation-channel-provenance-range",
                                    )
                                })?;
                            let channel_sources = channel_provenance
                                .as_slice()
                                .get(channel.provenance_offset..channel_end)
                                .ok_or_else(|| {
                                    gltf_parse_error(
                                        ctx,
                                        "glTF animation channel provenance is incomplete",
                                    )
                                })?;
                            for &source in channel_sources {
                                pending_provenance.try_push_reserved(source)?;
                            }
                            channels.try_push_reserved(channel)?;
                        }
                        AnimationMaterializationContract::DurationOnly(contract) => {
                            let (channel_duration, source) =
                                read_duration_channel(ctx, buffers, contract, work)?;
                            duration = duration.max(channel_duration);
                            pending_provenance.try_push_reserved(source)?;
                        }
                    }
                }
                try_extend_provenance_unique(ctx, &mut clip_provenance, &mut pending_provenance)?;
                let name = clip
                    .name
                    .map(|name| -> Result<_> {
                        let mut owned = ctx.metered_string(
                            ImportStorageClass::FormatNormalization,
                            "gltf-normalized-clip-name",
                        );
                        owned.try_reserve(name.len())?;
                        let mut rest = name;
                        while !rest.is_empty() {
                            work.poll()?;
                            let mut end = rest.len().min(PARSER_POLL_INTERVAL);
                            while !rest.is_char_boundary(end) {
                                end -= 1;
                            }
                            let (chunk, tail) = rest.split_at(end);
                            owned.try_push_str(chunk)?;
                            rest = tail;
                        }
                        Ok(owned)
                    })
                    .transpose()?;
                work.complete([0, 0, to_u64(clip.name.map_or(0, str::len))])?;
                clips.try_push_reserved(AnimationClipStorage {
                    source_index: clip.source_index,
                    name,
                    duration,
                    channel_offset,
                    channel_count: clip.retained_channel_count,
                    provenance_offset,
                    provenance_count: clip_provenance.len() - provenance_offset,
                })?;
            }
            if channels.len() != contracts.retained_channel_count {
                return Err(ContentError::from(super::GltfFailure::internal(
                    "animation storage did not materialize every admitted channel",
                )));
            }
            Ok(AnimationStorage {
                clips,
                channels,
                clip_provenance,
                channel_provenance,
            })
        },
    )
}

fn checked_contract_range<'a>(
    ctx: &ParseScope<'_>,
    contracts: &'a AnimationContracts<'_, '_>,
    clip: AnimationClipContract<'_>,
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
    work: &mut ImportWorkScope<'_, '_, 3>,
) -> Result<(f64, u32)> {
    let source = accessor_provenance(ctx, buffers, contract.source.sampler().input())?;
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
        work.complete([1, 1, 0])?;
    }
    if next_gltf_value("glTF animation duration input iteration", &mut input)?.is_some() {
        return Err(channel_error_at(
            ctx,
            contract.animation_index,
            contract.channel_index,
            "input accessor produced too many values",
        ));
    }
    Ok((previous.unwrap_or(0.0), source))
}

fn read_channel<'scope>(
    ctx: &'scope ParseScope<'_>,
    buffers: &[Option<BufferData<'_>>],
    node_ids: &[Option<NodeLocalId>],
    instance_ids: &[Option<u32>],
    contract: &AnimationChannelContract<'_>,
    work: &mut ImportWorkScope<'_, '_, 3>,
    provenance: &mut MeteredVec<'_, u32>,
) -> Result<(AnimationChannelStorage<'scope>, f64)> {
    let node = || {
        node_ids
            .get(contract.source_node_index)
            .copied()
            .flatten()
            .ok_or_else(|| {
                ContentError::from(super::GltfFailure::internal(
                    "selected animation target has no canonical node identity",
                ))
            })
    };
    let target = match contract.property {
        AnimationProperty::Translation => AnimationTargetAuthoring::NodeTranslation(node()?),
        AnimationProperty::Rotation => AnimationTargetAuthoring::NodeRotation(node()?),
        AnimationProperty::Scale => AnimationTargetAuthoring::NodeScale(node()?),
        AnimationProperty::MorphWeights => {
            let instance = instance_ids
                .get(contract.source_node_index)
                .copied()
                .flatten()
                .ok_or_else(|| {
                    ContentError::from(super::GltfFailure::internal(
                        "selected animation target has no canonical mesh instance identity",
                    ))
                })?;
            AnimationTargetAuthoring::MeshInstanceMorphWeights(instance)
        }
    };
    let provenance_offset = provenance.len();
    let sampler = contract.source.sampler();
    let input_source = accessor_provenance(ctx, buffers, sampler.input())?;
    let output_source = accessor_provenance(ctx, buffers, sampler.output())?;
    let (first, second) = if input_source <= output_source {
        (input_source, output_source)
    } else {
        (output_source, input_source)
    };
    provenance.try_push_reserved(first)?;
    if first != second {
        provenance.try_push_reserved(second)?;
    }
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
        .checked_mul(contract.value_width)
        .ok_or_else(|| ctx.allocation_failure("gltf-animation-value-count"))?;
    let mut values = try_vec_capacity(
        ctx,
        ImportStorageClass::SceneConstruction,
        scalar_count,
        "gltf-animation-values",
    )?;
    let tangent_count = if contract.interpolation == AnimationInterpolation::CubicSpline {
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
        (AnimationProperty::Translation, ReadOutputs::Translations(output)) => {
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
        (AnimationProperty::Rotation, ReadOutputs::Rotations(output)) => {
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
        (AnimationProperty::Scale, ReadOutputs::Scales(output)) => {
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
        (AnimationProperty::MorphWeights, ReadOutputs::MorphTargetWeights(output)) => {
            read_output_weights(
                ctx,
                contract,
                output.into_f32(),
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
            target,
            interpolation: contract.interpolation,
            times,
            values,
            in_tangents,
            out_tangents,
            provenance_offset,
            provenance_count: provenance.len() - provenance_offset,
        },
        duration,
    ))
}

fn read_times<'scope>(
    ctx: &'scope ParseScope<'_>,
    contract: &AnimationChannelContract<'_>,
    mut input: impl Iterator<Item = f32>,
    work: &mut ImportWorkScope<'_, '_, 3>,
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
        work.complete([1, 1, 0])?;
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

impl CubicPart {
    const fn at(interpolation: AnimationInterpolation, ordinal: usize) -> Self {
        if matches!(interpolation, AnimationInterpolation::CubicSpline) {
            match ordinal % 3 {
                0 => Self::InTangent,
                1 => Self::Value,
                _ => Self::OutTangent,
            }
        } else {
            Self::Value
        }
    }

    const fn non_finite_reason(self) -> &'static str {
        match self {
            Self::InTangent => "contains a non-finite animation in tangent",
            Self::Value => "contains a non-finite animation value",
            Self::OutTangent => "contains a non-finite animation out tangent",
        }
    }
}

fn read_output_weights(
    ctx: &ParseScope<'_>,
    contract: &AnimationChannelContract<'_>,
    mut output: impl Iterator<Item = f32>,
    values: &mut MeteredVec<'_, f32>,
    in_tangents: &mut MeteredVec<'_, f32>,
    out_tangents: &mut MeteredVec<'_, f32>,
    work: &mut ImportWorkScope<'_, '_, 3>,
) -> Result<()> {
    for ordinal in 0..contract.output_count {
        if ordinal.is_multiple_of(PARSER_POLL_INTERVAL) {
            work.poll()?;
        }
        let weight = next_gltf_value("glTF animation weight output iteration", &mut output)?
            .ok_or_else(|| channel_error(ctx, contract, "output accessor ended early"))?;
        // Each cubic part contains one whole weight vector, not one scalar.
        let part = CubicPart::at(contract.interpolation, ordinal / contract.value_width);
        if !weight.is_finite() {
            return Err(channel_error(ctx, contract, part.non_finite_reason()));
        }
        match part {
            CubicPart::InTangent => in_tangents.try_push_reserved(weight)?,
            CubicPart::Value => values.try_push_reserved(weight)?,
            CubicPart::OutTangent => out_tangents.try_push_reserved(weight)?,
        }
        work.complete([1, 1, 0])?;
    }
    if next_gltf_value("glTF animation weight output iteration", &mut output)?.is_some() {
        return Err(channel_error(
            ctx,
            contract,
            "output accessor produced too many values",
        ));
    }
    Ok(())
}

fn read_output_vectors<const N: usize>(
    ctx: &ParseScope<'_>,
    contract: &AnimationChannelContract<'_>,
    mut output: impl Iterator<Item = [f32; N]>,
    values: &mut MeteredVec<'_, f32>,
    in_tangents: &mut MeteredVec<'_, f32>,
    out_tangents: &mut MeteredVec<'_, f32>,
    work: &mut ImportWorkScope<'_, '_, 3>,
) -> Result<()> {
    if N != contract.value_width {
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
        let part = CubicPart::at(contract.interpolation, ordinal);
        if sample.iter().any(|value| !value.is_finite()) {
            return Err(channel_error(ctx, contract, part.non_finite_reason()));
        }
        if part == CubicPart::Value && contract.property == AnimationProperty::Rotation {
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
        work.complete([cells, cells, 0])?;
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

impl<'scope> AnimationStorage<'scope> {
    #[cfg(test)]
    pub(super) fn empty(ctx: &'scope ParseScope<'_>) -> Self {
        let class = ImportStorageClass::FormatNormalization;
        Self {
            clips: ctx.metered_vec(class, "gltf-empty-animation-clips"),
            channels: ctx.metered_vec(class, "gltf-empty-animation-channels"),
            clip_provenance: ctx.metered_vec(class, "gltf-empty-animation-clip-provenance"),
            channel_provenance: ctx.metered_vec(class, "gltf-empty-animation-channel-provenance"),
        }
    }

    pub(super) fn clip_count(&self) -> usize {
        self.clips.len()
    }

    pub(super) fn channel_count(&self) -> usize {
        self.channels.len()
    }

    pub(super) fn non_primary_provenance_count(&self) -> usize {
        self.clip_provenance
            .as_slice()
            .iter()
            .chain(self.channel_provenance.as_slice())
            .filter(|&&source| source != 0)
            .count()
    }

    pub(super) fn clip_provenance(&self, index: usize) -> Option<&[u32]> {
        let clip = self.clips.as_slice().get(index)?;
        let end = clip.provenance_offset.checked_add(clip.provenance_count)?;
        self.clip_provenance
            .as_slice()
            .get(clip.provenance_offset..end)
    }

    pub(super) fn channel_provenance(&self, index: usize) -> Option<&[u32]> {
        let channel = self.channels.as_slice().get(index)?;
        let end = channel
            .provenance_offset
            .checked_add(channel.provenance_count)?;
        self.channel_provenance
            .as_slice()
            .get(channel.provenance_offset..end)
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
                    if channel.interpolation == AnimationInterpolation::CubicSpline {
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
                    AnimationExtrapolation::SourceDefault,
                    AnimationExtrapolation::SourceDefault,
                    channel.times.as_slice(),
                    channel.values.as_slice(),
                    in_tangents,
                    out_tangents,
                ))?;
            }
            let authored_clip = AnimationClipAuthoring::new(
                authoring_u32(ctx, clip.source_index, "gltf-animation-source-clip-index")?,
                clip.duration,
                AnimationTimeDomain::Seconds,
                AuthoringRange::new(
                    authoring_u32(ctx, authoring_offset, "gltf-animation-channel-offset")?,
                    authoring_u32(ctx, source_channels.len(), "gltf-animation-channel-count")?,
                ),
            );
            clips.try_push_reserved(match clip.name.as_ref() {
                Some(name) => authored_clip.with_name(name.as_str()),
                None => authored_clip,
            })?;
        }
        action(AnimationAuthoringBatch::new(
            clips.as_slice(),
            channels.as_slice(),
        ))
    }
}
