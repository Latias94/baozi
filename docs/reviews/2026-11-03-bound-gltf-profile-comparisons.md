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

Ordinary measurements remain pending at this source checkpoint. Reuse the retained qualified
pre-hint control at `509b80de4fff2494d5315f7a17e086cb7afe2f26`: after rejection of the reader
hints, maintained runtime bytes exactly matched that control before this two-file change.
Collect fresh paired ordinary campaigns for both workers, including shared glTF scales 1, 8
and 32 and the six previously selected cases under the unchanged comparison protocol.
Those eight-case aggregates must not be ranked against the earlier six-case totals.

Fresh source/binary identities, independent per-lane/control/memory review and separate
instruction attribution decide whether to keep this owner. The original F64/grid costs,
short no-op preparation gaps and full fourteen-case/110-lane Windows/Linux release gates
remain open. Passing metadata tests or reducing cold binding does not establish native speed.
