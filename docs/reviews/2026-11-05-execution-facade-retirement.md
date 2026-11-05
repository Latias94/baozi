# Retire the Aliasable Execution Facade

The old `StoreExecutionWork`, `StoreExecutionServices`, `StoreBoundWorkGroup` and
`StoreExecutionPermit` surface and its operation/body/planning `execution()`
forwarders are removed. Their retained-layer Work/Group/Permit/Services/Storage
implementations are deleted. Production Import and geometry work already uses
StoreInvocation and StoreActionExecution; the old facade's remaining consumers
were forwarding methods, integration tests and a manual diagnostic.

The real retained-backing entry now borrows the unique InternalWork beside
ExclusiveStorage directly. It retains `ensure_work_open` before physical/account
gates and preserves that earliest rejection's lack of sticky mutation. The
existing account preflight, logical reservation, action, Internal share and
physical admission helper remains the single executor. Later errors preserve
the original Ledger versus ProductGraph cause mapping. Returned reservations
and physical leases retain their independent release lifetimes.

Live ordinary storage, exclusive execution, parser/action hosts, progress,
weighted control debt and physical resize/release remain. Unused facade-only
storage observation and progress-tuple helpers are removed. Raw legacy work
remaining/replenishment inspection is now test-only while the surviving startup
and ledger owners still require the aliasable backend.

## Regression migration

All 23 parent execution tests and 19 exclusive child tests remain. Nineteen
facade-dependent parent tests now exercise actual invocation/action/service
callbacks; four parser traces remain. Their oracles cover logical/physical
storage, progress, current policy/refill denial, cancellation frontiers, weighted
debt, foreign keys/tokens, incomplete work, unwind and terminal first cause.

Client code can now forget only a loan, while the actual host retains its frame.
Zero/completed forgotten loans therefore resolve normally. Tests retain failure
closure for incomplete actions, ignored errors, unwind and cancellation. Raw
unmatched/forgotten frame ownership still has kernel and actual-host oracles.
The preflight regression now follows that same real loan ownership rather than
constructing a removed by-value permit.

Storage-observation documentation includes a positive owned-scalar return and a
negative borrowed-observation escape using the actual generative host. The ignored
empty-action normal diagnostic uses the action host and schema v3. It remains a
separate manual diagnostic, with entry/binding/final checkpoints outside its timer;
neither production normal generation nor the frozen 110-lane harness changes.

The detailed old/new oracle map is retained as
`.git/baozi-review/performance-convergence/execution-facade-retirement-test-map-1.md`.

## Verification

Base commit is `d90ddfbf1273b066a7915893e51848788ae1fe34`. The final serial run,
exec27048, terminates successfully on frozen source2:

| Gate | Result |
| --- | --- |
| Workspace nextest | 3,589 passed; six existing skips |
| Workspace doctests | 187 passed |
| Workspace strict Clippy | All targets and all features; passed |
| Asset-consumer nextest | 138 passed; one existing skip |
| Asset-consumer strict Clippy | All targets and all features; passed |
| Workspace and consumer format checks | Both passed |

The workspace run is `34364a2e-7a4b-4191-ab15-47fc5edf5a53`; the consumer run is
`f6d094ff-54d5-4870-89e0-eba305931169`. All seven gates retain the same 1,165 input
hashes and 17 changed Rust paths. The frozen manifest is
`.git/baozi-review/performance-convergence/execution-facade-retirement-source-2.json`,
SHA-256 `e99b4806bb436d9a09405d98e35ae46e66630d29a0c34833b82553dbd62f34b8`.
The command/log receipt is
`target/review/convergence/execution-facade-retirement-qa-2.json`, SHA-256
`726b4672b8d6746e6a4fb01607380c6a5df402e798f58128afcc85d6b8f5ad39`.

The preceding Operation run has 1,073 passing tests and zero skips. Its actual
exec55614 terminal result and run `c8a2d872-5e8b-4bb1-9c57-466356967640` are recorded
in `target/review/convergence/execution-facade-retirement-operation-runtime-1.json`.
That file is an observed terminal-result record, not an independently captured
full stdout log. The preliminary migrated-test type-inference error is retained
separately from the successful final runtime result.

The first serial QA attempt passes workspace doctests, then fails strict Clippy
on the manual diagnostic's inline `StoreRuntimeError`. The diagnostic now follows
the existing action-host convention with a local, explained `result_large_err`
allowance. Production error representation and the timed action body are unchanged.
That failed attempt remains in `execution-facade-retirement-qa-1.json`.
Final source2 changes only that local diagnostic attribute relative to source1.
The independent final production review directly inspects the final affected
diff rather than inheriting pre-format byte equivalence. The independent test
review checks all 23 parent and 19 child tests and their physical-prefix,
cancellation and failure oracles. Neither finds a blocking source issue.
Independent final evidence review finds no blocking mismatch: all frozen source
and staged bytes, seven original log hashes, command scopes and terminal counts
reconcile. Its receipt is
`.git/baozi-review/performance-convergence/execution-facade-retirement-final-evidence-review-1.md`.
It inspects the completed artifacts; the actual exec27048 exit is observed by the
main agent rather than an independently executed compiler run.

## Remaining work

The next coherent share migration joins startup's four account shares,
publication account opening and canonical/evidence table creation. Table creation
needs numeric attribution while its admitted share is alive, so it must use the
same disjoint numeric/storage views rather than a second full-Ledger borrow.
Startup's deferred eight-entry layout must preserve current-policy projection,
pending consumption and live-publication error precedence.

Control/producer checkpoints and cold ledger transactions also retain LocalWork.
These require real caller migration before those owners can be removed.
Fixed/dynamic backing, shared promotion, exact administrative accounting and indexed selection remain
open under the [convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
This source deletion has no new ordinary timing, whole-process memory or heap
qualification. Release remains HOLD.
