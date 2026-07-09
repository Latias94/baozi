---
type: "Memory Event"
title: "Verification: Pinned Linux fuzz CI and rechecked Windows ASan"
description: "Pinned the Linux fuzz smoke job to nightly-2026-05-27 and rechecked local Windows ASan with LLVM 22.1.6; Windows still fails before target execution with STATUS_ENTRYPOINT_NOT_FOUND."
timestamp: 2026-07-09T05:16:14Z
event_kind: "Verification"
related_plan: "docs/plans/2026-07-09-002-feat-stl-importer-vertical-slice-plan.md"
---
# Event

The CI workflow now declares the MSRV, fuzz nightly, and `cargo-fuzz` version explicitly. The Rust
job uses `RUST_STABLE_VERSION=1.95.0`; the fuzz job uses
`RUST_FUZZ_NIGHTLY=nightly-2026-05-27` and `CARGO_FUZZ_VERSION=0.13.2`, matching the local LLVM
22.1.6 sanitizer runtime that was installed in a machine-local directory outside the repo.

Local verification passed for:

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo nextest run --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p baozi --no-default-features
cargo check -p baozi --features all-formats,native-fs
cargo check -p baozi --target wasm32-unknown-unknown --no-default-features --features format-stl
cargo check -p baozi --target wasm32-wasip1 --no-default-features --features format-stl,native-fs
cargo deny check
cargo +nightly-2026-05-27 fuzz check stl_import
cargo clippy -p baozi-format-stl --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic
```

Local Windows sanitizer execution was retried with:

```powershell
$installRoot = '<machine-local-LLVM-22.1.6>'
$env:PATH = "$installRoot\bin;$installRoot\lib\clang\22\lib\windows;$env:PATH"
cargo +nightly-2026-05-27 fuzz run stl_import -- -runs=256
```

It still failed before executing the fuzz target with `STATUS_ENTRYPOINT_NOT_FOUND`. This remains
Windows MSVC sanitizer runtime evidence rather than parser evidence. The Linux CI fuzz smoke run is
the authoritative sanitizer gate.

# Impact

Future agents should not treat local Windows `cargo fuzz run` failure as an STL parser failure when
the process exits before target execution. Use the CI job for sanitizer evidence, and use local
`cargo +nightly-2026-05-27 fuzz check stl_import` plus malformed tests for Windows development.

# Citations

- [CI workflow](../../../../../.github/workflows/ci.yml)
- [Fuzzing guide](../../../../contributing/fuzzing.md)
- [ADR 0019](../../../../adr/0019-parser-diagnostic-streaming-and-generated-code-contract.md)
- [STL importer plan](../../../../plans/2026-07-09-002-feat-stl-importer-vertical-slice-plan.md)
