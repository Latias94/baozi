# Startup Graph Keeps the Outer Ledger

Retained startup no longer transfers the numeric ledger into its attribution graph and
returns it as `StartedLedger` to rebuild the coordinator. The actual coordinator/common
is installed in the outer root before graph admission and remains there through sharing,
physical admission and payload construction. `StartedLedger`, `from_started_ledger` and
the account convenience constructor that created a ledger are deleted.

This is an ownership prerequisite in the
[lexical-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md),
after the [connected Import checkpoint](2026-11-05-connected-import-services-and-formats.md).
It does not complete initial outer binding, current-state attachment, shared promotion,
old numeric-owner deletion, main Rust application or performance convergence.

## Actual Transition

The existing operation-start ticket is acquired once, before coordinator/ledger creation.
That ticket prepares the graph against the ledger already held by the outer root.
Admitted, shared, Ready and uncertain-containment graph states own only their actual
storage owners, backing buffers, reservation cores, manifest and share envelope. Before
any account share, the graph checks all four storage identities against the supplied
existing ledger. A foreign ledger is rejected without work, sharing or physical mutation.

Ready returns only the attribution baseline and physical guard. Forgetting or containing
that private graph cannot retain a numeric ledger, a control owner or a common/root loan.
Its independently owned storage charges remain honest. Existing six reservation specs,
four real share steps, their completed work, physical manifest checks and admission-first
poison semantics remain unchanged. No counter kernel or algorithm is replaced.

Tests now exercise the actual ownership path instead of three retired source-string
assumptions. They preserve existing work and live scratch across graph preparation,
verify foreign-ledger refusal before sharing, bind the runtime baseline only to its real
owner, and forget Ready while the separate outer numeric ledger remains usable and
closable. A retained-root regression denies physical startup with the minimum legal
account byte limit, verifies the four shares and `8/8` work prefix, and checks that the
actual control remains in common until root destruction. The first version of that new
fixture used a forbidden zero-byte account limit and failed during setup; the failure
and its corrected successor remain recorded.

## Final Source and Verification

`target/review/convergence/startup-borrowed-graph-production-source-2.json` identifies
1,418 frozen files, set SHA-256
`89f2eab8237eadf17fe40520732b95a53930fa027f3c1fc1dde8fae8edba8a32`.
Exactly nine Operation files differ from the preceding connected Import source8.
The earlier production-only source1 excluded two then-moving test files and does not
qualify this final successor.

| Final source2 check | Result |
| --- | --- |
| Whole-workspace runtime | 3,514 passed; 6 existing skips; run `53ef44de-3f8d-4524-8c8b-01e44432aa51` |
| Whole-workspace documentation | 162 passed |
| Workspace strict Clippy | All targets/all features, warnings denied; passed |
| Asset-consumer runtime | 138 passed; 1 existing skip; run `e6763b96-47a4-4f3c-b014-c4c221730e5b` |
| Asset-consumer strict Clippy | All targets/all features, warnings denied; passed |
| Formatting | Whole workspace and the asset-consumer package passed |

All checks ran serially on Windows with Rust 1.95.0, locked/offline dependencies and one
build job; runtime checks used one test thread. Receipts are
`startup-borrowed-graph-production-qa-2.json` and
`startup-borrowed-graph-consumer-qa-2.json`. The corresponding
`startup-borrowed-graph-terminal-association-2.json` verifies every live/frozen file,
all fourteen before/after source maps and all seven actual log hashes.

Independent `startup-borrowed-graph-source-review-2.md`, under
`.git/baozi-review/performance-convergence/`, verifies the complete frozen source and the
nine-file delta without an actionable finding. It reviews source correctness; it does
not substitute for the subsequent terminal qualification above.

## Remaining Connection

The initial pure `LedgerLimits::try_qualify_startup` still precedes this binding path.
Core/Import still supply account, source, control and ledger inputs at their existing
start entry. The next connected change must bind those inputs outside client work,
retain the original admission ticket in the outer frame, and qualify attachment against
actual work, pending competition, tightened policies, phase state and live scratch.
Baseline attachment qualification must be separated from final exact-residue checks
without weakening publication. Shared-shell promotion, `LocalWork` retirement, main
integration and the unchanged Windows/Linux 14-case/110-lane time/memory and separate
heap gates remain open. This isolated source supplies no performance keep or release
qualification.
