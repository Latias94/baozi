use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, SequenceView, StoreValidationSession, StoredScalar,
};

use crate::SCENE_VALIDATION_POLL_INTERVAL;
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

#[derive(Clone, Copy)]
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
