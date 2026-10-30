//! Typed profile ownership. Segment-local references never store a second global index.
//!
//! Metadata descriptors belong to the current owner layout; their immutable payloads and each
//! lineage segment have separate retained owners so edits can share them without copying bytes.

use std::num::{NonZeroU32, NonZeroU64};

use baozi_operation::__retained_store_integration::authority::SharedRetainedStorage;

use crate::canonical_scene::store::production::{CanonicalProfileOwner, SessionOpaqueReason};
use crate::canonical_scene::store::profile_provenance::{
    ProfileProvenanceRef, ProfileSegmentAddressDomain, ProfileSegmentNonce,
};
use crate::provenance::{ContentDigest, EvidenceDigest, OptionsDigest, SourceObjectKind};

use super::{TableRange, TextRange};

pub(in crate::canonical_scene::store) mod build;
pub(in crate::canonical_scene::store) mod validation;

pub(in crate::canonical_scene::store) struct ProfileStorage {
    pub(in crate::canonical_scene::store) state: SharedRetainedStorage<ProfileState>,
    pub(in crate::canonical_scene::store) metadata: SharedRetainedStorage<MetadataPayloads>,
}

pub(in crate::canonical_scene::store) struct ProfileState {
    pub(in crate::canonical_scene::store) segments: Vec<StoredProfileSegment>,
    pub(in crate::canonical_scene::store) owners: Vec<StoredProfileOwner>,
    pub(in crate::canonical_scene::store) attachments: Vec<ProfileProvenanceRef>,
    pub(in crate::canonical_scene::store) metadata_entries: Vec<StoredMetadataEntry>,
    pub(in crate::canonical_scene::store) lineage_counts: LineageCounts,
}

pub(in crate::canonical_scene::store) struct StoredProfileSegment {
    pub(in crate::canonical_scene::store) lineage: SharedRetainedStorage<LineageSegment>,
    pub(in crate::canonical_scene::store) prefixes: LineageCounts,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct LineageCounts {
    pub(in crate::canonical_scene::store) resources: usize,
    pub(in crate::canonical_scene::store) sources: usize,
    pub(in crate::canonical_scene::store) provenance: usize,
    pub(in crate::canonical_scene::store) operations: usize,
    pub(in crate::canonical_scene::store) evidence: usize,
    pub(in crate::canonical_scene::store) parents: usize,
    pub(in crate::canonical_scene::store) wide: usize,
    pub(in crate::canonical_scene::store) text: usize,
    pub(in crate::canonical_scene::store) digests: usize,
}

impl LineageCounts {
    pub(in crate::canonical_scene::store) fn checked_add(self, other: Self) -> Option<Self> {
        Some(Self {
            resources: self.resources.checked_add(other.resources)?,
            sources: self.sources.checked_add(other.sources)?,
            provenance: self.provenance.checked_add(other.provenance)?,
            operations: self.operations.checked_add(other.operations)?,
            evidence: self.evidence.checked_add(other.evidence)?,
            parents: self.parents.checked_add(other.parents)?,
            wide: self.wide.checked_add(other.wide)?,
            text: self.text.checked_add(other.text)?,
            digests: self.digests.checked_add(other.digests)?,
        })
    }
}

pub(in crate::canonical_scene::store) struct LineageSegment {
    pub(in crate::canonical_scene::store) nonce: ProfileSegmentNonce,
    pub(in crate::canonical_scene::store) resources: Vec<StoredResource>,
    pub(in crate::canonical_scene::store) sources: Vec<StoredSource>,
    pub(in crate::canonical_scene::store) provenance: Vec<StoredProvenance>,
    pub(in crate::canonical_scene::store) operations: Vec<StoredOperation>,
    pub(in crate::canonical_scene::store) evidence: Vec<StoredOperationEvidence>,
    pub(in crate::canonical_scene::store) parents: Vec<ProfileProvenanceRef>,
    pub(in crate::canonical_scene::store) text: String,
    // Semantic totals remain independent of whether values are inline or pooled.
    pub(in crate::canonical_scene::store) wide_value_count: usize,
    pub(in crate::canonical_scene::store) digest_byte_count: usize,
}

impl LineageSegment {
    pub(in crate::canonical_scene::store) fn counts(&self) -> LineageCounts {
        LineageCounts {
            resources: self.resources.len(),
            sources: self.sources.len(),
            provenance: self.provenance.len(),
            operations: self.operations.len(),
            evidence: self.evidence.len(),
            parents: self.parents.len(),
            wide: self.wide_value_count,
            text: self.text.len(),
            digests: self.digest_byte_count,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) enum StoredResourceStability {
    Stable {
        owner: TextRange,
        schema: TextRange,
        version: NonZeroU32,
    },
    SessionOpaque(SessionOpaqueReason),
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredResource {
    pub(in crate::canonical_scene::store) identity: TextRange,
    pub(in crate::canonical_scene::store) stability: StoredResourceStability,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) enum StoredSourceLocator {
    Indexed {
        kind: SourceObjectKind,
        index: u64,
    },
    Namespaced {
        schema: TextRange,
        version: NonZeroU32,
        value: TextRange,
    },
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredSource {
    pub(in crate::canonical_scene::store) resource: u32,
    pub(in crate::canonical_scene::store) format: TextRange,
    pub(in crate::canonical_scene::store) locator: StoredSourceLocator,
    pub(in crate::canonical_scene::store) display_name: Option<TextRange>,
    pub(in crate::canonical_scene::store) content_digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) enum StoredOperationKind {
    Generated,
    Default,
    Normalize,
    Repair,
    Split,
    Merge,
    Convert,
    Transform,
    Optimize,
    Namespaced(TextRange),
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredOperation {
    pub(in crate::canonical_scene::store) kind: StoredOperationKind,
    pub(in crate::canonical_scene::store) options: OptionsDigest,
    pub(in crate::canonical_scene::store) evidence: TableRange<StoredOperationEvidence>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) enum StoredProvenanceKind {
    Source(u32),
    Operation(u32),
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredProvenance {
    pub(in crate::canonical_scene::store) kind: StoredProvenanceKind,
    pub(in crate::canonical_scene::store) depth: u32,
    pub(in crate::canonical_scene::store) parents: TableRange<ProfileProvenanceRef>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredOperationEvidence {
    pub(in crate::canonical_scene::store) code: TextRange,
    pub(in crate::canonical_scene::store) occurrences: NonZeroU64,
    pub(in crate::canonical_scene::store) digest: Option<EvidenceDigest>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredProfileOwner {
    pub(in crate::canonical_scene::store) owner: CanonicalProfileOwner,
    pub(in crate::canonical_scene::store) attachments: TableRange<ProfileProvenanceRef>,
    pub(in crate::canonical_scene::store) maximum_depth: u32,
}

pub(in crate::canonical_scene::store) struct MetadataPayloads {
    pub(in crate::canonical_scene::store) text: String,
    pub(in crate::canonical_scene::store) bytes: Vec<u8>,
    pub(in crate::canonical_scene::store) bools: Vec<u8>,
    pub(in crate::canonical_scene::store) i64s: Vec<i64>,
    pub(in crate::canonical_scene::store) u64s: Vec<u64>,
    pub(in crate::canonical_scene::store) f64s: Vec<f64>,
    pub(in crate::canonical_scene::store) f32s: Vec<f32>,
}

/// Each variant fixes its scalar pool and its fixed arity or array element shape.
#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) enum StoredMetadataValue {
    Bool(TableRange<u8>),
    I64(TableRange<i64>),
    U64(TableRange<u64>),
    F64(TableRange<f64>),
    String(TextRange),
    Bytes {
        media_type: TextRange,
        payload: TableRange<u8>,
    },
    Vec2(TableRange<f32>),
    Vec3(TableRange<f32>),
    Vec4(TableRange<f32>),
    Color(TableRange<f32>),
    Mat4(TableRange<f32>),
    BoolArray(TableRange<u8>),
    I64Array(TableRange<i64>),
    U64Array(TableRange<u64>),
    F64Array(TableRange<f64>),
    Vec2Array(TableRange<f32>),
    Vec3Array(TableRange<f32>),
    Vec4Array(TableRange<f32>),
    ColorArray(TableRange<f32>),
    Mat4Array(TableRange<f32>),
}

#[derive(Debug, Clone, Copy)]
pub(in crate::canonical_scene::store) struct StoredMetadataEntry {
    pub(in crate::canonical_scene::store) owner: CanonicalProfileOwner,
    pub(in crate::canonical_scene::store) key: TextRange,
    pub(in crate::canonical_scene::store) value: StoredMetadataValue,
}

impl StoredProfileSegment {
    pub(in crate::canonical_scene::store) fn domain(
        &self,
        ordinal: usize,
    ) -> Option<ProfileSegmentAddressDomain> {
        Some(ProfileSegmentAddressDomain::new(
            u32::try_from(ordinal).ok()?,
            self.lineage.as_ref().nonce,
            u32::try_from(self.prefixes.provenance).ok()?,
            u32::try_from(self.lineage.as_ref().provenance.len()).ok()?,
        ))
    }
}

impl ProfileStorage {
    pub(in crate::canonical_scene::store) fn resolve(
        &self,
        reference: ProfileProvenanceRef,
    ) -> Option<(u32, &StoredProvenance)> {
        let ordinal = usize::try_from(reference.segment()).ok()?;
        let segment = self.state.as_ref().segments.get(ordinal)?;
        let global = reference.resolve(segment.domain(ordinal)?)?;
        let row = segment
            .lineage
            .as_ref()
            .provenance
            .get(usize::try_from(reference.local()).ok()?)?;
        Some((global, row))
    }
}
