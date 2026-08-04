//! Fresh-generation retained materialization for source identity, provenance, and metadata.
//!
//! The profile has a deliberately asymmetric shape. Immutable scalar, text, and byte leaves
//! are owned by a lineage segment; the profile, segment, owner, and metadata directories are
//! generation-local record blocks. A future edit can therefore retain prior segment leaves while
//! rebuilding only its relationship records.

use baozi_operation::{
    ByteBlock, BytesPlan, CanonicalRecordRowWriter, RecordPlan, ScalarSequence, SequencePlan,
    StoreBuildError, StoreControlledError, StorePlanError, StoreRuntimeError, StoredScalar,
    Utf8Block, Utf8Plan,
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

use super::NodeSceneWorkMeter;
use crate::canonical_scene::store::construction::{SceneStoreAssembly, SceneStoreKeys};

mod lineage;
mod metadata;

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
}

#[derive(Clone, Copy)]
pub(super) struct RetainedProfileRefs {
    directory: CanonicalBlockRef<ProfileDirectorySchema>,
}

impl RetainedProfileRefs {
    pub(super) const fn directory(self) -> CanonicalBlockRef<ProfileDirectorySchema> {
        self.directory
    }
}

pub(super) fn retain_profile<R>(
    assembly: &mut SceneStoreAssembly,
    plans: &ProfileMaterializationPlans,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<RetainedProfileRefs, StoreRuntimeError> {
    // The assembly proves operation admission. Issuing here keeps pure authoring
    // preflight and operation-start rejection free of invisible nonce gaps.
    let segment_nonce =
        issue_profile_segment_nonce().map_err(|_| StoreRuntimeError::MembershipExhausted)?;
    let lineage =
        lineage::retain_lineage(assembly, plans, batch, segment_nonce, keys, work, failure)?;
    let metadata = metadata::retain_metadata(assembly, plans, batch, keys, work, failure)?;
    let segments = retain_segments(
        assembly,
        plans,
        batch,
        lineage,
        segment_nonce,
        keys,
        work,
        failure,
    )?;
    let owners = retain_owners(assembly, plans, batch, lineage, keys, work, failure)?;
    let directory = assembly
        .try_retain_canonical_records_controlled(
            plans.directory,
            keys.dimension,
            ProfileDirectorySchema,
            |writer, control| {
                work.observe(control, 1)?;
                writer
                    .write_row(|row| {
                        row.write_scalar(PROFILE_SCHEMA_REVISION)?;
                        row.write_scalar(1_u32)?;
                        row.write_record_ref(segments.into_store())?;
                        row.write_scalar(u32::try_from(batch.owner_count()).map_err(|_| failure)?)?;
                        row.write_record_ref(owners.into_store())?;
                        row.write_record_ref(metadata.directory.into_store())
                    })
                    .map_err(StoreControlledError::from)
            },
        )
        .map(CanonicalBlockRef::from_store)?;
    Ok(RetainedProfileRefs { directory })
}

#[allow(clippy::too_many_arguments)]
fn retain_segments<R>(
    assembly: &mut SceneStoreAssembly,
    plans: &ProfileMaterializationPlans,
    batch: &ValidatedProfileBatch<'_, R>,
    refs: ProfileLeafRefs,
    segment_nonce: ProfileSegmentNonce,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileSegmentTableSchema>, StoreRuntimeError> {
    let shape = batch.retention_shape();
    assembly
        .try_retain_canonical_records_controlled(
            plans.segments,
            keys.dimension,
            ProfileSegmentTableSchema,
            |writer, control| {
                work.observe(control, 1)?;
                writer
                    .write_row(|row| {
                        row.write_scalar(0_u32)?;
                        write_optional_sequence_ref(
                            row,
                            batch.resource_count(),
                            refs.resources,
                            failure,
                        )?;
                        write_optional_sequence_ref(
                            row,
                            batch.source_count(),
                            refs.sources,
                            failure,
                        )?;
                        write_optional_sequence_ref(
                            row,
                            batch.provenance_node_count(),
                            refs.provenance,
                            failure,
                        )?;
                        write_optional_sequence_ref(
                            row,
                            batch.operation_count(),
                            refs.operations,
                            failure,
                        )?;
                        write_optional_sequence_ref(
                            row,
                            shape.evidence_count(),
                            refs.evidence,
                            failure,
                        )?;
                        write_optional_sequence_ref(
                            row,
                            shape.parent_count(),
                            refs.parents,
                            failure,
                        )?;
                        write_optional_sequence_ref(row, shape.wide_count(), refs.wide, failure)?;
                        write_optional_utf8_ref(
                            row,
                            shape.lineage_text_bytes(),
                            refs.text,
                            failure,
                        )?;
                        write_optional_bytes_ref(
                            row,
                            shape.lineage_digest_bytes(),
                            refs.digests,
                            failure,
                        )?;
                        row.write_scalar(segment_nonce.get())?;
                        row.write_scalar(0_u32)
                    })
                    .map_err(StoreControlledError::from)
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_owners<R>(
    assembly: &mut SceneStoreAssembly,
    plans: &ProfileMaterializationPlans,
    batch: &ValidatedProfileBatch<'_, R>,
    refs: ProfileLeafRefs,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileOwnerTableSchema>, StoreRuntimeError> {
    let attachments = refs.attachments;
    assembly
        .try_retain_canonical_records_controlled(
            plans.owners,
            keys.dimension,
            ProfileOwnerTableSchema,
            |writer, control| {
                let mut attachment_offset = 0_u32;
                for ordinal in 0..batch.owner_count() {
                    work.observe(control, 1)?;
                    let (owner, provenance) = batch.owner_provenance_at(ordinal).ok_or(failure)?;
                    let attachment_count = u32::try_from(provenance.len()).map_err(|_| failure)?;
                    if attachment_count == 0 {
                        return Err(failure.into());
                    }
                    let (owner_kind, owner_index) = encode_owner(owner);
                    writer.write_row(|row| {
                        row.write_scalar(owner_kind)?;
                        row.write_scalar(owner_index)?;
                        row.write_scalar(attachment_offset)?;
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
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[derive(Clone, Copy)]
struct ProfileLeafRefs {
    resources: Option<CanonicalBlockRef<ProfileLineageResourceSchema>>,
    sources: Option<CanonicalBlockRef<ProfileLineageSourceSchema>>,
    provenance: Option<CanonicalBlockRef<ProfileLineageProvenanceSchema>>,
    operations: Option<CanonicalBlockRef<ProfileLineageOperationSchema>>,
    evidence: Option<CanonicalBlockRef<ProfileLineageEvidenceSchema>>,
    parents: Option<CanonicalBlockRef<ProfileLineageParentSchema>>,
    wide: Option<CanonicalBlockRef<ProfileLineageWideSchema>>,
    text: Option<CanonicalBlockRef<ProfileLineageTextSchema>>,
    digests: Option<CanonicalBlockRef<ProfileLineageDigestSchema>>,
    attachments: Option<CanonicalBlockRef<ProfileLineageAttachmentSchema>>,
}

#[derive(Clone, Copy)]
struct MetadataLeafRefs {
    entries: Option<CanonicalBlockRef<ProfileMetadataEntryTableSchema>>,
    text: Option<CanonicalBlockRef<ProfileMetadataTextSchema>>,
    bytes: Option<CanonicalBlockRef<ProfileMetadataBytesSchema>>,
    bools: Option<CanonicalBlockRef<ProfileMetadataBoolSchema>>,
    i64s: Option<CanonicalBlockRef<ProfileMetadataI64Schema>>,
    u64s: Option<CanonicalBlockRef<ProfileMetadataU64Schema>>,
    f64s: Option<CanonicalBlockRef<ProfileMetadataF64Schema>>,
    f32s: Option<CanonicalBlockRef<ProfileMetadataF32Schema>>,
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
    row: &mut CanonicalRecordRowWriter<'_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = ScalarSequence<T>>,
    T: StoredScalar,
{
    row.write_scalar(count)?;
    match (count, reference) {
        (0, None) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (0, Some(_)) | (1.., None) => Err(failure),
        (1.., Some(reference)) => row.write_sequence_ref(reference.into_store()),
    }
}

fn write_optional_utf8_ref<Tag>(
    row: &mut CanonicalRecordRowWriter<'_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = Utf8Block>,
{
    row.write_scalar(count)?;
    match (count, reference) {
        (0, None) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (0, Some(_)) | (1.., None) => Err(failure),
        (1.., Some(reference)) => row.write_utf8_ref(reference.into_store()),
    }
}

fn write_optional_bytes_ref<Tag>(
    row: &mut CanonicalRecordRowWriter<'_>,
    count: u32,
    reference: Option<CanonicalBlockRef<Tag>>,
    failure: StoreBuildError,
) -> Result<(), StoreBuildError>
where
    Tag: CanonicalSchema<StoreKind = ByteBlock>,
{
    row.write_scalar(count)?;
    match (count, reference) {
        (0, None) => row.write_scalar(PROFILE_ABSENT_REFERENCE),
        (0, Some(_)) | (1.., None) => Err(failure),
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
