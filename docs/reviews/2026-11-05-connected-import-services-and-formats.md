# Connected Import Services and Format Callers

The isolated Import migration now compiles and passes the complete workspace and real
asset-consumer gates. Detection, dependency acquisition, text, OBJ/MTL, PLY, STL and glTF
use short root, parent, child, services and storage loans over the same invocation. Import's
retired owning execution adapters are deleted, and their original test callers have moved.
This completes the connected Import construction checkpoint in the
[lexical-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
The Rust successor remains unapplied to main, and performance acceptance remains HOLD.

## Actual Error and Scan Boundaries

Independent review and real regressions exposed two error-ownership mistakes during
construction. A glTF format error was materialized only after its numeric region had
closed; moving materialization into that same root corrected the outer boundary. Existing
post-result ordinary polls could still overwrite an inner error after a child had stopped
execution. `poll_result` now uses the original ordinary checkpoint for success and the
existing prescribed exit checkpoint for error. It preserves control-error priority without
reopening failed work or dropping the prescribed poll. Original URI failure context survives.

The JSON scanner also returns a predicted syntax-budget boundary before publishing the
actual Tokens or Records ledger denial. Treating this handback as an actual callback error
stopped the invocation before the real denial. `ParseRoot::scan` maps the public
`ParseScanReturn::{Exhausted, Boundary}` into the existing closed scan host. Only the
predicted budget handback is a normal boundary; actual parse failures remain errors.
The same existing frame and numeric kernel retain the physical DecodeWork prefix and
release only its unused suffix. Exhausted remains strict, cancellation remains observable
at exit, and a previous error or caught unwind cannot become healthy through Boundary.

Constructing an owned format-error value remains distinct from returning `Err`. Existing
retained-output fixtures may construct that value, validate a candidate, and return it
later. Final Session enforcement still rejects an actually swallowed format error.
The new non-Send owned-error regression verifies unchanged `Rc` identity, a real first
write, exact attempted/consumed `2/1`, and rejection of later work before any unrelated
terminal abort is latched. Three public scan regressions cover boundary plus following
child (`4/3`), incomplete Exhausted (`3/2`), and cancellation at Boundary exit (`3/2`).

The [ordinary-entry baseline correction](2026-11-05-import-root-entry-baseline-correction.md)
still applies. The original ordinary root has one entry checkpoint; the existing parser
facade retains its second checkpoint. Algorithms, work weights, resource limits and
benchmark tasks are unchanged by this migration.

## Exact Source and Verification

The final manifest is
`target/review/convergence/import-real-services-connected-source-8.json`: **1,418 files**,
source-set SHA-256
`0c7e808989e587f80100f538d3a26a9a57ff64b975fb0e61c3b926c404eac073`.
Frozen bytes are under `.git/baozi-review/performance-convergence/` with the corresponding
`import-real-services-connected-source-8-frozen` name. All final checks run serially on
Windows with Rust 1.95.0, locked/offline dependencies, one build job, and one runtime
test thread.

| Final source8 check | Result |
| --- | --- |
| Whole-workspace runtime | 3,510 passed; 6 existing skips; run `2f3361fb-6768-4960-87e2-9ead10559a52` |
| Whole-workspace documentation | 162 passed |
| Workspace strict Clippy | All targets/all features, warnings denied; passed |
| Independent asset-consumer runtime | 138 passed; 1 existing skip; run `882c9cc0-d7a5-4607-96f0-ab3dc7ed7989` |
| Asset-consumer strict Clippy | All targets/all features, warnings denied; passed |
| Formatting | Whole workspace and the asset-consumer package passed |

The seven terminal checks are retained in `import-real-services-connected-qa-8.json`.
The corresponding `import-real-services-connected-terminal-association-8.json` recomputes
every live and frozen file hash, every before/after source map, and every log hash.
All match the final source8 manifest. Earlier failures remain immutable evidence:
source3 had 867/1,000 passes; source4's URI selection had 8/11 passes; source5 had
999/1,001 affected passes; source6 passed 1,004 affected tests and 162 doctests before
strict lint rejected migration redundancies. Source7 removed those redundancies, and
source8 adds four reasoned test-local large-error lint annotations. Those annotations
change no production statement, scenario or assertion and avoid boxing actual inline
errors solely for lint compliance.

## Independent Review and Remaining Work

Bounded source reports are retained under `.git/baozi-review/performance-convergence/`:

- `import-connected-gltf-source-review-1.md` and
  `import-connected-gltf-error-host-review-1.md` preserve the original glTF findings.
- `import-connected-error-value-source-review-1.md` records the owned-error-value
  correction and the initially masked reuse assertion.
- `import-connected-result-frontier-import-review-1.md` and
  `import-connected-result-frontier-gltf-review-1.md` close the reviewed error-frontier
  findings and the new test's masking problem.
- `import-connected-scan-handback-source-review-1.md` finds no actionable defect in the
  typed scan handback and its actual caller/tests.
- `import-connected-lint-cleanup-source-review-1.md` and
  `import-connected-test-lint-source-review-1.md` verify the subsequent bounded cleanups;
  the latter also independently verifies final source8's strict-lint association.

These source reviews have distinct scopes and do not imply a fresh audit of every
unchanged algorithm. Direct runtime combinations of Children/Parser `poll_result` with
an overriding control error and a panicking error destructor remain a coverage limit;
their existing containment paths were inspected. The separate final audit,
`import-connected-final-source-evidence-review-1.md`, independently verifies all 1,418
frozen files, all fourteen final source maps, all seven logs, the terminal association,
the referenced review identities and this checkpoint's reporting. It finds no evidence
mismatch or misleading qualification claim within that scope.

Current-state single-start attachment, shared-shell promotion, remaining `LocalWork` and
owning Operation paths, temporary parser-entry spelling, main Rust integration and the
unchanged Windows/Linux 14-case/110-lane time/memory campaign remain open. Separate heap
attribution is still required. No new ordinary performance measurement, optimization keep,
publication or release readiness is claimed by this checkpoint.
