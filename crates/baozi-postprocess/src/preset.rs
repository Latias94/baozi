use crate::pipeline::PostProcessStep;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostProcessPreset {
    Raw,
    RealtimeFast,
    RealtimeQuality,
    ToolingPreserve,
}

impl PostProcessPreset {
    pub fn steps(self) -> &'static [PostProcessStep] {
        match self {
            Self::Raw => &[],
            Self::RealtimeFast => &[
                PostProcessStep::Triangulate,
                PostProcessStep::GenerateBoundingBoxes,
            ],
            Self::RealtimeQuality => &[
                PostProcessStep::Triangulate,
                PostProcessStep::GenerateNormals,
                PostProcessStep::GenerateBoundingBoxes,
            ],
            Self::ToolingPreserve => &[PostProcessStep::GenerateBoundingBoxes],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_preset_is_an_empty_recipe() {
        assert!(PostProcessPreset::Raw.steps().is_empty());
    }

    #[test]
    fn quality_presets_have_distinct_expansions() {
        assert_ne!(
            PostProcessPreset::RealtimeFast.steps(),
            PostProcessPreset::RealtimeQuality.steps()
        );
    }
}
