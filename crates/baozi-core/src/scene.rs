use crate::Result;
use crate::material::{Material, Texture};
use crate::math::{Aabb, Color, Mat4, Vec2, Vec3, Vec4};
use crate::validation::validate_scene;
use std::collections::BTreeMap;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);

        impl $name {
            pub const fn new(index: u32) -> Self {
                Self(index)
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
            metadata: MetadataMap::new(),
        }
    }
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
    pub material: Option<MaterialId>,
    pub bounds: Option<Aabb>,
    pub metadata: MetadataMap,
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
            material: None,
            bounds: None,
            metadata: MetadataMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Animation {
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Camera {
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Light {
    pub name: Option<String>,
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
                metadata: MetadataMap::new(),
                space: SceneSpace::default(),
            },
        }
    }

    pub fn root(&self) -> NodeId {
        self.scene.root
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
