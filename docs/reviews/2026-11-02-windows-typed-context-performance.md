# Windows Typed-Context Import Measurement

Baozi implementation `89f4e0bba832a43be55ce60159cc3b1790040820` resolves the grid-import
regression observed at the P3 foundation checkpoint. Both substantial import lanes and both
complete pipelines improve against production control
`4455d03db190c6bef05ff5a1de024d2aac461465`. The integrated P3 performance gate is satisfied
for these six lanes; this is not full B1 or final qualification.

The unchanged ordinary collector ran memory-input repeated-16384 and grid-128 OBJ through
import, preparation and complete pipeline. Each lane retained twenty fresh-process AB/BA
pairs after three warmup pairs. All 120 retained pairs and 18 warmup pairs (36 warmup child
processes) succeeded, with the
unchanged precision, fields, input, normal policy and analytic semantic checks. No sample was
discarded. Builds and profiling were outside collection.

Rust 1.95.0 Cargo Release uses the ordinary System allocator, generic x64 MSVC, dynamic CRT,
no allocation interception and no diagnostic instrumentation. The immutable Assimp 6.0.5
F32/F64 binaries are the same files used by the earlier control series. The committed source,
worker binary, build metadata, every raw observation and collector invocation are retained.

## Stage Time

Times are median / IQR, milliseconds. Percent change compares Baozi with its production control.

| Case / endpoint | Control | P3 foundation | Integrated context | Change | Current Assimp |
| --- | ---: | ---: | ---: | ---: | ---: |
| repeated import | 100.281 / 0.716 | 101.199 / 1.123 | 42.075 / 0.877 | -58.0% | 7.942 / 0.374 |
| repeated prepare | 190.116 / 1.779 | 130.041 / 1.054 | 135.210 / 2.469 | -28.9% | 1.144 / 0.114 |
| repeated pipeline | 296.912 / 1.756 | 235.854 / 2.463 | 182.428 / 2.636 | -38.6% | 10.209 / 0.466 |
| grid import | 379.162 / 2.651 | 542.771 / 2.679 | 203.288 / 3.004 | -46.4% | 24.921 / 1.020 |
| grid prepare | 0.139 / 0.005 | 0.143 / 0.010 | 0.146 / 0.016 | descriptive | 0.042 / 0.014 |
| grid pipeline | 393.785 / 2.132 | 557.565 / 5.320 | 221.903 / 3.714 | -43.6% | 26.964 / 1.116 |

The larger import reductions exceed the observed dispersion. The substantial Assimp medians
are about 4–9% higher than the production-control series, and repeated preparation is about
9% higher. This host/run movement cannot explain Baozi's 46–58% import reduction; it does
limit fine-grained comparisons between separate series. The tiny grid preparation task is
descriptive and supplies no optimization or regression claim. Relative to P3, Baozi repeated
preparation rises about 4%, alongside about 12% Assimp movement; do not attribute that small
cross-run difference to the typed-context implementation.

The implementation removes whole-session projection and repeated parser group binding while
preserving immediate physical work completion. The combined change includes typed contexts,
canonical caller binding and real polygon action boundaries. These measurements establish a
retained integrated improvement, not an isolated vcall cost or an additive CPU breakdown.
There is still a large Assimp gap: these ordinary import tasks take roughly five and eight
times its time, and repeated preparation about 118 times. P5 must attribute the remaining
text, source-authoring, canonical construction and reading costs before another rewrite.

## Memory Scope

Whole-child peak working set / peak private commit medians, MiB, for the complete pipeline:

| Case | Baozi control | Baozi integrated | Assimp control | Assimp current |
| --- | ---: | ---: | ---: | ---: |
| repeated | 17.543 / 13.465 | 17.420 / 13.229 | 25.258 / 20.855 | 25.250 / 20.855 |
| grid | 37.877 / 39.328 | 37.514 / 38.791 | 35.045 / 30.990 | 35.051 / 30.998 |

These include setup, input, verification and serialization. They are not isolated stage peaks,
heap allocations or Linux RSS. Small differences do not establish a parser allocation cause.
Linux timing/RSS B1 and separate heap observations remain required against immutable B0.

## Evidence and Remaining Work

- Complete report: `target/assimp-comparison/Windows-p4-context/report.json`, SHA-256
  `b5f6466ac4107eb5b9f6ca45808793cf563b247d13b637096cf5e03d9cb85331`.
- Source/build/binary: `target/assimp-comparison/Windows-p4-context-build/`; worker SHA-256
  `6d65655445108f3edcd7da0a418b52dbf675287be42b9cd9145a9735f7de4701` and exact source tar
  `85208964e94171e78575367c52e5af2fd0954ba705393c4b6921f7f77507c7ce`.
- Calculation: `target/review/p4-windows-comparison.json`; ordinary collector argv and log
  retained beside it.
- Evidence archive: `target/review/p4-windows-evidence.zip`, 576 files, SHA-256
  `6b5e99ae05386e40c8375035b190b619a1efa8540cfc137fab89f7a63860da45`.
- Original [full Windows control](2026-11-02-windows-native-performance.md) and
  [P3 regression](2026-11-02-windows-bound-execution-performance.md) retain their original
  samples, statistics and dispositions.

Independent evidence review checked all 276 raw worker observations, recomputed the table
and matched the 1,337 source-archive files to the implementation commit. Its one count-label
finding is corrected above: six lanes times three warmups is 18 pairs/36 child processes.
The earlier P3 receipt used the same mistaken pair label; its correction changes no sample,
statistic or performance disposition.
The review and corrected receipt checkpoint are retained in
`target/review/p4-windows-review-addendum.zip`, SHA-256
`f9311def7be29ac113397ed2ee402d082042150edf0f121925b03a7bad943680`.

The separate control audit finds that not every parser loop currently advances the reviewed
256-action cadence. The operation mechanism's tests do not establish uniform first-party
parser coverage. Actual colored STL, binary PLY-list and glTF morph cancellation probes and
necessary corrections remain P6 work, followed by final-source performance qualification.
No new control/progress diagnostic is included in these timing workers. P5/P6, full Linux
B1/heap observations and U14 package/portable/sanitizer/GPU checks remain open. No publishing,
push, PR or native-target promotion follows from this measurement.
