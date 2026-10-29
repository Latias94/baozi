/// Maximum work between two cooperative cancellation polls.
pub const SCENE_VALIDATION_POLL_INTERVAL: u64 = 256;

/// A stable category of work charged while validating canonical authoring input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneValidationWork {
    GraphNode,
    GraphEdge,
    Entity,
    Reference,
    PayloadCell,
    MetadataEntry,
    MetadataByte,
    Traversal,
}

/// Immutable limits applied by canonical authoring and central validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneValidationLimits {
    max_nodes: u64,
    max_edges: u64,
    max_depth: u64,
    max_provenance_depth: u64,
    max_entities: u64,
    max_references: u64,
    max_payload_cells: u64,
    max_work: u64,
    max_metadata_entries: u64,
    max_metadata_bytes: u64,
    max_metadata_key_bytes: usize,
    max_metadata_string_bytes: usize,
    max_canonical_bytes: u64,
    max_scratch_bytes: u64,
    max_diagnostic_issues: u64,
    max_diagnostic_bytes: u64,
}

impl SceneValidationLimits {
    pub const fn with_max_nodes(mut self, value: u64) -> Self {
        self.max_nodes = value;
        self
    }

    pub const fn with_max_edges(mut self, value: u64) -> Self {
        self.max_edges = value;
        self
    }

    /// Sets Scene hierarchy depth, including the synthetic root.
    pub const fn with_max_depth(mut self, value: u64) -> Self {
        self.max_depth = value;
        self
    }

    /// Overrides lineage depth independently of the Scene hierarchy depth.
    pub const fn with_max_provenance_depth(mut self, value: u64) -> Self {
        self.max_provenance_depth = value;
        self
    }

    pub const fn with_max_entities(mut self, value: u64) -> Self {
        self.max_entities = value;
        self
    }

    pub const fn with_max_references(mut self, value: u64) -> Self {
        self.max_references = value;
        self
    }

    pub const fn with_max_payload_cells(mut self, value: u64) -> Self {
        self.max_payload_cells = value;
        self
    }

    pub const fn with_max_work(mut self, value: u64) -> Self {
        self.max_work = value;
        self
    }

    pub const fn with_max_metadata_entries(mut self, value: u64) -> Self {
        self.max_metadata_entries = value;
        self
    }

    pub const fn with_max_metadata_bytes(mut self, value: u64) -> Self {
        self.max_metadata_bytes = value;
        self
    }

    pub const fn with_max_metadata_key_bytes(mut self, value: usize) -> Self {
        self.max_metadata_key_bytes = value;
        self
    }

    pub const fn with_max_metadata_string_bytes(mut self, value: usize) -> Self {
        self.max_metadata_string_bytes = value;
        self
    }

    pub const fn with_max_canonical_bytes(mut self, value: u64) -> Self {
        self.max_canonical_bytes = value;
        self
    }

    pub const fn with_max_scratch_bytes(mut self, value: u64) -> Self {
        self.max_scratch_bytes = value;
        self
    }

    pub const fn with_max_diagnostic_issues(mut self, value: u64) -> Self {
        self.max_diagnostic_issues = value;
        self
    }

    pub const fn with_max_diagnostic_bytes(mut self, value: u64) -> Self {
        self.max_diagnostic_bytes = value;
        self
    }

    pub const fn max_nodes(self) -> u64 {
        self.max_nodes
    }

    pub const fn max_edges(self) -> u64 {
        self.max_edges
    }

    pub const fn max_depth(self) -> u64 {
        self.max_depth
    }

    pub const fn max_provenance_depth(self) -> u64 {
        self.max_provenance_depth
    }

    pub const fn max_entities(self) -> u64 {
        self.max_entities
    }

    pub const fn max_references(self) -> u64 {
        self.max_references
    }

    pub const fn max_payload_cells(self) -> u64 {
        self.max_payload_cells
    }

    pub const fn max_work(self) -> u64 {
        self.max_work
    }

    pub const fn max_metadata_entries(self) -> u64 {
        self.max_metadata_entries
    }

    pub const fn max_metadata_bytes(self) -> u64 {
        self.max_metadata_bytes
    }

    pub const fn max_metadata_key_bytes(self) -> usize {
        self.max_metadata_key_bytes
    }

    pub const fn max_metadata_string_bytes(self) -> usize {
        self.max_metadata_string_bytes
    }

    pub const fn max_canonical_bytes(self) -> u64 {
        self.max_canonical_bytes
    }

    pub const fn max_scratch_bytes(self) -> u64 {
        self.max_scratch_bytes
    }

    pub const fn max_diagnostic_issues(self) -> u64 {
        self.max_diagnostic_issues
    }

    pub const fn max_diagnostic_bytes(self) -> u64 {
        self.max_diagnostic_bytes
    }
}

impl Default for SceneValidationLimits {
    fn default() -> Self {
        Self {
            max_nodes: 1_000_000,
            max_edges: 4_000_000,
            max_depth: 100_000,
            max_provenance_depth: 100_000,
            max_entities: 2_000_000,
            max_references: 8_000_000,
            max_payload_cells: 100_000_000,
            max_work: 1_000_000_000,
            max_metadata_entries: 1_000_000,
            max_metadata_bytes: 64 * 1024 * 1024,
            max_metadata_key_bytes: 256,
            max_metadata_string_bytes: 1024 * 1024,
            max_canonical_bytes: 4 * 1024 * 1024 * 1024,
            max_scratch_bytes: 512 * 1024 * 1024,
            max_diagnostic_issues: 4_096,
            max_diagnostic_bytes: 4 * 1024 * 1024,
        }
    }
}
