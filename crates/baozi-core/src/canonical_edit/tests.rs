use super::{
    CanonicalEditCloneContext, CanonicalEditCommit, CanonicalEditError, CanonicalEditFailure,
    CanonicalEditPlan, CanonicalEditPublication, CanonicalInvalidations,
    CanonicalNodeTransformEdit, CanonicalNodeTransformEditStart, CanonicalRemapSet,
    CanonicalSubjectRemap,
};
use crate::animation::{AnimationClip, AnimationLimits, AnimationTimeDomain};
use crate::appearance::{
    AlphaMode, CanonicalMaterialDescriptor, CanonicalTextureBindingDescriptor, ColorInterpretation,
    MaterialFactors, ShadingModel, TextureBlendOperation, TextureChannelMapping,
    TextureChannelSwizzle, TextureCoordinateSource, TextureRole, TextureStrength, TextureTransform,
};
use crate::attribute::{
    Attribute, AttributeBuffer, AttributeDomain, AttributeFormat, AttributePayload,
    AttributeSemantic, ComponentType, Normalization,
};
use crate::canonical_scene::builder::CanonicalSceneBuilder;
use crate::canonical_scene::{CanonicalValidatedScene, SceneBounds};
use crate::canonical_validation::{
    CanonicalValidationError, CanonicalValidationLimit, validate_canonical_scene,
};
use crate::deform::{MorphTargetDefinition, Skin, SkinMatrixSpaces};
use crate::derived::{Derived, DerivedAbsenceReason, DerivedProducer};
use crate::derived_input::{
    logical_mesh_bounds_input_stamp, primitive_bounds_input_stamp, scene_bounds_input_stamp,
};
use crate::entity::{
    AuthoredTransform, AuthoredTransformKind, Camera, CameraProjection, Light, LightKind, Node,
};
use crate::extension::ExtensionSchemaSnapshot;
use crate::geometry::{
    IndexStream, LogicalMesh, LogicalMeshBounds, MeshInstance, MeshPrimitive, MorphBoundsPolicy,
    PrimitiveBounds, Topology,
};
use crate::identity::{
    MaterialIndex, NodeId, NodeIndex, SceneGenerationId, SceneLookupError, TextureId,
};
use crate::image::{CanonicalTextureDescriptor, Image, Sampler};
use crate::math::{Aabb, Color, Mat4, Quat, Vec3};
use crate::provenance::{Provenance, test_provenance_fixture};
use crate::remap::{RemapChange, RemapSpace};
use crate::scene_space::SceneSpace;
use crate::validation_control::{
    SCENE_VALIDATION_POLL_INTERVAL, SceneReservationKind, SceneValidationControl,
    SceneValidationLimits, SceneValidationWork,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use thiserror::Error;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
enum TestControlError {
    #[error("reservation belongs to another control owner")]
    WrongOwner,
    #[error("reservation kind does not match the requested operation")]
    WrongKind,
    #[error("injected control failure")]
    Injected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectedControlBehavior {
    Continue,
    Error,
    Panic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloneWorkPhase {
    Planning,
    Materialization,
}

fn apply_injected_behavior(
    behavior: InjectedControlBehavior,
    panic_message: &'static str,
) -> Result<(), TestControlError> {
    match behavior {
        InjectedControlBehavior::Continue => Ok(()),
        InjectedControlBehavior::Error => Err(TestControlError::Injected),
        InjectedControlBehavior::Panic => panic!("{panic_message}"),
    }
}

struct TestReservation {
    owner: Arc<()>,
    kind: SceneReservationKind,
    bytes: u64,
    published: bool,
    bundle_drop_state: Arc<AtomicU8>,
    rollback_observed_state: Arc<AtomicU8>,
    reservation_drop_count: Arc<AtomicUsize>,
    live_scratch_reservations: Arc<AtomicUsize>,
}

impl Drop for TestReservation {
    fn drop(&mut self) {
        self.reservation_drop_count.fetch_add(1, Ordering::SeqCst);
        if !self.published && self.kind != SceneReservationKind::ValidationScratch {
            self.rollback_observed_state.store(
                self.bundle_drop_state.load(Ordering::SeqCst),
                Ordering::SeqCst,
            );
        }
        if self.kind == SceneReservationKind::ValidationScratch {
            self.live_scratch_reservations
                .fetch_sub(1, Ordering::SeqCst);
        }
    }
}

struct TestControl {
    owner: Arc<()>,
    reserve_kinds: Vec<SceneReservationKind>,
    transition_count: usize,
    transition_required_bytes: Option<u64>,
    after_candidate_reserve_poll: InjectedControlBehavior,
    candidate_reserve_poll_pending: bool,
    after_transition_poll: InjectedControlBehavior,
    transitioned: bool,
    after_resize_poll: InjectedControlBehavior,
    resized: bool,
    materialization_interval_poll: InjectedControlBehavior,
    materialization_interval_poll_pending: bool,
    scratch_poll: InjectedControlBehavior,
    clone_work_failure: Option<(CloneWorkPhase, InjectedControlBehavior)>,
    candidate_live: bool,
    materialization_active: bool,
    materialization_clone_work: u64,
    panic_during_transition: bool,
    reject_transition: bool,
    max_reservation_bytes: Option<u64>,
    reject_resize: bool,
    candidate_resize_requests: Vec<(u64, u64)>,
    total_work: u64,
    clone_work: u64,
    poll_count: usize,
    reserve_requests: Vec<(SceneReservationKind, u64)>,
    bundle_drop_state: Arc<AtomicU8>,
    rollback_observed_state: Arc<AtomicU8>,
    reservation_drop_count: Arc<AtomicUsize>,
    live_scratch_reservations: Arc<AtomicUsize>,
}

impl TestControl {
    fn new() -> Self {
        Self {
            owner: Arc::new(()),
            reserve_kinds: Vec::new(),
            transition_count: 0,
            transition_required_bytes: None,
            after_candidate_reserve_poll: InjectedControlBehavior::Continue,
            candidate_reserve_poll_pending: false,
            after_transition_poll: InjectedControlBehavior::Continue,
            transitioned: false,
            after_resize_poll: InjectedControlBehavior::Continue,
            resized: false,
            materialization_interval_poll: InjectedControlBehavior::Continue,
            materialization_interval_poll_pending: false,
            scratch_poll: InjectedControlBehavior::Continue,
            clone_work_failure: None,
            candidate_live: false,
            materialization_active: false,
            materialization_clone_work: 0,
            panic_during_transition: false,
            reject_transition: false,
            max_reservation_bytes: None,
            reject_resize: false,
            candidate_resize_requests: Vec::new(),
            total_work: 0,
            clone_work: 0,
            poll_count: 0,
            reserve_requests: Vec::new(),
            bundle_drop_state: Arc::new(AtomicU8::new(0)),
            rollback_observed_state: Arc::new(AtomicU8::new(0)),
            reservation_drop_count: Arc::new(AtomicUsize::new(0)),
            live_scratch_reservations: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn shared_drop_state(&self) -> Arc<AtomicU8> {
        Arc::clone(&self.bundle_drop_state)
    }

    fn rollback_observed_state(&self) -> u8 {
        self.rollback_observed_state.load(Ordering::SeqCst)
    }

    fn reservation_drop_count(&self) -> usize {
        self.reservation_drop_count.load(Ordering::SeqCst)
    }

    fn live_scratch_reservations(&self) -> usize {
        self.live_scratch_reservations.load(Ordering::SeqCst)
    }

    fn candidate_reservation_bytes(&self) -> Option<u64> {
        self.reserve_requests.iter().find_map(|(kind, bytes)| {
            (*kind == SceneReservationKind::EditCandidate).then_some(*bytes)
        })
    }

    fn candidate_resize(&self) -> Option<(u64, u64)> {
        self.candidate_resize_requests.last().copied()
    }

    fn candidate_resize_count(&self) -> usize {
        self.candidate_resize_requests.len()
    }
}

impl SceneValidationControl for TestControl {
    type Error = TestControlError;
    type Reservation = TestReservation;

    fn poll(&mut self) -> Result<(), Self::Error> {
        self.poll_count += 1;
        if self.candidate_reserve_poll_pending {
            self.candidate_reserve_poll_pending = false;
            apply_injected_behavior(
                self.after_candidate_reserve_poll,
                "injected post-candidate-reserve poll panic",
            )?;
        }
        if self.resized {
            self.resized = false;
            apply_injected_behavior(self.after_resize_poll, "injected post-resize poll panic")?;
        }
        if self.materialization_interval_poll_pending {
            self.materialization_interval_poll_pending = false;
            apply_injected_behavior(
                self.materialization_interval_poll,
                "injected materialization interval poll panic",
            )?;
        }
        if self.live_scratch_reservations() != 0 {
            apply_injected_behavior(
                self.scratch_poll,
                "injected central-validation scratch poll panic",
            )?;
        }
        if self.transitioned {
            self.transitioned = false;
            apply_injected_behavior(
                self.after_transition_poll,
                "injected post-transition poll panic",
            )?;
        }
        Ok(())
    }

    fn charge_work(&mut self, work: SceneValidationWork, amount: u64) -> Result<(), Self::Error> {
        if work == SceneValidationWork::CloneCell {
            let phase = self
                .candidate_live
                .then_some(if self.materialization_active {
                    CloneWorkPhase::Materialization
                } else {
                    CloneWorkPhase::Planning
                });
            if let (Some(actual), Some((expected, behavior))) = (phase, self.clone_work_failure)
                && actual == expected
            {
                apply_injected_behavior(behavior, "injected clone-work charge panic")?;
            }
        }
        self.total_work = self
            .total_work
            .checked_add(amount)
            .expect("test total work must not overflow");
        if work == SceneValidationWork::CloneCell {
            self.clone_work = self
                .clone_work
                .checked_add(amount)
                .expect("test clone work must not overflow");
            if self.materialization_active {
                let before = self.materialization_clone_work;
                self.materialization_clone_work = before
                    .checked_add(amount)
                    .expect("test materialization work must not overflow");
                if before / SCENE_VALIDATION_POLL_INTERVAL
                    != self.materialization_clone_work / SCENE_VALIDATION_POLL_INTERVAL
                {
                    self.materialization_interval_poll_pending = true;
                }
            }
        }
        Ok(())
    }

    fn reserve(
        &mut self,
        kind: SceneReservationKind,
        bytes: u64,
    ) -> Result<Self::Reservation, Self::Error> {
        self.reserve_kinds.push(kind);
        self.reserve_requests.push((kind, bytes));
        if self
            .max_reservation_bytes
            .is_some_and(|maximum| bytes > maximum)
        {
            return Err(TestControlError::Injected);
        }
        if kind == SceneReservationKind::EditCandidate {
            self.candidate_live = true;
            self.candidate_reserve_poll_pending = true;
        } else if kind == SceneReservationKind::ValidationScratch {
            self.live_scratch_reservations
                .fetch_add(1, Ordering::SeqCst);
        }
        Ok(TestReservation {
            owner: Arc::clone(&self.owner),
            kind,
            bytes,
            published: false,
            bundle_drop_state: Arc::clone(&self.bundle_drop_state),
            rollback_observed_state: Arc::clone(&self.rollback_observed_state),
            reservation_drop_count: Arc::clone(&self.reservation_drop_count),
            live_scratch_reservations: Arc::clone(&self.live_scratch_reservations),
        })
    }

    fn validate_reservation(
        &self,
        reservation: &Self::Reservation,
        kind: SceneReservationKind,
    ) -> Result<(), Self::Error> {
        if !Arc::ptr_eq(&reservation.owner, &self.owner) {
            return Err(TestControlError::WrongOwner);
        }
        if reservation.kind != kind {
            return Err(TestControlError::WrongKind);
        }
        Ok(())
    }

    fn resize_reservation(
        &mut self,
        reservation: &mut Self::Reservation,
        kind: SceneReservationKind,
        required_bytes: u64,
    ) -> Result<(), Self::Error> {
        self.validate_reservation(reservation, kind)?;
        if kind == SceneReservationKind::EditCandidate {
            self.candidate_resize_requests
                .push((reservation.bytes, required_bytes));
        }
        if self.reject_resize
            || self
                .max_reservation_bytes
                .is_some_and(|maximum| required_bytes > maximum)
        {
            return Err(TestControlError::Injected);
        }
        reservation.bytes = required_bytes;
        self.resized = true;
        if kind == SceneReservationKind::EditCandidate {
            self.materialization_active = true;
        }
        Ok(())
    }

    fn transition_reservation(
        &mut self,
        reservation: &mut Self::Reservation,
        from: SceneReservationKind,
        to: SceneReservationKind,
        required_bytes: u64,
    ) -> Result<(), Self::Error> {
        self.transition_count += 1;
        if self.panic_during_transition {
            panic!("injected reservation transition panic");
        }
        self.validate_reservation(reservation, from)?;
        if self.reject_transition {
            return Err(TestControlError::Injected);
        }
        reservation.kind = to;
        reservation.bytes = required_bytes;
        self.transition_required_bytes = Some(required_bytes);
        self.transitioned = true;
        self.candidate_live = false;
        self.materialization_active = false;
        Ok(())
    }
}

fn scene_with_valid_bounds() -> CanonicalValidatedScene {
    scene_with_valid_bounds_and_transform(AuthoredTransform::identity_trs())
}

fn scene_with_valid_bounds_and_transform(
    root_transform: AuthoredTransform,
) -> CanonicalValidatedScene {
    let (arena, provenance) = test_provenance_fixture();
    let mut builder = CanonicalSceneBuilder::try_new(
        SceneSpace::unknown(),
        ExtensionSchemaSnapshot::empty(),
        arena,
        provenance,
        DerivedAbsenceReason::NotProvided,
    )
    .expect("canonical edit fixture builder");
    let root = builder
        .try_push_node(
            Node::try_new(None, root_transform, [], None, None, provenance, [])
                .expect("canonical edit fixture root"),
        )
        .expect("canonical edit fixture node storage");
    builder.set_root(root);
    let mut draft = builder.finish_draft();
    let stamp = scene_bounds_input_stamp(&draft.data)
        .expect("scene bounds fixture stamp")
        .stamp();
    draft.data.scene_bounds = SceneBounds::try_from_validated(Derived::valid(
        Aabb {
            min: Vec3::ZERO,
            max: Vec3::new(1.0, 1.0, 1.0),
        },
        draft.data.generation,
        stamp,
        DerivedProducer::source(),
    ))
    .expect("scene bounds fixture value");

    validate_canonical_scene(draft, SceneValidationLimits::default())
        .expect("canonical edit fixture should validate")
}

fn scene_with_three_nodes() -> CanonicalValidatedScene {
    let (arena, provenance) = test_provenance_fixture();
    let mut builder = CanonicalSceneBuilder::try_new(
        SceneSpace::unknown(),
        ExtensionSchemaSnapshot::empty(),
        arena,
        provenance,
        DerivedAbsenceReason::NotProvided,
    )
    .expect("multi-node edit fixture builder");
    let root = builder
        .try_push_node(
            Node::try_new(
                None,
                AuthoredTransform::identity_trs(),
                [NodeIndex::from_u32(1), NodeIndex::from_u32(2)],
                None,
                None,
                provenance,
                [],
            )
            .expect("multi-node root"),
        )
        .expect("multi-node root storage");
    builder
        .try_push_node(
            Node::try_new(
                None,
                AuthoredTransform::identity_trs(),
                [],
                None,
                None,
                provenance,
                [],
            )
            .expect("multi-node child"),
        )
        .expect("multi-node child storage");
    builder
        .try_push_node(
            Node::try_new(
                None,
                AuthoredTransform::try_matrix(Mat4::IDENTITY).expect("multi-node sibling matrix"),
                [],
                None,
                None,
                provenance,
                [],
            )
            .expect("multi-node sibling"),
        )
        .expect("multi-node sibling storage");
    builder.set_root(root);

    validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
        .expect("multi-node edit fixture should validate")
}

fn scene_with_many_nodes(node_count: usize) -> CanonicalValidatedScene {
    assert!(
        node_count > 1,
        "large-node fixture needs a root and children"
    );
    let (arena, provenance) = test_provenance_fixture();
    let mut builder = CanonicalSceneBuilder::try_new(
        SceneSpace::unknown(),
        ExtensionSchemaSnapshot::empty(),
        arena,
        provenance,
        DerivedAbsenceReason::NotProvided,
    )
    .expect("large-node edit fixture builder");
    let children = (1..node_count)
        .map(|index| {
            NodeIndex::from_u32(u32::try_from(index).expect("fixture node index must fit u32"))
        })
        .collect::<Vec<_>>();
    let root = builder
        .try_push_node(
            Node::try_new(
                None,
                AuthoredTransform::identity_trs(),
                children,
                None,
                None,
                provenance,
                [],
            )
            .expect("large-node root"),
        )
        .expect("large-node root storage");
    for _ in 1..node_count {
        builder
            .try_push_node(
                Node::try_new(
                    None,
                    AuthoredTransform::identity_trs(),
                    [],
                    None,
                    None,
                    provenance,
                    [],
                )
                .expect("large-node child"),
            )
            .expect("large-node child storage");
    }
    builder.set_root(root);

    validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
        .expect("large-node edit fixture should validate")
}

fn translated() -> AuthoredTransform {
    AuthoredTransform::try_trs(
        Vec3::new(3.0, 4.0, 5.0),
        Quat::IDENTITY,
        Vec3::new(1.0, 1.0, 1.0),
    )
    .expect("translated fixture transform")
}

fn positions(provenance: crate::provenance::Provenance) -> Attribute {
    Attribute::new(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        AttributeFormat::new(ComponentType::F32, 3, Normalization::None, None)
            .expect("position format fixture"),
        AttributePayload::dense(
            AttributeBuffer::from_f32_vec3(vec![
                Vec3::ZERO,
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ])
            .expect("position payload fixture"),
        ),
        provenance,
    )
    .expect("position attribute fixture")
}

fn primitive_with_valid_bounds(
    generation: crate::identity::SceneGenerationId,
    provenance: crate::provenance::Provenance,
) -> MeshPrimitive {
    fn build(bounds: PrimitiveBounds, provenance: crate::provenance::Provenance) -> MeshPrimitive {
        MeshPrimitive::try_new(
            3,
            Topology::TriangleList,
            IndexStream::non_indexed(),
            None,
            vec![positions(provenance)],
            None,
            Vec::new(),
            0,
            bounds,
            provenance,
        )
        .expect("primitive fixture")
    }

    let prototype = build(
        PrimitiveBounds::absent(
            DerivedAbsenceReason::NotProvided,
            MorphBoundsPolicy::BaseGeometry,
        ),
        provenance,
    );
    let stamp = primitive_bounds_input_stamp(&prototype, &[])
        .expect("primitive bounds fixture stamp")
        .stamp();
    build(
        PrimitiveBounds::try_from_validated(
            Derived::valid(
                Aabb {
                    min: Vec3::ZERO,
                    max: Vec3::new(1.0, 1.0, 0.0),
                },
                generation,
                stamp,
                DerivedProducer::source(),
            ),
            MorphBoundsPolicy::BaseGeometry,
        )
        .expect("primitive bounds fixture"),
        provenance,
    )
}

fn logical_mesh_with_valid_bounds(
    generation: crate::identity::SceneGenerationId,
    provenance: crate::provenance::Provenance,
) -> LogicalMesh {
    fn build(
        primitive: MeshPrimitive,
        bounds: LogicalMeshBounds,
        provenance: crate::provenance::Provenance,
    ) -> LogicalMesh {
        LogicalMesh::try_new(
            None,
            vec![primitive],
            Vec::new(),
            Vec::new(),
            bounds,
            provenance,
        )
        .expect("logical mesh fixture")
    }

    let prototype = build(
        primitive_with_valid_bounds(generation, provenance),
        LogicalMeshBounds::absent(
            DerivedAbsenceReason::NotProvided,
            MorphBoundsPolicy::BaseGeometry,
        ),
        provenance,
    );
    let stamp = logical_mesh_bounds_input_stamp(&prototype)
        .expect("logical mesh bounds fixture stamp")
        .stamp();
    build(
        primitive_with_valid_bounds(generation, provenance),
        LogicalMeshBounds::try_from_validated(
            Derived::valid(
                Aabb {
                    min: Vec3::ZERO,
                    max: Vec3::new(1.0, 1.0, 0.0),
                },
                generation,
                stamp,
                DerivedProducer::source(),
            ),
            MorphBoundsPolicy::BaseGeometry,
        )
        .expect("logical mesh bounds fixture"),
        provenance,
    )
}

fn scene_with_valid_local_and_scene_bounds() -> CanonicalValidatedScene {
    let (arena, provenance) = test_provenance_fixture();
    let mut builder = CanonicalSceneBuilder::try_new(
        SceneSpace::unknown(),
        ExtensionSchemaSnapshot::empty(),
        arena,
        provenance,
        DerivedAbsenceReason::NotProvided,
    )
    .expect("canonical edit local-bounds fixture builder");
    let generation = builder.generation();
    let root = builder
        .try_push_node(
            Node::try_new(
                None,
                AuthoredTransform::identity_trs(),
                [],
                None,
                None,
                provenance,
                [],
            )
            .expect("canonical edit local-bounds fixture root"),
        )
        .expect("canonical edit local-bounds fixture node storage");
    builder.set_root(root);
    builder
        .try_push_logical_mesh(logical_mesh_with_valid_bounds(generation, provenance))
        .expect("canonical edit local-bounds fixture mesh storage");
    let mut draft = builder.finish_draft();
    let stamp = scene_bounds_input_stamp(&draft.data)
        .expect("scene bounds fixture stamp")
        .stamp();
    draft.data.scene_bounds = SceneBounds::try_from_validated(Derived::valid(
        Aabb {
            min: Vec3::ZERO,
            max: Vec3::new(1.0, 1.0, 0.0),
        },
        generation,
        stamp,
        DerivedProducer::source(),
    ))
    .expect("scene bounds fixture value");

    validate_canonical_scene(draft, SceneValidationLimits::default())
        .expect("canonical edit local-bounds fixture should validate")
}

fn scene_with_three_primitives_and_absent_scene_bounds() -> CanonicalValidatedScene {
    let (arena, provenance) = test_provenance_fixture();
    let mut builder = CanonicalSceneBuilder::try_new(
        SceneSpace::unknown(),
        ExtensionSchemaSnapshot::empty(),
        arena,
        provenance,
        DerivedAbsenceReason::NotProvided,
    )
    .expect("canonical edit primitive-capacity fixture builder");
    let root = builder
        .try_push_node(
            Node::try_new(
                None,
                AuthoredTransform::identity_trs(),
                [],
                None,
                None,
                provenance,
                [],
            )
            .expect("canonical edit primitive-capacity fixture root"),
        )
        .expect("canonical edit primitive-capacity root storage");
    builder.set_root(root);

    let primitive = || {
        MeshPrimitive::try_new(
            0,
            Topology::Points,
            IndexStream::non_indexed(),
            None,
            Vec::new(),
            None,
            Vec::new(),
            0,
            PrimitiveBounds::absent(
                DerivedAbsenceReason::NotProvided,
                MorphBoundsPolicy::BaseGeometry,
            ),
            provenance,
        )
        .expect("canonical edit primitive-capacity fixture primitive")
    };
    let primitives = [primitive(), primitive(), primitive()]
        .into_iter()
        .collect::<Box<[_]>>()
        .into_vec();
    builder
        .try_push_logical_mesh(
            LogicalMesh::try_new(
                None,
                primitives,
                Vec::new(),
                Vec::new(),
                LogicalMeshBounds::absent(
                    DerivedAbsenceReason::NotProvided,
                    MorphBoundsPolicy::BaseGeometry,
                ),
                provenance,
            )
            .expect("canonical edit primitive-capacity logical mesh"),
        )
        .expect("canonical edit primitive-capacity logical-mesh storage");

    validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
        .expect("canonical edit primitive-capacity fixture should validate")
}

fn scene_with_many_vertices(vertex_count: usize) -> CanonicalValidatedScene {
    let (arena, provenance) = test_provenance_fixture();
    let mut builder = CanonicalSceneBuilder::try_new(
        SceneSpace::unknown(),
        ExtensionSchemaSnapshot::empty(),
        arena,
        provenance,
        DerivedAbsenceReason::NotProvided,
    )
    .expect("canonical edit large-remap fixture builder");
    let root = builder
        .try_push_node(
            Node::try_new(
                None,
                AuthoredTransform::identity_trs(),
                [],
                None,
                None,
                provenance,
                [],
            )
            .expect("canonical edit large-remap root"),
        )
        .expect("canonical edit large-remap root storage");
    builder.set_root(root);

    let attribute = Attribute::new(
        AttributeSemantic::Position,
        AttributeDomain::Vertex,
        AttributeFormat::new(ComponentType::F32, 3, Normalization::None, None)
            .expect("large-remap position format"),
        AttributePayload::dense(
            AttributeBuffer::from_f32_vec3(vec![Vec3::ZERO; vertex_count])
                .expect("large-remap position payload"),
        ),
        provenance,
    )
    .expect("large-remap position attribute");
    let primitive = MeshPrimitive::try_new(
        u32::try_from(vertex_count).expect("large-remap vertex count must fit u32"),
        Topology::Points,
        IndexStream::non_indexed(),
        None,
        vec![attribute],
        None,
        Vec::new(),
        0,
        PrimitiveBounds::absent(
            DerivedAbsenceReason::NotProvided,
            MorphBoundsPolicy::BaseGeometry,
        ),
        provenance,
    )
    .expect("large-remap primitive");
    builder
        .try_push_logical_mesh(
            LogicalMesh::try_new(
                None,
                vec![primitive],
                Vec::new(),
                Vec::new(),
                LogicalMeshBounds::absent(
                    DerivedAbsenceReason::NotProvided,
                    MorphBoundsPolicy::BaseGeometry,
                ),
                provenance,
            )
            .expect("large-remap logical mesh"),
        )
        .expect("large-remap logical mesh storage");

    validate_canonical_scene(builder.finish_draft(), SceneValidationLimits::default())
        .expect("large-remap fixture should validate")
}

mod admission;
mod behavior;
mod publication;
