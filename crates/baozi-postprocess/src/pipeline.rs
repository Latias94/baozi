use baozi_core::{Result, Scene};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostProcessStage {
    RawImported,
    ValidatedImported,
    PostProcessed,
    ValidatedOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostProcessStep {
    ValidateScene,
    ApplyGlobalScale,
    NormalizeCoordinates,
    Triangulate,
    SortByPrimitiveType,
    FindDegenerates,
    FindInvalidData,
    JoinIdenticalVertices,
    GenerateNormals,
    GenerateTangents,
    GenerateBoundingBoxes,
    OptimizeMeshes,
    OptimizeGraph,
}

impl PostProcessStep {
    pub const CANONICAL_ORDER: &'static [Self] = &[
        Self::ValidateScene,
        Self::ApplyGlobalScale,
        Self::NormalizeCoordinates,
        Self::Triangulate,
        Self::SortByPrimitiveType,
        Self::FindDegenerates,
        Self::FindInvalidData,
        Self::JoinIdenticalVertices,
        Self::GenerateNormals,
        Self::GenerateTangents,
        Self::GenerateBoundingBoxes,
        Self::OptimizeMeshes,
        Self::OptimizeGraph,
    ];

    pub fn is_destructive(self) -> bool {
        !matches!(self, Self::ValidateScene | Self::GenerateBoundingBoxes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostProcessPipeline {
    steps: Vec<PostProcessStep>,
}

impl PostProcessPipeline {
    pub fn new(steps: impl IntoIterator<Item = PostProcessStep>) -> Self {
        let requested: Vec<_> = steps.into_iter().collect();
        let steps = PostProcessStep::CANONICAL_ORDER
            .iter()
            .copied()
            .filter(|step| requested.contains(step))
            .collect();
        Self { steps }
    }

    pub fn steps(&self) -> &[PostProcessStep] {
        &self.steps
    }

    pub fn run(&self, scene: Scene) -> Result<Scene> {
        Ok(scene)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_uses_canonical_order() {
        let pipeline = PostProcessPipeline::new([
            PostProcessStep::GenerateNormals,
            PostProcessStep::Triangulate,
            PostProcessStep::ValidateScene,
        ]);
        assert_eq!(
            pipeline.steps(),
            &[
                PostProcessStep::ValidateScene,
                PostProcessStep::Triangulate,
                PostProcessStep::GenerateNormals
            ]
        );
    }
}
