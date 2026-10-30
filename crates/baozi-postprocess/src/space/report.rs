use baozi_core::scene::integration::SceneSpaceConversionEvidence;
use baozi_core::{SceneGenerationId, SceneSpaceField, TargetSpace};

/// Source/target declarations and effects retained independently of either Scene generation.
pub struct SpaceConversionReport {
    evidence: SceneSpaceConversionEvidence,
}

impl SpaceConversionReport {
    pub(crate) const fn new(evidence: SceneSpaceConversionEvidence) -> Self {
        Self { evidence }
    }
    pub const fn input_generation(&self) -> SceneGenerationId {
        self.evidence.input_generation()
    }
    pub const fn output_generation(&self) -> SceneGenerationId {
        self.evidence.output_generation()
    }
    pub fn source(&self) -> TargetSpace {
        self.evidence.source()
    }
    pub fn target(&self) -> TargetSpace {
        self.evidence.target()
    }
    /// Whether an unknown input declaration was filled from the explicit source assumption.
    pub fn assumed_source_field(&self, field: SceneSpaceField) -> bool {
        self.evidence.assumed_source_field(field)
    }
    /// A reversed triangle maps output lanes to source lanes as `[0, 2, 1]`. Vertex and Face
    /// identities are preserved. Non-triangle supported streams retain their corner order.
    pub fn reverses_triangle_winding(&self) -> bool {
        self.evidence.reverses_triangle_winding()
    }
}

impl std::fmt::Debug for SpaceConversionReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SpaceConversionReport")
            .field("input_generation", &self.input_generation())
            .field("output_generation", &self.output_generation())
            .field("source", &self.source())
            .field("target", &self.target())
            .field(
                "reverses_triangle_winding",
                &self.reverses_triangle_winding(),
            )
            .finish()
    }
}
