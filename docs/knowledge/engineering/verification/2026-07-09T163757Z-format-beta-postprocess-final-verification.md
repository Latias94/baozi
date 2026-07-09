---
type: "Verification Evidence"
title: "Format beta postprocess final verification"
description: "Verification Evidence for Format beta postprocess final verification."
timestamp: 2026-07-09T16:37:57Z
tags: ["baozi", "verification", "format-beta", "postprocess"]
---

# Verification

Final local verification for `docs/plans/2026-07-09-008-feat-format-beta-postprocess-plan.md`
after U6, U7, and the follow-up PLY resource-limit review fixes.

# Result

Passed locally except for Windows MSVC sanitizer execution, which is an environment/runtime
limitation already documented in `docs/contributing/fuzzing.md`. Fuzz targets compile locally, and
Linux CI remains the sanitizer-run authority.

# Evidence

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo nextest run --workspace --all-features`: 221 tests passed.
- `cargo test --doc --workspace --all-features`
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`
- `cargo deny check`
- `cargo check -p baozi --examples --all-features`
- `cargo bench -p baozi --bench import_baseline --no-run --all-features`
- `cargo check -p baozi --target wasm32-unknown-unknown --no-default-features --features format-stl,format-obj,format-gltf,format-ply`
- `cargo test -p baozi-format-gltf --tests --all-features`
- `cargo test -p baozi-format-ply --tests --all-features`
- `cargo test -p baozi-postprocess --all-features`
- `cargo test -p baozi-format-stl -p baozi-format-obj -p baozi-format-ply -p baozi-format-gltf --lib --all-features`
- `cargo +nightly-2026-05-27 fuzz check stl_import`
- `cargo +nightly-2026-05-27 fuzz check obj_import`
- `cargo +nightly-2026-05-27 fuzz check obj_postprocess`
- `cargo +nightly-2026-05-27 fuzz check ply_import`
- `cargo +nightly-2026-05-27 fuzz check gltf_import`
- `go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.12`

Windows fuzz smoke attempt:

- `cargo +nightly-2026-05-27 fuzz run stl_import -- -runs=256` without local LLVM runtime failed
  before target execution with `STATUS_DLL_NOT_FOUND`.
- Retried with `F:\MySoftware\LLVM-22.1.6\bin` and
  `F:\MySoftware\LLVM-22.1.6\lib\clang\22\lib\windows` on `PATH`; loader found the runtime but
  failed before target execution with `STATUS_ENTRYPOINT_NOT_FOUND`.
- This matches the documented Windows MSVC sanitizer runtime incompatibility class. It is not parser
  evidence; mandatory sanitizer smoke should be read from Linux CI.

# Follow-up

- Inspect GitHub Actions after pushing `main`.
- Keep `gltf_import` as check-only until the `gltf-rs` abort class is removed or isolated.

# Citations

- [Plan](../../../plans/2026-07-09-008-feat-format-beta-postprocess-plan.md)
- [Fuzzing policy](../../../contributing/fuzzing.md)
- [glTF backend notes](../../../research/gltf-rs-backend-notes.md)
