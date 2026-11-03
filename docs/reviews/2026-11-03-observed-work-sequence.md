# Invocation-bound Observed Work Sequence

R3 of the [Assimp convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md)
reuses one producer envelope across a nonempty bounded postprocess execution region. This
receipt qualifies the implementation and correctness. Independent ordinary review supports
an intermediate Windows keep, with the explicit grid and small-endpoint tradeoffs below.

## Replacement and Ownership

The first real action admits one unit; subsequent actions replenish exactly one only after the
preceding action has completed. Each normally returned action, including a semantic error,
completes immediately through the original numeric authority. There is no pre-admitted future
action, buffered completion, shadow ledger or relaxed work/storage allowance. Initial admission
and singleton replenishment share the same extracted arithmetic/check/commit implementation.
Current policy, mode, protected grant, duplicate ordering and pending headroom remain live.

`ActiveRuntime` independently tracks an in-flight action and ownership of the idle envelope.
Zero remaining is not closure. Successful region return explicitly closes the envelope and
checks quiescence before the final checkpoint. A normally returned known error explicitly
resolves its idle ownership without another poll and preserves its cause. An unmatched Begin
or actual unwind retains abandonment through the existing Drop path. Completion failure
immediately resolves the owner while preserving any already applied physical prefix and the
original failure; callers may check quiescence inside that failure boundary.

Actual Internal retained-share work can occur between leaves on the same authority; it does
not reopen the Producer lane or overwrite another owner's pending suffix. No numeric guard
or storage lock spans an action, allocation, control callback or capture/result destruction.
The old per-leaf open/finish path is removed from this runtime. The existing raw multi-unit and
parser protocols remain for their actual distinct consumers. This experiment changes no normal
math, precision, work vocabulary, selected fields, parser layout or comparison clock.

## Review and Qualification

Independent source review initially requested two missing destructor-driven unwind probes.
The added tests exercise an actual owned FnOnce capture dropping at callback return and a
completed leaf's returned value dropping while the region remains idle. Both require one
literal physical write, one drop, the exact attempted/consumed prefix, ProductGraph failure
evidence and no cleanup poll. They are not replaced by an explicit body-panic test.
The reviewer closed that finding and rechecked all seven applied/formatted files.

Sixteen new tests cover refill ownership, rejection/overflow, attempted-versus-consumed
precedence, live policy and role revocation, competition with real Internal pending work,
scratch admission/share/release, idle success/error/unwind and the destructor probes.
Existing failure, no-poll, immediate completion-error cleanup, zero/full-forget and
cancellation-bound assertions remain. Cancellation cases include a final short region and
0/1/31/32/33/255/256/257 completed leaves.

The first strict Clippy run rejected a large-error closure in explicit region cleanup.
Replacing `closed.and_then(...)` with an equivalent `match` retains inline errors and the
same evaluation order, with no new allocation or lint exemption. Source review accepted that
exact adjustment. Its failed log is retained. The final source passed:

| Check | Result |
| --- | --- |
| Operation/core/import/postprocess nextest | 2,282/2,282; six existing skips; `0939b408-d82a-4877-a38b-19269ffe6920` |
| Affected compiler contracts | Import 17; operation one positive plus 39 compile-fail; passed |
| Strict affected all-target/all-feature Clippy | Passed with `-D warnings` |
| Formatting | Passed |
| Real-format facade control assurance | 9/9; `2278b68d-6602-4d2d-8df9-ff2e5c79c9a7` |
| Asset consumer, all features | 120/120; one existing skip; `7226f75f-4b0c-422d-8593-a0d908dd8ed9` |
| Independent source review | No open P1/P2 at the final inspected hashes |
| Ordinary performance | Intermediate Windows keep after independent raw-data review; final convergence remains open |

Commands use Rust 1.95.0, locked/offline inputs, one build job and the shared target. Compiler
contracts ran before the closure-to-match correction; that correction changes no exposed
signature, lifetime or capability. Integrated tests and strict lint were rerun afterward.
Exact source hashes and command logs are `target/review/convergence/r3-*.json` and matching logs.
The preformat v2 patch, implementation handoff and independent source review remain under
`.git/baozi-review/performance-convergence/`.

The subsequently designed bounded normal-action vocabulary and exclusive invocation/reader
changes are not included. Source-level removal of envelope churn is an opportunity, not
evidence of native gain. Ordinary immutable workers must preserve the existing task and all
samples; the complete Windows/Linux release-convergence gates remain open.

## Ordinary Windows Checkpoint

Source `ae4c77af891e3d92340535c48408a32ffbec34e9` built an uninstrumented Rust 1.95.0
release worker with binary SHA-256
`2218f9314319e2bb0c043e81ca2014c87c9a93d1bc3657591df3bc8cd53b97ce` and source-archive hash
`30b2ab7b784ba3984364cc045b7cc0afea2f5ae6622792e018dc7c05eba8d7c8`.
Four serial ordinary campaigns, including a fresh reverse version order, each contain 23
memory-input lanes with twenty retained alternating AB/BA engine pairs after three warmup pairs.
All 4,232 comparison children succeeded. No compilation or profiling overlapped collection.
The collector, task, normal algorithm, output, budgets and cleanup clocks are unchanged.

| Endpoint | Initial R2 / R3 ms | Change | Reverse-confirmation R2 / R3 ms | Change |
| --- | ---: | ---: | ---: | ---: |
| Repeated OBJ normal preparation | 37.48735 / 23.71540 | -36.74% | 37.79805 / 23.75805 | -37.14% |
| Repeated OBJ pipeline | 73.89370 / 59.91400 | -18.92% | 75.74015 / 60.56980 | -20.03% |
| Grid OBJ pipeline | 163.03505 / 168.58630 | +3.40% | 168.49950 / 169.49345 | +0.59% |
| Shared glTF pipeline | 4.55845 / 4.63955 | +1.78% | 4.88240 / 4.62720 | -5.23% |
| Repeated STL pipeline | 3.08895 / 3.06520 | -0.77% | 3.18135 / 3.17595 | -0.17% |
| PLY points pipeline | 1.66895 / 1.65560 | -0.80% | 1.68080 / 1.69765 | +1.00% |
| F64 PLY pipeline | 1.33810 / 1.33590 | -0.16% | 1.37385 / 1.41520 | +3.01% |

The normal endpoint's approximately 14 ms reduction repeats in both version orders. Its
same-campaign Assimp controls move +4.24% initially and -3.93% in confirmation; R3 still costs
21.43 times that control in confirmation. This is evidence for the bounded execution-owner
opportunity, not a language or normal-estimator comparison proving equivalent general math.
Repeated pipeline's confirmation Assimp control moves -16.79%; credit the observed Baozi
absolute change without treating every ratio change as a source effect.

Several other lanes have positive median changes. Grid preparation rises 14.35 us / 9.99%
in confirmation, with its control +19.94%, after an initial -5.20%. Grid pipeline is positive
in both rounds while its controls move +14.26% / +7.58%. F64 pipeline's confirmation control
is +33.69%. These observations require dispersion/raw-data review; controls do not license
subtracting noise or calling every positive change equivalent. All samples remain retained.

The six pipeline medians sum to a 3.39% reduction initially and 5.63% in confirmation. The
unchanged scalar rule says keep for different reasons in the two rounds: maximum endpoint
initially, primary sum in confirmation. It is not a substitute for individual-lane review.
Confirmation maximum pipeline/endpoint ratios remain **13.82x / 106.53x**. Current memory
ratios and raw success observations do not establish a heap improvement or release readiness.
Independent review rehashed the source/build chain and reconstructed all ordinary samples and
distributions. Retain the substantial repeated-geometry improvement as an intermediate Windows
result, explicitly accepting the unresolved grid pipeline cost. Both grid engine-order subgroups
are positive in both campaigns; confirmation grid preparation is positive in both halves and
engine orders. These are concerns to carry into subsequent work, not cleared regressions.
There is no reproducible at-least-5% aggregate gain. Eighteen of 23 confirmation timing lanes
remain above 3x; all complete release gates stay open.

Every child has valid whole-lifetime Windows peak working-set and peak commit observations.
All corresponding R2/R3 distributions overlap. Baozi median working-set increases are at most
56 KiB initially / 44 KiB in confirmation; the largest relative commit increase is 0.3718%.
This establishes no measured material memory regression in these lanes, not a heap or memory
equality claim. The independent receipt is
`.git/baozi-review/performance-convergence/observed-sequence-measurement-review.md`.

| Report under `target/assimp-comparison/` | SHA-256 |
| --- | --- |
| `convergence-windows-r3-r2-control/report.json` | `4cd890d03745e9136450f04d88b89836a828cf8cd4b9a8bb7dd23e7fdbdfcee5` |
| `convergence-windows-r3/report.json` | `c41223303dc960b3df66496cad428f8e961ac08eaf8de2586fcac10309dd2192` |
| `convergence-windows-r3-confirm/report.json` | `0a672613e527ea022d2203488dc449b0d7ac781e918be76aaa210f3e9d096a31` |
| `convergence-windows-r3-r2-control-confirm/report.json` | `aec00fa6b3b75bac69d53a296a24d378caead6267c9ae3c67f5696b850ce9440` |

The build manifest, immutable ordinary worker, source archive and all commands/raw observations
are retained locally. `target/review/convergence/r3-*-summary.json`, decision inputs/outputs and
`r3-confirm-comparison.json` are read-only derivatives of those reports. New normal-vocabulary
and reader candidates remain isolated and are not part of these measured identities.

## Refreshed Linux Attribution

Six separate whole-worker Callgrind profiles use R3 source `ae4c77a` and symbolized worker
SHA-256 `1bff414350c731d6637c48b9906cf2f91fba0aefa8ee9f85d7207cac8a2afb2a`.
They ran after ordinary collection ended. Main reconciled raw totals and every exclusive flat
row using the existing owner classification; the independent reviewer separately checked the
supplied summary arithmetic, not every raw profile/build identity.

| Case | Actual baseline | Baseline Ir | R3 Ir | Change |
| --- | --- | ---: | ---: | ---: |
| Repeated OBJ | R2 | 1,254,867,325 | 1,102,432,336 | -12.15% |
| Grid OBJ | R2 | 3,072,657,675 | 3,064,230,183 | -0.27% |
| Shared glTF | R1 | 45,463,698 | 45,455,430 | -0.02% |
| Repeated STL | R1 | 31,044,583 | 31,538,357 | +1.59% |
| F64 PLY | R1 | 2,589,461 | 2,590,463 | +0.04% |
| PLY points | R1 | 9,852,016 | 9,652,868 | -2.02% |

Four baselines remain R1; they do not isolate R3's delta. Ir is not native CPU time, a stage
fraction or heap. Grid's slight instruction decrease does not erase its observed Windows
pipeline increase. These are not Linux native-time comparisons or cross-platform regression
clearance. Current operation-owner grouping still accounts for 64.22% of repeated-worker Ir,
52.22% of grid, 65.95% of glTF and 66.74% of STL. The next experiments still need to address
numeric authority/granularity, field binding and lifecycle costs at their owners.

Exact profiles, annotations, observations, hashes and reconciled summary are retained in
`target/review/convergence/r3-attribution/`. The shared-glTF sort still makes 13,419 cold
`charge_work` comparisons, supporting the separately scoped source-profile candidate without
crediting instruction headroom as a measured future gain.
