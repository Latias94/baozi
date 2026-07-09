use crate::pipeline::PostProcessStep;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostProcessPreset {
    Raw,
    RealtimeFast,
    RealtimeQuality,
    RealtimeMaxQuality,
    ToolingPreserve,
}

impl PostProcessPreset {
    pub fn steps(self) -> &'static [PostProcessStep] {
        match self {
            Self::Raw => &[PostProcessStep::ValidateScene],
            Self::RealtimeFast => &[
                PostProcessStep::ValidateScene,
                PostProcessStep::Triangulate,
                PostProcessStep::GenerateBoundingBoxes,
            ],
            Self::RealtimeQuality => &[
                PostProcessStep::ValidateScene,
                PostProcessStep::Triangulate,
                PostProcessStep::GenerateNormals,
                PostProcessStep::GenerateBoundingBoxes,
            ],
            Self::RealtimeMaxQuality => &[
                PostProcessStep::ValidateScene,
                PostProcessStep::Triangulate,
                PostProcessStep::GenerateNormals,
                PostProcessStep::GenerateBoundingBoxes,
            ],
            Self::ToolingPreserve => &[
                PostProcessStep::ValidateScene,
                PostProcessStep::GenerateBoundingBoxes,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_only_include_implemented_steps() {
        for preset in [
            PostProcessPreset::Raw,
            PostProcessPreset::RealtimeFast,
            PostProcessPreset::RealtimeQuality,
            PostProcessPreset::RealtimeMaxQuality,
            PostProcessPreset::ToolingPreserve,
        ] {
            assert!(preset.steps().iter().all(|step| step.is_implemented()));
        }
    }
}
