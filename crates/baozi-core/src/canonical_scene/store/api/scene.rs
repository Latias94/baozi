use baozi_operation::PhysicalStorageAccount;

use crate::identity::SceneGenerationId;

use super::super::construction::node_scene::ValidatedNodeScene;

/// Opaque owner of one centrally validated canonical Scene.
#[must_use = "the validated Scene owns retained canonical storage"]
pub struct ValidatedScene(ValidatedNodeScene);

impl ValidatedScene {
    pub(in crate::canonical_scene::store) const fn from_node_scene(
        scene: ValidatedNodeScene,
    ) -> Self {
        Self(scene)
    }

    /// Returns the generation reserved for this successfully published Scene.
    pub const fn generation(&self) -> SceneGenerationId {
        self.0.generation()
    }

    /// Returns the number of retained canonical blocks owned by this Scene.
    pub fn canonical_block_count(&self) -> usize {
        self.0.canonical_block_count()
    }

    pub(crate) fn account(&self) -> &PhysicalStorageAccount {
        self.0.account()
    }

    /// Restricts access to the sealed retained rail so semantic views and SceneEdit can project
    /// domain data without exposing the raw canonical store.
    pub(in crate::canonical_scene::store) const fn as_node_scene(&self) -> &ValidatedNodeScene {
        &self.0
    }
}
