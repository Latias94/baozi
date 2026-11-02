use super::{
    AccessorContract, AccessorOwner, AccessorSources, BufferData, ContentError,
    GEOMETRY_POLL_INTERVAL, MeshInstanceStorage, Result, accessor_provenance, authoring_u32,
    gltf_parse_error, to_u64, try_extend_provenance_unique, try_vec_capacity, try_vec_filled,
    validate_accessor, with_poll,
};
use crate::document::schema as json;
use baozi_import::authoring::{
    AnimationAuthoringBatch, AnimationChannelAuthoring, AnimationClipAuthoring,
    AnimationExtrapolation, AnimationInterpolation, AnimationTargetAuthoring, AnimationTimeDomain,
    AuthoringRange, NodeLocalId, Presence, Quat,
};
use baozi_import::{
    DiagnosticCode, ImportStorageClass, ImportWorkClass, ImportWorkScope, MeteredString,
    MeteredVec, ParseExecution, SourceScopeEntityKind,
};

const MAX_CHANNEL_PROVENANCE_SOURCES: usize = 2 * AccessorSources::MAX_SOURCES;

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
    source: crate::document::Channel<'document>,
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
    source: crate::document::Channel<'document>,
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
    CoreIgnored,
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
    ctx: &mut ParseExecution<'_, 'scope>,
    gltf: &'document crate::document::Document<'_>,
    selected_nodes: &[bool],
) -> Result<AnimationContracts<'scope, 'document>> {
    with_poll(ctx, |ctx| preflight_inner(ctx, gltf, selected_nodes))?
}

fn preflight_inner<'scope, 'document>(
    ctx: &mut ParseExecution<'_, 'scope>,
    gltf: &'document crate::document::Document<'_>,
    selected_nodes: &[bool],
) -> Result<AnimationContracts<'scope, 'document>> {
    let source_animations = &gltf.tables().animations;
    let mut source_channel_count = 0usize;
    for (index, animation) in source_animations.iter().enumerate() {
        super::poll_parser_loop(ctx, index)?;
        source_channel_count = source_channel_count
            .checked_add(animation.channels.len())
            .ok_or_else(|| ctx.allocation_failure("gltf-animation-channel-count"))?;
    }
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
        gltf.tables().meshes.len(),
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

    let work_group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
    for animation in gltf.animations() {
        let animation_index = animation.index();
        let source_animation = source_animations.get(animation_index).ok_or_else(|| {
            gltf_parse_error(format_args!(
                "glTF animation {animation_index} is out of range"
            ))
        })?;
        let materialization_offset = materialization_channels.len();
        let source_channel_offset = source_channels.len();
        let mut runtime_channel_count = 0usize;
        let mut animation_selected = false;
        let mut animation_retained_channel_count = 0usize;
        let mut animation_cells = 0usize;

        for (channel_index, runtime_channel) in animation.channels().enumerate() {
            ctx.with_work(
                &work_group,
                [1],
                |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
            )?;
            if visited_channels.is_multiple_of(GEOMETRY_POLL_INTERVAL) {
                ctx.poll()?;
            }
            visited_channels = visited_channels
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-animation-channel-progress"))?;
            runtime_channel_count = runtime_channel_count
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-animation-runtime-channel-count"))?;
            let source_channel = source_animation
                .channels
                .get(channel_index)
                .ok_or_else(|| {
                    gltf_parse_error(format_args!(
                        "glTF animation {animation_index} channel {channel_index} is out of range"
                    ))
                })?;
            let Some(source_node) = source_channel.target.node else {
                // Core channels without a node have no effect. Source preflight separately
                // rejects extension payloads that could introduce another targeting mechanism.
                source_channels.try_push_reserved(SourceChannelDisposition::CoreIgnored)?;
                continue;
            };
            let source_node_index = source_node.value();
            let Some(selected) = selected_nodes.get(source_node_index).copied() else {
                return Err(gltf_parse_error(format_args!(
                    "glTF animation {animation_index} channel {channel_index} targets missing node {source_node_index}"
                )));
            };
            let property = map_property(source_channel.target.path);
            let sampler = source_animation
                .samplers
                .get(source_channel.sampler.value())
                .ok_or_else(|| {
                    gltf_parse_error(
                        format_args!(
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
                AccessorContract::tightly_packed(
                    &[json::accessor::ComponentType::F32],
                    &[json::accessor::Type::Scalar],
                ),
            )?;
            if input.count == 0 {
                return Err(gltf_parse_error(format_args!(
                    "glTF animation {animation_index} channel {channel_index} input accessor is empty"
                )));
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
                return Err(gltf_parse_error(format_args!(
                    "glTF animation {animation_index} has a duplicate animation target at channel {channel_index}"
                )));
            }
            *seen = animation_index;

            let value_width = if property == AnimationProperty::MorphWeights {
                let source_node = gltf.tables().nodes.get(source_node_index).ok_or_else(|| {
                    ContentError::from(super::GltfFailure::internal(
                        "validated animation node reference is missing",
                    ))
                })?;
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
                let mesh = gltf.tables().meshes.get(mesh_index).ok_or_else(|| {
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

            let interpolation = map_interpolation(sampler.interpolation);
            if interpolation == AnimationInterpolation::CubicSpline && input.count < 2 {
                return Err(gltf_parse_error(format_args!(
                    "glTF animation {animation_index} channel {channel_index} cubic spline requires at least two keys"
                )));
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
                return Err(gltf_parse_error(format_args!(
                    "glTF animation {animation_index} channel {channel_index} output accessor count {} does not match expected {expected_output_count}",
                    output.count
                )));
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
            return Err(gltf_parse_error(format_args!(
                "glTF animation {animation_index} produced {runtime_channel_count} channels, expected {}",
                source_animation.channels.len()
            )));
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
        return Err(gltf_parse_error(format_args!(
            "glTF animation iterator produced {visited_channels} channels, expected {source_channel_count}"
        )));
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
    ctx: &mut ParseExecution<'_, '_>,
    animation_index: usize,
    dispositions: &[SourceChannelDisposition],
    animation_selected: bool,
) -> Result<()> {
    let mut object_path = ctx.metered_string(
        ImportStorageClass::FormatNormalization,
        "gltf-animation-loss-object-path",
    );
    object_path.try_reserve(ctx, 48)?;
    for (channel_index, disposition) in dispositions.iter().copied().enumerate() {
        if channel_index.is_multiple_of(super::GEOMETRY_POLL_INTERVAL) {
            ctx.poll()?;
        }
        let (code, reason) = match disposition {
            SourceChannelDisposition::Retained | SourceChannelDisposition::CoreIgnored => continue,
            SourceChannelDisposition::ScopeOmitted if !animation_selected => continue,
            SourceChannelDisposition::ScopeOmitted => (
                "gltf.animation_channel_outside_scene_omitted",
                "targets a node outside the selected scene",
            ),
        };
        object_path.truncate(0);
        object_path.try_write_fmt(
            ctx,
            format_args!("/animations/{animation_index}/channels/{channel_index}"),
        )?;
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

fn map_property(property: json::animation::Property) -> AnimationProperty {
    match property {
        json::animation::Property::Translation => AnimationProperty::Translation,
        json::animation::Property::Rotation => AnimationProperty::Rotation,
        json::animation::Property::Scale => AnimationProperty::Scale,
        json::animation::Property::MorphTargetWeights => AnimationProperty::MorphWeights,
    }
}

fn map_interpolation(interpolation: json::animation::Interpolation) -> AnimationInterpolation {
    match interpolation {
        json::animation::Interpolation::Step => AnimationInterpolation::Step,
        json::animation::Interpolation::Linear => AnimationInterpolation::Linear,
        json::animation::Interpolation::CubicSpline => AnimationInterpolation::CubicSpline,
    }
}

fn output_contract(property: AnimationProperty) -> AccessorContract {
    match property {
        AnimationProperty::Translation | AnimationProperty::Scale => {
            AccessorContract::tightly_packed(
                &[json::accessor::ComponentType::F32],
                &[json::accessor::Type::Vec3],
            )
        }
        // The glTF animation output profile permits float32 or normalized signed/unsigned
        // 8-/16-bit components for rotation and morph weights; weights remain SCALAR.
        AnimationProperty::Rotation | AnimationProperty::MorphWeights => {
            AccessorContract::tightly_packed(
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
            .require_normalized_integer()
        }
    }
}

pub(super) fn build_storage<'scope>(
    ctx: &mut ParseExecution<'_, 'scope>,
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
    let work_group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
    for (ordinal, instance) in mesh_instances.iter().enumerate() {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
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
    // Fixed accessor source collection stays separate from the later variable-size unions.
    let accessor_count = contracts
        .materialization_channels
        .len()
        .checked_add(contracts.retained_channel_count)
        .ok_or_else(|| ctx.allocation_failure("gltf-animation-provenance-count"))?;
    let provenance_work = to_u64(accessor_count)
        .checked_mul(AccessorSources::COLLECTION_WORK_BOUND)
        .ok_or_else(|| ctx.allocation_failure("gltf-animation-provenance-work"))?;
    let available_provenance_count = ctx.with_work_group(
        [(ImportWorkClass::WorkUnits, to_u64(buffers.len()))],
        |ctx, work| -> Result<_> {
            let mut count = 0usize;
            for (index, buffer) in buffers.iter().enumerate() {
                super::poll_parser_loop(ctx, index)?;
                count += usize::from(buffer.is_some());
                work.complete([1])?;
            }
            Ok(count.max(1))
        },
    )?;
    let maximum_channel_provenance = contracts
        .retained_channel_count
        .checked_mul(available_provenance_count.min(MAX_CHANNEL_PROVENANCE_SOURCES))
        .ok_or_else(|| ctx.allocation_failure("gltf-animation-channel-provenance-count"))?;
    let mut maximum_pending_provenance = 0;
    let mut maximum_clip_provenance = 0usize;
    for clip in contracts.clips.as_slice() {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        let references = clip
            .materialization_count
            .checked_add(clip.retained_channel_count)
            .and_then(|count| count.checked_mul(AccessorSources::MAX_SOURCES))
            .ok_or_else(|| ctx.allocation_failure("gltf-animation-clip-provenance-count"))?;
        maximum_pending_provenance = maximum_pending_provenance.max(references);
        maximum_clip_provenance = maximum_clip_provenance
            .checked_add(references.min(available_provenance_count))
            .ok_or_else(|| ctx.allocation_failure("gltf-animation-clip-provenance-count"))?;
    }
    ctx.charge_work(provenance_work)?;
    let mut clip_provenance = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        maximum_clip_provenance,
        "gltf-animation-clip-provenance",
    )?;
    let mut clip_ranges = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        contracts.clips.len(),
        "gltf-animation-clip-source-ranges",
    )?;
    let mut pending = try_vec_capacity(
        ctx,
        ImportStorageClass::FormatNormalization,
        maximum_pending_provenance,
        "gltf-animation-pending-provenance",
    )?;
    for contract in contracts.clips.as_slice() {
        ctx.poll()?;
        let start = clip_provenance.len();
        for channel in checked_contract_range(ctx, contracts, *contract)? {
            ctx.poll()?;
            let sampler = match channel {
                AnimationMaterializationContract::Retained(channel) => channel.source.sampler(),
                AnimationMaterializationContract::DurationOnly(channel) => channel.source.sampler(),
            };
            let input = accessor_provenance(ctx, buffers, sampler.input())?;
            pending.try_extend_from_slice_reserved(input.as_slice())?;
            if matches!(channel, AnimationMaterializationContract::Retained(_)) {
                let output = accessor_provenance(ctx, buffers, sampler.output())?;
                pending.try_extend_from_slice_reserved(output.as_slice())?;
            }
        }
        try_extend_provenance_unique(ctx, &mut clip_provenance, &mut pending)?;
        clip_ranges.try_push_reserved((start, clip_provenance.len() - start))?;
    }
    drop(pending);
    let storage = ctx.with_work_group(
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
        |ctx, work| -> Result<_> {
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
            let mut channel_provenance = try_vec_capacity(
                ctx,
                ImportStorageClass::FormatNormalization,
                maximum_channel_provenance,
                "gltf-animation-channel-provenance",
            )?;

            for (clip_index, clip) in contracts.clips.as_slice().iter().enumerate() {
                ctx.poll()?;
                let materialization_channels = checked_contract_range(ctx, contracts, *clip)?;
                let channel_offset = channels.len();
                let (provenance_offset, provenance_count) = clip_ranges.as_slice()[clip_index];
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
                            channels.try_push_reserved(channel)?;
                        }
                        AnimationMaterializationContract::DurationOnly(contract) => {
                            let (channel_duration, _sources) =
                                read_duration_channel(ctx, buffers, contract, work)?;
                            duration = duration.max(channel_duration);
                        }
                    }
                }
                let name = clip
                    .name
                    .map(|name| -> Result<_> {
                        let mut owned = ctx.metered_string(
                            ImportStorageClass::FormatNormalization,
                            "gltf-normalized-clip-name",
                        );
                        owned.try_reserve(ctx, name.len())?;
                        let mut rest = name;
                        while !rest.is_empty() {
                            ctx.poll()?;
                            let mut end = rest.len().min(super::BYTE_POLL_INTERVAL);
                            while !rest.is_char_boundary(end) {
                                end -= 1;
                            }
                            let (chunk, tail) = rest.split_at(end);
                            owned.try_push_str(ctx, chunk)?;
                            work.complete([0, 0, to_u64(chunk.len())])?;
                            rest = tail;
                        }
                        Ok(owned)
                    })
                    .transpose()?;
                clips.try_push_reserved(AnimationClipStorage {
                    source_index: clip.source_index,
                    name,
                    duration,
                    channel_offset,
                    channel_count: clip.retained_channel_count,
                    provenance_offset,
                    provenance_count,
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
    )?;
    Ok(storage)
}

fn checked_contract_range<'a>(
    ctx: &mut ParseExecution<'_, '_>,
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
    ctx: &mut ParseExecution<'_, '_>,
    buffers: &[Option<BufferData<'_>>],
    contract: &AnimationDurationContract<'_>,
    work: &mut ImportWorkScope<'_, 3>,
) -> Result<(f64, AccessorSources)> {
    let source = accessor_provenance(ctx, buffers, contract.source.sampler().input())?;
    let source_accessor = contract.source.sampler().input();
    let mut input = super::accessor::bind(ctx, source_accessor, buffers)?
        .scalars()
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
        if key.is_multiple_of(GEOMETRY_POLL_INTERVAL) {
            ctx.poll()?;
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
    ctx: &mut ParseExecution<'_, 'scope>,
    buffers: &[Option<BufferData<'_>>],
    node_ids: &[Option<NodeLocalId>],
    instance_ids: &[Option<u32>],
    contract: &AnimationChannelContract<'_>,
    work: &mut ImportWorkScope<'_, 3>,
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
    let input_sources = accessor_provenance(ctx, buffers, sampler.input())?;
    let output_sources = accessor_provenance(ctx, buffers, sampler.output())?;
    let input_sources = input_sources.as_slice();
    let output_sources = output_sources.as_slice();
    let source_count = input_sources.len() + output_sources.len();
    let mut sources = [0; MAX_CHANNEL_PROVENANCE_SOURCES];
    sources[..input_sources.len()].copy_from_slice(input_sources);
    sources[input_sources.len()..source_count].copy_from_slice(output_sources);
    let sources = &mut sources[..source_count];
    sources.sort_unstable();
    let mut previous = None;
    for &source in sources.iter() {
        if previous != Some(source) {
            provenance.try_push_reserved(source)?;
            previous = Some(source);
        }
    }
    let input = super::accessor::bind(ctx, sampler.input(), buffers)?
        .scalars()
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

    let output = super::accessor::bind(ctx, sampler.output(), buffers)?;
    match contract.property {
        AnimationProperty::Translation | AnimationProperty::Scale => {
            let output = output
                .vectors::<3>()
                .ok_or_else(|| channel_error(ctx, contract, "output accessor shape is invalid"))?;
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
        AnimationProperty::Rotation => {
            let output = output.vectors::<4>().ok_or_else(|| {
                channel_error(ctx, contract, "rotation accessor shape is invalid")
            })?;
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
        AnimationProperty::MorphWeights => {
            let output = output
                .scalars()
                .ok_or_else(|| channel_error(ctx, contract, "weight accessor shape is invalid"))?;
            read_output_weights(
                ctx,
                contract,
                output,
                &mut values,
                &mut in_tangents,
                &mut out_tangents,
                work,
            )?;
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
    ctx: &mut ParseExecution<'_, 'scope>,
    contract: &AnimationChannelContract<'_>,
    mut input: impl Iterator<Item = f32>,
    work: &mut ImportWorkScope<'_, 3>,
) -> Result<(MeteredVec<'scope, f64>, f64)> {
    let mut times = try_vec_capacity(
        ctx,
        ImportStorageClass::SceneConstruction,
        contract.key_count,
        "gltf-animation-times",
    )?;
    let mut previous = None;
    for key in 0..contract.key_count {
        if key.is_multiple_of(GEOMETRY_POLL_INTERVAL) {
            ctx.poll()?;
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
    ctx: &mut ParseExecution<'_, '_>,
    contract: &AnimationChannelContract<'_>,
    mut output: impl Iterator<Item = f32>,
    values: &mut MeteredVec<'_, f32>,
    in_tangents: &mut MeteredVec<'_, f32>,
    out_tangents: &mut MeteredVec<'_, f32>,
    work: &mut ImportWorkScope<'_, 3>,
) -> Result<()> {
    for ordinal in 0..contract.output_count {
        if ordinal.is_multiple_of(GEOMETRY_POLL_INTERVAL) {
            ctx.poll()?;
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
    ctx: &mut ParseExecution<'_, '_>,
    contract: &AnimationChannelContract<'_>,
    mut output: impl Iterator<Item = [f32; N]>,
    values: &mut MeteredVec<'_, f32>,
    in_tangents: &mut MeteredVec<'_, f32>,
    out_tangents: &mut MeteredVec<'_, f32>,
    work: &mut ImportWorkScope<'_, 3>,
) -> Result<()> {
    if N != contract.value_width {
        return Err(ContentError::from(super::GltfFailure::internal(
            "animation reader value width changed after preflight",
        )));
    }
    for ordinal in 0..contract.output_count {
        if ordinal.is_multiple_of(GEOMETRY_POLL_INTERVAL) {
            ctx.poll()?;
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
    _operation: &'static str,
    values: &mut impl Iterator<Item = T>,
) -> Result<Option<T>> {
    Ok(values.next())
}

fn channel_error(
    ctx: &mut ParseExecution<'_, '_>,
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
    ctx: &mut ParseExecution<'_, '_>,
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
    _ctx: &mut ParseExecution<'_, '_>,
    animation_index: usize,
    channel_index: usize,
    reason: &'static str,
) -> ContentError {
    gltf_parse_error(format_args!(
        "glTF animation {animation_index} channel {channel_index} {reason}"
    ))
}

impl<'scope> AnimationStorage<'scope> {
    #[cfg(test)]
    pub(super) fn empty(ctx: &mut ParseExecution<'_, 'scope>) -> Self {
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

    pub(super) fn non_primary_provenance_count(
        &self,
        ctx: &mut ParseExecution<'_, '_>,
    ) -> Result<usize> {
        let mut count = 0usize;
        let work_group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
        for sources in [
            self.clip_provenance.as_slice(),
            self.channel_provenance.as_slice(),
        ] {
            ctx.with_work(
                &work_group,
                [to_u64(sources.len())],
                |ctx, work| -> Result<()> {
                    for (index, &source) in sources.iter().enumerate() {
                        super::poll_parser_loop(ctx, index)?;
                        count = count.checked_add(usize::from(source != 0)).ok_or_else(|| {
                            ctx.allocation_failure("gltf-animation-provenance-count")
                        })?;
                        work.complete([1])?;
                    }
                    Ok(())
                },
            )?;
        }
        Ok(count)
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

    pub(super) fn authoring_tables<'model>(
        &'model self,
        ctx: &mut ParseExecution<'_, 'scope>,
    ) -> Result<AnimationAuthoringTables<'scope, 'model>> {
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
                    AnimationExtrapolation::Constant,
                    AnimationExtrapolation::Constant,
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
        Ok(AnimationAuthoringTables { clips, channels })
    }
}

pub(super) struct AnimationAuthoringTables<'session, 'model> {
    clips: MeteredVec<'session, AnimationClipAuthoring<'model>>,
    channels: MeteredVec<'session, AnimationChannelAuthoring<'model>>,
}

impl AnimationAuthoringTables<'_, '_> {
    pub(super) fn batch(&self) -> AnimationAuthoringBatch<'_> {
        AnimationAuthoringBatch::new(self.clips.as_slice(), self.channels.as_slice())
    }
}
