use crate::detect::exact_binary_facets;
use crate::error::{StlFailure, StlFailureCode};
use crate::{FORMAT_ID, ascii, binary};
use baozi_import::authoring::{
    AlphaMode, AnimationAuthoringBatch, AppearanceAuthoringBatch, AttributeDomain,
    AttributeSemantic, AuthoredTransform, AuthoringRange, Color, DeformationAuthoringBatch,
    FormatId, GeometryAttributeAuthoring, GeometryAuthoringBatch, GeometryLogicalMeshAuthoring,
    GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    LineageAuthoringBatch, LogicalMeshLocalId, LogicalMorphAuthoring, MaterialAuthoring,
    MaterialFactors, MaterialLocalId, MeshInstanceDeformationAuthoring, MetadataAuthoringBatch,
    NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, ObservedResourceAuthoring,
    OwnerProvenanceAuthoring, PbrMetallicRoughnessFactors, Presence, PrimitiveDeformationAuthoring,
    ProfileAuthoringBatch, ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring,
    ResourceIdentity, ResourceIdentityLocalId, SceneAuthoringBatch, SceneSpace,
    SessionOpaqueReason, ShadingModel, SourceIdentityAuthoring, SourceIdentityLocalId,
    SourceObjectKind, SourceObjectLocator, SpatialAuthoringBatch,
};
use baozi_import::{
    ImportStorageClass, ImportWorkClass, MeteredString, MeteredVec, ParseContext, ParseExecution,
    ParseFailure, ParseResult,
};

const MATERIAL_NAME: &str = "STL Default Material";
const FALLBACK_RESOURCE_IDENTITY: &str = "session/stl-primary";
const SOURCE_PROVENANCE: [ProvenanceLocalId; 1] = [ProvenanceLocalId::new(0)];

pub(crate) struct ParsedStl<'session> {
    pub meshes: MeteredVec<'session, ParsedMesh<'session>>,
    // The active object fallback, carried by either the material or vertex colors.
    pub material_color: Option<Color>,
}

pub(crate) struct ParsedMesh<'session> {
    pub name: Option<MeteredString<'session>>,
    // Fixed arrays keep parser storage metered while allowing a safe flattened scalar view.
    pub positions: MeteredVec<'session, [f32; 3]>,
    pub normals: MeteredVec<'session, [f32; 3]>,
    pub colors: Option<MeteredVec<'session, [f32; 4]>>,
}

#[derive(Clone, Copy)]
struct IndexRange {
    offset: usize,
    count: usize,
}

#[derive(Clone, Copy)]
struct PrimitivePatch {
    indices: IndexRange,
    attributes: AuthoringRange,
}

pub(crate) fn read_stl(context: &mut ParseContext<'_>) -> ParseResult<()> {
    let bytes = context.primary_bytes();
    let parsed = context.with_execution(|exec| match exact_binary_facets(bytes) {
        Some(facets) => binary::parse(exec, bytes, facets),
        None => ascii::parse(exec),
    })?;
    scene_from_parsed(context, parsed)
}

struct SceneTables<'session, 'input> {
    materials: MeteredVec<'session, MaterialAuthoring<'static>>,
    indices: MeteredVec<'session, u32>,
    roots: MeteredVec<'session, NodeLocalId>,
    nodes: MeteredVec<'session, NodeAuthoringNode<'input>>,
    logical_meshes: MeteredVec<'session, GeometryLogicalMeshAuthoring<'input>>,
    primitives: MeteredVec<'session, GeometryPrimitiveAuthoring<'static>>,
    primitive_patches: MeteredVec<'session, PrimitivePatch>,
    attributes: MeteredVec<'session, GeometryAttributeAuthoring<'input>>,
    mesh_instances: MeteredVec<'session, GeometryMeshInstanceAuthoring>,
    logical_morphs: MeteredVec<'session, LogicalMorphAuthoring<'static>>,
    primitive_deformations: MeteredVec<'session, PrimitiveDeformationAuthoring<'static>>,
    mesh_instance_deformations: MeteredVec<'session, MeshInstanceDeformationAuthoring<'static>>,
    owner_provenance: MeteredVec<'session, OwnerProvenanceAuthoring<'static>>,
    resource_identity: ResourceIdentity,
    resource_stability: SessionOpaqueReason,
    format: FormatId,
}

fn authoring_tables<'session, 'input>(
    exec: &mut ParseExecution<'_, 'session>,
    parsed: &'input ParsedStl<'session>,
) -> ParseResult<SceneTables<'session, 'input>> {
    if parsed.meshes.is_empty() {
        return Err(exec.format_failure(StlFailure::content(
            StlFailureCode::EmptyGeometry,
            "document",
            "no-non-empty-solids-or-facets",
        )));
    }

    let mut remaining_topology_cells = 0usize;
    let mut has_vertex_colors = false;
    for (index, mesh) in parsed.meshes.as_slice().iter().enumerate() {
        if index % 4096 == 0 {
            exec.poll()?;
        }
        validate_mesh_shape(exec, mesh)?;
        has_vertex_colors |= mesh.colors.is_some();
        remaining_topology_cells = remaining_topology_cells
            .checked_add(mesh.positions.len())
            .ok_or_else(|| exec.allocation_failure("stl-topology-cell-counter"))?;
    }
    let mut remaining_canonical_string_bytes = canonical_string_bytes(exec, parsed)?;

    let shading_model = ShadingModel::Unspecified;
    let alpha_mode = if parsed.material_color.is_some_and(|color| color.a < 1.0) {
        AlphaMode::Blend
    } else {
        AlphaMode::Opaque
    };
    // Facet colors already contain the object fallback, including its alpha.
    let base_color = if has_vertex_colors {
        Color::WHITE
    } else {
        parsed.material_color.unwrap_or(Color::WHITE)
    };
    let pbr_factors = PbrMetallicRoughnessFactors::try_new(base_color, 0.0, 1.0)
        .map_err(|_| canonical_contract_failure(exec, "stl-default-material-factors"))?;
    let material_factors = MaterialFactors::empty().with_pbr_metallic_roughness(Some(pbr_factors));
    let material = MaterialAuthoring::try_from_parts(
        AuthoringRange::new(0, 0),
        shading_model,
        alpha_mode,
        false,
        material_factors,
    )
    .map_err(|_| canonical_contract_failure(exec, "stl-default-material"))?
    .with_name(MATERIAL_NAME);
    let mut materials = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-materials",
    );
    remaining_canonical_string_bytes = remaining_canonical_string_bytes
        .checked_sub(MATERIAL_NAME.len())
        .ok_or_else(|| exec.allocation_failure("stl-canonical-string-counter"))?;
    let material_name_work = exec.bind_work_group([ImportWorkClass::CanonicalStringBytes])?;
    exec.with_work(
        &material_name_work,
        [usize_amount(
            exec,
            MATERIAL_NAME.len(),
            "stl-material-name",
        )?],
        |exec, work| work.run_all(|| materials.try_push(exec, material)),
    )?;

    let mut indices = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-indices",
    );
    let mut roots = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-roots",
    );
    let mut nodes = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-nodes",
    );
    let mut logical_meshes = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-logical-meshes",
    );
    let mut primitives = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-primitives",
    );
    let mut primitive_patches = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-primitive-patches",
    );
    let mut attributes = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-attributes",
    );
    let mut mesh_instances = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-mesh-instances",
    );
    let mut logical_morphs = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-logical-morphs",
    );
    let mut primitive_deformations = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-primitive-deformations",
    );
    let mut mesh_instance_deformations = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-mesh-instance-deformations",
    );

    let mesh_work = exec.bind_work_group([
        ImportWorkClass::TopologyCells,
        ImportWorkClass::CanonicalStringBytes,
        ImportWorkClass::Primitives,
        ImportWorkClass::AttributeStreams,
        ImportWorkClass::GraphNodes,
        ImportWorkClass::GraphEdges,
    ])?;
    for (mesh_index, parsed_mesh) in parsed.meshes.as_slice().iter().enumerate() {
        let topology_cells = parsed_mesh.positions.len();
        remaining_topology_cells = remaining_topology_cells
            .checked_sub(topology_cells)
            .ok_or_else(|| exec.allocation_failure("stl-topology-cell-counter"))?;
        let mesh_name_bytes = parsed_mesh.name.as_ref().map_or(0, MeteredString::len);
        let canonical_name_bytes = mesh_name_bytes
            .checked_mul(2)
            .ok_or_else(|| exec.allocation_failure("stl-string-byte-counter"))?;
        remaining_canonical_string_bytes = remaining_canonical_string_bytes
            .checked_sub(canonical_name_bytes)
            .ok_or_else(|| exec.allocation_failure("stl-canonical-string-counter"))?;
        let topology_cells = usize_amount(exec, topology_cells, "stl-topology-cells")?;
        let mesh_name_bytes = usize_amount(exec, mesh_name_bytes, "stl-mesh-name")?;
        let canonical_name_bytes =
            usize_amount(exec, canonical_name_bytes, "stl-canonical-string-bytes")?;
        let attribute_stream_count = 2 + u64::from(parsed_mesh.colors.is_some());
        let local_id = u32::try_from(mesh_index)
            .map_err(|_| exec.allocation_failure("stl-authoring-local-id"))?;

        exec.with_work(
            &mesh_work,
            [
                topology_cells,
                canonical_name_bytes,
                1,
                attribute_stream_count,
                1,
                1,
            ],
            |exec, work| -> ParseResult<()> {
                let index_range = work.run([topology_cells, 0, 0, 0, 0, 0], || {
                    append_sequential_indices(exec, &mut indices, parsed_mesh.positions.len())
                })?;

                work.run([0, mesh_name_bytes, 0, 0, 0, 0], || Ok(()))?;

                let attribute_offset = u32::try_from(attributes.len())
                    .map_err(|_| exec.allocation_failure("stl-authoring-attribute-offset"))?;
                let attribute_count = u32::try_from(attribute_stream_count)
                    .map_err(|_| exec.allocation_failure("stl-authoring-attribute-count"))?;
                let attribute_range = AuthoringRange::new(attribute_offset, attribute_count);
                work.run(
                    [0, mesh_name_bytes, 1, attribute_stream_count, 0, 0],
                    || -> ParseResult<()> {
                        attributes.try_push(
                            exec,
                            GeometryAttributeAuthoring::f32(
                                AttributeSemantic::Position,
                                AttributeDomain::Vertex,
                                3,
                                parsed_mesh.positions.as_slice().as_flattened(),
                                Presence::Absent,
                            ),
                        )?;
                        attributes.try_push(
                            exec,
                            GeometryAttributeAuthoring::f32(
                                AttributeSemantic::Normal,
                                AttributeDomain::Vertex,
                                3,
                                parsed_mesh.normals.as_slice().as_flattened(),
                                Presence::Absent,
                            ),
                        )?;
                        if let Some(colors) = &parsed_mesh.colors {
                            attributes.try_push(
                                exec,
                                GeometryAttributeAuthoring::f32(
                                    AttributeSemantic::Color(0),
                                    AttributeDomain::Vertex,
                                    4,
                                    colors.as_slice().as_flattened(),
                                    Presence::Absent,
                                ),
                            )?;
                        }

                        let primitive_offset = u32::try_from(primitives.len()).map_err(|_| {
                            exec.allocation_failure("stl-authoring-primitive-offset")
                        })?;
                        let mut logical_mesh = GeometryLogicalMeshAuthoring::new(
                            AuthoringRange::new(primitive_offset, 1),
                        );
                        if let Some(name) = &parsed_mesh.name {
                            logical_mesh = logical_mesh.with_name(name.as_str());
                        }
                        logical_meshes.try_push(exec, logical_mesh)?;
                        primitives.try_push(
                            exec,
                            GeometryPrimitiveAuthoring::new(
                                GeometryTopologyAuthoring::TriangleList,
                                Presence::Absent,
                                baozi_import::authoring::MaterialAssignment::Authored(
                                    MaterialLocalId::new(0),
                                ),
                                attribute_range,
                            ),
                        )?;
                        primitive_patches.try_push(
                            exec,
                            PrimitivePatch {
                                indices: index_range,
                                attributes: attribute_range,
                            },
                        )?;
                        logical_morphs.try_push(
                            exec,
                            LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[]),
                        )?;
                        primitive_deformations.try_push(
                            exec,
                            PrimitiveDeformationAuthoring::new(Presence::Absent, &[]),
                        )
                    },
                )?;

                work.run([0, 0, 0, 0, 1, 0], || -> ParseResult<()> {
                    let node_id = NodeLocalId::new(local_id);
                    let mut node = NodeAuthoringNode::new(
                        Presence::Absent,
                        AuthoredTransform::identity_trs(),
                        Presence::Absent,
                        Presence::Absent,
                    );
                    if let Some(name) = &parsed_mesh.name {
                        node = node.with_name(name.as_str());
                    }
                    roots.try_push(exec, node_id)?;
                    nodes.try_push(exec, node)
                })?;

                work.run([0, 0, 0, 0, 0, 1], || -> ParseResult<()> {
                    mesh_instances.try_push(
                        exec,
                        GeometryMeshInstanceAuthoring::new(
                            NodeLocalId::new(local_id),
                            LogicalMeshLocalId::new(local_id),
                        ),
                    )?;
                    mesh_instance_deformations.try_push(
                        exec,
                        MeshInstanceDeformationAuthoring::new(Presence::Absent, Presence::Absent),
                    )
                })
            },
        )?;
    }

    debug_assert_eq!(remaining_topology_cells, 0);
    debug_assert_eq!(remaining_canonical_string_bytes, 0);

    if primitives.len() != primitive_patches.len() {
        return Err(canonical_contract_failure(
            exec,
            "stl-authoring-primitive-patch-count",
        ));
    }
    let (resource_identity, resource_stability) = profile_resource_identity(exec)?;
    let format =
        FormatId::new(FORMAT_ID).map_err(|_| exec.allocation_failure("stl-profile-format-id"))?;
    let mut owner_provenance = exec.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-owner-provenance",
    );
    push_owner(exec, &mut owner_provenance, ProfileOwner::Scene)?;
    push_owner(exec, &mut owner_provenance, ProfileOwner::SyntheticRoot)?;
    for index in 0..nodes.len() {
        let owner = ProfileOwner::Node(NodeLocalId::new(profile_index(exec, index)?));
        push_owner(exec, &mut owner_provenance, owner)?;
    }
    push_owner(exec, &mut owner_provenance, ProfileOwner::Material(0))?;
    for index in 0..logical_meshes.len() {
        let owner = ProfileOwner::LogicalMesh(profile_index(exec, index)?);
        push_owner(exec, &mut owner_provenance, owner)?;
    }
    for index in 0..primitives.len() {
        let owner = ProfileOwner::Primitive(profile_index(exec, index)?);
        push_owner(exec, &mut owner_provenance, owner)?;
    }
    for index in 0..attributes.len() {
        let owner = ProfileOwner::Attribute(profile_index(exec, index)?);
        push_owner(exec, &mut owner_provenance, owner)?;
    }
    for index in 0..mesh_instances.len() {
        let owner = ProfileOwner::MeshInstance(profile_index(exec, index)?);
        push_owner(exec, &mut owner_provenance, owner)?;
    }

    Ok(SceneTables {
        materials,
        indices,
        roots,
        nodes,
        logical_meshes,
        primitives,
        primitive_patches,
        attributes,
        mesh_instances,
        logical_morphs,
        primitive_deformations,
        mesh_instance_deformations,
        owner_provenance,
        resource_identity,
        resource_stability,
        format,
    })
}

fn scene_from_parsed<'session>(
    context: &mut ParseContext<'session>,
    parsed: ParsedStl<'session>,
) -> ParseResult<()> {
    let SceneTables {
        materials,
        indices,
        roots,
        nodes,
        logical_meshes,
        primitives,
        primitive_patches,
        attributes,
        mesh_instances,
        logical_morphs,
        primitive_deformations,
        mesh_instance_deformations,
        owner_provenance,
        resource_identity,
        resource_stability,
        format,
    } = context.with_execution(|exec| authoring_tables(exec, &parsed))?;

    // Move the buffers apart before descriptors borrow the sealed index storage.
    let mut primitives: MeteredVec<'_, GeometryPrimitiveAuthoring<'_>> = primitives;
    context.with_execution(|exec| -> ParseResult<()> {
        for (primitive, patch) in primitives
            .as_mut_slice()
            .iter_mut()
            .zip(primitive_patches.as_slice())
        {
            let end = patch
                .indices
                .offset
                .checked_add(patch.indices.count)
                .ok_or_else(|| exec.allocation_failure("stl-authoring-index-range"))?;
            let values = indices
                .as_slice()
                .get(patch.indices.offset..end)
                .ok_or_else(|| canonical_contract_failure(exec, "stl-authoring-index-range"))?;
            *primitive = GeometryPrimitiveAuthoring::new(
                GeometryTopologyAuthoring::TriangleList,
                Presence::Present(values),
                baozi_import::authoring::MaterialAssignment::Authored(MaterialLocalId::new(0)),
                patch.attributes,
            );
        }
        Ok(())
    })?;

    let appearance = AppearanceAuthoringBatch::new(&[], &[], &[], &[], materials.as_slice());
    let geometry = GeometryAuthoringBatch::new(
        logical_meshes.as_slice(),
        primitives.as_slice(),
        attributes.as_slice(),
        mesh_instances.as_slice(),
    );
    let deformation = DeformationAuthoringBatch::new(
        &[],
        &[],
        logical_morphs.as_slice(),
        primitive_deformations.as_slice(),
        mesh_instance_deformations.as_slice(),
    );
    let node_batch = NodeAuthoringBatch::new(Presence::Present(roots.as_slice()), nodes.as_slice());
    let scene_space = SceneSpace::UNKNOWN;
    let spatial = SpatialAuthoringBatch::new(&scene_space, &[], &[]);
    let animations = AnimationAuthoringBatch::empty();
    let locator = SourceObjectLocator::indexed(SourceObjectKind::Document, 0);
    let resources = [ObservedResourceAuthoring::session_opaque(
        &resource_identity,
        resource_stability,
    )];
    let source_identities = [SourceIdentityAuthoring::new(
        ResourceIdentityLocalId::new(0),
        &format,
        &locator,
    )];
    let provenance_nodes = [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
        0,
    ))];
    let profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(
            &resources,
            &source_identities,
            &provenance_nodes,
            owner_provenance.as_slice(),
        ),
        MetadataAuthoringBatch::empty(),
    );

    context.submit_complete(SceneAuthoringBatch::new(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
        profile,
    ))
}

fn validate_mesh_shape(
    exec: &mut ParseExecution<'_, '_>,
    mesh: &ParsedMesh<'_>,
) -> ParseResult<()> {
    if mesh.positions.len() != mesh.normals.len()
        || mesh
            .colors
            .as_ref()
            .is_some_and(|colors| colors.len() != mesh.positions.len())
    {
        return Err(canonical_contract_failure(
            exec,
            "stl-attribute-cardinality",
        ));
    }
    Ok(())
}

fn append_sequential_indices<'session>(
    exec: &mut ParseExecution<'_, 'session>,
    indices: &mut MeteredVec<'session, u32>,
    vertex_count: usize,
) -> ParseResult<IndexRange> {
    let offset = indices.len();
    indices.try_reserve(exec, vertex_count)?;
    let mut start = 0usize;
    while start < vertex_count {
        exec.poll()?;
        let end = start.saturating_add(4096).min(vertex_count);
        for index in start..end {
            indices.try_push_reserved(
                u32::try_from(index).map_err(|_| exec.allocation_failure("stl-index-space"))?,
            )?;
        }
        start = end;
    }
    Ok(IndexRange {
        offset,
        count: vertex_count,
    })
}

fn canonical_string_bytes(
    exec: &mut ParseExecution<'_, '_>,
    parsed: &ParsedStl<'_>,
) -> ParseResult<usize> {
    let mut bytes = MATERIAL_NAME.len();
    for (index, mesh) in parsed.meshes.as_slice().iter().enumerate() {
        if index % 4096 == 0 {
            exec.poll()?;
        }
        if let Some(name) = &mesh.name {
            checked_add(exec, &mut bytes, name.len(), "stl-string-byte-counter")?;
            checked_add(exec, &mut bytes, name.len(), "stl-string-byte-counter")?;
        }
    }
    Ok(bytes)
}

fn checked_add(
    exec: &mut ParseExecution<'_, '_>,
    total: &mut usize,
    value: usize,
    resource: &'static str,
) -> ParseResult<()> {
    *total = total
        .checked_add(value)
        .ok_or_else(|| exec.allocation_failure(resource))?;
    Ok(())
}

fn push_owner<'session, 'input>(
    exec: &mut ParseExecution<'_, 'session>,
    owners: &mut MeteredVec<'session, OwnerProvenanceAuthoring<'input>>,
    owner: ProfileOwner,
) -> ParseResult<()> {
    owners.try_push(
        exec,
        OwnerProvenanceAuthoring::new(owner, &SOURCE_PROVENANCE),
    )
}

fn profile_resource_identity(
    exec: &mut ParseExecution<'_, '_>,
) -> ParseResult<(ResourceIdentity, SessionOpaqueReason)> {
    match ResourceIdentity::new(exec.source().as_str()) {
        Ok(identity) => Ok((identity, SessionOpaqueReason::CallerLabel)),
        Err(_) => ResourceIdentity::new(FALLBACK_RESOURCE_IDENTITY)
            .map(|identity| (identity, SessionOpaqueReason::SessionFallback))
            .map_err(|_| exec.allocation_failure("stl-profile-resource-identity")),
    }
}

fn profile_index(exec: &ParseExecution<'_, '_>, index: usize) -> Result<u32, ParseFailure> {
    u32::try_from(index).map_err(|_| exec.allocation_failure("stl-profile-owner-index"))
}

fn usize_amount(
    exec: &ParseExecution<'_, '_>,
    value: usize,
    resource: &'static str,
) -> Result<u64, ParseFailure> {
    u64::try_from(value).map_err(|_| exec.allocation_failure(resource))
}

fn canonical_contract_failure(
    exec: &mut ParseExecution<'_, '_>,
    subject: &'static str,
) -> ParseFailure {
    exec.format_failure(StlFailure::content(
        StlFailureCode::InternalInvariant,
        subject,
        "canonical-authoring-contract",
    ))
}
