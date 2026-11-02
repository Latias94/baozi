---
status: accepted
date: 2026-08-19
updated: 2026-11-02
authors:
  - Codex
decision_type: architecture
supersedes:
  - docs/adr/0022-per-import-resource-ledger-and-budget-accounting.md
evidence:
  - docs/plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md
  - docs/knowledge/engineering/verification/2026-07-10T023259Z-baozi-canonical-architecture-plan-review-passed.md
  - docs/knowledge/engineering/verification/2026-08-19T152712Z-post-u4-authority-reset-review.md
  - docs/conformance/contracts.toml
  - crates/baozi-operation/src/ledger.rs
  - crates/baozi-operation/src/ledger/work.rs
  - crates/baozi-operation/src/ledger/evidence.rs
  - crates/baozi-operation/src/concrete_store/activation/preflight.rs
  - docs/research/2026-11-02-typed-format-context-design.md
  - crates/baozi-import/src/scope.rs
  - crates/baozi-import/src/metered_storage.rs
  - crates/baozi-import/src/store.rs
  - crates/baozi-import/src/store/execution.rs
related:
  - docs/adr/0002-runtime-concurrency-observability.md
  - docs/adr/0009-data-ownership-zero-copy-lifetimes-and-memory-layout.md
  - docs/adr/0033-operation-control-and-observability.md
  - docs/adr/0044-retained-physical-storage-ownership-and-account-lineage.md
  - docs/adr/0045-canonical-scene-ownership-construction-and-generation.md
  - docs/adr/0046-import-session-lifecycle-and-outcome-ownership.md
---

# ADR 0047: Operation Ledger Evidence and Resource Accounting

## Status and Authority

This accepted ADR is the sole authority for operation attempted, consumed, live, peak, committed,
terminal-evidence, and phase-accounting semantics. It supersedes ADR 0022, whose private typestates,
complete dimension enumerations, byte transcripts, and migration-era command sequences remain
historical implementation context rather than architectural authority.

ADR 0044 separately owns retained physical allocation lifetime and account lineage. Domain crates
own their phase and dimension vocabularies. This ADR owns the common accounting meaning and the
rules that make evidence non-refundable and fail closed.

Acceptance does not itself claim implementation conformance, format maturity, or release readiness.

## Context

Local limits do not bound aggregate hostile work or simultaneous memory. A refundable counter lets
repeated failed attempts erase attacker-controlled work. A debit-only byte counter cannot describe
temporary live storage, peak use, retained output, or published failure evidence. Sharing physical
storage across Scene generations further requires logical operation evidence to remain distinct from
allocation ownership.

Baozi therefore needs one small accounting model shared by import, standalone Scene construction,
and postprocess without exposing a generic resource policy to format code.

## Decision

### One Ledger per Operation

Every admitted import, standalone canonical construction, and postprocess invocation owns one
operation ledger. Distinct operations never merge their histories into one ambiguous total. A facade
may present several snapshots together but cannot add counters with different phases or meanings.

`baozi-operation` owns the runtime-neutral accounting mechanism. The operation-owning crate defines
the closed phase and dimension vocabulary, limits, public interpretation, and typed outcome. Format
code can use only capabilities issued by that owner.

### Invocation Work and Independent Storage

One invocation owns attempted, consumed, pending-consumption, work-policy and scoped-work state.
`LocalWork` uses short numeric `RefCell` borrows. Bound roles and linear envelopes borrow that actual
owner; they retain neither a borrow guard nor reference-counted work ownership across an action.
Storage reservations and physical payload owners retain independently synchronized storage state
and may outlive or be released independently of the invocation. There is no shared-work backend or
second mutable accounting copy.

A dimension that accounts for both work and storage is split by field. Work limits accompany local
work; live, publication and commitment limits accompany storage. Tightening a mixed policy observes
and updates both halves in one joint transaction. Immutable identity catalogues and dense positions
are prepared before numeric guards; bound numeric actions do not compare owner keys or search them.

Completion does not itself end scoped ownership. A fully completed envelope must still explicitly
finish. Forgetting a permit, including one with zero remaining units, leaves persistent ownership
that rejects terminal finalization. Trusted wrappers may explicitly resolve a verified unfinished
suffix during error or unwind cleanup while retaining attempted work, completed prefixes and the
accepted failure. Raw abandonment and conflicting-borrow faults remain fail-closed.

Numeric work and shared storage guards do not cross allocator entry, owner-key comparisons,
extension calls or payload destruction. Interim snapshots first admit and allocate their backing,
then borrow current work and lock storage for one coherent copy into the qualified buffers. Cleanup
subtracts only the snapshot's own admission from current counters; it never restores an earlier
global baseline. The outgoing snapshot's backing is excluded from current ledger-owned evidence
while its construction overlap remains in the peak. Final composition uses preadmitted staging and
retains the metered catalogue owners through the observation interval.

### Import Execution Regions and Storage Identity

`ParseContext::with_execution` lends `ParseExecution<'region, 'session>` through a higher-ranked
callback whose result cannot depend on `'region`. The execution view borrows the existing
invocation's disjoint work, storage, and control capabilities. It creates no new ledger, numeric
backend, pending-work cache, or reference-counted work owner. The startup-admitted state slot is an
ordinary exclusive value; no whole-owner `RefCell` guard or saved session host mediates parser work
or container growth. The short numeric `LocalWork` borrows above remain part of the sole mechanism.
ADR 0046 owns capability issuance and the outer submission lifecycle.

Bind an `ImportWorkGroup<'region, N>` for the actual repeated dimensions at region or loop entry.
`with_work` and `with_parent_work` receive that group and pass both execution and the existing
`ImportWorkScope<'region, N>` to their callbacks. These are the invocation's real parser channels,
not observed-work envelopes that preconsume unused headroom. Cold convenience adapters may bind
once for a whole cold operation. Ordinary children remain non-nestable; a semantic parent may admit
sequential children through the explicit executor. Group admission preserves caller-ordered
attempted/item-limit checks; completion across the admitted dimensions remains atomic.

Complete each physical prefix immediately. Poll before automatic parent completion and before a
region returns success. Callback errors, incomplete returns, and cancellation retain the existing
cause precedence; cleanup may resolve only a verified suffix. Work groups, scopes, and control
cannot escape the region or cross threads. A forgotten scope remains outstanding in the same owner,
including a zero-sized or fully completed scope, and prevents region success and terminal freeze.

`MeteredVec`, `MeteredString`, `MeteredOrderedMap`, and `MeteredDecodedText` own their values and
capacity reservations together. Their independent identity uses the session's existing issuer and
terminal latch, without borrowing owner state, vocabulary, context, execution, or control. Such
containers may leave one execution region and be reused in another from the same session, or supply
borrowed authoring input after the region ends. Growth and metered copying receive the current
`ParseExecution` explicitly and verify issuer identity before allocation or accounting mutation.
Reserved writes need no executor when they only fill already admitted capacity: they do not enter
the allocator or introduce a hidden poll after source-unit completion. Their caller still owns the
bounded work and control boundary. Decoded logical bytes and allocator slack remain distinct.

Text advancement and recovery, source-scope observation/finalization, dependency acquisition, graph
updates, and hashing receive execution explicitly. Cursors and evidence builders retain genuine
source/profile/issuer state and owned reservations; dependency handles retain actual IO/issuer
lifetimes and snapshot owners. None reacquires a whole-session host or uses a context-address token.
Ending a region releases no container capacity by itself. The actual payload is destroyed before
its reservation. Temporary parser authoring storage is released before terminal freeze; retained
failure and outcome evidence keeps its actual owners under the rules below. Submission uses the
same dormant operation after the relevant region borrows end, without a second construction ledger.

### Evidence Planes

```mermaid
flowchart TB
    Admission[Attempt admission] --> Attempted[Monotonic attempted]
    Admission --> Action[Execute bounded action]
    Action -->|completed| Consumed[Monotonic consumed]
    Action -->|panic or not started| Prefix[Preserve completed prefix only]
    Reserve[Live reservation] --> Live[Current live]
    Live --> Peak[Monotonic peak]
    Live -->|verified release| Released[Decrease current live]
    Publish[Prepared terminal publication] --> Committed[Committed canonical output]
    Publish --> Evidence[Published outcome evidence]
```

The planes are non-interchangeable:

| Plane | Meaning | Refund rule |
| --- | --- | --- |
| Attempted | admitted work or item amount before the action | never decreases |
| Consumed | action units that completed, including a completed semantic rejection | never decreases |
| Live | currently owned temporary or attributable storage | decreases only after verified release |
| Peak | greatest live amount observed | never decreases |
| Committed output | logical canonical output published by the operation | written only at terminal preparation |
| Outcome evidence | protected terminal evidence published by the operation | written only through the terminal owner |

Attempted and consumed are not reservations. Live storage is not work. Committed logical output is
not a physical allocation owner. Published failure evidence is not ordinary parser capacity.

### Admission and Completed Prefixes

Known work is admitted before its action. Multi-dimension admission is atomic where one semantic
action requires all dimensions. A failed admission executes no action and records no fabricated
consumption.

Work completion is action-scoped. Normal return consumes the completed unit even when it reports a
domain rejection. Panic, cancellation observed before action start, or an action that never returns
does not consume that unit. Previously completed units are never refunded. Chunked work completes
each real chunk before admitting or polling the next one.

An exact semantic envelope is admitted once before canonical materialization or validation. Actual
actions consume that envelope through scoped units. Under-run, overrun, wrong dimension, or mismatched
description is a contract failure and cannot be repaired by caller-provided counts.

Asset snapshot byte admission grants reusable payload capacity within the operation. Legal short
reads consume only observed bytes and carry unused admitted capacity into later reads without
refunding or repeating attempted charges. Serial acquisition also carries unused capacity into later
resources by deriving each dimension's balance from `attempted - consumed`; no second budget is
created. Resource-class and aggregate dimensions are topped up independently, so their attempted
envelopes need not add up after credit crosses a resource-class boundary. Rejected attempted charges
are evidence of failure, never reusable credit. Each read call separately admits one `WorkUnits` action;
normal return, including EOF or a returned read error, completes that action, while panic leaves
only the earlier completed call prefix. Cooperative cancellation is checked before the next call.

Opaque asset open is a separately weighted action. `AssetIo::open_work_bound` declares a positive
conservative billing weight for the exact backend/path without IO or authority lookup. The owner
admits that weight before calling open and completes it on a normal `Ok` or `Err` return; unwind
completes none of it. Consumed `WorkUnits` here records the completed declared weight, not measured
backend iterations or syscalls. Native weights must cover the bounded path/link/retry implementation;
they do not certify RSS or wall time. The distinct `AssetOpen` counter completes only after a
returned object passes authority/path checks, so a returned open error consumes billed work but no
successful open. Cache hits and direct primary bytes perform no new open action. Bound errors and
global work rejection are terminal and cannot become optional-input omission. Snapshot byte counts
continue to consume only actually observed/copied bytes.

Once payload credit reaches a resource/backend or aggregate byte limit, one nonempty one-byte probe
may validate EOF. A zero result succeeds; a positive result causes a typed limit failure and is never
stored as payload. The probe is validation/read work rather than payload storage, and cannot enlarge
the successful snapshot's admitted byte envelope. Truthfully observed excess may advance existing
byte counters before rejection; a rejected attempted charge never fabricates consumed bytes or
erases previously completed consumption. Exact-length contract errors remain backend failures.

Input-byte ceilings constrain payload counters, independently of allocator capacity. Snapshot
capacity, allocator slack, and the temporary overlap of an admitted replacement buffer use the
existing `max_live_bytes` ceiling. Tightening a file's byte limit must not lower the live-memory
ceiling to that file length or reject an otherwise legal short-read stream during buffer growth.

### Live Storage

Live capacity is reserved before allocation or growth, bound to one operation/session owner, and
resized to the actual admitted capacity. Release occurs only after the owned allocation or evidence
has been destroyed and owner identity has been verified. `mem::forget`, wrong-owner release,
uncertain deallocation, or baseline drift fails closed rather than optimistically refunding.

Temporary parser storage uses one `baozi-import`-owned metered-storage module. Format crates may wrap
it semantically but cannot implement competing reserve/grow/shrink logic. Canonical retained storage
uses ADR 0044's physical owner and account; the operation ledger records why new retained storage was
created, not how long every shared allocation remains alive.

The same retained mechanism applies to import evidence and resource manifests. Graph-owned paths,
projected manifest paths, provisional quotient tables, and canonical hash scratch overlap until their
actual owners are destroyed. Sealing a manifest freezes its semantics; it does not release arrays or
path storage that remain in the provisional manifest during Scene construction.

Allocation-bound output attribution keeps its original phase/dimension placement. The central ledger
accumulates it in its existing fixed catalog rows while owners retain their physical leases. A weak
edge to that existing ledger allocation permits exact pre-publication rollback without allocating a
shared header per payload. No terminal reservation array grows with the maximum resource-relation
count. The joint terminal transition validates these amounts with its existing batches, then retires
operation attribution. The immutable terminal snapshot can therefore report zero operation-live
bytes while retained data remains truthfully charged to its physical account. An early token drop
without an allocation-bound physical owner is not a transfer.

Import's retained admission registers output attribution before allocating its evidence storage.
The containing owners keep the same reservations through terminal preparation, so this path does
not require a second unmetered walk over every diagnostic or manifest relation at finalization.
Transient graph/hash storage still uses ordinary reservations and is destroyed before the snapshot.
An operation-live zero at terminal therefore does not assert that physical evidence has been freed.

### Terminal Accounting

Each operation pre-admits a bounded protected terminal allowance inside the caller's hard total.
Ordinary work cannot spend that authority. Terminal preparation consumes it to publish rich failure
evidence or the minimal fail-closed fallback. Handoff exposes already prepared owners and performs no
new accounting mutation.

Recoverable central evidence-construction or validation failures return an unprepared linear
fallback owner when the call may still borrow caller-owned charged scratch. The caller ends those
borrows, destroys scratch, and only then prepares the protected terminal evidence. Deferring that
freeze preserves the original failure and cleanup protocol; it does not refund work, relax terminal
eligibility, or turn uncertain cleanup into an ordinary failure. Import retains this continuation
until parser storage is released, and geometry edits release their generated values before using it.

Terminal cause is sticky and monotonic. A later contract, control, product-graph, or ledger failure
cannot be hidden by an earlier requested rejection. Public snapshots expose domain-owned semantic
views, not raw mutable ledger keys or control internals.

## Alternatives Considered

### Option A: Independent local limits only

This cannot bound aggregate work, repeated dependency attempts, or simultaneous memory. Rejected.

### Option B: One refundable bytes-used counter

This conflates work, memory, output, and evidence and allows failed hostile attempts to regain budget.
Rejected.

### Option C: Separate attempted, consumed, live, peak, committed, and evidence planes

This preserves non-refundable work prefixes, exact memory lifetime, and terminal evidence while
keeping physical allocation ownership separate. Chosen.

### Ordinary Handle Work

Ordinary retained-handle cloning and construction consume the operation's general work budget.
Their envelope is independent of the number of record-store Scene members: resource acquisition,
manifest ownership, and report ownership also share retained handles. Scene structural-work limits
cover the actual record-store assembly and publication work. A typed-domain cutover can reduce that
record count without reducing the work admitted for unrelated resource ownership.

## Consequences

Positive:

- hostile retries cannot erase attempted or consumed work;
- current and peak memory remain observable independently of work;
- failure, panic, and cancellation preserve exact completed prefixes;
- terminal evidence remains available without unbounded emergency allocation; and
- physical sharing does not rewrite completed operation evidence.

Negative:

- every operation owner must define a complete phase/dimension vocabulary;
- actions must report completion at real semantic boundaries; and
- uncertain destruction or ownership mismatch deliberately causes containment.

## Risks and Mitigations

| Risk | Severity | Mitigation |
| --- | --- | --- |
| Work is charged before the action and falsely consumed | Critical | separate attempted admission from action-scoped completion |
| Panic refunds completed work or consumes the panicking unit | Critical | finalize only returned units and retain prior prefix |
| Item limits are bypassed by repeated replenishment | High | bind item limits to one logical item and reject unqualified replenishment |
| Allocation occurs before budget admission | Critical | reserve and validate ownership before allocation/growth |
| A container retains the executor or grows through a foreign session | High | retain an independent issuer/latch and require a matching explicit execution for growth |
| A region leaks work or silently closes a forgotten scope | Critical | use higher-ranked region lifetimes and verify outstanding ownership before region success and terminal freeze |
| Live bytes are refunded while storage remains reachable | Critical | release only after verified owner destruction; contain uncertain release |
| Shared retained storage is charged once per Scene | High | separate operation attribution from ADR 0044 physical ownership |
| Ordinary work exhausts failure reporting | High | protect a bounded terminal allowance inside the hard total |
| Raw ledger keys become public policy | High | expose only domain-owned dimensions and immutable snapshots |

## Success Metrics

| Metric | Target | Measurement |
| --- | --- | --- |
| Failed admission side effects | zero action calls and zero consumed units | exact-limit and one-short tests |
| Panic prefix | consumed equals the exact normally returned prefix | caught-panic action tests |
| Cancellation prefix | last completed chunk is retained and next action is not run | chunk-boundary cancellation tests |
| Live release | current live and active reservations reach zero after owner drop | allocation/drop-order tests |
| Peak accuracy | peak equals the greatest simultaneously owned admitted capacity | staged allocation tests |
| Semantic envelope | actual consumed work equals the sealed description on success | canonical construction/validation tests |
| Region ownership | work/control cannot escape or cross threads; session-bound containers can leave a region | compile-fail and compile-pass capability tests |
| Parser scope completion | forgotten zero/full scopes and incomplete callbacks cannot yield success | parser work and finalization contract tests |
| Container lifetime | same-session growth remains admitted; foreign-session growth has no allocation side effects; payload drops before reservation | metered-storage identity, growth, and drop tests |
| Physical/logical separation | shared payload is physically charged once without changing operation evidence | multi-generation lineage tests |
| Terminal availability | bounded rich failure or minimal fallback exists for every safely finalizable operation | terminal-budget matrix |

## Acceptance Criteria

- Every admitted operation has one ledger and one domain-owned phase/dimension vocabulary.
- Import execution borrows disjoint capabilities from an ordinary admitted state slot, without a
  whole-owner `RefCell`, saved parser host, or second work backend.
- Region-bound parser groups/scopes use the existing channels and preserve ordered admission,
  completed prefixes, parent/child rules, failure precedence, and outstanding-scope detection.
- Containers retain independent session identity and actual reservations, receive execution for
  growth, and may outlive a region without retaining work or control authority.
- Attempted, consumed, live, peak, committed output, and outcome evidence remain distinct.
- Attempted and consumed counters never decrease; completed work is never refunded.
- An action consumes only after normal return, including a returned semantic rejection.
- Live storage is admitted before allocation and released only after verified destruction.
- Semantic work comes from one sealed description and cannot be paired with arbitrary execution.
- Protected terminal work is bounded, included in the hard total, and inaccessible to ordinary work.
- Retained physical ownership follows ADR 0044 and cannot be reconstructed from ledger counters.
- Public outcomes expose immutable semantic evidence, not raw ledger or retained-store authority.

## Amendment History

### 2026-11-02: Separate import execution borrows from metered storage identity

- Affected: `Decision / Invocation Work and Independent Storage`, `Import Execution Regions and
  Storage Identity`, and their risks, success metrics, and acceptance criteria.
- Previous: the invocation-local work/storage split did not specify the replacement for import's
  whole-owner host projection or the region-independent identity of parser containers.
- New: higher-ranked `ParseExecution` regions borrow the existing disjoint capabilities; parser
  groups use the real work channels and containers retain an independent session issuer/latch.
  Growth receives execution explicitly; the state slot remains an ordinary admitted value.
- Impact: parser work and storage call sites change, while attempted/consumed prefixes, item limits,
  outstanding-scope detection, control precedence, capacity qualification, protected terminal work,
  payload-before-reservation destruction, and ADR 0044 physical ownership remain required.
- Review basis: the [typed-format/context design](../research/2026-11-02-typed-format-context-design.md),
  including its independent feasibility review, and the operation/import sources above. Compiler,
  runtime, memory, and performance qualification remain separate integration work.
- Rationale: this refines how import borrows the same ledger and storage owners; it does not select
  a new accounting model or change any evidence plane.
