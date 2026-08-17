#![allow(
    dead_code,
    reason = "each integration-test crate uses only the canonical Scene fixture variants it needs"
)]

use baozi_import::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AttributeDomain, AttributeSemantic,
    AuthoredTransform, AuthoringRange, DeformationAuthoringBatch, GeometryAttributeAuthoring,
    GeometryAuthoringBatch, GeometryLogicalMeshAuthoring, GeometryMeshInstanceAuthoring,
    GeometryPrimitiveAuthoring, GeometryTopologyAuthoring, LineageAuthoringBatch,
    LogicalMeshLocalId, LogicalMorphAuthoring, MeshInstanceDeformationAuthoring,
    MetadataAuthoringBatch, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, NodeParent,
    OperationKind, OptionsDigest, OwnerProvenanceAuthoring, ParseResult, ParseScope, Presence,
    PrimitiveDeformationAuthoring, ProfileAuthoringBatch, ProfileOwner, ProvenanceLocalId,
    ProvenanceNodeAuthoring, ProvenanceOperationAuthoring, SceneAuthoringBatch, SceneDraft,
    SceneSpace, SpatialAuthoringBatch,
};

/// Submits the smallest complete canonical Scene accepted by the production import rail.
pub(crate) fn submit_minimal_scene(scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
    let provenance_ids = [ProvenanceLocalId::new(0)];
    let operation_kind = OperationKind::Generated;
    let provenance_nodes = [ProvenanceNodeAuthoring::operation(
        ProvenanceOperationAuthoring::new(
            &operation_kind,
            OptionsDigest::sha256([0; 32]),
            &[],
            &[],
        ),
    )];
    let owner_provenance = [
        OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &provenance_ids),
    ];
    let roots = [];
    let nodes = [];
    let scene_space = SceneSpace::UNKNOWN;

    scope.submit_scene(SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&scene_space, &[], &[]),
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
        DeformationAuthoringBatch::empty(),
        AnimationAuthoringBatch::empty(),
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&[], &[], &provenance_nodes, &owner_provenance),
            MetadataAuthoringBatch::empty(),
        ),
    ))
}

/// Submits a complete Scene whose node-parent relation must be rejected by canonical preflight.
pub(crate) fn submit_invalid_parent_scene(scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
    let provenance_ids = [ProvenanceLocalId::new(0)];
    let operation_kind = OperationKind::Generated;
    let provenance_nodes = [ProvenanceNodeAuthoring::operation(
        ProvenanceOperationAuthoring::new(
            &operation_kind,
            OptionsDigest::sha256([0; 32]),
            &[],
            &[],
        ),
    )];
    let roots = [NodeLocalId::new(0)];
    let nodes = [
        NodeAuthoringNode::new(
            Presence::Absent,
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        ),
        NodeAuthoringNode::new(
            Presence::Present(NodeParent::new(NodeLocalId::new(2), 0)),
            AuthoredTransform::identity_trs(),
            Presence::Absent,
            Presence::Absent,
        ),
    ];
    let owner_provenance = [
        OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::Node(NodeLocalId::new(0)), &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::Node(NodeLocalId::new(1)), &provenance_ids),
    ];
    let scene_space = SceneSpace::UNKNOWN;

    scope.submit_scene(SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&scene_space, &[], &[]),
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::empty(),
        DeformationAuthoringBatch::empty(),
        AnimationAuthoringBatch::empty(),
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&[], &[], &provenance_nodes, &owner_provenance),
            MetadataAuthoringBatch::empty(),
        ),
    ))
}

/// Submits one root node with one point primitive and its aligned deformation/profile rows.
pub(crate) fn submit_point_geometry_scene(scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )];
    let positions = [0.0_f32, 0.0, 0.0];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        3,
        &positions,
        Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::Points,
        Presence::Absent,
        Presence::Absent,
        AuthoringRange::new(0, 1),
    )];
    let logical_meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let mesh_instances = [GeometryMeshInstanceAuthoring::new(
        NodeLocalId::new(0),
        LogicalMeshLocalId::new(0),
    )];
    let logical_morphs = [LogicalMorphAuthoring::new(AuthoringRange::new(0, 0), &[])];
    let primitive_deformations = [PrimitiveDeformationAuthoring::new(Presence::Absent, &[])];
    let mesh_instance_deformations = [MeshInstanceDeformationAuthoring::new(
        Presence::Absent,
        Presence::Absent,
    )];
    let provenance_ids = [ProvenanceLocalId::new(0)];
    let operation_kind = OperationKind::Generated;
    let provenance_nodes = [ProvenanceNodeAuthoring::operation(
        ProvenanceOperationAuthoring::new(
            &operation_kind,
            OptionsDigest::sha256([0; 32]),
            &[],
            &[],
        ),
    )];
    let owner_provenance = [
        OwnerProvenanceAuthoring::new(ProfileOwner::Scene, &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::SyntheticRoot, &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::Node(NodeLocalId::new(0)), &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::LogicalMesh(0), &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::Primitive(0), &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::Attribute(0), &provenance_ids),
        OwnerProvenanceAuthoring::new(ProfileOwner::MeshInstance(0), &provenance_ids),
    ];
    let scene_space = SceneSpace::UNKNOWN;

    scope.submit_scene(SceneAuthoringBatch::new(
        SpatialAuthoringBatch::new(&scene_space, &[], &[]),
        NodeAuthoringBatch::new(Presence::Present(&roots), &nodes),
        AppearanceAuthoringBatch::empty(),
        GeometryAuthoringBatch::new(&logical_meshes, &primitives, &attributes, &mesh_instances),
        DeformationAuthoringBatch::new(
            &[],
            &[],
            &logical_morphs,
            &primitive_deformations,
            &mesh_instance_deformations,
        ),
        AnimationAuthoringBatch::empty(),
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(&[], &[], &provenance_nodes, &owner_provenance),
            MetadataAuthoringBatch::empty(),
        ),
    ))
}
