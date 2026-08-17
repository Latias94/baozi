use baozi_operation::{
    CanonicalInheritanceSource, CanonicalRecordView, StoreBuildError, StoreControlledError,
    StoreRuntimeError,
};

use crate::canonical_scene::store::profile_provenance::{
    PROFILE_PROVENANCE_ADDRESS_SCALARS, ProfileSegmentAddressDomain, ProfileSegmentNonce,
    QualifiedProvenanceAddress, QualifiedProvenanceAddressSlice, issue_profile_segment_nonce,
};
use crate::canonical_scene::store::schema::CanonicalBlockRef;
use crate::canonical_scene::store::schema::profile::{
    PROFILE_ABSENT_REFERENCE, PROFILE_DIGEST_BLAKE3, PROFILE_DIGEST_SHA256,
    PROFILE_DIRECTORY_FIELD_COUNT, PROFILE_OPERATION_SCALAR_COUNT, PROFILE_OWNER_FIELD_COUNT,
    PROFILE_PROVENANCE_OPERATION, PROFILE_PROVENANCE_SCALAR_COUNT, PROFILE_SCHEMA_REVISION,
    PROFILE_SEGMENT_FIELD_COUNT, ProfileDirectorySchema, ProfileLineageAttachmentSchema,
    ProfileLineageDigestSchema, ProfileLineageOperationSchema, ProfileLineageParentSchema,
    ProfileLineageProvenanceSchema, ProfileMetadataDirectorySchema,
    ProfileMetadataEntryTableSchema, ProfileOwnerTableSchema, ProfileSegmentTableSchema,
    profile_directory_field, profile_owner_field, profile_provenance_scalar, profile_segment_field,
};
use crate::provenance::{DIGEST_LEN, DigestAlgorithm};

use super::super::super::{
    NODE_SCENE_BUILD_INVARIANT_CODE, NodeSceneWorkMeter, SceneStoreAssembly, SceneStoreKeys,
    process_slice_in_work_chunks,
};
use super::super::plan::BoundPendingNodeSceneEdit;

#[derive(Clone, Copy)]
pub(super) struct RewrittenProfileLineageLeaves {
    pub(super) attachments: CanonicalBlockRef<ProfileLineageAttachmentSchema>,
    pub(super) parents: CanonicalBlockRef<ProfileLineageParentSchema>,
    pub(super) provenance: CanonicalBlockRef<ProfileLineageProvenanceSchema>,
    pub(super) operation: CanonicalBlockRef<ProfileLineageOperationSchema>,
    pub(super) digest: CanonicalBlockRef<ProfileLineageDigestSchema>,
    pub(super) segment_nonce: ProfileSegmentNonce,
}

pub(super) fn rewrite_lineage_leaves<'scene>(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'scene>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
) -> Result<RewrittenProfileLineageLeaves, StoreRuntimeError> {
    let plan = pending.counts.profile;
    let failure = StoreBuildError::domain(NODE_SCENE_BUILD_INVARIANT_CODE)
        .map_err(StoreRuntimeError::Plan)?;
    if *pending.operation.kind() != pending.program.operation_kind()
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
    let source_addresses = QualifiedProvenanceAddressSlice::from_scalars(
        source_values,
        plan.source_attachment_count.as_usize(),
    )
    .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
    let target_start = plan.target_attachment_offset.as_usize();
    let target_end = target_start
        .checked_add(plan.target_attachment_count.as_usize())
        .ok_or(StoreRuntimeError::AccountingOverflow)?;
    let target_parents = source_addresses
        .get_range(target_start, plan.target_attachment_count.as_usize())
        .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
    let segment_nonce =
        issue_profile_segment_nonce().map_err(|_| StoreRuntimeError::MembershipExhausted)?;
    let fresh_address = QualifiedProvenanceAddress::new(
        plan.source_segment_count.as_u32(),
        segment_nonce,
        0,
        plan.fresh_provenance_index.as_u32(),
    )
    .ok_or(StoreRuntimeError::AccountingOverflow)?;

    // This exact source leaf must be mapped before explicit unchanged-leaf inheritance so the
    // old attachment storage is not retained into the edited generation.
    let attachments = assembly
        .try_rewrite_canonical_sequence_controlled(
            &pending.input.canonical,
            plan.source_attachments.into_store(),
            plan.attachments,
            keys.dimension,
            ProfileLineageAttachmentSchema,
            |writer, control| {
                let prefix = source_addresses.raw_prefix(target_start).ok_or(failure)?;
                let suffix = source_addresses.raw_suffix(target_end).ok_or(failure)?;
                process_slice_in_work_chunks(prefix, control, work, failure, |chunk| {
                    writer.extend_from_slice(chunk)
                })?;
                work.observe(
                    control,
                    u64::try_from(PROFILE_PROVENANCE_ADDRESS_SCALARS).map_err(|_| failure)?,
                )?;
                writer.extend_from_slice(&fresh_address.into_scalars())?;
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
                let mut maximum_depth = 0_u32;
                for parent in target_parents.iter() {
                    work.observe(control, 1)?;
                    let parent = parent.ok_or(failure)?;
                    let (global, depth) = resolve_qualified_depth(
                        pending, &segments, parent, control, work, failure,
                    )?;
                    if global >= plan.fresh_provenance_index.as_u32()
                        || previous.is_some_and(|previous| previous >= global)
                    {
                        return Err(failure.into());
                    }
                    work.observe(
                        control,
                        u64::try_from(PROFILE_PROVENANCE_ADDRESS_SCALARS).map_err(|_| failure)?,
                    )?;
                    writer.extend_from_slice(&parent.into_scalars())?;
                    maximum_depth = maximum_depth.max(depth);
                    previous = Some(global);
                }
                fresh_depth = Some(maximum_depth.checked_add(1).ok_or(failure)?);
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
                        u32::from(pending.program.shape().profile_operation_code()),
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
        segment_nonce,
    })
}

pub(super) fn rewrite_profile_records<'scene>(
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'scene>,
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
            &input.canonical,
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
            &input.canonical,
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
    assembly: &mut SceneStoreAssembly<CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'scene>,
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
            &pending.input.canonical,
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
                        row.write_bytes_ref(leaves.digest.into_store())?;
                        row.write_scalar(leaves.segment_nonce.get())?;
                        row.write_scalar(pending.counts.profile.fresh_provenance_index.as_u32())
                    })
                    .map_err(Into::into)
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn required_segment_view<'scene>(
    pending: &'scene BoundPendingNodeSceneEdit<'_>,
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

fn resolve_qualified_depth(
    pending: &BoundPendingNodeSceneEdit<'_>,
    segments: &CanonicalRecordView<'_>,
    address: QualifiedProvenanceAddress,
    control: &mut baozi_operation::RetainedStoreWorkControl<
        '_,
        super::super::super::super::SceneConstructionOwner,
        super::super::super::super::SceneConstructionPhase,
        super::super::super::super::SceneConstructionUnit,
    >,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<(u32, u32), StoreControlledError> {
    let segment_ordinal = address.segment_ordinal().ok_or(failure)?;
    let segment_row = usize::try_from(segment_ordinal).map_err(|_| failure)?;
    if segment_row >= pending.counts.profile.source_segment_count.as_usize() {
        return Err(failure.into());
    }
    work.observe(control, 1)?;
    if segments
        .scalar::<u32>(segment_row, profile_segment_field::SEGMENT)
        .ok_or(failure)?
        != segment_ordinal
    {
        return Err(failure.into());
    }
    let nonce = segments
        .scalar::<u64>(segment_row, profile_segment_field::NONCE)
        .and_then(ProfileSegmentNonce::from_stored)
        .ok_or(failure)?;
    let provenance_base = segments
        .scalar::<u32>(segment_row, profile_segment_field::PROVENANCE_BASE)
        .ok_or(failure)?;
    let count = segments
        .scalar::<u32>(segment_row, profile_segment_field::PROVENANCE_COUNT)
        .ok_or(failure)?;
    let global = address
        .validate_against(ProfileSegmentAddressDomain::new(
            segment_ordinal,
            nonce,
            provenance_base,
            count,
        ))
        .ok_or(failure)?;
    let local = global.checked_sub(provenance_base).ok_or(failure)?;
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
    let scalar = usize::try_from(local)
        .ok()
        .and_then(|row| row.checked_mul(PROFILE_PROVENANCE_SCALAR_COUNT))
        .and_then(|offset| offset.checked_add(profile_provenance_scalar::DEPTH))
        .ok_or(failure)?;
    work.observe(control, 1)?;
    let depth = *provenance.as_slice().get(scalar).ok_or(failure)?;
    Ok((global, depth))
}

const fn encode_digest_algorithm(algorithm: DigestAlgorithm) -> u8 {
    match algorithm {
        DigestAlgorithm::Sha256 => PROFILE_DIGEST_SHA256,
        DigestAlgorithm::Blake3 => PROFILE_DIGEST_BLAKE3,
    }
}
