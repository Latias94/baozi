use baozi_operation::__retained_store_integration::activation::{
    CanonicalBuildControl, CanonicalLane,
};
use baozi_operation::__retained_store_integration::authority::CanonicalInheritanceSource;
use baozi_operation::{
    CanonicalRecordView, OperationOwner, ProgressPhase, ProgressUnit, StoreBuildError,
    StoreControlledError, StoreRuntimeError,
};

use crate::canonical_scene::store::profile_provenance::{
    ProfileSegmentAddressDomain, ProfileSegmentNonce, QualifiedProvenanceAddress,
    QualifiedProvenanceAddressSlice, issue_profile_segment_nonce,
};
use crate::canonical_scene::store::schema::CanonicalBlockRef;
use crate::canonical_scene::store::schema::profile::{
    PROFILE_ABSENT_REFERENCE, PROFILE_DIGEST_BLAKE3, PROFILE_DIGEST_SHA256,
    PROFILE_DIRECTORY_FIELD_COUNT, PROFILE_OWNER_FIELD_COUNT, PROFILE_PROVENANCE_OPERATION,
    PROFILE_PROVENANCE_SCALAR_COUNT, PROFILE_SCHEMA_REVISION, PROFILE_SEGMENT_FIELD_COUNT,
    ProfileDirectorySchema, ProfileLineageAttachmentSchema, ProfileLineageDigestSchema,
    ProfileLineageOperationSchema, ProfileLineageParentSchema, ProfileLineageProvenanceSchema,
    ProfileMetadataDirectorySchema, ProfileMetadataEntryTableSchema, ProfileOwnerTableSchema,
    ProfileSegmentTableSchema, profile_directory_field, profile_owner_field,
    profile_provenance_scalar, profile_segment_field,
};
use crate::provenance::{DIGEST_LEN, DigestAlgorithm};

use super::super::super::NODE_SCENE_BUILD_INVARIANT_CODE;
use super::super::super::profile::{
    MetadataLeafRefs, ProfileLeafRefs, ProfileLineageMaterializedRefs, RetainedMetadataRefs,
    RetainedProfileRefs,
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

pub(super) fn rewrite_lineage_leaves<'lane, 'scene, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'scene>,
) -> Result<RewrittenProfileLineageLeaves, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let plan = pending.counts.profile;
    let failure = StoreBuildError::domain(NODE_SCENE_BUILD_INVARIANT_CODE)
        .map_err(StoreRuntimeError::Plan)?;
    if *pending.operation.kind() != pending.program.operation_kind() {
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
    let owners = pending
        .input
        .canonical
        .records(pending.input.proof.profile_owners().into_store())
        .ok_or(StoreRuntimeError::InvalidSourceSlot)?;
    let segment_nonce =
        issue_profile_segment_nonce().map_err(|_| StoreRuntimeError::MembershipExhausted)?;
    let output_domain = ProfileSegmentAddressDomain::new(
        plan.source_segment_count.as_u32(),
        segment_nonce,
        plan.fresh_provenance_index.as_u32(),
        plan.target_count.as_u32(),
    );
    let segments = required_segment_view(pending)?;
    let selected = |row: usize| -> Result<bool, StoreBuildError> {
        let kind = owners
            .scalar::<u8>(row, profile_owner_field::OWNER_KIND)
            .ok_or(failure)?;
        let index = owners
            .scalar::<u32>(row, profile_owner_field::OWNER_INDEX)
            .ok_or(failure)?;
        Ok(plan.selects(pending.input, kind, index))
    };
    let source_range = |row: usize| {
        let offset = owners
            .scalar::<u32>(row, profile_owner_field::ATTACHMENT_OFFSET)
            .ok_or(failure)? as usize;
        let count = owners
            .scalar::<u32>(row, profile_owner_field::ATTACHMENT_COUNT)
            .ok_or(failure)? as usize;
        source_addresses.get_range(offset, count).ok_or(failure)
    };
    // Replace the shared attachment leaf once. Each selected owner receives its own operation
    // provenance address; unaffected owner ranges keep their exact ancestor addresses.
    let attachments = lane
        .try_rewrite_sequence(
            &pending.input.canonical,
            plan.source_attachments.into_store(),
            plan.attachments,
            ProfileLineageAttachmentSchema,
            |control| {
                let mut local = 0_u32;
                for row in 0..owners.rows() {
                    control.poll()?;
                    if selected(row)? {
                        let address = output_domain.address(local).ok_or(failure)?;
                        control.extend_from_slice(&address.into_scalars())?;
                        local = local.checked_add(1).ok_or(failure)?;
                    } else {
                        for address in source_range(row)?.iter() {
                            control.extend_from_slice(&address.ok_or(failure)?.into_scalars())?;
                        }
                    }
                }
                if local != plan.target_count.as_u32() {
                    return Err(failure.into());
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)?;

    let parents = lane
        .try_retain_sequence(plan.parents, ProfileLineageParentSchema, |control| {
            for row in 0..owners.rows() {
                control.poll()?;
                if !selected(row)? {
                    continue;
                }
                let mut previous = None;
                for address in source_range(row)?.iter() {
                    let address = address.ok_or(failure)?;
                    let (global, _) =
                        resolve_qualified_depth(pending, &segments, address, control, failure)?;
                    if global >= plan.fresh_provenance_index.as_u32()
                        || previous.is_some_and(|prior| prior >= global)
                    {
                        return Err(failure.into());
                    }
                    control.extend_from_slice(&address.into_scalars())?;
                    previous = Some(global);
                }
            }
            Ok(())
        })
        .map(CanonicalBlockRef::from_store)?;
    let digest = lane
        .try_retain_bytes(plan.digest, ProfileLineageDigestSchema, |control| {
            for _ in 0..plan.target_count.as_usize() {
                control.write_bytes(pending.operation.options_digest().as_bytes())?;
            }
            Ok(())
        })
        .map(CanonicalBlockRef::from_store)?;
    let operation = lane
        .try_retain_sequence(plan.operation, ProfileLineageOperationSchema, |control| {
            for operation in 0..plan.target_count.as_usize() {
                let digest_offset = operation
                    .checked_mul(DIGEST_LEN)
                    .and_then(|offset| u32::try_from(offset).ok())
                    .ok_or(failure)?;
                control.extend_from_slice(&[
                    u32::from(pending.program.profile_operation_code()),
                    0,
                    0,
                    u32::from(encode_digest_algorithm(
                        pending.operation.options_digest().algorithm(),
                    )),
                    digest_offset,
                    u32::try_from(DIGEST_LEN).map_err(|_| failure)?,
                    0,
                    0,
                ])?;
            }
            Ok(())
        })
        .map(CanonicalBlockRef::from_store)?;
    let provenance = lane
        .try_retain_sequence(plan.provenance, ProfileLineageProvenanceSchema, |control| {
            let mut local = 0_u32;
            let mut parent_offset = 0_u32;
            for row in 0..owners.rows() {
                control.poll()?;
                if !selected(row)? {
                    continue;
                }
                let parents = source_range(row)?;
                let mut maximum_depth = 0_u32;
                let mut count = 0_u32;
                for parent in parents.iter() {
                    let (_, depth) = resolve_qualified_depth(
                        pending,
                        &segments,
                        parent.ok_or(failure)?,
                        control,
                        failure,
                    )?;
                    maximum_depth = maximum_depth.max(depth);
                    count = count.checked_add(1).ok_or(failure)?;
                }
                control.extend_from_slice(&[
                    u32::from(PROFILE_PROVENANCE_OPERATION),
                    local,
                    maximum_depth.checked_add(1).ok_or(failure)?,
                    parent_offset,
                    count,
                ])?;
                local = local.checked_add(1).ok_or(failure)?;
                parent_offset = parent_offset.checked_add(count).ok_or(failure)?;
            }
            if local != plan.target_count.as_u32()
                || parent_offset != plan.target_attachment_count.as_u32()
            {
                return Err(failure.into());
            }
            Ok(())
        })
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

pub(super) fn rewrite_profile_records<'lane, 'scene, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'scene>,
    segments: CanonicalBlockRef<ProfileSegmentTableSchema>,
    leaves: RewrittenProfileLineageLeaves,
    mut metadata_leaves: MetadataLeafRefs,
) -> Result<RetainedProfileRefs, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
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
    let owners = lane
        .try_rewrite_records(
            &input.canonical,
            input.proof.profile_owners().into_store(),
            pending.counts.profile.owners,
            ProfileOwnerTableSchema,
            |control| {
                let mut next_offset = 0_u32;
                for row_index in 0..source_owners.rows() {
                    let owner_kind = source_owners
                        .scalar::<u8>(row_index, profile_owner_field::OWNER_KIND)
                        .ok_or(failure)?;
                    let owner_index = source_owners
                        .scalar::<u32>(row_index, profile_owner_field::OWNER_INDEX)
                        .ok_or(failure)?;
                    let source_count = source_owners
                        .scalar::<u32>(row_index, profile_owner_field::ATTACHMENT_COUNT)
                        .ok_or(failure)?;
                    let attachment_offset = next_offset;
                    let attachment_count =
                        if pending
                            .counts
                            .profile
                            .selects(input, owner_kind, owner_index)
                        {
                            1
                        } else {
                            source_count
                        };
                    next_offset = next_offset.checked_add(attachment_count).ok_or(failure)?;
                    control.write_row(|row| {
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

    let entries = super::rewrite_optional_record(
        lane,
        input,
        input.proof.profile_metadata_entries(),
        usize::try_from(pending.counts.evidence.profile_metadata_entry_count)
            .map_err(|_| StoreRuntimeError::AccountingOverflow)?,
        ProfileMetadataEntryTableSchema,
    )?;
    if metadata_leaves.entries.is_some() {
        return Err(StoreRuntimeError::InvalidSourceSlot);
    }
    metadata_leaves.entries = entries;
    let metadata_directory = super::rewrite_required_record(
        lane,
        input,
        input.proof.profile_metadata_directory(),
        1,
        ProfileMetadataDirectorySchema,
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
    let directory = lane
        .try_rewrite_records(
            &input.canonical,
            input.proof.profile_directory().into_store(),
            baozi_operation::RecordPlan::new(1, PROFILE_DIRECTORY_FIELD_COUNT)
                .map_err(StoreRuntimeError::Plan)?,
            ProfileDirectorySchema,
            |control| {
                control.write_row(|row| {
                    row.write_scalar(PROFILE_SCHEMA_REVISION)?;
                    row.write_scalar(pending.counts.profile.output_segment_count.as_u32())?;
                    row.write_record_ref(segments.into_store())?;
                    row.write_scalar(pending.counts.evidence.profile_owner_count)?;
                    row.write_record_ref(owners.into_store())?;
                    row.write_record_ref(metadata_directory.into_store())
                })
            },
        )
        .map(CanonicalBlockRef::from_store)?;

    Ok(RetainedProfileRefs {
        directory,
        segments,
        owners,
        lineage: ProfileLineageMaterializedRefs::MappedPrefixAndAppend {
            prefix_segment_count: pending.counts.profile.source_segment_count.as_u32(),
            appended: ProfileLeafRefs {
                resources: None,
                sources: None,
                provenance: Some(leaves.provenance),
                operations: Some(leaves.operation),
                evidence: None,
                parents: Some(leaves.parents),
                wide: None,
                text: None,
                digests: Some(leaves.digest),
                attachments: Some(leaves.attachments),
            },
        },
        metadata: RetainedMetadataRefs {
            directory: metadata_directory,
            leaves: metadata_leaves,
        },
    })
}

pub(super) fn rewrite_segments<'lane, 'scene, O, PP, U>(
    lane: &mut CanonicalLane<'lane, O, PP, U, CanonicalInheritanceSource>,
    pending: &BoundPendingNodeSceneEdit<'scene>,
    leaves: RewrittenProfileLineageLeaves,
) -> Result<CanonicalBlockRef<ProfileSegmentTableSchema>, StoreRuntimeError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let failure = StoreBuildError::domain(NODE_SCENE_BUILD_INVARIANT_CODE)
        .map_err(StoreRuntimeError::Plan)?;
    let source_plan = baozi_operation::RecordPlan::new(
        pending.counts.profile.source_segment_count.as_usize(),
        PROFILE_SEGMENT_FIELD_COUNT,
    )
    .map_err(StoreRuntimeError::Plan)?;
    lane.try_copy_mapped_records_with_append(
        &pending.input.canonical,
        pending.input.proof.profile_segments().into_store(),
        source_plan,
        pending.counts.profile.segments,
        ProfileSegmentTableSchema,
        |control| {
            control.write_row(|row| {
                row.write_scalar(pending.counts.profile.source_segment_count.as_u32())?;
                row.write_scalar(0_u32)?;
                row.write_scalar(PROFILE_ABSENT_REFERENCE)?;
                row.write_scalar(0_u32)?;
                row.write_scalar(PROFILE_ABSENT_REFERENCE)?;
                row.write_scalar(pending.counts.profile.target_count.as_u32())?;
                row.write_sequence_ref(leaves.provenance.into_store())?;
                row.write_scalar(pending.counts.profile.target_count.as_u32())?;
                row.write_sequence_ref(leaves.operation.into_store())?;
                row.write_scalar(0_u32)?;
                row.write_scalar(PROFILE_ABSENT_REFERENCE)?;
                row.write_scalar(pending.counts.profile.target_attachment_count.as_u32())?;
                row.write_sequence_ref(leaves.parents.into_store())?;
                row.write_scalar(0_u32)?;
                row.write_scalar(PROFILE_ABSENT_REFERENCE)?;
                row.write_scalar(0_u32)?;
                row.write_scalar(PROFILE_ABSENT_REFERENCE)?;
                row.write_scalar(
                    u32::try_from(pending.counts.profile.digest.bytes()).map_err(|_| failure)?,
                )?;
                row.write_bytes_ref(leaves.digest.into_store())?;
                row.write_scalar(leaves.segment_nonce.get())?;
                row.write_scalar(pending.counts.profile.fresh_provenance_index.as_u32())
            })
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

fn resolve_qualified_depth<O, PP, U, W>(
    pending: &BoundPendingNodeSceneEdit<'_>,
    segments: &CanonicalRecordView<'_>,
    address: QualifiedProvenanceAddress,
    control: &mut CanonicalBuildControl<'_, '_, '_, O, PP, U, W>,
    failure: StoreBuildError,
) -> Result<(u32, u32), StoreControlledError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let segment_ordinal = address.segment_ordinal().ok_or(failure)?;
    let segment_row = usize::try_from(segment_ordinal).map_err(|_| failure)?;
    if segment_row >= pending.counts.profile.source_segment_count.as_usize() {
        return Err(failure.into());
    }
    control.poll()?;
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
    control.poll()?;
    let depth = *provenance.as_slice().get(scalar).ok_or(failure)?;
    Ok((global, depth))
}

const fn encode_digest_algorithm(algorithm: DigestAlgorithm) -> u8 {
    match algorithm {
        DigestAlgorithm::Sha256 => PROFILE_DIGEST_SHA256,
        DigestAlgorithm::Blake3 => PROFILE_DIGEST_BLAKE3,
    }
}
