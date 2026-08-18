//! Public and integration-facing views over the retained canonical Scene model.

use super::construction::node_scene::NodeSceneDraft;

pub(crate) mod authoring;
#[cfg(feature = "_u4-import")]
pub(crate) mod import;
pub(crate) mod scene;

/// A non-owning descriptor for one materialized canonical Scene directory.
///
/// This type is intentionally neither `Clone` nor `Copy`. The import session carries its matching
/// retained owner separately and must consume both values together.
#[must_use = "a Scene draft descriptor must be centrally prepared with its retained owner"]
pub struct SceneDraft(NodeSceneDraft);
