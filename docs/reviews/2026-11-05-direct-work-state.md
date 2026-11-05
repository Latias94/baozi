# Own Numeric WorkState Directly

Implementation and test migration are complete against `771cd66`. The workspace
all-feature and all-target checks pass without warnings. All seven final serial
gates and independent source/test/evidence reviews pass. No performance result is
assigned to this source.

## One Direct State and Its Actual Borrowers

Ledger now directly owns the existing immutable work catalog and WorkState row
buffer. LocalWork, BoundWorkGroup, numeric RefCell guards, availability probes and
borrow-fault recovery are deleted. Construction transfers the same admitted
buffers and capacities; the numeric WorkView/WorkLayout/WorkFrame/ParserFrame
kernel remains the single arithmetic and ownership implementation.

Actual numeric writes, policies, modes and publication changes take mutable
Ledger access. Cold writer binding retains caller-order diagnostics and the
existing distinction between ordered physical prefixes and atomic direct
consumption. Read-only range projection, snapshots, evidence and storage-only
release APIs retain shared access. External invocation entry still requires
quiescence; Internal binding can coexist with its actual enclosing frame.

InternalWork carries only a catalog slice and a mutable WorkState loan. Its
reborrow checks actual integrity. Frame resolution directly lends the state after
active, aggregation and shape checks, allowing valid pending cleanup while an
existing integrity fault remains recorded. There is no replacement owner,
counter mirror, test-only compatibility backend or unsafe interior mutability.

## Preserve Admission and Failure Ownership

The coordinator retains separate shared and mutable ledger accessors. Its poll
checkpoint split lends a disjoint mutable ledger for real work; progress/evidence
splits remain shared. Retained producer and control checkpoints use those loans.
Deferral eligibility and account identity checks still precede sealing; a deferred
stage is installed only after sealing succeeds, and rejection returns its owner.

Failure-work qualification destructures the baseline/account and coordinator
fields. The account preflight guard remains alive throughout actual policy
transfer, then ends before capability updates and whole-receiver error handling.
Transfer still projects both policies before committing either; ordinary policy
tightening retains its distinct install-before-limit-error behavior.

Startup retains deferred work/layout errors after storage/live checks and before
commitments. Its now-infallible numeric read adds no early integrity or quiescence
gate. Snapshot evidence admission and allocation still precede work-integrity
checking and joint storage observation. Actual work faults map to
SnapshotShapeMismatch; the impossible WorkStateUnavailable error is removed.
Rollback still destroys backing before releasing its evidence credit.

Consuming Ledger finish and proof detach mark actual catalog/row shape failure at
the former owner-consumption boundary. They preserve dirty final evidence and
the existing shape/ownership-before-live/pending error order. Escaping reservations,
batches, graphs and lease release continue to use the original storage Arc
identity, including foreign-owner and extra-strong-owner rejection.

## Test and Verification Status

Independent bounded design review accepts the cut after inspecting its account
guard, deferral order, consuming integrity, startup/snapshot order, actual frame
ownership and storage identity obligations. The design and interface receipts
are retained under `.git/baozi-review/performance-convergence/` as
`direct-work-state-design-review-1.md` and `direct-work-state-interface-1.md`.

Actual production exec76891 passes the complete workspace all-feature check
without warnings. Its original log is
`target/review/convergence/direct-work-state-workspace-check-1.log`.
The first all-target check, exec49447, runs during raw-test migration and rejects
remaining removed-type references and five external fixture mutability sites.
That construction failure is retained. The external sites now use actual mutable
loans; their behavioral assertions remain unchanged.

Caller-test migration preserves all 96 source tests and their 607 assertions in
Ledger, baseline attachment and coordinator. A further regression corrupts only
the catalog length before consuming the Ledger: shape failure must precede a live
owner error while retaining dirty 3/1 work, one live byte and one reservation.
Startup and snapshot tests replace
dynamic-guard-only faults with genuine header/catalog-shape failures, retaining
allocation failure precedence, pending observation, current limits and cleanup.
Raw work tests use catalog/state buffers and the actual kernel; 51 declarations
become 48 after four dynamic-borrow-only retirements and one mixed-case split.
Exclusive tests become 17 from the base's 19; parser retains all 27. Actual
host-owned zero/full/partial frames, abandonment, unwind and compile-fail escape
contracts remain required; a forgotten ordinary loan alone no longer creates a
dynamic borrow fault. Source inventories and intended oracles are not runtime
qualification. Detailed maps are `direct-work-state-{raw,exclusive-parser,ledger}-test-map-1.md`
and `direct-work-state-observation-worker-1.md` in the private review directory.

The final all-target workspace check completes successfully in 7.97 seconds
without warnings; its original log is
`target/review/convergence/direct-work-state-workspace-test-check-2.log`.
Final formatting is complete. Actual source1 QA exec32451 passes all 3,588
workspace tests (six existing skips) and 187 doctests, then fails strict Clippy
on the snapshot fixture's three-part return type. The correction names that same
tuple `SnapshotFixture`; production inputs, fixture bodies and assertions remain
identical. The original failed QA and source1 are retained.

Frozen source2 identifies all 1,165 tracked non-document inputs and twenty changed
Rust paths, SHA-256
`00ee43ce99aa4f9b0473383785d8bac6c0c5b4b26c5debc4aeae4c8ca9f851f2`, in
`.git/baozi-review/performance-convergence/direct-work-state-source-2.json`.
Actual exec84138 terminates successfully after all seven serial source2 gates:

| Gate | Result |
| --- | --- |
| Workspace nextest | 3,588 passed; six existing skips |
| Workspace doctests | 187 passed |
| Workspace strict Clippy | All targets/all features; passed |
| Asset-consumer nextest | 138 passed; one existing skip |
| Asset-consumer strict Clippy | All targets/all features; passed |
| Workspace and consumer format checks | Both passed |

Workspace run: `524625e3-79f0-4f8e-bb4c-0f882cf605bb`; consumer run:
`f76cae28-4d4f-4048-a4a2-720496c360dc`. All seven checks retain unchanged source2
hashes. The command/log receipt is
`target/review/convergence/direct-work-state-qa-2.json`, SHA-256
`819a6f339334f76c9f2d72856d1a08161115948cbc9c3e3fadb6396470e39ffe`.

Independent final production review directly inspects all eight production source1 diffs and verifies
the frozen inputs without a blocking finding. Independent test review covers the
twelve changed test files, their exact retirements and surviving physical/frame
oracles, also without a blocking finding. Their receipts are
`direct-work-state-production-review-1.md` and `direct-work-state-test-review-1.md`
in the same private directory. The test review counts seven representation-only
whole-test retirements, two mixed-case splits and one added consuming regression,
explaining the net four-test reduction. Neither static review supplies a runtime
verdict. Independent final evidence review reconciles all seven commands, original
log hashes, terminal counts, frozen inputs and staged Rust blobs without a blocking
mismatch. Reversing the unique tuple-alias edit reproduces source1's exact fixture
bytes; production inputs and test bodies/assertions are unchanged. Its receipt is
`direct-work-state-final-evidence-review-1.md` in the same private directory.
The reviewer inspects completed artifacts; the actual exec84138 terminal exit is
observed by the main agent rather than independently re-executed. QA1 remains failed.

## Remaining Convergence Work

The previous [instruction and heap checkpoint](2026-11-05-retained-owner-attribution-and-memory.md)
does not measure this representation. Fixed/dynamic backing, shared promotion,
exact administrative accounting and indexed selection remain under the
[convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
The subsequent [native/heap/placement checkpoint](2026-11-05-direct-work-state-performance.md)
retains all four original Linux campaigns, their loaded controls and unchanged
allocation observations. A separate identical-binary diagnostic identifies a
large `/mnt/f` executable-placement effect; it does not replace the original
matrix or establish convergence. Current-source Windows ordinary qualification,
the next complete Linux-filesystem comparison and release acceptance remain open.
