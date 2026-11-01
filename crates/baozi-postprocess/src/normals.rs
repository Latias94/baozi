//! Flat and smooth normal generation using the concrete Scene edit transaction.

mod error;
pub(crate) mod input;
pub(crate) mod kernel;
mod options;
mod report;

#[cfg(test)]
mod runtime_tests;

pub use error::{NormalCause, NormalFailure, NormalIssue};
pub use options::{ExistingNormals, NormalMode, NormalOptions};
pub use report::NormalGenerationReport;

use crate::{PostProcessControl, PostProcessRunFailure, PostProcessRunOutcome};
use baozi_core::{PrimitiveId, ValidatedScene};

/// Generates normals through the shared selected transaction. The explicit Tangent policy can
/// remove invalidated tangents or generate their replacement after the normals. Preserving an
/// existing Normal still allows the requested Tangent stage to run; the report records each
/// actual field effect.
#[allow(clippy::result_large_err)]
pub fn generate_normals(
    input: ValidatedScene,
    target: PrimitiveId,
    options: NormalOptions,
    tangents: crate::TangentPolicy,
    control: PostProcessControl,
) -> Result<PostProcessRunOutcome, PostProcessRunFailure> {
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.postprocess.normals").entered();
    crate::prepare::run(
        input,
        &[target],
        crate::TrianglePreparationOptions {
            lowering: crate::TriangleLowering::Preserve,
            normals: Some(options),
            tangents,
            ..Default::default()
        },
        control,
        crate::prepare::Invocation::Normals(target, options),
    )
}
