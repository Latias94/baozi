use baozi_core::__scene_integration::integration::{
    SceneGeometrySelectionEvidence, SceneGeometryTargetEvidence,
};

pub struct GeometryPreparationReport {
    pub(crate) evidence: SceneGeometrySelectionEvidence,
}
impl GeometryPreparationReport {
    pub(crate) const fn new(evidence: SceneGeometrySelectionEvidence) -> Self {
        Self { evidence }
    }
    pub fn reconstruction_work_items(&self) -> u64 {
        self.evidence.reconstruction_work_items()
    }
    pub fn geometry_reconstruction_count(&self) -> u32 {
        self.evidence.geometry_reconstruction_count()
    }
    pub fn reconstructed_primitive_count(&self) -> usize {
        self.evidence.reconstructed_primitive_count() as usize
    }
    pub fn len(&self) -> usize {
        self.evidence.len()
    }
    pub fn is_empty(&self) -> bool {
        self.evidence.is_empty()
    }
    pub fn target(&self, index: usize) -> Option<PreparedPrimitiveReport<'_>> {
        self.evidence
            .target(index)
            .map(|evidence| PreparedPrimitiveReport { evidence })
    }
    pub fn targets(&self) -> impl ExactSizeIterator<Item = PreparedPrimitiveReport<'_>> {
        (0..self.len()).map(|i| self.target(i).expect("published target"))
    }
}
impl std::fmt::Debug for GeometryPreparationReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeometryPreparationReport")
            .field("selected_targets", &self.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy)]
pub struct PreparedPrimitiveReport<'a> {
    evidence: SceneGeometryTargetEvidence<'a>,
}
impl<'a> PreparedPrimitiveReport<'a> {
    /// Whether this target's geometry changed in the shared publication. Even an unchanged
    /// target's output ID belongs to the final generation when another selected target changed.
    pub fn disposition(self) -> crate::PostProcessRunDisposition {
        if self.evidence.changed() {
            crate::PostProcessRunDisposition::Changed
        } else {
            crate::PostProcessRunDisposition::Unchanged
        }
    }
    pub fn input_target(self) -> baozi_core::PrimitiveId {
        self.evidence.input_target()
    }
    pub fn output_target(self) -> baozi_core::PrimitiveId {
        self.evidence.output_target()
    }
    pub fn source_corner_origins(self) -> Option<&'a [u32]> {
        self.evidence.corner_origins()
    }
    pub fn source_face_triangle_offsets(self) -> Option<&'a [u32]> {
        self.evidence.face_offsets()
    }
    pub fn output_normal_attribute_index(self) -> Option<u32> {
        self.evidence.normal_attribute_index()
    }
    pub fn output_tangent_attribute_index(self) -> Option<u32> {
        self.evidence.tangent_attribute_index()
    }
    pub fn removed_tangents(self) -> bool {
        self.evidence.removed_tangent()
    }
}
