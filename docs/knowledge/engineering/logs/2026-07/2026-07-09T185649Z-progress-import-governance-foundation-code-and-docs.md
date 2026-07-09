---
type: "Progress Log"
title: "Progress: import governance foundation code and docs"
description: "Hardened Baozi import registry/report APIs, expanded IR validation/snapshots, added postprocess facade path, fuzz target, examples, benches, release/security docs, and ADRs."
timestamp: 2026-07-09T18:56:49Z
status: "active"
producer_id: "codex-root"
related_plan: "docs/plans/2026-07-09-006-refactor-import-governance-foundation-plan.md"
---

# Summary

Implemented the main import-governance foundation slice:

- `FormatInfo` now uses constructors/accessors and carries media type, encoding, sidecar policy,
  docs path, maturity, capabilities, and notes.
- `Importer::register` and `ImporterRegistry::register` reject duplicate format IDs with
  `BaoziErrorKind::DuplicateFormatId`.
- `ImportReport` owns private fields with accessors, stage, stats, `map_scene`, and explicit
  postprocess facade helpers.
- Scene IR now includes skins, morph targets, custom vertex attributes, richer cameras/lights,
  animation channels, material properties, texture samplers, and texture transforms.
- Validator and `SceneSnapshot` cover the expanded IR.
- Added `obj_postprocess` fuzz target and updated CI matrices.
- Added `CHANGELOG.md`, `SECURITY.md`, release policy, PLY/glTF shell docs, and ADRs 0024-0026.

# Verification So Far

- `cargo check --workspace --all-targets`
- `cargo check --manifest-path fuzz/Cargo.toml`
- `cargo test -p baozi-core --test scene_validation`
- `cargo test -p baozi-test-support --test snapshot`
- `cargo test -p baozi --test importer_api`
- `cargo test -p baozi --test obj_facade`

# Remaining

Run full CI-equivalent local gates, commit logical slice, push `main`, and follow GitHub Actions.
