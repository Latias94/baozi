#![forbid(unsafe_code)]

//! Shared workflow for the edit-lifecycle fuzz entry and deterministic seed regressions.

pub mod edit_lifecycle;

#[allow(
    dead_code,
    reason = "the shared support also serves parser-only targets"
)]
#[path = "../fuzz_targets/support.rs"]
mod support;

#[cfg(test)]
mod edit_lifecycle_tests;
