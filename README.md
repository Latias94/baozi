# Baozi

Baozi is a Rust workspace for a model import library inspired by Assimp's breadth, but designed
around Rust ownership, typed errors, explicit post-processing, and a source-preserving scene IR.

Current status: experimental parser foundation. STL, OBJ/MTL, and PLY have owned parser paths with
fixture and fuzz coverage. The glTF/GLB selected-scene PBR, skin, morph, and TRS animation profile
is available through `format-gltf`; its [exact restrictions](docs/formats/gltf.md) apply.
Scene nodes, geometry, materials, images, skins, morphs, and animation have borrowed typed readers.
The optional `postprocess` feature provides explicit single-primitive triangulation, flat/smooth
normal generation, and whole-Scene coordinate/unit conversion including skins, morph deltas,
and TRS animation curves. Camera/light conversion and application vertex unification remain
in development.
Bounded snapshots are available separately for test and debug evidence.

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
| `baozi-postprocess` | Controlled Scene operations, transactional edits, and retained operation evidence |
| `baozi-format-stl` | STL importer |
| `baozi-format-obj` | OBJ/MTL importer |
| `baozi-format-ply` | PLY importer; admitted for source builds but not yet published |
| `baozi-format-gltf` | Selected glTF/GLB profile with a private maintained decoder; source-admitted |
| `baozi-test-support` | Format maturity and generated fixture helpers |
| `baozi-xtask` | Bounded repository conformance and architecture-support checks; ADR semantics remain human-reviewed |

## Format Status

The declared support matrix is tracked in
[`docs/formats/support-matrix.md`](docs/formats/support-matrix.md). A format is not considered stable
until it has documented capability coverage, fixtures, malformed input tests, fuzz coverage, and
promotion evidence.

## Quick Start

Import bytes from memory:

```rust
use baozi::{AttributeData, Importer, Result};

fn main() -> Result<()> {
    let report = Importer::new()?.read_bytes(
        "triangle.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
    )?;
    for mesh in report.scene().meshes() {
        for primitive in mesh.primitives() {
            if let Some(positions) = primitive.positions() {
                if let AttributeData::DenseF32 { lanes: 3, values } = positions.data() {
                    for position in values.chunks_exact(3) {
                        println!("position={position:?}");
                    }
                }
            }
        }
    }
    Ok(())
}
```

Run the facade examples:

```powershell
cargo run --locked -p baozi --example import_bytes
cargo run --locked -p baozi --example import_memory_sidecars
cargo run --locked -p baozi --example diagnostics
cargo run --locked -p baozi --example wasm_memory
```

Baozi requires the Rust standard library; `default-features = false` selects no built-in formats.
The curated `default-formats` set contains STL and OBJ/MTL, while `all-formats` also enables PLY and glTF/GLB.
Individual `format-stl`, `format-obj`, `format-ply`, and `format-gltf` features enable one parser each. Optional
`postprocess` exposes geometry operations as `baozi::postprocess`; `tracing` adds import and enabled
postprocess spans, and `mint` adds the current `Vec3` conversions. The examples and
benchmarks require `format-obj`.

`Importer::new()` registers every enabled built-in format. Use
`Importer::builder_with_builtin_formats()` or `ImporterBuilder::with_builtin_formats()` to extend
that registry before freezing it; `Importer::builder()` starts an empty registry. Format authoring
remains in the `baozi_import` extension API.

With `format-obj` and `postprocess` enabled, triangulate a polygon and generate smooth Corner normals:

```rust
use baozi::{AttributeDomain, Importer};
use baozi::postprocess::{NormalOptions, PostProcessControl, generate_normals, triangulate_primitive};

let import = Importer::new()?.read_bytes(
    "quad.obj",
    b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n",
)?;
let (scene, import_evidence) = import.into_parts();
let target = scene.meshes().next().unwrap().primitives().next().unwrap().id();
let outcome = triangulate_primitive(scene, target, PostProcessControl::default())?;
let (triangles, report) = outcome.into_parts();
assert!(report.matches_scene(&triangles));
let target = report.triangulation_remap().unwrap().output_target();
let (scene, normals_report) = generate_normals(
    triangles, target, NormalOptions::default(), PostProcessControl::default(),
)?.into_parts();
let target = normals_report.normal_generation().unwrap().output_target();
assert!(scene.primitive(target)?.normals(AttributeDomain::Corner).is_some());
```

Import evidence retains its original generation; the postprocess report identifies the output.
Failures return the original immutable Scene through `PostProcessRunFailure::into_parts()`.
Triangulation preserves authored attributes through explicit origins and rejects malformed or
numerically unresolved polygons. Its report owns the actual Corner/Face relation independently of
either Scene. Triangles still retain separate attribute domains; this operation does not produce a
unified application vertex buffer.

Normal generation preserves authored normals by default. `ExistingNormals::Replace` explicitly
replaces them, and `TangentHandling::Remove` authorizes deleting invalidated tangents and their
owner-local metadata. Smooth mode shares area-weighted normals by Vertex identity and Face
smoothing groups; group zero is hard per face, and UV seams do not split a shared Vertex.
Attribute lookup requires both semantic and domain, so `normals(AttributeDomain::Corner)` and
`texcoords(set, AttributeDomain::Vertex)` select the intended stream explicitly.

`convert_scene_space(scene, options, control)` converts an entire Scene to a validated
`TargetSpace`. The target contains handedness, signed up/front axes, meters per unit, and front-face
winding. Unknown source facts need explicit `source_assumptions`; an assumption conflicting with
a known declaration fails. UV origin and tangent-generation declarations are preserved. Positions,
normals, tangents, and authored Matrix/TRS transforms follow one basis conversion; reversed triangle
lanes move all Corner tuples with their indices. Inverse binds and skin corrections use matrix
conjugation, morph positions follow unit-scaled basis conversion, and animation values and cubic
derivatives follow their target semantics without normalization. The report retains source/target
facts and actual
assumptions after either Scene is dropped. Present semantics the current operation cannot convert
return typed capability errors and the original Scene. See the
[coordinate contract](docs/model/coordinate-and-render-conventions.md) for the current boundary.

Run the import benchmark harness without collecting measurements:

```powershell
cargo bench -p baozi --bench import_baseline --no-run --all-features
```

## Design Docs

Start here:

- [Active development plan](docs/plans/2026-10-29-001-refactor-library-delivery-plan.md)
- [2026-10-29 library state review](docs/reviews/2026-10-29-library-state-review.md)
- [`docs/research/assimp-replication-study.md`](docs/research/assimp-replication-study.md)
- [`docs/adr/README.md`](docs/adr/README.md)
- [`docs/adr/0001-baozi-assimp-compatible-architecture.md`](docs/adr/0001-baozi-assimp-compatible-architecture.md)
- [`docs/model/scene-ir.md`](docs/model/scene-ir.md)
- [Reading a validated Scene](docs/model/reading-scenes.md)
- [`docs/model/coordinate-and-render-conventions.md`](docs/model/coordinate-and-render-conventions.md)
- [`docs/security/parser-threat-model.md`](docs/security/parser-threat-model.md)
- [`docs/roadmap.md`](docs/roadmap.md)

## Development

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo nextest run --workspace --all-features
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

Baozi is licensed under either of
[Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option. See
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) for current third-party and reference-source
policy.
