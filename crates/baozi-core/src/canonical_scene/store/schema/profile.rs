//! Private retained schema for canonical source identity, provenance, and metadata.
//!
//! A lineage segment owns immutable typed leaves. A later Scene generation can
//! retain those leaves verbatim and rebuild only its record-block directory,
//! segment, and owner relations around them.

use baozi_operation::{ByteBlock, RecordBlock, ScalarSequence, Utf8Block};

use super::CanonicalSchema;

pub(in super::super) const PROFILE_SCHEMA_REVISION: u32 = 1;
pub(in super::super) const PROFILE_METADATA_SCHEMA_REVISION: u32 = 1;
pub(in super::super) const PROFILE_DIRECTORY_FIELD_COUNT: usize = 6;
pub(in super::super) const PROFILE_SEGMENT_FIELD_COUNT: usize = 21;
pub(in super::super) const PROFILE_OWNER_FIELD_COUNT: usize = 5;
pub(in super::super) const PROFILE_METADATA_DIRECTORY_FIELD_COUNT: usize = 17;
pub(in super::super) const PROFILE_METADATA_ENTRY_FIELD_COUNT: usize = 20;
pub(in super::super) const PROFILE_RESOURCE_SCALAR_COUNT: usize = 9;
pub(in super::super) const PROFILE_SOURCE_SCALAR_COUNT: usize = 19;
pub(in super::super) const PROFILE_PROVENANCE_SCALAR_COUNT: usize = 5;
pub(in super::super) const PROFILE_OPERATION_SCALAR_COUNT: usize = 8;
pub(in super::super) const PROFILE_EVIDENCE_SCALAR_COUNT: usize = 8;
pub(in super::super) const FRESH_PROFILE_MAX_CANONICAL_BLOCKS: usize = 22;

pub(in super::super) const PROFILE_DIRECTORY_LOGICAL_BYTES: u64 =
    PROFILE_DIRECTORY_FIELD_COUNT as u64 * 4;
pub(in super::super) const PROFILE_SEGMENT_LOGICAL_BYTES: u64 =
    (PROFILE_SEGMENT_FIELD_COUNT as u64 - 1) * 4 + 8;
pub(in super::super) const PROFILE_OWNER_LOGICAL_BYTES: u64 = 1 + (4 * 4);
pub(in super::super) const PROFILE_METADATA_DIRECTORY_LOGICAL_BYTES: u64 =
    PROFILE_METADATA_DIRECTORY_FIELD_COUNT as u64 * 4;
pub(in super::super) const PROFILE_METADATA_ENTRY_LOGICAL_BYTES: u64 = 3 + (10 * 4) + (7 * 4);

/// Zero is the only valid scalar encoding for an absent optional block reference.
pub(in super::super) const PROFILE_ABSENT_REFERENCE: u8 = 0;

pub(in super::super) const PROFILE_RESOURCE_STABILITY_STABLE: u8 = 0;
pub(in super::super) const PROFILE_RESOURCE_STABILITY_SESSION_OPAQUE: u8 = 1;

pub(in super::super) const PROFILE_SESSION_OPAQUE_CALLER_LABEL: u8 = 0;
pub(in super::super) const PROFILE_SESSION_OPAQUE_GENERATED: u8 = 1;
pub(in super::super) const PROFILE_SESSION_OPAQUE_UNREGISTERED_RESOLVER: u8 = 2;
pub(in super::super) const PROFILE_SESSION_OPAQUE_SESSION_FALLBACK: u8 = 3;

pub(in super::super) const PROFILE_LOCATOR_INDEXED: u8 = 0;
pub(in super::super) const PROFILE_LOCATOR_NAMESPACED: u8 = 1;

pub(in super::super) const PROFILE_SOURCE_OBJECT_DOCUMENT: u8 = 0;
pub(in super::super) const PROFILE_SOURCE_OBJECT_SCENE: u8 = 1;
pub(in super::super) const PROFILE_SOURCE_OBJECT_NODE: u8 = 2;
pub(in super::super) const PROFILE_SOURCE_OBJECT_MESH: u8 = 3;
pub(in super::super) const PROFILE_SOURCE_OBJECT_PRIMITIVE: u8 = 4;
pub(in super::super) const PROFILE_SOURCE_OBJECT_MATERIAL: u8 = 5;
pub(in super::super) const PROFILE_SOURCE_OBJECT_IMAGE: u8 = 6;
pub(in super::super) const PROFILE_SOURCE_OBJECT_TEXTURE: u8 = 7;
pub(in super::super) const PROFILE_SOURCE_OBJECT_SAMPLER: u8 = 8;
pub(in super::super) const PROFILE_SOURCE_OBJECT_SKIN: u8 = 9;
pub(in super::super) const PROFILE_SOURCE_OBJECT_ANIMATION: u8 = 10;
pub(in super::super) const PROFILE_SOURCE_OBJECT_ANIMATION_CHANNEL: u8 = 11;
pub(in super::super) const PROFILE_SOURCE_OBJECT_CAMERA: u8 = 12;
pub(in super::super) const PROFILE_SOURCE_OBJECT_LIGHT: u8 = 13;
pub(in super::super) const PROFILE_SOURCE_OBJECT_VERTEX: u8 = 14;
pub(in super::super) const PROFILE_SOURCE_OBJECT_FACE: u8 = 15;
pub(in super::super) const PROFILE_SOURCE_OBJECT_ELEMENT: u8 = 16;
pub(in super::super) const PROFILE_SOURCE_OBJECT_ROW: u8 = 17;
pub(in super::super) const PROFILE_SOURCE_OBJECT_STATEMENT: u8 = 18;
pub(in super::super) const PROFILE_SOURCE_OBJECT_SEGMENT: u8 = 19;
pub(in super::super) const PROFILE_SOURCE_OBJECT_BUFFER: u8 = 20;

pub(in super::super) const PROFILE_DIGEST_SHA256: u8 = 0;
pub(in super::super) const PROFILE_DIGEST_BLAKE3: u8 = 1;

pub(in super::super) const PROFILE_PROVENANCE_SOURCE: u8 = 0;
pub(in super::super) const PROFILE_PROVENANCE_OPERATION: u8 = 1;

pub(in super::super) const PROFILE_OPERATION_GENERATED: u8 = 0;
pub(in super::super) const PROFILE_OPERATION_DEFAULT: u8 = 1;
pub(in super::super) const PROFILE_OPERATION_NORMALIZE: u8 = 2;
pub(in super::super) const PROFILE_OPERATION_REPAIR: u8 = 3;
pub(in super::super) const PROFILE_OPERATION_SPLIT: u8 = 4;
pub(in super::super) const PROFILE_OPERATION_MERGE: u8 = 5;
pub(in super::super) const PROFILE_OPERATION_CONVERT: u8 = 6;
pub(in super::super) const PROFILE_OPERATION_TRANSFORM: u8 = 7;
pub(in super::super) const PROFILE_OPERATION_OPTIMIZE: u8 = 8;
pub(in super::super) const PROFILE_OPERATION_NAMESPACED: u8 = 9;

pub(in super::super) const PROFILE_OWNER_SCENE: u8 = 0;
pub(in super::super) const PROFILE_OWNER_SYNTHETIC_ROOT: u8 = 1;
pub(in super::super) const PROFILE_OWNER_NODE: u8 = 2;
pub(in super::super) const PROFILE_OWNER_CAMERA: u8 = 3;
pub(in super::super) const PROFILE_OWNER_LIGHT: u8 = 4;
pub(in super::super) const PROFILE_OWNER_IMAGE: u8 = 5;
pub(in super::super) const PROFILE_OWNER_SAMPLER: u8 = 6;
pub(in super::super) const PROFILE_OWNER_TEXTURE: u8 = 7;
pub(in super::super) const PROFILE_OWNER_TEXTURE_BINDING: u8 = 8;
pub(in super::super) const PROFILE_OWNER_MATERIAL: u8 = 9;
pub(in super::super) const PROFILE_OWNER_LOGICAL_MESH: u8 = 10;
pub(in super::super) const PROFILE_OWNER_PRIMITIVE: u8 = 11;
pub(in super::super) const PROFILE_OWNER_ATTRIBUTE: u8 = 12;
pub(in super::super) const PROFILE_OWNER_MESH_INSTANCE: u8 = 13;
pub(in super::super) const PROFILE_OWNER_SKIN: u8 = 14;
pub(in super::super) const PROFILE_OWNER_MORPH_DEFINITION: u8 = 15;
pub(in super::super) const PROFILE_OWNER_MORPH_DELTA: u8 = 16;
pub(in super::super) const PROFILE_OWNER_ANIMATION_CLIP: u8 = 17;
pub(in super::super) const PROFILE_OWNER_ANIMATION_CHANNEL: u8 = 18;

pub(in super::super) const PROFILE_METADATA_BOOL: u8 = 0;
pub(in super::super) const PROFILE_METADATA_I64: u8 = 1;
pub(in super::super) const PROFILE_METADATA_U64: u8 = 2;
pub(in super::super) const PROFILE_METADATA_F64: u8 = 3;
pub(in super::super) const PROFILE_METADATA_STRING: u8 = 4;
pub(in super::super) const PROFILE_METADATA_BYTES: u8 = 5;
pub(in super::super) const PROFILE_METADATA_VEC2: u8 = 6;
pub(in super::super) const PROFILE_METADATA_VEC3: u8 = 7;
pub(in super::super) const PROFILE_METADATA_VEC4: u8 = 8;
pub(in super::super) const PROFILE_METADATA_COLOR: u8 = 9;
pub(in super::super) const PROFILE_METADATA_MAT4: u8 = 10;
pub(in super::super) const PROFILE_METADATA_DENSE_ARRAY: u8 = 11;

pub(in super::super) const PROFILE_METADATA_ARRAY_BOOL: u8 = 0;
pub(in super::super) const PROFILE_METADATA_ARRAY_I64: u8 = 1;
pub(in super::super) const PROFILE_METADATA_ARRAY_U64: u8 = 2;
pub(in super::super) const PROFILE_METADATA_ARRAY_F64: u8 = 3;
pub(in super::super) const PROFILE_METADATA_ARRAY_VEC2: u8 = 4;
pub(in super::super) const PROFILE_METADATA_ARRAY_VEC3: u8 = 5;
pub(in super::super) const PROFILE_METADATA_ARRAY_VEC4: u8 = 6;
pub(in super::super) const PROFILE_METADATA_ARRAY_COLOR: u8 = 7;
pub(in super::super) const PROFILE_METADATA_ARRAY_MAT4: u8 = 8;

pub(in super::super) mod profile_directory_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const SEGMENT_COUNT: usize = 1;
    pub(crate) const SEGMENTS: usize = 2;
    pub(crate) const OWNER_COUNT: usize = 3;
    pub(crate) const OWNERS: usize = 4;
    pub(crate) const METADATA: usize = 5;
}

/// One row per segment retained by this Scene generation.
///
/// Every count/ref pair names a single immutable leaf owned by that segment.
/// This permits descendants to retain historical leaves without copying their
/// lineage payloads when appending a new segment. Segment order also defines
/// generation-wide resource, source, and provenance ordinals: each domain is
/// the prefix concatenation of that domain's leaves across all segments.
/// `NONCE` is process-unique and `PROVENANCE_BASE` fixes the first global
/// topology index owned by the segment.
pub(in super::super) mod profile_segment_field {
    pub(crate) const SEGMENT: usize = 0;
    pub(crate) const RESOURCE_COUNT: usize = 1;
    pub(crate) const RESOURCES: usize = 2;
    pub(crate) const SOURCE_COUNT: usize = 3;
    pub(crate) const SOURCES: usize = 4;
    pub(crate) const PROVENANCE_COUNT: usize = 5;
    pub(crate) const PROVENANCE_NODES: usize = 6;
    pub(crate) const OPERATION_COUNT: usize = 7;
    pub(crate) const OPERATIONS: usize = 8;
    pub(crate) const EVIDENCE_COUNT: usize = 9;
    pub(crate) const EVIDENCE: usize = 10;
    pub(crate) const PARENT_COUNT: usize = 11;
    pub(crate) const PARENTS: usize = 12;
    pub(crate) const WIDE_COUNT: usize = 13;
    pub(crate) const WIDE_VALUES: usize = 14;
    pub(crate) const TEXT_COUNT: usize = 15;
    pub(crate) const TEXTS: usize = 16;
    pub(crate) const DIGEST_COUNT: usize = 17;
    pub(crate) const DIGESTS: usize = 18;
    pub(crate) const NONCE: usize = 19;
    pub(crate) const PROVENANCE_BASE: usize = 20;
}

/// Fixed-width resource rows stored in `ProfileLineageResourceSchema`.
pub(in super::super) mod profile_resource_scalar {
    pub(crate) const IDENTITY_OFFSET: usize = 0;
    pub(crate) const IDENTITY_COUNT: usize = 1;
    pub(crate) const STABILITY_KIND: usize = 2;
    pub(crate) const STABILITY_OWNER_OFFSET: usize = 3;
    pub(crate) const STABILITY_OWNER_COUNT: usize = 4;
    pub(crate) const STABILITY_SCHEMA_OFFSET: usize = 5;
    pub(crate) const STABILITY_SCHEMA_COUNT: usize = 6;
    pub(crate) const STABILITY_SCHEMA_VERSION: usize = 7;
    pub(crate) const SESSION_OPAQUE_REASON: usize = 8;
}

/// Fixed-width source-identity rows stored in `ProfileLineageSourceSchema`.
/// `RESOURCE` is a generation-wide resource ordinal.
pub(in super::super) mod profile_source_scalar {
    pub(crate) const RESOURCE: usize = 0;
    pub(crate) const FORMAT_OFFSET: usize = 1;
    pub(crate) const FORMAT_COUNT: usize = 2;
    pub(crate) const LOCATOR_KIND: usize = 3;
    pub(crate) const INDEXED_OBJECT_KIND: usize = 4;
    pub(crate) const WIDE_OFFSET: usize = 5;
    pub(crate) const WIDE_COUNT: usize = 6;
    pub(crate) const LOCATOR_SCHEMA_OFFSET: usize = 7;
    pub(crate) const LOCATOR_SCHEMA_COUNT: usize = 8;
    pub(crate) const LOCATOR_SCHEMA_VERSION: usize = 9;
    pub(crate) const LOCATOR_VALUE_OFFSET: usize = 10;
    pub(crate) const LOCATOR_VALUE_COUNT: usize = 11;
    pub(crate) const DISPLAY_NAME_PRESENCE: usize = 12;
    pub(crate) const DISPLAY_NAME_OFFSET: usize = 13;
    pub(crate) const DISPLAY_NAME_COUNT: usize = 14;
    pub(crate) const CONTENT_DIGEST_PRESENCE: usize = 15;
    pub(crate) const CONTENT_DIGEST_ALGORITHM: usize = 16;
    pub(crate) const CONTENT_DIGEST_OFFSET: usize = 17;
    pub(crate) const CONTENT_DIGEST_COUNT: usize = 18;
}

/// Fixed-width provenance-node rows stored in `ProfileLineageProvenanceSchema`.
/// For source nodes, `VARIANT_INDEX` is a generation-wide source ordinal; for
/// operation nodes it is local to the segment's operation leaf.
pub(in super::super) mod profile_provenance_scalar {
    pub(crate) const KIND: usize = 0;
    pub(crate) const VARIANT_INDEX: usize = 1;
    pub(crate) const DEPTH: usize = 2;
    pub(crate) const PARENT_OFFSET: usize = 3;
    pub(crate) const PARENT_COUNT: usize = 4;
}

/// Fixed-width operation rows stored in `ProfileLineageOperationSchema`.
/// Parent ranges select qualified provenance addresses from the segment's
/// parent leaf, so a newly appended operation can reference historical nodes
/// without accepting a sibling segment with the same local index.
pub(in super::super) mod profile_operation_scalar {
    pub(crate) const KIND: usize = 0;
    pub(crate) const NAME_OFFSET: usize = 1;
    pub(crate) const NAME_COUNT: usize = 2;
    pub(crate) const OPTIONS_DIGEST_ALGORITHM: usize = 3;
    pub(crate) const OPTIONS_DIGEST_OFFSET: usize = 4;
    pub(crate) const OPTIONS_DIGEST_COUNT: usize = 5;
    pub(crate) const EVIDENCE_OFFSET: usize = 6;
    pub(crate) const EVIDENCE_COUNT: usize = 7;
}

/// Fixed-width durable evidence rows stored in `ProfileLineageEvidenceSchema`.
pub(in super::super) mod profile_evidence_scalar {
    pub(crate) const CODE_OFFSET: usize = 0;
    pub(crate) const CODE_COUNT: usize = 1;
    pub(crate) const OCCURRENCE_WIDE_OFFSET: usize = 2;
    pub(crate) const OCCURRENCE_WIDE_COUNT: usize = 3;
    pub(crate) const DIGEST_PRESENCE: usize = 4;
    pub(crate) const DIGEST_ALGORITHM: usize = 5;
    pub(crate) const DIGEST_OFFSET: usize = 6;
    pub(crate) const DIGEST_COUNT: usize = 7;
}

/// One row per canonical owner. The attachment range is non-empty and indexes
/// the generation-wide attachment leaf in producer order. Every attachment
/// value is a qualified provenance address; semantic views project only its
/// generation-wide topology index.
pub(in super::super) mod profile_owner_field {
    pub(crate) const OWNER_KIND: usize = 0;
    pub(crate) const OWNER_INDEX: usize = 1;
    pub(crate) const ATTACHMENT_OFFSET: usize = 2;
    pub(crate) const ATTACHMENT_COUNT: usize = 3;
    pub(crate) const ATTACHMENTS: usize = 4;
}

/// Directory for the compact typed metadata lanes.
pub(in super::super) mod profile_metadata_directory_field {
    pub(crate) const SCHEMA_REVISION: usize = 0;
    pub(crate) const ENTRY_COUNT: usize = 1;
    pub(crate) const ENTRIES: usize = 2;
    pub(crate) const TEXT_COUNT: usize = 3;
    pub(crate) const TEXTS: usize = 4;
    pub(crate) const BYTE_COUNT: usize = 5;
    pub(crate) const BYTES: usize = 6;
    pub(crate) const BOOL_COUNT: usize = 7;
    pub(crate) const BOOL_VALUES: usize = 8;
    pub(crate) const I64_COUNT: usize = 9;
    pub(crate) const I64_VALUES: usize = 10;
    pub(crate) const U64_COUNT: usize = 11;
    pub(crate) const U64_VALUES: usize = 12;
    pub(crate) const F64_COUNT: usize = 13;
    pub(crate) const F64_VALUES: usize = 14;
    pub(crate) const F32_COUNT: usize = 15;
    pub(crate) const F32_VALUES: usize = 16;
}

/// The closed ADR0023 metadata algebra over seven typed payload lanes.
///
/// `logical_element_count` preserves present-empty values independently from
/// the selected lane range. For a dense vector array it counts vectors, while
/// the f32 scalar range counts physical lanes.
pub(in super::super) mod profile_metadata_entry_field {
    pub(crate) const OWNER_KIND: usize = 0;
    pub(crate) const OWNER_INDEX: usize = 1;
    pub(crate) const KEY_OFFSET: usize = 2;
    pub(crate) const KEY_COUNT: usize = 3;
    pub(crate) const VALUE_KIND: usize = 4;
    pub(crate) const ARRAY_KIND: usize = 5;
    pub(crate) const LOGICAL_ELEMENT_COUNT: usize = 6;
    pub(crate) const AUX_TEXT_OFFSET: usize = 7;
    pub(crate) const AUX_TEXT_COUNT: usize = 8;
    pub(crate) const BYTE_OFFSET: usize = 9;
    pub(crate) const BYTE_COUNT: usize = 10;
    pub(crate) const SCALAR_OFFSET: usize = 11;
    pub(crate) const SCALAR_COUNT: usize = 12;
    pub(crate) const TEXTS: usize = 13;
    pub(crate) const BYTES: usize = 14;
    pub(crate) const BOOL_VALUES: usize = 15;
    pub(crate) const I64_VALUES: usize = 16;
    pub(crate) const U64_VALUES: usize = 17;
    pub(crate) const F64_VALUES: usize = 18;
    pub(crate) const F32_VALUES: usize = 19;
}

const _: [(); PROFILE_DIRECTORY_FIELD_COUNT] = [(); profile_directory_field::METADATA + 1];
const _: [(); PROFILE_SEGMENT_FIELD_COUNT] = [(); profile_segment_field::PROVENANCE_BASE + 1];
const _: [(); PROFILE_OWNER_FIELD_COUNT] = [(); profile_owner_field::ATTACHMENTS + 1];
const _: [(); PROFILE_METADATA_DIRECTORY_FIELD_COUNT] =
    [(); profile_metadata_directory_field::F32_VALUES + 1];
const _: [(); PROFILE_METADATA_ENTRY_FIELD_COUNT] =
    [(); profile_metadata_entry_field::F32_VALUES + 1];
const _: [(); PROFILE_RESOURCE_SCALAR_COUNT] =
    [(); profile_resource_scalar::SESSION_OPAQUE_REASON + 1];
const _: [(); PROFILE_SOURCE_SCALAR_COUNT] = [(); profile_source_scalar::CONTENT_DIGEST_COUNT + 1];
const _: [(); PROFILE_PROVENANCE_SCALAR_COUNT] = [(); profile_provenance_scalar::PARENT_COUNT + 1];
const _: [(); PROFILE_OPERATION_SCALAR_COUNT] = [(); profile_operation_scalar::EVIDENCE_COUNT + 1];
const _: [(); PROFILE_EVIDENCE_SCALAR_COUNT] = [(); profile_evidence_scalar::DIGEST_COUNT + 1];

macro_rules! record_schemas {
    ($($name:ident),+ $(,)?) => {
        $(
            #[derive(Clone, Copy, PartialEq, Eq)]
            pub(in super::super) struct $name;

            impl CanonicalSchema for $name {
                type StoreKind = RecordBlock;
            }
        )+
    };
}

macro_rules! scalar_schemas {
    ($scalar:ty; $($name:ident),+ $(,)?) => {
        $(
            #[derive(Clone, Copy, PartialEq, Eq)]
            pub(in super::super) struct $name;

            impl CanonicalSchema for $name {
                type StoreKind = ScalarSequence<$scalar>;
            }
        )+
    };
}

record_schemas!(
    ProfileDirectorySchema,
    ProfileSegmentTableSchema,
    ProfileOwnerTableSchema,
    ProfileMetadataDirectorySchema,
    ProfileMetadataEntryTableSchema,
);

scalar_schemas!(
    u32;
    ProfileLineageResourceSchema,
    ProfileLineageSourceSchema,
    ProfileLineageProvenanceSchema,
    ProfileLineageOperationSchema,
    ProfileLineageEvidenceSchema,
);

scalar_schemas!(
    u64;
    ProfileLineageWideSchema,
    ProfileLineageParentSchema,
    ProfileLineageAttachmentSchema,
);

scalar_schemas!(
    u8;
    ProfileMetadataBoolSchema,
);

scalar_schemas!(
    i64;
    ProfileMetadataI64Schema,
);

scalar_schemas!(
    u64;
    ProfileMetadataU64Schema,
);

scalar_schemas!(
    f64;
    ProfileMetadataF64Schema,
);

scalar_schemas!(
    f32;
    ProfileMetadataF32Schema,
);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ProfileLineageTextSchema;

impl CanonicalSchema for ProfileLineageTextSchema {
    type StoreKind = Utf8Block;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ProfileLineageDigestSchema;

impl CanonicalSchema for ProfileLineageDigestSchema {
    type StoreKind = ByteBlock;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ProfileMetadataTextSchema;

impl CanonicalSchema for ProfileMetadataTextSchema {
    type StoreKind = Utf8Block;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ProfileMetadataBytesSchema;

impl CanonicalSchema for ProfileMetadataBytesSchema {
    type StoreKind = ByteBlock;
}
