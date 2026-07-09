---
type: "Verification Evidence"
title: "glTF skin fuzz hardening verification"
description: "Verification Evidence for glTF skin fuzz hardening verification."
timestamp: 2026-07-09T14:10:42Z
tags: ["gltf", "fuzz", "ci", "verification", "wasm"]
related_plan: "docs/plans/2026-07-09-007-feat-gltf-resource-and-skin-plan.md"
---

# Verification

Commands run locally on Windows from the repository root.

# Result

Passed:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --all-features`
- `cargo nextest run --workspace --all-features` (`184` tests)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --doc --workspace --all-features`
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`
- `cargo deny check`
- `cargo check -p baozi --target wasm32-unknown-unknown --no-default-features --features format-gltf`
- `cargo +nightly-2026-05-27 fuzz check gltf_import`
- `actionlint`
- `cargo package -p baozi-core --allow-dirty --no-verify`

Environment-limited:

- `cargo +nightly-2026-05-27 fuzz run gltf_import -- -runs=256` compiled the fuzz target but failed
  to launch on Windows with `STATUS_DLL_NOT_FOUND`, consistent with the missing ASan runtime DLL
  issue. This did not reproduce an importer panic locally.
- `cargo package -p baozi --allow-dirty --no-verify` failed because `baozi-core` is not yet
  available from crates.io. This is expected before the first publish and matches the release policy
  requirement to publish internal crates before the facade.

# Evidence

The glTF crate tests include malformed accessor root/type checks, data URI limit checks, face/vertex
preallocation limit checks, Skin MVP import, missing inverse bind matrices, invalid skin joint index,
and inverse bind matrix count mismatch.

# Follow-up

After push, inspect the new GitHub Actions run for the Linux `gltf_import` sanitizer fuzz smoke.
If Linux still finds a panic, preserve the artifact and add it to `fuzz/corpus/gltf_import`.

# Citations

- Plan: [2026-07-09 glTF resource and skin plan](../../../plans/2026-07-09-007-feat-gltf-resource-and-skin-plan.md)
- Fuzzing docs: [fuzzing.md](../../../contributing/fuzzing.md)
- glTF support docs: [gltf.md](../../../formats/gltf.md)
- CI failure link: <https://github.com/Latias94/baozi/actions/runs/29019454481/job/86122242072>
