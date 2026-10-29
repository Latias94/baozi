//! Selected source identities and their canonical owners, independent of decoder lifetimes.

use super::{
    ContentError, GltfFailure, PreparedDocument, Result, authoring_u32, gltf_parse_error,
    model::NormalizedGltf,
};
use crate::source_map::{GltfSourceMap, SourceEntity, SourceEntityId, SourceEntityKind};
use baozi_import::authoring::{
    AttributeSemantic, ExtensionSchemaId, ProfileOwner, SourceLocatorValue, SourceObjectLocator,
};
use baozi_import::{
    CapacityPlan, ImportStorageClass, MeteredOpaqueAllowance, MeteredVec, ParseScope,
};
use gltf::json::{self, validation::Checked};

const SOURCE_SCHEMA: &str = "org.baozi.gltf:source-object";

/// Schema v1 has five ordered fields: `kind;parent;index;offset;length`.
/// Fields are labelled, decimal integers use no leading zeroes, and `-` means no parent.
/// Offsets include the GLB JSON chunk's raw base, so every range addresses primary bytes.
/// Object identities never contain names or canonical compact ordinals.
pub(super) struct SelectedSource<'scope> {
    pub(super) entity: SourceEntity,
    pub(super) locator: SourceObjectLocator,
    // The locator's String drops before the allowance for its actual capacity.
    _storage: MeteredOpaqueAllowance<'scope>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct OwnerSourceLink {
    pub(super) owner: ProfileOwner,
    pub(super) source: SourceEntityId,
}

/// Only selected declarations survive normalization. The session owns the primary snapshot;
/// this inventory owns the locators and their admitted allocations until authoring completes.
pub(super) struct SelectedSourceProfile<'scope> {
    entries: MeteredVec<'scope, SelectedSource<'scope>>,
    links: MeteredVec<'scope, OwnerSourceLink>,
}

pub(super) const fn entity(
    kind: SourceEntityKind,
    parent_index: Option<usize>,
    source_index: usize,
) -> SourceEntityId {
    SourceEntityId {
        kind,
        parent_index,
        source_index,
    }
}

pub(super) const DOCUMENT: SourceEntityId = entity(SourceEntityKind::Document, None, 0);

impl<'scope> SelectedSourceProfile<'scope> {
    pub(super) fn from_source_map(
        ctx: &'scope ParseScope<'_>,
        map: &GltfSourceMap<'_>,
        selected: impl Fn(SourceEntityId) -> bool,
    ) -> Result<Self> {
        let mut entries = ctx.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-selected-source-identities",
        );
        for &source in map.entities() {
            ctx.charge_work(1)?;
            if selected(source.id) {
                entries.try_push(source_locator(ctx, source)?)?;
            }
        }
        let profile = Self {
            entries,
            links: ctx.metered_vec(
                ImportStorageClass::FormatNormalization,
                "gltf-selected-source-owner-links",
            ),
        };
        profile.required(ctx, DOCUMENT)?;
        Ok(profile)
    }

    pub(super) fn entries(&self) -> &[SelectedSource<'scope>] {
        self.entries.as_slice()
    }

    pub(super) fn link_count(&self) -> usize {
        self.links.len()
    }

    pub(super) fn required(
        &self,
        ctx: &ParseScope<'_>,
        id: SourceEntityId,
    ) -> Result<&SelectedSource<'scope>> {
        charge_lookup(ctx, self.entries.len(), 1)?;
        self.entries
            .as_slice()
            .binary_search_by_key(&id, |source| source.entity.id)
            .ok()
            .and_then(|index| self.entries.as_slice().get(index))
            .ok_or_else(|| {
                ContentError::from(GltfFailure::internal(
                    "selected glTF owner has no admitted source identity",
                ))
            })
    }

    pub(super) fn link(
        &mut self,
        ctx: &ParseScope<'_>,
        owner: ProfileOwner,
        source: SourceEntityId,
    ) -> Result<()> {
        ctx.charge_work(1)?;
        self.required(ctx, source)?;
        self.links.try_push(OwnerSourceLink { owner, source })?;
        Ok(())
    }

    pub(super) fn finish(&mut self, ctx: &ParseScope<'_>) -> Result<()> {
        self.links
            .try_sort_unstable_by(|left, right| -> Result<_> {
                ctx.charge_work(1)?;
                Ok(left.cmp(right))
            })?;
        let retained = self
            .links
            .try_partition_dedup_by(|left, right| -> Result<_> {
                ctx.charge_work(1)?;
                Ok(left == right)
            })?;
        self.links.truncate(retained);
        Ok(())
    }

    pub(super) fn for_owner(
        &self,
        ctx: &ParseScope<'_>,
        owner: ProfileOwner,
    ) -> Result<&[OwnerSourceLink]> {
        let links = self.links.as_slice();
        charge_lookup(ctx, links.len(), 2)?;
        let start = links.partition_point(|link| link.owner < owner);
        let end = links.partition_point(|link| link.owner <= owner);
        if start == end {
            return Err(GltfFailure::internal("glTF canonical owner has no source mapping").into());
        }
        Ok(&links[start..end])
    }

    /// Resource leaves retain their existing indices. The selected document reuses leaf zero;
    /// all other selected declarations follow the resource leaves in source-identity order.
    pub(super) fn provenance(
        &self,
        ctx: &ParseScope<'_>,
        resource_count: usize,
        id: SourceEntityId,
    ) -> Result<u32> {
        charge_lookup(ctx, self.entries.len(), 1)?;
        let index = self
            .entries
            .as_slice()
            .binary_search_by_key(&id, |source| source.entity.id)
            .map_err(|_| {
                GltfFailure::internal("glTF source mapping references a missing identity")
            })?;
        if id == DOCUMENT {
            return Ok(0);
        }
        resource_count
            .checked_add(index)
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| u32::try_from(index).ok())
            .ok_or_else(|| GltfFailure::internal("glTF source identity count exceeds u32").into())
    }
}

/// Binary search and partition_point perform at most bit_length(len) + 1 comparisons.
/// Charge that bound before entering the bounded, allocation-free standard-library search.
fn charge_lookup(ctx: &ParseScope<'_>, length: usize, searches: u64) -> Result<()> {
    let comparisons = u64::from(usize::BITS - length.leading_zeros()) + 1;
    ctx.charge_work(comparisons * searches)?;
    Ok(())
}

fn source_locator<'scope>(
    ctx: &'scope ParseScope<'_>,
    source: SourceEntity,
) -> Result<SelectedSource<'scope>> {
    let end = source
        .range
        .byte_offset
        .checked_add(source.range.byte_length);
    if source.range.byte_length == 0
        || end.is_none_or(|end| end > super::to_u64(ctx.primary_bytes().len()))
    {
        return Err(gltf_parse_error(
            ctx,
            "selected glTF source range is outside primary bytes",
        ));
    }
    let kind = match source.id.kind {
        SourceEntityKind::Document => "document",
        SourceEntityKind::Scene => "scene",
        SourceEntityKind::Node => "node",
        SourceEntityKind::Mesh => "mesh",
        SourceEntityKind::Primitive => "primitive",
        SourceEntityKind::Material => "material",
        SourceEntityKind::Texture => "texture",
        SourceEntityKind::Image => "image",
        SourceEntityKind::Sampler => "sampler",
        SourceEntityKind::Skin => "skin",
        SourceEntityKind::Buffer => "buffer",
        SourceEntityKind::BufferView => "buffer-view",
        SourceEntityKind::Accessor => "accessor",
        SourceEntityKind::Animation => "animation",
        SourceEntityKind::AnimationChannel => "animation-channel",
        SourceEntityKind::AnimationSampler => "animation-sampler",
        SourceEntityKind::Camera => "camera",
    };
    let mut label = ctx.metered_string(
        ImportStorageClass::FormatNormalization,
        "gltf-source-object-locator-label",
    );
    label.try_write_fmt(format_args!("kind={kind};parent="))?;
    match source.id.parent_index {
        Some(index) => label.try_push_usize(index)?,
        None => label.try_push_str("-")?,
    }
    label.try_write_fmt(format_args!(
        ";index={};offset={};length={}",
        source.id.source_index, source.range.byte_offset, source.range.byte_length,
    ))?;
    let plan = CapacityPlan::for_type::<u8>(0, 0, label.len())
        .map_err(|_| ctx.allocation_failure("gltf-source-object-locator"))?;
    let envelope = plan
        .envelope_delta_bytes()
        .map_err(|_| ctx.allocation_failure("gltf-source-object-locator"))?;
    let storage =
        ctx.metered_opaque_allowance(ImportStorageClass::FormatNormalization, envelope)?;
    let value = SourceLocatorValue::new(label.as_str())
        .map_err(|_| ctx.allocation_failure("gltf-source-object-locator"))?;
    plan.finish(0, value.capacity_bytes()).map_err(|_| {
        ctx.format_failure(GltfFailure::internal(
            "gltf-source-object-locator-capacity-envelope",
        ))
    })?;
    let schema = ExtensionSchemaId::new(SOURCE_SCHEMA, 1)
        .map_err(|_| gltf_parse_error(ctx, "static glTF source locator schema is invalid"))?;
    let locator = SourceObjectLocator::try_namespaced(Some(schema), value)
        .map_err(|_| gltf_parse_error(ctx, "static glTF source locator schema is missing"))?;
    Ok(SelectedSource {
        entity: source,
        locator,
        _storage: storage,
    })
}

/// Called before decoder teardown. Only source ordinals, raw ranges, and canonical local owner
/// IDs cross this boundary; no JSON or backend object is retained by the normalized model.
pub(super) fn associate(
    ctx: &ParseScope<'_>,
    document: &gltf::Document,
    prepared: &PreparedDocument<'_, '_>,
    model: &mut NormalizedGltf<'_>,
) -> Result<()> {
    let profile = &mut model.sources;
    let root = document.as_json();
    let scene_source = prepared
        .node_graph
        .selection_outcome
        .selected_scene_index()
        .map_or(DOCUMENT, |index| {
            entity(SourceEntityKind::Scene, None, index)
        });
    profile.link(ctx, ProfileOwner::Scene, scene_source)?;
    profile.link(ctx, ProfileOwner::SyntheticRoot, scene_source)?;

    for (index, id) in model.nodes.node_ids.as_slice().iter().enumerate() {
        ctx.charge_work(1)?;
        if let Some(id) = id {
            profile.link(
                ctx,
                ProfileOwner::Node(*id),
                entity(SourceEntityKind::Node, None, index),
            )?;
        }
    }
    for (index, id) in model.cameras.camera_ids.as_slice().iter().enumerate() {
        ctx.charge_work(1)?;
        if let Some(id) = id {
            profile.link(
                ctx,
                ProfileOwner::Camera(id.index()),
                entity(SourceEntityKind::Camera, None, index),
            )?;
        }
    }
    for (index, instance) in model.nodes.mesh_instances.as_slice().iter().enumerate() {
        let owner =
            ProfileOwner::MeshInstance(authoring_u32(ctx, index, "gltf-source-mesh-instance")?);
        profile.link(
            ctx,
            owner,
            entity(SourceEntityKind::Node, None, instance.source_node_index),
        )?;
        let mesh = root
            .nodes
            .get(instance.source_node_index)
            .and_then(|node| node.mesh)
            .ok_or_else(|| gltf_parse_error(ctx, "glTF instance source mesh is missing"))?;
        profile.link(
            ctx,
            owner,
            entity(SourceEntityKind::Mesh, None, mesh.value()),
        )?;
    }

    for (index, id) in model.appearance.image_ids.as_slice().iter().enumerate() {
        ctx.charge_work(1)?;
        let Some(id) = id else { continue };
        let owner = ProfileOwner::Image(id.index());
        profile.link(ctx, owner, entity(SourceEntityKind::Image, None, index))?;
        if let Some(view) = root.images.get(index).and_then(|image| image.buffer_view) {
            link_view(ctx, profile, root, owner, view.value())?;
        }
    }
    for (index, id) in model.appearance.sampler_ids.as_slice().iter().enumerate() {
        ctx.charge_work(1)?;
        if let Some(id) = id {
            profile.link(
                ctx,
                ProfileOwner::Sampler(id.index()),
                entity(SourceEntityKind::Sampler, None, index),
            )?;
        }
    }
    for (index, id) in model.appearance.texture_ids.as_slice().iter().enumerate() {
        ctx.charge_work(1)?;
        let Some(id) = id else { continue };
        let source = entity(SourceEntityKind::Texture, None, index);
        profile.link(ctx, ProfileOwner::Texture(id.index()), source)?;
        if root
            .textures
            .get(index)
            .is_some_and(|texture| texture.sampler.is_none())
        {
            let sampler = model
                .appearance
                .default_sampler
                .ok_or_else(|| gltf_parse_error(ctx, "glTF implicit sampler source is missing"))?;
            profile.link(ctx, ProfileOwner::Sampler(sampler.index()), source)?;
        }
    }
    let mut binding_index = 0;
    for (index, id) in model.appearance.material_ids.as_slice().iter().enumerate() {
        ctx.charge_work(1)?;
        let Some(id) = id else { continue };
        let source = entity(SourceEntityKind::Material, None, index);
        profile.link(ctx, ProfileOwner::Material(id.index()), source)?;
        let material = root
            .materials
            .get(index)
            .ok_or_else(|| gltf_parse_error(ctx, "glTF material source is missing"))?;
        let pbr = &material.pbr_metallic_roughness;
        for texture in [
            pbr.base_color_texture
                .as_ref()
                .map(|info| info.index.value()),
            pbr.metallic_roughness_texture
                .as_ref()
                .map(|info| info.index.value()),
            material
                .normal_texture
                .as_ref()
                .map(|info| info.index.value()),
            material
                .occlusion_texture
                .as_ref()
                .map(|info| info.index.value()),
            material
                .emissive_texture
                .as_ref()
                .map(|info| info.index.value()),
        ]
        .into_iter()
        .flatten()
        {
            let owner = ProfileOwner::TextureBinding(authoring_u32(
                ctx,
                binding_index,
                "gltf-source-binding",
            )?);
            profile.link(ctx, owner, source)?;
            profile.link(ctx, owner, entity(SourceEntityKind::Texture, None, texture))?;
            binding_index += 1;
        }
    }
    if binding_index != model.appearance.bindings.len() {
        return Err(gltf_parse_error(
            ctx,
            "glTF binding source map is incomplete",
        ));
    }

    let mut primitive_index = 0;
    let mut attribute_index = 0;
    let mut mesh_index = 0;
    let mut definition_index = 0;
    let mut delta_index = 0;
    for (source_index, selected) in model
        .geometry
        .logical_mesh_ids
        .as_slice()
        .iter()
        .enumerate()
    {
        ctx.charge_work(1)?;
        if selected.is_none() {
            continue;
        }
        let mesh = root
            .meshes
            .get(source_index)
            .ok_or_else(|| gltf_parse_error(ctx, "glTF mesh source is missing"))?;
        let normalized = model
            .geometry
            .logical_meshes
            .as_slice()
            .get(mesh_index)
            .ok_or_else(|| gltf_parse_error(ctx, "glTF normalized mesh is missing"))?;
        let source = entity(SourceEntityKind::Mesh, None, source_index);
        profile.link(
            ctx,
            ProfileOwner::LogicalMesh(authoring_u32(ctx, mesh_index, "gltf-source-mesh")?),
            source,
        )?;
        mesh_index += 1;
        let mut width = 0;
        for primitive in &mesh.primitives {
            ctx.charge_work(1)?;
            if let Some(targets) = &primitive.targets {
                width = targets.len();
                break;
            }
        }
        for _ in 0..width {
            let owner = ProfileOwner::MorphDefinition(authoring_u32(
                ctx,
                definition_index,
                "gltf-source-morph-definition",
            )?);
            profile.link(ctx, owner, source)?;
            definition_index += 1;
        }
        if mesh.primitives.len() != normalized.primitives.len() {
            return Err(gltf_parse_error(
                ctx,
                "glTF primitive source map is incomplete",
            ));
        }
        for (index, (primitive, normalized)) in mesh
            .primitives
            .iter()
            .zip(normalized.primitives.as_slice())
            .enumerate()
        {
            let source = entity(SourceEntityKind::Primitive, Some(source_index), index);
            let owner = ProfileOwner::Primitive(authoring_u32(
                ctx,
                primitive_index,
                "gltf-source-primitive",
            )?);
            profile.link(ctx, owner, source)?;
            primitive_index += 1;
            // Includes the JOINTS/WEIGHTS inputs retained as influences rather than attributes.
            for accessor in primitive
                .attributes
                .values()
                .copied()
                .chain(primitive.indices)
            {
                link_accessor(ctx, profile, root, owner, accessor.value())?;
            }
            for attribute in normalized.attributes.as_slice() {
                let semantic = match &attribute.semantic {
                    AttributeSemantic::Position => json::mesh::Semantic::Positions,
                    AttributeSemantic::Normal => json::mesh::Semantic::Normals,
                    AttributeSemantic::Tangent => json::mesh::Semantic::Tangents,
                    AttributeSemantic::Texcoord(set) => json::mesh::Semantic::TexCoords(*set),
                    AttributeSemantic::Color(set) => json::mesh::Semantic::Colors(*set),
                    _ => {
                        return Err(gltf_parse_error(
                            ctx,
                            "glTF attribute source semantic is missing",
                        ));
                    }
                };
                charge_lookup(ctx, primitive.attributes.len(), 1)?;
                let accessor = primitive
                    .attributes
                    .get(&Checked::Valid(semantic))
                    .ok_or_else(|| {
                        gltf_parse_error(ctx, "glTF attribute source accessor is missing")
                    })?;
                let owner = ProfileOwner::Attribute(authoring_u32(
                    ctx,
                    attribute_index,
                    "gltf-source-attribute",
                )?);
                profile.link(ctx, owner, source)?;
                link_accessor(ctx, profile, root, owner, accessor.value())?;
                attribute_index += 1;
            }
            for target in primitive.targets.iter().flatten() {
                ctx.charge_work(1)?;
                for accessor in [target.positions, target.normals, target.tangents]
                    .into_iter()
                    .flatten()
                {
                    let owner = ProfileOwner::MorphDelta(authoring_u32(
                        ctx,
                        delta_index,
                        "gltf-source-morph-delta",
                    )?);
                    profile.link(ctx, owner, source)?;
                    link_accessor(ctx, profile, root, owner, accessor.value())?;
                    delta_index += 1;
                }
            }
        }
    }
    if mesh_index != model.geometry.logical_meshes.len()
        || definition_index != model.morphs.definitions.len()
        || delta_index != model.morphs.deltas.len()
    {
        return Err(gltf_parse_error(
            ctx,
            "glTF geometry source map is incomplete",
        ));
    }

    let mut skin_index = 0;
    for (index, selected) in model.skins.skin_ids.as_slice().iter().enumerate() {
        ctx.charge_work(1)?;
        if selected.is_none() {
            continue;
        }
        let owner = ProfileOwner::Skin(authoring_u32(ctx, skin_index, "gltf-source-skin")?);
        skin_index += 1;
        profile.link(ctx, owner, entity(SourceEntityKind::Skin, None, index))?;
        let skin = root
            .skins
            .get(index)
            .ok_or_else(|| gltf_parse_error(ctx, "glTF skin source is missing"))?;
        if let Some(accessor) = skin.inverse_bind_matrices {
            link_accessor(ctx, profile, root, owner, accessor.value())?;
        }
        for node in skin.joints.iter().copied().chain(skin.skeleton) {
            profile.link(
                ctx,
                owner,
                entity(SourceEntityKind::Node, None, node.value()),
            )?;
        }
    }

    let mut clip_index = 0;
    let mut channel_index = 0;
    let mut source_channel_index = 0;
    for (index, animation) in root.animations.iter().enumerate() {
        ctx.charge_work(1)?;
        let selected = prepared
            .animation_contracts
            .selected_animations
            .as_slice()
            .get(index)
            .copied()
            .ok_or_else(|| gltf_parse_error(ctx, "glTF animation source selection is missing"))?;
        let owner =
            ProfileOwner::AnimationClip(authoring_u32(ctx, clip_index, "gltf-source-animation")?);
        if selected {
            profile.link(ctx, owner, entity(SourceEntityKind::Animation, None, index))?;
            clip_index += 1;
        }
        for (local_channel, channel) in animation.channels.iter().enumerate() {
            ctx.charge_work(1)?;
            let retained = prepared
                .animation_contracts
                .source_channels
                .as_slice()
                .get(source_channel_index)
                .is_some_and(|disposition| disposition.is_retained());
            source_channel_index += 1;
            if !selected {
                continue;
            }
            let sampler_id = entity(
                SourceEntityKind::AnimationSampler,
                Some(index),
                channel.sampler.value(),
            );
            let sampler = animation
                .samplers
                .get(channel.sampler.value())
                .ok_or_else(|| gltf_parse_error(ctx, "glTF animation source sampler is missing"))?;
            // Omitted targets may contribute duration, but never receive a canonical channel owner.
            profile.link(ctx, owner, sampler_id)?;
            link_accessor(ctx, profile, root, owner, sampler.input.value())?;
            if retained {
                let owner = ProfileOwner::AnimationChannel(authoring_u32(
                    ctx,
                    channel_index,
                    "gltf-source-animation-channel",
                )?);
                channel_index += 1;
                profile.link(
                    ctx,
                    owner,
                    entity(
                        SourceEntityKind::AnimationChannel,
                        Some(index),
                        local_channel,
                    ),
                )?;
                profile.link(ctx, owner, sampler_id)?;
                link_accessor(ctx, profile, root, owner, sampler.input.value())?;
                link_accessor(ctx, profile, root, owner, sampler.output.value())?;
            }
        }
    }
    if clip_index != model.animations.clip_count()
        || channel_index != model.animations.channel_count()
        || source_channel_index != prepared.animation_contracts.source_channels.len()
    {
        return Err(gltf_parse_error(
            ctx,
            "glTF animation source map is incomplete",
        ));
    }
    profile.finish(ctx)
}

fn link_accessor(
    ctx: &ParseScope<'_>,
    profile: &mut SelectedSourceProfile<'_>,
    root: &json::Root,
    owner: ProfileOwner,
    index: usize,
) -> Result<()> {
    profile.link(ctx, owner, entity(SourceEntityKind::Accessor, None, index))?;
    let accessor = root
        .accessors
        .get(index)
        .ok_or_else(|| gltf_parse_error(ctx, "glTF source accessor is missing"))?;
    if let Some(view) = accessor.buffer_view {
        link_view(ctx, profile, root, owner, view.value())?;
    }
    if let Some(sparse) = &accessor.sparse {
        link_view(
            ctx,
            profile,
            root,
            owner,
            sparse.indices.buffer_view.value(),
        )?;
        link_view(ctx, profile, root, owner, sparse.values.buffer_view.value())?;
    }
    Ok(())
}

fn link_view(
    ctx: &ParseScope<'_>,
    profile: &mut SelectedSourceProfile<'_>,
    root: &json::Root,
    owner: ProfileOwner,
    index: usize,
) -> Result<()> {
    profile.link(
        ctx,
        owner,
        entity(SourceEntityKind::BufferView, None, index),
    )?;
    let view = root
        .buffer_views
        .get(index)
        .ok_or_else(|| gltf_parse_error(ctx, "glTF source buffer view is missing"))?;
    profile.link(
        ctx,
        owner,
        entity(SourceEntityKind::Buffer, None, view.buffer.value()),
    )
}
