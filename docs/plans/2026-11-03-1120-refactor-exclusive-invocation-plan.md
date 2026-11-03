---
title: "Make invocation execution exclusive and attach publication only for real writes"
type: refactor
status: in_progress
date: 2026-11-03
parent: 2026-11-03-0900-perf-assimp-convergence-plan.md
---

# Exclusive Invocation and Deferred Publication

This is the ownership replacement within the
[Assimp convergence plan](2026-11-03-0900-perf-assimp-convergence-plan.md). The user authorizes
breaking APIs and removal of obsolete implementations. The current per-action work protocol
and unconditional retained lifecycle are measured costs; preserving their permissive interfaces
is not a design requirement. Normal mathematics and format-specific shading policy are separate
from this change. No publication, push/PR or native target promotion is authorized here.

## Reviewed Decision

Use one host-owned invocation and lend a scoped mutable execution context. Work accounting
uses ordinary mutable row/header views over a frozen catalog, with one implementation of the
existing arithmetic and error-order rules. The host owns pending-action/parser metadata and
cleanup guards; the callback receives no owning permit or separate Begin/Complete pair.
Policy changes and observations occur through that context between actions. The physical leaf
receives no numeric mutation authority and cannot reenter the context in safe Rust.

Fixed postprocess arrays and dynamic import Vecs live in concrete lexical roots. One borrowed
invocation moves through execution, attachment and finalization inside the root's lifetime.
Published Scene storage, independently released storage leases, detached facts and genuine
containment owners may escape. Unfinished mutable invocation/candidate continuations do not.
An invariant scoped brand prevents mixing cached layouts from different contexts; equal
lifetimes alone are insufficient. No public backing generic, collection framework, trait-object
numeric backend or permanent RefCell compatibility path is planned.

Storage release remains independently synchronized because real leases can outlive the context
or move across threads. A small cold selector borrows fixed/dynamic storage roots or owns the
promoted Arc; the shared state uses compact owned rows. It dispatches representation access to
the same reservation, resize, release, policy and publication functions.

Independent review accepts this design with no open P1/P2 at its inspected version. Earlier
promotion, footprint and attachment-order findings are resolved by the rules below. The design
is not a compiled lifetime proof or a measured performance keep. Detailed source inventories,
the 301-line design and independent review are retained under
`.git/baozi-review/performance-convergence/exclusive-invocation-*.md`.

## Work, Failure and Control Contracts

- Bind catalog positions/duplicate order once. Preserve checked overflow, representable denied
  attempts, attempted-before-consumed rejection, immediate physical-prefix completion,
  pending competition, protected roles and registered-versus-caller diagnostic identity.
- A normally returned semantic error completes its bounded invocation before propagation.
  Parser methods complete only verified prefixes. Backend Ok/Err/EOF and caught extension
  panic retain their existing, different accounting and cause projection. No generic wrapper
  may treat all these returns alike.
- Private host action guards latch denial before unused captures drop, complete before returned
  values are inspected/disposed, and resolve only verified remaining admission on unwind.
  A client catching a leaf panic must observe a failed context. Region guards also cover idle
  body/capture/result destruction; known resolved errors are not abandonment.
- Preserve each qualified owner's control contract: entry/final/allocation/IO/semantic checks,
  late control evidence, parser error-exit polling, geometry's different known-error cleanup,
  64-comparison and text/copy frontiers. Preserve any qualified weighted normal-action debt;
  never restore an older per-getter catalog or silently reset debt on scope/role changes.
- No accounting lock or dynamic borrow guard spans a physical action, allocator, extension,
  observer or destructor. Ordinary exclusive row references are host-owned Rust borrows, not
  dynamically held RefMut guards. Shared account/poison/final eligibility checks remain.

Structured parser parents/children successively reborrow the same numeric view. A parent cannot
complete while its child is live, and a child cannot create a nested child. Compound token/push
and retained-share methods preserve admission, allocation, physical construction and completion
order. Session-owned containers may leave/reenter execution regions through their real storage
owners. Source/diagnostic/IO views must be disjoint from numerical mutation; narrow cold error
slots may retain their required immediate latching without becoming another work authority.

## Honest Administrative Metric and Promotion

Existing hard buffer-capacity and retained physical requested-layout obligations stay in full.
The current standard Arc ledger payload is not separately charged; the replacement adds its
known payload to administrative coverage. Rename administrative limits/accessors explicitly to
payload/capacity bytes. Cover known owned payloads, Vec capacities and conservative admitted
overlap once each. Inline root/facts layout bytes are separate facts. Opaque standard Arc header,
runtime and allocator-private overhead remain explicitly unmeasured, not reported as zero or
claimed to have a portable fixed layout. This metric is not process RSS or a physical sandbox.

At the first real escaping storage requirement:

1. Validate keys/control/work/account eligibility in the existing order while Inline is intact.
   Admit the known `Mutex<Option<OwnedStorageState>>` payload and any fixed-to-owned row buffers.
2. End numeric transactions. Allocate an unissued vacant shared shell and qualify actual row
   capacities. On rejection destroy real candidates before releasing their admissions; preserve
   the original state and truthful peaks. No token/observer can see the vacant shell.
3. Prove shell uniqueness, then move the complete authoritative state in one nonallocating,
   non-callback transition. Move dynamic Vecs; move fixed rows into already admitted buffers.
   Preserve policies/counters/peaks/phase/publication history without replay or restart.
4. Install Shared before issuing the first token. Reject nested promotion. Subsequent tokens
   use the same Arc/Weak identity; never demote or promote again.

Do not invent recoverable OOM from `Arc::new`, unsafe initialization, a dummy ledger, universal
Arc-header arithmetic or an allocator framework. Actual scratch remains live scratch, destroys
payload before releasing its lease, and is not classified as retained output. Uncertain cleanup
preserves charges/poison and owned containment.

Terminal facts observe administrative current/peak before actual host/shared metadata teardown.
A live shared payload is reported truthfully rather than forced to zero. Replace the old blanket
administrative-zero condition with structural coverage of precisely allowed remaining metadata.
Zero live scratch/reservations/pending work and all physical/account/publication gates remain.
Actual outgoing snapshot transfer and later root destruction retain their own ownership order.

## Implementation Sequence

1. **Prove the exclusive runner at the actual numeric owner.** Extract the existing row/header
   operations and private host guards, with real physical-prefix/error/unwind tests. Demonstrate
   a nested parser plus metered allocation/Internal share trace, not a mock counter backend.
   Intermediate development adapters must call the one kernel and disappear at cutover.
2. **Migrate real consumers and remove aliases.** Update operation/concrete wrappers, import
   store/scope/parser channels, metered storage, typed builders and postprocess instrumentation.
   Remove whole-Ledger references from storage/progress. Preserve the currently qualified
   reader/counter/control contracts; do not overwrite newer code from an old isolated copy.
   Compile-fail examples migrate to valid replacement APIs with nearby valid examples.
3. **Install roots, metric and consuming attachment.** Transfer dynamic buffers/charges, add
   the finite direct arrays, and qualify promotion. Carry root lifetimes through real activation,
   candidate and rejection types. One state-based attachment/start executor serves direct and
   import; import still requests its actual retained terminal bundle.
4. **Clean up before finish and integrate selection.** Split candidate validation from final
   consuming finish. End temporary borrows, destroy generated/replacement/selection scratch,
   then run terminal control and the existing joint publication protocol. Integrate the reviewed
   indexed single-classifier selection driver only after this seam works. Direct unchanged and
   rejected outcomes return independent inline facts; changed outcomes retain genuine remaps
   and domain evidence. Empty/single unchanged default operations request no invocation heap.
5. **Qualify and measure.** Close independent source findings, run affected integrated/compiler/
   consumer/lint gates, then build immutable ordinary workers. Record actual complete owner,
   context, result/error, shared payload and stack/allocation behavior. Re-profile and run full
   Windows/Linux matrices when the maintained workflows converge. An intermediate gain never
   clears the parent plan's all-endpoint/pipeline/memory gates.

Delete obsolete numeric RefCell/borrow-fault and shared work backdoors, transferable work/parser
permit APIs, separate Begin/Complete instrumentation, the old split execution tuple, unused live
observation handles, mandatory direct retained terminal/fallback graph, allocationful pre-read
bootstrap, evidence-only direct terminal path, scratch-as-output, full-Scene selection/second
unchanged scan, coordinator reconstruction and superseded startup/finish implementations.
Keep genuine escaping storage tokens and one common numerical/publication implementation.

## Meaningful Qualification

Use existing suites and literal output/state oracles. Required new boundaries include:

- Valid-API compile failures for context/layout escape, reentry, invariant cross-owner
  substitution, parent/child overlap and cross-thread use. Scoped-thread/trait witnesses must
  test the intended auto traits rather than fail merely for lacking `'static`. Owned facts and
  session containers must pass their corresponding escape/reentry examples.
- Same-kernel fixed/dynamic traces for duplicate/order/overflow, zero/exact/one-short limits,
  pending Internal competition and policy tightening. Equality between two wrappers alone is
  insufficient evidence of the intended result.
- Real writes before semantic error, caught/unhandled leaf panic, idle/capture/result Drop,
  completion failure and denied-capture Drop. Require exact attempted/consumed/pending values,
  original first cause, immediate cleanup and each owner's permitted poll behavior.
- PLY parent/child with actual metered storage, OBJ token/push and comparison tails, independent
  release during promotion/attachment/thread movement, and physical poison races.
- Shell/row allocation denial, nested promotion, one shared identity, exact capacity overlap,
  drop-before-release, active scratch blocking terminal and truthful positive metadata facts.
  Reject unclassified residue rather than accepting arbitrary nonzero terminal state.
- Original Scene/generation/provenance/numerical behavior and owned failure/containment after
  root exit; final cancellation/disposal before irreversible commit, with no fallible extension
  action afterward.

No test/script may approximate Rust name resolution, turn stale missing symbols into an ownership
proof, widen budgets, weaken mathematical results or move benchmark work outside its clock.

## Execution Record

- First proof is qualified at `45a09d19d948600d9b2a4261eb2e3394fe594ef1`: operation 905,
  core/import/postprocess 1420, glTF profile comparisons 9, control 9 and independent consumer
  138 tests pass, with affected doctests and strict lint/formatting. The
  [receipt](../reviews/2026-11-03-exclusive-invocation-proof.md) pins actual source, intended
  compiler diagnostics, physical-prefix/Drop oracles and the independent integration review.
  This is one kernel and a lifetime/owner proof, not real caller cutover or a performance keep.
- The 240-line caller migration map, SHA-256
  `5358d7b9ac30233812bb3efe794a8b850f5d9e09b2d512b41d47cc2d67009b9e`, closes its independent
  incomplete-success/importer-unwind exit-order and accounted-format-error host-service P2s.
  Known execution failure is distinct from terminal cause selection; private prescribed exit
  polling remains available while client bodies stay stopped. Ordinary semantic returns,
  verified parser pieces, backend Err/EOF and caught extension panic retain different rules.
- An operation-only exit implementation is deliberately not produced: the actual cold scope
  abort, registration poison and session post-parse selection cannot be inferred from generic
  `E` or the numeric cause slot. Its empty patch is a scoped stop receipt, not an implementation
  or qualification candidate. The next vertical cut must include the real import bridge.
- The concrete 138-line bridge design, SHA-256
  `2ff7ee5652f5968588bf6da0d3ef2a40be6c9701a3bcf41236767ae63204e864`, forwards the actual Dormant
  outcome owner's operation into the one executor. Stopped numeric state remains with that
  operation; registry disposition is disjoint session data that survives Dormant-to-Pending
  submission. A real registered Format path must prove the new parser roles, accounted failure,
  incomplete success, caught/uncaught panic, poison and late control before broader migration.
  Independent bounded design review accepts this frozen hash with no actionable P1/P2.
  Isolated implementation must also gate `submit_complete` and activation before the first
  mutation, preserve Pending's real post-submit panic/poison rejection, and keep disposition
  outside the moved root. No shadow counters, second runner or generic exit-policy framework
  is authorized to substitute for this real-owner proof. The subsequent
  [bridge receipt](../reviews/2026-11-03-exclusive-import-bridge.md) distinguishes isolated
  compiler/runtime qualification from actual main application and integrated checks.
- Actual bridge source is qualified at `67b5ca609470268b124fd1915741ee81105c4a24`:
  operation/core/import/postprocess 2346, all four formats 853, facade control 9 and independent
  asset consumer 138 pass, with 25 + 54 doctests, strict affected/consumer lint and formatting.
  Independent production, corrected private-frame oracle and exact 18-path application reviews
  close with no actionable P1/P2. The preserved Control-reentry regression is fixed at the
  common owner; the failed pending-consumed oracle is replaced without changing the kernel.
  This is real retained-owner/registry/submission qualification, with caller cutover and native
  performance keep still pending.
- The identical committed bridge then passes Linux operation/import 1371/1371, 25 + 54
  doctests and strict all-target/all-feature lint, with exact 18-path before/after identities.
  This additional bridge qualification does not claim the full Linux consumer/format matrix
  or clear any native timing/memory gate.
- The first real caller design, 86 lines, SHA-256
  `cc0012a53d580c78626adc1b6a5a0a5bacbdc56ea2a6d7f9dfff834f2964d8fd`, selects the entire
  binary STL decode function and one shared ordinary vector/string growth owner. Independent
  design review finds no actionable P1/P2. A concrete storage/poll reborrow and compound-host
  lifetime/error-order supplement must close before Rust implementation. Preserve existing
  allocation overlap, issuer checks, real poll/copy frontiers, exact five-lane facet prefixes,
  authored bits and canonical NaN errors. Remove binary's old execution-taking helpers and
  duplicated growth bodies in that cut; ASCII/text, other formats and canonical authoring
  remain genuine later callers. This is a reviewed next slice, not implemented cutover.
- Fixed/dynamic roots, honest administrative payload/capacity metric, consuming attachment,
  cleanup-before-finish, indexed selection and removal of transitional permits remain pending.
