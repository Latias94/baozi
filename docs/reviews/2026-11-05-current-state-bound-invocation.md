# Bind the Actual Invocation Before Retained Attachment

The isolated successor binds the original control, account, key domain, source generation,
ledger policy and operation-start admission before client work. Prework and retained
attachment use the same outer invocation and numeric kernel. Core's scene-edit entry,
both Import entry paths, and Postprocess preparation and space entries now use the
canonical bound-body driver.

This is a construction checkpoint in the
[lexical-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
Source6 passes the complete workspace and independent consumer gates after repairing a
real ordinary-stack regression exposed by source5. The Rust changes remain isolated and
unapplied to main. This component checkpoint is not a performance keep or release acceptance.

## Binding and Current-State Admission

`PendingRetainedAttachment` keeps the original single-use account admission and immutable
start plan while common/control remain in the outer root. `BoundStoreOperation::attach`
and `CanonicalBoundBodyStart::attach` accept no replacement account, source, control,
limits or domain. Existing raw and concrete startup methods use the same bind/attach
implementation.

The old zero-state `StartupLedgerPlan` and its duplicate pure validators are deleted.
Startup binds and preflights the existing ledger, including actual prework, pending
competition, tightened policies, phase state and live scratch. The attachment baseline
check is separate from the strict terminal residue check; final publication still rejects
unexplained live or pending state. Six startup reservation specs, four real account-share
steps, their work and physical manifest remain unchanged.

The protected failure dimension is configured before prework. Its attachment capacity
uses the existing read-only repeated-work kernel before startup effects. A second
post-share preflight is removed: those four ordinary shares cannot access or change the
separately configured protected pool, and no client callback intervenes. No second numeric
authority or replacement coordinator is introduced.

Returned attachment errors stop work and latch the actual first cause before returning.
Unexpected unwind reaches the same attachment guard. The real startup builder guards
still distinguish clean rejection from uncertain construction; a builder panic retains
its genuine unresolved charge and quarantines the account.

## Actual Outer Body and Callers

`run_bound_body` and `run_bound_derived_body` retain the actual client callback as a capture
of the existing controlled body driver. Client code receives a short permission over the
original binding frame. It cannot extract the root, replace binding inputs, escape the
body lifetime or move the permission to a scoped thread. Attachment returns only the
restricted canonical body operation.

Core alone supplies the actual canonical Scene source and its account. Both Import
entries share their existing execution, session staging and finalization after binding;
their old startup fixture adapter remains test-only. Postprocess's two real entries use
the same derived binding and preserve their existing preparation algorithms and polls.
The actual body and its captures are disposed while common remains armed, before finish
authority can run terminal progress or publication.

The bound frame itself is currently local to the adapter invocation and ends before
callback-capture disposal. Installed common/control remain root-owned. Postprocess still
attaches before preparation; this checkpoint does not supply a late-attachment facility
or an unattached unchanged/failure terminal.

## Source and Verification

The final manifest is `target/review/convergence/current-state-binding-source-6.json`:
**1,421 files**, source-set SHA-256
`d8e911f58b89c6e2381de06cc629e5dc480cba17a8d3509ffb35c40170d6c22f`.
Frozen bytes are in the corresponding `current-state-binding-source-6-frozen` directory
under `.git/baozi-review/performance-convergence/`. Source4 adds tests and documentation
to reviewed source3; source5 repairs two test defects. Source6 changes only Import's
initialization phase and private error constructor to repair the consumer stack failure.

| Final source6 check | Result |
| --- | --- |
| Whole-workspace runtime | 3,551 passed; 6 existing skips; run `80f99f11-ba74-4684-9348-58b72ae62050` |
| Whole-workspace documentation | 172 passed |
| Workspace strict Clippy | Passed, all targets/all features with warnings denied |
| Independent asset-consumer runtime | 138 passed; 1 existing skip; run `beab69e6-222d-4e47-bcbe-4e9a79913263` |
| Asset-consumer strict Clippy | Passed, all targets/all features with warnings denied |
| Workspace and consumer formatting | Both passed |

Checks run serially on Windows with Rust 1.95.0, locked/offline dependencies and one build
job; runtime checks use one test thread. The seven terminal results are recorded in
`current-state-binding-connected-qa-6.json`. The corresponding
`current-state-binding-connected-terminal-association-6.json` recomputes all 1,421 live
and frozen file hashes, all fourteen before/after source maps and all seven actual log
hashes. Every identity matches source6. The complete runtime includes the fifteen direct
binding, derived-source and startup-builder regressions introduced by this component.

Earlier failures remain retained. The original new forgotten-bound-view test compared
historical physical counters with their preallocation values; its repair preserves peaks
and cumulative admissions while checking released live storage. Source4's full QA stops
at compiler `E0583` because the new derived module lacked its explicit path; no runtime
pass belongs to that failed attempt. Independent test review also identified a masking
problem in the new result-destructor test. Source5 records whether the execution call
returned before explicit caller-side disposal, so a caller-side panic cannot satisfy the
internal-rejection assertion. Production was not changed for either test repair.

The source5 consumer then exposes a production regression on its unchanged 1 MiB Windows main
stack: the shared glTF case overflows even at the import endpoint, in both IO modes. CDB
shows the new outer import frame retaining initialization temporaries across the existing
glTF parser chain. Source6 separates that initialization into a narrow-error preparation
function before client execution; its original account and bind admission remain unchanged.
The original source5 result, 137 passes/one failure/one existing skip, and all eight
failing stage/IO probes remain retained. Its workspace/runtime/docs/lint passes do not
qualify that failed source. The existing failing consumer regression passes on source6,
as do all eight original stage/IO CLI probes with the same analytic
geometry and unchanged 1 MiB PE stack reserve. CDB measures the outer import frame at
207,600 bytes before the repair and 174,736 afterward; the caller's measured frame remains
160,336 bytes. These are build-specific frame observations, not portable stack bounds or
elapsed-time performance evidence.

## Independent Review and Remaining Work

Bounded reports under `.git/baozi-review/performance-convergence/` are:

- `current-state-binding-ledger-source-review-1.md`: current-state ledger predicates.
- `current-state-binding-outer-host-source-review-3.md`: raw bind/attach and actual outer
  callback, capture, forget, ticket and protected-preflight composition.
- `current-state-binding-real-callers-source-review-3.md`: Core, Import and Postprocess
  entry and cleanup migration.
- `current-state-binding-source4-test-delta-review-1.md`: new direct tests and paired
  compiler examples, including the two test findings above.
- `current-state-binding-source5-test-repair-review-1.md`: closes those two findings and
  verifies the production-identical repair without weakening existing assertions.
- `current-state-binding-stack-repair-source-review-1.md`: verifies source6's two-file
  initialization repair, original account and error identity, drop ordering, unchanged
  start-ticket path, and the bounded native diagnostic evidence without a finding.

Direct coverage remains bounded. The new cases cover foreign-plan entry denial and
capture disposal, derived prework/inheritance and same-account generation refusal,
forgotten startup permission, region-external caught prework unwind, rejected-result
destructor unwind, and fallback-builder rejection/panic after nonzero prework. They do
not add every combination of account-start denial, outer derived inheritance, foreign
account, forgotten attached canonical permission, region-internal caught parser unwind,
evidence-builder fault or protected-preflight rejection before startup effects. These
remain coverage limits; the source reviews do not establish production defects from them.

The next connected cut must retain the original account/ticket through capture cleanup
and final checks, then provide honest unattached terminal facts before moving actual
preparation ahead of attachment. Fixed/dynamic borrowed backing, shared-shell promotion,
exact administrative coverage, remaining `LocalWork` and owning-path deletion, main
integration, and the unchanged Windows/Linux 14-case/110-lane time/memory and separate
heap gates remain open. Algorithms, work weights, limits, benchmark tasks and clocks are
unchanged.
