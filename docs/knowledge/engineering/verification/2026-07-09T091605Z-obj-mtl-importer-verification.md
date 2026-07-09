---
type: Verification Evidence
title: OBJ MTL importer local verification
timestamp: 2026-07-09T09:16:05Z
tags: baozi,obj,mtl,fuzz,ci
related_plan: docs/plans/2026-07-09-005-feat-obj-mtl-importer-plan.md
git_branch: main
---

# Passed Gates

## Local

- `cargo fmt --all -- --check`
- `actionlint`
- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace` with 114 tests passed after review fixes
- `cargo test --doc --workspace --all-features`
- `RUSTDOCFLAGS=-D warnings cargo doc --workspace --all-features --no-deps`
- `cargo check -p baozi --no-default-features`
- `cargo check -p baozi --features all-formats,native-fs`
- `cargo check -p baozi --target wasm32-unknown-unknown --no-default-features --features format-stl,format-obj`
- `cargo check -p baozi --target wasm32-wasip1 --no-default-features --features format-stl,format-obj,native-fs`
- `cargo deny check`
- `cargo check --manifest-path fuzz\Cargo.toml`
- `cargo +nightly-2026-05-27 fuzz check obj_import`
- `cargo +nightly-2026-05-27 fuzz check stl_import`
- Focused OBJ/facade tests: `cargo test -p baozi-format-obj -- --nocapture` and `cargo test -p baozi --test obj_facade -- --nocapture`
- Native filesystem facade test: `cargo test -p baozi --features native-fs --test obj_facade -- --nocapture`

## Remote CI

- GitHub Actions CI run `29009062587` passed on `main` for commit `309ead2`.

# Environment-Limited Gate

`cargo +nightly-2026-05-27 fuzz run obj_import -- -runs=256` compiled the target, then failed on Windows MSVC with `STATUS_DLL_NOT_FOUND` before the local LLVM runtime was added to `PATH`.

After adding the matching LLVM 22.1.6 `bin` and Clang Windows runtime directories to the command-local `PATH`, both `obj_import` and `stl_import` compiled and failed at runtime with `STATUS_ENTRYPOINT_NOT_FOUND`. The same result was reproduced after review fixes with the single-format `obj_import` registry.

This matches the documented Windows ASan caveat in `docs/contributing/fuzzing.md`; Linux CI should be treated as sanitizer authority for the smoke run.

# Citations

- [Fuzzing policy](../../../contributing/fuzzing.md)
- [Plan verification contract](../../../plans/2026-07-09-005-feat-obj-mtl-importer-plan.md)
