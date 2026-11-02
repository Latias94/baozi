# Bounded Control, Separate from Work Accounting

Status: reviewed P1 control contract, awaiting ownership integration and implementation.
Date: 2026-11-02. Current production control: `4455d03db190c6bef05ff5a1de024d2aac461465`.
This belongs to the [operation architecture continuation](../plans/2026-11-02-1700-perf-import-operation-architecture-plan.md).
It does not claim that the current implementation already supplies this contract.

## Decision

Keep exact per-action work admission/completion in the invocation-local work owner. Poll user
cancellation/deadline controls separately: before an execution region, after at most **256
bounded library geometry actions**, at explicit requested checkpoints, and before success or
automatic semantic-parent completion. Text regions keep their existing **4,096-byte frontier**.
Progress delivery remains explicit and final; the periodic checkpoint does not invent progress
events. No user callback runs under a work borrow, storage lock or whole-session mutable borrow.

These are limits in documented algorithm units. They are not wall-clock guarantees for IO,
allocation, arbitrary extension callbacks, destructors, codecs or drivers. The constants are
one private implementation policy with public bounds, not a selectable backend/strategy framework.

## Why These Units

The current normal kernel emits 982,943 control sites for a 16,384-face original indexed
triangle case. Its geometry and public attribute reads take 6.7044 ms in the diagnostic with a
local counter; the real observed-unit protocol takes 127.0353 ms for the same count of empty
actions. Both are separate tasks. This demonstrates the need to avoid one full control/permit
protocol per comparison or scalar read, not a universal cancellation response time.

A geometry action must be bounded by a fixed amount of ordinary library work: one compare/
swap step, one scalar/vector lane operation, one dense attribute row read, one sparse-search
iteration, one fixed triangle arithmetic step, one reserved vector append, or another explicitly
fixed-size step. An entire mesh, unbounded face, sort, sparse search, string or user closure is
not one action merely because a function wraps it. Existing input-sized loops already expose
these sites in `normals/kernel.rs`, `prepare/input.rs`, `triangulate` and core semantic work.
Their work counters remain truthful; the cadence counter does not replace those counters.

With 256 sites per mandatory periodic poll, the observed normal kernel would perform at most
about 3,840 periodic polls, plus its explicit region/parent/final checkpoints, instead of
nearly a million full user-control invocations. This is a source-count projection, not a
measured post-refactor speedup. Tighter effective limits or explicit checkpoints may poll sooner.

Text uses bytes because character widths and scan strategies differ. Preserve `WorkCursor`'s
4,096-byte polling frontier, indivisible UTF-8 handling, and exact malformed/recovery-prefix
outcomes in `baozi-import/src/text_input.rs`. Its current `processed_to`/`completed_to` deferred
completion and later `flush` are **not** the new accounting contract. The work owner records
each declared bounded text action's actual completed prefix before another physical action or
observable seam; the byte frontier controls only full control polling. A text classification
action consumes its actual raw UTF-8 unit width (one to four bytes), while decoded-byte and
storage/copy accounting retain their separately named units. A known whole-scan envelope may
still be admitted once with its existing attempted semantics; this is not permission to invent
unperformed consumed work or replay completion at chunk end. Poll before crossing the frontier
and never split a text unit. Retain exact malformed and recovery-prefix tests rather than
pretending every character is one byte. Source policy, decoding/storage admission and raw/
decoded diagnostics remain distinct. This does not require a full control callback per character.

## Minimal Internal Control Interface

Conceptual responsibilities, not a second implementation:

- `ControlCursor` belongs to one active execution region and carries a bounded local debt.
  It contains no heap/event buffer and does not own the ledger or clone a cancellation Arc.
- `before_action()` checks an already latched terminal cause, then calls the coordinator's
  full poll only when the region begins or the debt reached 256. On success it permits the
  next bounded action; exact work admission still occurs before that physical action.
- `completed_action()` updates the local cadence debt after the physical action and truthful
  work completion. It cannot erase a physical prefix if a later checkpoint fails.
- `checkpoint()` performs a full poll and resets debt on success, even for an incomplete
  local work scope when that scope's new interface explicitly supports polling.
- `finish_region()` performs the required final checkpoint and prevents forgotten unfinished
  work from becoming success. Cancellation/deadline latched by any poll remains sticky.

The exact Rust lending shape is part of the accompanying ownership design. Work state is
borrowed only for numeric projection/commit, then released before full poll, physical action,
user code or storage activity. A cadence object must not independently own an unrestricted
second work authority or accumulate completed-work deltas for replay.

## Required Checkpoints and Failure Order

| Seam | Required behavior |
| --- | --- |
| Import/format invocation entry | Full poll before risky parser work; bounded detector remains separately constrained to its prefix. |
| Enter a bounded internal geometry/text region | Full poll before the first action; subsequent internal steps use the frontier. Do not start a new region for every scalar. |
| Explicit plugin `checkpoint` | Full poll immediately, even if no debt is due. It remains useful for an extension's own long loop. |
| Before an IO/dependency acquisition, allocation or external callback | Full poll outside all conflicting borrows, then normal authority/admission. The call itself may block and cannot be interrupted internally. |
| After extension callback normal return or error | Reconcile control with latched failure using the existing owner precedence; a final cancellation cannot silently turn into success. Internal fixed library closures are not separate extension invocations. |
| Before automatically completing a semantic parent | Full poll before parent completion; child prefixes survive, but the parent remains attempted-only if this poll fails. |
| After physically completed action | Record its consumed prefix before a due/final post-work poll. Cancellation never refunds attempted or completed work. |
| Before outcome eligibility/commit | Final progress and full poll after all fallible ordinary work, with protected terminal/failure work still governed by its owner. No control/user callback enters the pure prepared handoff. |
| During cleanup/unwind | Do not invoke untrusted callbacks from `Drop`; preserve physical cleanup and latched fault state. Reconciliation occurs at an owning catch/finalization seam. |

The breaking cadence changes when an **unobserved** concurrently raised signal is discovered.
If a deterministic format/resource failure occurs before the next due poll, retain its
documented reconciliation rule; do not invent a signal observation that did not occur. Once
control is latched, no new physical action is admitted. Domain/invalid-binding checks, ordinary
work admission, pending capacity and authoritative semantic rejection keep their defined order.
The existing callback-error/cancel, last-action cancellation and parent-cancel tests are the
concrete precedence cases to preserve while replacing exact poll-number schedules.

In particular, release an incomplete callback scope and reconcile control **before** synthesizing
an incomplete-success contract fault. A generic callback error can be returned unchanged to
its immediate caller while the owning scope latches cancellation for finalization. An
authoritative scene-validation rejection retains its owner exception; neither control-wins
nor first-fault-wins is a universal rule. Full poll itself invokes cancellation code under panic
containment, in sticky-cause/cancellation/deadline order; do not recursively apply the
before-external-callback checkpoint rule to the cancellation callback inside that poll.

## Evidence and Test Changes

Work evidence retains actual attempted/completed amounts at every action. There is no reserved
256-action speculative envelope, end-of-chunk accounting flush, idle cancellation success or
unmetered temporary counter stream. The independent cursor only decides when to consult user
control; resource limits still reject before the violating action, even between polls.

Use deterministic counter/barrier controls rather than elapsed-time assertions:

1. For a bounded region, raise cancellation after a known completed action; no more than 256
   further bounded actions execute, and the next/final checkpoint returns the exact control
   cause. Test 0, 1, 255, 256, 257 actions and the final short region.
2. A resource failure between polls records the exact rejected attempt and completed prefix.
   A control failure after a completed action retains that completed amount. No counter replay
   occurs when allocation, callback error, panic or parent completion fails.
3. Explicit checkpoints and a final cancellation are effective without needing 256 actions.
   A parent has no consumed completion until its final control check succeeds.
4. Preserve byte-frontier, multibyte/recovery, malformed-prefix and zero-resource-acquisition
   tests for OBJ/MTL/ASCII formats. Do not trade raw diagnostic fidelity for a faster scan.
5. Keep callback allocation/drop/reentrancy and concurrent independent operations passing;
   control/observer calls run outside work/storage guards and cannot observe half-committed
   numeric groups. Intermediate snapshots obey the ownership design's coherent observation.
6. Replace tests depending only on the old `polls > 100` or exact historical callback count
   with bounded responsiveness and unchanged semantic/evidence assertions. Keep actual
   early/middle/final cancellation scenarios and immutable input-generation checks.

Document the new bound in ADRs 0002, 0020, 0046 and 0047 when implementation lands. Historical
receipts retain their original cadence. A manual diagnostic with an empty/counting callback
does not qualify runtime control, ownership or a complete replacement operation; native
controlled variants and final integration tests do.

## Control-Contract Review

The bounded independent review accepts the geometry cadence and existing text frontier.
Its one P2 clarification is incorporated above: preserve the text polling/error-prefix
semantics, not the current delayed completion mechanism. The named callback/incomplete/parent/
semantic rejection precedence cases are also explicit. This approves a proposed contract,
not a concrete ownership lending implementation or runtime qualification.
