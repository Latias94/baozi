# Compact Owner-bound Work Capabilities

R2 of the [performance convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md)
replaces copied semantic keys inside repeated numeric work capabilities. It follows the retained
[semantic execution experiment](2026-11-03-bound-semantic-execution.md). This receipt initially
qualifies source and correctness; the separately reviewed ordinary measurements below support
a modest intermediate keep, with the stated endpoint and tail uncertainties.

## Representation and Contract

`BoundWorkGroup` retains the actual local owner reference, positions, duplicate representatives,
completion order and grant. It no longer copies every full semantic key into each parser scope
and observed permit. Keys remain in the immutable owner catalogue. Six existing private numeric
helpers receive static lazy key resolvers and materialize keys only while constructing an error;
the two inline overflow projections are lazy as well. The cold parent-facing validation seam
still accepts its existing key and uses the same numeric helpers.

There is one numeric kernel and one counter/policy authority. This change adds no allocated
layout, hash table, narrowed unchecked index, callback seam, ledger backend or shadow counter.
Binding comparisons remain outside numeric borrows. Internal key resolvers only copy immutable
metadata; no user callback, policy query, formatting, allocation or control action runs there.
Each operation preserves current arithmetic/check/commit order, duplicate-lane error selection,
representable rejected attempts, atomic direct completion and observed physical-prefix failures.
Current policy, mode, protected-role and outstanding-permit checks remain live.

After a successful binding, numeric errors name the registered catalogue key. An absent-key
binding error still names the caller's supplied key. For a generic key whose equality ignores
diagnostic labels, this deliberately canonicalizes the bound error's label. Baozi's concrete
key compares the complete identity. The breaking refactor documents and directly tests that
choice instead of retaining a hot copied array for unspecified diagnostic representation.

## Source Review and Tests

The four-file candidate was prepared in isolated copies while the immutable R1 worker was
built/measured. Independent review projected all 26 patch hunks against the actual base and
matched them to those copies. Main applied the patch only after R1 collection and attribution
completed. The reviewer separately inspected the formatting delta and actual production hashes;
no actionable P1/P2 finding remains for that snapshot.

Four focused tests check zero resolver evaluation on successful helper routes, exact identity
and one evaluation on failing helpers, noncontiguous/duplicate protected positions, and the
registered/caller diagnostic-label rule. Parser tests exercise post-binding protection and
replenishment at a nonzero catalogue position. Existing duplicate/overflow precedence, physical
prefix, policy tightening, role revocation, zero/full forgotten permit, comparator panic and
no-Ord/no-Debug numeric tests remain intact. No layout snapshot or test mirroring field order
was added.

| Check | Result |
| --- | --- |
| Operation/core/import/postprocess nextest | 2,266/2,266; six existing skips; `df51a838-91b3-4a52-991f-f5f9dbfef41e` |
| Affected compiler contracts | Import 17; operation one positive and 39 compile-fail; passed |
| Strict affected all-target/all-feature Clippy | Passed with `-D warnings` |
| Package formatting | Passed |
| Real-format facade control assurance | 9/9; `23fd4876-c525-4535-aedb-57b45231d3e3` |
| Asset consumer | 120/120; one existing skip; `18e37e89-5072-4099-b7c9-74f7926ed4fd` |
| Ordinary performance | Modest intermediate keep after independent Windows and targeted Linux review; final convergence remains open |

Commands use Rust 1.95.0, locked/offline inputs, one build job and the shared target. Exact source
hashes and command logs are `target/review/convergence/r2-*.json` and corresponding logs.
The patch, implementation note and independent source review are in
`.git/baozi-review/performance-convergence/`. R1 instruction reductions are not credited to R2.
Source-level laziness and a smaller private capability do not establish native speed, smaller
return ABI traffic, memory superiority or Assimp convergence; ordinary measurements decide that.

## Ordinary Measurement

Four fresh Windows campaigns each retain 23 memory-input lanes, twenty alternating AB/BA pairs
after three warmup pairs. Both R1/R2 orders were collected with immutable ordinary workers and
same-campaign Assimp controls. All 4,232 child observations succeeded. A subsequent ordinary
Linux check retains eight lanes for repeated/grid OBJ, with the same twenty-plus-three policy;
all 736 child observations succeeded. No compilation or profiling overlapped these measurements.
The collector, selected output, precision, normal policy and clock/cleanup boundaries are
unchanged. These pilots do not replace the final fourteen-case/110-lane matrices on both hosts.

Windows reverse-confirmation complete-pipeline medians:

| Case | R1 ms | R2 ms | Change |
| --- | ---: | ---: | ---: |
| Repeated OBJ 16,384 | 76.58950 | 73.68500 | -3.79% |
| Grid OBJ 128 | 172.25450 | 163.83355 | -4.89% |
| Shared glTF 32 | 4.89210 | 4.57660 | -6.45% |
| Repeated STL 1,024 | 3.18420 | 3.08360 | -3.16% |
| PLY points 1,024 | 1.68590 | 1.63185 | -3.21% |
| F64 PLY neighbors | 1.39645 | 1.37120 | -1.81% |

The initial campaign has the same pipeline directions. Independent raw-data review supports
only a modest intermediate gain. The confirmation pipeline sum improves 4.55%, below the 5%
scalar stability threshold; its scalar keep instead depends on the worst endpoint. Grid import
improves 8.88% while its Assimp control improves 9.33%, so the entire change cannot be credited
to this source delta. The F64 extraction improvement is real as observed but has no established
lazy-key mechanism: that extraction route was not changed. Do not claim a confirmed at-least-5%
aggregate gain or attribute every endpoint movement to R2.

Repeated-OBJ extraction in the reverse confirmation rises from 4.76955 to 4.93550 ms
(+165.95 us, +3.48%); the initial comparison instead falls 0.83%. Distributions overlap, which
does not prove equivalence. Keep this unresolved observation visible through the next targeted
comparison. All 23 lanes' two Windows whole-child memory methods were reviewed; the largest
R1-to-R2 median increase is below 0.38%, not evidence of a material heap improvement.

Linux R1-to-R2 medians:

| Case / endpoint | R1 ms | R2 ms | Change | Assimp control change |
| --- | ---: | ---: | ---: | ---: |
| Repeated OBJ import | 31.479918 | 29.855603 | -5.16% | +9.11% |
| Repeated OBJ prepare | 37.065174 | 36.673218 | -1.06% | +2.67% |
| Repeated OBJ extract | 4.468225 | 4.503532 | +0.79% | +1.95% |
| Repeated OBJ pipeline | 73.894615 | 71.328393 | -3.47% | +3.36% |
| Grid OBJ import | 144.968489 | 140.929303 | -2.79% | +0.86% |
| Grid OBJ prepare | 0.073311 | 0.075263 | +2.66% | +1.62% |
| Grid OBJ extract | 14.395456 | 14.475157 | +0.55% | +0.21% |
| Grid OBJ pipeline | 160.568125 | 157.478906 | -1.92% | +3.95% |

This check was prompted by current-source Callgrind's whole-worker grid increase of 3.68% Ir;
repeated instead decreases 1.69%. Named `borrow_active` attribution also changes substantially
with outlining. Instruction counts cannot establish native regression or speed. Ordinary grid
import/pipeline medians do not reproduce the direction of the Ir increase. Small extraction
and preparation increases remain disclosed; the ordinary check is limited to two cases and
does not establish Linux-wide equivalence. Independent review rehashed the source/build chain,
reconstructed every raw sample and distribution, and found no task or output reduction.

Retain R2 as a modest intermediate improvement. Linux's two pipeline medians together fall
2.41%, below a material 5% aggregate claim. Both candidate pipeline IQRs are wider; grid's worst
sample rises from 163.050458 to 179.338325 ms and repeated's from 75.567478 to 76.978642 ms.
All samples remain retained. Assimp controls are slower on all eight Linux lanes, so ratio
changes overstate Baozi's absolute median gains. This is one Linux version order, with ten
AB and ten BA retained engine pairs per lane, not interleaved R1/R2 or a reverse-order Linux
confirmation. Do not claim a tail improvement or a universal no-regression bound.

Median Baozi whole-child Linux RSS falls by 82–184 KiB across these lanes; distributions overlap
and the maximum Baozi/Assimp median ratio is 1.0362. This supports no material measured RSS
regression in this narrow check, not a material heap win. The instruction increase, small
endpoint increases and repeated-extraction question remain visible through subsequent work.

| Ordinary report under `target/assimp-comparison/` | SHA-256 |
| --- | --- |
| `convergence-windows-r2-r1-control/report.json` | `aa7bae16b5440be87ab398b7504797f195df05de1cd42df6a114e762bd5b60d4` |
| `convergence-windows-r2/report.json` | `35d2ce1af250bdf4c5e9fd38243a1f6caae36d52971bb1f8312993bab5e7e448` |
| `convergence-windows-r2-confirm/report.json` | `75c245f528121d9e9da546f8186ec6dbce721dd4451c2a4402daa54fcc9a2205` |
| `convergence-windows-r2-r1-control-confirm/report.json` | `aa421215c1649988f04ec99ebfb5dac6e299494315df2ce2e8f0d625873a71e4` |
| `convergence-linux-r2-r1/report.json` | `f62ea69c5435b2552b0f6c8bc96e05ecdf2e85b03388b99e9850291fa2207a30` |
| `convergence-linux-r2-r2/report.json` | `4e865f3dc81bf4f9dc0a76a67990efc3acff7b0f82971534ef3e2248c2ea4355` |

Build/source/binary identities and raw commands are in `target/review/convergence/r2-*.json`
and the respective ordinary `build.json` files. The independent Windows/raw and Linux-review
continuation is `.git/baozi-review/performance-convergence/compact-work-measurement-review.md`.
Windows confirmation still has maximum pipeline/endpoint ratios of **13.25x/107.97x**, with
19/23 timing lanes above 3x. Publication remains closed under the reviewed convergence plan.
