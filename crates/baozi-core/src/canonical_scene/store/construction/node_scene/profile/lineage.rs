use baozi_operation::{StoreBuildError, StoreRuntimeError};

use crate::canonical_scene::store::construction::node_scene::{
    NodeSceneWorkMeter, process_str_in_work_chunks,
};
use crate::canonical_scene::store::construction::{SceneStoreAssembly, SceneStoreKeys};
use crate::canonical_scene::store::production::{
    ProvenanceNodeAuthoring, ResourceIdentityStability, SessionOpaqueReason, ValidatedProfileBatch,
};
use crate::canonical_scene::store::profile_provenance::{
    PROFILE_PROVENANCE_ADDRESS_SCALARS, ProfileSegmentNonce, QualifiedProvenanceAddress,
};
use crate::canonical_scene::store::schema::CanonicalBlockRef;
use crate::canonical_scene::store::schema::profile::{
    PROFILE_DIGEST_BLAKE3, PROFILE_DIGEST_SHA256, PROFILE_EVIDENCE_SCALAR_COUNT,
    PROFILE_LOCATOR_INDEXED, PROFILE_LOCATOR_NAMESPACED, PROFILE_OPERATION_CONVERT,
    PROFILE_OPERATION_DEFAULT, PROFILE_OPERATION_GENERATED, PROFILE_OPERATION_MERGE,
    PROFILE_OPERATION_NAMESPACED, PROFILE_OPERATION_NORMALIZE, PROFILE_OPERATION_OPTIMIZE,
    PROFILE_OPERATION_REPAIR, PROFILE_OPERATION_SCALAR_COUNT, PROFILE_OPERATION_SPLIT,
    PROFILE_OPERATION_TRANSFORM, PROFILE_PROVENANCE_OPERATION, PROFILE_PROVENANCE_SCALAR_COUNT,
    PROFILE_PROVENANCE_SOURCE, PROFILE_RESOURCE_SCALAR_COUNT,
    PROFILE_RESOURCE_STABILITY_SESSION_OPAQUE, PROFILE_RESOURCE_STABILITY_STABLE,
    PROFILE_SESSION_OPAQUE_CALLER_LABEL, PROFILE_SESSION_OPAQUE_GENERATED,
    PROFILE_SESSION_OPAQUE_SESSION_FALLBACK, PROFILE_SESSION_OPAQUE_UNREGISTERED_RESOLVER,
    PROFILE_SOURCE_OBJECT_ANIMATION, PROFILE_SOURCE_OBJECT_ANIMATION_CHANNEL,
    PROFILE_SOURCE_OBJECT_CAMERA, PROFILE_SOURCE_OBJECT_DOCUMENT, PROFILE_SOURCE_OBJECT_ELEMENT,
    PROFILE_SOURCE_OBJECT_FACE, PROFILE_SOURCE_OBJECT_IMAGE, PROFILE_SOURCE_OBJECT_LIGHT,
    PROFILE_SOURCE_OBJECT_MATERIAL, PROFILE_SOURCE_OBJECT_MESH, PROFILE_SOURCE_OBJECT_NODE,
    PROFILE_SOURCE_OBJECT_PRIMITIVE, PROFILE_SOURCE_OBJECT_ROW, PROFILE_SOURCE_OBJECT_SAMPLER,
    PROFILE_SOURCE_OBJECT_SCENE, PROFILE_SOURCE_OBJECT_SEGMENT, PROFILE_SOURCE_OBJECT_SKIN,
    PROFILE_SOURCE_OBJECT_STATEMENT, PROFILE_SOURCE_OBJECT_TEXTURE, PROFILE_SOURCE_OBJECT_VERTEX,
    PROFILE_SOURCE_SCALAR_COUNT, ProfileLineageAttachmentSchema, ProfileLineageDigestSchema,
    ProfileLineageEvidenceSchema, ProfileLineageOperationSchema, ProfileLineageParentSchema,
    ProfileLineageProvenanceSchema, ProfileLineageResourceSchema, ProfileLineageSourceSchema,
    ProfileLineageTextSchema, ProfileLineageWideSchema,
};
use crate::provenance::{DigestAlgorithm, OperationKind, SourceObjectKind, SourceObjectLocator};

use super::{ProfileLeafRefs, ProfileMaterializationPlans};

const DIGEST_BYTES: u32 = 32;

#[derive(Clone, Copy, Default)]
struct TextLayout {
    source_start: u32,
    operation_start: u32,
    evidence_start: u32,
    end: u32,
}

#[derive(Clone, Copy, Default)]
struct DigestLayout {
    operation_start: u32,
    evidence_start: u32,
    end: u32,
}

#[derive(Clone, Copy, Default)]
struct WideLayout {
    evidence_start: u32,
    end: u32,
}

pub(super) fn retain_lineage<R>(
    assembly: &mut SceneStoreAssembly,
    plans: &ProfileMaterializationPlans,
    batch: &ValidatedProfileBatch<'_, R>,
    segment_nonce: ProfileSegmentNonce,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<ProfileLeafRefs, StoreRuntimeError> {
    let (text, text_layout) = match plans.lineage_text {
        Some(plan) => {
            let (reference, layout) = retain_text(assembly, plan, batch, keys, work, failure)?;
            (Some(reference), layout)
        }
        None => (None, TextLayout::default()),
    };
    let (digests, digest_layout) = match plans.lineage_digests {
        Some(plan) => {
            let (reference, layout) = retain_digests(assembly, plan, batch, keys, work, failure)?;
            (Some(reference), layout)
        }
        None => (None, DigestLayout::default()),
    };
    let (wide, wide_layout) = match plans.wide {
        Some(plan) => {
            let (reference, layout) =
                retain_wide_values(assembly, plan, batch, keys, work, failure)?;
            (Some(reference), layout)
        }
        None => (None, WideLayout::default()),
    };
    let parents = match plans.parents {
        Some(plan) => Some(retain_parents(
            assembly,
            plan,
            batch,
            segment_nonce,
            keys,
            work,
            failure,
        )?),
        None => None,
    };
    let attachments = match plans.attachments {
        Some(plan) => Some(retain_attachments(
            assembly,
            plan,
            batch,
            segment_nonce,
            keys,
            work,
            failure,
        )?),
        None => None,
    };
    let resources = match plans.resources {
        Some(plan) => Some(retain_resources(
            assembly,
            plan,
            batch,
            text_layout,
            keys,
            work,
            failure,
        )?),
        None => None,
    };
    let sources = match plans.sources {
        Some(plan) => Some(retain_sources(
            assembly,
            plan,
            batch,
            text_layout,
            digest_layout,
            wide_layout,
            keys,
            work,
            failure,
        )?),
        None => None,
    };
    let provenance = match plans.provenance {
        Some(plan) => Some(retain_provenance(
            assembly, plan, batch, keys, work, failure,
        )?),
        None => None,
    };
    let operations = match plans.operations {
        Some(plan) => Some(retain_operations(
            assembly,
            plan,
            batch,
            text_layout,
            digest_layout,
            keys,
            work,
            failure,
        )?),
        None => None,
    };
    let evidence = match plans.evidence {
        Some(plan) => Some(retain_evidence(
            assembly,
            plan,
            batch,
            text_layout,
            digest_layout,
            wide_layout,
            keys,
            work,
            failure,
        )?),
        None => None,
    };

    Ok(ProfileLeafRefs {
        resources,
        sources,
        provenance,
        operations,
        evidence,
        parents,
        wide,
        text,
        digests,
        attachments,
    })
}

fn retain_text<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::Utf8Plan,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<(CanonicalBlockRef<ProfileLineageTextSchema>, TextLayout), StoreRuntimeError> {
    let mut layout = TextLayout::default();
    let reference = assembly.try_retain_canonical_utf8_controlled(
        plan,
        keys.dimension,
        ProfileLineageTextSchema,
        |writer, control| {
            let mut cursor = 0_u32;

            macro_rules! append_text {
                ($value:expr) => {{
                    let value = $value;
                    let count = u32::try_from(value.len()).map_err(|_| failure)?;
                    process_str_in_work_chunks(value, control, work, failure, |chunk| {
                        writer.write_str(chunk)
                    })?;
                    cursor = cursor.checked_add(count).ok_or(failure)?;
                }};
            }

            for resource in batch.lineage().resources().iter().copied() {
                work.observe(control, 1)?;
                append_text!(resource.identity().as_str());
                if let ResourceIdentityStability::Stable { owner, schema, .. } =
                    resource.stability()
                {
                    append_text!(owner.as_str());
                    append_text!(schema.as_str());
                }
            }
            layout.source_start = cursor;

            for source in batch.lineage().source_identities().iter().copied() {
                work.observe(control, 1)?;
                append_text!(source.format().as_str());
                if let SourceObjectLocator::Namespaced(locator) = source.locator() {
                    append_text!(locator.schema_id().name().as_str());
                    append_text!(locator.value().as_str());
                }
                if let Some(display_name) = source.display_name() {
                    append_text!(display_name.as_str());
                }
            }
            layout.operation_start = cursor;

            for node in batch.lineage().provenance_nodes().iter().copied() {
                work.observe(control, 1)?;
                if let ProvenanceNodeAuthoring::Operation(operation) = node
                    && let OperationKind::Namespaced(name) = operation.kind()
                {
                    append_text!(name.as_str());
                }
            }
            layout.evidence_start = cursor;

            for node in batch.lineage().provenance_nodes().iter().copied() {
                work.observe(control, 1)?;
                if let ProvenanceNodeAuthoring::Operation(operation) = node {
                    for evidence in operation.evidence() {
                        work.observe(control, 1)?;
                        append_text!(evidence.code().as_str());
                    }
                }
            }
            layout.end = cursor;
            if layout.end != batch.retention_shape().lineage_text_bytes() {
                return Err(failure.into());
            }
            Ok(())
        },
    )?;
    Ok((CanonicalBlockRef::from_store(reference), layout))
}

fn retain_digests<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::BytesPlan,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<(CanonicalBlockRef<ProfileLineageDigestSchema>, DigestLayout), StoreRuntimeError> {
    let mut layout = DigestLayout::default();
    let reference = assembly.try_retain_canonical_bytes_controlled(
        plan,
        keys.dimension,
        ProfileLineageDigestSchema,
        |writer, control| {
            let mut cursor = 0_u32;

            macro_rules! append_digest {
                ($bytes:expr) => {{
                    let bytes = $bytes;
                    if bytes.len() != usize::try_from(DIGEST_BYTES).map_err(|_| failure)? {
                        return Err(failure.into());
                    }
                    work.observe(control, u64::from(DIGEST_BYTES))?;
                    writer.write_bytes(bytes)?;
                    cursor = cursor.checked_add(DIGEST_BYTES).ok_or(failure)?;
                }};
            }

            for source in batch.lineage().source_identities().iter().copied() {
                work.observe(control, 1)?;
                if let Some(digest) = source.content_digest() {
                    append_digest!(digest.as_bytes());
                }
            }
            layout.operation_start = cursor;

            for node in batch.lineage().provenance_nodes().iter().copied() {
                work.observe(control, 1)?;
                if let ProvenanceNodeAuthoring::Operation(operation) = node {
                    let options_digest = operation.options_digest();
                    append_digest!(options_digest.as_bytes());
                }
            }
            layout.evidence_start = cursor;

            for node in batch.lineage().provenance_nodes().iter().copied() {
                work.observe(control, 1)?;
                if let ProvenanceNodeAuthoring::Operation(operation) = node {
                    for evidence in operation.evidence() {
                        work.observe(control, 1)?;
                        if let Some(digest) = evidence.digest() {
                            append_digest!(digest.as_bytes());
                        }
                    }
                }
            }
            layout.end = cursor;
            if layout.end != batch.retention_shape().lineage_digest_bytes() {
                return Err(failure.into());
            }
            Ok(())
        },
    )?;
    Ok((CanonicalBlockRef::from_store(reference), layout))
}

fn retain_wide_values<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<u64>,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<(CanonicalBlockRef<ProfileLineageWideSchema>, WideLayout), StoreRuntimeError> {
    let mut layout = WideLayout::default();
    let reference = assembly.try_retain_canonical_sequence_controlled(
        plan,
        keys.dimension,
        ProfileLineageWideSchema,
        |writer, control| {
            let mut cursor = 0_u32;
            for source in batch.lineage().source_identities().iter().copied() {
                work.observe(control, 1)?;
                if let SourceObjectLocator::Indexed { index, .. } = source.locator() {
                    work.observe(control, 1)?;
                    writer.push(*index)?;
                    cursor = cursor.checked_add(1).ok_or(failure)?;
                }
            }
            layout.evidence_start = cursor;

            for node in batch.lineage().provenance_nodes().iter().copied() {
                work.observe(control, 1)?;
                if let ProvenanceNodeAuthoring::Operation(operation) = node {
                    for evidence in operation.evidence() {
                        work.observe(control, 1)?;
                        let occurrence_count = evidence.occurrence_count().get();
                        work.observe(control, 1)?;
                        writer.push(occurrence_count)?;
                        cursor = cursor.checked_add(1).ok_or(failure)?;
                    }
                }
            }
            layout.end = cursor;
            if layout.end != batch.retention_shape().wide_count() {
                return Err(failure.into());
            }
            Ok(())
        },
    )?;
    Ok((CanonicalBlockRef::from_store(reference), layout))
}

fn retain_parents<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<u64>,
    batch: &ValidatedProfileBatch<'_, R>,
    segment_nonce: ProfileSegmentNonce,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileLineageParentSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            ProfileLineageParentSchema,
            |writer, control| {
                let mut cursor = 0_u32;
                for node in batch.lineage().provenance_nodes().iter().copied() {
                    work.observe(control, 1)?;
                    if let ProvenanceNodeAuthoring::Operation(operation) = node {
                        for parent in operation.parents().iter().copied() {
                            work.observe(control, 1)?;
                            let address = QualifiedProvenanceAddress::new(
                                0,
                                segment_nonce,
                                parent.index(),
                                0,
                            )
                            .ok_or(failure)?;
                            work.observe(
                                control,
                                u64::try_from(PROFILE_PROVENANCE_ADDRESS_SCALARS)
                                    .map_err(|_| failure)?,
                            )?;
                            writer.extend_from_slice(&address.into_scalars())?;
                            cursor = cursor.checked_add(1).ok_or(failure)?;
                        }
                    }
                }
                if cursor != batch.retention_shape().parent_count() {
                    return Err(failure.into());
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_attachments<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<u64>,
    batch: &ValidatedProfileBatch<'_, R>,
    segment_nonce: ProfileSegmentNonce,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileLineageAttachmentSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            ProfileLineageAttachmentSchema,
            |writer, control| {
                let mut cursor = 0_u32;
                for owner_ordinal in 0..batch.owner_count() {
                    work.observe(control, 1)?;
                    let (_, provenance) =
                        batch.owner_provenance_at(owner_ordinal).ok_or(failure)?;
                    for provenance in provenance.iter().copied() {
                        work.observe(control, 1)?;
                        let address = QualifiedProvenanceAddress::new(
                            0,
                            segment_nonce,
                            provenance.index(),
                            0,
                        )
                        .ok_or(failure)?;
                        work.observe(
                            control,
                            u64::try_from(PROFILE_PROVENANCE_ADDRESS_SCALARS)
                                .map_err(|_| failure)?,
                        )?;
                        writer.extend_from_slice(&address.into_scalars())?;
                        cursor = cursor.checked_add(1).ok_or(failure)?;
                    }
                }
                if u64::from(cursor) != batch.owner_attachment_count()
                    || cursor != batch.retention_shape().attachment_count()
                {
                    return Err(failure.into());
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_resources<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<u32>,
    batch: &ValidatedProfileBatch<'_, R>,
    text_layout: TextLayout,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileLineageResourceSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            ProfileLineageResourceSchema,
            |writer, control| {
                let mut text_cursor = 0_u32;
                let row_work = u64::try_from(PROFILE_RESOURCE_SCALAR_COUNT).map_err(|_| failure)?;
                for resource in batch.lineage().resources().iter().copied() {
                    work.observe(control, 1)?;
                    let identity =
                        text_range(&mut text_cursor, resource.identity().as_str(), failure)?;
                    let (stability_kind, owner, schema, version, opaque_reason) =
                        match resource.stability() {
                            ResourceIdentityStability::Stable {
                                owner,
                                schema,
                                version,
                            } => (
                                u32::from(PROFILE_RESOURCE_STABILITY_STABLE),
                                text_range(&mut text_cursor, owner.as_str(), failure)?,
                                text_range(&mut text_cursor, schema.as_str(), failure)?,
                                version.get(),
                                0,
                            ),
                            ResourceIdentityStability::SessionOpaque(reason) => (
                                u32::from(PROFILE_RESOURCE_STABILITY_SESSION_OPAQUE),
                                empty_range(text_cursor),
                                empty_range(text_cursor),
                                0,
                                u32::from(encode_session_opaque_reason(*reason)),
                            ),
                        };
                    let row = [
                        identity.0,
                        identity.1,
                        stability_kind,
                        owner.0,
                        owner.1,
                        schema.0,
                        schema.1,
                        version,
                        opaque_reason,
                    ];
                    work.observe(control, row_work)?;
                    writer.extend_from_slice(&row)?;
                }
                if text_cursor != text_layout.source_start {
                    return Err(failure.into());
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_sources<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<u32>,
    batch: &ValidatedProfileBatch<'_, R>,
    text_layout: TextLayout,
    digest_layout: DigestLayout,
    wide_layout: WideLayout,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileLineageSourceSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            ProfileLineageSourceSchema,
            |writer, control| {
                let mut text_cursor = text_layout.source_start;
                let mut digest_cursor = 0_u32;
                let mut wide_cursor = 0_u32;
                let row_work = u64::try_from(PROFILE_SOURCE_SCALAR_COUNT).map_err(|_| failure)?;

                for source in batch.lineage().source_identities().iter().copied() {
                    work.observe(control, 1)?;
                    let format = text_range(&mut text_cursor, source.format().as_str(), failure)?;
                    let (
                        locator_kind,
                        indexed_object_kind,
                        wide,
                        locator_schema,
                        locator_schema_version,
                        locator_value,
                    ) = match source.locator() {
                        SourceObjectLocator::Indexed { kind, .. } => {
                            let wide = advance(&mut wide_cursor, 1, failure)?;
                            (
                                u32::from(PROFILE_LOCATOR_INDEXED),
                                u32::from(encode_source_object_kind(*kind)),
                                wide,
                                empty_range(text_cursor),
                                0,
                                empty_range(text_cursor),
                            )
                        }
                        SourceObjectLocator::Namespaced(locator) => (
                            u32::from(PROFILE_LOCATOR_NAMESPACED),
                            0,
                            empty_range(wide_cursor),
                            text_range(
                                &mut text_cursor,
                                locator.schema_id().name().as_str(),
                                failure,
                            )?,
                            locator.schema_id().version().get(),
                            text_range(&mut text_cursor, locator.value().as_str(), failure)?,
                        ),
                    };
                    let display_name = match source.display_name() {
                        Some(display_name) => Some(text_range(
                            &mut text_cursor,
                            display_name.as_str(),
                            failure,
                        )?),
                        None => None,
                    };
                    let digest = match source.content_digest() {
                        Some(digest) => Some((
                            u32::from(encode_digest_algorithm(digest.algorithm())),
                            advance(&mut digest_cursor, DIGEST_BYTES, failure)?,
                        )),
                        None => None,
                    };
                    let display_range = display_name.unwrap_or_else(|| empty_range(text_cursor));
                    let (digest_algorithm, digest_range) =
                        digest.unwrap_or_else(|| (0, empty_range(digest_cursor)));
                    let row = [
                        source.resource().index(),
                        format.0,
                        format.1,
                        locator_kind,
                        indexed_object_kind,
                        wide.0,
                        wide.1,
                        locator_schema.0,
                        locator_schema.1,
                        locator_schema_version,
                        locator_value.0,
                        locator_value.1,
                        u32::from(display_name.is_some()),
                        display_range.0,
                        display_range.1,
                        u32::from(digest.is_some()),
                        digest_algorithm,
                        digest_range.0,
                        digest_range.1,
                    ];
                    work.observe(control, row_work)?;
                    writer.extend_from_slice(&row)?;
                }
                if text_cursor != text_layout.operation_start
                    || digest_cursor != digest_layout.operation_start
                    || wide_cursor != wide_layout.evidence_start
                {
                    return Err(failure.into());
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn retain_provenance<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<u32>,
    batch: &ValidatedProfileBatch<'_, R>,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileLineageProvenanceSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            ProfileLineageProvenanceSchema,
            |writer, control| {
                let mut operation_index = 0_u32;
                let mut parent_cursor = 0_u32;
                let row_work =
                    u64::try_from(PROFILE_PROVENANCE_SCALAR_COUNT).map_err(|_| failure)?;
                for (node_index, node) in batch
                    .lineage()
                    .provenance_nodes()
                    .iter()
                    .copied()
                    .enumerate()
                {
                    work.observe(control, 1)?;
                    let node_index = u32::try_from(node_index).map_err(|_| failure)?;
                    let depth = batch
                        .provenance_depth(
                            crate::canonical_scene::store::production::ProvenanceLocalId::new(
                                node_index,
                            ),
                        )
                        .ok_or(failure)?;
                    let (kind, variant_index, parents) = match node {
                        ProvenanceNodeAuthoring::Source(source) => (
                            u32::from(PROFILE_PROVENANCE_SOURCE),
                            source.index(),
                            empty_range(parent_cursor),
                        ),
                        ProvenanceNodeAuthoring::Operation(operation) => {
                            let parent_count =
                                u32::try_from(operation.parents().len()).map_err(|_| failure)?;
                            let parents = advance(&mut parent_cursor, parent_count, failure)?;
                            let index = operation_index;
                            operation_index = operation_index.checked_add(1).ok_or(failure)?;
                            (u32::from(PROFILE_PROVENANCE_OPERATION), index, parents)
                        }
                    };
                    work.observe(control, row_work)?;
                    writer.extend_from_slice(&[
                        kind,
                        variant_index,
                        depth,
                        parents.0,
                        parents.1,
                    ])?;
                }
                if operation_index != batch.operation_count()
                    || parent_cursor != batch.retention_shape().parent_count()
                {
                    return Err(failure.into());
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_operations<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<u32>,
    batch: &ValidatedProfileBatch<'_, R>,
    text_layout: TextLayout,
    digest_layout: DigestLayout,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileLineageOperationSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            ProfileLineageOperationSchema,
            |writer, control| {
                let mut operation_count = 0_u32;
                let mut text_cursor = text_layout.operation_start;
                let mut digest_cursor = digest_layout.operation_start;
                let mut evidence_cursor = 0_u32;
                let row_work =
                    u64::try_from(PROFILE_OPERATION_SCALAR_COUNT).map_err(|_| failure)?;
                for node in batch.lineage().provenance_nodes().iter().copied() {
                    work.observe(control, 1)?;
                    let ProvenanceNodeAuthoring::Operation(operation) = node else {
                        continue;
                    };
                    let (kind, name) = match operation.kind() {
                        OperationKind::Namespaced(name) => (
                            u32::from(PROFILE_OPERATION_NAMESPACED),
                            text_range(&mut text_cursor, name.as_str(), failure)?,
                        ),
                        kind => (
                            u32::from(encode_operation_kind(kind)),
                            empty_range(text_cursor),
                        ),
                    };
                    let options_digest = advance(&mut digest_cursor, DIGEST_BYTES, failure)?;
                    let evidence_count =
                        u32::try_from(operation.evidence().len()).map_err(|_| failure)?;
                    let evidence = advance(&mut evidence_cursor, evidence_count, failure)?;
                    work.observe(control, row_work)?;
                    writer.extend_from_slice(&[
                        kind,
                        name.0,
                        name.1,
                        u32::from(encode_digest_algorithm(
                            operation.options_digest().algorithm(),
                        )),
                        options_digest.0,
                        options_digest.1,
                        evidence.0,
                        evidence.1,
                    ])?;
                    operation_count = operation_count.checked_add(1).ok_or(failure)?;
                }
                if operation_count != batch.operation_count()
                    || text_cursor != text_layout.evidence_start
                    || digest_cursor != digest_layout.evidence_start
                    || evidence_cursor != batch.retention_shape().evidence_count()
                {
                    return Err(failure.into());
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

#[allow(clippy::too_many_arguments)]
fn retain_evidence<R>(
    assembly: &mut SceneStoreAssembly,
    plan: baozi_operation::SequencePlan<u32>,
    batch: &ValidatedProfileBatch<'_, R>,
    text_layout: TextLayout,
    digest_layout: DigestLayout,
    wide_layout: WideLayout,
    keys: SceneStoreKeys,
    work: &mut NodeSceneWorkMeter,
    failure: StoreBuildError,
) -> Result<CanonicalBlockRef<ProfileLineageEvidenceSchema>, StoreRuntimeError> {
    assembly
        .try_retain_canonical_sequence_controlled(
            plan,
            keys.dimension,
            ProfileLineageEvidenceSchema,
            |writer, control| {
                let mut evidence_count = 0_u32;
                let mut text_cursor = text_layout.evidence_start;
                let mut digest_cursor = digest_layout.evidence_start;
                let mut wide_cursor = wide_layout.evidence_start;
                let row_work = u64::try_from(PROFILE_EVIDENCE_SCALAR_COUNT).map_err(|_| failure)?;
                for node in batch.lineage().provenance_nodes().iter().copied() {
                    work.observe(control, 1)?;
                    let ProvenanceNodeAuthoring::Operation(operation) = node else {
                        continue;
                    };
                    for evidence in operation.evidence() {
                        work.observe(control, 1)?;
                        let code = text_range(&mut text_cursor, evidence.code().as_str(), failure)?;
                        let occurrence = advance(&mut wide_cursor, 1, failure)?;
                        let digest = match evidence.digest() {
                            Some(digest) => Some((
                                u32::from(encode_digest_algorithm(digest.algorithm())),
                                advance(&mut digest_cursor, DIGEST_BYTES, failure)?,
                            )),
                            None => None,
                        };
                        let (digest_algorithm, digest_range) =
                            digest.unwrap_or_else(|| (0, empty_range(digest_cursor)));
                        work.observe(control, row_work)?;
                        writer.extend_from_slice(&[
                            code.0,
                            code.1,
                            occurrence.0,
                            occurrence.1,
                            u32::from(digest.is_some()),
                            digest_algorithm,
                            digest_range.0,
                            digest_range.1,
                        ])?;
                        evidence_count = evidence_count.checked_add(1).ok_or(failure)?;
                    }
                }
                if evidence_count != batch.retention_shape().evidence_count()
                    || text_cursor != text_layout.end
                    || digest_cursor != digest_layout.end
                    || wide_cursor != wide_layout.end
                {
                    return Err(failure.into());
                }
                Ok(())
            },
        )
        .map(CanonicalBlockRef::from_store)
}

fn text_range(
    cursor: &mut u32,
    value: &str,
    failure: StoreBuildError,
) -> Result<(u32, u32), StoreBuildError> {
    let count = u32::try_from(value.len()).map_err(|_| failure)?;
    advance(cursor, count, failure)
}

const fn empty_range(cursor: u32) -> (u32, u32) {
    (cursor, 0)
}

fn advance(
    cursor: &mut u32,
    count: u32,
    failure: StoreBuildError,
) -> Result<(u32, u32), StoreBuildError> {
    let offset = *cursor;
    *cursor = cursor.checked_add(count).ok_or(failure)?;
    Ok((offset, count))
}

const fn encode_session_opaque_reason(reason: SessionOpaqueReason) -> u8 {
    match reason {
        SessionOpaqueReason::CallerLabel => PROFILE_SESSION_OPAQUE_CALLER_LABEL,
        SessionOpaqueReason::Generated => PROFILE_SESSION_OPAQUE_GENERATED,
        SessionOpaqueReason::UnregisteredResolver => PROFILE_SESSION_OPAQUE_UNREGISTERED_RESOLVER,
        SessionOpaqueReason::SessionFallback => PROFILE_SESSION_OPAQUE_SESSION_FALLBACK,
    }
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

const fn encode_digest_algorithm(algorithm: DigestAlgorithm) -> u8 {
    match algorithm {
        DigestAlgorithm::Sha256 => PROFILE_DIGEST_SHA256,
        DigestAlgorithm::Blake3 => PROFILE_DIGEST_BLAKE3,
    }
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
