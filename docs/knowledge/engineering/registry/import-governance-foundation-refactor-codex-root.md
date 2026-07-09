---
type: "Work Registration"
title: "Import governance foundation refactor"
description: "Registration for Import governance foundation refactor."
timestamp: 2026-07-09T10:05:26Z
status: "active"
last_seen: 2026-07-09T19:20:00Z
producer_id: "codex-root"
related_plan: "docs\\plans\\2026-07-09-006-refactor-import-governance-foundation-plan.md"
git_branch: "main"
---

# Scope

Implement the import-governance foundation plan: harden facade/import registry public API, make
FormatInfo future-extensible, add ImportReport stats/stage accessors, expand Scene IR for future
PLY/glTF/FBX surfaces, extend validation/snapshots/fuzz gates, and add release/security docs.

# Current Claim

Core implementation is complete and locally checked:

- `cargo check --workspace --all-targets`
- `cargo check --manifest-path fuzz/Cargo.toml`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace`
- `cargo test --doc --workspace --all-features`
- `cargo doc --workspace --all-features --no-deps` with `RUSTDOCFLAGS=-D warnings`
- no-default, all-formats/native-fs, wasm32-unknown-unknown, and wasm32-wasip1 checks
- `cargo deny check`
- `cargo +nightly-2026-05-27 fuzz check stl_import`
- `cargo +nightly-2026-05-27 fuzz check obj_import`
- `cargo +nightly-2026-05-27 fuzz check obj_postprocess`
- `cargo bench -p baozi --no-run`

Local gaps: `go` is not installed, so actionlint was not run locally. Windows fuzz run smoke failed
with `STATUS_DLL_NOT_FOUND`, matching the documented ASan DLL/toolchain issue; Linux CI remains the
sanitizer authority.

# Latest Links

- `docs/plans/2026-07-09-006-refactor-import-governance-foundation-plan.md`
- `docs/adr/0024-public-api-hardening-and-stability-tiers.md`
- `docs/adr/0025-scene-ir-invariants-and-validation-boundaries.md`
- `docs/adr/0026-validation-snapshot-and-fuzz-gates.md`
- `CHANGELOG.md`
- `SECURITY.md`

# Handoff

If resumed before the final push, start with `git status --short`, commit the staged foundation
slice with a conventional commit, push `main`, then watch GitHub Actions. Pay attention to
`cargo-deny` wildcard warnings from `workspace = true`; the command exits successfully with the
current config.

# Citations
