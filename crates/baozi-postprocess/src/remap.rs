use std::{fmt, ops::Range};

use baozi_core::{__scene_integration::integration::SceneTriangulationEvidence, PrimitiveId};
use thiserror::Error;

/// A generation- and mesh-qualified source vertex endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrimitiveVertex {
    pub primitive: PrimitiveId,
    pub index: u32,
}

/// A generation- and mesh-qualified corner endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrimitiveCorner {
    pub primitive: PrimitiveId,
    pub index: u32,
}

/// A generation- and mesh-qualified face endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrimitiveFace {
    pub primitive: PrimitiveId,
    pub index: u32,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TriangulationRemapError {
    #[error("the remap endpoint belongs to a different primitive or generation")]
    ForeignOwner,
    #[error("the remap endpoint index is outside its domain")]
    OutOfRange,
}

/// A direct triangulation's complete input-to-final Corner/Face relation.
///
/// The actual rooted evidence store is owned here, independently of either Scene. Borrowing this
/// value borrows the operation's retained origins; no intermediate Scene generation is invented.
pub struct TriangulationRemap {
    evidence: SceneTriangulationEvidence,
}

impl TriangulationRemap {
    pub(crate) const fn new(evidence: SceneTriangulationEvidence) -> Self {
        Self { evidence }
    }

    pub fn input_target(&self) -> PrimitiveId {
        self.evidence.input_target()
    }
    pub fn output_target(&self) -> PrimitiveId {
        self.evidence.output_target()
    }

    /// Returns one source corner ordinal for every output corner, in triangle order.
    pub fn source_corner_origins(&self) -> &[u32] {
        self.evidence.source_corner_origins()
    }

    /// Returns source-face output-triangle prefix offsets, including both endpoints.
    pub fn source_face_triangle_offsets(&self) -> &[u32] {
        self.evidence.source_face_triangle_offsets()
    }

    pub const fn source_corner_count(&self) -> u32 {
        self.evidence.source_corner_count()
    }
    pub const fn vertex_count(&self) -> u32 {
        self.evidence.vertex_count()
    }

    /// Triangulation preserves all vertex values and their order; no vertex is added or removed.
    pub const fn preserves_vertex_identity(&self) -> bool {
        true
    }

    /// Authored fields, including normals/tangents, are preserved by their source domain origins.
    pub const fn preserves_source_fields(&self) -> bool {
        true
    }

    /// Bounds are affected; this declaration does not assert that a bounds cache exists.
    pub const fn invalidates_bounds(&self) -> bool {
        true
    }

    pub fn corner_origin(
        &self,
        endpoint: PrimitiveCorner,
    ) -> Result<PrimitiveCorner, TriangulationRemapError> {
        if endpoint.primitive != self.output_target() {
            return Err(TriangulationRemapError::ForeignOwner);
        }
        let index = *self
            .source_corner_origins()
            .get(endpoint.index as usize)
            .ok_or(TriangulationRemapError::OutOfRange)?;
        Ok(PrimitiveCorner {
            primitive: self.input_target(),
            index,
        })
    }

    /// Enumerates every occurrence of one source corner; repeated appearances remain explicit.
    pub fn corner_images(
        &self,
        endpoint: PrimitiveCorner,
    ) -> Result<impl Iterator<Item = PrimitiveCorner> + '_, TriangulationRemapError> {
        if endpoint.primitive != self.input_target() {
            return Err(TriangulationRemapError::ForeignOwner);
        }
        if endpoint.index >= self.source_corner_count() {
            return Err(TriangulationRemapError::OutOfRange);
        }
        Ok(self
            .source_corner_origins()
            .iter()
            .enumerate()
            .filter(move |(_, source)| **source == endpoint.index)
            .map(move |(index, _)| PrimitiveCorner {
                primitive: self.output_target(),
                index: index as u32,
            }))
    }

    /// Returns a contiguous output-face range; its endpoints belong to `output_target()`.
    pub fn face_range(
        &self,
        endpoint: PrimitiveFace,
    ) -> Result<Range<u32>, TriangulationRemapError> {
        if endpoint.primitive != self.input_target() {
            return Err(TriangulationRemapError::ForeignOwner);
        }
        let offsets = self.source_face_triangle_offsets();
        let index = endpoint.index as usize;
        let pair = offsets
            .get(index..)
            .and_then(|tail| tail.get(..2))
            .ok_or(TriangulationRemapError::OutOfRange)?;
        Ok(pair[0]..pair[1])
    }

    pub fn face_origin(
        &self,
        endpoint: PrimitiveFace,
    ) -> Result<PrimitiveFace, TriangulationRemapError> {
        if endpoint.primitive != self.output_target() {
            return Err(TriangulationRemapError::ForeignOwner);
        }
        let offsets = self.source_face_triangle_offsets();
        if offsets.last().is_none_or(|&end| endpoint.index >= end) {
            return Err(TriangulationRemapError::OutOfRange);
        }
        let index = offsets.partition_point(|&start| start <= endpoint.index) - 1;
        Ok(PrimitiveFace {
            primitive: self.input_target(),
            index: index as u32,
        })
    }
}

impl fmt::Debug for TriangulationRemap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TriangulationRemap")
            .field("input", &self.input_target())
            .field("output", &self.output_target())
            .field("source_corners", &self.source_corner_count())
            .field("output_corners", &self.source_corner_origins().len())
            .finish_non_exhaustive()
    }
}
