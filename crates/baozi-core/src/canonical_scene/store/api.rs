//! Public and integration-facing views over the retained canonical Scene model.

use super::construction::node_scene::NodeSceneDraft;

#[cfg(feature = "_scene-integration")]
pub(crate) mod authoring;
pub(crate) mod construction;
#[cfg(feature = "_scene-integration")]
pub(crate) mod integration;
pub(crate) mod scene;

/// A non-owning descriptor for one materialized canonical Scene directory.
///
/// This type is intentionally neither `Clone` nor `Copy`. The import session carries its matching
/// retained owner separately and must consume both values together.
#[must_use = "a Scene draft descriptor must be centrally prepared with its retained owner"]
pub struct SceneDraft(NodeSceneDraft);
