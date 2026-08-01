//! Private retained-store tracer for the canonical Scene migration.

pub(crate) mod api;
mod construction;
mod node_scene;
mod production;
mod schema;
mod validation;

#[cfg(test)]
pub(crate) use construction::node_scene::edit::CanonicalSubjectRemap;
pub(crate) use construction::node_scene::edit::{
    CanonicalInvalidations, CanonicalRemapSet, CanonicalRemapSetError,
};

#[cfg(test)]
mod tests;
