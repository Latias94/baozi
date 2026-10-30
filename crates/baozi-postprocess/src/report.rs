use std::fmt;

use baozi_core::{PrimitiveId, SceneGenerationId, ValidatedScene};

use crate::{
    NormalGenerationReport, NormalOptions, PostProcessResourceEvidence, TriangulationRemap,
};

/// The concrete operation evaluated against an input Scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostProcessOperation {
    /// Triangulate one generation-qualified input primitive.
    Triangulate { target: PrimitiveId },
    /// Generate a normal field with explicit authored-field treatment.
    GenerateNormals {
        target: PrimitiveId,
        options: NormalOptions,
    },
}

/// The successful disposition of one direct post-process invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PostProcessRunDisposition {
    /// The operation required no canonical write and returned the original Scene.
    Unchanged,
    /// A centrally validated replacement and its operation evidence were published together.
    Changed,
}

/// Immutable evidence for one direct operation.
#[derive(Debug)]
pub struct PostProcessReport {
    operation: PostProcessOperation,
    output_generation: SceneGenerationId,
    resources: PostProcessResourceEvidence,
    result: OperationResult,
}

#[derive(Debug)]
enum OperationResult {
    Unchanged,
    Triangulation(TriangulationRemap),
    Normals(NormalGenerationReport),
}

impl PostProcessReport {
    pub(crate) fn triangulation(
        target: PrimitiveId,
        output_generation: SceneGenerationId,
        resources: PostProcessResourceEvidence,
        remap: Option<TriangulationRemap>,
    ) -> Self {
        Self {
            operation: PostProcessOperation::Triangulate { target },
            output_generation,
            resources,
            result: match remap {
                Some(remap) => OperationResult::Triangulation(remap),
                None => OperationResult::Unchanged,
            },
        }
    }

    pub(crate) fn normals(
        target: PrimitiveId,
        options: NormalOptions,
        output_generation: SceneGenerationId,
        resources: PostProcessResourceEvidence,
        normal: Option<NormalGenerationReport>,
    ) -> Self {
        Self {
            operation: PostProcessOperation::GenerateNormals { target, options },
            output_generation,
            resources,
            result: match normal {
                Some(report) => OperationResult::Normals(report),
                None => OperationResult::Unchanged,
            },
        }
    }

    /// Returns the published normal-field and explicit source-loss evidence for a changed operation.
    pub const fn normal_generation(&self) -> Option<&NormalGenerationReport> {
        match &self.result {
            OperationResult::Normals(report) => Some(report),
            _ => None,
        }
    }

    /// Returns whether the operation preserved the input or published a replacement.
    pub const fn disposition(&self) -> PostProcessRunDisposition {
        match self.result {
            OperationResult::Unchanged => PostProcessRunDisposition::Unchanged,
            _ => PostProcessRunDisposition::Changed,
        }
    }

    /// Returns the concrete operation and its input target.
    pub const fn operation(&self) -> PostProcessOperation {
        self.operation
    }

    /// Returns the generation of the input target evaluated by this invocation.
    pub const fn input_generation(&self) -> SceneGenerationId {
        match self.operation {
            PostProcessOperation::Triangulate { target }
            | PostProcessOperation::GenerateNormals { target, .. } => target.generation(),
        }
    }

    /// Returns the generation owned by the successful result.
    pub const fn output_generation(&self) -> SceneGenerationId {
        self.output_generation
    }

    /// Returns the actual retained Corner/Face relation for a changed triangulation.
    pub const fn triangulation_remap(&self) -> Option<&TriangulationRemap> {
        match &self.result {
            OperationResult::Triangulation(remap) => Some(remap),
            _ => None,
        }
    }

    /// Returns whether the report is generation-bound to the provided Scene.
    pub fn matches_scene(&self, scene: &ValidatedScene) -> bool {
        self.output_generation == scene.generation()
    }

    /// Returns bounded resource evidence without exposing ledger keys or phases.
    pub const fn resources(&self) -> &PostProcessResourceEvidence {
        &self.resources
    }
}

/// The complete successful result of one direct operation.
#[must_use = "the post-process success carries the preserved or newly published Scene"]
pub struct PostProcessRunOutcome {
    scene: ValidatedScene,
    report: PostProcessReport,
}

impl PostProcessRunOutcome {
    pub(crate) const fn new(scene: ValidatedScene, report: PostProcessReport) -> Self {
        Self { scene, report }
    }

    /// Returns the successful disposition.
    pub const fn disposition(&self) -> PostProcessRunDisposition {
        self.report.disposition()
    }

    /// Returns the preserved input or newly published Scene.
    pub const fn scene(&self) -> &ValidatedScene {
        &self.scene
    }

    /// Returns immutable invocation evidence.
    pub const fn report(&self) -> &PostProcessReport {
        &self.report
    }

    /// Consumes the success into the Scene owner and invocation report.
    pub fn into_parts(self) -> (ValidatedScene, PostProcessReport) {
        (self.scene, self.report)
    }
}

impl fmt::Debug for PostProcessRunOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostProcessRunOutcome")
            .field("scene_generation", &self.scene.generation())
            .field(
                "scene_canonical_block_count",
                &self.scene.canonical_block_count(),
            )
            .field("report", &self.report)
            .finish()
    }
}
