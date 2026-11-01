---
status: completed
date: 2026-11-01
---

# glTF primitive modes and list consumption: execution review

This record covers the [selected primitive-mode slice](../plans/2026-11-01-004-feat-gltf-primitive-modes-plan.md)
after `3267064`. Sparse accessor decoding, additional influence sets and format extensions remain
outside this slice. Previous world-query and tangent qualification belongs to its own source state.
Implementation is committed as `d388d3e38657ab0c5af4e4d43af40ec14988b4dd`. The ten recorded
implementation/test file hashes were checked against staged content before commit; none changed
through integrated qualification. The identity manifest and execution logs are retained locally
in `target/review/primitive-modes-evidence.zip`.

## Implemented behavior and decisions

The importer preserves all seven source modes through the existing canonical topology vocabulary.
No selected primitive is filtered by mode. Source primitive order, explicit index order, absent
indices, material/instance references, attribute data, skin/morph data, source-owned extras and
resource closure remain source preserving. Mixed-mode tests share accessors and mesh instances,
reorder source mesh indices, and assert source identity without guessed index/name joins.

Format preflight enforces glTF's minimum/divisibility rules before dependency I/O. Canonical
topology element counts also charge point and line work/limits, fixing their previous zero-face
preflight accounting. Stored topology cells still count explicit indices; nonindexed draws retain
implicit indexing. During bounded index collection, original U8/U16/U32 restart sentinels and
Position-domain overflow are rejected before source width is lost. A declared positive index
accessor that unexpectedly cannot be read is an explicit failure, never a present-empty draw.

The semantic reference is the
[Khronos glTF mesh contract](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#meshes),
verified on 2026-11-01; the registry HTML identifies version 2.0.1 and source commit
`8e798b02d254cea97659a333cfcb20875b62bdd4`. Tests fix exact strip/fan assembly order, source-width
index restrictions and draw count rules. Repeated/degenerate elements are preserved.

An explicit borrowed `list_vertices` view meets the application-buffer requirement directly.
It exposes Points, LineList or TriangleList without materializing another canonical Scene.
Independent review confirmed the existing polygon triangulation materializer is specialized to
geometric polygon clipping and persistent remap evidence. Generalizing that transaction solely
for a borrowed consumption requirement would add unnecessary ownership and evidence machinery.
The existing `triangulate_primitive` operation remains available for polygons.

The shared public types are `ListVertices` and `ListVertex`; replaced triangle-only type names
are removed, with no compatibility aliases. The strict `triangle_vertices` entry returns the same
types while accepting only canonical TriangleList. The new view reports its output topology
separately from the borrowed primitive's source topology. Output index, source Corner, source
Vertex and source Face are distinct. Attribute, influence and morph samplers share one
implementation and preserve borrowed source storage, sparse defaults and scalar bits.

Counts expand with checked U32 arithmetic. Construction, random access and assembly mapping are
O(1); iteration allocates no heap memory. Short core-authored assemblies return zero tuples,
independently of glTF's stricter format minimums. Degenerate triangles retain original strip
parity, fan hubs retain each Face's values, and loops keep their final directed edge, including
two-point and already repeated-endpoint loops. Application code owns final allocation, checked
byte products, packing and cancellation of the extraction loop. This read-only view has no
operation ledger, publication, new generation, retained remap or implicit coordinate conversion.

The independent asset consumer explicitly chooses this view in direct `ApplicationAsset::from_scene`
extraction, reserves for the expanded tuple count, retains each draw's list topology and reads
morph values through the representative output tuple. Its separate `process_file` canonical
triangle edit/normal/space workflow retains its documented narrower scope.

## Review and focused behavior

Three Codex subagents contributed core architecture/implementation, importer tests/review, and
independent consumer integration/review. Static reviews found no remaining blocking issue in
assembly order, checked counts, short-topology behavior, source-domain selection, complete
deformation sampling, representative tuple indexing or source/resource preservation.

Core tests use handwritten Corner sequences for all modes with indexed and nonindexed input.
They assert independent Face/Corner/Vertex fields, source slice identity, sparse Corner values,
reverse/random/interleaved traversal, no allocations and unchanged Scene snapshots. Existing
deformation fixtures verify complete variable influence rows and dense/sparse morph sampling.
Private count tests cover the exact last representable expansion and first U32 overflow.

Importer tests cover all modes and unsigned index widths, omitted default mode, minimum and
invalid counts, invalid numeric modes, degenerate input, exact face/topology/work budgets,
source-owned JSON bytes, GLTF/GLB source ranges, resource reuse and omitted dependency closure.
In-range U8/U16 sentinel rejections have wider-component success controls, distinguishing restart
rejection from ordinary index bounds checking.

The independent self-authored consumer fixture contains all seven indexed/nonindexed modes,
shared skinned instances, morph overrides, authored normals/tangents/UVs, degenerates and an
unreferenced vertex. It reconstructs complete indexed tuples after Scene destruction and checks
two-point loop closure. These generated fixtures are not presented as unchanged upstream assets.

Two new-test corrections were required before qualification:

- The first importer focused run passed 49/50. A one-short total-work test expected a parser-owned
  scope error, but its final unit belongs to central publication and returns an operation-owned
  Ledger terminal. The corrected test checks that ownership boundary, zero committed members and
  released resources; exact total work still succeeds.
- The first consumer focused run passed 1/2. Its new morph Position accessor omitted required
  min/max metadata. Adding the independently calculated `[-0.5, -0.25, -0.5]` and `[1, 1, 0.5]`
  bounds made the valid fixture pass. No production rule was relaxed.

| Executed check | Result | Run identity |
| --- | --- | --- |
| Postprocess library and vertex integration | 103/103 pass | `8b5f6e1d-4e81-4b58-be30-3c83647bbfca` |
| glTF primitive modes and static mesh | 50/50 pass | `51e5efa3-4059-474f-975e-417b02317993` |
| Independent asset consumer primitive modes | 2/2 pass | `cd90f1bc-a7bb-4d52-b003-6345ce5a983b` |
| Workspace and independent asset consumer strict Clippy | Both pass with all targets/features and `-D warnings` | Local logs |
| Windows workspace, all features | 2,584/2,584 pass; 4 existing manual skips | `f1b365a5-ef07-43c5-8346-bc578914addc` |
| Independent asset consumer, all features | 31/31 pass; 1 existing manual skip | `39dc3e16-b85a-4974-8405-476a7517b6ec` |
| Workspace doctests, all features | 58/58 pass | Local log |
| Workspace rustdoc | Pass with `RUSTDOCFLAGS=-D warnings` | Local log |
| Workspace and independent asset consumer formatting | Both pass | `cargo fmt ... -- --check` |
| Independent Scene consumer, all features, Windows execution | Pass for glTF/OBJ/PLY/STL | Local log |
| Independent Scene consumer, all features, `wasm32-unknown-unknown` | Compile pass | No browser execution claim |
| Independent Scene consumer, all features, `wasm32-wasip1` | Compile pass | No WASI execution claim |

## Qualification boundary

Checks use Rust 1.95.0 on `x86_64-pc-windows-msvc`, one Cargo build at a time and the existing
root target directory. Direct Cargo commands use locked/offline dependencies. Local integrated
logs and source identities are stored under `target/review/primitive-modes-*`.
The existing `check_features.py` matrix uses `--locked` with `CARGO_NET_OFFLINE=true` for this run.

Executed integrated commands:

```text
cargo +1.95.0 clippy --locked --offline --workspace --all-targets --all-features --jobs 1 -- -D warnings
cargo +1.95.0 clippy --locked --offline --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --all-targets --all-features --jobs 1 -- -D warnings
cargo +1.95.0 nextest run --locked --offline --workspace --all-features --build-jobs 1 --test-threads 2 --no-fail-fast --status-level fail --final-status-level none
cargo +1.95.0 nextest run --locked --offline --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --all-features --build-jobs 1 --test-threads 1 --no-fail-fast --status-level fail --final-status-level none
cargo +1.95.0 test --locked --offline --workspace --all-features --doc --jobs 1
$env:RUSTDOCFLAGS = '-D warnings'
cargo +1.95.0 doc --locked --offline --workspace --all-features --no-deps --jobs 1
cargo +1.95.0 fmt --all -- --check
cargo +1.95.0 fmt --manifest-path tools/consumers/assets/Cargo.toml -- --check
$env:CARGO_NET_OFFLINE = 'true'
python scripts/check_features.py --toolchain 1.95.0 --case all
python scripts/check_features.py --toolchain 1.95.0 --check --target wasm32-unknown-unknown --case all
python scripts/check_features.py --toolchain 1.95.0 --check --target wasm32-wasip1 --case all
```

These tests establish selected-profile import and borrowed list consumption. They do not establish
canonical strip/fan/loop materialization, arbitrary canonical normal/space edits on those topologies,
complete glTF conformance, release/maturity promotion, new platform runtime results, sustained
fuzzing, or performance/memory-growth thresholds. Compile-only portable checks remain distinct
from browser or WASI execution. No dependency or snapshot schema change is required.
