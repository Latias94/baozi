# Direct WorkState: Native Matrix, Heap and Executable Placement

The cumulative startup/publication and direct numeric-state changes at `56eecd9`
do not establish performance convergence. All four original Linux campaigns
complete, but sixteen of twenty-eight candidate pipeline lanes exceed the 3x
same-campaign Assimp target in each candidate campaign. Every candidate
whole-child RSS ratio is below 3x. The two controls overlap Windows build/test
activity, so their differences from candidates are descriptive, not a clean
source-change effect. Current-source Windows ordinary qualification remains open.
Release and performance acceptance remain HOLD.

A separate identical-binary placement experiment identifies a substantial
environment cost: moving the unchanged debug0 executables from `/mnt/f` to the
Linux filesystem reduces the three small Baozi pipeline medians from 39–62 ms
to 0.58–1.05 ms, with whole-worker median major faults falling from 204–266 to
zero. Assimp also improves. This is evidence to correct the next Linux
measurement environment, not a source optimization, replacement of the original
matrix, endpoint-fault attribution or release acceptance.

The source and correctness qualification are recorded in
[direct numeric WorkState](2026-11-05-direct-work-state.md). This performance
comparison includes both that change and
[startup/publication borrowing](2026-11-05-startup-and-publication-share-borrowing.md)
against `05b7d9b`; individual-step causation is not available.

## Immutable Native Workers

| Identity | Control | Candidate |
| --- | --- | --- |
| Runtime revision | `05b7d9b8dd2bf4158319e202ba68963cfb64e01d` | `56eecd985693cba1826326169e6c673a7b36e3ff` |
| Source archive SHA-256 | `4c1170f3e41a09035ac5fdc5220b95b33cda7df6fc86113a2eb8630d1880f30e` | `ce19f777adf65a397cb1dcb9d29ebfd010af993a0c8287110b4dcb887ee1889e` |
| GNU ordinary executable SHA-256 | `e6ba1da06832a87810607c209e668824b14d7f3cb24cf86d59c01b8079d76d87` | `db4f9d8e1b465d852fcb2ef09eb4104b4abd978c5c4505b7fbbc00a14df6d6da` |
| Windows executable SHA-256 | `af5c906a9c503c0c7f97f06f8c1bc72397f5c7876756bcca5c8fe9ee9f2c3eb2` | `b3e0c420724d73cca30ebd1672c5e2be4ca3358b648c959166479d62031c872f` |

Both GNU ordinary workers are rebuilt serially from committed archives with
Rust 1.95.0, opt3/debug0, generic x86-64, System allocation and no allocation
metrics. Their 1,461/1,464 archive-file before/after maps are unchanged. Import
is explicitly non-fresh in both Cargo artifact records. The existing Linux
target cache is reused with one build job. Both workers pass the original
110-lane zero-warmup/one-pair functional preflight; these observations are
excluded from ordinary statistics.

Ordinary Assimp uses the immutable B0 F32/F64 debug0 copies, with SHA-256
`c08e25f8d0745d1e4981f3117d6d4f66eba2bcda9e165c1ff8bf6642ce1fdb50`
and `7454bc3f2974f6885f1c23e6c1376f52d8a62df63c31d4c2190a6ade157f35a7`.
They retain Assimp 6.0.5, Clang 21, generic opt3 CMake Release policy and their
original manifests. Rebuilt symbolized Assimp workers are used only for heap
attribution, never substituted into the ordinary matrix.

GNU ordinary build records are under
`target/assimp-comparison/convergence-linux-direct-work-state-{control,candidate}-ordinary-build/`.
The candidate Windows build and its original functional preflight also succeed.
Its prospective ordinary sequence reaches the 180-second named-tool start limit
before C0 starts. No Windows ordinary child or candidate timing/RSS sample is
created. Preserve `target/review/convergence/direct-work-state-C0-not-started-1.json`;
functional observations do not fill that gap.

## Four Original Linux Campaigns

The prospective decision fixes C0, N0, N1, C1 before collection. Every campaign
retains the original fourteen cases and 110 lanes, three warmup pairs and twenty
AB/BA pairs per lane. Fields, normals, precision, work budgets, clocks, output
lifetimes and failure semantics are unchanged. Each campaign completes 5,060
native children: 660 warmup and 4,400 retained observations. Its two additional
commands are source metadata checks. There are no failed commands or missing
ordinary lanes. Actual Windows wrapper exec50398 terminates with exit zero.

| Campaign | Role | Collector seconds | Linux tool overlap / snapshots | Windows tool overlap / snapshots |
| --- | --- | ---: | ---: | ---: |
| C0 | Control | 519.01 | 0 / 260 | 142 / 256 |
| N0 | Candidate | 526.69 | 0 / 264 | 0 / 259 |
| N1 | Candidate | 519.52 | 0 / 260 | 0 / 257 |
| C1 | Control | 573.36 | 0 / 287 | 94 / 282 |

Windows observations are correlated to each campaign's actual start/end window,
excluding the preceding builds and preflights. The overlapping names include
Cargo, rustc, linker and nextest. Linux's twenty-second named-tool start gate
does not establish Windows or global host quiet. Sampling named processes at
approximately two-second intervals also cannot exclude other load sources.
The two control overlaps preclude interpreting candidate/control differences as
an uncontaminated effect of the Rust change.

Each Baozi/Assimp ratio below is the median of the twenty individual pair ratios.
The even-sample median of reciprocals is recomputed rather than inverted after
taking a median. No lane, tail or less favorable repeat is removed or pooled.

| Candidate metric | N0 | N1 |
| --- | ---: | ---: |
| Native endpoint/pipeline lanes above 3x | 81 / 110 | 80 / 110 |
| Native endpoint/pipeline lanes at least 10x | 57 / 110 | 60 / 110 |
| Complete pipeline lanes above 3x | 16 / 28 | 16 / 28 |
| Maximum complete pipeline ratio | 15.136x | 15.141x |
| Maximum endpoint/pipeline ratio | 10,708.844x | 10,018.591x |
| Whole-child elapsed lanes above 3x | 10 / 110 | 14 / 110 |
| Maximum whole-child elapsed ratio | 3.810x | 3.790x |
| Whole-child RSS lanes above 3x | 0 / 110 | 0 / 110 |
| Maximum whole-child RSS ratio | 1.974x | 1.989x |

The extreme endpoint ratio is a short preparation task, not the full import
pipeline: F64 PLY preparation is approximately 4.7–5.1 ms for Baozi against an
Assimp median around 473–505 ns. Such denominators require absolute times and
environment attribution; they are not evidence of a universal 10,000x parser
difference. They remain recorded failures of the endpoint target in this matrix.

Representative complete-pipeline observations are below. The full 110-row table,
all four campaigns, per-engine medians/IQR/min/max and all retained pair ratios
remain available in the original reports and main-derived summary.

| Case / mode | Baozi N0 / N1 ms | Baozi/Assimp N0 / N1 | Whole-child RSS ratio N0 / N1 |
| --- | ---: | ---: | ---: |
| Repeated OBJ 1 / memory | 45.073 / 55.455 | 5.75x / 6.66x | 0.49x / 0.49x |
| Repeated OBJ 16384 / memory | 89.607 / 88.482 | 7.78x / 7.43x | 0.77x / 0.77x |
| Grid OBJ 128 / file | 219.988 / 228.656 | 4.94x / 4.77x | 1.02x / 1.02x |
| Grid OBJ 128 / memory | 194.830 / 198.748 | 8.25x / 8.15x | 1.02x / 1.02x |
| Shared glTF 1 / memory | 48.763 / 45.262 | 9.88x / 9.76x | 1.92x / 1.94x |
| Shared glTF 32 / memory | 48.640 / 52.384 | 9.84x / 10.92x | 1.96x / 1.99x |
| Repeated STL 1 / memory | 34.891 / 37.537 | 11.86x / 12.61x | 1.83x / 1.80x |
| Repeated STL 1024 / memory | 34.315 / 42.343 | 12.03x / 13.82x | 1.75x / 1.71x |
| F64 PLY neighbors / memory | 39.724 / 37.735 | 15.14x / 15.14x | 1.44x / 1.45x |
| PLY points 1024 / memory | 32.920 / 32.537 | 7.81x / 8.08x | 1.25x / 1.22x |

Candidate/control median native stage cost increases in 49 lanes for N0/C0 and
41 for N1/C1; twenty-six lanes, including eight pipelines, have a positive sign
in both contrasts. Examples include grid-128/file pipeline, +1.91%/+0.89%, and
F64-neighbors/memory pipeline, +3.56%/+1.00%. Repeated-16384/memory pipeline is
-8.94%/-3.41%. These are all retained descriptive contrasts under the stated
control load, not significant clean regressions or a performance keep.

Raw roots are
`target/assimp-comparison/convergence-linux-direct-work-state-{control,candidate,candidate-confirm,control-confirm}/`.
Receipts and the wrapper are `target/review/convergence/direct-work-state-linux-*-1.json`.
The complete table/summary are
`direct-work-state-linux-four-campaign-main-table-1.csv` and
`direct-work-state-linux-four-campaign-main-summary-1.json` in that evidence
directory. `direct-work-state-linux-load-summary-1.json` retains window-specific
load observations and their scope.

## Separate Whole-Worker Instructions

Actual exec28337 builds the exact candidate archive with opt3/debug1 and retains
six successful Callgrind profiles. Controls are the retained `05b7d9b` profiles
from the previous checkpoint, not fresh simultaneous controls. Neither symbol
worker supplies ordinary timing. Raw summary, totals and stderr Collected agree.

| Case | Control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| Shared glTF 1 | 5,928,267 | 5,920,175 | -0.1365% |
| Shared glTF 32 | 29,687,737 | 29,677,439 | -0.0347% |
| F64 PLY neighbors | 2,494,023 | 2,484,828 | -0.3687% |
| Repeated STL 1024 | 25,416,724 | 25,427,021 | +0.0405% |
| Repeated OBJ 16384 | 819,153,676 | 819,357,617 | +0.0249% |
| Grid OBJ 128 | 2,937,988,768 | 2,938,588,473 | +0.0204% |

Grid retains its Valgrind `brk segment overflow` warning. Whole-worker Ir includes
setup, validation, serialization and teardown; inclusive annotation parents are
not additive with exclusive children. These small mixed differences do not
establish native convergence or explain the tens-of-ms ordinary small-task cost.
Profiles and original logs remain under
`target/review/convergence/direct-work-state-attribution/`.

## Separate Whole-Worker Heap

Actual exec18267 completes eighteen fresh heaptrack observations, six cases each
for `05b7d9b`, `56eecd9` and retained symbolized Assimp. All tasks retain the
original pipeline/memory semantics. Both Baozi revisions have identical reported
allocation calls and rounded decimal heap peaks in every case.

| Case | Both Baozi calls | Assimp calls | Both Baozi temporary A/S | Assimp temporary A/S | Both Baozi peak | Assimp peak |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Repeated OBJ 16384 | 33,389 | 82,154 | 10/9 | 9/8 | 12.18M | 19.91M |
| Grid OBJ 128 | 99,425 | 360,697 | 33,262/33,261 | 10/9 | 28.82M | 28.69M |
| Shared glTF 32 | 1,116 | 845 | 14/13 | 16/15 | 416.52K | 174.36K |
| Repeated STL 1024 | 488 | 1,186 | 8/7 | 10/9 | 456.54K | 317.09K |
| F64 PLY neighbors | 518 | 187 | 10/9 | 17/14 | 135.96K | 2.18M |
| PLY points 1024 | 482 | 4,281 | 9/8 | 1,041/17 | 188.38K | 2.23M |

`A/S` preserves analysis/stderr temporary counts, including Assimp points'
1,041/17 discrepancy. Heap K/M units are rounded decimal reports, not byte-exact
peak equivalence. Exit-live reports remain 544B for Baozi and 4.10K for Assimp;
whole-worker exit observations alone do not identify a product leak. Tracked
calls include setup/preload/validation/serialization/teardown. Prior interception
calibration covers documented allocator paths, not every allocator. Instrumented
elapsed time and overhead RSS are excluded from ordinary conclusions.

The eighteen traces and original analysis/stdout/stderr logs remain in
`target/assimp-comparison/convergence-linux-direct-work-state-heap/`.
The main extraction is `target/review/convergence/direct-work-state-instrumented-main-summary-1.json`.

## Identical-Binary Filesystem Diagnostic

After both ordinary and instrumented runs terminate, actual exec35025 runs a
bounded placement diagnostic. The debug0 candidate and B0 Assimp bytes are copied
unchanged to the Linux filesystem; source/build identity and before/after binary
hashes remain equal. Filesystem stat reports `v9fs` versus `ext2/ext3` for all
three executable identities. Corpus paths, memory preload, pipeline clocks, selected
fields, normals, budgets and output lifetimes are unchanged. The source is not
rebuilt and no work is moved outside the clock.

For each of three cases and each engine, three warmup and twenty retained
alternating placement pairs produce 276 successful worker observations. GNU time
also records whole-worker CPU time, major/minor faults and RSS. The native
pipeline median and whole-worker median major faults are:

| Case | Engine | `/mnt/f` ms | Linux filesystem ms | `/mnt/f` major faults | Linux filesystem major faults |
| --- | --- | ---: | ---: | ---: | ---: |
| Shared glTF 1 | Baozi | 62.459 | 1.0515 | 266 | 0 |
| Shared glTF 1 | Assimp | 5.858 | 0.2177 | 28 | 0 |
| Repeated STL 1 | Baozi | 38.862 | 0.5808 | 204 | 0 |
| Repeated STL 1 | Assimp | 3.138 | 0.0891 | 17 | 0 |
| F64 PLY neighbors | Baozi | 41.823 | 0.6750 | 215 | 0 |
| F64 PLY neighbors | Assimp | 3.143 | 0.8653 | 13 | 0 |

These are per-engine placement observations, not a replacement same-campaign
Baozi/Assimp acceptance matrix. The strong placement effect and associated
whole-worker fault change support the inference that executable demand paging
contributes materially to the original small-task elapsed cost. Fault counters
cover the whole child and do not locate every fault inside the pipeline clock or
exclude other filesystem/scheduling effects. This diagnostic has no full-window
Windows load watcher and cannot prove global host quiet. Its six small-task
results do not extrapolate to every endpoint or large model.

The same placement diagnostic does not demonstrate memory reduction. Baozi's
whole-worker RSS medians increase from 11,098 to 12,286 KiB for shared glTF 1,
8,912 to 10,698 KiB for repeated STL 1, and 9,184 to 10,754 KiB for F64 PLY.
Keep these less favorable residency observations alongside the timing result;
they are neither endpoint heap peaks nor source-refactor memory changes.

Keep the original `/mnt/f` matrix, loaded controls and all failures visible.
The next complete Linux qualification should prospectively fix native Linux
filesystem placement for both engines and the unchanged corpus, retaining the
original fourteen cases, 110 lanes, clocks and all samples. Changing program
placement is an environmental correction, not a Rust performance improvement.
The diagnostic and original 276 stdout/stderr/time logs remain under
`target/review/convergence/direct-work-state-locality/`.

## Independent Review and Next Cut

Independent ordinary review reconstructs all 20,240 native children and checks
60,736 original files, all 110 lanes in each campaign, paired statistics,
geometry/output/work/width facts, GNU archive/build/binary identities and
window-specific load. Its five initial strict-check flags have evidence-backed
nonblocking dispositions. All four source-status stderr logs contain the same
64-byte `untracked cache is disabled on this system or location` Git warning;
exit codes are zero and stdout enumerates the five original notes. B0's CMake
file genuinely differs from the later source by the default-off
`BAOZI_ASSIMP_BUILD_CONTROL` diagnostic target, rather than only newlines.
Measured `src/main.cpp` bytes are unchanged and both ordinary workers retain
their original B0 hashes/build identities. Original warnings and differences
remain recorded; they are not replaced by assertions of empty stderr or equal
CMake bytes. No blocking native-data or identity mismatch remains.

Independent instrumented review reconciles twelve raw instruction profiles,
twenty-four annotations, eighteen fresh traces and fifty-four heap logs, with
both source/archive associations. It independently replays heap events and
confirms equal Baozi peaks as well as the reported rounded values, retaining all
temporary-count differences and both grid warnings. No blocking mismatch is
found. These reviewers inspect completed artifacts; main observes the actual
process terminal states. Parent monotonic elapsed readings cannot be independently
recovered from stdout; timestamp/containment/summary consistency is the bounded
check, not a second elapsed clock.

Their private receipts are
`.git/baozi-review/performance-convergence/direct-work-state-linux-ordinary-raw-review-1.md`
and `direct-work-state-attribution-and-heap-review-1.md` in the same directory.
Independent placement review checks six executable identities/filesystem types,
all 276 native observations and 828 original logs, every placement pair and
original task/clock/output fact. It reproduces all stage/fault summaries without
a blocking discrepancy and explicitly retains the unfavorable RSS observations.
Its receipt is `direct-work-state-locality-review-1.md` in that directory.
All three reviewers are complete and closed; none reruns the measurements or
supplies a release verdict.

The source cleanup remains a prerequisite for concrete lexical backing, not a
demonstrated speed or allocation win. The actual direct runtime now has six
dimensions and three phases; its dynamic LedgerLimits/phase buffers and eager
storage Arc still exist. The older five-dimension design must be refreshed
against current source. Fixed direct arrays, transferred dynamic import Vecs,
the same numeric/storage kernels, shell-before-state shared promotion, honest
administrative payload/capacity accounting, owned terminal facts,
cleanup-before-finish and indexed selection remain under the
[exclusive invocation plan](../plans/2026-11-03-1120-refactor-exclusive-invocation-plan.md).

Correct the next Linux measurement placement before using the ordinary matrix
to choose another parser/normal algorithm rewrite. Retain the independent CPU
and allocation evidence and measure the actual remaining fixed startup/storage
cost at its owner. Windows native qualification and every original release gate
remain open. No publication, push, PR, yanking or native target tuning occurs.
