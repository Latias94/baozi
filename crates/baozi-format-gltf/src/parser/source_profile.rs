//! Selected source identities and their canonical owners, independent of decoder lifetimes.

use super::{
    ContentError, GltfFailure, PreparedDocument, Result, authoring_u32, extras::SourceExtras,
    gltf_parse_error, model::NormalizedGltf,
};
use crate::document::schema as json;
use crate::source_map::{
    GltfSourceMap, SourceEntity, SourceEntityId, SourceEntityKind, SourcePayloadKind,
};
use baozi_import::ImportWorkClass;
use baozi_import::authoring::{
    AttributeSemantic, ExtensionSchemaId, ProfileOwner, SourceLocatorValue, SourceObjectLocator,
};
use baozi_import::{
    CapacityPlan, ImportStorageClass, MeteredOpaqueAllowance, MeteredVec, ParseParser, ParseResult,
    ParseRoot, ParseServices,
};

const SOURCE_SCHEMA: &str = "org.baozi.gltf:source-object";
const COMPARISONS_BETWEEN_POLLS: u64 = 64;

#[cfg(test)]
mod tests;

// Each original composite comparison remains one WorkUnit. Production callers compare only
// fixed owner/source ordinals and validated NamespacedName keys of at most 64 ASCII bytes.
struct ProfileComparisonWork<'meter, 'borrow, 'frame, 'ctx, 'op, 'session> {
    work: &'meter mut ParseParser<'borrow, 'frame, 'ctx, 'op, 'session, 1, true>,
    comparisons_until_poll: u64,
}

impl<'meter, 'borrow, 'frame, 'ctx, 'op, 'session>
    ProfileComparisonWork<'meter, 'borrow, 'frame, 'ctx, 'op, 'session>
{
    fn new(work: &'meter mut ParseParser<'borrow, 'frame, 'ctx, 'op, 'session, 1, true>) -> Self {
        Self {
            work,
            comparisons_until_poll: COMPARISONS_BETWEEN_POLLS,
        }
    }

    fn run<T>(&mut self, compare: impl FnOnce() -> T) -> ParseResult<T> {
        // Admit only this comparison; no later work precedes its physical completion.
        self.work.replenish(1)?;
        let value = self
            .work
            .run([1], || Ok::<_, baozi_import::ParseFailure>(compare()))?;
        #[cfg(test)]
        tests::comparison_completed();
        self.comparisons_until_poll -= 1;
        if self.comparisons_until_poll == 0 {
            self.work.poll()?;
            self.comparisons_until_poll = COMPARISONS_BETWEEN_POLLS;
        }
        Ok(value)
    }
}

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
    extras: MeteredVec<'scope, SourceExtras>,
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
        ctx: &mut ParseRoot<'_, '_, 'scope>,
        map: &GltfSourceMap<'_>,
        selected: impl Fn(SourceEntityId) -> bool,
    ) -> Result<Self> {
        let mut entries = ctx.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-selected-source-identities",
        );
        let mut extras = ctx.services()?.metered_vec(
            ImportStorageClass::FormatNormalization,
            "gltf-selected-source-extras",
        );
        let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        for &source in map.entities() {
            let selected_source = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(selected(source.id))
                })
            })?;
            if selected_source {
                {
                    let value = source_locator(&mut ctx.services()?, source)?;
                    entries.try_push(&mut ctx.services()?, value)
                }?;
                let payloads = {
                    let argument_0 = ctx.services()?.primary_bytes().len();
                    lookup(ctx, argument_0, 2, || map.payloads_for(source.id))
                }?;
                for payload in payloads {
                    let kind = ctx.child(&work_group, [1], |child| {
                        child.run([1], || Ok::<_, baozi_import::ParseFailure>(payload.kind))
                    })?;
                    if kind == SourcePayloadKind::Extras {
                        {
                            let value = SourceExtras::new(payload)?;
                            extras.try_push(&mut ctx.services()?, value)
                        }?;
                    }
                }
            }
        }
        ctx.poll()?;
        if extras.len() > 1 {
            ctx.child(&work_group, [0], |ledger_work| {
                let mut work = ProfileComparisonWork::new(ledger_work);
                #[cfg(test)]
                tests::comparison_scope(tests::ComparisonSite::ExtrasSort);
                extras.try_sort_unstable_by(|left, right| {
                    work.run(|| {
                        left.source
                            .cmp(&right.source)
                            .then_with(|| left.key.cmp(&right.key))
                    })
                })
            })?;
        }
        ctx.poll()?;
        let profile = Self {
            entries,
            links: ctx.services()?.metered_vec(
                ImportStorageClass::FormatNormalization,
                "gltf-selected-source-owner-links",
            ),
            extras,
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

    pub(super) fn extras(&self) -> &[SourceExtras] {
        self.extras.as_slice()
    }

    pub(super) fn required(
        &self,
        ctx: &mut ParseRoot<'_, '_, '_>,
        id: SourceEntityId,
    ) -> Result<&SelectedSource<'scope>> {
        lookup(ctx, self.entries.len(), 1, || {
            self.entries
                .as_slice()
                .binary_search_by_key(&id, |source| source.entity.id)
                .ok()
                .and_then(|index| self.entries.as_slice().get(index))
        })?
        .ok_or_else(|| {
            ContentError::from(GltfFailure::internal(
                "selected glTF owner has no admitted source identity",
            ))
        })
    }

    pub(super) fn link(
        &mut self,
        ctx: &mut ParseRoot<'_, '_, 'scope>,
        owner: ProfileOwner,
        source: SourceEntityId,
    ) -> Result<()> {
        let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        let link = ctx.child(&layout, [1], |child| {
            child.run([1], || {
                Ok::<_, baozi_import::ParseFailure>(OwnerSourceLink { owner, source })
            })
        })?;
        self.required(ctx, source)?;
        self.links.try_push(&mut ctx.services()?, link)?;
        Ok(())
    }

    pub(super) fn finish(&mut self, ctx: &mut ParseRoot<'_, '_, '_>) -> Result<()> {
        ctx.poll()?;
        if self.links.len() > 1 {
            let group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            ctx.child(&group, [0], |ledger_work| {
                let mut work = ProfileComparisonWork::new(ledger_work);
                #[cfg(test)]
                tests::comparison_scope(tests::ComparisonSite::LinkSort);
                self.links
                    .try_sort_unstable_by(|left, right| work.run(|| left.cmp(right)))
            })?;
            let retained = ctx.child(&group, [0], |ledger_work| {
                let mut work = ProfileComparisonWork::new(ledger_work);
                #[cfg(test)]
                tests::comparison_scope(tests::ComparisonSite::LinkDedup);
                self.links
                    .try_partition_dedup_by(|left, right| work.run(|| left == right))
            })?;
            self.links.truncate(retained);
        }
        ctx.poll()?;
        Ok(())
    }

    pub(super) fn for_owner(
        &self,
        ctx: &mut ParseRoot<'_, '_, '_>,
        owner: ProfileOwner,
    ) -> Result<&[OwnerSourceLink]> {
        let links = self.links.as_slice();
        let (start, end) = lookup(ctx, links.len(), 2, || {
            (
                links.partition_point(|link| link.owner < owner),
                links.partition_point(|link| link.owner <= owner),
            )
        })?;
        if start == end {
            return Err(GltfFailure::internal("glTF canonical owner has no source mapping").into());
        }
        Ok(&links[start..end])
    }

    /// Resource sources retain their existing indices. The document reuses source zero; other
    /// declarations follow the resource sources. Normalized source provenance leaves use this
    /// same order, while metadata explicitly references a SourceIdentityLocalId.
    pub(super) fn source_index(
        &self,
        ctx: &mut ParseRoot<'_, '_, '_>,
        resource_count: usize,
        id: SourceEntityId,
    ) -> Result<u32> {
        let index = lookup(ctx, self.entries.len(), 1, || {
            self.entries
                .as_slice()
                .binary_search_by_key(&id, |source| source.entity.id)
        })?
        .map_err(|_| GltfFailure::internal("glTF source mapping references a missing identity"))?;
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
fn lookup<T>(
    ctx: &mut ParseRoot<'_, '_, '_>,
    length: usize,
    searches: u64,
    search: impl FnOnce() -> T,
) -> Result<T> {
    let comparisons = u64::from(usize::BITS - length.leading_zeros()) + 1;
    let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    ctx.child(&layout, [comparisons * searches], |child| {
        child.run([comparisons * searches], || Ok::<_, ContentError>(search()))
    })
}

fn source_locator<'scope>(
    ctx: &mut ParseServices<'_, '_, 'scope>,
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
        SourceEntityKind::Light => "light",
    };
    let mut label = ctx.metered_string(
        ImportStorageClass::FormatNormalization,
        "gltf-source-object-locator-label",
    );
    label.try_write_fmt(ctx, format_args!("kind={kind};parent="))?;
    match source.id.parent_index {
        Some(index) => label.try_push_usize(ctx, index)?,
        None => label.try_push_str(ctx, "-")?,
    }
    label.try_write_fmt(
        ctx,
        format_args!(
            ";index={};offset={};length={}",
            source.id.source_index, source.range.byte_offset, source.range.byte_length,
        ),
    )?;
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
        .map_err(|_| gltf_parse_error("static glTF source locator schema is invalid"))?;
    let locator = SourceObjectLocator::try_namespaced(Some(schema), value)
        .map_err(|_| gltf_parse_error("static glTF source locator schema is missing"))?;
    Ok(SelectedSource {
        entity: source,
        locator,
        _storage: storage,
    })
}

/// Called before decoder teardown. Only source ordinals, raw ranges, and canonical local owner
/// IDs cross this boundary; no JSON or backend object is retained by the normalized model.
pub(super) fn associate<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
    document: &crate::document::Document<'_>,
    prepared: &PreparedDocument<'_, '_>,
    model: &mut NormalizedGltf<'session>,
) -> Result<()> {
    let profile = &mut model.sources;
    let root = document.tables();
    let scene_source = prepared
        .node_graph
        .selection_outcome
        .selected_scene_index()
        .map_or(DOCUMENT, |index| {
            entity(SourceEntityKind::Scene, None, index)
        });
    profile.link(ctx, ProfileOwner::Scene, scene_source)?;
    profile.link(ctx, ProfileOwner::SyntheticRoot, scene_source)?;

    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for (index, id) in model.nodes.node_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        if let Some(id) = id {
            profile.link(
                ctx,
                ProfileOwner::Node(*id),
                entity(SourceEntityKind::Node, None, index),
            )?;
        }
    }
    for (index, id) in model.cameras.camera_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        if let Some(id) = id {
            profile.link(
                ctx,
                ProfileOwner::Camera(id.index()),
                entity(SourceEntityKind::Camera, None, index),
            )?;
        }
    }
    for (index, id) in model.lights.light_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        if let Some(id) = id {
            profile.link(
                ctx,
                ProfileOwner::Light(id.index()),
                entity(SourceEntityKind::Light, None, index),
            )?;
        }
    }
    for (index, instance) in model.nodes.mesh_instances.as_slice().iter().enumerate() {
        let owner = ProfileOwner::MeshInstance(authoring_u32(
            &mut ctx.services()?,
            index,
            "gltf-source-mesh-instance",
        )?);
        profile.link(
            ctx,
            owner,
            entity(SourceEntityKind::Node, None, instance.source_node_index),
        )?;
        let mesh = root
            .nodes
            .get(instance.source_node_index)
            .and_then(|node| node.mesh)
            .ok_or_else(|| gltf_parse_error("glTF instance source mesh is missing"))?;
        profile.link(
            ctx,
            owner,
            entity(SourceEntityKind::Mesh, None, mesh.value()),
        )?;
    }

    for (index, id) in model.appearance.image_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        let Some(id) = id else { continue };
        let owner = ProfileOwner::Image(id.index());
        profile.link(ctx, owner, entity(SourceEntityKind::Image, None, index))?;
        if let Some(view) = root.images.get(index).and_then(|image| image.buffer_view) {
            link_view(ctx, profile, root, owner, view.value())?;
        }
    }
    for (index, id) in model.appearance.sampler_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        if let Some(id) = id {
            profile.link(
                ctx,
                ProfileOwner::Sampler(id.index()),
                entity(SourceEntityKind::Sampler, None, index),
            )?;
        }
    }
    for (index, id) in model.appearance.texture_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
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
                .ok_or_else(|| gltf_parse_error("glTF implicit sampler source is missing"))?;
            profile.link(ctx, ProfileOwner::Sampler(sampler.index()), source)?;
        }
    }
    if let Some(material) = model.appearance.default_material {
        profile.link(ctx, ProfileOwner::Material(material.index()), DOCUMENT)?;
    }
    let mut binding_index = 0;
    for (index, id) in model.appearance.material_ids.as_slice().iter().enumerate() {
        let id = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(id.as_ref()))
        })?;
        let Some(id) = id else { continue };
        let source = entity(SourceEntityKind::Material, None, index);
        profile.link(ctx, ProfileOwner::Material(id.index()), source)?;
        for binding in prepared
            .material_contracts
            .bindings(&mut ctx.services()?, index)?
        {
            let texture = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(binding.source_texture)
                })
            })?;
            let owner = ProfileOwner::TextureBinding(authoring_u32(
                &mut ctx.services()?,
                binding_index,
                "gltf-source-binding",
            )?);
            profile.link(ctx, owner, source)?;
            profile.link(ctx, owner, entity(SourceEntityKind::Texture, None, texture))?;
            binding_index += 1;
        }
    }
    if binding_index != model.appearance.bindings.len() {
        return Err(gltf_parse_error("glTF binding source map is incomplete"));
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
        let selected = ctx.child(&work_group, [1], |child| {
            child.run([1], || {
                Ok::<_, baozi_import::ParseFailure>(selected.is_some())
            })
        })?;
        if !selected {
            continue;
        }
        let mesh = root
            .meshes
            .get(source_index)
            .ok_or_else(|| gltf_parse_error("glTF mesh source is missing"))?;
        let normalized = model
            .geometry
            .logical_meshes
            .as_slice()
            .get(mesh_index)
            .ok_or_else(|| gltf_parse_error("glTF normalized mesh is missing"))?;
        let source = entity(SourceEntityKind::Mesh, None, source_index);
        {
            let argument_0 = ProfileOwner::LogicalMesh(authoring_u32(
                &mut ctx.services()?,
                mesh_index,
                "gltf-source-mesh",
            )?);
            profile.link(ctx, argument_0, source)
        }?;
        mesh_index += 1;
        let mut width = 0;
        for primitive in &mesh.primitives {
            let targets = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(primitive.targets.as_ref())
                })
            })?;
            if let Some(targets) = targets {
                width = targets.len();
                break;
            }
        }
        for _ in 0..width {
            let owner = ProfileOwner::MorphDefinition(authoring_u32(
                &mut ctx.services()?,
                definition_index,
                "gltf-source-morph-definition",
            )?);
            profile.link(ctx, owner, source)?;
            definition_index += 1;
        }
        if mesh.primitives.len() != normalized.primitives.len() {
            return Err(gltf_parse_error("glTF primitive source map is incomplete"));
        }
        for (index, (primitive, normalized)) in mesh
            .primitives
            .iter()
            .zip(normalized.primitives.as_slice())
            .enumerate()
        {
            let source = entity(SourceEntityKind::Primitive, Some(source_index), index);
            let owner = ProfileOwner::Primitive(authoring_u32(
                &mut ctx.services()?,
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
                            "glTF attribute source semantic is missing",
                        ));
                    }
                };
                let accessor = lookup(ctx, primitive.attributes.len(), 1, || {
                    primitive.attributes.get(&semantic)
                })?
                .ok_or_else(|| gltf_parse_error("glTF attribute source accessor is missing"))?;
                let owner = ProfileOwner::Attribute(authoring_u32(
                    &mut ctx.services()?,
                    attribute_index,
                    "gltf-source-attribute",
                )?);
                profile.link(ctx, owner, source)?;
                link_accessor(ctx, profile, root, owner, accessor.value())?;
                attribute_index += 1;
            }
            for target in primitive.targets.iter().flatten() {
                let accessors = ctx.child(&work_group, [1], |child| {
                    child.run([1], || {
                        Ok::<_, baozi_import::ParseFailure>([
                            target.positions,
                            target.normals,
                            target.tangents,
                        ])
                    })
                })?;
                for accessor in accessors.into_iter().flatten() {
                    let owner = ProfileOwner::MorphDelta(authoring_u32(
                        &mut ctx.services()?,
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
        return Err(gltf_parse_error("glTF geometry source map is incomplete"));
    }

    let mut skin_index = 0;
    for (index, selected) in model.skins.skin_ids.as_slice().iter().enumerate() {
        let selected = ctx.child(&work_group, [1], |child| {
            child.run([1], || {
                Ok::<_, baozi_import::ParseFailure>(selected.is_some())
            })
        })?;
        if !selected {
            continue;
        }
        let owner = ProfileOwner::Skin(authoring_u32(
            &mut ctx.services()?,
            skin_index,
            "gltf-source-skin",
        )?);
        skin_index += 1;
        profile.link(ctx, owner, entity(SourceEntityKind::Skin, None, index))?;
        let skin = root
            .skins
            .get(index)
            .ok_or_else(|| gltf_parse_error("glTF skin source is missing"))?;
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
        let selected = ctx.child(&work_group, [1], |child| {
            child.run([1], || {
                Ok::<_, baozi_import::ParseFailure>(
                    prepared
                        .animation_contracts
                        .selected_animations
                        .as_slice()
                        .get(index)
                        .copied(),
                )
            })
        })?;
        let selected = selected
            .ok_or_else(|| gltf_parse_error("glTF animation source selection is missing"))?;
        let owner = ProfileOwner::AnimationClip(authoring_u32(
            &mut ctx.services()?,
            clip_index,
            "gltf-source-animation",
        )?);
        if selected {
            profile.link(ctx, owner, entity(SourceEntityKind::Animation, None, index))?;
            clip_index += 1;
        }
        for (local_channel, channel) in animation.channels.iter().enumerate() {
            let has_target = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(channel.target.node.is_some())
                })
            })?;
            let retained = prepared
                .animation_contracts
                .source_channels
                .as_slice()
                .get(source_channel_index)
                .is_some_and(|disposition| disposition.is_retained());
            source_channel_index += 1;
            if !selected || !has_target {
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
                .ok_or_else(|| gltf_parse_error("glTF animation source sampler is missing"))?;
            // Omitted targets may contribute duration, but never receive a canonical channel owner.
            profile.link(ctx, owner, sampler_id)?;
            link_accessor(ctx, profile, root, owner, sampler.input.value())?;
            if retained {
                let owner = ProfileOwner::AnimationChannel(authoring_u32(
                    &mut ctx.services()?,
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
        return Err(gltf_parse_error("glTF animation source map is incomplete"));
    }
    profile.finish(ctx)
}

fn link_accessor<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
    profile: &mut SelectedSourceProfile<'session>,
    root: &json::Tables<'_>,
    owner: ProfileOwner,
    index: usize,
) -> Result<()> {
    profile.link(ctx, owner, entity(SourceEntityKind::Accessor, None, index))?;
    let accessor = root
        .accessors
        .get(index)
        .ok_or_else(|| gltf_parse_error("glTF source accessor is missing"))?;
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

fn link_view<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
    profile: &mut SelectedSourceProfile<'session>,
    root: &json::Tables<'_>,
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
        .ok_or_else(|| gltf_parse_error("glTF source buffer view is missing"))?;
    profile.link(
        ctx,
        owner,
        entity(SourceEntityKind::Buffer, None, view.buffer.value()),
    )
}
