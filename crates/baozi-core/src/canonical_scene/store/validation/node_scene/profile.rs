use baozi_operation::{
    CanonicalRecordRef, CanonicalRecordView, SequenceView, StoreValidationSession, StoredScalar,
};

use crate::SCENE_VALIDATION_POLL_INTERVAL;
use crate::canonical_scene::store::production::{
    CanonicalProfileOwner, ProvenanceLocalId, ProvenanceNodeAuthoring, ResourceIdentityStability,
    SessionOpaqueReason, ValidatedProfileBatch,
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

#[cfg(test)]
mod tests;

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
    for row in 0..usize::try_from(segment_count).ok()? {
        observe_work(1)?;
        let counts = resolve_segment_shape(session, segments, row)?;
        if counts.is_empty() {
            return None;
        }
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
    for row in 0..usize::try_from(segment_count).ok()? {
        observe_work(1)?;
        let counts = resolve_segment_shape(session, segments, row)?;
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
    let metadata = validate_metadata(
        session,
        metadata_ref,
        totals.provenance,
        owner_classes,
        expected,
        observe_work,
    )?;
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
) -> Option<SegmentCounts> {
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
    optional_sequence::<u32, ProfileLineageParentSchema>(
        session,
        segments,
        row,
        profile_segment_field::PARENTS,
        parents,
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
    Some(SegmentCounts {
        resources,
        sources,
        provenance,
        operations,
        evidence,
        parents,
        wide,
        text,
        digests,
    })
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
    let parents = required_sequence_slice::<u32, ProfileLineageParentSchema>(
        session,
        segments,
        row,
        profile_segment_field::PARENTS,
        counts.parents,
        1,
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

#[allow(clippy::too_many_arguments)]
fn validate_provenance<R>(
    provenance: &[u32],
    operations: &[u32],
    evidence: &[u32],
    parents: &[u32],
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
                let parent_end = parent_start.checked_add(usize::try_from(parent_count).ok()?)?;
                let parent_values = parents.get(parent_start..parent_end)?;
                let mut previous = None;
                let mut maximum_parent_depth = 0_u32;
                for (parent_index, parent) in parent_values.iter().copied().enumerate() {
                    observe_work(1)?;
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
    let mut attachment_values: Option<SequenceView<'_, u32>> = None;
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
        let reference = owners.sequence_ref::<u32, ProfileLineageAttachmentSchema>(
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
        let start = usize::try_from(offset).ok()?;
        let end = start.checked_add(usize::try_from(count).ok()?)?;
        let values = values.as_slice().get(start..end)?;
        let expected_values = expected.and_then(|expected| {
            expected
                .owner_provenance_at(owner_ordinal)
                .and_then(|(expected_owner, values)| (expected_owner == owner).then_some(values))
        });
        if expected_values.is_some_and(|expected| expected.len() != values.len()) {
            return None;
        }
        let mut previous_provenance = None;
        for (index, provenance) in values.iter().copied().enumerate() {
            observe_work(1)?;
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
    (values.len() == usize::try_from(attachment_cursor).ok()?).then_some((
        classes,
        attachment_cursor,
        attachment_ref?,
    ))
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

fn validate_metadata<R>(
    session: &mut StoreValidationSession<'_, '_>,
    directory_ref: CanonicalRecordRef<ProfileMetadataDirectorySchema>,
    provenance_count: u32,
    owner_classes: ProfileOwnerClassCounts,
    expected: Option<&ValidatedProfileBatch<'_, R>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<MetadataValidatedRefs> {
    observe_work(1)?;
    let directory = session.canonical_records(directory_ref)?;
    if directory.rows() != 1 || directory.fields() != PROFILE_METADATA_DIRECTORY_FIELD_COUNT {
        return None;
    }
    require_scalar(
        directory.scalar::<u32>(0, profile_metadata_directory_field::SCHEMA_REVISION),
        PROFILE_METADATA_SCHEMA_REVISION,
    )?;
    let entry_count = directory.scalar::<u32>(0, profile_metadata_directory_field::ENTRY_COUNT)?;
    let (entries_ref, entries) = optional_records::<ProfileMetadataEntryTableSchema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::ENTRIES,
        entry_count,
        PROFILE_METADATA_ENTRY_FIELD_COUNT,
    )?;
    let text_count = directory.scalar::<u32>(0, profile_metadata_directory_field::TEXT_COUNT)?;
    let (text_ref, text) = optional_utf8::<ProfileMetadataTextSchema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::TEXTS,
        text_count,
    )?;
    let byte_count = directory.scalar::<u32>(0, profile_metadata_directory_field::BYTE_COUNT)?;
    let (bytes_ref, bytes) = optional_bytes::<ProfileMetadataBytesSchema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::BYTES,
        byte_count,
    )?;
    let bool_count = directory.scalar::<u32>(0, profile_metadata_directory_field::BOOL_COUNT)?;
    let (bools_ref, bools) = optional_sequence::<u8, ProfileMetadataBoolSchema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::BOOL_VALUES,
        bool_count,
    )?;
    let i64_count = directory.scalar::<u32>(0, profile_metadata_directory_field::I64_COUNT)?;
    let (i64s_ref, i64s) = optional_sequence::<i64, ProfileMetadataI64Schema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::I64_VALUES,
        i64_count,
    )?;
    let u64_count = directory.scalar::<u32>(0, profile_metadata_directory_field::U64_COUNT)?;
    let (u64s_ref, u64s) = optional_sequence::<u64, ProfileMetadataU64Schema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::U64_VALUES,
        u64_count,
    )?;
    let f64_count = directory.scalar::<u32>(0, profile_metadata_directory_field::F64_COUNT)?;
    let (f64s_ref, f64s) = optional_sequence::<f64, ProfileMetadataF64Schema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::F64_VALUES,
        f64_count,
    )?;
    let f32_count = directory.scalar::<u32>(0, profile_metadata_directory_field::F32_COUNT)?;
    let (f32s_ref, f32s) = optional_sequence::<f32, ProfileMetadataF32Schema>(
        session,
        directory,
        0,
        profile_metadata_directory_field::F32_VALUES,
        f32_count,
    )?;
    if expected.is_some_and(|expected| {
        let shape = expected.retention_shape();
        entry_count != expected.metadata_entry_count()
            || text_count != shape.metadata_text_bytes()
            || byte_count != shape.metadata_payload_bytes()
            || bool_count != shape.metadata_bool_count()
            || i64_count != shape.metadata_i64_count()
            || u64_count != shape.metadata_u64_count()
            || f64_count != shape.metadata_f64_count()
            || f32_count != shape.metadata_f32_count()
    }) {
        return None;
    }

    let text_value = text.unwrap_or("");
    let byte_values = bytes.unwrap_or(&[]);
    let bool_values = bools.map(|value| value.as_slice()).unwrap_or(&[]);
    let i64_values = i64s.map(|value| value.as_slice()).unwrap_or(&[]);
    let u64_values = u64s.map(|value| value.as_slice()).unwrap_or(&[]);
    let f64_values = f64s.map(|value| value.as_slice()).unwrap_or(&[]);
    let f32_values = f32s.map(|value| value.as_slice()).unwrap_or(&[]);
    let entry_view = entries;
    let mut cursors = MetadataCursors::default();
    let mut previous_owner = None;
    let mut previous_key: Option<&str> = None;
    for row_index in 0..usize::try_from(entry_count).ok()? {
        observe_work(1)?;
        let row = entry_view?;
        let owner = decode_owner(
            row.scalar::<u8>(row_index, profile_metadata_entry_field::OWNER_KIND)?,
            row.scalar::<u32>(row_index, profile_metadata_entry_field::OWNER_INDEX)?,
        )?;
        if !owner_exists(owner_classes, owner) {
            return None;
        }
        let key = consume_text(
            text_value,
            &mut cursors.text,
            row.scalar::<u32>(row_index, profile_metadata_entry_field::KEY_OFFSET)?,
            row.scalar::<u32>(row_index, profile_metadata_entry_field::KEY_COUNT)?,
            true,
            expected
                .and_then(|expected| expected.metadata_entry(row_index))
                .map(|entry| entry.key().as_str()),
            observe_work,
        )?;
        if previous_owner.is_some_and(|previous| owner < previous)
            || previous_owner == Some(owner)
                && previous_key.is_some_and(|previous| previous.as_bytes() >= key.as_bytes())
        {
            return None;
        }
        previous_owner = Some(owner);
        previous_key = Some(key);
        let value_kind = row.scalar::<u8>(row_index, profile_metadata_entry_field::VALUE_KIND)?;
        let array_kind = row.scalar::<u8>(row_index, profile_metadata_entry_field::ARRAY_KIND)?;
        let logical_count = row.scalar::<u32>(
            row_index,
            profile_metadata_entry_field::LOGICAL_ELEMENT_COUNT,
        )?;
        let shape = metadata_shape(value_kind, array_kind, logical_count)?;
        let expected_entry = expected.and_then(|expected| expected.metadata_entry(row_index));
        if expected_entry.is_some_and(|entry| entry.owner() != owner) {
            return None;
        }
        let expected_shape = match expected_entry {
            Some(entry) => Some(expected_metadata_shape(entry.value())?),
            None => None,
        };
        if expected_shape.is_some_and(|expected| {
            expected.0 != value_kind || expected.1 != array_kind || expected.2 != logical_count
        }) {
            return None;
        }
        let auxiliary_count =
            row.scalar::<u32>(row_index, profile_metadata_entry_field::AUX_TEXT_COUNT)?;
        let expected_auxiliary = expected_entry.and_then(|entry| match entry.value() {
            ExtensionValueRef::String(value) => Some(value),
            ExtensionValueRef::Bytes { media_type, .. } => Some(media_type.as_str()),
            _ => None,
        });
        let auxiliary = consume_text(
            text_value,
            &mut cursors.text,
            row.scalar::<u32>(row_index, profile_metadata_entry_field::AUX_TEXT_OFFSET)?,
            auxiliary_count,
            shape.auxiliary_text && value_kind == PROFILE_METADATA_BYTES,
            expected_entry.and(expected_auxiliary),
            observe_work,
        )?;
        if shape.auxiliary_text
            != (value_kind == PROFILE_METADATA_STRING || value_kind == PROFILE_METADATA_BYTES)
            || !shape.auxiliary_text && auxiliary_count != 0
            || value_kind == PROFILE_METADATA_BYTES && auxiliary.is_empty()
        {
            return None;
        }
        let stored_byte_count =
            row.scalar::<u32>(row_index, profile_metadata_entry_field::BYTE_COUNT)?;
        let expected_payload = expected_entry.and_then(|entry| match entry.value() {
            ExtensionValueRef::Bytes { payload, .. } => Some(payload),
            _ => None,
        });
        consume_bytes(
            byte_values,
            &mut cursors.bytes,
            row.scalar::<u32>(row_index, profile_metadata_entry_field::BYTE_OFFSET)?,
            stored_byte_count,
            expected_payload,
            observe_work,
        )?;
        if shape.bytes != (value_kind == PROFILE_METADATA_BYTES)
            || !shape.bytes && stored_byte_count != 0
        {
            return None;
        }
        validate_entry_lane_refs(
            row, row_index, shape, text_ref, bytes_ref, bools_ref, i64s_ref, u64s_ref, f64s_ref,
            f32s_ref,
        )?;
        let scalar_offset =
            row.scalar::<u32>(row_index, profile_metadata_entry_field::SCALAR_OFFSET)?;
        let scalar_count =
            row.scalar::<u32>(row_index, profile_metadata_entry_field::SCALAR_COUNT)?;
        if scalar_count != shape.scalar_count {
            return None;
        }
        validate_metadata_payload(
            shape,
            scalar_offset,
            scalar_count,
            &mut cursors,
            bool_values,
            i64_values,
            u64_values,
            f64_values,
            f32_values,
            expected_entry.map(|entry| entry.value()),
            observe_work,
        )?;
        let provenance =
            row.scalar::<u32>(row_index, profile_metadata_entry_field::PROVENANCE_INDEX)?;
        if provenance >= provenance_count
            || expected_entry.is_some_and(|entry| provenance != entry.provenance().index())
        {
            return None;
        }
    }
    if cursors.text != text_count
        || cursors.bytes != byte_count
        || cursors.bools != bool_count
        || cursors.i64s != i64_count
        || cursors.u64s != u64_count
        || cursors.f64s != f64_count
        || cursors.f32s != f32_count
    {
        return None;
    }
    Some(MetadataValidatedRefs {
        entries: entries_ref,
        text: text_ref,
        bytes: bytes_ref,
        bools: bools_ref,
        i64s: i64s_ref,
        u64s: u64s_ref,
        f64s: f64s_ref,
        f32s: f32s_ref,
        entry_count,
        text_count,
        byte_count,
        bool_count,
        i64_count,
        u64_count,
        f64_count,
        f32_count,
    })
}

fn metadata_shape(value_kind: u8, array_kind: u8, logical_count: u32) -> Option<MetadataShape> {
    let scalar = |lane, scalar_count| MetadataShape {
        lane,
        scalar_count,
        auxiliary_text: false,
        bytes: false,
    };
    let shape = match value_kind {
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
    };
    Some(shape)
}

fn expected_metadata_shape(value: ExtensionValueRef<'_>) -> Option<(u8, u8, u32)> {
    Some(match value {
        ExtensionValueRef::Bool(_) => (PROFILE_METADATA_BOOL, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::I64(_) => (PROFILE_METADATA_I64, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::U64(_) => (PROFILE_METADATA_U64, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::F64(_) => (PROFILE_METADATA_F64, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::String(_) => (PROFILE_METADATA_STRING, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::Bytes { .. } => (PROFILE_METADATA_BYTES, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::Vec2(_) => (PROFILE_METADATA_VEC2, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::Vec3(_) => (PROFILE_METADATA_VEC3, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::Vec4(_) => (PROFILE_METADATA_VEC4, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::Color(_) => (PROFILE_METADATA_COLOR, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::Mat4(_) => (PROFILE_METADATA_MAT4, PROFILE_METADATA_ARRAY_BOOL, 1),
        ExtensionValueRef::DenseArray(values) => {
            let kind = match values {
                ExtensionArrayRef::Bool(_) => PROFILE_METADATA_ARRAY_BOOL,
                ExtensionArrayRef::I64(_) => PROFILE_METADATA_ARRAY_I64,
                ExtensionArrayRef::U64(_) => PROFILE_METADATA_ARRAY_U64,
                ExtensionArrayRef::F64(_) => PROFILE_METADATA_ARRAY_F64,
                ExtensionArrayRef::Vec2(_) => PROFILE_METADATA_ARRAY_VEC2,
                ExtensionArrayRef::Vec3(_) => PROFILE_METADATA_ARRAY_VEC3,
                ExtensionArrayRef::Vec4(_) => PROFILE_METADATA_ARRAY_VEC4,
                ExtensionArrayRef::Color(_) => PROFILE_METADATA_ARRAY_COLOR,
                ExtensionArrayRef::Mat4(_) => PROFILE_METADATA_ARRAY_MAT4,
            };
            (
                PROFILE_METADATA_DENSE_ARRAY,
                kind,
                u32::try_from(values.len()).ok()?,
            )
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_entry_lane_refs(
    row: CanonicalRecordView<'_>,
    index: usize,
    shape: MetadataShape,
    text: Option<CanonicalBlockRef<ProfileMetadataTextSchema>>,
    bytes: Option<CanonicalBlockRef<ProfileMetadataBytesSchema>>,
    bools: Option<CanonicalBlockRef<ProfileMetadataBoolSchema>>,
    i64s: Option<CanonicalBlockRef<ProfileMetadataI64Schema>>,
    u64s: Option<CanonicalBlockRef<ProfileMetadataU64Schema>>,
    f64s: Option<CanonicalBlockRef<ProfileMetadataF64Schema>>,
    f32s: Option<CanonicalBlockRef<ProfileMetadataF32Schema>>,
) -> Option<()> {
    require_utf8_lane(row, index, profile_metadata_entry_field::TEXTS, true, text)?;
    require_bytes_lane(
        row,
        index,
        profile_metadata_entry_field::BYTES,
        shape.bytes && row.scalar::<u32>(index, profile_metadata_entry_field::BYTE_COUNT)? != 0,
        bytes,
    )?;
    require_sequence_lane::<u8, ProfileMetadataBoolSchema>(
        row,
        index,
        profile_metadata_entry_field::BOOL_VALUES,
        shape.lane == MetadataLane::Bool && shape.scalar_count != 0,
        bools,
    )?;
    require_sequence_lane::<i64, ProfileMetadataI64Schema>(
        row,
        index,
        profile_metadata_entry_field::I64_VALUES,
        shape.lane == MetadataLane::I64 && shape.scalar_count != 0,
        i64s,
    )?;
    require_sequence_lane::<u64, ProfileMetadataU64Schema>(
        row,
        index,
        profile_metadata_entry_field::U64_VALUES,
        shape.lane == MetadataLane::U64 && shape.scalar_count != 0,
        u64s,
    )?;
    require_sequence_lane::<f64, ProfileMetadataF64Schema>(
        row,
        index,
        profile_metadata_entry_field::F64_VALUES,
        shape.lane == MetadataLane::F64 && shape.scalar_count != 0,
        f64s,
    )?;
    require_sequence_lane::<f32, ProfileMetadataF32Schema>(
        row,
        index,
        profile_metadata_entry_field::F32_VALUES,
        shape.lane == MetadataLane::F32 && shape.scalar_count != 0,
        f32s,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_metadata_payload(
    shape: MetadataShape,
    offset: u32,
    count: u32,
    cursors: &mut MetadataCursors,
    bools: &[u8],
    i64s: &[i64],
    u64s: &[u64],
    f64s: &[f64],
    f32s: &[f32],
    expected: Option<ExtensionValueRef<'_>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let count_usize = usize::try_from(count).ok()?;
    match shape.lane {
        MetadataLane::None => (offset == 0 && count == 0).then_some(()),
        MetadataLane::Bool => {
            if expected
                .is_some_and(|expected| metadata_bool_expected_len(expected) != Some(count_usize))
            {
                return None;
            }
            consume_validated_scalar_range(
                bools,
                &mut cursors.bools,
                offset,
                count,
                observe_work,
                |index, value| {
                    *value <= 1
                        && expected.is_none_or(|expected| {
                            metadata_bool_expected_at(expected, index) == Some(*value)
                        })
                },
            )?;
            Some(())
        }
        MetadataLane::I64 => {
            if expected
                .is_some_and(|expected| metadata_i64_expected_len(expected) != Some(count_usize))
            {
                return None;
            }
            consume_validated_scalar_range(
                i64s,
                &mut cursors.i64s,
                offset,
                count,
                observe_work,
                |index, value| {
                    expected.is_none_or(|expected| {
                        metadata_i64_expected_at(expected, index) == Some(*value)
                    })
                },
            )?;
            Some(())
        }
        MetadataLane::U64 => {
            if expected
                .is_some_and(|expected| metadata_u64_expected_len(expected) != Some(count_usize))
            {
                return None;
            }
            consume_validated_scalar_range(
                u64s,
                &mut cursors.u64s,
                offset,
                count,
                observe_work,
                |index, value| {
                    expected.is_none_or(|expected| {
                        metadata_u64_expected_at(expected, index) == Some(*value)
                    })
                },
            )?;
            Some(())
        }
        MetadataLane::F64 => {
            if expected
                .is_some_and(|expected| metadata_f64_expected_len(expected) != Some(count_usize))
            {
                return None;
            }
            consume_validated_scalar_range(
                f64s,
                &mut cursors.f64s,
                offset,
                count,
                observe_work,
                |index, value| {
                    value.is_finite()
                        && expected.is_none_or(|expected| {
                            metadata_f64_expected_at(expected, index)
                                .is_some_and(|expected| expected.to_bits() == value.to_bits())
                        })
                },
            )?;
            Some(())
        }
        MetadataLane::F32 => {
            if expected
                .is_some_and(|expected| metadata_f32_expected_len(expected) != Some(count_usize))
            {
                return None;
            }
            consume_validated_scalar_range(
                f32s,
                &mut cursors.f32s,
                offset,
                count,
                observe_work,
                |index, value| {
                    value.is_finite()
                        && expected.is_none_or(|expected| {
                            metadata_f32_expected_at(expected, index)
                                .is_some_and(|expected| expected.to_bits() == value.to_bits())
                        })
                },
            )?;
            Some(())
        }
    }
}

fn metadata_bool_expected_len(expected: ExtensionValueRef<'_>) -> Option<usize> {
    match expected {
        ExtensionValueRef::Bool(_) => Some(1),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::Bool(values)) => Some(values.len()),
        _ => None,
    }
}

fn metadata_bool_expected_at(expected: ExtensionValueRef<'_>, index: usize) -> Option<u8> {
    match expected {
        ExtensionValueRef::Bool(value) => (index == 0).then_some(u8::from(value)),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::Bool(values)) => {
            values.get(index).copied().map(u8::from)
        }
        _ => None,
    }
}

fn metadata_i64_expected_len(expected: ExtensionValueRef<'_>) -> Option<usize> {
    match expected {
        ExtensionValueRef::I64(_) => Some(1),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::I64(values)) => Some(values.len()),
        _ => None,
    }
}

fn metadata_i64_expected_at(expected: ExtensionValueRef<'_>, index: usize) -> Option<i64> {
    match expected {
        ExtensionValueRef::I64(value) => (index == 0).then_some(value),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::I64(values)) => values.get(index).copied(),
        _ => None,
    }
}

fn metadata_u64_expected_len(expected: ExtensionValueRef<'_>) -> Option<usize> {
    match expected {
        ExtensionValueRef::U64(_) => Some(1),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::U64(values)) => Some(values.len()),
        _ => None,
    }
}

fn metadata_u64_expected_at(expected: ExtensionValueRef<'_>, index: usize) -> Option<u64> {
    match expected {
        ExtensionValueRef::U64(value) => (index == 0).then_some(value),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::U64(values)) => values.get(index).copied(),
        _ => None,
    }
}

fn metadata_f64_expected_len(expected: ExtensionValueRef<'_>) -> Option<usize> {
    match expected {
        ExtensionValueRef::F64(_) => Some(1),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::F64(values)) => Some(values.len()),
        _ => None,
    }
}

fn metadata_f64_expected_at(expected: ExtensionValueRef<'_>, index: usize) -> Option<f64> {
    match expected {
        ExtensionValueRef::F64(value) => (index == 0).then_some(value),
        ExtensionValueRef::DenseArray(ExtensionArrayRef::F64(values)) => values.get(index).copied(),
        _ => None,
    }
}

fn metadata_f32_expected_len(expected: ExtensionValueRef<'_>) -> Option<usize> {
    match expected {
        ExtensionValueRef::Vec2(_) => Some(2),
        ExtensionValueRef::Vec3(_) => Some(3),
        ExtensionValueRef::Vec4(_) | ExtensionValueRef::Color(_) => Some(4),
        ExtensionValueRef::Mat4(_) => Some(16),
        ExtensionValueRef::DenseArray(values) => {
            let (count, width) = match values {
                ExtensionArrayRef::Vec2(values) => (values.len(), 2),
                ExtensionArrayRef::Vec3(values) => (values.len(), 3),
                ExtensionArrayRef::Vec4(values) => (values.len(), 4),
                ExtensionArrayRef::Color(values) => (values.len(), 4),
                ExtensionArrayRef::Mat4(values) => (values.len(), 16),
                _ => return None,
            };
            count.checked_mul(width)
        }
        _ => None,
    }
}

fn metadata_f32_expected_at(expected: ExtensionValueRef<'_>, index: usize) -> Option<f32> {
    match expected {
        ExtensionValueRef::Vec2(value) => [value.x, value.y].get(index).copied(),
        ExtensionValueRef::Vec3(value) => [value.x, value.y, value.z].get(index).copied(),
        ExtensionValueRef::Vec4(value) => [value.x, value.y, value.z, value.w].get(index).copied(),
        ExtensionValueRef::Color(value) => [value.r, value.g, value.b, value.a].get(index).copied(),
        ExtensionValueRef::Mat4(value) => value
            .cols
            .get(index.checked_div(4)?)?
            .get(index.checked_rem(4)?)
            .copied(),
        ExtensionValueRef::DenseArray(values) => match values {
            ExtensionArrayRef::Vec2(values) => {
                let value = values.get(index.checked_div(2)?)?;
                [value.x, value.y].get(index.checked_rem(2)?).copied()
            }
            ExtensionArrayRef::Vec3(values) => {
                let value = values.get(index.checked_div(3)?)?;
                [value.x, value.y, value.z]
                    .get(index.checked_rem(3)?)
                    .copied()
            }
            ExtensionArrayRef::Vec4(values) => {
                let value = values.get(index.checked_div(4)?)?;
                [value.x, value.y, value.z, value.w]
                    .get(index.checked_rem(4)?)
                    .copied()
            }
            ExtensionArrayRef::Color(values) => {
                let value = values.get(index.checked_div(4)?)?;
                [value.r, value.g, value.b, value.a]
                    .get(index.checked_rem(4)?)
                    .copied()
            }
            ExtensionArrayRef::Mat4(values) => {
                let value = values.get(index.checked_div(16)?)?;
                let cell = index.checked_rem(16)?;
                value
                    .cols
                    .get(cell.checked_div(4)?)?
                    .get(cell.checked_rem(4)?)
                    .copied()
            }
            _ => None,
        },
        _ => None,
    }
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

fn optional_records<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
    fields: usize,
) -> Option<(
    Option<CanonicalBlockRef<Tag>>,
    Option<CanonicalRecordView<'storage>>,
)>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::RecordBlock>,
{
    if count == 0 {
        require_absent(record, row, field)?;
        return Some((None, None));
    }
    let reference = record.record_ref::<Tag>(row, field)?;
    let view = session.canonical_records(reference)?;
    (view.rows() == usize::try_from(count).ok()? && view.fields() == fields)
        .then_some((Some(CanonicalBlockRef::from_store(reference)), Some(view)))
}

fn required_records<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
    fields: usize,
) -> Option<(CanonicalBlockRef<Tag>, CanonicalRecordView<'storage>)>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::RecordBlock>,
{
    let (reference, view) = optional_records(session, record, row, field, count, fields)?;
    Some((reference?, view?))
}

fn optional_sequence<'storage, T, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
) -> Option<OptionalCanonicalSequence<'storage, T, Tag>>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = baozi_operation::ScalarSequence<T>>,
{
    if count == 0 {
        require_absent(record, row, field)?;
        return Some((None, None));
    }
    let reference = record.sequence_ref::<T, Tag>(row, field)?;
    let view = session.canonical_sequence(reference)?;
    (view.len() == usize::try_from(count).ok()?)
        .then_some((Some(CanonicalBlockRef::from_store(reference)), Some(view)))
}

fn optional_utf8<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
) -> Option<OptionalCanonicalValue<'storage, Tag, str>>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::Utf8Block>,
{
    if count == 0 {
        require_absent(record, row, field)?;
        return Some((None, None));
    }
    let reference = record.utf8_ref::<Tag>(row, field)?;
    let value = session.canonical_utf8(reference)?;
    (value.len() == usize::try_from(count).ok()?)
        .then_some((Some(CanonicalBlockRef::from_store(reference)), Some(value)))
}

fn optional_bytes<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
) -> Option<OptionalCanonicalValue<'storage, Tag, [u8]>>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::ByteBlock>,
{
    if count == 0 {
        require_absent(record, row, field)?;
        return Some((None, None));
    }
    let reference = record.bytes_ref::<Tag>(row, field)?;
    let value = session.canonical_bytes(reference)?;
    (value.len() == usize::try_from(count).ok()?)
        .then_some((Some(CanonicalBlockRef::from_store(reference)), Some(value)))
}

fn required_sequence_slice<'storage, T, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    logical_count: u32,
    stride: usize,
) -> Option<&'storage [T]>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = baozi_operation::ScalarSequence<T>>,
{
    let physical = logical_count.checked_mul(u32::try_from(stride).ok()?)?;
    let (_, view) = optional_sequence::<T, Tag>(session, record, row, field, physical)?;
    Some(view.map(|view| view.as_slice()).unwrap_or(&[]))
}

fn required_utf8<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
) -> Option<&'storage str>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::Utf8Block>,
{
    let (_, value) = optional_utf8::<Tag>(session, record, row, field, count)?;
    Some(value.unwrap_or(""))
}

fn required_bytes<'storage, Tag>(
    session: &mut StoreValidationSession<'_, 'storage>,
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    count: u32,
) -> Option<&'storage [u8]>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::ByteBlock>,
{
    let (_, value) = optional_bytes::<Tag>(session, record, row, field, count)?;
    Some(value.unwrap_or(&[]))
}

fn require_absent(record: CanonicalRecordView<'_>, row: usize, field: usize) -> Option<()> {
    require_scalar(record.scalar::<u8>(row, field), PROFILE_ABSENT_REFERENCE)
}

fn require_utf8_lane<Tag>(
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    used: bool,
    expected: Option<CanonicalBlockRef<Tag>>,
) -> Option<()>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::Utf8Block>,
{
    if !used {
        return require_absent(record, row, field);
    }
    (CanonicalBlockRef::from_store(record.utf8_ref::<Tag>(row, field)?) == expected?).then_some(())
}

fn require_bytes_lane<Tag>(
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    used: bool,
    expected: Option<CanonicalBlockRef<Tag>>,
) -> Option<()>
where
    Tag: CanonicalSchema<StoreKind = baozi_operation::ByteBlock>,
{
    if !used {
        return require_absent(record, row, field);
    }
    (CanonicalBlockRef::from_store(record.bytes_ref::<Tag>(row, field)?) == expected?).then_some(())
}

fn require_sequence_lane<T, Tag>(
    record: CanonicalRecordView<'_>,
    row: usize,
    field: usize,
    used: bool,
    expected: Option<CanonicalBlockRef<Tag>>,
) -> Option<()>
where
    T: StoredScalar,
    Tag: CanonicalSchema<StoreKind = baozi_operation::ScalarSequence<T>>,
{
    if !used {
        return require_absent(record, row, field);
    }
    (CanonicalBlockRef::from_store(record.sequence_ref::<T, Tag>(row, field)?) == expected?)
        .then_some(())
}

fn scalar_row(values: &[u32], row: u32, stride: usize) -> Option<&[u32]> {
    let start = usize::try_from(row).ok()?.checked_mul(stride)?;
    values.get(start..start.checked_add(stride)?)
}

fn observe_and_match_bytes(
    value: &[u8],
    expected: Option<&[u8]>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let chunk_size = usize::try_from(SCENE_VALIDATION_POLL_INTERVAL)
        .ok()
        .filter(|chunk_size| *chunk_size != 0)?;
    let mut matches = expected.is_none_or(|expected| expected.len() == value.len());
    for (offset, chunk) in value.chunks(chunk_size).enumerate() {
        observe_work(u64::try_from(chunk.len()).ok()?)?;
        if let Some(expected) = expected {
            let start = offset.checked_mul(chunk_size)?;
            let end = start.checked_add(chunk.len())?;
            matches &= expected.get(start..end) == Some(chunk);
        }
    }
    matches.then_some(())
}

#[allow(clippy::too_many_arguments)]
fn consume_text<'storage>(
    text: &'storage str,
    cursor: &mut u32,
    offset: u32,
    count: u32,
    nonempty: bool,
    expected: Option<&str>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<&'storage str> {
    if offset != *cursor || nonempty && count == 0 {
        return None;
    }
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(count).ok()?)?;
    let value = text.get(start..end)?;
    observe_and_match_bytes(value.as_bytes(), expected.map(str::as_bytes), observe_work)?;
    *cursor = cursor.checked_add(count)?;
    Some(value)
}

#[allow(clippy::too_many_arguments)]
fn consume_optional_text<'storage>(
    text: &'storage str,
    cursor: &mut u32,
    presence: u32,
    offset: u32,
    count: u32,
    expected: Option<Option<&str>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<Option<&'storage str>> {
    match presence {
        0 => {
            require_empty_text_range(*cursor, offset, count)?;
            if expected.is_some_and(|expected| expected.is_some()) {
                return None;
            }
            Some(None)
        }
        1 => {
            if expected.is_some_and(|expected| expected.is_none()) {
                return None;
            }
            consume_text(
                text,
                cursor,
                offset,
                count,
                true,
                expected.flatten(),
                observe_work,
            )
            .map(Some)
        }
        _ => None,
    }
}

fn consume_bytes<'storage>(
    bytes: &'storage [u8],
    cursor: &mut u32,
    offset: u32,
    count: u32,
    expected: Option<&[u8]>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<&'storage [u8]> {
    if offset != *cursor {
        return None;
    }
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(count).ok()?)?;
    let value = bytes.get(start..end)?;
    observe_and_match_bytes(value, expected, observe_work)?;
    *cursor = cursor.checked_add(count)?;
    Some(value)
}

fn consume_required_digest(
    bytes: &[u8],
    cursor: &mut u32,
    algorithm: u32,
    offset: u32,
    count: u32,
    expected: Option<(DigestAlgorithm, [u8; 32])>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    let algorithm = decode_digest_algorithm(algorithm)?;
    if count != DIGEST_BYTES || expected.is_some_and(|expected| expected.0 != algorithm) {
        return None;
    }
    consume_bytes(
        bytes,
        cursor,
        offset,
        count,
        expected.as_ref().map(|expected| expected.1.as_slice()),
        observe_work,
    )?;
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn consume_optional_digest(
    bytes: &[u8],
    cursor: &mut u32,
    presence: u32,
    algorithm: u32,
    offset: u32,
    count: u32,
    expected: Option<Option<(DigestAlgorithm, [u8; 32])>>,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<()> {
    match presence {
        0 => {
            if algorithm != 0
                || offset != *cursor
                || count != 0
                || expected.is_some_and(|expected| expected.is_some())
            {
                return None;
            }
        }
        1 => {
            if expected.is_some_and(|expected| expected.is_none()) {
                return None;
            }
            consume_required_digest(
                bytes,
                cursor,
                algorithm,
                offset,
                count,
                expected.flatten(),
                observe_work,
            )?;
        }
        _ => return None,
    }
    Some(())
}

fn consume_wide<'storage>(
    values: &'storage [u64],
    cursor: &mut u32,
    offset: u32,
    count: u32,
    required_count: u32,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
) -> Option<&'storage [u64]> {
    if count != required_count || offset != *cursor {
        return None;
    }
    observe_work(u64::from(count))?;
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(count).ok()?)?;
    let result = values.get(start..end)?;
    *cursor = cursor.checked_add(count)?;
    Some(result)
}

fn consume_validated_scalar_range<'storage, T: StoredScalar>(
    values: &'storage [T],
    cursor: &mut u32,
    offset: u32,
    count: u32,
    observe_work: &mut impl FnMut(u64) -> Option<()>,
    mut validate: impl FnMut(usize, &T) -> bool,
) -> Option<&'storage [T]> {
    if offset != *cursor {
        return None;
    }
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(count).ok()?)?;
    let result = values.get(start..end)?;
    let chunk_size = usize::try_from(SCENE_VALIDATION_POLL_INTERVAL)
        .ok()
        .filter(|chunk_size| *chunk_size != 0)?;
    let mut valid = true;
    for (chunk_ordinal, chunk) in result.chunks(chunk_size).enumerate() {
        observe_work(u64::try_from(chunk.len()).ok()?)?;
        let chunk_start = chunk_ordinal.checked_mul(chunk_size)?;
        for (index, value) in chunk.iter().enumerate() {
            valid &= validate(chunk_start.checked_add(index)?, value);
        }
    }
    if !valid {
        return None;
    }
    *cursor = cursor.checked_add(count)?;
    Some(result)
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
