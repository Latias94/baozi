# Bound Attribute Readers and Public Consumer Adoption

This experiment follows the retained
[observed-work sequence](2026-11-03-observed-work-sequence.md) in the
[Assimp convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
It moves selected-field discovery outside application tuple loops and repairs borrowing at
the existing core reader. It changes no normal algorithm, geometric data or timing boundary.

## Ownership and Single Sampling Implementation

Core `AttributeRef` inspection getters and `element` borrow `&self`. Element dispatch matches
the borrowed payload and uses the existing dense/sparse helpers, whose bodies are unchanged.
Returned typed slices retain the Scene lifetime after a temporary descriptor is dropped.
Explicit descriptor/data inspection still returns its existing Copy value. This receiver change
is intentionally breaking for explicit UFCS/function-pointer callers; the one repository UFCS
caller is migrated without a compatibility alias.

`ListVertices::attribute(semantic, domain)` binds a `ListAttribute` to the exact owning layout
and descriptor once. `attributes()` enumerates bound fields lazily in retained order. A bound
stream stores only that layout and original attribute; its constructor remains private.
`element(&self, draw_index)` uses the existing checked topology/source-coordinate mapping and
the same core element reader. `elements(self)` owns the borrowed view for an exact-size,
double-ended, cloneable iterator. Metadata and source identity remain available separately.
The tuple convenience reader uses the same source-domain mapping and core sampler.

An initial isolated proposal duplicated dense/sparse sampling in postprocess to avoid a
by-value core receiver. Main rejected it and replaced the receiver at its owner. That sampler
is absent from production; no alternate payload representation, index cache, lookup table,
unsafe rebinding or second topology implementation was introduced.

Bindings and sampling allocate nothing. Vertex/Corner/Face domains, all seven supported list
assemblies, strip parity, loop closure, repeated/degenerate indices, missing versus present-empty
fields, sparse explicit/default slice identity and F32/F64/U32/I32 bits remain intact. These
readers start no work ledger; the application still owns copying and interruption of its loop.

## Real Applications and Measurement Scope

The actual asset application's `buffers.rs` binds Position, selected Normal/Tangent/Color and
every selected UV stream once per draw. Existing UV selection storage now holds its bindings;
no second UV-binding Vec is allocated. Records are larger borrowed views, so this is not a
claim of smaller metadata memory. Source-Vertex/bitwise indexing, all selected UV sets,
influences, morph representatives, material/tangent declarations, numeric conversions and
detached output ownership remain unchanged. The replaced tuple-level exact-key lookups are
removed from those selected-field loops.

The comparison adapter adopts the same ordinary public API **only inside `visit_draw`**.
Domain ambiguity, missing-field policy, topology admission, per-tuple Position/Normal/UV
conversion order and visitor errors remain. Binding performs no scalar preflight and does
not reject a missing Position on an empty layout before any tuple is read. There is no zip
truncation. Native setup/cleanup, selected detached F64 output, placements, corpus, precision,
normal policy, work allowances, clocks and collector are unchanged. New workers must identify
this adoption rather than relabel R1/R2/R3 evidence.

## Independent Review and Correctness Checkpoint

Independent library and adoption reviews reconstruct both patches against their pinned bases
and establish no open P1/P2 at the inspected snapshots. A subsequent nine-file review finds
only formatting changes and the reviewed UFCS adaptation in actual production. The two
consumers and newly added test file are included in the post-format source freeze.

Tests use real public import and central validation. They cover dense/sparse values and
original pointer/bit identity, 16-lane fields, every list topology, source domains/seams,
empty/missing distinctions, iterator direction/clones, temporary wrapper lifetimes and
zero-allocation reads. Consumer probes retain literal error ordering, visitor interruption,
non-F32-representable F64 values and negative-zero output after Scene destruction.

| Check | Result at this checkpoint |
| --- | --- |
| Core/postprocess all-feature nextest | 961/961; six existing skips; `e0b1e0f3-2ab1-4646-8f08-791703a91c05` |
| Affected compiler contracts | Core six negative; postprocess one positive plus three negative; passed |
| Strict affected all-target/all-feature Clippy | Passed with `-D warnings` |
| Strict standalone asset-consumer Clippy | Passed with `-D warnings` |
| Package and standalone-consumer formatting | Passed |
| Asset-consumer nextest | 124/124; one existing skip; `0037a4f1-8b6e-419c-ae53-c99b02e9a3a1` |
| Ordinary performance | Bounded intermediate Windows keep after independent review; F64 extraction regression and other positive lanes remain open |

Commands use Rust 1.95.0, locked/offline dependencies, one build job and the shared target.
The first reader check's wrapper records only changed existing crate files; the supplemental
`target/review/convergence/r4-source-freeze.json` records all nine actual files during that
check, including the new source and both consumers. Independent review reconciles that freeze
with the reviewed copies. No source changed after formatting. Following checks record both
source roots and new source before/after execution. This is an evidence-scope correction,
not a second run relabelled as the first or a claim that its initial wrapper covered more files.

Exact commands/logs are `target/review/convergence/r4-*.json` and matching logs. Isolated v1/v2
copies, adoption scope, patches and the independent reviews remain under
`.git/baozi-review/performance-convergence/`. Source-level removal of repeated lookup and
descriptor transport does not establish optimized ABI traffic, native speed, memory savings
or Assimp convergence. Ordinary immutable workers decide the performance result separately.

## Ordinary Windows Result and Required Repairs

R4 source `1065e0469ad688bcfa1e2f7d204243be58d85d00` produced ordinary release worker
SHA-256 `6b6ff412f501d08d850375078ec3a56b4c289e0646ba5cb08c4b3bd8bd7b27d6`, with source-archive
hash `93a44a6cdf32f68a5ecfa06e98b1f73e81267fc09a0b6017f6a43c0b3a973148`. Four serial
23-lane memory-input campaigns compare it with retained R3 in both version orders. Each has
twenty retained alternating AB/BA engine pairs after three warmup pairs. All 4,232 children
succeed. No build or profiler overlaps ordinary collection. Independent review reconstructs
the source/binary chain, raw observations, statistics and task equivalence, including the
prospectively authorized public-API adoption in `visit_draw`.

Selected extraction, milliseconds:

| Case | Initial R3 / R4 | Change | Reverse-confirmation R3 / R4 | Change | Confirmation R4 / Assimp |
| --- | ---: | ---: | ---: | ---: | ---: |
| Repeated OBJ | 4.80630 / 0.83490 | -82.63% | 4.90135 / 0.71610 | -85.39% | 1.68x |
| Grid OBJ | 15.30860 / 2.32575 | -84.81% | 15.52575 / 1.95980 | -87.38% | 1.80x |
| Shared glTF | 0.03960 / 0.03380 | -14.65% | 0.03980 / 0.03290 | -17.34% | 2.58x |
| Repeated STL | 0.16745 / 0.04000 | -76.11% | 0.16700 / 0.03700 | -77.84% | 1.87x |
| PLY points | 0.06465 / 0.02520 | -61.02% | 0.06155 / 0.02360 | -61.66% | 4.87x |
| F64 PLY neighbors | 0.01020 / 0.01375 | +34.80% | 0.01070 / 0.01380 | +28.97% | 12.55x |

Five extraction tasks improve repeatedly; repeated/grid/STL/points ranges are disjoint in
both rounds. **F64-neighbor extraction is a confirmed regression**, +3.55 us / +3.10 us,
with disjoint Baozi ranges and unchanged Assimp medians. Keep it as an explicit repair item;
its small absolute duration does not make it noise or clear its release gate. Current-source
attribution must examine fixed reader/setup/transport costs before selecting a correction.
Do not add a size-specific bypass, second sampler, hidden cache or changed benchmark task.

Pipeline medians, milliseconds:

| Case | Initial R3 / R4 | Reverse-confirmation R3 / R4 |
| --- | ---: | ---: |
| Repeated OBJ | 60.50100 / 58.95115 | 61.90890 / 56.98450 |
| Grid OBJ | 164.16805 / 160.53805 | 165.15575 / 154.08945 |
| Shared glTF | 4.56680 / 4.73200 | 4.61700 / 4.72910 |
| Repeated STL | 3.07555 / 2.98400 | 3.08930 / 3.05810 |
| PLY points | 1.64010 / 1.68845 | 1.67385 / 1.63410 |
| F64 PLY neighbors | 1.37925 / 1.42285 | 1.40130 / 1.37590 |

The six pipeline medians sum to -2.13% initially and -6.72% in confirmation. There is no
reproducible at-least-5% aggregate or uniform pipeline claim. glTF pipeline rises 3.62% / 2.43%;
STL import rises 1.87% / 3.98% and preparation 4.74% / 7.32%; F64 preparation rises 5.90% /
1.50%. Preserve those and every other positive-lane disposition. Control drift is substantial
in some initial lanes: grid import Assimp +16.28% and extraction +30.67%. Confirmation controls
are steadier for the large extraction gains; no drift correction or sample exclusion is used.
The scalar pilot is inconclusive; confirmation says keep for the pipeline sum only. Those
scalar decisions do not override individual endpoint regressions.

Independent review supports retaining the shared public-reader architecture as an intermediate
improvement while its named costs are repaired. Whole-child working-set median increases are
at most 40 KiB in either round; maximum relative working-set/commit increases are 0.4421% /
0.3717%. Some small working-set shifts have disjoint distributions. This is no material measured
memory regression, not equality, stage heap or a memory-saving claim. The maximum same-method
Baozi/Assimp median is 1.6723. Confirmation still has 15/23 timing lanes and 4/6 pipelines above
3x, with maximum endpoint/pipeline ratios 101.61x / 13.83x. The release gate remains open.

| Report under `target/assimp-comparison/` | SHA-256 |
| --- | --- |
| `convergence-windows-r4-r3-control/report.json` | `e69977244586e1aecae605ef3cc0b2199433f9eaa7c50542e1a796ee398d0984` |
| `convergence-windows-r4/report.json` | `21772af039132c4a91e99da3f339e35e65985a515733b8dbec31800a693d1438` |
| `convergence-windows-r4-confirm/report.json` | `42b202be0536cadab041a1eae82d8c9cfb989ae3a05326d65d1561a36d240ce0` |
| `convergence-windows-r4-r3-control-confirm/report.json` | `765e6a4723799093c298d95e41ac2d1d0502a82f322cdbe4c6b073de8424bb56` |

Detailed dispositions and raw reconciliation are in
`.git/baozi-review/performance-convergence/bound-reader-measurement-review.md`.
Six separate Linux profiles completed afterward with the same R4 source and symbol worker
SHA-256 `6d076af045734e5779b84300fe82d73d9f54a23f444ec2d5e4749a6ceb2d26ad`.
Main reconciled every exclusive row with raw totals using the unchanged owner classification.
Against R3, whole-worker Ir falls 4.76% repeated, 5.85% grid, 0.25% glTF, 4.75% STL, 5.07%
points and 0.12% F64. These are not stage fractions or native-time measurements. In particular,
F64's slight whole-worker instruction decrease does not clear its confirmed Windows extraction
regression. Raw/annotated profiles and the summary remain in
`target/review/convergence/r4-attribution/`; fixed-reader cost investigation is separate work.
