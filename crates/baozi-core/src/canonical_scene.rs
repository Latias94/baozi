use crate::animation::{AnimationClip, AnimationTimeDomain};
use crate::appearance::{CanonicalMaterialDescriptor, Material, MaterialView};
use crate::attribute::{Attribute, AttributeDomain, F32Vec3View};
use crate::canonical_edit::{
    CanonicalEditError, CanonicalEditFailure, CanonicalNodeTransformEdit,
    CanonicalNodeTransformEditStart,
};
use crate::deform::{
    CanonicalSkinDescriptor, InfluenceRows, MorphTargetDefinition, PrimitiveMorphTarget, Skin,
    SkinMatrixSpaces,
};
use crate::derived::{Derived, DerivedAbsenceReason};
use crate::entity::{AuthoredTransform, Camera, EntityName, Light};
use crate::extension::{
    ControlledRetainedFootprintError, ExtensionEntry, ExtensionEntryClassification,
    ExtensionEntryValidationError, ExtensionSchemaSnapshot,
    extension_entries_retained_allocation_bytes_controlled, validate_unique_extension_entry_keys,
};
use crate::geometry::{
    CanonicalMeshInstanceDescriptor, CanonicalMeshInstanceResolveError, GeometryBoundsError,
    IndexStream, LogicalMesh, LogicalMeshBounds, MeshInstance, MeshInstanceError, MeshPrimitive,
    PrimitiveBounds, PrimitiveCardinality, Topology, validate_aabb,
};
use crate::identity::{
    AnimationClipId, AnimationClipIndex, CameraId, CameraIndex, ImageId, ImageIndex, LightId,
    LightIndex, LogicalMeshId, LogicalMeshIndex, MaterialId, MaterialIndex, MeshInstanceId,
    MeshInstanceIndex, MorphTargetId, NodeId, NodeIndex, PrimitiveId, SamplerId, SamplerIndex,
    SceneGenerationError, SceneGenerationId, SceneLookupError, SceneStorageIndexError, SkinId,
    SkinIndex, TextureId, TextureIndex, lookup_owned, lookup_scene_wide,
};
use crate::image::{
    CanonicalTextureDescriptor, Image, ImageView, Sampler, SamplerView, Texture, TextureView,
};
use crate::math::{Aabb, Mat4, Vec3};
use crate::provenance::{Provenance, ProvenanceArena, ProvenanceAttachments};
use crate::scene_space::SceneSpace;
use crate::shared_storage::{SharedStorage, SharedStorageError};
use crate::validation_control::{
    SceneValidationControl, SceneValidationLimits, SceneValidationStats,
};
use baozi_operation::{CapacityPlanError, GrowableCapacityPlan, ReplacementCapacityPlan};
use std::convert::Infallible;
use thiserror::Error;

mod node;
pub(crate) mod store;
pub(crate) use node::batch::CanonicalNodeDescriptor;
pub use node::{Node, NodeError};

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
    let plan = GrowableCapacityPlan::for_type::<ExtensionEntry>(
        entries.len(),
        entries.capacity(),
        desired,
    )
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

pub(crate) mod builder;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalPreparedAllocation {
    Collection,
    ValueStorage,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalSceneBuildError {
    #[error(transparent)]
    Generation(#[from] SceneGenerationError),
    #[error(transparent)]
    Index(#[from] SceneStorageIndexError),
    #[error(transparent)]
    Reference(#[from] SceneLookupError),
    #[error(transparent)]
    Node(#[from] NodeError),
    #[error(transparent)]
    MeshInstance(#[from] MeshInstanceError),
    #[error("canonical scene collection size overflowed for {collection:?}")]
    CollectionSizeOverflow {
        collection: CanonicalSceneCollection,
    },
    #[error("canonical scene allocation failed for {collection:?}")]
    AllocationFailed {
        collection: CanonicalSceneCollection,
    },
    #[error("canonical scene capacity plan for {actual:?} cannot prepare {expected:?}")]
    CapacityPlanCollectionMismatch {
        expected: CanonicalSceneCollection,
        actual: CanonicalSceneCollection,
    },
    #[error("canonical scene capacity plan failed for {collection:?}")]
    CapacityPlan {
        collection: CanonicalSceneCollection,
        #[source]
        cause: CapacityPlanError,
    },
    #[error(
        "canonical scene prepared {allocation:?} allocation for {collection:?} retained {actual} bytes beyond admitted {admitted} bytes"
    )]
    PreparedAllocationUnderestimated {
        collection: CanonicalSceneCollection,
        allocation: CanonicalPreparedAllocation,
        admitted: u64,
        actual: u64,
    },
    #[error(
        "canonical node batch requires pristine node storage, observed len/capacity {length}/{capacity} and root_set={root_set}"
    )]
    NodeBatchNotPristine {
        length: usize,
        capacity: usize,
        root_set: bool,
    },
    #[error("canonical node batch must contain at least one node")]
    EmptyNodeBatch,
    #[error("canonical node batch plan belongs to a different scene generation")]
    ForeignNodeBatchPlan,
    #[error("node slot belongs to a different scoped batch")]
    ForeignNodeBatchSlot,
    #[error("node slot {actual} is out of range for a batch of {node_count} nodes")]
    NodeBatchSlotOutOfRange { actual: usize, node_count: usize },
    #[error(
        "node batch must initialize batch ordinals monotonically; expected {expected}, observed {actual}"
    )]
    NodeBatchInitializationOrder { expected: usize, actual: usize },
    #[error(
        "prepared node child count differs from its plan; expected {expected}, observed {actual}"
    )]
    NodeBatchChildrenCountMismatch { expected: usize, actual: usize },
    #[error("node batch initialized {actual} of {expected} declared nodes")]
    IncompleteNodeBatch { expected: usize, actual: usize },
    #[error("complete node batch did not select a root")]
    MissingNodeBatchRoot,
    #[error("node batch root was already selected")]
    DuplicateNodeBatchRoot,
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
    use super::builder::{CanonicalPreparedSceneInsert, CanonicalSceneBuilder};
    use super::*;
    use crate::animation::{AnimationLimits, AnimationTimeDomain};
    use crate::appearance::{
        AlphaMode, CanonicalTextureBindingDescriptor, ColorInterpretation, MaterialFactors,
        ShadingModel, TextureBlendOperation, TextureChannelMapping, TextureChannelSwizzle,
        TextureCoordinateSource, TextureRole, TextureStrength, TextureTransform,
    };
    use crate::attribute::{
        Attribute, AttributeBuffer, AttributeDomain, AttributeFormat, AttributePayload,
        AttributeSemantic, ComponentType, Normalization,
    };
    use crate::deform::SkinMatrixSpaces;
    use crate::derived::DerivedAbsenceReason;
    use crate::entity::{
        AuthoredTransform, Camera, CameraProjection, Light, LightEmission, LightKind,
    };
    use crate::extension::ExtensionSchemaSnapshot;
    use crate::geometry::{
        IndexStream, LogicalMeshBounds, MeshPrimitive, MorphBoundsPolicy, PrimitiveBounds, Topology,
    };
    use crate::image::{CanonicalTextureDescriptor, Image, Sampler};
    use crate::math::{Color, Mat4, Vec3};
    use crate::provenance::{Provenance, test_provenance_fixture};
    use crate::scene_space::SceneSpace;
    use crate::shared_storage::fail_next_allocation_for_test;
    use crate::validation_control::SceneValidationLimits;

    fn root_node(provenance: crate::provenance::Provenance) -> Node {
        Node::try_new_unchecked_references_for_test(
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

    fn unlit_material(provenance: Provenance) -> CanonicalMaterialDescriptor {
        CanonicalMaterialDescriptor::try_new(
            None,
            ShadingModel::Unlit,
            AlphaMode::Opaque,
            false,
            MaterialFactors::empty(),
            [],
            provenance,
            [],
        )
        .expect("material fixture should be valid")
    }

    fn base_color_binding(
        texture: TextureId,
        provenance: Provenance,
    ) -> CanonicalTextureBindingDescriptor {
        CanonicalTextureBindingDescriptor::try_new(
            texture,
            TextureRole::BaseColor,
            TextureCoordinateSource::uv_set(0).expect("UV set zero should be valid"),
            TextureChannelMapping::swizzle(TextureChannelSwizzle::RGBA),
            ColorInterpretation::SrgbColor,
            TextureTransform::IDENTITY,
            TextureStrength::ONE,
            TextureBlendOperation::Replace,
            None,
            provenance,
            [],
        )
        .expect("base-color binding fixture should be valid")
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
                    LightEmission::try_from_scaled_linear_srgb(Color::WHITE, 2.0).unwrap(),
                    provenance,
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        let node_plan = builder.node_batch_plan(2).unwrap();
        let committed_nodes = builder
            .prepare_node_batch(node_plan, |batch| {
                let root = batch.slot(0)?;
                let child = batch.slot(1)?;
                let root_children_plan = batch.node_children_plan(1)?;
                let root_children = batch.prepare_node_children(root_children_plan, [child])?;
                let root_descriptor = CanonicalNodeDescriptor::new(
                    None,
                    AuthoredTransform::identity_trs(),
                    root_children,
                    Some(camera),
                    Some(light),
                    provenance.into(),
                    Vec::new(),
                );
                batch.initialize_node(root, root_descriptor)?;

                let child_children_plan = batch.node_children_plan(0)?;
                let child_children = batch.prepare_node_children(child_children_plan, [])?;
                let child_descriptor = CanonicalNodeDescriptor::new(
                    None,
                    AuthoredTransform::identity_trs(),
                    child_children,
                    None,
                    None,
                    provenance.into(),
                    Vec::new(),
                );
                batch.initialize_node(child, child_descriptor)?;
                batch.set_root(root)
            })
            .unwrap()
            .commit();
        let root_id = committed_nodes.root();
        assert_eq!(committed_nodes.node_id(1).unwrap().index(), 1);
        let image = builder
            .try_push_image(
                Image::try_encoded([1, 2, 3], None, None, None, None, provenance).unwrap(),
            )
            .unwrap();
        let sampler = builder
            .try_push_sampler(Sampler::canonical_default(provenance))
            .unwrap();
        let texture = builder
            .try_push_texture(CanonicalTextureDescriptor::new(image, sampler, provenance))
            .unwrap();
        let material_id = builder
            .try_push_material(
                CanonicalMaterialDescriptor::try_new(
                    None,
                    ShadingModel::Unlit,
                    AlphaMode::Opaque,
                    false,
                    MaterialFactors::empty(),
                    [base_color_binding(texture, provenance)],
                    provenance,
                    [],
                )
                .unwrap(),
            )
            .unwrap();
        let material = MaterialIndex::from_handle(builder.generation(), material_id).unwrap();
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
        let skin_id = builder
            .try_push_skin(
                CanonicalSkinDescriptor::try_new(
                    None,
                    [root_id],
                    [Mat4::IDENTITY],
                    Some(root_id),
                    SkinMatrixSpaces::canonical(),
                    provenance,
                )
                .unwrap(),
            )
            .unwrap();
        builder
            .try_push_mesh_instance(CanonicalMeshInstanceDescriptor::new(
                root_id,
                logical_mesh,
                Some(skin_id),
                None,
                Vec::new(),
                provenance,
            ))
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
            .try_push_node_unchecked_for_test(root_node(provenance))
            .expect("root allocation should succeed");
        builder.set_root_unchecked_for_test(root);
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
    fn prepared_node_insert_keeps_the_builder_unchanged_until_commit() {
        let (provenance_arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene builder fixture should allocate");
        let plan = builder
            .collection_capacity_plan(CanonicalSceneCollection::Nodes)
            .expect("node insertion should have a capacity plan");
        let admitted = plan
            .collection_envelope_delta_bytes()
            .expect("node insertion collection envelope should fit");

        let prepared = builder
            .prepare_push_node_unchecked_for_test(root_node(provenance), plan)
            .expect("node insertion should prepare off to the side");
        assert!(prepared.replaces_collection_allocation());
        assert!(prepared.collection_retained_allocation_bytes() <= admitted);
        assert_eq!(prepared.value_storage_retained_allocation_bytes(), 0);
        drop(prepared);
        assert!(builder.data.nodes.is_empty());

        let plan = builder
            .collection_capacity_plan(CanonicalSceneCollection::Nodes)
            .expect("retry should have a fresh capacity plan");
        let prepared = builder
            .prepare_push_node_unchecked_for_test(root_node(provenance), plan)
            .expect("retry should prepare");
        let node = prepared.commit();

        assert_eq!(node.as_u32(), 0);
        assert_eq!(builder.data.nodes.len(), 1);
    }

    #[test]
    fn prepared_node_insert_uses_spare_capacity_only_at_commit() {
        let (provenance_arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene builder fixture should allocate");
        builder
            .try_push_node_unchecked_for_test(root_node(provenance))
            .expect("fixture should establish reusable node capacity");
        builder.data.nodes.clear();
        let original_capacity = builder.data.nodes.capacity();
        assert!(original_capacity > 0);

        let plan = builder
            .collection_capacity_plan(CanonicalSceneCollection::Nodes)
            .expect("spare node capacity should have a plan");
        assert!(!plan.replaces_collection_allocation());
        let prepared = builder
            .prepare_push_node_unchecked_for_test(root_node(provenance), plan)
            .expect("spare capacity should prepare without replacement storage");
        assert!(!prepared.replaces_collection_allocation());
        assert_eq!(prepared.collection_retained_allocation_bytes(), 0);
        assert_eq!(prepared.value_storage_retained_allocation_bytes(), 0);
        drop(prepared);
        assert!(builder.data.nodes.is_empty());
        assert_eq!(builder.data.nodes.capacity(), original_capacity);

        let plan = builder
            .collection_capacity_plan(CanonicalSceneCollection::Nodes)
            .expect("retry should have a fresh capacity plan");
        let node = builder
            .prepare_push_node_unchecked_for_test(root_node(provenance), plan)
            .expect("retry should prepare")
            .commit();

        assert_eq!(node.as_u32(), 0);
        assert_eq!(builder.data.nodes.len(), 1);
        assert_eq!(builder.data.nodes.capacity(), original_capacity);
    }

    #[test]
    fn prepared_insert_rejects_wrong_and_stale_capacity_plans_without_mutation() {
        let (provenance_arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene builder fixture should allocate");
        let wrong = builder
            .collection_capacity_plan(CanonicalSceneCollection::Materials)
            .expect("material insertion should have a capacity plan");
        let wrong_error =
            match builder.prepare_push_node_unchecked_for_test(root_node(provenance), wrong) {
                Ok(_) => panic!("a material capacity plan must not prepare a node"),
                Err(error) => error,
            };
        assert_eq!(
            wrong_error,
            CanonicalSceneBuildError::CapacityPlanCollectionMismatch {
                expected: CanonicalSceneCollection::Nodes,
                actual: CanonicalSceneCollection::Materials,
            }
        );
        assert!(builder.data.nodes.is_empty());

        let stale = builder
            .collection_capacity_plan(CanonicalSceneCollection::Nodes)
            .expect("node insertion should have a capacity plan");
        builder
            .try_push_node_unchecked_for_test(root_node(provenance))
            .expect("intervening insertion should succeed");
        let stale_error =
            match builder.prepare_push_node_unchecked_for_test(root_node(provenance), stale) {
                Ok(_) => panic!("a stale capacity plan must not prepare a node"),
                Err(error) => error,
            };
        assert!(matches!(
            stale_error,
            CanonicalSceneBuildError::CapacityPlan {
                collection: CanonicalSceneCollection::Nodes,
                cause: baozi_operation::CapacityPlanError::StalePlan { .. },
            }
        ));
        assert_eq!(builder.data.nodes.len(), 1);
    }

    #[test]
    fn prepared_shared_insert_accounts_storage_and_publishes_only_at_commit() {
        let (provenance_arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene builder fixture should allocate");
        let plan = builder
            .collection_capacity_plan(CanonicalSceneCollection::Materials)
            .expect("material insertion should have a capacity plan");
        let collection_admitted = plan
            .collection_envelope_delta_bytes()
            .expect("material collection envelope should fit");
        let value_admitted = plan.value_storage_envelope_bytes();

        let prepared = builder
            .prepare_push_material(unlit_material(provenance), plan)
            .expect("shared material insertion should prepare");
        assert!(prepared.replaces_collection_allocation());
        assert!(prepared.collection_retained_allocation_bytes() > 0);
        assert!(prepared.collection_retained_allocation_bytes() <= collection_admitted);
        assert!(prepared.value_storage_retained_allocation_bytes() > 0);
        assert!(prepared.value_storage_retained_allocation_bytes() <= value_admitted);
        drop(prepared);
        assert!(builder.data.materials.is_empty());
        assert_eq!(builder.data.materials.capacity(), 0);

        let plan = builder
            .collection_capacity_plan(CanonicalSceneCollection::Materials)
            .expect("retry should have a fresh capacity plan");
        let material = builder
            .prepare_push_material(unlit_material(provenance), plan)
            .expect("retry should prepare")
            .commit();

        assert_eq!(material.generation(), builder.generation());
        assert_eq!(material.index(), 0);
        assert_eq!(builder.data.materials.len(), 1);
    }

    #[test]
    fn prepared_shared_insert_keeps_value_storage_separate_from_spare_collection_capacity() {
        let (provenance_arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene builder fixture should allocate");
        builder
            .try_push_material(unlit_material(provenance))
            .expect("fixture should establish reusable material capacity");
        builder.data.materials.clear();

        let plan = builder
            .collection_capacity_plan(CanonicalSceneCollection::Materials)
            .expect("spare material capacity should have a plan");
        assert!(!plan.replaces_collection_allocation());
        assert_eq!(
            plan.collection_envelope_delta_bytes()
                .expect("spare collection envelope should fit"),
            0
        );
        assert!(plan.value_storage_envelope_bytes() > 0);

        let prepared = builder
            .prepare_push_material(unlit_material(provenance), plan)
            .expect("shared material should prepare in spare collection capacity");
        assert!(!prepared.replaces_collection_allocation());
        assert_eq!(prepared.collection_retained_allocation_bytes(), 0);
        assert!(prepared.value_storage_retained_allocation_bytes() > 0);
        assert!(
            prepared.value_storage_retained_allocation_bytes()
                <= plan.value_storage_envelope_bytes()
        );
        prepared.commit();

        assert_eq!(builder.data.materials.len(), 1);
    }

    #[test]
    fn failed_shared_value_preparation_preserves_spare_collection_capacity() {
        let (provenance_arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene builder fixture should allocate");
        builder
            .try_push_material(unlit_material(provenance))
            .expect("fixture should establish reusable material capacity");
        builder.data.materials.clear();
        let original_capacity = builder.data.materials.capacity();

        let mut underestimated = builder
            .collection_capacity_plan(CanonicalSceneCollection::Materials)
            .expect("spare material capacity should have a plan");
        underestimated.prepared_value_storage_bytes = 0;
        let error = match builder.prepare_push_material(unlit_material(provenance), underestimated)
        {
            Ok(_) => panic!("underestimated shared value storage must not prepare"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            CanonicalSceneBuildError::PreparedAllocationUnderestimated {
                collection: CanonicalSceneCollection::Materials,
                allocation: CanonicalPreparedAllocation::ValueStorage,
                ..
            }
        ));
        assert!(builder.data.materials.is_empty());
        assert_eq!(builder.data.materials.capacity(), original_capacity);

        let plan = builder
            .collection_capacity_plan(CanonicalSceneCollection::Materials)
            .expect("retry should have a fresh capacity plan");
        fail_next_allocation_for_test();
        let error = match builder.prepare_push_material(unlit_material(provenance), plan) {
            Ok(_) => panic!("the injected shared allocation failure must abort preparation"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            CanonicalSceneBuildError::AllocationFailed {
                collection: CanonicalSceneCollection::Materials,
            }
        );
        assert!(builder.data.materials.is_empty());
        assert_eq!(builder.data.materials.capacity(), original_capacity);

        let plan = builder
            .collection_capacity_plan(CanonicalSceneCollection::Materials)
            .expect("retry should have a fresh capacity plan");
        let material = builder
            .prepare_push_material(unlit_material(provenance), plan)
            .expect("builder should remain usable after failed preparation")
            .commit();
        assert_eq!(material.generation(), builder.generation());
        assert_eq!(material.index(), 0);
        assert_eq!(builder.data.materials.len(), 1);
        assert_eq!(builder.data.materials.capacity(), original_capacity);
    }

    #[test]
    fn underestimated_shared_storage_admission_preserves_the_live_collection() {
        let (provenance_arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene builder fixture should allocate");
        let mut plan = builder
            .collection_capacity_plan(CanonicalSceneCollection::Materials)
            .expect("material insertion should have a capacity plan");
        plan.prepared_value_storage_bytes = 0;
        let admitted = plan.value_storage_envelope_bytes();

        let error = match builder.prepare_push_material(unlit_material(provenance), plan) {
            Ok(_) => panic!("under-admitted shared storage must not prepare"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            CanonicalSceneBuildError::PreparedAllocationUnderestimated {
                collection: CanonicalSceneCollection::Materials,
                allocation: CanonicalPreparedAllocation::ValueStorage,
                admitted: actual_admitted,
                actual,
            } if actual_admitted == admitted && actual > admitted
        ));
        assert!(builder.data.materials.is_empty());
        assert_eq!(builder.data.materials.capacity(), 0);
    }

    #[test]
    fn failed_shared_value_preparation_does_not_grow_the_live_collection() {
        let (provenance_arena, provenance) = test_provenance_fixture();
        let mut builder = CanonicalSceneBuilder::try_new(
            SceneSpace::unknown(),
            ExtensionSchemaSnapshot::empty(),
            provenance_arena,
            provenance,
            DerivedAbsenceReason::NotProvided,
        )
        .expect("scene builder fixture should allocate");
        let plan = builder
            .collection_capacity_plan(CanonicalSceneCollection::Materials)
            .expect("material insertion should have a capacity plan");
        let material = unlit_material(provenance);
        fail_next_allocation_for_test();

        let error = match builder.prepare_push_material(material, plan) {
            Ok(_) => panic!("the injected shared allocation failure must abort preparation"),
            Err(error) => error,
        };

        assert_eq!(
            error,
            CanonicalSceneBuildError::AllocationFailed {
                collection: CanonicalSceneCollection::Materials,
            }
        );
        assert!(builder.data.materials.is_empty());
        assert_eq!(builder.data.materials.capacity(), 0);
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

        let (material_id, material) = scene.materials().next().unwrap();
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
        assert_eq!(
            material
                .texture_bindings()
                .next()
                .expect("fixture material should retain one binding")
                .texture(),
            texture_id
        );

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
        assert_eq!(scene.light(light_id).unwrap().emission().intensity(), 2.0);

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
