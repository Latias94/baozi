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
    SourceObjectKind, SourceObjectLocator, SpatialAuthoringBatch, Vec3,
};
use baozi_import::{
    ImportStorageClass, ImportWorkClass, MeteredString, MeteredVec, ParseFailure, ParseResult,
    ParseScope, SceneDraft,
};

const MATERIAL_NAME: &str = "STL Default Material";
const FALLBACK_RESOURCE_IDENTITY: &str = "session/stl-primary";
const SOURCE_PROVENANCE: [ProvenanceLocalId; 1] = [ProvenanceLocalId::new(0)];

pub(crate) struct ParsedStl<'scope> {
    pub meshes: MeteredVec<'scope, ParsedMesh<'scope>>,
    pub material_color: Option<Color>,
}

pub(crate) struct ParsedMesh<'scope> {
    pub name: Option<MeteredString<'scope>>,
    // Fixed arrays keep parser storage metered while allowing a safe flattened scalar view.
    pub positions: MeteredVec<'scope, [f32; 3]>,
    pub normals: MeteredVec<'scope, [f32; 3]>,
    pub colors: Option<MeteredVec<'scope, [f32; 4]>>,
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

pub(crate) fn read_stl(scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
    let bytes = scope.primary_bytes();
    let parsed = match exact_binary_facets(bytes) {
        Some(facets) => binary::parse(scope, bytes, facets)?,
        None => ascii::parse(scope)?,
    };
    scene_from_parsed(scope, parsed)
}

fn scene_from_parsed<'scope>(
    scope: &'scope ParseScope<'_>,
    parsed: ParsedStl<'scope>,
) -> ParseResult<SceneDraft> {
    if parsed.meshes.is_empty() {
        return Err(scope.format_failure(StlFailure::content(
            StlFailureCode::EmptyGeometry,
            "document",
            "no-non-empty-solids-or-facets",
        )));
    }

    let mut remaining_topology_cells = 0usize;
    for (index, mesh) in parsed.meshes.as_slice().iter().enumerate() {
        if index % 4096 == 0 {
            scope.poll()?;
        }
        validate_mesh_shape(scope, mesh)?;
        remaining_topology_cells = remaining_topology_cells
            .checked_add(mesh.positions.len())
            .ok_or_else(|| scope.allocation_failure("stl-topology-cell-counter"))?;
    }
    let mut remaining_canonical_string_bytes = canonical_string_bytes(scope, &parsed)?;

    let shading_model = ShadingModel::Unspecified;
    let alpha_mode = AlphaMode::Opaque;
    let base_color = parsed.material_color.unwrap_or(Color::WHITE);
    let pbr_factors = PbrMetallicRoughnessFactors::try_new(base_color, 0.0, 1.0)
        .map_err(|_| canonical_contract_failure(scope, "stl-default-material-factors"))?;
    let material_factors = MaterialFactors::try_new(Vec3::ZERO, Some(pbr_factors), None, None)
        .map_err(|_| canonical_contract_failure(scope, "stl-default-material-factors"))?;
    let material = MaterialAuthoring::try_from_parts(
        AuthoringRange::new(0, 0),
        &shading_model,
        &alpha_mode,
        false,
        material_factors,
    )
    .map_err(|_| canonical_contract_failure(scope, "stl-default-material"))?
    .with_name(MATERIAL_NAME);
    let mut materials = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-materials",
    );
    remaining_canonical_string_bytes = remaining_canonical_string_bytes
        .checked_sub(MATERIAL_NAME.len())
        .ok_or_else(|| scope.allocation_failure("stl-canonical-string-counter"))?;
    scope.with_work(
        ImportWorkClass::CanonicalStringBytes,
        usize_amount(scope, MATERIAL_NAME.len(), "stl-material-name")?,
        |work| work.run_all(|| materials.try_push(material)),
    )?;

    let mut indices = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-indices",
    );
    let mut roots = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-roots",
    );
    let mut nodes = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-nodes",
    );
    let mut logical_meshes = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-logical-meshes",
    );
    let mut primitives = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-primitives",
    );
    let mut primitive_patches = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-primitive-patches",
    );
    let mut attributes = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-attributes",
    );
    let mut mesh_instances = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-mesh-instances",
    );
    let mut logical_morphs = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-logical-morphs",
    );
    let mut primitive_deformations = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-primitive-deformations",
    );
    let mut mesh_instance_deformations = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-mesh-instance-deformations",
    );

    for (mesh_index, parsed_mesh) in parsed.meshes.as_slice().iter().enumerate() {
        let topology_cells = parsed_mesh.positions.len();
        remaining_topology_cells = remaining_topology_cells
            .checked_sub(topology_cells)
            .ok_or_else(|| scope.allocation_failure("stl-topology-cell-counter"))?;
        let mesh_name_bytes = parsed_mesh.name.as_ref().map_or(0, MeteredString::len);
        let canonical_name_bytes = mesh_name_bytes
            .checked_mul(2)
            .ok_or_else(|| scope.allocation_failure("stl-string-byte-counter"))?;
        remaining_canonical_string_bytes = remaining_canonical_string_bytes
            .checked_sub(canonical_name_bytes)
            .ok_or_else(|| scope.allocation_failure("stl-canonical-string-counter"))?;
        let topology_cells = usize_amount(scope, topology_cells, "stl-topology-cells")?;
        let mesh_name_bytes = usize_amount(scope, mesh_name_bytes, "stl-mesh-name")?;
        let canonical_name_bytes =
            usize_amount(scope, canonical_name_bytes, "stl-canonical-string-bytes")?;
        let attribute_stream_count = 2 + u64::from(parsed_mesh.colors.is_some());
        let local_id = u32::try_from(mesh_index)
            .map_err(|_| scope.allocation_failure("stl-authoring-local-id"))?;

        scope.with_work_group(
            [
                (ImportWorkClass::TopologyCells, topology_cells),
                (ImportWorkClass::CanonicalStringBytes, canonical_name_bytes),
                (ImportWorkClass::Primitives, 1),
                (ImportWorkClass::AttributeStreams, attribute_stream_count),
                (ImportWorkClass::GraphNodes, 1),
                (ImportWorkClass::GraphEdges, 1),
            ],
            |work| -> ParseResult<()> {
                let index_range = work.run([topology_cells, 0, 0, 0, 0, 0], || {
                    append_sequential_indices(scope, &mut indices, parsed_mesh.positions.len())
                })?;

                work.run([0, mesh_name_bytes, 0, 0, 0, 0], || Ok(()))?;

                let attribute_offset = u32::try_from(attributes.len())
                    .map_err(|_| scope.allocation_failure("stl-authoring-attribute-offset"))?;
                let attribute_count = u32::try_from(attribute_stream_count)
                    .map_err(|_| scope.allocation_failure("stl-authoring-attribute-count"))?;
                let attribute_range = AuthoringRange::new(attribute_offset, attribute_count);
                work.run(
                    [0, mesh_name_bytes, 1, attribute_stream_count, 0, 0],
                    || -> ParseResult<()> {
                        attributes.try_push(GeometryAttributeAuthoring::f32(
                            AttributeSemantic::Position,
                            AttributeDomain::Vertex,
                            3,
                            parsed_mesh.positions.as_slice().as_flattened(),
                            Presence::Absent,
                        ))?;
                        attributes.try_push(GeometryAttributeAuthoring::f32(
                            AttributeSemantic::Normal,
                            AttributeDomain::Vertex,
                            3,
                            parsed_mesh.normals.as_slice().as_flattened(),
                            Presence::Absent,
                        ))?;
                        if let Some(colors) = &parsed_mesh.colors {
                            attributes.try_push(GeometryAttributeAuthoring::f32(
                                AttributeSemantic::Color(0),
                                AttributeDomain::Vertex,
                                4,
                                colors.as_slice().as_flattened(),
                                Presence::Absent,
                            ))?;
                        }

                        let primitive_offset = u32::try_from(primitives.len()).map_err(|_| {
                            scope.allocation_failure("stl-authoring-primitive-offset")
                        })?;
                        let mut logical_mesh = GeometryLogicalMeshAuthoring::new(
                            AuthoringRange::new(primitive_offset, 1),
                        );
                        if let Some(name) = &parsed_mesh.name {
                            logical_mesh = logical_mesh.with_name(name.as_str());
                        }
                        logical_meshes.try_push(logical_mesh)?;
                        primitives.try_push(GeometryPrimitiveAuthoring::new(
                            GeometryTopologyAuthoring::TriangleList,
                            Presence::Absent,
                            Presence::Present(MaterialLocalId::new(0)),
                            attribute_range,
                        ))?;
                        primitive_patches.try_push(PrimitivePatch {
                            indices: index_range,
                            attributes: attribute_range,
                        })?;
                        logical_morphs
                            .try_push(LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[]))?;
                        primitive_deformations
                            .try_push(PrimitiveDeformationAuthoring::new(Presence::Absent, &[]))
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
                    roots.try_push(node_id)?;
                    nodes.try_push(node)
                })?;

                work.run([0, 0, 0, 0, 0, 1], || -> ParseResult<()> {
                    mesh_instances.try_push(GeometryMeshInstanceAuthoring::new(
                        NodeLocalId::new(local_id),
                        LogicalMeshLocalId::new(local_id),
                    ))?;
                    mesh_instance_deformations.try_push(MeshInstanceDeformationAuthoring::new(
                        Presence::Absent,
                        Presence::Absent,
                    ))
                })
            },
        )?;
    }

    debug_assert_eq!(remaining_topology_cells, 0);
    debug_assert_eq!(remaining_canonical_string_bytes, 0);

    if primitives.len() != primitive_patches.len() {
        return Err(canonical_contract_failure(
            scope,
            "stl-authoring-primitive-patch-count",
        ));
    }
    // Seal the flat index buffer before borrowing its ranges into primitive descriptors.
    // This avoids self-referential storage and keeps every dynamic allocation session-metered.
    for (primitive, patch) in primitives
        .as_mut_slice()
        .iter_mut()
        .zip(primitive_patches.as_slice())
    {
        let end = patch
            .indices
            .offset
            .checked_add(patch.indices.count)
            .ok_or_else(|| scope.allocation_failure("stl-authoring-index-range"))?;
        let values = indices
            .as_slice()
            .get(patch.indices.offset..end)
            .ok_or_else(|| canonical_contract_failure(scope, "stl-authoring-index-range"))?;
        *primitive = GeometryPrimitiveAuthoring::new(
            GeometryTopologyAuthoring::TriangleList,
            Presence::Present(values),
            Presence::Present(MaterialLocalId::new(0)),
            patch.attributes,
        );
    }

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

    let (resource_identity, resource_stability) = profile_resource_identity(scope)?;
    let format =
        FormatId::new(FORMAT_ID).map_err(|_| scope.allocation_failure("stl-profile-format-id"))?;
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
    let mut owner_provenance = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "stl-authoring-owner-provenance",
    );
    push_owner(&mut owner_provenance, ProfileOwner::Scene)?;
    push_owner(&mut owner_provenance, ProfileOwner::SyntheticRoot)?;
    for index in 0..nodes.len() {
        push_owner(
            &mut owner_provenance,
            ProfileOwner::Node(NodeLocalId::new(profile_index(scope, index)?)),
        )?;
    }
    push_owner(&mut owner_provenance, ProfileOwner::Material(0))?;
    for index in 0..logical_meshes.len() {
        push_owner(
            &mut owner_provenance,
            ProfileOwner::LogicalMesh(profile_index(scope, index)?),
        )?;
    }
    for index in 0..primitives.len() {
        push_owner(
            &mut owner_provenance,
            ProfileOwner::Primitive(profile_index(scope, index)?),
        )?;
    }
    for index in 0..attributes.len() {
        push_owner(
            &mut owner_provenance,
            ProfileOwner::Attribute(profile_index(scope, index)?),
        )?;
    }
    for index in 0..mesh_instances.len() {
        push_owner(
            &mut owner_provenance,
            ProfileOwner::MeshInstance(profile_index(scope, index)?),
        )?;
    }
    let profile = ProfileAuthoringBatch::new(
        LineageAuthoringBatch::new(
            &resources,
            &source_identities,
            &provenance_nodes,
            owner_provenance.as_slice(),
        ),
        MetadataAuthoringBatch::empty(),
    );

    scope.submit_scene(SceneAuthoringBatch::new(
        spatial,
        node_batch,
        appearance,
        geometry,
        deformation,
        animations,
        profile,
    ))
}

fn validate_mesh_shape(scope: &ParseScope<'_>, mesh: &ParsedMesh<'_>) -> ParseResult<()> {
    if mesh.positions.len() != mesh.normals.len()
        || mesh
            .colors
            .as_ref()
            .is_some_and(|colors| colors.len() != mesh.positions.len())
    {
        return Err(canonical_contract_failure(
            scope,
            "stl-attribute-cardinality",
        ));
    }
    Ok(())
}

fn append_sequential_indices(
    scope: &ParseScope<'_>,
    indices: &mut MeteredVec<'_, u32>,
    vertex_count: usize,
) -> ParseResult<IndexRange> {
    let offset = indices.len();
    indices.try_reserve(vertex_count)?;
    let mut start = 0usize;
    while start < vertex_count {
        scope.poll()?;
        let end = start.saturating_add(4096).min(vertex_count);
        for index in start..end {
            indices.try_push_reserved(
                u32::try_from(index).map_err(|_| scope.allocation_failure("stl-index-space"))?,
            )?;
        }
        start = end;
    }
    Ok(IndexRange {
        offset,
        count: vertex_count,
    })
}

fn canonical_string_bytes(scope: &ParseScope<'_>, parsed: &ParsedStl<'_>) -> ParseResult<usize> {
    let mut bytes = MATERIAL_NAME.len();
    for (index, mesh) in parsed.meshes.as_slice().iter().enumerate() {
        if index % 4096 == 0 {
            scope.poll()?;
        }
        if let Some(name) = &mesh.name {
            checked_add(scope, &mut bytes, name.len(), "stl-string-byte-counter")?;
            checked_add(scope, &mut bytes, name.len(), "stl-string-byte-counter")?;
        }
    }
    Ok(bytes)
}

fn checked_add(
    scope: &ParseScope<'_>,
    total: &mut usize,
    value: usize,
    resource: &'static str,
) -> ParseResult<()> {
    *total = total
        .checked_add(value)
        .ok_or_else(|| scope.allocation_failure(resource))?;
    Ok(())
}

fn push_owner<'input>(
    owners: &mut MeteredVec<'_, OwnerProvenanceAuthoring<'input>>,
    owner: ProfileOwner,
) -> ParseResult<()> {
    owners.try_push(OwnerProvenanceAuthoring::new(owner, &SOURCE_PROVENANCE))
}

fn profile_resource_identity(
    scope: &ParseScope<'_>,
) -> ParseResult<(ResourceIdentity, SessionOpaqueReason)> {
    match ResourceIdentity::new(scope.source().as_str()) {
        Ok(identity) => Ok((identity, SessionOpaqueReason::CallerLabel)),
        Err(_) => ResourceIdentity::new(FALLBACK_RESOURCE_IDENTITY)
            .map(|identity| (identity, SessionOpaqueReason::SessionFallback))
            .map_err(|_| scope.allocation_failure("stl-profile-resource-identity")),
    }
}

fn profile_index(scope: &ParseScope<'_>, index: usize) -> Result<u32, ParseFailure> {
    u32::try_from(index).map_err(|_| scope.allocation_failure("stl-profile-owner-index"))
}

fn usize_amount(
    scope: &ParseScope<'_>,
    value: usize,
    resource: &'static str,
) -> Result<u64, ParseFailure> {
    u64::try_from(value).map_err(|_| scope.allocation_failure(resource))
}

fn canonical_contract_failure(scope: &ParseScope<'_>, subject: &'static str) -> ParseFailure {
    scope.format_failure(StlFailure::content(
        StlFailureCode::InternalInvariant,
        subject,
        "canonical-authoring-contract",
    ))
}
