use crate::material::{Material, Texture};
use crate::math::{Aabb, Color, Mat4, Vec2, Vec3, Vec4};
use crate::ordered_map::{OrderedMapError, OrderedMapReplacementPlan, PreparedOrderedMapInsert};
use crate::scene_storage::{clone_scene_data_with, unwrap_uncontrolled};
use crate::validation_control::NoopSceneValidationControl;
use crate::{
    SceneBuildError, SceneBuildResource, SceneFootprint, SceneReservationKind,
    SceneValidationControl, SceneValidationError, SceneValidationFailure, SceneValidationLimits,
    SceneValidationStats, SceneValidationSuccess,
};
use baozi_operation::ReplacementCapacityPlan;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u32);

        impl $name {
            pub const fn new(index: u32) -> Self {
                Self(index)
            }

            pub const fn as_u32(self) -> u32 {
                self.0
            }

            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

id_type!(NodeId);
id_type!(MeshId);
id_type!(MaterialId);
id_type!(TextureId);
id_type!(AnimationId);
id_type!(CameraId);
id_type!(LightId);
id_type!(SkinId);

/// A dense collection owned by one canonical scene payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneCollection {
    Nodes,
    Meshes,
    Materials,
    Textures,
    Animations,
    Cameras,
    Lights,
    Skins,
}

impl std::fmt::Display for SceneCollection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Nodes => "nodes",
            Self::Meshes => "meshes",
            Self::Materials => "materials",
            Self::Textures => "textures",
            Self::Animations => "animations",
            Self::Cameras => "cameras",
            Self::Lights => "lights",
            Self::Skins => "skins",
        })
    }
}

/// A capacity plan bound to one concrete [`SceneBuilder`] resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneCapacityPlan {
    resource: SceneBuildResource,
    original_len: usize,
    original_capacity: usize,
    additional: usize,
    replacement: Option<ReplacementCapacityPlan>,
}

impl SceneCapacityPlan {
    /// Returns the complete candidate-allocation envelope admitted before allocator entry.
    pub fn envelope_delta_bytes(&self) -> Result<u64, SceneBuildError> {
        self.replacement.map_or(Ok(0), |plan| {
            plan.candidate_envelope_bytes()
                .map_err(|cause| SceneBuildError::CapacityPlan {
                    resource: self.resource,
                    cause,
                })
        })
    }

    pub const fn replaces_allocation(&self) -> bool {
        self.replacement.is_some()
    }
}

/// Coupled plans for inserting a node and linking it into its parent's child list.
#[derive(Debug)]
pub struct SceneNodeCapacityPlan {
    parent: NodeId,
    nodes: SceneCapacityPlan,
    parent_children: SceneCapacityPlan,
}

impl SceneNodeCapacityPlan {
    pub fn node_envelope_bytes(&self) -> Result<u64, SceneBuildError> {
        self.nodes.envelope_delta_bytes()
    }

    pub fn parent_children_envelope_bytes(&self) -> Result<u64, SceneBuildError> {
        self.parent_children.envelope_delta_bytes()
    }

    pub const fn replaces_nodes(&self) -> bool {
        self.nodes.replaces_allocation()
    }

    pub const fn replaces_parent_children(&self) -> bool {
        self.parent_children.replaces_allocation()
    }
}

/// A fully allocated collection insertion whose commit performs no allocation or callback.
pub struct PreparedSceneInsert<'owner, T, Id> {
    owner: &'owner mut Vec<T>,
    replacement: Option<Vec<T>>,
    value: Option<T>,
    id: Id,
    replacement_bytes: u64,
}

/// A compound node/parent-child insertion prepared without mutating the builder.
pub struct PreparedNodeInsert<'owner> {
    builder: &'owner mut SceneBuilder,
    node_replacement: Option<Vec<Node>>,
    child_replacement: Option<Vec<NodeId>>,
    node: Option<Node>,
    parent: NodeId,
    id: NodeId,
    node_replacement_bytes: u64,
    child_replacement_bytes: u64,
}

/// A fully allocated node mesh-binding insertion.
pub struct PreparedNodeMeshBindingInsert<'owner> {
    bindings: &'owner mut Vec<MeshBinding>,
    replacement: Option<Vec<MeshBinding>>,
    binding: Option<MeshBinding>,
    replacement_bytes: u64,
}

/// A full-allocation replacement plan for the scene metadata map.
pub struct SceneMetadataCapacityPlan {
    inner: OrderedMapReplacementPlan<String, MetadataValue>,
}

impl SceneMetadataCapacityPlan {
    pub fn envelope_bytes(&self) -> Result<u64, OrderedMapError> {
        self.inner.envelope_bytes()
    }

    pub const fn replaces_allocation(&self) -> bool {
        self.inner.replaces_allocation()
    }
}

/// A scene metadata insertion prepared without mutating the live map.
pub struct PreparedSceneMetadataInsert<'owner> {
    inner: PreparedOrderedMapInsert<'owner, String, MetadataValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveTopology {
    Points,
    Lines,
    Triangles,
    Polygons,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handedness {
    Right,
    Left,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneSpace {
    pub handedness: Handedness,
    pub up_axis: Option<Axis>,
    pub front_axis: Option<Axis>,
    pub unit_scale_to_meters: Option<f32>,
}

impl Default for SceneSpace {
    fn default() -> Self {
        Self {
            handedness: Handedness::Unknown,
            up_axis: None,
            front_axis: None,
            unit_scale_to_meters: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MetadataValue {
    Bool(bool),
    I64(i64),
    F64(f64),
    String(String),
}

pub type MetadataMap = crate::OrderedMap<String, MetadataValue>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshBinding {
    pub mesh: MeshId,
    pub skin: Option<SkinId>,
}

impl MeshBinding {
    pub fn new(mesh: MeshId) -> Self {
        Self { mesh, skin: None }
    }

    pub fn skinned(mesh: MeshId, skin: SkinId) -> Self {
        Self {
            mesh,
            skin: Some(skin),
        }
    }
}

impl From<MeshId> for MeshBinding {
    fn from(mesh: MeshId) -> Self {
        Self::new(mesh)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: Option<String>,
    pub transform: Mat4,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub mesh_bindings: Vec<MeshBinding>,
    pub camera: Option<CameraId>,
    pub light: Option<LightId>,
    pub metadata: MetadataMap,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            name: None,
            transform: Mat4::IDENTITY,
            parent: None,
            children: Vec::new(),
            mesh_bindings: Vec::new(),
            camera: None,
            light: None,
            metadata: MetadataMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VertexAttributeSemantic {
    Position,
    Normal,
    Tangent,
    Texcoord(u32),
    Color(u32),
    Joints(u32),
    Weights(u32),
    Custom(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum VertexAttributeData {
    F32(Vec<f32>),
    Vec2(Vec<Vec2>),
    Vec3(Vec<Vec3>),
    Vec4(Vec<Vec4>),
    U16x4(Vec<[u16; 4]>),
    U32(Vec<u32>),
    I32(Vec<i32>),
}

impl VertexAttributeData {
    pub fn len(&self) -> usize {
        match self {
            Self::F32(values) => values.len(),
            Self::Vec2(values) => values.len(),
            Self::Vec3(values) => values.len(),
            Self::Vec4(values) => values.len(),
            Self::U16x4(values) => values.len(),
            Self::U32(values) => values.len(),
            Self::I32(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VertexAttribute {
    pub name: String,
    pub semantic: VertexAttributeSemantic,
    pub data: VertexAttributeData,
    pub metadata: MetadataMap,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MorphTarget {
    pub name: Option<String>,
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub tangents: Vec<Vec4>,
    pub metadata: MetadataMap,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub name: Option<String>,
    pub topology: PrimitiveTopology,
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub tangents: Vec<Vec4>,
    pub texcoords: Vec<Vec<Vec2>>,
    pub colors: Vec<Vec<Color>>,
    pub indices: Vec<u32>,
    pub face_vertex_counts: Vec<u32>,
    pub material: Option<MaterialId>,
    pub joint_indices: Vec<[u16; 4]>,
    pub joint_weights: Vec<[f32; 4]>,
    pub morph_targets: Vec<MorphTarget>,
    pub custom_attributes: Vec<VertexAttribute>,
    pub bounds: Option<Aabb>,
    pub metadata: MetadataMap,
}

impl Mesh {
    pub fn element_count(&self) -> usize {
        if self.indices.is_empty() {
            self.positions.len()
        } else {
            self.indices.len()
        }
    }

    pub fn polygon_face_count(&self) -> Option<usize> {
        (self.topology == PrimitiveTopology::Polygons).then_some(self.face_vertex_counts.len())
    }
}

impl Default for Mesh {
    fn default() -> Self {
        Self {
            name: None,
            topology: PrimitiveTopology::Triangles,
            positions: Vec::new(),
            normals: Vec::new(),
            tangents: Vec::new(),
            texcoords: Vec::new(),
            colors: Vec::new(),
            indices: Vec::new(),
            face_vertex_counts: Vec::new(),
            material: None,
            joint_indices: Vec::new(),
            joint_weights: Vec::new(),
            morph_targets: Vec::new(),
            custom_attributes: Vec::new(),
            bounds: None,
            metadata: MetadataMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationInterpolation {
    Step,
    Linear,
    CubicSpline,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationProperty {
    Translation,
    Rotation,
    Scale,
    MorphWeights,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnimationTarget {
    pub node: NodeId,
    pub property: AnimationProperty,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnimationValues {
    Translations(Vec<Vec3>),
    Rotations(Vec<Vec4>),
    Scales(Vec<Vec3>),
    MorphWeights {
        values: Vec<f32>,
        weights_per_keyframe: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimationChannel {
    pub target: AnimationTarget,
    pub interpolation: AnimationInterpolation,
    pub times_seconds: Vec<f32>,
    pub values: AnimationValues,
    pub metadata: MetadataMap,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Animation {
    pub name: Option<String>,
    pub channels: Vec<AnimationChannel>,
    pub metadata: MetadataMap,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum CameraProjection {
    Perspective {
        yfov_radians: f32,
        aspect_ratio: Option<f32>,
        znear: f32,
        zfar: Option<f32>,
    },
    Orthographic {
        xmag: f32,
        ymag: f32,
        znear: f32,
        zfar: f32,
    },
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Camera {
    pub name: Option<String>,
    pub projection: CameraProjection,
    pub metadata: MetadataMap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
    Area,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Light {
    pub name: Option<String>,
    pub kind: LightKind,
    pub color: Color,
    pub intensity: f32,
    pub range: Option<f32>,
    pub inner_cone_angle: Option<f32>,
    pub outer_cone_angle: Option<f32>,
    pub metadata: MetadataMap,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Skin {
    pub name: Option<String>,
    pub joints: Vec<NodeId>,
    pub inverse_bind_matrices: Vec<Mat4>,
    pub skeleton_root: Option<NodeId>,
    pub metadata: MetadataMap,
}

#[derive(Debug, PartialEq)]
pub(crate) struct SceneData {
    pub(crate) root: NodeId,
    pub(crate) nodes: Vec<Node>,
    pub(crate) meshes: Vec<Mesh>,
    pub(crate) materials: Vec<Material>,
    pub(crate) textures: Vec<Texture>,
    pub(crate) animations: Vec<Animation>,
    pub(crate) cameras: Vec<Camera>,
    pub(crate) lights: Vec<Light>,
    pub(crate) skins: Vec<Skin>,
    pub(crate) metadata: MetadataMap,
    pub(crate) space: SceneSpace,
}

/// An opaque, unvalidated scene produced by [`SceneBuilder`].
///
/// A draft cannot be inspected as a published scene or converted into a
/// [`ValidatedScene`] without passing through [`crate::validate_scene`].
///
/// ```compile_fail
/// let draft = baozi_core::SceneBuilder::try_new().unwrap().finish_draft();
/// let _ = draft.data;
/// ```
#[derive(Debug)]
pub struct SceneDraft {
    pub(crate) data: SceneData,
    #[cfg(test)]
    drop_probe: Option<SceneDraftDropProbe>,
}

#[cfg(test)]
#[derive(Debug)]
struct SceneDraftDropProbe(std::sync::Arc<std::sync::atomic::AtomicU8>);

#[cfg(test)]
impl Drop for SceneDraftDropProbe {
    fn drop(&mut self) {
        assert_eq!(
            self.0.swap(1, Ordering::SeqCst),
            0,
            "the scene draft must be dropped before its reservation"
        );
    }
}

/// The runtime identity of one validated scene generation.
///
/// Values are intentionally opaque. Callers compare generation identities but
/// cannot manufacture them or treat them as persistent asset identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SceneGenerationId(u64);

static NEXT_SCENE_GENERATION: AtomicU64 = AtomicU64::new(1);

/// A read-only scene paired with an unforgeable central-validation proof.
///
/// The type has no public constructor and exposes no mutable access to its
/// payload. Use [`ValidatedScene::begin_edit`] for transactional changes.
///
/// ```compile_fail
/// let draft = baozi_core::SceneBuilder::try_new().unwrap().finish_draft();
/// let scene = baozi_core::validate_scene(draft).unwrap();
/// scene.nodes()[0].children.clear();
/// ```
///
/// ```compile_fail
/// let draft = baozi_core::SceneBuilder::try_new().unwrap().finish_draft();
/// let scene = baozi_core::validate_scene(draft).unwrap();
/// let _forged = baozi_core::ValidatedScene { ..scene };
/// ```
///
/// ```compile_fail
/// let draft = baozi_core::SceneBuilder::try_new().unwrap().finish_draft();
/// let scene = baozi_core::validate_scene(draft).unwrap();
/// let _implicit_allocation = scene.clone();
/// ```
#[derive(Debug, PartialEq)]
pub struct ValidatedScene {
    generation: SceneGenerationId,
    data: SceneData,
    validation_stats: SceneValidationStats,
    #[cfg(test)]
    drop_probe: Option<ValidatedSceneDropProbe>,
}

#[cfg(test)]
#[derive(Debug)]
struct ValidatedSceneDropProbe(std::sync::Arc<std::sync::atomic::AtomicU8>);

#[cfg(test)]
impl PartialEq for ValidatedSceneDropProbe {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

#[cfg(test)]
impl Drop for ValidatedSceneDropProbe {
    fn drop(&mut self) {
        self.0.store(1, Ordering::SeqCst);
    }
}

impl ValidatedScene {
    fn from_validated_data(
        data: SceneData,
        validation_stats: SceneValidationStats,
    ) -> Result<Self, SceneValidationError> {
        let generation = NEXT_SCENE_GENERATION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map(SceneGenerationId)
            .map_err(|_| SceneValidationError::GenerationSpaceExhausted)?;
        Ok(Self {
            generation,
            data,
            validation_stats,
            #[cfg(test)]
            drop_probe: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn install_drop_probe(
        &mut self,
        state: std::sync::Arc<std::sync::atomic::AtomicU8>,
    ) {
        self.drop_probe = Some(ValidatedSceneDropProbe(state));
    }

    pub const fn generation(&self) -> SceneGenerationId {
        self.generation
    }

    pub fn root(&self) -> NodeId {
        self.data.root
    }

    pub fn nodes(&self) -> &[Node] {
        &self.data.nodes
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.data.nodes.get(id.index())
    }

    pub fn meshes(&self) -> &[Mesh] {
        &self.data.meshes
    }

    pub fn mesh(&self, id: MeshId) -> Option<&Mesh> {
        self.data.meshes.get(id.index())
    }

    pub fn materials(&self) -> &[Material] {
        &self.data.materials
    }

    pub fn material(&self, id: MaterialId) -> Option<&Material> {
        self.data.materials.get(id.index())
    }

    pub fn textures(&self) -> &[Texture] {
        &self.data.textures
    }

    pub fn texture(&self, id: TextureId) -> Option<&Texture> {
        self.data.textures.get(id.index())
    }

    pub fn animations(&self) -> &[Animation] {
        &self.data.animations
    }

    pub fn animation(&self, id: AnimationId) -> Option<&Animation> {
        self.data.animations.get(id.index())
    }

    pub fn cameras(&self) -> &[Camera] {
        &self.data.cameras
    }

    pub fn camera(&self, id: CameraId) -> Option<&Camera> {
        self.data.cameras.get(id.index())
    }

    pub fn lights(&self) -> &[Light] {
        &self.data.lights
    }

    pub fn light(&self, id: LightId) -> Option<&Light> {
        self.data.lights.get(id.index())
    }

    pub fn skins(&self) -> &[Skin] {
        &self.data.skins
    }

    pub fn skin(&self, id: SkinId) -> Option<&Skin> {
        self.data.skins.get(id.index())
    }

    pub fn metadata(&self) -> &MetadataMap {
        &self.data.metadata
    }

    pub fn space(&self) -> SceneSpace {
        self.data.space
    }

    /// Returns the immutable evidence produced when this generation was minted.
    pub const fn validation_stats(&self) -> SceneValidationStats {
        self.validation_stats
    }

    pub const fn footprint(&self) -> SceneFootprint {
        self.validation_stats.footprint()
    }

    /// Creates an isolated edit candidate through the default bounded clone path.
    pub fn begin_edit(&self) -> Result<SceneEdit, SceneValidationError> {
        let mut control = NoopSceneValidationControl;
        unwrap_uncontrolled(self.begin_edit_with(SceneValidationLimits::default(), &mut control))
    }

    /// Reserves the complete candidate before any deep-clone allocation.
    pub fn begin_edit_with<C: SceneValidationControl>(
        &self,
        limits: SceneValidationLimits,
        control: &mut C,
    ) -> Result<SceneEdit<C::Reservation>, SceneValidationFailure<C::Error>> {
        let (data, reservation) =
            clone_scene_data_with(&self.data, self.footprint(), limits, control)?;
        Ok(SceneEdit {
            base_generation: self.generation,
            data,
            reservation,
        })
    }
}

/// Validates every core invariant and publishes a new read-only scene generation.
///
/// This is the no-control entry to the central validation path from
/// [`SceneDraft`] to [`ValidatedScene`].
pub fn validate_scene(scene: SceneDraft) -> Result<ValidatedScene, SceneValidationError> {
    let mut control = NoopSceneValidationControl;
    unwrap_uncontrolled(validate_scene_with(
        scene,
        SceneValidationLimits::default(),
        &mut control,
    ))
    .map(SceneValidationSuccess::into_unreserved_scene)
}

/// Validates a draft with owner-neutral limits, work charging, and reservations.
///
/// The returned reservation remains live for as long as the owning operation
/// retains the validated result. An adapter may transfer its existing builder
/// reservation from inside `reserve` instead of reserving the same bytes twice.
pub fn validate_scene_with<C: SceneValidationControl>(
    scene: SceneDraft,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<SceneValidationSuccess<C::Reservation>, SceneValidationFailure<C::Error>> {
    let stats = crate::validation::validate_scene_data(&scene.data, limits, control)?;
    let reservation = control
        .reserve(
            SceneReservationKind::CanonicalOutput,
            stats.footprint().accounted_payload_bytes(),
        )
        .map_err(SceneValidationFailure::Control)?;
    let reserved = ReservedSceneDraft { scene, reservation };
    control.poll().map_err(SceneValidationFailure::Control)?;
    reserved
        .into_success(stats)
        .map_err(SceneValidationFailure::Scene)
}

struct ReservedSceneDraft<R> {
    scene: SceneDraft,
    reservation: R,
}

impl<R> ReservedSceneDraft<R> {
    fn into_success(
        self,
        stats: SceneValidationStats,
    ) -> Result<SceneValidationSuccess<R>, SceneValidationError> {
        let Self { scene, reservation } = self;
        let scene = ValidatedScene::from_validated_data(scene.data, stats)?;
        Ok(SceneValidationSuccess::new(scene, reservation))
    }
}

/// An isolated mutable candidate derived from a validated scene.
///
/// Mutations are never visible through the input scene. [`SceneEdit::commit`]
/// publishes a fresh generation only after complete central validation.
#[must_use = "dropping an edit releases its candidate reservation without publishing"]
pub struct SceneEdit<R = ()> {
    base_generation: SceneGenerationId,
    data: SceneData,
    reservation: R,
}

impl<R> fmt::Debug for SceneEdit<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SceneEdit")
            .field("base_generation", &self.base_generation)
            .field("data", &self.data)
            .field("reservation", &"<reservation>")
            .finish()
    }
}

impl<R> SceneEdit<R> {
    pub const fn base_generation(&self) -> SceneGenerationId {
        self.base_generation
    }

    pub fn root(&self) -> NodeId {
        self.data.root
    }

    pub fn nodes(&self) -> &[Node] {
        &self.data.nodes
    }

    pub fn nodes_mut(&mut self) -> &mut [Node] {
        &mut self.data.nodes
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.data.nodes.get_mut(id.index())
    }

    pub fn meshes(&self) -> &[Mesh] {
        &self.data.meshes
    }

    pub fn meshes_mut(&mut self) -> &mut [Mesh] {
        &mut self.data.meshes
    }

    pub fn mesh_mut(&mut self, id: MeshId) -> Option<&mut Mesh> {
        self.data.meshes.get_mut(id.index())
    }

    pub fn materials_mut(&mut self) -> &mut [Material] {
        &mut self.data.materials
    }

    pub fn textures_mut(&mut self) -> &mut [Texture] {
        &mut self.data.textures
    }

    pub fn animations_mut(&mut self) -> &mut [Animation] {
        &mut self.data.animations
    }

    pub fn cameras_mut(&mut self) -> &mut [Camera] {
        &mut self.data.cameras
    }

    pub fn lights_mut(&mut self) -> &mut [Light] {
        &mut self.data.lights
    }

    pub fn skins_mut(&mut self) -> &mut [Skin] {
        &mut self.data.skins
    }

    pub fn metadata_mut(&mut self) -> &mut MetadataMap {
        &mut self.data.metadata
    }

    pub fn space_mut(&mut self) -> &mut SceneSpace {
        &mut self.data.space
    }

    /// Validates the candidate and transfers its live reservation to canonical output.
    pub fn commit_with<C>(
        self,
        limits: SceneValidationLimits,
        control: &mut C,
    ) -> Result<SceneValidationSuccess<C::Reservation>, SceneValidationFailure<C::Error>>
    where
        C: SceneValidationControl<Reservation = R>,
    {
        control
            .validate_reservation(&self.reservation, SceneReservationKind::EditCandidate)
            .map_err(SceneValidationFailure::Control)?;
        let mut edit = self;
        let stats = crate::validation::validate_scene_data(&edit.data, limits, control)?;
        control
            .transition_reservation(
                &mut edit.reservation,
                SceneReservationKind::EditCandidate,
                SceneReservationKind::CanonicalOutput,
                stats.footprint().accounted_payload_bytes(),
            )
            .map_err(SceneValidationFailure::Control)?;
        control.poll().map_err(SceneValidationFailure::Control)?;
        let Self {
            base_generation: _,
            data,
            reservation,
        } = edit;
        let scene = ValidatedScene::from_validated_data(data, stats)?;
        Ok(SceneValidationSuccess::new(scene, reservation))
    }
}

impl SceneEdit {
    pub fn commit(self) -> Result<ValidatedScene, SceneValidationError> {
        let mut control = NoopSceneValidationControl;
        unwrap_uncontrolled(self.commit_with(SceneValidationLimits::default(), &mut control))
            .map(SceneValidationSuccess::into_unreserved_scene)
    }
}

#[derive(Debug)]
pub struct SceneBuilder {
    pub(crate) data: SceneData,
}

impl SceneBuilder {
    /// Plans the root-node allocation for operation-ledger admission.
    pub fn initial_capacity_plan() -> Result<SceneCapacityPlan, SceneBuildError> {
        capacity_plan(&Vec::<Node>::new(), 1, SceneBuildResource::Nodes)
    }

    /// Creates the canonical root through the shared fallible capacity policy.
    pub fn try_new() -> Result<Self, SceneBuildError> {
        let plan = Self::initial_capacity_plan()?;
        Self::try_new_with_capacity_plan(plan)
    }

    /// Creates the root after a caller has admitted the supplied envelope.
    pub fn try_new_with_capacity_plan(plan: SceneCapacityPlan) -> Result<Self, SceneBuildError> {
        let root = NodeId::new(0);
        let mut nodes = Vec::new();
        let (replacement, _) = allocate_replacement(&mut nodes, plan, SceneBuildResource::Nodes)?;
        if let Some(replacement) = replacement {
            nodes = replacement;
        }
        if nodes.capacity() == 0 {
            return Err(SceneBuildError::CapacityPlan {
                resource: SceneBuildResource::Nodes,
                cause: baozi_operation::CapacityPlanError::InvalidState {
                    len: 1,
                    capacity: 0,
                },
            });
        }
        nodes.push(Node::default());
        Ok(Self {
            data: SceneData {
                root,
                nodes,
                meshes: Vec::new(),
                materials: Vec::new(),
                textures: Vec::new(),
                animations: Vec::new(),
                cameras: Vec::new(),
                lights: Vec::new(),
                skins: Vec::new(),
                metadata: MetadataMap::new(),
                space: SceneSpace::default(),
            },
        })
    }

    pub fn root(&self) -> NodeId {
        self.data.root
    }

    pub fn mesh_count(&self) -> usize {
        self.data.meshes.len()
    }

    pub fn node_count(&self) -> usize {
        self.data.nodes.len()
    }

    /// Plans a deterministic capacity envelope for one scene collection.
    pub fn collection_capacity_plan(
        &self,
        collection: SceneCollection,
        additional: usize,
    ) -> Result<SceneCapacityPlan, SceneBuildError> {
        match collection {
            SceneCollection::Nodes => {
                capacity_plan(&self.data.nodes, additional, SceneBuildResource::Nodes)
            }
            SceneCollection::Meshes => {
                capacity_plan(&self.data.meshes, additional, SceneBuildResource::Meshes)
            }
            SceneCollection::Materials => capacity_plan(
                &self.data.materials,
                additional,
                SceneBuildResource::Materials,
            ),
            SceneCollection::Textures => capacity_plan(
                &self.data.textures,
                additional,
                SceneBuildResource::Textures,
            ),
            SceneCollection::Animations => capacity_plan(
                &self.data.animations,
                additional,
                SceneBuildResource::Animations,
            ),
            SceneCollection::Cameras => {
                capacity_plan(&self.data.cameras, additional, SceneBuildResource::Cameras)
            }
            SceneCollection::Lights => {
                capacity_plan(&self.data.lights, additional, SceneBuildResource::Lights)
            }
            SceneCollection::Skins => {
                capacity_plan(&self.data.skins, additional, SceneBuildResource::Skins)
            }
        }
    }

    pub fn node_children_capacity_plan(
        &self,
        parent: NodeId,
        additional: usize,
    ) -> Result<SceneCapacityPlan, SceneBuildError> {
        let node =
            self.data
                .nodes
                .get(parent.index())
                .ok_or(SceneBuildError::ParentNodeOutOfRange {
                    parent,
                    node_count: self.data.nodes.len(),
                })?;
        capacity_plan(&node.children, additional, SceneBuildResource::NodeChildren)
    }

    pub fn node_insert_capacity_plan(
        &self,
        parent: NodeId,
    ) -> Result<SceneNodeCapacityPlan, SceneBuildError> {
        Ok(SceneNodeCapacityPlan {
            parent,
            nodes: self.collection_capacity_plan(SceneCollection::Nodes, 1)?,
            parent_children: self.node_children_capacity_plan(parent, 1)?,
        })
    }

    pub fn add_node(&mut self, node: Node) -> Result<NodeId, SceneBuildError> {
        self.add_child_node(self.data.root, node)
    }

    pub fn add_child_node(
        &mut self,
        parent: NodeId,
        node: Node,
    ) -> Result<NodeId, SceneBuildError> {
        let plan = self.node_insert_capacity_plan(parent)?;
        self.prepare_add_child_node(parent, node, plan)
            .map(PreparedNodeInsert::commit)
    }

    pub fn prepare_add_child_node(
        &mut self,
        parent: NodeId,
        mut node: Node,
        plan: SceneNodeCapacityPlan,
    ) -> Result<PreparedNodeInsert<'_>, SceneBuildError> {
        if parent != plan.parent || parent.index() >= self.data.nodes.len() {
            return Err(SceneBuildError::ParentNodeOutOfRange {
                parent,
                node_count: self.data.nodes.len(),
            });
        }
        if !node.children.is_empty() {
            return Err(SceneBuildError::InsertedNodeHasChildren);
        }
        let id = next_id(self.data.nodes.len(), SceneCollection::Nodes).map(NodeId::new)?;
        node.parent = Some(parent);
        let (node_replacement, node_replacement_bytes) =
            allocate_replacement(&mut self.data.nodes, plan.nodes, SceneBuildResource::Nodes)?;
        ensure_insertion_slot(
            &self.data.nodes,
            node_replacement.as_ref(),
            SceneBuildResource::Nodes,
        )?;
        let parent_node = self.data.nodes.get_mut(parent.index()).ok_or(
            SceneBuildError::ParentNodeOutOfRange {
                parent,
                node_count: id.index(),
            },
        )?;
        let (child_replacement, child_replacement_bytes) = allocate_replacement(
            &mut parent_node.children,
            plan.parent_children,
            SceneBuildResource::NodeChildren,
        )?;
        ensure_insertion_slot(
            &parent_node.children,
            child_replacement.as_ref(),
            SceneBuildResource::NodeChildren,
        )?;
        Ok(PreparedNodeInsert {
            builder: self,
            node_replacement,
            child_replacement,
            node: Some(node),
            parent,
            id,
            node_replacement_bytes,
            child_replacement_bytes,
        })
    }

    pub fn add_mesh(&mut self, mesh: Mesh) -> Result<MeshId, SceneBuildError> {
        let plan = self.collection_capacity_plan(SceneCollection::Meshes, 1)?;
        self.prepare_add_mesh(mesh, plan)
            .map(PreparedSceneInsert::commit)
    }

    pub fn prepare_add_mesh(
        &mut self,
        mesh: Mesh,
        plan: SceneCapacityPlan,
    ) -> Result<PreparedSceneInsert<'_, Mesh, MeshId>, SceneBuildError> {
        let id = next_id(self.data.meshes.len(), SceneCollection::Meshes).map(MeshId::new)?;
        prepare_insert(
            &mut self.data.meshes,
            mesh,
            id,
            plan,
            SceneBuildResource::Meshes,
        )
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.data.nodes.get_mut(id.index())
    }

    pub fn node_mesh_binding_capacity_plan(
        &self,
        id: NodeId,
        additional: usize,
    ) -> Result<SceneCapacityPlan, SceneBuildError> {
        let node =
            self.data
                .nodes
                .get(id.index())
                .ok_or(SceneBuildError::ParentNodeOutOfRange {
                    parent: id,
                    node_count: self.data.nodes.len(),
                })?;
        capacity_plan(
            &node.mesh_bindings,
            additional,
            SceneBuildResource::NodeMeshBindings,
        )
    }

    pub fn prepare_add_node_mesh_binding(
        &mut self,
        id: NodeId,
        binding: MeshBinding,
        plan: SceneCapacityPlan,
    ) -> Result<PreparedNodeMeshBindingInsert<'_>, SceneBuildError> {
        let node_count = self.data.nodes.len();
        let bindings = &mut self
            .data
            .nodes
            .get_mut(id.index())
            .ok_or(SceneBuildError::ParentNodeOutOfRange {
                parent: id,
                node_count,
            })?
            .mesh_bindings;
        let (replacement, replacement_bytes) =
            allocate_replacement(bindings, plan, SceneBuildResource::NodeMeshBindings)?;
        ensure_insertion_slot(
            bindings,
            replacement.as_ref(),
            SceneBuildResource::NodeMeshBindings,
        )?;
        Ok(PreparedNodeMeshBindingInsert {
            bindings,
            replacement,
            binding: Some(binding),
            replacement_bytes,
        })
    }

    pub fn add_node_mesh_binding(
        &mut self,
        id: NodeId,
        binding: MeshBinding,
    ) -> Result<(), SceneBuildError> {
        let plan = self.node_mesh_binding_capacity_plan(id, 1)?;
        self.prepare_add_node_mesh_binding(id, binding, plan)
            .map(PreparedNodeMeshBindingInsert::commit)
    }

    pub fn mesh_mut(&mut self, id: MeshId) -> Option<&mut Mesh> {
        self.data.meshes.get_mut(id.index())
    }

    pub fn add_material(&mut self, material: Material) -> Result<MaterialId, SceneBuildError> {
        let plan = self.collection_capacity_plan(SceneCollection::Materials, 1)?;
        self.prepare_add_material(material, plan)
            .map(PreparedSceneInsert::commit)
    }

    pub fn prepare_add_material(
        &mut self,
        material: Material,
        plan: SceneCapacityPlan,
    ) -> Result<PreparedSceneInsert<'_, Material, MaterialId>, SceneBuildError> {
        let id =
            next_id(self.data.materials.len(), SceneCollection::Materials).map(MaterialId::new)?;
        prepare_insert(
            &mut self.data.materials,
            material,
            id,
            plan,
            SceneBuildResource::Materials,
        )
    }

    pub fn material_mut(&mut self, id: MaterialId) -> Option<&mut Material> {
        self.data.materials.get_mut(id.index())
    }

    pub fn add_texture(&mut self, texture: Texture) -> Result<TextureId, SceneBuildError> {
        let plan = self.collection_capacity_plan(SceneCollection::Textures, 1)?;
        self.prepare_add_texture(texture, plan)
            .map(PreparedSceneInsert::commit)
    }

    pub fn prepare_add_texture(
        &mut self,
        texture: Texture,
        plan: SceneCapacityPlan,
    ) -> Result<PreparedSceneInsert<'_, Texture, TextureId>, SceneBuildError> {
        let id =
            next_id(self.data.textures.len(), SceneCollection::Textures).map(TextureId::new)?;
        prepare_insert(
            &mut self.data.textures,
            texture,
            id,
            plan,
            SceneBuildResource::Textures,
        )
    }

    pub fn texture_mut(&mut self, id: TextureId) -> Option<&mut Texture> {
        self.data.textures.get_mut(id.index())
    }

    pub fn add_animation(&mut self, animation: Animation) -> Result<AnimationId, SceneBuildError> {
        let plan = self.collection_capacity_plan(SceneCollection::Animations, 1)?;
        self.prepare_add_animation(animation, plan)
            .map(PreparedSceneInsert::commit)
    }

    pub fn prepare_add_animation(
        &mut self,
        animation: Animation,
        plan: SceneCapacityPlan,
    ) -> Result<PreparedSceneInsert<'_, Animation, AnimationId>, SceneBuildError> {
        let id = next_id(self.data.animations.len(), SceneCollection::Animations)
            .map(AnimationId::new)?;
        prepare_insert(
            &mut self.data.animations,
            animation,
            id,
            plan,
            SceneBuildResource::Animations,
        )
    }

    pub fn add_camera(&mut self, camera: Camera) -> Result<CameraId, SceneBuildError> {
        let plan = self.collection_capacity_plan(SceneCollection::Cameras, 1)?;
        self.prepare_add_camera(camera, plan)
            .map(PreparedSceneInsert::commit)
    }

    pub fn prepare_add_camera(
        &mut self,
        camera: Camera,
        plan: SceneCapacityPlan,
    ) -> Result<PreparedSceneInsert<'_, Camera, CameraId>, SceneBuildError> {
        let id = next_id(self.data.cameras.len(), SceneCollection::Cameras).map(CameraId::new)?;
        prepare_insert(
            &mut self.data.cameras,
            camera,
            id,
            plan,
            SceneBuildResource::Cameras,
        )
    }

    pub fn add_light(&mut self, light: Light) -> Result<LightId, SceneBuildError> {
        let plan = self.collection_capacity_plan(SceneCollection::Lights, 1)?;
        self.prepare_add_light(light, plan)
            .map(PreparedSceneInsert::commit)
    }

    pub fn prepare_add_light(
        &mut self,
        light: Light,
        plan: SceneCapacityPlan,
    ) -> Result<PreparedSceneInsert<'_, Light, LightId>, SceneBuildError> {
        let id = next_id(self.data.lights.len(), SceneCollection::Lights).map(LightId::new)?;
        prepare_insert(
            &mut self.data.lights,
            light,
            id,
            plan,
            SceneBuildResource::Lights,
        )
    }

    pub fn add_skin(&mut self, skin: Skin) -> Result<SkinId, SceneBuildError> {
        let plan = self.collection_capacity_plan(SceneCollection::Skins, 1)?;
        self.prepare_add_skin(skin, plan)
            .map(PreparedSceneInsert::commit)
    }

    pub fn prepare_add_skin(
        &mut self,
        skin: Skin,
        plan: SceneCapacityPlan,
    ) -> Result<PreparedSceneInsert<'_, Skin, SkinId>, SceneBuildError> {
        let id = next_id(self.data.skins.len(), SceneCollection::Skins).map(SkinId::new)?;
        prepare_insert(
            &mut self.data.skins,
            skin,
            id,
            plan,
            SceneBuildResource::Skins,
        )
    }

    pub fn metadata_mut(&mut self) -> &mut MetadataMap {
        &mut self.data.metadata
    }

    pub fn metadata_capacity_plan(
        &self,
        key: &str,
    ) -> Result<SceneMetadataCapacityPlan, OrderedMapError> {
        let additional = usize::from(!self.data.metadata.contains_key(key));
        self.data
            .metadata
            .replacement_plan(additional)
            .map(|inner| SceneMetadataCapacityPlan { inner })
    }

    pub fn prepare_insert_metadata(
        &mut self,
        key: String,
        value: MetadataValue,
        plan: SceneMetadataCapacityPlan,
    ) -> Result<PreparedSceneMetadataInsert<'_>, OrderedMapError> {
        self.data
            .metadata
            .prepare_replacement_insert(plan.inner, key, value)
            .map(|inner| PreparedSceneMetadataInsert { inner })
    }

    pub fn insert_metadata(
        &mut self,
        key: String,
        value: MetadataValue,
    ) -> Result<Option<MetadataValue>, OrderedMapError> {
        let plan = self.metadata_capacity_plan(&key)?;
        self.prepare_insert_metadata(key, value, plan)
            .map(PreparedSceneMetadataInsert::commit)
            .map(|retired| retired.map(|(_, value)| value))
    }

    pub fn space_mut(&mut self) -> &mut SceneSpace {
        &mut self.data.space
    }

    pub fn finish_draft(self) -> SceneDraft {
        SceneDraft {
            data: self.data,
            #[cfg(test)]
            drop_probe: None,
        }
    }
}

fn capacity_plan<T>(
    collection: &Vec<T>,
    additional: usize,
    resource: SceneBuildResource,
) -> Result<SceneCapacityPlan, SceneBuildError> {
    let required_capacity = collection
        .len()
        .checked_add(additional)
        .ok_or(SceneBuildError::CapacityAccountingOverflow { resource })?;
    let replacement = if required_capacity <= collection.capacity() {
        None
    } else {
        Some(
            ReplacementCapacityPlan::for_type::<T>(
                collection.len(),
                collection.capacity(),
                required_capacity,
            )
            .map_err(|cause| SceneBuildError::CapacityPlan { resource, cause })?,
        )
    };
    Ok(SceneCapacityPlan {
        resource,
        original_len: collection.len(),
        original_capacity: collection.capacity(),
        additional,
        replacement,
    })
}

fn allocate_replacement<T>(
    collection: &mut Vec<T>,
    plan: SceneCapacityPlan,
    resource: SceneBuildResource,
) -> Result<(Option<Vec<T>>, u64), SceneBuildError> {
    if plan.resource != resource {
        return Err(SceneBuildError::CapacityPlanResourceMismatch {
            expected: resource,
            actual: plan.resource,
        });
    }
    if collection.len() != plan.original_len || collection.capacity() != plan.original_capacity {
        return Err(SceneBuildError::CapacityPlan {
            resource,
            cause: baozi_operation::CapacityPlanError::StalePlan {
                expected_len: plan.original_len,
                expected_capacity: plan.original_capacity,
                actual_len: collection.len(),
                actual_capacity: collection.capacity(),
            },
        });
    }
    let required_capacity = collection
        .len()
        .checked_add(plan.additional)
        .ok_or(SceneBuildError::CapacityAccountingOverflow { resource })?;
    let Some(replacement_plan) = plan.replacement else {
        if required_capacity > collection.capacity() {
            return Err(SceneBuildError::CapacityPlan {
                resource,
                cause: baozi_operation::CapacityPlanError::InvalidState {
                    len: required_capacity,
                    capacity: collection.capacity(),
                },
            });
        }
        return Ok((None, 0));
    };

    replacement_plan
        .verify_source(collection.len(), collection.capacity())
        .map_err(|cause| SceneBuildError::CapacityPlan { resource, cause })?;
    let candidate_plan = replacement_plan.candidate();
    candidate_plan
        .verify_preflight(0, 0)
        .map_err(|cause| SceneBuildError::CapacityPlan { resource, cause })?;
    let mut replacement = Vec::new();
    replacement
        .try_reserve_exact(candidate_plan.allocator_additional())
        .map_err(|_| SceneBuildError::AllocationFailed { resource })?;
    let replacement_bytes = candidate_plan
        .finish(replacement.len(), replacement.capacity())
        .map_err(|cause| SceneBuildError::CapacityPlan { resource, cause })?;
    Ok((Some(replacement), replacement_bytes))
}

fn prepare_insert<'owner, T, Id>(
    collection: &'owner mut Vec<T>,
    value: T,
    id: Id,
    plan: SceneCapacityPlan,
    resource: SceneBuildResource,
) -> Result<PreparedSceneInsert<'owner, T, Id>, SceneBuildError> {
    let (replacement, replacement_bytes) = allocate_replacement(collection, plan, resource)?;
    ensure_insertion_slot(collection, replacement.as_ref(), resource)?;
    Ok(PreparedSceneInsert {
        owner: collection,
        replacement,
        value: Some(value),
        id,
        replacement_bytes,
    })
}

fn ensure_insertion_slot<T>(
    collection: &Vec<T>,
    replacement: Option<&Vec<T>>,
    resource: SceneBuildResource,
) -> Result<(), SceneBuildError> {
    let required_len = collection
        .len()
        .checked_add(1)
        .ok_or(SceneBuildError::CapacityAccountingOverflow { resource })?;
    let candidate_capacity = replacement.map_or_else(|| collection.capacity(), Vec::capacity);
    if required_len > candidate_capacity {
        return Err(SceneBuildError::CapacityPlan {
            resource,
            cause: baozi_operation::CapacityPlanError::InvalidState {
                len: required_len,
                capacity: candidate_capacity,
            },
        });
    }
    Ok(())
}

impl<T, Id> PreparedSceneInsert<'_, T, Id> {
    pub const fn replacement_bytes(&self) -> u64 {
        self.replacement_bytes
    }

    pub const fn replaces_allocation(&self) -> bool {
        self.replacement.is_some()
    }

    /// Commits into already admitted storage and releases the retired allocation before returning.
    pub fn commit(mut self) -> Id {
        let value = self
            .value
            .take()
            .expect("a prepared scene value is committed exactly once");
        if let Some(mut replacement) = self.replacement.take() {
            assert!(
                replacement.capacity() > self.owner.len(),
                "a scene replacement must admit every live value and the insertion"
            );
            replacement.append(self.owner);
            replacement.push(value);
            std::mem::swap(self.owner, &mut replacement);
            drop(replacement);
        } else {
            assert!(
                self.owner.len() < self.owner.capacity(),
                "an in-place scene insertion requires an admitted slot"
            );
            self.owner.push(value);
        }
        self.id
    }
}

impl PreparedNodeInsert<'_> {
    pub const fn node_replacement_bytes(&self) -> u64 {
        self.node_replacement_bytes
    }

    pub const fn parent_children_replacement_bytes(&self) -> u64 {
        self.child_replacement_bytes
    }

    pub const fn replaces_nodes(&self) -> bool {
        self.node_replacement.is_some()
    }

    pub const fn replaces_parent_children(&self) -> bool {
        self.child_replacement.is_some()
    }

    /// Commits the node and parent edge as one allocation-free mutation.
    pub fn commit(mut self) -> NodeId {
        let node = self
            .node
            .take()
            .expect("a prepared node is committed exactly once");
        assert!(
            node.children.is_empty(),
            "an inserted node cannot pre-own children"
        );
        assert!(
            self.parent.index() < self.builder.data.nodes.len(),
            "a prepared node parent remains in range while the candidate borrows the builder"
        );
        let node_capacity = self
            .node_replacement
            .as_ref()
            .map_or_else(|| self.builder.data.nodes.capacity(), Vec::capacity);
        assert!(
            node_capacity > self.builder.data.nodes.len(),
            "a prepared node insertion requires an admitted slot"
        );
        let parent_node = &self.builder.data.nodes[self.parent.index()];
        let child_capacity = self
            .child_replacement
            .as_ref()
            .map_or_else(|| parent_node.children.capacity(), Vec::capacity);
        assert!(
            child_capacity > parent_node.children.len(),
            "a prepared parent-child insertion requires an admitted slot"
        );

        if let Some(mut replacement) = self.node_replacement.take() {
            replacement.append(&mut self.builder.data.nodes);
            commit_node_insert(
                &mut replacement,
                self.parent,
                self.id,
                node,
                self.child_replacement.take(),
            );
            std::mem::swap(&mut self.builder.data.nodes, &mut replacement);
            drop(replacement);
        } else {
            commit_node_insert(
                &mut self.builder.data.nodes,
                self.parent,
                self.id,
                node,
                self.child_replacement.take(),
            );
        }
        self.id
    }
}

fn commit_node_insert(
    nodes: &mut Vec<Node>,
    parent: NodeId,
    id: NodeId,
    node: Node,
    child_replacement: Option<Vec<NodeId>>,
) {
    let parent_node = &mut nodes[parent.index()];
    if let Some(mut replacement) = child_replacement {
        replacement.append(&mut parent_node.children);
        replacement.push(id);
        std::mem::swap(&mut parent_node.children, &mut replacement);
        drop(replacement);
    } else {
        parent_node.children.push(id);
    }
    nodes.push(node);
}

impl PreparedNodeMeshBindingInsert<'_> {
    pub const fn replacement_bytes(&self) -> u64 {
        self.replacement_bytes
    }

    pub const fn replaces_allocation(&self) -> bool {
        self.replacement.is_some()
    }

    pub fn commit(mut self) {
        let binding = self
            .binding
            .take()
            .expect("a prepared mesh binding is committed exactly once");
        if let Some(mut replacement) = self.replacement.take() {
            assert!(
                replacement.capacity() > self.bindings.len(),
                "a mesh-binding replacement must admit every live binding and the insertion"
            );
            replacement.append(self.bindings);
            replacement.push(binding);
            std::mem::swap(self.bindings, &mut replacement);
            drop(replacement);
        } else {
            assert!(
                self.bindings.len() < self.bindings.capacity(),
                "an in-place mesh-binding insertion requires an admitted slot"
            );
            self.bindings.push(binding);
        }
    }
}

impl PreparedSceneMetadataInsert<'_> {
    pub const fn replacement_bytes(&self) -> u64 {
        self.inner.replacement_bytes()
    }

    pub const fn replaces_allocation(&self) -> bool {
        self.inner.replaces_allocation()
    }

    pub fn commit(self) -> Option<(String, MetadataValue)> {
        self.inner.commit()
    }
}

fn next_id(len: usize, collection: SceneCollection) -> Result<u32, SceneBuildError> {
    u32::try_from(len).map_err(|_| SceneBuildError::IdentifierSpaceExhausted { collection })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU8;

    #[derive(Debug)]
    struct FinalPollError;

    impl fmt::Display for FinalPollError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("final validation poll rejected")
        }
    }

    impl Error for FinalPollError {}

    struct DraftReservationDropProbe(Arc<AtomicU8>);

    impl Drop for DraftReservationDropProbe {
        fn drop(&mut self) {
            assert_eq!(
                self.0.swap(2, Ordering::SeqCst),
                1,
                "the scene draft must be dropped before reservation rollback"
            );
        }
    }

    struct FinalPollReservation {
        kind: SceneReservationKind,
        _canonical: Option<DraftReservationDropProbe>,
    }

    struct FinalPollControl {
        state: Arc<AtomicU8>,
        reserved: bool,
        panic_on_final_poll: bool,
    }

    impl SceneValidationControl for FinalPollControl {
        type Error = FinalPollError;
        type Reservation = FinalPollReservation;

        fn poll(&mut self) -> Result<(), Self::Error> {
            if !self.reserved {
                return Ok(());
            }
            if self.panic_on_final_poll {
                panic!("final validation poll panic fixture");
            }
            Err(FinalPollError)
        }

        fn charge_work(
            &mut self,
            _work: crate::SceneValidationWork,
            _amount: u64,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn reserve(
            &mut self,
            kind: SceneReservationKind,
            _bytes: u64,
        ) -> Result<Self::Reservation, Self::Error> {
            if kind == SceneReservationKind::CanonicalOutput {
                self.reserved = true;
                Ok(FinalPollReservation {
                    kind,
                    _canonical: Some(DraftReservationDropProbe(Arc::clone(&self.state))),
                })
            } else {
                Ok(FinalPollReservation {
                    kind,
                    _canonical: None,
                })
            }
        }

        fn validate_reservation(
            &mut self,
            reservation: &Self::Reservation,
            kind: SceneReservationKind,
        ) -> Result<(), Self::Error> {
            if reservation.kind != kind {
                return Err(FinalPollError);
            }
            Ok(())
        }

        fn resize_reservation(
            &mut self,
            _reservation: &mut Self::Reservation,
            _kind: SceneReservationKind,
            _required_bytes: u64,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn transition_reservation(
            &mut self,
            _reservation: &mut Self::Reservation,
            _from: SceneReservationKind,
            _to: SceneReservationKind,
            _required_bytes: u64,
        ) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn final_poll_draft(state: &Arc<AtomicU8>) -> SceneDraft {
        let mut draft = SceneBuilder::try_new()
            .expect("final-poll fixture builder")
            .finish_draft();
        draft.drop_probe = Some(SceneDraftDropProbe(Arc::clone(state)));
        draft
    }

    #[test]
    fn failed_final_poll_drops_scene_draft_before_reservation_rollback() {
        let state = Arc::new(AtomicU8::new(0));
        let mut control = FinalPollControl {
            state: Arc::clone(&state),
            reserved: false,
            panic_on_final_poll: false,
        };
        let error = validate_scene_with(
            final_poll_draft(&state),
            SceneValidationLimits::default(),
            &mut control,
        )
        .expect_err("the final poll must reject publication");

        assert!(matches!(error, SceneValidationFailure::Control(_)));
        assert_eq!(state.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn panicking_final_poll_drops_scene_draft_before_reservation_rollback() {
        let state = Arc::new(AtomicU8::new(0));
        let mut control = FinalPollControl {
            state: Arc::clone(&state),
            reserved: false,
            panic_on_final_poll: true,
        };
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = validate_scene_with(
                final_poll_draft(&state),
                SceneValidationLimits::default(),
                &mut control,
            );
        }));

        assert!(unwind.is_err());
        assert_eq!(state.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn builder_creates_owned_scene() {
        let mut builder = SceneBuilder::try_new().unwrap();
        let material = builder.add_material(Material::default()).unwrap();
        let mesh = builder
            .add_mesh(Mesh {
                material: Some(material),
                positions: vec![
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(0.0, 1.0, 0.0),
                ],
                ..Mesh::default()
            })
            .unwrap();
        let node = builder
            .add_child_node(
                builder.root(),
                Node {
                    mesh_bindings: vec![MeshBinding::new(mesh)],
                    ..Node::default()
                },
            )
            .unwrap();

        let scene = crate::validate_scene(builder.finish_draft()).unwrap();
        assert_eq!(scene.root(), NodeId::new(0));
        assert_eq!(scene.nodes()[scene.root().index()].children, vec![node]);
        assert_eq!(
            scene.nodes()[node.index()].mesh_bindings,
            vec![MeshBinding::new(mesh)]
        );
        assert_eq!(scene.meshes()[mesh.index()].material, Some(material));
    }

    #[test]
    fn repeated_node_replacements_have_geometric_total_move_work() {
        const NODE_COUNT: usize = 4_096;

        let mut builder = SceneBuilder::try_new().unwrap();
        let mut parent = builder.root();
        let mut previous_capacity = builder.data.nodes.capacity();
        let mut replacement_count = 0_usize;
        let mut total_moved_nodes = 0_usize;

        for _ in 1..NODE_COUNT {
            let source_len = builder.data.nodes.len();
            parent = builder.add_child_node(parent, Node::default()).unwrap();
            let current_capacity = builder.data.nodes.capacity();
            if current_capacity != previous_capacity {
                replacement_count += 1;
                total_moved_nodes = total_moved_nodes
                    .checked_add(source_len)
                    .expect("small geometric move count");
                previous_capacity = current_capacity;
            }
        }

        assert!(replacement_count <= NODE_COUNT.ilog2() as usize + 1);
        assert!(total_moved_nodes < NODE_COUNT * 2);
    }

    #[test]
    fn prepared_collection_candidate_preserves_the_owner_until_commit() {
        let mut builder = SceneBuilder::try_new().unwrap();
        builder
            .add_mesh(Mesh {
                name: Some("first".to_owned()),
                ..Mesh::default()
            })
            .unwrap();
        let original = builder.data.meshes.clone();
        let plan = builder
            .collection_capacity_plan(SceneCollection::Meshes, 1)
            .unwrap();
        let admitted = plan.envelope_delta_bytes().unwrap();
        let prepared = builder
            .prepare_add_mesh(
                Mesh {
                    name: Some("second".to_owned()),
                    ..Mesh::default()
                },
                plan,
            )
            .unwrap();
        assert!(prepared.replaces_allocation());
        assert!(prepared.replacement_bytes() <= admitted);
        drop(prepared);
        assert_eq!(builder.data.meshes, original);

        let plan = builder
            .collection_capacity_plan(SceneCollection::Meshes, 1)
            .unwrap();
        let id = builder
            .prepare_add_mesh(
                Mesh {
                    name: Some("second".to_owned()),
                    ..Mesh::default()
                },
                plan,
            )
            .unwrap()
            .commit();

        assert_eq!(id, MeshId::new(1));
        assert_eq!(builder.data.meshes[0].name.as_deref(), Some("first"));
        assert_eq!(builder.data.meshes[1].name.as_deref(), Some("second"));
    }

    #[test]
    fn prepared_node_candidate_commits_node_and_parent_edge_atomically() {
        let mut builder = SceneBuilder::try_new().unwrap();
        let root = builder.root();
        let plan = builder.node_insert_capacity_plan(root).unwrap();
        let node_admitted = plan.node_envelope_bytes().unwrap();
        let child_admitted = plan.parent_children_envelope_bytes().unwrap();
        let prepared = builder
            .prepare_add_child_node(
                root,
                Node {
                    name: Some("child".to_owned()),
                    ..Node::default()
                },
                plan,
            )
            .unwrap();
        assert!(prepared.replaces_nodes());
        assert!(prepared.replaces_parent_children());
        assert!(prepared.node_replacement_bytes() <= node_admitted);
        assert!(prepared.parent_children_replacement_bytes() <= child_admitted);
        drop(prepared);
        assert_eq!(builder.data.nodes.len(), 1);
        assert!(builder.data.nodes[root.index()].children.is_empty());

        let plan = builder.node_insert_capacity_plan(root).unwrap();
        let child = builder
            .prepare_add_child_node(
                root,
                Node {
                    name: Some("child".to_owned()),
                    ..Node::default()
                },
                plan,
            )
            .unwrap()
            .commit();
        assert_eq!(builder.data.nodes.len(), 2);
        assert_eq!(builder.data.nodes[root.index()].children, vec![child]);
        assert_eq!(builder.data.nodes[child.index()].parent, Some(root));
        assert_eq!(
            builder.data.nodes[child.index()].name.as_deref(),
            Some("child")
        );
    }

    #[test]
    fn builder_rejects_wrong_stale_and_under_admitted_plans_without_mutation() {
        let mut builder = SceneBuilder::try_new().unwrap();
        let wrong = builder
            .collection_capacity_plan(SceneCollection::Materials, 1)
            .unwrap();
        let wrong_error = match builder.prepare_add_mesh(Mesh::default(), wrong) {
            Ok(_) => panic!("a material plan must not prepare mesh storage"),
            Err(error) => error,
        };
        assert!(matches!(
            wrong_error,
            SceneBuildError::CapacityPlanResourceMismatch {
                expected: SceneBuildResource::Meshes,
                actual: SceneBuildResource::Materials,
            }
        ));
        assert!(builder.data.meshes.is_empty());

        let stale = builder
            .collection_capacity_plan(SceneCollection::Meshes, 1)
            .unwrap();
        builder.add_mesh(Mesh::default()).unwrap();
        let state = builder.data.meshes.clone();
        let capacity = builder.data.meshes.capacity();
        let stale_error = match builder.prepare_add_mesh(Mesh::default(), stale) {
            Ok(_) => panic!("a stale plan must not prepare replacement storage"),
            Err(error) => error,
        };
        assert!(matches!(
            stale_error,
            SceneBuildError::CapacityPlan {
                cause: baozi_operation::CapacityPlanError::StalePlan { .. },
                ..
            }
        ));
        assert_eq!(builder.data.meshes, state);
        assert_eq!(builder.data.meshes.capacity(), capacity);

        let mut empty = SceneBuilder::try_new().unwrap();
        let under_admitted = empty
            .collection_capacity_plan(SceneCollection::Meshes, 0)
            .unwrap();
        let under_admitted_error = match empty.prepare_add_mesh(Mesh::default(), under_admitted) {
            Ok(_) => panic!("a plan without an insertion slot must be rejected during prepare"),
            Err(error) => error,
        };
        assert!(matches!(
            under_admitted_error,
            SceneBuildError::CapacityPlan {
                cause: baozi_operation::CapacityPlanError::InvalidState {
                    len: 1,
                    capacity: 0,
                },
                ..
            }
        ));
        assert!(empty.data.meshes.is_empty());
        assert_eq!(empty.data.meshes.capacity(), 0);
    }

    #[test]
    fn prepared_metadata_candidate_preserves_sorting_and_returns_retired_storage() {
        let mut builder = SceneBuilder::try_new().unwrap();
        builder
            .insert_metadata("z:key".to_owned(), MetadataValue::I64(1))
            .unwrap();
        let original = builder.data.metadata.clone();
        let plan = builder.metadata_capacity_plan("a:key").unwrap();
        let admitted = plan.envelope_bytes().unwrap();
        let prepared = builder
            .prepare_insert_metadata("a:key".to_owned(), MetadataValue::Bool(true), plan)
            .unwrap();
        assert!(prepared.replaces_allocation());
        assert!(prepared.replacement_bytes() <= admitted);
        drop(prepared);
        assert_eq!(builder.data.metadata, original);

        let plan = builder.metadata_capacity_plan("a:key").unwrap();
        assert_eq!(
            builder
                .prepare_insert_metadata("a:key".to_owned(), MetadataValue::Bool(true), plan)
                .unwrap()
                .commit(),
            None
        );
        assert_eq!(
            builder
                .data
                .metadata
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["a:key", "z:key"]
        );

        let plan = builder.metadata_capacity_plan("a:key").unwrap();
        let retired = builder
            .prepare_insert_metadata("a:key".to_owned(), MetadataValue::I64(2), plan)
            .unwrap()
            .commit();
        assert_eq!(
            retired,
            Some(("a:key".to_owned(), MetadataValue::Bool(true)))
        );
        assert_eq!(
            builder.data.metadata.get("a:key"),
            Some(&MetadataValue::I64(2))
        );
    }

    #[test]
    fn controlled_builder_creation_exposes_root_envelope_before_allocation() {
        let plan = SceneBuilder::initial_capacity_plan().unwrap();
        let envelope = plan.envelope_delta_bytes().unwrap();
        assert!(envelope >= std::mem::size_of::<Node>() as u64);

        let builder = SceneBuilder::try_new_with_capacity_plan(plan).unwrap();
        assert_eq!(builder.root(), NodeId::new(0));
        assert_eq!(builder.data.nodes.capacity(), 1);
    }
}
