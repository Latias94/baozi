//! Core-owned, authority-free canonical Scene construction.
//!
//! `SceneConstruction` is the only core value that may carry an authored Scene batch across the
//! import seam before semantic preflight.  It owns the construction brand and bounded input
//! envelope, but it never owns an operation, store key, physical account, or outcome authority.
//! The import session remains responsible for pairing it with its sole retained-store owner.

use std::fmt;

use super::super::construction::SceneConstructionStartError as CoreSceneConstructionStartError;
use super::super::construction::node_scene::NodeSceneActivationRequest;
use super::super::production::input::SceneAuthoringBatch;
use crate::validation_control::SceneValidationLimits;

/// One authority-free, single-use canonical Scene construction.
///
/// The value is deliberately not `Clone`, `Copy`, or `Debug`: duplicating a construction would
/// create a second fact source for the same authored batch and construction brand.  Callers must
/// consume it exactly once through the hidden import seam, where the session-owned outcome owner
/// supplies the retained-store authority.
///
/// ```compile_fail,E0599
/// fn duplicate(
///     construction: baozi_core::__canonical_import::SceneConstruction<'_>,
/// ) -> baozi_core::__canonical_import::SceneConstruction<'_> {
///     construction.clone()
/// }
/// ```
#[must_use = "a SceneConstruction must be submitted to one import session owner"]
pub struct SceneConstruction<'input>(NodeSceneActivationRequest<'input>);

impl<'input> SceneConstruction<'input> {
    /// Admits a complete authoring batch without acquiring operation or storage authority.
    #[allow(
        clippy::result_large_err,
        reason = "the bounded construction error remains allocation-free across the hidden seam"
    )]
    pub fn try_new(
        input: SceneAuthoringBatch<'input>,
        validation_limits: SceneValidationLimits,
    ) -> Result<Self, SceneConstructionStartError> {
        NodeSceneActivationRequest::try_new(input, validation_limits)
            .map(Self)
            .map_err(SceneConstructionStartError)
    }

    /// Transfers the private request to the import-only lane adapter.
    pub(in crate::canonical_scene::store) fn into_node_scene_request(
        self,
    ) -> NodeSceneActivationRequest<'input> {
        self.0
    }
}

/// Failure while admitting an authority-free [`SceneConstruction`].
#[derive(Debug)]
pub struct SceneConstructionStartError(CoreSceneConstructionStartError);

impl fmt::Display for SceneConstructionStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for SceneConstructionStartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}
