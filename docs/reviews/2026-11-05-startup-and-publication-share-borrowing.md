# Borrow Startup and Publication Shares

Implementation and test migration are complete against `609d0d6`. Production
workspace and Operation all-target compilation pass without warnings. The final
seven serial workspace/consumer gates and independent final evidence review pass.
This document does not establish performance acceptance.

## One Numeric Owner Through Startup and Publication

Startup preholding, publication account opening and canonical/evidence table
creation now borrow the existing InternalWork and disjoint storage rather than
retaining a shared full-Ledger permit. The numeric owner remains in the invocation.
The four startup shares resolve separately before the aggregate physical startup
transition. Graph/ledger identity is checked before splitting numeric work, and
Ready/containment owners retain storage without borrowing the numeric ledger.

Startup bindings now retain a deferred `Result<WorkLayout<8>, LedgerCause<_>>`
and the original amounts. Production admission observes the same WorkState through
the unique view. Test-only shared preflight reuses the same projection and kernel,
including already pending work. Current publication and live limits precede the
deferred work error; work precedes commitments. Projection charges no work and
adds no quiescence requirement. Six reservation cores still commit before eight
backing allocations, with the existing physical-before-logical rollback order.

Actual clone/handle execution uses the existing borrowed share frame and physical
executor. Startup passes each step's complete charge pair, preserving duplicate
dimensions and non-unit amounts. Attempted work is admitted before the clone;
Consumed advances only after its actual reference clone and typed handle. A later
rejection retains prior physical work and releases only unfinished pending capacity.

Publication opening retains its account action and set-id admission before share
work. Table creation retains account-health preflight, batch slot, account action,
share work, logical attribution, physical admission, clone/handle, allocation,
membership and batch insertion in that order. Attribution borrows the admitted
share's numeric view beside storage, retaining protected destination checks without
acquiring a second full-Ledger reference. Existing panic boundaries and cleanup
classification remain; no control poll is added or removed by these helpers.

## Delete Retired Owners With Their Last Callers

`RetainedShareWorkPermit`, its old admission helpers and the unused account-sharing
entry are deleted. Once these actual callers migrate, compilation identifies
`LinearWorkBatchPermit` as production-dead. That owner, its BoundWorkGroup admission
method and the three test-only Ledger constructors are also removed. Their useful
regressions are being moved to the existing WorkLayout/WorkFrame kernel and actual
borrowed hosts, without a renamed test-only copy of the old owning adapter.

The cold BoundWorkGroup charge/preflight methods and LocalWork remain because
direct accounting, policy, mode transitions, range qualification and terminal
observations still use them. Their remaining deletion is separate actual caller
work; this cut does not claim that LocalWork is retired.

## Verification Status

The first compilation exposes a closure inferred as FnMut while returning a
borrowed share across `catch_unwind`. Moving the unique loan explicitly into the
FnOnce body fixes that boundary. The second compilation succeeds but reports the
now-unused linear owner. After deleting it, actual exec90910 completes the third
workspace all-feature check without warnings. All attempts and original logs are
retained under `target/review/convergence/startup-share-workspace-check-*.log`.

Existing startup regressions retain current-policy/pending-consumption projection,
foreign-ledger rejection, four completed shares, physical no-mutation rollback,
uncertain panic and Ready drop/forget behavior. Additional probes distinguish
forgotten immutable versus mutable guards at the original read/admission boundaries.
The new share regression uses duplicate costs 2 and 3, with exact successful 20/20
and rejected 15/10 work, literal four/two account clones and unchanged physical
storage. The final workspace run executes these regressions successfully.

Operation's all-target/all-feature compilation also succeeds without warnings in
actual exec19446. Final formatting is complete. The first full workspace run,
actual exec26248, finishes with 3,591 passing tests, one failure and six existing
skips. The new nonunit-share fixture requested attempted limit 14 and consumed
limit 20, which LedgerLimits correctly rejects before the scenario runs. It now
uses valid 14/14 limits to test attempted-error precedence; the separate 20/14
row still isolates consumed denial. The original failure and source1 remain
retained in QA1.

Frozen source2 differs only in that one test-data row. It identifies all 1,165
tracked non-document inputs and twenty changed Rust paths, SHA-256
`08831182f94bc2894417827ec63705624af82aa037605450d93dfce529d26cd1`, in
`.git/baozi-review/performance-convergence/startup-share-source-2.json`.
Actual exec55021 terminates successfully with all seven serial gates on source2:

| Gate | Result |
| --- | --- |
| Workspace nextest | 3,592 passed; six existing skips |
| Workspace doctests | 187 passed |
| Workspace strict Clippy | All targets/all features; passed |
| Asset-consumer nextest | 138 passed; one existing skip |
| Asset-consumer strict Clippy | All targets/all features; passed |
| Workspace and consumer format checks | Both passed |

The workspace run is `06bfa2e4-39e2-4bfe-b9b0-9d0868a12797`; the consumer run is
`8726f352-0f58-4f8c-9a80-642fa467946d`. All seven checks retain unchanged input
hashes. The command/log receipt is
`target/review/convergence/startup-share-qa-2.json`, SHA-256
`5a1390bceb134fd0c11d1f7cf0449383296aa727c3a50bf6d08f86d3be82f901`.

Independent production and test reviews directly inspect the twenty-file source1
diff and verify its frozen input identities without a blocking finding. Their
receipts are `startup-share-production-review-1.md` and
`startup-share-test-review-1.md` in the same private directory. The bounded
`startup-share-test-followup-2.md` reproduces the sole source2 fixture correction,
confirms unchanged production inputs and verifies all current source2 hashes.
Neither static review overrides QA1's failed execution. The final independent
evidence review reconciles all seven command/log identities, terminal counts,
current frozen inputs and staged Rust blobs without a blocking mismatch. Its
receipt is `startup-share-final-evidence-review-1.md` in the same private
directory. The reviewer inspects the completed artifacts; the actual exec55021
terminal exit is observed by the main agent, not independently re-executed.

The raw test migration maps are
`startup-share-raw-kernel-test-map-1.md` and
`startup-share-raw-caller-test-map-1.md` in the same private review directory.
Caller migration preserves all 105 tests in its three-file entry inventory.
Raw kernel migration deletes one adapter-only consuming-finish case; fixed-size
completion arrays replace its removed runtime-index API. Explicit abandoned
kernel resolution is distinguished from real host Drop/unwind coverage, which
remains in the actual action, retained builder and borrowed-share tests.

## Remaining Convergence Work

The [last measured cumulative source](2026-11-05-retained-owner-attribution-and-memory.md)
has modest instruction savings and unchanged selected heap observations. No result
from that source is reassigned to this implementation. Fixed/dynamic
backing, shared promotion, exact administrative accounting, cold-owner deletion and
indexed selection remain under the
[convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
Current-source ordinary Windows/Linux timing and whole-child memory qualification
are still required before release.

The next coherent representation cut removes LocalWork, BoundWorkGroup and
numeric RefCell together with their remaining shared-write API choices. Store the
existing WorkState and catalog directly in Ledger; cold writers, mode transitions
and policy changes take mutable loans, while snapshots and range qualification
keep read-only access. The mutable control-checkpoint split, deferral sealing
chain and account preflight's disjoint loan must migrate with that change. Keep
the shared storage Arc for escaping release ownership and the same numeric kernel.
Obsolete dynamic-borrow-only faults can disappear with that representation;
actual frame ownership, shape/integrity faults and host cleanup must remain.
This next cut remains analysis only in this checkpoint.
