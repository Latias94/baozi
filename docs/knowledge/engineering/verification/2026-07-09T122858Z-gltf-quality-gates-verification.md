---
type: Verification Evidence
title: glTF quality gates local verification
timestamp: 2026-07-09T12:28:58Z
tags:
  - baozi
  - gltf
  - verification
status: passed-with-local-tooling-note
---

# Passed

- `cargo test -p baozi-format-gltf`
- `cargo test -p baozi --features format-gltf --test gltf_facade`
- `cargo check --manifest-path fuzz/Cargo.toml`
- `cargo +nightly-2026-05-27 fuzz check gltf_import`
- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo nextest run --workspace --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --doc --workspace --all-features`
- `RUSTDOCFLAGS=-D warnings cargo doc --workspace --all-features --no-deps`
- `cargo deny check`
- `cargo check -p baozi --no-default-features`
- `cargo check -p baozi --features all-formats,native-fs`
- `cargo check -p baozi --target wasm32-unknown-unknown --no-default-features --features format-stl,format-obj`
- `cargo check -p baozi --target wasm32-wasip1 --no-default-features --features format-stl,format-obj,native-fs`
- `cargo package -p baozi-core --allow-dirty --no-verify`
- PyYAML parse for `.github/workflows/ci.yml` and `.github/workflows/fuzz.yml`

# Local Tooling Note

`go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.12` was not run locally because Go is not
installed on this Windows machine. The GitHub Actions `Workflow lint` job remains the actionlint
authority.
