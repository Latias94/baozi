use crate::material::{Material, Texture};
use crate::math::{Aabb, Color, Mat4, Vec2, Vec3, Vec4};
use crate::scene_storage::{clone_scene_data_with, unwrap_uncontrolled};
use crate::validation_control::NoopSceneValidationControl;
use crate::{
    SceneBuildError, SceneBuildResource, SceneFootprint, SceneReservationKind,
    SceneValidationControl, SceneValidationError, SceneValidationFailure, SceneValidationLimits,
    SceneValidationStats, SceneValidationSuccess,
};
use baozi_operation::CapacityPlan;
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
    inner: CapacityPlan,
}

impl SceneCapacityPlan {
    /// Returns the byte increase that must be admitted before allocator entry.
    pub fn envelope_delta_bytes(self) -> Result<u64, SceneBuildError> {
        self.inner
            .envelope_delta_bytes()
            .map_err(|cause| SceneBuildError::CapacityPlan {
                resource: self.resource,
                cause,
            })
    }
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
        })
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
    let canonical_output = control
        .reserve(
            SceneReservationKind::CanonicalOutput,
            stats.footprint().accounted_payload_bytes(),
        )
        .map_err(SceneValidationFailure::Control)?;
    control.poll().map_err(SceneValidationFailure::Control)?;
    let scene = ValidatedScene::from_validated_data(scene.data, stats)?;
    Ok(SceneValidationSuccess::new(scene, canonical_output))
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
        let Self {
            base_generation: _,
            data,
            reservation,
        } = self;
        let stats = crate::validation::validate_scene_data(&data, limits, control)?;
        let canonical_output = control
            .transition_reservation(
                reservation,
                SceneReservationKind::EditCandidate,
                SceneReservationKind::CanonicalOutput,
                stats.footprint().accounted_payload_bytes(),
            )
            .map_err(SceneValidationFailure::Control)?;
        control.poll().map_err(SceneValidationFailure::Control)?;
        let scene = ValidatedScene::from_validated_data(data, stats)?;
        Ok(SceneValidationSuccess::new(scene, canonical_output))
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
        apply_capacity_plan(&mut nodes, plan, SceneBuildResource::Nodes)?;
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

    /// Applies an admitted plan and returns the actual retained byte growth.
    pub fn apply_collection_capacity_plan(
        &mut self,
        collection: SceneCollection,
        plan: SceneCapacityPlan,
    ) -> Result<u64, SceneBuildError> {
        match collection {
            SceneCollection::Nodes => {
                apply_capacity_plan(&mut self.data.nodes, plan, SceneBuildResource::Nodes)
            }
            SceneCollection::Meshes => {
                apply_capacity_plan(&mut self.data.meshes, plan, SceneBuildResource::Meshes)
            }
            SceneCollection::Materials => apply_capacity_plan(
                &mut self.data.materials,
                plan,
                SceneBuildResource::Materials,
            ),
            SceneCollection::Textures => {
                apply_capacity_plan(&mut self.data.textures, plan, SceneBuildResource::Textures)
            }
            SceneCollection::Animations => apply_capacity_plan(
                &mut self.data.animations,
                plan,
                SceneBuildResource::Animations,
            ),
            SceneCollection::Cameras => {
                apply_capacity_plan(&mut self.data.cameras, plan, SceneBuildResource::Cameras)
            }
            SceneCollection::Lights => {
                apply_capacity_plan(&mut self.data.lights, plan, SceneBuildResource::Lights)
            }
            SceneCollection::Skins => {
                apply_capacity_plan(&mut self.data.skins, plan, SceneBuildResource::Skins)
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

    pub fn apply_node_children_capacity_plan(
        &mut self,
        parent: NodeId,
        plan: SceneCapacityPlan,
    ) -> Result<u64, SceneBuildError> {
        let node_count = self.data.nodes.len();
        let node = self
            .data
            .nodes
            .get_mut(parent.index())
            .ok_or(SceneBuildError::ParentNodeOutOfRange { parent, node_count })?;
        apply_capacity_plan(&mut node.children, plan, SceneBuildResource::NodeChildren)
    }

    pub fn add_node(&mut self, node: Node) -> Result<NodeId, SceneBuildError> {
        self.push_node(Some(self.data.root), node)
    }

    pub fn add_child_node(
        &mut self,
        parent: NodeId,
        node: Node,
    ) -> Result<NodeId, SceneBuildError> {
        if parent.index() >= self.data.nodes.len() {
            return Err(SceneBuildError::ParentNodeOutOfRange {
                parent,
                node_count: self.data.nodes.len(),
            });
        }
        self.push_node(Some(parent), node)
    }

    fn push_node(
        &mut self,
        parent: Option<NodeId>,
        mut node: Node,
    ) -> Result<NodeId, SceneBuildError> {
        let id = next_id(self.data.nodes.len(), SceneCollection::Nodes).map(NodeId::new)?;
        if let Some(parent) = parent {
            let children = &mut self.data.nodes[parent.index()].children;
            try_reserve_one(children, SceneBuildResource::NodeChildren)?;
        }
        try_reserve_one(&mut self.data.nodes, SceneBuildResource::Nodes)?;
        node.parent = parent;
        self.data.nodes.push(node);
        if let Some(parent) = parent
            && let Some(parent_node) = self.data.nodes.get_mut(parent.index())
        {
            parent_node.children.push(id);
        }
        Ok(id)
    }

    pub fn add_mesh(&mut self, mesh: Mesh) -> Result<MeshId, SceneBuildError> {
        let id = next_id(self.data.meshes.len(), SceneCollection::Meshes).map(MeshId::new)?;
        try_reserve_one(&mut self.data.meshes, SceneBuildResource::Meshes)?;
        self.data.meshes.push(mesh);
        Ok(id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.data.nodes.get_mut(id.index())
    }

    pub fn mesh_mut(&mut self, id: MeshId) -> Option<&mut Mesh> {
        self.data.meshes.get_mut(id.index())
    }

    pub fn add_material(&mut self, material: Material) -> Result<MaterialId, SceneBuildError> {
        let id =
            next_id(self.data.materials.len(), SceneCollection::Materials).map(MaterialId::new)?;
        try_reserve_one(&mut self.data.materials, SceneBuildResource::Materials)?;
        self.data.materials.push(material);
        Ok(id)
    }

    pub fn material_mut(&mut self, id: MaterialId) -> Option<&mut Material> {
        self.data.materials.get_mut(id.index())
    }

    pub fn add_texture(&mut self, texture: Texture) -> Result<TextureId, SceneBuildError> {
        let id =
            next_id(self.data.textures.len(), SceneCollection::Textures).map(TextureId::new)?;
        try_reserve_one(&mut self.data.textures, SceneBuildResource::Textures)?;
        self.data.textures.push(texture);
        Ok(id)
    }

    pub fn texture_mut(&mut self, id: TextureId) -> Option<&mut Texture> {
        self.data.textures.get_mut(id.index())
    }

    pub fn add_animation(&mut self, animation: Animation) -> Result<AnimationId, SceneBuildError> {
        let id = next_id(self.data.animations.len(), SceneCollection::Animations)
            .map(AnimationId::new)?;
        try_reserve_one(&mut self.data.animations, SceneBuildResource::Animations)?;
        self.data.animations.push(animation);
        Ok(id)
    }

    pub fn add_camera(&mut self, camera: Camera) -> Result<CameraId, SceneBuildError> {
        let id = next_id(self.data.cameras.len(), SceneCollection::Cameras).map(CameraId::new)?;
        try_reserve_one(&mut self.data.cameras, SceneBuildResource::Cameras)?;
        self.data.cameras.push(camera);
        Ok(id)
    }

    pub fn add_light(&mut self, light: Light) -> Result<LightId, SceneBuildError> {
        let id = next_id(self.data.lights.len(), SceneCollection::Lights).map(LightId::new)?;
        try_reserve_one(&mut self.data.lights, SceneBuildResource::Lights)?;
        self.data.lights.push(light);
        Ok(id)
    }

    pub fn add_skin(&mut self, skin: Skin) -> Result<SkinId, SceneBuildError> {
        let id = next_id(self.data.skins.len(), SceneCollection::Skins).map(SkinId::new)?;
        try_reserve_one(&mut self.data.skins, SceneBuildResource::Skins)?;
        self.data.skins.push(skin);
        Ok(id)
    }

    pub fn metadata_mut(&mut self) -> &mut MetadataMap {
        &mut self.data.metadata
    }

    pub fn space_mut(&mut self) -> &mut SceneSpace {
        &mut self.data.space
    }

    pub fn finish_draft(self) -> SceneDraft {
        SceneDraft { data: self.data }
    }
}

fn try_reserve_one<T>(
    collection: &mut Vec<T>,
    resource: SceneBuildResource,
) -> Result<(), SceneBuildError> {
    let plan = capacity_plan(collection, 1, resource)?;
    apply_capacity_plan(collection, plan, resource).map(|_| ())
}

fn capacity_plan<T>(
    collection: &Vec<T>,
    additional: usize,
    resource: SceneBuildResource,
) -> Result<SceneCapacityPlan, SceneBuildError> {
    let desired_capacity = collection
        .len()
        .checked_add(additional)
        .ok_or(SceneBuildError::CapacityAccountingOverflow { resource })?;
    let inner =
        CapacityPlan::for_type::<T>(collection.len(), collection.capacity(), desired_capacity)
            .map_err(|cause| SceneBuildError::CapacityPlan { resource, cause })?;
    Ok(SceneCapacityPlan { resource, inner })
}

fn apply_capacity_plan<T>(
    collection: &mut Vec<T>,
    plan: SceneCapacityPlan,
    resource: SceneBuildResource,
) -> Result<u64, SceneBuildError> {
    if plan.resource != resource {
        return Err(SceneBuildError::CapacityPlanResourceMismatch {
            expected: resource,
            actual: plan.resource,
        });
    }
    plan.inner
        .verify_preflight(collection.len(), collection.capacity())
        .map_err(|cause| SceneBuildError::CapacityPlan { resource, cause })?;
    collection
        .try_reserve_exact(plan.inner.allocator_additional())
        .map_err(|_| SceneBuildError::AllocationFailed { resource })?;
    plan.inner
        .finish(collection.len(), collection.capacity())
        .map_err(|cause| SceneBuildError::CapacityPlan { resource, cause })
}

fn next_id(len: usize, collection: SceneCollection) -> Result<u32, SceneBuildError> {
    u32::try_from(len).map_err(|_| SceneBuildError::IdentifierSpaceExhausted { collection })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn builder_exposes_two_phase_capacity_admission() {
        let mut builder = SceneBuilder::try_new().unwrap();
        let plan = builder
            .collection_capacity_plan(SceneCollection::Meshes, 1)
            .unwrap();
        let admitted = plan.envelope_delta_bytes().unwrap();
        let actual = builder
            .apply_collection_capacity_plan(SceneCollection::Meshes, plan)
            .unwrap();

        assert!(actual <= admitted);
        assert_eq!(
            builder
                .collection_capacity_plan(SceneCollection::Meshes, 1)
                .unwrap()
                .envelope_delta_bytes()
                .unwrap(),
            0
        );

        let child_plan = builder
            .node_children_capacity_plan(builder.root(), 1)
            .unwrap();
        let child_admitted = child_plan.envelope_delta_bytes().unwrap();
        let child_actual = builder
            .apply_node_children_capacity_plan(builder.root(), child_plan)
            .unwrap();
        assert!(child_actual <= child_admitted);
    }

    #[test]
    fn builder_rejects_a_plan_for_the_wrong_collection() {
        let mut builder = SceneBuilder::try_new().unwrap();
        let plan = builder
            .collection_capacity_plan(SceneCollection::Meshes, 1)
            .unwrap();

        assert!(matches!(
            builder.apply_collection_capacity_plan(SceneCollection::Materials, plan),
            Err(SceneBuildError::CapacityPlanResourceMismatch {
                expected: SceneBuildResource::Materials,
                actual: SceneBuildResource::Meshes,
            })
        ));
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
