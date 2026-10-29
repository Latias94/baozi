//! Selection-aware checks over the existing admission scanner's raw source map.

use super::{
    GltfFailure, NodeGraph, ParseScope, ResourceClosure, Result, animation, gltf_parse_error,
};
use crate::source_map::{
    GltfSourceMap, SourceEntityId, SourceEntityKind, SourceExtensionKind, SourcePayloadKind,
    SourcePayloadSite,
};

pub(super) fn preflight(
    ctx: &ParseScope<'_>,
    map: &GltfSourceMap<'_>,
    document: &gltf::Document,
    graph: &NodeGraph<'_, '_>,
    closure: &ResourceClosure<'_>,
    animations: &animation::AnimationContracts<'_, '_>,
) -> Result<()> {
    let required = |kind, parent_index, source_index| -> Result<()> {
        let comparisons = u64::from(usize::BITS - map.entities().len().leading_zeros()) + 1;
        ctx.charge_work(comparisons)?;
        map.get(SourceEntityId {
            kind,
            parent_index,
            source_index,
        })
        .ok_or_else(|| gltf_parse_error(ctx, "selected glTF entity has no raw source range"))?;
        Ok(())
    };
    required(SourceEntityKind::Document, None, 0)?;
    if let Some(index) = graph.selection_outcome.selected_scene_index() {
        required(SourceEntityKind::Scene, None, index)?;
    }
    for (kind, selection) in [
        (SourceEntityKind::Node, graph.selected.as_slice()),
        (SourceEntityKind::Mesh, closure.meshes.as_slice()),
        (SourceEntityKind::Material, closure.materials.as_slice()),
        (SourceEntityKind::Texture, closure.textures.as_slice()),
        (SourceEntityKind::Image, closure.images.as_slice()),
        (SourceEntityKind::Sampler, closure.samplers.as_slice()),
        (SourceEntityKind::Skin, closure.skins.as_slice()),
        (SourceEntityKind::Buffer, closure.buffers.as_slice()),
        (
            SourceEntityKind::BufferView,
            closure.buffer_views.as_slice(),
        ),
        (SourceEntityKind::Accessor, closure.accessors.as_slice()),
        (SourceEntityKind::Camera, closure.cameras.as_slice()),
        (
            SourceEntityKind::Animation,
            animations.selected_animations.as_slice(),
        ),
    ] {
        for (index, selected) in selection.iter().enumerate() {
            ctx.charge_work(1)?;
            if *selected {
                required(kind, None, index)?;
            }
        }
    }
    for (index, mesh) in document.as_json().meshes.iter().enumerate() {
        ctx.charge_work(1)?;
        if closure.meshes.as_slice()[index] {
            for primitive in 0..mesh.primitives.len() {
                required(SourceEntityKind::Primitive, Some(index), primitive)?;
            }
        }
    }
    for (index, clip) in document.as_json().animations.iter().enumerate() {
        ctx.charge_work(1)?;
        if !animations.selected_animations.as_slice()[index] {
            continue;
        }
        for channel in 0..clip.channels.len() {
            required(SourceEntityKind::AnimationChannel, Some(index), channel)?;
        }
        let range = &closure.animation_sampler_ranges.as_slice()[index];
        for sampler in 0..clip.samplers.len() {
            ctx.charge_work(1)?;
            if closure.animation_samplers.as_slice()[range.start + sampler] {
                required(SourceEntityKind::AnimationSampler, Some(index), sampler)?;
            }
        }
    }

    // Payload ownership comes from scanner contexts, so unknown data cannot disappear in serde.
    for entity in map.entities() {
        ctx.charge_work(1)?;
        if !selected(entity.id, document, graph, closure, animations) {
            continue;
        }
        // Every payload occupies primary bytes, so the byte length bounds both searches even
        // though the scanner intentionally keeps its payload storage private.
        let comparisons = u64::from(usize::BITS - ctx.primary_bytes().len().leading_zeros()) + 1;
        ctx.charge_work(2 * comparisons)?;
        for payload in map.payloads_for(entity.id) {
            ctx.charge_work(1)?;
            let range = payload.value_range;
            let fail = |subject, reason| {
                GltfFailure::unsupported_at(subject, range.byte_offset, range.byte_length, reason)
            };
            match payload.kind {
                SourcePayloadKind::Extras => {
                    return Err(
                        fail("extras", "glTF extras are outside the admitted profile").into(),
                    );
                }
                SourcePayloadKind::Extensions => {
                    let offset = usize::try_from(range.byte_offset).ok();
                    if offset.and_then(|offset| ctx.primary_bytes().get(offset)) != Some(&b'{') {
                        return Err(GltfFailure::invalid_json(
                            "extensions",
                            range.byte_offset,
                            range.byte_length,
                            "glTF extensions must be an object",
                        )
                        .into());
                    }
                }
                SourcePayloadKind::Extension(SourceExtensionKind::KhrTextureTransform)
                    if matches!(
                        payload.site,
                        SourcePayloadSite::BaseColorTexture
                            | SourcePayloadSite::MetallicRoughnessTexture
                            | SourcePayloadSite::NormalTexture
                            | SourcePayloadSite::OcclusionTexture
                            | SourcePayloadSite::EmissiveTexture
                    ) => {}
                SourcePayloadKind::Extension(_) => {
                    return Err(fail(
                        "extension-payload",
                        "glTF extension payload is outside the admitted profile",
                    )
                    .into());
                }
            }
        }
    }
    Ok(())
}

pub(super) fn selected(
    entity: SourceEntityId,
    document: &gltf::Document,
    graph: &NodeGraph<'_, '_>,
    closure: &ResourceClosure<'_>,
    animations: &animation::AnimationContracts<'_, '_>,
) -> bool {
    let index = entity.source_index;
    let at = |selection: &[bool]| selection.get(index).copied().unwrap_or(false);
    match entity.kind {
        SourceEntityKind::Document => true,
        SourceEntityKind::Scene => graph.selection_outcome.selected_scene_index() == Some(index),
        SourceEntityKind::Node => at(graph.selected.as_slice()),
        SourceEntityKind::Mesh => at(closure.meshes.as_slice()),
        SourceEntityKind::Primitive => entity.parent_index.is_some_and(|mesh| {
            closure
                .meshes
                .as_slice()
                .get(mesh)
                .copied()
                .unwrap_or(false)
        }),
        SourceEntityKind::Material => at(closure.materials.as_slice()),
        SourceEntityKind::Texture => at(closure.textures.as_slice()),
        SourceEntityKind::Image => at(closure.images.as_slice()),
        SourceEntityKind::Sampler => at(closure.samplers.as_slice()),
        SourceEntityKind::Skin => at(closure.skins.as_slice()),
        SourceEntityKind::Buffer => at(closure.buffers.as_slice()),
        SourceEntityKind::BufferView => at(closure.buffer_views.as_slice()),
        SourceEntityKind::Accessor => at(closure.accessors.as_slice()),
        SourceEntityKind::Camera => at(closure.cameras.as_slice()),
        SourceEntityKind::Animation => at(animations.selected_animations.as_slice()),
        SourceEntityKind::AnimationChannel => entity.parent_index.is_some_and(|animation| {
            let selected_clip = animations
                .selected_animations
                .as_slice()
                .get(animation)
                .copied()
                .unwrap_or(false);
            selected_clip
                && document
                    .as_json()
                    .animations
                    .get(animation)
                    .and_then(|clip| clip.channels.get(index))
                    .and_then(|channel| graph.selected.as_slice().get(channel.target.node.value()))
                    .copied()
                    .unwrap_or(false)
        }),
        SourceEntityKind::AnimationSampler => entity
            .parent_index
            .and_then(|animation| closure.animation_sampler_ranges.as_slice().get(animation))
            .and_then(|range| range.start.checked_add(index))
            .and_then(|index| closure.animation_samplers.as_slice().get(index))
            .copied()
            .unwrap_or(false),
    }
}
