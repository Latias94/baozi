# Direct Assimp geometry comparison

This private executable implements the fixed [comparison protocol](../../../docs/testing/assimp-comparison.md).
It measures a public Assimp native-scene import, explicit preparation, selected owned extraction, or the complete pipeline.
It is a finite development comparator, not a general exporter or a format-compatibility claim.
No Baozi library or production implementation is linked or changed.

## Source and configuration

Use Assimp **6.0.5**, commit **`392a658f9c271be965271f45e7521a1b80ea4392`**, already available on disk.
The later `repo-ref/assimp` checkout is for reading only and is rejected as the primary source.
CMake uses `add_subdirectory`; it never fetches source, dependencies, images or test assets.
Selective source archives must include root `assimp.pc.in`: upstream configures it unconditionally, even with installation disabled.
Unused readers, all exporters, upstream tools/samples/tests, Hunter, Draco and USD/VRML integrations are disabled.
The default static F32 configuration enables only OBJ, STL, PLY and glTF readers, with bundled local zlib.

`BAOZI_ASSIMP_PRECISION=f32|f64` explicitly controls both Assimp and consumer headers.
A compile-time assertion checks `sizeof(ai_real)` against that configuration.
Before import, a runtime check compares it with `aiGetCompileFlags()` and requires version 6.0.5.
A clean Git source must resolve to the full pinned commit, and its runtime revision must match.
An exported source archive needs no standalone `.git` and is accepted with an `archive-declared-pin` identity.
`BAOZI_ASSIMP_SOURCE_REVISION` explicitly declares the full pin (defaulting to the required commit); any different declaration is rejected.
The archive's version is checked, but its complete contents cannot be attributed to a commit by that check or declaration.
The integration owner must retain the archive/source digest and independent pin provenance before admitting results from an archive.
CMake suppresses upstream Git discovery for archives, which otherwise can report an enclosing Baozi repository's revision.

The separately built **F64 variant enables PLY only**.
At this release, `code/AssetLib/glTF2/glTF2Asset.inl`'s `Accessor::ExtractData` copies accessor bytes into destination elements rather than converting F32 components to F64.
Consequently the F64 variant must not be taken as evidence of double-width glTF support.
No reference source is patched to work around that behavior.

Configure/build/test commands below are instructions for the integration owner; no execution is implied by this README.
Run builds serially, outside measured workers, and retain their exact commands and logs.
Use the same compiler, optimization, CPU flags, link mode, runtime and allocator policy for the corresponding Baozi observations.

```text
cmake -S tools/consumers/assimp -B target/assimp-comparison/build -G Ninja -DCMAKE_BUILD_TYPE=Release -DBAOZI_ASSIMP_SOURCE_DIR=<existing-release-source> -DBAOZI_ASSIMP_PRECISION=f32 -DBUILD_SHARED_LIBS=OFF -DBUILD_TESTING=ON
cmake --build target/assimp-comparison/build --parallel 1
ctest --test-dir target/assimp-comparison/build --output-on-failure -j 1

cmake -S tools/consumers/assimp -B target/assimp-comparison/build-f64 -G Ninja -DCMAKE_BUILD_TYPE=Release -DBAOZI_ASSIMP_SOURCE_DIR=<existing-release-source> -DBAOZI_ASSIMP_PRECISION=f64 -DBUILD_SHARED_LIBS=OFF -DBUILD_TESTING=ON
cmake --build target/assimp-comparison/build-f64 --parallel 1
ctest --test-dir target/assimp-comparison/build-f64 --output-on-failure -j 1
```

Generate the original corpus beforehand using the repository's documented generator.
`BAOZI_ASSIMP_CORPUS_DIR` defaults to `target/assimp-comparison/corpus` in the repository; supply an absolute directory if it lives elsewhere.
Use distinct build directories for the two ABIs; do not mix their headers, objects or shared libraries.
For multi-configuration generators add `--config Release` to build and `-C Release` to CTest; executables may be under `Release/`.
`BUILD_SHARED_LIBS=ON` and `USE_STATIC_CRT=ON` are explicit alternatives that must be recorded, not silently mixed with the default series.
If shared libraries are selected, arrange the runtime library search path before starting workers and hash the loaded libraries too.

Each build emits `comparison-build-<configuration>.txt` alongside its CMake cache.
Retain that file, `CMakeCache.txt`, verbose compile/link commands, source/archive digests, executable/library SHA-256 hashes, compiler/stdlib/CRT versions, effective CPU flags and host information.
The build file records configuration; it does not replace an external build/source manifest or identify the platform allocator's implementation.
Ordinary timings and RSS require optimized, uninstrumented builds.
Heap profiling is a separate externally qualified campaign; this consumer does not install allocator hooks.

## Worker interface and output

```text
baozi-assimp-probe ROOT RELATIVE_ASSET --comparison import|prepare|extract|pipeline --io-mode file|memory --fields position|position-normal|position-normal-uv0 --normals preserve|missing --max-work-units N
```

All five options are mandatory and may appear in any order.
Unknown tasks/selections are rejected instead of invoking conversion, image extraction or an optional postprocess preset.
`N` is an unsigned 64-bit **Baozi-specific request record**.
Assimp has no equivalent work budget: the report always says `max_work_units_enforced: false`, including when `N` is zero.
Use an external collector timeout to bound the child process; this argument is not a cancellation or allocation limit.

The root asset must resolve to a regular file inside `ROOT`; parent traversal and escaping symlinks are rejected.
Both modes call the public `Importer::ReadFile` with zero optional flags and a root path carrying the original extension.
File mode forwards to Assimp's default filesystem backend and observes non-primary `Open` attempts without interrupting native fallback behavior.
In this release, OBJ material-library opens are reached through explicit `mtllib` or its fallback: any such attempt makes this single-file geometry task `unsupported` at operation `import`, even if the importer returns a Scene or finds a fallback MTL.
This is a bounded OBJ admission rule, not a general claim that every failed IO candidate in every reader is a required dependency.
The native file backend's filesystem access remains unrestricted; unmatched image tasks also remain subject to the Scene-property checks.

Memory mode preloads the exact complete single-file input and installs a private IO system that serves borrowed bytes only when the requested path exactly matches the primary path.
Repeated primary opens receive independent stream cursors; every non-primary `Open` is denied and produces `policy_rejected` at operation `import`.
The implementation reuses Assimp's public `MemoryIOStream` without using `MemoryIOSystem` or `ReadFileFromMemory`.
The latter installs a wrapper that matches the `$$$___magic___$$$` prefix, so an authored sidecar with that prefix can otherwise receive the primary buffer without reaching a custom deny handler.
The exact-primary authority removes that resource substitution and performs no filesystem fallback.
In both modes, `Exists` remains an availability query; a negative optional query alone never poisons import admission.
Reports name both backend and policy; these modes do not claim identical filesystem security behavior.

Each invocation emits exactly one JSON object on stdout and sends failure diagnostics to stderr.
The schema is `baozi-assimp-observation/v1`, engine `assimp`, with all protocol observation fields and nullable `output`.
Successful exit is zero; typed failure is nonzero.
Failures contain `error` and the actual `failure_stage` operation (`arguments`, `runtime`, `input`, `setup`, `import`, `prepare`, `extract`, `cleanup`, or `observe`), with unavailable observations/timings null.
The finite status vocabulary is `success`, `policy_rejected`, `unsupported`, `semantic_mismatch`, `timeout`, and `crash`.
A hard native crash or external timeout can prevent worker JSON; the supervising collector owns those outcomes and retains partial logs.

Widths have explicit scopes:

| Field | Meaning |
| --- | --- |
| `scalar_bytes` | Retained public Assimp geometry width, `sizeof(ai_real)`; not inferred from the copied destination and not a claim about original accessor widths |
| `library_scalar_bytes` | Runtime library compile-flag width, checked against the consumer |
| `computation_scalar_bytes` | Native normal generation, UV restoration and world-matrix composition use `ai_real` |
| `observation_scalar_bytes` | Bounds, area and translation sums are computed in binary64 |
| `output_scalar_bytes` | All copied positions, normals, UV0 and placement matrices use binary64 |
| `finite` | `true` after selected geometry/matrix/aggregate checks, `false` on detected nonfinite values, `null` when no such check completed |

Import labels glTF UV0 as `assimp-native-one-minus-source-v`; preparation restores source V explicitly.
Import may report null requested normal/UV bounds when the native fields are missing.
Later endpoints reject missing requested fields.
`preserve` never generates missing normals; `missing` uses only `aiProcess_GenSmoothNormals`, preserving authored normals, with smoothing angle explicitly set to 175 degrees.
The admitted generated normal cases are planar, so this does not claim general equivalence between smoothing algorithms.
Import uses zero optional postprocess flags and still includes Assimp's built-in `ScenePreprocessor`.
Only existing triangle lists and PLY position-only points are admitted; there is no implicit triangulation, welding, axis/unit conversion, instance baking or optimization.
Point cases have no preparation endpoint or normal-generation pass.
Texture/image, skin, morph and animation tasks are rejected as unmatched rather than contributing geometry-only ratios.

The copied private output contains per-draw expanded arrays and separate placements with a `uint32_t` draw reference and column-major F64 world matrix.
Draw payload remains mesh-local and is counted once per Assimp primitive mesh, regardless of how many nodes reference it.
There are no copied materials, general Scene mirrors, asset exporters or unrequested arrays.
Only aggregate facts and selected output byte counts are serialized; the private arrays exist for ownership and full-tuple semantic evidence.
`logical_bytes` counts initialized selected array elements plus 128 matrix bytes and four reference bytes per placement.
`capacity_bytes` counts actual vector capacities, including allocated draw/placement elements, their padding/vector descriptors and all selected array storage.
It excludes stack objects, allocator metadata and all native-scene/preloaded-input allocations; it is not a heap or RSS metric.

## Clock and ownership boundaries

The implementation uses monotonic `steady_clock` nanoseconds.
No worker reuses a previous Scene.

| Endpoint | Before clock | Timed | At stop |
| --- | --- | --- | --- |
| Import | Input/IO, fresh importer, explicit properties | One public import with zero optional flags | Native Scene alive; no preparation or copied output |
| Prepare | Fresh import complete | Target collection, field/topology admission, missing-normal generation if requested, selected glTF UV restoration, intermediate cleanup | Prepared Scene alive; no copied output |
| Extract | Fresh import/preparation complete | Allocate/copy selected arrays and placements | Scene and output alive together |
| Pipeline | Arguments and optional complete memory preload | File IO/path setup, importer/properties/IO setup, import, preparation, selected extraction, `FreeScene`, importer/IO and temporary input destruction | Detached output and symmetric memory preload remain |

Native import/prepare observations stream public mesh data and placements after their clocks, without allocating a copied output.
Extraction observations validate selected arrays after the extraction clock while the Scene still exists.
Pipeline observations validate only detached arrays after every native object is destroyed.
Structural task admission and bounds-safe indexing are necessary parts of executing preparation/extraction; finite checks and observation arithmetic are outside the timer.
Stage clocks are not summed into a pipeline estimate.
Every process peak includes all setup and checking, even when only extraction is timed.

## Evidence and integration checks

CTest reads the generated small cases but never derives expected values from imported results or the generated manifest's aggregate observations.
The expected tuples, diagonal, winding, matrices, normals and UVs are independently authored in C++.
Additional tiny PLY/OBJ/GLB sources are authored directly by the semantic tests in an isolated temporary directory.

| CTest scenario | Observable evidence |
| --- | --- |
| `assimp-runtime` | Explicit configure/header/library width and release agreement; fresh importer; admitted/disabled readers |
| `assimp-grid` (F32) | Every expanded tuple of the 2x2 grid against cell/diagonal formulas, winding and coverage, authored normals, UVs, selected bytes and native-only endpoint output absence |
| `assimp-normals` (F32) | Missing normals remain absent at import; planar generation and deliberately opposite authored normals coexist without overwriting |
| `assimp-shared` (F32) | Eight unique primitive definitions and sixteen placements; complete native/restored UV tuples; all world-matrix components; independent nested rotation/translation/nonuniform-scale composition without baked positions |
| `assimp-precision` | Generated F64 neighbors and an independently authored adjacent binary64 pair, exact width-sensitive values/area and signed zero; F32 explicitly demonstrates collision and cannot qualify a F64 ratio |
| `assimp-ownership` | Arrays/placements readable after `FreeScene`, importer/IO destruction and input overwrite/destruction; additional heap reuse and pipeline output reads |
| `assimp-points` | All 1,024 analytic point tuples, no fabricated faces/area and typed rejection of a preparation endpoint |
| `assimp-formats` | Complete small PLY/STL triangle tuples and analytic observations for all admitted endpoints/IO modes (PLY only for F64) |
| `assimp-rejection` | Untriangulated polygon, missing fields/input, root traversal, missing explicit OBJ MTL, magic-prefix memory sidecar denial, readable unmatched MTL/image task, optional `Exists`, independent primary reopens and nonfinite output |
| `assimp-json` | Fixed CLI and integer budget record, one-object success/failure schema, status vocabulary, null unavailable facts and escaped diagnostics |

Before collecting ratios, the integration owner must:

1. Run the two serial configure/build/CTest sequences above on the exact pinned release source. Confirm PointPLY success on the actual release build; source inspection alone does not close that gate.
2. Run both probes through the collector for every declared case, stage and IO mode, selecting the F64 Assimp binary only for the F64 precision lane. Parse actual stdout with the collector, retain typed rejections, and require analytic observation checks before admitting a pair.
3. Exercise CLI failures as actual subprocesses: malformed budget/selection, absent input, unmatched image/sidecar, point preparation. In both import and pipeline, the explicit missing-MTL file case must report `unsupported`, and the magic-prefix memory case must report `policy_rejected`, each with `failure_stage: "import"`. Require one parseable object and nonzero exit; keep successful single-file OBJ/GLB memory cases and no-`mtllib` file cases as positive controls. Do not infer subprocess behavior merely from linked core tests.
4. Retain binary/source/build identities and the independent small-case evidence with the raw B0 samples. Larger grid/repeated/shared families rely on that characterized construction plus bounded streaming facts and exact input digests; aggregate equality alone is not a universal semantic proof.
5. Keep warm-ups, repetitions, alternating order, whole-process RSS and any separately qualified heap campaign in the parent collector. No performance result or allocator-interception qualification is asserted here.

Builds, CTest and real subprocess/collector checks are required execution evidence, not source-review substitutes.
