#![allow(
    dead_code,
    reason = "each integration-test crate uses only the canonical Scene fixture variants it needs"
)]

use baozi_import::{
    AnimationAuthoringBatch, AppearanceAuthoringBatch, AuthoredTransform,
    DeformationAuthoringBatch, DependencyOutcome, DependencyRoleId, DependencySnapshot,
    GeometryAuthoringBatch, LineageAuthoringBatch, MetadataAuthoringBatch, NodeAuthoringBatch,
    NodeAuthoringNode, NodeLocalId, NodeParent, OperationKind, OptionsDigest,
    OwnerProvenanceAuthoring, ParseResult, ParseScope, Presence, ProfileAuthoringBatch,
    ProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ProvenanceOperationAuthoring,
    SceneAuthoringBatch, SceneDraft, SceneSpace, SpatialAuthoringBatch,
};

pub(crate) fn root_dependency<'scope>(
    scope: &'scope ParseScope<'_>,
    role: &DependencyRoleId,
    hostile_reference: &str,
) -> ParseResult<DependencyOutcome<'scope>> {
    let origin = scope.root_origin();
    scope.dependency(&origin, role, hostile_reference)
}

pub(crate) fn required_root_dependency<'scope>(
    scope: &'scope ParseScope<'_>,
    role: &DependencyRoleId,
    hostile_reference: &str,
) -> ParseResult<DependencySnapshot<'scope>> {
    let outcome = root_dependency(scope, role, hostile_reference)?;
    scope.into_available_dependency(outcome)
}

/// Submits the smallest complete canonical Scene accepted by the production import rail.
pub(crate) fn submit_minimal_scene(scope: &mut ParseScope<'_>) -> ParseResult<SceneDraft> {
    submit_scene_with_space(scope, SceneSpace::UNKNOWN)
}

/// Submits a complete node-only Scene large enough for validation-stage control tests.
pub(crate) fn submit_node_scene(
    scope: &mut ParseScope<'_>,
    node_count: usize,
) -> ParseResult<SceneDraft> {
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
    let roots = (0..node_count)
        .map(|index| NodeLocalId::new(u32::try_from(index).expect("test node index fits u32")))
        .collect::<Vec<_>>();
    let nodes = (0..node_count)
        .map(|_| {
            NodeAuthoringNode::new(
                Presence::Absent,
                AuthoredTransform::identity_trs(),
                Presence::Absent,
                Presence::Absent,
            )
        })
        .collect::<Vec<_>>();
    let owner_provenance = std::iter::once(ProfileOwner::Scene)
        .chain(std::iter::once(ProfileOwner::SyntheticRoot))
        .chain(roots.iter().copied().map(ProfileOwner::Node))
        .map(|owner| OwnerProvenanceAuthoring::new(owner, &provenance_ids))
        .collect::<Vec<_>>();
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

/// Submits the minimal complete Scene with a caller-selected coordinate profile.
pub(crate) fn submit_scene_with_space(
    scope: &mut ParseScope<'_>,
    scene_space: SceneSpace,
) -> ParseResult<SceneDraft> {
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
