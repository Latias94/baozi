# Pre-release Evolution Execution Record

Date: 2026-11-02, Asia/Shanghai.

The [reviewed development plan](../plans/2026-11-02-0923-refactor-pre-release-evolution-plan.md) governs the finite development tranche before publication.
Execution began from `4ff688d` after the user authorized Goal-backed fearless refactoring.
The first work establishes comparable probes and an immutable performance baseline before production semantics change.
No benchmark result, new renderer capability or final package qualification follows merely from these tool checks.

## Reproducible Cases

Commit `bed89fb` adds fourteen original single-file cases with exact source hashes and analytic expectations.
The families are repeated-face OBJ, increasing-unique-vertex OBJ grids, image-free shared-primitive GLB, repeated STL, F32/F64 PLY triangles and PLY points.
Their fixed [protocol](../testing/assimp-comparison.md) separates native import, preparation, selected output extraction and complete-pipeline lifetimes.
The real-asset corpus remains separately visible; unmatched material/image/deformation work is not admitted to comparison ratios.

Four independent Python generator checks first failed on the missing entry point, then passed after implementation.
They verify deterministic source bytes/hashes, small-grid construction, GLB resource/placement structure and distinct F64 values that collide in F32.
No Baozi or Assimp result supplies those source expectations.

## Baozi Comparison Endpoints

Commit `17b0ec7` adds the comparison endpoint to the standalone asset consumer; the full indexed asset workflow remains independent.
Only public reads build selected F64 position/normal/UV arrays and separate draw-placement matrices/references.
Import/preparation observations stream native data after their timers; the complete pipeline includes destruction of native Scene/evidence/importer/IO before observing detached output.
Source scalar width is distinct from output/observation width; native arithmetic without one global width is explicitly unknown.

| Check | Actual result | Evidence identity |
| --- | --- | --- |
| Initial comparison regressions | Four failures on the absent comparison CLI, before implementation | nextest `f5477880-d0ca-4ab6-ad0e-d4b4e8418c85` |
| Initial CLI error regression | Invalid policy produced no JSON, before the error-boundary correction | nextest `cf19090a-9eb1-46f2-9d37-9961ba2ee614` |
| Final comparison semantics | 10/10 pass on Windows x86_64 MSVC, Rust 1.95 debug | nextest `d2e1d182-2d9a-4615-960a-537744bbb97a` |
| Existing workflows | 19/19 pass, one existing manual case excluded | nextest `e9e755ff-4717-4a07-9756-af382d6ad08c` |
| Static checks | Strict all-target/all-feature consumer Clippy and consumer format check pass | Rust 1.95, shared root target, one build job |
| Corpus smoke | All 55 admitted case/endpoint combinations succeed in memory mode | `target/assimp-comparison/baozi-debug-smoke.json`; before the untimed diagnostic correction, not speed evidence |

Exact small-case assertions cover every grid tuple, GLB UV/normal tuple, placement matrix/draw reference, STL/F32-PLY/point payload, F64 neighbors and signed zero after native teardown.
A separate native reviewer found two measurement/equivalence-evidence issues and one CLI error issue.
The diagnostic pass was removed from pipeline timing, analytic coverage was completed and parsing/execution now share one JSON failure boundary.
The reviewer independently rechecked all three corrections; its report and source fingerprints are retained in `.git/baozi-review/pre-release-evolution-execution/rust-comparison-review.md`.

## External Source and Memory Calibration

The primary external source is unmodified Assimp v6.0.5, `392a658f9c271be965271f45e7521a1b80ea4392`.
A selective `git archive` plus its required root `assimp.pc.in` input supplies the local Linux source; the existing reference checkout is untouched.
`target/assimp-comparison/assimp-source-manifest.json` records the archive identity and 1,651 file digests; `assimp-source-supplement.json` binds the additional pkg-config input to the same commit.
An archive-declared pin is labelled separately from a Git-verified checkout.

Linux/WSL Ubuntu-26.04 provides Clang 21.1.8, Ninja/CMake, Rust 1.95 and GNU time.
Heaptrack 1.5.0 was installed for a separate instrumented campaign.
Original Rust System and C++ calibration programs allocate/reallocate touched storage, use aligned storage and allocate on another thread.
Their checksums match; heaptrack traces show the expected main reallocation, aligned allocation and thread allocation sites for both.
Raw programs, binary identities, invocations, traces, analysis and separate ordinary RSS logs are under `target/assimp-comparison/calibration/`.
Those observations establish the exercised interception paths only; they are not Baozi/Assimp workload comparisons, allocator completeness claims or stage-memory results.

## Released-Assimp Endpoints and Collector

Commit `197ee89` adds the direct C++ comparator; `2f5ebfa` adds balanced serial collection and explicit child timeouts.
The F32 build enables OBJ/STL/PLY/glTF and the separate F64 build enables PLY only.
Both use Clang 21.1.8, CMake Release and one build job on Linux x86_64 GNU under Ubuntu-26.04/WSL.
Public import includes each library's native Scene obligations; neither endpoint is labelled parser-only.

| Check | Actual result | Evidence identity |
| --- | --- | --- |
| Assimp F32 semantic cases | 10/10 pass after IO corrections | `/home/frankorz/baozi-assimp-comparison-build-f32/Testing/Temporary/LastTest.log` |
| Assimp F64 PLY semantic cases | 7/7 pass after IO corrections | `/home/frankorz/baozi-assimp-comparison-build-f64/Testing/Temporary/LastTest.log` |
| Pre-correction IO reproduction | Four actual CLI successes where the finite task requires failure | `target/assimp-comparison/io-regressions-before-fix-valid-args/reproductions.json` |
| Corrected CLI cases | 16/16 pass, including all four IO regressions and positive single-file controls | `target/assimp-comparison/io-regressions-after-fix/reproductions.json` |
| Collector checks on Windows | 16 pass; Linux-only touched-buffer RSS check skipped | Python unittest, 17 cases, 16.540 seconds |
| Collector checks on Linux | 17/17 pass, including GNU time RSS check | Python unittest, 84.181 seconds |
| Paired Linux smoke | 110/110 admitted case/stage/IO pairs pass; all 220 child RSS observations available | `target/assimp-comparison/linux-paired-smoke/report.json`, one retained pair, no warmup; not B0 |

Independent C++ review identified two concrete IO mismatches: missing explicit OBJ material-library opens could be ignored, and `ReadFileFromMemory` could substitute the primary bytes for a magic-prefix sidecar.
The finite file profile now observes default-backend opens and rejects unmatched OBJ sidecar tasks.
Memory mode uses public `ReadFile` with an exact-primary borrowed-byte IO system, independent stream cursors and no filesystem fallback.
Optional negative `Exists` queries do not poison admission.
Both findings were independently rechecked as resolved; the private report is `assimp-probe-review.md` under the execution review directory.

Collector review found missing timeouts at existing CI/example call sites and a Python 3.11-only hash API despite the documented Python 3.10 floor.
Callers now supply a finite timeout, and hashing streams through a bounded buffer.
The final reviewed script/test hashes are `a553d226f9080f10b55dfa23b3f8db97840ae3247b86dd93f878557f364b9707` and `ff1b9b264b80e56fa8d50c44061ce14c309199a341a155ebcc339739f17273a6`.
Per-child raw logs and handled-interrupt records remain available; the complete comparison report checkpoints once per lane.
Uncatchable termination can leave newer raw logs beyond that checkpoint, so no per-child crash-durability claim is made.

No production semantics have changed since `21ed79a`; the only changed file under production crate directories is a benchmark timing comment.
`target/assimp-comparison/B0-builds/` binds the smoke binaries, source/archive identities, exact compile/link commands and toolchain/host data.
Any subsequent tool correction requires its own binary identity before B0 collection.

## Immutable Timing and RSS Baseline

B0 completed on Linux x86_64 GNU under Ubuntu-26.04/WSL from 02:57:32 to 03:05:01 UTC on 2026-11-02.
All 110 case/stage/IO lanes have three warm-up pairs and twenty retained pairs: 5,060 successful child observations, 2,200 retained matched pairs and 4,400 retained whole-child RSS values.
No failure or unavailable retained RSS observation was discarded.
The report is `target/assimp-comparison/B0/report.json`, SHA-256 `fcd4371db801ebd15aec2cff060461127b093fd420c7fb78717d7d1913614f8c`.
It names source `197ee890dec1e97a43240e0e4f1cac0cb03dfe63` and records dirty state: the untracked execution/research drafts and preserved user notes are not production changes.
The optimized Baozi binary was built at `17b0ec7`; its compiling sources did not change before the source/build manifest was captured at `197ee89`. This is source-closure identity, not a claim of another build at that later commit.
Normal binary SHA-256 identities are Baozi `d9a6d4e07a11e45dcefc5596498e53e6b9ef0ba663841df873204328dbde48ad`, Assimp F32 `c08e25f8d0745d1e4981f3117d6d4f66eba2bcda9e165c1ff8bf6642ce1fdb50`, and Assimp F64 PLY `7454bc3f2974f6885f1c23e6c1376f52d8a62df63c31d4c2190a6ade157f35a7`.

Baozi's median relative IQR across lanes is 6.20% for endpoint time, 8.18% for complete-child wall time and 1.64% for RSS.
Fifteen endpoint-time lanes and twenty-three parent-time lanes exceed 25% relative IQR; their raw data remains and their timing is descriptive.
RSS relative IQR stays between 0.29% and 3.23%.
The [per-lane regression budget](../testing/assimp-b0-budget.json), fixed before production changes or optimization, retains the B0 median/IQR and B1 median limit for all 110 lanes.
Its rule is median plus `max(3 * IQR, measurement floor)`; floors are 1 microsecond and 4 KiB respectively.
Stored limits round that result upward to an integer nanosecond or byte. This explicit rounding rule resolves the independent data review's P3 precision finding; medians, IQRs, limits and all raw B0 values are unchanged.
Noisy time lanes require a retained confirmation series before attributing a future breach to code.

The following slice is the uninstrumented `pipeline` endpoint in memory-input mode. Times are median / IQR in milliseconds; RSS is the median whole-child peak in MiB. Other endpoints, file IO and every sample remain in the complete report.

| Case | Baozi time ms | Assimp time ms | Baozi peak MiB | Assimp peak MiB |
| --- | ---: | ---: | ---: | ---: |
| obj-repeated-1 | 0.791 / 0.040 | 4.339 / 0.168 | 9.572 | 20.715 |
| obj-repeated-1024 | 20.470 / 0.465 | 4.767 / 0.324 | 10.115 | 20.965 |
| obj-repeated-16384 | 345.526 / 2.693 | 8.843 / 0.301 | 19.586 | 24.215 |
| obj-grid-2 | 0.810 / 0.025 | 4.615 / 0.197 | 8.846 | 20.715 |
| obj-grid-32 | 33.880 / 0.423 | 5.465 / 0.570 | 10.053 | 21.463 |
| obj-grid-128 | 544.727 / 2.993 | 20.484 / 0.631 | 39.867 | 33.859 |
| gltf-shared-1 | 1.613 / 0.108 | 0.205 / 0.017 | 9.297 | 5.748 |
| gltf-shared-8 | 3.165 / 0.036 | 0.213 / 0.015 | 9.367 | 5.748 |
| gltf-shared-32 | 9.616 / 0.103 | 0.244 / 0.010 | 9.605 | 5.760 |
| stl-repeated-1 | 0.604 / 0.040 | 0.093 / 0.005 | 8.500 | 4.980 |
| stl-repeated-1024 | 6.191 / 0.063 | 0.195 / 0.013 | 8.982 | 5.502 |
| ply-triangle-1 | 0.683 / 0.034 | 0.726 / 0.151 | 8.445 | 6.461 |
| ply-f64-neighbors | 0.691 / 0.042 | 0.708 / 0.031 | 8.467 | 6.254 |
| ply-points-1024 | 3.454 / 0.072 | 0.789 / 0.060 | 7.584 | 6.715 |

These are costs of the actual public native-scene contracts on matched input and selected output.
They do not establish identical native storage/validation obligations, parser-only speed, stage-specific RSS or a general winner.
Large OBJ pipeline cases demonstrate substantial Baozi cost that must be profiled in the optimization slice; some smaller cases favor either engine.

## Separate Heap Observations

A separate Linux heaptrack 1.5 campaign completed twelve successful observations: six fixed cases, one fresh whole-pipeline memory-input worker per engine.
Both engines were rebuilt with optimized code and debug symbols; their different identities, exact commands, CMake caches and trace hashes are retained under `target/assimp-comparison/{linux-baozi-heap-build,linux-assimp-heap-build,B0-heap}/`.
The F64 PLY case uses the separate Assimp F64 build. The calibration above supplies the exercised interception evidence.

The following peak is tracked live allocation bytes across the **whole child**, including input preload, setup, validation and serialization. Heaptrack's printed K/M suffixes use decimal units.
It is neither a native-Scene estimate nor RSS nor a stage-specific peak. Each row is one instrumented observation rather than twenty repeated timing samples.

| Case | Engine | Allocation calls | Tracked heap peak |
| --- | --- | ---: | ---: |
| obj-repeated-16384 | baozi | 33,330 | 12.51M |
| obj-repeated-16384 | assimp | 82,154 | 19.91M |
| obj-grid-128 | baozi | 99,375 | 39.18M |
| obj-grid-128 | assimp | 360,697 | 28.69M |
| gltf-shared-32 | baozi | 1,067 | 420.84K |
| gltf-shared-32 | assimp | 845 | 174.36K |
| stl-repeated-1024 | baozi | 441 | 456.68K |
| stl-repeated-1024 | assimp | 1,186 | 317.09K |
| ply-f64-neighbors | baozi | 471 | 136.09K |
| ply-f64-neighbors | assimp | 187 | 2.18M |
| ply-points-1024 | baozi | 432 | 188.51K |
| ply-points-1024 | assimp | 4,281 | 2.23M |

Instrumented runtime and instrumented RSS are excluded from ordinary speed/RSS comparisons.
The terminal unfreed allocations include process/runtime state; these totals alone establish no Baozi or Assimp library leak.
Debug-symbol binaries and raw traces are preserved so later attribution does not depend on a subsequently overwritten build path.

## Baseline Review and Preservation

An independent native reviewer recomputed the lane statistics and paired ratios from every retained sample, checked serial AB/BA ordering and raw GNU time values, and verified corpus/build/binary identities.
It found no P0–P2 issue. Its P3 on undocumented upward rounding is resolved explicitly in the budget interpretation above.
The reviewed data and budget values are unchanged; the fixed-source private receipt is `.git/baozi-review/pre-release-evolution-execution/b0-review.md`.

`target/assimp-comparison/B0-evidence.zip` preserves 15,344 entries (46,817,668 bytes), including ordinary raw observations/builds, the pre-refactor source archive, corpus, calibration, debug-symbol builds and heap traces.
Archive SHA-256 is `cac541c2f8a70a261120e61116ad01104d5f4506a4a75628e286e81315c3ec4a`; its file-digest manifest SHA-256 is `6c0eca8cfc68ef2ea5a4d8291a58d293f9b8f22dd7be43ec17fe5812ce072d02`.
These ignored local artifacts are retained evidence, not a remotely published archive. The archive records the original B0 protocol and frozen budget; later explanatory documentation does not rewrite those observations.

## Quaternion Policy Adjudication

The [source/validator investigation](../research/2026-11-02-quaternion-compatibility.md) retains SimpleSkin as a documented Baozi policy rejection, with unchanged fixture bytes and unchanged canonical tolerance.
The pinned official validator reports zero errors/warnings/infos and one unrelated hint. Eight independently decoded keys exceed Baozi's narrower squared-length threshold.
The capability manifest records both outcomes, rather than treating the official asset as generally invalid.
No production arithmetic, authored/effective representation or normalization policy changed in this slice.

Characterization adds adjacent-F32 boundary vectors to math, authored transforms and public glTF import/snapshot reads, including negated rotations, signed zero and a value that distinguishes length from squared length.
An owned-evaluator endpoint check proves its temporary normalization without rewriting a tolerated key.
Existing cubic-derivative, retained-validation, reflected-conversion and upstream-rejection checks remain in use.

| Check | Actual result | Evidence identity |
| --- | --- | --- |
| Core boundary and existing semantic cases | 6/6 pass | nextest `b1018d63-771d-4df7-9a00-dada7f72d1ee` |
| glTF public animation/snapshot cases | 3/3 pass | nextest `01ef4ac8-a408-4b5b-8d70-95a921934d91` |
| Detached evaluation and unchanged rejection | 5/5 pass | nextest `2d183b30-2c69-4e65-b689-2cba2b053e26` |
| Static checks | core/glTF and asset consumer strict all-target/all-feature Clippy; both format checks pass | Rust 1.95, Windows x86_64 MSVC, serial shared target |

Independent review recomputed the source/boundary arithmetic and checked the common admission predicate, public retention, conversion and temporary-evaluation distinction; it found no actionable issue.
Its fixed-source receipt is `.git/baozi-review/pre-release-evolution-execution/quaternion-review.md`.

## Remaining Verification

Timing/RSS B0, the separate heap observations and the regression policy are now recorded above. Production semantics remained unchanged throughout these campaigns.
Subsequent material/light/render/device/quantization changes, canonical v1 cutover, B1 comparisons and final package/portable qualification remain ahead.
Historical qualification stays bound to the source in its original record.
No registry publication/yank, push/PR or native target promotion has occurred in this work.
