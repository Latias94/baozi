# Finish one parser piece through its original host

Status: prospective design, not implementation GO or a measured improvement.
The implementation baseline is qualified `0452a73`; rejected root-local body
candidate `d62cf67` remains isolated. Original release/main gates remain HOLD.

## Problem and attribution

Fresh, independently reviewed whole-worker profiles of baseline `0452a73`
remain available in the retained root-local experiment package. A read-only
locating pass sums mutually exclusive self-cost annotation rows for parser
scope entry, guarded prefix, parser-frame completion, read-row service, parser
admission and ordinary work completion. The addressed totals are 26.99984% of
repeated OBJ and 45.63817% of grid OBJ Ir. These include required accounting,
actual parsing and control; they are not removable-cost estimates or native
CPU percentages. The locating receipt is
`parser-region-refresh-locating-1.json` under private convergence evidence.

One concrete structural duplication is independent of those percentages:
OBJ token insertion opens `children.child(layout, amounts, |work| ...)` only
to call `work.run_returned_with_storage(...)` once. Operation constructs a
ParserHost for the child, then a ParserActionHost for that one callback, repeats
entry checks, commits its prefix, resolves the inner guard and immediately
closes the outer frame. Import similarly enters a child adapter solely to
enter the one-piece adapter. There is no compound body to resume after that
piece. Storage growth and rich format failure are real and must remain.

The native fault-address checkpoint separately locates finite startup and
stack growth; it supplies no causal argument for this parser design. Five
small finite catalog lookups are a lower-ceiling source opportunity, deferred
while this larger repeated parser boundary is evaluated.

## Alternatives and selected owner

| Alternative | Decision |
| --- | --- |
| Keep a persistent parser channel across many tokens/lines | Could remove more entry work, but would change per-item limits, close/exit polls and interleaved scanner/token nesting. Defer without changing those contracts. |
| Add an adapter that merely calls child then the existing prefix runner | Does not remove either guard/check layer; no source-retention reason. |
| Remove numerical completion checks or revive the rejected read allowance | Invalid scope; pending competition, aliases, exact returned prefixes and cold failures remain. |
| Let the original parser host execute and settle its single piece | Select as the bounded design. Retain one actual frame/guard and the original numeric kernel. |

The outer ParserHost owns the actual frame until settlement. A private shared
entry/finish helper serves both the existing compound parser route and the new
single-piece route; do not copy a second begin/close/error/control algorithm.
The compound route retains ParserActionHost where a body can catch a piece
panic and later attempt reuse. The single-piece route exposes no resumable
parser view or recovery callback around its physical action, so the original
outer host can retain the interrupted-piece obligation itself.

No new owning frame union, ledger, cached counter, proof object, provider,
dynamic dispatch, unsafe placement, allocation or global compiler flag is
needed. This is not an N==1 optimization. Alias-aware arbitrary-N numerical
transactions remain in the existing kernel.

## Concrete seam and ordering

Expose one narrow single-piece storage runner from Operation through the
existing root/parent child-admission loan. Import projects it as a method on
`ParseChildren`, taking its already bound `ParseWorkLayout<N>`, original
attempted amounts and one physical storage callback. The callback returns
`Result<(T, [u64; N]), ParsePieceError<P>>`, as the existing returned-prefix
runner does. It cannot return or borrow a resumable parser role.

1. Keep Import terminal preflight, actual layout/role/channel validation,
   caller-ordered attempted admission and per-item limit order before physical
   action. The original callback remains stored in its Option until admitted;
   project a runtime failure before disposing uncalled captures.
2. Preserve registry before/after costs and direct-mode debt/control at their
   existing positions relative to frame installation, callback, completion
   and close. Do not remove the prescribed successful/failed child exit poll
   merely because a wrapper disappears. Retain progress/cancellation semantics.
3. Give only the original disjoint ordinary storage view to the callback.
   Its Internal admissions and allocation failures can affect the same actual
   work rows. Do not precompute completion capacity, assume zero pending work
   or commit before the callback returns.
4. On a normal successful callback, immediately commit the literal returned
   amounts through the existing alias-aware atomic prefix kernel. Keep amount
   bounds, checked arithmetic, pending competition and consumed limits. Failed
   numeric completion retains previous work and leaves its transaction atomic.
5. This is exact child completion, not a healthy scan boundary. A successful
   callback returning less than the admitted suffix must retain the existing
   incomplete-success failure, exit-control precedence and sticky state. A
   callback error or unwind cannot release a healthy suffix.
6. Materialize rich format evidence or preserve the original ParseFailure
   while its original frame and failure-storage service remain available,
   then close in the same order. Reuse the existing format failure owner and
   projection helpers. The projector precedes destructor work that may panic.
7. The actual host stays armed across callback, error projection/materializer
   and relevant output destruction. On unwind, retain committed earlier work,
   exclude the interrupted piece, resolve only the original channel and keep
   original caught-unwind/registry poison semantics. An external caller may
   catch the public runner's panic; later reuse must remain rejected.
8. A completed owning result may escape; host/frame/storage loans cannot.
   A forgotten temporary storage/view must not forget the host-owned frame.

Initial consumer migration is OBJ token insertion and the equivalent MTL
token insertion only where the body consists of exactly one returned-storage
piece. Remove their old single-use helper chains once replaced. Keep general
compound children and prefix APIs where real callers still require them;
they are not compatibility shims. Do not change grammar, token/range/column
semantics, scan buffers, token retention or many-piece topology processing.

## Initial write scope and checks

- Operation: `concrete_store/execution/exclusive.rs`, the narrow admission
  forwarding/export seam and direct owner tests. No work-kernel algorithm or
  general WorkFrame change.
- Import: `scope/parser_execution.rs`, existing public export if needed and
  focused contract tests for the new loan/error boundary.
- OBJ: `obj.rs` and the equivalent MTL token call site, only after confirming
  its one-piece shape. Keep source-order grammar/recovery/cancellation tests.

Independent design/source review must test real counterexamples: aliased
dimensions, pending Internal competition from storage, tighter consumed than
attempted limit, partial returned amount, ignored error, uncalled capture Drop
panic, action/output/error-producer panic, first cause and later forbidden
reuse. Reuse the original parent/prefix tests rather than weaken assertions.
Add focused positive/compile-fail borrowing witnesses for the new seam only.
Keep actual token observations, work totals and cancellation frontiers exact.

Qualify affected nextest, doctests, strict all-target/all-feature Clippy and
format first, then all original workspace/public-consumer gates. Rust 1.95,
Cargo jobs1, nextest threads1 and current repository/existing target remain.

## Prospective measured decision

After exact source/QA qualification, build matched GNU opt3/debug1/System
workers from baseline `0452a73` and the candidate in current repo/target.
Inspect generated code for the migrated call sites to confirm the old nested
single-piece runner is actually removed, not relocated into another adapter.
Retain mandatory begin/complete/close, failure and control work.

Use the original seven diagnostic tasks, including STL1, and require all
seven complete literal non-clock output pairs equal. The fixed selection
predicate is at least one original OBJ whole-worker Ir decrease of at least
1%, with no task above a 1% increase. Report every sign, warning, failure and
unknown. This is only experiment selection, not native/source acceptance.

Then use unchanged Windows/GNU fourteen cases/110 lanes, three warmup pairs
and twenty retained serial AB/BA pairs in C0/N0/N1/C1, comparable whole-child
memory and separate heap. No own build/profiler overlaps ordinary clocks.
Keep only demonstrated same-task native improvement beyond dispersion with
no unexplained affected regression, under the original convergence targets.
A failed screen remains failed and does not invite threshold changes or
broader ownership rewrites to save the experiment.
