use std::marker::PhantomData;

use thiserror::Error;

use crate::identity::{NodeId, SceneGenerationId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DerivedAbsenceReason {
    Invalidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CanonicalInvalidations {
    scene_bounds: Option<DerivedAbsenceReason>,
}

impl CanonicalInvalidations {
    pub(crate) const fn scene_bounds_invalidated() -> Self {
        Self {
            scene_bounds: Some(DerivedAbsenceReason::Invalidated),
        }
    }

    pub(crate) const fn scene_bounds(self) -> Option<DerivedAbsenceReason> {
        self.scene_bounds
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemapChange {
    ExplicitlyUnchanged,
}

pub(crate) struct NodeRemapSubject;
pub(crate) struct MeshInstanceRemapSubject;
pub(crate) struct LogicalMeshRemapSubject;
pub(crate) struct PrimitiveRemapSubject;
pub(crate) struct MorphTargetRemapSubject;
pub(crate) struct MaterialRemapSubject;
pub(crate) struct ImageRemapSubject;
pub(crate) struct SamplerRemapSubject;
pub(crate) struct TextureRemapSubject;
pub(crate) struct SkinRemapSubject;
pub(crate) struct AnimationClipRemapSubject;
pub(crate) struct CameraRemapSubject;
pub(crate) struct LightRemapSubject;
pub(crate) struct VertexRemapSubject;
pub(crate) struct CornerRemapSubject;
pub(crate) struct FaceRemapSubject;
pub(crate) struct InstanceRemapSubject;

pub(crate) struct CanonicalSubjectRemap<E> {
    input_generation: SceneGenerationId,
    output_generation: SceneGenerationId,
    endpoint: PhantomData<fn(E) -> E>,
}

impl<E> CanonicalSubjectRemap<E> {
    const fn unchanged(
        input_generation: SceneGenerationId,
        output_generation: SceneGenerationId,
    ) -> Self {
        Self {
            input_generation,
            output_generation,
            endpoint: PhantomData,
        }
    }

    pub(crate) const fn input_generation(&self) -> SceneGenerationId {
        self.input_generation
    }

    pub(crate) const fn output_generation(&self) -> SceneGenerationId {
        self.output_generation
    }

    pub(crate) const fn change(&self) -> RemapChange {
        RemapChange::ExplicitlyUnchanged
    }
}

impl CanonicalSubjectRemap<NodeRemapSubject> {
    pub(crate) fn try_rebind_unchanged(
        &self,
        endpoint: NodeId,
    ) -> Result<Option<NodeId>, CanonicalRemapError> {
        let actual = endpoint.generation();
        if actual != self.input_generation {
            return Err(CanonicalRemapError::InputGenerationMismatch {
                expected: self.input_generation,
                actual,
            });
        }
        Ok(Some(endpoint.rebind(self.output_generation)))
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalRemapError {
    #[error("remap endpoint belongs to generation {actual:?}, expected {expected:?}")]
    InputGenerationMismatch {
        expected: SceneGenerationId,
        actual: SceneGenerationId,
    },
}

pub(crate) struct CanonicalRemapSet {
    input_generation: SceneGenerationId,
    output_generation: SceneGenerationId,
}

impl CanonicalRemapSet {
    pub(crate) fn try_all_unchanged(
        input_generation: SceneGenerationId,
        output_generation: SceneGenerationId,
    ) -> Result<Self, CanonicalRemapSetError> {
        if input_generation == output_generation {
            return Err(CanonicalRemapSetError::SameGeneration);
        }
        Ok(Self {
            input_generation,
            output_generation,
        })
    }

    const fn unchanged<E>(&self) -> CanonicalSubjectRemap<E> {
        CanonicalSubjectRemap::unchanged(self.input_generation, self.output_generation)
    }

    pub(crate) const fn nodes(&self) -> CanonicalSubjectRemap<NodeRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn mesh_instances(&self) -> CanonicalSubjectRemap<MeshInstanceRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn logical_meshes(&self) -> CanonicalSubjectRemap<LogicalMeshRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn primitives(&self) -> CanonicalSubjectRemap<PrimitiveRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn morph_targets(&self) -> CanonicalSubjectRemap<MorphTargetRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn materials(&self) -> CanonicalSubjectRemap<MaterialRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn images(&self) -> CanonicalSubjectRemap<ImageRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn samplers(&self) -> CanonicalSubjectRemap<SamplerRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn textures(&self) -> CanonicalSubjectRemap<TextureRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn skins(&self) -> CanonicalSubjectRemap<SkinRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn animation_clips(&self) -> CanonicalSubjectRemap<AnimationClipRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn cameras(&self) -> CanonicalSubjectRemap<CameraRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn lights(&self) -> CanonicalSubjectRemap<LightRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn vertices(&self) -> CanonicalSubjectRemap<VertexRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn corners(&self) -> CanonicalSubjectRemap<CornerRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn faces(&self) -> CanonicalSubjectRemap<FaceRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn instances(&self) -> CanonicalSubjectRemap<InstanceRemapSubject> {
        self.unchanged()
    }

    pub(crate) const fn retained_allocation_bytes(&self) -> u64 {
        0
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalRemapSetError {
    #[error("canonical remap generations must be distinct")]
    SameGeneration,
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;

    use super::*;

    fn next_generation(source: &AtomicU64) -> SceneGenerationId {
        SceneGenerationId::reserve_from(source)
            .expect("test generation")
            .into_id()
    }

    #[test]
    fn remap_set_rejects_a_same_generation_boundary() {
        let source = AtomicU64::new(0);
        let generation = next_generation(&source);

        assert!(matches!(
            CanonicalRemapSet::try_all_unchanged(generation, generation),
            Err(CanonicalRemapSetError::SameGeneration)
        ));
    }

    #[test]
    fn node_rebind_rejects_a_foreign_input_generation() {
        let source = AtomicU64::new(0);
        let input = next_generation(&source);
        let output = next_generation(&source);
        let foreign = next_generation(&source);
        let remaps = CanonicalRemapSet::try_all_unchanged(input, output)
            .expect("distinct generation boundary");

        assert_eq!(
            remaps.nodes().try_rebind_unchanged(NodeId::new(foreign, 0)),
            Err(CanonicalRemapError::InputGenerationMismatch {
                expected: input,
                actual: foreign,
            })
        );
    }
}
