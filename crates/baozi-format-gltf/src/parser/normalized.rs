//! Canonical authoring from the owned, backend-independent glTF model.

use super::{
    ContentError, GltfFailure, Result, animation, authoring_u32, gltf_parse_error,
    model::*,
    morph, poll_parser_loop,
    source_profile::{self, SelectedSourceProfile},
    to_u64, uri, with_content,
};
use crate::FORMAT_ID;
use baozi_import::ImportWorkClass;
use baozi_import::authoring::*;
use baozi_import::{
    ImportStorageClass, MeteredVec, ParseContext, ParseRoot, SourceScopeSelectionRequest,
};
use sha2::{Digest, Sha256};

#[cfg(test)]
mod tests;

pub(super) fn submit<'session>(
    context: &mut ParseContext<'session, '_>,
    model: &NormalizedGltf<'session>,
) -> Result<()> {
    with_content(context, |ctx| {
        model
            .sources
            .required(ctx, source_profile::DOCUMENT)
            .map(|_| ())
    })?;
    submit_parts(
        context,
        &model.appearance,
        &model.geometry,
        &model.cameras,
        &model.lights,
        &model.nodes,
        &model.skins,
        &model.instance_deformations,
        &model.buffers,
        &model.external_images,
        &model.animations,
        &model.morphs,
        &model.sources,
        model.selection_request,
    )
}

struct OwnerSource {
    owner: ProfileOwner,
    provenance_offset: usize,
    provenance_count: usize,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn submit_parts<'session>(
    context: &mut ParseContext<'session, '_>,
    appearance_storage: &AppearanceStorage<'session>,
    geometry_storage: &GeometryStorage<'session>,
    cameras: &CameraStorage<'session>,
    lights: &super::lights::LightStorage<'session>,
    nodes: &NodeStorage<'session>,
    skins: &SkinStorage<'session>,
    instance_deformations: &MeteredVec<'session, InstanceDeformationStorage<'session>>,
    buffers: &BufferResources<'session>,
    external_images: &ExternalImageResources<'session>,
    animation_storage: &animation::AnimationStorage<'session>,
    morph_storage: &morph::MorphStorage<'session>,
    selected_sources: &SelectedSourceProfile<'session>,
    selection_request: SourceScopeSelectionRequest,
) -> Result<()> {
    // Each descriptor layer borrows stable outer storage from the preceding layer.
    // Only the outer context can submit after every execution-region borrow ends.
    let (
        images,
        bindings,
        materials,
        logical_meshes,
        primitives,
        attributes,
        mesh_instances,
        skin_authoring,
        logical_morphs,
        morph_deltas,
    ) = with_content(context, |ctx| -> Result<_> {
        let mut images = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-images",
        );
        images.try_reserve(&mut ctx.services()?, appearance_storage.images.len())?;
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
                            gltf_parse_error("glTF encoded image payload map is incomplete")
                        })?;
                    ImageAuthoring::encoded(bytes.as_slice(), media_type)
                }
                ImagePayloadStorage::SessionResource(resource) => {
                    ImageAuthoring::session_resource(*resource, media_type)
                }
            };
            images.try_push_reserved(authored)?;
        }

        let mut bindings = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-material-bindings",
        );
        bindings.try_reserve(&mut ctx.services()?, appearance_storage.bindings.len())?;
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
            .map_err(|_| gltf_parse_error("glTF texture binding semantics are invalid"))?;
            bindings.try_push_reserved(authored)?;
        }

        let mut materials = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-materials",
        );
        materials.try_reserve(&mut ctx.services()?, appearance_storage.materials.len())?;
        for material in appearance_storage.materials.as_slice() {
            let mut authored = MaterialAuthoring::try_from_parts(
                material.bindings,
                material.shading_model,
                material.alpha_mode,
                material.double_sided,
                material.factors,
            )
            .map_err(|_| gltf_parse_error("glTF material semantics are invalid"))?;
            if let Some(name) = material.name.as_ref() {
                authored = authored.with_name(name.as_str());
            }
            materials.try_push_reserved(authored)?;
        }

        let mut logical_meshes = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-logical-meshes",
        );
        let mut primitives = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-primitives",
        );
        let mut attributes = ctx.services()?.metered_vec(
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
        logical_meshes.try_reserve(&mut ctx.services()?, geometry_storage.logical_meshes.len())?;
        primitives.try_reserve(&mut ctx.services()?, primitive_count)?;
        attributes.try_reserve(&mut ctx.services()?, attribute_count)?;
        for mesh in geometry_storage.logical_meshes.as_slice() {
            let primitive_offset = authoring_u32(
                &mut ctx.services()?,
                primitives.len(),
                "gltf-complete-batch-primitive-offset",
            )?;
            for primitive in mesh.primitives.as_slice() {
                let attribute_offset = authoring_u32(
                    &mut ctx.services()?,
                    attributes.len(),
                    "gltf-complete-batch-attribute-offset",
                )?;
                for attribute in primitive.attributes.as_slice() {
                    let mut authored = GeometryAttributeAuthoring::f32(
                        attribute.semantic.clone(),
                        AttributeDomain::Vertex,
                        attribute.lanes,
                        attribute.values.as_slice(),
                        Presence::Absent,
                    );
                    if attribute.semantic == AttributeSemantic::Tangent {
                        // glTF defines bitangent reconstruction without identifying the
                        // construction algorithm.
                        authored = authored.with_tangent_conventions(TangentConventions {
                            basis: TangentBasisConvention::Unknown,
                            encoding: TangentHandednessEncoding::SignInW,
                        });
                    }
                    attributes.try_push_reserved(authored)?;
                }
                let indices = if primitive.has_indices {
                    Presence::Present(primitive.indices.as_slice())
                } else {
                    Presence::Absent
                };
                primitives.try_push_reserved(GeometryPrimitiveAuthoring::new(
                    primitive.topology,
                    indices,
                    primitive.material_assignment,
                    AuthoringRange::new(
                        attribute_offset,
                        authoring_u32(
                            &mut ctx.services()?,
                            primitive.attributes.len(),
                            "gltf-complete-batch-attribute-count",
                        )?,
                    ),
                ))?;
            }
            let mut authored = GeometryLogicalMeshAuthoring::new(AuthoringRange::new(
                primitive_offset,
                authoring_u32(
                    &mut ctx.services()?,
                    mesh.primitives.len(),
                    "gltf-complete-batch-logical-mesh-primitive-count",
                )?,
            ));
            if let Some(name) = mesh.name.as_ref() {
                authored = authored.with_name(name.as_str());
            }
            logical_meshes.try_push_reserved(authored)?;
        }

        let mut mesh_instances = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-mesh-instances",
        );
        mesh_instances.try_reserve(&mut ctx.services()?, nodes.mesh_instances.len())?;
        for instance in nodes.mesh_instances.as_slice() {
            mesh_instances.try_push_reserved(instance.authored)?;
        }

        let mut skin_authoring = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-skins",
        );
        skin_authoring.try_reserve(&mut ctx.services()?, skins.skins.len())?;
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

        let mut logical_morphs = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-logical-morphs",
        );
        logical_morphs.try_reserve(&mut ctx.services()?, logical_meshes.len())?;
        if morph_storage.logical_meshes.len() != logical_meshes.len()
            || morph_storage.primitives.len() != primitives.len()
        {
            return Err(gltf_parse_error("glTF morph tables disagree with geometry"));
        }
        let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        for mesh in morph_storage.logical_meshes.as_slice() {
            let authored = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(LogicalMorphAuthoring::new(
                        mesh.definitions,
                        mesh.weights.as_slice(),
                    ))
                })
            })?;
            logical_morphs.try_push_reserved(authored)?;
        }
        let mut morph_deltas = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-morph-deltas",
        );
        morph_deltas.try_reserve(&mut ctx.services()?, morph_storage.deltas.len())?;
        for delta in morph_storage.deltas.as_slice() {
            let authored = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(morph::delta_authoring(delta))
                })
            })?;
            morph_deltas.try_push_reserved(authored)?;
        }
        Ok((
            images,
            bindings,
            materials,
            logical_meshes,
            primitives,
            attributes,
            mesh_instances,
            skin_authoring,
            logical_morphs,
            morph_deltas,
        ))
    })?;

    let morph_targets = with_content(context, |ctx| -> Result<_> {
        let mut morph_targets = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-morph-targets",
        );
        morph_targets.try_reserve(&mut ctx.services()?, morph_storage.targets.len())?;
        let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        for target in morph_storage.targets.as_slice() {
            let authored = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(morph::target_authoring(
                        target,
                        morph_deltas.as_slice(),
                    ))
                })
            })?;
            morph_targets.try_push_reserved(authored)?;
        }
        Ok(morph_targets)
    })?;

    let primitive_deformations = with_content(context, |ctx| -> Result<_> {
        let mut primitive_deformations = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-primitive-deformations",
        );
        primitive_deformations.try_reserve(&mut ctx.services()?, primitives.len())?;
        let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
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
                let range = ctx
                    .child(&work_group, [1], |child| {
                        child.run([1], || {
                            Ok::<_, baozi_import::ParseFailure>(
                                morph_storage
                                    .primitives
                                    .as_slice()
                                    .get(primitive_deformations.len()),
                            )
                        })
                    })?
                    .ok_or_else(|| {
                        gltf_parse_error("glTF primitive morph target range is missing")
                    })?;
                let targets = morph_targets.as_slice().get(range.clone()).ok_or_else(|| {
                    gltf_parse_error("glTF primitive morph target range is invalid")
                })?;
                primitive_deformations
                    .try_push_reserved(PrimitiveDeformationAuthoring::new(influences, targets))?;
            }
        }

        Ok(primitive_deformations)
    })?;

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
    let mesh_instance_deformations = with_content(context, |ctx| -> Result<_> {
        let mut mesh_instance_deformations = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-instance-deformations",
        );
        mesh_instance_deformations
            .try_reserve(&mut ctx.services()?, instance_deformations.len())?;
        let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        for instance in instance_deformations.as_slice() {
            let authored = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(MeshInstanceDeformationAuthoring::new(
                        instance.skin,
                        instance
                            .weights
                            .as_ref()
                            .map_or(Presence::Absent, |weights| {
                                Presence::Present(weights.as_slice())
                            }),
                    ))
                })
            })?;
            mesh_instance_deformations.try_push_reserved(authored)?;
        }
        Ok(mesh_instance_deformations)
    })?;

    let deformation = DeformationAuthoringBatch::new(
        skin_authoring.as_slice(),
        morph_storage.definitions.as_slice(),
        logical_morphs.as_slice(),
        primitive_deformations.as_slice(),
        mesh_instance_deformations.as_slice(),
    );
    // The asset front is +Z; camera-local viewing axes are a separate declaration.
    let scene_space = SceneSpace {
        handedness: Handedness::Right,
        up_axis: Axis::PositiveY,
        front_axis: Axis::PositiveZ,
        unit_scale_to_meters: Some(1.0),
        front_face_winding: FrontFaceWinding::CounterClockwise,
        uv_origin: UvOrigin::UpperLeft,
    };
    let spatial = SpatialAuthoringBatch::new(
        &scene_space,
        cameras.authored.as_slice(),
        lights.authored.as_slice(),
    );
    let authored_nodes = with_content(context, |ctx| -> Result<_> {
        let mut authored_nodes = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-nodes",
        );
        authored_nodes.try_reserve(&mut ctx.services()?, nodes.nodes.len())?;
        if nodes.names.len() != nodes.nodes.len() {
            return Err(gltf_parse_error(
                "glTF normalized node names are incomplete",
            ));
        }
        let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        for (node, name) in nodes.nodes.as_slice().iter().zip(nodes.names.as_slice()) {
            let node = ctx.child(&work_group, [1], |child| {
                child.run([1], || {
                    Ok::<_, baozi_import::ParseFailure>(
                        name.as_ref()
                            .map_or(*node, |name| node.with_name(name.as_str())),
                    )
                })
            })?;
            authored_nodes.try_push_reserved(node)?;
        }
        Ok(authored_nodes)
    })?;

    let node_batch = NodeAuthoringBatch::new(
        Presence::Present(nodes.roots.as_slice()),
        authored_nodes.as_slice(),
    );
    let primary_identity = with_content(context, |ctx| {
        let argument_0 = ctx.services()?.source().as_str();
        uri::profile_identity(ctx, argument_0)
    })?;
    let (resource_count, resources, snapshots, locators) =
        with_content(context, |ctx| -> Result<_> {
            let resource_count = buffers
                .external_resource_count()
                .checked_add(external_images.values.len())
                .and_then(|count| count.checked_add(1))
                .ok_or_else(|| ctx.allocation_failure("gltf-profile-resource-count"))?;
            let mut resources = ctx.services()?.metered_vec(
                ImportStorageClass::SceneConstruction,
                "gltf-complete-batch-resources",
            );
            let mut snapshots = ctx.services()?.metered_vec(
                ImportStorageClass::SceneConstruction,
                "gltf-complete-batch-resource-snapshots",
            );
            let mut locators = ctx.services()?.metered_vec(
                ImportStorageClass::SceneConstruction,
                "gltf-complete-batch-source-locators",
            );
            resources.try_reserve(&mut ctx.services()?, resource_count)?;
            snapshots.try_reserve(&mut ctx.services()?, resource_count)?;
            locators.try_reserve(&mut ctx.services()?, resource_count - 1)?;
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

            Ok((resource_count, resources, snapshots, locators))
        })?;

    let format = with_content(context, |_ctx| -> Result<_> {
        FormatId::new(FORMAT_ID)
            .map_err(|_| gltf_parse_error("static glTF format identity is invalid"))
    })?;
    let (source_identities, mut provenance_nodes, owner_sources, owner_source_values) =
        with_content(context, |ctx| -> Result<_> {
            let mut source_identities = ctx.services()?.metered_vec(
                ImportStorageClass::SceneConstruction,
                "gltf-complete-batch-source-identities",
            );
            let mut provenance_nodes = ctx.services()?.metered_vec(
                ImportStorageClass::SceneConstruction,
                "gltf-complete-batch-provenance-nodes",
            );
            let source_count = resource_count
                .checked_add(selected_sources.entries().len() - 1)
                .ok_or_else(|| ctx.allocation_failure("gltf-profile-source-count"))?;
            source_identities.try_reserve(&mut ctx.services()?, source_count)?;
            let operation_count = 1
                + usize::from(appearance_storage.default_sampler.is_some())
                + usize::from(appearance_storage.default_material.is_some());
            {
                let value = source_count
                    .checked_add(operation_count)
                    .ok_or_else(|| ctx.allocation_failure("gltf-profile-provenance-count"))?;
                provenance_nodes.try_reserve(&mut ctx.services()?, value)
            }?;
            let document_locator = &selected_sources
                .required(ctx, source_profile::DOCUMENT)?
                .locator;
            let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            ctx.child(
                &work_group,
                [to_u64(resource_count)],
                |child| -> Result<()> {
                    for index in 0..resource_count {
                        poll_parser_loop(&mut child.services()?, index)?;
                        let errors = child.error_view();
                        child.run([1], || -> Result<()> {
                            let local = u32::try_from(index).map_err(|_| {
                                errors.allocation_failure("gltf-profile-source-identity")
                            })?;
                            let locator = if index == 0 {
                                document_locator
                            } else {
                                locators.as_slice().get(index - 1).ok_or_else(|| {
                                    gltf_parse_error(
                                        "glTF profile source locator table is incomplete",
                                    )
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
                            Ok(())
                        })?;
                        #[cfg(test)]
                        crate::control_tests::resource_source_completed(index + 1);
                    }
                    Ok(())
                },
            )?;
            for source in selected_sources.entries().iter().skip(1) {
                ctx.child(&work_group, [1], |child| -> Result<()> {
                    let errors = child.error_view();
                    child.run([1], || -> Result<()> {
                        let local = u32::try_from(source_identities.len()).map_err(|_| {
                            errors.allocation_failure("gltf-profile-source-identity")
                        })?;
                        source_identities.try_push_reserved(SourceIdentityAuthoring::new(
                            ResourceIdentityLocalId::new(0),
                            &format,
                            &source.locator,
                        ))?;
                        provenance_nodes.try_push_reserved(ProvenanceNodeAuthoring::source(
                            SourceIdentityLocalId::new(local),
                        ))?;
                        Ok(())
                    })
                })?;
            }

            let owner_count = [
                2,
                nodes.nodes.len(),
                cameras.authored.len(),
                lights.authored.len(),
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
                non_primary_provenance_count(
                    ctx,
                    appearance_storage
                        .images
                        .as_slice()
                        .iter()
                        .map(|image| std::slice::from_ref(&image.provenance)),
                )?,
                non_primary_provenance_count(
                    ctx,
                    std::iter::once(appearance_storage.sampler_provenance.as_slice()),
                )?,
                non_primary_provenance_count(
                    ctx,
                    std::iter::once(appearance_storage.texture_provenance.as_slice()),
                )?,
                non_primary_provenance_count(
                    ctx,
                    appearance_storage
                        .bindings
                        .as_slice()
                        .iter()
                        .map(|binding| std::slice::from_ref(&binding.provenance)),
                )?,
                non_primary_provenance_count(
                    ctx,
                    std::iter::once(geometry_storage.primitive_provenance.as_slice()),
                )?,
                non_primary_provenance_count(
                    ctx,
                    geometry_storage
                        .logical_meshes
                        .as_slice()
                        .iter()
                        .flat_map(|mesh| mesh.primitives.as_slice())
                        .flat_map(|primitive| primitive.attributes.as_slice())
                        .map(|attribute| attribute.provenance.as_slice()),
                )?,
                non_primary_provenance_count(
                    ctx,
                    skins
                        .skins
                        .as_slice()
                        .iter()
                        .map(|skin| skin.provenance.as_slice()),
                )?,
                non_primary_provenance_count(
                    ctx,
                    morph_storage
                        .deltas
                        .as_slice()
                        .iter()
                        .map(|delta| delta.provenance.as_slice()),
                )?,
                animation_storage.non_primary_provenance_count(ctx)?,
            ]
            .into_iter()
            .try_fold(0usize, |count, value| count.checked_add(value))
            .ok_or_else(|| ctx.allocation_failure("gltf-profile-owner-source-count"))?;
            let owner_source_count = owner_count
                .checked_add(additional_owner_source_count)
                .and_then(|count| count.checked_add(selected_sources.link_count()))
                .ok_or_else(|| ctx.allocation_failure("gltf-profile-owner-source-count"))?;
            let mut owner_sources = ctx.services()?.metered_vec(
                ImportStorageClass::SceneConstruction,
                "gltf-complete-batch-owner-sources",
            );
            let mut owner_source_values = ctx.services()?.metered_vec(
                ImportStorageClass::SceneConstruction,
                "gltf-complete-batch-owner-source-values",
            );
            owner_sources.try_reserve(&mut ctx.services()?, owner_count)?;
            owner_source_values.try_reserve(&mut ctx.services()?, owner_source_count)?;
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
                {
                    let argument_0 = ProfileOwner::Node(NodeLocalId::new(authoring_u32(
                        &mut ctx.services()?,
                        index,
                        "gltf-profile-node-owner",
                    )?));
                    push_owner_source(
                        ctx,
                        &mut owner_sources,
                        &mut owner_source_values,
                        selected_sources,
                        resource_count,
                        argument_0,
                        0,
                    )
                }?;
            }
            for index in 0..cameras.authored.len() {
                {
                    let argument_0 = ProfileOwner::Camera(authoring_u32(
                        &mut ctx.services()?,
                        index,
                        "gltf-profile-camera-owner",
                    )?);
                    push_owner_source(
                        ctx,
                        &mut owner_sources,
                        &mut owner_source_values,
                        selected_sources,
                        resource_count,
                        argument_0,
                        0,
                    )
                }?;
            }
            for index in 0..lights.authored.len() {
                {
                    let argument_0 = ProfileOwner::Light(authoring_u32(
                        &mut ctx.services()?,
                        index,
                        "gltf-profile-light-owner",
                    )?);
                    push_owner_source(
                        ctx,
                        &mut owner_sources,
                        &mut owner_source_values,
                        selected_sources,
                        resource_count,
                        argument_0,
                        0,
                    )
                }?;
            }
            for (index, image) in appearance_storage.images.as_slice().iter().enumerate() {
                let provenance = image.provenance;
                let index = authoring_u32(&mut ctx.services()?, index, "gltf-profile-image-owner")?;
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
                let index =
                    authoring_u32(&mut ctx.services()?, index, "gltf-profile-sampler-owner")?;
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
                let index =
                    authoring_u32(&mut ctx.services()?, index, "gltf-profile-texture-owner")?;
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
                let index = authoring_u32(
                    &mut ctx.services()?,
                    index,
                    "gltf-profile-texture-binding-owner",
                )?;
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
                        .ok_or_else(|| {
                            ctx.allocation_failure("gltf-profile-primitive-provenance")
                        })?;
                    let provenance = geometry_storage
                        .primitive_provenance
                        .as_slice()
                        .get(primitive.provenance_offset..provenance_end)
                        .ok_or_else(|| {
                            gltf_parse_error("glTF primitive provenance is incomplete")
                        })?;
                    {
                        let argument_0 = ProfileOwner::Primitive(authoring_u32(
                            &mut ctx.services()?,
                            primitive_owner_index,
                            "gltf-profile-primitive-owner",
                        )?);
                        push_owner_sources(
                            ctx,
                            &mut owner_sources,
                            &mut owner_source_values,
                            selected_sources,
                            resource_count,
                            argument_0,
                            provenance.iter().copied(),
                        )
                    }?;
                    primitive_owner_index = primitive_owner_index
                        .checked_add(1)
                        .ok_or_else(|| ctx.allocation_failure("gltf-profile-primitive-owner"))?;
                    for attribute in primitive.attributes.as_slice() {
                        {
                            let argument_0 = ProfileOwner::Attribute(authoring_u32(
                                &mut ctx.services()?,
                                attribute_owner_index,
                                "gltf-profile-attribute-owner",
                            )?);
                            push_owner_sources(
                                ctx,
                                &mut owner_sources,
                                &mut owner_source_values,
                                selected_sources,
                                resource_count,
                                argument_0,
                                attribute.provenance.as_slice().iter().copied(),
                            )
                        }?;
                        attribute_owner_index =
                            attribute_owner_index.checked_add(1).ok_or_else(|| {
                                ctx.allocation_failure("gltf-profile-attribute-owner")
                            })?;
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
                {
                    let argument_0 = ProfileOwner::Skin(authoring_u32(
                        &mut ctx.services()?,
                        index,
                        "gltf-profile-skin-owner",
                    )?);
                    push_owner_sources(
                        ctx,
                        &mut owner_sources,
                        &mut owner_source_values,
                        selected_sources,
                        resource_count,
                        argument_0,
                        skin.provenance.as_slice().iter().copied(),
                    )
                }?;
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
                let provenance = ctx.child(&work_group, [1], |child| {
                    child.run([1], || {
                        Ok::<_, baozi_import::ParseFailure>(delta.provenance.as_slice())
                    })
                })?;
                {
                    let argument_0 = ProfileOwner::MorphDelta(authoring_u32(
                        &mut ctx.services()?,
                        index,
                        "gltf-profile-morph-delta-owner",
                    )?);
                    push_owner_sources(
                        ctx,
                        &mut owner_sources,
                        &mut owner_source_values,
                        selected_sources,
                        resource_count,
                        argument_0,
                        provenance.iter().copied(),
                    )
                }?;
            }
            for index in 0..animation_storage.clip_count() {
                let provenance = animation_storage.clip_provenance(index).ok_or_else(|| {
                    gltf_parse_error("glTF animation clip provenance is incomplete")
                })?;
                {
                    let argument_0 = ProfileOwner::AnimationClip(authoring_u32(
                        &mut ctx.services()?,
                        index,
                        "gltf-profile-animation-clip-owner",
                    )?);
                    push_owner_sources(
                        ctx,
                        &mut owner_sources,
                        &mut owner_source_values,
                        selected_sources,
                        resource_count,
                        argument_0,
                        provenance.iter().copied(),
                    )
                }?;
            }
            for index in 0..animation_storage.channel_count() {
                let provenance = animation_storage.channel_provenance(index).ok_or_else(|| {
                    gltf_parse_error("glTF animation channel provenance is incomplete")
                })?;
                {
                    let argument_0 = ProfileOwner::AnimationChannel(authoring_u32(
                        &mut ctx.services()?,
                        index,
                        "gltf-profile-animation-channel-owner",
                    )?);
                    push_owner_sources(
                        ctx,
                        &mut owner_sources,
                        &mut owner_source_values,
                        selected_sources,
                        resource_count,
                        argument_0,
                        provenance.iter().copied(),
                    )
                }?;
            }

            Ok((
                source_identities,
                provenance_nodes,
                owner_sources,
                owner_source_values,
            ))
        })?;

    let (root_options, root_evidence, root_attachment) =
        with_content(context, |ctx| -> Result<_> {
            let projection_sources =
                selected_sources.for_owner(ctx, ProfileOwner::SyntheticRoot)?;
            let [projection] = projection_sources else {
                return Err(gltf_parse_error(
                    "glTF synthetic root has no unique projection",
                ));
            };
            let projection = selected_sources.source(ctx, *projection)?.entity.id;
            let projection_tag = match projection.kind {
                crate::source_map::SourceEntityKind::Document => 0_u8,
                crate::source_map::SourceEntityKind::Scene => 1_u8,
                _ => {
                    return Err(gltf_parse_error(
                        "glTF synthetic root projection kind is invalid",
                    ));
                }
            };
            let projection_index = u64::try_from(projection.source_index)
                .map_err(|_| ctx.allocation_failure("gltf-profile-projection-index"))?;
            // Fixed framing: request mode/index, then resolved projection kind/index.
            // DocumentDefault has no requested locator and encodes its index lane as zero.
            let mut instruction = [0_u8; 18];
            if let SourceScopeSelectionRequest::ExplicitScene { scene_index } = selection_request {
                instruction[0] = 1;
                instruction[1..9].copy_from_slice(&scene_index.to_be_bytes());
            }
            instruction[9] = projection_tag;
            instruction[10..].copy_from_slice(&projection_index.to_be_bytes());
            let (root_options, root_evidence) =
                generated_evidence(ctx, "org.baozi.gltf:projection-root-v1", &instruction)?;
            let root_attachment = [ProvenanceLocalId::new(authoring_u32(
                &mut ctx.services()?,
                provenance_nodes.len(),
                "gltf-profile-generated-root",
            )?)];
            Ok((root_options, root_evidence, root_attachment))
        })?;

    let root_kind = OperationKind::Generated;
    with_content(context, |ctx| -> Result<()> {
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

        Ok(())
    })?;

    let default_sampler_owner = appearance_storage
        .default_sampler
        .map(|sampler| ProfileOwner::Sampler(sampler.index()));
    let (default_options, default_evidence, default_sampler_attachment) =
        with_content(context, |ctx| -> Result<_> {
            let (default_options, default_evidence) =
                generated_evidence(ctx, "org.baozi.gltf:implicit-sampler-v1", &[0; 9])?;
            let default_sampler_attachment = [ProvenanceLocalId::new(authoring_u32(
                &mut ctx.services()?,
                provenance_nodes.len(),
                "gltf-profile-default-sampler",
            )?)];
            Ok((
                default_options,
                default_evidence,
                default_sampler_attachment,
            ))
        })?;

    let default_kind = OperationKind::Default;
    with_content(context, |ctx| -> Result<()> {
        if let Some(owner) = default_sampler_owner {
            provenance_nodes.try_push_reserved(ProvenanceNodeAuthoring::operation(
                ProvenanceOperationAuthoring::new(
                    &default_kind,
                    default_options,
                    owner_sources_for(ctx, &owner_sources, &owner_source_values, owner)?,
                    &default_evidence,
                ),
            ))?;
        }

        Ok(())
    })?;

    let default_material_owner = appearance_storage
        .default_material
        .map(|material| ProfileOwner::Material(material.index()));
    let (material_generated, default_material_attachment) =
        with_content(context, |ctx| -> Result<_> {
            let material_generated = default_material_owner
                .map(|_| generated_evidence(ctx, "org.baozi.gltf:implicit-material-v1", &[]))
                .transpose()?;
            let default_material_attachment = [ProvenanceLocalId::new(authoring_u32(
                &mut ctx.services()?,
                provenance_nodes.len(),
                "gltf-profile-default-material",
            )?)];
            Ok((material_generated, default_material_attachment))
        })?;

    let material_kind = OperationKind::Generated;
    with_content(context, |ctx| -> Result<()> {
        if let (Some(owner), Some((material_options, material_evidence))) =
            (default_material_owner, material_generated.as_ref())
        {
            provenance_nodes.try_push_reserved(ProvenanceNodeAuthoring::operation(
                ProvenanceOperationAuthoring::new(
                    &material_kind,
                    *material_options,
                    owner_sources_for(ctx, &owner_sources, &owner_source_values, owner)?,
                    material_evidence,
                ),
            ))?;
        }

        Ok(())
    })?;

    let owner_provenance = with_content(context, |ctx| -> Result<_> {
        let mut owner_provenance = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-owner-provenance",
        );
        owner_provenance.try_reserve(&mut ctx.services()?, owner_sources.len())?;
        for owner in owner_sources.as_slice() {
            let end = owner
                .provenance_offset
                .checked_add(owner.provenance_count)
                .ok_or_else(|| ctx.allocation_failure("gltf-profile-owner-provenance-range"))?;
            let source_parents = owner_source_values
                .as_slice()
                .get(owner.provenance_offset..end)
                .ok_or_else(|| gltf_parse_error("glTF owner provenance is incomplete"))?;
            let provenance = if owner.owner == ProfileOwner::SyntheticRoot {
                root_attachment.as_slice()
            } else if Some(owner.owner) == default_sampler_owner {
                default_sampler_attachment.as_slice()
            } else if Some(owner.owner) == default_material_owner {
                default_material_attachment.as_slice()
            } else {
                source_parents
            };
            owner_provenance
                .try_push_reserved(OwnerProvenanceAuthoring::new(owner.owner, provenance))?;
        }

        Ok(owner_provenance)
    })?;

    let (source_name_key, extras_media_type) = with_content(context, |_ctx| -> Result<_> {
        let source_name_key = NamespacedName::new("org.baozi.gltf:source-name")
            .map_err(|_| gltf_parse_error("static glTF metadata key is invalid"))?;
        let extras_media_type = ExtensionMediaType::new(super::extras::MEDIA_TYPE)
            .map_err(|_| gltf_parse_error("static glTF extras media type is invalid"))?;
        Ok((source_name_key, extras_media_type))
    })?;
    let metadata = with_content(context, |ctx| -> Result<_> {
        let mut metadata = ctx.services()?.metered_vec(
            ImportStorageClass::SceneConstruction,
            "gltf-complete-batch-metadata",
        );
        let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
        for extras in selected_sources.extras() {
            let source_id = ctx.child(&work_group, [1], |child| {
                child.run([1], || Ok::<_, baozi_import::ParseFailure>(extras.source))
            })?;
            // Source leaves and source identities share the authoring index order. The metadata
            // references the source identity, never a canonical entity or a provenance attachment.
            let source = selected_sources.source_index(ctx, resource_count, source_id)?;
            {
                let value = MetadataAuthoringEntry::for_source(
                    SourceIdentityLocalId::new(source),
                    &extras.key,
                    ExtensionValueRef::Bytes {
                        media_type: &extras_media_type,
                        payload: extras.bytes(&mut ctx.services()?)?,
                    },
                );
                metadata.try_push(&mut ctx.services()?, value)
            }?;
        }
        for (index, name) in cameras.names.as_slice().iter().enumerate() {
            if let Some(name) = name {
                {
                    let value = MetadataAuthoringEntry::new(
                        ProfileOwner::Camera(authoring_u32(
                            &mut ctx.services()?,
                            index,
                            "gltf-metadata-camera",
                        )?),
                        &source_name_key,
                        ExtensionValueRef::String(name.as_str()),
                    );
                    metadata.try_push(&mut ctx.services()?, value)
                }?;
            }
        }
        for (owner, names) in [
            (
                ProfileOwner::Light as fn(u32) -> ProfileOwner,
                lights.names.as_slice(),
            ),
            (
                ProfileOwner::Image as fn(u32) -> ProfileOwner,
                appearance_storage.image_names.as_slice(),
            ),
            (
                ProfileOwner::Sampler as fn(u32) -> ProfileOwner,
                appearance_storage.sampler_names.as_slice(),
            ),
            (
                ProfileOwner::Texture as fn(u32) -> ProfileOwner,
                appearance_storage.texture_names.as_slice(),
            ),
        ] {
            for (index, name) in names.iter().enumerate() {
                let name = ctx.child(&work_group, [1], |child| {
                    child.run([1], || Ok::<_, baozi_import::ParseFailure>(name.as_ref()))
                })?;
                if let Some(name) = name {
                    {
                        let value = MetadataAuthoringEntry::new(
                            owner(authoring_u32(
                                &mut ctx.services()?,
                                index,
                                "gltf-metadata-source-name",
                            )?),
                            &source_name_key,
                            ExtensionValueRef::String(name.as_str()),
                        );
                        metadata.try_push(&mut ctx.services()?, value)
                    }?;
                }
            }
        }

        Ok(metadata)
    })?;

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
    let animations = with_content(context, |ctx| {
        animation_storage.authoring_tables(&mut ctx.services()?)
    })?;
    context.submit_complete(SceneAuthoringBatch::new(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations.batch(),
        profile,
    ))?;
    Ok(())
}

fn owner_sources_for<'a>(
    ctx: &mut ParseRoot<'_, '_, '_>,
    owners: &MeteredVec<'_, OwnerSource>,
    values: &'a MeteredVec<'_, ProvenanceLocalId>,
    target: ProfileOwner,
) -> Result<&'a [ProvenanceLocalId]> {
    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for owner in owners.as_slice() {
        let matches = ctx.child(&work_group, [1], |child| {
            child.run([1], || {
                Ok::<_, baozi_import::ParseFailure>(owner.owner == target)
            })
        })?;
        if matches {
            let end = owner
                .provenance_offset
                .checked_add(owner.provenance_count)
                .ok_or_else(|| ctx.allocation_failure("gltf-profile-generated-parents"))?;
            return values
                .as_slice()
                .get(owner.provenance_offset..end)
                .ok_or_else(|| {
                    gltf_parse_error("glTF generated owner has incomplete source parents")
                });
        }
    }
    Err(gltf_parse_error(
        "glTF generated owner has no source parents",
    ))
}

// Each versioned instruction identifies the normalization rule and its immutable inputs.
// Its source parents bind the generated owner to actual declarations without inventing a locator.
fn generated_evidence(
    ctx: &mut ParseRoot<'_, '_, '_>,
    code: &'static str,
    instruction: &[u8],
) -> Result<(OptionsDigest, [OperationEvidenceSummary; 1])> {
    let amount = to_u64(code.len()) + to_u64(instruction.len());
    let layout = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    let bytes: [u8; 32] = ctx.child(&layout, [amount], |child| {
        child.run([amount], || {
            let mut digest = Sha256::new();
            digest.update(code.as_bytes());
            digest.update(instruction);
            Ok::<_, baozi_import::ParseFailure>(digest.finalize().into())
        })
    })?;
    let code = NamespacedName::new(code)
        .map_err(|_| gltf_parse_error("static glTF generated evidence code is invalid"))?;
    let evidence = OperationEvidenceSummary::new(code, 1, Some(EvidenceDigest::sha256(bytes)))
        .map_err(|_| gltf_parse_error("static glTF generated evidence count is invalid"))?;
    Ok((OptionsDigest::sha256(bytes), [evidence]))
}

fn non_primary_provenance_count<'a>(
    ctx: &mut ParseRoot<'_, '_, '_>,
    provenance: impl IntoIterator<Item = &'a [u32]>,
) -> Result<usize> {
    provenance
        .into_iter()
        .try_fold(0usize, |mut count, sources| {
            let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
            let length = ctx.child(&work_group, [1], |child| {
                child.run([1], || Ok::<_, baozi_import::ParseFailure>(sources.len()))
            })?;
            ctx.child(&work_group, [to_u64(length)], |child| -> Result<()> {
                let errors = child.error_view();
                for (index, &source) in sources.iter().enumerate() {
                    poll_parser_loop(&mut child.services()?, index)?;
                    child.run([1], || -> Result<()> {
                        count = count.checked_add(usize::from(source != 0)).ok_or_else(|| {
                            errors.allocation_failure("gltf-profile-owner-source-count")
                        })?;
                        Ok(())
                    })?;
                }
                Ok(())
            })?;
            Ok(count)
        })
}

fn push_owner_source<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
    owners: &mut MeteredVec<'session, OwnerSource>,
    values: &mut MeteredVec<'session, ProvenanceLocalId>,
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

fn push_owner_sources<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
    owners: &mut MeteredVec<'session, OwnerSource>,
    values: &mut MeteredVec<'session, ProvenanceLocalId>,
    selected_sources: &SelectedSourceProfile<'_>,
    resource_count: usize,
    owner: ProfileOwner,
    provenance: impl IntoIterator<Item = u32>,
) -> Result<()> {
    let offset = values.len();
    values.try_push(&mut ctx.services()?, ProvenanceLocalId::new(0))?;
    let mut previous = 0;
    let work_group = ctx.bind_group([ImportWorkClass::WorkUnits])?;
    for source in provenance {
        let source = ctx.child(&work_group, [1], |child| {
            child.run([1], || Ok::<_, baozi_import::ParseFailure>(source))
        })?;
        if source == 0 {
            continue;
        }
        if source <= previous {
            return Err(ContentError::from(GltfFailure::internal(
                "owner provenance sources are not strictly ordered",
            )));
        }
        values.try_push(&mut ctx.services()?, ProvenanceLocalId::new(source))?;
        previous = source;
    }
    for &ordinal in selected_sources.for_owner(ctx, owner)? {
        let source = selected_sources.ordinal_source_index(ctx, resource_count, ordinal)?;
        if source == 0 {
            continue;
        }
        if source <= previous {
            return Err(GltfFailure::internal(
                "selected glTF owner source identities are not ordered",
            )
            .into());
        }
        values.try_push(&mut ctx.services()?, ProvenanceLocalId::new(source))?;
        previous = source;
    }
    owners.try_push(
        &mut ctx.services()?,
        OwnerSource {
            owner,
            provenance_offset: offset,
            provenance_count: values.len() - offset,
        },
    )?;
    Ok(())
}

fn push_indexed_owner_sources<'session>(
    ctx: &mut ParseRoot<'_, '_, 'session>,
    owners: &mut MeteredVec<'session, OwnerSource>,
    values: &mut MeteredVec<'session, ProvenanceLocalId>,
    selected_sources: &SelectedSourceProfile<'_>,
    resource_count: usize,
    count: usize,
    owner: impl Fn(u32) -> ProfileOwner,
) -> Result<()> {
    for index in 0..count {
        {
            let argument_0 = owner(authoring_u32(
                &mut ctx.services()?,
                index,
                "gltf-profile-owner-index",
            )?);
            push_owner_source(
                ctx,
                owners,
                values,
                selected_sources,
                resource_count,
                argument_0,
                0,
            )
        }?;
    }
    Ok(())
}
