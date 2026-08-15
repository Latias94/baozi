#![allow(
    clippy::result_large_err,
    reason = "profile authoring failures retain bounded control evidence inline without a failure-path allocation"
)]

use baozi_operation::{
    CapacityPlan, CapacityPlanError, OperationOwner, ProgressPhase, ProgressUnit,
    RetainedStoreReservation,
};
use std::error::Error;
use thiserror::Error;

use crate::canonical_scene::store::profile_provenance::PROFILE_PROVENANCE_ADDRESS_LOGICAL_BYTES;
use crate::canonical_scene::store::schema::profile::{
    PROFILE_DIRECTORY_LOGICAL_BYTES, PROFILE_EVIDENCE_SCALAR_COUNT,
    PROFILE_METADATA_DIRECTORY_LOGICAL_BYTES, PROFILE_METADATA_ENTRY_LOGICAL_BYTES,
    PROFILE_OPERATION_SCALAR_COUNT, PROFILE_OWNER_LOGICAL_BYTES, PROFILE_PROVENANCE_SCALAR_COUNT,
    PROFILE_RESOURCE_SCALAR_COUNT, PROFILE_SEGMENT_LOGICAL_BYTES, PROFILE_SOURCE_SCALAR_COUNT,
};
use crate::extension::{ExtensionArrayRef, ExtensionValueRef};
use crate::provenance::{MAX_OPERATION_EVIDENCE_SUMMARIES, OperationKind, SourceObjectLocator};
use crate::{SCENE_VALIDATION_POLL_INTERVAL, SceneValidationLimits, SceneValidationWork};

use super::super::control::{
    CanonicalAuthoringControl, OperationAuthoringControl, OperationAuthoringControlError,
};
use super::super::input::ProfileAuthoringBatch;
use super::super::input::lineage::{
    LineageAuthoringBatch, ObservedResourceAuthoring, ProfileOwner, ProvenanceLocalId,
    ProvenanceNodeAuthoring, ProvenanceOperationAuthoring, ResourceIdentityStability,
    SourceIdentityAuthoring, SourceIdentityLocalId,
};
use super::super::input::metadata::{MetadataAuthoringBatch, MetadataAuthoringEntry};
use super::ValidatedNodeBatch;
use super::animation::ValidatedAnimationBatch;
use super::appearance::ValidatedAppearanceBatch;
use super::deformation::ValidatedDeformationBatch;
use super::geometry::ValidatedGeometryBatch;
use super::spatial::ValidatedSpatialBatch;

mod lineage;
mod metadata;

use lineage::*;
use metadata::validate_metadata;

const LOGICAL_U8_BYTES: u64 = 1;
const LOGICAL_U32_BYTES: u64 = 4;
const LOGICAL_U64_BYTES: u64 = 8;
const LOGICAL_F32_BYTES: u64 = 4;
const LOGICAL_F64_BYTES: u64 = 8;
const LOGICAL_REFERENCE_BYTES: u64 = 4;
const LOGICAL_DIGEST_BYTES: u64 = 33;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum ProfileAuthoringLimit {
    Nodes,
    Edges,
    Depth,
    Entities,
    References,
    PayloadCells,
    MetadataEntries,
    MetadataBytes,
    MetadataKeyBytes,
    MetadataStringBytes,
    CanonicalBytes,
    Work,
    ScratchBytes,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum ProfileOwnerCatalogError {
    #[error(
        "validated node mapping has {mapping_count} entries for {canonical_count} canonical nodes"
    )]
    NodeMappingMismatch {
        mapping_count: usize,
        canonical_count: u64,
    },
    #[error("profile owner catalog accounting overflowed")]
    AccountingOverflow,
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum ProfileAuthoringSemanticError {
    #[error("profile authoring count for {collection} exceeds the u32 canonical domain")]
    CountDomainTooLarge { collection: &'static str },
    #[error("{limit:?} value {actual} exceeds limit {maximum}")]
    LimitExceeded {
        limit: ProfileAuthoringLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("profile authoring accounting overflowed for {dimension}")]
    AccountingOverflow { dimension: &'static str },
    #[error(
        "source identity {source_index} references resource {resource} outside resource count {resource_count}"
    )]
    ResourceOutOfRange {
        source_index: usize,
        resource: u32,
        resource_count: u32,
    },
    #[error(
        "source provenance node {node} references source identity {source_index} outside source count {source_count}"
    )]
    SourceIdentityOutOfRange {
        node: u32,
        source_index: u32,
        source_count: u32,
    },
    #[error("provenance node {node} references parent {parent}, which is not before the child")]
    ParentNotBeforeChild { node: u32, parent: u32 },
    #[error("provenance node {node} repeats parent {parent}")]
    DuplicateParent { node: u32, parent: u32 },
    #[error("provenance node {node} parent {parent} is not in strictly increasing order")]
    ParentOrder { node: u32, parent: u32 },
    #[error("provenance operation {node} has {actual} evidence rows; maximum is {maximum}")]
    EvidenceLimitExceeded {
        node: u32,
        actual: usize,
        maximum: usize,
    },
    #[error("provenance operation {node} evidence is not strictly ordered at index {evidence}")]
    EvidenceOrder { node: u32, evidence: usize },
    #[error("profile owner {owner:?} is outside its validated domain")]
    OwnerOutOfRange { owner: ProfileOwner },
    #[error("profile owner {owner:?} has more than one provenance row")]
    DuplicateOwnerProvenance { owner: CanonicalProfileOwner },
    #[error("profile owner {owner:?} has no provenance row")]
    MissingOwnerProvenance { owner: CanonicalProfileOwner },
    #[error("profile owner {owner:?} has an empty provenance row")]
    EmptyOwnerProvenance { owner: CanonicalProfileOwner },
    #[error(
        "profile owner {owner:?} provenance attachment {current} must follow {previous} in strictly increasing order"
    )]
    OwnerProvenanceOrder {
        owner: CanonicalProfileOwner,
        previous: u32,
        current: u32,
    },
    #[error("{usage} references provenance {provenance} outside node count {provenance_count}")]
    ProvenanceOutOfRange {
        usage: &'static str,
        provenance: u32,
        provenance_count: u32,
    },
    #[error("metadata entry {entry} contains a non-finite value")]
    MetadataNonFinite { entry: usize },
    #[error("metadata entries {first} and {duplicate} repeat one owner-local key")]
    DuplicateMetadataKey { first: usize, duplicate: usize },
    #[error(
        "metadata entry {entry} key must follow entry {previous} for canonical owner {owner:?}"
    )]
    MetadataKeyOrder {
        previous: usize,
        entry: usize,
        owner: CanonicalProfileOwner,
    },
    #[error("scratch capacity planning failed for {resource}: {cause}")]
    ScratchCapacity {
        resource: &'static str,
        #[source]
        cause: CapacityPlanError,
    },
    #[error("scratch allocation failed for {resource}")]
    ScratchAllocation { resource: &'static str },
}

#[derive(Debug, Error)]
pub(in crate::canonical_scene::store) enum ProfileAuthoringFailure<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Semantic(#[from] ProfileAuthoringSemanticError),
    #[error("profile authoring validation control failed")]
    Control(#[source] E),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(in crate::canonical_scene::store) enum CanonicalProfileOwner {
    Scene,
    SyntheticRoot,
    Node(u32),
    Camera(u32),
    Light(u32),
    Image(u32),
    Sampler(u32),
    Texture(u32),
    TextureBinding(u32),
    Material(u32),
    LogicalMesh(u32),
    Primitive(u32),
    Attribute(u32),
    MeshInstance(u32),
    Skin(u32),
    MorphDefinition(u32),
    MorphDelta(u32),
    AnimationClip(u32),
    AnimationChannel(u32),
}

#[derive(Debug, Clone, Copy)]
struct ProfileOwnerCounts {
    nodes: u32,
    cameras: u32,
    lights: u32,
    images: u32,
    samplers: u32,
    textures: u32,
    texture_bindings: u32,
    materials: u32,
    logical_meshes: u32,
    primitives: u32,
    attributes: u32,
    mesh_instances: u32,
    skins: u32,
    morph_definitions: u32,
    morph_deltas: u32,
    animation_clips: u32,
    animation_channels: u32,
}

#[derive(Debug, Clone, Copy)]
struct ProfileOwnerOffsets {
    node: u64,
    camera: u64,
    light: u64,
    image: u64,
    sampler: u64,
    texture: u64,
    texture_binding: u64,
    material: u64,
    logical_mesh: u64,
    primitive: u64,
    attribute: u64,
    mesh_instance: u64,
    skin: u64,
    morph_definition: u64,
    morph_delta: u64,
    animation_clip: u64,
    animation_channel: u64,
    total: u64,
}

impl ProfileOwnerOffsets {
    fn new(counts: ProfileOwnerCounts) -> Result<Self, ProfileOwnerCatalogError> {
        let node = 2_u64;
        let camera = catalog_add(node, counts.nodes)?;
        let light = catalog_add(camera, counts.cameras)?;
        let image = catalog_add(light, counts.lights)?;
        let sampler = catalog_add(image, counts.images)?;
        let texture = catalog_add(sampler, counts.samplers)?;
        let texture_binding = catalog_add(texture, counts.textures)?;
        let material = catalog_add(texture_binding, counts.texture_bindings)?;
        let logical_mesh = catalog_add(material, counts.materials)?;
        let primitive = catalog_add(logical_mesh, counts.logical_meshes)?;
        let attribute = catalog_add(primitive, counts.primitives)?;
        let mesh_instance = catalog_add(attribute, counts.attributes)?;
        let skin = catalog_add(mesh_instance, counts.mesh_instances)?;
        let morph_definition = catalog_add(skin, counts.skins)?;
        let morph_delta = catalog_add(morph_definition, counts.morph_definitions)?;
        let animation_clip = catalog_add(morph_delta, counts.morph_deltas)?;
        let animation_channel = catalog_add(animation_clip, counts.animation_clips)?;
        let total = catalog_add(animation_channel, counts.animation_channels)?;
        Ok(Self {
            node,
            camera,
            light,
            image,
            sampler,
            texture,
            texture_binding,
            material,
            logical_mesh,
            primitive,
            attribute,
            mesh_instance,
            skin,
            morph_definition,
            morph_delta,
            animation_clip,
            animation_channel,
            total,
        })
    }
}

fn catalog_add(offset: u64, count: u32) -> Result<u64, ProfileOwnerCatalogError> {
    offset
        .checked_add(u64::from(count))
        .ok_or(ProfileOwnerCatalogError::AccountingOverflow)
}

/// Read-only owner directory derived from all six independently validated domains.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct ProfileOwnerCatalog<'nodes> {
    canonical_index_by_local: &'nodes [u32],
    counts: ProfileOwnerCounts,
    offsets: ProfileOwnerOffsets,
}

impl<'nodes> ProfileOwnerCatalog<'nodes> {
    pub(in crate::canonical_scene::store) fn from_validated<SR, NR, AR, GR, DR, XR>(
        spatial: &ValidatedSpatialBatch<'_, SR>,
        nodes: &'nodes ValidatedNodeBatch<'_, NR>,
        appearance: &ValidatedAppearanceBatch<'_, AR>,
        geometry: &ValidatedGeometryBatch<'_, GR>,
        deformation: &ValidatedDeformationBatch<'_, DR>,
        animation: &ValidatedAnimationBatch<'_, XR>,
    ) -> Result<Self, ProfileOwnerCatalogError> {
        let canonical_index_by_local = nodes.canonical_index_by_local();
        let canonical_count = u64::try_from(canonical_index_by_local.len())
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or(ProfileOwnerCatalogError::AccountingOverflow)?;
        if canonical_count != nodes.canonical_node_count() {
            return Err(ProfileOwnerCatalogError::NodeMappingMismatch {
                mapping_count: canonical_index_by_local.len(),
                canonical_count: nodes.canonical_node_count(),
            });
        }
        let counts = ProfileOwnerCounts {
            nodes: u32::try_from(canonical_index_by_local.len())
                .map_err(|_| ProfileOwnerCatalogError::AccountingOverflow)?,
            cameras: spatial.camera_count(),
            lights: spatial.light_count(),
            images: appearance.image_count(),
            samplers: appearance.sampler_count(),
            textures: appearance.texture_count(),
            texture_bindings: appearance.binding_count(),
            materials: appearance.material_count(),
            logical_meshes: geometry.logical_mesh_count(),
            primitives: geometry.primitive_count(),
            attributes: geometry.attribute_count(),
            mesh_instances: geometry.mesh_instance_count(),
            skins: deformation.skin_count(),
            morph_definitions: deformation.morph_definition_count(),
            morph_deltas: deformation.morph_delta_count(),
            animation_clips: animation.clip_count(),
            animation_channels: animation.channel_count(),
        };
        let offsets = ProfileOwnerOffsets::new(counts)?;
        Ok(Self {
            canonical_index_by_local,
            counts,
            offsets,
        })
    }

    pub(in crate::canonical_scene::store) const fn owner_count(self) -> u64 {
        self.offsets.total
    }

    pub(in crate::canonical_scene::store) fn owner_at(
        self,
        ordinal: u64,
    ) -> Option<CanonicalProfileOwner> {
        let owner = match ordinal {
            0 => CanonicalProfileOwner::Scene,
            1 => CanonicalProfileOwner::SyntheticRoot,
            value if value < self.offsets.camera => {
                CanonicalProfileOwner::Node(owner_index(value, self.offsets.node)?.checked_add(1)?)
            }
            value if value < self.offsets.light => {
                CanonicalProfileOwner::Camera(owner_index(value, self.offsets.camera)?)
            }
            value if value < self.offsets.image => {
                CanonicalProfileOwner::Light(owner_index(value, self.offsets.light)?)
            }
            value if value < self.offsets.sampler => {
                CanonicalProfileOwner::Image(owner_index(value, self.offsets.image)?)
            }
            value if value < self.offsets.texture => {
                CanonicalProfileOwner::Sampler(owner_index(value, self.offsets.sampler)?)
            }
            value if value < self.offsets.texture_binding => {
                CanonicalProfileOwner::Texture(owner_index(value, self.offsets.texture)?)
            }
            value if value < self.offsets.material => CanonicalProfileOwner::TextureBinding(
                owner_index(value, self.offsets.texture_binding)?,
            ),
            value if value < self.offsets.logical_mesh => {
                CanonicalProfileOwner::Material(owner_index(value, self.offsets.material)?)
            }
            value if value < self.offsets.primitive => {
                CanonicalProfileOwner::LogicalMesh(owner_index(value, self.offsets.logical_mesh)?)
            }
            value if value < self.offsets.attribute => {
                CanonicalProfileOwner::Primitive(owner_index(value, self.offsets.primitive)?)
            }
            value if value < self.offsets.mesh_instance => {
                CanonicalProfileOwner::Attribute(owner_index(value, self.offsets.attribute)?)
            }
            value if value < self.offsets.skin => {
                CanonicalProfileOwner::MeshInstance(owner_index(value, self.offsets.mesh_instance)?)
            }
            value if value < self.offsets.morph_definition => {
                CanonicalProfileOwner::Skin(owner_index(value, self.offsets.skin)?)
            }
            value if value < self.offsets.morph_delta => CanonicalProfileOwner::MorphDefinition(
                owner_index(value, self.offsets.morph_definition)?,
            ),
            value if value < self.offsets.animation_clip => {
                CanonicalProfileOwner::MorphDelta(owner_index(value, self.offsets.morph_delta)?)
            }
            value if value < self.offsets.animation_channel => {
                CanonicalProfileOwner::AnimationClip(owner_index(
                    value,
                    self.offsets.animation_clip,
                )?)
            }
            value if value < self.offsets.total => CanonicalProfileOwner::AnimationChannel(
                owner_index(value, self.offsets.animation_channel)?,
            ),
            _ => return None,
        };
        Some(owner)
    }

    fn normalize(
        self,
        owner: ProfileOwner,
    ) -> Result<(CanonicalProfileOwner, usize), ProfileAuthoringSemanticError> {
        let (canonical, ordinal) = match owner {
            ProfileOwner::Scene => (CanonicalProfileOwner::Scene, 0),
            ProfileOwner::SyntheticRoot => (CanonicalProfileOwner::SyntheticRoot, 1),
            ProfileOwner::Node(local) => {
                let local_index = usize::try_from(local.index())
                    .map_err(|_| ProfileAuthoringSemanticError::OwnerOutOfRange { owner })?;
                let canonical = *self
                    .canonical_index_by_local
                    .get(local_index)
                    .ok_or(ProfileAuthoringSemanticError::OwnerOutOfRange { owner })?;
                if canonical == 0 || canonical > self.counts.nodes {
                    return Err(ProfileAuthoringSemanticError::OwnerOutOfRange { owner });
                }
                (
                    CanonicalProfileOwner::Node(canonical),
                    self.offsets
                        .node
                        .checked_add(u64::from(canonical - 1))
                        .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                            dimension: "node owner ordinal",
                        })?,
                )
            }
            ProfileOwner::Camera(index) => indexed_owner(
                owner,
                index,
                self.counts.cameras,
                self.offsets.camera,
                CanonicalProfileOwner::Camera,
            )?,
            ProfileOwner::Light(index) => indexed_owner(
                owner,
                index,
                self.counts.lights,
                self.offsets.light,
                CanonicalProfileOwner::Light,
            )?,
            ProfileOwner::Image(index) => indexed_owner(
                owner,
                index,
                self.counts.images,
                self.offsets.image,
                CanonicalProfileOwner::Image,
            )?,
            ProfileOwner::Sampler(index) => indexed_owner(
                owner,
                index,
                self.counts.samplers,
                self.offsets.sampler,
                CanonicalProfileOwner::Sampler,
            )?,
            ProfileOwner::Texture(index) => indexed_owner(
                owner,
                index,
                self.counts.textures,
                self.offsets.texture,
                CanonicalProfileOwner::Texture,
            )?,
            ProfileOwner::TextureBinding(index) => indexed_owner(
                owner,
                index,
                self.counts.texture_bindings,
                self.offsets.texture_binding,
                CanonicalProfileOwner::TextureBinding,
            )?,
            ProfileOwner::Material(index) => indexed_owner(
                owner,
                index,
                self.counts.materials,
                self.offsets.material,
                CanonicalProfileOwner::Material,
            )?,
            ProfileOwner::LogicalMesh(index) => indexed_owner(
                owner,
                index,
                self.counts.logical_meshes,
                self.offsets.logical_mesh,
                CanonicalProfileOwner::LogicalMesh,
            )?,
            ProfileOwner::Primitive(index) => indexed_owner(
                owner,
                index,
                self.counts.primitives,
                self.offsets.primitive,
                CanonicalProfileOwner::Primitive,
            )?,
            ProfileOwner::Attribute(index) => indexed_owner(
                owner,
                index,
                self.counts.attributes,
                self.offsets.attribute,
                CanonicalProfileOwner::Attribute,
            )?,
            ProfileOwner::MeshInstance(index) => indexed_owner(
                owner,
                index,
                self.counts.mesh_instances,
                self.offsets.mesh_instance,
                CanonicalProfileOwner::MeshInstance,
            )?,
            ProfileOwner::Skin(index) => indexed_owner(
                owner,
                index,
                self.counts.skins,
                self.offsets.skin,
                CanonicalProfileOwner::Skin,
            )?,
            ProfileOwner::MorphDefinition(index) => indexed_owner(
                owner,
                index,
                self.counts.morph_definitions,
                self.offsets.morph_definition,
                CanonicalProfileOwner::MorphDefinition,
            )?,
            ProfileOwner::MorphDelta(index) => indexed_owner(
                owner,
                index,
                self.counts.morph_deltas,
                self.offsets.morph_delta,
                CanonicalProfileOwner::MorphDelta,
            )?,
            ProfileOwner::AnimationClip(index) => indexed_owner(
                owner,
                index,
                self.counts.animation_clips,
                self.offsets.animation_clip,
                CanonicalProfileOwner::AnimationClip,
            )?,
            ProfileOwner::AnimationChannel(index) => indexed_owner(
                owner,
                index,
                self.counts.animation_channels,
                self.offsets.animation_channel,
                CanonicalProfileOwner::AnimationChannel,
            )?,
        };
        let ordinal = usize::try_from(ordinal).map_err(|_| {
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "profile owner ordinal",
            }
        })?;
        Ok((canonical, ordinal))
    }
}

fn owner_index(ordinal: u64, offset: u64) -> Option<u32> {
    u32::try_from(ordinal.checked_sub(offset)?).ok()
}

fn indexed_owner(
    authored: ProfileOwner,
    index: u32,
    count: u32,
    offset: u64,
    canonical: impl FnOnce(u32) -> CanonicalProfileOwner,
) -> Result<(CanonicalProfileOwner, u64), ProfileAuthoringSemanticError> {
    if index >= count {
        return Err(ProfileAuthoringSemanticError::OwnerOutOfRange { owner: authored });
    }
    let ordinal = offset.checked_add(u64::from(index)).ok_or(
        ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "profile owner ordinal",
        },
    )?;
    Ok((canonical(index), ordinal))
}

#[derive(Debug, Clone, Copy)]
struct ValidatedOwnerProvenancePlan {
    owner: CanonicalProfileOwner,
    input_index: usize,
}

#[derive(Debug, Clone, Copy)]
struct ValidatedMetadataPlan {
    owner: CanonicalProfileOwner,
    input_index: usize,
}

struct ProfileValidationScratch<R> {
    provenance_depths: Vec<u32>,
    owner_plans: Vec<ValidatedOwnerProvenancePlan>,
    metadata_owner_counts: Vec<u32>,
    metadata_owner_cursors: Vec<u32>,
    metadata_last_inputs: Vec<u32>,
    metadata_plans: Vec<ValidatedMetadataPlan>,
    // Every scratch allocation above must drop before its accounting token.
    reservation: R,
}

/// One validated source identity joined to its separately authored observed resource.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct ValidatedSourceIdentity<'view, 'input> {
    source: SourceIdentityAuthoring<'input>,
    resource: ObservedResourceAuthoring<'input>,
    _view: std::marker::PhantomData<&'view ()>,
}

impl<'input> ValidatedSourceIdentity<'_, 'input> {
    pub(in crate::canonical_scene::store) const fn resource(
        self,
    ) -> ObservedResourceAuthoring<'input> {
        self.resource
    }

    pub(in crate::canonical_scene::store) const fn source(self) -> SourceIdentityAuthoring<'input> {
        self.source
    }
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct ValidatedMetadataEntry<'input> {
    owner: CanonicalProfileOwner,
    input: MetadataAuthoringEntry<'input>,
}

impl<'input> ValidatedMetadataEntry<'input> {
    pub(in crate::canonical_scene::store) const fn owner(self) -> CanonicalProfileOwner {
        self.owner
    }

    pub(in crate::canonical_scene::store) const fn key(
        self,
    ) -> &'input crate::namespaced::NamespacedName {
        self.input.key()
    }

    pub(in crate::canonical_scene::store) const fn value(self) -> ExtensionValueRef<'input> {
        self.input.value()
    }
}

#[must_use = "dropping the validated profile batch releases its compiled-plan reservation"]
pub(in crate::canonical_scene::store) struct ValidatedProfileBatch<'input, R> {
    compiled: ProfileValidationStorage<R>,
    input: ProfileAuthoringBatch<'input>,
}

struct ProfileValidationStorage<R> {
    resource_count: u32,
    source_count: u32,
    provenance_node_count: u32,
    operation_count: u32,
    parent_edge_count: u64,
    owner_count: u32,
    owner_attachment_count: u64,
    metadata_entry_count: u32,
    entity_count: u64,
    edge_count: u64,
    reference_count: u64,
    payload_cell_count: u64,
    metadata_bytes: u64,
    canonical_bytes: u64,
    retention_shape: ProfileRetentionShape,
    validation_work: u64,
    scratch_bytes: u64,
    maximum_depth: u32,
    provenance_depths: Vec<u32>,
    owner_plans: Vec<ValidatedOwnerProvenancePlan>,
    metadata_plans: Vec<ValidatedMetadataPlan>,
    #[cfg(test)]
    storage_drop_probe: Option<ProfileStorageDropProbe>,
    // Every compiled-plan allocation above must drop before its accounting token.
    reservation: R,
}

impl<'input, R> ValidatedProfileBatch<'input, R> {
    const fn new(
        input: ProfileAuthoringBatch<'input>,
        compiled: ProfileValidationStorage<R>,
    ) -> Self {
        Self { compiled, input }
    }

    /// Fresh producer construction always emits one lineage segment.
    /// Derived operations append later segments through the retained edit path.
    pub(in crate::canonical_scene::store) const fn segment_count(&self) -> u32 {
        1
    }

    pub(in crate::canonical_scene::store) const fn lineage(&self) -> LineageAuthoringBatch<'input> {
        self.input.lineage()
    }

    pub(in crate::canonical_scene::store) const fn metadata(
        &self,
    ) -> MetadataAuthoringBatch<'input> {
        self.input.metadata()
    }

    pub(in crate::canonical_scene::store) const fn resource_count(&self) -> u32 {
        self.compiled.resource_count
    }

    pub(in crate::canonical_scene::store) const fn source_count(&self) -> u32 {
        self.compiled.source_count
    }

    pub(in crate::canonical_scene::store) const fn operation_count(&self) -> u32 {
        self.compiled.operation_count
    }

    pub(in crate::canonical_scene::store) const fn provenance_node_count(&self) -> u32 {
        self.compiled.provenance_node_count
    }

    pub(in crate::canonical_scene::store) const fn parent_edge_count(&self) -> u64 {
        self.compiled.parent_edge_count
    }

    pub(in crate::canonical_scene::store) const fn owner_attachment_count(&self) -> u64 {
        self.compiled.owner_attachment_count
    }

    pub(in crate::canonical_scene::store) fn owner_count(&self) -> u64 {
        u64::from(self.compiled.owner_count)
    }

    pub(in crate::canonical_scene::store) fn owner_at(
        &self,
        ordinal: u64,
    ) -> Option<CanonicalProfileOwner> {
        self.compiled
            .owner_plans
            .get(usize::try_from(ordinal).ok()?)
            .map(|plan| plan.owner)
    }

    pub(in crate::canonical_scene::store) const fn metadata_entry_count(&self) -> u32 {
        self.compiled.metadata_entry_count
    }

    pub(in crate::canonical_scene::store) const fn entity_count(&self) -> u64 {
        self.compiled.entity_count
    }

    pub(in crate::canonical_scene::store) const fn edge_count(&self) -> u64 {
        self.compiled.edge_count
    }

    pub(in crate::canonical_scene::store) const fn reference_count(&self) -> u64 {
        self.compiled.reference_count
    }

    pub(in crate::canonical_scene::store) const fn payload_cell_count(&self) -> u64 {
        self.compiled.payload_cell_count
    }

    pub(in crate::canonical_scene::store) const fn metadata_bytes(&self) -> u64 {
        self.compiled.metadata_bytes
    }

    pub(in crate::canonical_scene::store) const fn canonical_bytes(&self) -> u64 {
        self.compiled.canonical_bytes
    }

    pub(in crate::canonical_scene::store) const fn retention_shape(&self) -> ProfileRetentionShape {
        self.compiled.retention_shape
    }

    pub(in crate::canonical_scene::store) const fn validation_work(&self) -> u64 {
        self.compiled.validation_work
    }

    pub(in crate::canonical_scene::store) const fn scratch_bytes(&self) -> u64 {
        self.compiled.scratch_bytes
    }

    pub(in crate::canonical_scene::store) const fn maximum_depth(&self) -> u32 {
        self.compiled.maximum_depth
    }

    pub(in crate::canonical_scene::store) fn source_identity(
        &self,
        source: SourceIdentityLocalId,
    ) -> Option<ValidatedSourceIdentity<'_, 'input>> {
        let lineage = self.input.lineage();
        let source = *lineage
            .source_identities()
            .get(usize::try_from(source.index()).ok()?)?;
        let resource = *lineage
            .resources()
            .get(usize::try_from(source.resource().index()).ok()?)?;
        Some(ValidatedSourceIdentity {
            source,
            resource,
            _view: std::marker::PhantomData,
        })
    }

    pub(in crate::canonical_scene::store) fn source_leaf(
        &self,
        node: ProvenanceLocalId,
    ) -> Option<SourceIdentityLocalId> {
        match self
            .input
            .lineage()
            .provenance_nodes()
            .get(usize::try_from(node.index()).ok()?)?
        {
            ProvenanceNodeAuthoring::Source(source) => Some(*source),
            ProvenanceNodeAuthoring::Operation(_) => None,
        }
    }

    pub(in crate::canonical_scene::store) fn operation(
        &self,
        node: ProvenanceLocalId,
    ) -> Option<ProvenanceOperationAuthoring<'input>> {
        match self
            .input
            .lineage()
            .provenance_nodes()
            .get(usize::try_from(node.index()).ok()?)?
        {
            ProvenanceNodeAuthoring::Source(_) => None,
            ProvenanceNodeAuthoring::Operation(operation) => Some(*operation),
        }
    }

    pub(in crate::canonical_scene::store) fn provenance_depth(
        &self,
        node: ProvenanceLocalId,
    ) -> Option<u32> {
        self.compiled
            .provenance_depths
            .get(usize::try_from(node.index()).ok()?)
            .copied()
    }

    pub(in crate::canonical_scene::store) fn owner_provenance(
        &self,
        owner: CanonicalProfileOwner,
    ) -> Option<&'input [ProvenanceLocalId]> {
        let plan = self
            .compiled
            .owner_plans
            .binary_search_by_key(&owner, |plan| plan.owner)
            .ok()
            .and_then(|index| self.compiled.owner_plans.get(index))?;
        let input = self
            .input
            .lineage()
            .owner_provenance()
            .get(plan.input_index)?;
        Some(input.provenance())
    }

    pub(in crate::canonical_scene::store) fn owner_provenance_at(
        &self,
        ordinal: u64,
    ) -> Option<(CanonicalProfileOwner, &'input [ProvenanceLocalId])> {
        let plan = *self
            .compiled
            .owner_plans
            .get(usize::try_from(ordinal).ok()?)?;
        let input = self
            .input
            .lineage()
            .owner_provenance()
            .get(plan.input_index)?;
        Some((plan.owner, input.provenance()))
    }

    pub(in crate::canonical_scene::store) fn metadata_entry(
        &self,
        canonical_index: usize,
    ) -> Option<ValidatedMetadataEntry<'input>> {
        let plan = *self.compiled.metadata_plans.get(canonical_index)?;
        let input = *self.input.metadata().entries().get(plan.input_index)?;
        Some(ValidatedMetadataEntry {
            owner: plan.owner,
            input,
        })
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) fn install_storage_drop_probe(
        &mut self,
        stage: std::rc::Rc<std::cell::Cell<u8>>,
    ) {
        self.compiled.storage_drop_probe = Some(ProfileStorageDropProbe(stage));
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store::production) fn into_compiled_facts_for_test(
        self,
    ) -> ([u32; 5], [usize; 3], R) {
        let ProfileValidationStorage {
            resource_count,
            source_count,
            provenance_node_count,
            owner_count,
            metadata_entry_count,
            provenance_depths,
            owner_plans,
            metadata_plans,
            reservation,
            ..
        } = self.compiled;
        (
            [
                resource_count,
                source_count,
                provenance_node_count,
                owner_count,
                metadata_entry_count,
            ],
            [
                provenance_depths.len(),
                owner_plans.len(),
                metadata_plans.len(),
            ],
            reservation,
        )
    }
}

#[cfg(test)]
struct ProfileStorageDropProbe(std::rc::Rc<std::cell::Cell<u8>>);

#[cfg(test)]
impl Drop for ProfileStorageDropProbe {
    fn drop(&mut self) {
        assert_eq!(
            self.0.replace(1),
            0,
            "compiled profile storage must begin the coupled drop sequence"
        );
    }
}

type ProfileValidationResult<T, C> =
    Result<T, ProfileAuthoringFailure<<C as CanonicalAuthoringControl>::Error>>;

struct ProfileValidationContext<'control, C>
where
    C: CanonicalAuthoringControl,
{
    limits: SceneValidationLimits,
    control: &'control mut C,
    work: u64,
    work_since_poll: u64,
}

impl<'control, C> ProfileValidationContext<'control, C>
where
    C: CanonicalAuthoringControl,
{
    const fn new(limits: SceneValidationLimits, control: &'control mut C) -> Self {
        Self {
            limits,
            control,
            work: 0,
            work_since_poll: 0,
        }
    }

    fn poll_now(&mut self) -> ProfileValidationResult<(), C> {
        self.control
            .poll()
            .map_err(ProfileAuthoringFailure::Control)?;
        self.work_since_poll = 0;
        Ok(())
    }

    fn run_unit<R>(
        &mut self,
        category: SceneValidationWork,
        action: impl FnOnce() -> R,
    ) -> ProfileValidationResult<R, C> {
        let final_work =
            self.work
                .checked_add(1)
                .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "profile validation work",
                })?;
        enforce_limit::<C::Error>(
            ProfileAuthoringLimit::Work,
            final_work,
            self.limits.max_work(),
        )?;
        let result = self
            .control
            .run_unit(category, action)
            .map_err(ProfileAuthoringFailure::Control)?;
        self.work = final_work;
        self.work_since_poll += 1;
        if self.work_since_poll == SCENE_VALIDATION_POLL_INTERVAL {
            self.poll_now()?;
        }
        Ok(result)
    }

    fn run_semantic_unit<T>(
        &mut self,
        category: SceneValidationWork,
        action: impl FnOnce() -> Result<T, ProfileAuthoringSemanticError>,
    ) -> ProfileValidationResult<T, C> {
        self.run_unit(category, action)?
            .map_err(ProfileAuthoringFailure::Semantic)
    }

    fn reserve_scratch(&mut self, bytes: u64) -> ProfileValidationResult<C::Reservation, C> {
        enforce_limit::<C::Error>(
            ProfileAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        let reservation = self
            .control
            .reserve_scratch(bytes)
            .map_err(ProfileAuthoringFailure::Control)?;
        self.control
            .validate_scratch(&reservation)
            .map_err(ProfileAuthoringFailure::Control)?;
        Ok(reservation)
    }

    fn resize_scratch(
        &mut self,
        reservation: &mut C::Reservation,
        bytes: u64,
    ) -> ProfileValidationResult<(), C> {
        enforce_limit::<C::Error>(
            ProfileAuthoringLimit::ScratchBytes,
            bytes,
            self.limits.max_scratch_bytes(),
        )?;
        self.control
            .resize_scratch(reservation, bytes)
            .map_err(ProfileAuthoringFailure::Control)
    }

    fn finish(&mut self) -> ProfileValidationResult<(), C> {
        self.poll_now()
    }
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct ProfileRetentionShape {
    evidence_count: u32,
    parent_count: u32,
    wide_count: u32,
    lineage_text_bytes: u32,
    lineage_digest_bytes: u32,
    attachment_count: u32,
    metadata_text_bytes: u32,
    metadata_payload_bytes: u32,
    metadata_bool_count: u32,
    metadata_i64_count: u32,
    metadata_u64_count: u32,
    metadata_f64_count: u32,
    metadata_f32_count: u32,
    canonical_block_count: u32,
    payload_block_count: u32,
    shareable_leaf_block_count: u32,
    canonical_bytes: u64,
}

impl ProfileRetentionShape {
    pub(in crate::canonical_scene::store) const fn evidence_count(self) -> u32 {
        self.evidence_count
    }

    pub(in crate::canonical_scene::store) const fn parent_count(self) -> u32 {
        self.parent_count
    }

    pub(in crate::canonical_scene::store) const fn wide_count(self) -> u32 {
        self.wide_count
    }

    pub(in crate::canonical_scene::store) const fn lineage_text_bytes(self) -> u32 {
        self.lineage_text_bytes
    }

    pub(in crate::canonical_scene::store) const fn lineage_digest_bytes(self) -> u32 {
        self.lineage_digest_bytes
    }

    pub(in crate::canonical_scene::store) const fn attachment_count(self) -> u32 {
        self.attachment_count
    }

    pub(in crate::canonical_scene::store) const fn metadata_text_bytes(self) -> u32 {
        self.metadata_text_bytes
    }

    pub(in crate::canonical_scene::store) const fn metadata_payload_bytes(self) -> u32 {
        self.metadata_payload_bytes
    }

    pub(in crate::canonical_scene::store) const fn metadata_bool_count(self) -> u32 {
        self.metadata_bool_count
    }

    pub(in crate::canonical_scene::store) const fn metadata_i64_count(self) -> u32 {
        self.metadata_i64_count
    }

    pub(in crate::canonical_scene::store) const fn metadata_u64_count(self) -> u32 {
        self.metadata_u64_count
    }

    pub(in crate::canonical_scene::store) const fn metadata_f64_count(self) -> u32 {
        self.metadata_f64_count
    }

    pub(in crate::canonical_scene::store) const fn metadata_f32_count(self) -> u32 {
        self.metadata_f32_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_block_count(self) -> u32 {
        self.canonical_block_count
    }

    pub(in crate::canonical_scene::store) const fn payload_block_count(self) -> u32 {
        self.payload_block_count
    }

    pub(in crate::canonical_scene::store) const fn shareable_leaf_block_count(self) -> u32 {
        self.shareable_leaf_block_count
    }

    pub(in crate::canonical_scene::store) const fn canonical_bytes(self) -> u64 {
        self.canonical_bytes
    }
}

#[derive(Default)]
struct ProfileMetrics {
    entities: u64,
    edges: u64,
    references: u64,
    payload_cells: u64,
    metadata_bytes: u64,
    canonical_bytes: u64,
    parent_edges: u64,
    owner_attachments: u64,
    operation_count: u32,
    maximum_depth: u32,
    evidence_count: u64,
    wide_count: u64,
    lineage_text_bytes: u64,
    lineage_digest_bytes: u64,
    metadata_text_bytes: u64,
    metadata_payload_bytes: u64,
    metadata_bool_count: u64,
    metadata_i64_count: u64,
    metadata_u64_count: u64,
    metadata_f64_count: u64,
    metadata_f32_count: u64,
}

impl ProfileMetrics {
    fn add_entity(&mut self, maximum: u64) -> Result<(), ProfileAuthoringSemanticError> {
        let entities = self.entities.checked_add(1).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "profile entities",
            },
        )?;
        require_limit(ProfileAuthoringLimit::Entities, entities, maximum)?;
        self.entities = entities;
        Ok(())
    }

    fn add_edge(&mut self, maximum: u64) -> Result<(), ProfileAuthoringSemanticError> {
        let edges =
            self.edges
                .checked_add(1)
                .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "profile edges",
                })?;
        require_limit(ProfileAuthoringLimit::Edges, edges, maximum)?;
        self.edges = edges;
        Ok(())
    }

    fn add_reference(&mut self, maximum: u64) -> Result<(), ProfileAuthoringSemanticError> {
        let references = self.references.checked_add(1).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "profile references",
            },
        )?;
        require_limit(ProfileAuthoringLimit::References, references, maximum)?;
        self.references = references;
        Ok(())
    }

    fn add_payload_cell(&mut self, maximum: u64) -> Result<(), ProfileAuthoringSemanticError> {
        let payload_cells = self.payload_cells.checked_add(1).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "profile payload cells",
            },
        )?;
        require_limit(ProfileAuthoringLimit::PayloadCells, payload_cells, maximum)?;
        self.payload_cells = payload_cells;
        Ok(())
    }

    fn add_relation<C>(
        &mut self,
        context: &mut ProfileValidationContext<'_, C>,
    ) -> ProfileValidationResult<(), C>
    where
        C: CanonicalAuthoringControl,
    {
        let maximum_edges = context.limits.max_edges();
        context.run_semantic_unit(SceneValidationWork::GraphEdge, || {
            self.add_edge(maximum_edges)
        })?;
        let maximum_references = context.limits.max_references();
        context.run_semantic_unit(SceneValidationWork::Reference, || {
            self.add_reference(maximum_references)
        })?;
        Ok(())
    }

    fn add_payload<C>(
        &mut self,
        cells: u64,
        context: &mut ProfileValidationContext<'_, C>,
    ) -> ProfileValidationResult<(), C>
    where
        C: CanonicalAuthoringControl,
    {
        let maximum = context.limits.max_payload_cells();
        for _ in 0..cells {
            context.run_semantic_unit(SceneValidationWork::PayloadCell, || {
                self.add_payload_cell(maximum)
            })?;
        }
        Ok(())
    }
}

fn retention_shape_from_metrics(
    resource_count: u32,
    source_count: u32,
    provenance_node_count: u32,
    operation_count: u32,
    owner_count: u32,
    metadata_entry_count: u32,
    metrics: &ProfileMetrics,
) -> Result<ProfileRetentionShape, ProfileAuthoringSemanticError> {
    let evidence_count = retention_count(metrics.evidence_count, "profile evidence rows")?;
    let parent_count = retention_count(metrics.parent_edges, "profile parent values")?;
    let wide_count = retention_count(metrics.wide_count, "profile wide values")?;
    let lineage_text_bytes =
        retention_count(metrics.lineage_text_bytes, "profile lineage text bytes")?;
    let lineage_digest_bytes =
        retention_count(metrics.lineage_digest_bytes, "profile lineage digest bytes")?;
    let attachment_count = retention_count(metrics.owner_attachments, "profile owner attachments")?;
    let metadata_text_bytes =
        retention_count(metrics.metadata_text_bytes, "profile metadata text bytes")?;
    let metadata_payload_bytes = retention_count(
        metrics.metadata_payload_bytes,
        "profile metadata payload bytes",
    )?;
    let metadata_bool_count = retention_count(
        metrics.metadata_bool_count,
        "profile metadata boolean values",
    )?;
    let metadata_i64_count =
        retention_count(metrics.metadata_i64_count, "profile metadata signed values")?;
    let metadata_u64_count = retention_count(
        metrics.metadata_u64_count,
        "profile metadata unsigned values",
    )?;
    let metadata_f64_count =
        retention_count(metrics.metadata_f64_count, "profile metadata f64 values")?;
    let metadata_f32_count =
        retention_count(metrics.metadata_f32_count, "profile metadata f32 values")?;

    let payload_block_count = [
        lineage_text_bytes,
        lineage_digest_bytes,
        metadata_text_bytes,
        metadata_payload_bytes,
        metadata_bool_count,
        metadata_i64_count,
        metadata_u64_count,
        metadata_f64_count,
        metadata_f32_count,
    ]
    .into_iter()
    .try_fold(0_u32, |count, value| {
        count.checked_add(u32::from(value != 0)).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "profile payload block count",
            },
        )
    })?;
    let structural_leaf_count = [
        resource_count,
        source_count,
        provenance_node_count,
        operation_count,
        evidence_count,
        parent_count,
        wide_count,
        attachment_count,
    ]
    .into_iter()
    .try_fold(0_u32, |count, value| {
        count.checked_add(u32::from(value != 0)).ok_or(
            ProfileAuthoringSemanticError::AccountingOverflow {
                dimension: "profile structural leaf count",
            },
        )
    })?;
    let canonical_block_count = 4_u32
        .checked_add(structural_leaf_count)
        .and_then(|count| count.checked_add(payload_block_count))
        .and_then(|count| count.checked_add(u32::from(metadata_entry_count != 0)))
        .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "profile canonical block count",
        })?;
    let shareable_leaf_block_count = structural_leaf_count
        .checked_add(payload_block_count)
        .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "profile shareable leaf block count",
        })?;

    let resource_scalars = scalar_row_bytes(
        resource_count,
        PROFILE_RESOURCE_SCALAR_COUNT,
        "profile resource rows",
    )?;
    let source_scalars = scalar_row_bytes(
        source_count,
        PROFILE_SOURCE_SCALAR_COUNT,
        "profile source rows",
    )?;
    let provenance_scalars = scalar_row_bytes(
        provenance_node_count,
        PROFILE_PROVENANCE_SCALAR_COUNT,
        "profile provenance rows",
    )?;
    let operation_scalars = scalar_row_bytes(
        operation_count,
        PROFILE_OPERATION_SCALAR_COUNT,
        "profile operation rows",
    )?;
    let evidence_scalars = scalar_row_bytes(
        evidence_count,
        PROFILE_EVIDENCE_SCALAR_COUNT,
        "profile evidence rows",
    )?;
    let canonical_bytes = checked_sum(
        [
            PROFILE_DIRECTORY_LOGICAL_BYTES,
            PROFILE_SEGMENT_LOGICAL_BYTES,
            PROFILE_METADATA_DIRECTORY_LOGICAL_BYTES,
            u64::from(owner_count)
                .checked_mul(PROFILE_OWNER_LOGICAL_BYTES)
                .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "profile owner row bytes",
                })?,
            u64::from(metadata_entry_count)
                .checked_mul(PROFILE_METADATA_ENTRY_LOGICAL_BYTES)
                .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "profile metadata row bytes",
                })?,
            resource_scalars,
            source_scalars,
            provenance_scalars,
            operation_scalars,
            evidence_scalars,
            u64::from(parent_count) * PROFILE_PROVENANCE_ADDRESS_LOGICAL_BYTES,
            u64::from(wide_count) * LOGICAL_U64_BYTES,
            u64::from(lineage_text_bytes),
            u64::from(lineage_digest_bytes),
            u64::from(attachment_count) * PROFILE_PROVENANCE_ADDRESS_LOGICAL_BYTES,
            u64::from(metadata_text_bytes),
            u64::from(metadata_payload_bytes),
            u64::from(metadata_bool_count) * LOGICAL_U8_BYTES,
            u64::from(metadata_i64_count) * LOGICAL_U64_BYTES,
            u64::from(metadata_u64_count) * LOGICAL_U64_BYTES,
            u64::from(metadata_f64_count) * LOGICAL_F64_BYTES,
            u64::from(metadata_f32_count) * LOGICAL_F32_BYTES,
        ],
        "profile retained logical bytes",
    )?;

    Ok(ProfileRetentionShape {
        evidence_count,
        parent_count,
        wide_count,
        lineage_text_bytes,
        lineage_digest_bytes,
        attachment_count,
        metadata_text_bytes,
        metadata_payload_bytes,
        metadata_bool_count,
        metadata_i64_count,
        metadata_u64_count,
        metadata_f64_count,
        metadata_f32_count,
        canonical_block_count,
        payload_block_count,
        shareable_leaf_block_count,
        canonical_bytes,
    })
}

fn scalar_row_bytes(
    rows: u32,
    fields: usize,
    dimension: &'static str,
) -> Result<u64, ProfileAuthoringSemanticError> {
    u64::from(rows)
        .checked_mul(usize_to_u64(fields, dimension)?)
        .and_then(|values| values.checked_mul(LOGICAL_U32_BYTES))
        .ok_or(ProfileAuthoringSemanticError::AccountingOverflow { dimension })
}

fn retention_count(
    value: u64,
    collection: &'static str,
) -> Result<u32, ProfileAuthoringSemanticError> {
    u32::try_from(value)
        .map_err(|_| ProfileAuthoringSemanticError::CountDomainTooLarge { collection })
}

pub(in crate::canonical_scene::store) fn validate_profile_batch<'input, O, PP, U>(
    profile: ProfileAuthoringBatch<'input>,
    owners: ProfileOwnerCatalog<'_>,
    limits: SceneValidationLimits,
    control: &mut OperationAuthoringControl<'_, O, PP, U>,
) -> Result<
    ValidatedProfileBatch<'input, RetainedStoreReservation<O>>,
    ProfileAuthoringFailure<OperationAuthoringControlError>,
>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    validate_profile_batch_inner(profile, owners, limits, control)
}

#[cfg(test)]
pub(in crate::canonical_scene::store) fn validate_profile_batch_with_control<'input, C>(
    profile: ProfileAuthoringBatch<'input>,
    owners: ProfileOwnerCatalog<'_>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedProfileBatch<'input, C::Reservation>, ProfileAuthoringFailure<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    validate_profile_batch_inner(profile, owners, limits, control)
}

fn validate_profile_batch_inner<'input, C>(
    profile: ProfileAuthoringBatch<'input>,
    owners: ProfileOwnerCatalog<'_>,
    limits: SceneValidationLimits,
    control: &mut C,
) -> Result<ValidatedProfileBatch<'input, C::Reservation>, ProfileAuthoringFailure<C::Error>>
where
    C: CanonicalAuthoringControl,
{
    let lineage = profile.lineage();
    let metadata = profile.metadata();
    let mut context = ProfileValidationContext::new(limits, control);
    context.poll_now()?;

    let resource_count = collection_count(lineage.resources().len(), "observed resources")?;
    let source_count = collection_count(lineage.source_identities().len(), "source identities")?;
    let provenance_node_count =
        collection_count(lineage.provenance_nodes().len(), "provenance nodes")?;
    let owner_count = u32::try_from(owners.owner_count()).map_err(|_| {
        ProfileAuthoringSemanticError::CountDomainTooLarge {
            collection: "profile owners",
        }
    })?;
    let metadata_entry_count = collection_count(metadata.entries().len(), "metadata entries")?;
    enforce_limit::<C::Error>(
        ProfileAuthoringLimit::Nodes,
        u64::from(provenance_node_count),
        limits.max_nodes(),
    )?;
    enforce_limit::<C::Error>(
        ProfileAuthoringLimit::MetadataEntries,
        u64::from(metadata_entry_count),
        limits.max_metadata_entries(),
    )?;

    let initialization_work = owners
        .owner_count()
        .checked_mul(4)
        .and_then(|work| work.checked_add(u64::from(metadata_entry_count)))
        .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "profile scratch initialization work",
        })?;
    enforce_limit::<C::Error>(
        ProfileAuthoringLimit::Work,
        initialization_work,
        limits.max_work(),
    )?;

    let owner_count_usize = usize::try_from(owner_count).map_err(|_| {
        ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "profile owner plan count",
        }
    })?;
    let depth_plan =
        scratch_plan::<u32>(lineage.provenance_nodes().len(), "provenance depth table")?;
    let owner_plan =
        scratch_plan::<ValidatedOwnerProvenancePlan>(owner_count_usize, "profile owner plans")?;
    let metadata_owner_plan =
        scratch_plan::<u32>(owner_count_usize, "profile metadata owner index")?;
    let metadata_plan =
        scratch_plan::<ValidatedMetadataPlan>(metadata.entries().len(), "profile metadata plans")?;
    let scratch_envelope = checked_sum(
        [
            capacity_envelope(depth_plan, "provenance depth table")?,
            capacity_envelope(owner_plan, "profile owner plans")?,
            capacity_envelope(metadata_owner_plan, "profile metadata owner counts")?,
            capacity_envelope(metadata_owner_plan, "profile metadata owner cursors")?,
            capacity_envelope(metadata_owner_plan, "profile metadata owner last inputs")?,
            capacity_envelope(metadata_plan, "profile metadata plans")?,
        ],
        "profile scratch envelope",
    )?;
    let mut reservation = context.reserve_scratch(scratch_envelope)?;
    let (provenance_depths, depth_bytes) = allocate_scratch(depth_plan, "provenance depth table")?;
    let (owner_plans, owner_bytes) = allocate_scratch(owner_plan, "profile owner plans")?;
    let (metadata_owner_counts, metadata_owner_count_bytes) =
        allocate_scratch(metadata_owner_plan, "profile metadata owner counts")?;
    let (metadata_owner_cursors, metadata_owner_cursor_bytes) =
        allocate_scratch(metadata_owner_plan, "profile metadata owner cursors")?;
    let (metadata_last_inputs, metadata_last_input_bytes) =
        allocate_scratch(metadata_owner_plan, "profile metadata owner last inputs")?;
    let (metadata_plans, metadata_plan_bytes) =
        allocate_scratch(metadata_plan, "profile metadata plans")?;
    let peak_scratch_bytes = checked_sum(
        [
            depth_bytes,
            owner_bytes,
            metadata_owner_count_bytes,
            metadata_owner_cursor_bytes,
            metadata_last_input_bytes,
            metadata_plan_bytes,
        ],
        "profile scratch allocation",
    )?;
    let retained_scratch_bytes = checked_sum(
        [depth_bytes, owner_bytes, metadata_plan_bytes],
        "retained profile scratch allocation",
    )?;
    context.resize_scratch(&mut reservation, peak_scratch_bytes)?;
    let mut storage = ProfileValidationScratch {
        provenance_depths,
        owner_plans,
        metadata_owner_counts,
        metadata_owner_cursors,
        metadata_last_inputs,
        metadata_plans,
        reservation,
    };

    initialize_owner_plans(owners, &mut storage.owner_plans, &mut context)?;

    let mut metrics = ProfileMetrics::default();
    validate_resources(lineage, &mut metrics, &mut context)?;
    validate_source_identities(lineage, resource_count, &mut metrics, &mut context)?;
    validate_provenance_nodes(
        lineage,
        source_count,
        &mut storage.provenance_depths,
        &mut metrics,
        &mut context,
    )?;
    validate_owner_provenance(
        lineage,
        owners,
        provenance_node_count,
        &mut storage.owner_plans,
        &mut metrics,
        &mut context,
    )?;
    validate_metadata(
        metadata,
        owners,
        limits,
        &mut storage,
        &mut metrics,
        &mut context,
    )?;

    let retention_shape = retention_shape_from_metrics(
        resource_count,
        source_count,
        provenance_node_count,
        metrics.operation_count,
        owner_count,
        metadata_entry_count,
        &metrics,
    )?;
    metrics.canonical_bytes = retention_shape.canonical_bytes();

    enforce_limit::<C::Error>(
        ProfileAuthoringLimit::CanonicalBytes,
        metrics.canonical_bytes,
        limits.max_canonical_bytes(),
    )?;

    let ProfileValidationScratch {
        provenance_depths,
        owner_plans,
        metadata_owner_counts,
        metadata_owner_cursors,
        metadata_last_inputs,
        metadata_plans,
        reservation,
    } = storage;
    drop(metadata_last_inputs);
    drop(metadata_owner_cursors);
    drop(metadata_owner_counts);
    let compiled = ProfileValidationStorage {
        resource_count,
        source_count,
        provenance_node_count,
        operation_count: metrics.operation_count,
        parent_edge_count: metrics.parent_edges,
        owner_count,
        owner_attachment_count: metrics.owner_attachments,
        metadata_entry_count,
        entity_count: metrics.entities,
        edge_count: metrics.edges,
        reference_count: metrics.references,
        payload_cell_count: metrics.payload_cells,
        metadata_bytes: metrics.metadata_bytes,
        canonical_bytes: metrics.canonical_bytes,
        retention_shape,
        validation_work: context.work,
        scratch_bytes: retained_scratch_bytes,
        maximum_depth: metrics.maximum_depth,
        provenance_depths,
        owner_plans,
        metadata_plans,
        #[cfg(test)]
        storage_drop_probe: None,
        reservation,
    };
    let mut validated = ValidatedProfileBatch::new(profile, compiled);
    context.resize_scratch(&mut validated.compiled.reservation, retained_scratch_bytes)?;
    context.finish()?;
    Ok(validated)
}

fn validate_resources<C>(
    lineage: LineageAuthoringBatch<'_>,
    metrics: &mut ProfileMetrics,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    let maximum_entities = context.limits.max_entities();
    for resource in lineage.resources().iter().copied() {
        let cells = context.run_semantic_unit(SceneValidationWork::Entity, || {
            let text_bytes = usize_to_u64(
                resource.identity().as_str().len(),
                "resource identity bytes",
            )?;
            let stability_text_bytes = stability_text_bytes(resource.stability())?;
            checked_add(
                &mut metrics.lineage_text_bytes,
                text_bytes.checked_add(stability_text_bytes).ok_or(
                    ProfileAuthoringSemanticError::AccountingOverflow {
                        dimension: "resource lineage text bytes",
                    },
                )?,
                "profile lineage text bytes",
            )?;
            let (stability_cells, _stability_bytes) = stability_measure(resource.stability())?;
            let cells = text_bytes.checked_add(stability_cells).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "resource identity semantic cells",
                },
            )?;
            metrics.add_entity(maximum_entities)?;
            Ok(cells)
        })?;
        metrics.add_payload(cells, context)?;
    }
    Ok(())
}

fn validate_source_identities<C>(
    lineage: LineageAuthoringBatch<'_>,
    resource_count: u32,
    metrics: &mut ProfileMetrics,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    let maximum_entities = context.limits.max_entities();
    let maximum_edges = context.limits.max_edges();
    let maximum_references = context.limits.max_references();
    for (source_index, source) in lineage.source_identities().iter().copied().enumerate() {
        let cells = context.run_semantic_unit(SceneValidationWork::Entity, || {
            let (cells, _bytes) = source_identity_measure(source)?;
            account_source_retention(source, metrics)?;
            metrics.add_entity(maximum_entities)?;
            Ok(cells)
        })?;
        context.run_semantic_unit(SceneValidationWork::GraphEdge, || {
            metrics.add_edge(maximum_edges)
        })?;
        context.run_semantic_unit(SceneValidationWork::Reference, || {
            if source.resource().index() >= resource_count {
                return Err(ProfileAuthoringSemanticError::ResourceOutOfRange {
                    source_index,
                    resource: source.resource().index(),
                    resource_count,
                });
            }
            metrics.add_reference(maximum_references)
        })?;
        metrics.add_payload(cells, context)?;
    }
    Ok(())
}

fn validate_provenance_nodes<C>(
    lineage: LineageAuthoringBatch<'_>,
    source_count: u32,
    depths: &mut Vec<u32>,
    metrics: &mut ProfileMetrics,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    let maximum_entities = context.limits.max_entities();
    let maximum_edges = context.limits.max_edges();
    let maximum_references = context.limits.max_references();
    let maximum_depth = context.limits.max_depth();
    for (node_index, node) in lineage.provenance_nodes().iter().copied().enumerate() {
        let node_id = u32::try_from(node_index).map_err(|_| {
            ProfileAuthoringSemanticError::CountDomainTooLarge {
                collection: "provenance nodes",
            }
        })?;
        let depth = match node {
            ProvenanceNodeAuthoring::Source(source) => {
                context.run_semantic_unit(SceneValidationWork::GraphNode, || {
                    require_limit(ProfileAuthoringLimit::Depth, 1, maximum_depth)?;
                    metrics.add_entity(maximum_entities)
                })?;
                context.run_semantic_unit(SceneValidationWork::GraphEdge, || {
                    metrics.add_edge(maximum_edges)
                })?;
                context.run_semantic_unit(SceneValidationWork::Reference, || {
                    if source.index() >= source_count {
                        return Err(ProfileAuthoringSemanticError::SourceIdentityOutOfRange {
                            node: node_id,
                            source_index: source.index(),
                            source_count,
                        });
                    }
                    metrics.add_reference(maximum_references)
                })?;
                metrics.add_payload(1, context)?;
                1
            }
            ProvenanceNodeAuthoring::Operation(operation) => {
                context.run_semantic_unit(SceneValidationWork::GraphNode, || {
                    if operation.evidence().len() > MAX_OPERATION_EVIDENCE_SUMMARIES {
                        return Err(ProfileAuthoringSemanticError::EvidenceLimitExceeded {
                            node: node_id,
                            actual: operation.evidence().len(),
                            maximum: MAX_OPERATION_EVIDENCE_SUMMARIES,
                        });
                    }
                    require_limit(ProfileAuthoringLimit::Depth, 1, maximum_depth)?;
                    metrics.operation_count = metrics.operation_count.checked_add(1).ok_or(
                        ProfileAuthoringSemanticError::AccountingOverflow {
                            dimension: "provenance operation count",
                        },
                    )?;
                    metrics.add_entity(maximum_entities)?;
                    Ok(())
                })?;
                let depth = validate_operation_node(node_id, operation, depths, metrics, context)?;
                let (cells, _bytes) = operation_measure(operation)?;
                account_operation_retention(operation, metrics)?;
                metrics.add_payload(cells, context)?;
                depth
            }
        };
        metrics.maximum_depth = metrics.maximum_depth.max(depth);
        depths.push(depth);
    }
    Ok(())
}

fn validate_operation_node<C>(
    node: u32,
    operation: ProvenanceOperationAuthoring<'_>,
    depths: &[u32],
    metrics: &mut ProfileMetrics,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<u32, C>
where
    C: CanonicalAuthoringControl,
{
    for (evidence, pair) in operation.evidence().windows(2).enumerate() {
        context.run_semantic_unit(SceneValidationWork::Traversal, || {
            if pair[0].code() >= pair[1].code() {
                return Err(ProfileAuthoringSemanticError::EvidenceOrder {
                    node,
                    evidence: evidence + 1,
                });
            }
            Ok(())
        })?;
    }

    let mut maximum_parent_depth = 0_u32;
    let mut previous = None;
    let maximum_edges = context.limits.max_edges();
    let maximum_references = context.limits.max_references();
    let maximum_depth = context.limits.max_depth();
    for parent in operation.parents().iter().copied() {
        let parent_edges = metrics.parent_edges;
        let updated_parent_edges =
            context.run_semantic_unit(SceneValidationWork::GraphEdge, || {
                if let Some(previous) = previous {
                    if parent.index() == previous {
                        return Err(ProfileAuthoringSemanticError::DuplicateParent {
                            node,
                            parent: parent.index(),
                        });
                    }
                    if parent.index() < previous {
                        return Err(ProfileAuthoringSemanticError::ParentOrder {
                            node,
                            parent: parent.index(),
                        });
                    }
                }
                let parent_edges = parent_edges.checked_add(1).ok_or(
                    ProfileAuthoringSemanticError::AccountingOverflow {
                        dimension: "provenance parent edges",
                    },
                )?;
                metrics.add_edge(maximum_edges)?;
                Ok(parent_edges)
            })?;
        metrics.parent_edges = updated_parent_edges;
        previous = Some(parent.index());
        let parent_depth = context.run_semantic_unit(SceneValidationWork::Reference, || {
            if parent.index() >= node {
                return Err(ProfileAuthoringSemanticError::ParentNotBeforeChild {
                    node,
                    parent: parent.index(),
                });
            }
            let parent_index = usize::try_from(parent.index()).map_err(|_| {
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "provenance parent index",
                }
            })?;
            let parent_depth = *depths.get(parent_index).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "compiled provenance parent depth",
                },
            )?;
            let depth = parent_depth.checked_add(1).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "provenance depth",
                },
            )?;
            require_limit(
                ProfileAuthoringLimit::Depth,
                u64::from(depth),
                maximum_depth,
            )?;
            metrics.add_reference(maximum_references)?;
            Ok(parent_depth)
        })?;
        maximum_parent_depth = maximum_parent_depth.max(parent_depth);
    }
    let depth = maximum_parent_depth.checked_add(1).ok_or(
        ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "provenance depth",
        },
    )?;
    Ok(depth)
}

fn initialize_owner_plans<C>(
    owners: ProfileOwnerCatalog<'_>,
    plans: &mut Vec<ValidatedOwnerProvenancePlan>,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    let owner_count = usize::try_from(owners.owner_count()).map_err(|_| {
        ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "compiled profile owner count",
        }
    })?;
    if plans.capacity() < owner_count {
        return Err(ProfileAuthoringSemanticError::ScratchAllocation {
            resource: "profile owner plans",
        }
        .into());
    }
    for ordinal in 0..owners.owner_count() {
        let owner =
            owners
                .owner_at(ordinal)
                .ok_or(ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "compiled profile owner ordinal",
                })?;
        context.run_unit(SceneValidationWork::Traversal, || {
            plans.push(ValidatedOwnerProvenancePlan {
                owner,
                input_index: usize::MAX,
            });
        })?;
    }
    if u64::try_from(plans.len()).map_err(|_| {
        ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "compiled profile owner count",
        }
    })? != owners.owner_count()
    {
        return Err(ProfileAuthoringSemanticError::AccountingOverflow {
            dimension: "compiled profile owner count",
        }
        .into());
    }
    Ok(())
}

fn validate_owner_provenance<C>(
    lineage: LineageAuthoringBatch<'_>,
    owners: ProfileOwnerCatalog<'_>,
    provenance_count: u32,
    plans: &mut [ValidatedOwnerProvenancePlan],
    metrics: &mut ProfileMetrics,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    C: CanonicalAuthoringControl,
{
    let maximum_edges = context.limits.max_edges();
    let maximum_references = context.limits.max_references();
    for (input_index, authored) in lineage.owner_provenance().iter().copied().enumerate() {
        let owner = context.run_semantic_unit(SceneValidationWork::Traversal, || {
            let (owner, ordinal) = owners.normalize(authored.owner())?;
            let plan = plans.get_mut(ordinal).ok_or(
                ProfileAuthoringSemanticError::AccountingOverflow {
                    dimension: "compiled owner ordinal",
                },
            )?;
            if plan.input_index != usize::MAX {
                return Err(ProfileAuthoringSemanticError::DuplicateOwnerProvenance { owner });
            }
            if authored.provenance().is_empty() {
                return Err(ProfileAuthoringSemanticError::EmptyOwnerProvenance { owner });
            }
            *plan = ValidatedOwnerProvenancePlan { owner, input_index };
            Ok(owner)
        })?;
        let mut previous = None;
        for provenance in authored.provenance().iter().copied() {
            let owner_attachments = metrics.owner_attachments;
            let updated_owner_attachments =
                context.run_semantic_unit(SceneValidationWork::GraphEdge, || {
                    if let Some(previous) = previous
                        && provenance.index() <= previous
                    {
                        return Err(ProfileAuthoringSemanticError::OwnerProvenanceOrder {
                            owner,
                            previous,
                            current: provenance.index(),
                        });
                    }
                    let owner_attachments = owner_attachments.checked_add(1).ok_or(
                        ProfileAuthoringSemanticError::AccountingOverflow {
                            dimension: "owner provenance attachments",
                        },
                    )?;
                    metrics.add_edge(maximum_edges)?;
                    Ok(owner_attachments)
                })?;
            metrics.owner_attachments = updated_owner_attachments;
            previous = Some(provenance.index());
            context.run_semantic_unit(SceneValidationWork::Reference, || {
                require_provenance(provenance, provenance_count, "owner provenance attachment")?;
                metrics.add_reference(maximum_references)
            })?;
        }
    }

    if lineage.owner_provenance().len() == plans.len() {
        return Ok(());
    }
    for plan in plans.iter() {
        context.run_semantic_unit(SceneValidationWork::Traversal, || {
            if plan.input_index == usize::MAX {
                return Err(ProfileAuthoringSemanticError::MissingOwnerProvenance {
                    owner: plan.owner,
                });
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn require_provenance(
    provenance: ProvenanceLocalId,
    provenance_count: u32,
    usage: &'static str,
) -> Result<(), ProfileAuthoringSemanticError> {
    if provenance.index() < provenance_count {
        Ok(())
    } else {
        Err(ProfileAuthoringSemanticError::ProvenanceOutOfRange {
            usage,
            provenance: provenance.index(),
            provenance_count,
        })
    }
}

fn scratch_plan<T>(
    count: usize,
    resource: &'static str,
) -> Result<CapacityPlan, ProfileAuthoringSemanticError> {
    CapacityPlan::for_type::<T>(0, 0, count)
        .map_err(|cause| ProfileAuthoringSemanticError::ScratchCapacity { resource, cause })
}

fn capacity_envelope(
    plan: CapacityPlan,
    resource: &'static str,
) -> Result<u64, ProfileAuthoringSemanticError> {
    plan.envelope_delta_bytes()
        .map_err(|cause| ProfileAuthoringSemanticError::ScratchCapacity { resource, cause })
}

fn allocate_scratch<T>(
    plan: CapacityPlan,
    resource: &'static str,
) -> Result<(Vec<T>, u64), ProfileAuthoringSemanticError> {
    plan.verify_preflight(0, 0)
        .map_err(|cause| ProfileAuthoringSemanticError::ScratchCapacity { resource, cause })?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(plan.allocator_additional())
        .map_err(|_| ProfileAuthoringSemanticError::ScratchAllocation { resource })?;
    let bytes = plan
        .finish(values.len(), values.capacity())
        .map_err(|cause| ProfileAuthoringSemanticError::ScratchCapacity { resource, cause })?;
    Ok((values, bytes))
}

fn initialize_scratch<T, C>(
    values: &mut Vec<T>,
    length: usize,
    value: T,
    resource: &'static str,
    context: &mut ProfileValidationContext<'_, C>,
) -> ProfileValidationResult<(), C>
where
    T: Copy,
    C: CanonicalAuthoringControl,
{
    if !values.is_empty() || values.capacity() < length {
        return Err(ProfileAuthoringSemanticError::ScratchAllocation { resource }.into());
    }
    while values.len() < length {
        context.run_unit(SceneValidationWork::Traversal, || {
            values.push(value);
        })?;
    }
    Ok(())
}

fn collection_count(
    count: usize,
    collection: &'static str,
) -> Result<u32, ProfileAuthoringSemanticError> {
    u32::try_from(count)
        .map_err(|_| ProfileAuthoringSemanticError::CountDomainTooLarge { collection })
}

fn usize_to_u64(
    value: usize,
    dimension: &'static str,
) -> Result<u64, ProfileAuthoringSemanticError> {
    u64::try_from(value)
        .map_err(|_| ProfileAuthoringSemanticError::AccountingOverflow { dimension })
}

fn checked_add(
    value: &mut u64,
    amount: u64,
    dimension: &'static str,
) -> Result<(), ProfileAuthoringSemanticError> {
    *value = value
        .checked_add(amount)
        .ok_or(ProfileAuthoringSemanticError::AccountingOverflow { dimension })?;
    Ok(())
}

fn checked_sum<const N: usize>(
    values: [u64; N],
    dimension: &'static str,
) -> Result<u64, ProfileAuthoringSemanticError> {
    values
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
        .ok_or(ProfileAuthoringSemanticError::AccountingOverflow { dimension })
}

fn enforce_limit<E>(
    limit: ProfileAuthoringLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), ProfileAuthoringFailure<E>>
where
    E: Error + Send + Sync + 'static,
{
    require_limit(limit, actual, maximum).map_err(ProfileAuthoringFailure::Semantic)
}

fn require_limit(
    limit: ProfileAuthoringLimit,
    actual: u64,
    maximum: u64,
) -> Result<(), ProfileAuthoringSemanticError> {
    if actual <= maximum {
        Ok(())
    } else {
        Err(ProfileAuthoringSemanticError::LimitExceeded {
            limit,
            actual,
            maximum,
        })
    }
}
