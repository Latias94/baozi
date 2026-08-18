/// A payload-free primitive topology category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TopologyKind {
    Points,
    LineList,
    LineStrip,
    LineLoop,
    TriangleList,
    TriangleStrip,
    TriangleFan,
    Polygons,
}

/// Cardinalities proven by canonical primitive validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrimitiveCardinality {
    vertex_count: u32,
    corner_count: u32,
    face_count: u32,
}

impl PrimitiveCardinality {
    pub(crate) const fn from_validated_counts(
        vertex_count: u32,
        corner_count: u32,
        face_count: u32,
    ) -> Self {
        Self {
            vertex_count,
            corner_count,
            face_count,
        }
    }

    pub const fn vertex_count(self) -> u32 {
        self.vertex_count
    }

    pub const fn corner_count(self) -> u32 {
        self.corner_count
    }

    pub const fn face_count(self) -> u32 {
        self.face_count
    }
}
