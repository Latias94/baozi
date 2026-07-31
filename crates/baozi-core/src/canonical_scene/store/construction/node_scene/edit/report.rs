use crate::derived::DerivedAbsenceReason;
use crate::identity::SceneGenerationId;
use crate::remap::{
    AnimationClipRemapSpace, CameraRemapSpace, CornerRemapSpace, ExplicitUnchangedRemap,
    FaceRemapSpace, ImageRemapSpace, InstanceRemapSpace, LightRemapSpace, LogicalMeshRemapSpace,
    MaterialRemapSpace, MeshInstanceRemapSpace, MorphTargetRemapSpace, NodeRemapSpace,
    PrimitiveRemapSpace, RemapChange, RemapError, RemapGenerationBoundary,
    RemapGenerationBoundaryError, RemapSpace, SamplerRemapSpace, SkinRemapSpace, TextureRemapSpace,
    VertexRemapSpace,
};
use thiserror::Error;

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

pub(crate) struct CanonicalSubjectRemap<S: RemapSpace> {
    unchanged: ExplicitUnchangedRemap<S>,
}

impl<S: RemapSpace> CanonicalSubjectRemap<S> {
    pub(crate) fn input_generation(&self) -> SceneGenerationId {
        self.unchanged.input_generation()
    }

    pub(crate) fn output_generation(&self) -> SceneGenerationId {
        self.unchanged.output_generation()
    }

    pub(crate) fn change(&self) -> RemapChange {
        self.unchanged.change()
    }

    pub(crate) fn try_rebind_unchanged(
        &self,
        endpoint: S::Endpoint,
    ) -> Result<Option<S::Endpoint>, RemapError<S::Endpoint>> {
        self.unchanged.try_rebind(endpoint).map(Some)
    }
}

pub(crate) struct CanonicalRemapSet {
    nodes: CanonicalSubjectRemap<NodeRemapSpace>,
    mesh_instances: CanonicalSubjectRemap<MeshInstanceRemapSpace>,
    logical_meshes: CanonicalSubjectRemap<LogicalMeshRemapSpace>,
    primitives: CanonicalSubjectRemap<PrimitiveRemapSpace>,
    morph_targets: CanonicalSubjectRemap<MorphTargetRemapSpace>,
    materials: CanonicalSubjectRemap<MaterialRemapSpace>,
    images: CanonicalSubjectRemap<ImageRemapSpace>,
    samplers: CanonicalSubjectRemap<SamplerRemapSpace>,
    textures: CanonicalSubjectRemap<TextureRemapSpace>,
    skins: CanonicalSubjectRemap<SkinRemapSpace>,
    animation_clips: CanonicalSubjectRemap<AnimationClipRemapSpace>,
    cameras: CanonicalSubjectRemap<CameraRemapSpace>,
    lights: CanonicalSubjectRemap<LightRemapSpace>,
    vertices: CanonicalSubjectRemap<VertexRemapSpace>,
    corners: CanonicalSubjectRemap<CornerRemapSpace>,
    faces: CanonicalSubjectRemap<FaceRemapSpace>,
    instances: CanonicalSubjectRemap<InstanceRemapSpace>,
}

impl CanonicalRemapSet {
    pub(crate) fn try_all_unchanged(
        from_generation: SceneGenerationId,
        to_generation: SceneGenerationId,
    ) -> Result<Self, CanonicalRemapSetError> {
        let boundary = RemapGenerationBoundary::try_new(from_generation, to_generation)
            .map_err(CanonicalRemapSetError::Generation)?;
        macro_rules! unchanged {
            ($space:ty) => {
                CanonicalSubjectRemap {
                    unchanged: ExplicitUnchangedRemap::<$space>::from_boundary(boundary),
                }
            };
        }
        Ok(Self {
            nodes: unchanged!(NodeRemapSpace),
            mesh_instances: unchanged!(MeshInstanceRemapSpace),
            logical_meshes: unchanged!(LogicalMeshRemapSpace),
            primitives: unchanged!(PrimitiveRemapSpace),
            morph_targets: unchanged!(MorphTargetRemapSpace),
            materials: unchanged!(MaterialRemapSpace),
            images: unchanged!(ImageRemapSpace),
            samplers: unchanged!(SamplerRemapSpace),
            textures: unchanged!(TextureRemapSpace),
            skins: unchanged!(SkinRemapSpace),
            animation_clips: unchanged!(AnimationClipRemapSpace),
            cameras: unchanged!(CameraRemapSpace),
            lights: unchanged!(LightRemapSpace),
            vertices: unchanged!(VertexRemapSpace),
            corners: unchanged!(CornerRemapSpace),
            faces: unchanged!(FaceRemapSpace),
            instances: unchanged!(InstanceRemapSpace),
        })
    }

    pub(crate) const fn nodes(&self) -> &CanonicalSubjectRemap<NodeRemapSpace> {
        &self.nodes
    }

    pub(crate) const fn mesh_instances(&self) -> &CanonicalSubjectRemap<MeshInstanceRemapSpace> {
        &self.mesh_instances
    }

    pub(crate) const fn logical_meshes(&self) -> &CanonicalSubjectRemap<LogicalMeshRemapSpace> {
        &self.logical_meshes
    }

    pub(crate) const fn primitives(&self) -> &CanonicalSubjectRemap<PrimitiveRemapSpace> {
        &self.primitives
    }

    pub(crate) const fn morph_targets(&self) -> &CanonicalSubjectRemap<MorphTargetRemapSpace> {
        &self.morph_targets
    }

    pub(crate) const fn materials(&self) -> &CanonicalSubjectRemap<MaterialRemapSpace> {
        &self.materials
    }

    pub(crate) const fn images(&self) -> &CanonicalSubjectRemap<ImageRemapSpace> {
        &self.images
    }

    pub(crate) const fn samplers(&self) -> &CanonicalSubjectRemap<SamplerRemapSpace> {
        &self.samplers
    }

    pub(crate) const fn textures(&self) -> &CanonicalSubjectRemap<TextureRemapSpace> {
        &self.textures
    }

    pub(crate) const fn skins(&self) -> &CanonicalSubjectRemap<SkinRemapSpace> {
        &self.skins
    }

    pub(crate) const fn animation_clips(&self) -> &CanonicalSubjectRemap<AnimationClipRemapSpace> {
        &self.animation_clips
    }

    pub(crate) const fn cameras(&self) -> &CanonicalSubjectRemap<CameraRemapSpace> {
        &self.cameras
    }

    pub(crate) const fn lights(&self) -> &CanonicalSubjectRemap<LightRemapSpace> {
        &self.lights
    }

    pub(crate) const fn vertices(&self) -> &CanonicalSubjectRemap<VertexRemapSpace> {
        &self.vertices
    }

    pub(crate) const fn corners(&self) -> &CanonicalSubjectRemap<CornerRemapSpace> {
        &self.corners
    }

    pub(crate) const fn faces(&self) -> &CanonicalSubjectRemap<FaceRemapSpace> {
        &self.faces
    }

    pub(crate) const fn instances(&self) -> &CanonicalSubjectRemap<InstanceRemapSpace> {
        &self.instances
    }

    pub(crate) const fn retained_allocation_bytes(&self) -> u64 {
        0
    }
}

#[derive(Debug, Error)]
pub(crate) enum CanonicalRemapSetError {
    #[error("canonical remap generation boundary failed")]
    Generation(#[source] RemapGenerationBoundaryError),
}
