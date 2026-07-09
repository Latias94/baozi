---
type: Work Progress
title: OBJ MTL importer U6-U8 progress
timestamp: 2026-07-09T09:16:05Z
tags: baozi,obj,mtl,ce-work
status: active
related_plan: docs/plans/2026-07-09-005-feat-obj-mtl-importer-plan.md
git_branch: main
---

# Summary

The OBJ/MTL importer plan has completed the public facade, fuzz/CI, and format documentation slices in the working tree.

# Completed Since Commit 699944f

- Added `crates/baozi/tests/obj_facade.rs` to prove public `Importer` behavior for bytes, repeated use, content detection, memory MTL sidecars, and post-process triangulation.
- Added `obj_import` cargo-fuzz target and hand-authored corpus seeds under `fuzz/corpus/obj_import/`.
- Updated CI and scheduled fuzz workflows to run STL and OBJ fuzz targets through a matrix.
- Updated browser WASM and WASI native-fs checks to include `format-obj`.
- Updated `baozi-format-obj::format_info()` and format docs to claim only the implemented experimental support tier.
- Added `docs/formats/obj.md`, updated `docs/formats/support-matrix.md`, `docs/contributing/fuzzing.md`, and `docs/roadmap.md`.
- Fixed two clippy warnings in `crates/baozi-format-obj/src/mesh_builder.rs`.

# Open Threads

- Local Windows MSVC sanitizer smoke compiles but cannot run because the sanitizer runtime entry points do not match the Rust nightly. Linux CI remains the authoritative ASan gate.
- `docs/plans/2026-07-09-005-feat-obj-mtl-importer-vertical-slice-plan.md` is an untracked duplicate plan artifact that was not created by the current implementation pass and should remain unstaged unless the user decides otherwise.

# Next Action

Run the ce-work shipping tail: review current diff, apply review fixes, commit the U6-U8 slice, push `main`, and watch CI.

# Citations

- [Plan](../../../plans/2026-07-09-005-feat-obj-mtl-importer-plan.md)
- Commit `99a674e docs(plan): add obj mtl importer execution plan`
- Commit `699944f feat(obj): import static obj geometry and mtl materials`
