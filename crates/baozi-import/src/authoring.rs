//! Canonical Scene authoring values for custom format importers.
//!
//! These values describe canonical input without carrying storage, ledger, activation, or
//! publication authority. Submit a complete batch through [`crate::ParseScope::submit_scene`].
//! Static plugins can submit [`StaticSceneBuilder`] through
//! [`crate::ParseScope::submit_static_scene`] to receive document provenance automatically.
//!
//! The vocabulary intentionally has no duplicate crate-root facade.
//!
//! ```compile_fail,E0432
//! use baozi_import::SceneAuthoringBatch;
//! ```

pub use baozi_core::scene::authoring::*;

pub(crate) mod static_scene;

pub use static_scene::StaticSceneBuilder;
