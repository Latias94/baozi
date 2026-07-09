use baozi_core::Scene;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSummary {
    pub node_count: usize,
    pub mesh_count: usize,
    pub material_count: usize,
    pub texture_count: usize,
}

impl SceneSummary {
    pub fn from_scene(scene: &Scene) -> Self {
        Self {
            node_count: scene.nodes.len(),
            mesh_count: scene.meshes.len(),
            material_count: scene.materials.len(),
            texture_count: scene.textures.len(),
        }
    }
}
