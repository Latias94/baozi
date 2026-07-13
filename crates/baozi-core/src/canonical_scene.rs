use crate::animation::AnimationClip;
use crate::appearance::Material;
use crate::derived::{Derived, DerivedAbsenceReason};
use crate::entity::{Camera, Light, Node};
use crate::extension::{
    ControlledRetainedFootprintError, ExtensionEntry, ExtensionEntryClassification,
    ExtensionEntryValidationError, ExtensionSchemaSnapshot,
    extension_entries_retained_allocation_bytes_controlled, validate_unique_extension_entry_keys,
};
use crate::geometry::{GeometryBoundsError, LogicalMesh, MeshInstance, validate_aabb};
use crate::identity::{
    AnimationClipIndex, CameraIndex, ImageIndex, LightIndex, LogicalMeshIndex, MaterialIndex,
    MeshInstanceIndex, NodeId, NodeIndex, SamplerIndex, SceneGenerationError, SceneGenerationId,
    SceneLookupError, SceneStorageIndexError, SkinIndex, TextureIndex, lookup_scene_wide,
};
use crate::image::{Image, Sampler, Texture};
use crate::math::Aabb;
use crate::provenance::{ProvenanceArena, ProvenanceAttachments};
use crate::scene_space::SceneSpace;
use crate::shared_storage::{SharedStorage, SharedStorageError};
use crate::validation_control::SceneValidationStats;
use baozi_operation::CapacityPlan;
use std::convert::Infallible;
use thiserror::Error;

pub(crate) const MAX_SCENE_METADATA_ENTRIES: usize = 4_096;

/// Scene-owned descriptive metadata. Behavioral extension state has typed owners elsewhere.
#[derive(Debug, PartialEq)]
pub(crate) struct SceneMetadata {
    entries: Vec<ExtensionEntry>,
}

impl SceneMetadata {
    pub(crate) fn try_new(
        entries: impl IntoIterator<Item = ExtensionEntry>,
    ) -> Result<Self, SceneMetadataError> {
        let mut entries = entries.into_iter();
        let (lower, _) = entries.size_hint();
        if lower > MAX_SCENE_METADATA_ENTRIES {
            return Err(SceneMetadataError::LimitExceeded {
                maximum: MAX_SCENE_METADATA_ENTRIES,
            });
        }

        let mut collected = Vec::new();
        try_reserve_metadata(&mut collected, lower)?;
        for entry in entries.by_ref() {
            if entry.classification() != ExtensionEntryClassification::DescriptiveMetadata {
                return Err(SceneMetadataError::BehavioralEntry);
            }
            if collected.len() == MAX_SCENE_METADATA_ENTRIES {
                return Err(SceneMetadataError::LimitExceeded {
                    maximum: MAX_SCENE_METADATA_ENTRIES,
                });
            }
            if collected.len() == collected.capacity() {
                let desired = collected
                    .len()
                    .checked_add(1)
                    .ok_or(SceneMetadataError::AllocationFailed)?;
                try_reserve_metadata(&mut collected, desired)?;
            }
            collected.push(entry);
        }
        validate_unique_extension_entry_keys(&collected)?;
        Ok(Self { entries: collected })
    }

    pub(crate) const fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(crate) fn entries(&self) -> &[ExtensionEntry] {
        &self.entries
    }

    pub(crate) fn retained_allocation_bytes(&self) -> Result<usize, SceneMetadataError> {
        match self.retained_allocation_bytes_controlled(&mut || Ok::<(), Infallible>(())) {
            Ok(bytes) => Ok(bytes),
            Err(ControlledRetainedFootprintError::AccountingOverflow) => {
                Err(SceneMetadataError::AccountingOverflow)
            }
            Err(ControlledRetainedFootprintError::Control(error)) => match error {},
        }
    }

    pub(crate) fn retained_allocation_bytes_controlled<E>(
        &self,
        charge_work: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<usize, ControlledRetainedFootprintError<E>> {
        extension_entries_retained_allocation_bytes_controlled(&self.entries, charge_work)
    }
}

impl Default for SceneMetadata {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SceneMetadataError {
    #[error("scene metadata contains a behavior-bearing extension entry")]
    BehavioralEntry,
    #[error("scene metadata entry count exceeds {maximum}")]
    LimitExceeded { maximum: usize },
    #[error("scene metadata allocation failed")]
    AllocationFailed,
    #[error("scene metadata retained-byte accounting overflowed")]
    AccountingOverflow,
    #[error(transparent)]
    EntryKeys(#[from] ExtensionEntryValidationError),
}

fn try_reserve_metadata(
    entries: &mut Vec<ExtensionEntry>,
    desired: usize,
) -> Result<(), SceneMetadataError> {
    if desired <= entries.capacity() {
        return Ok(());
    }
    let plan = CapacityPlan::for_type::<ExtensionEntry>(entries.len(), entries.capacity(), desired)
        .map_err(|_| SceneMetadataError::AllocationFailed)?;
    entries
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| SceneMetadataError::AllocationFailed)?;
    plan.finish(entries.len(), entries.capacity())
        .map_err(|_| SceneMetadataError::AllocationFailed)?;
    Ok(())
}

/// Scene-space bounds, distinct from primitive-local and logical-mesh-local bounds.
#[derive(Debug, PartialEq)]
pub(crate) struct SceneBounds {
    derived: Derived<Aabb>,
}

impl SceneBounds {
    pub(crate) fn try_from_validated(derived: Derived<Aabb>) -> Result<Self, GeometryBoundsError> {
        let bounds = derived
            .value()
            .copied()
            .ok_or(GeometryBoundsError::MissingValidatedValue)?;
        validate_aabb(bounds)?;
        Ok(Self { derived })
    }

    pub(crate) const fn absent(reason: DerivedAbsenceReason) -> Self {
        Self {
            derived: Derived::absent(reason),
        }
    }

    pub(crate) const fn derived(&self) -> &Derived<Aabb> {
        &self.derived
    }

    pub(crate) fn retained_producer_allocation_bytes(
        &self,
    ) -> Result<usize, crate::provenance::ProvenanceFootprintError> {
        self.derived.retained_producer_allocation_bytes()
    }
}

pub(crate) struct CanonicalSceneData {
    pub(crate) generation: SceneGenerationId,
    pub(crate) root: Option<NodeIndex>,
    pub(crate) scene_space: SceneSpace,
    pub(crate) provenance: ProvenanceAttachments,
    pub(crate) nodes: Vec<SharedStorage<Node>>,
    pub(crate) mesh_instances: Vec<SharedStorage<MeshInstance>>,
    pub(crate) logical_meshes: Vec<SharedStorage<LogicalMesh>>,
    pub(crate) materials: Vec<SharedStorage<Material>>,
    pub(crate) images: Vec<SharedStorage<Image>>,
    pub(crate) samplers: Vec<SharedStorage<Sampler>>,
    pub(crate) textures: Vec<SharedStorage<Texture>>,
    pub(crate) skins: Vec<SharedStorage<crate::deform::Skin>>,
    pub(crate) animation_clips: Vec<SharedStorage<AnimationClip>>,
    pub(crate) cameras: Vec<SharedStorage<Camera>>,
    pub(crate) lights: Vec<SharedStorage<Light>>,
    pub(crate) extension_schemas: ExtensionSchemaSnapshot,
    pub(crate) metadata: SceneMetadata,
    pub(crate) provenance_arena: ProvenanceArena,
    pub(crate) scene_bounds: SceneBounds,
}

pub(crate) struct CanonicalSceneDraft {
    pub(crate) data: CanonicalSceneData,
}

pub(crate) struct CanonicalSceneBuilder {
    data: CanonicalSceneData,
}

impl CanonicalSceneBuilder {
    pub(crate) fn try_new(
        scene_space: SceneSpace,
        extension_schemas: ExtensionSchemaSnapshot,
        provenance_arena: ProvenanceArena,
        provenance: impl Into<ProvenanceAttachments>,
        absent_scene_bounds: DerivedAbsenceReason,
    ) -> Result<Self, CanonicalSceneBuildError> {
        Ok(Self {
            data: CanonicalSceneData {
                generation: SceneGenerationId::allocate()?,
                root: None,
                scene_space,
                provenance: provenance.into(),
                nodes: Vec::new(),
                mesh_instances: Vec::new(),
                logical_meshes: Vec::new(),
                materials: Vec::new(),
                images: Vec::new(),
                samplers: Vec::new(),
                textures: Vec::new(),
                skins: Vec::new(),
                animation_clips: Vec::new(),
                cameras: Vec::new(),
                lights: Vec::new(),
                extension_schemas,
                metadata: SceneMetadata::empty(),
                provenance_arena,
                scene_bounds: SceneBounds::absent(absent_scene_bounds),
            },
        })
    }

    pub(crate) const fn generation(&self) -> SceneGenerationId {
        self.data.generation
    }

    pub(crate) fn try_push_node(
        &mut self,
        node: Node,
    ) -> Result<NodeIndex, CanonicalSceneBuildError> {
        let index = NodeIndex::try_from(self.data.nodes.len())?;
        try_push_shared(&mut self.data.nodes, node, CanonicalSceneCollection::Nodes)?;
        Ok(index)
    }

    pub(crate) fn try_push_mesh_instance(
        &mut self,
        instance: MeshInstance,
    ) -> Result<MeshInstanceIndex, CanonicalSceneBuildError> {
        let index = MeshInstanceIndex::try_from(self.data.mesh_instances.len())?;
        try_push_shared(
            &mut self.data.mesh_instances,
            instance,
            CanonicalSceneCollection::MeshInstances,
        )?;
        Ok(index)
    }

    pub(crate) fn try_push_logical_mesh(
        &mut self,
        mesh: LogicalMesh,
    ) -> Result<LogicalMeshIndex, CanonicalSceneBuildError> {
        let index = LogicalMeshIndex::try_from(self.data.logical_meshes.len())?;
        try_push_shared(
            &mut self.data.logical_meshes,
            mesh,
            CanonicalSceneCollection::LogicalMeshes,
        )?;
        Ok(index)
    }

    pub(crate) fn try_push_material(
        &mut self,
        material: Material,
    ) -> Result<MaterialIndex, CanonicalSceneBuildError> {
        let index = MaterialIndex::try_from(self.data.materials.len())?;
        try_push_shared(
            &mut self.data.materials,
            material,
            CanonicalSceneCollection::Materials,
        )?;
        Ok(index)
    }

    pub(crate) fn try_push_image(
        &mut self,
        image: Image,
    ) -> Result<ImageIndex, CanonicalSceneBuildError> {
        let index = ImageIndex::try_from(self.data.images.len())?;
        try_push_shared(
            &mut self.data.images,
            image,
            CanonicalSceneCollection::Images,
        )?;
        Ok(index)
    }

    pub(crate) fn try_push_sampler(
        &mut self,
        sampler: Sampler,
    ) -> Result<SamplerIndex, CanonicalSceneBuildError> {
        let index = SamplerIndex::try_from(self.data.samplers.len())?;
        try_push_shared(
            &mut self.data.samplers,
            sampler,
            CanonicalSceneCollection::Samplers,
        )?;
        Ok(index)
    }

    pub(crate) fn try_push_texture(
        &mut self,
        texture: Texture,
    ) -> Result<TextureIndex, CanonicalSceneBuildError> {
        let index = TextureIndex::try_from(self.data.textures.len())?;
        try_push_shared(
            &mut self.data.textures,
            texture,
            CanonicalSceneCollection::Textures,
        )?;
        Ok(index)
    }

    pub(crate) fn try_push_skin(
        &mut self,
        skin: crate::deform::Skin,
    ) -> Result<SkinIndex, CanonicalSceneBuildError> {
        let index = SkinIndex::try_from(self.data.skins.len())?;
        try_push_shared(&mut self.data.skins, skin, CanonicalSceneCollection::Skins)?;
        Ok(index)
    }

    pub(crate) fn try_push_animation_clip(
        &mut self,
        clip: AnimationClip,
    ) -> Result<AnimationClipIndex, CanonicalSceneBuildError> {
        let index = AnimationClipIndex::try_from(self.data.animation_clips.len())?;
        try_push_shared(
            &mut self.data.animation_clips,
            clip,
            CanonicalSceneCollection::AnimationClips,
        )?;
        Ok(index)
    }

    pub(crate) fn try_push_camera(
        &mut self,
        camera: Camera,
    ) -> Result<CameraIndex, CanonicalSceneBuildError> {
        let index = CameraIndex::try_from(self.data.cameras.len())?;
        try_push_shared(
            &mut self.data.cameras,
            camera,
            CanonicalSceneCollection::Cameras,
        )?;
        Ok(index)
    }

    pub(crate) fn try_push_light(
        &mut self,
        light: Light,
    ) -> Result<LightIndex, CanonicalSceneBuildError> {
        let index = LightIndex::try_from(self.data.lights.len())?;
        try_push_shared(
            &mut self.data.lights,
            light,
            CanonicalSceneCollection::Lights,
        )?;
        Ok(index)
    }

    pub(crate) fn set_metadata(&mut self, metadata: SceneMetadata) {
        self.data.metadata = metadata;
    }

    pub(crate) fn set_scene_bounds(&mut self, bounds: SceneBounds) {
        self.data.scene_bounds = bounds;
    }

    pub(crate) fn set_root(&mut self, root: NodeIndex) {
        self.data.root = Some(root);
    }

    pub(crate) fn finish_draft(self) -> CanonicalSceneDraft {
        CanonicalSceneDraft { data: self.data }
    }
}

fn try_push_shared<T>(
    values: &mut Vec<SharedStorage<T>>,
    value: T,
    collection: CanonicalSceneCollection,
) -> Result<(), CanonicalSceneBuildError> {
    let desired = values
        .len()
        .checked_add(1)
        .ok_or(CanonicalSceneBuildError::CollectionSizeOverflow { collection })?;
    let plan = CapacityPlan::for_type::<SharedStorage<T>>(values.len(), values.capacity(), desired)
        .map_err(|_| CanonicalSceneBuildError::CollectionSizeOverflow { collection })?;
    let value = SharedStorage::try_new(value).map_err(|SharedStorageError::AllocationFailed| {
        CanonicalSceneBuildError::AllocationFailed { collection }
    })?;
    if desired > values.capacity() {
        values
            .try_reserve_exact(plan.allocator_additional())
            .map_err(|_| CanonicalSceneBuildError::AllocationFailed { collection })?;
        plan.finish(values.len(), values.capacity())
            .map_err(|_| CanonicalSceneBuildError::AllocationFailed { collection })?;
    }
    values.push(value);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalSceneCollection {
    Nodes,
    MeshInstances,
    LogicalMeshes,
    Materials,
    Images,
    Samplers,
    Textures,
    Skins,
    AnimationClips,
    Cameras,
    Lights,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalSceneBuildError {
    #[error(transparent)]
    Generation(#[from] SceneGenerationError),
    #[error(transparent)]
    Index(#[from] SceneStorageIndexError),
    #[error("canonical scene collection size overflowed for {collection:?}")]
    CollectionSizeOverflow {
        collection: CanonicalSceneCollection,
    },
    #[error("canonical scene allocation failed for {collection:?}")]
    AllocationFailed {
        collection: CanonicalSceneCollection,
    },
}

pub(crate) struct CanonicalValidatedScene {
    data: CanonicalSceneData,
    validation_stats: SceneValidationStats,
    _proof: crate::canonical_validation::CanonicalValidationProof,
}

impl CanonicalValidatedScene {
    pub(super) const fn from_validated_parts(
        data: CanonicalSceneData,
        validation_stats: SceneValidationStats,
        proof: crate::canonical_validation::CanonicalValidationProof,
    ) -> Self {
        Self {
            data,
            validation_stats,
            _proof: proof,
        }
    }

    pub(crate) const fn generation(&self) -> SceneGenerationId {
        self.data.generation
    }

    pub(crate) const fn validation_stats(&self) -> SceneValidationStats {
        self.validation_stats
    }

    pub(crate) fn node(&self, id: NodeId) -> Result<&Node, SceneLookupError> {
        lookup_scene_wide(self.data.generation, id, &self.data.nodes).map(SharedStorage::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derived::DerivedAbsenceReason;
    use crate::entity::{AuthoredTransform, Camera, CameraProjection, Light, LightKind, Node};
    use crate::extension::ExtensionSchemaSnapshot;
    use crate::image::{Image, Sampler, Texture};
    use crate::math::Color;
    use crate::provenance::test_provenance_fixture;
    use crate::scene_space::SceneSpace;
    use crate::validation_control::SceneValidationLimits;

    fn root_node(provenance: crate::provenance::Provenance) -> Node {
        Node::try_new(
            None,
            AuthoredTransform::identity_trs(),
            [],
            None,
            None,
            provenance,
            [],
        )
        .expect("root node fixture should be valid")
    }

    #[test]
    fn root_only_draft_requires_central_validation_to_publish() {
        let (provenance_arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene generation should remain available");
        let root = builder
            .try_push_node(root_node(provenance))
            .expect("root allocation should succeed");
        builder.set_root(root);
        let generation = builder.generation();

        let scene = crate::canonical_validation::validate_canonical_scene(
            builder.finish_draft(),
            SceneValidationLimits::default(),
        )
        .expect("root-only canonical scene should validate");

        assert_eq!(scene.generation(), generation);
        assert!(scene.node(root.handle(generation)).is_ok());
        assert_eq!(scene.validation_stats().canonical_counts().nodes, 1);
    }

    #[test]
    fn builder_assigns_private_indexes_for_scene_wide_collections() {
        let (provenance_arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene generation should remain available");

        let camera = Camera::try_new(
            None,
            CameraProjection::perspective(1.0, None, 0.1, None).unwrap(),
            provenance,
            [],
        )
        .unwrap();
        let light = Light::try_new(
            None,
            LightKind::Directional,
            Color::linear_rgba(1.0, 1.0, 1.0, 1.0),
            1.0,
            provenance,
            [],
        )
        .unwrap();
        let image = Image::try_encoded([1, 2, 3], None, None, None, None, provenance).unwrap();
        let sampler = Sampler::canonical_default(provenance);

        let camera_index = builder.try_push_camera(camera).unwrap();
        let light_index = builder.try_push_light(light).unwrap();
        let image_index = builder.try_push_image(image).unwrap();
        let sampler_index = builder.try_push_sampler(sampler).unwrap();
        let texture_index = builder
            .try_push_texture(Texture::new(image_index, sampler_index, provenance))
            .unwrap();

        assert_eq!(camera_index.as_u32(), 0);
        assert_eq!(light_index.as_u32(), 0);
        assert_eq!(image_index.as_u32(), 0);
        assert_eq!(sampler_index.as_u32(), 0);
        assert_eq!(texture_index.as_u32(), 0);
    }
}
