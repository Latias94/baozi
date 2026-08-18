//! Experimental Scene authoring values for custom format importers.
//!
//! These values describe canonical input without carrying storage, ledger, activation, or
//! publication authority. Submit a complete batch through [`crate::ParseScope::submit_scene`].
//!
//! The vocabulary intentionally has no duplicate crate-root facade.
//!
//! ```compile_fail,E0432
//! use baozi_import::SceneAuthoringBatch;
//! ```

pub use baozi_core::__canonical_authoring::*;
