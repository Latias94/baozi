# Session-bound Canonical Semantic Execution

This is the first implementation experiment of the
[Assimp convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md), following
the [current cost diagnosis](../research/2026-11-03-performance-convergence-diagnosis.md).
It replaces repeated semantic-host binding; it is not a complete convergence result.

## Replacement

Canonical semantic preflight borrows the existing work, storage and control capabilities once
from its operation and binds its work dimension once. Each real action still admits one unit
before execution, completes that unit immediately after normal return, explicitly resolves the
permit and only then advances the control cadence. A returned domain error is completed work;
an unwinding action is attempted-only. No numeric borrow spans an action or destructor.

The session retains scoped scratch reservations, single-use completion branding, nonempty
completion, first-cause precedence and contained callback/result destruction. It uses the
invocation's existing at-most-256-action cadence, with entry, scratch and final checkpoints.
The old semantic-session batch/cold producer-host path and its now-unused cadence method are
removed. This experiment does not change normal arithmetic, OBJ remaps, parser staging or
canonical payload layouts, and does not depend on the separate compact-capability draft.

Pre-cancelled control may now reject `begin_semantic_work` before constructing the session.
No action or work unit runs. Cancellation after entry is still observed by the bounded or final
checkpoint. The active session now borrows the local numeric authority directly; its capability
and scoped lineage restrictions are qualified separately by compiler contracts.

## Failure Corrections Found During Qualification

The first operation run passed 858/859 tests. The existing test for an already selected Ledger
failure exposed an entry regression: the new ordinary execution checkpoint returned an opaque
stopped state, which activation misclassified as an ownership contract failure. Independent
source review found the same problem. The corrected scoped outcome owner resolves an existing
terminal before lending ordinary execution or invoking the semantic transaction, retaining the
original terminal code and the established `SuccessOutcomeRejected`/NeverActivated wrapper.
It does not make arbitrary stopped/contract errors recoverable.

The first affected-caller run then passed 1,399/1,400 tests. The unchanged import test for late
cancellation after Ledger rejection showed that the early terminal exit also needed a control
observation before retaining its failure owner. The corrected cold branch uses the existing
terminal-control poll inside the containment catch boundary. A later cancellation is recorded
without replacing the already selected Ledger code. The source review's earlier conclusion that
downstream fallback preparation alone covered this boundary was withdrawn and corrected.

Tests retain the original healthy fallback expectations. New literal probes verify no semantic
transaction executes on prior failure, capture-drop panic still contains the owner, and forgotten
zero/full permits cannot use the healthy exit. One initial zero-envelope fixture incorrectly used
the legacy zero-rejecting host; it was corrected to the existing zero-capable execution API.
All original failed logs remain retained rather than relabelled as passing evidence.

## Qualification Checkpoint

All commands use Rust 1.95.0, locked/offline resolution, one build job and the existing target.

| Check | Result |
| --- | --- |
| Corrected operation/core/import/postprocess nextest | 2,262/2,262; six existing skips; `6ff4e406-5340-48a6-a55b-ccbf72a081f0` |
| Affected ownership/lifetime/publication doctests | Import 17; operation one positive plus 39 compile-fail; all passed |
| Affected all-target/all-feature strict Clippy | Passed with `-D warnings` |
| Real-format facade control assurance | 9/9; `ab88fff9-2a58-4cf0-828a-95fb79e1ed27` |
| Package formatting | Passed |
| Independent source review | Initial P1 and subsequent control-evidence correction closed; no open P1/P2 at final inspected hashes |
| Asset consumer, all features | 120/120; one existing skip; `e8d0127e-9c89-42a5-ad04-0f5ef6c41291` |
| Ordinary performance and subsequent attribution | Pending at this checkpoint |

The detailed local command/hash receipts are `target/review/convergence/r1-*.json` and matching
logs. The independent design/source reports and experiment ledger are under
`.git/baozi-review/performance-convergence/`. Runtime snapshots are identified by file hashes in
each command receipt; the source commit and immutable ordinary worker will identify the measured
candidate. No performance improvement or release eligibility is claimed before that measurement.
