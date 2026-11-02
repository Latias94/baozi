---
title: Replace hot-path operation accounting and simplify format construction
type: perf
status: ready-for-ownership-design
date: 2026-11-02
---

# Operation and Import Architecture Continuation

## Authority and Goal

The user explicitly requested root-cause analysis before more optimization, and authorizes
breaking replacement of parser traits, registration, ownership and core representations.
This amends U13/U14 of the active
[pre-release evolution plan](2026-11-02-0923-refactor-pre-release-evolution-plan.md). Its already
completed product units remain valid at their recorded sources. Full B1 and final U14 execute
after this continuation; this document does not mark the original goal complete or authorize
publishing, push/PR or native-target promotion.

Goal: remove repeated interpretation/synchronization of pure work authority from tight loops,
use one authoritative operation owner, and expose a smaller coherent format construction
interface. Preserve supported scene semantics and meaningful resource/failure guarantees.
Retain measured changes and delete replaced implementations instead of adding compatibility
aliases or duplicate accounting modes.

## Grounding

The [diagnosis](../research/2026-11-02-parser-operation-cost-diagnosis.md) records the causal
Candidate 2 timing improvement, the complete current
[Windows 110-lane comparison](../reviews/2026-11-02-windows-native-performance.md), and the
Windows 6.7 ms/982,943-callback normal-kernel diagnostic versus the 190 ms complete preparation,
plus 127.0 ms for the same count of empty actions through the real operation protocol.
The remaining gap is large and not wholly attributed. Registry dispatch is coarse; uniform
streaming and zero intermediates are not Assimp's actual design.

Production control is `4455d03db190c6bef05ff5a1de024d2aac461465`. Its Windows and Linux immutable
binaries remain separate. Historical B0 is never overwritten. No speedup target is invented
from instruction fractions or a stripped diagnostic task.

## Decisions to Implement

1. **One local work owner, shared independent storage owners.** Work counters, policies,
   pending consumption and active scoped work live with the invocation. Storage reservations,
   physical payload destruction and independently surviving publication/output owners keep
   required synchronization. Work capability/permit types cannot outlive or cross their owner.
   Independent operations may execute concurrently.
2. **Bind once, then execute against dense positions.** Freeze branded work roles, duplicate
   groups and required permutations at operation/format-plan preparation. Inner work does not
   sort semantic keys, binary-search catalogs, clone a ledger Arc or acquire a storage mutex.
   A raw malformed/foreign key is rejected at the binding seam, not trusted by convention.
3. **One numeric kernel and one evidence authority.** Checked attempted/consumed/pending
   projection remains shared logic; a new local implementation must not record into a second
   legacy ledger. Snapshot/terminal composition joins local work and shared storage exactly
   once, under a coherent sealing/borrow rule. Allocation/evidence admission remains metered.
4. **Separate action accounting and control cadence.** Complete physical prefixes are
   recorded exactly. Cancellation/progress polling has a documented bounded cadence independent
   of each scalar arithmetic step. The initial design candidate is a small fixed geometry
   action bound and the existing bounded text-byte frontier; choose exact constants from
   worst-case action cost and current callback semantics in P1. Keep a final mandatory poll.
   No speculative precharge or deferred replay of a chunk's work is allowed.
5. **A format owns its typed options; an import plan executes itself.** Keep runtime erasure
   at real registration/detection/invocation seams. A private adapter checks types once and
   calls a concrete synchronous parse context. A plan retains its registry and compiled
   options; executing a foreign plan on another importer becomes unrepresentable.
6. **Host-owned scene construction and publication.** A parser synchronously submits or
   finishes into its context and returns `ParseResult<()>`. Candidate publication stays
   private; public `SceneDraft` is removed. Ordinary omitted domains/provenance defaults come
   from one builder. Borrowed inputs remain useful for small custom formats. Transfer of large
   payload ownership is added only for measured compatible canonical layouts.

The current low-level generic ledger is not a compatibility requirement. Audit actual callers,
then replace or internalize its unrestricted shared-work surface. Do not keep two permanent
runtime backends to avoid making the ownership decision. Existing metadata-based governance
checks remain metadata checks; no source/call-graph proof framework is introduced.

## Required Semantics and Deliberate Breaking Changes

| Preserve as a requirement | May deliberately change and must be documented |
| --- | --- |
| Admission before physical action; representable rejected attempted work is non-refundable; overflow retains last valid state | Public trait/type/method names, request execution pairing and raw ledger exposure |
| Atomic direct consumed groups; pending headroom; truthful physical completion prefixes and suffix release | Exact historical cancellation/progress callback counts, with a replacement bounded cadence and final poll |
| Foreign-domain and invalid-binding rejection; latched failures and semantic rejection precedence | Where invalid unbound runtime keys can be expressed; bound interfaces remove impossible misuse |
| No guard or conflicting mutable borrow across user control, parser callback, allocator or user drop | Internal session/host layering, temporary vectors and permit ownership |
| Independent storage drop/resize and physical destruction order; coherent final evidence | Obsolete object-safety/key-hook tests and migration aliases |
| F32/F64/sparse/indexed source semantics, selection closure, dependency authority and detached ownership | Overly coupled complete authoring descriptor shape, with one replacement module |

Do not equate compatibility with correctness. Review each changed expectation against the new
contract. Retain independent numeric/semantic oracles, and replace tests that only preserve an
obsolete shape. Do not silently weaken budgets, malformed-input rejection or evidence truth.

## Execution Units

### P0. Freeze evidence and choose the attribution seam

Retain the full Windows report, Linux Candidate 2 six-lane disposition and manual kernel
diagnostic with source/build/binary hashes, samples and limitations. Run/inspect the manual
diagnostic as an explicit optimized test, not an ordinary timing assertion. Record initial
whole operation versus preallocated kernel differences without treating subtraction as exact
accounting CPU time. The original B0 and invalid filesystem/profile experiments remain intact.

### P1. Review the work/storage split and bounded control contract

Write the exact state ownership/lifetime transition and actual cross-thread caller inventory
before changing it. Cover active producer, storage/drop owners, internal share work, fallback,
snapshot, publication and terminal detach. Decide the fixed control cadence and which current
poll-sensitive tests express essential precedence versus historical scheduling.

The reviewed gate must explicitly cover mixed dimensions at field level: attempted/consumed
and their policies are local; live/commit/publication fields and policies retain their own
authority. Policy tightening and combined observations cannot read stale duplicated policies.
Explain controlled builders borrowing the same work owner as enclosing handle-share work,
parent/child scopes, persistent outstanding/pending/fault state after `mem::forget`, protected
terminal work after ordinary sealing, and intermediate snapshots during independent storage
drop. P1 may revise or reject this split if those constraints cannot be met coherently.

The [reviewed bounded-control contract](../research/2026-11-02-bounded-operation-control-design.md)
specifies 256 bounded library geometry actions, the existing 4,096-byte text frontier, mandatory
external/parent/final checkpoints and exact failure reconciliation. It explicitly replaces
deferred text completion with truthful local completion and does not promise elapsed-time
cancellation for blocking operations. Its review is closed; ownership integration remains P1.

Scope: `baozi-operation` ledger/coordinator/retained/concrete-store owners and relevant ADRs;
`baozi-import` callback/session ownership; postprocess runtime. Review correctness and
feasibility independently. A design that requires a guard across callbacks or double recording
does not proceed. The exact public representation remains reviewable Rust types, not a policy DSL.

### P2. Replace the work engine and all internal work-bearing callers

Implement the single authoritative local numeric work owner, branded bound roles/groups,
short-lived action admission/completion and coherent evidence sealing. Migrate parser direct
groups, postprocess units, canonical semantic units, retained-share/handle work and terminal
helpers. Independent storage/physical/publication owners keep the shared state they need.
Remove the replaced work route when its callers migrate; avoid a permanent dual owner.

Use existing operation numeric, atomicity, partial completion, overflow, poison/drop,
publication and cross-thread tests. Add focused lifetime/foreign-binding and split-snapshot
tests only for the new risks. Verify real callbacks can still allocate/drop storage, observe
progress and fail without borrow panics or lock inversion. Forgotten scoped work must still
prevent an untruthful successful finalization.

### P3. Apply the new execution/control seam to import and preparation

Replace repeated whole-session host projection and fixed-group sorting with invocation-bound
work capabilities. Use the same seam in normal initialization, kernel callbacks and canonical
semantic work. Poll user controls at the reviewed bound and before terminal success, outside
all conflicting borrows. Actions continue to update attempted/completed state truthfully.

Run exact output, resource-limit, callback failure, cancellation, unwind, selection and detached
ownership tests across actual formats/postprocess. Update poll-schedule-specific tests to the
new bound and retain early/late failure ordering controls. No parser may bypass the work owner.

First measure Windows repeated prepare/import and grid import with the unchanged ordinary
collector. Require the mechanism to remove hot work locks/searches/Arc ownership and produce
a native improvement beyond dispersion, with a stable Assimp control and no semantic loss.
If it does not, diagnose the remaining phase instead of expanding the accounting machinery.

### P4. Replace the cross-format construction interface

Introduce typed format definition/options, a private erased registry adapter, self-executing
immutable plan and concrete parse context. Migrate STL/OBJ/PLY/glTF and real custom importers
atomically. Remove separate public runtime option-key hooks, parse-side downcast protocol,
`SceneDraft`, arbitrary importer/plan pairing and their compatibility surfaces.

Keep bounded deterministic detection, pre-IO option validation, registry poison/panic
containment and concurrent independent invocation. Host scene construction supplies omitted
domains/default provenance and retains one complete-input route for complex features. Do not
introduce a static built-in fast path alongside custom parsing. This interface change has
correctness/locality acceptance independent of any unmeasured vcall speedup claim.

### P5. Measure remaining text, construction and reader costs; replace only demonstrated waste

Attribute normalization/text planning, format parsing, source authoring, canonical
materialization/validation and detached reading on current large OBJ plus representative
STL/glTF/PLY. Use small local phase spans or a qualified profiler; no new generic benchmark
framework. Do not reuse an unreliable timer profile as CPU percentages.

Concrete candidate seams are borrowed validated UTF-8 with bounded forward line traversal,
reused line/token storage, flat face/corner construction, host-owned compatible numeric payloads
and bound typed attribute views. Preserve raw/recovered ranges, malformed-input outcomes,
precision, authored ordering and source scope. Full streaming is optional and must address
actual IO/lifetime requirements. Counting passes or allocations alone does not qualify a rewrite.
If these remaining costs are small after P3, omit the corresponding implementation and record
that decision; this does not create another generic scene graph or decoder.

### P6. Complete native comparison and the original final qualification

Retain per-change Windows measured variants against `4455d03`, then run the unchanged complete
Windows series. Refresh full Linux B1 and separate heap observations against immutable B0,
including every breach disposition under the frozen policy. Respect host/filesystem/build
identity and keep Windows working set/commit separate from Linux RSS.

Execute the original U14 matrix once on final integrated source: serialized Rust 1.95 host
nextest/Clippy/fmt/doctests/docs, direct/excluded consumers, unchanged-package consumers without
root patches, browser compilation, actual WASI, scoped ASan/fuzz and named Vulkan readback.
The already prepared commands are execution aids, not passing evidence. Close independent
findings, source-qualified documents and scoped local Conventional Commits before completing
the original goal. No publishing, push or target promotion follows.

## Review and Acceptance

- Confirm P1's ownership and control decisions before implementation. Review the replacement,
  not how faithfully it preserves obsolete wrappers.
- Each retained performance result has the same actual task, source bytes, precision, output,
  resource policy and ordinary build controls. Diagnostic ablations remain separately labelled.
- New work mutation has no repeated catalog lookup/sort/shared accounting lock/permit Arc clone
  per inner action. Independent storage operations may still synchronize.
- Final evidence has one owner and a coherent snapshot; no delayed work replay, silent
  counter loss, relaxed malformed input or lost physical completion is accepted as a speedup.
- Trait and storage changes remove their predecessors and migrate meaningful real consumer tests.
- A smaller safe implementation that meets these criteria wins over a generalized framework.
  Necessary breaking changes and rewrites are explicitly authorized; arbitrary scope expansion
  and claims of full Assimp/glTF parity are not.

## Initial Independent Review (2026-11-02)

Feasibility review approves the bounded P1 investigation and finds no established blocker to
the direction. It does **not** approve a work-engine rewrite before the ownership/cadence sketch
is reviewed. Evidence review verifies the complete Windows and six-lane Linux samples, both
manual diagnostic methods and their limits; its historical instruction-total precision finding
is corrected in the diagnosis. Both diagnostics were then executed together and retained with
their exact source/binary, so the earlier kernel receipt is not reused for an expanded test file.
No production work-owner replacement is claimed by this planning receipt.
