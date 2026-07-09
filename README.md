# Baozi

Baozi is a Rust workspace for a model import library inspired by Assimp's breadth, but designed
around Rust ownership, typed errors, explicit post-processing, and a source-preserving scene IR.

Current status: experimental parser foundation. STL, OBJ/MTL, and PLY have owned parser paths with
fixture and fuzz coverage. glTF/GLB is available behind an experimental private backend while Baozi
keeps the public API independent from `gltf-rs`.

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

## Quick Start

Import bytes from memory:

```rust
use baozi::{Importer, Result};

fn main() -> Result<()> {
    let report = Importer::new().read_bytes(
        "triangle.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
    )?;
    println!("meshes={}", report.scene().meshes.len());
    Ok(())
}
```

Run the facade examples:

```powershell
cargo run -p baozi --example import_bytes
cargo run -p baozi --example import_memory_sidecars
cargo run -p baozi --example diagnostics
cargo run -p baozi --example postprocess
cargo run -p baozi --example wasm_memory
```

Run the import benchmark harness without collecting measurements:

```powershell
cargo bench -p baozi --bench import_baseline --no-run --all-features
```

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
cargo test --doc --workspace --all-features
cargo check -p baozi --examples --all-features
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
