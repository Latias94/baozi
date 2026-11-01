//! Selection-aware checks over the existing admission scanner's raw source map.

use super::{
    GltfFailure, KHR_MATERIALS_UNLIT, NodeGraph, ParseScope, ResourceClosure, Result, animation,
    gltf_parse_error, to_u64,
};
use crate::source_map::{
    GltfSourceMap, SourceEntityId, SourceEntityKind, SourceExtensionKind, SourcePayloadKind,
    SourcePayloadSite,
};

pub(super) fn preflight(
    ctx: &ParseScope<'_>,
    map: &GltfSourceMap<'_>,
    document: &crate::document::Document<'_>,
    graph: &NodeGraph<'_, '_>,
    closure: &ResourceClosure<'_>,
    animations: &animation::AnimationContracts<'_, '_>,
) -> Result<()> {
    for (index, image) in document.tables().images.iter().enumerate() {
        ctx.charge_work(1)?;
        if !closure.images.as_slice()[index] {
            continue;
        }
        if image.uri.is_some() == image.buffer_view.is_some() {
            return Err(GltfFailure::invalid_document(
                "selected glTF image requires exactly one URI or bufferView source",
            )
            .into());
        }
        if image.buffer_view.is_some() && image.mime_type.is_none() {
            return Err(GltfFailure::invalid_document(
                "selected glTF bufferView image requires mimeType",
            )
            .into());
        }
    }
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
    for (index, mesh) in document.tables().meshes.iter().enumerate() {
        ctx.charge_work(1)?;
        if closure.meshes.as_slice()[index] {
            for primitive in 0..mesh.primitives.len() {
                required(SourceEntityKind::Primitive, Some(index), primitive)?;
            }
        }
    }
    for (index, clip) in document.tables().animations.iter().enumerate() {
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
    ctx.charge_work(to_u64(document.tables().extensions_used.len()))?;
    let unlit_declared = document
        .tables()
        .extensions_used
        .iter()
        .any(|name| name == KHR_MATERIALS_UNLIT);
    let mut extras = super::extras::ExtrasPreflight::default();
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
                    extras.add(ctx, payload)?;
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
                SourcePayloadKind::Extension(SourceExtensionKind::KhrMaterialsUnlit)
                    if entity.id.kind == SourceEntityKind::Material
                        && payload.site == SourcePayloadSite::Entity =>
                {
                    let offset = usize::try_from(range.byte_offset).ok();
                    if offset.and_then(|offset| ctx.primary_bytes().get(offset)) != Some(&b'{') {
                        return Err(GltfFailure::invalid_json(
                            "KHR_materials_unlit",
                            range.byte_offset,
                            range.byte_length,
                            "glTF KHR_materials_unlit must be an object",
                        )
                        .into());
                    }
                    if !unlit_declared {
                        return Err(GltfFailure::invalid_json(
                            "KHR_materials_unlit",
                            range.byte_offset,
                            range.byte_length,
                            "glTF KHR_materials_unlit must be declared in extensionsUsed",
                        )
                        .into());
                    }
                }
                SourcePayloadKind::ExtensionProperty => {
                    return Err(fail(
                        "extension-property",
                        "glTF extension property is outside the admitted profile",
                    )
                    .into());
                }
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
    document: &crate::document::Document<'_>,
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
                    .tables()
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
