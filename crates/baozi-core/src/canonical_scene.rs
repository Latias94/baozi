use crate::animation::{AnimationClip, AnimationTimeDomain};
use crate::appearance::{Material, MaterialView};
use crate::attribute::{Attribute, AttributeDomain, F32Vec3View};
use crate::canonical_edit::{
    CanonicalEditError, CanonicalEditFailure, CanonicalNodeTransformEdit,
    CanonicalNodeTransformEditStart,
};
use crate::deform::{
    InfluenceRows, MorphTargetDefinition, PrimitiveMorphTarget, Skin, SkinMatrixSpaces,
};
use crate::derived::{Derived, DerivedAbsenceReason};
use crate::entity::{AuthoredTransform, Camera, EntityName, Light, Node};
use crate::extension::{
    ControlledRetainedFootprintError, ExtensionEntry, ExtensionEntryClassification,
    ExtensionEntryValidationError, ExtensionSchemaSnapshot,
    extension_entries_retained_allocation_bytes_controlled, validate_unique_extension_entry_keys,
};
use crate::geometry::{
    GeometryBoundsError, IndexStream, LogicalMesh, LogicalMeshBounds, MeshInstance, MeshPrimitive,
    PrimitiveBounds, PrimitiveCardinality, Topology, validate_aabb,
};
use crate::identity::{
    AnimationClipId, AnimationClipIndex, CameraId, CameraIndex, ImageId, ImageIndex, LightId,
    LightIndex, LogicalMeshId, LogicalMeshIndex, MaterialId, MaterialIndex, MeshInstanceId,
    MeshInstanceIndex, MorphTargetId, NodeId, NodeIndex, PrimitiveId, SamplerId, SamplerIndex,
    SceneGenerationError, SceneGenerationId, SceneLookupError, SceneStorageIndexError, SkinId,
    SkinIndex, TextureId, TextureIndex, lookup_owned, lookup_scene_wide,
};
use crate::image::{Image, ImageView, Sampler, SamplerView, Texture, TextureView};
use crate::math::{Aabb, Mat4, Vec3};
use crate::provenance::{Provenance, ProvenanceArena, ProvenanceAttachments};
use crate::scene_space::SceneSpace;
use crate::shared_storage::{SharedStorage, SharedStorageError};
use crate::validation_control::{
    SceneValidationControl, SceneValidationLimits, SceneValidationStats,
};
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
    pub(crate) provenance: SharedStorage<ProvenanceAttachments>,
    pub(crate) nodes: Vec<Node>,
    pub(crate) mesh_instances: Vec<SharedStorage<MeshInstance>>,
    pub(crate) logical_meshes: Vec<LogicalMesh>,
    pub(crate) materials: Vec<SharedStorage<Material>>,
    pub(crate) images: Vec<SharedStorage<Image>>,
    pub(crate) samplers: Vec<SharedStorage<Sampler>>,
    pub(crate) textures: Vec<SharedStorage<Texture>>,
    pub(crate) skins: Vec<SharedStorage<crate::deform::Skin>>,
    pub(crate) animation_clips: Vec<SharedStorage<AnimationClip>>,
    pub(crate) cameras: Vec<SharedStorage<Camera>>,
    pub(crate) lights: Vec<SharedStorage<Light>>,
    pub(crate) extension_schemas: SharedStorage<ExtensionSchemaSnapshot>,
    pub(crate) metadata: SharedStorage<SceneMetadata>,
    pub(crate) provenance_arena: SharedStorage<ProvenanceArena>,
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
        let provenance = provenance.into();
        let provenance_retained_bytes = provenance.retained_allocation_bytes();
        let provenance = try_share_scene_payload(
            provenance,
            provenance_retained_bytes,
            CanonicalSceneSharedPayload::Provenance,
        )?;
        let extension_schema_retained_bytes = extension_schemas.retained_allocation_bytes();
        let extension_schemas = try_share_scene_payload(
            extension_schemas,
            extension_schema_retained_bytes,
            CanonicalSceneSharedPayload::ExtensionSchemas,
        )?;
        let metadata = try_share_scene_payload(
            SceneMetadata::empty(),
            0,
            CanonicalSceneSharedPayload::Metadata,
        )?;
        let provenance_arena =
            SharedStorage::try_new(provenance_arena).map_err(|error| match error {
                SharedStorageError::AllocationFailed => {
                    CanonicalSceneBuildError::ProvenanceArenaAllocationFailed
                }
                SharedStorageError::AccountingOverflow => {
                    CanonicalSceneBuildError::ProvenanceArenaAccountingOverflow
                }
            })?;
        Ok(Self {
            data: CanonicalSceneData {
                generation: SceneGenerationId::allocate()?,
                root: None,
                scene_space,
                provenance,
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
                metadata,
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
        try_push_owned(&mut self.data.nodes, node, CanonicalSceneCollection::Nodes)?;
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
        try_push_owned(
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

    pub(crate) fn set_metadata(
        &mut self,
        metadata: SceneMetadata,
    ) -> Result<(), CanonicalSceneBuildError> {
        let retained_bytes = metadata.retained_allocation_bytes().map_err(|_| {
            CanonicalSceneBuildError::SharedPayloadAccountingOverflow {
                payload: CanonicalSceneSharedPayload::Metadata,
            }
        })?;
        self.data.metadata = try_share_scene_payload(
            metadata,
            retained_bytes,
            CanonicalSceneSharedPayload::Metadata,
        )?;
        Ok(())
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
    try_push_prepared(values, collection, || {
        SharedStorage::try_new(value).map_err(|error| match error {
            SharedStorageError::AllocationFailed => {
                CanonicalSceneBuildError::AllocationFailed { collection }
            }
            SharedStorageError::AccountingOverflow => {
                CanonicalSceneBuildError::CollectionSizeOverflow { collection }
            }
        })
    })
}

fn try_push_owned<T>(
    values: &mut Vec<T>,
    value: T,
    collection: CanonicalSceneCollection,
) -> Result<(), CanonicalSceneBuildError> {
    try_push_prepared(values, collection, || Ok(value))
}

fn try_push_prepared<T>(
    values: &mut Vec<T>,
    collection: CanonicalSceneCollection,
    prepare: impl FnOnce() -> Result<T, CanonicalSceneBuildError>,
) -> Result<(), CanonicalSceneBuildError> {
    let desired = values
        .len()
        .checked_add(1)
        .ok_or(CanonicalSceneBuildError::CollectionSizeOverflow { collection })?;
    let plan = CapacityPlan::for_type::<T>(values.len(), values.capacity(), desired)
        .map_err(|_| CanonicalSceneBuildError::CollectionSizeOverflow { collection })?;
    let value = prepare()?;
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

fn try_share_scene_payload<T>(
    value: T,
    nested_allocation_bytes: usize,
    payload: CanonicalSceneSharedPayload,
) -> Result<SharedStorage<T>, CanonicalSceneBuildError> {
    SharedStorage::try_new_prepared(value, nested_allocation_bytes).map_err(|error| match error {
        SharedStorageError::AllocationFailed => {
            CanonicalSceneBuildError::SharedPayloadAllocationFailed { payload }
        }
        SharedStorageError::AccountingOverflow => {
            CanonicalSceneBuildError::SharedPayloadAccountingOverflow { payload }
        }
    })
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalSceneSharedPayload {
    Provenance,
    ExtensionSchemas,
    Metadata,
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
    #[error("canonical provenance arena shared allocation failed")]
    ProvenanceArenaAllocationFailed,
    #[error("canonical provenance arena shared allocation accounting overflowed")]
    ProvenanceArenaAccountingOverflow,
    #[error("canonical scene shared allocation failed for {payload:?}")]
    SharedPayloadAllocationFailed {
        payload: CanonicalSceneSharedPayload,
    },
    #[error("canonical scene shared allocation accounting overflowed for {payload:?}")]
    SharedPayloadAccountingOverflow {
        payload: CanonicalSceneSharedPayload,
    },
}

pub(crate) struct CanonicalValidatedScene {
    data: CanonicalSceneData,
    validation_stats: SceneValidationStats,
    _proof: crate::canonical_validation::CanonicalValidationProof,
}

struct StableEnumerate<I> {
    inner: I,
    next: u32,
}

impl<I> StableEnumerate<I> {
    const fn new(inner: I) -> Self {
        Self { inner, next: 0 }
    }
}

impl<I: Iterator> Iterator for StableEnumerate<I> {
    type Item = (u32, I::Item);

    fn next(&mut self) -> Option<Self::Item> {
        let value = self.inner.next()?;
        let index = self.next;
        self.next = self.next.wrapping_add(1);
        Some((index, value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<I: ExactSizeIterator> ExactSizeIterator for StableEnumerate<I> {}
impl<I: std::iter::FusedIterator> std::iter::FusedIterator for StableEnumerate<I> {}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NodeView<'a> {
    node: &'a Node,
    generation: SceneGenerationId,
}

impl<'a> NodeView<'a> {
    pub(crate) fn name(self) -> Option<&'a EntityName> {
        self.node.name()
    }

    pub(crate) const fn authored_transform(self) -> AuthoredTransform {
        self.node.authored_transform()
    }

    pub(crate) fn fork_with_authored_transform(
        self,
        authored_transform: AuthoredTransform,
    ) -> Node {
        self.node.with_authored_transform(authored_transform)
    }

    pub(crate) fn local_matrix(self) -> Mat4 {
        self.node.local_matrix()
    }

    pub(crate) fn children(self) -> impl ExactSizeIterator<Item = NodeId> + 'a {
        let generation = self.generation;
        self.node
            .children()
            .map(move |child| child.handle(generation))
    }

    pub(crate) fn camera(self) -> Option<CameraId> {
        self.node
            .camera_index()
            .map(|camera| camera.handle(self.generation))
    }

    pub(crate) fn light(self) -> Option<LightId> {
        self.node
            .light_index()
            .map(|light| light.handle(self.generation))
    }

    pub(crate) fn provenance(self) -> &'a ProvenanceAttachments {
        self.node.provenance()
    }

    pub(crate) fn extensions(self) -> impl ExactSizeIterator<Item = &'a ExtensionEntry> {
        self.node.extensions()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MeshInstanceView<'a> {
    instance: &'a MeshInstance,
    generation: SceneGenerationId,
}

impl<'a> MeshInstanceView<'a> {
    pub(crate) const fn owner(self) -> NodeId {
        self.instance.owner_index().handle(self.generation)
    }

    pub(crate) const fn logical_mesh(self) -> LogicalMeshId {
        self.instance.logical_mesh_index().handle(self.generation)
    }

    pub(crate) const fn skin(self) -> Option<SkinId> {
        match self.instance.skin_index() {
            Some(skin) => Some(skin.handle(self.generation)),
            None => None,
        }
    }

    pub(crate) fn morph_weight_override(self) -> Option<&'a [f32]> {
        self.instance.morph_weight_override()
    }

    pub(crate) fn attributes(self) -> &'a [Attribute] {
        self.instance.attributes()
    }

    pub(crate) const fn domain_count(self, domain: AttributeDomain) -> Option<u32> {
        self.instance.domain_count(domain)
    }

    pub(crate) const fn provenance(self) -> &'a ProvenanceAttachments {
        self.instance.provenance()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LogicalMeshView<'a> {
    mesh: &'a LogicalMesh,
    id: LogicalMeshId,
}

impl<'a> LogicalMeshView<'a> {
    pub(crate) const fn id(self) -> LogicalMeshId {
        self.id
    }

    pub(crate) fn name(self) -> Option<&'a EntityName> {
        self.mesh.name()
    }

    pub(crate) fn primitives(
        self,
    ) -> impl ExactSizeIterator<Item = (PrimitiveId, PrimitiveView<'a>)> + 'a {
        let owner = self.id;
        StableEnumerate::new(self.mesh.primitives().iter()).map(move |(local, primitive)| {
            let id = PrimitiveId::new(owner, local);
            (id, PrimitiveView { primitive, id })
        })
    }

    pub(crate) fn primitive(self, id: PrimitiveId) -> Result<PrimitiveView<'a>, SceneLookupError> {
        lookup_owned(self.id.generation(), self.id, id, self.mesh.primitives())
            .map(|primitive| PrimitiveView { primitive, id })
    }

    pub(crate) fn morph_targets(
        self,
    ) -> impl ExactSizeIterator<Item = (MorphTargetId, &'a MorphTargetDefinition)> + 'a {
        let owner = self.id;
        StableEnumerate::new(self.mesh.morph_target_definitions().iter())
            .map(move |(local, target)| (MorphTargetId::new(owner, local), target))
    }

    pub(crate) fn morph_target(
        self,
        id: MorphTargetId,
    ) -> Result<&'a MorphTargetDefinition, SceneLookupError> {
        lookup_owned(
            self.id.generation(),
            self.id,
            id,
            self.mesh.morph_target_definitions(),
        )
    }

    pub(crate) fn default_morph_weights(self) -> &'a [f32] {
        self.mesh.default_morph_weights()
    }

    pub(crate) const fn bounds(self) -> &'a LogicalMeshBounds {
        self.mesh.bounds()
    }

    pub(crate) fn provenance(self) -> &'a ProvenanceAttachments {
        self.mesh.provenance()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PrimitiveView<'a> {
    primitive: &'a MeshPrimitive,
    id: PrimitiveId,
}

impl<'a> PrimitiveView<'a> {
    pub(crate) const fn id(self) -> PrimitiveId {
        self.id
    }

    pub(crate) fn topology(self) -> &'a Topology {
        self.primitive.topology()
    }

    pub(crate) fn index_stream(self) -> &'a IndexStream {
        self.primitive.index_stream()
    }

    pub(crate) fn cardinality(self) -> PrimitiveCardinality {
        self.primitive.cardinality()
    }

    pub(crate) fn domain_count(self, domain: AttributeDomain) -> Option<u32> {
        self.primitive.domain_count(domain)
    }

    pub(crate) fn material(self) -> Option<MaterialId> {
        self.primitive
            .material_index()
            .map(|material| material.handle(self.id.generation()))
    }

    pub(crate) fn attributes(self) -> &'a [Attribute] {
        self.primitive.attributes()
    }

    pub(crate) fn influences(self) -> Option<&'a InfluenceRows> {
        self.primitive.influences()
    }

    pub(crate) fn morph_targets(self) -> &'a [PrimitiveMorphTarget] {
        self.primitive.morph_targets()
    }

    pub(crate) const fn bounds(self) -> &'a PrimitiveBounds {
        self.primitive.bounds()
    }

    pub(crate) fn provenance(self) -> &'a ProvenanceAttachments {
        self.primitive.provenance()
    }

    pub(crate) fn positions(self) -> Option<F32Vec3View<'a>> {
        self.position_attribute().and_then(Attribute::positions)
    }

    pub(crate) fn as_dense_positions(self) -> Option<&'a [Vec3]> {
        self.position_attribute()
            .and_then(Attribute::as_dense_positions)
    }

    fn position_attribute(self) -> Option<&'a Attribute> {
        self.primitive.attributes().iter().find(|attribute| {
            attribute.semantic() == &crate::attribute::AttributeSemantic::Position
                && attribute.domain() == AttributeDomain::Vertex
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SkinView<'a> {
    skin: &'a Skin,
    generation: SceneGenerationId,
}

impl<'a> SkinView<'a> {
    pub(crate) const fn name(self) -> Option<&'a EntityName> {
        self.skin.name()
    }

    pub(crate) fn joints(self) -> impl ExactSizeIterator<Item = NodeId> + 'a {
        let generation = self.generation;
        self.skin
            .joints()
            .iter()
            .map(move |joint| joint.handle(generation))
    }

    pub(crate) fn inverse_bind_matrices(self) -> &'a [Mat4] {
        self.skin.inverse_bind_matrices()
    }

    pub(crate) const fn skeleton_root_hint(self) -> Option<NodeId> {
        match self.skin.skeleton_root_hint() {
            Some(root) => Some(root.handle(self.generation)),
            None => None,
        }
    }

    pub(crate) const fn matrix_spaces(self) -> SkinMatrixSpaces {
        self.skin.matrix_spaces()
    }

    pub(crate) const fn provenance(self) -> Provenance {
        self.skin.provenance()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AnimationClipView<'a> {
    clip: &'a AnimationClip,
}

impl<'a> AnimationClipView<'a> {
    pub(crate) const fn duration(self) -> f64 {
        self.clip.duration()
    }

    pub(crate) const fn time_domain(self) -> &'a AnimationTimeDomain {
        self.clip.time_domain()
    }

    pub(crate) fn channel_count(self) -> usize {
        self.clip.channels().len()
    }

    pub(crate) const fn provenance(self) -> Provenance {
        self.clip.provenance()
    }
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

    pub(crate) const fn data(&self) -> &CanonicalSceneData {
        &self.data
    }

    pub(crate) fn try_edit_node_transform(
        &self,
        node: NodeId,
        authored_transform: AuthoredTransform,
        limits: SceneValidationLimits,
    ) -> Result<CanonicalNodeTransformEditStart<'_>, CanonicalEditError> {
        CanonicalNodeTransformEdit::try_begin(self, node, authored_transform, limits)
    }

    pub(crate) fn try_edit_node_transform_with<C: SceneValidationControl>(
        &self,
        node: NodeId,
        authored_transform: AuthoredTransform,
        limits: SceneValidationLimits,
        control: &mut C,
    ) -> Result<CanonicalNodeTransformEditStart<'_, C::Reservation>, CanonicalEditFailure<C::Error>>
    {
        CanonicalNodeTransformEdit::try_begin_with(self, node, authored_transform, limits, control)
    }

    pub(crate) const fn scene_bounds(&self) -> &SceneBounds {
        &self.data.scene_bounds
    }

    pub(crate) const fn root(&self) -> Option<NodeId> {
        match self.data.root {
            Some(root) => Some(root.handle(self.data.generation)),
            None => None,
        }
    }

    pub(crate) fn nodes(&self) -> impl ExactSizeIterator<Item = (NodeId, NodeView<'_>)> + '_ {
        let generation = self.data.generation;
        StableEnumerate::new(self.data.nodes.iter()).map(move |(index, node)| {
            (
                NodeId::new(generation, index),
                NodeView { node, generation },
            )
        })
    }

    pub(crate) fn node(&self, id: NodeId) -> Result<NodeView<'_>, SceneLookupError> {
        lookup_scene_wide(self.data.generation, id, &self.data.nodes).map(|node| NodeView {
            node,
            generation: self.data.generation,
        })
    }

    pub(crate) fn mesh_instances(
        &self,
    ) -> impl ExactSizeIterator<Item = (MeshInstanceId, MeshInstanceView<'_>)> + '_ {
        let generation = self.data.generation;
        StableEnumerate::new(self.data.mesh_instances.iter()).map(move |(index, instance)| {
            (
                MeshInstanceId::new(generation, index),
                MeshInstanceView {
                    instance: instance.as_ref(),
                    generation,
                },
            )
        })
    }

    pub(crate) fn mesh_instance(
        &self,
        id: MeshInstanceId,
    ) -> Result<MeshInstanceView<'_>, SceneLookupError> {
        lookup_scene_wide(self.data.generation, id, &self.data.mesh_instances).map(|instance| {
            MeshInstanceView {
                instance: instance.as_ref(),
                generation: self.data.generation,
            }
        })
    }

    pub(crate) fn logical_meshes(
        &self,
    ) -> impl ExactSizeIterator<Item = (LogicalMeshId, LogicalMeshView<'_>)> + '_ {
        let generation = self.data.generation;
        StableEnumerate::new(self.data.logical_meshes.iter()).map(move |(index, mesh)| {
            let id = LogicalMeshId::new(generation, index);
            (id, LogicalMeshView { mesh, id })
        })
    }

    pub(crate) fn logical_mesh(
        &self,
        id: LogicalMeshId,
    ) -> Result<LogicalMeshView<'_>, SceneLookupError> {
        lookup_scene_wide(self.data.generation, id, &self.data.logical_meshes)
            .map(|mesh| LogicalMeshView { mesh, id })
    }

    pub(crate) fn materials(
        &self,
    ) -> impl ExactSizeIterator<Item = (MaterialId, MaterialView<'_>)> + '_ {
        let generation = self.data.generation;
        StableEnumerate::new(self.data.materials.iter()).map(move |(index, material)| {
            (
                MaterialId::new(generation, index),
                material.as_ref().view(generation),
            )
        })
    }

    pub(crate) fn material(&self, id: MaterialId) -> Result<MaterialView<'_>, SceneLookupError> {
        lookup_scene_wide(self.data.generation, id, &self.data.materials)
            .map(|material| material.as_ref().view(self.data.generation))
    }

    pub(crate) fn images(&self) -> impl ExactSizeIterator<Item = (ImageId, ImageView<'_>)> + '_ {
        let generation = self.data.generation;
        StableEnumerate::new(self.data.images.iter())
            .map(move |(index, image)| (ImageId::new(generation, index), image.as_ref().view()))
    }

    pub(crate) fn image(&self, id: ImageId) -> Result<ImageView<'_>, SceneLookupError> {
        lookup_scene_wide(self.data.generation, id, &self.data.images)
            .map(|image| image.as_ref().view())
    }

    pub(crate) fn samplers(
        &self,
    ) -> impl ExactSizeIterator<Item = (SamplerId, SamplerView<'_>)> + '_ {
        let generation = self.data.generation;
        StableEnumerate::new(self.data.samplers.iter()).map(move |(index, sampler)| {
            (SamplerId::new(generation, index), sampler.as_ref().view())
        })
    }

    pub(crate) fn sampler(&self, id: SamplerId) -> Result<SamplerView<'_>, SceneLookupError> {
        lookup_scene_wide(self.data.generation, id, &self.data.samplers)
            .map(|sampler| sampler.as_ref().view())
    }

    pub(crate) fn textures(&self) -> impl ExactSizeIterator<Item = (TextureId, TextureView)> + '_ {
        let generation = self.data.generation;
        StableEnumerate::new(self.data.textures.iter()).map(move |(index, texture)| {
            (
                TextureId::new(generation, index),
                (*texture.as_ref()).view(generation),
            )
        })
    }

    pub(crate) fn texture(&self, id: TextureId) -> Result<TextureView, SceneLookupError> {
        lookup_scene_wide(self.data.generation, id, &self.data.textures)
            .map(|texture| (*texture.as_ref()).view(self.data.generation))
    }

    pub(crate) fn skins(&self) -> impl ExactSizeIterator<Item = (SkinId, SkinView<'_>)> + '_ {
        let generation = self.data.generation;
        StableEnumerate::new(self.data.skins.iter()).map(move |(index, skin)| {
            (
                SkinId::new(generation, index),
                SkinView {
                    skin: skin.as_ref(),
                    generation,
                },
            )
        })
    }

    pub(crate) fn skin(&self, id: SkinId) -> Result<SkinView<'_>, SceneLookupError> {
        lookup_scene_wide(self.data.generation, id, &self.data.skins).map(|skin| SkinView {
            skin: skin.as_ref(),
            generation: self.data.generation,
        })
    }

    pub(crate) fn animation_clips(
        &self,
    ) -> impl ExactSizeIterator<Item = (AnimationClipId, AnimationClipView<'_>)> + '_ {
        let generation = self.data.generation;
        StableEnumerate::new(self.data.animation_clips.iter()).map(move |(index, clip)| {
            (
                AnimationClipId::new(generation, index),
                AnimationClipView {
                    clip: clip.as_ref(),
                },
            )
        })
    }

    pub(crate) fn animation_clip(
        &self,
        id: AnimationClipId,
    ) -> Result<AnimationClipView<'_>, SceneLookupError> {
        lookup_scene_wide(self.data.generation, id, &self.data.animation_clips).map(|clip| {
            AnimationClipView {
                clip: clip.as_ref(),
            }
        })
    }

    pub(crate) fn cameras(&self) -> impl ExactSizeIterator<Item = (CameraId, &Camera)> + '_ {
        let generation = self.data.generation;
        StableEnumerate::new(self.data.cameras.iter())
            .map(move |(index, camera)| (CameraId::new(generation, index), camera.as_ref()))
    }

    pub(crate) fn camera(&self, id: CameraId) -> Result<&Camera, SceneLookupError> {
        lookup_scene_wide(self.data.generation, id, &self.data.cameras).map(SharedStorage::as_ref)
    }

    pub(crate) fn lights(&self) -> impl ExactSizeIterator<Item = (LightId, &Light)> + '_ {
        let generation = self.data.generation;
        StableEnumerate::new(self.data.lights.iter())
            .map(move |(index, light)| (LightId::new(generation, index), light.as_ref()))
    }

    pub(crate) fn light(&self, id: LightId) -> Result<&Light, SceneLookupError> {
        lookup_scene_wide(self.data.generation, id, &self.data.lights).map(SharedStorage::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{AnimationLimits, AnimationTimeDomain};
    use crate::appearance::{AlphaMode, MaterialFactors, ShadingModel};
    use crate::attribute::{
        Attribute, AttributeBuffer, AttributeDomain, AttributeFormat, AttributePayload,
        AttributeSemantic, ComponentType, Normalization,
    };
    use crate::deform::{Skin, SkinMatrixSpaces};
    use crate::derived::DerivedAbsenceReason;
    use crate::entity::{AuthoredTransform, Camera, CameraProjection, Light, LightKind, Node};
    use crate::extension::ExtensionSchemaSnapshot;
    use crate::geometry::{
        IndexStream, LogicalMeshBounds, MeshPrimitive, MorphBoundsPolicy, PrimitiveBounds, Topology,
    };
    use crate::image::{Image, Sampler, Texture};
    use crate::math::{Color, Mat4, Vec3};
    use crate::provenance::{Provenance, test_provenance_fixture};
    use crate::scene_space::SceneSpace;
    use crate::shared_storage::fail_next_allocation_for_test;
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

    fn positions(values: Vec<Vec3>, provenance: Provenance) -> Attribute {
        Attribute::new(
            AttributeSemantic::Position,
            AttributeDomain::Vertex,
            AttributeFormat::new(ComponentType::F32, 3, Normalization::None, None).unwrap(),
            AttributePayload::dense(AttributeBuffer::from_f32_vec3(values).unwrap()),
            provenance,
        )
        .unwrap()
    }

    fn primitive(
        values: Vec<Vec3>,
        material: MaterialIndex,
        provenance: Provenance,
    ) -> MeshPrimitive {
        let vertex_count = u32::try_from(values.len()).expect("fixture vertex count should fit");
        MeshPrimitive::try_new(
            vertex_count,
            Topology::Points,
            IndexStream::non_indexed(),
            Some(material),
            vec![positions(values, provenance)],
            None,
            Vec::new(),
            1,
            PrimitiveBounds::absent(
                DerivedAbsenceReason::NotProvided,
                MorphBoundsPolicy::DefaultWeights,
            ),
            provenance,
        )
        .unwrap()
    }

    fn fully_populated_scene() -> CanonicalValidatedScene {
        let (provenance_arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .unwrap();

        let camera = builder
            .try_push_camera(
                Camera::try_new(
                    None,
                    CameraProjection::perspective(1.0, None, 0.1, None).unwrap(),
                    provenance,
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        let light = builder
            .try_push_light(
                Light::try_new(
                    None,
                    LightKind::Directional,
                    Color::linear_rgba(1.0, 1.0, 1.0, 1.0),
                    2.0,
                    provenance,
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        let root = builder
            .try_push_node(
                Node::try_new(
                    None,
                    AuthoredTransform::identity_trs(),
                    [NodeIndex::from_u32(1)],
                    Some(camera),
                    Some(light),
                    provenance,
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        let child = builder.try_push_node(root_node(provenance)).unwrap();
        assert_eq!(child.as_u32(), 1);
        builder.set_root(root);

        let image = builder
            .try_push_image(
                Image::try_encoded([1, 2, 3], None, None, None, None, provenance).unwrap(),
            )
            .unwrap();
        let sampler = builder
            .try_push_sampler(Sampler::canonical_default(provenance))
            .unwrap();
        builder
            .try_push_texture(Texture::new(image, sampler, provenance))
            .unwrap();
        let material = builder
            .try_push_material(
                Material::try_new(
                    None,
                    ShadingModel::Unlit,
                    AlphaMode::Opaque,
                    false,
                    MaterialFactors::empty(),
                    [],
                    provenance,
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        let logical_mesh = builder
            .try_push_logical_mesh(
                LogicalMesh::try_new(
                    None,
                    vec![
                        primitive(
                            vec![Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)],
                            material,
                            provenance,
                        ),
                        primitive(vec![Vec3::new(7.0, 8.0, 9.0)], material, provenance),
                    ],
                    [crate::deform::MorphTargetDefinition::try_new(None, provenance).unwrap()]
                        .into(),
                    vec![0.0],
                    LogicalMeshBounds::absent(
                        DerivedAbsenceReason::NotProvided,
                        MorphBoundsPolicy::DefaultWeights,
                    ),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();
        let skin = builder
            .try_push_skin(
                Skin::try_new(
                    None,
                    [root],
                    [Mat4::IDENTITY],
                    Some(root),
                    SkinMatrixSpaces::canonical(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();
        builder
            .try_push_mesh_instance(
                MeshInstance::try_new(
                    root,
                    logical_mesh,
                    Some(skin),
                    1,
                    None,
                    Vec::new(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();
        builder
            .try_push_animation_clip(
                AnimationClip::try_new(
                    0.0,
                    AnimationTimeDomain::Seconds,
                    [],
                    provenance,
                    &AnimationLimits::canonical(),
                )
                .unwrap(),
            )
            .unwrap();

        crate::canonical_validation::validate_canonical_scene(
            builder.finish_draft(),
            SceneValidationLimits::default(),
        )
        .expect("fully populated canonical scene should validate")
    }

    #[test]
    fn builder_reports_the_failed_shared_scene_payload() {
        let (provenance_arena, provenance) = test_provenance_fixture();
        fail_next_allocation_for_test();

        let error = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .err()
        .expect("the injected shared allocation failure must be reported");

        assert_eq!(
            error,
            CanonicalSceneBuildError::SharedPayloadAllocationFailed {
                payload: CanonicalSceneSharedPayload::Provenance,
            }
        );
    }

    #[test]
    fn failed_metadata_replacement_preserves_existing_shared_payload() {
        let (provenance_arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene builder fixture should allocate");
        let original = builder.data.metadata.clone();
        fail_next_allocation_for_test();

        let error = builder
            .set_metadata(SceneMetadata::empty())
            .expect_err("the injected metadata allocation failure must be reported");

        assert_eq!(
            error,
            CanonicalSceneBuildError::SharedPayloadAllocationFailed {
                payload: CanonicalSceneSharedPayload::Metadata,
            }
        );
        assert!(builder.data.metadata.ptr_eq(&original));
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

    #[test]
    fn validated_scene_qualifies_every_scene_wide_collection_and_root() {
        let scene = fully_populated_scene();
        let generation = scene.generation();
        let root = scene.root().expect("fixture has one root");
        assert_eq!(root.generation(), generation);

        let nodes = scene.nodes().collect::<Vec<_>>();
        assert_eq!(nodes.len(), 2);
        let (node_id, node) = nodes[0];
        assert_eq!(node_id, root);
        assert_eq!(node.children().next(), Some(nodes[1].0));
        assert_eq!(
            scene.node(node_id).unwrap().camera().unwrap().generation(),
            generation
        );
        assert_eq!(node.light().unwrap().generation(), generation);

        let (instance_id, instance) = scene.mesh_instances().next().unwrap();
        assert_eq!(scene.mesh_instance(instance_id).unwrap().owner(), root);
        assert_eq!(instance.logical_mesh().generation(), generation);
        assert_eq!(instance.skin().unwrap().generation(), generation);

        let (mesh_id, _) = scene.logical_meshes().next().unwrap();
        assert_eq!(scene.logical_mesh(mesh_id).unwrap().id(), mesh_id);

        let (material_id, _) = scene.materials().next().unwrap();
        assert!(!scene.material(material_id).unwrap().double_sided());

        let (image_id, _) = scene.images().next().unwrap();
        assert_eq!(scene.image(image_id).unwrap().encoded().bytes(), [1, 2, 3]);

        let (sampler_id, _) = scene.samplers().next().unwrap();
        assert_eq!(
            scene.sampler(sampler_id).unwrap().state(),
            crate::image::SamplerState::CANONICAL_DEFAULT
        );

        let (texture_id, texture) = scene.textures().next().unwrap();
        assert_eq!(scene.texture(texture_id).unwrap(), texture);
        assert_eq!(texture.image(), image_id);
        assert_eq!(texture.sampler(), sampler_id);

        let (skin_id, skin) = scene.skins().next().unwrap();
        assert_eq!(scene.skin(skin_id).unwrap().joints().next(), Some(root));
        assert_eq!(skin.skeleton_root_hint(), Some(root));

        let (clip_id, _) = scene.animation_clips().next().unwrap();
        assert_eq!(scene.animation_clip(clip_id).unwrap().duration(), 0.0);

        let (camera_id, _) = scene.cameras().next().unwrap();
        assert!(matches!(
            scene.camera(camera_id).unwrap().projection().kind(),
            crate::entity::CameraProjectionKind::Perspective
        ));

        let (light_id, _) = scene.lights().next().unwrap();
        assert_eq!(scene.light(light_id).unwrap().intensity(), 2.0);

        assert_eq!(scene.nodes().len(), 2);
        for count in [
            scene.mesh_instances().len(),
            scene.logical_meshes().len(),
            scene.materials().len(),
            scene.images().len(),
            scene.samplers().len(),
            scene.textures().len(),
            scene.skins().len(),
            scene.animation_clips().len(),
            scene.cameras().len(),
            scene.lights().len(),
        ] {
            assert_eq!(count, 1);
        }
    }

    #[test]
    fn logical_mesh_views_preserve_owned_ids_and_typed_position_storage() {
        let scene = fully_populated_scene();
        let (mesh_id, mesh) = scene.logical_meshes().next().unwrap();
        let primitives = mesh.primitives().collect::<Vec<_>>();
        assert_eq!(primitives.len(), 2);

        for (primitive_id, primitive) in &primitives {
            assert_eq!(primitive_id.logical_mesh(), mesh_id);
            assert_eq!(primitive.id(), *primitive_id);
            assert_eq!(
                scene
                    .logical_mesh(mesh_id)
                    .unwrap()
                    .primitive(*primitive_id)
                    .unwrap()
                    .id(),
                *primitive_id
            );
        }

        let first = scene
            .logical_mesh(mesh_id)
            .unwrap()
            .primitive(primitives[0].0)
            .unwrap();
        let typed = first.positions().unwrap();
        let dense = first.as_dense_positions().unwrap();
        assert_eq!(typed.as_typed_slice().unwrap().as_ptr(), dense.as_ptr());
        assert_eq!(typed.get(1), Some(Vec3::new(4.0, 5.0, 6.0)));

        let (morph_id, _) = mesh.morph_targets().next().unwrap();
        assert_eq!(morph_id.logical_mesh(), mesh_id);
        assert!(mesh.morph_target(morph_id).is_ok());
    }

    #[test]
    fn read_boundary_rejects_stale_out_of_range_and_foreign_owner_handles_first() {
        let scene = fully_populated_scene();
        let generation = scene.generation();
        let foreign_generation = SceneGenerationId::allocate().unwrap();
        let (mesh_id, mesh) = scene.logical_meshes().next().unwrap();

        assert!(matches!(
            scene.node(NodeId::new(foreign_generation, u32::MAX)),
            Err(SceneLookupError::GenerationMismatch { .. })
        ));
        assert!(matches!(
            scene.node(NodeId::new(generation, u32::MAX)),
            Err(SceneLookupError::OutOfRange { .. })
        ));

        let stale_owner = LogicalMeshId::new(foreign_generation, u32::MAX);
        assert!(matches!(
            mesh.primitive(PrimitiveId::new(stale_owner, u32::MAX)),
            Err(SceneLookupError::GenerationMismatch { .. })
        ));
        assert!(matches!(
            mesh.morph_target(MorphTargetId::new(stale_owner, u32::MAX)),
            Err(SceneLookupError::GenerationMismatch { .. })
        ));

        let foreign_owner = LogicalMeshId::new(generation, mesh_id.index() + 1);
        assert!(matches!(
            mesh.primitive(PrimitiveId::new(foreign_owner, u32::MAX)),
            Err(SceneLookupError::OwnerMismatch { .. })
        ));
        assert!(matches!(
            mesh.morph_target(MorphTargetId::new(foreign_owner, u32::MAX)),
            Err(SceneLookupError::OwnerMismatch { .. })
        ));
        assert!(matches!(
            mesh.primitive(PrimitiveId::new(mesh_id, u32::MAX)),
            Err(SceneLookupError::OutOfRange { .. })
        ));
        assert!(matches!(
            mesh.morph_target(MorphTargetId::new(mesh_id, u32::MAX)),
            Err(SceneLookupError::OutOfRange { .. })
        ));
    }
}
