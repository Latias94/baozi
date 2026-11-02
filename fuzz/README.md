# Bounded edit lifecycle fuzzing

`edit_lifecycle` imports a real OBJ or PLY payload through a cached public `ImportPlan`. It edits one
selected primitive in a fixed order: applicable triangulation, applicable normal generation, then
whole-Scene coordinate conversion. A separate header flag instead prepares all triangle-like
primitives in one transaction before any conversion. There are at most three operation calls. All successful imports
and edit results traverse public node, mesh, instance, attribute-element and triangle-tuple readers,
and capture a complete bounded semantic digest through the existing fuzz snapshot support.

The small `baozi-fuzz` library is shared by this target and its deterministic seed tests. It does
not author Scenes, generate geometry, schedule arbitrary passes or add production control APIs.

## Input bytes

The input is a four-byte header followed immediately by at most 8,192 bytes of asset content.
Short headers and inputs longer than 8,196 bytes are ignored before importing. There is no sidecar
or secondary buffer; external dependencies are disabled.

| Byte | Meaning |
| --- | --- |
| `0` | Bit 0 chooses OBJ (`0`) or PLY (`1`). Bits 1–7 choose a primitive ordinal modulo the imported primitive count, in mesh/primitive iteration order. |
| `1` | Bit 0 requests triangulation; bit 1 normal generation; bit 2 coordinate conversion. Bit 3 replaces the first two operations with one selected triangle preparation over all triangle-like primitives. Other bits are ignored. Order is fixed regardless of flags. |
| `2` | Bit 0 chooses flat normals instead of smooth; bit 1 explicitly replaces existing normals; bit 2 requests reflected centimeters instead of the standard target; bit 3 omits the explicit standard source assumptions, allowing the real missing-source rejection path. Other bits are ignored. |
| `3` | Value modulo 4: `0` uses active control, `1` pre-cancels triangulation, `2` pre-cancels normal generation, `3` pre-cancels conversion. Either `1` or `2` pre-cancels selected preparation. |
| `4..` | Original source-file bytes; malformed payloads use the actual parser rejection path. |

Triangulation accepts Polygon and TriangleList inputs; the latter can return `Unchanged`. Normal
generation runs only when the selected primitive is a TriangleList, including after triangulation.
The conversion source assumptions explicitly declare the otherwise unknown OBJ/PLY conventions;
the harness never infers those facts from file values. All primitive IDs are reacquired after edits.

Limits admit at most four meshes, eight primitives, 128 vertices, 64 faces, 512 topology cells,
64 attribute streams and 8,192 attribute cells. Import work is capped at 1,000,000 units, live
import capacity at 16 MiB and canonical payload at 2 MiB. Public reads allow the bounded
triangulation expansion, with a 131,072-element traversal backstop. Snapshot capture retains the
existing 64 KiB preview, 256 preview items and 4,000,000 work-unit bounds; its digest covers all
admitted semantics, including values outside the preview. Postprocessing uses the existing
operation resource policy; input bounds and at most three calls bound this harness's edit work.
Selected preparation uses explicit 4,000,000 work-unit and 16 MiB storage caps.

Every rejected edit must return its input generation and complete digest unchanged and permit
the same borrowed reads, including lookup with its original ID. `Unchanged` success has the same
checks. `Changed` success requires a new generation, rejects the old primitive ID, checks report
generation binding and reads the published output. A pre-cancelled call that reaches its control
checkpoint must report `Cancelled { origin: Signal }` through `PostProcessCancellationToken` and
`PostProcessControl`. Resource-policy initialization or the operation's `Start` can fail before
that checkpoint; the arbitrary-input harness allows those startup failures while still requiring
unchanged generation, digest and reads. The fixed cancellation seeds require actual signal
cancellation under healthy startup admission. These are deterministic **pre-execution**
cancellations; the harness makes no claim about cancelling in the middle of a geometry kernel.

## Independently authored corpus

All files in `corpus/edit_lifecycle/` are small handwritten source assets with the binary header
above. None are upstream samples or generated Scenes. Tests assert actual `Changed` reports
separately for triangulation, normal generation and conversion, plus public numeric expectations.

| Seeds | Intended path |
| --- | --- |
| `triangulate_quad.bin` | Textured non-square quad becomes two triangles, `Changed`. |
| `normals_smooth.bin`, `normals_flat.bin` | Missing normals generate a real field; the smooth case retains a UV seam and shared smoothing group. |
| `normals_replace.bin` | Explicit replacement changes authored +X normals to the expected +Z field. |
| `space_centimeters.bin` | Reflection and unit conversion change positions to independently expected values. |
| `workflow_two_meshes.bin`, `workflow_quad_ply.bin` | All three operations return `Changed`, with fresh IDs between publications; the OBJ seed selects the second mesh. |
| `prepare_two_meshes.bin` | Both polygon meshes lower and generate normals with one reconstruction/publication; a companion deterministic test cancels it with the exact original Scene. |
| `rejected_polygon.bin`, `rejected_space.bin` | Deterministic polygon-kernel and missing-source-convention rejections preserve generation, digest and reads. |
| `cancel_triangulate.bin`, `cancel_normals.bin`, `cancel_space.bin` | Each operation is cancelled through its pre-cancelled public token and returns the exact original Scene. |
| `workflow_cancel_normals.bin` | Triangulation publishes, normal generation is cancelled without changing that generation/digest, then conversion publishes successfully. |

## Feature-reaching glTF seeds

The four original fixtures below extend `corpus/gltf_import/` and the existing `gltf_import`
target. JSON inputs use embedded data URIs or the target's first-NUL split followed by bytes for
`buffer.bin`. They require no upstream model files, images or network access.

| Seed | Required successful observation |
| --- | --- |
| `scene_one_selection.gltf` | Document default selects scene 1, retains `Chosen`/`Leaf` and their translations, and records the omitted node from scene 0. |
| `material_lobes_light.gltf` | A selected triangle retains emissive factor and strength, clearcoat and sheen; its node retains a punctual point light with known intensity, tint and range. |
| `quantized_attributes.gltfbin` | Three vertices decode signed-byte positions/normals and signed-short UVs, each with explicit four-byte stride; padding bytes are not attribute values. |
| `implicit_zero_attributes.gltf` | Buffer-free accessors materialize positive-zero positions/UVs/RGB and indices, plus implicit alpha 1, without dependency acquisition. |

Deterministic `gltf_seed_tests` use the same `support::import_plan(GltfImporter, FORMAT_ID)`
and default limits as the target. Each fixture must import successfully through public
`MemoryAssetIo`/`ImportPlan::read_asset`, satisfy literal semantic or numeric expectations, capture the
existing bounded snapshot, and release all import-ledger reservations. These assertions do not
depend on mutation finding an accepted document. The library validation command below includes
these tests; neither their addition nor this description claims an executed fuzz campaign.

## Serial validation and retained campaigns

After changing dependencies, let Cargo update `fuzz/Cargo.lock` before using `--locked`; do not
edit it manually. This target adds the existing workspace `baozi` and `baozi-postprocess` packages.
The focused deterministic tests can run without building the libFuzzer binaries:

```text
cargo +1.95.0 update --manifest-path fuzz/Cargo.toml --workspace
cargo +1.95.0 nextest run --locked --manifest-path fuzz/Cargo.toml --target-dir target --lib --build-jobs 1 --test-threads 1
cargo +nightly-2026-10-28 fuzz check edit_lifecycle
cargo +nightly-2026-10-28 fuzz run --sanitizer address edit_lifecycle -- -runs=256 -max_len=8196
cargo +nightly-2026-10-28 fuzz run --sanitizer address edit_lifecycle -- -max_total_time=1800 -max_len=8196
cargo +nightly-2026-10-28 fuzz run --sanitizer address edit_lifecycle -- -max_total_time=120 -max_len=8196
```

The last command resumes the corpus saved by the preceding command in
`fuzz/corpus/edit_lifecycle/`. Run the sanitizer commands on Linux with cargo-fuzz 0.13.2. Retain
stdout/stderr and exit status for each run, source SHA and dirty diff, `rustc -Vv`, cargo-fuzz
version, exact arguments, the resulting corpus and any crashes. A short run is exploration
evidence, not proof of edit safety. No campaign result is implied by these instructions.

Revision-qualified executed observations and local retained artifact paths belong to the
[asset workflow expansion review](../docs/reviews/2026-11-01-asset-workflow-expansion.md#v2--edit-lifecycle-fuzz-execution).

The scheduled workflow keeps its corpus/log artifact upload on success or failure. A manual run
can set `resume_run_id` to a previous **Scheduled Fuzz** run in the same repository. Each matrix
job downloads its existing `<target>-fuzz-*` artifact, restores only `corpus/<target>/` entries
into the normal corpus directory and records the requested run ID in `environment.log`. A missing
artifact or empty restored corpus fails explicitly. Resuming a run predating `edit_lifecycle`
will therefore fail that matrix job. Uploads retain the resumed corpus and the new logs as before.
