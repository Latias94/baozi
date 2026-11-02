//! Global schema/reference hooks. Selected interpretation remains in parser contracts.

use super::schema::{self as s};
use crate::GltfFailure;
use baozi_import::{ImportStorageClass, ImportWorkClass, MeteredVec, ParseResult, ParseScope};

pub(super) fn validate(ctx: &ParseScope<'_>, root: &s::Tables<'_>) -> ParseResult<()> {
    extensions(ctx, root)?;
    let fail = |reason| ctx.format_failure(GltfFailure::invalid_document(reason));
    let reference = |id: s::Index, count: usize| -> ParseResult<()> {
        ctx.charge_work(1)?;
        if id.value() < count {
            Ok(())
        } else {
            Err(fail("glTF core reference is out of range"))
        }
    };
    if let Some(id) = root.scene {
        reference(id, root.scenes.len())?;
    }
    for scene in &root.scenes {
        for &id in &scene.nodes {
            reference(id, root.nodes.len())?;
        }
    }
    for node in &root.nodes {
        ctx.charge_work(1)?;
        for &id in &node.children {
            reference(id, root.nodes.len())?;
        }
        if let Some(id) = node.mesh {
            reference(id, root.meshes.len())?;
        }
        if let Some(id) = node.skin {
            reference(id, root.skins.len())?;
        }
        if let Some(id) = node.camera {
            reference(id, root.cameras.len())?;
        }
    }
    for view in &root.buffer_views {
        reference(view.buffer, root.buffers.len())?;
    }
    for (index, accessor) in root.accessors.iter().enumerate() {
        ctx.charge_work(1)?;
        if accessor.count == 0 {
            return Err(ctx.format_failure(GltfFailure::invalid_accessor(
                Some(index),
                "accessor count must be positive",
            )));
        }
        if accessor.buffer_view.is_none() && accessor.byte_offset.is_some() {
            return Err(ctx.format_failure(GltfFailure::invalid_accessor(
                Some(index),
                "accessor without bufferView must not define byteOffset",
            )));
        }
        if let Some(id) = accessor.buffer_view {
            reference(id, root.buffer_views.len())?;
        }
        if let Some(sparse) = &accessor.sparse {
            reference(sparse.indices.buffer_view, root.buffer_views.len())?;
            reference(sparse.values.buffer_view, root.buffer_views.len())?;
        }
    }
    for mesh in &root.meshes {
        for primitive in &mesh.primitives {
            ctx.charge_work(1)?;
            for &id in primitive.attributes.values() {
                reference(id, root.accessors.len())?;
            }
            if let Some(id) = primitive.indices {
                reference(id, root.accessors.len())?;
            }
            if let Some(id) = primitive.material {
                reference(id, root.materials.len())?;
            }
            let position = primitive
                .attributes
                .get(&s::mesh::Semantic::Positions)
                .ok_or_else(|| fail("glTF primitive requires POSITION"))?;
            let accessor = &root.accessors[position.value()];
            if accessor.min.and_then(s::accessor::Bounds::vec3).is_none()
                || accessor.max.and_then(s::accessor::Bounds::vec3).is_none()
            {
                return Err(fail(
                    "glTF POSITION accessor requires numeric VEC3 min and max",
                ));
            }
            if let Some(targets) = &primitive.targets {
                for target in targets {
                    ctx.charge_work(1)?;
                    for id in [target.positions, target.normals, target.tangents]
                        .into_iter()
                        .flatten()
                    {
                        reference(id, root.accessors.len())?;
                    }
                    for (_, id) in &target.other_attributes.0 {
                        reference(*id, root.accessors.len())?;
                    }
                }
            }
        }
    }
    for skin in &root.skins {
        ctx.charge_work(1)?;
        for &id in &skin.joints {
            reference(id, root.nodes.len())?;
        }
        if let Some(id) = skin.skeleton {
            reference(id, root.nodes.len())?;
        }
        if let Some(id) = skin.inverse_bind_matrices {
            reference(id, root.accessors.len())?;
        }
    }
    for material in &root.materials {
        ctx.charge_work(1)?;
        for info in [
            &material.pbr_metallic_roughness.base_color_texture,
            &material.pbr_metallic_roughness.metallic_roughness_texture,
            &material.normal_texture,
            &material.occlusion_texture,
            &material.emissive_texture,
        ]
        .into_iter()
        .flatten()
        {
            reference(info.index, root.textures.len())?;
        }
    }
    for texture in &root.textures {
        reference(texture.source, root.images.len())?;
        if let Some(id) = texture.sampler {
            reference(id, root.samplers.len())?;
        }
    }
    for image in &root.images {
        ctx.charge_work(1)?;
        if let Some(id) = image.buffer_view {
            reference(id, root.buffer_views.len())?;
        }
    }
    for animation in &root.animations {
        ctx.charge_work(1)?;
        for sampler in &animation.samplers {
            reference(sampler.input, root.accessors.len())?;
            reference(sampler.output, root.accessors.len())?;
        }
        for channel in &animation.channels {
            reference(channel.sampler, animation.samplers.len())?;
            if let Some(node) = channel.target.node {
                reference(node, root.nodes.len())?;
                if root.nodes[node.value()].matrix.is_some()
                    && !matches!(
                        channel.target.path,
                        s::animation::Property::MorphTargetWeights
                    )
                {
                    return Err(ctx.format_failure(GltfFailure::invalid_document(
                        "glTF TRS animation targets a matrix-authored node",
                    )));
                }
            }
        }
    }
    Ok(())
}

fn extensions(ctx: &ParseScope<'_>, root: &s::Tables<'_>) -> ParseResult<()> {
    let used = sorted_names(ctx, &root.extensions_used, "extensionsUsed")?;
    let required = sorted_names(ctx, &root.extensions_required, "extensionsRequired")?;
    for &name in required.as_slice() {
        ctx.charge_work(1)?;
        if used.as_slice().binary_search(&name).is_err() {
            return Err(
                ctx.format_failure(GltfFailure::invalid_document(format_args!(
                    "required extension {name:?} missing from extensionsUsed"
                ))),
            );
        }
        if !super::is_supported_extension(name) {
            return Err(ctx.format_failure(GltfFailure::unsupported(
                "required-extension",
                format_args!("glTF required extension {name:?} is not supported"),
            )));
        }
    }
    Ok(())
}

fn sorted_names<'scope, 'document>(
    ctx: &'scope ParseScope<'_>,
    names: &'document [s::Text<'_>],
    field: &'static str,
) -> ParseResult<MeteredVec<'scope, &'document str>> {
    let mut sorted = ctx.metered_vec(
        ImportStorageClass::BackendDecode,
        "gltf-document-extension-names",
    );
    sorted.try_reserve(names.len())?;
    for (index, name) in names.iter().enumerate() {
        ctx.charge_work(1)?;
        if name.is_empty() {
            return Err(
                ctx.format_failure(GltfFailure::invalid_document(format_args!(
                    "glTF {field} entry {index} has an empty extension name"
                ))),
            );
        }
        sorted.try_push_reserved(name.as_str())?;
    }
    if sorted.len() > 1 {
        let levels = u64::from(usize::BITS - (sorted.len() - 1).leading_zeros());
        let bound =
            (sorted.len() as u64).saturating_mul(levels.saturating_mul(4).saturating_add(4));
        ctx.with_work(
            ImportWorkClass::WorkUnits,
            bound,
            |work| -> ParseResult<()> {
                let mut comparisons = 0usize;
                sorted.try_sort_unstable_by(|a, b| -> ParseResult<_> {
                    if comparisons.is_multiple_of(4096) {
                        work.poll()?;
                    }
                    comparisons += 1;
                    Ok(a.cmp(b))
                })?;
                work.complete_all()
            },
        )?;
    }
    for pair in sorted.as_slice().windows(2) {
        ctx.charge_work(1)?;
        if pair[0] == pair[1] {
            return Err(
                ctx.format_failure(GltfFailure::invalid_document(format_args!(
                    "glTF {field} contains duplicate extension {:?}",
                    pair[1]
                ))),
            );
        }
    }
    Ok(sorted)
}
