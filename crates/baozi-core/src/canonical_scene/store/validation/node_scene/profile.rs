use baozi_operation::__retained_store_integration::activation::{
    CanonicalValidationMember, CanonicalValidationSession,
};
use baozi_operation::{
    ByteBlock, CanonicalRecordRef, CanonicalRecordView, OperationOwner, ProgressPhase,
    ProgressUnit, RecordBlock, ScalarSequence, SequenceView, StoreValidationSession,
    StoreWorkError, StoredScalar, Utf8Block,
};

use crate::SCENE_VALIDATION_POLL_INTERVAL;
use crate::canonical_scene::store::construction::node_scene::{
    ProfileLeafRefs, ProfileLineageMaterializedRefs,
};
use crate::canonical_scene::store::production::{
    CanonicalProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ResourceIdentityStability,
    SessionOpaqueReason, ValidatedProfileBatch,
};
use crate::canonical_scene::store::profile_provenance::{
    PROFILE_PROVENANCE_ADDRESS_SCALARS, ProfileSegmentAddressDomain, ProfileSegmentNonce,
    QualifiedProvenanceAddress, QualifiedProvenanceAddressSlice,
};
use crate::canonical_scene::store::schema::profile::*;
use crate::canonical_scene::store::schema::{CanonicalBlockRef, CanonicalSchema};
use crate::extension::{ExtensionArrayRef, ExtensionValueRef};
use crate::provenance::{
    DigestAlgorithm, MAX_OPERATION_EVIDENCE_SUMMARIES, OperationKind, SourceObjectKind,
    SourceObjectLocator,
};

const DIGEST_BYTES: u32 = 32;
type OptionalCanonicalSequence<'storage, T, Tag> = (
    Option<CanonicalBlockRef<Tag>>,
    Option<SequenceView<'storage, T>>,
);
type OptionalCanonicalValue<'storage, Tag, Value> =
    (Option<CanonicalBlockRef<Tag>>, Option<&'storage Value>);

mod metadata;
mod storage;
#[cfg(test)]
mod tests;

use metadata::validate_metadata;
use storage::*;

/// Caller-owned scratch used to prove cross-segment provenance depth in linear time.
pub(in crate::canonical_scene::store) struct ProfileValidationScratch<'scratch> {
    provenance_depths: &'scratch mut [u32],
}

impl<'scratch> ProfileValidationScratch<'scratch> {
    pub(in crate::canonical_scene::store) const fn new(
        provenance_depths: &'scratch mut [u32],
    ) -> Self {
        Self { provenance_depths }
    }

    pub(in crate::canonical_scene::store) fn required_provenance_depths(
        provenance_count: u32,
    ) -> Option<usize> {
        usize::try_from(provenance_count).ok()
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct ProfileOwnerClassCounts {
    pub(super) nodes: u32,
    pub(super) cameras: u32,
    pub(super) lights: u32,
    pub(super) images: u32,
    pub(super) samplers: u32,
    pub(super) textures: u32,
    pub(super) texture_bindings: u32,
    pub(super) materials: u32,
    pub(super) logical_meshes: u32,
    pub(super) primitives: u32,
    pub(super) attributes: u32,
    pub(super) mesh_instances: u32,
    pub(super) skins: u32,
    pub(super) morph_definitions: u32,
    pub(super) morph_deltas: u32,
    pub(super) animation_clips: u32,
    pub(super) animation_channels: u32,
}

/// Typed projection retained after central profile validation.
///
/// Segment-local leaf references remain in the validated segment table. This keeps the
/// projection fixed-size while preserving every typed reference required by edit rewrite,
/// semantic inspection, and snapshot traversal.
#[derive(Clone, Copy)]
pub(super) struct ProfileValidatedRefs {
    pub(super) directory: CanonicalBlockRef<ProfileDirectorySchema>,
    pub(super) segments: CanonicalBlockRef<ProfileSegmentTableSchema>,
    pub(super) owners: CanonicalBlockRef<ProfileOwnerTableSchema>,
    pub(super) attachments: CanonicalBlockRef<ProfileLineageAttachmentSchema>,
    pub(super) metadata_directory: CanonicalBlockRef<ProfileMetadataDirectorySchema>,
    pub(super) metadata_entries: Option<CanonicalBlockRef<ProfileMetadataEntryTableSchema>>,
    pub(super) metadata_text: Option<CanonicalBlockRef<ProfileMetadataTextSchema>>,
    pub(super) metadata_bytes: Option<CanonicalBlockRef<ProfileMetadataBytesSchema>>,
    pub(super) metadata_bools: Option<CanonicalBlockRef<ProfileMetadataBoolSchema>>,
    pub(super) metadata_i64s: Option<CanonicalBlockRef<ProfileMetadataI64Schema>>,
    pub(super) metadata_u64s: Option<CanonicalBlockRef<ProfileMetadataU64Schema>>,
    pub(super) metadata_f64s: Option<CanonicalBlockRef<ProfileMetadataF64Schema>>,
    pub(super) metadata_f32s: Option<CanonicalBlockRef<ProfileMetadataF32Schema>>,
    pub(super) segment_count: u32,
    pub(super) resource_count: u32,
    pub(super) source_count: u32,
    pub(super) provenance_count: u32,
    pub(super) operation_count: u32,
    pub(super) evidence_count: u32,
    pub(super) parent_count: u32,
    pub(super) wide_count: u32,
    pub(super) lineage_text_bytes: u32,
    pub(super) lineage_digest_bytes: u32,
    pub(super) attachment_count: u32,
    pub(super) owner_count: u32,
    pub(super) metadata_entry_count: u32,
    pub(super) metadata_text_bytes: u32,
    pub(super) metadata_payload_bytes: u32,
    pub(super) metadata_bool_count: u32,
    pub(super) metadata_i64_count: u32,
    pub(super) metadata_u64_count: u32,
    pub(super) metadata_f64_count: u32,
    pub(super) metadata_f32_count: u32,
    pub(super) canonical_block_count: u32,
    pub(super) payload_leaf_count: u32,
    pub(super) shareable_leaf_count: u32,
    pub(super) maximum_provenance_depth: u32,
    pub(super) owner_classes: ProfileOwnerClassCounts,
}

/// Validation-scoped typed members retained after metered fresh-profile validation.
///
/// These members can only be paired with the materialization plan while the same central
/// validation session remains active. Durable store references are minted by the caller after
/// every member has been paired with its independently retained output slot.
#[derive(Clone, Copy)]
pub(super) struct ProfileValidatedMembers<'session> {
    pub(super) directory: CanonicalValidationMember<'session, RecordBlock, ProfileDirectorySchema>,
    pub(super) segments:
        CanonicalValidationMember<'session, RecordBlock, ProfileSegmentTableSchema>,
    pub(super) owners: CanonicalValidationMember<'session, RecordBlock, ProfileOwnerTableSchema>,
    pub(super) attachments:
        CanonicalValidationMember<'session, ScalarSequence<u64>, ProfileLineageAttachmentSchema>,
    pub(super) metadata_directory:
        CanonicalValidationMember<'session, RecordBlock, ProfileMetadataDirectorySchema>,
    pub(super) metadata_entries:
        Option<CanonicalValidationMember<'session, RecordBlock, ProfileMetadataEntryTableSchema>>,
    pub(super) metadata_text:
        Option<CanonicalValidationMember<'session, Utf8Block, ProfileMetadataTextSchema>>,
    pub(super) metadata_bytes:
        Option<CanonicalValidationMember<'session, ByteBlock, ProfileMetadataBytesSchema>>,
    pub(super) metadata_bools:
        Option<CanonicalValidationMember<'session, ScalarSequence<u8>, ProfileMetadataBoolSchema>>,
    pub(super) metadata_i64s:
        Option<CanonicalValidationMember<'session, ScalarSequence<i64>, ProfileMetadataI64Schema>>,
    pub(super) metadata_u64s:
        Option<CanonicalValidationMember<'session, ScalarSequence<u64>, ProfileMetadataU64Schema>>,
    pub(super) metadata_f64s:
        Option<CanonicalValidationMember<'session, ScalarSequence<f64>, ProfileMetadataF64Schema>>,
    pub(super) metadata_f32s:
        Option<CanonicalValidationMember<'session, ScalarSequence<f32>, ProfileMetadataF32Schema>>,
    pub(super) segment_count: u32,
    pub(super) resource_count: u32,
    pub(super) source_count: u32,
    pub(super) provenance_count: u32,
    pub(super) operation_count: u32,
    pub(super) evidence_count: u32,
    pub(super) parent_count: u32,
    pub(super) wide_count: u32,
    pub(super) lineage_text_bytes: u32,
    pub(super) lineage_digest_bytes: u32,
    pub(super) attachment_count: u32,
    pub(super) owner_count: u32,
    pub(super) metadata_entry_count: u32,
    pub(super) metadata_text_bytes: u32,
    pub(super) metadata_payload_bytes: u32,
    pub(super) metadata_bool_count: u32,
    pub(super) metadata_i64_count: u32,
    pub(super) metadata_u64_count: u32,
    pub(super) metadata_f64_count: u32,
    pub(super) metadata_f32_count: u32,
    pub(super) canonical_block_count: u32,
    pub(super) payload_leaf_count: u32,
    pub(super) shareable_leaf_count: u32,
    pub(super) maximum_provenance_depth: u32,
    pub(super) owner_classes: ProfileOwnerClassCounts,
}

/// Shape-only authority for one fresh canonical profile validation.
///
/// The descriptor contains no source values. It is derived from the sealed materialization plan
/// and exists solely to bind scratch sizes and the exact central-validation work envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::canonical_scene::store) struct ProfileMeteredCounts {
    pub(in crate::canonical_scene::store) segment_count: u32,
    pub(in crate::canonical_scene::store) resource_count: u32,
    pub(in crate::canonical_scene::store) source_count: u32,
    pub(in crate::canonical_scene::store) provenance_count: u32,
    pub(in crate::canonical_scene::store) operation_count: u32,
    pub(in crate::canonical_scene::store) evidence_count: u32,
    pub(in crate::canonical_scene::store) parent_count: u32,
    pub(in crate::canonical_scene::store) wide_count: u32,
    pub(in crate::canonical_scene::store) lineage_text_bytes: u32,
    pub(in crate::canonical_scene::store) lineage_digest_bytes: u32,
    pub(in crate::canonical_scene::store) owner_count: u32,
    pub(in crate::canonical_scene::store) attachment_count: u32,
    pub(in crate::canonical_scene::store) metadata_entry_count: u32,
    pub(in crate::canonical_scene::store) metadata_text_bytes: u32,
    pub(in crate::canonical_scene::store) metadata_payload_bytes: u32,
    pub(in crate::canonical_scene::store) metadata_bool_count: u32,
    pub(in crate::canonical_scene::store) metadata_i64_count: u32,
    pub(in crate::canonical_scene::store) metadata_u64_count: u32,
    pub(in crate::canonical_scene::store) metadata_f64_count: u32,
    pub(in crate::canonical_scene::store) metadata_f32_count: u32,
    pub(in crate::canonical_scene::store) canonical_block_count: u32,
    pub(in crate::canonical_scene::store) payload_leaf_count: u32,
    pub(in crate::canonical_scene::store) shareable_leaf_count: u32,
}

impl ProfileMeteredCounts {
    pub(in crate::canonical_scene::store) fn scratch_bytes(self) -> Option<u64> {
        let u32_bytes = u64::try_from(std::mem::size_of::<u32>()).ok()?;
        let u64_bytes = u64::try_from(std::mem::size_of::<u64>()).ok()?;
        let i64_bytes = u64::try_from(std::mem::size_of::<i64>()).ok()?;
        let f64_bytes = u64::try_from(std::mem::size_of::<f64>()).ok()?;
        let f32_bytes = u64::try_from(std::mem::size_of::<f32>()).ok()?;
        let address_scalars = u64::try_from(PROFILE_PROVENANCE_ADDRESS_SCALARS).ok()?;
        [
            u64::from(self.segment_count).checked_mul(
                u64::try_from(std::mem::size_of::<ProfileSegmentValidationScratch<'static>>())
                    .ok()?,
            )?,
            u64::from(self.resource_count)
                .checked_mul(u64::try_from(PROFILE_RESOURCE_SCALAR_COUNT).ok()?)?
                .checked_mul(u32_bytes)?,
            u64::from(self.source_count)
                .checked_mul(u64::try_from(PROFILE_SOURCE_SCALAR_COUNT).ok()?)?
                .checked_mul(u32_bytes)?,
            u64::from(self.provenance_count)
                .checked_mul(u64::try_from(PROFILE_PROVENANCE_SCALAR_COUNT).ok()?)?
                .checked_mul(u32_bytes)?,
            u64::from(self.operation_count)
                .checked_mul(u64::try_from(PROFILE_OPERATION_SCALAR_COUNT).ok()?)?
                .checked_mul(u32_bytes)?,
            u64::from(self.evidence_count)
                .checked_mul(u64::try_from(PROFILE_EVIDENCE_SCALAR_COUNT).ok()?)?
                .checked_mul(u32_bytes)?,
            u64::from(self.parent_count)
                .checked_mul(address_scalars)?
                .checked_mul(u64_bytes)?,
            u64::from(self.wide_count).checked_mul(u64_bytes)?,
            u64::from(self.lineage_text_bytes),
            u64::from(self.lineage_digest_bytes),
            u64::from(self.provenance_count).checked_mul(u32_bytes)?,
            u64::from(self.owner_count)
                .checked_mul(4)?
                .checked_mul(u32_bytes)?,
            u64::from(self.attachment_count)
                .checked_mul(address_scalars)?
                .checked_mul(u64_bytes)?,
            u64::from(self.metadata_entry_count)
                .checked_mul(13)?
                .checked_mul(u32_bytes)?,
            u64::from(self.metadata_text_bytes),
            u64::from(self.metadata_payload_bytes),
            u64::from(self.metadata_bool_count),
            u64::from(self.metadata_i64_count).checked_mul(i64_bytes)?,
            u64::from(self.metadata_u64_count).checked_mul(u64_bytes)?,
            u64::from(self.metadata_f64_count).checked_mul(f64_bytes)?,
            u64::from(self.metadata_f32_count).checked_mul(f32_bytes)?,
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
    }

    pub(in crate::canonical_scene::store) fn member_read_work(self) -> Option<u64> {
        let address_scalars = u64::try_from(PROFILE_PROVENANCE_ADDRESS_SCALARS).ok()?;
        [
            u64::try_from(PROFILE_DIRECTORY_FIELD_COUNT).ok()?,
            u64::from(self.segment_count)
                .checked_mul(u64::try_from(PROFILE_SEGMENT_FIELD_COUNT).ok()?)?,
            u64::from(self.resource_count)
                .checked_mul(u64::try_from(PROFILE_RESOURCE_SCALAR_COUNT).ok()?)?,
            u64::from(self.source_count)
                .checked_mul(u64::try_from(PROFILE_SOURCE_SCALAR_COUNT).ok()?)?,
            u64::from(self.provenance_count)
                .checked_mul(u64::try_from(PROFILE_PROVENANCE_SCALAR_COUNT).ok()?)?,
            u64::from(self.operation_count)
                .checked_mul(u64::try_from(PROFILE_OPERATION_SCALAR_COUNT).ok()?)?,
            u64::from(self.evidence_count)
                .checked_mul(u64::try_from(PROFILE_EVIDENCE_SCALAR_COUNT).ok()?)?,
            u64::from(self.parent_count).checked_mul(address_scalars)?,
            u64::from(self.wide_count),
            u64::from(self.lineage_text_bytes),
            u64::from(self.lineage_digest_bytes),
            u64::from(self.owner_count)
                .checked_mul(u64::try_from(PROFILE_OWNER_FIELD_COUNT).ok()?)?,
            u64::from(self.attachment_count).checked_mul(address_scalars)?,
            u64::try_from(PROFILE_METADATA_DIRECTORY_FIELD_COUNT).ok()?,
            u64::from(self.metadata_entry_count)
                .checked_mul(u64::try_from(PROFILE_METADATA_ENTRY_FIELD_COUNT).ok()?)?,
            u64::from(self.metadata_text_bytes),
            u64::from(self.metadata_payload_bytes),
            u64::from(self.metadata_bool_count),
            u64::from(self.metadata_i64_count),
            u64::from(self.metadata_u64_count),
            u64::from(self.metadata_f64_count),
            u64::from(self.metadata_f32_count),
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
    }

    pub(in crate::canonical_scene::store) fn scratch_initialization_work(self) -> Option<u64> {
        let address_scalars = u64::try_from(PROFILE_PROVENANCE_ADDRESS_SCALARS).ok()?;
        [
            u64::from(self.segment_count),
            u64::from(self.resource_count)
                .checked_mul(u64::try_from(PROFILE_RESOURCE_SCALAR_COUNT).ok()?)?,
            u64::from(self.source_count)
                .checked_mul(u64::try_from(PROFILE_SOURCE_SCALAR_COUNT).ok()?)?,
            u64::from(self.provenance_count)
                .checked_mul(u64::try_from(PROFILE_PROVENANCE_SCALAR_COUNT).ok()?)?,
            u64::from(self.operation_count)
                .checked_mul(u64::try_from(PROFILE_OPERATION_SCALAR_COUNT).ok()?)?,
            u64::from(self.evidence_count)
                .checked_mul(u64::try_from(PROFILE_EVIDENCE_SCALAR_COUNT).ok()?)?,
            u64::from(self.parent_count).checked_mul(address_scalars)?,
            u64::from(self.wide_count),
            u64::from(self.lineage_text_bytes),
            u64::from(self.lineage_digest_bytes),
            u64::from(self.provenance_count),
            u64::from(self.owner_count).checked_mul(4)?,
            u64::from(self.attachment_count).checked_mul(address_scalars)?,
            u64::from(self.metadata_entry_count).checked_mul(13)?,
            u64::from(self.metadata_text_bytes),
            u64::from(self.metadata_payload_bytes),
            u64::from(self.metadata_bool_count),
            u64::from(self.metadata_i64_count),
            u64::from(self.metadata_u64_count),
            u64::from(self.metadata_f64_count),
            u64::from(self.metadata_f32_count),
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
    }

    pub(in crate::canonical_scene::store) fn semantic_work(self) -> Option<u64> {
        [
            u64::from(self.resource_count),
            u64::from(self.source_count),
            u64::from(self.provenance_count).checked_mul(2)?,
            u64::from(self.operation_count).checked_mul(2)?,
            u64::from(self.evidence_count),
            u64::from(self.parent_count),
            u64::from(self.owner_count),
            u64::from(self.attachment_count),
            u64::from(self.metadata_entry_count),
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
    }

    pub(in crate::canonical_scene::store) fn validation_work(self) -> Option<u64> {
        self.scratch_initialization_work()?
            .checked_add(self.member_read_work()?)?
            .checked_add(self.semantic_work()?)
    }
}

/// Reservation-backed projection used while canonical profile members are visited exactly once.
pub(in crate::canonical_scene::store) struct ProfileMeteredScratch<'scratch, 'session> {
    pub(in crate::canonical_scene::store) segments:
        &'scratch mut [ProfileSegmentValidationScratch<'session>],
    pub(in crate::canonical_scene::store) resources: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) sources: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) provenance: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) operations: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) evidence: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) parents: &'scratch mut [u64],
    pub(in crate::canonical_scene::store) wide: &'scratch mut [u64],
    pub(in crate::canonical_scene::store) lineage_text: &'scratch mut [u8],
    pub(in crate::canonical_scene::store) lineage_digests: &'scratch mut [u8],
    pub(in crate::canonical_scene::store) provenance_depths: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) owner_rows: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) attachments: &'scratch mut [u64],
    pub(in crate::canonical_scene::store) metadata_rows: &'scratch mut [u32],
    pub(in crate::canonical_scene::store) metadata_text: &'scratch mut [u8],
    pub(in crate::canonical_scene::store) metadata_bytes: &'scratch mut [u8],
    pub(in crate::canonical_scene::store) metadata_bools: &'scratch mut [u8],
    pub(in crate::canonical_scene::store) metadata_i64s: &'scratch mut [i64],
    pub(in crate::canonical_scene::store) metadata_u64s: &'scratch mut [u64],
    pub(in crate::canonical_scene::store) metadata_f64s: &'scratch mut [f64],
    pub(in crate::canonical_scene::store) metadata_f32s: &'scratch mut [f32],
}

pub(super) fn validate_profile_graph<R>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<ProfileDirectorySchema>,
    expected: &ValidatedProfileBatch<'_, R>,
    scratch: &mut ProfileValidationScratch<'_>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<ProfileValidatedRefs> {
    validate_profile_structure(
        session,
        directory_ref,
        Some(expected),
        scratch,
        observe_work,
    )
}

pub(super) fn validate_profile_canonical_structure(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<ProfileDirectorySchema>,
    scratch: &mut ProfileValidationScratch<'_>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<ProfileValidatedRefs> {
    validate_profile_structure::<()>(session, directory_ref, None, scratch, observe_work)
}

#[derive(Clone, Copy, Default)]
struct ProfileTotals {
    resources: u32,
    sources: u32,
    provenance: u32,
    operations: u32,
    evidence: u32,
    parents: u32,
    wide: u32,
    text: u32,
    digests: u32,
    shareable_leaves: u32,
    payload_leaves: u32,
}

#[derive(Clone, Copy, Default)]
struct SegmentCounts {
    resources: u32,
    sources: u32,
    provenance: u32,
    operations: u32,
    evidence: u32,
    parents: u32,
    wide: u32,
    text: u32,
    digests: u32,
}

/// Reservation-backed descriptor for one typed profile segment.
///
/// The descriptor keeps each segment's address domain and typed members adjacent to the
/// aggregate scratch ranges populated from those members. It is initialized inside the exact
/// validation permit and cannot outlive that validation session.
#[derive(Clone, Copy)]
pub(in crate::canonical_scene::store) struct ProfileSegmentValidationScratch<'session> {
    counts: SegmentCounts,
    prefixes: ProfileTotals,
    domain: Option<ProfileSegmentAddressDomain>,
    resources: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, ProfileLineageResourceSchema>,
    >,
    sources: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, ProfileLineageSourceSchema>,
    >,
    provenance: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, ProfileLineageProvenanceSchema>,
    >,
    operations: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, ProfileLineageOperationSchema>,
    >,
    evidence: Option<
        CanonicalValidationMember<'session, ScalarSequence<u32>, ProfileLineageEvidenceSchema>,
    >,
    parents: Option<
        CanonicalValidationMember<'session, ScalarSequence<u64>, ProfileLineageParentSchema>,
    >,
    wide:
        Option<CanonicalValidationMember<'session, ScalarSequence<u64>, ProfileLineageWideSchema>>,
    text: Option<CanonicalValidationMember<'session, Utf8Block, ProfileLineageTextSchema>>,
    digests: Option<CanonicalValidationMember<'session, ByteBlock, ProfileLineageDigestSchema>>,
}

impl ProfileSegmentValidationScratch<'static> {
    pub(in crate::canonical_scene::store) const EMPTY: Self = Self {
        counts: SegmentCounts {
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
        prefixes: ProfileTotals {
            resources: 0,
            sources: 0,
            provenance: 0,
            operations: 0,
            evidence: 0,
            parents: 0,
            wide: 0,
            text: 0,
            digests: 0,
            shareable_leaves: 0,
            payload_leaves: 0,
        },
        domain: None,
        resources: None,
        sources: None,
        provenance: None,
        operations: None,
        evidence: None,
        parents: None,
        wide: None,
        text: None,
        digests: None,
    };
}

impl SegmentCounts {
    const fn is_empty(self) -> bool {
        self.resources == 0
            && self.sources == 0
            && self.provenance == 0
            && self.operations == 0
            && self.evidence == 0
            && self.parents == 0
            && self.wide == 0
            && self.text == 0
            && self.digests == 0
    }
}

impl ProfileTotals {
    fn append(&mut self, segment: SegmentCounts) -> Option<()> {
        self.resources = self.resources.checked_add(segment.resources)?;
        self.sources = self.sources.checked_add(segment.sources)?;
        self.provenance = self.provenance.checked_add(segment.provenance)?;
        self.operations = self.operations.checked_add(segment.operations)?;
        self.evidence = self.evidence.checked_add(segment.evidence)?;
        self.parents = self.parents.checked_add(segment.parents)?;
        self.wide = self.wide.checked_add(segment.wide)?;
        self.text = self.text.checked_add(segment.text)?;
        self.digests = self.digests.checked_add(segment.digests)?;
        self.shareable_leaves = self.shareable_leaves.checked_add(
            u32::try_from(
                [
                    segment.resources,
                    segment.sources,
                    segment.provenance,
                    segment.operations,
                    segment.evidence,
                    segment.parents,
                    segment.wide,
                    segment.text,
                    segment.digests,
                ]
                .into_iter()
                .filter(|count| *count != 0)
                .count(),
            )
            .ok()?,
        )?;
        self.payload_leaves = self
            .payload_leaves
            .checked_add(u32::from(segment.text != 0))?;
        self.payload_leaves = self
            .payload_leaves
            .checked_add(u32::from(segment.digests != 0))?;
        Some(())
    }
}

fn metered_physical_len(count: u32, stride: usize) -> Option<usize> {
    usize::try_from(count).ok()?.checked_mul(stride)
}

fn metered_scratch_matches(
    counts: ProfileMeteredCounts,
    scratch: &ProfileMeteredScratch<'_, '_>,
) -> bool {
    let address_stride = PROFILE_PROVENANCE_ADDRESS_SCALARS;
    scratch.segments.len() == usize::try_from(counts.segment_count).unwrap_or(usize::MAX)
        && scratch.resources.len()
            == metered_physical_len(counts.resource_count, PROFILE_RESOURCE_SCALAR_COUNT)
                .unwrap_or(usize::MAX)
        && scratch.sources.len()
            == metered_physical_len(counts.source_count, PROFILE_SOURCE_SCALAR_COUNT)
                .unwrap_or(usize::MAX)
        && scratch.provenance.len()
            == metered_physical_len(counts.provenance_count, PROFILE_PROVENANCE_SCALAR_COUNT)
                .unwrap_or(usize::MAX)
        && scratch.operations.len()
            == metered_physical_len(counts.operation_count, PROFILE_OPERATION_SCALAR_COUNT)
                .unwrap_or(usize::MAX)
        && scratch.evidence.len()
            == metered_physical_len(counts.evidence_count, PROFILE_EVIDENCE_SCALAR_COUNT)
                .unwrap_or(usize::MAX)
        && scratch.parents.len()
            == metered_physical_len(counts.parent_count, address_stride).unwrap_or(usize::MAX)
        && scratch.wide.len() == usize::try_from(counts.wide_count).unwrap_or(usize::MAX)
        && scratch.lineage_text.len()
            == usize::try_from(counts.lineage_text_bytes).unwrap_or(usize::MAX)
        && scratch.lineage_digests.len()
            == usize::try_from(counts.lineage_digest_bytes).unwrap_or(usize::MAX)
        && scratch.provenance_depths.len()
            == usize::try_from(counts.provenance_count).unwrap_or(usize::MAX)
        && scratch.owner_rows.len()
            == metered_physical_len(counts.owner_count, 4).unwrap_or(usize::MAX)
        && scratch.attachments.len()
            == metered_physical_len(counts.attachment_count, address_stride).unwrap_or(usize::MAX)
        && scratch.metadata_rows.len()
            == metered_physical_len(counts.metadata_entry_count, 13).unwrap_or(usize::MAX)
        && scratch.metadata_text.len()
            == usize::try_from(counts.metadata_text_bytes).unwrap_or(usize::MAX)
        && scratch.metadata_bytes.len()
            == usize::try_from(counts.metadata_payload_bytes).unwrap_or(usize::MAX)
        && scratch.metadata_bools.len()
            == usize::try_from(counts.metadata_bool_count).unwrap_or(usize::MAX)
        && scratch.metadata_i64s.len()
            == usize::try_from(counts.metadata_i64_count).unwrap_or(usize::MAX)
        && scratch.metadata_u64s.len()
            == usize::try_from(counts.metadata_u64_count).unwrap_or(usize::MAX)
        && scratch.metadata_f64s.len()
            == usize::try_from(counts.metadata_f64_count).unwrap_or(usize::MAX)
        && scratch.metadata_f32s.len()
            == usize::try_from(counts.metadata_f32_count).unwrap_or(usize::MAX)
}

fn visit_sequence_into<'session, 'storage, 'control, 'operation, O, PP, U, T, Tag>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<CanonicalValidationMember<'session, ScalarSequence<T>, Tag>>,
    output: &mut [T],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    T: StoredScalar,
    Tag: Copy + Send + Sync + 'static,
{
    if output.is_empty() {
        return Ok(member.is_none());
    }
    let Some(member) = member else {
        return Ok(false);
    };
    session.visit_canonical_sequence(member, output.len(), |index, value| {
        let Some(destination) = output.get_mut(index) else {
            return false;
        };
        *destination = value;
        true
    })
}

fn visit_sequence_into_validated<'session, 'storage, 'control, 'operation, O, PP, U, T, Tag>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<CanonicalValidationMember<'session, ScalarSequence<T>, Tag>>,
    output: &mut [T],
    mut validate: impl FnMut(T) -> bool,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    T: StoredScalar,
    Tag: Copy + Send + Sync + 'static,
{
    if output.is_empty() {
        return Ok(member.is_none());
    }
    let Some(member) = member else {
        return Ok(false);
    };
    session.visit_canonical_sequence(member, output.len(), |index, value| {
        let Some(destination) = output.get_mut(index) else {
            return false;
        };
        *destination = value;
        validate(value)
    })
}

fn visit_bytes_into<'session, 'storage, 'control, 'operation, O, PP, U, Tag>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<CanonicalValidationMember<'session, ByteBlock, Tag>>,
    output: &mut [u8],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: Copy + Send + Sync + 'static,
{
    if output.is_empty() {
        return Ok(member.is_none());
    }
    let Some(member) = member else {
        return Ok(false);
    };
    session.visit_canonical_bytes(member, output.len(), |index, value| {
        let Some(destination) = output.get_mut(index) else {
            return false;
        };
        *destination = value;
        true
    })
}

fn visit_utf8_into<'session, 'storage, 'control, 'operation, O, PP, U, Tag>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    member: Option<CanonicalValidationMember<'session, Utf8Block, Tag>>,
    output: &mut [u8],
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
    Tag: Copy + Send + Sync + 'static,
{
    if output.is_empty() {
        return Ok(member.is_none());
    }
    let Some(member) = member else {
        return Ok(false);
    };
    session.visit_canonical_utf8_bytes(member, output.len(), |index, value| {
        let Some(destination) = output.get_mut(index) else {
            return false;
        };
        *destination = value;
        true
    })
}

#[derive(Clone, Copy)]
struct ProfileTypedMetadataMembers<'session> {
    entries:
        Option<CanonicalValidationMember<'session, RecordBlock, ProfileMetadataEntryTableSchema>>,
    text: Option<CanonicalValidationMember<'session, Utf8Block, ProfileMetadataTextSchema>>,
    bytes: Option<CanonicalValidationMember<'session, ByteBlock, ProfileMetadataBytesSchema>>,
    bools:
        Option<CanonicalValidationMember<'session, ScalarSequence<u8>, ProfileMetadataBoolSchema>>,
    i64s:
        Option<CanonicalValidationMember<'session, ScalarSequence<i64>, ProfileMetadataI64Schema>>,
    u64s:
        Option<CanonicalValidationMember<'session, ScalarSequence<u64>, ProfileMetadataU64Schema>>,
    f64s:
        Option<CanonicalValidationMember<'session, ScalarSequence<f64>, ProfileMetadataF64Schema>>,
    f32s:
        Option<CanonicalValidationMember<'session, ScalarSequence<f32>, ProfileMetadataF32Schema>>,
}

fn metered_metadata_shape(
    value_kind: u8,
    array_kind: u8,
    logical_count: u32,
) -> Option<MetadataShape> {
    let scalar = |lane, scalar_count| MetadataShape {
        lane,
        scalar_count,
        auxiliary_text: false,
        bytes: false,
    };
    Some(match value_kind {
        PROFILE_METADATA_BOOL
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::Bool, 1)
        }
        PROFILE_METADATA_I64 if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 => {
            scalar(MetadataLane::I64, 1)
        }
        PROFILE_METADATA_U64 if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 => {
            scalar(MetadataLane::U64, 1)
        }
        PROFILE_METADATA_F64 if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 => {
            scalar(MetadataLane::F64, 1)
        }
        PROFILE_METADATA_STRING
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            MetadataShape {
                lane: MetadataLane::None,
                scalar_count: 0,
                auxiliary_text: true,
                bytes: false,
            }
        }
        PROFILE_METADATA_BYTES
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            MetadataShape {
                lane: MetadataLane::None,
                scalar_count: 0,
                auxiliary_text: true,
                bytes: true,
            }
        }
        PROFILE_METADATA_VEC2
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::F32, 2)
        }
        PROFILE_METADATA_VEC3
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::F32, 3)
        }
        PROFILE_METADATA_VEC4 | PROFILE_METADATA_COLOR
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::F32, 4)
        }
        PROFILE_METADATA_MAT4
            if array_kind == PROFILE_METADATA_ARRAY_BOOL && logical_count == 1 =>
        {
            scalar(MetadataLane::F32, 16)
        }
        PROFILE_METADATA_DENSE_ARRAY => {
            let (lane, width) = match array_kind {
                PROFILE_METADATA_ARRAY_BOOL => (MetadataLane::Bool, 1),
                PROFILE_METADATA_ARRAY_I64 => (MetadataLane::I64, 1),
                PROFILE_METADATA_ARRAY_U64 => (MetadataLane::U64, 1),
                PROFILE_METADATA_ARRAY_F64 => (MetadataLane::F64, 1),
                PROFILE_METADATA_ARRAY_VEC2 => (MetadataLane::F32, 2),
                PROFILE_METADATA_ARRAY_VEC3 => (MetadataLane::F32, 3),
                PROFILE_METADATA_ARRAY_VEC4 | PROFILE_METADATA_ARRAY_COLOR => {
                    (MetadataLane::F32, 4)
                }
                PROFILE_METADATA_ARRAY_MAT4 => (MetadataLane::F32, 16),
                _ => return None,
            };
            scalar(lane, logical_count.checked_mul(width)?)
        }
        _ => return None,
    })
}

fn optional_lineage_member_identifies<Kind, Tag>(
    member: Option<CanonicalValidationMember<'_, Kind, Tag>>,
    slot: Option<CanonicalBlockRef<Tag>>,
) -> bool
where
    Tag: CanonicalSchema<StoreKind = Kind>,
{
    match (member, slot) {
        (Some(member), Some(slot)) => member.identifies(slot.into_store()),
        (None, None) => true,
        _ => false,
    }
}

fn segment_lineage_identifies(
    segment: ProfileSegmentValidationScratch<'_>,
    expected: ProfileLeafRefs,
) -> bool {
    optional_lineage_member_identifies(segment.resources, expected.resources)
        && optional_lineage_member_identifies(segment.sources, expected.sources)
        && optional_lineage_member_identifies(segment.provenance, expected.provenance)
        && optional_lineage_member_identifies(segment.operations, expected.operations)
        && optional_lineage_member_identifies(segment.evidence, expected.evidence)
        && optional_lineage_member_identifies(segment.parents, expected.parents)
        && optional_lineage_member_identifies(segment.wide, expected.wide)
        && optional_lineage_member_identifies(segment.text, expected.text)
        && optional_lineage_member_identifies(segment.digests, expected.digests)
}

fn materialized_lineage_identifies(
    segments: &[ProfileSegmentValidationScratch<'_>],
    expected: ProfileLineageMaterializedRefs,
) -> bool {
    match expected {
        ProfileLineageMaterializedRefs::ExactSingle(expected) => {
            let [segment] = segments else {
                return false;
            };
            segment_lineage_identifies(*segment, expected)
        }
        ProfileLineageMaterializedRefs::MappedPrefixAndAppend {
            prefix_segment_count,
            appended,
        } => usize::try_from(prefix_segment_count)
            .ok()
            .and_then(|prefix| {
                (segments.len() == prefix.checked_add(1)?)
                    .then(|| segments.get(prefix).copied())
                    .flatten()
            })
            .is_some_and(|segment| segment_lineage_identifies(segment, appended)),
    }
}

#[allow(clippy::too_many_lines)]
fn read_profile_segments_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    segments: CanonicalValidationMember<'session, RecordBlock, ProfileSegmentTableSchema>,
    counts: ProfileMeteredCounts,
    output: &mut [ProfileSegmentValidationScratch<'session>],
) -> Result<Option<ProfileTotals>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mut totals = ProfileTotals::default();
    let mut previous_nonce = None;
    let complete = session.visit_canonical_records(
        segments,
        usize::try_from(counts.segment_count).unwrap_or(usize::MAX),
        PROFILE_SEGMENT_FIELD_COUNT,
        |row_index, row| {
            let ordinal = row.scalar(profile_segment_field::SEGMENT, |value: u32| value)?;
            let resources_count =
                row.scalar(profile_segment_field::RESOURCE_COUNT, |value: u32| value)?;
            let sources_count =
                row.scalar(profile_segment_field::SOURCE_COUNT, |value: u32| value)?;
            let provenance_count =
                row.scalar(profile_segment_field::PROVENANCE_COUNT, |value: u32| value)?;
            let operations_count =
                row.scalar(profile_segment_field::OPERATION_COUNT, |value: u32| value)?;
            let evidence_count =
                row.scalar(profile_segment_field::EVIDENCE_COUNT, |value: u32| value)?;
            let parents_count =
                row.scalar(profile_segment_field::PARENT_COUNT, |value: u32| value)?;
            let wide_count = row.scalar(profile_segment_field::WIDE_COUNT, |value: u32| value)?;
            let text_count = row.scalar(profile_segment_field::TEXT_COUNT, |value: u32| value)?;
            let digest_count =
                row.scalar(profile_segment_field::DIGEST_COUNT, |value: u32| value)?;
            let nonce = row.scalar(profile_segment_field::NONCE, |value: u64| value)?;
            let provenance_base =
                row.scalar(profile_segment_field::PROVENANCE_BASE, |value: u32| value)?;
            let (
                Some(ordinal),
                Some(resources_count),
                Some(sources_count),
                Some(provenance_count),
                Some(operations_count),
                Some(evidence_count),
                Some(parents_count),
                Some(wide_count),
                Some(text_count),
                Some(digest_count),
                Some(nonce),
                Some(provenance_base),
            ) = (
                ordinal,
                resources_count,
                sources_count,
                provenance_count,
                operations_count,
                evidence_count,
                parents_count,
                wide_count,
                text_count,
                digest_count,
                nonce,
                provenance_base,
            )
            else {
                return Ok(false);
            };

            let mut valid = true;
            macro_rules! sequence_member {
                ($count:expr, $field:expr, $scalar:ty, $schema:ty) => {{
                    if $count == 0 {
                        let absent = row.scalar($field, |value: u8| value)?;
                        valid &= absent == Some(PROFILE_ABSENT_REFERENCE);
                        None
                    } else {
                        row.sequence_ref::<$scalar, $schema, _>($field, |value| value)?
                    }
                }};
            }
            macro_rules! utf8_member {
                ($count:expr, $field:expr, $schema:ty) => {{
                    if $count == 0 {
                        let absent = row.scalar($field, |value: u8| value)?;
                        valid &= absent == Some(PROFILE_ABSENT_REFERENCE);
                        None
                    } else {
                        row.utf8_ref::<$schema, _>($field, |value| value)?
                    }
                }};
            }
            macro_rules! bytes_member {
                ($count:expr, $field:expr, $schema:ty) => {{
                    if $count == 0 {
                        let absent = row.scalar($field, |value: u8| value)?;
                        valid &= absent == Some(PROFILE_ABSENT_REFERENCE);
                        None
                    } else {
                        row.bytes_ref::<$schema, _>($field, |value| value)?
                    }
                }};
            }

            let resources = sequence_member!(
                resources_count,
                profile_segment_field::RESOURCES,
                u32,
                ProfileLineageResourceSchema
            );
            let sources = sequence_member!(
                sources_count,
                profile_segment_field::SOURCES,
                u32,
                ProfileLineageSourceSchema
            );
            let provenance = sequence_member!(
                provenance_count,
                profile_segment_field::PROVENANCE_NODES,
                u32,
                ProfileLineageProvenanceSchema
            );
            let operations = sequence_member!(
                operations_count,
                profile_segment_field::OPERATIONS,
                u32,
                ProfileLineageOperationSchema
            );
            let evidence = sequence_member!(
                evidence_count,
                profile_segment_field::EVIDENCE,
                u32,
                ProfileLineageEvidenceSchema
            );
            let parents = sequence_member!(
                parents_count,
                profile_segment_field::PARENTS,
                u64,
                ProfileLineageParentSchema
            );
            let wide = sequence_member!(
                wide_count,
                profile_segment_field::WIDE_VALUES,
                u64,
                ProfileLineageWideSchema
            );
            let text = utf8_member!(
                text_count,
                profile_segment_field::TEXTS,
                ProfileLineageTextSchema
            );
            let digests = bytes_member!(
                digest_count,
                profile_segment_field::DIGESTS,
                ProfileLineageDigestSchema
            );
            let Some(nonce) = ProfileSegmentNonce::from_stored(nonce) else {
                return Ok(false);
            };
            let segment_counts = SegmentCounts {
                resources: resources_count,
                sources: sources_count,
                provenance: provenance_count,
                operations: operations_count,
                evidence: evidence_count,
                parents: parents_count,
                wide: wide_count,
                text: text_count,
                digests: digest_count,
            };
            let expected_ordinal = match u32::try_from(row_index) {
                Ok(value) => value,
                Err(_) => return Ok(false),
            };
            if !valid
                || ordinal != expected_ordinal
                || provenance_base != totals.provenance
                || previous_nonce.is_some_and(|previous| nonce <= previous)
                || segment_counts.is_empty()
            {
                return Ok(false);
            }
            let Some(destination) = output.get_mut(row_index) else {
                return Ok(false);
            };
            *destination = ProfileSegmentValidationScratch {
                counts: segment_counts,
                prefixes: totals,
                domain: Some(ProfileSegmentAddressDomain::new(
                    ordinal,
                    nonce,
                    provenance_base,
                    provenance_count,
                )),
                resources,
                sources,
                provenance,
                operations,
                evidence,
                parents,
                wide,
                text,
                digests,
            };
            previous_nonce = Some(nonce);
            Ok(totals.append(segment_counts).is_some())
        },
    )?;
    if !complete
        || totals.resources != counts.resource_count
        || totals.sources != counts.source_count
        || totals.provenance != counts.provenance_count
        || totals.operations != counts.operation_count
        || totals.evidence != counts.evidence_count
        || totals.parents != counts.parent_count
        || totals.wide != counts.wide_count
        || totals.text != counts.lineage_text_bytes
        || totals.digests != counts.lineage_digest_bytes
    {
        return Ok(None);
    }
    Ok(Some(totals))
}

fn metered_segment_range(prefix: u32, count: u32, stride: usize) -> Option<std::ops::Range<usize>> {
    let start = usize::try_from(prefix).ok()?.checked_mul(stride)?;
    let len = usize::try_from(count).ok()?.checked_mul(stride)?;
    Some(start..start.checked_add(len)?)
}

fn visit_profile_segment_members<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    scratch: &mut ProfileMeteredScratch<'_, 'session>,
) -> Result<bool, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    for index in 0..scratch.segments.len() {
        let segment = scratch.segments[index];
        macro_rules! output_range {
            ($field:ident, $count:ident, $stride:expr) => {{
                let Some(range) =
                    metered_segment_range(segment.prefixes.$count, segment.counts.$count, $stride)
                else {
                    return Ok(false);
                };
                let Some(output) = scratch.$field.get_mut(range) else {
                    return Ok(false);
                };
                output
            }};
        }
        if !visit_sequence_into(
            session,
            segment.resources,
            output_range!(resources, resources, PROFILE_RESOURCE_SCALAR_COUNT),
        )? || !visit_sequence_into(
            session,
            segment.sources,
            output_range!(sources, sources, PROFILE_SOURCE_SCALAR_COUNT),
        )? || !visit_sequence_into(
            session,
            segment.provenance,
            output_range!(provenance, provenance, PROFILE_PROVENANCE_SCALAR_COUNT),
        )? || !visit_sequence_into(
            session,
            segment.operations,
            output_range!(operations, operations, PROFILE_OPERATION_SCALAR_COUNT),
        )? || !visit_sequence_into(
            session,
            segment.evidence,
            output_range!(evidence, evidence, PROFILE_EVIDENCE_SCALAR_COUNT),
        )? || !visit_sequence_into(
            session,
            segment.parents,
            output_range!(parents, parents, PROFILE_PROVENANCE_ADDRESS_SCALARS),
        )? || !visit_sequence_into(session, segment.wide, output_range!(wide, wide, 1))?
            || !visit_utf8_into(session, segment.text, output_range!(lineage_text, text, 1))?
            || !visit_bytes_into(
                session,
                segment.digests,
                output_range!(lineage_digests, digests, 1),
            )?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[allow(clippy::too_many_lines)]
pub(super) fn validate_profile_graph_metered<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    directory: CanonicalValidationMember<'session, RecordBlock, ProfileDirectorySchema>,
    counts: ProfileMeteredCounts,
    expected_lineage: ProfileLineageMaterializedRefs,
    scratch: &mut ProfileMeteredScratch<'_, 'session>,
) -> Result<Option<ProfileValidatedMembers<'session>>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    if !metered_scratch_matches(counts, scratch) || counts.owner_count < 2 {
        return Ok(None);
    }

    let mut schema_revision = None;
    let mut segment_count = None;
    let mut segments = None;
    let mut owner_count = None;
    let mut owners = None;
    let mut metadata_directory = None;
    let directory_complete = session.visit_canonical_records(
        directory,
        1,
        PROFILE_DIRECTORY_FIELD_COUNT,
        |_row_index, row| {
            schema_revision =
                row.scalar(profile_directory_field::SCHEMA_REVISION, |value: u32| value)?;
            segment_count =
                row.scalar(profile_directory_field::SEGMENT_COUNT, |value: u32| value)?;
            segments = row.record_ref::<ProfileSegmentTableSchema, _>(
                profile_directory_field::SEGMENTS,
                |value| value,
            )?;
            owner_count = row.scalar(profile_directory_field::OWNER_COUNT, |value: u32| value)?;
            owners = row.record_ref::<ProfileOwnerTableSchema, _>(
                profile_directory_field::OWNERS,
                |value| value,
            )?;
            metadata_directory = row.record_ref::<ProfileMetadataDirectorySchema, _>(
                profile_directory_field::METADATA,
                |value| value,
            )?;
            Ok(true)
        },
    )?;
    let (Some(segments), Some(owners), Some(metadata_directory)) =
        (segments, owners, metadata_directory)
    else {
        return Ok(None);
    };
    if counts.segment_count == 0
        || !directory_complete
        || schema_revision != Some(PROFILE_SCHEMA_REVISION)
        || segment_count != Some(counts.segment_count)
        || owner_count != Some(counts.owner_count)
    {
        return Ok(None);
    }

    let Some(profile_totals) =
        read_profile_segments_metered(session, segments, counts, scratch.segments)?
    else {
        return Ok(None);
    };
    if !materialized_lineage_identifies(scratch.segments, expected_lineage) {
        return Ok(None);
    }

    let mut metadata_schema = None;
    let mut metadata_entry_count = None;
    let mut metadata_entries = None;
    let mut metadata_text_count = None;
    let mut metadata_text = None;
    let mut metadata_byte_count = None;
    let mut metadata_bytes = None;
    let mut metadata_bool_count = None;
    let mut metadata_bools = None;
    let mut metadata_i64_count = None;
    let mut metadata_i64s = None;
    let mut metadata_u64_count = None;
    let mut metadata_u64s = None;
    let mut metadata_f64_count = None;
    let mut metadata_f64s = None;
    let mut metadata_f32_count = None;
    let mut metadata_f32s = None;
    let mut metadata_directory_valid = true;
    let metadata_directory_complete = session.visit_canonical_records(
        metadata_directory,
        1,
        PROFILE_METADATA_DIRECTORY_FIELD_COUNT,
        |_row_index, row| {
            metadata_schema = row.scalar(
                profile_metadata_directory_field::SCHEMA_REVISION,
                |value: u32| value,
            )?;
            metadata_entry_count = row.scalar(
                profile_metadata_directory_field::ENTRY_COUNT,
                |value: u32| value,
            )?;
            metadata_text_count = row.scalar(
                profile_metadata_directory_field::TEXT_COUNT,
                |value: u32| value,
            )?;
            metadata_byte_count = row.scalar(
                profile_metadata_directory_field::BYTE_COUNT,
                |value: u32| value,
            )?;
            metadata_bool_count = row.scalar(
                profile_metadata_directory_field::BOOL_COUNT,
                |value: u32| value,
            )?;
            metadata_i64_count = row
                .scalar(profile_metadata_directory_field::I64_COUNT, |value: u32| {
                    value
                })?;
            metadata_u64_count = row
                .scalar(profile_metadata_directory_field::U64_COUNT, |value: u32| {
                    value
                })?;
            metadata_f64_count = row
                .scalar(profile_metadata_directory_field::F64_COUNT, |value: u32| {
                    value
                })?;
            metadata_f32_count = row
                .scalar(profile_metadata_directory_field::F32_COUNT, |value: u32| {
                    value
                })?;
            macro_rules! record_member {
                ($count:expr, $field:expr, $schema:ty) => {{
                    if $count == 0 {
                        let absent = row.scalar($field, |value: u8| value)?;
                        metadata_directory_valid &= absent == Some(PROFILE_ABSENT_REFERENCE);
                        None
                    } else {
                        row.record_ref::<$schema, _>($field, |value| value)?
                    }
                }};
            }
            macro_rules! sequence_member {
                ($count:expr, $field:expr, $scalar:ty, $schema:ty) => {{
                    if $count == 0 {
                        let absent = row.scalar($field, |value: u8| value)?;
                        metadata_directory_valid &= absent == Some(PROFILE_ABSENT_REFERENCE);
                        None
                    } else {
                        row.sequence_ref::<$scalar, $schema, _>($field, |value| value)?
                    }
                }};
            }
            macro_rules! utf8_member {
                ($count:expr, $field:expr, $schema:ty) => {{
                    if $count == 0 {
                        let absent = row.scalar($field, |value: u8| value)?;
                        metadata_directory_valid &= absent == Some(PROFILE_ABSENT_REFERENCE);
                        None
                    } else {
                        row.utf8_ref::<$schema, _>($field, |value| value)?
                    }
                }};
            }
            macro_rules! bytes_member {
                ($count:expr, $field:expr, $schema:ty) => {{
                    if $count == 0 {
                        let absent = row.scalar($field, |value: u8| value)?;
                        metadata_directory_valid &= absent == Some(PROFILE_ABSENT_REFERENCE);
                        None
                    } else {
                        row.bytes_ref::<$schema, _>($field, |value| value)?
                    }
                }};
            }
            metadata_entries = record_member!(
                counts.metadata_entry_count,
                profile_metadata_directory_field::ENTRIES,
                ProfileMetadataEntryTableSchema
            );
            metadata_text = utf8_member!(
                counts.metadata_text_bytes,
                profile_metadata_directory_field::TEXTS,
                ProfileMetadataTextSchema
            );
            metadata_bytes = bytes_member!(
                counts.metadata_payload_bytes,
                profile_metadata_directory_field::BYTES,
                ProfileMetadataBytesSchema
            );
            metadata_bools = sequence_member!(
                counts.metadata_bool_count,
                profile_metadata_directory_field::BOOL_VALUES,
                u8,
                ProfileMetadataBoolSchema
            );
            metadata_i64s = sequence_member!(
                counts.metadata_i64_count,
                profile_metadata_directory_field::I64_VALUES,
                i64,
                ProfileMetadataI64Schema
            );
            metadata_u64s = sequence_member!(
                counts.metadata_u64_count,
                profile_metadata_directory_field::U64_VALUES,
                u64,
                ProfileMetadataU64Schema
            );
            metadata_f64s = sequence_member!(
                counts.metadata_f64_count,
                profile_metadata_directory_field::F64_VALUES,
                f64,
                ProfileMetadataF64Schema
            );
            metadata_f32s = sequence_member!(
                counts.metadata_f32_count,
                profile_metadata_directory_field::F32_VALUES,
                f32,
                ProfileMetadataF32Schema
            );
            Ok(metadata_directory_valid)
        },
    )?;
    if !metadata_directory_complete
        || !metadata_directory_valid
        || metadata_schema != Some(PROFILE_METADATA_SCHEMA_REVISION)
        || metadata_entry_count != Some(counts.metadata_entry_count)
        || metadata_text_count != Some(counts.metadata_text_bytes)
        || metadata_byte_count != Some(counts.metadata_payload_bytes)
        || metadata_bool_count != Some(counts.metadata_bool_count)
        || metadata_i64_count != Some(counts.metadata_i64_count)
        || metadata_u64_count != Some(counts.metadata_u64_count)
        || metadata_f64_count != Some(counts.metadata_f64_count)
        || metadata_f32_count != Some(counts.metadata_f32_count)
    {
        return Ok(None);
    }
    let metadata_members = ProfileTypedMetadataMembers {
        entries: metadata_entries,
        text: metadata_text,
        bytes: metadata_bytes,
        bools: metadata_bools,
        i64s: metadata_i64s,
        u64s: metadata_u64s,
        f64s: metadata_f64s,
        f32s: metadata_f32s,
    };

    let mut attachments = None;
    let mut owner_refs_valid = true;
    let owners_complete = session.visit_canonical_records(
        owners,
        usize::try_from(counts.owner_count).unwrap_or(usize::MAX),
        PROFILE_OWNER_FIELD_COUNT,
        |row_index, row| {
            let kind = row.scalar(profile_owner_field::OWNER_KIND, |value: u8| value)?;
            let index = row.scalar(profile_owner_field::OWNER_INDEX, |value: u32| value)?;
            let offset = row.scalar(profile_owner_field::ATTACHMENT_OFFSET, |value: u32| value)?;
            let count = row.scalar(profile_owner_field::ATTACHMENT_COUNT, |value: u32| value)?;
            let member = row.sequence_ref::<u64, ProfileLineageAttachmentSchema, _>(
                profile_owner_field::ATTACHMENTS,
                |value| value,
            )?;
            let (Some(kind), Some(index), Some(offset), Some(count), Some(member)) =
                (kind, index, offset, count, member)
            else {
                return Ok(false);
            };
            let Some(start) = row_index.checked_mul(4) else {
                return Ok(false);
            };
            let Some(values) = scratch.owner_rows.get_mut(start..start + 4) else {
                return Ok(false);
            };
            values.copy_from_slice(&[u32::from(kind), index, offset, count]);
            owner_refs_valid &= attachments.is_none_or(|existing| existing == member);
            attachments = Some(member);
            Ok(owner_refs_valid)
        },
    )?;
    let Some(attachments) = attachments else {
        return Ok(None);
    };
    if !owners_complete
        || !owner_refs_valid
        || !expected_lineage
            .attachments()
            .is_some_and(|slot| attachments.identifies(slot.into_store()))
    {
        return Ok(None);
    }

    if let Some(entries) = metadata_members.entries {
        let mut entry_refs_valid = true;
        let entries_complete = session.visit_canonical_records(
            entries,
            usize::try_from(counts.metadata_entry_count).unwrap_or(usize::MAX),
            PROFILE_METADATA_ENTRY_FIELD_COUNT,
            |row_index, row| {
                let owner_kind =
                    row.scalar(profile_metadata_entry_field::OWNER_KIND, |value: u8| value)?;
                let owner_index = row
                    .scalar(profile_metadata_entry_field::OWNER_INDEX, |value: u32| {
                        value
                    })?;
                let key_offset =
                    row.scalar(profile_metadata_entry_field::KEY_OFFSET, |value: u32| value)?;
                let key_count =
                    row.scalar(profile_metadata_entry_field::KEY_COUNT, |value: u32| value)?;
                let value_kind =
                    row.scalar(profile_metadata_entry_field::VALUE_KIND, |value: u8| value)?;
                let array_kind =
                    row.scalar(profile_metadata_entry_field::ARRAY_KIND, |value: u8| value)?;
                let logical_count = row.scalar(
                    profile_metadata_entry_field::LOGICAL_ELEMENT_COUNT,
                    |value: u32| value,
                )?;
                let auxiliary_offset = row.scalar(
                    profile_metadata_entry_field::AUX_TEXT_OFFSET,
                    |value: u32| value,
                )?;
                let auxiliary_count = row.scalar(
                    profile_metadata_entry_field::AUX_TEXT_COUNT,
                    |value: u32| value,
                )?;
                let byte_offset = row
                    .scalar(profile_metadata_entry_field::BYTE_OFFSET, |value: u32| {
                        value
                    })?;
                let byte_count =
                    row.scalar(profile_metadata_entry_field::BYTE_COUNT, |value: u32| value)?;
                let scalar_offset = row
                    .scalar(profile_metadata_entry_field::SCALAR_OFFSET, |value: u32| {
                        value
                    })?;
                let scalar_count = row
                    .scalar(profile_metadata_entry_field::SCALAR_COUNT, |value: u32| {
                        value
                    })?;
                let (
                    Some(owner_kind),
                    Some(owner_index),
                    Some(key_offset),
                    Some(key_count),
                    Some(value_kind),
                    Some(array_kind),
                    Some(logical_count),
                    Some(auxiliary_offset),
                    Some(auxiliary_count),
                    Some(byte_offset),
                    Some(byte_count),
                    Some(scalar_offset),
                    Some(scalar_count),
                ) = (
                    owner_kind,
                    owner_index,
                    key_offset,
                    key_count,
                    value_kind,
                    array_kind,
                    logical_count,
                    auxiliary_offset,
                    auxiliary_count,
                    byte_offset,
                    byte_count,
                    scalar_offset,
                    scalar_count,
                )
                else {
                    return Ok(false);
                };
                let Some(start) = row_index.checked_mul(13) else {
                    return Ok(false);
                };
                let Some(values) = scratch.metadata_rows.get_mut(start..start + 13) else {
                    return Ok(false);
                };
                values.copy_from_slice(&[
                    u32::from(owner_kind),
                    owner_index,
                    key_offset,
                    key_count,
                    u32::from(value_kind),
                    u32::from(array_kind),
                    logical_count,
                    auxiliary_offset,
                    auxiliary_count,
                    byte_offset,
                    byte_count,
                    scalar_offset,
                    scalar_count,
                ]);
                let shape = metered_metadata_shape(value_kind, array_kind, logical_count);
                entry_refs_valid &= shape.is_some();
                let shape = shape.unwrap_or(MetadataShape {
                    lane: MetadataLane::None,
                    scalar_count: 0,
                    auxiliary_text: false,
                    bytes: false,
                });
                macro_rules! lane_ref {
                    ($used:expr, $field:expr, sequence $scalar:ty, $schema:ty, $expected:expr) => {{
                        if $used {
                            row.sequence_ref::<$scalar, $schema, _>($field, |value| value)?
                                .is_some_and(|member| Some(member) == $expected)
                        } else {
                            row.scalar($field, |value: u8| value)? == Some(PROFILE_ABSENT_REFERENCE)
                        }
                    }};
                    ($used:expr, $field:expr, utf8 $schema:ty, $expected:expr) => {{
                        if $used {
                            row.utf8_ref::<$schema, _>($field, |value| value)?
                                .is_some_and(|member| Some(member) == $expected)
                        } else {
                            row.scalar($field, |value: u8| value)? == Some(PROFILE_ABSENT_REFERENCE)
                        }
                    }};
                    ($used:expr, $field:expr, bytes $schema:ty, $expected:expr) => {{
                        if $used {
                            row.bytes_ref::<$schema, _>($field, |value| value)?
                                .is_some_and(|member| Some(member) == $expected)
                        } else {
                            row.scalar($field, |value: u8| value)? == Some(PROFILE_ABSENT_REFERENCE)
                        }
                    }};
                }
                entry_refs_valid &= lane_ref!(
                    true,
                    profile_metadata_entry_field::TEXTS,
                    utf8 ProfileMetadataTextSchema,
                    metadata_members.text
                );
                entry_refs_valid &= lane_ref!(
                    shape.bytes && byte_count != 0,
                    profile_metadata_entry_field::BYTES,
                    bytes ProfileMetadataBytesSchema,
                    metadata_members.bytes
                );
                entry_refs_valid &= lane_ref!(
                    shape.lane == MetadataLane::Bool && scalar_count != 0,
                    profile_metadata_entry_field::BOOL_VALUES,
                    sequence u8,
                    ProfileMetadataBoolSchema,
                    metadata_members.bools
                );
                entry_refs_valid &= lane_ref!(
                    shape.lane == MetadataLane::I64 && scalar_count != 0,
                    profile_metadata_entry_field::I64_VALUES,
                    sequence i64,
                    ProfileMetadataI64Schema,
                    metadata_members.i64s
                );
                entry_refs_valid &= lane_ref!(
                    shape.lane == MetadataLane::U64 && scalar_count != 0,
                    profile_metadata_entry_field::U64_VALUES,
                    sequence u64,
                    ProfileMetadataU64Schema,
                    metadata_members.u64s
                );
                entry_refs_valid &= lane_ref!(
                    shape.lane == MetadataLane::F64 && scalar_count != 0,
                    profile_metadata_entry_field::F64_VALUES,
                    sequence f64,
                    ProfileMetadataF64Schema,
                    metadata_members.f64s
                );
                entry_refs_valid &= lane_ref!(
                    shape.lane == MetadataLane::F32 && scalar_count != 0,
                    profile_metadata_entry_field::F32_VALUES,
                    sequence f32,
                    ProfileMetadataF32Schema,
                    metadata_members.f32s
                );
                Ok(entry_refs_valid)
            },
        )?;
        if !entries_complete || !entry_refs_valid {
            return Ok(None);
        }
    } else if counts.metadata_entry_count != 0 {
        return Ok(None);
    }

    if !visit_profile_segment_members(session, scratch)?
        || !visit_sequence_into(session, Some(attachments), scratch.attachments)?
        || !visit_utf8_into(session, metadata_members.text, scratch.metadata_text)?
        || !visit_bytes_into(session, metadata_members.bytes, scratch.metadata_bytes)?
        || !visit_sequence_into_validated(
            session,
            metadata_members.bools,
            scratch.metadata_bools,
            |value| value <= 1,
        )?
        || !visit_sequence_into(session, metadata_members.i64s, scratch.metadata_i64s)?
        || !visit_sequence_into(session, metadata_members.u64s, scratch.metadata_u64s)?
        || !visit_sequence_into_validated(
            session,
            metadata_members.f64s,
            scratch.metadata_f64s,
            f64::is_finite,
        )?
        || !visit_sequence_into_validated(
            session,
            metadata_members.f32s,
            scratch.metadata_f32s,
            f32::is_finite,
        )?
    {
        return Ok(None);
    }

    let semantic = validate_metered_profile_semantics(session, counts, scratch)?;
    let Some((owner_classes, maximum_provenance_depth)) = semantic else {
        return Ok(None);
    };

    let metadata_shareable_leaf_count = u32::try_from(
        [
            metadata_members.text.is_some(),
            metadata_members.bytes.is_some(),
            metadata_members.bools.is_some(),
            metadata_members.i64s.is_some(),
            metadata_members.u64s.is_some(),
            metadata_members.f64s.is_some(),
            metadata_members.f32s.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count(),
    )
    .ok();
    let Some(metadata_shareable_leaf_count) = metadata_shareable_leaf_count else {
        return Ok(None);
    };
    let Some(shareable_leaf_count) = profile_totals
        .shareable_leaves
        .checked_add(1)
        .and_then(|value| value.checked_add(metadata_shareable_leaf_count))
    else {
        return Ok(None);
    };
    let Some(payload_leaf_count) = profile_totals
        .payload_leaves
        .checked_add(metadata_shareable_leaf_count)
    else {
        return Ok(None);
    };
    let Some(canonical_block_count) = 4_u32
        .checked_add(u32::from(metadata_members.entries.is_some()))
        .and_then(|value| value.checked_add(shareable_leaf_count))
    else {
        return Ok(None);
    };
    if canonical_block_count != counts.canonical_block_count
        || payload_leaf_count != counts.payload_leaf_count
        || shareable_leaf_count != counts.shareable_leaf_count
    {
        return Ok(None);
    }

    Ok(Some(ProfileValidatedMembers {
        directory,
        segments,
        owners,
        attachments,
        metadata_directory,
        metadata_entries: metadata_members.entries,
        metadata_text: metadata_members.text,
        metadata_bytes: metadata_members.bytes,
        metadata_bools: metadata_members.bools,
        metadata_i64s: metadata_members.i64s,
        metadata_u64s: metadata_members.u64s,
        metadata_f64s: metadata_members.f64s,
        metadata_f32s: metadata_members.f32s,
        segment_count: counts.segment_count,
        resource_count: counts.resource_count,
        source_count: counts.source_count,
        provenance_count: counts.provenance_count,
        operation_count: counts.operation_count,
        evidence_count: counts.evidence_count,
        parent_count: counts.parent_count,
        wide_count: counts.wide_count,
        lineage_text_bytes: counts.lineage_text_bytes,
        lineage_digest_bytes: counts.lineage_digest_bytes,
        attachment_count: counts.attachment_count,
        owner_count: counts.owner_count,
        metadata_entry_count: counts.metadata_entry_count,
        metadata_text_bytes: counts.metadata_text_bytes,
        metadata_payload_bytes: counts.metadata_payload_bytes,
        metadata_bool_count: counts.metadata_bool_count,
        metadata_i64_count: counts.metadata_i64_count,
        metadata_u64_count: counts.metadata_u64_count,
        metadata_f64_count: counts.metadata_f64_count,
        metadata_f32_count: counts.metadata_f32_count,
        canonical_block_count,
        payload_leaf_count,
        shareable_leaf_count,
        maximum_provenance_depth,
        owner_classes,
    }))
}

fn run_profile_semantic_unit<'session, 'storage, 'control, 'operation, O, PP, U, R>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    action: impl FnOnce() -> Option<R>,
) -> Result<Option<R>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    Ok(session.run_validation_unit(action)?.flatten())
}

fn validate_metered_profile_semantics<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    counts: ProfileMeteredCounts,
    scratch: &mut ProfileMeteredScratch<'_, 'session>,
) -> Result<Option<(ProfileOwnerClassCounts, u32)>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let mut maximum_depth = 0_u32;
    for segment_index in 0..scratch.segments.len() {
        let segment = scratch.segments[segment_index];
        let (Some(lineage_text), Some(lineage_digests), Some(wide_values)) = (
            metered_segment_slice(
                scratch.lineage_text,
                segment.prefixes.text,
                segment.counts.text,
                1,
            )
            .and_then(|bytes| std::str::from_utf8(bytes).ok()),
            metered_segment_slice(
                scratch.lineage_digests,
                segment.prefixes.digests,
                segment.counts.digests,
                1,
            ),
            metered_segment_slice(scratch.wide, segment.prefixes.wide, segment.counts.wide, 1),
        ) else {
            return Ok(None);
        };
        let mut text_cursor = 0_u32;
        let mut digest_cursor = 0_u32;
        let mut wide_cursor = 0_u32;
        let Some(resources) = metered_segment_slice(
            scratch.resources,
            segment.prefixes.resources,
            segment.counts.resources,
            PROFILE_RESOURCE_SCALAR_COUNT,
        ) else {
            return Ok(None);
        };
        for row in resources.chunks_exact(PROFILE_RESOURCE_SCALAR_COUNT) {
            let Some(()) = run_profile_semantic_unit(session, || {
                let mut no_work = |_units| Some(());
                validate_resources::<()>(row, lineage_text, &mut text_cursor, None, &mut no_work)
            })?
            else {
                return Ok(None);
            };
        }
        let Some(sources) = metered_segment_slice(
            scratch.sources,
            segment.prefixes.sources,
            segment.counts.sources,
            PROFILE_SOURCE_SCALAR_COUNT,
        ) else {
            return Ok(None);
        };
        let Some(visible_resource_end) = segment
            .prefixes
            .resources
            .checked_add(segment.counts.resources)
        else {
            return Ok(None);
        };
        for row in sources.chunks_exact(PROFILE_SOURCE_SCALAR_COUNT) {
            let Some(()) = run_profile_semantic_unit(session, || {
                let mut no_work = |_units| Some(());
                validate_sources::<()>(
                    row,
                    lineage_text,
                    lineage_digests,
                    wide_values,
                    &mut text_cursor,
                    &mut digest_cursor,
                    &mut wide_cursor,
                    visible_resource_end,
                    segment.prefixes.resources,
                    None,
                    &mut no_work,
                )
            })?
            else {
                return Ok(None);
            };
        }
        let (Some(provenance), Some(operations), Some(evidence), Some(parents)) = (
            metered_segment_slice(
                scratch.provenance,
                segment.prefixes.provenance,
                segment.counts.provenance,
                PROFILE_PROVENANCE_SCALAR_COUNT,
            ),
            metered_segment_slice(
                scratch.operations,
                segment.prefixes.operations,
                segment.counts.operations,
                PROFILE_OPERATION_SCALAR_COUNT,
            ),
            metered_segment_slice(
                scratch.evidence,
                segment.prefixes.evidence,
                segment.counts.evidence,
                PROFILE_EVIDENCE_SCALAR_COUNT,
            ),
            metered_segment_slice(
                scratch.parents,
                segment.prefixes.parents,
                segment.counts.parents,
                PROFILE_PROVENANCE_ADDRESS_SCALARS,
            ),
        ) else {
            return Ok(None);
        };
        let Some(segment_depth) = validate_metered_provenance_semantics(
            session,
            segment,
            scratch.segments,
            provenance,
            operations,
            evidence,
            parents,
            wide_values,
            lineage_digests,
            scratch.provenance_depths,
            lineage_text,
            &mut text_cursor,
            &mut digest_cursor,
            &mut wide_cursor,
        )?
        else {
            return Ok(None);
        };
        if text_cursor != segment.counts.text
            || digest_cursor != segment.counts.digests
            || wide_cursor != segment.counts.wide
        {
            return Ok(None);
        }
        maximum_depth = maximum_depth.max(segment_depth);
    }
    let Some(owner_classes) =
        validate_metered_owner_semantics(session, counts, scratch.segments, scratch)?
    else {
        return Ok(None);
    };
    let Some(()) = validate_metered_metadata_semantics(session, counts, owner_classes, scratch)?
    else {
        return Ok(None);
    };
    Ok(Some((owner_classes, maximum_depth)))
}

fn metered_segment_slice<T>(values: &[T], prefix: u32, count: u32, stride: usize) -> Option<&[T]> {
    values.get(metered_segment_range(prefix, count, stride)?)
}

enum MeteredProvenanceStep {
    Source,
    Operation { parent_count: u32 },
}

#[allow(clippy::too_many_arguments)]
fn validate_metered_provenance_semantics<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    segment: ProfileSegmentValidationScratch<'session>,
    segments: &[ProfileSegmentValidationScratch<'session>],
    provenance: &[u32],
    operations: &[u32],
    evidence: &[u32],
    parent_scalars: &[u64],
    wide: &[u64],
    digests: &[u8],
    depths: &mut [u32],
    text: &str,
    text_cursor: &mut u32,
    digest_cursor: &mut u32,
    wide_cursor: &mut u32,
) -> Result<Option<u32>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(parents) = QualifiedProvenanceAddressSlice::from_scalars(
        parent_scalars,
        usize::try_from(segment.counts.parents).unwrap_or(usize::MAX),
    ) else {
        return Ok(None);
    };
    let mut operation_cursor = 0_u32;
    let mut evidence_cursor = 0_u32;
    let mut parent_cursor = 0_u32;
    let mut maximum_depth = 0_u32;
    for (local_index, row) in provenance
        .chunks_exact(PROFILE_PROVENANCE_SCALAR_COUNT)
        .enumerate()
    {
        let Some(local_index) = u32::try_from(local_index).ok() else {
            return Ok(None);
        };
        let Some(global_index) = segment.prefixes.provenance.checked_add(local_index) else {
            return Ok(None);
        };
        let Some(step) =
            run_profile_semantic_unit(session, || match row[profile_provenance_scalar::KIND] {
                value if value == u32::from(PROFILE_PROVENANCE_SOURCE) => {
                    if row[profile_provenance_scalar::VARIANT_INDEX]
                        >= segment
                            .prefixes
                            .sources
                            .checked_add(segment.counts.sources)?
                        || row[profile_provenance_scalar::DEPTH] != 1
                        || require_range(
                            parent_cursor,
                            row[profile_provenance_scalar::PARENT_OFFSET],
                            row[profile_provenance_scalar::PARENT_COUNT],
                            0,
                        )
                        .is_none()
                    {
                        return None;
                    }
                    *depths.get_mut(usize::try_from(global_index).ok()?)? = 1;
                    Some(MeteredProvenanceStep::Source)
                }
                value if value == u32::from(PROFILE_PROVENANCE_OPERATION) => {
                    let parent_count = row[profile_provenance_scalar::PARENT_COUNT];
                    (row[profile_provenance_scalar::VARIANT_INDEX] == operation_cursor
                        && require_range(
                            parent_cursor,
                            row[profile_provenance_scalar::PARENT_OFFSET],
                            parent_count,
                            parent_count,
                        )
                        .is_some())
                    .then_some(MeteredProvenanceStep::Operation { parent_count })
                }
                _ => None,
            })?
        else {
            return Ok(None);
        };
        match step {
            MeteredProvenanceStep::Source => maximum_depth = maximum_depth.max(1),
            MeteredProvenanceStep::Operation { parent_count } => {
                let Some(parent_values) = parents.get_range(
                    usize::try_from(parent_cursor).unwrap_or(usize::MAX),
                    usize::try_from(parent_count).unwrap_or(usize::MAX),
                ) else {
                    return Ok(None);
                };
                let mut previous = None;
                let mut maximum_parent_depth = 0_u32;
                for address in parent_values.iter() {
                    let Some((parent, parent_depth)) = run_profile_semantic_unit(session, || {
                        let parent = validate_metered_qualified_address(segments, address?)?;
                        if parent >= global_index
                            || previous.is_some_and(|previous| parent <= previous)
                        {
                            return None;
                        }
                        Some((parent, *depths.get(usize::try_from(parent).ok()?)?))
                    })?
                    else {
                        return Ok(None);
                    };
                    previous = Some(parent);
                    maximum_parent_depth = maximum_parent_depth.max(parent_depth);
                }
                let Some(depth) = maximum_parent_depth.checked_add(1) else {
                    return Ok(None);
                };
                let Some(()) = run_profile_semantic_unit(session, || {
                    if row[profile_provenance_scalar::DEPTH] != depth {
                        return None;
                    }
                    let mut no_work = |_units| Some(());
                    validate_operation_header(
                        operations,
                        operation_cursor,
                        &mut evidence_cursor,
                        text,
                        digests,
                        text_cursor,
                        digest_cursor,
                        None,
                        &mut no_work,
                    )?;
                    *depths.get_mut(usize::try_from(global_index).ok()?)? = depth;
                    Some(())
                })?
                else {
                    return Ok(None);
                };
                parent_cursor = match parent_cursor.checked_add(parent_count) {
                    Some(value) => value,
                    None => return Ok(None),
                };
                operation_cursor = match operation_cursor.checked_add(1) {
                    Some(value) => value,
                    None => return Ok(None),
                };
                maximum_depth = maximum_depth.max(depth);
            }
        }
    }
    if operation_cursor != segment.counts.operations
        || evidence_cursor != segment.counts.evidence
        || parent_cursor != segment.counts.parents
    {
        return Ok(None);
    }

    let mut operation_cursor = 0_u32;
    let mut evidence_cursor = 0_u32;
    for row in provenance.chunks_exact(PROFILE_PROVENANCE_SCALAR_COUNT) {
        let Some(is_operation) = run_profile_semantic_unit(session, || {
            Some(row[profile_provenance_scalar::KIND] == u32::from(PROFILE_PROVENANCE_OPERATION))
        })?
        else {
            return Ok(None);
        };
        if !is_operation {
            continue;
        }
        let Some(evidence_count) = run_profile_semantic_unit(session, || {
            let operation =
                scalar_row(operations, operation_cursor, PROFILE_OPERATION_SCALAR_COUNT)?;
            let count = operation[profile_operation_scalar::EVIDENCE_COUNT];
            (operation[profile_operation_scalar::EVIDENCE_OFFSET] == evidence_cursor)
                .then_some(count)
        })?
        else {
            return Ok(None);
        };
        let mut previous_code: Option<&str> = None;
        for local in 0..evidence_count {
            let Some(index) = evidence_cursor.checked_add(local) else {
                return Ok(None);
            };
            let Some(code) = run_profile_semantic_unit(session, || {
                let mut no_work = |_units| Some(());
                let code = validate_evidence(
                    evidence,
                    index,
                    text,
                    digests,
                    wide,
                    text_cursor,
                    digest_cursor,
                    wide_cursor,
                    None,
                    &mut no_work,
                )?;
                previous_code
                    .is_none_or(|previous| previous.as_bytes() < code.as_bytes())
                    .then_some(code)
            })?
            else {
                return Ok(None);
            };
            previous_code = Some(code);
        }
        evidence_cursor = match evidence_cursor.checked_add(evidence_count) {
            Some(value) => value,
            None => return Ok(None),
        };
        operation_cursor = match operation_cursor.checked_add(1) {
            Some(value) => value,
            None => return Ok(None),
        };
    }
    Ok((operation_cursor == segment.counts.operations
        && evidence_cursor == segment.counts.evidence)
        .then_some(maximum_depth))
}

fn validate_metered_qualified_address(
    segments: &[ProfileSegmentValidationScratch<'_>],
    address: QualifiedProvenanceAddress,
) -> Option<u32> {
    let ordinal = address.segment_ordinal()?;
    let segment = segments.get(usize::try_from(ordinal).ok()?)?;
    address.validate_against(segment.domain?)
}

fn validate_metered_owner_semantics<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    counts: ProfileMeteredCounts,
    segments: &[ProfileSegmentValidationScratch<'_>],
    scratch: &ProfileMeteredScratch<'_, '_>,
) -> Result<Option<ProfileOwnerClassCounts>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(attachments) = QualifiedProvenanceAddressSlice::from_scalars(
        scratch.attachments,
        usize::try_from(counts.attachment_count).unwrap_or(usize::MAX),
    ) else {
        return Ok(None);
    };
    let mut classes = ProfileOwnerClassCounts::default();
    let mut attachment_cursor = 0_u32;
    let mut previous_owner = None;
    for (row_index, row) in scratch.owner_rows.chunks_exact(4).enumerate() {
        let Some((owner, count)) = run_profile_semantic_unit(session, || {
            let owner = decode_owner(u8::try_from(row[0]).ok()?, row[1])?;
            record_owner(&mut classes, owner, row_index, previous_owner)?;
            let count = row[3];
            (count != 0 && row[2] == attachment_cursor).then_some((owner, count))
        })?
        else {
            return Ok(None);
        };
        previous_owner = Some(owner);
        let Some(values) = attachments.get_range(
            usize::try_from(attachment_cursor).unwrap_or(usize::MAX),
            usize::try_from(count).unwrap_or(usize::MAX),
        ) else {
            return Ok(None);
        };
        let mut previous_provenance = None;
        for address in values.iter() {
            let Some(provenance) = run_profile_semantic_unit(session, || {
                let provenance = validate_metered_qualified_address(segments, address?)?;
                (provenance < counts.provenance_count
                    && previous_provenance.is_none_or(|previous| provenance > previous))
                .then_some(provenance)
            })?
            else {
                return Ok(None);
            };
            previous_provenance = Some(provenance);
        }
        attachment_cursor = match attachment_cursor.checked_add(count) {
            Some(value) => value,
            None => return Ok(None),
        };
    }
    Ok((attachment_cursor == counts.attachment_count).then_some(classes))
}

fn validate_metered_metadata_semantics<'session, 'storage, 'control, 'operation, O, PP, U>(
    session: &mut CanonicalValidationSession<'session, 'storage, 'control, 'operation, O, PP, U>,
    counts: ProfileMeteredCounts,
    owner_classes: ProfileOwnerClassCounts,
    scratch: &ProfileMeteredScratch<'_, '_>,
) -> Result<Option<()>, StoreWorkError>
where
    O: OperationOwner,
    PP: ProgressPhase,
    U: ProgressUnit,
{
    let Some(text) = std::str::from_utf8(scratch.metadata_text).ok() else {
        return Ok(None);
    };
    let mut cursors = MetadataCursors::default();
    let mut previous_owner = None;
    let mut previous_key: Option<&str> = None;
    for row in scratch.metadata_rows.chunks_exact(13) {
        let Some((owner, key)) = run_profile_semantic_unit(session, || {
            let mut no_work = |_units| Some(());
            let owner = decode_owner(u8::try_from(row[0]).ok()?, row[1])?;
            if !owner_exists(owner_classes, owner) {
                return None;
            }
            let key = consume_text(
                text,
                &mut cursors.text,
                row[2],
                row[3],
                true,
                None,
                &mut no_work,
            )?;
            if previous_owner.is_some_and(|previous| owner < previous)
                || previous_owner == Some(owner)
                    && previous_key.is_some_and(|previous| previous.as_bytes() >= key.as_bytes())
            {
                return None;
            }
            let value_kind = u8::try_from(row[4]).ok()?;
            let array_kind = u8::try_from(row[5]).ok()?;
            let shape = metered_metadata_shape(value_kind, array_kind, row[6])?;
            let auxiliary_count = row[8];
            let auxiliary = consume_text(
                text,
                &mut cursors.text,
                row[7],
                auxiliary_count,
                shape.auxiliary_text && value_kind == PROFILE_METADATA_BYTES,
                None,
                &mut no_work,
            )?;
            if shape.auxiliary_text
                != (value_kind == PROFILE_METADATA_STRING || value_kind == PROFILE_METADATA_BYTES)
                || !shape.auxiliary_text && auxiliary_count != 0
                || value_kind == PROFILE_METADATA_BYTES && auxiliary.is_empty()
            {
                return None;
            }
            consume_bytes(
                scratch.metadata_bytes,
                &mut cursors.bytes,
                row[9],
                row[10],
                None,
                &mut no_work,
            )?;
            if shape.bytes != (value_kind == PROFILE_METADATA_BYTES)
                || !shape.bytes && row[10] != 0
                || row[12] != shape.scalar_count
                || validate_metered_metadata_ranges(shape, row[11], row[12], &mut cursors).is_none()
            {
                return None;
            }
            Some((owner, key))
        })?
        else {
            return Ok(None);
        };
        previous_owner = Some(owner);
        previous_key = Some(key);
    }
    Ok((cursors.text == counts.metadata_text_bytes
        && cursors.bytes == counts.metadata_payload_bytes
        && cursors.bools == counts.metadata_bool_count
        && cursors.i64s == counts.metadata_i64_count
        && cursors.u64s == counts.metadata_u64_count
        && cursors.f64s == counts.metadata_f64_count
        && cursors.f32s == counts.metadata_f32_count)
        .then_some(()))
}

fn validate_metered_metadata_ranges(
    shape: MetadataShape,
    offset: u32,
    count: u32,
    cursors: &mut MetadataCursors,
) -> Option<()> {
    let cursor = match shape.lane {
        MetadataLane::None => return (offset == 0 && count == 0).then_some(()),
        MetadataLane::Bool => &mut cursors.bools,
        MetadataLane::I64 => &mut cursors.i64s,
        MetadataLane::U64 => &mut cursors.u64s,
        MetadataLane::F64 => &mut cursors.f64s,
        MetadataLane::F32 => &mut cursors.f32s,
    };
    if offset != *cursor {
        return None;
    }
    *cursor = cursor.checked_add(count)?;
    Some(())
}

fn validate_profile_structure<R>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<ProfileDirectorySchema>,
    expected: Option<&ValidatedProfileBatch<'_, R>>,
    scratch: &mut ProfileValidationScratch<'_>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<ProfileValidatedRefs> {
    observe_work(1)?;
    let directory = session.canonical_records(directory_ref)?;
    if directory.rows() != 1 || directory.fields() != PROFILE_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, profile_directory_field::SCHEMA_REVISION),
        PROFILE_SCHEMA_REVISION,
    )?;

    let segment_count = directory.scalar::<u32>(0, profile_directory_field::SEGMENT_COUNT)?;
    if segment_count == 0 || expected.is_some_and(|_| segment_count != 1) {
        return None;
    }
    let (segments_ref, segments) = required_records::<ProfileSegmentTableSchema>(
        session,
        directory,
        0,
        profile_directory_field::SEGMENTS,
        segment_count,
        PROFILE_SEGMENT_FIELD_COUNT,
    )?;

    let mut totals = ProfileTotals::default();
    let mut previous_nonce = None;
    for row in 0..usize::try_from(segment_count).ok()? {
        observe_work(1)?;
        let (counts, domain) =
            resolve_segment_shape(session, segments, row, totals.provenance, previous_nonce)?;
        if counts.is_empty() {
            return None;
        }
        previous_nonce = Some(domain.nonce());
        totals.append(counts)?;
    }

    if scratch.provenance_depths.len() != usize::try_from(totals.provenance).ok()? {
        return None;
    }
    if let Some(expected) = expected {
        let shape = expected.retention_shape();
        if totals.resources != expected.resource_count()
            || totals.sources != expected.source_count()
            || totals.provenance != expected.provenance_node_count()
            || totals.operations != expected.operation_count()
            || totals.evidence != shape.evidence_count()
            || totals.parents != shape.parent_count()
            || totals.wide != shape.wide_count()
            || totals.text != shape.lineage_text_bytes()
            || totals.digests != shape.lineage_digest_bytes()
        {
            return None;
        }
    }

    let mut prefixes = ProfileTotals::default();
    let mut maximum_depth = 0_u32;
    let mut previous_nonce = None;
    for row in 0..usize::try_from(segment_count).ok()? {
        observe_work(1)?;
        let (counts, domain) =
            resolve_segment_shape(session, segments, row, prefixes.provenance, previous_nonce)?;
        validate_segment(
            session,
            segments,
            row,
            counts,
            prefixes,
            expected,
            scratch.provenance_depths,
            &mut maximum_depth,
            observe_work,
        )?;
        previous_nonce = Some(domain.nonce());
        prefixes.append(counts)?;
    }
    if prefixes.resources != totals.resources
        || prefixes.sources != totals.sources
        || prefixes.provenance != totals.provenance
        || prefixes.operations != totals.operations
        || prefixes.evidence != totals.evidence
        || prefixes.parents != totals.parents
        || prefixes.wide != totals.wide
        || prefixes.text != totals.text
        || prefixes.digests != totals.digests
        || prefixes.shareable_leaves != totals.shareable_leaves
        || prefixes.payload_leaves != totals.payload_leaves
    {
        return None;
    }
    if expected.is_some_and(|expected| maximum_depth != expected.maximum_depth()) {
        return None;
    }

    let owner_count = directory.scalar::<u32>(0, profile_directory_field::OWNER_COUNT)?;
    if owner_count < 2
        || expected.is_some_and(|expected| u64::from(owner_count) != expected.owner_count())
    {
        return None;
    }
    let (owners_ref, owners) = required_records::<ProfileOwnerTableSchema>(
        session,
        directory,
        0,
        profile_directory_field::OWNERS,
        owner_count,
        PROFILE_OWNER_FIELD_COUNT,
    )?;
    let (owner_classes, attachment_count, attachments) = validate_owners(
        session,
        owners,
        owner_count,
        segments,
        totals.provenance,
        expected,
        observe_work,
    )?;
    if expected
        .is_some_and(|expected| u64::from(attachment_count) != expected.owner_attachment_count())
    {
        return None;
    }

    let metadata_ref = directory
        .record_ref::<ProfileMetadataDirectorySchema>(0, profile_directory_field::METADATA)?;
    let metadata = validate_metadata(session, metadata_ref, owner_classes, expected, observe_work)?;
    let metadata_shareable_leaf_count = u32::try_from(
        [
            metadata.text.is_some(),
            metadata.bytes.is_some(),
            metadata.bools.is_some(),
            metadata.i64s.is_some(),
            metadata.u64s.is_some(),
            metadata.f64s.is_some(),
            metadata.f32s.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count(),
    )
    .ok()?;
    let shareable_leaf_count = totals
        .shareable_leaves
        .checked_add(1)?
        .checked_add(metadata_shareable_leaf_count)?;
    let payload_leaf_count = totals
        .payload_leaves
        .checked_add(metadata_shareable_leaf_count)?;
    let canonical_block_count = 4_u32
        .checked_add(u32::from(metadata.entries.is_some()))?
        .checked_add(shareable_leaf_count)?;
    if expected.is_some_and(|expected| {
        canonical_block_count != expected.retention_shape().canonical_block_count()
            || payload_leaf_count != expected.retention_shape().payload_block_count()
            || shareable_leaf_count != expected.retention_shape().shareable_leaf_block_count()
    }) {
        return None;
    }

    Some(ProfileValidatedRefs {
        directory: CanonicalBlockRef::from_store(directory_ref),
        segments: segments_ref,
        owners: owners_ref,
        attachments,
        metadata_directory: CanonicalBlockRef::from_store(metadata_ref),
        metadata_entries: metadata.entries,
        metadata_text: metadata.text,
        metadata_bytes: metadata.bytes,
        metadata_bools: metadata.bools,
        metadata_i64s: metadata.i64s,
        metadata_u64s: metadata.u64s,
        metadata_f64s: metadata.f64s,
        metadata_f32s: metadata.f32s,
        segment_count,
        resource_count: totals.resources,
        source_count: totals.sources,
        provenance_count: totals.provenance,
        operation_count: totals.operations,
        evidence_count: totals.evidence,
        parent_count: totals.parents,
        wide_count: totals.wide,
        lineage_text_bytes: totals.text,
        lineage_digest_bytes: totals.digests,
        attachment_count,
        owner_count,
        metadata_entry_count: metadata.entry_count,
        metadata_text_bytes: metadata.text_count,
        metadata_payload_bytes: metadata.byte_count,
        metadata_bool_count: metadata.bool_count,
        metadata_i64_count: metadata.i64_count,
        metadata_u64_count: metadata.u64_count,
        metadata_f64_count: metadata.f64_count,
        metadata_f32_count: metadata.f32_count,
        canonical_block_count,
        payload_leaf_count,
        shareable_leaf_count,
        maximum_provenance_depth: maximum_depth,
        owner_classes,
    })
}

fn resolve_segment_shape(
    session: &mut StoreValidationSession<'_, '_>,
    segments: CanonicalRecordView<'_>,
    row: usize,
    expected_provenance_base: u32,
    previous_nonce: Option<ProfileSegmentNonce>,
) -> Option<(SegmentCounts, ProfileSegmentAddressDomain)> {
    require_scalar(
        segments.scalar::<u32>(row, profile_segment_field::SEGMENT),
        u32::try_from(row).ok()?,
    )?;
    let resources = segments.scalar::<u32>(row, profile_segment_field::RESOURCE_COUNT)?;
    optional_sequence::<u32, ProfileLineageResourceSchema>(
        session,
        segments,
        row,
        profile_segment_field::RESOURCES,
        resources.checked_mul(u32::try_from(PROFILE_RESOURCE_SCALAR_COUNT).ok()?)?,
    )?;
    let sources = segments.scalar::<u32>(row, profile_segment_field::SOURCE_COUNT)?;
    optional_sequence::<u32, ProfileLineageSourceSchema>(
        session,
        segments,
        row,
        profile_segment_field::SOURCES,
        sources.checked_mul(u32::try_from(PROFILE_SOURCE_SCALAR_COUNT).ok()?)?,
    )?;
    let provenance = segments.scalar::<u32>(row, profile_segment_field::PROVENANCE_COUNT)?;
    optional_sequence::<u32, ProfileLineageProvenanceSchema>(
        session,
        segments,
        row,
        profile_segment_field::PROVENANCE_NODES,
        provenance.checked_mul(u32::try_from(PROFILE_PROVENANCE_SCALAR_COUNT).ok()?)?,
    )?;
    let operations = segments.scalar::<u32>(row, profile_segment_field::OPERATION_COUNT)?;
    optional_sequence::<u32, ProfileLineageOperationSchema>(
        session,
        segments,
        row,
        profile_segment_field::OPERATIONS,
        operations.checked_mul(u32::try_from(PROFILE_OPERATION_SCALAR_COUNT).ok()?)?,
    )?;
    let evidence = segments.scalar::<u32>(row, profile_segment_field::EVIDENCE_COUNT)?;
    optional_sequence::<u32, ProfileLineageEvidenceSchema>(
        session,
        segments,
        row,
        profile_segment_field::EVIDENCE,
        evidence.checked_mul(u32::try_from(PROFILE_EVIDENCE_SCALAR_COUNT).ok()?)?,
    )?;
    let parents = segments.scalar::<u32>(row, profile_segment_field::PARENT_COUNT)?;
    optional_sequence::<u64, ProfileLineageParentSchema>(
        session,
        segments,
        row,
        profile_segment_field::PARENTS,
        parents.checked_mul(u32::try_from(PROFILE_PROVENANCE_ADDRESS_SCALARS).ok()?)?,
    )?;
    let wide = segments.scalar::<u32>(row, profile_segment_field::WIDE_COUNT)?;
    optional_sequence::<u64, ProfileLineageWideSchema>(
        session,
        segments,
        row,
        profile_segment_field::WIDE_VALUES,
        wide,
    )?;
    let text = segments.scalar::<u32>(row, profile_segment_field::TEXT_COUNT)?;
    optional_utf8::<ProfileLineageTextSchema>(
        session,
        segments,
        row,
        profile_segment_field::TEXTS,
        text,
    )?;
    let digests = segments.scalar::<u32>(row, profile_segment_field::DIGEST_COUNT)?;
    optional_bytes::<ProfileLineageDigestSchema>(
        session,
        segments,
        row,
        profile_segment_field::DIGESTS,
        digests,
    )?;
    let nonce = segments
        .scalar::<u64>(row, profile_segment_field::NONCE)
        .and_then(ProfileSegmentNonce::from_stored)?;
    if previous_nonce.is_some_and(|previous| nonce <= previous) {
        return None;
    }
    require_scalar(
        segments.scalar::<u32>(row, profile_segment_field::PROVENANCE_BASE),
        expected_provenance_base,
    )?;
    let counts = SegmentCounts {
        resources,
        sources,
        provenance,
        operations,
        evidence,
        parents,
        wide,
        text,
        digests,
    };
    let ordinal = u32::try_from(row).ok()?;
    Some((
        counts,
        ProfileSegmentAddressDomain::new(ordinal, nonce, expected_provenance_base, provenance),
    ))
}

#[allow(clippy::too_many_arguments)]
fn validate_segment<R>(
    session: &mut StoreValidationSession<'_, '_>,
    segments: CanonicalRecordView<'_>,
    row: usize,
    counts: SegmentCounts,
    prefixes: ProfileTotals,
    expected: Option<&ValidatedProfileBatch<'_, R>>,
    depths: &mut [u32],
    maximum_depth: &mut u32,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let resources = required_sequence_slice::<u32, ProfileLineageResourceSchema>(
        session,
        segments,
        row,
        profile_segment_field::RESOURCES,
        counts.resources,
        PROFILE_RESOURCE_SCALAR_COUNT,
    )?;
    let sources = required_sequence_slice::<u32, ProfileLineageSourceSchema>(
        session,
        segments,
        row,
        profile_segment_field::SOURCES,
        counts.sources,
        PROFILE_SOURCE_SCALAR_COUNT,
    )?;
    let provenance = required_sequence_slice::<u32, ProfileLineageProvenanceSchema>(
        session,
        segments,
        row,
        profile_segment_field::PROVENANCE_NODES,
        counts.provenance,
        PROFILE_PROVENANCE_SCALAR_COUNT,
    )?;
    let operations = required_sequence_slice::<u32, ProfileLineageOperationSchema>(
        session,
        segments,
        row,
        profile_segment_field::OPERATIONS,
        counts.operations,
        PROFILE_OPERATION_SCALAR_COUNT,
    )?;
    let evidence = required_sequence_slice::<u32, ProfileLineageEvidenceSchema>(
        session,
        segments,
        row,
        profile_segment_field::EVIDENCE,
        counts.evidence,
        PROFILE_EVIDENCE_SCALAR_COUNT,
    )?;
    let parent_scalars = required_sequence_slice::<u64, ProfileLineageParentSchema>(
        session,
        segments,
        row,
        profile_segment_field::PARENTS,
        counts.parents,
        PROFILE_PROVENANCE_ADDRESS_SCALARS,
    )?;
    let parents = QualifiedProvenanceAddressSlice::from_scalars(
        parent_scalars,
        usize::try_from(counts.parents).ok()?,
    )?;
    let wide = required_sequence_slice::<u64, ProfileLineageWideSchema>(
        session,
        segments,
        row,
        profile_segment_field::WIDE_VALUES,
        counts.wide,
        1,
    )?;
    let text = required_utf8::<ProfileLineageTextSchema>(
        session,
        segments,
        row,
        profile_segment_field::TEXTS,
        counts.text,
    )?;
    let digests = required_bytes::<ProfileLineageDigestSchema>(
        session,
        segments,
        row,
        profile_segment_field::DIGESTS,
        counts.digests,
    )?;

    let expected = if row == 0 { expected } else { None };
    if expected.is_some_and(|expected| {
        counts.resources != expected.resource_count()
            || counts.sources != expected.source_count()
            || counts.provenance != expected.provenance_node_count()
            || counts.operations != expected.operation_count()
    }) {
        return None;
    }
    let mut text_cursor = 0_u32;
    validate_resources(resources, text, &mut text_cursor, expected, observe_work)?;
    let mut digest_cursor = 0_u32;
    let mut wide_cursor = 0_u32;
    let visible_resource_end = prefixes.resources.checked_add(counts.resources)?;
    let visible_source_end = prefixes.sources.checked_add(counts.sources)?;
    validate_sources(
        sources,
        text,
        digests,
        wide,
        &mut text_cursor,
        &mut digest_cursor,
        &mut wide_cursor,
        visible_resource_end,
        prefixes.resources,
        expected,
        observe_work,
    )?;
    validate_provenance(
        provenance,
        operations,
        evidence,
        parents,
        segments,
        text,
        digests,
        wide,
        &mut text_cursor,
        &mut digest_cursor,
        &mut wide_cursor,
        prefixes,
        visible_source_end,
        depths,
        maximum_depth,
        expected,
        observe_work,
    )?;
    (text_cursor == counts.text && digest_cursor == counts.digests && wide_cursor == counts.wide)
        .then_some(())
}

fn validate_resources<R>(
    values: &[u32],
    text: &str,
    text_cursor: &mut u32,
    expected: Option<&ValidatedProfileBatch<'_, R>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    for (index, row) in values
        .chunks_exact(PROFILE_RESOURCE_SCALAR_COUNT)
        .enumerate()
    {
        observe_work(1)?;
        let expected_resource =
            expected.and_then(|expected| expected.lineage().resources().get(index).copied());
        consume_text(
            text,
            text_cursor,
            row[profile_resource_scalar::IDENTITY_OFFSET],
            row[profile_resource_scalar::IDENTITY_COUNT],
            true,
            expected_resource.map(|resource| resource.identity().as_str()),
            observe_work,
        )?;
        match row[profile_resource_scalar::STABILITY_KIND] {
            value if value == u32::from(PROFILE_RESOURCE_STABILITY_STABLE) => {
                let expected_stability =
                    expected_resource.and_then(|resource| match resource.stability() {
                        ResourceIdentityStability::Stable {
                            owner,
                            schema,
                            version,
                        } => Some((owner.as_str(), schema.as_str(), version.get())),
                        ResourceIdentityStability::SessionOpaque(_) => None,
                    });
                if expected_resource.is_some() && expected_stability.is_none() {
                    return None;
                }
                consume_text(
                    text,
                    text_cursor,
                    row[profile_resource_scalar::STABILITY_OWNER_OFFSET],
                    row[profile_resource_scalar::STABILITY_OWNER_COUNT],
                    true,
                    expected_stability.map(|value| value.0),
                    observe_work,
                )?;
                consume_text(
                    text,
                    text_cursor,
                    row[profile_resource_scalar::STABILITY_SCHEMA_OFFSET],
                    row[profile_resource_scalar::STABILITY_SCHEMA_COUNT],
                    true,
                    expected_stability.map(|value| value.1),
                    observe_work,
                )?;
                let version = row[profile_resource_scalar::STABILITY_SCHEMA_VERSION];
                if version == 0
                    || expected_stability.is_some_and(|expected| version != expected.2)
                    || row[profile_resource_scalar::SESSION_OPAQUE_REASON] != 0
                {
                    return None;
                }
            }
            value if value == u32::from(PROFILE_RESOURCE_STABILITY_SESSION_OPAQUE) => {
                require_empty_text_range(
                    *text_cursor,
                    row[profile_resource_scalar::STABILITY_OWNER_OFFSET],
                    row[profile_resource_scalar::STABILITY_OWNER_COUNT],
                )?;
                require_empty_text_range(
                    *text_cursor,
                    row[profile_resource_scalar::STABILITY_SCHEMA_OFFSET],
                    row[profile_resource_scalar::STABILITY_SCHEMA_COUNT],
                )?;
                let reason = row[profile_resource_scalar::SESSION_OPAQUE_REASON];
                if row[profile_resource_scalar::STABILITY_SCHEMA_VERSION] != 0
                    || decode_session_opaque_reason(reason).is_none()
                {
                    return None;
                }
                if let Some(expected_resource) = expected_resource {
                    let ResourceIdentityStability::SessionOpaque(expected_reason) =
                        expected_resource.stability()
                    else {
                        return None;
                    };
                    if reason != u32::from(encode_session_opaque_reason(*expected_reason)) {
                        return None;
                    }
                }
            }
            _ => return None,
        }
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_sources<R>(
    values: &[u32],
    text: &str,
    digests: &[u8],
    wide: &[u64],
    text_cursor: &mut u32,
    digest_cursor: &mut u32,
    wide_cursor: &mut u32,
    visible_resource_end: u32,
    resource_prefix: u32,
    expected: Option<&ValidatedProfileBatch<'_, R>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    for (index, row) in values.chunks_exact(PROFILE_SOURCE_SCALAR_COUNT).enumerate() {
        observe_work(1)?;
        let expected_source = expected
            .and_then(|expected| expected.lineage().source_identities().get(index).copied());
        let resource = row[profile_source_scalar::RESOURCE];
        if resource >= visible_resource_end
            || expected_source.is_some_and(|source| {
                resource_prefix.checked_add(source.resource().index()) != Some(resource)
            })
        {
            return None;
        }
        consume_text(
            text,
            text_cursor,
            row[profile_source_scalar::FORMAT_OFFSET],
            row[profile_source_scalar::FORMAT_COUNT],
            true,
            expected_source.map(|source| source.format().as_str()),
            observe_work,
        )?;
        match row[profile_source_scalar::LOCATOR_KIND] {
            value if value == u32::from(PROFILE_LOCATOR_INDEXED) => {
                let object_kind = row[profile_source_scalar::INDEXED_OBJECT_KIND];
                let expected_locator = expected_source.and_then(|source| match source.locator() {
                    SourceObjectLocator::Indexed { kind, index } => Some((*kind, *index)),
                    SourceObjectLocator::Namespaced(_) => None,
                });
                if decode_source_object_kind(object_kind).is_none()
                    || expected_source.is_some() && expected_locator.is_none()
                    || expected_locator.is_some_and(|value| {
                        object_kind != u32::from(encode_source_object_kind(value.0))
                    })
                {
                    return None;
                }
                let wide_value = consume_wide(
                    wide,
                    wide_cursor,
                    row[profile_source_scalar::WIDE_OFFSET],
                    row[profile_source_scalar::WIDE_COUNT],
                    1,
                    observe_work,
                )?;
                if expected_locator.is_some_and(|expected| wide_value[0] != expected.1) {
                    return None;
                }
                require_empty_text_range(
                    *text_cursor,
                    row[profile_source_scalar::LOCATOR_SCHEMA_OFFSET],
                    row[profile_source_scalar::LOCATOR_SCHEMA_COUNT],
                )?;
                require_empty_text_range(
                    *text_cursor,
                    row[profile_source_scalar::LOCATOR_VALUE_OFFSET],
                    row[profile_source_scalar::LOCATOR_VALUE_COUNT],
                )?;
                if row[profile_source_scalar::LOCATOR_SCHEMA_VERSION] != 0 {
                    return None;
                }
            }
            value if value == u32::from(PROFILE_LOCATOR_NAMESPACED) => {
                let expected_locator = expected_source.and_then(|source| match source.locator() {
                    SourceObjectLocator::Namespaced(locator) => Some(locator),
                    SourceObjectLocator::Indexed { .. } => None,
                });
                if expected_source.is_some() && expected_locator.is_none()
                    || row[profile_source_scalar::INDEXED_OBJECT_KIND] != 0
                {
                    return None;
                }
                require_empty_wide_range(
                    *wide_cursor,
                    row[profile_source_scalar::WIDE_OFFSET],
                    row[profile_source_scalar::WIDE_COUNT],
                )?;
                consume_text(
                    text,
                    text_cursor,
                    row[profile_source_scalar::LOCATOR_SCHEMA_OFFSET],
                    row[profile_source_scalar::LOCATOR_SCHEMA_COUNT],
                    true,
                    expected_locator.map(|locator| locator.schema_id().name().as_str()),
                    observe_work,
                )?;
                let version = row[profile_source_scalar::LOCATOR_SCHEMA_VERSION];
                if version == 0
                    || expected_locator
                        .is_some_and(|locator| version != locator.schema_id().version().get())
                {
                    return None;
                }
                consume_text(
                    text,
                    text_cursor,
                    row[profile_source_scalar::LOCATOR_VALUE_OFFSET],
                    row[profile_source_scalar::LOCATOR_VALUE_COUNT],
                    true,
                    expected_locator.map(|locator| locator.value().as_str()),
                    observe_work,
                )?;
            }
            _ => return None,
        }
        let expected_display = expected_source.and_then(|source| source.display_name());
        consume_optional_text(
            text,
            text_cursor,
            row[profile_source_scalar::DISPLAY_NAME_PRESENCE],
            row[profile_source_scalar::DISPLAY_NAME_OFFSET],
            row[profile_source_scalar::DISPLAY_NAME_COUNT],
            expected_source.map(|_| expected_display.map(|value| value.as_str())),
            observe_work,
        )?;
        let expected_digest = expected_source.and_then(|source| source.content_digest());
        consume_optional_digest(
            digests,
            digest_cursor,
            row[profile_source_scalar::CONTENT_DIGEST_PRESENCE],
            row[profile_source_scalar::CONTENT_DIGEST_ALGORITHM],
            row[profile_source_scalar::CONTENT_DIGEST_OFFSET],
            row[profile_source_scalar::CONTENT_DIGEST_COUNT],
            expected_source
                .map(|_| expected_digest.map(|digest| (digest.algorithm(), *digest.as_bytes()))),
            observe_work,
        )?;
    }
    Some(())
}

fn validate_qualified_address(
    segments: CanonicalRecordView<'_>,
    address: QualifiedProvenanceAddress,
) -> Option<u32> {
    let ordinal = address.segment_ordinal()?;
    let row = usize::try_from(ordinal).ok()?;
    require_scalar(
        segments.scalar::<u32>(row, profile_segment_field::SEGMENT),
        ordinal,
    )?;
    let nonce = segments
        .scalar::<u64>(row, profile_segment_field::NONCE)
        .and_then(ProfileSegmentNonce::from_stored)?;
    let base = segments.scalar::<u32>(row, profile_segment_field::PROVENANCE_BASE)?;
    let count = segments.scalar::<u32>(row, profile_segment_field::PROVENANCE_COUNT)?;
    address.validate_against(ProfileSegmentAddressDomain::new(
        ordinal, nonce, base, count,
    ))
}

#[allow(clippy::too_many_arguments)]
fn validate_provenance<R>(
    provenance: &[u32],
    operations: &[u32],
    evidence: &[u32],
    parents: QualifiedProvenanceAddressSlice<'_>,
    segments: CanonicalRecordView<'_>,
    text: &str,
    digests: &[u8],
    wide: &[u64],
    text_cursor: &mut u32,
    digest_cursor: &mut u32,
    wide_cursor: &mut u32,
    prefixes: ProfileTotals,
    visible_source_end: u32,
    depths: &mut [u32],
    maximum_depth: &mut u32,
    expected: Option<&ValidatedProfileBatch<'_, R>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let mut operation_cursor = 0_u32;
    let mut evidence_cursor = 0_u32;
    let mut parent_cursor = 0_u32;
    for (local_index, row) in provenance
        .chunks_exact(PROFILE_PROVENANCE_SCALAR_COUNT)
        .enumerate()
    {
        observe_work(1)?;
        let local_index = u32::try_from(local_index).ok()?;
        let global_index = prefixes.provenance.checked_add(local_index)?;
        let expected_node = expected.and_then(|expected| {
            expected
                .lineage()
                .provenance_nodes()
                .get(usize::try_from(local_index).ok()?)
                .copied()
        });
        let depth = match row[profile_provenance_scalar::KIND] {
            value if value == u32::from(PROFILE_PROVENANCE_SOURCE) => {
                let source = row[profile_provenance_scalar::VARIANT_INDEX];
                if source >= visible_source_end
                    || expected_node.is_some_and(|node| {
                        !matches!(node, ProvenanceNodeAuthoring::Source(expected_source)
                            if prefixes.sources.checked_add(expected_source.index()) == Some(source))
                    })
                    || row[profile_provenance_scalar::DEPTH] != 1
                {
                    return None;
                }
                require_range(
                    parent_cursor,
                    row[profile_provenance_scalar::PARENT_OFFSET],
                    row[profile_provenance_scalar::PARENT_COUNT],
                    0,
                )?;
                1
            }
            value if value == u32::from(PROFILE_PROVENANCE_OPERATION) => {
                if row[profile_provenance_scalar::VARIANT_INDEX] != operation_cursor {
                    return None;
                }
                let expected_operation = expected_node.and_then(|node| match node {
                    ProvenanceNodeAuthoring::Operation(operation) => Some(operation),
                    ProvenanceNodeAuthoring::Source(_) => None,
                });
                if expected_node.is_some() && expected_operation.is_none() {
                    return None;
                }
                let parent_count = row[profile_provenance_scalar::PARENT_COUNT];
                require_range(
                    parent_cursor,
                    row[profile_provenance_scalar::PARENT_OFFSET],
                    parent_count,
                    parent_count,
                )?;
                let parent_start = usize::try_from(parent_cursor).ok()?;
                let parent_values =
                    parents.get_range(parent_start, usize::try_from(parent_count).ok()?)?;
                let mut previous = None;
                let mut maximum_parent_depth = 0_u32;
                for (parent_index, address) in parent_values.iter().enumerate() {
                    observe_work(1)?;
                    let parent = validate_qualified_address(segments, address?)?;
                    if parent >= global_index || previous.is_some_and(|previous| parent <= previous)
                    {
                        return None;
                    }
                    if expected_operation.is_some_and(|operation| {
                        operation
                            .parents()
                            .get(parent_index)
                            .is_none_or(|expected_parent| {
                                prefixes.provenance.checked_add(expected_parent.index())
                                    != Some(parent)
                            })
                    }) {
                        return None;
                    }
                    maximum_parent_depth =
                        maximum_parent_depth.max(*depths.get(usize::try_from(parent).ok()?)?);
                    previous = Some(parent);
                }
                if expected_operation
                    .is_some_and(|operation| operation.parents().len() != parent_values.len())
                {
                    return None;
                }
                parent_cursor = parent_cursor.checked_add(parent_count)?;
                validate_operation_header(
                    operations,
                    operation_cursor,
                    &mut evidence_cursor,
                    text,
                    digests,
                    text_cursor,
                    digest_cursor,
                    expected_operation,
                    observe_work,
                )?;
                operation_cursor = operation_cursor.checked_add(1)?;
                maximum_parent_depth.checked_add(1)?
            }
            _ => return None,
        };
        if row[profile_provenance_scalar::DEPTH] != depth
            || expected.is_some_and(|expected| {
                expected.provenance_depth(ProvenanceLocalId::new(local_index)) != Some(depth)
            })
        {
            return None;
        }
        *depths.get_mut(usize::try_from(global_index).ok()?)? = depth;
        *maximum_depth = (*maximum_depth).max(depth);
    }
    let operation_count = u32::try_from(operations.len() / PROFILE_OPERATION_SCALAR_COUNT).ok()?;
    let evidence_count = u32::try_from(evidence.len() / PROFILE_EVIDENCE_SCALAR_COUNT).ok()?;
    if operation_cursor != operation_count
        || evidence_cursor != evidence_count
        || parent_cursor != u32::try_from(parents.len()).ok()?
    {
        return None;
    }

    let mut operation_cursor = 0_u32;
    let mut evidence_cursor = 0_u32;
    for (local_index, row) in provenance
        .chunks_exact(PROFILE_PROVENANCE_SCALAR_COUNT)
        .enumerate()
    {
        observe_work(1)?;
        if row[profile_provenance_scalar::KIND] != u32::from(PROFILE_PROVENANCE_OPERATION) {
            continue;
        }
        let expected_operation = expected.and_then(|expected| {
            match expected
                .lineage()
                .provenance_nodes()
                .get(local_index)
                .copied()?
            {
                ProvenanceNodeAuthoring::Operation(operation) => Some(operation),
                ProvenanceNodeAuthoring::Source(_) => None,
            }
        });
        validate_operation_evidence(
            operations,
            evidence,
            operation_cursor,
            &mut evidence_cursor,
            text,
            digests,
            wide,
            text_cursor,
            digest_cursor,
            wide_cursor,
            expected_operation,
            observe_work,
        )?;
        operation_cursor = operation_cursor.checked_add(1)?;
    }
    (operation_cursor == operation_count && evidence_cursor == evidence_count).then_some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_operation_header(
    operations: &[u32],
    operation_index: u32,
    evidence_cursor: &mut u32,
    text: &str,
    digests: &[u8],
    text_cursor: &mut u32,
    digest_cursor: &mut u32,
    expected: Option<crate::canonical_scene::store::production::ProvenanceOperationAuthoring<'_>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    observe_work(1)?;
    let row = scalar_row(operations, operation_index, PROFILE_OPERATION_SCALAR_COUNT)?;
    let kind = row[profile_operation_scalar::KIND];
    match kind {
        value if value == u32::from(PROFILE_OPERATION_NAMESPACED) => {
            let expected_name = expected.and_then(|operation| match operation.kind() {
                OperationKind::Namespaced(name) => Some(name.as_str()),
                _ => None,
            });
            if expected.is_some() && expected_name.is_none() {
                return None;
            }
            consume_text(
                text,
                text_cursor,
                row[profile_operation_scalar::NAME_OFFSET],
                row[profile_operation_scalar::NAME_COUNT],
                true,
                expected_name,
                observe_work,
            )?;
        }
        value if decode_operation_kind(value).is_some() => {
            require_empty_text_range(
                *text_cursor,
                row[profile_operation_scalar::NAME_OFFSET],
                row[profile_operation_scalar::NAME_COUNT],
            )?;
            if expected.is_some_and(|operation| {
                value != u32::from(encode_operation_kind(operation.kind()))
            }) {
                return None;
            }
        }
        _ => return None,
    }
    let expected_digest = expected.map(|operation| operation.options_digest());
    consume_required_digest(
        digests,
        digest_cursor,
        row[profile_operation_scalar::OPTIONS_DIGEST_ALGORITHM],
        row[profile_operation_scalar::OPTIONS_DIGEST_OFFSET],
        row[profile_operation_scalar::OPTIONS_DIGEST_COUNT],
        expected_digest.map(|digest| (digest.algorithm(), *digest.as_bytes())),
        observe_work,
    )?;
    let count = row[profile_operation_scalar::EVIDENCE_COUNT];
    if usize::try_from(count).ok()? > MAX_OPERATION_EVIDENCE_SUMMARIES {
        return None;
    }
    require_range(
        *evidence_cursor,
        row[profile_operation_scalar::EVIDENCE_OFFSET],
        count,
        count,
    )?;
    let expected_rows = expected.map(|operation| operation.evidence());
    if expected_rows.is_some_and(|rows| rows.len() != usize::try_from(count).unwrap_or(usize::MAX))
    {
        return None;
    }
    *evidence_cursor = evidence_cursor.checked_add(count)?;
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_operation_evidence(
    operations: &[u32],
    evidence: &[u32],
    operation_index: u32,
    evidence_cursor: &mut u32,
    text: &str,
    digests: &[u8],
    wide: &[u64],
    text_cursor: &mut u32,
    digest_cursor: &mut u32,
    wide_cursor: &mut u32,
    expected: Option<crate::canonical_scene::store::production::ProvenanceOperationAuthoring<'_>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    observe_work(1)?;
    let row = scalar_row(operations, operation_index, PROFILE_OPERATION_SCALAR_COUNT)?;
    let count = row[profile_operation_scalar::EVIDENCE_COUNT];
    require_range(
        *evidence_cursor,
        row[profile_operation_scalar::EVIDENCE_OFFSET],
        count,
        count,
    )?;
    let expected_rows = expected.map(|operation| operation.evidence());
    let mut previous_code: Option<&str> = None;
    for local in 0..count {
        let index = evidence_cursor.checked_add(local)?;
        let expected_row = expected_rows.and_then(|rows| rows.get(usize::try_from(local).ok()?));
        let code = validate_evidence(
            evidence,
            index,
            text,
            digests,
            wide,
            text_cursor,
            digest_cursor,
            wide_cursor,
            expected_row,
            observe_work,
        )?;
        if previous_code.is_some_and(|previous| previous.as_bytes() >= code.as_bytes()) {
            return None;
        }
        previous_code = Some(code);
    }
    *evidence_cursor = evidence_cursor.checked_add(count)?;
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn validate_evidence<'storage>(
    evidence: &[u32],
    index: u32,
    text: &'storage str,
    digests: &[u8],
    wide: &[u64],
    text_cursor: &mut u32,
    digest_cursor: &mut u32,
    wide_cursor: &mut u32,
    expected: Option<&crate::provenance::OperationEvidenceSummary>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<&'storage str> {
    observe_work(1)?;
    let row = scalar_row(evidence, index, PROFILE_EVIDENCE_SCALAR_COUNT)?;
    let code = consume_text(
        text,
        text_cursor,
        row[profile_evidence_scalar::CODE_OFFSET],
        row[profile_evidence_scalar::CODE_COUNT],
        true,
        expected.map(|evidence| evidence.code().as_str()),
        observe_work,
    )?;
    let occurrence = consume_wide(
        wide,
        wide_cursor,
        row[profile_evidence_scalar::OCCURRENCE_WIDE_OFFSET],
        row[profile_evidence_scalar::OCCURRENCE_WIDE_COUNT],
        1,
        observe_work,
    )?;
    if occurrence[0] == 0
        || expected.is_some_and(|expected| occurrence[0] != expected.occurrence_count().get())
    {
        return None;
    }
    let expected_digest = expected.and_then(|evidence| evidence.digest());
    consume_optional_digest(
        digests,
        digest_cursor,
        row[profile_evidence_scalar::DIGEST_PRESENCE],
        row[profile_evidence_scalar::DIGEST_ALGORITHM],
        row[profile_evidence_scalar::DIGEST_OFFSET],
        row[profile_evidence_scalar::DIGEST_COUNT],
        expected.map(|_| expected_digest.map(|digest| (digest.algorithm(), *digest.as_bytes()))),
        observe_work,
    )?;
    Some(code)
}

fn validate_owners<R>(
    session: &mut StoreValidationSession<'_, '_>,
    owners: CanonicalRecordView<'_>,
    owner_count: u32,
    segments: CanonicalRecordView<'_>,
    provenance_count: u32,
    expected: Option<&ValidatedProfileBatch<'_, R>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<(
    ProfileOwnerClassCounts,
    u32,
    CanonicalBlockRef<ProfileLineageAttachmentSchema>,
)> {
    let mut classes = ProfileOwnerClassCounts::default();
    let mut attachment_cursor = 0_u32;
    let mut attachment_ref = None;
    let mut attachment_values: Option<SequenceView<'_, u64>> = None;
    let mut previous_owner = None;
    for row in 0..usize::try_from(owner_count).ok()? {
        observe_work(1)?;
        let owner_ordinal = u64::try_from(row).ok()?;
        let owner = decode_owner(
            owners.scalar::<u8>(row, profile_owner_field::OWNER_KIND)?,
            owners.scalar::<u32>(row, profile_owner_field::OWNER_INDEX)?,
        )?;
        record_owner(&mut classes, owner, row, previous_owner)?;
        if expected.is_some_and(|expected| expected.owner_at(owner_ordinal) != Some(owner)) {
            return None;
        }
        previous_owner = Some(owner);
        let offset = owners.scalar::<u32>(row, profile_owner_field::ATTACHMENT_OFFSET)?;
        let count = owners.scalar::<u32>(row, profile_owner_field::ATTACHMENT_COUNT)?;
        if count == 0 || offset != attachment_cursor {
            return None;
        }
        let reference = owners.sequence_ref::<u64, ProfileLineageAttachmentSchema>(
            row,
            profile_owner_field::ATTACHMENTS,
        )?;
        let typed = CanonicalBlockRef::from_store(reference);
        if attachment_ref.is_some_and(|existing| existing != typed) {
            return None;
        }
        attachment_ref = Some(typed);
        let values = match attachment_values {
            Some(values) => values,
            None => {
                let values = session.canonical_sequence(reference)?;
                attachment_values = Some(values);
                values
            }
        };
        let scalar_values = values.as_slice();
        if scalar_values.len() % PROFILE_PROVENANCE_ADDRESS_SCALARS != 0 {
            return None;
        }
        let all_addresses = QualifiedProvenanceAddressSlice::from_scalars(
            scalar_values,
            scalar_values.len() / PROFILE_PROVENANCE_ADDRESS_SCALARS,
        )?;
        let values =
            all_addresses.get_range(usize::try_from(offset).ok()?, usize::try_from(count).ok()?)?;
        let expected_values = expected.and_then(|expected| {
            expected
                .owner_provenance_at(owner_ordinal)
                .and_then(|(expected_owner, values)| (expected_owner == owner).then_some(values))
        });
        if expected_values.is_some_and(|expected| expected.len() != values.len()) {
            return None;
        }
        let mut previous_provenance = None;
        for (index, address) in values.iter().enumerate() {
            observe_work(1)?;
            let provenance = validate_qualified_address(segments, address?)?;
            if provenance >= provenance_count
                || previous_provenance.is_some_and(|previous| provenance <= previous)
                || expected_values.is_some_and(|expected| {
                    expected
                        .get(index)
                        .is_none_or(|expected| expected.index() != provenance)
                })
            {
                return None;
            }
            previous_provenance = Some(provenance);
        }
        attachment_cursor = attachment_cursor.checked_add(count)?;
    }
    let values = attachment_values?;
    let address_count = values
        .len()
        .checked_div(PROFILE_PROVENANCE_ADDRESS_SCALARS)?;
    (values.len() % PROFILE_PROVENANCE_ADDRESS_SCALARS == 0
        && address_count == usize::try_from(attachment_cursor).ok()?)
    .then_some((classes, attachment_cursor, attachment_ref?))
}

#[derive(Clone, Copy)]
struct MetadataValidatedRefs {
    entries: Option<CanonicalBlockRef<ProfileMetadataEntryTableSchema>>,
    text: Option<CanonicalBlockRef<ProfileMetadataTextSchema>>,
    bytes: Option<CanonicalBlockRef<ProfileMetadataBytesSchema>>,
    bools: Option<CanonicalBlockRef<ProfileMetadataBoolSchema>>,
    i64s: Option<CanonicalBlockRef<ProfileMetadataI64Schema>>,
    u64s: Option<CanonicalBlockRef<ProfileMetadataU64Schema>>,
    f64s: Option<CanonicalBlockRef<ProfileMetadataF64Schema>>,
    f32s: Option<CanonicalBlockRef<ProfileMetadataF32Schema>>,
    entry_count: u32,
    text_count: u32,
    byte_count: u32,
    bool_count: u32,
    i64_count: u32,
    u64_count: u32,
    f64_count: u32,
    f32_count: u32,
}

#[derive(Default)]
struct MetadataCursors {
    text: u32,
    bytes: u32,
    bools: u32,
    i64s: u32,
    u64s: u32,
    f64s: u32,
    f32s: u32,
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
    scalar_count: u32,
    auxiliary_text: bool,
    bytes: bool,
}

fn record_owner(
    counts: &mut ProfileOwnerClassCounts,
    owner: CanonicalProfileOwner,
    row: usize,
    previous: Option<CanonicalProfileOwner>,
) -> Option<()> {
    if row == 0 && owner != CanonicalProfileOwner::Scene
        || row == 1 && owner != CanonicalProfileOwner::SyntheticRoot
        || row > 1 && owner <= previous?
    {
        return None;
    }
    let (count, index, base) = match owner {
        CanonicalProfileOwner::Scene | CanonicalProfileOwner::SyntheticRoot => return Some(()),
        CanonicalProfileOwner::Node(index) => (&mut counts.nodes, index, 1),
        CanonicalProfileOwner::Camera(index) => (&mut counts.cameras, index, 0),
        CanonicalProfileOwner::Light(index) => (&mut counts.lights, index, 0),
        CanonicalProfileOwner::Image(index) => (&mut counts.images, index, 0),
        CanonicalProfileOwner::Sampler(index) => (&mut counts.samplers, index, 0),
        CanonicalProfileOwner::Texture(index) => (&mut counts.textures, index, 0),
        CanonicalProfileOwner::TextureBinding(index) => (&mut counts.texture_bindings, index, 0),
        CanonicalProfileOwner::Material(index) => (&mut counts.materials, index, 0),
        CanonicalProfileOwner::LogicalMesh(index) => (&mut counts.logical_meshes, index, 0),
        CanonicalProfileOwner::Primitive(index) => (&mut counts.primitives, index, 0),
        CanonicalProfileOwner::Attribute(index) => (&mut counts.attributes, index, 0),
        CanonicalProfileOwner::MeshInstance(index) => (&mut counts.mesh_instances, index, 0),
        CanonicalProfileOwner::Skin(index) => (&mut counts.skins, index, 0),
        CanonicalProfileOwner::MorphDefinition(index) => (&mut counts.morph_definitions, index, 0),
        CanonicalProfileOwner::MorphDelta(index) => (&mut counts.morph_deltas, index, 0),
        CanonicalProfileOwner::AnimationClip(index) => (&mut counts.animation_clips, index, 0),
        CanonicalProfileOwner::AnimationChannel(index) => {
            (&mut counts.animation_channels, index, 0)
        }
    };
    if index != count.checked_add(base)? {
        return None;
    }
    *count = count.checked_add(1)?;
    Some(())
}

fn owner_exists(counts: ProfileOwnerClassCounts, owner: CanonicalProfileOwner) -> bool {
    match owner {
        CanonicalProfileOwner::Scene | CanonicalProfileOwner::SyntheticRoot => true,
        CanonicalProfileOwner::Node(index) => (1..=counts.nodes).contains(&index),
        CanonicalProfileOwner::Camera(index) => index < counts.cameras,
        CanonicalProfileOwner::Light(index) => index < counts.lights,
        CanonicalProfileOwner::Image(index) => index < counts.images,
        CanonicalProfileOwner::Sampler(index) => index < counts.samplers,
        CanonicalProfileOwner::Texture(index) => index < counts.textures,
        CanonicalProfileOwner::TextureBinding(index) => index < counts.texture_bindings,
        CanonicalProfileOwner::Material(index) => index < counts.materials,
        CanonicalProfileOwner::LogicalMesh(index) => index < counts.logical_meshes,
        CanonicalProfileOwner::Primitive(index) => index < counts.primitives,
        CanonicalProfileOwner::Attribute(index) => index < counts.attributes,
        CanonicalProfileOwner::MeshInstance(index) => index < counts.mesh_instances,
        CanonicalProfileOwner::Skin(index) => index < counts.skins,
        CanonicalProfileOwner::MorphDefinition(index) => index < counts.morph_definitions,
        CanonicalProfileOwner::MorphDelta(index) => index < counts.morph_deltas,
        CanonicalProfileOwner::AnimationClip(index) => index < counts.animation_clips,
        CanonicalProfileOwner::AnimationChannel(index) => index < counts.animation_channels,
    }
}

fn decode_owner(kind: u8, index: u32) -> Option<CanonicalProfileOwner> {
    Some(match kind {
        PROFILE_OWNER_SCENE if index == 0 => CanonicalProfileOwner::Scene,
        PROFILE_OWNER_SYNTHETIC_ROOT if index == 0 => CanonicalProfileOwner::SyntheticRoot,
        PROFILE_OWNER_NODE if index != 0 => CanonicalProfileOwner::Node(index),
        PROFILE_OWNER_CAMERA => CanonicalProfileOwner::Camera(index),
        PROFILE_OWNER_LIGHT => CanonicalProfileOwner::Light(index),
        PROFILE_OWNER_IMAGE => CanonicalProfileOwner::Image(index),
        PROFILE_OWNER_SAMPLER => CanonicalProfileOwner::Sampler(index),
        PROFILE_OWNER_TEXTURE => CanonicalProfileOwner::Texture(index),
        PROFILE_OWNER_TEXTURE_BINDING => CanonicalProfileOwner::TextureBinding(index),
        PROFILE_OWNER_MATERIAL => CanonicalProfileOwner::Material(index),
        PROFILE_OWNER_LOGICAL_MESH => CanonicalProfileOwner::LogicalMesh(index),
        PROFILE_OWNER_PRIMITIVE => CanonicalProfileOwner::Primitive(index),
        PROFILE_OWNER_ATTRIBUTE => CanonicalProfileOwner::Attribute(index),
        PROFILE_OWNER_MESH_INSTANCE => CanonicalProfileOwner::MeshInstance(index),
        PROFILE_OWNER_SKIN => CanonicalProfileOwner::Skin(index),
        PROFILE_OWNER_MORPH_DEFINITION => CanonicalProfileOwner::MorphDefinition(index),
        PROFILE_OWNER_MORPH_DELTA => CanonicalProfileOwner::MorphDelta(index),
        PROFILE_OWNER_ANIMATION_CLIP => CanonicalProfileOwner::AnimationClip(index),
        PROFILE_OWNER_ANIMATION_CHANNEL => CanonicalProfileOwner::AnimationChannel(index),
        _ => return None,
    })
}

fn require_range(cursor: u32, offset: u32, count: u32, expected_count: u32) -> Option<()> {
    (offset == cursor && count == expected_count).then_some(())
}

fn require_empty_text_range(cursor: u32, offset: u32, count: u32) -> Option<()> {
    require_range(cursor, offset, count, 0)
}

fn require_empty_wide_range(cursor: u32, offset: u32, count: u32) -> Option<()> {
    require_range(cursor, offset, count, 0)
}

fn require_scalar<T: PartialEq>(actual: Option<T>, expected: T) -> Option<()> {
    (actual? == expected).then_some(())
}

const fn encode_session_opaque_reason(reason: SessionOpaqueReason) -> u8 {
    match reason {
        SessionOpaqueReason::CallerLabel => PROFILE_SESSION_OPAQUE_CALLER_LABEL,
        SessionOpaqueReason::Generated => PROFILE_SESSION_OPAQUE_GENERATED,
        SessionOpaqueReason::UnregisteredResolver => PROFILE_SESSION_OPAQUE_UNREGISTERED_RESOLVER,
        SessionOpaqueReason::SessionFallback => PROFILE_SESSION_OPAQUE_SESSION_FALLBACK,
    }
}

fn decode_session_opaque_reason(value: u32) -> Option<SessionOpaqueReason> {
    Some(match u8::try_from(value).ok()? {
        PROFILE_SESSION_OPAQUE_CALLER_LABEL => SessionOpaqueReason::CallerLabel,
        PROFILE_SESSION_OPAQUE_GENERATED => SessionOpaqueReason::Generated,
        PROFILE_SESSION_OPAQUE_UNREGISTERED_RESOLVER => SessionOpaqueReason::UnregisteredResolver,
        PROFILE_SESSION_OPAQUE_SESSION_FALLBACK => SessionOpaqueReason::SessionFallback,
        _ => return None,
    })
}

const fn encode_source_object_kind(kind: SourceObjectKind) -> u8 {
    match kind {
        SourceObjectKind::Document => PROFILE_SOURCE_OBJECT_DOCUMENT,
        SourceObjectKind::Scene => PROFILE_SOURCE_OBJECT_SCENE,
        SourceObjectKind::Node => PROFILE_SOURCE_OBJECT_NODE,
        SourceObjectKind::Mesh => PROFILE_SOURCE_OBJECT_MESH,
        SourceObjectKind::Primitive => PROFILE_SOURCE_OBJECT_PRIMITIVE,
        SourceObjectKind::Material => PROFILE_SOURCE_OBJECT_MATERIAL,
        SourceObjectKind::Image => PROFILE_SOURCE_OBJECT_IMAGE,
        SourceObjectKind::Texture => PROFILE_SOURCE_OBJECT_TEXTURE,
        SourceObjectKind::Sampler => PROFILE_SOURCE_OBJECT_SAMPLER,
        SourceObjectKind::Skin => PROFILE_SOURCE_OBJECT_SKIN,
        SourceObjectKind::Animation => PROFILE_SOURCE_OBJECT_ANIMATION,
        SourceObjectKind::AnimationChannel => PROFILE_SOURCE_OBJECT_ANIMATION_CHANNEL,
        SourceObjectKind::Camera => PROFILE_SOURCE_OBJECT_CAMERA,
        SourceObjectKind::Light => PROFILE_SOURCE_OBJECT_LIGHT,
        SourceObjectKind::Vertex => PROFILE_SOURCE_OBJECT_VERTEX,
        SourceObjectKind::Face => PROFILE_SOURCE_OBJECT_FACE,
        SourceObjectKind::Element => PROFILE_SOURCE_OBJECT_ELEMENT,
        SourceObjectKind::Row => PROFILE_SOURCE_OBJECT_ROW,
        SourceObjectKind::Statement => PROFILE_SOURCE_OBJECT_STATEMENT,
        SourceObjectKind::Segment => PROFILE_SOURCE_OBJECT_SEGMENT,
    }
}

fn decode_source_object_kind(value: u32) -> Option<SourceObjectKind> {
    Some(match u8::try_from(value).ok()? {
        PROFILE_SOURCE_OBJECT_DOCUMENT => SourceObjectKind::Document,
        PROFILE_SOURCE_OBJECT_SCENE => SourceObjectKind::Scene,
        PROFILE_SOURCE_OBJECT_NODE => SourceObjectKind::Node,
        PROFILE_SOURCE_OBJECT_MESH => SourceObjectKind::Mesh,
        PROFILE_SOURCE_OBJECT_PRIMITIVE => SourceObjectKind::Primitive,
        PROFILE_SOURCE_OBJECT_MATERIAL => SourceObjectKind::Material,
        PROFILE_SOURCE_OBJECT_IMAGE => SourceObjectKind::Image,
        PROFILE_SOURCE_OBJECT_TEXTURE => SourceObjectKind::Texture,
        PROFILE_SOURCE_OBJECT_SAMPLER => SourceObjectKind::Sampler,
        PROFILE_SOURCE_OBJECT_SKIN => SourceObjectKind::Skin,
        PROFILE_SOURCE_OBJECT_ANIMATION => SourceObjectKind::Animation,
        PROFILE_SOURCE_OBJECT_ANIMATION_CHANNEL => SourceObjectKind::AnimationChannel,
        PROFILE_SOURCE_OBJECT_CAMERA => SourceObjectKind::Camera,
        PROFILE_SOURCE_OBJECT_LIGHT => SourceObjectKind::Light,
        PROFILE_SOURCE_OBJECT_VERTEX => SourceObjectKind::Vertex,
        PROFILE_SOURCE_OBJECT_FACE => SourceObjectKind::Face,
        PROFILE_SOURCE_OBJECT_ELEMENT => SourceObjectKind::Element,
        PROFILE_SOURCE_OBJECT_ROW => SourceObjectKind::Row,
        PROFILE_SOURCE_OBJECT_STATEMENT => SourceObjectKind::Statement,
        PROFILE_SOURCE_OBJECT_SEGMENT => SourceObjectKind::Segment,
        _ => return None,
    })
}

const fn encode_operation_kind(kind: &OperationKind) -> u8 {
    match kind {
        OperationKind::Generated => PROFILE_OPERATION_GENERATED,
        OperationKind::Default => PROFILE_OPERATION_DEFAULT,
        OperationKind::Normalize => PROFILE_OPERATION_NORMALIZE,
        OperationKind::Repair => PROFILE_OPERATION_REPAIR,
        OperationKind::Split => PROFILE_OPERATION_SPLIT,
        OperationKind::Merge => PROFILE_OPERATION_MERGE,
        OperationKind::Convert => PROFILE_OPERATION_CONVERT,
        OperationKind::Transform => PROFILE_OPERATION_TRANSFORM,
        OperationKind::Optimize => PROFILE_OPERATION_OPTIMIZE,
        OperationKind::Namespaced(_) => PROFILE_OPERATION_NAMESPACED,
    }
}

fn decode_operation_kind(value: u32) -> Option<()> {
    matches!(
        u8::try_from(value).ok()?,
        PROFILE_OPERATION_GENERATED
            | PROFILE_OPERATION_DEFAULT
            | PROFILE_OPERATION_NORMALIZE
            | PROFILE_OPERATION_REPAIR
            | PROFILE_OPERATION_SPLIT
            | PROFILE_OPERATION_MERGE
            | PROFILE_OPERATION_CONVERT
            | PROFILE_OPERATION_TRANSFORM
            | PROFILE_OPERATION_OPTIMIZE
    )
    .then_some(())
}

fn decode_digest_algorithm(value: u32) -> Option<DigestAlgorithm> {
    Some(match u8::try_from(value).ok()? {
        PROFILE_DIGEST_SHA256 => DigestAlgorithm::Sha256,
        PROFILE_DIGEST_BLAKE3 => DigestAlgorithm::Blake3,
        _ => return None,
    })
}
