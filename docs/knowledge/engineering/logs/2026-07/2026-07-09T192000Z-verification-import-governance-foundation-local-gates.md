---
type: "Verification Log"
title: "Verification: import governance foundation local gates"
description: "Local verification for the import-governance foundation refactor before commit and push."
timestamp: 2026-07-09T19:20:00Z
status: "complete"
producer_id: "codex-root"
related_plan: "docs/plans/2026-07-09-006-refactor-import-governance-foundation-plan.md"
---

# Passed

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace`
- `cargo test --doc --workspace --all-features`
- `cargo doc --workspace --all-features --no-deps` with `RUSTDOCFLAGS=-D warnings`
- `cargo check -p baozi --no-default-features`
- `cargo check -p baozi --features all-formats,native-fs`
- `cargo check -p baozi --target wasm32-unknown-unknown --no-default-features --features format-stl,format-obj`
- `cargo check -p baozi --target wasm32-wasip1 --no-default-features --features format-stl,format-obj,native-fs`
- `cargo deny check`
- `cargo check --manifest-path fuzz/Cargo.toml`
- `cargo +nightly-2026-05-27 fuzz check stl_import`
- `cargo +nightly-2026-05-27 fuzz check obj_import`
- `cargo +nightly-2026-05-27 fuzz check obj_postprocess`
- `cargo bench -p baozi --no-run`
- `git diff --check`

# Local Environment Gaps

- `go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.12` did not run because `go` is not
  installed locally. GitHub Actions still runs workflow lint.
- `cargo +nightly-2026-05-27 fuzz run stl_import -- -runs=32` built the target but failed to start
  with `STATUS_DLL_NOT_FOUND`, the documented Windows MSVC ASan runtime issue. Linux CI remains the
  sanitizer evidence gate.

# Notes

`cargo deny check` exits successfully. It still emits wildcard warnings for `workspace = true`
internal dependencies because cargo-deny classifies workspace path dependencies as wildcard-like.
