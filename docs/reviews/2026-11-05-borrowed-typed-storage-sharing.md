# Borrow Typed-Storage Share Work

The real `CanonicalLane::try_share_storage` path now borrows the existing numeric
owner through its reference clone and retained-handle construction. It no longer
creates and discards an unrelated `RetainedWorkControl` to obtain a full shared
Ledger reference. Its unused materialization-dimension argument is removed.
Public source-account checks, account preflight, action admission and the
clone-before-handle completion order remain at their existing boundaries.

`BorrowedRetainedShareWork` owns a unique `WorkView` loan and the existing
`WorkLayout`/`WorkFrame` metadata. Completing either physical prefix performs no
RefCell borrow. Partial resolution preserves completed work and releases only
the unused pending suffix; abandoned ownership remains an integrity failure.
No callback receives this private numeric owner.

The existing allocation-sharing executor is now one retained-private function.
The old share permit and the new borrowed host delegate to it, preserving the
physical clone, membership clearing and failure-owner return without widening
the visibility of retained allocation types. Account-sharing execution continues
to use the existing `AdmittedShareWork` contract.

An Internal action can reborrow while an enclosing frame remains active. This
entry retains current key, mode and protected-role checks and rejects forgotten
dynamic guards. External ordinary invocation entry still requires quiescence.
`LocalWork::bind_layout` is shared with the remaining legacy binding adapter;
there is no second arithmetic implementation or numeric state.

## Regression coverage

Seven new focused oracles cover both forgotten dynamic guard forms, a nested
Internal envelope with the original outer pending prefix intact, duplicate-row
clone/handle completion, attempted/consumed admission denial, and a fault injected
after the actual reference clone. The latter returns the cloned physical owner
with one consumed unit and resolves only its pending suffix. Tests also check
source-first release, final-owner release and unchanged physical charges.
Direct host tests now check abandonment and explicit partial resolution before
and after a manually recorded clone checkpoint. Abandonment rejects quiescence
and finalization while preserving the last attempted/consumed snapshot; explicit
partial resolution preserves that prefix and leaves the ledger quiescent. These
two metadata tests do not claim to execute a physical reference clone.

The existing real preparation/share/build integration test additionally checks
that the derived table points to the source backing. It continues to exercise
Internal share completion before Producer admission, another table construction
afterward, and exact account restoration.

## Qualification

Base commit: `4efe6c4f427125c2d4fb937572cde672961fdc9d`.
Source and verification records are retained under
`.git/baozi-review/performance-convergence/borrowed-typed-share-*` and
`target/review/convergence/borrowed-typed-share-*`, respectively.

Source3 changes only the new test file from source2: it replaces a Clippy-rejected
`.err().expect()` assertion and adds the two direct host oracles. Production source
remains byte-identical. The failed first Clippy result is retained in QA1; it is
not described as a successful run. QA1's 188 workspace doctests qualify the
unchanged production source. The six subsequent source3 gates all pass:

| Gate | Result |
| --- | --- |
| Operation runtime | 1,054 passed, zero skipped; run `8b3a7438-7893-4768-a086-6cfdea094d8c` |
| Workspace strict Clippy | All targets and features, warnings denied |
| Independent asset consumer | 138 passed, one existing skip; run `572533d2-8738-4f5b-abd8-6c1564d28586` |
| Consumer strict Clippy | All targets and features, warnings denied |
| Workspace and consumer formatting | Both passed |

Source3 manifest SHA-256 is
`3f2a650210f6846d5eb4609dad54a4488277f42b25ccda2ccb19c1e370bf981b`.
The final receipt is `target/review/convergence/borrowed-typed-share-qa-2.json`,
SHA-256 `1165f78e95b438f2f59655cd8e40aa433296bb42da695ceb3ccb693f24e090f0`.
Its orchestrator actually exits zero; all ten reviewed inputs remain unchanged
through its six gates. Cargo runs serially with locked/offline dependencies,
one build job, one test thread and the shared root target. No full workspace
runtime or new performance result is assigned to this affected-gate checkpoint.

Independent production and test-only follow-up reviews find no blocking defect.
They record direct coverage limits for distinct-dimension/mode/error combinations,
actual kernel/handle completion faults, and public-path clone-call observation
under an enclosing Producer/parser frame. The Drop test alone does not prove
pending release because the integrity error takes precedence over that check.
Existing common-kernel coverage and source equivalence support these cases;
the new tests do not establish them all.

Independent final evidence review finds no mismatch in the ten source identities,
six terminal command results, raw log hashes/counts, shared target or the retained
source2 doctest association. The bounded report is
`.git/baozi-review/performance-convergence/borrowed-typed-share-final-evidence-review-1.md`.
It also verifies that the five unrelated user notes remain untracked and untouched;
it does not claim new performance qualification.

## Remaining work

This is the typed-storage share leaf of the connected retained-work design.
Canonical block attribution, controlled builders and validation, inheritance,
protected evidence, startup/publication shares and surviving full-Ledger aliases
still require migration. Their final callers must move before their old permits
and `LocalWork` can be deleted.

No new performance or heap result is assigned to this source checkpoint. The
unchanged Windows/Linux time and whole-process memory comparisons and separate
heap qualification remain open under the
[convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
Release remains HOLD.
