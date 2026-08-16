use std::cell::Cell;
use std::rc::Rc;
use std::sync::OnceLock;

use thiserror::Error;

use crate::entity::{AuthoredTransform, AuthoredTransformKind};
use crate::math::Mat4;
use crate::provenance::{ResourceIdentity, SourceObjectKind, SourceObjectLocator};
use crate::{FormatId, SceneValidationLimits, SceneValidationWork};

use super::plan::{
    NodeAuthoringLimit, NodeAuthoringSemanticError, validate_node_batch_with_control,
};
use super::*;

mod animation;
mod appearance;
mod deformation;
mod geometry;
mod profile;
mod relation_limits;
mod scene;
mod spatial;

const PROFILE_SOURCE_PROVENANCE: [ProvenanceLocalId; 1] = [ProvenanceLocalId::new(0)];

struct ProfileFixtureVocabulary {
    resource: ResourceIdentity,
    format: FormatId,
    locator: SourceObjectLocator,
}

fn profile_fixture_vocabulary() -> &'static ProfileFixtureVocabulary {
    static VOCABULARY: OnceLock<ProfileFixtureVocabulary> = OnceLock::new();
    VOCABULARY.get_or_init(|| ProfileFixtureVocabulary {
        resource: ResourceIdentity::new("fixtures/production/input")
            .expect("test profile resource identity"),
        format: FormatId::new("org.baozi.fixture").expect("test profile format"),
        locator: SourceObjectLocator::indexed(SourceObjectKind::Document, 0),
    })
}

struct ProfileFixture {
    resources: [ObservedResourceAuthoring<'static>; 1],
    sources: [SourceIdentityAuthoring<'static>; 1],
    provenance: [ProvenanceNodeAuthoring<'static>; 1],
    owners: Vec<OwnerProvenanceAuthoring<'static>>,
}

impl ProfileFixture {
    fn batch(&self) -> ProfileAuthoringBatch<'_> {
        ProfileAuthoringBatch::new(
            LineageAuthoringBatch::new(
                &self.resources,
                &self.sources,
                &self.provenance,
                &self.owners,
            ),
            MetadataAuthoringBatch::empty(),
        )
    }
}

fn profile_fixture(
    spatial: SpatialAuthoringBatch<'_>,
    nodes: NodeAuthoringBatch<'_>,
    appearance: AppearanceAuthoringBatch<'_>,
    geometry: GeometryAuthoringBatch<'_>,
    deformation: DeformationAuthoringBatch<'_>,
    animation: AnimationAuthoringBatch<'_>,
) -> ProfileFixture {
    let vocabulary = profile_fixture_vocabulary();
    let mut owners = vec![ProfileOwner::Scene, ProfileOwner::SyntheticRoot];
    owners.extend(nodes.nodes().iter().enumerate().map(|(index, _)| {
        ProfileOwner::Node(NodeLocalId::new(
            u32::try_from(index).expect("test node index"),
        ))
    }));
    owners.extend(indexed_profile_owners(
        spatial.cameras().len(),
        ProfileOwner::Camera,
    ));
    owners.extend(indexed_profile_owners(
        spatial.lights().len(),
        ProfileOwner::Light,
    ));
    owners.extend(indexed_profile_owners(
        appearance.images().len(),
        ProfileOwner::Image,
    ));
    owners.extend(indexed_profile_owners(
        appearance.samplers().len(),
        ProfileOwner::Sampler,
    ));
    owners.extend(indexed_profile_owners(
        appearance.textures().len(),
        ProfileOwner::Texture,
    ));
    owners.extend(indexed_profile_owners(
        appearance.bindings().len(),
        ProfileOwner::TextureBinding,
    ));
    owners.extend(indexed_profile_owners(
        appearance.materials().len(),
        ProfileOwner::Material,
    ));
    owners.extend(indexed_profile_owners(
        geometry.logical_meshes().len(),
        ProfileOwner::LogicalMesh,
    ));
    owners.extend(indexed_profile_owners(
        geometry.primitives().len(),
        ProfileOwner::Primitive,
    ));
    owners.extend(indexed_profile_owners(
        geometry.attributes().len(),
        ProfileOwner::Attribute,
    ));
    owners.extend(indexed_profile_owners(
        geometry.mesh_instances().len(),
        ProfileOwner::MeshInstance,
    ));
    owners.extend(indexed_profile_owners(
        deformation.skins().len(),
        ProfileOwner::Skin,
    ));
    owners.extend(indexed_profile_owners(
        deformation.morph_target_definitions().len(),
        ProfileOwner::MorphDefinition,
    ));
    let morph_delta_count = deformation
        .primitives()
        .iter()
        .flat_map(|primitive| primitive.morph_targets())
        .map(|target| target.deltas().len())
        .sum();
    owners.extend(indexed_profile_owners(
        morph_delta_count,
        ProfileOwner::MorphDelta,
    ));
    owners.extend(indexed_profile_owners(
        animation.clips().len(),
        ProfileOwner::AnimationClip,
    ));
    owners.extend(indexed_profile_owners(
        animation.channels().len(),
        ProfileOwner::AnimationChannel,
    ));

    ProfileFixture {
        resources: [ObservedResourceAuthoring::session_opaque(
            &vocabulary.resource,
            SessionOpaqueReason::CallerLabel,
        )],
        sources: [SourceIdentityAuthoring::new(
            ResourceIdentityLocalId::new(0),
            &vocabulary.format,
            &vocabulary.locator,
        )],
        provenance: [ProvenanceNodeAuthoring::source(SourceIdentityLocalId::new(
            0,
        ))],
        owners: owners
            .into_iter()
            .map(|owner| OwnerProvenanceAuthoring::new(owner, &PROFILE_SOURCE_PROVENANCE))
            .collect(),
    }
}

fn indexed_profile_owners(
    len: usize,
    owner: impl Fn(u32) -> ProfileOwner,
) -> impl Iterator<Item = ProfileOwner> {
    (0..len).map(move |index| owner(u32::try_from(index).expect("test profile owner index")))
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
enum TestControlError {
    #[error("test control cancelled validation")]
    Cancelled,
    #[error("test control rejected validation work")]
    WorkRejected,
    #[error("test control rejected reservation resize")]
    ResizeRejected,
}

struct TestReservation {
    live: Rc<Cell<usize>>,
    current_bytes: Rc<Cell<u64>>,
    drop_stage: Option<Rc<Cell<u8>>>,
    bytes: u64,
}

impl Drop for TestReservation {
    fn drop(&mut self) {
        if let Some(stage) = &self.drop_stage {
            assert_eq!(
                stage.replace(2),
                1,
                "compiled storage must drop before its reservation"
            );
        }
        let live = self.live.get();
        assert!(live > 0, "a reservation must be released exactly once");
        self.live.set(live - 1);
        self.current_bytes.set(
            self.current_bytes
                .get()
                .checked_sub(self.bytes)
                .expect("test reservation bytes must remain live until drop"),
        );
    }
}

#[derive(Default)]
struct TestControl {
    live: Rc<Cell<usize>>,
    current_bytes: Rc<Cell<u64>>,
    peak_bytes: Rc<Cell<u64>>,
    polls: u64,
    work: u64,
    traversal_work: u64,
    last_work: Option<SceneValidationWork>,
    reserves: u64,
    resizes: u64,
    cancel_on_poll: Option<u64>,
    reject_work_after: Option<u64>,
    reject_resize_on: Option<u64>,
    reservation_drop_stage: Option<Rc<Cell<u8>>>,
}

impl TestControl {
    fn peak_bytes(&self) -> u64 {
        self.peak_bytes.get()
    }

    fn traversal_work(&self) -> u64 {
        self.traversal_work
    }

    fn last_work(&self) -> Option<SceneValidationWork> {
        self.last_work
    }

    fn update_peak_bytes(&self) {
        self.peak_bytes
            .set(self.peak_bytes.get().max(self.current_bytes.get()));
    }
}

impl CanonicalAuthoringControl for TestControl {
    type Error = TestControlError;
    type Reservation = TestReservation;

    fn poll(&mut self) -> Result<(), Self::Error> {
        self.polls += 1;
        if self.cancel_on_poll == Some(self.polls) {
            Err(TestControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn run_unit<R>(
        &mut self,
        work: SceneValidationWork,
        mut action: impl FnMut() -> R,
    ) -> Result<R, Self::Error> {
        let final_work = self.work.saturating_add(1);
        if self
            .reject_work_after
            .is_some_and(|maximum| final_work > maximum)
        {
            return Err(TestControlError::WorkRejected);
        }
        let result = action();
        self.work = final_work;
        self.last_work = Some(work);
        if matches!(work, SceneValidationWork::Traversal) {
            self.traversal_work = self.traversal_work.saturating_add(1);
        }
        Ok(result)
    }

    fn reserve_scratch(&mut self, bytes: u64) -> Result<Self::Reservation, Self::Error> {
        self.reserves += 1;
        self.live.set(self.live.get() + 1);
        self.current_bytes.set(
            self.current_bytes
                .get()
                .checked_add(bytes)
                .expect("test reservation bytes must be representable"),
        );
        self.update_peak_bytes();
        Ok(TestReservation {
            live: Rc::clone(&self.live),
            current_bytes: Rc::clone(&self.current_bytes),
            drop_stage: self.reservation_drop_stage.as_ref().map(Rc::clone),
            bytes,
        })
    }

    fn validate_scratch(&mut self, _reservation: &Self::Reservation) -> Result<(), Self::Error> {
        Ok(())
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut Self::Reservation,
        required_bytes: u64,
    ) -> Result<(), Self::Error> {
        self.resizes += 1;
        if self.reject_resize_on == Some(self.resizes) {
            return Err(TestControlError::ResizeRejected);
        }
        let without_reservation = self
            .current_bytes
            .get()
            .checked_sub(reservation.bytes)
            .expect("test reservation resize must retain its prior bytes");
        self.current_bytes.set(
            without_reservation
                .checked_add(required_bytes)
                .expect("test reservation resize bytes must be representable"),
        );
        reservation.bytes = required_bytes;
        self.update_peak_bytes();
        Ok(())
    }
}

fn validate<'a>(
    roots: Presence<&'a [NodeLocalId]>,
    nodes: &'a [NodeAuthoringNode],
    limits: SceneValidationLimits,
    control: &mut TestControl,
) -> Result<ValidatedNodeBatch<'a, TestReservation>, NodeAuthoringFailure<TestControlError>> {
    validate_node_batch_with_control(NodeAuthoringBatch::new(roots, nodes), limits, control)
}

fn parent(node: u32, child_ordinal: u32) -> Presence<NodeParent> {
    Presence::Present(NodeParent::new(NodeLocalId::new(node), child_ordinal))
}

fn trs(parent: Presence<NodeParent>) -> NodeAuthoringNode {
    NodeAuthoringNode::new(
        parent,
        AuthoredTransform::identity_trs(),
        Presence::Absent,
        Presence::Absent,
    )
}

fn matrix(parent: Presence<NodeParent>) -> NodeAuthoringNode {
    NodeAuthoringNode::new(
        parent,
        AuthoredTransform::try_matrix(Mat4::IDENTITY).expect("identity matrix transform"),
        Presence::Absent,
        Presence::Absent,
    )
}

fn expect_semantic(
    result: Result<ValidatedNodeBatch<'_, TestReservation>, NodeAuthoringFailure<TestControlError>>,
) -> NodeAuthoringSemanticError {
    match result {
        Err(NodeAuthoringFailure::Semantic(error)) => error,
        Err(NodeAuthoringFailure::Control(error)) => {
            panic!("expected semantic failure, got control failure: {error}")
        }
        Ok(batch) => panic!("expected semantic failure, got {batch:?}"),
    }
}

#[test]
fn single_trs_authoring_work_is_accounted_exactly() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();

    let batch = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("single TRS node must validate");

    assert_eq!(control.work, 25);
    drop(batch);
}

#[test]
fn compiled_node_plan_does_not_borrow_authored_storage() {
    let mut control = TestControl::default();
    let compiled = {
        let roots = vec![NodeLocalId::new(0)];
        let nodes = vec![trs(Presence::Absent)];
        validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default(),
            &mut control,
        )
        .expect("temporary authored storage must compile into owned node facts")
        .into_compiled_facts_for_test()
    };

    assert_eq!(compiled.0, 1);
    assert_eq!(compiled.1, 1);
    assert_eq!(compiled.2, [0]);
    assert_eq!(compiled.3, [1]);

    drop(compiled);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn valid_forest_compiles_parent_before_child_order_and_counts() {
    let roots = [NodeLocalId::new(2), NodeLocalId::new(0)];
    let nodes = [
        trs(Presence::Absent),
        matrix(parent(0, 0)),
        matrix(Presence::Absent),
        trs(parent(2, 0)),
        trs(parent(1, 0)),
    ];
    let mut control = TestControl::default();

    let batch = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("valid authored forest");

    assert_eq!(batch.authored_node_count(), 5);
    assert_eq!(batch.source_root_count(), 2);
    assert_eq!(batch.canonical_node_count(), 6);
    assert_eq!(batch.reference_count(), 5);
    assert_eq!(batch.matrix_scalar_count(), 32);
    assert_eq!(batch.trs_scalar_count(), 30);
    assert_eq!(batch.canonical_bytes(), 512);
    assert_eq!(batch.canonical_order(), &[2, 0, 3, 1, 4]);
    assert_eq!(batch.canonical_index_by_local(), &[2, 4, 1, 3, 5]);
    assert_eq!(control.live.get(), 1);
    assert!(control.work > 0);

    drop(batch);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn child_ordinals_preserve_sibling_order_independently_of_local_ids() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent), trs(parent(0, 1)), trs(parent(0, 0))];
    let mut control = TestControl::default();

    let batch = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("ordinal-ordered siblings must validate");

    assert_eq!(batch.canonical_order(), &[0, 2, 1]);
    assert_eq!(batch.canonical_index_by_local(), &[1, 3, 2]);
}

#[test]
fn local_node_array_reordering_preserves_the_canonical_tree() {
    fn canonical_shape<R>(batch: &ValidatedNodeBatch<'_, R>) -> Vec<(u32, AuthoredTransformKind)> {
        let input = batch.input();
        batch
            .canonical_order()
            .iter()
            .map(|&local| {
                let node = input.nodes()[local];
                let parent = match node.parent() {
                    Presence::Absent => 0,
                    Presence::Present(parent) => {
                        let parent_local = usize::try_from(parent.node().index())
                            .expect("test local node identity must fit usize");
                        batch.canonical_index_by_local()[parent_local]
                    }
                };
                (parent, node.authored_transform().kind())
            })
            .collect()
    }

    let roots_a = [NodeLocalId::new(0)];
    let nodes_a = [
        trs(Presence::Absent),
        matrix(parent(0, 1)),
        trs(parent(0, 0)),
    ];
    let roots_b = [NodeLocalId::new(2)];
    let nodes_b = [
        matrix(parent(2, 1)),
        trs(parent(2, 0)),
        trs(Presence::Absent),
    ];
    let mut control_a = TestControl::default();
    let mut control_b = TestControl::default();
    let batch_a = validate(
        Presence::Present(&roots_a),
        &nodes_a,
        SceneValidationLimits::default(),
        &mut control_a,
    )
    .expect("first local layout must validate");
    let batch_b = validate(
        Presence::Present(&roots_b),
        &nodes_b,
        SceneValidationLimits::default(),
        &mut control_b,
    )
    .expect("reordered local layout must validate");

    assert_eq!(
        canonical_shape(&batch_a),
        canonical_shape(&batch_b),
        "canonical identity and sibling order must not depend on producer-local indices"
    );
}

#[test]
fn invalid_child_ordinals_are_rejected_before_materialization() {
    let single_roots = [NodeLocalId::new(0)];
    let single_nodes = [trs(Presence::Absent), trs(parent(0, 1))];
    let mut control = TestControl::default();

    assert!(matches!(
        expect_semantic(validate(
            Presence::Present(&single_roots),
            &single_nodes,
            SceneValidationLimits::default(),
            &mut control,
        )),
        NodeAuthoringSemanticError::ChildOrdinalOutOfRange {
            node: 1,
            parent: 0,
            ordinal: 1,
            child_count: 1
        }
    ));

    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent), trs(parent(0, 0)), trs(parent(0, 0))];
    assert!(matches!(
        expect_semantic(validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default(),
            &mut control,
        )),
        NodeAuthoringSemanticError::DuplicateChildOrdinal {
            parent: 0,
            ordinal: 0,
            first: 1,
            duplicate: 2
        }
    ));
}

#[test]
fn present_empty_is_valid_and_absent_is_not() {
    let roots = [];
    let nodes = [];
    let mut control = TestControl::default();

    let batch = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default().with_max_canonical_bytes(88),
        &mut control,
    )
    .expect("present empty forest");
    assert_eq!(batch.canonical_node_count(), 1);
    assert_eq!(batch.reference_count(), 0);
    assert_eq!(batch.matrix_scalar_count(), 0);
    assert_eq!(batch.trs_scalar_count(), 0);
    assert_eq!(batch.canonical_bytes(), 88);
    assert!(batch.canonical_order().is_empty());
    assert!(batch.canonical_index_by_local().is_empty());
    drop(batch);

    assert!(matches!(
        expect_semantic(validate(
            Presence::Absent,
            &nodes,
            SceneValidationLimits::default(),
            &mut control,
        )),
        NodeAuthoringSemanticError::RootsAbsent
    ));
    assert_eq!(control.last_work(), Some(SceneValidationWork::GraphNode));
}

#[test]
fn out_of_range_root_and_parent_are_rejected() {
    let nodes = [trs(Presence::Absent)];
    let roots = [NodeLocalId::new(1)];
    let mut control = TestControl::default();
    assert!(matches!(
        expect_semantic(validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default(),
            &mut control,
        )),
        NodeAuthoringSemanticError::RootOutOfRange {
            root_index: 0,
            root: 1,
            node_count: 1
        }
    ));

    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent), trs(parent(2, 0))];
    assert!(matches!(
        expect_semantic(validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default(),
            &mut control,
        )),
        NodeAuthoringSemanticError::ParentOutOfRange {
            node: 1,
            parent: 2,
            node_count: 2
        }
    ));
}

#[test]
fn duplicate_roots_and_root_parent_mismatches_are_rejected() {
    let duplicate_roots = [NodeLocalId::new(0), NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();
    assert!(matches!(
        expect_semantic(validate(
            Presence::Present(&duplicate_roots),
            &nodes,
            SceneValidationLimits::default(),
            &mut control,
        )),
        NodeAuthoringSemanticError::DuplicateRoot {
            root: 0,
            duplicate_index: 1
        }
    ));

    let roots = [NodeLocalId::new(0), NodeLocalId::new(1)];
    let nodes = [trs(parent(1, 0)), trs(Presence::Absent)];
    assert!(matches!(
        expect_semantic(validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default(),
            &mut control,
        )),
        NodeAuthoringSemanticError::RootHasParent { root: 0, parent: 1 }
    ));

    let roots = [];
    let nodes = [trs(Presence::Absent)];
    assert!(matches!(
        expect_semantic(validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default(),
            &mut control,
        )),
        NodeAuthoringSemanticError::MissingRoot { node: 0 }
    ));
}

#[test]
fn self_parent_and_cycles_are_rejected() {
    let roots = [NodeLocalId::new(0)];
    let self_parent = [trs(Presence::Absent), trs(parent(1, 0))];
    let mut control = TestControl::default();
    assert!(matches!(
        expect_semantic(validate(
            Presence::Present(&roots),
            &self_parent,
            SceneValidationLimits::default(),
            &mut control,
        )),
        NodeAuthoringSemanticError::SelfParent { node: 1 }
    ));

    let cycle = [trs(Presence::Absent), trs(parent(2, 0)), trs(parent(1, 0))];
    assert!(matches!(
        expect_semantic(validate(
            Presence::Present(&roots),
            &cycle,
            SceneValidationLimits::default(),
            &mut control,
        )),
        NodeAuthoringSemanticError::Cycle { node } if node == 1 || node == 2
    ));
}

#[test]
fn depth_limit_is_enforced_during_iterative_resolution() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent), trs(parent(0, 0)), trs(parent(1, 0))];
    let mut control = TestControl::default();

    assert!(matches!(
        expect_semantic(validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default().with_max_depth(3),
            &mut control,
        )),
        NodeAuthoringSemanticError::LimitExceeded {
            limit: NodeAuthoringLimit::Depth,
            actual: 4,
            maximum: 3
        }
    ));
}

#[test]
fn depth_limits_include_the_synthetic_root() {
    let empty_roots = [];
    let empty_nodes = [];
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let mut control = TestControl::default();

    assert!(matches!(
        expect_semantic(validate(
            Presence::Present(&empty_roots),
            &empty_nodes,
            SceneValidationLimits::default().with_max_depth(0),
            &mut control,
        )),
        NodeAuthoringSemanticError::LimitExceeded {
            limit: NodeAuthoringLimit::Depth,
            actual: 1,
            maximum: 0
        }
    ));
    drop(
        validate(
            Presence::Present(&empty_roots),
            &empty_nodes,
            SceneValidationLimits::default().with_max_depth(1),
            &mut control,
        )
        .expect("the synthetic root alone has depth one"),
    );
    assert!(matches!(
        expect_semantic(validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default().with_max_depth(1),
            &mut control,
        )),
        NodeAuthoringSemanticError::LimitExceeded {
            limit: NodeAuthoringLimit::Depth,
            actual: 2,
            maximum: 1
        }
    ));
    drop(
        validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default().with_max_depth(2),
            &mut control,
        )
        .expect("one source root below the synthetic root has depth two"),
    );
}

#[test]
fn node_edge_semantic_reference_scratch_and_work_limits_are_typed() {
    let empty_roots = [];
    let empty_nodes = [];
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];

    let cases = [
        (
            Presence::Present(empty_roots.as_slice()),
            empty_nodes.as_slice(),
            SceneValidationLimits::default().with_max_nodes(0),
            NodeAuthoringLimit::Nodes,
        ),
        (
            Presence::Present(roots.as_slice()),
            nodes.as_slice(),
            SceneValidationLimits::default().with_max_payload_cells(9),
            NodeAuthoringLimit::SemanticCells,
        ),
        (
            Presence::Present(roots.as_slice()),
            nodes.as_slice(),
            SceneValidationLimits::default().with_max_edges(0),
            NodeAuthoringLimit::Edges,
        ),
        (
            Presence::Present(roots.as_slice()),
            nodes.as_slice(),
            SceneValidationLimits::default().with_max_references(0),
            NodeAuthoringLimit::References,
        ),
        (
            Presence::Present(roots.as_slice()),
            nodes.as_slice(),
            SceneValidationLimits::default().with_max_canonical_bytes(167),
            NodeAuthoringLimit::CanonicalBytes,
        ),
        (
            Presence::Present(roots.as_slice()),
            nodes.as_slice(),
            SceneValidationLimits::default().with_max_scratch_bytes(0),
            NodeAuthoringLimit::ScratchBytes,
        ),
        (
            Presence::Present(empty_roots.as_slice()),
            empty_nodes.as_slice(),
            SceneValidationLimits::default().with_max_work(0),
            NodeAuthoringLimit::Work,
        ),
    ];

    for (roots, nodes, limits, expected_limit) in cases {
        let mut control = TestControl::default();
        assert!(matches!(
            expect_semantic(validate(roots, nodes, limits, &mut control)),
            NodeAuthoringSemanticError::LimitExceeded { limit, .. }
                if limit == expected_limit
        ));
    }
}

#[test]
fn initial_cancellation_and_control_budget_rejection_are_control_failures() {
    let roots = [];
    let nodes = [];
    let mut cancelled = TestControl {
        cancel_on_poll: Some(1),
        ..TestControl::default()
    };
    assert!(matches!(
        validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default(),
            &mut cancelled,
        ),
        Err(NodeAuthoringFailure::Control(TestControlError::Cancelled))
    ));
    assert_eq!(cancelled.reserves, 0);
    assert_eq!(cancelled.work, 0);

    let mut budget = TestControl {
        reject_work_after: Some(0),
        ..TestControl::default()
    };
    assert!(matches!(
        validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default(),
            &mut budget,
        ),
        Err(NodeAuthoringFailure::Control(
            TestControlError::WorkRejected
        ))
    ));
    assert_eq!(budget.reserves, 0);
}

#[test]
fn periodic_cancellation_releases_live_scratch() {
    let nodes = vec![trs(Presence::Absent); 64];
    let roots = (0_u32..64).map(NodeLocalId::new).collect::<Vec<_>>();
    let mut control = TestControl {
        cancel_on_poll: Some(2),
        ..TestControl::default()
    };

    assert!(matches!(
        validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default(),
            &mut control,
        ),
        Err(NodeAuthoringFailure::Control(TestControlError::Cancelled))
    ));
    assert_eq!(control.reserves, 1);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn final_poll_cancellation_releases_compiled_plan_scratch() {
    let roots = [];
    let nodes = [];
    let mut control = TestControl {
        cancel_on_poll: Some(2),
        ..TestControl::default()
    };

    assert!(matches!(
        validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default(),
            &mut control,
        ),
        Err(NodeAuthoringFailure::Control(TestControlError::Cancelled))
    ));
    assert_eq!(control.reserves, 1);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn semantic_failure_releases_graph_scratch() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent), trs(parent(2, 0)), trs(parent(1, 0))];
    let mut control = TestControl::default();

    assert!(matches!(
        validate(
            Presence::Present(&roots),
            &nodes,
            SceneValidationLimits::default(),
            &mut control,
        ),
        Err(NodeAuthoringFailure::Semantic(
            NodeAuthoringSemanticError::Cycle { .. }
        ))
    ));
    assert_eq!(control.reserves, 1);
    assert_eq!(control.live.get(), 0);
}

#[test]
fn scratch_resize_rejections_release_the_complete_storage_owner() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];

    for rejected_resize in [1, 2] {
        let mut control = TestControl {
            reject_resize_on: Some(rejected_resize),
            ..TestControl::default()
        };
        assert!(matches!(
            validate(
                Presence::Present(&roots),
                &nodes,
                SceneValidationLimits::default(),
                &mut control,
            ),
            Err(NodeAuthoringFailure::Control(
                TestControlError::ResizeRejected
            ))
        ));
        assert_eq!(control.resizes, rejected_resize);
        assert_eq!(control.live.get(), 0);
    }
}

#[test]
fn compiled_storage_drops_before_its_reservation() {
    let roots = [NodeLocalId::new(0)];
    let nodes = [trs(Presence::Absent)];
    let stage = Rc::new(Cell::new(0));
    let mut control = TestControl {
        reservation_drop_stage: Some(Rc::clone(&stage)),
        ..TestControl::default()
    };

    let mut batch = validate(
        Presence::Present(&roots),
        &nodes,
        SceneValidationLimits::default(),
        &mut control,
    )
    .expect("valid single-root batch");
    batch.install_storage_drop_probe(Rc::clone(&stage));
    assert_eq!(control.live.get(), 1);
    assert_eq!(control.resizes, 2);

    drop(batch);
    assert_eq!(stage.get(), 2);
    assert_eq!(control.live.get(), 0);
}
