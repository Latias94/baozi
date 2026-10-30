//! Concrete rooted evidence for the closed set of Scene edits.

use baozi_operation::RecordBlock;

use crate::canonical_scene::store::schema::CanonicalSchema;

/// Root tag for an edit outcome; distinct from fresh-import Scene evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneEditEvidenceTag;

/// Flattened primitive-local source-corner triples, in output triangle order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneGeometryCornerOriginsTag;

/// Source-face to contiguous output-triangle prefix offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneGeometryFaceOffsetsTag;

impl CanonicalSchema for SceneEditEvidenceTag {
    type StoreKind = RecordBlock;
}

pub(super) const REVISION: u32 = 4;
pub(super) const OPERATION_TRIANGULATE: u32 = 1;
pub(super) const OPERATION_NORMALS: u32 = 2;
pub(super) const OPERATION_SPACE_CONVERSION: u32 = 3;
pub(super) const OPERATION_SEMANTIC_VERSION: u32 = 1;
pub(super) const OUTCOME_CHANGED: u32 = 1;
pub(super) const OUTCOME_FAILED: u32 = 2;

// Values encode declared effects, not claims that a cached value exists.
pub(super) const PRESERVE_VERTEX_IDENTITY: u32 = 1;
pub(super) const PRESERVE_SOURCE_FIELDS_BY_ORIGIN: u32 = 2;
pub(super) const PRESERVE_SCALAR_BITS_AND_SPARSE_ENCODING: u32 = 4;
pub(super) const INVALIDATE_PRIMITIVE_MESH_SCENE_BOUNDS: u32 = 7;

pub(super) mod field {
    pub(crate) const REVISION: usize = 0;
    pub(crate) const OUTCOME: usize = 1;
    pub(crate) const SEMANTIC_VERSION: usize = 2;
    pub(crate) const OPERATION_KIND: usize = 3;
    pub(crate) const TARGET_MESH: usize = 4;
    pub(crate) const TARGET_PRIMITIVE: usize = 5;
    pub(crate) const SOURCE_PRIMITIVE_ROW: usize = 6;
    pub(crate) const VERTEX_COUNT: usize = 7;
    pub(crate) const SOURCE_CORNER_COUNT: usize = 8;
    pub(crate) const SOURCE_FACE_COUNT: usize = 9;
    pub(crate) const OUTPUT_CORNER_COUNT: usize = 10;
    pub(crate) const OUTPUT_FACE_COUNT: usize = 11;
    pub(crate) const ATTRIBUTE_COUNT: usize = 12;
    pub(crate) const PRESERVATION_RULES: usize = 13;
    pub(crate) const INVALIDATIONS: usize = 14;
    pub(crate) const OPTIONS_ALGORITHM: usize = 15;
    pub(crate) const OPTIONS_DIGEST_FIRST: usize = 16;
    pub(crate) const SOURCE_UNIT_BITS: usize = 20;
    pub(crate) const TARGET_UNIT_BITS: usize = 21;
    pub(crate) const SCENE_COUNTS: usize = 22;
    pub(crate) const CORNER_ORIGINS: usize = 23;
    pub(crate) const FACE_OFFSETS: usize = 24;
}

/// Complete spatial facts; no primitive coordinates are implied by these cells.
pub(super) mod space_field {
    pub(crate) const SOURCE_HANDEDNESS: usize = 4;
    pub(crate) const SOURCE_UP: usize = 5;
    pub(crate) const SOURCE_FRONT: usize = 6;
    pub(crate) const SOURCE_WINDING: usize = 7;
    pub(crate) const TARGET_HANDEDNESS: usize = 8;
    pub(crate) const TARGET_UP: usize = 9;
    pub(crate) const TARGET_FRONT: usize = 10;
    pub(crate) const TARGET_WINDING: usize = 11;
    pub(crate) const ASSUMED_MASK: usize = 12;
    pub(crate) const REVERSE_WINDING: usize = 13;
    pub(crate) const CHANGES_COORDINATES: usize = 14;
}

/// Operation-specific cells in a normal-generation root. Other cells retain the common prefix.
pub(super) mod normal_field {
    pub(crate) const REMOVED_METADATA: usize = 7;
    pub(crate) const OUTPUT_NORMAL_ATTRIBUTE: usize = 12;
    pub(crate) const EFFECTS: usize = 13;
    pub(crate) const OUTPUT_ATTRIBUTE_COUNT: usize = 14;

    pub(crate) const REPLACED_NORMAL: u32 = 1;
    pub(crate) const REMOVED_TANGENT: u32 = 2;
    pub(crate) const SMOOTH: u32 = 4;
}

pub(super) const ROOT_FIELDS: usize = 25;
const _: [(); ROOT_FIELDS] = [(); field::FACE_OFFSETS + 1];
