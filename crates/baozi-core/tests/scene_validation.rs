use baozi_core::{
    Animation, AnimationChannel, AnimationInterpolation, AnimationProperty, AnimationTarget,
    AnimationValues, Camera, CameraProjection, ColorSpace, Light, LightKind, Material, MaterialId,
    MaterialProperty, Mesh, MeshBinding, MeshId, MorphTarget, Node, NodeId, PrimitiveTopology,
    SCENE_VALIDATION_POLL_INTERVAL, SceneBuildError, SceneBuilder, SceneMetadataOwner,
    SceneReservationKind, SceneStorageFootprint, SceneValidationCode, SceneValidationControl,
    SceneValidationData, SceneValidationError, SceneValidationFailure, SceneValidationLimit,
    SceneValidationLimits, SceneValidationPath, SceneValidationWork, Skin, SkinId, Texture,
    TextureId, TextureRole, TextureSlot, TextureSource, ValidatedScene, Vec3, VertexAttribute,
    VertexAttributeData, VertexAttributeSemantic, validate_scene, validate_scene_with,
};
use std::cell::Cell;
use std::error::Error as _;
use std::fmt;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestControlError {
    Cancelled,
    PublicationRejected,
    ResizeRejected,
    ScratchReservationReleasedEarly,
    WrongReservationKind,
}

impl fmt::Display for TestControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Cancelled => "cancelled by test control",
            Self::PublicationRejected => "publication rejected by test control",
            Self::ResizeRejected => "reservation resize rejected by test control",
            Self::ScratchReservationReleasedEarly => {
                "scratch reservation released before its payload"
            }
            Self::WrongReservationKind => "wrong reservation kind",
        })
    }
}

impl std::error::Error for TestControlError {}

struct TestReservation {
    id: usize,
    kind: SceneReservationKind,
    bytes: u64,
    live_bytes: Rc<Cell<u64>>,
    live_scratch_reservations: Rc<Cell<usize>>,
}

impl Drop for TestReservation {
    fn drop(&mut self) {
        self.live_bytes.set(
            self.live_bytes
                .get()
                .checked_sub(self.bytes)
                .expect("reservation bytes must remain live until drop"),
        );
        if self.kind == SceneReservationKind::ValidationScratch {
            self.live_scratch_reservations.set(
                self.live_scratch_reservations
                    .get()
                    .checked_sub(1)
                    .expect("scratch reservation must remain live until drop"),
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReservationEvent {
    Reserve {
        id: usize,
        kind: SceneReservationKind,
        bytes: u64,
    },
    Transition {
        id: usize,
        from: SceneReservationKind,
        to: SceneReservationKind,
        required_bytes: u64,
    },
}

struct TestControl {
    polls: usize,
    cancel_on_poll: Option<usize>,
    work: u64,
    live_bytes: Rc<Cell<u64>>,
    live_scratch_reservations: Rc<Cell<usize>>,
    peak_live_bytes: u64,
    next_reservation_id: usize,
    events: Vec<ReservationEvent>,
    scratch_reservations_created: usize,
    verify_scratch_on_next_entity: bool,
    verified_scratch_lifetime: bool,
    reject_resize: bool,
}

impl TestControl {
    fn new() -> Self {
        Self {
            polls: 0,
            cancel_on_poll: None,
            work: 0,
            live_bytes: Rc::new(Cell::new(0)),
            live_scratch_reservations: Rc::new(Cell::new(0)),
            peak_live_bytes: 0,
            next_reservation_id: 1,
            events: Vec::new(),
            scratch_reservations_created: 0,
            verify_scratch_on_next_entity: false,
            verified_scratch_lifetime: false,
            reject_resize: false,
        }
    }

    fn cancel_on(poll: usize) -> Self {
        let mut control = Self::new();
        control.cancel_on_poll = Some(poll);
        control
    }

    fn set_cancel_on(&mut self, poll: usize) {
        self.cancel_on_poll = Some(poll);
    }

    fn live_bytes(&self) -> u64 {
        self.live_bytes.get()
    }

    fn record_live(&mut self, bytes: u64) {
        self.live_bytes.set(bytes);
        self.peak_live_bytes = self.peak_live_bytes.max(bytes);
    }
}

impl SceneValidationControl for TestControl {
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

    fn charge_work(&mut self, work: SceneValidationWork, amount: u64) -> Result<(), Self::Error> {
        if self.verify_scratch_on_next_entity && work == SceneValidationWork::Entity {
            if self.live_scratch_reservations.get() == 0 {
                return Err(TestControlError::ScratchReservationReleasedEarly);
            }
            self.verify_scratch_on_next_entity = false;
            self.verified_scratch_lifetime = true;
        }
        self.work = self
            .work
            .checked_add(amount)
            .expect("test work accounting must not overflow");
        Ok(())
    }

    fn reserve(
        &mut self,
        kind: SceneReservationKind,
        bytes: u64,
    ) -> Result<Self::Reservation, Self::Error> {
        let id = self.next_reservation_id;
        self.next_reservation_id += 1;
        self.events
            .push(ReservationEvent::Reserve { id, kind, bytes });
        self.record_live(self.live_bytes().checked_add(bytes).unwrap());
        if kind == SceneReservationKind::ValidationScratch {
            self.live_scratch_reservations.set(
                self.live_scratch_reservations
                    .get()
                    .checked_add(1)
                    .expect("scratch reservation count must not overflow"),
            );
            self.scratch_reservations_created += 1;
            if self.scratch_reservations_created == 2 && bytes > 0 {
                self.verify_scratch_on_next_entity = true;
            }
        }
        Ok(TestReservation {
            id,
            kind,
            bytes,
            live_bytes: Rc::clone(&self.live_bytes),
            live_scratch_reservations: Rc::clone(&self.live_scratch_reservations),
        })
    }

    fn resize_reservation(
        &mut self,
        reservation: &mut Self::Reservation,
        kind: SceneReservationKind,
        required_bytes: u64,
    ) -> Result<(), Self::Error> {
        if reservation.kind != kind {
            return Err(TestControlError::WrongReservationKind);
        }
        if self.reject_resize {
            return Err(TestControlError::ResizeRejected);
        }
        self.events.push(ReservationEvent::Transition {
            id: reservation.id,
            from: kind,
            to: kind,
            required_bytes,
        });
        let without_old = self
            .live_bytes()
            .checked_sub(reservation.bytes)
            .expect("resized reservation bytes must remain live");
        self.record_live(without_old.checked_add(required_bytes).unwrap());
        reservation.bytes = required_bytes;
        Ok(())
    }

    fn transition_reservation(
        &mut self,
        mut reservation: Self::Reservation,
        from: SceneReservationKind,
        to: SceneReservationKind,
        required_bytes: u64,
    ) -> Result<Self::Reservation, Self::Error> {
        if reservation.kind != from {
            return Err(TestControlError::WrongReservationKind);
        }
        self.events.push(ReservationEvent::Transition {
            id: reservation.id,
            from,
            to,
            required_bytes,
        });
        let without_old = self
            .live_bytes()
            .checked_sub(reservation.bytes)
            .expect("transitioned reservation bytes must remain live");
        self.record_live(without_old.checked_add(required_bytes).unwrap());
        reservation.kind = to;
        reservation.bytes = required_bytes;
        Ok(reservation)
    }
}

fn assert_cancelled(error: &SceneValidationFailure<TestControlError>) {
    assert!(matches!(
        error,
        SceneValidationFailure::Control(TestControlError::Cancelled)
    ));
    assert!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<TestControlError>())
            .is_some()
    );
}

fn valid_triangle_builder() -> SceneBuilder {
    let mut builder = SceneBuilder::try_new().unwrap();
    let material = builder.add_material(Material::default()).unwrap();
    let mesh = builder
        .add_mesh(Mesh {
            topology: PrimitiveTopology::Triangles,
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            material: Some(material),
            ..Mesh::default()
        })
        .unwrap();
    builder
        .add_child_node(
            builder.root(),
            Node {
                mesh_bindings: vec![MeshBinding::new(mesh)],
                ..Node::default()
            },
        )
        .unwrap();
    builder
}

fn valid_triangle_scene() -> ValidatedScene {
    validate_scene(valid_triangle_builder().finish_draft()).unwrap()
}

fn valid_polygon_builder() -> SceneBuilder {
    let mut builder = SceneBuilder::try_new().unwrap();
    let mesh = builder
        .add_mesh(Mesh {
            topology: PrimitiveTopology::Polygons,
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(1.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            indices: vec![0, 1, 2, 3],
            face_vertex_counts: vec![4],
            ..Mesh::default()
        })
        .unwrap();
    builder
        .add_child_node(
            builder.root(),
            Node {
                mesh_bindings: vec![MeshBinding::new(mesh)],
                ..Node::default()
            },
        )
        .unwrap();
    builder
}

fn assert_invalid(builder: SceneBuilder, expected: SceneValidationCode) {
    let issue = invalid_issue(builder);
    assert_eq!(issue.code(), expected);
}

fn invalid_issue(builder: SceneBuilder) -> baozi_core::SceneValidationIssue {
    let error = validate_scene(builder.finish_draft()).unwrap_err();
    let SceneValidationError::InvalidScene { issue } = error else {
        panic!("expected invalid scene, got {error:?}");
    };
    issue
}

#[test]
fn root_only_draft_is_published_as_read_only_scene() {
    let scene = validate_scene(SceneBuilder::try_new().unwrap().finish_draft()).unwrap();

    assert_eq!(scene.root(), NodeId::new(0));
    assert_eq!(scene.nodes().len(), 1);
    assert!(scene.meshes().is_empty());
    assert!(scene.metadata().is_empty());
    assert_eq!(scene.node(scene.root()), scene.nodes().first());
}

#[test]
fn every_successful_validation_mints_a_fresh_generation() {
    let first = validate_scene(SceneBuilder::try_new().unwrap().finish_draft()).unwrap();
    let second = validate_scene(SceneBuilder::try_new().unwrap().finish_draft()).unwrap();

    assert_ne!(first.generation(), second.generation());
}

#[test]
fn concurrent_validation_never_reuses_a_generation() {
    const SCENE_COUNT: usize = 64;

    let mut generations = std::thread::scope(|scope| {
        let handles = (0..SCENE_COUNT)
            .map(|_| {
                scope.spawn(|| {
                    validate_scene(SceneBuilder::try_new().unwrap().finish_draft())
                        .unwrap()
                        .generation()
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    generations.sort_unstable();
    generations.dedup();

    assert_eq!(generations.len(), SCENE_COUNT);
}

#[test]
fn invalid_draft_never_publishes_a_proof() {
    let mut builder = valid_triangle_builder();
    builder.mesh_mut(MeshId::new(0)).unwrap().indices = vec![0, 1, 99];

    let issue = invalid_issue(builder);
    assert_eq!(issue.code(), SceneValidationCode::OutOfRange);
    assert_eq!(
        issue.path(),
        SceneValidationPath::MeshElement {
            mesh: 0,
            element: 99,
        }
    );
    assert_eq!(
        issue.data(),
        SceneValidationData::Index {
            value: 99,
            upper_bound: 3,
        }
    );
    assert!(std::mem::size_of_val(&issue) <= 128);
}

#[test]
fn builder_rejects_an_out_of_range_parent() {
    let mut builder = SceneBuilder::try_new().unwrap();
    let error = builder
        .add_child_node(NodeId::new(99), Node::default())
        .unwrap_err();

    assert_eq!(
        error,
        SceneBuildError::ParentNodeOutOfRange {
            parent: NodeId::new(99),
            node_count: 1,
        }
    );
}

#[test]
fn deeply_nested_scene_is_validated_without_recursion() {
    const NODE_COUNT: usize = 100_000;

    let mut builder = SceneBuilder::try_new().unwrap();
    let mut parent = builder.root();
    for _ in 1..NODE_COUNT {
        parent = builder.add_child_node(parent, Node::default()).unwrap();
    }

    let scene = validate_scene(builder.finish_draft()).unwrap();
    assert_eq!(scene.nodes().len(), NODE_COUNT);
}

#[test]
fn duplicate_child_reference_is_rejected() {
    let mut builder = SceneBuilder::try_new().unwrap();
    let child = builder.add_node(Node::default()).unwrap();
    builder
        .node_mut(builder.root())
        .unwrap()
        .children
        .push(child);

    assert_invalid(builder, SceneValidationCode::DuplicateReachability);
}

#[test]
fn unreachable_node_is_rejected() {
    let mut builder = SceneBuilder::try_new().unwrap();
    builder.add_node(Node::default()).unwrap();
    builder.node_mut(builder.root()).unwrap().children.clear();

    assert_invalid(builder, SceneValidationCode::Unreachable);
}

#[test]
fn polygon_cardinality_and_topology_are_validated() {
    validate_scene(valid_polygon_builder().finish_draft()).unwrap();

    let mut missing_counts = valid_polygon_builder();
    missing_counts
        .mesh_mut(MeshId::new(0))
        .unwrap()
        .face_vertex_counts
        .clear();
    assert_invalid(missing_counts, SceneValidationCode::MissingRequiredData);

    let mut wrong_count = valid_polygon_builder();
    wrong_count
        .mesh_mut(MeshId::new(0))
        .unwrap()
        .face_vertex_counts = vec![3];
    assert_invalid(wrong_count, SceneValidationCode::LengthMismatch);

    let mut too_short = valid_polygon_builder();
    let mesh = too_short.mesh_mut(MeshId::new(0)).unwrap();
    mesh.indices = vec![0, 1];
    mesh.face_vertex_counts = vec![2];
    assert_invalid(too_short, SceneValidationCode::MinimumNotMet);

    let mut triangle_with_counts = valid_triangle_builder();
    triangle_with_counts
        .mesh_mut(MeshId::new(0))
        .unwrap()
        .face_vertex_counts = vec![3];
    assert_invalid(triangle_with_counts, SceneValidationCode::UnexpectedData);
}

#[test]
fn material_and_texture_references_are_validated() {
    let mut missing_material = valid_triangle_builder();
    missing_material.mesh_mut(MeshId::new(0)).unwrap().material = Some(MaterialId::new(99));
    assert_invalid(missing_material, SceneValidationCode::OutOfRange);

    let mut missing_slot_texture = valid_triangle_builder();
    missing_slot_texture
        .material_mut(MaterialId::new(0))
        .unwrap()
        .texture_slots
        .push(TextureSlot {
            texture: TextureId::new(99),
            role: TextureRole::Diffuse,
            color_space: ColorSpace::Srgb,
            uv_set: 0,
            scale: 1.0,
            transform: Default::default(),
            source_key: Some("map_Kd".to_owned()),
        });
    assert_invalid(missing_slot_texture, SceneValidationCode::OutOfRange);

    let mut missing_property_texture = valid_triangle_builder();
    missing_property_texture
        .material_mut(MaterialId::new(0))
        .unwrap()
        .properties
        .try_insert(
            "obj:diffuse_texture".to_owned(),
            MaterialProperty::Texture(TextureId::new(99)),
        )
        .unwrap();
    assert_invalid(missing_property_texture, SceneValidationCode::OutOfRange);

    let mut unnamespaced_property = valid_triangle_builder();
    unnamespaced_property
        .material_mut(MaterialId::new(0))
        .unwrap()
        .properties
        .try_insert("roughness".to_owned(), MaterialProperty::F64(1.0))
        .unwrap();
    assert!(matches!(
        validate_scene(unnamespaced_property.finish_draft()).unwrap_err(),
        SceneValidationError::InvalidMetadataKey { .. }
    ));
}

#[test]
fn a_valid_material_texture_reference_is_published() {
    let mut builder = SceneBuilder::try_new().unwrap();
    let texture = builder
        .add_texture(Texture {
            name: Some("diffuse".to_owned()),
            source: TextureSource::External {
                uri: "textures/diffuse.png".to_owned(),
            },
            sampler: Default::default(),
            metadata: Default::default(),
        })
        .unwrap();
    let material = builder
        .add_material(Material {
            texture_slots: vec![TextureSlot {
                texture,
                role: TextureRole::Diffuse,
                color_space: ColorSpace::Srgb,
                uv_set: 0,
                scale: 1.0,
                transform: Default::default(),
                source_key: Some("map_Kd".to_owned()),
            }],
            ..Material::default()
        })
        .unwrap();
    let mesh = builder
        .add_mesh(Mesh {
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            material: Some(material),
            ..Mesh::default()
        })
        .unwrap();
    builder
        .add_child_node(
            builder.root(),
            Node {
                mesh_bindings: vec![MeshBinding::new(mesh)],
                ..Node::default()
            },
        )
        .unwrap();

    let scene = validate_scene(builder.finish_draft()).unwrap();
    assert_eq!(
        scene.texture(texture).unwrap().name.as_deref(),
        Some("diffuse")
    );
}

#[test]
fn vertex_morph_and_joint_channels_are_validated() {
    let mut bad_attribute = valid_triangle_builder();
    bad_attribute
        .mesh_mut(MeshId::new(0))
        .unwrap()
        .custom_attributes
        .push(VertexAttribute {
            name: "ply:temperature".to_owned(),
            semantic: VertexAttributeSemantic::Custom("temperature".to_owned()),
            data: VertexAttributeData::F32(vec![1.0, 2.0]),
            metadata: Default::default(),
        });
    assert_invalid(bad_attribute, SceneValidationCode::LengthMismatch);

    let mut empty_morph = valid_triangle_builder();
    empty_morph
        .mesh_mut(MeshId::new(0))
        .unwrap()
        .morph_targets
        .push(MorphTarget::default());
    assert_invalid(empty_morph, SceneValidationCode::Empty);

    let mut mismatched_joint_channels = valid_triangle_builder();
    let mesh = mismatched_joint_channels.mesh_mut(MeshId::new(0)).unwrap();
    mesh.joint_indices = vec![[0, 1, 2, 3]];
    mesh.joint_weights = vec![[1.0, 0.0, 0.0, 0.0]];
    assert_invalid(
        mismatched_joint_channels,
        SceneValidationCode::LengthMismatch,
    );

    let mut no_skin = valid_triangle_builder();
    let mesh = no_skin.mesh_mut(MeshId::new(0)).unwrap();
    mesh.joint_indices = vec![[0, 0, 0, 0]; 3];
    mesh.joint_weights = vec![[1.0, 0.0, 0.0, 0.0]; 3];
    assert_invalid(no_skin, SceneValidationCode::RequiresSkin);
}

#[test]
fn skin_references_and_palette_indices_are_validated() {
    let mut builder = valid_triangle_builder();
    let skin = builder
        .add_skin(Skin {
            joints: vec![NodeId::new(1)],
            ..Skin::default()
        })
        .unwrap();
    builder.node_mut(NodeId::new(1)).unwrap().mesh_bindings[0].skin = Some(skin);
    let mesh = builder.mesh_mut(MeshId::new(0)).unwrap();
    mesh.joint_indices = vec![[0, 1, 0, 0]; 3];
    mesh.joint_weights = vec![[1.0, 0.0, 0.0, 0.0]; 3];
    assert_invalid(builder, SceneValidationCode::OutOfRange);

    let mut bad_joint = valid_triangle_builder();
    let skin = bad_joint
        .add_skin(Skin {
            joints: vec![NodeId::new(99)],
            ..Skin::default()
        })
        .unwrap();
    bad_joint.node_mut(NodeId::new(1)).unwrap().mesh_bindings[0].skin = Some(skin);
    assert_invalid(bad_joint, SceneValidationCode::OutOfRange);

    let mut bad_root = valid_triangle_builder();
    bad_root
        .add_skin(Skin {
            joints: vec![NodeId::new(1)],
            skeleton_root: Some(NodeId::new(99)),
            ..Skin::default()
        })
        .unwrap();
    assert_invalid(bad_root, SceneValidationCode::OutOfRange);

    let mut wrong_matrices = valid_triangle_builder();
    wrong_matrices
        .add_skin(Skin {
            joints: vec![NodeId::new(1)],
            inverse_bind_matrices: vec![baozi_core::Mat4::IDENTITY, baozi_core::Mat4::IDENTITY],
            ..Skin::default()
        })
        .unwrap();
    assert_invalid(wrong_matrices, SceneValidationCode::LengthMismatch);
}

#[test]
fn camera_light_and_animation_payloads_are_validated() {
    let mut bad_camera = valid_triangle_builder();
    bad_camera
        .add_camera(Camera {
            projection: CameraProjection::Perspective {
                yfov_radians: f32::NAN,
                aspect_ratio: None,
                znear: 0.1,
                zfar: Some(100.0),
            },
            ..Camera::default()
        })
        .unwrap();
    assert_invalid(bad_camera, SceneValidationCode::NonFinite);

    let mut bad_light = valid_triangle_builder();
    bad_light
        .add_light(Light {
            kind: LightKind::Point,
            intensity: -1.0,
            ..Light::default()
        })
        .unwrap();
    assert_invalid(bad_light, SceneValidationCode::Negative);

    let mut bad_target = valid_triangle_builder();
    bad_target
        .add_animation(Animation {
            channels: vec![AnimationChannel {
                target: AnimationTarget {
                    node: NodeId::new(99),
                    property: AnimationProperty::Translation,
                },
                interpolation: AnimationInterpolation::Linear,
                times_seconds: vec![0.0],
                values: AnimationValues::Translations(vec![Vec3::new(0.0, 0.0, 0.0)]),
                metadata: Default::default(),
            }],
            ..Animation::default()
        })
        .unwrap();
    assert_invalid(bad_target, SceneValidationCode::OutOfRange);

    let mut wrong_value_kind = valid_triangle_builder();
    wrong_value_kind
        .add_animation(Animation {
            channels: vec![AnimationChannel {
                target: AnimationTarget {
                    node: NodeId::new(0),
                    property: AnimationProperty::Translation,
                },
                interpolation: AnimationInterpolation::Linear,
                times_seconds: vec![0.0],
                values: AnimationValues::Rotations(vec![baozi_core::Vec4::new(0.0, 0.0, 0.0, 1.0)]),
                metadata: Default::default(),
            }],
            ..Animation::default()
        })
        .unwrap();
    assert_invalid(wrong_value_kind, SceneValidationCode::KindMismatch);
}

#[test]
fn graph_references_and_numeric_values_are_validated() {
    let mut bad_mesh = valid_triangle_builder();
    bad_mesh.node_mut(NodeId::new(1)).unwrap().mesh_bindings[0].mesh = MeshId::new(99);
    assert_invalid(bad_mesh, SceneValidationCode::OutOfRange);

    let mut bad_child = valid_triangle_builder();
    bad_child
        .node_mut(NodeId::new(0))
        .unwrap()
        .children
        .push(NodeId::new(99));
    assert_invalid(bad_child, SceneValidationCode::OutOfRange);

    let mut non_finite = valid_triangle_builder();
    non_finite.mesh_mut(MeshId::new(0)).unwrap().positions[1] = Vec3::new(f32::NAN, 0.0, 0.0);
    assert_invalid(non_finite, SceneValidationCode::NonFinite);

    let mut empty = valid_triangle_builder();
    empty.mesh_mut(MeshId::new(0)).unwrap().positions.clear();
    assert_invalid(empty, SceneValidationCode::Empty);
}

#[test]
fn successful_edit_publishes_a_fresh_generation_without_mutating_input() {
    let input = valid_triangle_scene();
    let input_generation = input.generation();
    let original = input.mesh(MeshId::new(0)).unwrap().positions[0];
    let replacement = Vec3::new(2.0, 3.0, 4.0);

    let mut edit = input.begin_edit().unwrap();
    assert_eq!(edit.base_generation(), input_generation);
    edit.mesh_mut(MeshId::new(0)).unwrap().positions[0] = replacement;
    let output = edit.commit().unwrap();

    assert_ne!(output.generation(), input_generation);
    assert_eq!(input.mesh(MeshId::new(0)).unwrap().positions[0], original);
    assert_eq!(
        output.mesh(MeshId::new(0)).unwrap().positions[0],
        replacement
    );
}

#[test]
fn sibling_edits_publish_distinct_generations() {
    let input = valid_triangle_scene();
    let first = input.begin_edit().unwrap().commit().unwrap();
    let second = input.begin_edit().unwrap().commit().unwrap();

    assert_ne!(first.generation(), input.generation());
    assert_ne!(second.generation(), input.generation());
    assert_ne!(first.generation(), second.generation());
}

#[test]
fn failed_edit_publishes_nothing_and_preserves_input_proof() {
    let input = valid_triangle_scene();
    let input_generation = input.generation();
    let input_mesh = input.mesh(MeshId::new(0)).unwrap().clone();

    let mut edit = input.begin_edit().unwrap();
    edit.mesh_mut(MeshId::new(0)).unwrap().positions.clear();
    let error = edit.commit().unwrap_err();

    assert!(matches!(error, SceneValidationError::InvalidScene { .. }));
    assert_eq!(input.generation(), input_generation);
    assert_eq!(input.mesh(MeshId::new(0)).unwrap(), &input_mesh);
}

#[test]
fn foreign_skin_id_is_rejected_before_publication() {
    let mut builder = valid_triangle_builder();
    builder.node_mut(NodeId::new(1)).unwrap().mesh_bindings[0].skin = Some(SkinId::new(99));

    assert_invalid(builder, SceneValidationCode::OutOfRange);
}

#[test]
fn controlled_validation_can_cancel_before_work_starts() {
    let mut control = TestControl::cancel_on(1);

    let error = validate_scene_with(
        SceneBuilder::try_new().unwrap().finish_draft(),
        SceneValidationLimits::default(),
        &mut control,
    )
    .unwrap_err();

    assert_eq!(control.polls, 1);
    assert_cancelled(&error);
    assert_eq!(control.live_bytes(), 0);
}

#[test]
fn controlled_validation_polls_during_a_deep_graph_walk() {
    const NODE_COUNT: usize = 2_000;
    const CANCEL_DURING_VALIDATION: usize = 30;

    let mut builder = SceneBuilder::try_new().unwrap();
    let mut parent = builder.root();
    for _ in 1..NODE_COUNT {
        parent = builder.add_child_node(parent, Node::default()).unwrap();
    }
    let mut control = TestControl::cancel_on(CANCEL_DURING_VALIDATION);

    let error = validate_scene_with(
        builder.finish_draft(),
        SceneValidationLimits::default(),
        &mut control,
    )
    .unwrap_err();

    assert_eq!(control.polls, CANCEL_DURING_VALIDATION);
    assert_cancelled(&error);
    assert_eq!(control.live_bytes(), 0);
}

#[test]
fn controlled_validation_charges_footprint_work_while_visiting_nested_channels() {
    const EMPTY_CHANNEL_COUNT: usize = SCENE_VALIDATION_POLL_INTERVAL as usize * 4;

    let mut builder = valid_triangle_builder();
    builder.mesh_mut(MeshId::new(0)).unwrap().texcoords =
        (0..EMPTY_CHANNEL_COUNT).map(|_| Vec::new()).collect();
    let mut control = TestControl::cancel_on(2);

    let error = validate_scene_with(
        builder.finish_draft(),
        SceneValidationLimits::default(),
        &mut control,
    )
    .unwrap_err();

    assert_cancelled(&error);
    assert!(
        control.work >= SCENE_VALIDATION_POLL_INTERVAL,
        "cancellation must be observed from work charged inside the footprint traversal"
    );
    assert!(control.work < EMPTY_CHANNEL_COUNT as u64);
}

#[test]
fn controlled_validation_polls_after_canonical_reservation_before_publication() {
    let mut successful_control = TestControl::new();
    let success = validate_scene_with(
        SceneBuilder::try_new().unwrap().finish_draft(),
        SceneValidationLimits::default(),
        &mut successful_control,
    )
    .unwrap();
    let final_poll = successful_control.polls;
    drop(success);
    assert_eq!(successful_control.live_bytes(), 0);

    let mut control = TestControl::cancel_on(final_poll);
    let error = validate_scene_with(
        SceneBuilder::try_new().unwrap().finish_draft(),
        SceneValidationLimits::default(),
        &mut control,
    )
    .unwrap_err();

    assert_eq!(control.polls, final_poll);
    assert!(control.events.iter().any(|event| matches!(
        event,
        ReservationEvent::Reserve {
            kind: SceneReservationKind::CanonicalOutput,
            ..
        }
    )));
    assert_cancelled(&error);
    assert_eq!(control.live_bytes(), 0);
}

#[test]
fn cancelled_edit_preserves_the_input_scene_and_generation() {
    let input = valid_triangle_scene();
    let input_generation = input.generation();
    let input_mesh = input.mesh(MeshId::new(0)).unwrap().clone();
    let mut control = TestControl::new();
    let mut edit = input
        .begin_edit_with(SceneValidationLimits::default(), &mut control)
        .unwrap();
    edit.mesh_mut(MeshId::new(0)).unwrap().positions[0] = Vec3::new(9.0, 8.0, 7.0);
    control.set_cancel_on(control.polls + 1);

    let error = edit
        .commit_with(SceneValidationLimits::default(), &mut control)
        .unwrap_err();

    assert_cancelled(&error);
    assert_eq!(input.generation(), input_generation);
    assert_eq!(input.mesh(MeshId::new(0)).unwrap(), &input_mesh);
    assert_eq!(control.live_bytes(), 0);
}

#[test]
fn validation_success_owns_exact_scene_evidence_and_a_live_reservation() {
    let builder = valid_triangle_builder();
    let measured_before_validation = builder.storage_footprint().unwrap();
    let mut control = TestControl::new();

    let success = validate_scene_with(
        builder.finish_draft(),
        SceneValidationLimits::default(),
        &mut control,
    )
    .unwrap();
    let stats = success.stats();

    assert_eq!(stats.footprint(), measured_before_validation);
    assert_eq!(stats.canonical_counts().nodes, 2);
    assert_eq!(stats.canonical_counts().edges, 1);
    assert_eq!(stats.canonical_counts().payload_cells, 3);
    assert_eq!(
        control.live_bytes(),
        stats.footprint().accounted_payload_bytes()
    );
    let scene = success
        .try_publish(|reservation| {
            assert_eq!(reservation.kind, SceneReservationKind::CanonicalOutput);
            assert_eq!(
                control.live_bytes(),
                stats.footprint().accounted_payload_bytes()
            );
            drop(reservation);
            Ok::<(), TestControlError>(())
        })
        .unwrap();
    assert_eq!(control.live_bytes(), 0);
    assert_eq!(scene.validation_stats(), stats);
    assert_eq!(scene.footprint(), stats.footprint());
}

#[test]
fn rejected_publication_releases_the_scene_reservation_without_returning_a_scene() {
    let mut control = TestControl::new();
    let success = validate_scene_with(
        valid_triangle_builder().finish_draft(),
        SceneValidationLimits::default(),
        &mut control,
    )
    .unwrap();

    let error = success
        .try_publish(|reservation| {
            drop(reservation);
            Err(TestControlError::PublicationRejected)
        })
        .unwrap_err();

    assert_eq!(error, TestControlError::PublicationRejected);
    assert_eq!(control.live_bytes(), 0);
}

#[test]
fn mesh_binding_scratch_remains_reserved_during_mesh_validation() {
    let mut control = TestControl::new();
    let success = validate_scene_with(
        valid_triangle_builder().finish_draft(),
        SceneValidationLimits::default(),
        &mut control,
    )
    .unwrap();

    assert!(control.verified_scratch_lifetime);
    success
        .try_publish(|reservation| {
            drop(reservation);
            Ok::<(), TestControlError>(())
        })
        .unwrap();
}

#[test]
fn validation_limits_bound_every_structural_dimension_before_publication() {
    let cases = [
        (
            SceneValidationLimits::default().with_max_nodes(1),
            SceneValidationLimit::Nodes,
        ),
        (
            SceneValidationLimits::default().with_max_edges(0),
            SceneValidationLimit::Edges,
        ),
        (
            SceneValidationLimits::default().with_max_depth(1),
            SceneValidationLimit::Depth,
        ),
        (
            SceneValidationLimits::default().with_max_payload_cells(2),
            SceneValidationLimit::PayloadCells,
        ),
        (
            SceneValidationLimits::default().with_max_work(1),
            SceneValidationLimit::Work,
        ),
        (
            SceneValidationLimits::default().with_max_scratch_bytes(0),
            SceneValidationLimit::ScratchBytes,
        ),
    ];

    for (limits, expected_limit) in cases {
        let mut control = TestControl::new();
        let error = validate_scene_with(
            valid_triangle_builder().finish_draft(),
            limits,
            &mut control,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            SceneValidationFailure::Scene(SceneValidationError::LimitExceeded { limit, .. })
                if limit == expected_limit
        ));
        assert_eq!(control.live_bytes(), 0);
    }
}

#[test]
fn metadata_keys_values_and_controls_are_closed_and_bounded() {
    let mut invalid_key = SceneBuilder::try_new().unwrap();
    invalid_key
        .metadata_mut()
        .try_insert(
            "Invalid:key".to_owned(),
            baozi_core::MetadataValue::Bool(true),
        )
        .unwrap();
    let error = validate_scene(invalid_key.finish_draft()).unwrap_err();
    assert!(matches!(
        error,
        SceneValidationError::InvalidMetadataKey {
            owner: SceneMetadataOwner::Scene,
            ..
        }
    ));

    let mut non_finite = SceneBuilder::try_new().unwrap();
    non_finite
        .metadata_mut()
        .try_insert(
            "test:value".to_owned(),
            baozi_core::MetadataValue::F64(f64::NAN),
        )
        .unwrap();
    assert!(matches!(
        validate_scene(non_finite.finish_draft()).unwrap_err(),
        SceneValidationError::InvalidMetadataValue { .. }
    ));

    let mut control_character = SceneBuilder::try_new().unwrap();
    control_character
        .metadata_mut()
        .try_insert(
            "test:value".to_owned(),
            baozi_core::MetadataValue::String("line\nbreak".to_owned()),
        )
        .unwrap();
    assert!(matches!(
        validate_scene(control_character.finish_draft()).unwrap_err(),
        SceneValidationError::InvalidMetadataValue { .. }
    ));

    let mut bounded = SceneBuilder::try_new().unwrap();
    bounded
        .metadata_mut()
        .try_insert(
            "test:value".to_owned(),
            baozi_core::MetadataValue::String("12345".to_owned()),
        )
        .unwrap();
    let mut control = TestControl::new();
    let error = validate_scene_with(
        bounded.finish_draft(),
        SceneValidationLimits::default().with_max_metadata_string_bytes(4),
        &mut control,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        SceneValidationFailure::Scene(SceneValidationError::InvalidMetadataValue { .. })
    ));
}

#[test]
fn every_nested_metadata_owner_is_checked_by_the_central_validator() {
    let mut builder = valid_triangle_builder();
    builder
        .node_mut(NodeId::new(1))
        .unwrap()
        .metadata
        .try_insert(
            "not-namespaced".to_owned(),
            baozi_core::MetadataValue::Bool(true),
        )
        .unwrap();

    let error = validate_scene(builder.finish_draft()).unwrap_err();
    assert!(matches!(
        error,
        SceneValidationError::InvalidMetadataKey {
            owner: SceneMetadataOwner::Node(1),
            ..
        }
    ));
}

#[test]
fn canonical_names_reject_control_characters_without_echoing_hostile_text() {
    let mut builder = valid_triangle_builder();
    builder.node_mut(NodeId::new(1)).unwrap().name = Some("bad\0name".to_owned());

    let error = validate_scene(builder.finish_draft()).unwrap_err();
    assert!(matches!(
        error,
        SceneValidationError::InvalidCanonicalString { .. }
    ));
    assert!(!error.to_string().contains("bad\0name"));
}

#[test]
fn controlled_clone_reserves_before_work_and_releases_on_cancellation() {
    const POSITION_COUNT: usize = SCENE_VALIDATION_POLL_INTERVAL as usize * 8;

    let mut builder = SceneBuilder::try_new().unwrap();
    builder
        .add_mesh(Mesh {
            topology: PrimitiveTopology::Points,
            positions: vec![Vec3::new(1.0, 2.0, 3.0); POSITION_COUNT],
            ..Mesh::default()
        })
        .unwrap();
    let input = validate_scene(builder.finish_draft()).unwrap();
    let mut control = TestControl::cancel_on(2);

    let error = input
        .begin_edit_with(SceneValidationLimits::default(), &mut control)
        .unwrap_err();

    assert!(matches!(
        control.events.first(),
        Some(ReservationEvent::Reserve {
            kind: SceneReservationKind::EditCandidate,
            ..
        })
    ));
    assert_cancelled(&error);
    assert!(
        control.work < POSITION_COUNT as u64,
        "the clone must charge and copy in bounded chunks rather than pre-charging the full vector"
    );
    assert_eq!(control.live_bytes(), 0);
}

#[test]
fn rejected_clone_resize_keeps_the_envelope_live_until_clone_drop() {
    let input = valid_triangle_scene();
    let mut control = TestControl::new();
    control.reject_resize = true;

    let error = input
        .begin_edit_with(SceneValidationLimits::default(), &mut control)
        .unwrap_err();

    assert!(matches!(
        error,
        SceneValidationFailure::Control(TestControlError::ResizeRejected)
    ));
    assert!(control.peak_live_bytes > 0);
    assert_eq!(control.live_bytes(), 0);
}

#[test]
fn successful_edit_transitions_one_candidate_reservation_without_double_counting() {
    let input = valid_triangle_scene();
    let mut control = TestControl::new();
    let mut edit = input
        .begin_edit_with(SceneValidationLimits::default(), &mut control)
        .unwrap();
    let (candidate_id, candidate_envelope) = control
        .events
        .iter()
        .find_map(|event| match event {
            ReservationEvent::Reserve {
                id,
                kind: SceneReservationKind::EditCandidate,
                bytes,
            } => Some((*id, *bytes)),
            _ => None,
        })
        .unwrap();
    let candidate_actual = control
        .events
        .iter()
        .find_map(|event| match event {
            ReservationEvent::Transition {
                id,
                from: SceneReservationKind::EditCandidate,
                to: SceneReservationKind::EditCandidate,
                required_bytes,
            } if *id == candidate_id => Some(*required_bytes),
            _ => None,
        })
        .unwrap();
    assert!(candidate_actual <= candidate_envelope);
    edit.mesh_mut(MeshId::new(0)).unwrap().positions[0] = Vec3::new(4.0, 5.0, 6.0);
    let candidate_peak = control.peak_live_bytes;

    let success = edit
        .commit_with(SceneValidationLimits::default(), &mut control)
        .unwrap();

    let candidate_reserve = control.events.iter().find_map(|event| match event {
        ReservationEvent::Reserve {
            id,
            kind: SceneReservationKind::EditCandidate,
            ..
        } => Some(*id),
        _ => None,
    });
    assert!(control.events.iter().any(|event| matches!(
        event,
        ReservationEvent::Transition {
            id,
            from: SceneReservationKind::EditCandidate,
            to: SceneReservationKind::CanonicalOutput,
            ..
        } if Some(*id) == candidate_reserve
    )));
    assert!(!control.events.iter().any(|event| matches!(
        event,
        ReservationEvent::Reserve {
            kind: SceneReservationKind::CanonicalOutput,
            ..
        }
    )));
    assert!(
        control.peak_live_bytes
            <= candidate_peak
                .checked_add(success.stats().scratch_peak_bytes())
                .unwrap()
    );
    success
        .try_publish(|reservation| {
            drop(reservation);
            Ok::<(), TestControlError>(())
        })
        .unwrap();
    assert_eq!(control.live_bytes(), 0);
}
