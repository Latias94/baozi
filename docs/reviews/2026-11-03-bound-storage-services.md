# Bound Storage and Control Services

Date: 2026-11-03. Runtime base:
`48d6f17e2f0829bdd5d50c8d7c48cea72ce343a1`; plan commit:
`29c269c35b90f221fae77c9845194cf73a825a1e`.
This is the implementation receipt for the
[bound services plan](../plans/2026-11-03-1800-refactor-bound-storage-services-plan.md).
Runtime is source-qualified on Windows at
`314d0fe3855cdd53159d2ffc0d6e10206d299b9f`. The ordinary worker is prepared separately;
no native performance disposition belongs to this receipt yet.

## Owner Replacement

The [matched call-owner investigation](../research/2026-11-03-ordinary-storage-binding-costs.md)
locates repeated construction of ordinary storage/poll descriptors in legacy growing
container calls. The replacement binds that service once at region entry and lends
a short mutable reference to its field. No numeric, normal-generation, parser,
capacity-growth or comparison algorithm changes.

- `StoreExecutionServices` replaces the separate `StoreExecutionStorage` and
  `StoreExecutionControl` integration views. Entry returns work plus services.
  Arbitrary storage/control sibling joining is removed; foreign key and reservation
  ownership remain runtime rejection contracts.
- `StoreOrdinaryStorage<'b, 'op, ...>` borrows the bound raw ordinary field. Its
  construction and reborrow copy only the small wrapper references; they neither
  reconstruct inner storage/poll/state descriptors nor poll.
- `OperationProgressState` owns metadata borrows separately from the unique actual
  control/terminal borrow. Both reporting routes call one reporting body. The outer
  execution checkpoint and the reporting poll are retained, as are evidence growth,
  observer limits, cancellation and panic disposal.
- Exclusive parts own disjoint work and ordinary fields. Failure/debt/state references
  reside only in ordinary, removing three duplicate references. Their actual Cells,
  physical account, Internal share plan and numeric authority remain the same.
- The private cold retained helper constructs its actual retained role directly.
  It preserves its prior eligibility, Internal-share admission and release behavior
  without obtaining/discarding a full ordinary execution tuple.
- Import separates temporary-loan, invariant operation and container-session
  lifetimes. Vocabulary is borrowed for the short loan. Canonical scratch,
  postprocess runtime and its normal diagnostic use the same services owner; their
  actual tasks, work counts, timers and numeric expressions are unchanged.

One vector/string growth implementation and one guarded compound executor remain.
Old storage/control types, sibling join helpers and unused descriptor-reborrow
methods are deleted. There is no compatibility adapter, second backend, additional
account/counter or dynamic dispatch. Existing state checks remain in this slice.

## Error, Resource and Borrow Contracts

Capacity-fit requests still poll. Source identity, old-plus-new replacement admission,
actual allocator-capacity qualification, UTF-8 copy frontiers, partial append and
payload-before-token retirement retain their existing order. Ordinary and retained
token reconciliation use the same helper and preserve physical envelopes.

The compound runner borrows work/frame and ordinary fields separately. Admission
rejection projects its cause before unused values/captures are disposed; rejected
completion projects before returned values drop. Accounted semantic failure still
materializes while the actual failed parser frame is active, with the action guard
disarmed. Exact first Control, caught/unhandled unwind, KnownError, prescribed exit
and final control behavior remain separate contracts.

Independent concrete-design review found one P2 before implementation: the proposed
vocabulary reference required the invariant operation lifetime instead of the short
loan. Corrected design v2, SHA-256
`03cffb70ee8da5592a9509f9ec7c1eaff2caabc8871d0742a6a767021e2d5401`,
uses the short lifetime without extending ParseViews, session or numeric borrows.
The independent v2 delta review closes that P2.

## Retained Failures and Qualification Checkpoints

The first operation-library compiler checkpoint exposes a missed progress-state
limit argument after control separation. Passing the existing `max_states` value
at the same insertion frontier fixes it. The next library check passes; three
unused old helpers are then removed.

The first unit compilation finds one private oracle's stale `parts.ledger` field;
only its owner path is updated. The following 535-test run passes 529 and fails six
existing source-shape tests because Python's Windows writes introduced CRLF into
their inspected source. Restoring the repository's LF bytes preserves every oracle;
the exact six-test rerun passes. No source parser or new interpretation is added.
All failed receipts remain under `target/review/convergence/bound-owner-*`.

After the initial full formatting, actual integrated nextest passes **2357/2357**,
six existing skips, run `fc5a8dd3-06a2-480f-9e66-df1db821edca`. All 21 changed Rust
paths match before and after. Import **34** and operation **56** doctests also pass.
These checkpoints precede two transparent private type aliases and the subsequent
test-only coverage expansion; their exact source identities remain recorded.

Strict all-target Clippy initially rejects two tuple signatures for type complexity.
Two private tuple aliases replace those signature spellings without changing fields,
function bodies or semantics. No lint suppression is added.

Independent core and caller source reviews find no P1/P2. The caller review identifies
two test-coverage gaps in the new foreign-token oracle. Its subsequent test-only matrix
covers ordinary/retained tokens across validation, ordinary resize, services resize
and transition. It asserts preserved logical/physical coverage on rejection, successful
source checkpoint, honest shrink/release, receiving first cause/no-poll behavior and
the source's final Requested/71 cause. The actual eight-combination test passes, run
`1875fc0e-f3ca-43b6-8be2-c683f83367ed`; independent test-delta review closes both gaps.

Final-current-source verification is:

| Check | Actual result |
| --- | --- |
| Operation/core/import/postprocess nextest | 2357/2357; six existing skips; `a7f2d170-e162-44f3-aaa0-36d53bfbfa5a` |
| glTF/OBJ/PLY/STL nextest | 855/855; no skips; `d1cf7e02-bcf4-4abf-92a4-5c59487d992b` |
| Facade control assurance | 9/9; no skips; `6fb41b10-9917-4313-94d4-db61c48b1174` |
| Independent asset consumer, all features | 138/138; one existing manual skip; `3cc67d85-fc4f-472b-b442-84ec16629936` |
| Affected and independent consumer strict Clippy | Both pass, all targets/features, `-D warnings` |
| Actual-method loan compiler probes | 11/11: six intended borrow/lifetime negatives and five positives |
| Formatting | Passed; final source identities remain unchanged through checks |

The full import 34 + operation 56 doctest checkpoint precedes only the two transparent
aliases and integration-test strengthening. Their independent delta review preserves
all bodies and compiler contracts; the eleven affected loan probes run on final bytes
with actual stderr. No negative relies on unresolved imports or a missing API.

All nine final checks in `target/review/convergence/bound-owner-main-qualification-1.json`
record identical 21-path before/after identities. Aggregate SHA-256 is
`5cc2b74363aa3d803999c6f3275fec90728ba98f465d3755e43b62240acdfe2a`.
The sorted UTF-8 `path + NUL + lowercase SHA-256 + LF` source-set identity is
`17126ed54615138f35e67720e30ff49e8bb3aa4fd5ecc732ed3bfefefaaa3bb2`.
`bound-owner-source-commit-1.json` verifies all 21 committed Git blobs against those
qualified bytes. The source reviews, formatted/alias/test deltas and actual compiler
reasons are retained under `.git/baozi-review/performance-convergence/bound-owner-*`.
The finite source-qualification bundle
`target/review/convergence/bound-owner-source-qualification-evidence-2.zip` contains
73 named evidence/source files, with every member rehashed. SHA-256 is
`bd3b1621fbcbb1d026dfc5a21fcaa7866e6fd9e8f77b8305d1ed85611a93aa7b`.
It preserves the failed checkpoints and supplied Windows gates; later ordinary and
Linux results are separate artifacts. Bundle 2 corrects bundle 1's scope metadata
by removing its one ordinary-worker identity member; the original bundle is retained.

Affected Linux qualification is pending. No ordinary candidate/control campaign has
run for this replacement at this source-qualification checkpoint.

## Performance Boundary

The earlier [binary STL performance HOLD](2026-11-03-binary-stl-performance-hold.md)
and all historical endpoint, memory and tail costs remain open. The located Linux
constructor instructions are not a promised removable saving or a Windows latency
explanation. Only fresh unchanged-task ordinary measurements can establish the next
performance disposition. The full Windows/Linux 14-case/110-lane release targets,
roots/metrics/consuming attachment and the separate glTF source-direction P2 remain
outside this implementation proof.
