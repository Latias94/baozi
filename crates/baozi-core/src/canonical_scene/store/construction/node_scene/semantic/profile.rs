use baozi_operation::{
    CanonicalRecordView, PublishedCanonicalStore, RecordBlock, ScalarSequence, SequenceView,
    StoredScalar, Utf8Block,
};

use crate::canonical_scene::store::production::{CanonicalProfileOwner, SessionOpaqueReason};
use crate::canonical_scene::store::profile_provenance::{
    PROFILE_PROVENANCE_ADDRESS_SCALARS, QualifiedProvenanceAddressSlice,
};
use crate::canonical_scene::store::schema::profile::*;
use crate::canonical_scene::store::schema::{
    CanonicalBlockRef, CanonicalSchema, SceneDirectorySchema,
};
use crate::canonical_scene::store::validation::NodeSceneValidationProof;
use crate::provenance::{DigestAlgorithm, SourceObjectKind};

use super::{NodeSceneSemanticResult, invariant, read_scalar, read_u32_as_usize};

type SceneCanonicalStore = PublishedCanonicalStore<RecordBlock, SceneDirectorySchema>;
type OptionalCanonicalValue<'scene, Tag, Value> =
    (Option<CanonicalBlockRef<Tag>>, Option<&'scene Value>);

mod metadata;
mod storage;

pub(in crate::canonical_scene::store) use metadata::{
    NodeSceneProfileMetadataArray, NodeSceneProfileMetadataArrayKind,
    NodeSceneProfileMetadataEntry, NodeSceneProfileMetadataValue,
};
use metadata::{decode_metadata_value, metadata_shape, require_metadata_lane_references};
use storage::*;

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileProvenanceIndices<'scene> {
    addresses: QualifiedProvenanceAddressSlice<'scene>,
}

impl<'scene> NodeSceneProfileProvenanceIndices<'scene> {
    const fn new(addresses: QualifiedProvenanceAddressSlice<'scene>) -> Self {
        Self { addresses }
    }

    pub(in crate::canonical_scene::store) const fn len(self) -> usize {
        self.addresses.len()
    }

    pub(in crate::canonical_scene::store) fn iter(
        self,
    ) -> impl ExactSizeIterator<Item = Option<u32>> + 'scene {
        self.addresses.global_indices()
    }
}

impl std::fmt::Debug for NodeSceneProfileProvenanceIndices<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<'left, 'right> PartialEq<NodeSceneProfileProvenanceIndices<'right>>
    for NodeSceneProfileProvenanceIndices<'left>
{
    fn eq(&self, other: &NodeSceneProfileProvenanceIndices<'right>) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl Eq for NodeSceneProfileProvenanceIndices<'_> {}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct ProfileLineageCounts {
    resources: usize,
    sources: usize,
    provenance: usize,
    operations: usize,
    evidence: usize,
    parents: usize,
    wide: usize,
    text: usize,
    digests: usize,
}

impl ProfileLineageCounts {
    fn checked_add(self, other: Self) -> NodeSceneSemanticResult<Self> {
        Ok(Self {
            resources: checked_add(self.resources, other.resources)?,
            sources: checked_add(self.sources, other.sources)?,
            provenance: checked_add(self.provenance, other.provenance)?,
            operations: checked_add(self.operations, other.operations)?,
            evidence: checked_add(self.evidence, other.evidence)?,
            parents: checked_add(self.parents, other.parents)?,
            wide: checked_add(self.wide, other.wide)?,
            text: checked_add(self.text, other.text)?,
            digests: checked_add(self.digests, other.digests)?,
        })
    }
}

#[derive(Clone, Copy)]
struct ProfileSegmentProgress {
    next_segment: usize,
    lineage_prefixes: ProfileLineageCounts,
}

impl ProfileSegmentProgress {
    const fn empty() -> Self {
        Self {
            next_segment: 0,
            lineage_prefixes: ProfileLineageCounts {
                resources: 0,
                sources: 0,
                provenance: 0,
                operations: 0,
                evidence: 0,
                parents: 0,
                wide: 0,
                text: 0,
                digests: 0,
            },
        }
    }
}

struct ProfileLineageCursor<'scene> {
    progress: ProfileSegmentProgress,
    active_segment: Option<ProfileSegmentDecoder<'scene>>,
}

impl ProfileLineageCursor<'_> {
    const fn empty() -> Self {
        Self {
            progress: ProfileSegmentProgress::empty(),
            active_segment: None,
        }
    }

    fn reset(&mut self) {
        self.progress = ProfileSegmentProgress::empty();
        self.active_segment = None;
    }
}

#[derive(Clone, Copy)]
pub(super) struct NodeSceneProfileTables<'scene> {
    canonical: &'scene SceneCanonicalStore,
    segments: CanonicalRecordView<'scene>,
    owners: CanonicalRecordView<'scene>,
    attachments_ref: CanonicalBlockRef<ProfileLineageAttachmentSchema>,
    attachments: QualifiedProvenanceAddressSlice<'scene>,
    metadata_entries: Option<CanonicalRecordView<'scene>>,
    metadata_text_ref: Option<CanonicalBlockRef<ProfileMetadataTextSchema>>,
    metadata_bytes_ref: Option<CanonicalBlockRef<ProfileMetadataBytesSchema>>,
    metadata_bools_ref: Option<CanonicalBlockRef<ProfileMetadataBoolSchema>>,
    metadata_i64s_ref: Option<CanonicalBlockRef<ProfileMetadataI64Schema>>,
    metadata_u64s_ref: Option<CanonicalBlockRef<ProfileMetadataU64Schema>>,
    metadata_f64s_ref: Option<CanonicalBlockRef<ProfileMetadataF64Schema>>,
    metadata_f32s_ref: Option<CanonicalBlockRef<ProfileMetadataF32Schema>>,
    metadata_text: Option<&'scene str>,
    metadata_bytes: Option<&'scene [u8]>,
    metadata_bools: Option<&'scene [u8]>,
    metadata_i64s: Option<&'scene [i64]>,
    metadata_u64s: Option<&'scene [u64]>,
    metadata_f64s: Option<&'scene [f64]>,
    metadata_f32s: Option<&'scene [f32]>,
    lineage_counts: ProfileLineageCounts,
}

impl<'scene> NodeSceneProfileTables<'scene> {
    pub(super) fn try_new(
        canonical: &'scene SceneCanonicalStore,
        proof: NodeSceneValidationProof,
    ) -> NodeSceneSemanticResult<Self> {
        let directory = canonical
            .records(proof.profile_directory().into_store())
            .ok_or_else(|| invariant("validated profile directory is unavailable or mistyped"))?;
        require_table_layout(
            directory,
            1,
            PROFILE_DIRECTORY_FIELD_COUNT,
            "validated profile directory layout disagrees with its schema",
        )?;
        if read_scalar::<u32>(
            directory,
            0,
            profile_directory_field::SCHEMA_REVISION,
            "validated profile schema revision is unreadable",
        )? != PROFILE_SCHEMA_REVISION
        {
            return Err(invariant(
                "validated profile schema revision is unsupported",
            ));
        }

        let segment_count = read_u32_as_usize(
            directory,
            0,
            profile_directory_field::SEGMENT_COUNT,
            "validated profile segment count is unreadable",
        )?;
        if segment_count == 0 || segment_count != proof.profile_segment_count() {
            return Err(invariant(
                "validated profile segment count disagrees with validation proof",
            ));
        }
        require_record_reference(
            directory,
            0,
            profile_directory_field::SEGMENTS,
            proof.profile_segments(),
            "validated profile segment-table reference is unreadable",
            "validated profile segment-table reference disagrees with validation proof",
        )?;
        let segments = canonical
            .records(proof.profile_segments().into_store())
            .ok_or_else(|| {
                invariant("validated profile segment table is unavailable or mistyped")
            })?;
        require_table_layout(
            segments,
            segment_count,
            PROFILE_SEGMENT_FIELD_COUNT,
            "validated profile segment-table layout disagrees with validation proof",
        )?;
        let lineage_counts = ProfileLineageCounts {
            resources: u32_to_usize(proof.profile_resource_count())?,
            sources: u32_to_usize(proof.profile_source_count())?,
            provenance: u32_to_usize(proof.profile_provenance_count())?,
            operations: u32_to_usize(proof.profile_operation_count())?,
            evidence: u32_to_usize(proof.profile_evidence_count())?,
            parents: u32_to_usize(proof.profile_parent_count())?,
            wide: u32_to_usize(proof.profile_wide_count())?,
            text: u32_to_usize(proof.profile_lineage_text_bytes())?,
            digests: u32_to_usize(proof.profile_lineage_digest_bytes())?,
        };

        let owner_count = read_u32_as_usize(
            directory,
            0,
            profile_directory_field::OWNER_COUNT,
            "validated profile owner count is unreadable",
        )?;
        if owner_count < 2 {
            return Err(invariant(
                "validated profile has fewer than its required owners",
            ));
        }
        require_record_reference(
            directory,
            0,
            profile_directory_field::OWNERS,
            proof.profile_owners(),
            "validated profile owner-table reference is unreadable",
            "validated profile owner-table reference disagrees with validation proof",
        )?;
        let owners = canonical
            .records(proof.profile_owners().into_store())
            .ok_or_else(|| invariant("validated profile owner table is unavailable or mistyped"))?;
        require_table_layout(
            owners,
            owner_count,
            PROFILE_OWNER_FIELD_COUNT,
            "validated profile owner-table layout disagrees with its directory",
        )?;
        let attachments_ref = owners
            .sequence_ref::<u64, ProfileLineageAttachmentSchema>(
                0,
                profile_owner_field::ATTACHMENTS,
            )
            .map(CanonicalBlockRef::from_store)
            .ok_or_else(|| invariant("validated profile attachment reference is unreadable"))?;
        let attachments = canonical
            .sequence(attachments_ref.into_store())
            .map(|values: SequenceView<'scene, u64>| values.as_slice())
            .filter(|values| !values.is_empty())
            .ok_or_else(|| invariant("validated profile attachments are unavailable or empty"))?;
        let attachments = QualifiedProvenanceAddressSlice::from_scalars(
            attachments,
            u32_to_usize(proof.counts().profile_attachment_count)?,
        )
        .ok_or_else(|| invariant("validated profile attachment width is inconsistent"))?;

        require_record_reference(
            directory,
            0,
            profile_directory_field::METADATA,
            proof.profile_metadata_directory(),
            "validated profile metadata-directory reference is unreadable",
            "validated profile metadata-directory reference disagrees with validation proof",
        )?;
        let metadata_directory = canonical
            .records(proof.profile_metadata_directory().into_store())
            .ok_or_else(|| {
                invariant("validated profile metadata directory is unavailable or mistyped")
            })?;
        require_table_layout(
            metadata_directory,
            1,
            PROFILE_METADATA_DIRECTORY_FIELD_COUNT,
            "validated profile metadata-directory layout disagrees with its schema",
        )?;
        if read_scalar::<u32>(
            metadata_directory,
            0,
            profile_metadata_directory_field::SCHEMA_REVISION,
            "validated profile metadata schema revision is unreadable",
        )? != PROFILE_METADATA_SCHEMA_REVISION
        {
            return Err(invariant(
                "validated profile metadata schema revision is unsupported",
            ));
        }

        let entry_count = read_u32_as_usize(
            metadata_directory,
            0,
            profile_metadata_directory_field::ENTRY_COUNT,
            "validated profile metadata-entry count is unreadable",
        )?;
        let metadata_entries = resolve_optional_table(
            canonical,
            metadata_directory,
            profile_metadata_directory_field::ENTRIES,
            entry_count,
            PROFILE_METADATA_ENTRY_FIELD_COUNT,
            proof.profile_metadata_entries(),
            "validated profile metadata-entry table is inconsistent",
        )?;

        let (metadata_text_ref, metadata_text) = resolve_optional_utf8(
            canonical,
            metadata_directory,
            profile_metadata_directory_field::TEXT_COUNT,
            profile_metadata_directory_field::TEXTS,
            "validated profile metadata text lane is inconsistent",
        )?;
        let (metadata_bytes_ref, metadata_bytes) = resolve_optional_bytes(
            canonical,
            metadata_directory,
            profile_metadata_directory_field::BYTE_COUNT,
            profile_metadata_directory_field::BYTES,
            "validated profile metadata byte lane is inconsistent",
        )?;
        let (metadata_bools_ref, metadata_bools) =
            resolve_optional_sequence::<u8, ProfileMetadataBoolSchema>(
                canonical,
                metadata_directory,
                0,
                profile_metadata_directory_field::BOOL_COUNT,
                profile_metadata_directory_field::BOOL_VALUES,
                1,
                "validated profile metadata bool lane is inconsistent",
            )?;
        let (metadata_i64s_ref, metadata_i64s) =
            resolve_optional_sequence::<i64, ProfileMetadataI64Schema>(
                canonical,
                metadata_directory,
                0,
                profile_metadata_directory_field::I64_COUNT,
                profile_metadata_directory_field::I64_VALUES,
                1,
                "validated profile metadata i64 lane is inconsistent",
            )?;
        let (metadata_u64s_ref, metadata_u64s) =
            resolve_optional_sequence::<u64, ProfileMetadataU64Schema>(
                canonical,
                metadata_directory,
                0,
                profile_metadata_directory_field::U64_COUNT,
                profile_metadata_directory_field::U64_VALUES,
                1,
                "validated profile metadata u64 lane is inconsistent",
            )?;
        let (metadata_f64s_ref, metadata_f64s) =
            resolve_optional_sequence::<f64, ProfileMetadataF64Schema>(
                canonical,
                metadata_directory,
                0,
                profile_metadata_directory_field::F64_COUNT,
                profile_metadata_directory_field::F64_VALUES,
                1,
                "validated profile metadata f64 lane is inconsistent",
            )?;
        let (metadata_f32s_ref, metadata_f32s) =
            resolve_optional_sequence::<f32, ProfileMetadataF32Schema>(
                canonical,
                metadata_directory,
                0,
                profile_metadata_directory_field::F32_COUNT,
                profile_metadata_directory_field::F32_VALUES,
                1,
                "validated profile metadata f32 lane is inconsistent",
            )?;

        Ok(Self {
            canonical,
            segments,
            owners,
            attachments_ref,
            attachments,
            metadata_entries,
            metadata_text_ref,
            metadata_bytes_ref,
            metadata_bools_ref,
            metadata_i64s_ref,
            metadata_u64s_ref,
            metadata_f64s_ref,
            metadata_f32s_ref,
            metadata_text,
            metadata_bytes,
            metadata_bools,
            metadata_i64s,
            metadata_u64s,
            metadata_f64s,
            metadata_f32s,
            lineage_counts,
        })
    }

    pub(super) const fn into_cursor(self) -> NodeSceneProfileCursor<'scene> {
        NodeSceneProfileCursor {
            tables: self,
            phase: ProfileCursorPhase::Resources,
            lineage: ProfileLineageCursor::empty(),
            next_owner: 0,
            next_attachment: 0,
            previous_owner: None,
            next_metadata_entry: 0,
            metadata_text_cursor: 0,
            metadata_byte_cursor: 0,
            metadata_bool_cursor: 0,
            metadata_i64_cursor: 0,
            metadata_u64_cursor: 0,
            metadata_f64_cursor: 0,
            metadata_f32_cursor: 0,
            previous_metadata_owner: None,
            previous_metadata_key: None,
            #[cfg(test)]
            decoded_lineage_rows: [0; 4],
            #[cfg(test)]
            lineage_segment_checkpoints: [0; 4],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ProfileCursorPhase {
    Resources,
    Sources,
    Provenance,
    Evidence,
    Owners,
    Metadata,
}

#[cfg(test)]
fn lineage_phase_slot(phase: ProfileCursorPhase) -> NodeSceneSemanticResult<usize> {
    match phase {
        ProfileCursorPhase::Resources => Ok(0),
        ProfileCursorPhase::Sources => Ok(1),
        ProfileCursorPhase::Provenance => Ok(2),
        ProfileCursorPhase::Evidence => Ok(3),
        ProfileCursorPhase::Owners | ProfileCursorPhase::Metadata => Err(invariant(
            "profile test counter received a non-lineage phase",
        )),
    }
}

/// One bounded advance through the proof-qualified profile projection.
///
/// A checkpoint resolves at most one physical lineage segment. It deliberately
/// carries no semantic value: physical segment framing is retained-storage
/// topology, not canonical Scene meaning.
pub(in crate::canonical_scene::store) enum NodeSceneProfileStep<T> {
    Checkpoint,
    Item(T),
    End,
}

#[must_use = "the profile cursor must be consumed or deliberately discarded"]
pub(in crate::canonical_scene::store) struct NodeSceneProfileCursor<'scene> {
    tables: NodeSceneProfileTables<'scene>,
    phase: ProfileCursorPhase,
    lineage: ProfileLineageCursor<'scene>,
    next_owner: usize,
    next_attachment: usize,
    previous_owner: Option<CanonicalProfileOwner>,
    next_metadata_entry: usize,
    metadata_text_cursor: usize,
    metadata_byte_cursor: usize,
    metadata_bool_cursor: usize,
    metadata_i64_cursor: usize,
    metadata_u64_cursor: usize,
    metadata_f64_cursor: usize,
    metadata_f32_cursor: usize,
    previous_metadata_owner: Option<CanonicalProfileOwner>,
    previous_metadata_key: Option<&'scene str>,
    #[cfg(test)]
    decoded_lineage_rows: [usize; 4],
    #[cfg(test)]
    lineage_segment_checkpoints: [usize; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileMetadataLanes<'scene> {
    text: Option<&'scene str>,
    bytes: Option<&'scene [u8]>,
    bools: Option<&'scene [u8]>,
    i64s: Option<&'scene [i64]>,
    u64s: Option<&'scene [u64]>,
    f64s: Option<&'scene [f64]>,
    f32s: Option<&'scene [f32]>,
}

impl<'scene> NodeSceneProfileMetadataLanes<'scene> {
    pub(in crate::canonical_scene::store) const fn text(self) -> Option<&'scene str> {
        self.text
    }

    pub(in crate::canonical_scene::store) const fn bytes(self) -> Option<&'scene [u8]> {
        self.bytes
    }

    pub(in crate::canonical_scene::store) const fn bools(self) -> Option<&'scene [u8]> {
        self.bools
    }

    pub(in crate::canonical_scene::store) const fn i64s(self) -> Option<&'scene [i64]> {
        self.i64s
    }

    pub(in crate::canonical_scene::store) const fn u64s(self) -> Option<&'scene [u64]> {
        self.u64s
    }

    pub(in crate::canonical_scene::store) const fn f64s(self) -> Option<&'scene [f64]> {
        self.f64s
    }

    pub(in crate::canonical_scene::store) const fn f32s(self) -> Option<&'scene [f32]> {
        self.f32s
    }
}

impl<'scene> NodeSceneProfileCursor<'scene> {
    pub(in crate::canonical_scene::store) const fn resource_count(&self) -> usize {
        self.tables.lineage_counts.resources
    }

    pub(in crate::canonical_scene::store) const fn source_count(&self) -> usize {
        self.tables.lineage_counts.sources
    }

    pub(in crate::canonical_scene::store) const fn provenance_count(&self) -> usize {
        self.tables.lineage_counts.provenance
    }

    pub(in crate::canonical_scene::store) const fn operation_count(&self) -> usize {
        self.tables.lineage_counts.operations
    }

    pub(in crate::canonical_scene::store) const fn evidence_count(&self) -> usize {
        self.tables.lineage_counts.evidence
    }

    pub(in crate::canonical_scene::store) const fn parent_count(&self) -> usize {
        self.tables.lineage_counts.parents
    }

    pub(in crate::canonical_scene::store) const fn wide_count(&self) -> usize {
        self.tables.lineage_counts.wide
    }

    pub(in crate::canonical_scene::store) const fn lineage_text_bytes(&self) -> usize {
        self.tables.lineage_counts.text
    }

    pub(in crate::canonical_scene::store) const fn lineage_digest_bytes(&self) -> usize {
        self.tables.lineage_counts.digests
    }

    pub(in crate::canonical_scene::store) const fn owner_count(&self) -> usize {
        self.tables.owners.rows()
    }

    pub(in crate::canonical_scene::store) const fn attachment_count(&self) -> usize {
        self.tables.attachments.len()
    }

    pub(in crate::canonical_scene::store) const fn metadata_entry_count(&self) -> usize {
        match self.tables.metadata_entries {
            Some(entries) => entries.rows(),
            None => 0,
        }
    }

    pub(in crate::canonical_scene::store) const fn metadata_lanes(
        &self,
    ) -> NodeSceneProfileMetadataLanes<'scene> {
        NodeSceneProfileMetadataLanes {
            text: self.tables.metadata_text,
            bytes: self.tables.metadata_bytes,
            bools: self.tables.metadata_bools,
            i64s: self.tables.metadata_i64s,
            u64s: self.tables.metadata_u64s,
            f64s: self.tables.metadata_f64s,
            f32s: self.tables.metadata_f32s,
        }
    }

    pub(in crate::canonical_scene::store) fn step_resource(
        &mut self,
    ) -> NodeSceneSemanticResult<NodeSceneProfileStep<NodeSceneProfileResource<'scene>>> {
        self.enter_lineage_phase(ProfileCursorPhase::Resources)?;
        if let Some(segment) = &mut self.lineage.active_segment {
            if let Some(resource) = segment.next_resource()? {
                #[cfg(test)]
                self.record_decoded_lineage_row(ProfileCursorPhase::Resources)?;
                return Ok(NodeSceneProfileStep::Item(resource));
            }
            self.complete_active_segment()?;
        }
        self.open_next_segment()
    }

    pub(in crate::canonical_scene::store) fn step_source(
        &mut self,
    ) -> NodeSceneSemanticResult<NodeSceneProfileStep<NodeSceneProfileSource<'scene>>> {
        self.enter_lineage_phase(ProfileCursorPhase::Sources)?;
        if let Some(segment) = &mut self.lineage.active_segment {
            if let Some(source) = segment.next_source()? {
                #[cfg(test)]
                self.record_decoded_lineage_row(ProfileCursorPhase::Sources)?;
                return Ok(NodeSceneProfileStep::Item(source));
            }
            self.complete_active_segment()?;
        }
        self.open_next_segment()
    }

    pub(in crate::canonical_scene::store) fn step_provenance(
        &mut self,
    ) -> NodeSceneSemanticResult<NodeSceneProfileStep<NodeSceneProfileProvenance<'scene>>> {
        self.enter_lineage_phase(ProfileCursorPhase::Provenance)?;
        if let Some(segment) = &mut self.lineage.active_segment {
            if let Some(provenance) = segment.next_provenance()? {
                #[cfg(test)]
                self.record_decoded_lineage_row(ProfileCursorPhase::Provenance)?;
                return Ok(NodeSceneProfileStep::Item(provenance));
            }
            self.complete_active_segment()?;
        }
        self.open_next_segment()
    }

    pub(in crate::canonical_scene::store) fn step_evidence(
        &mut self,
    ) -> NodeSceneSemanticResult<NodeSceneProfileStep<NodeSceneProfileEvidence<'scene>>> {
        self.enter_lineage_phase(ProfileCursorPhase::Evidence)?;
        if let Some(segment) = &mut self.lineage.active_segment {
            if let Some(evidence) = segment.next_evidence()? {
                #[cfg(test)]
                self.record_decoded_lineage_row(ProfileCursorPhase::Evidence)?;
                return Ok(NodeSceneProfileStep::Item(evidence));
            }
            self.complete_active_segment()?;
        }
        self.open_next_segment()
    }

    pub(in crate::canonical_scene::store) fn next_owner(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneProfileOwner<'scene>>> {
        self.enter_owner_phase()?;
        if self.next_owner == self.tables.owners.rows() {
            return Ok(None);
        }
        if self.next_owner > self.tables.owners.rows() {
            return Err(invariant(
                "profile owner cursor advanced beyond its proof-qualified count",
            ));
        }

        let row = self.next_owner;
        let owner = decode_owner(
            read_scalar::<u8>(
                self.tables.owners,
                row,
                profile_owner_field::OWNER_KIND,
                "validated profile owner kind is unreadable",
            )?,
            read_scalar::<u32>(
                self.tables.owners,
                row,
                profile_owner_field::OWNER_INDEX,
                "validated profile owner index is unreadable",
            )?,
        )?;
        require_owner_order(row, owner, self.previous_owner)?;

        let attachment_offset = read_u32_as_usize(
            self.tables.owners,
            row,
            profile_owner_field::ATTACHMENT_OFFSET,
            "validated profile attachment offset is unreadable",
        )?;
        let attachment_count = read_u32_as_usize(
            self.tables.owners,
            row,
            profile_owner_field::ATTACHMENT_COUNT,
            "validated profile attachment count is unreadable",
        )?;
        if attachment_count == 0 || attachment_offset != self.next_attachment {
            return Err(invariant(
                "validated profile attachment range is not canonical",
            ));
        }
        require_sequence_reference::<u64, ProfileLineageAttachmentSchema>(
            self.tables.owners,
            row,
            profile_owner_field::ATTACHMENTS,
            self.tables.attachments_ref,
            "validated profile attachment reference is unreadable",
            "validated profile attachment reference disagrees across owners",
        )?;
        let attachments = self
            .tables
            .attachments
            .get_range(attachment_offset, attachment_count)
            .ok_or_else(|| invariant("validated profile attachment range is unreadable"))?;

        self.next_attachment = checked_add(attachment_offset, attachment_count)?;
        self.next_owner = advance(row, "profile owner cursor index overflows")?;
        self.previous_owner = Some(owner);
        Ok(Some(NodeSceneProfileOwner {
            owner,
            attachment_offset,
            attachments: NodeSceneProfileProvenanceIndices::new(attachments),
        }))
    }

    pub(in crate::canonical_scene::store) fn next_metadata_entry(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneProfileMetadataEntry<'scene>>> {
        self.enter_metadata_phase()?;
        let entry_count = self.metadata_entry_count();
        if self.next_metadata_entry == entry_count {
            return Ok(None);
        }
        if self.next_metadata_entry > entry_count {
            return Err(invariant(
                "profile metadata cursor advanced beyond its proof-qualified count",
            ));
        }
        let entries = self
            .tables
            .metadata_entries
            .ok_or_else(|| invariant("validated profile metadata-entry table is absent"))?;
        let row = self.next_metadata_entry;
        let owner = decode_owner(
            read_scalar::<u8>(
                entries,
                row,
                profile_metadata_entry_field::OWNER_KIND,
                "validated profile metadata owner kind is unreadable",
            )?,
            read_scalar::<u32>(
                entries,
                row,
                profile_metadata_entry_field::OWNER_INDEX,
                "validated profile metadata owner index is unreadable",
            )?,
        )?;
        let key = consume_utf8(
            self.tables.metadata_text,
            &mut self.metadata_text_cursor,
            read_u32_as_usize(
                entries,
                row,
                profile_metadata_entry_field::KEY_OFFSET,
                "validated profile metadata key offset is unreadable",
            )?,
            read_u32_as_usize(
                entries,
                row,
                profile_metadata_entry_field::KEY_COUNT,
                "validated profile metadata key count is unreadable",
            )?,
            true,
            "validated profile metadata key range is inconsistent",
        )?;
        if self
            .previous_metadata_owner
            .is_some_and(|previous| owner < previous)
            || self.previous_metadata_owner == Some(owner)
                && self
                    .previous_metadata_key
                    .is_some_and(|previous| previous.as_bytes() >= key.as_bytes())
        {
            return Err(invariant(
                "validated profile metadata entries are not canonically ordered",
            ));
        }

        let value_kind = read_scalar::<u8>(
            entries,
            row,
            profile_metadata_entry_field::VALUE_KIND,
            "validated profile metadata value kind is unreadable",
        )?;
        let array_kind = read_scalar::<u8>(
            entries,
            row,
            profile_metadata_entry_field::ARRAY_KIND,
            "validated profile metadata array kind is unreadable",
        )?;
        let logical_count = read_u32_as_usize(
            entries,
            row,
            profile_metadata_entry_field::LOGICAL_ELEMENT_COUNT,
            "validated profile metadata logical count is unreadable",
        )?;
        let shape = metadata_shape(value_kind, array_kind, logical_count)?;

        let auxiliary = consume_utf8(
            self.tables.metadata_text,
            &mut self.metadata_text_cursor,
            read_u32_as_usize(
                entries,
                row,
                profile_metadata_entry_field::AUX_TEXT_OFFSET,
                "validated profile metadata auxiliary-text offset is unreadable",
            )?,
            read_u32_as_usize(
                entries,
                row,
                profile_metadata_entry_field::AUX_TEXT_COUNT,
                "validated profile metadata auxiliary-text count is unreadable",
            )?,
            value_kind == PROFILE_METADATA_BYTES,
            "validated profile metadata auxiliary-text range is inconsistent",
        )?;
        if !shape.auxiliary_text && !auxiliary.is_empty() {
            return Err(invariant(
                "validated profile metadata value has unexpected auxiliary text",
            ));
        }

        let byte_offset = read_u32_as_usize(
            entries,
            row,
            profile_metadata_entry_field::BYTE_OFFSET,
            "validated profile metadata byte offset is unreadable",
        )?;
        let byte_count = read_u32_as_usize(
            entries,
            row,
            profile_metadata_entry_field::BYTE_COUNT,
            "validated profile metadata byte count is unreadable",
        )?;
        if byte_offset != self.metadata_byte_cursor || !shape.bytes && byte_count != 0 {
            return Err(invariant(
                "validated profile metadata byte range is inconsistent",
            ));
        }
        let payload = slice_optional_range(
            self.tables.metadata_bytes,
            byte_offset,
            byte_count,
            "validated profile metadata byte range is unreadable",
        )?;
        self.metadata_byte_cursor = checked_add(byte_offset, byte_count)?;

        require_metadata_lane_references(entries, row, shape, &self.tables, byte_count)?;
        let scalar_offset = read_u32_as_usize(
            entries,
            row,
            profile_metadata_entry_field::SCALAR_OFFSET,
            "validated profile metadata scalar offset is unreadable",
        )?;
        let scalar_count = read_u32_as_usize(
            entries,
            row,
            profile_metadata_entry_field::SCALAR_COUNT,
            "validated profile metadata scalar count is unreadable",
        )?;
        if scalar_count != shape.scalar_count {
            return Err(invariant(
                "validated profile metadata scalar count disagrees with its value kind",
            ));
        }
        let value = self.read_metadata_value(
            value_kind,
            array_kind,
            logical_count,
            shape,
            auxiliary,
            payload,
            scalar_offset,
            scalar_count,
        )?;
        let provenance = read_u32_as_usize(
            entries,
            row,
            profile_metadata_entry_field::PROVENANCE_INDEX,
            "validated profile metadata provenance index is unreadable",
        )?;
        if provenance >= self.tables.lineage_counts.provenance {
            return Err(invariant(
                "validated profile metadata provenance is outside provenance space",
            ));
        }

        self.next_metadata_entry = advance(row, "profile metadata cursor index overflows")?;
        self.previous_metadata_owner = Some(owner);
        self.previous_metadata_key = Some(key);
        Ok(Some(NodeSceneProfileMetadataEntry::new(
            owner, key, value, provenance,
        )))
    }

    pub(in crate::canonical_scene::store) fn finish(mut self) -> NodeSceneSemanticResult<()> {
        self.enter_metadata_phase()?;
        if self.next_metadata_entry != self.metadata_entry_count()
            || self.metadata_text_cursor != payload_len_str(self.tables.metadata_text)
            || self.metadata_byte_cursor != payload_len(self.tables.metadata_bytes)
            || self.metadata_bool_cursor != payload_len(self.tables.metadata_bools)
            || self.metadata_i64_cursor != payload_len(self.tables.metadata_i64s)
            || self.metadata_u64_cursor != payload_len(self.tables.metadata_u64s)
            || self.metadata_f64_cursor != payload_len(self.tables.metadata_f64s)
            || self.metadata_f32_cursor != payload_len(self.tables.metadata_f32s)
        {
            return Err(invariant(
                "profile cursor ended before all metadata rows and lanes were exhausted",
            ));
        }
        Ok(())
    }

    fn enter_lineage_phase(&mut self, target: ProfileCursorPhase) -> NodeSceneSemanticResult<()> {
        if target > ProfileCursorPhase::Evidence {
            return Err(invariant("profile lineage phase is invalid"));
        }
        if self.phase > target {
            return Err(invariant(
                "profile cursor cannot return to an earlier lineage phase",
            ));
        }
        if self.phase < target {
            if !self.lineage_is_exhausted() {
                return Err(invariant(
                    "profile lineage phase must be exhausted before advancing",
                ));
            }
            self.lineage.reset();
            self.phase = target;
        }
        Ok(())
    }

    fn lineage_is_exhausted(&self) -> bool {
        self.lineage.active_segment.is_none()
            && self.lineage.progress.next_segment == self.tables.segments.rows()
            && self.lineage.progress.lineage_prefixes == self.tables.lineage_counts
    }

    fn open_next_segment<T>(&mut self) -> NodeSceneSemanticResult<NodeSceneProfileStep<T>> {
        let row = self.lineage.progress.next_segment;
        if row == self.tables.segments.rows() {
            return Ok(NodeSceneProfileStep::End);
        }
        if row > self.tables.segments.rows() {
            return Err(invariant(
                "profile lineage cursor advanced beyond its proof-qualified segments",
            ));
        }

        let segment = resolve_segment(
            self.tables.canonical,
            self.tables.segments,
            row,
            self.lineage.progress.lineage_prefixes,
        )?;
        self.lineage.active_segment = Some(segment);
        #[cfg(test)]
        self.record_lineage_segment_checkpoint()?;
        Ok(NodeSceneProfileStep::Checkpoint)
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn decoded_lineage_rows_for_test(
        &self,
    ) -> [usize; 4] {
        self.decoded_lineage_rows
    }

    #[cfg(test)]
    pub(in crate::canonical_scene::store) const fn lineage_segment_checkpoints_for_test(
        &self,
    ) -> [usize; 4] {
        self.lineage_segment_checkpoints
    }

    #[cfg(test)]
    fn record_decoded_lineage_row(
        &mut self,
        phase: ProfileCursorPhase,
    ) -> NodeSceneSemanticResult<()> {
        let slot = lineage_phase_slot(phase)?;
        self.decoded_lineage_rows[slot] = self.decoded_lineage_rows[slot]
            .checked_add(1)
            .ok_or_else(|| invariant("profile test decode counter overflows"))?;
        Ok(())
    }

    #[cfg(test)]
    fn record_lineage_segment_checkpoint(&mut self) -> NodeSceneSemanticResult<()> {
        let slot = lineage_phase_slot(self.phase)?;
        self.lineage_segment_checkpoints[slot] = self.lineage_segment_checkpoints[slot]
            .checked_add(1)
            .ok_or_else(|| invariant("profile test checkpoint counter overflows"))?;
        Ok(())
    }

    fn complete_active_segment(&mut self) -> NodeSceneSemanticResult<()> {
        let segment = self
            .lineage
            .active_segment
            .take()
            .ok_or_else(|| invariant("profile lineage segment is unavailable"))?;
        self.lineage.progress = segment.completed_progress;
        Ok(())
    }

    fn enter_owner_phase(&mut self) -> NodeSceneSemanticResult<()> {
        if self.phase > ProfileCursorPhase::Owners {
            return Err(invariant("profile cursor cannot return to the owner phase"));
        }
        if self.phase < ProfileCursorPhase::Owners
            && (self.phase != ProfileCursorPhase::Evidence || !self.lineage_is_exhausted())
        {
            return Err(invariant(
                "global profile lineage must be exhausted before owners",
            ));
        }
        self.phase = ProfileCursorPhase::Owners;
        Ok(())
    }

    fn enter_metadata_phase(&mut self) -> NodeSceneSemanticResult<()> {
        if self.phase < ProfileCursorPhase::Metadata {
            self.enter_owner_phase()?;
        }
        if self.next_owner != self.tables.owners.rows()
            || self.next_attachment != self.tables.attachments.len()
        {
            return Err(invariant(
                "profile owners must be exhausted before metadata",
            ));
        }
        self.phase = ProfileCursorPhase::Metadata;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn read_metadata_value(
        &mut self,
        value_kind: u8,
        array_kind: u8,
        logical_count: usize,
        shape: MetadataShape,
        auxiliary: &'scene str,
        payload: &'scene [u8],
        offset: usize,
        count: usize,
    ) -> NodeSceneSemanticResult<NodeSceneProfileMetadataValue<'scene>> {
        let lane = match shape.lane {
            MetadataLane::None => {
                if offset != 0 || count != 0 {
                    return Err(invariant(
                        "validated profile metadata non-scalar range is not empty",
                    ));
                }
                NodeSceneProfileMetadataArray::None
            }
            MetadataLane::Bool => {
                let values = consume_sequence(
                    self.tables.metadata_bools,
                    &mut self.metadata_bool_cursor,
                    offset,
                    count,
                    "validated profile metadata bool range is inconsistent",
                )?;
                NodeSceneProfileMetadataArray::Bool(values)
            }
            MetadataLane::I64 => NodeSceneProfileMetadataArray::I64(consume_sequence(
                self.tables.metadata_i64s,
                &mut self.metadata_i64_cursor,
                offset,
                count,
                "validated profile metadata i64 range is inconsistent",
            )?),
            MetadataLane::U64 => NodeSceneProfileMetadataArray::U64(consume_sequence(
                self.tables.metadata_u64s,
                &mut self.metadata_u64_cursor,
                offset,
                count,
                "validated profile metadata u64 range is inconsistent",
            )?),
            MetadataLane::F64 => {
                let values = consume_sequence(
                    self.tables.metadata_f64s,
                    &mut self.metadata_f64_cursor,
                    offset,
                    count,
                    "validated profile metadata f64 range is inconsistent",
                )?;
                NodeSceneProfileMetadataArray::F64(values)
            }
            MetadataLane::F32 => {
                let values = consume_sequence(
                    self.tables.metadata_f32s,
                    &mut self.metadata_f32_cursor,
                    offset,
                    count,
                    "validated profile metadata f32 range is inconsistent",
                )?;
                NodeSceneProfileMetadataArray::F32(values)
            }
        };

        decode_metadata_value(
            value_kind,
            array_kind,
            logical_count,
            auxiliary,
            payload,
            lane,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileOwner<'scene> {
    owner: CanonicalProfileOwner,
    attachment_offset: usize,
    attachments: NodeSceneProfileProvenanceIndices<'scene>,
}

impl<'scene> NodeSceneProfileOwner<'scene> {
    pub(in crate::canonical_scene::store) const fn owner(self) -> CanonicalProfileOwner {
        self.owner
    }

    pub(in crate::canonical_scene::store) const fn attachment_offset(self) -> usize {
        self.attachment_offset
    }

    pub(in crate::canonical_scene::store) const fn attachments(
        self,
    ) -> NodeSceneProfileProvenanceIndices<'scene> {
        self.attachments
    }
}

#[must_use = "the profile segment decoder must be consumed or deliberately discarded"]
struct ProfileSegmentDecoder<'scene> {
    completed_progress: ProfileSegmentProgress,
    counts: ProfileLineageCounts,
    prefixes: ProfileLineageCounts,
    resources: &'scene [u32],
    sources: &'scene [u32],
    provenance: &'scene [u32],
    operations: &'scene [u32],
    evidence: &'scene [u32],
    parents: QualifiedProvenanceAddressSlice<'scene>,
    wide: &'scene [u64],
    text: &'scene str,
    digests: &'scene [u8],
    next_resource: usize,
    next_source: usize,
    next_provenance: usize,
    next_evidence: usize,
}

// Central validation proves lane continuity once. Semantic projection therefore
// addresses each proof-qualified row range directly instead of replaying prior
// domains to reconstruct a physical lane cursor.
fn read_utf8_at<'scene>(
    text: &'scene str,
    offset: usize,
    count: usize,
    nonempty: bool,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene str> {
    let mut direct = offset;
    consume_utf8(Some(text), &mut direct, offset, count, nonempty, reason)
}

fn read_optional_utf8_at<'scene>(
    text: &'scene str,
    presence: u32,
    offset: u32,
    count: u32,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<&'scene str>> {
    let mut direct = u32_to_usize(offset)?;
    consume_optional_utf8(text, &mut direct, presence, offset, count, reason)
}

fn read_required_digest_at<'scene>(
    digests: &'scene [u8],
    algorithm: u32,
    offset: u32,
    count: u32,
    reason: &'static str,
) -> NodeSceneSemanticResult<NodeSceneProfileDigest<'scene>> {
    let mut direct = u32_to_usize(offset)?;
    consume_required_digest(digests, &mut direct, algorithm, offset, count, reason)
}

fn read_optional_digest_at<'scene>(
    digests: &'scene [u8],
    presence: u32,
    algorithm: u32,
    offset: u32,
    count: u32,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<NodeSceneProfileDigest<'scene>>> {
    let mut direct = u32_to_usize(offset)?;
    consume_optional_digest(
        digests,
        &mut direct,
        presence,
        algorithm,
        offset,
        count,
        reason,
    )
}

fn read_slice_at<'scene, T>(
    values: &'scene [T],
    offset: usize,
    count: usize,
    expected_count: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<&'scene [T]> {
    let mut direct = offset;
    consume_slice(values, &mut direct, offset, count, expected_count, reason)
}

fn require_empty_range_at(
    payload_len: usize,
    offset: u32,
    count: u32,
    reason: &'static str,
) -> NodeSceneSemanticResult<()> {
    let direct = u32_to_usize(offset)?;
    if direct > payload_len {
        return Err(invariant(reason));
    }
    require_empty_range(direct, offset, count, reason)
}

impl<'scene> ProfileSegmentDecoder<'scene> {
    fn next_resource(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneProfileResource<'scene>>> {
        let Some(row) = next_row(self.next_resource, self.counts.resources)? else {
            return Ok(None);
        };
        let values = scalar_row(
            self.resources,
            row,
            PROFILE_RESOURCE_SCALAR_COUNT,
            "validated profile resource row is unreadable",
        )?;
        let identity = read_utf8_at(
            self.text,
            u32_to_usize(values[profile_resource_scalar::IDENTITY_OFFSET])?,
            u32_to_usize(values[profile_resource_scalar::IDENTITY_COUNT])?,
            true,
            "validated profile resource identity range is inconsistent",
        )?;
        let stability = match values[profile_resource_scalar::STABILITY_KIND] {
            value if value == u32::from(PROFILE_RESOURCE_STABILITY_STABLE) => {
                let owner = read_utf8_at(
                    self.text,
                    u32_to_usize(values[profile_resource_scalar::STABILITY_OWNER_OFFSET])?,
                    u32_to_usize(values[profile_resource_scalar::STABILITY_OWNER_COUNT])?,
                    true,
                    "validated stable-resource owner range is inconsistent",
                )?;
                let schema = read_utf8_at(
                    self.text,
                    u32_to_usize(values[profile_resource_scalar::STABILITY_SCHEMA_OFFSET])?,
                    u32_to_usize(values[profile_resource_scalar::STABILITY_SCHEMA_COUNT])?,
                    true,
                    "validated stable-resource schema range is inconsistent",
                )?;
                let schema_version = values[profile_resource_scalar::STABILITY_SCHEMA_VERSION];
                if schema_version == 0
                    || values[profile_resource_scalar::SESSION_OPAQUE_REASON] != 0
                {
                    return Err(invariant(
                        "validated stable-resource classification is inconsistent",
                    ));
                }
                NodeSceneProfileResourceStability::Stable {
                    owner,
                    schema,
                    schema_version,
                }
            }
            value if value == u32::from(PROFILE_RESOURCE_STABILITY_SESSION_OPAQUE) => {
                require_empty_range_at(
                    self.text.len(),
                    values[profile_resource_scalar::STABILITY_OWNER_OFFSET],
                    values[profile_resource_scalar::STABILITY_OWNER_COUNT],
                    "validated opaque-resource owner range is not empty",
                )?;
                require_empty_range_at(
                    self.text.len(),
                    values[profile_resource_scalar::STABILITY_SCHEMA_OFFSET],
                    values[profile_resource_scalar::STABILITY_SCHEMA_COUNT],
                    "validated opaque-resource schema range is not empty",
                )?;
                if values[profile_resource_scalar::STABILITY_SCHEMA_VERSION] != 0 {
                    return Err(invariant(
                        "validated opaque-resource schema version is nonzero",
                    ));
                }
                NodeSceneProfileResourceStability::SessionOpaque(decode_session_opaque_reason(
                    values[profile_resource_scalar::SESSION_OPAQUE_REASON],
                )?)
            }
            _ => {
                return Err(invariant(
                    "validated profile resource stability kind is unknown",
                ));
            }
        };

        self.next_resource = advance(row, "profile resource cursor index overflows")?;
        Ok(Some(NodeSceneProfileResource {
            index: checked_add(self.prefixes.resources, row)?,
            identity,
            stability,
        }))
    }

    fn next_source(&mut self) -> NodeSceneSemanticResult<Option<NodeSceneProfileSource<'scene>>> {
        let Some(row) = next_row(self.next_source, self.counts.sources)? else {
            return Ok(None);
        };
        let values = scalar_row(
            self.sources,
            row,
            PROFILE_SOURCE_SCALAR_COUNT,
            "validated profile source row is unreadable",
        )?;
        let resource = u32_to_usize(values[profile_source_scalar::RESOURCE])?;
        let visible_resource_end = checked_add(self.prefixes.resources, self.counts.resources)?;
        if resource >= visible_resource_end {
            return Err(invariant(
                "validated profile source resource is outside resource space",
            ));
        }
        let format = read_utf8_at(
            self.text,
            u32_to_usize(values[profile_source_scalar::FORMAT_OFFSET])?,
            u32_to_usize(values[profile_source_scalar::FORMAT_COUNT])?,
            true,
            "validated profile source format range is inconsistent",
        )?;
        let locator = match values[profile_source_scalar::LOCATOR_KIND] {
            value if value == u32::from(PROFILE_LOCATOR_INDEXED) => {
                let kind =
                    decode_source_object_kind(values[profile_source_scalar::INDEXED_OBJECT_KIND])?;
                let wide = read_slice_at(
                    self.wide,
                    u32_to_usize(values[profile_source_scalar::WIDE_OFFSET])?,
                    u32_to_usize(values[profile_source_scalar::WIDE_COUNT])?,
                    1,
                    "validated indexed source locator range is inconsistent",
                )?;
                require_empty_range_at(
                    self.text.len(),
                    values[profile_source_scalar::LOCATOR_SCHEMA_OFFSET],
                    values[profile_source_scalar::LOCATOR_SCHEMA_COUNT],
                    "validated indexed source locator schema range is not empty",
                )?;
                require_empty_range_at(
                    self.text.len(),
                    values[profile_source_scalar::LOCATOR_VALUE_OFFSET],
                    values[profile_source_scalar::LOCATOR_VALUE_COUNT],
                    "validated indexed source locator value range is not empty",
                )?;
                if values[profile_source_scalar::LOCATOR_SCHEMA_VERSION] != 0 {
                    return Err(invariant(
                        "validated indexed source locator has a schema version",
                    ));
                }
                NodeSceneProfileSourceLocator::Indexed {
                    kind,
                    index: wide[0],
                }
            }
            value if value == u32::from(PROFILE_LOCATOR_NAMESPACED) => {
                if values[profile_source_scalar::INDEXED_OBJECT_KIND] != 0 {
                    return Err(invariant(
                        "validated namespaced source locator has an indexed kind",
                    ));
                }
                require_empty_range_at(
                    self.wide.len(),
                    values[profile_source_scalar::WIDE_OFFSET],
                    values[profile_source_scalar::WIDE_COUNT],
                    "validated namespaced source locator wide range is not empty",
                )?;
                let schema = read_utf8_at(
                    self.text,
                    u32_to_usize(values[profile_source_scalar::LOCATOR_SCHEMA_OFFSET])?,
                    u32_to_usize(values[profile_source_scalar::LOCATOR_SCHEMA_COUNT])?,
                    true,
                    "validated namespaced source locator schema range is inconsistent",
                )?;
                let schema_version = values[profile_source_scalar::LOCATOR_SCHEMA_VERSION];
                if schema_version == 0 {
                    return Err(invariant(
                        "validated namespaced source locator schema version is zero",
                    ));
                }
                let value = read_utf8_at(
                    self.text,
                    u32_to_usize(values[profile_source_scalar::LOCATOR_VALUE_OFFSET])?,
                    u32_to_usize(values[profile_source_scalar::LOCATOR_VALUE_COUNT])?,
                    true,
                    "validated namespaced source locator value range is inconsistent",
                )?;
                NodeSceneProfileSourceLocator::Namespaced {
                    schema,
                    schema_version,
                    value,
                }
            }
            _ => {
                return Err(invariant(
                    "validated profile source locator kind is unknown",
                ));
            }
        };
        let display_name = read_optional_utf8_at(
            self.text,
            values[profile_source_scalar::DISPLAY_NAME_PRESENCE],
            values[profile_source_scalar::DISPLAY_NAME_OFFSET],
            values[profile_source_scalar::DISPLAY_NAME_COUNT],
            "validated profile source display-name range is inconsistent",
        )?;
        let content_digest = read_optional_digest_at(
            self.digests,
            values[profile_source_scalar::CONTENT_DIGEST_PRESENCE],
            values[profile_source_scalar::CONTENT_DIGEST_ALGORITHM],
            values[profile_source_scalar::CONTENT_DIGEST_OFFSET],
            values[profile_source_scalar::CONTENT_DIGEST_COUNT],
            "validated profile source content digest is inconsistent",
        )?;

        self.next_source = advance(row, "profile source cursor index overflows")?;
        Ok(Some(NodeSceneProfileSource {
            index: checked_add(self.prefixes.sources, row)?,
            resource,
            format,
            locator,
            display_name,
            content_digest,
        }))
    }

    fn next_provenance(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneProfileProvenance<'scene>>> {
        let Some(row) = next_row(self.next_provenance, self.counts.provenance)? else {
            return Ok(None);
        };
        let values = scalar_row(
            self.provenance,
            row,
            PROFILE_PROVENANCE_SCALAR_COUNT,
            "validated profile provenance row is unreadable",
        )?;
        let index = checked_add(self.prefixes.provenance, row)?;
        let depth = u32_to_usize(values[profile_provenance_scalar::DEPTH])?;
        let parent_offset = u32_to_usize(values[profile_provenance_scalar::PARENT_OFFSET])?;
        let parent_count = u32_to_usize(values[profile_provenance_scalar::PARENT_COUNT])?;
        let parents = self
            .parents
            .get_range(parent_offset, parent_count)
            .ok_or_else(|| invariant("validated profile provenance parent range is unreadable"))?;
        let kind = match values[profile_provenance_scalar::KIND] {
            value if value == u32::from(PROFILE_PROVENANCE_SOURCE) => {
                if depth != 1 || parent_count != 0 {
                    return Err(invariant(
                        "validated source provenance node has operation structure",
                    ));
                }
                let source = u32_to_usize(values[profile_provenance_scalar::VARIANT_INDEX])?;
                let visible_source_end = checked_add(self.prefixes.sources, self.counts.sources)?;
                if source >= visible_source_end {
                    return Err(invariant(
                        "validated source provenance node is outside source space",
                    ));
                }
                NodeSceneProfileProvenanceKind::Source { source }
            }
            value if value == u32::from(PROFILE_PROVENANCE_OPERATION) => {
                let operation_index =
                    u32_to_usize(values[profile_provenance_scalar::VARIANT_INDEX])?;
                if operation_index >= self.counts.operations || depth == 0 {
                    return Err(invariant(
                        "validated operation provenance header is inconsistent",
                    ));
                }
                let operation = self.read_operation(operation_index)?;
                NodeSceneProfileProvenanceKind::Operation(operation)
            }
            _ => {
                return Err(invariant(
                    "validated profile provenance-node kind is unknown",
                ));
            }
        };
        self.next_provenance = advance(row, "profile provenance cursor index overflows")?;
        Ok(Some(NodeSceneProfileProvenance {
            index,
            depth,
            parent_offset: checked_add(self.prefixes.parents, parent_offset)?,
            parents: NodeSceneProfileProvenanceIndices::new(parents),
            kind,
        }))
    }

    fn next_evidence(
        &mut self,
    ) -> NodeSceneSemanticResult<Option<NodeSceneProfileEvidence<'scene>>> {
        let Some(row) = next_row(self.next_evidence, self.counts.evidence)? else {
            return Ok(None);
        };
        let values = scalar_row(
            self.evidence,
            row,
            PROFILE_EVIDENCE_SCALAR_COUNT,
            "validated profile evidence row is unreadable",
        )?;
        let code = read_utf8_at(
            self.text,
            u32_to_usize(values[profile_evidence_scalar::CODE_OFFSET])?,
            u32_to_usize(values[profile_evidence_scalar::CODE_COUNT])?,
            true,
            "validated profile evidence code range is inconsistent",
        )?;
        let occurrence = read_slice_at(
            self.wide,
            u32_to_usize(values[profile_evidence_scalar::OCCURRENCE_WIDE_OFFSET])?,
            u32_to_usize(values[profile_evidence_scalar::OCCURRENCE_WIDE_COUNT])?,
            1,
            "validated profile evidence occurrence range is inconsistent",
        )?[0];
        if occurrence == 0 {
            return Err(invariant(
                "validated profile evidence occurrence count is zero",
            ));
        }
        let digest = read_optional_digest_at(
            self.digests,
            values[profile_evidence_scalar::DIGEST_PRESENCE],
            values[profile_evidence_scalar::DIGEST_ALGORITHM],
            values[profile_evidence_scalar::DIGEST_OFFSET],
            values[profile_evidence_scalar::DIGEST_COUNT],
            "validated profile evidence digest is inconsistent",
        )?;
        self.next_evidence = advance(row, "profile evidence cursor index overflows")?;
        Ok(Some(NodeSceneProfileEvidence {
            index: checked_add(self.prefixes.evidence, row)?,
            code,
            occurrence_count: occurrence,
            digest,
        }))
    }

    fn read_operation(
        &self,
        row: usize,
    ) -> NodeSceneSemanticResult<NodeSceneProfileOperation<'scene>> {
        let values = scalar_row(
            self.operations,
            row,
            PROFILE_OPERATION_SCALAR_COUNT,
            "validated profile operation row is unreadable",
        )?;
        let name_offset = values[profile_operation_scalar::NAME_OFFSET];
        let name_count = values[profile_operation_scalar::NAME_COUNT];
        let kind =
            if values[profile_operation_scalar::KIND] == u32::from(PROFILE_OPERATION_NAMESPACED) {
                NodeSceneProfileOperationKind::Namespaced(read_utf8_at(
                    self.text,
                    u32_to_usize(name_offset)?,
                    u32_to_usize(name_count)?,
                    true,
                    "validated namespaced profile operation name is inconsistent",
                )?)
            } else {
                require_empty_range_at(
                    self.text.len(),
                    name_offset,
                    name_count,
                    "validated standard profile operation name is not empty",
                )?;
                decode_standard_operation_kind(values[profile_operation_scalar::KIND])?
            };
        let options_digest = read_required_digest_at(
            self.digests,
            values[profile_operation_scalar::OPTIONS_DIGEST_ALGORITHM],
            values[profile_operation_scalar::OPTIONS_DIGEST_OFFSET],
            values[profile_operation_scalar::OPTIONS_DIGEST_COUNT],
            "validated profile operation options digest is inconsistent",
        )?;
        let evidence_offset = u32_to_usize(values[profile_operation_scalar::EVIDENCE_OFFSET])?;
        let evidence_count = u32_to_usize(values[profile_operation_scalar::EVIDENCE_COUNT])?;
        if checked_add(evidence_offset, evidence_count)? > self.counts.evidence {
            return Err(invariant(
                "validated profile operation evidence range is inconsistent",
            ));
        }
        Ok(NodeSceneProfileOperation {
            index: checked_add(self.prefixes.operations, row)?,
            kind,
            options_digest,
            evidence_offset: checked_add(self.prefixes.evidence, evidence_offset)?,
            evidence_count,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneProfileResourceStability<'scene> {
    Stable {
        owner: &'scene str,
        schema: &'scene str,
        schema_version: u32,
    },
    SessionOpaque(SessionOpaqueReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileResource<'scene> {
    index: usize,
    identity: &'scene str,
    stability: NodeSceneProfileResourceStability<'scene>,
}

impl<'scene> NodeSceneProfileResource<'scene> {
    pub(in crate::canonical_scene::store) const fn index(self) -> usize {
        self.index
    }

    pub(in crate::canonical_scene::store) const fn identity(self) -> &'scene str {
        self.identity
    }

    pub(in crate::canonical_scene::store) const fn stability(
        self,
    ) -> NodeSceneProfileResourceStability<'scene> {
        self.stability
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneProfileSourceLocator<'scene> {
    Indexed {
        kind: SourceObjectKind,
        index: u64,
    },
    Namespaced {
        schema: &'scene str,
        schema_version: u32,
        value: &'scene str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileDigest<'scene> {
    algorithm: DigestAlgorithm,
    bytes: &'scene [u8],
}

impl<'scene> NodeSceneProfileDigest<'scene> {
    pub(in crate::canonical_scene::store) const fn algorithm(self) -> DigestAlgorithm {
        self.algorithm
    }

    pub(in crate::canonical_scene::store) const fn bytes(self) -> &'scene [u8] {
        self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileSource<'scene> {
    index: usize,
    resource: usize,
    format: &'scene str,
    locator: NodeSceneProfileSourceLocator<'scene>,
    display_name: Option<&'scene str>,
    content_digest: Option<NodeSceneProfileDigest<'scene>>,
}

impl<'scene> NodeSceneProfileSource<'scene> {
    pub(in crate::canonical_scene::store) const fn index(self) -> usize {
        self.index
    }

    pub(in crate::canonical_scene::store) const fn resource(self) -> usize {
        self.resource
    }

    pub(in crate::canonical_scene::store) const fn format(self) -> &'scene str {
        self.format
    }

    pub(in crate::canonical_scene::store) const fn locator(
        self,
    ) -> NodeSceneProfileSourceLocator<'scene> {
        self.locator
    }

    pub(in crate::canonical_scene::store) const fn display_name(self) -> Option<&'scene str> {
        self.display_name
    }

    pub(in crate::canonical_scene::store) const fn content_digest(
        self,
    ) -> Option<NodeSceneProfileDigest<'scene>> {
        self.content_digest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneProfileOperationKind<'scene> {
    Generated,
    Default,
    Normalize,
    Repair,
    Split,
    Merge,
    Convert,
    Transform,
    Optimize,
    Namespaced(&'scene str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileOperation<'scene> {
    index: usize,
    kind: NodeSceneProfileOperationKind<'scene>,
    options_digest: NodeSceneProfileDigest<'scene>,
    evidence_offset: usize,
    evidence_count: usize,
}

impl<'scene> NodeSceneProfileOperation<'scene> {
    pub(in crate::canonical_scene::store) const fn index(self) -> usize {
        self.index
    }

    pub(in crate::canonical_scene::store) const fn kind(
        self,
    ) -> NodeSceneProfileOperationKind<'scene> {
        self.kind
    }

    pub(in crate::canonical_scene::store) const fn options_digest(
        self,
    ) -> NodeSceneProfileDigest<'scene> {
        self.options_digest
    }

    pub(in crate::canonical_scene::store) const fn evidence_offset(self) -> usize {
        self.evidence_offset
    }

    pub(in crate::canonical_scene::store) const fn evidence_count(self) -> usize {
        self.evidence_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) enum NodeSceneProfileProvenanceKind<'scene> {
    Source { source: usize },
    Operation(NodeSceneProfileOperation<'scene>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileProvenance<'scene> {
    index: usize,
    depth: usize,
    parent_offset: usize,
    parents: NodeSceneProfileProvenanceIndices<'scene>,
    kind: NodeSceneProfileProvenanceKind<'scene>,
}

impl<'scene> NodeSceneProfileProvenance<'scene> {
    pub(in crate::canonical_scene::store) const fn index(self) -> usize {
        self.index
    }

    pub(in crate::canonical_scene::store) const fn depth(self) -> usize {
        self.depth
    }

    pub(in crate::canonical_scene::store) const fn parent_offset(self) -> usize {
        self.parent_offset
    }

    pub(in crate::canonical_scene::store) const fn parents(
        self,
    ) -> NodeSceneProfileProvenanceIndices<'scene> {
        self.parents
    }

    pub(in crate::canonical_scene::store) const fn kind(
        self,
    ) -> NodeSceneProfileProvenanceKind<'scene> {
        self.kind
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::canonical_scene::store) struct NodeSceneProfileEvidence<'scene> {
    index: usize,
    code: &'scene str,
    occurrence_count: u64,
    digest: Option<NodeSceneProfileDigest<'scene>>,
}

impl<'scene> NodeSceneProfileEvidence<'scene> {
    pub(in crate::canonical_scene::store) const fn index(self) -> usize {
        self.index
    }

    pub(in crate::canonical_scene::store) const fn code(self) -> &'scene str {
        self.code
    }

    pub(in crate::canonical_scene::store) const fn occurrence_count(self) -> u64 {
        self.occurrence_count
    }

    pub(in crate::canonical_scene::store) const fn digest(
        self,
    ) -> Option<NodeSceneProfileDigest<'scene>> {
        self.digest
    }
}

fn read_segment_counts(
    segments: CanonicalRecordView<'_>,
    row: usize,
) -> NodeSceneSemanticResult<ProfileLineageCounts> {
    if read_u32_as_usize(
        segments,
        row,
        profile_segment_field::SEGMENT,
        "validated profile segment identity is unreadable",
    )? != row
    {
        return Err(invariant(
            "validated profile segment identity disagrees with its row",
        ));
    }
    let counts = ProfileLineageCounts {
        resources: read_u32_as_usize(
            segments,
            row,
            profile_segment_field::RESOURCE_COUNT,
            "validated profile resource count is unreadable",
        )?,
        sources: read_u32_as_usize(
            segments,
            row,
            profile_segment_field::SOURCE_COUNT,
            "validated profile source count is unreadable",
        )?,
        provenance: read_u32_as_usize(
            segments,
            row,
            profile_segment_field::PROVENANCE_COUNT,
            "validated profile provenance count is unreadable",
        )?,
        operations: read_u32_as_usize(
            segments,
            row,
            profile_segment_field::OPERATION_COUNT,
            "validated profile operation count is unreadable",
        )?,
        evidence: read_u32_as_usize(
            segments,
            row,
            profile_segment_field::EVIDENCE_COUNT,
            "validated profile evidence count is unreadable",
        )?,
        parents: read_u32_as_usize(
            segments,
            row,
            profile_segment_field::PARENT_COUNT,
            "validated profile parent count is unreadable",
        )?,
        wide: read_u32_as_usize(
            segments,
            row,
            profile_segment_field::WIDE_COUNT,
            "validated profile wide-value count is unreadable",
        )?,
        text: read_u32_as_usize(
            segments,
            row,
            profile_segment_field::TEXT_COUNT,
            "validated profile text-byte count is unreadable",
        )?,
        digests: read_u32_as_usize(
            segments,
            row,
            profile_segment_field::DIGEST_COUNT,
            "validated profile digest-byte count is unreadable",
        )?,
    };
    if counts == ProfileLineageCounts::default() {
        return Err(invariant(
            "validated profile contains an empty lineage segment",
        ));
    }
    Ok(counts)
}

fn resolve_segment<'scene>(
    canonical: &'scene SceneCanonicalStore,
    segments: CanonicalRecordView<'scene>,
    row: usize,
    prefixes: ProfileLineageCounts,
) -> NodeSceneSemanticResult<ProfileSegmentDecoder<'scene>> {
    let counts = read_segment_counts(segments, row)?;
    let completed_progress = ProfileSegmentProgress {
        next_segment: advance(row, "profile segment cursor index overflows")?,
        lineage_prefixes: prefixes.checked_add(counts)?,
    };
    let (_, resources) = resolve_optional_sequence::<u32, ProfileLineageResourceSchema>(
        canonical,
        segments,
        row,
        profile_segment_field::RESOURCE_COUNT,
        profile_segment_field::RESOURCES,
        PROFILE_RESOURCE_SCALAR_COUNT,
        "validated profile resource leaf is inconsistent",
    )?;
    let (_, sources) = resolve_optional_sequence::<u32, ProfileLineageSourceSchema>(
        canonical,
        segments,
        row,
        profile_segment_field::SOURCE_COUNT,
        profile_segment_field::SOURCES,
        PROFILE_SOURCE_SCALAR_COUNT,
        "validated profile source leaf is inconsistent",
    )?;
    let (_, provenance) = resolve_optional_sequence::<u32, ProfileLineageProvenanceSchema>(
        canonical,
        segments,
        row,
        profile_segment_field::PROVENANCE_COUNT,
        profile_segment_field::PROVENANCE_NODES,
        PROFILE_PROVENANCE_SCALAR_COUNT,
        "validated profile provenance leaf is inconsistent",
    )?;
    let (_, operations) = resolve_optional_sequence::<u32, ProfileLineageOperationSchema>(
        canonical,
        segments,
        row,
        profile_segment_field::OPERATION_COUNT,
        profile_segment_field::OPERATIONS,
        PROFILE_OPERATION_SCALAR_COUNT,
        "validated profile operation leaf is inconsistent",
    )?;
    let (_, evidence) = resolve_optional_sequence::<u32, ProfileLineageEvidenceSchema>(
        canonical,
        segments,
        row,
        profile_segment_field::EVIDENCE_COUNT,
        profile_segment_field::EVIDENCE,
        PROFILE_EVIDENCE_SCALAR_COUNT,
        "validated profile evidence leaf is inconsistent",
    )?;
    let (_, parent_scalars) = resolve_optional_sequence::<u64, ProfileLineageParentSchema>(
        canonical,
        segments,
        row,
        profile_segment_field::PARENT_COUNT,
        profile_segment_field::PARENTS,
        PROFILE_PROVENANCE_ADDRESS_SCALARS,
        "validated profile parent leaf is inconsistent",
    )?;
    let (_, wide) = resolve_optional_sequence::<u64, ProfileLineageWideSchema>(
        canonical,
        segments,
        row,
        profile_segment_field::WIDE_COUNT,
        profile_segment_field::WIDE_VALUES,
        1,
        "validated profile wide-value leaf is inconsistent",
    )?;
    let (_, text) = resolve_optional_utf8_at::<ProfileLineageTextSchema>(
        canonical,
        segments,
        row,
        profile_segment_field::TEXT_COUNT,
        profile_segment_field::TEXTS,
        "validated profile text leaf is inconsistent",
    )?;
    let (_, digests) = resolve_optional_bytes_at::<ProfileLineageDigestSchema>(
        canonical,
        segments,
        row,
        profile_segment_field::DIGEST_COUNT,
        profile_segment_field::DIGESTS,
        "validated profile digest leaf is inconsistent",
    )?;

    if resources.map_or(0, <[u32]>::len)
        != counts
            .resources
            .checked_mul(PROFILE_RESOURCE_SCALAR_COUNT)
            .ok_or_else(|| invariant("validated profile resource leaf length overflows"))?
        || sources.map_or(0, <[u32]>::len)
            != counts
                .sources
                .checked_mul(PROFILE_SOURCE_SCALAR_COUNT)
                .ok_or_else(|| invariant("validated profile source leaf length overflows"))?
        || provenance.map_or(0, <[u32]>::len)
            != counts
                .provenance
                .checked_mul(PROFILE_PROVENANCE_SCALAR_COUNT)
                .ok_or_else(|| invariant("validated profile provenance leaf length overflows"))?
        || operations.map_or(0, <[u32]>::len)
            != counts
                .operations
                .checked_mul(PROFILE_OPERATION_SCALAR_COUNT)
                .ok_or_else(|| invariant("validated profile operation leaf length overflows"))?
        || evidence.map_or(0, <[u32]>::len)
            != counts
                .evidence
                .checked_mul(PROFILE_EVIDENCE_SCALAR_COUNT)
                .ok_or_else(|| invariant("validated profile evidence leaf length overflows"))?
        || parent_scalars.map_or(0, <[u64]>::len)
            != counts
                .parents
                .checked_mul(PROFILE_PROVENANCE_ADDRESS_SCALARS)
                .ok_or_else(|| invariant("validated profile parent scalar count overflows"))?
        || wide.map_or(0, <[u64]>::len) != counts.wide
        || text.map_or(0, str::len) != counts.text
        || digests.map_or(0, <[u8]>::len) != counts.digests
    {
        return Err(invariant(
            "validated profile segment leaf lengths disagree with its row",
        ));
    }

    let parents = QualifiedProvenanceAddressSlice::from_scalars(
        parent_scalars.unwrap_or(&[]),
        counts.parents,
    )
    .ok_or_else(|| invariant("validated profile parent address width is inconsistent"))?;

    Ok(ProfileSegmentDecoder {
        completed_progress,
        counts,
        prefixes,
        resources: resources.unwrap_or(&[]),
        sources: sources.unwrap_or(&[]),
        provenance: provenance.unwrap_or(&[]),
        operations: operations.unwrap_or(&[]),
        evidence: evidence.unwrap_or(&[]),
        parents,
        wide: wide.unwrap_or(&[]),
        text: text.unwrap_or(""),
        digests: digests.unwrap_or(&[]),
        next_resource: 0,
        next_source: 0,
        next_provenance: 0,
        next_evidence: 0,
    })
}

fn resolve_optional_table<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    directory: CanonicalRecordView<'scene>,
    field: usize,
    rows: usize,
    fields: usize,
    proof_reference: Option<CanonicalBlockRef<Tag>>,
    reason: &'static str,
) -> NodeSceneSemanticResult<Option<CanonicalRecordView<'scene>>>
where
    Tag: CanonicalSchema<StoreKind = RecordBlock>,
{
    match (rows, proof_reference) {
        (0, None) => {
            require_absent(directory, 0, field, reason)?;
            Ok(None)
        }
        (0, Some(_)) | (_, None) => Err(invariant(reason)),
        (_, Some(reference)) => {
            require_record_reference(directory, 0, field, reference, reason, reason)?;
            let view = canonical
                .records(reference.into_store())
                .ok_or_else(|| invariant(reason))?;
            require_table_layout(view, rows, fields, reason)?;
            Ok(Some(view))
        }
    }
}

fn resolve_optional_utf8<'scene>(
    canonical: &'scene SceneCanonicalStore,
    directory: CanonicalRecordView<'scene>,
    count_field: usize,
    reference_field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<OptionalCanonicalValue<'scene, ProfileMetadataTextSchema, str>> {
    resolve_optional_utf8_at::<ProfileMetadataTextSchema>(
        canonical,
        directory,
        0,
        count_field,
        reference_field,
        reason,
    )
}

fn resolve_optional_bytes<'scene>(
    canonical: &'scene SceneCanonicalStore,
    directory: CanonicalRecordView<'scene>,
    count_field: usize,
    reference_field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<OptionalCanonicalValue<'scene, ProfileMetadataBytesSchema, [u8]>> {
    resolve_optional_bytes_at::<ProfileMetadataBytesSchema>(
        canonical,
        directory,
        0,
        count_field,
        reference_field,
        reason,
    )
}

fn resolve_optional_utf8_at<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    records: CanonicalRecordView<'scene>,
    row: usize,
    count_field: usize,
    reference_field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<OptionalCanonicalValue<'scene, Tag, str>>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    let count = read_u32_as_usize(records, row, count_field, reason)?;
    if count == 0 {
        require_absent(records, row, reference_field, reason)?;
        return Ok((None, None));
    }
    let reference = records
        .utf8_ref::<Tag>(row, reference_field)
        .map(CanonicalBlockRef::from_store)
        .ok_or_else(|| invariant(reason))?;
    let value = canonical
        .utf8(reference.into_store())
        .filter(|value| value.len() == count)
        .ok_or_else(|| invariant(reason))?;
    Ok((Some(reference), Some(value)))
}

fn resolve_optional_bytes_at<'scene, Tag>(
    canonical: &'scene SceneCanonicalStore,
    records: CanonicalRecordView<'scene>,
    row: usize,
    count_field: usize,
    reference_field: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<OptionalCanonicalValue<'scene, Tag, [u8]>>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::ByteBlock>,
{
    let count = read_u32_as_usize(records, row, count_field, reason)?;
    if count == 0 {
        require_absent(records, row, reference_field, reason)?;
        return Ok((None, None));
    }
    let reference = records
        .bytes_ref::<Tag>(row, reference_field)
        .map(CanonicalBlockRef::from_store)
        .ok_or_else(|| invariant(reason))?;
    let value = canonical
        .bytes(reference.into_store())
        .filter(|value| value.len() == count)
        .ok_or_else(|| invariant(reason))?;
    Ok((Some(reference), Some(value)))
}

#[allow(clippy::too_many_arguments)]
fn resolve_optional_sequence<'scene, T, Tag>(
    canonical: &'scene SceneCanonicalStore,
    records: CanonicalRecordView<'scene>,
    row: usize,
    count_field: usize,
    reference_field: usize,
    stride: usize,
    reason: &'static str,
) -> NodeSceneSemanticResult<OptionalCanonicalValue<'scene, Tag, [T]>>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
{
    let count = read_u32_as_usize(records, row, count_field, reason)?;
    if count == 0 {
        require_absent(records, row, reference_field, reason)?;
        return Ok((None, None));
    }
    let expected = count.checked_mul(stride).ok_or_else(|| invariant(reason))?;
    let reference = records
        .sequence_ref::<T, Tag>(row, reference_field)
        .map(CanonicalBlockRef::from_store)
        .ok_or_else(|| invariant(reason))?;
    let values: SequenceView<'scene, T> = canonical
        .sequence(reference.into_store())
        .ok_or_else(|| invariant(reason))?;
    if values.len() != expected {
        return Err(invariant(reason));
    }
    Ok((Some(reference), Some(values.as_slice())))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MetadataLane {
    None,
    Bool,
    I64,
    U64,
    F64,
    F32,
}

#[derive(Clone, Copy)]
struct MetadataShape {
    lane: MetadataLane,
    scalar_count: usize,
    auxiliary_text: bool,
    bytes: bool,
}

fn decode_owner(kind: u8, index: u32) -> NodeSceneSemanticResult<CanonicalProfileOwner> {
    match kind {
        PROFILE_OWNER_SCENE if index == 0 => Ok(CanonicalProfileOwner::Scene),
        PROFILE_OWNER_SYNTHETIC_ROOT if index == 0 => Ok(CanonicalProfileOwner::SyntheticRoot),
        PROFILE_OWNER_NODE if index != 0 => Ok(CanonicalProfileOwner::Node(index)),
        PROFILE_OWNER_CAMERA => Ok(CanonicalProfileOwner::Camera(index)),
        PROFILE_OWNER_LIGHT => Ok(CanonicalProfileOwner::Light(index)),
        PROFILE_OWNER_IMAGE => Ok(CanonicalProfileOwner::Image(index)),
        PROFILE_OWNER_SAMPLER => Ok(CanonicalProfileOwner::Sampler(index)),
        PROFILE_OWNER_TEXTURE => Ok(CanonicalProfileOwner::Texture(index)),
        PROFILE_OWNER_TEXTURE_BINDING => Ok(CanonicalProfileOwner::TextureBinding(index)),
        PROFILE_OWNER_MATERIAL => Ok(CanonicalProfileOwner::Material(index)),
        PROFILE_OWNER_LOGICAL_MESH => Ok(CanonicalProfileOwner::LogicalMesh(index)),
        PROFILE_OWNER_PRIMITIVE => Ok(CanonicalProfileOwner::Primitive(index)),
        PROFILE_OWNER_ATTRIBUTE => Ok(CanonicalProfileOwner::Attribute(index)),
        PROFILE_OWNER_MESH_INSTANCE => Ok(CanonicalProfileOwner::MeshInstance(index)),
        PROFILE_OWNER_SKIN => Ok(CanonicalProfileOwner::Skin(index)),
        PROFILE_OWNER_MORPH_DEFINITION => Ok(CanonicalProfileOwner::MorphDefinition(index)),
        PROFILE_OWNER_MORPH_DELTA => Ok(CanonicalProfileOwner::MorphDelta(index)),
        PROFILE_OWNER_ANIMATION_CLIP => Ok(CanonicalProfileOwner::AnimationClip(index)),
        PROFILE_OWNER_ANIMATION_CHANNEL => Ok(CanonicalProfileOwner::AnimationChannel(index)),
        _ => Err(invariant("validated profile owner kind is unknown")),
    }
}

fn require_owner_order(
    row: usize,
    owner: CanonicalProfileOwner,
    previous: Option<CanonicalProfileOwner>,
) -> NodeSceneSemanticResult<()> {
    if row == 0 && owner != CanonicalProfileOwner::Scene
        || row == 1 && owner != CanonicalProfileOwner::SyntheticRoot
        || row > 1 && previous.is_none_or(|previous| owner <= previous)
    {
        return Err(invariant(
            "validated profile owners are not canonically ordered",
        ));
    }
    Ok(())
}

fn decode_session_opaque_reason(value: u32) -> NodeSceneSemanticResult<SessionOpaqueReason> {
    match u8::try_from(value)
        .map_err(|_| invariant("validated session-opaque reason does not fit u8"))?
    {
        PROFILE_SESSION_OPAQUE_CALLER_LABEL => Ok(SessionOpaqueReason::CallerLabel),
        PROFILE_SESSION_OPAQUE_GENERATED => Ok(SessionOpaqueReason::Generated),
        PROFILE_SESSION_OPAQUE_UNREGISTERED_RESOLVER => {
            Ok(SessionOpaqueReason::UnregisteredResolver)
        }
        PROFILE_SESSION_OPAQUE_SESSION_FALLBACK => Ok(SessionOpaqueReason::SessionFallback),
        _ => Err(invariant(
            "validated profile session-opaque reason is unknown",
        )),
    }
}

fn decode_source_object_kind(value: u32) -> NodeSceneSemanticResult<SourceObjectKind> {
    match u8::try_from(value)
        .map_err(|_| invariant("validated source-object kind does not fit u8"))?
    {
        PROFILE_SOURCE_OBJECT_DOCUMENT => Ok(SourceObjectKind::Document),
        PROFILE_SOURCE_OBJECT_SCENE => Ok(SourceObjectKind::Scene),
        PROFILE_SOURCE_OBJECT_NODE => Ok(SourceObjectKind::Node),
        PROFILE_SOURCE_OBJECT_MESH => Ok(SourceObjectKind::Mesh),
        PROFILE_SOURCE_OBJECT_PRIMITIVE => Ok(SourceObjectKind::Primitive),
        PROFILE_SOURCE_OBJECT_MATERIAL => Ok(SourceObjectKind::Material),
        PROFILE_SOURCE_OBJECT_IMAGE => Ok(SourceObjectKind::Image),
        PROFILE_SOURCE_OBJECT_TEXTURE => Ok(SourceObjectKind::Texture),
        PROFILE_SOURCE_OBJECT_SAMPLER => Ok(SourceObjectKind::Sampler),
        PROFILE_SOURCE_OBJECT_SKIN => Ok(SourceObjectKind::Skin),
        PROFILE_SOURCE_OBJECT_ANIMATION => Ok(SourceObjectKind::Animation),
        PROFILE_SOURCE_OBJECT_ANIMATION_CHANNEL => Ok(SourceObjectKind::AnimationChannel),
        PROFILE_SOURCE_OBJECT_CAMERA => Ok(SourceObjectKind::Camera),
        PROFILE_SOURCE_OBJECT_LIGHT => Ok(SourceObjectKind::Light),
        PROFILE_SOURCE_OBJECT_VERTEX => Ok(SourceObjectKind::Vertex),
        PROFILE_SOURCE_OBJECT_FACE => Ok(SourceObjectKind::Face),
        PROFILE_SOURCE_OBJECT_ELEMENT => Ok(SourceObjectKind::Element),
        PROFILE_SOURCE_OBJECT_ROW => Ok(SourceObjectKind::Row),
        PROFILE_SOURCE_OBJECT_STATEMENT => Ok(SourceObjectKind::Statement),
        PROFILE_SOURCE_OBJECT_SEGMENT => Ok(SourceObjectKind::Segment),
        _ => Err(invariant("validated profile source-object kind is unknown")),
    }
}

fn decode_standard_operation_kind(
    value: u32,
) -> NodeSceneSemanticResult<NodeSceneProfileOperationKind<'static>> {
    match u8::try_from(value).map_err(|_| invariant("validated operation kind does not fit u8"))? {
        PROFILE_OPERATION_GENERATED => Ok(NodeSceneProfileOperationKind::Generated),
        PROFILE_OPERATION_DEFAULT => Ok(NodeSceneProfileOperationKind::Default),
        PROFILE_OPERATION_NORMALIZE => Ok(NodeSceneProfileOperationKind::Normalize),
        PROFILE_OPERATION_REPAIR => Ok(NodeSceneProfileOperationKind::Repair),
        PROFILE_OPERATION_SPLIT => Ok(NodeSceneProfileOperationKind::Split),
        PROFILE_OPERATION_MERGE => Ok(NodeSceneProfileOperationKind::Merge),
        PROFILE_OPERATION_CONVERT => Ok(NodeSceneProfileOperationKind::Convert),
        PROFILE_OPERATION_TRANSFORM => Ok(NodeSceneProfileOperationKind::Transform),
        PROFILE_OPERATION_OPTIMIZE => Ok(NodeSceneProfileOperationKind::Optimize),
        _ => Err(invariant(
            "validated profile standard operation kind is unknown",
        )),
    }
}

fn decode_digest_algorithm(value: u32) -> NodeSceneSemanticResult<DigestAlgorithm> {
    match u8::try_from(value)
        .map_err(|_| invariant("validated digest algorithm does not fit u8"))?
    {
        PROFILE_DIGEST_SHA256 => Ok(DigestAlgorithm::Sha256),
        PROFILE_DIGEST_BLAKE3 => Ok(DigestAlgorithm::Blake3),
        _ => Err(invariant("validated profile digest algorithm is unknown")),
    }
}

#[cfg(test)]
mod provenance_index_tests {
    use super::*;

    fn indices(scalars: &[u64]) -> NodeSceneProfileProvenanceIndices<'_> {
        NodeSceneProfileProvenanceIndices::new(
            QualifiedProvenanceAddressSlice::from_scalars(scalars, 1)
                .expect("test qualified provenance address"),
        )
    }

    #[test]
    fn semantic_indices_ignore_physical_address_qualification() {
        let left_scalars = [0, 7, 3, 3];
        let right_scalars = [1, 11, 1, 3];
        let different_global_scalars = [1, 11, 2, 4];

        let left = indices(&left_scalars);
        let right = indices(&right_scalars);
        let different_global = indices(&different_global_scalars);

        assert_eq!(left, right);
        assert_ne!(left, different_global);
        assert_eq!(format!("{left:?}"), "[Some(3)]");
    }
}
