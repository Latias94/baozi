# Resolve a preparation selection once and retain its actual readers

Status: source `6203ce2` is implemented and passes all seven integrated gates,
the prospective seven-task instruction predicate and independent actual
source/QA/archive, instruction and compiled-owner reviews. Both ordinary
candidate builds complete in the existing repository/target. Original
Windows/GNU ordinary and separate heap qualification remain pending;
native retention/main/release remain HOLD. See the
[actual checkpoint](../reviews/2026-11-09-prepare-selection-owner.md).

The fixed b918 ordinary attempt 2 ends incomplete before this implementation
starts; its missing candidate clocks remain unavailable. The prospective design
below is retained as the source and measurement contract. New workers use the
separate `6203ce2` identity and do not alter the earlier b918 evidence.

The exact private reviewed draft SHA-256 is
`425b6ea499ff7bf91682f9c77bfe3a86fbe683e3083e476657bb45d04db0b640`.
The independent `prepare-selection-owner-plan-review-1.md` report SHA-256 is
`70e03f6e3350725b52901369b32604774188039ce69dabbf0628a36ae281e457`.
Named study/review artifacts below remain under the local performance review
directory. The actual error/work study at review time hashes to
`ac8e0c4e0592c85d2b34704408ef2daefb4620d1f1de6b4935c89c123935faf0`;
the earlier supplied `c452fc...` observation is not relabeled byte-identical.
This is plan-level acceptance, not executed source/compiler/native acceptance.

## Problem and measured owner

The successful Preserve body in `prepare/input.rs` currently allocates a
sorted-ID scratch vector, copies/heapsorts opaque target IDs, resolves each
handle and discards its reader, then allocates a second selected-reader vector.
It walks every mesh/primitive, binary-searches membership and constructs the
selected readers again before duplicate/metadata handling. A one-target call
still allocates both scratch backings. `PreparedSelection::Unchanged` already
avoids changed-output descriptors and attachment; those earlier deletions are
not new opportunities.

The source/binary-bound existing Ir study
`finite-clean-preparation-existing-ir-study-1.md` identifies actual successful
`prepare::run` totals: GLB1 41,779, GLB32 425,543 and STL1 41,271 Ir. GLB32 has
900 actual action begin/complete calls each, costing 283,600 Ir, 66.64% of this
entry. Two scratch reservations execute per case. The original ledger
constructor is only 1,233 Ir. These are overlapping guest-instruction scopes,
not native elapsed shares, type sizes or a prediction of removable work.

The public Windows layout probe separately measures the actual invocation
error at 10,464 bytes and public Result at 12,688. It does not select a root
boxing change. Whole-root and attachment-only indirection still have unresolved
bootstrap/funding/retirement counterexamples. Keep this selection change at
the actual producer and reuse the existing execution and ownership machinery.

## Selected interface direction and completed Core proof

Core should explicitly own the within-Scene order of successfully resolved
primitive handles. Preferred small contract: for handles successfully resolved
by one current `ValidatedScene`, `PrimitiveId::Ord` agrees with that Scene's
logical mesh/primitive enumeration order. Cross-Scene order has no source-order
meaning; stale/foreign handles still require the actual Core resolver.

`prepare-selection-core-order-contract-study-1.md` independently establishes
this property from actual generation/mesh/local ordering, checked lookup and
enumeration paths, including mapping, edits and sharing. Select a scoped public
documentation guarantee on PrimitiveId only; no new comparator API or Core
lookup/storage change is required. Ord remains a lawful total order outside
the semantic scope. This guarantee concerns canonical Scene order, not source
file numeric IDs, physical buffer order or correspondence between generations.
Postprocess does not decode private IDs. No selection planner, callback/work
framework, identity certificate or cache is added.

## One private owner and one traversal of requested handles

Use a private concrete selection owner in Postprocess, with exactly the Scene
lifetime already carried by `PrimitiveRef`:

```text
SelectedPrimitives<'scene>
  Empty
  One([PrimitiveRef<'scene>; 1])
  Many(ScratchVec<PrimitiveRef<'scene>>)
```

It exposes one borrowed source-ordered slice to the existing metadata/effect
loop and unchanged-prefix backfill. This is a single-use concrete owner, not a
generic small-vector library. It holds no ledger, numerical counter, work
permit, admission token or cloned Scene/account. `Many` owns actual admitted
storage and drops before the existing final gate. Empty/One own no scratch
backing, reservation or storage-promotion token.

1. Preserve request/policy/source/start admission before inspection. Empty
   selects Empty and retains the original region/final gates.
2. Resolve a one-target request through one actual metered Core lookup into
   the inline One owner. Retain actual lookup error and geometry metadata gates.
3. For many targets, admit one requested-capacity scratch owner before
   allocator entry. Resolve each requested handle once in caller order; after
   a successful lookup, store that actual reader with a real metered append.
   No unmetered preliminary pass or second resolver is permitted.
4. Sort retained readers by the independently established Core source order.
   Preserve bounded comparison/swap actions and the same executor/panic guard.
   One private reader heapsort can replace the old ID heapsort initially; no
   custom general sorting framework or duplicate slow algorithm is retained.
5. Check adjacent duplicates after all requested handles resolve and before
   any selected topology/effect classification. Empty/One need no duplicate
   scan. Use the one ordered slice for the existing metadata, effect kernels,
   lazily constructed output and prefix-backfill behavior.
6. Delete the sorted-ID scratch owner, discarded-reader resolution loop,
   full-Scene selection traversal, membership binary search and retired ID-sort
   implementation together. Do not keep a compatibility branch or fallback.

## Explicit breaking error and work contracts

The candidate deliberately proposes **caller-order first Core lookup failure**
among requested handles. The old producer sorts opaque IDs before resolving;
multiple invalid handles can therefore choose a different first error. Document
this as the selected new contract, cover combined-invalid cases and check the
actual public error taxonomy. This does not change sticky first trusted
resource/control cause, the denial-before-action frontier, actual completed
prefix, original owned error/residue, or cleanup/final-control precedence.
All requested handles must resolve before DuplicateTarget or geometry
acceptance. Do not preserve old error ordering by adding another owner or
preliminary/repeated lookup pass.

The completed `prepare-selection-error-work-contract-study-1.md` confirms
that existing public docs/tests do not promise an order among multiple invalid
handles. Per-handle generation-before-range and owner/range precedence remain
unchanged. Foreign and stale selection handles both report GenerationMismatch
with distinct actual generations; do not invent a separate Foreign error or
an OwnerMismatch result for ordinary scene.primitive lookup. One metered lookup
finishes its numerical completion before the inner Core Result is projected.
New lookup/allocation/sort ordering can change which not-yet-observed denial
or error wins; preserve actual first-cause and completed-prefix behavior for
the new physical trace, not a replay of the retired algorithm.

Successful selected order, primitive membership, generated geometry, remaps,
report order, output capacity, final generation and source association must
remain exact. Unsupported/Preserve topology rejects before zero-face unchanged;
original normal/tangent/layout checks remain. This is not a Preserve success
bypass or a lower-budget exemption.

Charge actual new lookup, append, comparison, swap, duplicate and metadata
actions under the existing bounded action contract; delete charges only with
the corresponding removed physical work. Inline construction/result transport
is not a fake heap action. Storage growth/capacity is separately admitted and
reconciled. Do not pad the new algorithm to historical action totals, enlarge
budgets, defer numeric completion, cache headroom or suppress work comparison.
The `bounded-normal-actions/v2` identifier need not change merely because the
algorithm performs fewer real named actions; inspect its documented meaning
before any version/interface decision. Exact emitted `import_work` remains
unchanged: these are Postprocess actions after the consumer's import snapshot.

## Scope and verification

Expected runtime write set is Core identity/order documentation (or one
concrete comparator if required), Postprocess `prepare/input.rs`, the smallest
private selection child if it deepens that owner, and directly affected public
docs/tests. Existing action/runtime/ledger/physical/control kernels, normal
algorithms, Import/glTF parser, public result/error ownership, dependencies and
ordinary collectors remain outside the runtime write scope. Do not retain two
selection producers or start a resource/backing framework.

Meaningful oracles must cover: reversed many targets across logical meshes;
shared/edited Scene identities; every requested handle resolved before duplicate
or topology rejection; new caller-order combined-invalid selection; One/Empty
without selection backing or promotion; actual many-capacity denial; completed
sort/append/lookup prefixes at control/resource rejection; mid-output and
unchanged-prefix cleanup; final observer rejection after selection drop; and
existing original-source ownership through normal and panic exits. Retain
current assertions unless they explicitly describe removed actions/allocations.
Amend those count/cap premises prospectively, not by loosening an unrelated
first-cause, capacity or leak assertion after failure.

Concrete test migration from the completed study: singleton unchanged no
longer has selection staging, while a duplicate two-target request still has
real many-selection staging. The cleanup hook observes the remaining selected
owner and actual selected length, never a fictional sorted-ID allocation.
Delayed-output storage denial tracks three buffers instead of four, and
second-backfill denial four instead of five. Keep the exact retained backfill
delta attempted=before+2/consumed=before+1, actual live-capacity-plus-output
denial, exactly-once payload/prefix disposal and no attachment. The independent
attached cleanup family and synthetic action-kernel oracles remain unchanged.
Use an existing trusted Core fixture for range-invalid IDs; do not expose
forging or write a Rust/source verifier to manufacture a public impossible ID.

Run serial affected Core/Postprocess/consumer tests and strict lint, followed
by the original seven integrated gates for a source-frozen candidate. Preserve
all failed numbered attempts. Independent Core contract, producer/error/control
source and actual QA/archive reviews precede fixed-binary measurements.

## Prospective measurement and keep decision

Control for this next experiment is fully source-qualified b918; its missing
ordinary candidate samples/retention remain explicit. Do not infer that using
it as a fixed control retroactively accepts its speed. Commit a new candidate
only after exact source QA association and keep its worker identities distinct.

Use the same original seven pipeline/memory instruction tasks. Before candidate
sampling, freeze a small diagnostic addition that reads the exact public
`prepare::run` incoming edge from the same existing Callgrind files for GLB1,
GLB32 and STL1; this is offline attribution, not another worker or timing scope.
Prospective selection requires at least one of those actual prepare-entry Ir
totals to fall 10%, at least one corresponding complete-worker total to fall
0.2%, and none of all seven whole-worker totals to increase above 1%. Use
integer predicates (`10*N <= 9*B`, `500*N <= 499*B`, `100*N <= 101*B`) and keep
all signs. If exact-edge attribution cannot be made unambiguous, resolve that
before sampling or reject this diagnostic design; do not fit a predicate later.

These bounded target-owner screens do not weaken the original end-to-end
engineering targets. Ordinary Windows/GNU retain all 14 cases/110 lanes and
C0/N0/N1/C1, three warmups and twenty fresh retained AB/BA pairs, unchanged
clocks/statistics/corpus/Assimp/tasks, plus comparable whole-child peaks and
separate representative heap. Every complete raw non-clock observation,
including import_work and output capacities, must be literal-equal. Do not
reuse the glTF work-delta comparator to waive a new Postprocess mismatch.
Accept a runtime for speed only with material same-task ordinary improvement
beyond dispersion and no unexplained serious regression. Missing brackets,
load or failed gates remain missing/failed, not success. No native/source keep,
main merge or release GO follows from this draft or a diagnostic pass.

## Sequence

The Core ordering and error/work contract studies are complete. Review this
exact draft for correctness/minimality and incorporate concrete amendments. Record
the final contract and selection measurement before implementation. Finish
the fixed current b918 ordinary/heap disposition first; implementation starts
on a separate next branch with scoped Conventional Commits. No push, PR,
publication/yank or native promotion is included.
