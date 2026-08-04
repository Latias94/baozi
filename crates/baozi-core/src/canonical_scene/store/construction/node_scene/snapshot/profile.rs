use crate::canonical_scene::store::production::{CanonicalProfileOwner, SessionOpaqueReason};
use crate::provenance::{DigestAlgorithm, SourceObjectKind};

use super::super::semantic::{
    NodeSceneProfileCursor, NodeSceneProfileDigest, NodeSceneProfileEvidence,
    NodeSceneProfileMetadataArray, NodeSceneProfileMetadataArrayKind,
    NodeSceneProfileMetadataEntry, NodeSceneProfileMetadataValue, NodeSceneProfileOperation,
    NodeSceneProfileOperationKind, NodeSceneProfileOwner, NodeSceneProfileProvenance,
    NodeSceneProfileProvenanceIndices, NodeSceneProfileProvenanceKind, NodeSceneProfileResource,
    NodeSceneProfileResourceStability, NodeSceneProfileSource, NodeSceneProfileSourceLocator,
    NodeSceneProfileStep, NodeSceneSemanticResult,
};
use super::{
    CanonicalDigestEncoder, NodeSceneSnapshotError, PRESENCE_ABSENT, PRESENCE_PRESENT, PreviewSink,
    SnapshotWorkMeter,
};

const TAG_PROFILE: u8 = 200;
const TAG_COLLECTION: u8 = 201;
const TAG_ROW: u8 = 202;
const TAG_FIELD: u8 = 203;
const TAG_KIND: u8 = 204;
const TAG_INDEX: u8 = 205;
const TAG_PRESENCE: u8 = 206;
const TAG_COUNT: u8 = 207;
const TAG_U8: u8 = 208;
const TAG_U32: u8 = 209;
const TAG_U64: u8 = 210;
const TAG_I64: u8 = 211;
const TAG_F32_BITS: u8 = 212;
const TAG_F64_BITS: u8 = 213;
const TAG_BYTES: u8 = 214;
const TAG_REFERENCE: u8 = 215;

const PROFILE_PAYLOAD_CHUNK_BYTES: usize = 4 * 1024;

const COLLECTION_RESOURCES: u8 = 1;
const COLLECTION_SOURCES: u8 = 2;
const COLLECTION_PROVENANCE: u8 = 3;
const COLLECTION_EVIDENCE: u8 = 4;
const COLLECTION_OWNERS: u8 = 5;
const COLLECTION_METADATA: u8 = 6;

const ROW_RESOURCE: u8 = 1;
const ROW_SOURCE: u8 = 2;
const ROW_PROVENANCE: u8 = 3;
const ROW_EVIDENCE: u8 = 4;
const ROW_OWNER: u8 = 5;
const ROW_METADATA: u8 = 6;

const RESOURCE_IDENTITY: u8 = 0;
const RESOURCE_STABILITY_KIND: u8 = 1;
const RESOURCE_STABILITY_OWNER: u8 = 2;
const RESOURCE_STABILITY_SCHEMA: u8 = 3;
const RESOURCE_STABILITY_VERSION: u8 = 4;
const RESOURCE_OPAQUE_REASON: u8 = 5;

const SOURCE_RESOURCE: u8 = 0;
const SOURCE_FORMAT: u8 = 1;
const SOURCE_LOCATOR_KIND: u8 = 2;
const SOURCE_OBJECT_KIND: u8 = 3;
const SOURCE_OBJECT_INDEX: u8 = 4;
const SOURCE_LOCATOR_SCHEMA: u8 = 5;
const SOURCE_LOCATOR_SCHEMA_VERSION: u8 = 6;
const SOURCE_LOCATOR_VALUE: u8 = 7;
const SOURCE_DISPLAY_NAME: u8 = 8;
const SOURCE_CONTENT_DIGEST: u8 = 9;

const PROVENANCE_DEPTH: u8 = 0;
const PROVENANCE_PARENT_OFFSET: u8 = 1;
const PROVENANCE_PARENTS: u8 = 2;
const PROVENANCE_KIND: u8 = 3;
const PROVENANCE_SOURCE: u8 = 4;
const PROVENANCE_OPERATION_INDEX: u8 = 5;
const PROVENANCE_OPERATION_KIND: u8 = 6;
const PROVENANCE_OPERATION_NAME: u8 = 7;
const PROVENANCE_OPTIONS_DIGEST: u8 = 8;
const PROVENANCE_EVIDENCE_OFFSET: u8 = 9;
const PROVENANCE_EVIDENCE_COUNT: u8 = 10;

const EVIDENCE_CODE: u8 = 0;
const EVIDENCE_OCCURRENCE_COUNT: u8 = 1;
const EVIDENCE_DIGEST: u8 = 2;

const OWNER_KIND: u8 = 0;
const OWNER_INDEX: u8 = 1;
const OWNER_ATTACHMENT_OFFSET: u8 = 2;
const OWNER_ATTACHMENTS: u8 = 3;

const METADATA_OWNER_KIND: u8 = 0;
const METADATA_OWNER_INDEX: u8 = 1;
const METADATA_KEY: u8 = 2;
const METADATA_VALUE_KIND: u8 = 3;
const METADATA_ARRAY_ELEMENT_KIND: u8 = 4;
const METADATA_LOGICAL_ELEMENT_COUNT: u8 = 5;
const METADATA_VALUE: u8 = 6;
const METADATA_MEDIA_TYPE: u8 = 7;
const METADATA_PROVENANCE: u8 = 8;
const METADATA_ARRAY_STORAGE_KIND: u8 = 9;

const PROFILE_METADATA_TEXT_LANE: u8 = 0;
const PROFILE_METADATA_BYTE_LANE: u8 = 1;
const PROFILE_METADATA_BOOL_LANE: u8 = 2;
const PROFILE_METADATA_I64_LANE: u8 = 3;
const PROFILE_METADATA_U64_LANE: u8 = 4;
const PROFILE_METADATA_F64_LANE: u8 = 5;
const PROFILE_METADATA_F32_LANE: u8 = 6;

#[derive(Clone, Copy)]
pub(super) struct ProfileCollectionCounts {
    pub(super) resources: usize,
    pub(super) sources: usize,
    pub(super) provenance_nodes: usize,
    pub(super) operations: usize,
    pub(super) evidence: usize,
    pub(super) owners: usize,
    pub(super) attachments: usize,
    pub(super) metadata_entries: usize,
}

#[derive(Clone, Copy, Default)]
pub(super) struct ProfilePayloadTotals {
    pub(super) parent_references: u64,
    pub(super) wide_values: u64,
    pub(super) lineage_text_bytes: u64,
    pub(super) lineage_digest_bytes: u64,
    pub(super) metadata_text_bytes: u64,
    pub(super) metadata_payload_bytes: u64,
    pub(super) metadata_bool_scalars: u64,
    pub(super) metadata_i64_scalars: u64,
    pub(super) metadata_u64_scalars: u64,
    pub(super) metadata_f64_scalars: u64,
    pub(super) metadata_f32_scalars: u64,
}

#[allow(clippy::too_many_lines)]
pub(super) fn write_profile(
    cursor: &mut NodeSceneProfileCursor<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(ProfileCollectionCounts, ProfilePayloadTotals), NodeSceneSnapshotError> {
    let counts = ProfileCollectionCounts {
        resources: cursor.resource_count(),
        sources: cursor.source_count(),
        provenance_nodes: cursor.provenance_count(),
        operations: cursor.operation_count(),
        evidence: cursor.evidence_count(),
        owners: cursor.owner_count(),
        attachments: cursor.attachment_count(),
        metadata_entries: cursor.metadata_entry_count(),
    };
    let lanes = cursor.metadata_lanes();
    let totals = ProfilePayloadTotals {
        parent_references: count_as_u64(cursor.parent_count(), "profile parent references")?,
        wide_values: count_as_u64(cursor.wide_count(), "profile wide values")?,
        lineage_text_bytes: count_as_u64(cursor.lineage_text_bytes(), "profile lineage text")?,
        lineage_digest_bytes: count_as_u64(
            cursor.lineage_digest_bytes(),
            "profile lineage digests",
        )?,
        metadata_text_bytes: count_as_u64(
            lanes.text().map_or(0, str::len),
            "profile metadata text",
        )?,
        metadata_payload_bytes: count_as_u64(
            lanes.bytes().map_or(0, <[u8]>::len),
            "profile metadata bytes",
        )?,
        metadata_bool_scalars: count_as_u64(
            lanes.bools().map_or(0, <[u8]>::len),
            "profile metadata bool scalars",
        )?,
        metadata_i64_scalars: count_as_u64(
            lanes.i64s().map_or(0, <[i64]>::len),
            "profile metadata i64 scalars",
        )?,
        metadata_u64_scalars: count_as_u64(
            lanes.u64s().map_or(0, <[u64]>::len),
            "profile metadata u64 scalars",
        )?,
        metadata_f64_scalars: count_as_u64(
            lanes.f64s().map_or(0, <[f64]>::len),
            "profile metadata f64 scalars",
        )?,
        metadata_f32_scalars: count_as_u64(
            lanes.f32s().map_or(0, <[f32]>::len),
            "profile metadata f32 scalars",
        )?,
    };

    work.observe()?;
    transcript.write_tag(TAG_PROFILE)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    preview.push_str("profile(present)\n");

    write_metadata_lane_shape(
        transcript,
        work,
        PROFILE_METADATA_TEXT_LANE,
        lanes.text().map(str::len),
    )?;
    write_metadata_lane_shape(
        transcript,
        work,
        PROFILE_METADATA_BYTE_LANE,
        lanes.bytes().map(<[u8]>::len),
    )?;
    write_metadata_lane_shape(
        transcript,
        work,
        PROFILE_METADATA_BOOL_LANE,
        lanes.bools().map(<[u8]>::len),
    )?;
    write_metadata_lane_shape(
        transcript,
        work,
        PROFILE_METADATA_I64_LANE,
        lanes.i64s().map(<[i64]>::len),
    )?;
    write_metadata_lane_shape(
        transcript,
        work,
        PROFILE_METADATA_U64_LANE,
        lanes.u64s().map(<[u64]>::len),
    )?;
    write_metadata_lane_shape(
        transcript,
        work,
        PROFILE_METADATA_F64_LANE,
        lanes.f64s().map(<[f64]>::len),
    )?;
    write_metadata_lane_shape(
        transcript,
        work,
        PROFILE_METADATA_F32_LANE,
        lanes.f32s().map(<[f32]>::len),
    )?;

    write_resources(cursor, counts.resources, transcript, preview, work)?;
    write_sources(cursor, counts.sources, transcript, preview, work)?;
    write_provenance(cursor, counts.provenance_nodes, transcript, preview, work)?;
    write_evidence(cursor, counts.evidence, transcript, preview, work)?;
    write_owners(cursor, counts, transcript, preview, work)?;
    write_metadata(cursor, counts, transcript, preview, work)?;
    preview.push_str("profile_end\n");
    Ok((counts, totals))
}

fn next_lineage_item<T>(
    work: &mut SnapshotWorkMeter<'_>,
    mut step: impl FnMut() -> NodeSceneSemanticResult<NodeSceneProfileStep<T>>,
) -> Result<Option<T>, NodeSceneSnapshotError> {
    loop {
        // Admission and cancellation polling happen before either one physical
        // segment checkpoint or one canonical row is decoded.
        work.observe()?;
        match step().map_err(super::map_snapshot_invariant)? {
            NodeSceneProfileStep::Checkpoint => {}
            NodeSceneProfileStep::Item(item) => return Ok(Some(item)),
            NodeSceneProfileStep::End => return Ok(None),
        }
    }
}

fn write_resources(
    cursor: &mut NodeSceneProfileCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_collection(
        transcript,
        preview,
        work,
        COLLECTION_RESOURCES,
        "profile_resources",
        true,
        count,
    )?;
    for _ in 0..count {
        let resource = next_lineage_item(work, || cursor.step_resource())?.ok_or(
            NodeSceneSnapshotError::InconsistentView {
                reason: "profile resource collection ended before declared total",
            },
        )?;
        write_resource(resource, transcript, preview, work)?;
    }
    if next_lineage_item(work, || cursor.step_resource())?.is_some() {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "profile resource collection exceeds its declared total",
        });
    }
    Ok(())
}

fn write_resource(
    resource: NodeSceneProfileResource<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_row(transcript, ROW_RESOURCE, resource.index())?;
    write_bytes_field(
        transcript,
        work,
        RESOURCE_IDENTITY,
        resource.identity().as_bytes(),
    )?;
    match resource.stability() {
        NodeSceneProfileResourceStability::Stable {
            owner,
            schema,
            schema_version,
        } => {
            write_u8_field(transcript, work, RESOURCE_STABILITY_KIND, 0)?;
            write_bytes_field(transcript, work, RESOURCE_STABILITY_OWNER, owner.as_bytes())?;
            write_bytes_field(
                transcript,
                work,
                RESOURCE_STABILITY_SCHEMA,
                schema.as_bytes(),
            )?;
            write_u32_field(transcript, work, RESOURCE_STABILITY_VERSION, schema_version)?;
        }
        NodeSceneProfileResourceStability::SessionOpaque(reason) => {
            write_u8_field(transcript, work, RESOURCE_STABILITY_KIND, 1)?;
            write_u8_field(
                transcript,
                work,
                RESOURCE_OPAQUE_REASON,
                encode_opaque_reason(reason),
            )?;
        }
    }
    let render = preview.begin_geometry_item("profile_resource", resource.index())?;
    preview.geometry_field_usize(render, "identity_bytes", resource.identity().len());
    preview.end_geometry_item(render);
    Ok(())
}

fn write_sources(
    cursor: &mut NodeSceneProfileCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_collection(
        transcript,
        preview,
        work,
        COLLECTION_SOURCES,
        "profile_sources",
        true,
        count,
    )?;
    for _ in 0..count {
        let source = next_lineage_item(work, || cursor.step_source())?.ok_or(
            NodeSceneSnapshotError::InconsistentView {
                reason: "profile source collection ended before declared total",
            },
        )?;
        write_source(source, transcript, preview, work)?;
    }
    if next_lineage_item(work, || cursor.step_source())?.is_some() {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "profile source collection exceeds its declared total",
        });
    }
    Ok(())
}

fn write_source(
    source: NodeSceneProfileSource<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_row(transcript, ROW_SOURCE, source.index())?;
    write_usize_field(transcript, work, SOURCE_RESOURCE, source.resource())?;
    write_bytes_field(transcript, work, SOURCE_FORMAT, source.format().as_bytes())?;
    match source.locator() {
        NodeSceneProfileSourceLocator::Indexed { kind, index } => {
            write_u8_field(transcript, work, SOURCE_LOCATOR_KIND, 0)?;
            write_u8_field(
                transcript,
                work,
                SOURCE_OBJECT_KIND,
                encode_source_object_kind(kind),
            )?;
            write_u64_field(transcript, work, SOURCE_OBJECT_INDEX, index)?;
        }
        NodeSceneProfileSourceLocator::Namespaced {
            schema,
            schema_version,
            value,
        } => {
            write_u8_field(transcript, work, SOURCE_LOCATOR_KIND, 1)?;
            write_bytes_field(transcript, work, SOURCE_LOCATOR_SCHEMA, schema.as_bytes())?;
            write_u32_field(
                transcript,
                work,
                SOURCE_LOCATOR_SCHEMA_VERSION,
                schema_version,
            )?;
            write_bytes_field(transcript, work, SOURCE_LOCATOR_VALUE, value.as_bytes())?;
        }
    }
    write_optional_bytes_field(
        transcript,
        work,
        SOURCE_DISPLAY_NAME,
        source.display_name().map(str::as_bytes),
    )?;
    write_digest_field(
        transcript,
        work,
        SOURCE_CONTENT_DIGEST,
        source.content_digest(),
    )?;

    let render = preview.begin_geometry_item("profile_source", source.index())?;
    preview.geometry_field_usize(render, "resource", source.resource());
    preview.geometry_field_usize(render, "format_bytes", source.format().len());
    preview.geometry_field_usize(
        render,
        "display_name_bytes",
        source.display_name().map_or(0, str::len),
    );
    preview.end_geometry_item(render);
    Ok(())
}

fn write_provenance(
    cursor: &mut NodeSceneProfileCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_collection(
        transcript,
        preview,
        work,
        COLLECTION_PROVENANCE,
        "profile_provenance",
        true,
        count,
    )?;
    for _ in 0..count {
        let provenance = next_lineage_item(work, || cursor.step_provenance())?.ok_or(
            NodeSceneSnapshotError::InconsistentView {
                reason: "profile provenance collection ended before declared total",
            },
        )?;
        write_provenance_row(provenance, transcript, preview, work)?;
    }
    if next_lineage_item(work, || cursor.step_provenance())?.is_some() {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "profile provenance collection exceeds its declared total",
        });
    }
    Ok(())
}

fn write_provenance_row(
    provenance: NodeSceneProfileProvenance<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_row(transcript, ROW_PROVENANCE, provenance.index())?;
    write_usize_field(transcript, work, PROVENANCE_DEPTH, provenance.depth())?;
    write_usize_field(
        transcript,
        work,
        PROVENANCE_PARENT_OFFSET,
        provenance.parent_offset(),
    )?;
    write_provenance_sequence_field(transcript, work, PROVENANCE_PARENTS, provenance.parents())?;
    let kind = match provenance.kind() {
        NodeSceneProfileProvenanceKind::Source { source } => {
            write_u8_field(transcript, work, PROVENANCE_KIND, 0)?;
            write_usize_field(transcript, work, PROVENANCE_SOURCE, source)?;
            0
        }
        NodeSceneProfileProvenanceKind::Operation(operation) => {
            write_u8_field(transcript, work, PROVENANCE_KIND, 1)?;
            write_operation(operation, transcript, work)?;
            1
        }
    };

    let render = preview.begin_geometry_item("profile_provenance", provenance.index())?;
    preview.geometry_field_usize(render, "depth", provenance.depth());
    preview.geometry_field_usize(render, "parent_offset", provenance.parent_offset());
    preview.geometry_field_usize(render, "parent_count", provenance.parents().len());
    preview.geometry_field_u32(render, "kind", kind);
    preview.end_geometry_item(render);
    Ok(())
}

fn write_operation(
    operation: NodeSceneProfileOperation<'_>,
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    write_usize_field(
        transcript,
        work,
        PROVENANCE_OPERATION_INDEX,
        operation.index(),
    )?;
    let (kind, name) = encode_operation_kind(operation.kind());
    write_u8_field(transcript, work, PROVENANCE_OPERATION_KIND, kind)?;
    write_optional_bytes_field(
        transcript,
        work,
        PROVENANCE_OPERATION_NAME,
        name.map(str::as_bytes),
    )?;
    write_required_digest_field(
        transcript,
        work,
        PROVENANCE_OPTIONS_DIGEST,
        operation.options_digest(),
    )?;
    write_usize_field(
        transcript,
        work,
        PROVENANCE_EVIDENCE_OFFSET,
        operation.evidence_offset(),
    )?;
    write_usize_field(
        transcript,
        work,
        PROVENANCE_EVIDENCE_COUNT,
        operation.evidence_count(),
    )
}

fn write_evidence(
    cursor: &mut NodeSceneProfileCursor<'_>,
    count: usize,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_collection(
        transcript,
        preview,
        work,
        COLLECTION_EVIDENCE,
        "profile_evidence",
        true,
        count,
    )?;
    for _ in 0..count {
        let evidence = next_lineage_item(work, || cursor.step_evidence())?.ok_or(
            NodeSceneSnapshotError::InconsistentView {
                reason: "profile evidence collection ended before declared total",
            },
        )?;
        write_evidence_row(evidence, transcript, preview, work)?;
    }
    if next_lineage_item(work, || cursor.step_evidence())?.is_some() {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "profile evidence collection exceeds its declared total",
        });
    }
    Ok(())
}

fn write_evidence_row(
    evidence: NodeSceneProfileEvidence<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_row(transcript, ROW_EVIDENCE, evidence.index())?;
    write_bytes_field(transcript, work, EVIDENCE_CODE, evidence.code().as_bytes())?;
    write_u64_field(
        transcript,
        work,
        EVIDENCE_OCCURRENCE_COUNT,
        evidence.occurrence_count(),
    )?;
    write_digest_field(transcript, work, EVIDENCE_DIGEST, evidence.digest())?;

    let render = preview.begin_geometry_item("profile_evidence", evidence.index())?;
    preview.geometry_field_usize(render, "code_bytes", evidence.code().len());
    preview.geometry_field_u64(render, "occurrences", evidence.occurrence_count());
    preview.end_geometry_item(render);
    Ok(())
}

fn write_owners(
    cursor: &mut NodeSceneProfileCursor<'_>,
    counts: ProfileCollectionCounts,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_collection(
        transcript,
        preview,
        work,
        COLLECTION_OWNERS,
        "profile_owners",
        true,
        counts.owners,
    )?;
    let mut observed_attachments = 0_usize;
    for ordinal in 0..counts.owners {
        work.observe()?;
        let owner = cursor
            .next_owner()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "profile owner collection ended before declared total",
            })?;
        observed_attachments = observed_attachments
            .checked_add(owner.attachments().len())
            .ok_or(NodeSceneSnapshotError::SemanticTotalOverflow {
                total: "profile owner attachments",
            })?;
        write_owner(ordinal, owner, transcript, preview, work)?;
    }
    if observed_attachments != counts.attachments {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "profile owner attachments disagree with their declared total",
        });
    }
    Ok(())
}

fn write_owner(
    ordinal: usize,
    owner: NodeSceneProfileOwner<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_row(transcript, ROW_OWNER, ordinal)?;
    let (owner_kind, owner_index) = encode_owner(owner.owner());
    write_u8_field(transcript, work, OWNER_KIND, owner_kind)?;
    write_u32_field(transcript, work, OWNER_INDEX, owner_index)?;
    write_usize_field(
        transcript,
        work,
        OWNER_ATTACHMENT_OFFSET,
        owner.attachment_offset(),
    )?;
    write_provenance_sequence_field(transcript, work, OWNER_ATTACHMENTS, owner.attachments())?;

    let render = preview.begin_geometry_item("profile_owner", ordinal)?;
    preview.geometry_field_u32(render, "kind", u32::from(owner_kind));
    preview.geometry_field_u32(render, "index", owner_index);
    preview.geometry_field_usize(render, "attachment_offset", owner.attachment_offset());
    preview.geometry_field_usize(render, "attachment_count", owner.attachments().len());
    preview.end_geometry_item(render);
    Ok(())
}

fn write_metadata(
    cursor: &mut NodeSceneProfileCursor<'_>,
    counts: ProfileCollectionCounts,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_collection(
        transcript,
        preview,
        work,
        COLLECTION_METADATA,
        "profile_metadata",
        counts.metadata_entries != 0,
        counts.metadata_entries,
    )?;
    for ordinal in 0..counts.metadata_entries {
        work.observe()?;
        let entry = cursor
            .next_metadata_entry()
            .map_err(super::map_snapshot_invariant)?
            .ok_or(NodeSceneSnapshotError::InconsistentView {
                reason: "profile metadata collection ended before declared total",
            })?;
        write_metadata_entry(ordinal, entry, transcript, preview, work)?;
    }
    Ok(())
}

fn write_metadata_entry(
    ordinal: usize,
    entry: NodeSceneProfileMetadataEntry<'_>,
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    begin_row(transcript, ROW_METADATA, ordinal)?;
    let (owner_kind, owner_index) = encode_owner(entry.owner());
    write_u8_field(transcript, work, METADATA_OWNER_KIND, owner_kind)?;
    write_u32_field(transcript, work, METADATA_OWNER_INDEX, owner_index)?;
    write_bytes_field(transcript, work, METADATA_KEY, entry.key().as_bytes())?;
    let value_kind = write_metadata_value(entry.value(), transcript, work)?;
    write_usize_field(transcript, work, METADATA_PROVENANCE, entry.provenance())?;

    let render = preview.begin_geometry_item("profile_metadata", ordinal)?;
    preview.geometry_field_u32(render, "owner_kind", u32::from(owner_kind));
    preview.geometry_field_u32(render, "owner_index", owner_index);
    preview.geometry_field_usize(render, "key_bytes", entry.key().len());
    preview.geometry_field_u32(render, "value_kind", u32::from(value_kind));
    preview.geometry_field_usize(render, "provenance", entry.provenance());
    preview.end_geometry_item(render);
    Ok(())
}

fn write_metadata_value(
    value: NodeSceneProfileMetadataValue<'_>,
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<u8, NodeSceneSnapshotError> {
    let kind = match value {
        NodeSceneProfileMetadataValue::Bool(value) => {
            write_u8_field(transcript, work, METADATA_VALUE_KIND, 0)?;
            write_usize_field(transcript, work, METADATA_LOGICAL_ELEMENT_COUNT, 1)?;
            write_u8_field(transcript, work, METADATA_VALUE, u8::from(value))?;
            0
        }
        NodeSceneProfileMetadataValue::I64(value) => {
            write_u8_field(transcript, work, METADATA_VALUE_KIND, 1)?;
            write_usize_field(transcript, work, METADATA_LOGICAL_ELEMENT_COUNT, 1)?;
            write_i64_field(transcript, work, METADATA_VALUE, value)?;
            1
        }
        NodeSceneProfileMetadataValue::U64(value) => {
            write_u8_field(transcript, work, METADATA_VALUE_KIND, 2)?;
            write_usize_field(transcript, work, METADATA_LOGICAL_ELEMENT_COUNT, 1)?;
            write_u64_field(transcript, work, METADATA_VALUE, value)?;
            2
        }
        NodeSceneProfileMetadataValue::F64(value) => {
            write_u8_field(transcript, work, METADATA_VALUE_KIND, 3)?;
            write_usize_field(transcript, work, METADATA_LOGICAL_ELEMENT_COUNT, 1)?;
            write_f64_field(transcript, work, METADATA_VALUE, value)?;
            3
        }
        NodeSceneProfileMetadataValue::String(value) => {
            write_u8_field(transcript, work, METADATA_VALUE_KIND, 4)?;
            write_usize_field(transcript, work, METADATA_LOGICAL_ELEMENT_COUNT, 1)?;
            write_bytes_field(transcript, work, METADATA_VALUE, value.as_bytes())?;
            4
        }
        NodeSceneProfileMetadataValue::Bytes {
            media_type,
            payload,
        } => {
            write_u8_field(transcript, work, METADATA_VALUE_KIND, 5)?;
            write_usize_field(transcript, work, METADATA_LOGICAL_ELEMENT_COUNT, 1)?;
            write_bytes_field(transcript, work, METADATA_MEDIA_TYPE, media_type.as_bytes())?;
            write_bytes_field(transcript, work, METADATA_VALUE, payload)?;
            5
        }
        NodeSceneProfileMetadataValue::Vec2(values) => {
            write_fixed_f32_metadata(transcript, work, 6, values, 2)?;
            6
        }
        NodeSceneProfileMetadataValue::Vec3(values) => {
            write_fixed_f32_metadata(transcript, work, 7, values, 3)?;
            7
        }
        NodeSceneProfileMetadataValue::Vec4(values) => {
            write_fixed_f32_metadata(transcript, work, 8, values, 4)?;
            8
        }
        NodeSceneProfileMetadataValue::Color(values) => {
            write_fixed_f32_metadata(transcript, work, 9, values, 4)?;
            9
        }
        NodeSceneProfileMetadataValue::Mat4(values) => {
            write_fixed_f32_metadata(transcript, work, 10, values, 16)?;
            10
        }
        NodeSceneProfileMetadataValue::DenseArray {
            element_kind,
            logical_element_count,
            values,
        } => {
            write_u8_field(transcript, work, METADATA_VALUE_KIND, 11)?;
            write_u8_field(
                transcript,
                work,
                METADATA_ARRAY_ELEMENT_KIND,
                encode_metadata_array_kind(element_kind),
            )?;
            write_usize_field(
                transcript,
                work,
                METADATA_LOGICAL_ELEMENT_COUNT,
                logical_element_count,
            )?;
            write_metadata_array(values, transcript, work)?;
            11
        }
    };
    Ok(kind)
}

fn write_fixed_f32_metadata(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    value_kind: u8,
    values: &[f32],
    expected_width: usize,
) -> Result<(), NodeSceneSnapshotError> {
    if values.len() != expected_width {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "profile metadata vector width disagrees with its semantic variant",
        });
    }
    write_u8_field(transcript, work, METADATA_VALUE_KIND, value_kind)?;
    write_usize_field(transcript, work, METADATA_LOGICAL_ELEMENT_COUNT, 1)?;
    write_f32_sequence_field(transcript, work, METADATA_VALUE, values)
}

fn write_metadata_array(
    values: NodeSceneProfileMetadataArray<'_>,
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    match values {
        NodeSceneProfileMetadataArray::None => Err(NodeSceneSnapshotError::InconsistentView {
            reason: "dense profile metadata array has no typed storage lane",
        }),
        NodeSceneProfileMetadataArray::Bool(values) => {
            write_u8_field(transcript, work, METADATA_ARRAY_STORAGE_KIND, 1)?;
            write_u8_sequence_field(transcript, work, METADATA_VALUE, values)
        }
        NodeSceneProfileMetadataArray::I64(values) => {
            write_u8_field(transcript, work, METADATA_ARRAY_STORAGE_KIND, 2)?;
            write_i64_sequence_field(transcript, work, METADATA_VALUE, values)
        }
        NodeSceneProfileMetadataArray::U64(values) => {
            write_u8_field(transcript, work, METADATA_ARRAY_STORAGE_KIND, 3)?;
            write_u64_sequence_field(transcript, work, METADATA_VALUE, values)
        }
        NodeSceneProfileMetadataArray::F64(values) => {
            write_u8_field(transcript, work, METADATA_ARRAY_STORAGE_KIND, 4)?;
            write_f64_sequence_field(transcript, work, METADATA_VALUE, values)
        }
        NodeSceneProfileMetadataArray::F32(values) => {
            write_u8_field(transcript, work, METADATA_ARRAY_STORAGE_KIND, 5)?;
            write_f32_sequence_field(transcript, work, METADATA_VALUE, values)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn begin_collection(
    transcript: &mut CanonicalDigestEncoder,
    preview: &mut PreviewSink,
    work: &mut SnapshotWorkMeter<'_>,
    kind: u8,
    name: &'static str,
    present: bool,
    count: usize,
) -> Result<(), NodeSceneSnapshotError> {
    work.observe()?;
    transcript.write_tag(TAG_COLLECTION)?;
    transcript.tagged_u8(TAG_KIND, kind)?;
    transcript.tagged_u8(
        TAG_PRESENCE,
        if present {
            PRESENCE_PRESENT
        } else {
            PRESENCE_ABSENT
        },
    )?;
    transcript.tagged_usize(TAG_COUNT, count)?;
    if present {
        preview.begin_geometry_collection(name, count);
    } else if count == 0 {
        preview.push_str(name);
        preview.push_str("(absent,total=0)\n");
    } else {
        return Err(NodeSceneSnapshotError::InconsistentView {
            reason: "absent profile collection has a nonzero declared total",
        });
    }
    Ok(())
}

fn begin_row(
    transcript: &mut CanonicalDigestEncoder,
    kind: u8,
    index: usize,
) -> Result<(), NodeSceneSnapshotError> {
    transcript.write_tag(TAG_ROW)?;
    transcript.tagged_u8(TAG_KIND, kind)?;
    transcript.tagged_usize(TAG_INDEX, index)
}

fn write_metadata_lane_shape(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    lane: u8,
    count: Option<usize>,
) -> Result<(), NodeSceneSnapshotError> {
    write_field_header(transcript, work, lane)?;
    match count {
        Some(count) => {
            transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
            transcript.tagged_usize(TAG_COUNT, count)
        }
        None => {
            transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?;
            transcript.tagged_usize(TAG_COUNT, 0)
        }
    }
}

fn write_field_header(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
) -> Result<(), NodeSceneSnapshotError> {
    work.observe()?;
    transcript.tagged_u8(TAG_FIELD, field)
}

fn write_u8_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    value: u8,
) -> Result<(), NodeSceneSnapshotError> {
    write_field_header(transcript, work, field)?;
    transcript.tagged_u8(TAG_U8, value)
}

fn write_u32_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    value: u32,
) -> Result<(), NodeSceneSnapshotError> {
    write_field_header(transcript, work, field)?;
    transcript.tagged_u32(TAG_U32, value)
}

fn write_u64_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    value: u64,
) -> Result<(), NodeSceneSnapshotError> {
    write_field_header(transcript, work, field)?;
    transcript.tagged_u64(TAG_U64, value)
}

fn write_i64_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    value: i64,
) -> Result<(), NodeSceneSnapshotError> {
    write_field_header(transcript, work, field)?;
    transcript.write_tag(TAG_I64)?;
    transcript.write_bytes(&value.to_le_bytes())
}

fn write_f64_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    value: f64,
) -> Result<(), NodeSceneSnapshotError> {
    write_field_header(transcript, work, field)?;
    transcript.tagged_u64(TAG_F64_BITS, value.to_bits())
}

fn write_usize_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    value: usize,
) -> Result<(), NodeSceneSnapshotError> {
    write_field_header(transcript, work, field)?;
    transcript.tagged_usize(TAG_U64, value)
}

fn write_bytes_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    value: &[u8],
) -> Result<(), NodeSceneSnapshotError> {
    write_field_header(transcript, work, field)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    write_metered_bytes(transcript, work, value)
}

fn write_optional_bytes_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    value: Option<&[u8]>,
) -> Result<(), NodeSceneSnapshotError> {
    write_field_header(transcript, work, field)?;
    match value {
        Some(value) => {
            transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
            write_metered_bytes(transcript, work, value)
        }
        None => {
            transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?;
            transcript.tagged_usize(TAG_COUNT, 0)
        }
    }
}

fn write_metered_bytes(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    value: &[u8],
) -> Result<(), NodeSceneSnapshotError> {
    transcript.write_tag(TAG_BYTES)?;
    let length =
        u64::try_from(value.len()).map_err(|_| NodeSceneSnapshotError::TranscriptLengthOverflow)?;
    transcript.write_bytes(&length.to_le_bytes())?;
    for chunk in value.chunks(PROFILE_PAYLOAD_CHUNK_BYTES) {
        work.observe()?;
        transcript.write_bytes(chunk)?;
    }
    Ok(())
}

fn write_digest_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    digest: Option<NodeSceneProfileDigest<'_>>,
) -> Result<(), NodeSceneSnapshotError> {
    write_field_header(transcript, work, field)?;
    match digest {
        Some(digest) => {
            transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
            write_digest_payload(transcript, work, digest)
        }
        None => {
            transcript.tagged_u8(TAG_PRESENCE, PRESENCE_ABSENT)?;
            transcript.tagged_usize(TAG_COUNT, 0)
        }
    }
}

fn write_required_digest_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    digest: NodeSceneProfileDigest<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    write_field_header(transcript, work, field)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    write_digest_payload(transcript, work, digest)
}

fn write_digest_payload(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    digest: NodeSceneProfileDigest<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    transcript.tagged_u8(TAG_KIND, encode_digest_algorithm(digest.algorithm()))?;
    write_metered_bytes(transcript, work, digest.bytes())
}

fn write_provenance_sequence_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    values: NodeSceneProfileProvenanceIndices<'_>,
) -> Result<(), NodeSceneSnapshotError> {
    write_sequence_header(transcript, work, field, values.len())?;
    for value in values.iter() {
        let value = value.ok_or(NodeSceneSnapshotError::InconsistentView {
            reason: "validated qualified provenance address is unreadable",
        })?;
        work.observe()?;
        transcript.tagged_u32(TAG_REFERENCE, value)?;
    }
    Ok(())
}

fn write_u8_sequence_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    values: &[u8],
) -> Result<(), NodeSceneSnapshotError> {
    write_sequence_header(transcript, work, field, values.len())?;
    for value in values.iter().copied() {
        work.observe()?;
        transcript.tagged_u8(TAG_U8, value)?;
    }
    Ok(())
}

fn write_i64_sequence_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    values: &[i64],
) -> Result<(), NodeSceneSnapshotError> {
    write_sequence_header(transcript, work, field, values.len())?;
    for value in values.iter().copied() {
        work.observe()?;
        transcript.write_tag(TAG_I64)?;
        transcript.write_bytes(&value.to_le_bytes())?;
    }
    Ok(())
}

fn write_u64_sequence_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    values: &[u64],
) -> Result<(), NodeSceneSnapshotError> {
    write_sequence_header(transcript, work, field, values.len())?;
    for value in values.iter().copied() {
        work.observe()?;
        transcript.tagged_u64(TAG_U64, value)?;
    }
    Ok(())
}

fn write_f64_sequence_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    values: &[f64],
) -> Result<(), NodeSceneSnapshotError> {
    write_sequence_header(transcript, work, field, values.len())?;
    for value in values.iter().copied() {
        work.observe()?;
        transcript.tagged_u64(TAG_F64_BITS, value.to_bits())?;
    }
    Ok(())
}

fn write_f32_sequence_field(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    values: &[f32],
) -> Result<(), NodeSceneSnapshotError> {
    write_sequence_header(transcript, work, field, values.len())?;
    for value in values.iter().copied() {
        work.observe()?;
        transcript.tagged_u32(TAG_F32_BITS, value.to_bits())?;
    }
    Ok(())
}

fn write_sequence_header(
    transcript: &mut CanonicalDigestEncoder,
    work: &mut SnapshotWorkMeter<'_>,
    field: u8,
    count: usize,
) -> Result<(), NodeSceneSnapshotError> {
    write_field_header(transcript, work, field)?;
    transcript.tagged_u8(TAG_PRESENCE, PRESENCE_PRESENT)?;
    transcript.tagged_usize(TAG_COUNT, count)
}

const fn encode_opaque_reason(reason: SessionOpaqueReason) -> u8 {
    match reason {
        SessionOpaqueReason::CallerLabel => 0,
        SessionOpaqueReason::Generated => 1,
        SessionOpaqueReason::UnregisteredResolver => 2,
        SessionOpaqueReason::SessionFallback => 3,
    }
}

const fn encode_source_object_kind(kind: SourceObjectKind) -> u8 {
    match kind {
        SourceObjectKind::Document => 0,
        SourceObjectKind::Scene => 1,
        SourceObjectKind::Node => 2,
        SourceObjectKind::Mesh => 3,
        SourceObjectKind::Primitive => 4,
        SourceObjectKind::Material => 5,
        SourceObjectKind::Image => 6,
        SourceObjectKind::Texture => 7,
        SourceObjectKind::Sampler => 8,
        SourceObjectKind::Skin => 9,
        SourceObjectKind::Animation => 10,
        SourceObjectKind::AnimationChannel => 11,
        SourceObjectKind::Camera => 12,
        SourceObjectKind::Light => 13,
        SourceObjectKind::Vertex => 14,
        SourceObjectKind::Face => 15,
        SourceObjectKind::Element => 16,
        SourceObjectKind::Row => 17,
        SourceObjectKind::Statement => 18,
        SourceObjectKind::Segment => 19,
    }
}

const fn encode_operation_kind<'scene>(
    kind: NodeSceneProfileOperationKind<'scene>,
) -> (u8, Option<&'scene str>) {
    match kind {
        NodeSceneProfileOperationKind::Generated => (0, None),
        NodeSceneProfileOperationKind::Default => (1, None),
        NodeSceneProfileOperationKind::Normalize => (2, None),
        NodeSceneProfileOperationKind::Repair => (3, None),
        NodeSceneProfileOperationKind::Split => (4, None),
        NodeSceneProfileOperationKind::Merge => (5, None),
        NodeSceneProfileOperationKind::Convert => (6, None),
        NodeSceneProfileOperationKind::Transform => (7, None),
        NodeSceneProfileOperationKind::Optimize => (8, None),
        NodeSceneProfileOperationKind::Namespaced(name) => (9, Some(name)),
    }
}

const fn encode_digest_algorithm(algorithm: DigestAlgorithm) -> u8 {
    match algorithm {
        DigestAlgorithm::Sha256 => 0,
        DigestAlgorithm::Blake3 => 1,
    }
}

const fn encode_owner(owner: CanonicalProfileOwner) -> (u8, u32) {
    match owner {
        CanonicalProfileOwner::Scene => (0, 0),
        CanonicalProfileOwner::SyntheticRoot => (1, 0),
        CanonicalProfileOwner::Node(index) => (2, index),
        CanonicalProfileOwner::Camera(index) => (3, index),
        CanonicalProfileOwner::Light(index) => (4, index),
        CanonicalProfileOwner::Image(index) => (5, index),
        CanonicalProfileOwner::Sampler(index) => (6, index),
        CanonicalProfileOwner::Texture(index) => (7, index),
        CanonicalProfileOwner::TextureBinding(index) => (8, index),
        CanonicalProfileOwner::Material(index) => (9, index),
        CanonicalProfileOwner::LogicalMesh(index) => (10, index),
        CanonicalProfileOwner::Primitive(index) => (11, index),
        CanonicalProfileOwner::Attribute(index) => (12, index),
        CanonicalProfileOwner::MeshInstance(index) => (13, index),
        CanonicalProfileOwner::Skin(index) => (14, index),
        CanonicalProfileOwner::MorphDefinition(index) => (15, index),
        CanonicalProfileOwner::MorphDelta(index) => (16, index),
        CanonicalProfileOwner::AnimationClip(index) => (17, index),
        CanonicalProfileOwner::AnimationChannel(index) => (18, index),
    }
}

const fn encode_metadata_array_kind(kind: NodeSceneProfileMetadataArrayKind) -> u8 {
    match kind {
        NodeSceneProfileMetadataArrayKind::Bool => 0,
        NodeSceneProfileMetadataArrayKind::I64 => 1,
        NodeSceneProfileMetadataArrayKind::U64 => 2,
        NodeSceneProfileMetadataArrayKind::F64 => 3,
        NodeSceneProfileMetadataArrayKind::Vec2 => 4,
        NodeSceneProfileMetadataArrayKind::Vec3 => 5,
        NodeSceneProfileMetadataArrayKind::Vec4 => 6,
        NodeSceneProfileMetadataArrayKind::Color => 7,
        NodeSceneProfileMetadataArrayKind::Mat4 => 8,
    }
}

fn count_as_u64(count: usize, total: &'static str) -> Result<u64, NodeSceneSnapshotError> {
    u64::try_from(count).map_err(|_| NodeSceneSnapshotError::SemanticTotalOverflow { total })
}

#[cfg(test)]
mod tests {
    use baozi_operation::CancellationToken;

    use super::super::NODE_SCENE_SNAPSHOT_POLL_INTERVAL;
    use super::*;

    #[test]
    fn checkpoint_polling_stops_before_the_next_segment_step() {
        let cancellation = CancellationToken::new();
        let mut work = SnapshotWorkMeter::new(u64::MAX, Some(&cancellation));
        let mut calls = 0_u64;

        let result: Result<Option<()>, _> = next_lineage_item(&mut work, || {
            calls += 1;
            if calls == 1 {
                cancellation.cancel();
            }
            Ok(NodeSceneProfileStep::Checkpoint)
        });

        assert!(matches!(result, Err(NodeSceneSnapshotError::Cancelled)));
        assert_eq!(calls, NODE_SCENE_SNAPSHOT_POLL_INTERVAL - 1);
    }

    #[test]
    fn one_short_checkpoint_budget_rejects_before_decoding_the_next_row() {
        let mut work = SnapshotWorkMeter::new(1, None);
        let mut calls = 0_u64;

        let result: Result<Option<()>, _> = next_lineage_item(&mut work, || {
            calls += 1;
            Ok(if calls == 1 {
                NodeSceneProfileStep::Checkpoint
            } else {
                NodeSceneProfileStep::Item(())
            })
        });

        assert!(matches!(
            result,
            Err(NodeSceneSnapshotError::WorkBudgetExceeded { .. })
        ));
        assert_eq!(calls, 1);
    }
}
