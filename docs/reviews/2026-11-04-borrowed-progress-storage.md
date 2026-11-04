# Borrowed Progress Storage

This isolated component connects progress reporting to the unique numeric invocation
under the [lexical-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
Full-service Import, text/format migration, obsolete-owner deletion, main Rust integration
and [performance acceptance](2026-11-04-lexical-host-performance-checkpoint.md) remain open.
The component is not a newly measured optimization.

## Ownership and Behavior

`OperationProgressState` previously held a complete `&Ledger`, although its only storage
operation grew progress evidence capacity. That reference could not coexist with the
replacement invocation's unique work view. `ProgressEvidenceStorage` instead borrows the
same shared storage state and calls the existing evidence allocator with purpose Progress.
It owns no ledger, numeric work, cloned Arc or backing allocation. The coordinator still
owns the progress Vec and its evidence-byte release obligation.

The coordinator splits actual progress metadata, poll and ledger fields. After the ledger
lends exclusive work and ordinary storage, progress binds only to the storage view.
Legacy progress callers use the same state and algorithm. The replaced ledger growth
convenience and exclusive poll-only split are removed; existing internal fixtures bind
the actual progress fields and retain their previous assertions.

`StoreInvocation::report_progress` preserves the existing ordinary checkpoint followed by
the progress algorithm's own poll. Its parser role reborrows the same invocation;
Import root/parser methods perform existing preflight and vocabulary-aware error projection.
No second progress algorithm, numeric provider, work owner or partial-finish API is added.

## Actual Regressions

Two coordinator tests exercise progress capacity while actual exclusive work exists.
One reports after the first physical parser prefix, verifies unchanged `(2, 1, 0)`,
retains actual Vec capacity bytes, completes the remaining prefix and finishes with
`(2, 2)` and zero current evidence bytes. The other denies growth at an actual zero
evidence budget, keeps the Vec and retained bytes empty and preserves the control cause
through final evidence.

Two real public custom-importer cases exercise successful reports and observer cancellation.
The successful case reports root `0`, active child `1` and root `2`; each report polls
twice without consuming extra parser work. It closes the numeric region and submits
the real Scene once. Cancellation at child `1` retains the
literal `(2, 1)` prefix and its primary Observer cause despite subsequent signal
cancellation. Six subsequent work/progress/region/submission attempts fail; no further
physical callback or Scene submission occurs and live storage returns to zero.
External counters and the final result guard against assertion panics being mistaken for
an expected importer failure.

## Sources and Qualification

Source1 contains 1,415 files, set
`c5efd04176c7fa31fdafa5c6320a4297e2d9518fe39db87836444166d4eeaa8a`.
Its first focused run (`a076c8b6-dd76-4044-a801-f3c48358194b`) is retained as
three passes and one failed test; no later source1 checks ran. The new cancellation test
incorrectly expected the primary control cause to also appear as supplemental
`terminal_control_cause`. The existing API documents that field as a later interruption
alongside a primary failure; primary control selection intentionally leaves it empty.

Source2 changes only that new assertion to `is_none`, retaining the explicit primary
Observer cause, work, poll, refusal and submission assertions. All production and other
source bytes are identical to source1. Its 1,415-file set is
`bfd1b51233796b9e0081441da5ba2ef868b86cd1dd7891e2b89e79cb4d14fd98`.
All four focused tests pass (`128a2cd2-222f-4f8b-8230-16d612c48397`), as do 142
Operation/Import doctests. Source2's full workspace run
`4d1fd05e-c09f-44c5-92b2-f64c1a6e4b7f` passes all 3,495 runtime tests across
151 binaries, with six existing skips. All 138 asset-consumer runtime tests pass,
with one existing skip (`8f67be52-9d81-4a8b-ac06-fd30a64a7246`). Strict all-target,
all-feature workspace and consumer Clippy and the root format check also pass. The main
thread runs these checks serially, locked/offline with toolchain 1.95.0, one build job
and one nextest thread, reusing the main target directory.

Independent source review verifies the full frozen base/source1/source2 file maps and
the test-only correction. It finds no new production defect and retains two non-blocking
coverage limits: successful root event delivery lacks a direct observer oracle, and
active-prefix first/second-poll, capacity-denial and observer-panic failures are not all
directly exercised through the new forwarding path. Existing control/evidence tests
exercise the reused mechanisms. These limits remain explicit follow-up work for the
connected real Import migration; they are not claimed closed by this checkpoint.

Exact source manifests, original logs and serial QA receipts are under
`target/review/convergence/`, with prefix `borrowed-progress-` and source/QA suffixes
`1` and `2`. Frozen sources and independent review belong under
`.git/baozi-review/performance-convergence/`. Source1's failed evidence is preserved.
Main's `borrowed-progress-terminal-association-1.json` verifies both full frozen sets,
the exact live source2 set, all eight original QA logs and sixteen before/after maps.
The preparatory formatter changes its input; only its after map is labeled source1.
Independent `borrowed-progress-final-evidence-review-1.md` verifies these associations
and actual result counts without a mismatch, preserves the failed source1 run and
keeps both coverage limits open. The original source assessment remains immutable as
`borrowed-progress-source-review-1.md`. No full-service/format migration, main Rust
integration or ordinary performance acceptance follows from these bounded results.
