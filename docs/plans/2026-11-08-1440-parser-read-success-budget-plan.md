# Reuse exact success capacity within one parser read-row loan

Status: bounded design reviewed; current source revalidated, not implemented.
The fixed import policy-owner measurements are terminal and remain separate.
Native convergence, main/runtime integration and release remain HOLD.

## Demonstrated work and selected interface

`ParserReadRow` already exclusively borrows the actual WorkEntry and actual
frame remainder. Text planning and emission complete physical reads through
that loan between the existing control checkpoints. Each positive completion
currently repeats consumed checked-add, effective pending checked-add, attempted
comparison and consumed-limit comparison. No safe competing capability can
change those numeric fields while the exclusive row is alive.

Derive one disposable exact success capacity at the existing validated row
entry. The public interface, actual counters and physical read algorithm remain.
This removes repeated arithmetic from successful completions without pending
reservation, batching, deferred commits or precharging physical reads. Keep the
existing numeric checker as the single cold failure algorithm. The removable
instruction share and native saving are unknown; the fixed paired instruction
experiment decides whether to retain the candidate for further qualification.

The earlier bounded design review closes with GO. Current kernel, raw-row tests,
Store wrapper and Import text scanner have exactly the reviewed SHA-256 values;
the later policy-owner change alters other initialization/test paths. The
association is `parser-read-row-success-budget-current-association-1.json`.
The control for this new experiment is qualified source
`bdc92b3fa5ca523a99920ff980b909ecfe6d61e9`, rather than the earlier draft's
7b8 control. This changes no arithmetic proof or qualification target, but all
new measurements must bind to the new control and candidate identities.

## Exact invariant and failure behavior

Let C/P/A/L/R be the actual consumed, pending, attempted, consumed limit and
frame remainder. At entry compute one `u64` capacity:

```text
b = min(R, min(A,L) - (C+P))
```

Use checked addition/subtraction; any failure gives zero capacity without an
entry error, callback or numeric mutation. Existing layout, shape, grant, mode,
role and frame validation retain their original order before derivation.

After the physical reader returns, amount <= b immediately increments actual C,
decrements actual R and decrements b, with no intervening fallible action or
callback. For positive amount, the inequality proves all arithmetic representable
and C+P+amount <= A and <= L. Zero changes nothing even for an initially invalid
numeric state. Capacity is exactly the current positive-success allowance,
not an authoritative counter, snapshot, replay right or new pending owner.

For amount > b, run the original checker against current actual fields in its
original order: frame overrun, zero, consumed addition overflow, effective
addition overflow, attempted precedence, then consumed limit. Rejection changes
neither actual C/R/P/A nor b. A rejected raw call followed by a smaller legal
call within the same loan must still succeed. Positive cold success is impossible
under the exact invariant; add no new error, resynchronization or proof object.
The general multi-row ParserFrame completion algorithm remains unchanged.

Store stopped/debt checks and first cause remain ahead of numeric completion;
zero still incurs its original action debt. Store failure remains sticky even
though the raw row permits retry. Actual prefix commits still precede reserved
writes, policy checks and other fallible work. Unwind/cleanup use the actual
owners; dropping capacity settles no work. End the loan at every original
RawPoll/DebtPoll and derive a new allowance afterward. Preserve cancellation,
raw frontier, debt cadence, UTF-8 classification and returned-width ordering.

## Scope, tests and execution

Production scope is only `crates/baozi-operation/src/ledger/work/kernel.rs`;
test scope is its existing `work/parser/tests.rs`. No public signature, Import
parser, allocator, resource graph, physical storage owner, provider, owning
union, instrumentation, benchmark boundary or dependency changes. Add one
`u64` to the private loan, not copied authoritative numeric fields.

Extend the existing raw-row harness with exact success capacity then first
excess; rejection then smaller success within one loan; initial effective>A,
effective>L and C+P overflow; consumed/effective overflow near u64::MAX; zero
for each invalid initial state; and frame overrun before numeric invalidity.
Assert exact typed dimension/class/value/limit and actual attempted/consumed/
pending/remainder after handback. Repeated prefix scenarios may inspect actual
fields after separate loans, but the retry sequence must use one loan. Keep all
existing row entry, pending competition, zero, quota, host debt/cancellation,
sticky failure, unwind, compile-fail lifetime and text physical-prefix fixtures.
No test accessor, mirrored ledger, property framework or synthetic counter probe.

Freeze actual inputs, run serial affected Operation/Import checks and original
seven workspace/independent-consumer gates, with independent source and actual
QA association review. Rust 1.95, Cargo jobs1, nextest threads1, current directory
and existing target remain. No source writer overlaps formatting/freezing/QA.
Preserve unrelated user notes and all failed numbered attempts.

## Fixed measurement and disposition

Use matched GNU opt3/debug1 System workers on the original six pipeline/memory
tasks. Require complete literal non-clock equality, preserving numeric spelling,
types, work and output. At least one OBJ must satisfy
`100*candidate <= 99*control`; every case must satisfy
`100*candidate <= 101*control`. Keep all adverse signs and profiler warnings;
no alternate task or post-result predicate. Instruction failure rejects or
separately redesigns this candidate, without widening its loan or quota policy.

Instruction PASS does not establish native speed or source retention. Keep the
original fourteen cases/110 lanes, three warmup/twenty retained fresh AB/BA pairs
on Windows and GNU, exact source/binary/config identities, and separate calibrated
representative heap attribution. Every pipeline median <=3x same-campaign Assimp,
no confirmed endpoint >=10x and comparable whole-child memory <=3x remain the
full-goal targets. Preserve missing campaigns, failures, load observations and
all earlier regressions. No main/runtime integration or release follows until
the original required evidence and targets actually close.
