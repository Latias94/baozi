# Bound glTF Source-Profile Comparison Work

This is a separate owner replacement within the
[Assimp convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
The glTF source profile sorts owner/source associations and extras. Its comparators formerly
entered `charge_work(1)` separately for each comparison. This change binds the existing
parser-work capability once per scope and retains immediate admission/completion for every
actual comparison. It does not replace the decoder, sorting algorithm or metadata model.

## Authority, Completion and Control

The private `ProfileComparisonWork` borrows the real `ImportWorkScope` and holds a countdown.
It replenishes exactly one unit, executes the original comparison once and immediately
completes `[1]`. A denied admission executes no comparison. An unwinding comparison is not
completed; completion failure prevents another comparison. No allocation, numeric guard,
control callback or deferred completion separates a normal return from its accounting.

Extras reuse the collection group after collection scopes close. Links bind one group for
separate sort and dedup scopes, each with zero initial work. No future batch is admitted.
Scope exit retains its exact/partial cleanup and final poll. A local poll follows every 64
completed comparisons, with owner entry/final checks and short-tail scope checks retained.
Sort-exit rejection prevents dedup; dedup-exit rejection prevents truncation. Empty/singleton
paths retain control boundaries without synthetic comparison work. The additional local
bound does not replace the R5 invocation control owner or weighted normal vocabulary.

Production keys are fixed source/owner identifiers or validated bounded namespaced keys.
The comparison body adds no raw JSON scan, unbounded user callback, payload copy or storage.
WorkUnits retain their existing unlimited per-item parser rule. Numeric allowance, comparison
units and resource capacity do not change.

## Exact Semantics and Removed Paths

The original `MeteredVec::try_sort_unstable_by` and `try_partition_dedup_by` execute the same
composite comparisons: extras source followed by lazy key order, link owner/source order and
exact pair equality. Copy suffix truncation keeps its capacity and adds no duplicate-drop
charge. Source indices, owner variants, raw ranges, locator construction, resource-relative
association and retained bytes remain unchanged. Missing, explicit null, escaped keys,
Unicode, signed-zero spelling and large integer text are not reconstructed.

Delete the three comparator-local cold-charge paths for extras sort, link sort and link
dedup. There is no compatibility route at those sites. The other thirteen existing function
bodies, including lookup charging and associations, are byte-identical. No dependency,
catalog version, normal policy, benchmark adapter or timing boundary changes.

## Source and Correctness Qualification

Independent design/source reviews reconstruct the frozen two-file patch. The actual-source
delta review confirms the production file equals that candidate exactly; only the new tests
receive formatting. No actionable P1/P2 remains at those snapshots.

Nine new tests use real public import sessions and terminal evidence. Literal physical
closure counts and work prefixes check zero/short admission, known error and an unreturned
panic. Actual link/extras owners cover sort/dedup output, capacity, 64-comparison and short-tail
cancellation, empty/singleton boundaries, unchanged comparison counts and zero-charge suffix
truncation. Real JSON/GLB imports retain literal extras bytes, selected/omitted source rows,
source/resource/provenance/metadata counts and detached evidence after report destruction.

Windows qualification uses Rust 1.95.0, locked/offline dependencies, one build job and the
shared target:

| Check | Result |
| --- | --- |
| Strict glTF all-target/all-feature Clippy | Passed with `-D warnings` |
| glTF all-feature nextest | 517/517; no skips; `99029ec6-b6f1-4295-a9ca-5c1521111788` |
| Standalone asset-consumer nextest | 138/138; one existing skip; `f84a52c5-0146-400d-9e59-8869ba40ac61` |
| glTF formatting | Passed; final rerun idempotent |

The first formatting command succeeds, then the task-local recorder asserts because its
explicit new `gltf-fmt` step was missing from the formatting-mutation exemption. The original
receipt and a correction record remain. Only the recorder's explicit step list is corrected;
the idempotent rerun, strict lint and tests use the same final source. This is not a masked
Cargo/test failure or a semantic source change.

Exact source/check histories are under `target/review/convergence/gltf-profile-*.json` and
matching logs; the original patch and independent reviews remain under
`.git/baozi-review/performance-convergence/gltf-profile-*`. These checks establish correctness
for this slice, not a performance keep.

## Measurement Boundary

At source qualification, ordinary evidence was pending; its later disposition follows below.
Reuse the retained qualified
pre-hint control at `509b80de4fff2494d5315f7a17e086cb7afe2f26`: after rejection of the reader
hints, maintained runtime bytes exactly matched that control before this two-file change.
Collect fresh paired ordinary campaigns for both workers, including shared glTF scales 1, 8
and 32 and the six previously selected cases under the unchanged comparison protocol.
Those eight-case aggregates must not be ranked against the earlier six-case totals.

Fresh source/binary identities, independent per-lane/control/memory review and separate
instruction attribution decide whether to keep this owner. The original F64/grid costs,
short no-op preparation gaps and full fourteen-case/110-lane Windows/Linux release gates
remain open. Passing metadata tests or reducing cold binding does not establish native speed.

## Ordinary Windows Result and Accepted Intermediate Costs

Qualified source is `ae11d58a6702a4f2a54910b277043d9aa9dbfbc2`; its ordinary worker has
SHA-256 `d669853b46381221e6e8f7f529398aaadd18bc3253fe20d8b83f46010b0a3ed0` and source
archive `052d50a0566ce81199ec622b5e6263827b8707724d04d580f113da000b277a17`.
Four serial eight-case, 31-lane memory-input campaigns use twenty alternating retained AB/BA
engine pairs after three warmup pairs, with reversed version order for confirmation.
All 5,704 children succeed; builds and profilers run separately afterward.

Independent review reconstructs all raw records, statistics, source/binary identities,
unchanged task and successful work/output signatures. It recommends an intermediate keep
for glTF32's material import/pipeline improvement and glTF8's repeated central gains.
Main accepts that bounded disposition with every named extraction/memory cost below left
open for explanation/repair. This is neither a general no-regression claim nor permission
to release while an endpoint remains over the parent plan's target.

Times are milliseconds, control / R7:

| glTF scale / endpoint | Initial | Change | Reverse confirmation | Change | Reverse R7 / Assimp |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 / import | 1.70480 / 1.59415 | -6.49% | 1.71160 / 1.65455 | -3.33% | 7.40x |
| 1 / pipeline | 2.01420 / 1.93085 | -4.14% | 2.05850 / 1.94125 | -5.70% | 7.20x |
| 8 / import | 2.21100 / 1.99330 | -9.85% | 2.20815 / 1.98635 | -10.04% | 8.70x |
| 8 / pipeline | 2.55645 / 2.27160 | -11.14% | 2.50955 / 2.31075 | -7.92% | 8.06x |
| 32 / import | 4.19020 / 3.14410 | -24.97% | 4.29250 / 3.27115 | -23.79% | 11.83x |
| 32 / pipeline | 4.59225 / 3.54910 | -22.72% | 4.71780 / 3.64260 | -22.79% | 10.41x |
| 1 / extract | 0.01060 / 0.01175 | +10.85% | 0.01090 / 0.01195 | +9.63% | 9.96x |
| 8 / extract | 0.01460 / 0.01560 | +6.85% | 0.01480 / 0.01565 | +5.74% | 4.01x |
| 32 / extract | 0.02830 / 0.02840 | +0.35% | 0.02830 / 0.02975 | +5.12% | 2.26x |
| F64 neighbors / extract | 0.01010 / 0.01080 | +6.93% | 0.01020 / 0.01105 | +8.33% | 10.52x |

glTF32 import and pipeline have disjoint old/new Baozi sample ranges in both rounds. glTF1/8
extraction instead increases repeatedly, with upward central bands and positive half/order
splits; these are not dismissed as outliers or hidden by import gains. Their full distributions
overlap and glTF1 dispersion limits the percentage precision. glTF32 extraction has a small
pilot change below spread and a larger confirmation concern with overlapping quartiles;
retain that narrower finding rather than call it a robust two-round 5% regression.

F64 extraction's additional 0.70 / 0.85 us is a separate repeated central cost, with upward
quartile bands and positive splits. Reader source and non-timing signatures are unchanged;
these data establish no Windows code-layout, allocator or particular call-route mechanism.
Neither an import-only patch nor fewer Linux instructions proves no extraction cost.
Original R4 F64, R5 grid-confirmation and rejected reader-hint evidence remain valid.

Unrelated lane movement, control drift and every parent/tail observation remain in the
independent receipt. No sample is trimmed. Eight-case totals are not ranked against earlier
six-case scores. Both rounds still have 23/31 timing lanes and six of eight pipelines above
3x Assimp, with maximum endpoint ratios 103.90x / 104.86x and pipeline ratios 12.48x / 12.34x.
The near-0.18 ms unchanged preparation lifecycle remains a major open cost.

All 31 lanes' comparable Windows memory peaks are whole-child lifetime high-water marks,
not stage heap or retained payload size. No material memory breach is established, but
repeated commit increases remain explicit: STL import/preparation about 20--24 KiB, glTF1
extraction/pipeline about 14--24 KiB. Maximum median commit increase is 2.21%; maximum
working-set ratio is 1.67300x / 1.67194x. No storage optimization is credited to smaller
process-wide working-set medians, and no Linux memory result is implied.

| Report under `target/assimp-comparison/` | SHA-256 |
| --- | --- |
| `convergence-windows-gltf-profile-control/report.json` | `be735b2208525cde708236aac3b59e34254110ba97b4fc45c3d9c551f17704f8` |
| `convergence-windows-gltf-profile/report.json` | `a618a4b82eeabf54cbfe8e9b23338efd7938cf0cdc68e38c1ea44469e5cd7b2d` |
| `convergence-windows-gltf-profile-confirm/report.json` | `9aec75edf6212d51be8896a29de8ac77db86f78e225ffc89149c865098bed51f` |
| `convergence-windows-gltf-profile-control-confirm/report.json` | `3980488608732de85fac48fea254967d8e8f2f2c3ab61244a60b21f0a00b168c` |

The full independent disposition is
`.git/baozi-review/performance-convergence/gltf-profile-measurement-review.md`. Future
node-view, exclusive-context and payload-adoption sources need their own evidence; these
measurements cannot qualify their keep or retroactively repair a failed lane.

## Separate Refreshed Instruction Evidence

After ordinary collection, eight Linux whole-worker profiles use the same R7 source archive
and symbol binary `eb15424d6013c22c0353a01ddf0d84bb98fef66814cd47be1142e1c567ce44f1`.
Main reconciles every raw total with all exclusive flat rows using the unchanged owner
classification. Against earlier R5 profiles, glTF32 Ir falls 27.55%, 46,297,927 to 33,542,736;
the other five common workers change by less than 0.02%. Current glTF1/8 profiles have no
paired old profile and supply no relative improvement claim.

R5 also predates the consumer policy, so the instruction comparison is not a pure two-file
source contrast. These are whole-worker instruction owners including setup/checks/teardown,
not native CPU-stage percentages or Linux ordinary parity. Raw profiles and exact summary
remain under `target/review/convergence/gltf-profile-attribution/`, summary SHA-256
`2c1b96c758466f34d3bdb190504ec675ae9968af7967f208ab6ae964bc6d6ea5`.
This evidence does not remove any positive Windows lane or complete the release matrix.
