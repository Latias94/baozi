# Baozi and Assimp Comparison Protocol

This finite development benchmark compares public native-scene import, explicit geometry preparation, owned output extraction and the complete pipeline.
It does not measure parser-only work or establish general format compatibility.
The current protocol and generated corpus are v1; results bind their exact file digests and source revisions as well as these labels.

## Corpus and Builds

Generate the original, image-free corpus outside measured processes:

```text
python scripts/generate_asset_workloads.py --comparison --output-dir target/assimp-comparison/corpus
```

`tools/consumers/assets/fixtures/comparison-cases.json` owns the finite families and sizes.
The generated `comparison-cases.json` binds every input's bytes and SHA-256 to independent analytic expectations, not either importer's output.
Existing real-asset successes and named rejections from `expectations.json` remain a separate capability corpus until their requested work is matched on both sides.
Missing images/sidecars, skin evaluation and full material extraction cannot silently disappear from one engine's timed task.

Use optimized uninstrumented binaries for timing and ordinary RSS.
Primary Assimp is v6.0.5, commit `392a658f9c271be965271f45e7521a1b80ea4392`, with only OBJ/STL/PLY/glTF importers and no optional postprocess preset.
Build Assimp's double-precision variant separately for F64 PLY; record matching header/library precision, compiler, CPU flags, link mode, CRT/stdlib and allocator.
The later development reference checkout is not the primary release comparator.
Do not change its contents to obtain the release source.

## Fixed Worker Interface

Both executables accept the same comparison arguments, while the Baozi executable keeps its independent full-asset workflow:

```text
PROBE ROOT RELATIVE_ASSET --comparison STAGE --io-mode MODE --fields FIELDS --normals POLICY --max-work-units N
```

- `STAGE`: `import`, `prepare`, `extract`, `pipeline`.
- `MODE`: `file` or `memory`. Memory mode preloads one complete single-file input before any stage clock; no sidecar fallback is allowed.
- `FIELDS`: `position`, `position-normal`, `position-normal-uv0`. No unrequested material/image/animation extraction occurs.
- `POLICY`: `preserve` or `missing`. The latter generates missing smooth normals during preparation; already authored normals are retained. Initial equivalence cases are planar.
- Geometry remains in the source coordinate space. No unit/basis conversion, welding, influence limiting, cache optimization or instance baking is enabled.
- The comparison API is private to these tools. It is not another public Scene or GPU ABI.

Assimp includes its built-in ScenePreprocessor even with zero optional flags.
The native Scenes also differ in layout, validation, retained source/evidence and resource-control
obligations. Import results compare the cost of each actual public contract on the same input;
they do not isolate parser cost or claim identical retained storage. The selected detached-output
task establishes its own shared semantics without suppressing either library's required work.
For glTF UV0, Assimp's native V convention is retained in import observations and restored to source convention as explicit preparation work for subsequent endpoints.
The initial corpus contains only triangle lists and points; point cases have no preparation endpoint.
Both workers reject unsupported topology or missing requested fields rather than fabricate values.
The memory/file modes use the same bytes; their backend and security-policy differences are named in reports.

## Timers and Ownership

| Endpoint | Before clock | Timed work | State at stop |
| --- | --- | --- | --- |
| Import | Input/IO, importer and compiled request/properties exist | One public native-scene import | Scene/evidence alive, no preparation/extraction |
| Prepare | Fresh import complete | Target collection, explicit normals/topology/UV work, intermediate report/old-generation cleanup | Prepared Scene/evidence alive |
| Extract | Fresh import and preparation complete | Construct only selected owned fields and draw placements | Scene and output alive together |
| Pipeline | Arguments and optional single-file preload complete | IO/importer/request setup, import, preparation, extraction, destruction of native Scene/evidence/importer/IO | Only detached output and symmetric preloaded input remain |
| Parent wall | Before child launch | Complete child lifetime | Includes startup, semantic checks and JSON serialization |

Use monotonic nanosecond clocks.
Stage workers validate outside their timer; the pipeline validates its detached output after native cleanup.
Do not sum stage clocks to invent complete-pipeline time.
Stage-specific peak RSS is not available: even an extraction worker's process peak includes setup, prior import/preparation and final checking.
The full asset consumer's source-tracing and snapshot work are absent from this comparison path.

## Observation and Semantic Evidence

Each worker writes one JSON object on stdout, with diagnostics on stderr:

```json
{
  "schema": "baozi-assimp-observation/v1",
  "engine": "baozi",
  "status": "success",
  "stage": "extract",
  "io_mode": "memory",
  "fields": "position-normal",
  "normal_policy": "missing",
  "scalar_bytes": 4,
  "computation_scalar_bytes": null,
  "observation_scalar_bytes": 8,
  "output_scalar_bytes": 8,
  "stage_ns": 1000,
  "observation": {
    "draws": 1,
    "placements": 1,
    "tuples": 3,
    "bounds": [[0, 0, 0], [1, 1, 0]],
    "area": 0.5,
    "normal_bounds": [[0, 0, 1], [0, 0, 1]],
    "uv0_bounds": null,
    "translation_x_sum": 0
  },
  "output": {"logical_bytes": 280, "capacity_bytes": 384}
}
```

`scalar_bytes` identifies retained/source geometry width where the worker can establish it; mixed/unknown widths require an explicit diagnostic rather than claiming F64 based on its destination.
All copied comparison geometry/instance transforms use F64, and each report records computation/output widths separately.
An engine without one global native arithmetic width reports it as null and explains the scope;
F64 observation or output storage does not imply F64 native parsing or preparation.
`draws` counts primitive definitions (Assimp's primitive meshes), and `placements` counts draw placements; no count is multiplied into the unique draw payload.
`tuples` counts expanded triangle corners or points.
Draw positions remain mesh-local; separate column-major F64 world transforms identify placements.
`translation_x_sum` sums each draw placement's world X translation.
UV0 bounds after preparation use the source convention; import observations label the native convention.
If selected fields are absent before preparation, import observation reports null for those fields; that absence is not filled by an unmeasured generation pass.
No copied comparison output is constructed for import/prepare; their `output` is null and observations stream public native data outside the stage clock.

The private output consists of per-draw position arrays, optional normal/UV0 arrays and separate draw-placement references/transforms.
Report logical selected payload bytes and actual allocated capacities separately, including placement matrices/references.
Output lifetime tests read these arrays after every native object is destroyed.
Do not substitute the larger indexed ApplicationAsset or copy its materials into this task.

Aggregate observations above are sanity checks, not a general proof of semantic equality.
Before a case contributes ratios, both probes' independent tests verify complete small-case tuples, face winding/coverage, UV convention, normal policy, shared placement transforms and owned output lifetime.
Grid triangles use the same generated diagonal; small-grid tests verify every tuple against the construction formula.
F64-neighbor tests preserve adjacent values that collide in F32.
Large cases use the already characterized family plus bounded streaming observations, input hashes and deterministic operation contracts.
Hash equality proves repeatability, not universal equivalence across arbitrary meshes.

Statuses are `success`, `policy_rejected`, `unsupported`, `semantic_mismatch`, `timeout`, or `crash`.
Workers return a nonzero exit code for failures; the collector retains their object/logs, preserves partial results and continues other cases.
Only two successes with the same admitted case/stage/IO/precision/fields/normal policy can supply a ratio.
The collector checks analytic source facts with a documented tolerance appropriate to the source width; F64-neighbor checks use width-sensitive tuples in semantic tests, not a loose aggregate tolerance.

## Repetitions and Memory

Use three untimed warm-up observations and twenty retained fresh-process observations, serially alternating AB/BA engine order.
Caches are warm; fresh processes do not establish cold filesystem caches.
Retain every sample, median, interquartile range, min/max, timer-resolution limitations, case order and host load.
Do not put builds, fixture generation, downloads, profiler injection or global cache flushing inside timed workers.

Linux `/usr/bin/time -f %M` supplies each child's whole-lifetime peak RSS in KiB, converted to bytes.
Windows peak working set, current private commit and peak private commit remain distinct optional metrics from the documented process API.
Never subtract idle RSS or consecutive high-water marks to invent stage peaks.
Linux/Windows values are separate series.

Heaptrack is a separate instrumented campaign on both native executables with debug symbols.
Verify Rust System/malloc/realloc/alignment/C++ new/thread/dependency interception first.
Do not use profiled timings as speed results or the profiler process's RSS as the ordinary worker RSS.
Unverified/unavailable heap metrics are null with a reason.
Baozi ledgers, Rust-only allocation-counter requests and Assimp Scene-size estimates remain differently scoped diagnostics.

Capture B0 before any production semantics change and B1 using the same case/protocol/build policy after the cutover.
Choose and record regression budgets from actual B0 variance before optimizing; no budget or performance result exists yet.
Report individual case/capability results instead of an aggregate winner or assumed superiority.
