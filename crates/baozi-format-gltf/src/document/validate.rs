//! Global schema/reference hooks. Selected interpretation remains in parser contracts.

use super::mesh_encoding::{
    MeshAttributeUse, MeshEncodingClass, mesh_attribute_encoding, optional_morph_semantic,
};
use super::schema::{self as s};
use crate::GltfFailure;
use baozi_import::{
    ImportStorageClass, ImportWorkClass, ImportWorkScope, MeteredVec, ParseExecution, ParseResult,
};
use std::cmp::Ordering;

pub(super) fn validate(
    ctx: &mut ParseExecution<'_, '_>,
    root: &s::Tables<'_>,
) -> ParseResult<bool> {
    let has_quantization = extensions(ctx, root)?;
    let fail = |reason, ctx: &mut ParseExecution<'_, '_>| {
        ctx.format_failure(GltfFailure::invalid_document(reason))
    };
    let reference =
        |id: s::Index, count: usize, ctx: &mut ParseExecution<'_, '_>| -> ParseResult<()> {
            ctx.charge_work(1)?;
            if id.value() < count {
                Ok(())
            } else {
                Err(fail("glTF core reference is out of range", ctx))
            }
        };
    if let Some(id) = root.scene {
        reference(id, root.scenes.len(), ctx)?;
    }
    for scene in &root.scenes {
        for &id in &scene.nodes {
            reference(id, root.nodes.len(), ctx)?;
        }
    }
    let work_group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
    for node in &root.nodes {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        for &id in &node.children {
            reference(id, root.nodes.len(), ctx)?;
        }
        if let Some(id) = node.mesh {
            reference(id, root.meshes.len(), ctx)?;
        }
        if let Some(id) = node.skin {
            reference(id, root.skins.len(), ctx)?;
        }
        if let Some(id) = node.camera {
            reference(id, root.cameras.len(), ctx)?;
        }
    }
    for view in &root.buffer_views {
        reference(view.buffer, root.buffers.len(), ctx)?;
    }
    for (index, accessor) in root.accessors.iter().enumerate() {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
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
            reference(id, root.buffer_views.len(), ctx)?;
        }
        if let Some(sparse) = &accessor.sparse {
            reference(sparse.indices.buffer_view, root.buffer_views.len(), ctx)?;
            reference(sparse.values.buffer_view, root.buffer_views.len(), ctx)?;
        }
    }
    for mesh in &root.meshes {
        for primitive in &mesh.primitives {
            ctx.with_work(
                &work_group,
                [1],
                |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
            )?;
            for (&semantic, &id) in &primitive.attributes {
                reference(id, root.accessors.len(), ctx)?;
                require_quantization(
                    ctx,
                    root,
                    id,
                    MeshAttributeUse::Base(semantic),
                    has_quantization,
                )?;
            }
            if let Some(id) = primitive.indices {
                reference(id, root.accessors.len(), ctx)?;
            }
            if let Some(id) = primitive.material {
                reference(id, root.materials.len(), ctx)?;
            }
            let position = primitive
                .attributes
                .get(&s::mesh::Semantic::Positions)
                .ok_or_else(|| fail("glTF primitive requires POSITION", ctx))?;
            let accessor = &root.accessors[position.value()];
            if accessor.min.is_none_or(|bounds| bounds.len != 3)
                || accessor.max.is_none_or(|bounds| bounds.len != 3)
            {
                return Err(ctx.format_failure(GltfFailure::invalid_accessor(
                    Some(position.value()),
                    "glTF POSITION accessor requires numeric VEC3 min and max",
                )));
            }
            if let Some(targets) = &primitive.targets {
                for target in targets {
                    ctx.with_work(
                        &work_group,
                        [1],
                        |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
                    )?;
                    for (semantic, id) in [
                        (s::mesh::Semantic::Positions, target.positions),
                        (s::mesh::Semantic::Normals, target.normals),
                        (s::mesh::Semantic::Tangents, target.tangents),
                    ] {
                        let Some(id) = id else { continue };
                        reference(id, root.accessors.len(), ctx)?;
                        require_quantization(
                            ctx,
                            root,
                            id,
                            MeshAttributeUse::Morph(semantic),
                            has_quantization,
                        )?;
                    }
                    for (name, id) in &target.other_attributes.0 {
                        reference(*id, root.accessors.len(), ctx)?;
                        ctx.with_work(
                            &work_group,
                            [name.len() as u64],
                            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
                        )?;
                        if let Some(semantic) = optional_morph_semantic(name.as_str()) {
                            require_quantization(
                                ctx,
                                root,
                                *id,
                                MeshAttributeUse::Morph(semantic),
                                has_quantization,
                            )?;
                        }
                    }
                }
            }
        }
    }
    for skin in &root.skins {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        for &id in &skin.joints {
            reference(id, root.nodes.len(), ctx)?;
        }
        if let Some(id) = skin.skeleton {
            reference(id, root.nodes.len(), ctx)?;
        }
        if let Some(id) = skin.inverse_bind_matrices {
            reference(id, root.accessors.len(), ctx)?;
        }
    }
    for material in &root.materials {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
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
            reference(info.index, root.textures.len(), ctx)?;
        }
    }
    for texture in &root.textures {
        reference(texture.source, root.images.len(), ctx)?;
        if let Some(id) = texture.sampler {
            reference(id, root.samplers.len(), ctx)?;
        }
    }
    for image in &root.images {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        if let Some(id) = image.buffer_view {
            reference(id, root.buffer_views.len(), ctx)?;
        }
    }
    for animation in &root.animations {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
        for sampler in &animation.samplers {
            reference(sampler.input, root.accessors.len(), ctx)?;
            reference(sampler.output, root.accessors.len(), ctx)?;
        }
        for channel in &animation.channels {
            reference(channel.sampler, animation.samplers.len(), ctx)?;
            if let Some(node) = channel.target.node {
                reference(node, root.nodes.len(), ctx)?;
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
    Ok(has_quantization)
}

fn require_quantization(
    ctx: &mut ParseExecution<'_, '_>,
    root: &s::Tables<'_>,
    id: s::Index,
    usage: MeshAttributeUse,
    has_quantization: bool,
) -> ParseResult<()> {
    let accessor = &root.accessors[id.value()];
    if mesh_attribute_encoding(usage, accessor.component_type, accessor.normalized).is_some_and(
        |encoding| {
            encoding.class == MeshEncodingClass::Quantization
                && encoding.dimensions.contains(&accessor.type_)
        },
    ) && !has_quantization
    {
        return Err(ctx.format_failure(GltfFailure::invalid_document(format_args!(
            "integer {} accessor requires KHR_mesh_quantization in extensionsUsed and extensionsRequired",
            usage.label(),
        ))));
    }
    Ok(())
}

fn extensions(ctx: &mut ParseExecution<'_, '_>, root: &s::Tables<'_>) -> ParseResult<bool> {
    let used = sorted_names(ctx, &root.extensions_used, "extensionsUsed")?;
    let required = sorted_names(ctx, &root.extensions_required, "extensionsRequired")?;
    for &name in required.as_slice() {
        if !contains_name(ctx, used.as_slice(), name)? {
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
    ctx.charge_work(u64::from(usize::BITS) * 2)?;
    let has_quantization = used
        .as_slice()
        .binary_search(&super::KHR_MESH_QUANTIZATION)
        .is_ok();
    if has_quantization
        && required
            .as_slice()
            .binary_search(&super::KHR_MESH_QUANTIZATION)
            .is_err()
    {
        return Err(ctx.format_failure(GltfFailure::invalid_document(
            "KHR_mesh_quantization in extensionsUsed must also be in extensionsRequired",
        )));
    }
    Ok(has_quantization)
}

fn contains_name(
    ctx: &mut ParseExecution<'_, '_>,
    names: &[&str],
    name: &str,
) -> ParseResult<bool> {
    ctx.with_work_group([(ImportWorkClass::WorkUnits, 0)], |ctx, work| {
        let mut left = 0;
        let mut right = names.len();
        while left < right {
            let middle = left + (right - left) / 2;
            match compare_names(ctx, work, names[middle], name)? {
                Ordering::Less => left = middle + 1,
                Ordering::Greater => right = middle,
                Ordering::Equal => return Ok(true),
            }
        }
        Ok(false)
    })
}

fn sorted_names<'scope, 'document>(
    ctx: &mut ParseExecution<'_, 'scope>,
    names: &'document [s::Text<'_>],
    field: &'static str,
) -> ParseResult<MeteredVec<'scope, &'document str>> {
    let mut sorted = ctx.metered_vec(
        ImportStorageClass::BackendDecode,
        "gltf-document-extension-names",
    );
    sorted.try_reserve(ctx, names.len())?;
    let work_group = ctx.bind_work_group([ImportWorkClass::WorkUnits])?;
    for (index, name) in names.iter().enumerate() {
        ctx.with_work(
            &work_group,
            [1],
            |_exec, work| -> baozi_import::ParseResult<()> { work.complete_all() },
        )?;
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
        ctx.with_work_group(
            [(ImportWorkClass::WorkUnits, 0)],
            |ctx, work| -> ParseResult<()> {
                let mut comparisons = 0usize;
                sorted.try_sort_unstable_by(|a, b| -> ParseResult<_> {
                    if comparisons.is_multiple_of(baozi_import::MAX_GEOMETRY_ACTIONS_BETWEEN_POLLS)
                    {
                        ctx.poll()?;
                    }
                    let ordering = compare_names(ctx, work, a, b)?;
                    comparisons += 1;
                    Ok(ordering)
                })?;
                Ok(())
            },
        )?;
    }
    for pair in sorted.as_slice().windows(2) {
        let ordering = ctx.with_work(&work_group, [0], |ctx, work| {
            compare_names(ctx, work, pair[0], pair[1])
        })?;
        if ordering == Ordering::Equal {
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

fn compare_names(
    ctx: &mut ParseExecution<'_, '_>,
    work: &mut ImportWorkScope<'_, 1>,
    left: &str,
    right: &str,
) -> ParseResult<Ordering> {
    for (index, (left, right)) in left.bytes().zip(right.bytes()).enumerate() {
        // Each comparison visits two raw bytes, independently of the sort's action cadence.
        if index.is_multiple_of(crate::parser::BYTE_POLL_INTERVAL / 2) {
            ctx.poll()?;
        }
        work.replenish(1)?;
        let ordering = left.cmp(&right);
        work.complete([1])?;
        #[cfg(test)]
        crate::control_tests::extension_pair_completed(index + 1);
        if ordering != Ordering::Equal {
            return Ok(ordering);
        }
    }
    work.replenish(1)?;
    let ordering = left.len().cmp(&right.len());
    work.complete([1])?;
    Ok(ordering)
}
