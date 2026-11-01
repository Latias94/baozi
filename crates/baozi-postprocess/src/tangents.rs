//! Triangle UV Derivative v1 tangent generation using the concrete Scene edit transaction.

mod error;
pub(crate) mod input;
pub(crate) mod kernel;
mod options;
mod report;
mod wide;

#[cfg(test)]
mod runtime_tests;

pub use error::{TangentCause, TangentFailure};
pub use kernel::TangentIssue;
pub use options::{ExistingTangents, TangentOptions};
pub use report::TangentGenerationReport;

use crate::{PostProcessControl, PostProcessRunFailure, PostProcessRunOutcome};
use baozi_core::{PrimitiveId, ValidatedScene};

/// Generates Triangle UV Derivative tangents through the shared selected transaction.
#[allow(clippy::result_large_err)]
pub fn generate_tangents(
    input: ValidatedScene,
    target: PrimitiveId,
    options: TangentOptions,
    control: PostProcessControl,
) -> Result<PostProcessRunOutcome, PostProcessRunFailure> {
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.postprocess.tangents").entered();
    crate::prepare::run(
        input,
        &[target],
        crate::TrianglePreparationOptions {
            lowering: crate::TriangleLowering::Preserve,
            tangents: crate::TangentPolicy::Generate(options),
            ..Default::default()
        },
        control,
        crate::prepare::Invocation::Tangents(target, options),
    )
}
