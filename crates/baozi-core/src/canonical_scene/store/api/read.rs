//! Borrowed application views. Storage and publication details remain private.

use super::super::construction::node_scene::semantic::{
    NodeSceneSemanticError, NodeSceneSemanticResult,
};
use super::scene::ValidatedScene;
use crate::identity::{SceneEntityKind, SceneGenerationId, SceneLookupError};

mod animation;
mod appearance;
mod deformation;
mod geometry;
mod node;
mod spatial;

pub use animation::{AnimationChannelRef, AnimationClipRef, AnimationTarget};
pub use appearance::{ImageRef, MaterialBindingRef, MaterialRef, SamplerRef, TextureRef};
pub use deformation::{
    InfluenceRef, InfluenceRowRef, InfluenceRows, MorphDeltaData, MorphDeltaRef,
    MorphTargetDefinitionRef, MorphTargetRef, SkinRef,
};
pub use geometry::{
    AttributeData, AttributeRef, AttributeSemanticRef, LogicalMeshRef, MeshInstanceRef,
    PrimitiveRef,
};
pub use node::{NodeChildren, NodeRef};
pub use spatial::{CameraRef, LightRef, SceneSpaceRef, TangentBasisRef, TangentEncodingRef};

fn expect_validated<T>(result: NodeSceneSemanticResult<T>) -> T {
    result.unwrap_or_else(|error: NodeSceneSemanticError| {
        panic!(
            "validated Scene storage invariant failed: {}",
            error.reason()
        )
    })
}

fn checked_index(
    scene: &ValidatedScene,
    kind: SceneEntityKind,
    generation: SceneGenerationId,
    index: u32,
    count: usize,
) -> Result<usize, SceneLookupError> {
    if generation != scene.generation() {
        return Err(SceneLookupError::GenerationMismatch {
            kind,
            expected: scene.generation(),
            actual: generation,
        });
    }
    let index = usize::try_from(index).map_err(|_| SceneLookupError::OutOfRange {
        kind,
        length: count,
    })?;
    if index >= count {
        return Err(SceneLookupError::OutOfRange {
            kind,
            length: count,
        });
    }
    Ok(index)
}
