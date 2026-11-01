//! Direct triangulation; postprocess owns the runtime, core owns the Scene edit and publication.

pub(crate) mod input;

use crate::{PostProcessControl, PostProcessRunFailure, PostProcessRunOutcome};
use baozi_core::{PrimitiveId, ValidatedScene};

/// Lowers one polygon, strip, or fan through the shared selected transaction, preserving exact
/// source assembly and all retained attribute domains. Triangle lists are an exact no-op.
#[allow(clippy::result_large_err)]
pub fn triangulate_primitive(
    input: ValidatedScene,
    target: PrimitiveId,
    control: PostProcessControl,
) -> Result<PostProcessRunOutcome, PostProcessRunFailure> {
    #[cfg(feature = "tracing")]
    let _span = tracing::info_span!("baozi.postprocess.triangulate").entered();
    crate::prepare::run(
        input,
        &[target],
        crate::TrianglePreparationOptions::default(),
        control,
        crate::prepare::Invocation::Triangulation(target),
    )
}
