# Borrowed Geometry Action Execution

This component follows [final invocation authority](2026-11-04-final-invocation-authority.md)
in the isolated [lexical-host refactor](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
It moves the actual Postprocess action adapter onto the existing exclusive numeric kernel.
Frozen source2 passes workspace runtime, affected compiler documentation, independent
consumer, strict lint and formatting checks. Independent connected-source and final evidence
reviews are complete without a finding. The main Rust checkout has not received the refactor.

## Actual Ownership Change

`StoreActionExecution` owns a short `StoreInvocation` loan, one bound singleton layout,
the actual pending `WorkFrame` and the captured action cost. Its outer entry owns the host;
the callback receives only a mutable borrow. The host and its unfinished frame cannot be
extracted or returned. Owned results can leave after explicit close.

The real work rows/header are borrowed once through `ExclusiveWork`/`WorkView`. Begin,
completion, replenishment and resolution call the existing `WorkLayout`/`WorkFrame` kernel.
There is no new numeric backend or whole-Ledger alias alongside that unique loan. Completed
leaves keep the one producer envelope at zero remaining and replenish it for the next real
action, preserving its explicit ownership requirement.

Postprocess `ActiveRuntime` is now a type alias for that host. Its old work/group/services,
owning permit, Begin/Complete implementation and cleanup dispatcher are deleted. The actual
normal, triangulation, preparation and space consumers retain their literal event calls.
ScratchVec reserves/reconciles through the new host and keeps payload-before-lease cleanup.
No numerical kernel, action weight, selected task or benchmark boundary changes.

The two entry implementations share the same invocation-context constructor. Control-cost
calculation remains the existing helper; ordinary services and the new action host share
the same `ExecutionOrdinaryStorage` cadence methods. The original ordinary/parser execution
paths remain real consumers, not a new compatibility backend for geometry.

## Preserved Failure and Control Behavior

- An outstanding Begin rejects another Begin before polling/admission. A captured cost is
  installed only after the actual one-unit admission succeeds.
- Completion records the literal physical prefix before returning a ledger failure. It
  clears the captured action and resolves pending ownership before returning that cause.
- Successful exit explicitly closes the producer envelope, checks quiescence and polls.
  Known errors close without another poll and retain the original callback error. Unmatched
  Begin rejects successful close, even when owner-only fault injection completed its frame.
- Unwind/drop with an owned envelope latches the existing ordinary-permit abandonment cause
  and resolves its uncompleted suffix without inventing consumption or polling. Existing
  first-cause precedence remains in the actual operation owner; resolution faults retain
  their ledger failure and unresolved evidence.
- Scratch uses the same logical storage, physical account and Internal share-work executor.
  It may coexist with the producer envelope; real clone/handle charges and release order
  remain observable in the consumer regressions.

## Consumer and Owner Regression Boundaries

All fourteen Postprocess execution regression functions remain. Their public physical-prefix,
exact cause, poll, cancellation and physical-refund assertions remain at the actual consumer.
Cold work observations use copied `work_counters` facts instead of borrowing the old private
permit. One weighted test is renamed to describe that public contract.

Four Operation-owner tests cover the relocated private oracles: synthetic completion with
an unmatched Begin, failed-completion immediate resolution, singleton replenishment and
Attempted/Consumed denied refill. The unmatched test measures the actual cancellation signal
and proves that rejected close and subsequent Drop do not poll in either physical-completion
branch. Denied refill retains the existing zero-remaining frame, captures no new action cost,
preserves `(2, 1, 0)` and the original ledger cause, and explicitly closes without polling.
Public Rustdoc pairs allow an owned result and reject returning the host's mutable loan.

## Source and Qualification State

Source2 contains 1,414 files with source-set SHA-256
`a45bef1b315c41fa7adf4faaabecb3970c297623008e37d0ad9218c05eb575a9`.
Its complete manifest/QA are `target/review/convergence/borrowed-geometry-development-source-2.json`
and `borrowed-geometry-development-qa-2.json`. Frozen trees and independent reviews are under
`.git/baozi-review/performance-convergence/`. Set encoding is sorted workspace-relative UTF-8
path + NUL + lowercase SHA-256 + LF.

The exact source2 results are:

| Check | Result |
| --- | --- |
| Whole-workspace nextest, all features | **3,473 passed**, 6 existing skips, 151 binaries; run `8ed25fbc-f46a-4780-9b26-7344bd4342c2` |
| Operation/Core/Import/Postprocess doctests | **144 passed**, no failures or skips |
| Independent asset consumer nextest | **138 passed**, 1 existing skip; run `1a91eef7-cb31-4807-bee3-877f29316773` |
| Workspace and consumer Clippy | All targets/features, `-D warnings`, both passed |
| Workspace and exact consumer formatting | Both checks passed |

All builds/tests use Rust 1.95.0, locked/offline dependencies and serialized one-job
execution. The runtime's separate one-test filtered subprocess is nested evidence and is
not added to the 3,473 nextest total. Both complete live/frozen trees match all 1,414 paths
and hashes. All eight logs match their receipt hashes. The seven verification commands'
14 maps match source2; the preparatory formatter's two maps also match this source.

| Artifact | SHA-256 |
| --- | --- |
| Source2 manifest | `e6c6520a3dfe7a375f1cf3ff2474afdaed8d7c686d89bf62e0dac87f16fdf470` |
| Source2 serial QA2 | `109444f9f0708b71ddec8147e254c467a176155e6fa320ca05dc334f606024bf` |
| Independent initial Operation-owner review | `3bf65c8ae75442c843acea068a4cb64175e2c309a5bd5589c5bc64395213cb4d` |
| Independent connected source2 review | `02139eed8beab614a94d016160b0d80eae439aecd0e00e96cae2c44d8b112403` |
| Independent final source2 evidence association | `4cf8c816daf9c7ace7c7a58de77288bb562256198b9a1edc71e84f1947ce9434` |

The connected independent review finds no introduced defect or remaining source-oracle gap.
It checks the ten-file Operation and two-file Postprocess delta, actual G1/G2 owner
assertions, all fourteen consumer scenarios and unchanged numerical kernels/weights.
The final independent review verifies both complete source trees, all command maps and
logs, actual totals and unchanged skip declarations. It also confirms the four owner tests'
compiled registration and inclusion in the unfiltered workspace run, without inventing
individual PASS rows or adding the nested subprocess to the total. No bounded source2
qualification gate or evidence-association gap remains.

Predecessor development evidence is separate: Operation library/all-target checks, connected
Core/Import/Postprocess all-target checks, scoped strict lint, three initial owner tests and
99 Operation doctests pass. Independent owner review finds no confirmed defect but requires
the two explicit private-state/poll coverage additions described above. Source1 includes them
but fails compilation because its test fixture uses the Postprocess wrapper's cancellation
method name on OperationControl. Source2 changes only that test call to the actual
`with_cancellation`; the original source1 and E0599 log remain immutable. Earlier passes do
not substitute for source2's actual complete results above.

## Remaining Work

This is the geometry action consumer migration. `LocalWork` and `BoundWorkGroup` still serve
real Import, semantic and startup consumers and must be removed in the connected borrowed
backing migration. Current-state single-start attachment, shared-shell-before-state promotion,
exact accounting, main Rust application and all unchanged Windows/Linux 14-case/110-lane
timing/memory requirements remain open. No new performance campaign or performance keep
belongs to this component; release remains HOLD.
