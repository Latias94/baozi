//! Fresh-generation retained materialization for source identity, provenance, and metadata.
//!
//! The profile has a deliberately asymmetric shape. Immutable scalar, text, and byte leaves
//! are owned by a lineage segment; the profile, segment, owner, and metadata directories are
//! generation-local record blocks. A future edit can therefore retain prior segment leaves while
//! rebuilding only its relationship records.

use baozi_operation::__canonical_activation::{CanonicalLane, CanonicalRecordBuildRow};
use baozi_operation::{
    ByteBlock, BytesPlan, OperationOwner, ProgressPhase, ProgressUnit, RecordPlan, ScalarSequence,
    SequencePlan, StoreBuildError, StoreControlledError, StorePlanError, StoreRuntimeError,
    StoredScalar, Utf8Block, Utf8Plan,
};

use crate::canonical_scene::store::production::{CanonicalProfileOwner, ValidatedProfileBatch};
use crate::canonical_scene::store::profile_provenance::{
    PROFILE_PROVENANCE_ADDRESS_SCALARS, ProfileSegmentNonce, issue_profile_segment_nonce,
};
use crate::canonical_scene::store::schema::profile::{
    FRESH_PROFILE_MAX_CANONICAL_BLOCKS, PROFILE_ABSENT_REFERENCE, PROFILE_DIRECTORY_FIELD_COUNT,
    PROFILE_EVIDENCE_SCALAR_COUNT, PROFILE_METADATA_DIRECTORY_FIELD_COUNT,
    PROFILE_METADATA_ENTRY_FIELD_COUNT, PROFILE_OPERATION_SCALAR_COUNT, PROFILE_OWNER_FIELD_COUNT,
    PROFILE_PROVENANCE_SCALAR_COUNT, PROFILE_RESOURCE_SCALAR_COUNT, PROFILE_SCHEMA_REVISION,
    PROFILE_SEGMENT_FIELD_COUNT, PROFILE_SOURCE_SCALAR_COUNT, ProfileDirectorySchema,
    ProfileLineageAttachmentSchema, ProfileLineageDigestSchema, ProfileLineageEvidenceSchema,
    ProfileLineageOperationSchema, ProfileLineageParentSchema, ProfileLineageProvenanceSchema,
    ProfileLineageResourceSchema, ProfileLineageSourceSchema, ProfileLineageTextSchema,
    ProfileLineageWideSchema, ProfileMetadataBoolSchema, ProfileMetadataBytesSchema,
    ProfileMetadataEntryTableSchema, ProfileMetadataF32Schema, ProfileMetadataF64Schema,
    ProfileMetadataI64Schema, ProfileMetadataTextSchema, ProfileMetadataU64Schema,
    ProfileOwnerTableSchema, ProfileSegmentTableSchema,
};
use crate::canonical_scene::store::schema::{CanonicalBlockRef, CanonicalSchema};
use crate::canonical_scene::store::validation::ProfileMeteredCounts;

use super::{
    checked_validation_work_sum, optional_bytes_validation_work, optional_record_validation_work,
    optional_sequence_validation_work, optional_utf8_validation_work, record_validation_work,
};

mod lineage;
mod metadata;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProfileBuildFault {
    None,
    SegmentOrdinal,
    SegmentNonce,
    SegmentProvenanceBase,
    SegmentResourceReference,
    OwnerKind,
    OwnerAttachmentOffset,
    MetadataEntriesReference,
    MetadataScalarCount,
    MetadataBoolLaneReference,
}

impl ProfileBuildFault {
    pub(super) const fn none() -> Self {
        Self::None
    }

    #[cfg(test)]
    pub(super) const ALL: [Self; 9] = [
        Self::SegmentOrdinal,
        Self::SegmentNonce,
        Self::SegmentProvenanceBase,
        Self::SegmentResourceReference,
        Self::OwnerKind,
        Self::OwnerAttachmentOffset,
        Self::MetadataEntriesReference,
        Self::MetadataScalarCount,
        Self::MetadataBoolLaneReference,
    ];

    fn take(&mut self, selected: Self) -> bool {
        if *self == selected {
            *self = Self::None;
            true
        } else {
            false
        }
    }

    fn replace_once<T: Copy>(&mut self, selected: Self, value: T, replacement: T) -> T {
        if self.take(selected) {
            replacement
        } else {
            value
        }
    }

    fn segment_ordinal(&mut self, value: u32) -> u32 {
        self.replace_once(Self::SegmentOrdinal, value, 1)
    }

    fn segment_nonce(&mut self, value: u64) -> u64 {
        self.replace_once(Self::SegmentNonce, value, 0)
    }

    fn segment_provenance_base(&mut self, value: u32) -> u32 {
        self.replace_once(Self::SegmentProvenanceBase, value, 1)
    }

    fn omit_segment_resource_reference(&mut self) -> bool {
        self.take(Self::SegmentResourceReference)
    }

    fn owner_kind(&mut self, value: u8) -> u8 {
        self.replace_once(Self::OwnerKind, value, u8::MAX)
    }

    fn owner_attachment_offset(&mut self, value: u32) -> u32 {
        self.replace_once(Self::OwnerAttachmentOffset, value, u32::MAX)
    }

    fn omit_metadata_entries_reference(&mut self) -> bool {
        self.take(Self::MetadataEntriesReference)
    }

    fn metadata_scalar_count(&mut self, value: u32) -> u32 {
        self.replace_once(Self::MetadataScalarCount, value, u32::MAX)
    }

    fn omit_metadata_bool_lane_reference(&mut self) -> bool {
        self.take(Self::MetadataBoolLaneReference)
    }
}

/// Preflighted retained-block shape for one fresh profile segment.
///
/// This intentionally has no public generic mutation API. The caller builds every immutable
/// leaf under the admitted `try_retain` closure, then emits the generation-local records that
/// reference those leaves.
pub(super) struct ProfileMaterializationPlans {
    directory: RecordPlan,
    segments: RecordPlan,
    owners: RecordPlan,
    metadata_directory: RecordPlan,
    resources: Option<SequencePlan<u32>>,
    sources: Option<SequencePlan<u32>>,
    provenance: Option<SequencePlan<u32>>,
    operations: Option<SequencePlan<u32>>,
    evidence: Option<SequencePlan<u32>>,
    parents: Option<SequencePlan<u64>>,
    wide: Option<SequencePlan<u64>>,
    lineage_text: Option<Utf8Plan>,
    lineage_digests: Option<BytesPlan>,
    attachments: Option<SequencePlan<u64>>,
    metadata_entries: Option<RecordPlan>,
    metadata_text: Option<Utf8Plan>,
    metadata_bytes: Option<BytesPlan>,
    metadata_bool: Option<SequencePlan<u8>>,
    metadata_i64: Option<SequencePlan<i64>>,
    metadata_u64: Option<SequencePlan<u64>>,
    metadata_f64: Option<SequencePlan<f64>>,
    metadata_f32: Option<SequencePlan<f32>>,
    canonical_block_count: usize,
    payload_block_count: u32,
    shareable_leaf_block_count: u32,
    evidence_counts: ProfileEvidenceCounts,
}

#[derive(Clone, Copy)]
pub(super) struct ProfileEvidenceCounts {
    pub(super) segments: u32,
    pub(super) resources: u32,
    pub(super) sources: u32,
    pub(super) provenance: u32,
    pub(super) operations: u32,
    pub(super) parents: u32,
    pub(super) owners: u32,
    pub(super) attachments: u32,
    pub(super) metadata_entries: u32,
}

impl ProfileMaterializationPlans {
    pub(super) fn new<R>(batch: &ValidatedProfileBatch<'_, R>) -> Result<Self, StorePlanError> {
        let shape = batch.retention_shape();
        let resource_scalars = rows(batch.resource_count(), PROFILE_RESOURCE_SCALAR_COUNT)?;
        let source_scalars = rows(batch.source_count(), PROFILE_SOURCE_SCALAR_COUNT)?;
        let provenance_scalars = rows(
            batch.provenance_node_count(),
            PROFILE_PROVENANCE_SCALAR_COUNT,
        )?;
        let operation_scalars = rows(batch.operation_count(), PROFILE_OPERATION_SCALAR_COUNT)?;
        let evidence_scalars = rows(shape.evidence_count(), PROFILE_EVIDENCE_SCALAR_COUNT)?;
        let resources = sequence_plan(resource_scalars)?;
        let sources = sequence_plan(source_scalars)?;
        let provenance = sequence_plan(provenance_scalars)?;
        let operations = sequence_plan(operation_scalars)?;
        let evidence = sequence_plan(evidence_scalars)?;
        let parents = provenance_address_sequence_plan(shape.parent_count())?;
        let wide = sequence_plan_u64(shape.wide_count())?;
        let lineage_text = utf8_plan(shape.lineage_text_bytes())?;
        let lineage_digests = bytes_plan(shape.lineage_digest_bytes())?;
        let attachments = provenance_address_sequence_plan(shape.attachment_count())?;
        let metadata_entries = record_plan(
            batch.metadata_entry_count(),
            PROFILE_METADATA_ENTRY_FIELD_COUNT,
        )?;
        let metadata_text = utf8_plan(shape.metadata_text_bytes())?;
        let metadata_bytes = bytes_plan(shape.metadata_payload_bytes())?;
        let metadata_bool = sequence_plan_u8(shape.metadata_bool_count())?;
        let metadata_i64 = sequence_plan_i64(shape.metadata_i64_count())?;
        let metadata_u64 = sequence_plan_u64(shape.metadata_u64_count())?;
        let metadata_f64 = sequence_plan_f64(shape.metadata_f64_count())?;
        let metadata_f32 = sequence_plan_f32(shape.metadata_f32_count())?;

        let optional_blocks = [
            resources.is_some(),
            sources.is_some(),
            provenance.is_some(),
            operations.is_some(),
            evidence.is_some(),
            parents.is_some(),
            wide.is_some(),
            lineage_text.is_some(),
            lineage_digests.is_some(),
            attachments.is_some(),
            metadata_entries.is_some(),
            metadata_text.is_some(),
            metadata_bytes.is_some(),
            metadata_bool.is_some(),
            metadata_i64.is_some(),
            metadata_u64.is_some(),
            metadata_f64.is_some(),
            metadata_f32.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        let canonical_block_count = 4_usize
            .checked_add(optional_blocks)
            .ok_or(StorePlanError::SizeOverflow)?;
        if canonical_block_count > FRESH_PROFILE_MAX_CANONICAL_BLOCKS
            || u32::try_from(canonical_block_count).map_err(|_| StorePlanError::SizeOverflow)?
                != shape.canonical_block_count()
        {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let payload_block_count = u32::try_from(
            [
                lineage_text.is_some(),
                lineage_digests.is_some(),
                metadata_text.is_some(),
                metadata_bytes.is_some(),
                metadata_bool.is_some(),
                metadata_i64.is_some(),
                metadata_u64.is_some(),
                metadata_f64.is_some(),
                metadata_f32.is_some(),
            ]
            .into_iter()
            .filter(|present| *present)
            .count(),
        )
        .map_err(|_| StorePlanError::SizeOverflow)?;
        if payload_block_count != shape.payload_block_count() {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let shareable_leaf_block_count = u32::try_from(
            optional_blocks
                .checked_sub(usize::from(metadata_entries.is_some()))
                .ok_or(StorePlanError::InvalidBlockTable)?,
        )
        .map_err(|_| StorePlanError::SizeOverflow)?;
        if shareable_leaf_block_count != shape.shareable_leaf_block_count() {
            return Err(StorePlanError::InvalidBlockTable);
        }
        let evidence_counts = ProfileEvidenceCounts {
            segments: batch.segment_count(),
            resources: batch.resource_count(),
            sources: batch.source_count(),
            provenance: batch.provenance_node_count(),
            operations: batch.operation_count(),
            parents: shape.parent_count(),
            owners: u32::try_from(batch.owner_count()).map_err(|_| StorePlanError::SizeOverflow)?,
            attachments: shape.attachment_count(),
            metadata_entries: batch.metadata_entry_count(),
        };

        Ok(Self {
            directory: RecordPlan::new(1, PROFILE_DIRECTORY_FIELD_COUNT)?,
            segments: RecordPlan::new(
                usize::try_from(batch.segment_count()).map_err(|_| StorePlanError::SizeOverflow)?,
                PROFILE_SEGMENT_FIELD_COUNT,
            )?,
            owners: RecordPlan::new(
                usize::try_from(batch.owner_count()).map_err(|_| StorePlanError::SizeOverflow)?,
                PROFILE_OWNER_FIELD_COUNT,
            )?,
            metadata_directory: RecordPlan::new(1, PROFILE_METADATA_DIRECTORY_FIELD_COUNT)?,
            resources,
            sources,
            provenance,
            operations,
            evidence,
            parents,
            wide,
            lineage_text,
            lineage_digests,
            attachments,
            metadata_entries,
            metadata_text,
            metadata_bytes,
            metadata_bool,
            metadata_i64,
            metadata_u64,
            metadata_f64,
            metadata_f32,
            canonical_block_count,
            payload_block_count,
            shareable_leaf_block_count,
            evidence_counts,
        })
    }

    pub(super) const fn canonical_block_count(&self) -> usize {
        self.canonical_block_count
    }
    pub(super) const fn payload_block_count(&self) -> u32 {
        self.payload_block_count
    }

    pub(super) const fn shareable_leaf_block_count(&self) -> u32 {
        self.shareable_leaf_block_count
    }

    pub(super) const fn evidence_counts(&self) -> ProfileEvidenceCounts {
        self.evidence_counts
    }

    pub(super) fn metered_counts(&self) -> Result<ProfileMeteredCounts, StorePlanError> {
        Ok(ProfileMeteredCounts {
            segment_count: 1,
            resource_count: self.evidence_counts.resources,
            source_count: self.evidence_counts.sources,
            provenance_count: self.evidence_counts.provenance,
            operation_count: self.evidence_counts.operations,
            evidence_count: optional_sequence_row_count(
                self.evidence,
                PROFILE_EVIDENCE_SCALAR_COUNT,
            )?,
            parent_count: self.evidence_counts.parents,
            wide_count: optional_sequence_count(self.wide)?,
            lineage_text_bytes: optional_utf8_byte_count(self.lineage_text)?,
            lineage_digest_bytes: optional_bytes_count(self.lineage_digests)?,
            owner_count: self.evidence_counts.owners,
            attachment_count: self.evidence_counts.attachments,
            metadata_entry_count: self.evidence_counts.metadata_entries,
            metadata_text_bytes: optional_utf8_byte_count(self.metadata_text)?,
            metadata_payload_bytes: optional_bytes_count(self.metadata_bytes)?,
            metadata_bool_count: optional_sequence_count(self.metadata_bool)?,
            metadata_i64_count: optional_sequence_count(self.metadata_i64)?,
            metadata_u64_count: optional_sequence_count(self.metadata_u64)?,
            metadata_f64_count: optional_sequence_count(self.metadata_f64)?,
            metadata_f32_count: optional_sequence_count(self.metadata_f32)?,
            canonical_block_count: u32::try_from(self.canonical_block_count)
                .map_err(|_| StorePlanError::SizeOverflow)?,
            payload_leaf_count: self.payload_block_count,
            shareable_leaf_count: self.shareable_leaf_block_count,
        })
    }

    pub(super) fn validation_work_units(&self) -> Result<u64, StorePlanError> {
        checked_validation_work_sum([
            record_validation_work(self.directory)?,
            record_validation_work(self.segments)?,
            record_validation_work(self.owners)?,
            record_validation_work(self.metadata_directory)?,
            optional_sequence_validation_work(self.resources)?,
            optional_sequence_validation_work(self.sources)?,
            optional_sequence_validation_work(self.provenance)?,
            optional_sequence_validation_work(self.operations)?,
            optional_sequence_validation_work(self.evidence)?,
            optional_sequence_validation_work(self.parents)?,
            optional_sequence_validation_work(self.wide)?,
            optional_utf8_validation_work(self.lineage_text)?,
            optional_bytes_validation_work(self.lineage_digests)?,
            optional_sequence_validation_work(self.attachments)?,
            optional_record_validation_work(self.metadata_entries)?,
            optional_utf8_validation_work(self.metadata_text)?,
            optional_bytes_validation_work(self.metadata_bytes)?,
            optional_sequence_validation_work(self.metadata_bool)?,
            optional_sequence_validation_work(self.metadata_i64)?,
            optional_sequence_validation_work(self.metadata_u64)?,
            optional_sequence_validation_work(self.metadata_f64)?,
            optional_sequence_validation_work(self.metadata_f32)?,
        ])
    }
}

fn optional_sequence_count<T: StoredScalar>(
    plan: Option<SequencePlan<T>>,
) -> Result<u32, StorePlanError> {
    u32::try_from(plan.map_or(0, SequencePlan::element_count))
        .map_err(|_| StorePlanError::SizeOverflow)
}

fn optional_sequence_row_count<T: StoredScalar>(
    plan: Option<SequencePlan<T>>,
    width: usize,
) -> Result<u32, StorePlanError> {
    if width == 0 {
        return Err(StorePlanError::InvalidBlockTable);
    }
    let elements = plan.map_or(0, SequencePlan::element_count);
    if !elements.is_multiple_of(width) {
        return Err(StorePlanError::InvalidBlockTable);
    }
    u32::try_from(elements / width).map_err(|_| StorePlanError::SizeOverflow)
}

fn optional_utf8_byte_count(plan: Option<Utf8Plan>) -> Result<u32, StorePlanError> {
    u32::try_from(plan.map_or(0, Utf8Plan::bytes)).map_err(|_| StorePlanError::SizeOverflow)
}

fn optional_bytes_count(plan: Option<BytesPlan>) -> Result<u32, StorePlanError> {
    u32::try_from(plan.map_or(0, BytesPlan::bytes)).map_err(|_| StorePlanError::SizeOverflow)
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct RetainedProfileRefs {
    pub(in crate::canonical_scene::store) directory: CanonicalBlockRef<ProfileDirectorySchema>,
    pub(in crate::canonical_scene::store) segments: CanonicalBlockRef<ProfileSegmentTableSchema>,
    pub(in crate::canonical_scene::store) owners: CanonicalBlockRef<ProfileOwnerTableSchema>,
    pub(in crate::canonical_scene::store) lineage: ProfileLineageMaterializedRefs,
    pub(in crate::canonical_scene::store) metadata: metadata::RetainedMetadataRefs,
}

impl RetainedProfileRefs {
    pub(super) const fn directory(self) -> CanonicalBlockRef<ProfileDirectorySchema> {
        self.directory
    }
}

pub(super) fn retain_profile<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &ProfileMaterializationPlans,
    batch: &ValidatedProfileBatch<'_, R>,
    faults: &mut ProfileBuildFault,
    failure: StoreBuildError,
) -> Result<RetainedProfileRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    // The canonical lane proves operation admission. Issuing here keeps pure authoring
    // preflight and operation-start rejection free of invisible nonce gaps.
    let segment_nonce =
        issue_profile_segment_nonce().map_err(|_| StoreRuntimeError::MembershipExhausted)?;
    let lineage = lineage::retain_lineage(lane, plans, batch, segment_nonce, failure)?;
    let metadata = metadata::retain_metadata(lane, plans, batch, faults, failure)?;
    let segments = retain_segments(lane, plans, batch, lineage, segment_nonce, faults, failure)?;
    let owners = retain_owners(lane, plans, batch, lineage, faults, failure)?;
    let directory = lane
        .try_retain_records(plans.directory, ProfileDirectorySchema, |control| {
            control.write_row(|row| {
                row.write_scalar(PROFILE_SCHEMA_REVISION)?;
                row.write_scalar(1_u32)?;
                row.write_record_ref(segments.into_store())?;
                row.write_scalar(u32::try_from(batch.owner_count()).map_err(|_| failure)?)?;
                row.write_record_ref(owners.into_store())?;
                row.write_record_ref(metadata.directory.into_store())
            })
        })
        .map(CanonicalBlockRef::from_store)?;
    Ok(RetainedProfileRefs {
        directory,
        segments,
        owners,
        lineage: ProfileLineageMaterializedRefs::ExactSingle(lineage),
        metadata,
    })
}

fn retain_segments<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &ProfileMaterializationPlans,
    batch: &ValidatedProfileBatch<'_, R>,
    refs: ProfileLeafRefs,
    segment_nonce: ProfileSegmentNonce,
    faults: &mut ProfileBuildFault,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileSegmentTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let shape = batch.retention_shape();
    lane.try_retain_records(plans.segments, ProfileSegmentTableSchema, |control| {
        control.write_row(|row| {
            row.write_scalar(faults.segment_ordinal(0))?;
            if faults.omit_segment_resource_reference() {
                row.write_scalar(batch.resource_count())?;
                row.write_scalar(PROFILE_ABSENT_REFERENCE)?;
            } else {
                write_optional_sequence_ref(row, batch.resource_count(), refs.resources, failure)?;
            }
            write_optional_sequence_ref(row, batch.source_count(), refs.sources, failure)?;
            write_optional_sequence_ref(
                row,
                batch.provenance_node_count(),
                refs.provenance,
                failure,
            )?;
            write_optional_sequence_ref(row, batch.operation_count(), refs.operations, failure)?;
            write_optional_sequence_ref(row, shape.evidence_count(), refs.evidence, failure)?;
            write_optional_sequence_ref(row, shape.parent_count(), refs.parents, failure)?;
            write_optional_sequence_ref(row, shape.wide_count(), refs.wide, failure)?;
            write_optional_utf8_ref(row, shape.lineage_text_bytes(), refs.text, failure)?;
            write_optional_bytes_ref(row, shape.lineage_digest_bytes(), refs.digests, failure)?;
            row.write_scalar(faults.segment_nonce(segment_nonce.get()))?;
            row.write_scalar(faults.segment_provenance_base(0))
        })
    })
    .map(CanonicalBlockRef::from_store)
}

fn retain_owners<'lane, O, PP, U, L, R>(
    lane: &mut CanonicalLane<'lane, O, PP, U, L>,
    plans: &ProfileMaterializationPlans,
    batch: &ValidatedProfileBatch<'_, R>,
    refs: ProfileLeafRefs,
    faults: &mut ProfileBuildFault,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileOwnerTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let attachments = refs.attachments;
    lane.try_retain_records(plans.owners, ProfileOwnerTableSchema, |control| {
        let mut attachment_offset = 0_u32;
        for ordinal in 0..batch.owner_count() {
            let (owner, provenance) = batch.owner_provenance_at(ordinal).ok_or(failure)?;
            let attachment_count = u32::try_from(provenance.len()).map_err(|_| failure)?;
            if attachment_count == 0 {
                return Err(failure.into());
            }
            let (owner_kind, owner_index) = encode_owner(owner);
            let owner_kind = faults.owner_kind(owner_kind);
            let stored_attachment_offset = faults.owner_attachment_offset(attachment_offset);
            control.write_row(|row| {
                row.write_scalar(owner_kind)?;
                row.write_scalar(owner_index)?;
                row.write_scalar(stored_attachment_offset)?;
                row.write_scalar(attachment_count)?;
                row.write_sequence_ref(attachments.ok_or(failure)?.into_store())
            })?;
            attachment_offset = attachment_offset
                .checked_add(attachment_count)
                .ok_or(failure)?;
        }
        if attachment_offset != batch.retention_shape().attachment_count() {
            return Err(failure.into());
        }
        Ok(())
    })
    .map(CanonicalBlockRef::from_store)
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct ProfileLeafRefs {
    pub(in crate::canonical_scene::store) resources:
        Option<CanonicalBlockRef<ProfileLineageResourceSchema>>,
    pub(in crate::canonical_scene::store) sources:
        Option<CanonicalBlockRef<ProfileLineageSourceSchema>>,
    pub(in crate::canonical_scene::store) provenance:
        Option<CanonicalBlockRef<ProfileLineageProvenanceSchema>>,
    pub(in crate::canonical_scene::store) operations:
        Option<CanonicalBlockRef<ProfileLineageOperationSchema>>,
    pub(in crate::canonical_scene::store) evidence:
        Option<CanonicalBlockRef<ProfileLineageEvidenceSchema>>,
    pub(in crate::canonical_scene::store) parents:
        Option<CanonicalBlockRef<ProfileLineageParentSchema>>,
    pub(in crate::canonical_scene::store) wide: Option<CanonicalBlockRef<ProfileLineageWideSchema>>,
    pub(in crate::canonical_scene::store) text: Option<CanonicalBlockRef<ProfileLineageTextSchema>>,
    pub(in crate::canonical_scene::store) digests:
        Option<CanonicalBlockRef<ProfileLineageDigestSchema>>,
    pub(in crate::canonical_scene::store) attachments:
        Option<CanonicalBlockRef<ProfileLineageAttachmentSchema>>,
}

/// Independently retained lineage members expected from one materialized profile graph.
///
/// Fresh construction owns one exact segment. Derived edits map an already validated prefix and
/// append one exact segment while replacing the generation-wide attachment member.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) enum ProfileLineageMaterializedRefs {
    ExactSingle(ProfileLeafRefs),
    MappedPrefixAndAppend {
        prefix_segment_count: u32,
        appended: ProfileLeafRefs,
    },
}

impl ProfileLineageMaterializedRefs {
    pub(in crate::canonical_scene::store) const fn attachments(
        self,
    ) -> Option<CanonicalBlockRef<ProfileLineageAttachmentSchema>> {
        match self {
            Self::ExactSingle(refs) | Self::MappedPrefixAndAppend { appended: refs, .. } => {
                refs.attachments
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct MetadataLeafRefs {
    pub(in crate::canonical_scene::store) entries:
        Option<CanonicalBlockRef<ProfileMetadataEntryTableSchema>>,
    pub(in crate::canonical_scene::store) text:
        Option<CanonicalBlockRef<ProfileMetadataTextSchema>>,
    pub(in crate::canonical_scene::store) bytes:
        Option<CanonicalBlockRef<ProfileMetadataBytesSchema>>,
    pub(in crate::canonical_scene::store) bools:
        Option<CanonicalBlockRef<ProfileMetadataBoolSchema>>,
    pub(in crate::canonical_scene::store) i64s: Option<CanonicalBlockRef<ProfileMetadataI64Schema>>,
    pub(in crate::canonical_scene::store) u64s: Option<CanonicalBlockRef<ProfileMetadataU64Schema>>,
    pub(in crate::canonical_scene::store) f64s: Option<CanonicalBlockRef<ProfileMetadataF64Schema>>,
    pub(in crate::canonical_scene::store) f32s: Option<CanonicalBlockRef<ProfileMetadataF32Schema>>,
}

fn encode_owner(owner: CanonicalProfileOwner) -> (u8, u32) {
    use crate::canonical_scene::store::schema::profile::*;

    match owner {
        CanonicalProfileOwner::Scene => (PROFILE_OWNER_SCENE, 0),
        CanonicalProfileOwner::SyntheticRoot => (PROFILE_OWNER_SYNTHETIC_ROOT, 0),
        CanonicalProfileOwner::Node(index) => (PROFILE_OWNER_NODE, index),
        CanonicalProfileOwner::Camera(index) => (PROFILE_OWNER_CAMERA, index),
        CanonicalProfileOwner::Light(index) => (PROFILE_OWNER_LIGHT, index),
        CanonicalProfileOwner::Image(index) => (PROFILE_OWNER_IMAGE, index),
        CanonicalProfileOwner::Sampler(index) => (PROFILE_OWNER_SAMPLER, index),
        CanonicalProfileOwner::Texture(index) => (PROFILE_OWNER_TEXTURE, index),
        CanonicalProfileOwner::TextureBinding(index) => (PROFILE_OWNER_TEXTURE_BINDING, index),
        CanonicalProfileOwner::Material(index) => (PROFILE_OWNER_MATERIAL, index),
        CanonicalProfileOwner::LogicalMesh(index) => (PROFILE_OWNER_LOGICAL_MESH, index),
        CanonicalProfileOwner::Primitive(index) => (PROFILE_OWNER_PRIMITIVE, index),
        CanonicalProfileOwner::Attribute(index) => (PROFILE_OWNER_ATTRIBUTE, index),
        CanonicalProfileOwner::MeshInstance(index) => (PROFILE_OWNER_MESH_INSTANCE, index),
        CanonicalProfileOwner::Skin(index) => (PROFILE_OWNER_SKIN, index),
        CanonicalProfileOwner::MorphDefinition(index) => (PROFILE_OWNER_MORPH_DEFINITION, index),
        CanonicalProfileOwner::MorphDelta(index) => (PROFILE_OWNER_MORPH_DELTA, index),
        CanonicalProfileOwner::AnimationClip(index) => (PROFILE_OWNER_ANIMATION_CLIP, index),
        CanonicalProfileOwner::AnimationChannel(index) => (PROFILE_OWNER_ANIMATION_CHANNEL, index),
    }
}

fn write_optional_sequence_ref<Tag, T>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
    T: StoredScalar,
{
    row.write_scalar(count)?;
    match (count, reference) {
        (0, None) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (0, Some(_)) | (1.., None) => Err(failure.into()),
        (1.., Some(reference)) => row.write_sequence_ref(reference.into_store()),
    }
}

fn write_optional_utf8_ref<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    row.write_scalar(count)?;
    match (count, reference) {
        (0, None) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (0, Some(_)) | (1.., None) => Err(failure.into()),
        (1.., Some(reference)) => row.write_utf8_ref(reference.into_store()),
    }
}

fn write_optional_bytes_ref<Tag>(
    row: &mut CanonicalRecordBuildRow<'_, '_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreControlledError>
where
    Tag: CanonicalSchema<StoreKind = ByteBlock>,
{
    row.write_scalar(count)?;
    match (count, reference) {
        (0, None) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (0, Some(_)) | (1.., None) => Err(failure.into()),
        (1.., Some(reference)) => row.write_bytes_ref(reference.into_store()),
    }
}

fn rows(count: u32, stride: usize) -> Result<u32, StorePlanError> {
    count
        .checked_mul(u32::try_from(stride).map_err(|_| StorePlanError::SizeOverflow)?)
        .ok_or(StorePlanError::SizeOverflow)
}

fn record_plan(count: u32, fields: usize) -> Result<Option<RecordPlan>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(RecordPlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
            fields,
        )?))
    }
}
fn sequence_plan(count: u32) -> Result<Option<SequencePlan<u32>>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(SequencePlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}

fn provenance_address_sequence_plan(
    count: u32,
) -> Result<Option<SequencePlan<u64>>, StorePlanError> {
    if count == 0 {
        return Ok(None);
    }
    let count = usize::try_from(count)
        .map_err(|_| StorePlanError::SizeOverflow)?
        .checked_mul(PROFILE_PROVENANCE_ADDRESS_SCALARS)
        .ok_or(StorePlanError::SizeOverflow)?;
    SequencePlan::new(count).map(Some)
}
fn sequence_plan_u8(count: u32) -> Result<Option<SequencePlan<u8>>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(SequencePlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}
fn sequence_plan_i64(count: u32) -> Result<Option<SequencePlan<i64>>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(SequencePlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}
fn sequence_plan_u64(count: u32) -> Result<Option<SequencePlan<u64>>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(SequencePlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}
fn sequence_plan_f64(count: u32) -> Result<Option<SequencePlan<f64>>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(SequencePlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}
fn sequence_plan_f32(count: u32) -> Result<Option<SequencePlan<f32>>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(SequencePlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}
fn utf8_plan(count: u32) -> Result<Option<Utf8Plan>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(Utf8Plan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}
fn bytes_plan(count: u32) -> Result<Option<BytesPlan>, StorePlanError> {
    if count == 0 {
        Ok(None)
    } else {
        Ok(Some(BytesPlan::new(
            usize::try_from(count).map_err(|_| StorePlanError::SizeOverflow)?,
        )?))
    }
}
