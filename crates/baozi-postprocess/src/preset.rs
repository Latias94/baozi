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
                PostProcessStep::FindDegenerates,
                PostProcessStep::FindInvalidData,
                PostProcessStep::GenerateNormals,
                PostProcessStep::GenerateTangents,
                PostProcessStep::GenerateBoundingBoxes,
            ],
            Self::RealtimeMaxQuality => &[
                PostProcessStep::ValidateScene,
                PostProcessStep::Triangulate,
                PostProcessStep::FindDegenerates,
                PostProcessStep::FindInvalidData,
                PostProcessStep::JoinIdenticalVertices,
                PostProcessStep::GenerateNormals,
                PostProcessStep::GenerateTangents,
                PostProcessStep::GenerateBoundingBoxes,
                PostProcessStep::OptimizeMeshes,
                PostProcessStep::OptimizeGraph,
            ],
            Self::ToolingPreserve => &[
                PostProcessStep::ValidateScene,
                PostProcessStep::FindInvalidData,
                PostProcessStep::GenerateBoundingBoxes,
            ],
        }
    }
}
