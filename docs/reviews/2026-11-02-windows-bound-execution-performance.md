# Windows Bound-Execution Foundation Measurement

Baozi source `46eb6c27b4f272d956eb5ae4889a0470eec3a26c`, compared with the frozen
`4455d03db190c6bef05ff5a1de024d2aac461465` production control. This measures the P2/P3
foundation before the production import-context cutover. It does not pass integrated P3
performance acceptance: normal preparation improves, while the larger text import regresses.

The unchanged ordinary collector ran six memory-input lanes with three warmups and twenty
retained fresh-process pairs each, alternating AB/BA. All 120 retained pairs and 36 warmup pairs
succeeded; no failed lane or discarded sample. Rust 1.95.0 Cargo Release, ordinary System
allocator, generic x64 MSVC, dynamic CRT, no allocation interception. Assimp 6.0.5 uses the same
immutable binaries as the control series. Input/field/precision/normal policy and the collector's
analytic checks are unchanged. No builds or profiler ran during collection.

## Stage time

Times are median / IQR in milliseconds. Changes compare each engine with its own earlier series.

| Case / endpoint | Baozi control | Baozi foundation | Change | Assimp control | Assimp current |
| --- | ---: | ---: | ---: | ---: | ---: |
| repeated-16384 import | 100.281 / 0.716 | 101.199 / 1.123 | +0.9% | 7.632 / 0.352 | 7.425 / 0.269 |
| repeated-16384 prepare | 190.116 / 1.779 | 130.041 / 1.054 | -31.6% | 1.051 / 0.044 | 1.021 / 0.027 |
| repeated-16384 pipeline | 296.912 / 1.756 | 235.854 / 2.463 | -20.6% | 9.692 / 0.255 | 9.166 / 0.260 |
| grid-128 import | 379.162 / 2.651 | 542.771 / 2.679 | +43.2% | 23.564 / 0.558 | 23.017 / 0.602 |
| grid-128 prepare | 0.139 / 0.006 | 0.143 / 0.010 | descriptive | 0.033 / 0.007 | 0.030 / 0.003 |
| grid-128 pipeline | 393.785 / 2.132 | 557.565 / 5.320 | +41.6% | 25.141 / 0.510 | 24.718 / 0.625 |

Repeated preparation's approximately 60 ms reduction substantially exceeds either run's IQR;
the full preparation is still about 127 times this Assimp task. This combined foundation does
not isolate the contribution of the numeric owner, control cadence, leaf definition, compiler
layout or other changes. The former manual empty-action/kernel diagnostics are not an additive
explanation of these ordinary timings.

The approximately 164 ms grid-import increase also substantially exceeds dispersion. The
Assimp stage medians move between -1.7% and -8.4%, including the tiny preparation endpoint;
they do not explain a 43.2% Baozi import regression. Ordinary repeated import is effectively
unchanged at this resolution. Keep the complete regressed samples as evidence rather than
reverting to delayed completion or calling the whole change a performance success.

## Memory scope

The pipeline's whole-child peak working set / peak commit medians, in MiB:

| Case | Baozi control | Baozi foundation | Assimp control | Assimp current |
| --- | ---: | ---: | ---: | ---: |
| repeated-16384 | 17.543 / 13.465 | 17.637 / 14.133 | 25.258 / 20.855 | 25.258 / 20.857 |
| grid-128 | 37.877 / 39.328 | 37.924 / 39.344 | 35.045 / 30.990 | 35.051 / 30.990 |

These Windows counters cover setup, checking and serialization as well as the timed task.
They are not stage peaks, heap measurements or Linux RSS. No parser-only memory improvement is
established. The repeated case's commit increase is retained without claiming an allocation
cause from this aggregate counter.

## Next causal seam

The selected parser still uses `ParseScope` and `ImportStoreHost`. Immediate text-unit completion
currently goes through per-call group construction/sorting, the whole-session projection and
ledger binding. This is a concrete remaining path, and a plausible contributor to the larger
text case's regression; the six-lane result alone does not prove its CPU fraction.

Implement the [accepted exclusive-context cutover](../research/2026-11-02-typed-format-context-design.md)
with actual production capability/container borrowing, retain immediate completed prefixes and
bind repeated parser groups once. Repeat these same ordinary lanes after integration. Attribute
the remaining canonical/text/reader costs in P5 rather than assuming full streaming is necessary.
The integrated implementation must resolve the regression or retain an explicit measured
semantic-cost disposition; this foundation receipt does not grant that disposition.

## Retained artifacts

- Current complete samples and worker output: `target/assimp-comparison/Windows-p3-foundation/report.json`,
  SHA-256 `457c04e888a88b54e6d8e17c0236a0886c5a29c3f0d7b7a6a34ffa2be2440401`.
- Current ordinary binary, build metadata, compiler identity and exact source archive:
  `target/assimp-comparison/Windows-p3-foundation-build/`.
- Source archive SHA-256: `e4eda673743f909c6b561d1171ec72b8f01855906ebdd6cf578bf8689e8f5423`.
- Per-lane control/current calculation: `target/review/p3-windows-comparison.json`.
- Retained current samples, source/build/binary and calculation archive:
  `target/review/p3-windows-evidence.zip`, SHA-256
  `389d6cf51731cbff0a5b0ac0c66375e50e1758d4e76021525d7e671e5d29f1b4`.
- The earlier complete 110-lane control remains unchanged in
  `target/assimp-comparison/Windows-native/report.json` and its
  [source-qualified receipt](2026-11-02-windows-native-performance.md).

Full B1, Linux heap observations and final U14 remain open. No release, publish, push, PR or
native-target promotion follows from this bounded measurement.
