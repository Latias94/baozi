# Baozi

Baozi is a Rust workspace for a model import library inspired by Assimp's breadth, but designed
around Rust ownership, typed errors, explicit post-processing, and a source-preserving scene IR.

Current status: experimental scaffold. The workspace declares format crates and architecture policy,
but parser implementations are intentionally not claimed as supported yet.

## Goals

- Load many 3D asset formats through one Rust-native facade.
- Keep parser crates replaceable behind stable importer contracts.
- Preserve source data by default, then normalize through explicit post-process steps.
- Support common engine and tooling workflows without exposing C/C++ ABI assumptions.
- Keep the public license friendly for downstream MIT or Apache-2.0 projects.

## Workspace Layout

| Crate | Purpose |
| --- | --- |
| `baozi` | Public facade and feature-gated format registration |
| `baozi-core` | Scene IR, IDs, diagnostics, errors, math, materials |
| `baozi-io` | Asset IO, URI/path policy, resource limits |
| `baozi-import` | Format importer traits, registry, capability metadata |
| `baozi-postprocess` | Post-process pipeline contracts and presets |
| `baozi-format-stl` | STL importer crate shell |
| `baozi-format-obj` | OBJ/MTL importer crate shell |
| `baozi-format-ply` | PLY importer crate shell |
| `baozi-format-gltf` | glTF/GLB importer crate shell |
| `baozi-test-support` | Test snapshots and fixture helpers |

## Format Status

The declared support matrix is tracked in
[`docs/formats/support-matrix.md`](docs/formats/support-matrix.md). A format is not considered stable
until it has documented capability coverage, fixtures, malformed input tests, fuzz coverage, and
promotion evidence.

## Design Docs

Start here:

- [`docs/research/assimp-replication-study.md`](docs/research/assimp-replication-study.md)
- [`docs/adr/0001-baozi-assimp-compatible-architecture.md`](docs/adr/0001-baozi-assimp-compatible-architecture.md)
- [`docs/model/scene-ir.md`](docs/model/scene-ir.md)
- [`docs/model/coordinate-and-render-conventions.md`](docs/model/coordinate-and-render-conventions.md)
- [`docs/security/parser-threat-model.md`](docs/security/parser-threat-model.md)
- [`docs/roadmap.md`](docs/roadmap.md)

## Development

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo test --doc --workspace
cargo deny check
```

Use `cargo nextest` for tests where available. Keep parser behavior limit-aware and panic-averse.
Parser fuzzing commands and Windows sanitizer notes live in
[`docs/contributing/fuzzing.md`](docs/contributing/fuzzing.md).
GitHub Actions policy, workflow linting, and CI tool pinning are documented in
[`docs/contributing/ci.md`](docs/contributing/ci.md).

## License

Baozi is intended to be distributed under `MIT OR Apache-2.0`. See
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) for current third-party and reference-source
policy.
