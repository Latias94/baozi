use crate::Result;
use crate::material::{Material, Texture};
use crate::math::{Aabb, Color, Mat4, Vec2, Vec3, Vec4};
use crate::validation::validate_scene;
use std::collections::BTreeMap;

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

pub type MetadataMap = BTreeMap<String, MetadataValue>;

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: Option<String>,
    pub transform: Mat4,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub meshes: Vec<MeshId>,
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
            meshes: Vec::new(),
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
    pub skin: Option<SkinId>,
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
            skin: None,
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

#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub root: NodeId,
    pub nodes: Vec<Node>,
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub animations: Vec<Animation>,
    pub cameras: Vec<Camera>,
    pub lights: Vec<Light>,
    pub skins: Vec<Skin>,
    pub metadata: MetadataMap,
    pub space: SceneSpace,
}

#[derive(Debug)]
pub struct SceneBuilder {
    scene: Scene,
}

impl Default for SceneBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneBuilder {
    pub fn new() -> Self {
        let root = NodeId::new(0);
        Self {
            scene: Scene {
                root,
                nodes: vec![Node::default()],
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
        }
    }

    pub fn root(&self) -> NodeId {
        self.scene.root
    }

    pub fn mesh_count(&self) -> usize {
        self.scene.meshes.len()
    }

    pub fn add_node(&mut self, node: Node) -> NodeId {
        self.push_node(Some(self.scene.root), node)
    }

    pub fn add_child_node(&mut self, parent: NodeId, node: Node) -> Result<NodeId> {
        if parent.index() >= self.scene.nodes.len() {
            return Err(crate::BaoziError::InvalidScene {
                message: "parent node is out of range".to_owned(),
            });
        }
        Ok(self.push_node(Some(parent), node))
    }

    fn push_node(&mut self, parent: Option<NodeId>, mut node: Node) -> NodeId {
        let id = NodeId::new(self.scene.nodes.len() as u32);
        node.parent = parent;
        self.scene.nodes.push(node);
        if let Some(parent) = parent
            && let Some(parent_node) = self.scene.nodes.get_mut(parent.index())
        {
            parent_node.children.push(id);
        }
        id
    }

    pub fn add_mesh(&mut self, mesh: Mesh) -> MeshId {
        let id = MeshId::new(self.scene.meshes.len() as u32);
        self.scene.meshes.push(mesh);
        id
    }

    pub fn add_material(&mut self, material: Material) -> MaterialId {
        let id = MaterialId::new(self.scene.materials.len() as u32);
        self.scene.materials.push(material);
        id
    }

    pub fn add_texture(&mut self, texture: Texture) -> TextureId {
        let id = TextureId::new(self.scene.textures.len() as u32);
        self.scene.textures.push(texture);
        id
    }

    pub fn add_animation(&mut self, animation: Animation) -> AnimationId {
        let id = AnimationId::new(self.scene.animations.len() as u32);
        self.scene.animations.push(animation);
        id
    }

    pub fn add_camera(&mut self, camera: Camera) -> CameraId {
        let id = CameraId::new(self.scene.cameras.len() as u32);
        self.scene.cameras.push(camera);
        id
    }

    pub fn add_light(&mut self, light: Light) -> LightId {
        let id = LightId::new(self.scene.lights.len() as u32);
        self.scene.lights.push(light);
        id
    }

    pub fn add_skin(&mut self, skin: Skin) -> SkinId {
        let id = SkinId::new(self.scene.skins.len() as u32);
        self.scene.skins.push(skin);
        id
    }

    pub fn finish(self) -> Result<Scene> {
        validate_scene(&self.scene)?;
        Ok(self.scene)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_creates_owned_scene() {
        let mut builder = SceneBuilder::new();
        let material = builder.add_material(Material::default());
        let mesh = builder.add_mesh(Mesh {
            material: Some(material),
            positions: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            ..Mesh::default()
        });
        let node = builder
            .add_child_node(
                builder.root(),
                Node {
                    meshes: vec![mesh],
                    ..Node::default()
                },
            )
            .unwrap();

        let scene = builder.finish().unwrap();
        assert_eq!(scene.root, NodeId::new(0));
        assert_eq!(scene.nodes[scene.root.index()].children, vec![node]);
        assert_eq!(scene.nodes[node.index()].meshes, vec![mesh]);
        assert_eq!(scene.meshes[mesh.index()].material, Some(material));
    }
}
