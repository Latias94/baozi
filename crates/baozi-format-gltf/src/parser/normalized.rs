//! Canonical authoring from the owned, backend-independent glTF model.

use super::{
    ContentError, GltfFailure, Result, animation, authoring_u32, gltf_parse_error,
    model::*,
    morph,
    source_profile::{self, SelectedSourceProfile},
    uri,
};
use crate::FORMAT_ID;
use baozi_import::authoring::*;
use baozi_import::{ImportStorageClass, MeteredVec, ParseScope, SceneDraft};
use sha2::{Digest, Sha256};

#[cfg(test)]
mod tests;

pub(super) fn submit(ctx: &ParseScope<'_>, model: &NormalizedGltf<'_>) -> Result<SceneDraft> {
    model.sources.required(ctx, source_profile::DOCUMENT)?;
    submit_parts(
        ctx,
        &model.appearance,
        &model.geometry,
        &model.cameras,
        &model.nodes,
        &model.skins,
        &model.instance_deformations,
        &model.buffers,
        &model.external_images,
        &model.animations,
        &model.morphs,
        &model.sources,
    )
}

struct OwnerSource {
    owner: ProfileOwner,
    provenance_offset: usize,
    provenance_count: usize,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn submit_parts(
    ctx: &ParseScope<'_>,
    appearance_storage: &AppearanceStorage<'_>,
    geometry_storage: &GeometryStorage<'_>,
    cameras: &CameraStorage<'_>,
    nodes: &NodeStorage<'_>,
    skins: &SkinStorage<'_>,
    instance_deformations: &MeteredVec<'_, InstanceDeformationStorage<'_>>,
    buffers: &BufferResources<'_>,
    external_images: &ExternalImageResources<'_>,
    animation_storage: &animation::AnimationStorage<'_>,
    morph_storage: &morph::MorphStorage<'_>,
    selected_sources: &SelectedSourceProfile<'_>,
) -> Result<SceneDraft> {
    let mut images = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-images",
    );
    images.try_reserve(appearance_storage.images.len())?;
    for image in appearance_storage.images.as_slice() {
        let media_type = image
            .media_type
            .as_ref()
            .map_or(Presence::Absent, Presence::Present);
        let authored = match &image.payload {
            ImagePayloadStorage::Encoded(index) => {
                let bytes = appearance_storage
                    .encoded_image_payloads
                    .as_slice()
                    .get(*index)
                    .ok_or_else(|| {
                        gltf_parse_error(ctx, "glTF encoded image payload map is incomplete")
                    })?;
                ImageAuthoring::encoded(bytes.as_slice(), media_type)
            }
            ImagePayloadStorage::SessionResource(resource) => {
                ImageAuthoring::session_resource(*resource, media_type)
            }
        };
        images.try_push_reserved(authored)?;
    }

    let mut bindings = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-material-bindings",
    );
    bindings.try_reserve(appearance_storage.bindings.len())?;
    for binding in appearance_storage.bindings.as_slice() {
        let authored = MaterialBindingAuthoring::try_from_parts(
            binding.texture,
            binding.role,
            binding.uv_set,
            binding.channel_mapping,
            binding.color_interpretation,
            binding.transform,
            binding.strength,
            binding.blend,
            Some(&binding.source_key),
        )
        .map_err(|_| gltf_parse_error(ctx, "glTF texture binding semantics are invalid"))?;
        bindings.try_push_reserved(authored)?;
    }

    let mut materials = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-materials",
    );
    let shading_model = ShadingModel::PbrMetallicRoughness;
    materials.try_reserve(appearance_storage.materials.len())?;
    for material in appearance_storage.materials.as_slice() {
        let mut authored = MaterialAuthoring::try_from_parts(
            material.bindings,
            shading_model,
            material.alpha_mode,
            material.double_sided,
            material.factors,
        )
        .map_err(|_| gltf_parse_error(ctx, "glTF material semantics are invalid"))?;
        if let Some(name) = material.name.as_ref() {
            authored = authored.with_name(name.as_str());
        }
        materials.try_push_reserved(authored)?;
    }

    let mut logical_meshes = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-logical-meshes",
    );
    let mut primitives = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-primitives",
    );
    let mut attributes = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-attributes",
    );
    let primitive_count = geometry_storage
        .logical_meshes
        .as_slice()
        .iter()
        .try_fold(0usize, |count, mesh| {
            count.checked_add(mesh.primitives.len())
        })
        .ok_or_else(|| ctx.allocation_failure("gltf-complete-batch-primitive-count"))?;
    let attribute_count = geometry_storage
        .logical_meshes
        .as_slice()
        .iter()
        .flat_map(|mesh| mesh.primitives.as_slice())
        .try_fold(0usize, |count, primitive| {
            count.checked_add(primitive.attributes.len())
        })
        .ok_or_else(|| ctx.allocation_failure("gltf-complete-batch-attribute-count"))?;
    logical_meshes.try_reserve(geometry_storage.logical_meshes.len())?;
    primitives.try_reserve(primitive_count)?;
    attributes.try_reserve(attribute_count)?;
    for mesh in geometry_storage.logical_meshes.as_slice() {
        let primitive_offset = authoring_u32(
            ctx,
            primitives.len(),
            "gltf-complete-batch-primitive-offset",
        )?;
        for primitive in mesh.primitives.as_slice() {
            let attribute_offset = authoring_u32(
                ctx,
                attributes.len(),
                "gltf-complete-batch-attribute-offset",
            )?;
            for attribute in primitive.attributes.as_slice() {
                attributes.try_push_reserved(GeometryAttributeAuthoring::f32(
                    attribute.semantic.clone(),
                    AttributeDomain::Vertex,
                    attribute.lanes,
                    attribute.values.as_slice(),
                    Presence::Absent,
                ))?;
            }
            let indices = if primitive.has_indices {
                Presence::Present(primitive.indices.as_slice())
            } else {
                Presence::Absent
            };
            let material = primitive
                .material
                .map_or(Presence::Absent, Presence::Present);
            primitives.try_push_reserved(GeometryPrimitiveAuthoring::new(
                primitive.topology,
                indices,
                material,
                AuthoringRange::new(
                    attribute_offset,
                    authoring_u32(
                        ctx,
                        primitive.attributes.len(),
                        "gltf-complete-batch-attribute-count",
                    )?,
                ),
            ))?;
        }
        let mut authored = GeometryLogicalMeshAuthoring::new(AuthoringRange::new(
            primitive_offset,
            authoring_u32(
                ctx,
                mesh.primitives.len(),
                "gltf-complete-batch-logical-mesh-primitive-count",
            )?,
        ));
        if let Some(name) = mesh.name.as_ref() {
            authored = authored.with_name(name.as_str());
        }
        logical_meshes.try_push_reserved(authored)?;
    }

    let mut mesh_instances = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-mesh-instances",
    );
    mesh_instances.try_reserve(nodes.mesh_instances.len())?;
    for instance in nodes.mesh_instances.as_slice() {
        mesh_instances.try_push_reserved(instance.authored)?;
    }

    let mut skin_authoring = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-skins",
    );
    skin_authoring.try_reserve(skins.skins.len())?;
    for skin in skins.skins.as_slice() {
        let name = skin
            .name
            .as_ref()
            .map_or(Presence::Absent, |name| Presence::Present(name.as_str()));
        let skeleton_root = skin
            .skeleton_root
            .map_or(Presence::Absent, Presence::Present);
        skin_authoring.try_push_reserved(SkinAuthoring::new(
            name,
            skin.joints.as_slice(),
            skin.inverse_bind_matrices.as_slice(),
            skeleton_root,
            SkinMatrixSpaces::canonical(),
        ))?;
    }

    let mut logical_morphs = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-logical-morphs",
    );
    logical_morphs.try_reserve(logical_meshes.len())?;
    if morph_storage.logical_meshes.len() != logical_meshes.len()
        || morph_storage.primitives.len() != primitives.len()
    {
        return Err(gltf_parse_error(
            ctx,
            "glTF morph tables disagree with geometry",
        ));
    }
    for mesh in morph_storage.logical_meshes.as_slice() {
        ctx.charge_work(1)?;
        logical_morphs.try_push_reserved(LogicalMorphAuthoring::new(
            mesh.definitions,
            mesh.weights.as_slice(),
        ))?;
    }
    let mut morph_deltas = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-morph-deltas",
    );
    morph_deltas.try_reserve(morph_storage.deltas.len())?;
    for delta in morph_storage.deltas.as_slice() {
        ctx.charge_work(1)?;
        morph_deltas.try_push_reserved(morph::delta_authoring(delta))?;
    }
    let mut morph_targets = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-morph-targets",
    );
    morph_targets.try_reserve(morph_storage.targets.len())?;
    for target in morph_storage.targets.as_slice() {
        ctx.charge_work(1)?;
        morph_targets
            .try_push_reserved(morph::target_authoring(target, morph_deltas.as_slice()))?;
    }
    let mut primitive_deformations = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-primitive-deformations",
    );
    primitive_deformations.try_reserve(primitives.len())?;
    for mesh in geometry_storage.logical_meshes.as_slice() {
        for primitive in mesh.primitives.as_slice() {
            let influences = if primitive.influences.is_empty() {
                Presence::Absent
            } else {
                Presence::Present(InfluenceRowsAuthoring::new(
                    primitive.influence_offsets.as_slice(),
                    primitive.influences.as_slice(),
                ))
            };
            ctx.charge_work(1)?;
            let range = morph_storage
                .primitives
                .as_slice()
                .get(primitive_deformations.len())
                .ok_or_else(|| {
                    gltf_parse_error(ctx, "glTF primitive morph target range is missing")
                })?;
            let targets = morph_targets.as_slice().get(range.clone()).ok_or_else(|| {
                gltf_parse_error(ctx, "glTF primitive morph target range is invalid")
            })?;
            primitive_deformations
                .try_push_reserved(PrimitiveDeformationAuthoring::new(influences, targets))?;
        }
    }

    let appearance = AppearanceAuthoringBatch::new(
        images.as_slice(),
        appearance_storage.samplers.as_slice(),
        appearance_storage.textures.as_slice(),
        bindings.as_slice(),
        materials.as_slice(),
    );
    let geometry = GeometryAuthoringBatch::new(
        logical_meshes.as_slice(),
        primitives.as_slice(),
        attributes.as_slice(),
        mesh_instances.as_slice(),
    );
    let mut mesh_instance_deformations = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-instance-deformations",
    );
    mesh_instance_deformations.try_reserve(instance_deformations.len())?;
    for instance in instance_deformations.as_slice() {
        ctx.charge_work(1)?;
        mesh_instance_deformations.try_push_reserved(MeshInstanceDeformationAuthoring::new(
            instance.skin,
            instance
                .weights
                .as_ref()
                .map_or(Presence::Absent, |weights| {
                    Presence::Present(weights.as_slice())
                }),
        ))?;
    }
    let deformation = DeformationAuthoringBatch::new(
        skin_authoring.as_slice(),
        morph_storage.definitions.as_slice(),
        logical_morphs.as_slice(),
        primitive_deformations.as_slice(),
        mesh_instance_deformations.as_slice(),
    );
    let scene_space = SceneSpace::STANDARD_TARGET;
    let spatial = SpatialAuthoringBatch::new(&scene_space, cameras.authored.as_slice(), &[]);
    let mut authored_nodes = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-nodes",
    );
    authored_nodes.try_reserve(nodes.nodes.len())?;
    if nodes.names.len() != nodes.nodes.len() {
        return Err(gltf_parse_error(
            ctx,
            "glTF normalized node names are incomplete",
        ));
    }
    for (node, name) in nodes.nodes.as_slice().iter().zip(nodes.names.as_slice()) {
        ctx.charge_work(1)?;
        let node = name
            .as_ref()
            .map_or(*node, |name| node.with_name(name.as_str()));
        authored_nodes.try_push_reserved(node)?;
    }
    let node_batch = NodeAuthoringBatch::new(
        Presence::Present(nodes.roots.as_slice()),
        authored_nodes.as_slice(),
    );
    let primary_identity = uri::profile_identity(ctx, ctx.source().as_str())?;
    let resource_count = buffers
        .external_resource_count()
        .checked_add(external_images.values.len())
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| ctx.allocation_failure("gltf-profile-resource-count"))?;
    let mut resources = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-resources",
    );
    let mut snapshots = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-resource-snapshots",
    );
    let mut locators = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-source-locators",
    );
    resources.try_reserve(resource_count)?;
    snapshots.try_reserve(resource_count)?;
    locators.try_reserve(resource_count - 1)?;
    resources.try_push_reserved(ObservedResourceAuthoring::session_opaque(
        primary_identity.value(),
        SessionOpaqueReason::CallerLabel,
    ))?;
    snapshots.try_push_reserved(ResourceSnapshotAuthoring::absent())?;
    for (buffer_index, identity) in buffers.external_resources() {
        resources.try_push_reserved(ObservedResourceAuthoring::session_opaque(
            identity,
            SessionOpaqueReason::UnregisteredResolver,
        ))?;
        snapshots.try_push_reserved(ResourceSnapshotAuthoring::absent())?;
        locators.try_push_reserved(SourceObjectLocator::indexed(
            SourceObjectKind::Buffer,
            u64::try_from(buffer_index)
                .map_err(|_| ctx.allocation_failure("gltf-profile-buffer-locator"))?,
        ))?;
    }
    for resource in external_images.values.as_slice() {
        resources.try_push_reserved(ObservedResourceAuthoring::session_opaque(
            resource.identity.value(),
            SessionOpaqueReason::UnregisteredResolver,
        ))?;
        snapshots.try_push_reserved(ResourceSnapshotAuthoring::snapshotted(
            resource.snapshot.bytes(),
        ))?;
        locators.try_push_reserved(SourceObjectLocator::indexed(
            SourceObjectKind::Image,
            u64::try_from(resource.image_index)
                .map_err(|_| ctx.allocation_failure("gltf-profile-image-locator"))?,
        ))?;
    }

    let format = FormatId::new(FORMAT_ID)
        .map_err(|_| gltf_parse_error(ctx, "static glTF format identity is invalid"))?;
    let mut source_identities = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-source-identities",
    );
    let mut provenance_nodes = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-provenance-nodes",
    );
    let source_count = resource_count
        .checked_add(selected_sources.entries().len() - 1)
        .ok_or_else(|| ctx.allocation_failure("gltf-profile-source-count"))?;
    source_identities.try_reserve(source_count)?;
    let operation_count = 1 + usize::from(appearance_storage.default_sampler.is_some());
    provenance_nodes.try_reserve(
        source_count
            .checked_add(operation_count)
            .ok_or_else(|| ctx.allocation_failure("gltf-profile-provenance-count"))?,
    )?;
    for index in 0..resource_count {
        let local = authoring_u32(ctx, index, "gltf-profile-source-identity")?;
        let locator = if index == 0 {
            &selected_sources
                .required(ctx, source_profile::DOCUMENT)?
                .locator
        } else {
            locators.as_slice().get(index - 1).ok_or_else(|| {
                gltf_parse_error(ctx, "glTF profile source locator table is incomplete")
            })?
        };
        source_identities.try_push_reserved(SourceIdentityAuthoring::new(
            ResourceIdentityLocalId::new(local),
            &format,
            locator,
        ))?;
        provenance_nodes.try_push_reserved(ProvenanceNodeAuthoring::source(
            SourceIdentityLocalId::new(local),
        ))?;
    }
    for source in selected_sources.entries().iter().skip(1) {
        ctx.charge_work(1)?;
        let local = authoring_u32(ctx, source_identities.len(), "gltf-profile-source-identity")?;
        source_identities.try_push_reserved(SourceIdentityAuthoring::new(
            ResourceIdentityLocalId::new(0),
            &format,
            &source.locator,
        ))?;
        provenance_nodes.try_push_reserved(ProvenanceNodeAuthoring::source(
            SourceIdentityLocalId::new(local),
        ))?;
    }

    let owner_count = [
        2,
        nodes.nodes.len(),
        cameras.authored.len(),
        appearance_storage.images.len(),
        appearance_storage.samplers.len(),
        appearance_storage.textures.len(),
        appearance_storage.bindings.len(),
        materials.len(),
        logical_meshes.len(),
        primitives.len(),
        attributes.len(),
        mesh_instances.len(),
        skins.skins.len(),
        morph_storage.definitions.len(),
        morph_storage.deltas.len(),
        animation_storage.clip_count(),
        animation_storage.channel_count(),
    ]
    .into_iter()
    .try_fold(0usize, |count, value| count.checked_add(value))
    .ok_or_else(|| ctx.allocation_failure("gltf-profile-owner-count"))?;
    let additional_owner_source_count = [
        appearance_storage
            .images
            .as_slice()
            .iter()
            .filter(|image| image.provenance != 0)
            .count(),
        appearance_storage
            .sampler_provenance
            .as_slice()
            .iter()
            .filter(|&&source| source != 0)
            .count(),
        appearance_storage
            .texture_provenance
            .as_slice()
            .iter()
            .filter(|&&source| source != 0)
            .count(),
        appearance_storage
            .bindings
            .as_slice()
            .iter()
            .filter(|binding| binding.provenance != 0)
            .count(),
        geometry_storage
            .primitive_provenance
            .as_slice()
            .iter()
            .filter(|&&source| source != 0)
            .count(),
        geometry_storage
            .logical_meshes
            .as_slice()
            .iter()
            .flat_map(|mesh| mesh.primitives.as_slice())
            .flat_map(|primitive| primitive.attributes.as_slice())
            .filter(|attribute| attribute.provenance != 0)
            .count(),
        skins
            .skins
            .as_slice()
            .iter()
            .filter(|skin| skin.provenance != 0)
            .count(),
        morph_storage
            .deltas
            .as_slice()
            .iter()
            .filter(|delta| delta.provenance != 0)
            .count(),
        animation_storage.non_primary_provenance_count(),
    ]
    .into_iter()
    .try_fold(0usize, |count, value| count.checked_add(value))
    .ok_or_else(|| ctx.allocation_failure("gltf-profile-owner-source-count"))?;
    let owner_source_count = owner_count
        .checked_add(additional_owner_source_count)
        .and_then(|count| count.checked_add(selected_sources.link_count()))
        .ok_or_else(|| ctx.allocation_failure("gltf-profile-owner-source-count"))?;
    let mut owner_sources = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-owner-sources",
    );
    let mut owner_source_values = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-owner-source-values",
    );
    owner_sources.try_reserve(owner_count)?;
    owner_source_values.try_reserve(owner_source_count)?;
    push_owner_source(
        ctx,
        &mut owner_sources,
        &mut owner_source_values,
        selected_sources,
        resource_count,
        ProfileOwner::Scene,
        0,
    )?;
    push_owner_source(
        ctx,
        &mut owner_sources,
        &mut owner_source_values,
        selected_sources,
        resource_count,
        ProfileOwner::SyntheticRoot,
        0,
    )?;
    for index in 0..nodes.nodes.len() {
        push_owner_source(
            ctx,
            &mut owner_sources,
            &mut owner_source_values,
            selected_sources,
            resource_count,
            ProfileOwner::Node(NodeLocalId::new(authoring_u32(
                ctx,
                index,
                "gltf-profile-node-owner",
            )?)),
            0,
        )?;
    }
    for index in 0..cameras.authored.len() {
        push_owner_source(
            ctx,
            &mut owner_sources,
            &mut owner_source_values,
            selected_sources,
            resource_count,
            ProfileOwner::Camera(authoring_u32(ctx, index, "gltf-profile-camera-owner")?),
            0,
        )?;
    }
    for (index, image) in appearance_storage.images.as_slice().iter().enumerate() {
        let provenance = image.provenance;
        let index = authoring_u32(ctx, index, "gltf-profile-image-owner")?;
        push_owner_source(
            ctx,
            &mut owner_sources,
            &mut owner_source_values,
            selected_sources,
            resource_count,
            ProfileOwner::Image(index),
            provenance,
        )?;
    }
    for (index, &provenance) in appearance_storage
        .sampler_provenance
        .as_slice()
        .iter()
        .enumerate()
    {
        let index = authoring_u32(ctx, index, "gltf-profile-sampler-owner")?;
        push_owner_source(
            ctx,
            &mut owner_sources,
            &mut owner_source_values,
            selected_sources,
            resource_count,
            ProfileOwner::Sampler(index),
            provenance,
        )?;
    }
    for (index, &provenance) in appearance_storage
        .texture_provenance
        .as_slice()
        .iter()
        .enumerate()
    {
        let index = authoring_u32(ctx, index, "gltf-profile-texture-owner")?;
        push_owner_source(
            ctx,
            &mut owner_sources,
            &mut owner_source_values,
            selected_sources,
            resource_count,
            ProfileOwner::Texture(index),
            provenance,
        )?;
    }
    for (index, binding) in appearance_storage.bindings.as_slice().iter().enumerate() {
        let index = authoring_u32(ctx, index, "gltf-profile-texture-binding-owner")?;
        push_owner_source(
            ctx,
            &mut owner_sources,
            &mut owner_source_values,
            selected_sources,
            resource_count,
            ProfileOwner::TextureBinding(index),
            binding.provenance,
        )?;
    }
    push_indexed_owner_sources(
        ctx,
        &mut owner_sources,
        &mut owner_source_values,
        selected_sources,
        resource_count,
        materials.len(),
        ProfileOwner::Material,
    )?;
    push_indexed_owner_sources(
        ctx,
        &mut owner_sources,
        &mut owner_source_values,
        selected_sources,
        resource_count,
        logical_meshes.len(),
        ProfileOwner::LogicalMesh,
    )?;
    let mut primitive_owner_index = 0usize;
    let mut attribute_owner_index = 0usize;
    for mesh in geometry_storage.logical_meshes.as_slice() {
        for primitive in mesh.primitives.as_slice() {
            let provenance_end = primitive
                .provenance_offset
                .checked_add(primitive.provenance_count)
                .ok_or_else(|| ctx.allocation_failure("gltf-profile-primitive-provenance"))?;
            let provenance = geometry_storage
                .primitive_provenance
                .as_slice()
                .get(primitive.provenance_offset..provenance_end)
                .ok_or_else(|| gltf_parse_error(ctx, "glTF primitive provenance is incomplete"))?;
            push_owner_sources(
                ctx,
                &mut owner_sources,
                &mut owner_source_values,
                selected_sources,
                resource_count,
                ProfileOwner::Primitive(authoring_u32(
                    ctx,
                    primitive_owner_index,
                    "gltf-profile-primitive-owner",
                )?),
                provenance.iter().copied(),
            )?;
            primitive_owner_index = primitive_owner_index
                .checked_add(1)
                .ok_or_else(|| ctx.allocation_failure("gltf-profile-primitive-owner"))?;
            for attribute in primitive.attributes.as_slice() {
                push_owner_source(
                    ctx,
                    &mut owner_sources,
                    &mut owner_source_values,
                    selected_sources,
                    resource_count,
                    ProfileOwner::Attribute(authoring_u32(
                        ctx,
                        attribute_owner_index,
                        "gltf-profile-attribute-owner",
                    )?),
                    attribute.provenance,
                )?;
                attribute_owner_index = attribute_owner_index
                    .checked_add(1)
                    .ok_or_else(|| ctx.allocation_failure("gltf-profile-attribute-owner"))?;
            }
        }
    }
    push_indexed_owner_sources(
        ctx,
        &mut owner_sources,
        &mut owner_source_values,
        selected_sources,
        resource_count,
        mesh_instances.len(),
        ProfileOwner::MeshInstance,
    )?;
    for (index, skin) in skins.skins.as_slice().iter().enumerate() {
        push_owner_source(
            ctx,
            &mut owner_sources,
            &mut owner_source_values,
            selected_sources,
            resource_count,
            ProfileOwner::Skin(authoring_u32(ctx, index, "gltf-profile-skin-owner")?),
            skin.provenance,
        )?;
    }
    push_indexed_owner_sources(
        ctx,
        &mut owner_sources,
        &mut owner_source_values,
        selected_sources,
        resource_count,
        morph_storage.definitions.len(),
        ProfileOwner::MorphDefinition,
    )?;
    for (index, delta) in morph_storage.deltas.as_slice().iter().enumerate() {
        ctx.charge_work(1)?;
        push_owner_source(
            ctx,
            &mut owner_sources,
            &mut owner_source_values,
            selected_sources,
            resource_count,
            ProfileOwner::MorphDelta(authoring_u32(ctx, index, "gltf-profile-morph-delta-owner")?),
            delta.provenance,
        )?;
    }
    for index in 0..animation_storage.clip_count() {
        let provenance = animation_storage
            .clip_provenance(index)
            .ok_or_else(|| gltf_parse_error(ctx, "glTF animation clip provenance is incomplete"))?;
        push_owner_sources(
            ctx,
            &mut owner_sources,
            &mut owner_source_values,
            selected_sources,
            resource_count,
            ProfileOwner::AnimationClip(authoring_u32(
                ctx,
                index,
                "gltf-profile-animation-clip-owner",
            )?),
            provenance.iter().copied(),
        )?;
    }
    for index in 0..animation_storage.channel_count() {
        let provenance = animation_storage.channel_provenance(index).ok_or_else(|| {
            gltf_parse_error(ctx, "glTF animation channel provenance is incomplete")
        })?;
        push_owner_sources(
            ctx,
            &mut owner_sources,
            &mut owner_source_values,
            selected_sources,
            resource_count,
            ProfileOwner::AnimationChannel(authoring_u32(
                ctx,
                index,
                "gltf-profile-animation-channel-owner",
            )?),
            provenance.iter().copied(),
        )?;
    }

    let projection_sources = selected_sources.for_owner(ctx, ProfileOwner::SyntheticRoot)?;
    let [projection] = projection_sources else {
        return Err(gltf_parse_error(
            ctx,
            "glTF synthetic root has no unique projection",
        ));
    };
    let projection_tag = match projection.source.kind {
        crate::source_map::SourceEntityKind::Document => 0_u8,
        crate::source_map::SourceEntityKind::Scene => 1_u8,
        _ => {
            return Err(gltf_parse_error(
                ctx,
                "glTF synthetic root projection kind is invalid",
            ));
        }
    };
    let projection_index = u64::try_from(projection.source.source_index)
        .map_err(|_| ctx.allocation_failure("gltf-profile-projection-index"))?;
    let (root_options, root_evidence) = generated_evidence(
        ctx,
        "org.baozi.gltf:projection-root-v1",
        projection_tag,
        projection_index,
    )?;
    let root_kind = OperationKind::Generated;
    let root_attachment = [ProvenanceLocalId::new(authoring_u32(
        ctx,
        provenance_nodes.len(),
        "gltf-profile-generated-root",
    )?)];
    provenance_nodes.try_push_reserved(ProvenanceNodeAuthoring::operation(
        ProvenanceOperationAuthoring::new(
            &root_kind,
            root_options,
            owner_sources_for(
                ctx,
                &owner_sources,
                &owner_source_values,
                ProfileOwner::SyntheticRoot,
            )?,
            &root_evidence,
        ),
    ))?;

    let default_owner = appearance_storage
        .default_sampler
        .map(|sampler| ProfileOwner::Sampler(sampler.index()));
    let (default_options, default_evidence) =
        generated_evidence(ctx, "org.baozi.gltf:implicit-sampler-v1", 0, 0)?;
    let default_kind = OperationKind::Default;
    let default_attachment = [ProvenanceLocalId::new(authoring_u32(
        ctx,
        provenance_nodes.len(),
        "gltf-profile-default-sampler",
    )?)];
    if let Some(owner) = default_owner {
        provenance_nodes.try_push_reserved(ProvenanceNodeAuthoring::operation(
            ProvenanceOperationAuthoring::new(
                &default_kind,
                default_options,
                owner_sources_for(ctx, &owner_sources, &owner_source_values, owner)?,
                &default_evidence,
            ),
        ))?;
    }

    let mut owner_provenance = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-owner-provenance",
    );
    owner_provenance.try_reserve(owner_sources.len())?;
    for owner in owner_sources.as_slice() {
        let end = owner
            .provenance_offset
            .checked_add(owner.provenance_count)
            .ok_or_else(|| ctx.allocation_failure("gltf-profile-owner-provenance-range"))?;
        let source_parents = owner_source_values
            .as_slice()
            .get(owner.provenance_offset..end)
            .ok_or_else(|| gltf_parse_error(ctx, "glTF owner provenance is incomplete"))?;
        let provenance = if owner.owner == ProfileOwner::SyntheticRoot {
            root_attachment.as_slice()
        } else if Some(owner.owner) == default_owner {
            default_attachment.as_slice()
        } else {
            source_parents
        };
        owner_provenance
            .try_push_reserved(OwnerProvenanceAuthoring::new(owner.owner, provenance))?;
    }

    let source_name_key = NamespacedName::new("org.baozi.gltf:source-name")
        .map_err(|_| gltf_parse_error(ctx, "static glTF metadata key is invalid"))?;
    let mut metadata = ctx.metered_vec(
        ImportStorageClass::SceneConstruction,
        "gltf-complete-batch-metadata",
    );
    for (index, name) in cameras.names.as_slice().iter().enumerate() {
        if let Some(name) = name {
            metadata.try_push(MetadataAuthoringEntry::new(
                ProfileOwner::Camera(authoring_u32(ctx, index, "gltf-metadata-camera")?),
                &source_name_key,
                ExtensionValueRef::String(name.as_str()),
            ))?;
        }
    }
    for (index, name) in appearance_storage
        .texture_names
        .as_slice()
        .iter()
        .enumerate()
    {
        let index = authoring_u32(ctx, index, "gltf-metadata-image")?;
        if let Some(name) = name.as_ref() {
            metadata.try_push(MetadataAuthoringEntry::new(
                ProfileOwner::Texture(index),
                &source_name_key,
                ExtensionValueRef::String(name.as_str()),
            ))?;
        }
    }

    let profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(
            resources.as_slice(),
            source_identities.as_slice(),
            provenance_nodes.as_slice(),
            owner_provenance.as_slice(),
        ),
        MetadataAuthoringBatch::new(metadata.as_slice()),
    )
    .with_resource_snapshots(snapshots.as_slice());
    animation_storage.with_authoring_batch(ctx, |animations| {
        Ok(ctx.submit_scene(SceneAuthoringBatch::new(
            spatial,
            node_batch,
            appearance,
            geometry,
            deformation,
            animations,
            profile,
        ))?)
    })
}

fn owner_sources_for<'a>(
    ctx: &ParseScope<'_>,
    owners: &MeteredVec<'_, OwnerSource>,
    values: &'a MeteredVec<'_, ProvenanceLocalId>,
    target: ProfileOwner,
) -> Result<&'a [ProvenanceLocalId]> {
    for owner in owners.as_slice() {
        ctx.charge_work(1)?;
        if owner.owner == target {
            let end = owner
                .provenance_offset
                .checked_add(owner.provenance_count)
                .ok_or_else(|| ctx.allocation_failure("gltf-profile-generated-parents"))?;
            return values
                .as_slice()
                .get(owner.provenance_offset..end)
                .ok_or_else(|| {
                    gltf_parse_error(ctx, "glTF generated owner has incomplete source parents")
                });
        }
    }
    Err(gltf_parse_error(
        ctx,
        "glTF generated owner has no source parents",
    ))
}

// The v1 instruction digest identifies the fixed normalization rule and source projection.
// Its source parents bind the generated owner to actual declarations without inventing a locator.
fn generated_evidence(
    ctx: &ParseScope<'_>,
    code: &'static str,
    projection_kind: u8,
    projection_index: u64,
) -> Result<(OptionsDigest, [OperationEvidenceSummary; 1])> {
    ctx.charge_work(code.len() as u64 + 9)?;
    let mut digest = Sha256::new();
    digest.update(code.as_bytes());
    digest.update([projection_kind]);
    digest.update(projection_index.to_be_bytes());
    let bytes: [u8; 32] = digest.finalize().into();
    let code = NamespacedName::new(code)
        .map_err(|_| gltf_parse_error(ctx, "static glTF generated evidence code is invalid"))?;
    let evidence = OperationEvidenceSummary::new(code, 1, Some(EvidenceDigest::sha256(bytes)))
        .map_err(|_| gltf_parse_error(ctx, "static glTF generated evidence count is invalid"))?;
    Ok((OptionsDigest::sha256(bytes), [evidence]))
}

fn push_owner_source(
    ctx: &ParseScope<'_>,
    owners: &mut MeteredVec<'_, OwnerSource>,
    values: &mut MeteredVec<'_, ProvenanceLocalId>,
    selected_sources: &SelectedSourceProfile<'_>,
    resource_count: usize,
    owner: ProfileOwner,
    provenance: u32,
) -> Result<()> {
    push_owner_sources(
        ctx,
        owners,
        values,
        selected_sources,
        resource_count,
        owner,
        [provenance],
    )
}

fn push_owner_sources(
    ctx: &ParseScope<'_>,
    owners: &mut MeteredVec<'_, OwnerSource>,
    values: &mut MeteredVec<'_, ProvenanceLocalId>,
    selected_sources: &SelectedSourceProfile<'_>,
    resource_count: usize,
    owner: ProfileOwner,
    provenance: impl IntoIterator<Item = u32>,
) -> Result<()> {
    let offset = values.len();
    values.try_push(ProvenanceLocalId::new(0))?;
    let mut previous = 0;
    for source in provenance {
        ctx.charge_work(1)?;
        if source == 0 {
            continue;
        }
        if source <= previous {
            return Err(ContentError::from(GltfFailure::internal(
                "owner provenance sources are not strictly ordered",
            )));
        }
        values.try_push(ProvenanceLocalId::new(source))?;
        previous = source;
    }
    for link in selected_sources.for_owner(ctx, owner)? {
        ctx.charge_work(1)?;
        let source = selected_sources.provenance(ctx, resource_count, link.source)?;
        if source == 0 {
            continue;
        }
        if source <= previous {
            return Err(GltfFailure::internal(
                "selected glTF owner source identities are not ordered",
            )
            .into());
        }
        values.try_push(ProvenanceLocalId::new(source))?;
        previous = source;
    }
    owners.try_push(OwnerSource {
        owner,
        provenance_offset: offset,
        provenance_count: values.len() - offset,
    })?;
    Ok(())
}

fn push_indexed_owner_sources(
    ctx: &ParseScope<'_>,
    owners: &mut MeteredVec<'_, OwnerSource>,
    values: &mut MeteredVec<'_, ProvenanceLocalId>,
    selected_sources: &SelectedSourceProfile<'_>,
    resource_count: usize,
    count: usize,
    owner: impl Fn(u32) -> ProfileOwner,
) -> Result<()> {
    for index in 0..count {
        push_owner_source(
            ctx,
            owners,
            values,
            selected_sources,
            resource_count,
            owner(authoring_u32(ctx, index, "gltf-profile-owner-index")?),
            0,
        )?;
    }
    Ok(())
}
