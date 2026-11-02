# Linux B1, Heap Observations and Budget Dispositions

The corrected ordinary B1 campaign completes **110/110 lanes**, 2,200 retained pairs and
330 warmup pairs, with all 5,060 child observations successful. Its ordinary Baozi worker is
source `79158120f4aa975e85a5a6ba2fa39afd85c4cdd7`, with runtime implementation unchanged
from `52446ec8511b7dfba2e5352ce6941558d85e0ae2`. The collector at
`088d7176d5982d1ae9e5045c0250dc6f608f66c1` additionally binds supplied build metadata to
the executable bytes before measurement; it changes no worker task, timer or statistical rule.
This report is not final U14 or registry qualification.

## Identity Correction and Ordinary Method

The first `B1/` run and its four elapsed repeats are **disqualified as a matched ordinary
comparison**. Mutable external Assimp build paths had been overwritten by symbolized heap
builds; their actual hashes differed from the supplied ordinary B0 manifests. Every original
sample and the initial 97-metric budget result remain retained. They are not overwritten,
relabelled as green, or used as the qualified B1 results below.

The correction copies the archived ordinary controls to new immutable ext4 paths, checks their
hashes against both B0 and the build metadata, then reruns the complete unchanged matrix into
`B1-ordinary/`. Both control hashes match ordinary B0 exactly:

| Worker | SHA-256 |
| --- | --- |
| Baozi ordinary B1 | `7994f1eb0f1d32f46d0b2b9f53707dae74861a90e5f872bcd90108d159f707e6` |
| Assimp ordinary F32 | `c08e25f8d0745d1e4981f3117d6d4f66eba2bcda9e165c1ff8bf6642ce1fdb50` |
| Assimp ordinary F64 | `7454bc3f2974f6885f1c23e6c1376f52d8a62df63c31d4c2190a6ade157f35a7` |

Rust 1.95.0 release optimization, generic x86_64 GNU, System allocation and no interception
match the ordinary build policy. Assimp remains the pinned 6.0.5 release with the recorded
precision/Clang/link/allocator policy. All three prelaunch manifest identities are verified.
Workers execute on ext4, matching B0's filesystem placement; inputs remain the identical
hashed corpus. File/memory IO, fields, normal policies, scalar widths and endpoints are
unchanged. Three warmup and twenty retained alternating AB/BA pairs run per lane. Builds,
downloads and profilers stay outside measured workers.

The metadata guard has nineteen passing CLI tests and one Windows platform skip, including
stale and identity-free manifests rejected before worker launch. Independent source review
finds no actionable issue. This prevents recurrence without interpreting compiler flags or
creating a new benchmark framework. Unattested optional metadata remains explicitly labelled.

## Qualified Memory-Input Pipelines

Times are medians in milliseconds; memory is whole-child peak RSS in MiB. All file-input and
separate endpoint statistics, IQRs and raw samples remain in the full report. No stage clocks
are added/subtracted as a pipeline decomposition and no Windows memory metric is used here.

| Case | Baozi B0 ms | Baozi B1 ms | Assimp B1 ms | Baozi B1 RSS | Assimp B1 RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Repeated OBJ 1 | 0.791 | 0.777 | 4.280 | 10.066 | 20.818 |
| Repeated OBJ 1,024 | 20.470 | 6.426 | 4.783 | 10.637 | 21.070 |
| Repeated OBJ 16,384 | 345.526 | 88.782 | 8.817 | 19.031 | 24.332 |
| Grid OBJ 2 | 0.810 | 0.706 | 4.684 | 9.420 | 20.822 |
| Grid OBJ 32 | 33.880 | 11.969 | 5.480 | 10.268 | 21.518 |
| Grid OBJ 128 | 544.727 | 189.241 | 20.269 | 34.625 | 33.916 |
| Shared glTF 1 | 1.613 | 1.161 | 0.154 | 10.035 | 5.803 |
| Shared glTF 8 | 3.165 | 1.853 | 0.177 | 9.979 | 5.807 |
| Shared glTF 32 | 9.616 | 4.352 | 0.191 | 10.250 | 5.756 |
| Binary STL repeated 1 | 0.604 | 0.599 | 0.068 | 9.203 | 5.027 |
| Binary STL repeated 1,024 | 6.191 | 3.930 | 0.177 | 9.469 | 5.607 |
| PLY triangle 1 | 0.683 | 0.634 | 0.700 | 9.170 | 6.568 |
| F64 PLY neighbors | 0.691 | 0.677 | 0.752 | 9.285 | 6.268 |
| PLY points 1,024 | 3.454 | 1.196 | 0.781 | 8.059 | 6.760 |

The large repeated/grid pipelines improve about 74.3%/65.3% against immutable B0, retaining
the finite semantic task. Baozi still has a substantial Assimp gap in several tasks. The
table does not imply arbitrary-model parity or a universal speed/memory winner.

## Frozen Budget Result and Explicit Costs

The original frozen policy is unchanged: B0 median plus max(three B0 IQRs, 1 microsecond for
time or 4 KiB for RSS), with every stored limit retained. Corrected B1 has **63 metric breaches:
57 whole-child RSS, six parent elapsed, zero stage time**. The derived original budget JSON
keeps `requires_attribution`; this report does not claim all limits were met.

All six elapsed breaches receive the same-lane twenty-pair/three-warmup repeat with the correct
immutable ordinary controls. Repeated medians are within their original limits; high original
IQRs and the complete original samples remain retained. This supports an explicit process
startup/scheduling-noise disposition rather than a stage CPU regression. Old B0 `noisy=false`
flags are not rewritten.

| Lane | Original B1 parent ms | Repeat parent ms | Frozen limit ms |
| --- | ---: | ---: | ---: |
| Shared glTF 8, extract/file | 31.454 | 23.522 | 27.888 |
| Shared glTF 8, import/memory | 40.550 | 23.166 | 27.561 |
| Shared glTF 8, prepare/file | 25.922 | 24.225 | 24.994 |
| Grid OBJ 2, prepare/file | 35.298 | 24.865 | 26.920 |
| F64 PLY neighbors, pipeline/file | 27.626 | 22.872 | 26.300 |
| PLY points 1,024, import/memory | 39.459 | 23.416 | 29.532 |

The 57 whole-child RSS breaches are accepted as the observed memory cost of retaining the
current typed/context/control and semantic closure; every breach remains above its unchanged
limit. Fixed-image growth is a supported contributing explanation from the bounded small-worker
exit maps, not a proven exclusive or identical cause of all 57 ordinary peaks. Across all lanes,
Baozi's median RSS delta is +532 KiB; Assimp's is +94 KiB. Large
grid pipeline RSS falls from 39.867 to 34.625 MiB; repeated pipeline falls from 19.586 to
19.031 MiB. The smaller-input changes are consistent with a resident-image contribution.

A separate GDB 17.1 diagnostic observes two ordinary B0/B1 pairs in opposite order, with
ASLR enabled, at `exit_group` entry after cleanup for repeated-1 OBJ import/file. Current
mapping differences are +416/+536 KiB, of which file pages are +404/+520 KiB and anonymous
pages +12/+16 KiB. Heap mapping adds 8 KiB in each pair; stack changes by -4/0 KiB. Static
loadable file content grows by 812.75 KiB, which was only a locating clue, not RSS evidence.
GDB timings are unused. The installed debugger adds seven packages with zero upgrades;
the retained Baozi-linked libc, libgcc, libm and loader hashes match before/after. Recorded
libc, libgcc, libstdc++ and Python package versions also match.

This establishes file-page dominance at those observed small-worker exit points. It does
**not** decompose every lane's lifetime peak, attribute every byte to one feature, subtract
idle RSS or prove that all 57 peaks have identical causes. The disposition explicitly accepts
the measured whole-worker footprint while retaining required correctness/control semantics;
further image-size reduction remains future measured work. Independent disposition review
accepts this bounded cost decision with the corrected causal and library-identity wording.

## Separate Heap Observations

Heaptrack 1.5.0 performs exactly twelve fresh whole-worker memory pipeline observations:
six cases times Baozi/Assimp, with the separate F64 control for F64 PLY. All succeed and match
the frozen analytic facts. The symbolized Baozi worker SHA-256 is
`ac9e45decf69ad865b07ebdf72ebd01f114a36ff56b85f85163f4e0866365fa3`; its build policy/source
matches ordinary B1 except debug information. Assimp uses the retained B0 symbolized hashes
`aee51d34...` and `9f44667e...`, appropriate for this separate heap campaign. The already
verified Rust System/malloc/realloc/alignment/thread/C++ interception calibration remains its
scope. The tool and runtime identities are retained; no uninstrumented heap claim is made.

K/M below are heaptrack's rounded decimal units, not MiB. Counts are whole-worker allocation
function calls, including setup, preload, output, checking and serialization.

| Case | Baozi B0 / B1 heap peak | Assimp heap peak | Baozi B0 / B1 calls | Assimp calls |
| --- | ---: | ---: | ---: | ---: |
| Repeated OBJ 16,384 | 12.51M / 12.18M | 19.91M | 33,330 / 33,389 | 82,154 |
| Grid OBJ 128 | 39.18M / 28.83M | 28.69M | 99,375 / 99,437 | 360,697 |
| Shared glTF 32 | 420.84K / 421.42K | 174.36K | 1,067 / 1,128 | 845 |
| Binary STL repeated 1,024 | 456.68K / 461.44K | 317.09K | 441 / 500 | 1,186 |
| F64 PLY neighbors | 136.09K / 140.86K | 2.18M | 471 / 530 | 187 |
| PLY points 1,024 | 188.51K / 193.28K | 2.23M | 432 / 482 | 4,281 |

The grid reduction is substantial; the other heap peaks can rise slightly. Current Baozi
exit-live memory is reported as 544 B and Assimp as 4.10K by this tool. These process/tool
results are retained rather than relabelled zero or used as a memory-safety proof. Heaptrack
RSS includes profiler overhead and is not ordinary B1 RSS. Profiled runtimes supply no speed
ratios. No stage heap metric is invented.

## Final Instruction Check and Stopping Decision

Six fresh whole-worker Callgrind 3.26.0 `Ir` observations match semantic facts and reconcile
all flat exclusive rows. Grid/repeated totals move from 6,828,426,425/3,577,591,805 before the
P5 candidates to 3,318,117,260/1,492,581,301. The other four totals move by about 0.6-1.5%.
These are whole-worker instruction counts, not CPU-time percentages or isolated algorithm
costs. Native ordinary results, rather than reduced Ir alone, qualify the retained changes.

P5 retains the demonstrated remap/order/numeric simplifications and stops this finite tranche.
Borrowed text, flat syntax staging, payload adoption and typed reader views remain candidates
for separately measured follow-up. The remaining shared work, public-contract and scene costs
do not justify another speculative decoder or ownership rewrite before final qualification.

## Evidence and Remaining Work

- Qualified ordinary report: `target/assimp-comparison/B1-ordinary/report.json`, SHA-256
  `411e16420b46fd8f91793ccd70280fa1c8e75298860273eab993f43fc09510af`.
- Original immutable limits and actual breaches: `docs/testing/assimp-b0-budget.json` and
  `target/assimp-comparison/B1-ordinary-budget.json`.
- Explicit per-breach disposition: `target/review/b1-final-budget-disposition.json`;
  repeated observations in `B1-ordinary-repeat-*`.
- Identity incident and preserved invalid results: `target/review/b1-control-identity-incident.json`,
  original `B1/` and `B1-repeat-*`; they are not qualified controls.
- Separate heap traces/report: `B1-heap/`; summarized in `target/review/b1-heap-summary.json`.
- GDB maps/runtime identity: `target/review/rss-attribution/`.
- Final raw Ir profiles: `target/review/final-attribution/`, with reconciled summary.

Independent ordinary B1 review finds no actionable data/identity/statistical issue and confirms
all 330 frozen metric comparisons. Independent disposition review verifies every breached row,
all six repeat statistics and both mapping pairs; its two wording findings are corrected.
Final integrated U14 remains separate. No registry publication, yank, push/PR or native promotion
follows from these results.

`target/review/final-linux-evidence.json` inventories 32,092 retained files, including the
qualified observations, failed-identity runs, repeats, raw heap/Ir/mapping evidence and reviews.
The matching `target/review/final-linux-evidence.zip` SHA-256 is
`db19cc8b3549826b8072b2f8559c8f1a59038a309349ad51993ca9c045c6ed3b`.
