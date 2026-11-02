# Bounded Source Remaps and Stable Normal Ordering

Implementation `52446ec8511b7dfba2e5352ce6941558d85e0ae2` retains two measured algorithm
changes from P5 of the
[operation architecture plan](../plans/2026-11-02-1700-perf-import-operation-architecture-plan.md).
They follow the [actual current-source attribution](../research/2026-11-03-current-import-cost-attribution.md)
and the separately measured parser-completion change `bb6972c...`. The comparison below uses
that numeric-only checkpoint as its before source, avoiding crediting its improvement again.

## Implementation and Deliberate Boundaries

OBJ uses per-segment source-index remaps when all needed source domains fit the Corner count
and their complete lookup/unique-ID capacity envelopes fit the replaced reference temporaries.
Positions retain first-encounter order and distinct source identity; complete UV/Normal tables
sort unique source IDs, retaining ascending source order and F32 bits. Referenced UV rows alone
choose width. Partial-channel loss policy, semantic Corner cells, generated Vertex admission,
descriptor/provenance order and the host canonical construction remain unchanged. Source-sparse
segments use the general occurrence-sort algorithm. Choice uses checked metadata only;
actual admission/allocation failure is terminal and never triggers an alternative attempt.

Each new map/list is metered, initialization is bounded by segment Corners, and all lookup
scratch drops before returning authoring streams. There is no input-wide cache or repeated
scan of unused global source tables. New linear passes use 32-row scopes with fixed bounded
row actions and immediate physical completion. Unique-list comparisons retain the existing
per-leaf admission/completion and independent 64-leaf control cadence.

Indexed Smooth normals use stable source-Vertex buckets when `0 < V <= C` and groups are absent
or uniformly nonzero. A V-slot cursor tail shares the C-slot order allocation and is bounded
by C; group uniformity is proved under actual work after the existing position/face validation.
Forward scatter gives the same `(vertex, original_corner)` order as the prior total key.
Mixed/zero groups and source-sparse domains use the general key sort. Both routes share all
cross computation, F32/F64/sparse reads, compensated/error-aware accumulation, normalization,
orientation, validation and publication. Work/storage rejection does not silently retry.

Exact WorkUnits, scratch peaks and rejected admission schedules deliberately follow the chosen
algorithm. The sparse-memory protections and semantic resource dimensions remain requirements.
The cursor tail can increase scratch for eligible mixed-group inputs; its true admission is
tested, including rejection before any order allocation/output and preservation of the old
Scene. This is bounded scratch, not a promise that every input needs less memory.

## Qualification

| Check | Result |
| --- | --- |
| Both affected packages, all targets/features | Compile passed after two mutable failure-context signatures were corrected |
| New physical/order tests and runtime witnesses | 16/16; `96ce594e-5818-4fb2-bb9e-7515a93f1814` |
| Complete OBJ/postprocess semantics, resources and precision | 439/439; two existing manual skips; `4fb78582-4297-420f-9d76-8016acd555d3` |
| Strict affected all-target/all-feature Clippy | Passed with `-D warnings` |
| Affected formatting | Passed |
| Final public facade control matrix, eight actual encodings | 9/9; `150a54b4-0e49-4c45-aded-563632c19b5f` |
| Final normal bucket/physical/runtime frontier refresh | 11/11; `c892d8b7-2d76-45eb-86e6-eb5cbf5ee375` |
| Independent adversarial source reviews | OBJ and normals closed with no P1/P2 finding |

The complete 439-test run preceded an iterator-only Clippy correction in the uniform-group
proof. The final eleven-test refresh covers that proof and its physical/runtime boundaries.
The main owner checked that the value read remains inside the admitted action. Final U14
will refresh the integrated source; these scoped runs are not relabelled as a workspace pass.
Initial compile/lint failures remain retained beside successful retries.

Private staging assertions distinguish equal-valued source IDs, signed zero, first-use Position
order, ascending channel order, source ID zero and dense/sparse/dense segment isolation.
Physical probes cover all eleven new linear OBJ passes, short tails and denied next admission.
Normal tests supply literal order/physical-prefix oracles, dirty cursor shapes, uniform/mixed/
zero groups, F32/F64 and sparse precision, final cancellation and enlarged allocation rejection.
Existing unused-source growth, exact-budget, Corner seam and numerical tests remain intact.

## Ordinary Windows Result

The unchanged collector uses immutable release/System-allocator workers, the same corpus,
precision, endpoint and IO policy, with no allocation interception or diagnostic instrumentation.
Each of six memory lanes retains twenty AB/BA pairs after three warmup pairs: 120 retained
pairs and eighteen warmup pairs, all semantically successful. No sample is discarded.

Times are median / IQR, milliseconds. Change compares Baozi with the numeric-only checkpoint.

| Lane | Before Baozi | Current Baozi | Change | Current Assimp |
| --- | ---: | ---: | ---: | ---: |
| Repeated OBJ 16,384 import | 54.943 / 0.900 | 36.786 / 0.576 | -33.0% | 7.500 / 0.311 |
| Repeated OBJ preparation | 129.535 / 0.915 | 48.732 / 0.475 | -62.4% | 1.060 / 0.054 |
| Repeated OBJ pipeline | 189.636 / 1.542 | 90.847 / 1.187 | -52.1% | 9.270 / 0.427 |
| Grid OBJ 128 import | 287.995 / 2.340 | 178.630 / 2.211 | -38.0% | 23.331 / 0.251 |
| Grid OBJ preparation | 0.136 / 0.013 | 0.145 / 0.011 | descriptive | 0.034 / 0.005 |
| Grid OBJ pipeline | 304.038 / 1.635 | 194.523 / 1.812 | -36.0% | 24.863 / 0.418 |

Substantial same-lane Assimp medians move by about 0.1-1.7%, while Baozi's substantial tasks
improve well beyond dispersion. The very short no-change preparation lane remains descriptive.
This is one integrated algorithm change: task locality supports the diagnosis, but these
independent endpoint medians are not additive CPU components or isolated per-change costs.
The Assimp gap remains large, especially for generated normals. No parity or general speed
superiority is established.

Whole-child peak working set / peak private commit medians, MiB, for complete pipelines:

| Case | Baozi before | Baozi current | Assimp before | Assimp current |
| --- | ---: | ---: | ---: | ---: |
| Repeated | 17.457 / 13.227 | 16.928 / 12.885 | 25.258 / 20.855 | 25.256 / 20.852 |
| Grid | 37.547 / 38.789 | 32.172 / 29.529 | 35.053 / 30.998 | 35.055 / 30.994 |

These include setup, input, checking, output and serialization, not isolated stage heap or Linux
RSS. They support retaining this bounded candidate, without extending its memory result to
unmeasured models. Complete Windows, Linux B1/heap and final integrated U14 remain required.

## Evidence

- `target/review/p5-algorithms-tested-source.json`: eleven implementation paths and exact
  tested hashes; every path matched before staging.
- `target/review/p5-algorithms-verification.json`: successful and failed command/log identities.
- `target/assimp-comparison/Windows-p5-algorithms/report.json`: raw samples, dispersion,
  controls and whole-child memory; SHA-256
  `26055fd319c4133e155db5a4739e5408e322244df89ad43375b4a77be8f0e970`.
- `Windows-p5-algorithms-build/baozi-probe.exe`: immutable ordinary worker SHA-256
  `708ce98698435a72e1e3274058c9b1ebba1c3bfb224b1aeef3b38722938a7333`.
- `Windows-p5-algorithms-build/source.tar`: committed source archive SHA-256
  `b08f7f5a3ce87b78f9c0808b41e58a34ce545c5f6c74ce484d6a7ca43ea607aa`.

`target/review/p5-algorithms-evidence.json` inventories the implementation/review reports and
all these observations. `target/review/p5-algorithms-evidence.zip` retains 598 files,
SHA-256 `3b54440e69dc12808aa5a912c576898d6cb4e733a3347df5330038343fad608a`.
