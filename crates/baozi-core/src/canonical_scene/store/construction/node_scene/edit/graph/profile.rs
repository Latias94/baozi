use baozi_operation::{
    CanonicalInheritanceSource, CanonicalRecordView, StoreBuildError, StoreControlledError,
    StoreRuntimeError,
};

use crate::canonical_scene::store::schema::CanonicalBlockRef;
use crate::canonical_scene::store::schema::profile::{
    PROFILE_ABSENT_REFERENCE, PROFILE_DIGEST_BLAKE3, PROFILE_DIGEST_SHA256,
    PROFILE_DIRECTORY_FIELD_COUNT, PROFILE_OPERATION_SCALAR_COUNT, PROFILE_OPERATION_TRANSFORM,
    PROFILE_OWNER_FIELD_COUNT, PROFILE_PROVENANCE_OPERATION, PROFILE_PROVENANCE_SCALAR_COUNT,
    PROFILE_SCHEMA_REVISION, PROFILE_SEGMENT_FIELD_COUNT, ProfileDirectorySchema,
    ProfileLineageAttachmentSchema, ProfileLineageDigestSchema, ProfileLineageOperationSchema,
    ProfileLineageParentSchema, ProfileLineageProvenanceSchema, ProfileMetadataDirectorySchema,
    ProfileMetadataEntryTableSchema, ProfileOwnerTableSchema, ProfileSegmentTableSchema,
    profile_directory_field, profile_owner_field, profile_provenance_scalar, profile_segment_field,
};
use crate::provenance::{DIGEST_LEN, DigestAlgorithm, OperationKind};

use super::super::super::{
    NODE_SCENE_BUILD_INVARIANT_CODE, NodeSceneWorkMeter, SceneStoreAssembly, SceneStoreKeys,
    process_slice_in_work_chunks,
};
use super::super::plan::PendingNodeTransformEdit;

#[derive(Clone, Copy)]
pub(super) struct RewrittenProfileLineageLeaves {
    pub(super) attachments: CanonicalBlockRef<ProfileLineageAttachmentSchema>,
    pub(super) parents: CanonicalBlockRef<ProfileLineageParentSchema>,
    pub(super) provenance: CanonicalBlockRef<ProfileLineageProvenanceSchema>,
    pub(super) operation: CanonicalBlockRef<ProfileLineageOperationSchema>,
    pub(super) digest: CanonicalBlockRef<ProfileLineageDigestSchema>,
}

pub(super) fn rewrite_lineage_leaves<'scene>(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource<'scene>>,
    pending: &PendingNodeTransformEdit<'scene>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<RewrittenProfileLineageLeaves, StoreRuntimeError> {
    let plan = pending.counts.profile;
    let failure = StoreBuildError::domain(NODE_SCENE_BUILD_INVARIANT_CODE)
        .map_err(StoreRuntimeError::Plan)?;
    if !matches!(pending.operation.kind(), OperationKind::Transform)
        || pending.operation.evidence().len() != 0
    {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    let source_attachments = pending
        .input
        .canonical
        .sequence(plan.source_attachments.into_store())
        .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
    let source_values = source_attachments.as_slice();
    if source_values.len() != plan.source_attachment_count.as_usize() {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    let target_start = plan.target_attachment_offset.as_usize();
    let target_end = target_start
        .checked_add(plan.target_attachment_count.as_usize())
        .ok_or(StoreRuntimeError::AccountingOverflow)?;
    let target_parents = source_values
        .get(target_start..target_end)
        .ok_or(StoreRuntimeError::InvalidSourceSlot)?;

    // This exact source leaf must be mapped before bulk inheritance so the old attachment
    // storage is not retained into the edited generation.
    let attachments = assembly
        .try_rewrite_canonical_sequence_controlled(
            plan.source_attachments.into_store(),
            plan.attachments,
            keys.dimension,
            ProfileLineageAttachmentSchema,
            |writer, control| {
                let prefix = source_values.get(..target_start).ok_or(failure)?;
                let suffix = source_values.get(target_end..).ok_or(failure)?;
                process_slice_in_work_chunks(prefix, control, work, failure, |chunk| {
                    writer.extend_from_slice(chunk)
                })?;
                work.observe(control, 1)?;
                writer.push(plan.fresh_provenance_index.as_u32())?;
                process_slice_in_work_chunks(suffix, control, work, failure, |chunk| {
                    writer.extend_from_slice(chunk)
                })
            },
        )
        .map(CanonicalBlockRef::from_store)?;

    let segments = required_segment_view(pending)?;
    let mut fresh_depth = None;
    let parents = assembly
        .try_retain_canonical_sequence_controlled(
            plan.parents,
            keys.dimension,
            ProfileLineageParentSchema,
            |writer, control| {
                let mut previous = None;
                for parent in target_parents.iter().copied() {
                    work.observe(control, 1)?;
                    if parent >= plan.fresh_provenance_index.as_u32()
                        || previous.is_some_and(|previous| previous >= parent)
                    {
                        return Err(failure.into());
                    }
                    writer.push(parent)?;
                    previous = Some(parent);
                }
                fresh_depth = Some(resolve_fresh_depth(
                    pending,
                    &segments,
                    target_parents,
                    control,
                    work,
                    failure,
                )?);
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)?;
    let fresh_depth = fresh_depth.ok_or(StoreRuntimeError::InvalidSourceSlot)?;

    let digest = assembly
        .try_retain_canonical_bytes_controlled(
            plan.digest,
            keys.dimension,
            ProfileLineageDigestSchema,
            |writer, control| {
                work.observe(control, u64::try_from(DIGEST_LEN).map_err(|_| failure)?)?;
                writer
                    .write_bytes(pending.operation.options_digest().as_bytes())
                    .map_err(Into::into)
            },
        )
        .map(CanonicalBlockRef::from_store)?;

    let operation = assembly
        .try_retain_canonical_sequence_controlled(
            plan.operation,
            keys.dimension,
            ProfileLineageOperationSchema,
            |writer, control| {
                work.observe(
                    control,
                    u64::try_from(PROFILE_OPERATION_SCALAR_COUNT).map_err(|_| failure)?,
                )?;
                writer
                    .extend_from_slice(&[
                        u32::from(PROFILE_OPERATION_TRANSFORM),
                        0,
                        0,
                        u32::from(encode_digest_algorithm(
                            pending.operation.options_digest().algorithm(),
                        )),
                        0,
                        u32::try_from(DIGEST_LEN).map_err(|_| failure)?,
                        0,
                        0,
                    ])
                    .map_err(Into::into)
            },
        )
        .map(CanonicalBlockRef::from_store)?;

    let provenance = assembly
        .try_retain_canonical_sequence_controlled(
            plan.provenance,
            keys.dimension,
            ProfileLineageProvenanceSchema,
            |writer, control| {
                work.observe(
                    control,
                    u64::try_from(PROFILE_PROVENANCE_SCALAR_COUNT).map_err(|_| failure)?,
                )?;
                writer
                    .extend_from_slice(&[
                        u32::from(PROFILE_PROVENANCE_OPERATION),
                        0,
                        fresh_depth,
                        0,
                        plan.target_attachment_count.as_u32(),
                    ])
                    .map_err(Into::into)
            },
        )
        .map(CanonicalBlockRef::from_store)?;

    Ok(RewrittenProfileLineageLeaves {
        attachments,
        parents,
        provenance,
        operation,
        digest,
    })
}

pub(super) fn rewrite_profile_records<'scene>(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource<'scene>>,
    pending: &PendingNodeTransformEdit<'scene>,
    segments: CanonicalBlockRef<ProfileSegmentTableSchema>,
    leaves: RewrittenProfileLineageLeaves,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<CanonicalBlockRef<ProfileDirectorySchema>, StoreRuntimeError> {
    let failure = StoreBuildError::domain(NODE_SCENE_BUILD_INVARIANT_CODE)
        .map_err(StoreRuntimeError::Plan)?;
    let input = pending.input;
    let source_owners = input
        .canonical
        .records(input.proof.profile_owners().into_store())
        .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
    if source_owners.rows() != pending.counts.profile.owners.rows()
        || source_owners.fields() != PROFILE_OWNER_FIELD_COUNT
    {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    let removed_attachments = pending
        .counts
        .profile
        .target_attachment_count
        .as_u32()
        .checked_sub(1)
        .ok_or(StoreRuntimeError::AccountingOverflow)?;
    let owners = assembly
        .try_rewrite_canonical_records_with_controlled(
            input.proof.profile_owners().into_store(),
            pending.counts.profile.owners,
            keys.dimension,
            ProfileOwnerTableSchema,
            |writer, control| {
                for row_index in 0..source_owners.rows() {
                    work.observe(control, 1)?;
                    let owner_kind = source_owners
                        .scalar::<u8>(row_index, profile_owner_field::OWNER_KIND)
                        .ok_or(failure)?;
                    let owner_index = source_owners
                        .scalar::<u32>(row_index, profile_owner_field::OWNER_INDEX)
                        .ok_or(failure)?;
                    let source_offset = source_owners
                        .scalar::<u32>(row_index, profile_owner_field::ATTACHMENT_OFFSET)
                        .ok_or(failure)?;
                    let source_count = source_owners
                        .scalar::<u32>(row_index, profile_owner_field::ATTACHMENT_COUNT)
                        .ok_or(failure)?;
                    let (attachment_offset, attachment_count) =
                        match row_index.cmp(&pending.counts.profile.target_owner_row.as_usize()) {
                            std::cmp::Ordering::Less => (source_offset, source_count),
                            std::cmp::Ordering::Equal => (source_offset, 1),
                            std::cmp::Ordering::Greater => (
                                source_offset
                                    .checked_sub(removed_attachments)
                                    .ok_or(failure)?,
                                source_count,
                            ),
                        };
                    writer.write_row(|row| {
                        row.write_scalar(owner_kind)?;
                        row.write_scalar(owner_index)?;
                        row.write_scalar(attachment_offset)?;
                        row.write_scalar(attachment_count)?;
                        row.write_sequence_ref(leaves.attachments.into_store())
                    })?;
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)?;

    super::rewrite_optional_record(
        assembly,
        input,
        input.proof.profile_metadata_entries(),
        usize::try_from(pending.counts.evidence.profile_metadata_entry_count)
            .map_err(|_| StoreRuntimeError::AccountingOverflow)?,
        ProfileMetadataEntryTableSchema,
        keys,
    )?;
    let metadata = super::rewrite_required_record(
        assembly,
        input,
        input.proof.profile_metadata_directory(),
        1,
        ProfileMetadataDirectorySchema,
        keys,
    )?;

    let source_directory = input
        .canonical
        .records(input.proof.profile_directory().into_store())
        .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
    if source_directory.rows() != 1
        || source_directory.fields() != PROFILE_DIRECTORY_FIELD_COUNT
        || source_directory.scalar::<u32>(0, profile_directory_field::SCHEMA_REVISION)
            != Some(PROFILE_SCHEMA_REVISION)
    {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    assembly
        .try_rewrite_canonical_records_with_controlled(
            input.proof.profile_directory().into_store(),
            baozi_operation::RecordPlan::new(1, PROFILE_DIRECTORY_FIELD_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            keys.dimension,
            ProfileDirectorySchema,
            |writer, control| {
                work.observe(control, 1)?;
                writer
                    .write_row(|row| {
                        row.write_scalar(PROFILE_SCHEMA_REVISION)?;
                        row.write_scalar(pending.counts.profile.output_segment_count.as_u32())?;
                        row.write_record_ref(segments.into_store())?;
                        row.write_scalar(pending.counts.evidence.profile_owner_count)?;
                        row.write_record_ref(owners.into_store())?;
                        row.write_record_ref(metadata.into_store())
                    })
                    .map_err(Into::into)
            },
        )
        .map(CanonicalBlockRef::from_store)
}

pub(super) fn rewrite_segments<'scene>(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource<'scene>>,
    pending: &PendingNodeTransformEdit<'scene>,
    leaves: RewrittenProfileLineageLeaves,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<CanonicalBlockRef<ProfileSegmentTableSchema>, StoreRuntimeError> {
    let failure = StoreBuildError::domain(NODE_SCENE_BUILD_INVARIANT_CODE)
        .map_err(StoreRuntimeError::Plan)?;
    let source_plan = baozi_operation::RecordPlan::new(
        pending.counts.profile.source_segment_count.as_usize(),
        PROFILE_SEGMENT_FIELD_COUNT,
    )
    .map_err(StoreRuntimeError::Plan)?;
    assembly
        .try_rewrite_canonical_records_with_mapped_prefix_controlled(
            pending.input.proof.profile_segments().into_store(),
            source_plan,
            pending.counts.profile.segments,
            keys.dimension,
            ProfileSegmentTableSchema,
            |writer, control| {
                work.observe(control, 1)?;
                writer
                    .write_row(|row| {
                        row.write_scalar(pending.counts.profile.source_segment_count.as_u32())?;
                        row.write_scalar(0_u32)?;
                        row.write_scalar(PROFILE_ABSENT_REFERENCE)?;
                        row.write_scalar(0_u32)?;
                        row.write_scalar(PROFILE_ABSENT_REFERENCE)?;
                        row.write_scalar(1_u32)?;
                        row.write_sequence_ref(leaves.provenance.into_store())?;
                        row.write_scalar(1_u32)?;
                        row.write_sequence_ref(leaves.operation.into_store())?;
                        row.write_scalar(0_u32)?;
                        row.write_scalar(PROFILE_ABSENT_REFERENCE)?;
                        row.write_scalar(pending.counts.profile.target_attachment_count.as_u32())?;
                        row.write_sequence_ref(leaves.parents.into_store())?;
                        row.write_scalar(0_u32)?;
                        row.write_scalar(PROFILE_ABSENT_REFERENCE)?;
                        row.write_scalar(0_u32)?;
                        row.write_scalar(PROFILE_ABSENT_REFERENCE)?;
                        row.write_scalar(u32::try_from(DIGEST_LEN).map_err(|_| failure)?)?;
                        row.write_bytes_ref(leaves.digest.into_store())
                    })
                    .map_err(Into::into)
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn required_segment_view<'scene>(
    pending: &'scene PendingNodeTransformEdit<'_>,
) -> Result<CanonicalRecordView<'scene>, StoreRuntimeError> {
    let segments = pending
        .input
        .canonical
        .records(pending.input.proof.profile_segments().into_store())
        .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
    if segments.rows() != pending.counts.profile.source_segment_count.as_usize()
        || segments.fields() != PROFILE_SEGMENT_FIELD_COUNT
    {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    Ok(segments)
}

fn resolve_fresh_depth(
    pending: &PendingNodeTransformEdit<'_>,
    segments: &CanonicalRecordView<'_>,
    parents: &[u32],
    control: &mut baozi_operation::RetainedStoreWorkControl<
        '_,
        super::super::super::super::SceneConstructionOwner,
        super::super::super::super::SceneConstructionPhase,
        super::super::super::super::SceneConstructionUnit,
    >,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<u32, StoreControlledError> {
    let mut parent_cursor = 0_usize;
    let mut provenance_base = 0_u32;
    let mut maximum_depth = 0_u32;

    for segment_row in 0..segments.rows() {
        if parent_cursor == parents.len() {
            break;
        }
        work.observe(control, 1)?;
        let count = segments
            .scalar::<u32>(segment_row, profile_segment_field::PROVENANCE_COUNT)
            .ok_or(failure)?;
        let end = provenance_base.checked_add(count).ok_or(failure)?;
        let next_parent = *parents.get(parent_cursor).ok_or(failure)?;
        if next_parent < provenance_base {
            return Err(failure.into());
        }
        if next_parent < end {
            let provenance_ref = segments
                .sequence_ref::<u32, ProfileLineageProvenanceSchema>(
                    segment_row,
                    profile_segment_field::PROVENANCE_NODES,
                )
                .ok_or(failure)?;
            let provenance = pending
                .input
                .canonical
                .sequence(provenance_ref)
                .ok_or(failure)?;
            let expected_scalars = usize::try_from(count)
                .ok()
                .and_then(|count| count.checked_mul(PROFILE_PROVENANCE_SCALAR_COUNT))
                .ok_or(failure)?;
            if provenance.as_slice().len() != expected_scalars {
                return Err(failure.into());
            }
            while parent_cursor < parents.len() {
                let parent = parents[parent_cursor];
                if parent >= end {
                    break;
                }
                let local = parent.checked_sub(provenance_base).ok_or(failure)?;
                let scalar = usize::try_from(local)
                    .ok()
                    .and_then(|row| row.checked_mul(PROFILE_PROVENANCE_SCALAR_COUNT))
                    .and_then(|offset| offset.checked_add(profile_provenance_scalar::DEPTH))
                    .ok_or(failure)?;
                work.observe(control, 1)?;
                let depth = *provenance.as_slice().get(scalar).ok_or(failure)?;
                maximum_depth = maximum_depth.max(depth);
                parent_cursor = parent_cursor.checked_add(1).ok_or(failure)?;
            }
        }
        provenance_base = end;
    }
    if parent_cursor != parents.len() {
        return Err(failure.into());
    }
    maximum_depth.checked_add(1).ok_or_else(|| failure.into())
}

const fn encode_digest_algorithm(algorithm: DigestAlgorithm) -> u8 {
    match algorithm {
        DigestAlgorithm::Sha256 => PROFILE_DIGEST_SHA256,
        DigestAlgorithm::Blake3 => PROFILE_DIGEST_BLAKE3,
    }
}
