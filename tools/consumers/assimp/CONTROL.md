# Assimp progress and false-return control observations

`src/control.cpp` implements a separate C++17 diagnostic executable for the
unmodified Assimp **6.0.5** release, revision
`392a658f9c271be965271f45e7521a1b80ea4392`. It records progress callbacks and their
boolean decisions around a fixed set of geometry imports. It collects no timing
and does not implement cancellation on Assimp's behalf.

The fixed F32 matrix has been compiled and executed on Windows and Linux; see
the [qualified observations](../../../docs/reviews/2026-11-02-assimp-control-observations.md)
for exact source, build and report identities. Ordinary probe endpoints,
fixtures, timings, and benchmark protocol remain independent of this executable.

## Integration and invocation

The existing CMake project includes an optional target, disabled by default:

```cmake
option(BAOZI_ASSIMP_BUILD_CONTROL "Build finite progress/cancellation diagnostics" OFF)
if(BAOZI_ASSIMP_BUILD_CONTROL)
    add_executable(baozi-assimp-control src/control.cpp src/main.cpp)
    target_compile_definitions(baozi-assimp-control PRIVATE BAOZI_ASSIMP_NO_MAIN)
    configure_probe(baozi-assimp-control)
endif()
```

Enable `BAOZI_ASSIMP_BUILD_CONTROL=ON` in the existing F32 configuration, then
build target `baozi-assimp-control`. It uses the same pinned `assimp` target,
headers, link settings, and scalar/source-identity definitions as the ordinary
probe. Retained source is at
`target/assimp-comparison/windows-assimp-6.0.5-source`. `src/main.cpp` supplies only
the linked comparison implementation; this diagnostic calls its public
`comparison::check_runtime` and does not call its benchmark operations or clocks.
The existing F64 configuration enables PLY only and is rejected by this matrix.

```text
baozi-assimp-control ROOT REPORT.JSON
```

`ROOT` must not exist, and its parent must already be a directory. `REPORT.JSON`
may be a filename, such as `control.json`, or a path resolving to a file directly
inside `ROOT`. Its name must differ from `fixtures`. Relative paths with a parent
component resolve from the working directory; a bare filename resolves inside
`ROOT`. For example, after the owner builds the optional target:

```text
baozi-assimp-control target/assimp-comparison/p6-control-windows-f32 control.json
baozi-assimp-control target/assimp-comparison/p6-control-linux-f32 control.json
```

Use a different fresh root for each invocation. An existing root, including an
empty directory or a symlink, is refused. `std::filesystem::create_directory`
claims the root without retrying or clearing it. The executable creates only
the eight fixed files in `ROOT/fixtures/` and the requested report in `ROOT`.
It never removes artifacts, including partial artifacts after a failure. Use
task-owned roots without concurrent external edits. This is ordinary local
artifact management using C++17 filesystem APIs, not a hostile-filesystem sandbox.

Exit status is **0** when all 75 records and expected checks pass, **1** when a
completed report contains a discrepancy, and **2** for argument, runtime, fixture,
or report setup/output failures. Stdout contains a short report-location message;
the JSON is in the report file. Setup failures go to stderr and can leave no
report or an incomplete report. A native crash also can leave partial JSON;
retain it and the supervising process outcome without treating it as a matrix
result. An external timeout, if required, belongs to the integration owner.

## Fixed inputs and operation budget

All inputs are originally authored by this executable. Each contains exactly
one counterclockwise triangle with positions `(0,0,0)`, `(1,0,0)`, `(0,1,0)`.
Only STL has authored normals, all `(0,0,1)`. There are no sidecars, images,
compression extensions, animation, or external resources.

| Filename | Representation and purpose |
| --- | --- |
| `short.obj` | Short newline-terminated OBJ without normals or materials; also used for fresh P0-P3 imports |
| `cache-boundaries.obj` | Exactly 35,127,296 bytes (33.5 MiB), same triangle, padded with comment lines of at most 64 bytes; crosses the pinned 16 MiB OBJ cache twice |
| `ascii.stl` | One ASCII facet with an authored positive-Z normal |
| `binary.stl` | 80-byte header, LE facet count, one 50-byte binary facet |
| `ascii.ply` | Three F32 positions and one triangle, ASCII |
| `binary-le.ply` | Three binary32 positions and one `uchar`/`uint` index list, little-endian |
| `embedded.gltf` | glTF 2.0 JSON with a 42-byte base64 data URI buffer |
| `embedded.glb` | glTF 2.0 GLB with the same 42-byte logical buffer; JSON and BIN chunks padded to four bytes |

Both IO modes use `Importer::ReadFile(path, 0)` with a new importer and an
importer-owned progress handler for every sequence. The `P` cases freshly import
the unchanged `short.obj` without normals, then call only
`ApplyPostProcessing(aiProcess_GenSmoothNormals)`, with the smoothing angle set to
175 degrees. They retain both import and final geometry observations.

| Cases | Fixtures | IO modes | Import sequences | Additional operations |
| --- | --- | --- | --- | --- |
| I0-I3 | All eight | File and memory | 64 | None |
| P0-P3 | `short.obj` | File and memory | 8 | Eight public postprocess calls |
| U0-U2 | None | None | 0 | Three standalone calls through `ProgressHandler &` |

Total: **72 import sequences plus 3 adapter calls**, producing 75 records. There
are no warmups, repetitions, retries, fixture selection flags, or matrix expansion.
Every I0 baseline precedes its corresponding I1-I3 cases. P0 precedes P1-P3 in
each IO mode. A failed baseline does not silently drop later cases. If setup for
an individual sequence fails, its record identifies the missing public calls;
the final `matrix_complete` check requires 72 actual `ReadFile` calls and eight
actual `ApplyPostProcessing` calls.

File mode forwards to Assimp's `DefaultIOSystem`. A stream wrapper counts IO
without changing read/seek results. Memory mode preloads the complete exact input
before installing the handler. It serves the pinned public `MemoryIOStream` only
for exact primary-path matches; each open receives an independent cursor. Other
opens are denied, with no filesystem fallback. It does not use `MemoryIOSystem`,
`ReadFileFromMemory`, or the magic-filename route. `Exists` is an availability
query, not an abort request. Unexpected non-primary opens or failed opens make a
record fail qualification rather than being relabeled as cancellation.

## Trigger definitions and interpretation

The observer overrides the two void adapters to tag `(kind, current, total)` and
calls their unchanged base implementation. Its `Update` records the float and
the boolean it returns. It returns false from the selected event onward. It does
not throw, fail IO to force an outcome, invoke importer methods from a callback,
or stop the library when its request is ignored.

| Case | Selection |
| --- | --- |
| I0 | Accept all callbacks; establish that fixture/mode's qualified baseline and last file-read ordinal |
| I1 | First file-read callback, after detection and size lookup |
| I2 | First file-read callback with `0 < current < total`; never substitute a final callback |
| I3 | Last file-read ordinal from the successful I0 for the same fixture/mode |
| P0 | Accept import and postprocess callbacks |
| P1 | First postprocess callback |
| P2 | First postprocess callback with `0 < current < total` |
| P3 | Postprocess callback with `total > 0 && current == total` |
| U0 | `ProgressHandler &.Update()` using the public default argument; expected value `-1` |
| U1 | `UpdateFileRead(0, 0)`; expected value `0.5` |
| U2 | `UpdatePostProcess(0, 0)`; expected value `1` |

I3's ordinal selection matters: OBJ's parser can report the full cached file
size before native scene construction. `current == total` alone cannot identify
the common reader's final boundary. The larger OBJ provides strict interior
events; absence of such an event in the other seven inputs is a separately
reported coverage limit. Postprocess step indices include registered inactive
steps and do not demonstrate callbacks within normal generation.

Both qualified hosts observed 40 `false_return_ignored` import records, 14
`trigger_not_reached` records, and 18 `accepted` import records, matching the
pinned source prediction. The three adapter records each produced
`adapter_observed`. These results qualify the fixed release and cases only.

Result classification is deliberately conservative:

| Result | Evidence |
| --- | --- |
| `accepted` | No false-return policy, successful scene and independent geometry checks |
| `trigger_not_reached` | A false-return policy was requested, no selected event occurred, and import/geometry succeeded |
| `false_return_ignored` | A selected event returned false and import/final geometry still succeeded |
| `unrelated_failure` | Missing/incorrect scene or geometry, caught exception, library exception, unexpected IO failure, or incomplete trace |
| `cancelled_after_false` | Reserved for a null result causally attributable to false; this implementation has no qualifying attribution source and never emits it |

In particular, a successful baseline plus a later null result does not establish
cancellation. A null after false is flagged `null_after_false_unattributed` and
classified as `unrelated_failure`, preserving the error and exception evidence.
The code does not search error strings for words such as "abort" or "cancel".
A library-caught exception reported by `GetException` also prevents a successful
control classification. Investigate any discrepancy before asserting a P6
guarantee. Exit zero means the inspected behavior matched the defined checks; it
does not mean Assimp honored false-return cancellation.

The source propagation boundary is explicit: pinned
`include/assimp/ProgressHandler.hpp:106-122` calls `Update` from each void adapter
and discards its boolean result. The selected common importer and OBJ call sites
invoke those void adapters, so false supplies no abort branch there. This
executable lets that unmodified path run and records its outcome. The documented
public false-return promise is not evidence that the selected implementation
honors it.

## Report and evidence bounds

The standalone schema is `baozi-assimp-control/v1`. Each import record contains
`engine`, `io`, `fixture`, trigger selection/reachability, first false event,
`scene_kind`, `result`, native error/exception evidence, import/final geometry,
and individual checks. Raw events retain one-based `event_ordinal`, per-kind
ordinal, adapter kind and integer inputs, mapped float, return decision, and an
IO counter snapshot. `current` and `total` are null for a direct `Update` call.
The final summary records the actual public-call counts and complete-matrix and
check outcomes.

Geometry checks independently require one mesh, three vertices, one face, three
indices, valid indices, all three ordered analytic triangle corners, and the
expected normal count/direction. P cases additionally require normals absent at
import and generated at completion. Controlled cases compare geometry and raw
callback routes with their successful I0 or P0 baseline. Callback-shape checks
validate entry/final read boundaries, interior coverage, absence of import-only
postprocess/unknown events, adapter float mapping, and the observed registered
postprocess sequence. They do not parse the model files again or mirror Assimp's
scene construction.

Callback recording uses **256 preallocated events per record**. It makes no
allocations or exception-producing calls from `Update`; the void overrides only
tag and invoke the base adapters. If a trace overflows, counting and the selected
return policy continue unchanged, and qualification fails after the operation.
Trace storage, memory bytes, and IO counters outlive the importer and its owned
handlers. Importer destruction of the handler is recorded. Scene observations
occur before importer destruction; serialization occurs afterward.

OBJ checks progress inside its line-reading loop
(`code/AssetLib/Obj/ObjFileParser.cpp:134-152`), but only invokes the adapter when
`lastFilePos < streamBuffer.getFilePos()`. The pinned position advances on cache
refill, rather than on every line. Consequently the source prediction is three
file callbacks for the short OBJ and five for the 33.5 MiB OBJ (common entry,
three cache positions, common final), despite its many short comment lines.
The other six inputs predict two file callbacks. A P sequence predicts the short
OBJ's three file callbacks plus `N + 1` postprocess callbacks, where `N` is the
actual registered step count. These are source predictions, not a public callback
count limit or executed counts. The fixed 256-event capacity is an observation
budget: any different line/cache cadence or excessive step count sets overflow
and fails qualification without allocating, throwing, or changing the library's
control flow.

IO snapshots distinguish the first false event, public-operation return, and
importer destruction. Explicit before-return deltas exclude later importer
cleanup. Counts include existence/path comparisons, opens, stream operations,
and stream destruction; byte counts include only bytes returned by `Read`.
They are observations, not cancellation or work budgets. Preloading and fixture
generation are outside these counts and cannot be retroactively cancelled.

Diagnostic strings are capped at 4,096 source bytes; artifact paths longer than
that are rejected. Control bytes and non-ASCII bytes are JSON `\u00xx` escaped,
so truncated or non-UTF8 native errors cannot corrupt JSON. These escapes preserve
source byte values; non-ASCII text is not Unicode-normalized. Nonfinite callback
values serialize as null with `value_finite: false` and fail the import trace
checks. No scene arrays, payload bytes, clock values, or unbounded logs are
embedded in the report.

Fixture entries contain the fixed generator version, byte counts, and decimal
FNV-1a-64 fingerprints. FNV is only a reproducibility fingerprint, not a
cryptographic provenance check. Memory preload verifies this fingerprint before
calling Assimp. Runtime identity includes the expected source pin, existing
configured source identity, actual library version/revision/compile flags,
scalar width, compiler, platform, and debug macro. It reuses the ordinary runtime
version/scalar/pin check rather than creating another verifier.

Keep the actual JSON and all eight inputs with the pinned source/archive SHA-256
receipts, build configuration/commands, executable and loaded-library SHA-256
hashes, and process outcome. The embedded metadata does not replace those external
source/build identities. Keep this instrumented diagnostic outside ordinary
benchmark timing/RSS qualification. The qualified observations retain serial
Windows/Linux runs and negative CLI checks; changed source, library or fixtures
require fresh qualification.
