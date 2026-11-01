use baozi_core::__scene_integration::integration::SceneTangentGenerationEvidence;
use baozi_core::PrimitiveId;

/// Published tangent-generation effects, retained independently of the Scene owner.
pub struct TangentGenerationReport {
    evidence: SceneTangentGenerationEvidence,
}

impl TangentGenerationReport {
    pub(crate) const fn new(evidence: SceneTangentGenerationEvidence) -> Self {
        Self { evidence }
    }

    /// Input target whose Vertex, Corner, and Face identities are preserved.
    pub const fn input_target(&self) -> PrimitiveId {
        self.evidence.input_target()
    }

    /// The target primitive in the new generation.
    pub const fn output_target(&self) -> PrimitiveId {
        self.evidence.output_target()
    }

    /// Whether replacing an authored TANGENT stream was explicitly authorized and applied.
    pub const fn replaced_authored_tangents(&self) -> bool {
        self.evidence.replaced_tangent()
    }

    /// The generated field's index in the Scene-wide attribute table.
    pub const fn output_tangent_attribute_index(&self) -> u32 {
        self.evidence.tangent_attribute_index()
    }
}

impl std::fmt::Debug for TangentGenerationReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TangentGenerationReport")
            .field("input_target", &self.input_target())
            .field("output_target", &self.output_target())
            .field(
                "replaced_authored_tangents",
                &self.replaced_authored_tangents(),
            )
            .field(
                "output_tangent_attribute_index",
                &self.output_tangent_attribute_index(),
            )
            .finish()
    }
}
