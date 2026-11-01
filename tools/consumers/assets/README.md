# Offline Real-Asset Consumer

This unpublished standalone application exercises the public Baozi API with unchanged upstream
assets and actual capability-bound filesystem access. It imports a model, triangulates polygons,
converts the complete Scene to a known CCW processing space, preserves authored normals or generates
missing normals, copies application buffers, and destroys the Scene before returning those
buffers and detached import evidence.
Before extraction it walks the retained provenance of each primitive through the public borrowed
readers, sharing one visited set across draws. This reports reached source objects without
inventing a persistent application provenance graph.

`process_file` and `process` deliberately use a triangle processing workflow: polygon primitives
are triangulated, TriangleList primitives are accepted, and points, lines, strips, fans, and loops
are rejected before space conversion and normal generation. `ApplicationAsset::from_scene` is a
separate direct extraction path for an already validated Scene. It explicitly uses `list_vertices`
to accept all seven glTF modes without editing the Scene, generating attributes, converting units
or coordinates, or baking instance transforms. Callers choose any processing needed before direct
extraction; polygons still require explicit triangulation. Broader extraction support does not
broaden the CLI's editing workflow.

The target is left-handed, positive Y up, negative Z front, centimeters, with counter-clockwise front
faces. UVs remain in their declared source convention. Formats with unknown spatial facts, including
OBJ, need an explicit caller assumption. The native dependency remains an unregistered candidate;
this tool does not add a filesystem backend to the portable library or its release target list.
Library callers choosing a clockwise target receive a final winding conversion after normal
generation. The tests cover both source windings against both target windings.
glTF explicitly declares a +Z asset front, distinct from camera-local -Z viewing axes; conversion
to this application's default target maps `(x, y, z)` to `(100x, 100y, -100z)`.

Run from the repository root, reusing its target directory:

```text
cargo +1.95.0 run --locked --release --manifest-path tools/consumers/assets/Cargo.toml --target-dir target -- tools/consumers/assets/fixtures upstream/Models/BoxTextured/glTF/BoxTextured.gltf
cargo +1.95.0 run --locked --release --manifest-path tools/consumers/assets/Cargo.toml --target-dir target -- /path/to/assets model.obj --assume-standard-space
cargo +1.95.0 nextest run --locked --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --all-features --build-jobs 1 --test-threads 1
cargo +1.95.0 nextest run --locked --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --test primitive_modes --build-jobs 1 --test-threads 1
```

`ROOT` is an ambient directory used to acquire the native capability; the asset path is relative to
it. Dependencies resolve through the same capability. A successful invocation emits one JSON
summary. Failures emit a cause chain on stderr and return a nonzero exit code. There is no automatic
retry, hidden coordinate assumption, or automatic budget increase. `--max-work-units N` explicitly
sets the import work allowance; its default is 200,000,000, including filesystem accounting.
Postprocess operations use their documented default per-operation limits.

## Application layout

This is a concrete extraction example, not a new canonical representation or a general exporter:

- Each primitive becomes a separate indexed draw with a public `DrawBuffer::topology` of Points,
  LineList, or TriangleList. Direct extraction retains the Corner order of source lists and points;
  LineStrip/LineLoop expand to LineList and TriangleStrip/TriangleFan expand to TriangleList.
  For source face `f`, strip triangles use `[f, f + 1, f + 2]` on even faces and
  `[f, f + 2, f + 1]` on odd faces; fan triangles use `[f + 1, f + 2, 0]`.
  Loop edges use `[f, (f + 1) % n]`, retaining both directed edges for a two-point loop.
  Degenerate elements remain in the output and still advance strip parity. Source primitive
  topology, indices, and shared mesh/instance relationships remain unchanged.
- Reuse requires the same `PrimitiveVertex` identity and identical `to_bits()` values in every
  selected Position, Normal, Tangent (including
  W), UV0 and COLOR0 lane. These application fields use F64, with exact F32 widening. RGB colors
  acquire an explicit alpha of one; RGBA and absent colors remain distinct. Indices follow first
  expanded tuple encounter order, independently of hash map iteration, and retain list element
  order and triangle winding. Distinct source vertices, signed-zero seams, and separate draw/material
  boundaries stay distinct; there is no epsilon comparison or position welding. Simultaneous normal/tangent/
  UV0/COLOR0 domains require an explicit application policy and are rejected here.
- Skin influences use variable-length CSR rows without truncation. Each representative copies its
  complete joint/weight row once, preserving empty rows, all lanes, their sequence, and F32 bits.
  The core's separate source-order ordinals remain outside this selected application layout.
  Every authored Position, Normal, and Tangent morph delta stream uses the same indexed vertex
  order. Missing sparse entries use the public logical value read, and no new stream is synthesized.
  Representatives record expanded output tuple ordinals, so morph reads address that same tuple
  even when strips, fans, and loops repeat or reorder source Corners.
  Influences and morph deltas are Vertex-domain values, so source identity guarantees their
  equivalence within a key. Any future selected Corner/Face deformation fields must also extend
  that key or keep their tuples separate.
- Owned node hierarchy and authored transforms, mesh instances and their effective morph weights,
  joint references and inverse bind matrices, animation times/values/cubic derivatives, material
  shading model, alpha mode/cutoff, double-sided state, factors and texture image/UV/transform
  references, and encoded images survive Scene destruction.
  Opaque Scene IDs are mapped to application indices only after the final edit.
- Source-object glTF JSON metadata is copied explicitly with sanitized resource identity, format,
  versioned namespaced locator, site key, media type, and exact payload bytes. This subset omits
  entity metadata, non-JSON values, and other formats' metadata. It does not parse or rewrite extras.
  The unchanged `MorphPrimitivesTest` checks all 147 original `/asset/extras` bytes after edits and
  Scene destruction, together with both primitives, shared default morph weights, and its JPEG.
- This application selects UV0/COLOR0 and a material/animation subset. It does not retain every material
  channel, binding strength, sampler, skin-space declaration, animation time-domain/extrapolation
  declaration, camera, light, custom attribute, or provenance attachment. It is not a round-trip
  serializer or renderer. Image decoding, animation evaluation, skinning, tangent generation, GPU
  packing are outside this example. `mesh_local_bounds` aggregates base
  mesh-local positions; it does not apply instances, node transforms, skinning, or morph weights.

For glTF `KHR_materials_unlit`, the owned material records `ShadingModel::Unlit`; its retained PBR
base color, BaseColor binding and COLOR0 supply color, while alpha coverage and sidedness remain
explicit. Lighting factors and bindings remain authored fallback data. The example copies these
facts without rendering or silently applying PBR lighting to Unlit. The self-authored
`unlit_materials` integration tests cover optional/required declarations, texture transforms,
fallback resources, space edits and detached owned values.

`BasePoseSpatialSnapshot::from_scene` is a separate, opt-in query before Scene destruction. It
returns an owned `BasePoseWorldTransforms` table and scene-wide `Option<Aabb>` in the Scene's current
coordinate convention and units. It composes authored base poses as `parent_world * local`, without
sampling animation. Bounds transform every Position vertex per instance, including vertices omitted
from the draw indices; uninstanced meshes do not contribute, and no vertices produce `None`.
Bound skins and effective nonzero morph weights return a `SpatialQueryError`. The existing
`ApplicationAsset::from_scene` extraction path remains available for those assets. The caller supplies
`SpatialQueryLimits` and optional cancellation; the same limits apply independently to transform
construction and bounds traversal. Both outputs survive Scene destruction. `world_bounds` tests use
a self-authored shared-instance glTF with ancestor transforms, nonuniform/negative scale, shear and
an unreferenced vertex, plus the unchanged rigged, morph and animated upstream models.

Topology/normal operations preserve mesh and primitive order. Their handles belong to a Scene
generation, so the application reacquires each primitive after every publication. A two-polygon
integration fixture exercises consecutive edits across multiple primitives. The unchanged real
models and their independent expected facts are documented in [fixtures/README.md](fixtures/README.md).

The borrowed `list_vertices` reader is a zero-allocation view of `ListVertex` tuples. Its `index()`
is the expanded output tuple ordinal; `source_corner()`, `source_vertex()`, and `source_face()`
retain the original endpoints for field and deformation reads. `triangle_vertices` is the strict
TriangleList convenience returning the same `ListVertices` view; it does not expand other modes.
Indexing is private application policy; the consumer reserves its index buffer from the expanded
`tuples.len()`, and its buffer/map allocations use fallible reservation. The library checks the
expanded tuple count against u32 and returns `VertexLayoutError::SizeOverflow` on overflow.
Allocation or count failures propagate through the application `Result`, including that typed
error and structured `TryReserveError` and `TryFromIntError` causes. Application allocations are
outside the library's operation ledger.

The independent `primitive_modes` tests directly extract self-authored glTF data covering all seven
modes with indexed and nonindexed streams, exact source Corner and Face order, a degenerate strip
triangle followed by an odd-parity triangle, fan order, and both directed edges of a two-point loop.
They verify unchanged source topology and indices, first-encounter deduplication, F64 attribute bits,
complete influence rows and morph deltas, shared geometry with instance-specific morph weights and
skin palette references, and owned buffers after Scene destruction. They exercise direct extraction
without calling the separate triangle processing workflow.

Indexing tests reconstruct every original draw Corner through the indices and compare all selected
F64 fields and exported F32 deformation values by `to_bits()`. They cover UV/Normal seams, same-source
signed-zero UV and Tangent/W-only seams, five-influence and empty rows, coincident source vertices,
repeat ordering, and owned data after Scene destruction. The quad produces 4 vertices / 6 indices;
both `BoxTextured` and `BoxInterleaved` produce 24 / 36. The existing workflow assertions retain
independent source counts: `RiggedSimple` is 160 / 564, `AnimatedMorphCube` is 24 / 36, and the two
`MorphPrimitivesTest` draws are 21 / 72 and 9 / 24.

## Advisory profiling

The thin Python driver builds one release executable and launches a fresh process for each
successful fixture. Python 3.10 or newer is required. These measurements are observations without
performance thresholds:

```text
python scripts/run_asset_profile.py --toolchain 1.95.0
python scripts/run_asset_profile.py --toolchain 1.95.0 --allocation-metrics
python scripts/run_asset_profile.py --toolchain 1.95.0 --root /path/to/assets --asset model.obj --assume-standard-space
python scripts/run_asset_profile.py --toolchain 1.95.0 --root target/asset-workloads-v1 --asset static-quads-128.obj --asset static-quads-256.obj --assume-standard-space --max-work-units 200000000 --repetitions 3
```

`--asset` can be repeated. `--artifact-root NEW_DIR` selects a new output directory; the default is
`target/asset-profiles/<UTC timestamp>`. An existing directory is rejected to preserve prior evidence.
Reports include Git revision/dirty state, `rustc -vV`, actual argv, return codes, elapsed process time,
per-stage consumer summaries, and separate stdout/stderr logs. On a failure, the partial report is
retained and the nonzero result is propagated. CI exercises the consumer on the three exact native
candidate hosts and retains both Linux release profile variants.
`--repetitions N` starts a new consumer process for every selected asset in each repetition;
it does not reuse an imported Scene or average away individual observations. Normal timing and
the separately built `--allocation-metrics` variant belong in different artifact directories.

Consumer summaries separate import, postprocess, provenance traversal and buffer extraction time.
`traced_primitive_owners`, `traced_provenance_nodes` and `traced_source_objects` count the owners
and distinct retained ancestors actually reached. Library ledgers cover their existing operation
scopes; the application's visited sets and indexed buffers use application allocation accounting.
The V1 families and semantic expectations are described in
[fixtures/README.md](fixtures/README.md#v1-workload-definitions-and-regressions); actual executed
measurements belong to the revision-qualified
[expansion review](../../../docs/reviews/2026-11-01-asset-workflow-expansion.md).

The memory measurements answer different questions:

| Field | Scope |
| --- | --- |
| `import_peak_live_bytes` | Peak library-accounted import reservations, including declared allowances; not an exact allocator or process measurement. |
| `max_postprocess_peak_live_bytes` | Maximum of individual postprocess operation peaks, not their sum or an aggregate physical peak. |
| `allocations.requested_peak_bytes` | Optional calling-thread allocator-request peak during root open, import, edits, and extraction; excludes other threads and allocations outside the Rust allocator. |
| `allocations.requested_total_bytes` / `allocation_requests` | Cumulative requested bytes and allocation requests in that same instrumented scope. |
| `allocations.requested_retained_bytes_after_scene_drop` | Requested bytes still retained for application buffers and original detached import evidence after Scene destruction; not a leak assertion. |
| `peak_rss_bytes` in the Python report | Individual child peak RSS from Linux GNU `/usr/bin/time -f %M`, converted from KiB; includes the executable/runtime and process pages. Other hosts report null with an explicit reason. |

The optional `allocation-metrics` feature adds allocator instrumentation and changes timings.
Uninstrumented runs emit `allocations: null`. Stage clocks exclude some setup and teardown; Python
elapsed time includes process startup. These small fixtures establish a reproducible functional and
measurement baseline, not a throughput benchmark or a production memory bound. Keep the two variants
separate, and record the host, source revision, toolchain, and source assets when comparing runs.
