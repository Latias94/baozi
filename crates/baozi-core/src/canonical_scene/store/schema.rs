use std::marker::PhantomData;

use baozi_operation::{CanonicalStoreRef, RecordBlock, ScalarSequence};

pub(super) const SCHEMA_REVISION: u32 = 1;
pub(super) const NODE_COUNT: usize = 2;
pub(super) const REFERENCE_COUNT: usize = 1;
pub(super) const MATRIX_SCALAR_COUNT: usize = 32;
pub(super) const MATRIX_SCALARS_PER_NODE: usize = 16;
pub(super) const DIRECTORY_FIELD_COUNT: usize = 14;
pub(super) const NODE_FIELD_COUNT: usize = 7;
pub(super) const REFERENCE_FIELD_COUNT: usize = 3;
pub(super) const EVIDENCE_FIELD_COUNT: usize = 4;

pub(super) mod directory_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const ROOT_NODE: usize = 1;
    pub(crate) const NODE_COUNT: usize = 2;
    pub(crate) const NODES: usize = 3;
    pub(crate) const REFERENCE_COUNT: usize = 4;
    pub(crate) const REFERENCES: usize = 5;
    pub(crate) const ROOT_CHILDREN_PRESENCE: usize = 6;
    pub(crate) const ROOT_CHILDREN_COUNT: usize = 7;
    pub(crate) const CHILD_CHILDREN_PRESENCE: usize = 8;
    pub(crate) const CHILD_CHILDREN_COUNT: usize = 9;
    pub(crate) const METADATA_PRESENCE: usize = 10;
    pub(crate) const METADATA_COUNT: usize = 11;
    pub(crate) const MATRIX_SCALAR_COUNT: usize = 12;
    pub(crate) const MATRICES: usize = 13;
}

pub(super) mod node_field {
    pub(crate) const NODE: usize = 0;
    pub(crate) const PARENT_PRESENCE: usize = 1;
    pub(crate) const PARENT_REFERENCE_OFFSET: usize = 2;
    pub(crate) const PARENT_REFERENCE_COUNT: usize = 3;
    pub(crate) const MATRIX_OFFSET: usize = 4;
    pub(crate) const MATRIX_LENGTH: usize = 5;
    pub(crate) const MATRIX: usize = 6;
}

pub(super) mod reference_field {
    pub(crate) const REFERENCE: usize = 0;
    pub(crate) const PARENT: usize = 1;
    pub(crate) const CHILD: usize = 2;
}

pub(super) mod evidence_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const OUTCOME: usize = 1;
    pub(crate) const CANONICAL_BLOCK_COUNT: usize = 2;
    pub(crate) const NODE_COUNT: usize = 3;
}

pub(super) const PRESENCE_ABSENT: u8 = 0;
pub(super) const PRESENCE_PRESENT: u8 = 1;
pub(super) const EVIDENCE_OUTCOME_CANDIDATE: u32 = 1;
pub(super) const CANONICAL_BLOCK_COUNT: u32 = 4;

pub(super) const NODE_SCENE_SCHEMA_REVISION: u32 = 1;
pub(super) const NODE_SCENE_DIRECTORY_FIELD_COUNT: usize = 11;
pub(super) const NODE_SCENE_NODE_FIELD_COUNT: usize = 8;
pub(super) const NODE_SCENE_CHILD_REFERENCE_FIELD_COUNT: usize = 2;
pub(super) const NODE_SCENE_EVIDENCE_FIELD_COUNT: usize = 4;
pub(super) const NODE_SCENE_REQUIRED_CANONICAL_BLOCK_COUNT: u32 = 2;
pub(super) const NODE_SCENE_POPULATED_CANONICAL_BLOCK_COUNT: u32 = 4;
pub(super) const NODE_SCENE_MATRIX_SCALAR_COUNT: usize = 16;
pub(super) const NODE_SCENE_TRS_SCALAR_COUNT: usize = 10;
pub(super) const NODE_SCENE_TRANSFORM_SYNTHETIC: u8 = 0;
pub(super) const NODE_SCENE_TRANSFORM_MATRIX: u8 = 1;
pub(super) const NODE_SCENE_TRANSFORM_TRS: u8 = 2;
pub(super) const NODE_SCENE_EVIDENCE_OUTCOME_CANDIDATE: u32 = 1;

pub(super) mod node_scene_directory_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const ROOT_NODE: usize = 1;
    pub(crate) const NODE_COUNT: usize = 2;
    pub(crate) const NODES: usize = 3;
    pub(crate) const REFERENCE_PRESENCE: usize = 4;
    pub(crate) const REFERENCE_COUNT: usize = 5;
    pub(crate) const REFERENCES: usize = 6;
    pub(crate) const TRANSFORM_PRESENCE: usize = 7;
    pub(crate) const TRANSFORM_SCALAR_COUNT: usize = 8;
    pub(crate) const TRANSFORMS: usize = 9;
    pub(crate) const SOURCE_ROOT_COUNT: usize = 10;
}

pub(super) mod node_scene_node_field {
    pub(crate) const NODE: usize = 0;
    pub(crate) const CHILDREN_PRESENCE: usize = 1;
    pub(crate) const CHILD_REFERENCE_OFFSET: usize = 2;
    pub(crate) const CHILD_REFERENCE_COUNT: usize = 3;
    pub(crate) const TRANSFORM_KIND: usize = 4;
    pub(crate) const TRANSFORM_OFFSET: usize = 5;
    pub(crate) const TRANSFORM_LENGTH: usize = 6;
    pub(crate) const TRANSFORMS: usize = 7;
}

pub(super) mod node_scene_child_reference_field {
    pub(crate) const REFERENCE: usize = 0;
    pub(crate) const CHILD: usize = 1;
}

pub(super) mod node_scene_evidence_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const OUTCOME: usize = 1;
    pub(crate) const CANONICAL_BLOCK_COUNT: usize = 2;
    pub(crate) const NODE_COUNT: usize = 3;
}

pub(super) trait CanonicalSchema: Copy + Send + Sync + 'static {
    type StoreKind;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct DirectorySchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct NodeTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct ReferenceTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct AuthoredMatrixSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct OutcomeEvidenceSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct NodeSceneDirectorySchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct NodeSceneNodeTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct NodeSceneChildReferenceTableSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct NodeSceneAuthoredTransformSchema;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct NodeSceneOutcomeEvidenceSchema;

impl CanonicalSchema for DirectorySchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for NodeTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for ReferenceTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for AuthoredMatrixSchema {
    type StoreKind = ScalarSequence<f32>;
}

impl CanonicalSchema for NodeSceneDirectorySchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for NodeSceneNodeTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for NodeSceneChildReferenceTableSchema {
    type StoreKind = RecordBlock;
}

impl CanonicalSchema for NodeSceneAuthoredTransformSchema {
    type StoreKind = ScalarSequence<f32>;
}

/// Non-owning core projection over one destination-qualified concrete-store slot.
pub(super) struct CanonicalBlockRef<Schema: CanonicalSchema> {
    raw: CanonicalStoreRef<Schema::StoreKind, Schema>,
    marker: PhantomData<fn() -> Schema>,
}

impl<Schema: CanonicalSchema> Copy for CanonicalBlockRef<Schema> {}

impl<Schema: CanonicalSchema> Clone for CanonicalBlockRef<Schema> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Schema: CanonicalSchema> PartialEq for CanonicalBlockRef<Schema> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<Schema: CanonicalSchema> Eq for CanonicalBlockRef<Schema> {}

impl<Schema: CanonicalSchema> std::fmt::Debug for CanonicalBlockRef<Schema> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CanonicalBlockRef(..)")
    }
}

impl<Schema: CanonicalSchema> CanonicalBlockRef<Schema> {
    pub(super) const fn from_store(raw: CanonicalStoreRef<Schema::StoreKind, Schema>) -> Self {
        Self {
            raw,
            marker: PhantomData,
        }
    }

    pub(super) const fn into_store(self) -> CanonicalStoreRef<Schema::StoreKind, Schema> {
        self.raw
    }
}

/// Test-only retained-store protocol stress fixture; not production Scene IR.
#[cfg(test)]
mod complex_fixture;

#[cfg(test)]
pub(super) use complex_fixture::*;
