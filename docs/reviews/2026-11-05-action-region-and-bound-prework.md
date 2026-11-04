# Share Action Execution Before and After Attachment

The isolated successor uses one action-region runner for attached operations, bound
prework and the canonical bound-body permission. It repairs two reproduced execution
failures while preserving the existing numeric kernel, physical action weights, pending
prefix resolution and control checkpoints.

This is a correctness checkpoint in the
[lexical invocation-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
The Rust remains in the isolated compiler workspace. Main integration, actual late
preparation, backing/promotion and ordinary performance acceptance are separate work.

## Reproduced Failures and Shared Repair

Two direct tests against the preceding qualified source fail: ignoring an actual semantic
`Err`, and catching a panic before the first numeric action, both allow another action
region to enter. The retained red run records two failures, not a successful qualification.

`InvocationRegionGuard` is extracted from the existing exclusive-region host and reused by
`execute_action_region`. It borrows only execution/failure state; it adds no numeric alias.
An actual error return stops ordinary work. Unwind, incomplete actions and rejected-result
destruction remain under that guard. Normally returned physical prefixes, action weights,
pending-frame resolution and the original entry/success-exit polling remain intact.

The first affected run then exposes an existing Postprocess fixture that handles a real
error but still demands successful terminal publication. Its source1 result remains
2,511 passed, one failed and six existing skips. Source2 changes only that test: the
handled-error branch requests its actual domain fallback and requires Requested class
with exact code 41; the success branch still requires unchanged Success. Both retain the
original error identity, physical `[1, 2, 3]`, exact work prefix and exit-poll assertions.
Accepting an arbitrary failure would not satisfy this oracle.

## Frozen Source and Complete Checks

The final manifest is `target/review/convergence/action-region-boundary-source-2.json`:
1,421 files, source-set SHA-256
`420304e1740704664a86e626317b0d287916e6047fcea0c28634386ed16ec50d`.
It is a test-only successor of the reviewed source1 production repair.

| Source2 check | Actual result |
| --- | --- |
| Whole-workspace nextest | 3,557 passed; six existing skips; run `74f17111-3a02-4647-9105-e96c31e877e5` |
| Whole-workspace doctests | 174 passed |
| Workspace strict Clippy | Passed, all targets/all features, warnings denied |
| Independent asset consumer | 138 passed; one existing skip; run `37e687cc-84d3-41a6-8145-0a48999faa17` |
| Consumer strict Clippy | Passed, all targets/all features, warnings denied |
| Workspace and consumer formatting | Both passed |

Checks run serially on Windows using Rust 1.95.0, locked/offline dependencies, one build
job and one nextest test thread. All seven child commands and the observed parent finish
successfully. The QA manifest, seven original logs and historical terminal association are
under `target/review/convergence/`, with prefix `action-region-boundary` and revision 2.
The association captures live/frozen equality before subsequent construction edits;
it does not assert that the advancing live workspace still equals this snapshot.

## Independent Review and Limits

The bounded reports under `.git/baozi-review/performance-convergence/` are:

- `action-region-boundary-source-review-1.md`, covering source1 production and the source2
  fixture correction, with no remaining actionable component finding.
- `action-region-boundary-final-evidence-review-2.md`, independently hashing all 1,421
  frozen files, reconstructing the set digest, checking all 14 QA source maps and seven
  log hashes, and reconciling raw results and preserved failures without a mismatch.

Direct coverage does not include every combination. An incomplete action combined with
a rejected returned owner's destructor panic, and bound prework panic before its first
numeric action followed by an attachment attempt, still lack their own direct execution
evidence. The reports do not promote those gaps to proven production defects.

The next implementation retains the original account/admission through capture cleanup,
extracts honest unattached facts, and moves actual preparation/unchanged classification
before attachment. Those advancing changes do not inherit this source2 qualification.
Fixed/dynamic backing, shared-shell promotion, exact administration accounting, remaining
owning-path deletion and main Rust integration remain open. The unchanged Windows/Linux
14-case/110-lane time, whole-child memory and separate heap gates remain HOLD; this
checkpoint supplies no new timing, allocation-free, portable stack or release claim.
