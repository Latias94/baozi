use super::NormalMode;
use baozi_core::PrimitiveId;
use baozi_core::scene::integration::SceneNormalGenerationEvidence;

/// Published normal-generation effects, retained independently of the Scene owner.
pub struct NormalGenerationReport {
    evidence: SceneNormalGenerationEvidence,
}

impl NormalGenerationReport {
    pub(crate) const fn new(evidence: SceneNormalGenerationEvidence) -> Self {
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
    /// The normal algorithm applied to this primitive.
    pub const fn mode(&self) -> NormalMode {
        if self.evidence.smooth() {
            NormalMode::Smooth
        } else {
            NormalMode::Flat
        }
    }
    /// Whether replacing authored NORMAL values was explicitly authorized and applied.
    pub const fn replaced_authored_normals(&self) -> bool {
        self.evidence.replaced_normal()
    }
    /// Whether deleting an invalidated TANGENT field was explicitly authorized and applied.
    pub const fn removed_tangents(&self) -> bool {
        self.evidence.removed_tangent()
    }
    /// Number of owner-local metadata entries removed with the deleted TANGENT field.
    pub const fn removed_metadata_entries(&self) -> u32 {
        self.evidence.removed_metadata_entries()
    }
    /// The generated field's index in the Scene-wide attribute table.
    pub const fn output_normal_attribute_index(&self) -> u32 {
        self.evidence.normal_attribute_index()
    }
}

impl std::fmt::Debug for NormalGenerationReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NormalGenerationReport")
            .field("input_target", &self.input_target())
            .field("output_target", &self.output_target())
            .field("mode", &self.mode())
            .field(
                "replaced_authored_normals",
                &self.replaced_authored_normals(),
            )
            .field("removed_tangents", &self.removed_tangents())
            .field("removed_metadata_entries", &self.removed_metadata_entries())
            .finish()
    }
}
