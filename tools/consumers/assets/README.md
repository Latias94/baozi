# Offline Real-Asset Consumer

This unpublished standalone application exercises the public Baozi API with unchanged upstream
assets and actual capability-bound filesystem access. It imports a model, triangulates polygons,
converts the complete Scene to a known CCW processing space, preserves authored normals or generates
missing normals, copies application buffers, and destroys the Scene before returning those
buffers and detached import evidence.

The target is left-handed, positive Y up, negative Z front, centimeters, with counter-clockwise front
faces. UVs remain in their declared source convention. Formats with unknown spatial facts, including
OBJ, need an explicit caller assumption. The native dependency remains an unregistered candidate;
this tool does not add a filesystem backend to the portable library or its release target list.
Library callers choosing a clockwise target receive a final winding conversion after normal
generation. The tests cover both source windings against both target windings.

Run from the repository root, reusing its target directory:

```text
cargo +1.95.0 run --locked --release --manifest-path tools/consumers/assets/Cargo.toml --target-dir target -- tools/consumers/assets/fixtures upstream/Models/BoxTextured/glTF/BoxTextured.gltf
cargo +1.95.0 run --locked --release --manifest-path tools/consumers/assets/Cargo.toml --target-dir target -- /path/to/assets model.obj --assume-standard-space
cargo +1.95.0 nextest run --locked --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --all-features --build-jobs 1 --test-threads 1
```

`ROOT` is an ambient directory used to acquire the native capability; the asset path is relative to
it. Dependencies resolve through the same capability. A successful invocation emits one JSON
summary. Failures emit a cause chain on stderr and return a nonzero exit code. There is no automatic
retry, hidden coordinate assumption, or automatic budget increase. `--max-work-units N` explicitly
sets the import work allowance; its default is 200,000,000, including filesystem accounting.
Postprocess operations use their documented default per-operation limits.

## Application layout

This is a concrete extraction example, not a new canonical representation or a general exporter:

- Each primitive becomes a separate draw. Complete triangle tuples are expanded to draw order with
  indices `0..corner_count`; vertices are not welded. Positions, normals, authored tangents, and UV0
  use F64, with exact F32 widening. Simultaneous normal/tangent/UV0 domains require an explicit
  application policy and are rejected here rather than resolved by precedence.
- Skin influences use variable-length CSR rows without truncation. All authored Position, Normal,
  and Tangent morph delta streams expand to the same draw order. Missing sparse entries use the
  public logical value read, and no new morph stream is synthesized.
- Owned node hierarchy and authored transforms, mesh instances and their effective morph weights,
  joint references and inverse bind matrices, animation times/values/cubic derivatives, material
  factors and texture image/UV/transform references, and encoded images survive Scene destruction.
  Opaque Scene IDs are mapped to application indices only after the final edit.
- This application selects UV0 and a material/animation subset. It does not retain every material
  channel, binding strength, sampler, skin-space declaration, animation time-domain/extrapolation
  declaration, camera, light, custom attribute, or provenance attachment. It is not a round-trip
  serializer or renderer. Image decoding, animation evaluation, skinning, tangent generation, GPU
  packing, and world-space bounds are outside this example. `mesh_local_bounds` aggregates base
  mesh-local positions; it does not apply instances, node transforms, skinning, or morph weights.

Topology/normal operations preserve mesh and primitive order. Their handles belong to a Scene
generation, so the application reacquires each primitive after every publication. A two-polygon
integration fixture exercises consecutive edits across multiple primitives. The unchanged real
models and their independent expected facts are documented in [fixtures/README.md](fixtures/README.md).

## Advisory profiling

The thin Python driver builds one release executable and launches a fresh process for each
successful fixture. Python 3.10 or newer is required. These measurements are observations without
performance thresholds:

```text
python scripts/run_asset_profile.py --toolchain 1.95.0
python scripts/run_asset_profile.py --toolchain 1.95.0 --allocation-metrics
python scripts/run_asset_profile.py --toolchain 1.95.0 --root /path/to/assets --asset model.obj --assume-standard-space
```

`--asset` can be repeated. `--artifact-root NEW_DIR` selects a new output directory; the default is
`target/asset-profiles/<UTC timestamp>`. An existing directory is rejected to preserve prior evidence.
Reports include Git revision/dirty state, `rustc -vV`, actual argv, return codes, elapsed process time,
per-stage consumer summaries, and separate stdout/stderr logs. On a failure, the partial report is
retained and the nonzero result is propagated. CI exercises the consumer on the three exact native
candidate hosts and retains both Linux release profile variants.

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
