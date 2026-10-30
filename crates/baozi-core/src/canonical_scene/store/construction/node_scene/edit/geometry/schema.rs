//! Concrete rooted outcome evidence for the one triangulation edit.

use baozi_operation::RecordBlock;

use crate::canonical_scene::store::schema::CanonicalSchema;

/// Root tag for an edit outcome; distinct from fresh-import Scene evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneGeometryEditEvidenceTag;

/// Flattened primitive-local source-corner triples, in output triangle order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneGeometryCornerOriginsTag;

/// Source-face to contiguous output-triangle prefix offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneGeometryFaceOffsetsTag;

impl CanonicalSchema for SceneGeometryEditEvidenceTag {
    type StoreKind = RecordBlock;
}

pub(super) const REVISION: u32 = 2;
pub(super) const OPERATION_TRIANGULATE: u32 = 1;
pub(super) const TRIANGULATION_SEMANTIC_VERSION: u32 = 1;
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
    pub(crate) const SCENE_COUNTS: usize = 20;
    pub(crate) const CORNER_ORIGINS: usize = 21;
    pub(crate) const FACE_OFFSETS: usize = 22;
}

pub(super) const ROOT_FIELDS: usize = 23;
const _: [(); ROOT_FIELDS] = [(); field::FACE_OFFSETS + 1];
