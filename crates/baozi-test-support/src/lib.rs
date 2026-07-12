#![forbid(unsafe_code)]

pub mod scale_fixture;
pub mod snapshot;
pub mod support_matrix;

use baozi_core::ValidatedScene;

pub use scale_fixture::{ObjScale, ObjScaleFixture, ObjScaleFixtureError};
pub use snapshot::{SceneSnapshot, SnapshotOptions};
pub use support_matrix::{SupportMatrixColumn, assert_support_matrix_row};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSummary {
    pub node_count: usize,
    pub mesh_count: usize,
    pub material_count: usize,
    pub texture_count: usize,
}

impl SceneSummary {
    pub fn from_scene(scene: &ValidatedScene) -> Self {
        Self {
            node_count: scene.nodes().len(),
            mesh_count: scene.meshes().len(),
            material_count: scene.materials().len(),
            texture_count: scene.textures().len(),
        }
    }
}
