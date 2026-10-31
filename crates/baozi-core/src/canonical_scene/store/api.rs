//! Public and integration-facing views over the retained canonical Scene model.

use super::construction::node_scene::NodeSceneDraft;

pub(crate) mod authoring;
pub(crate) mod construction;
pub(crate) mod integration;
pub(crate) mod read;
pub(crate) mod scene;
pub(crate) mod validation;

/// A non-owning descriptor for one materialized canonical Scene directory.
///
/// This type is intentionally neither `Clone` nor `Copy`. The import session carries its matching
/// retained owner separately and must consume both values together.
#[must_use = "a Scene draft descriptor must be centrally prepared with its retained owner"]
pub struct SceneDraft(NodeSceneDraft);
