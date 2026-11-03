# Rejected Reader Composition Inline Experiment

The [bound-reader qualification](2026-11-03-bound-attribute-readers.md) retains a confirmed
small F64 extraction regression. Read-only investigation identifies out-of-line composition
and aggregate transport as a possible target, without establishing the Windows mechanism.
This bounded experiment adds ordinary `#[inline]` to just `ListVertices::attribute` and
`ListAttribute::element`. Their bodies, single mapper, core sampler and public semantics
are unchanged. **Reject these two hints and remove them from maintained source.**

## Source and Correctness

Fresh control source is `509b80de4fff2494d5315f7a17e086cb7afe2f26`, including the separately
qualified consumer normal intent. Candidate source is
`844d414350d330ab9fd1f93d75c50ec66b9d3073`. The exact runtime difference is two inserted
attributes in one file. Comparison tasks, fields, policies, F64 output, clocks, collector,
corpus and Assimp workers are unchanged. The consumer's real SourceFormat default does not
replace the comparison's explicit Smooth request.

Independent source review finds no actionable P1/P2. Rust 1.95.0 locked/offline qualification,
one Cargo job and the shared target pass: 965 core/postprocess tests with six existing skips
(`8c33141f-bbd8-4353-b035-f27636efabe6`), core/postprocess positive/negative doctests, strict
affected Clippy, formatting and 138 consumer tests with one existing skip
(`0573d937-16ac-41bc-8887-65de7b06164c`). No tests are added to mirror optimization hints.
`target/review/convergence/reader-codegen-tested-source.json` binds these checks to the source.

| Ordinary worker | SHA-256 |
| --- | --- |
| Control | `622023ef8225fcc35a1a171af1b9464f3d41915aac0a4beda71409d6e966f931` |
| Candidate | `4bca638acfcd9abd8273a22b20c5439793b1034e5d7bd929092811e4e1c5950c` |
| Control source archive | `d5c4dfb1b3415b487298b324da335716ba4b5e36aa66d91ffcab9feb95680e77` |
| Candidate source archive | `9b2677a8d14c7da7020cd26812229b7a4e9ed6555857e4ab10d4080a8628ab31` |

## Ordinary Evidence and Rejection

Four serial Windows campaigns use the unchanged six-case, 23-lane memory-input task, twenty
retained alternating AB/BA engine pairs after three warmup pairs per lane, and reversed
version order for confirmation. All 4,232 children succeed. No build or profiler overlaps
ordinary collection. Independent review reconstructs raw observations, statistics, task,
source/binary identities, controls and memory; it recommends rejection.

Extraction times are milliseconds, control / candidate:

| Case | Initial | Change | Reverse confirmation | Change |
| --- | ---: | ---: | ---: | ---: |
| Repeated OBJ | 0.69965 / 0.63335 | -9.48% | 0.69350 / 0.62330 | -10.12% |
| Grid OBJ | 1.95820 / 1.80365 | -7.89% | 1.93630 / 1.81675 | -6.17% |
| Shared glTF | 0.02870 / 0.02980 | +3.83% | 0.02800 / 0.02910 | +3.93% |
| Repeated STL | 0.03340 / 0.03275 | -1.95% | 0.03400 / 0.03445 | +1.32% |
| PLY points | 0.02220 / 0.02205 | -0.68% | 0.02170 / 0.02245 | +3.46% |
| F64 neighbors | 0.01005 / 0.01185 | +17.91% | 0.01010 / 0.01185 | +17.33% |

F64 costs an additional 1.80 / 1.75 us in both orders. Its middle halves are separated,
half/order splits retain the increase and Assimp stays at 1.00 us in all four campaigns.
This is a repeated endpoint cost, not a single outlier or a small absolute duration to
exempt. Full ranges overlap; no stronger distributional separation is claimed. glTF also
increases in both rounds. Large extraction gains cannot offset failure of the endpoint this
experiment intended to repair. The six-pipeline median sum changes only -1.13% / -0.14%.
Every parent/tail observation is retained; no sample is trimmed.

Whole-child Windows working-set and commit medians show no material confirmed regression;
maximum same-campaign working-set ratios are 1.66929x / 1.66939x. These process-lifetime
peaks are not stage heap or storage-efficiency evidence. A bimodal repeated-import commit
distribution prevents crediting one lower median as an allocation improvement. Fifteen of
23 timing lanes and four of six pipelines still exceed 3x Assimp. Memory does not clear the
endpoint failure or qualify Linux and the full release matrix.

| Report under `target/assimp-comparison/` | SHA-256 |
| --- | --- |
| `convergence-windows-reader-control/report.json` | `f8f186ff91f06067a1a484534c758feadedd3aa27925ae43754536c988033d4e` |
| `convergence-windows-reader-codegen/report.json` | `ab467d4921e936e96207cff9606d08f4dda158540e9d279a9267377d3d52072a` |
| `convergence-windows-reader-codegen-confirm/report.json` | `2c95cb5fe027647930df63f434263f09c9b4d19af2871fb5f1d5b98f14ddd95f` |
| `convergence-windows-reader-control-confirm/report.json` | `a19ada9e458b3509c72305e7a2334cf982cb6cc0e786bb982443163560179c0c` |

The detailed independent receipt is
`.git/baozi-review/performance-convergence/reader-codegen-measurement-review.md`.
Preserve the original R4 F64, R5 grid-confirmation and this rejected candidate's evidence.
None is cleared by removing the hints or by a later unrelated source change.

## Actual Executed Composition: Diagnostic Only

After ordinary collection, a separate Linux symbol worker at the candidate source, hash
`d575930ab571ab8127b077fc70d4b5ed98907a00b517b12de01b223c7cecec92`, runs grid and F64
whole-worker Callgrind profiles. Flat exclusive sums exactly reconcile with raw totals.
Executed wrapper calls from F64 extraction disappear: it calls the existing mapper and core
sampler directly, three times each, with one direct memcpy still present. The inlined source
locations are attributed to the actual extraction function. The existing base-pose subtree
remains 3,208 Ir.

Against the earlier R5 symbol profile, F64 extraction is 7,648 / 7,560 Ir, and grid extraction
is 36,581,506 / 32,944,083 Ir. That older symbol source also predates the consumer policy,
so this is not a pure two-line source comparison. It confirms the candidate's executed Linux
composition, not Windows lowering, native-time attribution or a successful F64 repair. The
ordinary fresh control above decides rejection even if an instrumented route uses fewer
instructions. Raw profiles and `executed-composition.json` remain under
`target/review/convergence/reader-codegen-attribution/`.
