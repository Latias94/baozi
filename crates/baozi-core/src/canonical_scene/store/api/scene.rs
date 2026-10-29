use std::fmt;

use crate::identity::SceneGenerationId;

use super::super::construction::node_scene::SceneStorage;
use super::super::construction::node_scene::snapshot::{
    NodeSceneSnapshot, NodeSceneSnapshotError, NodeSceneSnapshotLimits,
};

/// Opaque owner of one centrally validated canonical Scene.
#[must_use = "the validated Scene owns retained canonical storage"]
pub struct ValidatedScene(SceneStorage);

impl ValidatedScene {
    pub(in crate::canonical_scene::store) const fn from_node_scene(scene: SceneStorage) -> Self {
        Self(scene)
    }

    /// Returns the generation reserved for this successfully published Scene.
    pub const fn generation(&self) -> SceneGenerationId {
        self.0.generation()
    }

    /// Returns the number of directory-backed retained blocks owned by this Scene.
    ///
    /// Private typed node and geometry owners are separate from this legacy block count.
    pub fn canonical_block_count(&self) -> usize {
        self.0.canonical_block_count()
    }

    /// Restricts access to the sealed retained rail so semantic views and SceneEdit can project
    /// domain data without exposing the raw canonical store.
    pub(in crate::canonical_scene::store) const fn as_node_scene(&self) -> &SceneStorage {
        &self.0
    }

    /// Captures a bounded semantic snapshot without exposing retained storage or validation proof.
    pub fn capture_snapshot(
        &self,
        limits: SceneSnapshotLimits,
    ) -> Result<SceneSnapshot, SceneSnapshotError> {
        self.as_node_scene()
            .capture_snapshot(limits.0, None)
            .map(SceneSnapshot)
            .map_err(SceneSnapshotError)
    }
}

/// Hard bounds for one semantic Scene snapshot capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneSnapshotLimits(NodeSceneSnapshotLimits);

impl SceneSnapshotLimits {
    /// Validates all allocation, collection-preview, and traversal-work limits before capture.
    pub fn try_new(
        max_preview_bytes: usize,
        max_collection_items: usize,
        max_work: u64,
    ) -> Result<Self, SceneSnapshotError> {
        NodeSceneSnapshotLimits::try_new(max_preview_bytes, max_collection_items, max_work)
            .map(Self)
            .map_err(SceneSnapshotError)
    }
}

/// Complete canonical digest, bounded preview, and semantic totals for one retained Scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSnapshot(NodeSceneSnapshot);

impl SceneSnapshot {
    /// Returns the complete canonical digest.
    pub const fn digest(&self) -> [u8; 32] {
        self.0.digest
    }

    /// Returns the human-reviewable prefix emitted from the same semantic traversal as the digest.
    pub fn preview(&self) -> &str {
        &self.0.preview.text
    }

    /// Returns whether the preview omitted bytes because of its configured byte limit.
    pub const fn preview_bytes_truncated(&self) -> bool {
        self.0.preview.bytes_truncated
    }

    /// Returns whether the preview omitted collection items because of its configured item limit.
    pub const fn preview_items_truncated(&self) -> bool {
        self.0.preview.items_truncated
    }

    /// Returns the total canonical node count.
    pub const fn node_count(&self) -> usize {
        self.0.totals.node_count
    }

    /// Returns the total root-node count.
    pub const fn root_count(&self) -> usize {
        self.0.totals.root_count
    }

    /// Returns the total material count.
    pub const fn material_count(&self) -> usize {
        self.0.totals.material_count
    }

    /// Returns the total image count.
    pub const fn image_count(&self) -> usize {
        self.0.totals.image_count
    }

    /// Returns the total sampler count.
    pub const fn sampler_count(&self) -> usize {
        self.0.totals.sampler_count
    }

    /// Returns the total texture count.
    pub const fn texture_count(&self) -> usize {
        self.0.totals.texture_count
    }

    /// Returns the total material-binding count.
    pub const fn material_binding_count(&self) -> usize {
        self.0.totals.material_binding_count
    }

    /// Returns the total logical-mesh count.
    pub const fn logical_mesh_count(&self) -> usize {
        self.0.totals.logical_mesh_count
    }

    /// Returns the total primitive count.
    pub const fn primitive_count(&self) -> usize {
        self.0.totals.primitive_count
    }

    /// Returns the total geometry-attribute count.
    pub const fn attribute_count(&self) -> usize {
        self.0.totals.attribute_count
    }

    /// Returns the total mesh-instance count.
    pub const fn mesh_instance_count(&self) -> usize {
        self.0.totals.mesh_instance_count
    }

    /// Returns the total number of explicitly retained primitive indices.
    pub const fn primitive_index_count(&self) -> u64 {
        self.0.totals.primitive_index_count
    }

    /// Returns the total number of retained polygon-boundary entries.
    pub const fn polygon_boundary_count(&self) -> u64 {
        self.0.totals.polygon_boundary_count
    }

    /// Returns the total number of canonical geometry scalars.
    pub const fn canonical_scalar_count(&self) -> u64 {
        self.0.totals.canonical_scalar_count
    }

    /// Returns the total canonical f32 scalar count across geometry attributes.
    pub const fn f32_scalar_count(&self) -> u64 {
        self.0.totals.f32_scalar_count
    }

    /// Returns the total canonical f64 scalar count across geometry attributes.
    pub const fn f64_scalar_count(&self) -> u64 {
        self.0.totals.f64_scalar_count
    }

    /// Returns the total canonical u32 scalar count across geometry attributes.
    pub const fn u32_scalar_count(&self) -> u64 {
        self.0.totals.u32_scalar_count
    }

    /// Returns the total canonical i32 scalar count across geometry attributes.
    pub const fn i32_scalar_count(&self) -> u64 {
        self.0.totals.i32_scalar_count
    }

    /// Returns the total encoded image payload bytes.
    pub const fn image_payload_bytes(&self) -> u64 {
        self.0.totals.image_payload_bytes
    }

    /// Returns the total material-factor scalar count.
    pub const fn material_factor_scalar_count(&self) -> u64 {
        self.0.totals.material_factor_scalar_count
    }

    /// Returns the total retained profile resource count.
    pub const fn profile_resource_count(&self) -> usize {
        self.0.totals.profile_resource_count
    }

    /// Returns the total retained source-identity count.
    pub const fn profile_source_count(&self) -> usize {
        self.0.totals.profile_source_count
    }

    /// Returns the total retained provenance-node count.
    pub const fn profile_provenance_node_count(&self) -> usize {
        self.0.totals.profile_provenance_node_count
    }

    /// Returns retained provenance operation nodes in this debug snapshot.
    pub const fn profile_operation_count(&self) -> usize {
        self.0.totals.profile_operation_count
    }

    /// Returns durable operation evidence summaries in this debug snapshot.
    pub const fn profile_evidence_count(&self) -> usize {
        self.0.totals.profile_evidence_count
    }

    /// Returns ordered operation parent references in this debug snapshot.
    pub const fn profile_parent_reference_count(&self) -> u64 {
        self.0.totals.profile_parent_reference_count
    }

    /// Returns the total canonical owner count carrying provenance.
    pub const fn profile_owner_count(&self) -> usize {
        self.0.totals.profile_owner_count
    }

    /// Returns the total provenance attachments across canonical owners.
    pub const fn profile_attachment_count(&self) -> usize {
        self.0.totals.profile_attachment_count
    }

    /// Returns the total profile metadata entry count.
    pub const fn profile_metadata_entry_count(&self) -> usize {
        self.0.totals.profile_metadata_entry_count
    }
}

/// Snapshot capture failed before producing a complete digest and totals.
#[derive(Debug)]
pub struct SceneSnapshotError(NodeSceneSnapshotError);

impl fmt::Display for SceneSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for SceneSnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}
