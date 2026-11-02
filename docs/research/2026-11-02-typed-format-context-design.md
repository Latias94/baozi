---
title: Typed format invocations and exclusive parse contexts
date: 2026-11-02
status: accepted-for-implementation
---

# Typed Format and Parse Context Cutover

This implements the import portion of P3 together with P4 of the
[architecture continuation](../plans/2026-11-02-1700-perf-import-operation-architecture-plan.md).
The user explicitly authorizes breaking replacement. P1/P2's single invocation-local work
owner, independent synchronized storage owners, failure precedence and terminal protocol remain
requirements. No performance result or implementation completion is established by this design.
The separate P3 geometry/text foundation is being qualified before this atomic interface cutover.

## Concrete obstruction

`ImportStoreHost` currently projects work, control and storage through one
`RefCell<ImportStoreSessionState>`. `MeteredVec` and other containers retain that host.
Returning a bound work group beyond the projection is not a legal borrow. Holding the
projection across a callback conflicts with real nested storage and child-work calls; extracting
the owner instead makes the captured host unavailable. Adding another host route would preserve
the coupling that P3 intends to remove.

The replacement is one exclusive parse context. Execution regions borrow the existing owner's
disjoint capabilities in place. Containers own storage reservations and an independent session
brand; growth receives the current execution capability explicitly. Region borrows end before
the context consumes the dormant owner for canonical submission.

## Ownership and interfaces

| Owner | Holds | Does not hold |
| --- | --- | --- |
| `ImportPlan` | Immutable registry snapshot, resolved configuration, prepared typed invocations, account factory | Input, active work or mutable control |
| `ParseContext<'session>` | Exclusive session state access; independent source/services | A duplicated ledger or pending work deltas |
| `ParseExecution<'region, 'session>` | Existing borrowed work/storage/control views; explicit services access | Publication authority or a pointer back to the context |
| Bound group / work scope | Existing invocation-bound group/channel and exact admitted suffix | Whole-session host, shared work ownership or independent counters |
| Metered container | Values, reservation, class/resource and independent session identity | Context/executor reference or a saved host used for future growth |
| Pending canonical candidate | Existing operation-owned validated output and evidence | A descriptor sent through and returned by a format parser |

Representative signatures, subject to the actual production compile gate:

```rust
impl<'session> ParseContext<'session> {
    pub fn with_execution<R>(
        &mut self,
        action: impl for<'region> FnOnce(
            &mut ParseExecution<'region, 'session>,
        ) -> ParseResult<R>,
    ) -> ParseResult<R>;

    pub fn submit(&mut self, input: SceneBuilder<'_>) -> ParseResult<()>;
    pub fn submit_complete(&mut self, input: SceneAuthoringBatch<'_>) -> ParseResult<()>;
}
```

The result is independent of the region lifetime. Work groups, scopes and control cannot escape.
An owned metered container may leave a region because it borrows only the independent session
issuer, not the execution object. Immutable input and dependency-authority getters return their
actual session lifetimes, so reading source bytes does not borrow the whole mutable executor.
Keep existing payload-before-reservation destruction and capacity qualification.

Work helpers receive both executor and scope explicitly. A semantic parent may admit sequential
children through that executor; ordinary children remain non-nestable. Bind actual repeated
const-N groups at loop/region entry, preserving caller-ordered attempted/item-limit checks and
atomic completed groups. Use the existing parser channels, not observed envelopes that precharge
consumed headroom. Complete physical prefixes immediately. Poll before parent completion and
region success; reconcile callback errors and incomplete returns with existing precedence.
Forgotten zero/full scopes remain outstanding in the sole owner.

All state-slot borrows end before user callbacks, allocation or destruction. The context's
exclusive state slot replaces its current whole-owner `RefCell`. Keep startup-admitted storage,
contained transitions, the physical poison anchor and every retained terminal owner. Do not use
unsafe owner pointers, self-referential storage, `Rc`/`Arc` work or a second numeric backend.

## Typed registration and plans

Replace `FormatImporter` with `Format`, whose associated `Options` type is passed directly to
`parse(&self, &Self::Options, &mut ParseContext<'_>) -> ParseResult<()>`. Typed default/layer
compilation returns host-created `CompiledOptions<Self::Options>`. Keep bounded compilation
scopes, result-byte accounting, reservation completion checks, versioned wire envelopes,
schema validation and preset/caller presence semantics. No-options formats use a small metered
default helper; there is no unmetered allocating `Default` route.

Erase the prepared invocation privately: a generic adapter captures the concrete format and
its correctly typed options and exposes one internal parse method. This removes the public
codec/runtime-key agreement and parse-side downcast. Capture metadata once; preserve stable
registration/override ordering, default caching, panic containment, shared poison health and
racing-poison checks. Independent invocations remain concurrent. No built-in bypass is added.

Replace `PreparedImportRequest` with `ImportPlan`, with no compatibility alias. The plan already
owns the registry and executes its own `read_bytes`, `read_asset` and resolver variant.
Remove methods that pair another `Importer` with a plan and then reject a foreign registry at
runtime. Preserve account limits across clones. Default convenience methods compile and execute
the same plan implementation.

Remove public `SceneDraft` delivery. Parser success and the private pending candidate remain
separate facts: missing or second submission, parse error/panic after submission, swallowed
failure and racing extension poison must prevent success. Core retains root/brand/generation
validation at the existing freeze/handoff seam; a parser-held copy of those facts is unnecessary.
Do not introduce a second scene or another canonical validation/publication implementation.

## Concrete borrowing migrations

- Text cursors and recovery trackers retain source/profile/classification state. Advancement and
  diagnostic callbacks receive the executor explicitly. Preserve the newly immediate raw-unit
  completion, 4,096-byte frontier, BOM, CRLF, recovery and original source ranges.
- Source-scope builders retain independent issuer identity and evidence storage. Observations and
  finalization receive execution explicitly; remove their context/self-address token protocol.
- Dependency handles retain genuine IO/issuer/session lifetimes and owned snapshot storage.
  Cache, resource-graph, hashing and IO helpers receive execution; they do not reacquire a host.
- STL/PLY/OBJ decoding regions return owned parsed payloads, then build borrowed authoring tables
  in staged outer locals. Submit only after the relevant region ends.
- glTF keeps the normalized model outside authoring regions. Animation descriptor tables own
  channel/clip vectors borrowing that model's samples. Construct the final borrowed batch from
  outer tables after region exit. Do not return a batch borrowing a local vector or pack mutually
  borrowing tables into one self-referential prepared object.
- Reuse the current static-scene defaulting implementation for omitted domains and provenance.
  Keep one complete authoring input route for complex scenes.

Migrate the four built-ins, real custom formats, facade, tools, fuzz entries and meaningful tests
atomically. Delete replaced keys, codec hooks, draft exports, foreign importer/plan pairing and
host fallback paths. Historical source-qualified evidence remains unchanged.

## Entry and acceptance gates

Before distributing format edits, compile the actual capabilities in a concrete context that
binds a parser group, allocates/grows storage inside its callback, admits a child, returns an
owned session-branded container and then submits borrowed input after the region. Include a
complex glTF-style staged descriptor borrow. A standalone lifetime sketch is insufficient proof.

Retain runtime coverage for pre-IO option rejection, real format precision/selection/dependency
semantics, callback failure/control precedence, exact work/storage prefixes, forgotten scopes,
missing/duplicate submission and post-submission failure, extension poison and concurrent plans.
Compile-fail tests cover escaping/cross-thread work and removed public pairing; compile-pass
custom consumers demonstrate typed options, ordinary omitted defaults and detached output.

Independent feasibility review and this compile gate precede the broad cutover. Afterwards run
the affected regression/lint/documentation checks and ordinary Windows measurements. P5/P6,
full Linux B1/heap observations and final U14 remain required by the existing active goal.

## Independent feasibility review

The bounded independent review inspected this design, the detailed caller inventory and the
actual current operation, container, session, animation and freeze APIs. It found no blocking
lifetime or semantic design gap. In particular, region-independent container ownership and
staged animation descriptor borrowing have a safe implementation route; removing the parser
descriptor round trip does not require replacing core publication checks. The review requires
the concrete entry gate above before broad format edits. It ran no compiler, build, test or
benchmark and does not certify implemented behavior or release readiness.
