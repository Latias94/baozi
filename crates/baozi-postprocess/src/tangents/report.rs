use baozi_core::__scene_integration::integration::SceneGeometrySelectionEvidence;
use baozi_core::PrimitiveId;

/// Published tangent-generation effects, retained independently of the Scene owner.
pub struct TangentGenerationReport {
    preparation: crate::GeometryPreparationReport,
}

impl TangentGenerationReport {
    pub const fn preparation(&self) -> &crate::GeometryPreparationReport {
        &self.preparation
    }
    pub(crate) const fn selected(evidence: SceneGeometrySelectionEvidence) -> Self {
        Self {
            preparation: crate::GeometryPreparationReport::new(evidence),
        }
    }

    /// Input target whose Vertex, Corner, and Face identities are preserved.
    pub fn input_target(&self) -> PrimitiveId {
        self.preparation
            .evidence
            .target(0)
            .expect("published tangent target")
            .input_target()
    }

    /// The target primitive in the new generation.
    pub fn output_target(&self) -> PrimitiveId {
        self.preparation
            .evidence
            .target(0)
            .expect("published tangent target")
            .output_target()
    }

    /// Whether replacing an authored TANGENT stream was explicitly authorized and applied.
    pub fn replaced_authored_tangents(&self) -> bool {
        self.preparation
            .evidence
            .target(0)
            .expect("published tangent target")
            .replaced_tangent()
    }

    /// The generated field's index in the Scene-wide attribute table.
    pub fn output_tangent_attribute_index(&self) -> u32 {
        self.preparation
            .evidence
            .target(0)
            .expect("published tangent target")
            .tangent_attribute_index()
            .expect("published tangent")
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
