# Current Import and Preparation Cost Attribution

This continues P5 of the
[operation architecture plan](../plans/2026-11-02-1700-perf-import-operation-architecture-plan.md).
It records observations after the parser-control correction, before another production
change. No elapsed-time percentage is inferred from instruction counts.

## Ordinary Windows Checkpoint

The ordinary worker at `98abe39680cc339b92460c46c21de98d51a5b4eb` uses the unchanged
collector, corpus, precision, endpoints, allocator and release policy. Six memory lanes each
retain twenty AB/BA pairs after three warmup pairs: 120 retained pairs and eighteen warmup
pairs, all semantically successful. No allocator interception, profiler or progress diagnostic
is enabled in this worker. The comparison source is the preceding typed-context worker at
`89f4e0bba832a43be55ce60159cc3b1790040820`.

| Lane | Previous Baozi median ms | Current Baozi median ms | Previous Assimp median ms | Current Assimp median ms |
| --- | ---: | ---: | ---: | ---: |
| Repeated OBJ 16,384 import | 42.075 | 58.369 | 7.942 | 10.005 |
| Repeated OBJ preparation | 135.210 | 129.594 | 1.144 | 1.084 |
| Repeated OBJ pipeline | 182.428 | 191.835 | 10.209 | 9.540 |
| Grid OBJ 128 import | 203.288 | 300.984 | 24.921 | 23.541 |
| Grid OBJ preparation | 0.146 | 0.145 | 0.042 | 0.029 |
| Grid OBJ pipeline | 221.903 | 315.371 | 26.964 | 25.016 |

Grid import and pipeline regress materially while their same-lane Assimp controls remain
near the preceding result. Repeated import's Assimp control also moved, so that cross-run
change is less clean. The very short no-change preparation lane remains descriptive. The
control correction is required for truthful physical prefixes and bounded cancellation;
reverting it is not a valid performance candidate. Separate endpoint medians are not added
or subtracted as an exact pipeline decomposition.

The report is `target/assimp-comparison/Windows-p5-control/report.json`, SHA-256
`84a5a91f4112795bf0952f154b29b01199becc8a011c78ca4d786dfb03f11347`. All raw samples,
dispersion, whole-child working-set/commit observations and build identities remain there.
Worker SHA-256 is `8c63a847acadc139d797639d4bc5ce7c5075d0b7db53df8bc4ce4d484dd05476`.
`target/review/p5-control-disposition.json` retains both runs' statistics. This checkpoint
is not the complete final Windows series or Linux B1.

## Current Whole-Worker Instruction Profiles

The Linux worker is built from `daf378d23f354c65a03febb871ea361c9249b451`, whose runtime
source is unchanged from `98abe396...`. Rust 1.95.0 release optimization, debug level 1 and
System allocation are recorded in `target/assimp-comparison/P5-attribution-build/build.json`.
The worker enables no allocation-metrics feature. Its SHA-256 is
`cce1d03f2d557a8ac2844ab2f9aa82a87903ae4e2ab44dce3c88f0bf40cfd4b9`.

Valgrind/Callgrind 3.26.0 collected one fresh memory pipeline worker per case, recording only
`Ir`. Input hashes and all analytic observations match the frozen comparison manifest.
Profiles include setup, preload, import, preparation, detached extraction, checking,
serialization and teardown. They are neither isolated-stage profiles nor native time/RSS
measurements. The grid run reports `brk segment overflow` with allocator fallback under
instrumentation; semantic output still succeeds, and this limits allocation interpretation.

| Case | Whole-worker instructions |
| --- | ---: |
| Grid OBJ 128 | 6,828,426,425 |
| Repeated OBJ 16,384 | 3,577,591,805 |
| Shared glTF 32 | 54,498,933 |
| Binary STL repeated 1,024 | 50,652,209 |
| Binary PLY points 1,024 | 12,912,754 |
| F64 PLY neighbors | 2,881,898 |

The flat exclusive rows sum exactly to each program total. Source/function-owner groups in
`target/review/p5-attribution/exclusive-summary.json` count each row once, including inlined
locations. Shared work, allocation and reader functions stay with their own owner or the
residual; they are not reassigned to every caller. Inclusive edges explain routes and are
not added to exclusive totals.

For grid, parser-work channel owners account for 2,727,052,954 exclusive instructions
(39.94% of whole-worker Ir), and comparison/sort owners for 696,164,265 (10.20%). These are
function-owner fractions, not phase CPU-time shares. The call graph records 6,367,753 calls
from the mesh-builder sort route into `ComparisonWork::run_leaf`, with the same number of
per-leaf `replenish` and `complete` calls. Immediate work truth exposes the cost of sorting
occurrence-sized references through a general fallible comparison protocol. Text's two passes
exist, but pass counting does not establish that text is the first rewrite to make.

For repeated geometry, the graph records 2,633,684 entries into each of
`ActiveRuntime::begin_action` and `complete_action`. Smooth-normal ordering uses a heap sort
of every corner by source vertex, smoothing group, flat-face identity and original corner.
The last component intentionally preserves original accumulation order. This source has only
three positions and one shared nonzero smoothing group; its large action count therefore
justifies assessing a stable vertex-bucket alternative before changing numerical semantics.
The profile does not establish that the geometry kernel itself occupies only the exclusive
`normals` function fraction: its shared work/read children are separate owners.

## Next Measured Candidates

1. Assess a source-index remap for OBJ positions and a unique-source projection for complete
   normal/texcoord channels. Preserve first-use position order, ascending authored source order
   for retained channel values, distinct equal-valued source indices and partial-channel
   policy. Account for many unused input values and many segments before choosing scratch
   ownership. Do not replace the old sort with an unbounded hash operation or hidden storage.
2. Assess stable source-vertex buckets for a single nonzero smoothing group. Preserve original
   corner order within each bucket, area weighting, F32/F64/sparse reads, group-zero face
   isolation, finite checks and provisional output. Mixed groups still require the general
   grouping contract. Bound additional scratch against the actual corner/source domain.

Each retained change needs source review, exact semantic/resource/control tests and an ordinary
Windows before/after measurement with the same worker protocol. A lower Ir total alone is not
a performance acceptance. Borrowed UTF-8, flat face staging, payload adoption and typed reader
views remain later measurement choices; none is required merely because it appears in P5's
candidate list. Full Windows, Linux B1/heap and original U14 remain open.

`target/review/p5-attribution-evidence.zip` retains 661 files: immutable Windows and Linux
workers/builds/source archives, all ordinary samples, all six raw profiles and annotations,
the exclusive-row reconciliation, exact commands and read-only attribution/readiness reports.
SHA-256: `6670481e360d80330f6096601bb3867e3fc8706e5c0bdfc453a234236617ebee`.
