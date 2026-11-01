# Baozi

Baozi is a Rust workspace for a model import library inspired by Assimp's breadth, but designed
around Rust ownership, typed errors, explicit post-processing, and a source-preserving scene IR.

Current status: experimental parser foundation. STL, OBJ/MTL, and PLY have owned parser paths with
fixture and fuzz coverage. The glTF/GLB selected-scene PBR, skin, morph, and TRS animation profile
is available through `format-gltf`; its [exact restrictions](docs/formats/gltf.md) apply.
Scene nodes, geometry, materials, images, skins, morphs, and animation have borrowed typed readers.
The optional `postprocess` feature provides atomic selected-primitive triangle preparation,
flat/smooth normals, explicit tangent generation, and whole-Scene coordinate/unit conversion including skins, morph deltas,
TRS animation curves, camera projections, light definitions, and material lengths.
Its allocation-free `triangle_vertices` reader exposes borrowed application tuples; applications
select their fields and allocate their own vertex/index buffers.
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
| `baozi-format-gltf` | Selected glTF/GLB profile with a private metered document decoder; source-admitted |
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

For host files and relative sidecars, the [host bundle example](crates/baozi/examples/import_host_bundle.rs)
snapshots an explicit approved OBJ/MTL/image list into `MemoryAssetIo`, with per-file and aggregate
limits. Run `cargo run --locked -p baozi --example import_host_bundle -- <bundle-directory>` with
the three relative paths listed in the example. The application chooses filesystem authority and
link policy; the importer receives immutable memory assets.

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

With `format-obj` and `postprocess` enabled, lower a polygon and generate Corner normals in one
transaction:

```rust
use baozi::{AttributeDomain, Importer};
use baozi::postprocess::{
    NormalOptions, PostProcessControl, TrianglePreparationOptions, prepare_triangles,
};

let import = Importer::new()?.read_bytes(
    "quad.obj",
    b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n",
)?;
let (scene, import_evidence) = import.into_parts();
let target = scene.meshes().next().unwrap().primitives().next().unwrap().id();
let (scene, report) = prepare_triangles(
    scene,
    &[target],
    TrianglePreparationOptions {
        normals: Some(NormalOptions::default()),
        ..Default::default()
    },
    PostProcessControl::default(),
)?.into_parts();
assert!(report.matches_scene(&scene));
let preparation = report.geometry_preparation().unwrap();
assert_eq!(preparation.geometry_reconstruction_count(), 1);
let target = preparation.target(0).unwrap().output_target();
assert!(scene.primitive(target)?.normals(AttributeDomain::Corner).is_some());
```

Import evidence retains its original generation; the postprocess report identifies the output.
Failures return the original immutable Scene through `PostProcessRunFailure::into_parts()`.
The selected request accepts any number of current-generation primitive IDs. It plans and
reconstructs geometry once, then publishes one generation. Duplicate/stale IDs, a later target
failure, cancellation, or exhausted `PostProcessBudgets` return the original Scene. Reports retain
final-generation IDs for every selected target, including unchanged targets in a mixed request.
Lowering preserves exact strip/fan assembly and polygon Corner/Face origins. Triangulation rejects malformed or
numerically unresolved polygons. Its report owns the actual Corner/Face relation independently of
either Scene. Triangles still retain separate attribute domains; this operation does not produce a
vertex buffer. The `triangle_vertices` reader described below supplies their draw-order tuples.

Normal generation preserves authored normals by default. `ExistingNormals::Replace` explicitly
replaces them, and `TangentPolicy::Remove` authorizes deleting invalidated tangents and their
owner-local metadata. Smooth mode shares area-weighted normals by Vertex identity and Face
smoothing groups; group zero is hard per face, and UV seams do not split a shared Vertex.
Attribute lookup requires both semantic and domain, so `normals(AttributeDomain::Corner)` and
`texcoords(set, AttributeDomain::Vertex)` select the intended stream explicitly.

`NormalOrientation::StoredOrder` uses the retained triangle tuple. `SceneFrontFace` uses the
Scene's winding declaration and requires an explicit assumption when it is unknown. The request
keeps the Scene's spatial declaration and index order. `generate_normals(scene, target, options,
tangent_policy, control)` and the other single-target functions use this same transaction.

`generate_tangents(scene, target, TangentOptions::new(normal_domain, uv_set, uv_domain), control)`
selects the Normal and UV fields explicitly and emits dense Corner F32x4 or F64x4 according to
the widest dependency or replaced field. The Triangle UV Derivative
v1 algorithm preserves mirrored seams, uses `SignInW`, and performs no cross-face welding or
averaging. It does not claim MikkTSpace compatibility. Existing Tangents are preserved by default;
`ExistingTangents::Replace` replaces a single authored stream. Each Tangent owns its construction
and encoding declarations. Degenerate frames and authored morph deltas on the target reject
without replacing the Scene. Position, Normal, Tangent and Texcoord attributes retain their
declared F32/F64 precision; application packing is an explicit conversion.

`convert_scene_space(scene, options, control)` converts an entire Scene to a validated
`TargetSpace`. The target contains handedness, signed up/front axes, meters per unit, and front-face
winding. Unknown source facts need explicit `source_assumptions`; an assumption conflicting with
a known declaration fails. UV origin and tangent-generation declarations are preserved. Positions,
normals, tangents, and authored Matrix/TRS transforms follow one basis conversion; reversed triangle
lanes move all Corner tuples with their indices. Inverse binds and skin corrections use matrix
conjugation, morph positions follow unit-scaled basis conversion, and animation values and cubic
derivatives follow their target semantics without normalization. The report retains source/target
facts and actual assumptions after either Scene is dropped. Camera/light intrinsic axes transform
with the basis while node hierarchy stays fixed; projection distances and light ranges scale with
units, and photometric intensity stays unchanged. Supplemental material thickness and signed
displacement strength scale by the positive unit ratio; unchanged image and text payloads remain
shared. Present semantics the current operation cannot convert return typed capability errors and
the original Scene. See the
[coordinate contract](docs/model/coordinate-and-render-conventions.md) for the current boundary.

After the required operations, `baozi::postprocess::triangle_vertices(primitive)` borrows one
application tuple per draw Corner of a `TriangleList`. Vertex, Corner, and Face fields keep their
semantic/domain keys and scalar types; influence rows and requested morph deltas follow the same
source Vertex. Equal tuples may repeat. Applications choose their concrete buffers and packing,
and may collect the implicit `0..corner_count` output index stream. See
[application triangle buffers](docs/model/reading-scenes.md#application-triangle-buffers) for the
borrowed lifetimes and extraction contract.

Run the import benchmark harness without collecting measurements:

```powershell
cargo bench -p baozi --bench import_baseline --no-run --all-features
```

## Design Docs

Start here:

- [Completed library delivery plan](docs/plans/2026-10-29-001-refactor-library-delivery-plan.md)
- [Offline real-asset consumer and profiling](tools/consumers/assets/README.md)
- [Real-asset integration findings and measurements](docs/reviews/2026-10-31-real-asset-consumer.md)
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
