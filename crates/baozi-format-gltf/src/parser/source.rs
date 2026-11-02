//! Selection-aware checks over the existing admission scanner's raw source map.

use super::{
    GltfFailure, NodeGraph, ParseExecution, ResourceClosure, Result, animation, gltf_parse_error,
};
use crate::document::{
    KHR_LIGHTS_PUNCTUAL, KHR_MATERIALS_CLEARCOAT, KHR_MATERIALS_EMISSIVE_STRENGTH,
    KHR_MATERIALS_SHEEN, KHR_MATERIALS_UNLIT, KHR_TEXTURE_TRANSFORM,
};
use crate::source_map::{
    GltfSourceMap, SourceEntityId, SourceEntityKind, SourceExtensionKind, SourcePayload,
    SourcePayloadKind, SourcePayloadSite,
};
use baozi_import::ImportWorkClass;

pub(super) fn preflight(
    ctx: &mut ParseExecution<'_, '_>,
    map: &GltfSourceMap<'_>,
    document: &crate::document::Document<'_>,
    graph: &NodeGraph<'_, '_>,
    closure: &ResourceClosure<'_>,
    animations: &animation::AnimationContracts<'_, '_>,
) -> Result<()> {
    let work_group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
    for (index, image) in document.tables().images.iter().enumerate() {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
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
    let required =
        |kind, parent_index, source_index, ctx: &mut ParseExecution<'_, '_>| -> Result<()> {
            let comparisons = u64::from(usize::BITS - map.entities().len().leading_zeros()) + 1;
            ctx.charge_work(comparisons)?;
            map.get(SourceEntityId {
                kind,
                parent_index,
                source_index,
            })
            .ok_or_else(|| gltf_parse_error("selected glTF entity has no raw source range"))?;
            Ok(())
        };
    required(SourceEntityKind::Document, None, 0, ctx)?;
    if let Some(index) = graph.selection_outcome.selected_scene_index() {
        required(SourceEntityKind::Scene, None, index, ctx)?;
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
        (SourceEntityKind::Light, closure.lights.as_slice()),
        (
            SourceEntityKind::Animation,
            animations.selected_animations.as_slice(),
        ),
    ] {
        for (index, selected) in selection.iter().enumerate() {
            ctx.with_work(
                &work_group,
                [1],
                |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
            )?;
            if *selected {
                required(kind, None, index, ctx)?;
            }
        }
    }
    for (index, mesh) in document.tables().meshes.iter().enumerate() {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        if closure.meshes.as_slice()[index] {
            for primitive in 0..mesh.primitives.len() {
                required(SourceEntityKind::Primitive, Some(index), primitive, ctx)?;
            }
        }
    }
    for (index, clip) in document.tables().animations.iter().enumerate() {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        if !animations.selected_animations.as_slice()[index] {
            continue;
        }
        for channel in 0..clip.channels.len() {
            required(
                SourceEntityKind::AnimationChannel,
                Some(index),
                channel,
                ctx,
            )?;
        }
        let range = &closure.animation_sampler_ranges.as_slice()[index];
        for sampler in 0..clip.samplers.len() {
            ctx.with_work(
                &work_group,
                [1],
                |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
            )?;
            if closure.animation_samplers.as_slice()[range.start + sampler] {
                required(
                    SourceEntityKind::AnimationSampler,
                    Some(index),
                    sampler,
                    ctx,
                )?;
            }
        }
    }

    // Payload ownership comes from scanner contexts, so unknown data cannot disappear in serde.
    let mut unlit_declared = false;
    let mut strength_declared = false;
    let mut clearcoat_declared = false;
    let mut sheen_declared = false;
    let mut transform_declared = false;
    let mut lights_declared = false;
    for name in &document.tables().extensions_used {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        match name.as_str() {
            KHR_MATERIALS_UNLIT => unlit_declared = true,
            KHR_MATERIALS_EMISSIVE_STRENGTH => strength_declared = true,
            KHR_MATERIALS_CLEARCOAT => clearcoat_declared = true,
            KHR_MATERIALS_SHEEN => sheen_declared = true,
            KHR_TEXTURE_TRANSFORM => transform_declared = true,
            KHR_LIGHTS_PUNCTUAL => lights_declared = true,
            _ => {}
        }
    }
    let mut extras = super::extras::ExtrasPreflight::default();
    for entity in map.entities() {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        let entity_selected = selected(entity.id, document, graph, closure, animations);
        // Node absence only permits the core ignore rule. Inspect this channel's extension
        // containers and members even in an omitted clip, without retaining its extras.
        let core_ignored_channel = entity.id.kind == SourceEntityKind::AnimationChannel
            && entity
                .id
                .parent_index
                .and_then(|animation| document.tables().animations.get(animation))
                .and_then(|animation| animation.channels.get(entity.id.source_index))
                .is_some_and(|channel| channel.target.node.is_none());
        if !entity_selected && !core_ignored_channel {
            continue;
        }
        // Every payload occupies primary bytes, so the byte length bounds both searches even
        // though the scanner intentionally keeps its payload storage private.
        let comparisons = u64::from(usize::BITS - ctx.primary_bytes().len().leading_zeros()) + 1;
        ctx.with_work(
            &work_group,
            [2 * comparisons],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        for payload in map.payloads_for(entity.id) {
            ctx.with_work(
                &work_group,
                [1],
                |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
            )?;
            let range = payload.value_range;
            let fail = |subject, reason| {
                GltfFailure::unsupported_at(subject, range.byte_offset, range.byte_length, reason)
            };
            match payload.kind {
                SourcePayloadKind::Extras => {
                    if entity_selected {
                        extras.add(ctx, payload)?;
                    }
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
                    if entity.id.kind == SourceEntityKind::Material
                        && matches!(
                            payload.site,
                            SourcePayloadSite::BaseColorTexture
                                | SourcePayloadSite::MetallicRoughnessTexture
                                | SourcePayloadSite::NormalTexture
                                | SourcePayloadSite::OcclusionTexture
                                | SourcePayloadSite::EmissiveTexture
                                | SourcePayloadSite::ClearcoatTexture
                                | SourcePayloadSite::ClearcoatRoughnessTexture
                                | SourcePayloadSite::ClearcoatNormalTexture
                                | SourcePayloadSite::SheenColorTexture
                                | SourcePayloadSite::SheenRoughnessTexture
                        ) =>
                {
                    validate_extension_payload(
                        ctx,
                        payload,
                        KHR_TEXTURE_TRANSFORM,
                        transform_declared,
                    )?;
                }
                SourcePayloadKind::Extension(SourceExtensionKind::KhrLightsPunctual)
                    if matches!(
                        entity.id.kind,
                        SourceEntityKind::Document | SourceEntityKind::Node
                    ) && payload.site == SourcePayloadSite::Entity =>
                {
                    validate_extension_payload(ctx, payload, KHR_LIGHTS_PUNCTUAL, lights_declared)?;
                }
                SourcePayloadKind::Extension(kind)
                    if entity.id.kind == SourceEntityKind::Material
                        && payload.site == SourcePayloadSite::Entity =>
                {
                    let (name, declared) = match kind {
                        SourceExtensionKind::KhrMaterialsUnlit => {
                            (KHR_MATERIALS_UNLIT, unlit_declared)
                        }
                        SourceExtensionKind::KhrMaterialsEmissiveStrength => {
                            (KHR_MATERIALS_EMISSIVE_STRENGTH, strength_declared)
                        }
                        SourceExtensionKind::KhrMaterialsClearcoat => {
                            (KHR_MATERIALS_CLEARCOAT, clearcoat_declared)
                        }
                        SourceExtensionKind::KhrMaterialsSheen => {
                            (KHR_MATERIALS_SHEEN, sheen_declared)
                        }
                        _ => {
                            return Err(fail(
                                "extension-payload",
                                "glTF extension payload is outside the admitted profile",
                            )
                            .into());
                        }
                    };
                    validate_extension_payload(ctx, payload, name, declared)?;
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

fn validate_extension_payload(
    ctx: &mut ParseExecution<'_, '_>,
    payload: &SourcePayload,
    name: &'static str,
    declared: bool,
) -> Result<()> {
    let range = payload.value_range;
    let offset = usize::try_from(range.byte_offset).ok();
    if offset.and_then(|offset| ctx.primary_bytes().get(offset)) != Some(&b'{') {
        return Err(GltfFailure::invalid_json(
            name,
            range.byte_offset,
            range.byte_length,
            format_args!("glTF {name} must be an object"),
        )
        .into());
    }
    if !declared {
        return Err(GltfFailure::invalid_json(
            name,
            range.byte_offset,
            range.byte_length,
            format_args!("glTF {name} must be declared in extensionsUsed"),
        )
        .into());
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
        SourceEntityKind::Light => at(closure.lights.as_slice()),
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
                    .and_then(|channel| channel.target.node)
                    .and_then(|node| graph.selected.as_slice().get(node.value()))
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
