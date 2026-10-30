//! Borrowed static Scene input with session-owned document provenance.

use super::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AuthoringRange, CameraProjection,
    ContentDigest, DIGEST_LEN, DeformationAuthoringBatch, FormatId, GeometryAttributeAuthoring,
    GeometryAuthoringBatch, GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring,
    GeometryPrimitiveAuthoring, ImageAuthoring, LightDefinition, LineageAuthoringBatch,
    LogicalMorphAuthoring, MAX_RESOURCE_IDENTITY_LEN, MaterialAuthoring, MaterialBindingAuthoring,
    MeshInstanceDeformationAuthoring, MetadataAuthoringBatch, NodeAuthoringBatch,
    NodeAuthoringNode, NodeLocalId, ObservedResourceAuthoring, OwnerProvenanceAuthoring, Presence,
    PrimitiveDeformationAuthoring, ProfileAuthoringBatch, ProfileOwner, ProvenanceLocalId,
    ProvenanceNodeAuthoring, ResourceIdentity, ResourceIdentityError, ResourceIdentityLocalId,
    ResourceSnapshotAuthoring, SamplerAuthoring, SceneAuthoringBatch, SceneSpace,
    SessionOpaqueReason, SourceIdentityAuthoring, SourceIdentityLocalId, SourceObjectKind,
    SourceObjectLocator, SpatialAuthoringBatch, TextureAuthoring,
};
use crate::metered_storage::capacity_plan_cause;
use crate::{ImportStorageClass, ManifestDigest, MeteredOpaqueAllowance, ParseResult, ParseScope};
use baozi_operation::CapacityPlan;

static UNKNOWN_SPACE: SceneSpace = SceneSpace::UNKNOWN;

/// Borrowed node, spatial, geometry, and appearance input for a static Scene.
///
/// Construction allocates nothing. Space declarations default to unknown; geometry and
/// appearance default to empty. Submission fills geometry-aligned empty deformation rows and
/// attaches every canonical owner to the primary document's source identity, using the selected
/// format and the session's existing content digest. It does not invent per-object locators or
/// generated-operation provenance.
///
/// Images may carry encoded bytes or reference primary-document resource zero. Importers that
/// need other resource identities, metadata, deformation, animation, or more
/// precise provenance should use [`ParseScope::submit_scene`] with [`SceneAuthoringBatch`].
/// All input still passes through the same central Scene validation and publication boundary.
#[derive(Debug, Clone, Copy)]
#[must_use = "submit borrowed static Scene input through ParseScope::submit_static_scene"]
pub struct StaticSceneBuilder<'input> {
    scene_space: &'input SceneSpace,
    cameras: &'input [CameraProjection],
    lights: &'input [LightDefinition],
    roots: Presence<&'input [NodeLocalId]>,
    nodes: &'input [NodeAuthoringNode<'input>],
    logical_meshes: &'input [GeometryLogicalMeshAuthoring<'input>],
    primitives: &'input [GeometryPrimitiveAuthoring<'input>],
    attributes: &'input [GeometryAttributeAuthoring<'input>],
    mesh_instances: &'input [GeometryMeshInstanceAuthoring],
    images: &'input [ImageAuthoring<'input>],
    samplers: &'input [SamplerAuthoring],
    textures: &'input [TextureAuthoring],
    bindings: &'input [MaterialBindingAuthoring<'input>],
    materials: &'input [MaterialAuthoring<'input>],
}

impl<'input> StaticSceneBuilder<'input> {
    /// Starts a static Scene, preserving absent and present-empty root declarations.
    pub const fn new(
        roots: Presence<&'input [NodeLocalId]>,
        nodes: &'input [NodeAuthoringNode<'input>],
    ) -> Self {
        Self {
            scene_space: &UNKNOWN_SPACE,
            cameras: &[],
            lights: &[],
            roots,
            nodes,
            logical_meshes: &[],
            primitives: &[],
            attributes: &[],
            mesh_instances: &[],
            images: &[],
            samplers: &[],
            textures: &[],
            bindings: &[],
            materials: &[],
        }
    }

    /// Uses explicit source space declarations without transforming any input.
    pub const fn with_scene_space(mut self, scene_space: &'input SceneSpace) -> Self {
        self.scene_space = scene_space;
        self
    }

    /// Borrows complete camera/light definitions; placement remains node-owned.
    pub const fn with_spatial_definitions(
        mut self,
        cameras: &'input [CameraProjection],
        lights: &'input [LightDefinition],
    ) -> Self {
        self.cameras = cameras;
        self.lights = lights;
        self
    }

    /// Borrows the existing flat geometry tables in their producer-local order.
    pub const fn with_geometry(
        mut self,
        logical_meshes: &'input [GeometryLogicalMeshAuthoring<'input>],
        primitives: &'input [GeometryPrimitiveAuthoring<'input>],
        attributes: &'input [GeometryAttributeAuthoring<'input>],
        mesh_instances: &'input [GeometryMeshInstanceAuthoring],
    ) -> Self {
        self.logical_meshes = logical_meshes;
        self.primitives = primitives;
        self.attributes = attributes;
        self.mesh_instances = mesh_instances;
        self
    }

    /// Borrows appearance tables; every row receives document-wide source provenance.
    pub const fn with_appearance(
        mut self,
        images: &'input [ImageAuthoring<'input>],
        samplers: &'input [SamplerAuthoring],
        textures: &'input [TextureAuthoring],
        bindings: &'input [MaterialBindingAuthoring<'input>],
        materials: &'input [MaterialAuthoring<'input>],
    ) -> Self {
        self.images = images;
        self.samplers = samplers;
        self.textures = textures;
        self.bindings = bindings;
        self.materials = materials;
        self
    }
}

pub(crate) fn submit(
    scope: &ParseScope<'_>,
    input: StaticSceneBuilder<'_>,
    format: &FormatId,
    content_digest: ManifestDigest,
) -> ParseResult<crate::SceneDraft> {
    scope.poll()?;
    let document = document_resource(scope, content_digest)?;
    let locator = SourceObjectLocator::indexed(SourceObjectKind::Document, 0);
    let resources = [ObservedResourceAuthoring::session_opaque(
        &document.identity,
        document.reason,
    )];
    let sources = [
        SourceIdentityAuthoring::new(ResourceIdentityLocalId::new(0), format, &locator)
            .with_content_digest(ContentDigest::sha256(content_digest.as_bytes())),
    ];
    let provenance = [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
        0,
    ))];
    let source_provenance = [ProvenanceLocalId::new(0)];

    let mut logical_morphs = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "static-scene-logical-morphs",
    );
    logical_morphs.try_resize(
        input.logical_meshes.len(),
        LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[]),
    )?;
    let mut primitive_deformations = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "static-scene-primitive-deformations",
    );
    primitive_deformations.try_resize(
        input.primitives.len(),
        PrimitiveDeformationAuthoring::new(Presence::Absent, &[]),
    )?;
    let mut instance_deformations = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "static-scene-instance-deformations",
    );
    instance_deformations.try_resize(
        input.mesh_instances.len(),
        MeshInstanceDeformationAuthoring::new(Presence::Absent, Presence::Absent),
    )?;

    type OwnerConstructor = fn(u32) -> ProfileOwner;
    let owner_domains: [(usize, OwnerConstructor); 12] = [
        (input.nodes.len(), |index| {
            ProfileOwner::Node(NodeLocalId::new(index))
        }),
        (input.cameras.len(), ProfileOwner::Camera),
        (input.lights.len(), ProfileOwner::Light),
        (input.images.len(), ProfileOwner::Image),
        (input.samplers.len(), ProfileOwner::Sampler),
        (input.textures.len(), ProfileOwner::Texture),
        (input.bindings.len(), ProfileOwner::TextureBinding),
        (input.materials.len(), ProfileOwner::Material),
        (input.logical_meshes.len(), ProfileOwner::LogicalMesh),
        (input.primitives.len(), ProfileOwner::Primitive),
        (input.attributes.len(), ProfileOwner::Attribute),
        (input.mesh_instances.len(), ProfileOwner::MeshInstance),
    ];
    let owner_count = owner_domains
        .iter()
        .try_fold(2_usize, |total, (count, _)| {
            total
                .checked_add(*count)
                .ok_or_else(|| scope.resource_limit_exceeded("static-scene-owner-count", u64::MAX))
        })?;
    let mut owners = scope.metered_vec(
        ImportStorageClass::FormatNormalization,
        "static-scene-owner-provenance",
    );
    owners.try_reserve(owner_count)?;
    for owner in [ProfileOwner::Scene, ProfileOwner::SyntheticRoot] {
        owners.try_push(OwnerProvenanceAuthoring::new(owner, &source_provenance))?;
    }
    for (count, owner) in owner_domains {
        for index in 0..count {
            let index = u32::try_from(index).map_err(|_| {
                scope.resource_limit_exceeded("static-scene-owner-index", u64::from(u32::MAX))
            })?;
            owners.try_push(OwnerProvenanceAuthoring::new(
                owner(index),
                &source_provenance,
            ))?;
        }
    }

    let snapshots = [ResourceSnapshotAuthoring::snapshotted(
        scope.primary_bytes(),
    )];
    scope.submit_scene(SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(input.scene_space, input.cameras, input.lights),
        NodeAuthoringBatch::new(input.roots, input.nodes),
        AppearanceAuthoringBatch::new(
            input.images,
            input.samplers,
            input.textures,
            input.bindings,
            input.materials,
        ),
        GeometryAuthoringBatch::new(
            input.logical_meshes,
            input.primitives,
            input.attributes,
            input.mesh_instances,
        ),
        DeformationAuthoringBatch::new(
            &[],
            &[],
            logical_morphs.as_slice(),
            primitive_deformations.as_slice(),
            instance_deformations.as_slice(),
        ),
        AnimationAuthoringBatch::empty(),
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&resources, &sources, &provenance, owners.as_slice()),
            MetadataAuthoringBatch::empty(),
        )
        .with_resource_snapshots(&snapshots),
    ))
}

struct DocumentResource<'scope> {
    // The identity must be freed before releasing its allocation allowance.
    identity: ResourceIdentity,
    _storage: MeteredOpaqueAllowance<'scope>,
    reason: SessionOpaqueReason,
}

fn document_resource<'scope>(
    scope: &'scope ParseScope<'_>,
    digest: ManifestDigest,
) -> ParseResult<DocumentResource<'scope>> {
    let try_identity = |value: &str, reason| -> ParseResult<Option<DocumentResource<'scope>>> {
        if value.len() > MAX_RESOURCE_IDENTITY_LEN {
            return Ok(None);
        }
        let plan = CapacityPlan::for_type::<u8>(0, 0, value.len()).map_err(|cause| {
            scope.source_scope_abort(capacity_plan_cause("static-scene-document-identity", cause))
        })?;
        let bytes = plan.envelope_delta_bytes().map_err(|cause| {
            scope.source_scope_abort(capacity_plan_cause("static-scene-document-identity", cause))
        })?;
        let storage =
            scope.metered_opaque_allowance(ImportStorageClass::FormatNormalization, bytes)?;
        let identity = match ResourceIdentity::new(value) {
            Ok(identity) => identity,
            Err(ResourceIdentityError::AllocationFailed) => {
                return Err(scope.allocation_failure("static-scene-document-identity"));
            }
            Err(_) => return Ok(None),
        };
        plan.finish(0, identity.capacity_bytes()).map_err(|cause| {
            scope.source_scope_abort(capacity_plan_cause("static-scene-document-identity", cause))
        })?;
        Ok(Some(DocumentResource {
            identity,
            _storage: storage,
            reason,
        }))
    };

    if let Some(document) = try_identity(scope.source().as_str(), SessionOpaqueReason::CallerLabel)?
    {
        return Ok(document);
    }

    // A logical AssetPath can exceed ResourceIdentity's bound or contain '#'. Use the actual
    // document digest as its bounded fallback identity, without truncating or inventing a path.
    const PREFIX: &[u8] = b"document/sha256/";
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut fallback = [0_u8; PREFIX.len() + DIGEST_LEN * 2];
    fallback[..PREFIX.len()].copy_from_slice(PREFIX);
    for (byte, pair) in digest
        .as_bytes()
        .into_iter()
        .zip(fallback[PREFIX.len()..].chunks_exact_mut(2))
    {
        pair[0] = HEX[usize::from(byte >> 4)];
        pair[1] = HEX[usize::from(byte & 0x0f)];
    }
    let fallback = std::str::from_utf8(&fallback).map_err(|_| {
        scope.descriptor_contract_failure("static-scene-document-identity-encoding")
    })?;
    try_identity(fallback, SessionOpaqueReason::SessionFallback)?
        .ok_or_else(|| scope.descriptor_contract_failure("static-scene-document-identity-rejected"))
}
