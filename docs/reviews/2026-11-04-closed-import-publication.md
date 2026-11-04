# Closed Import Scene Publication

This records a bounded isolated successor to the [closed Scene-edit publication](2026-11-04-closed-scene-edit-publication.md)
under the [lexical invocation-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
The source is not applied to main Rust. The complete import outer-body/cell migration and
posthandoff terminal projection are not qualified here. There is no new ordinary performance
measurement or change to the Windows/Linux 14-case/110-lane release gate.

## Concrete Interface and Ownership

Core's actual `consume_scene_freeze_permit` returns the fixed owned
`Result<PreparedScenePublication, ScenePublicationError>`. Its alternatives preserve success,
rich failure and minimal fallback, with optional control and parser/generation rejection facts.
The real import caller uses that result. Caller rejection/preparation callbacks, arbitrary R,
public handoff tokens, public joint aliases and postcommit root-pairing errors are removed.

The central proof retains the evidence root already inspected by metered validation. Recovery
vacancy and both exact canonical/evidence roots are admitted before generation. Parser rejection
also passes this admission and then disposes the unused generation source without invoking it.
Generation-source panic preserves its original payload and contains the frozen candidate.

Prepared success privately couples the actual generation, core tables/proof and exact prepared
store. Its consuming handoff only moves those owners; no caller can independently exchange the
token and store or perform a later rejecting root comparison. This does not close the separate
late-joint-failure cleanup obligation.

Parser rejection keeps stable code `0x5343_1001`; generation exhaustion keeps `0x5343_0003`.
Both retain the existing rich-evidence validation and frozen preparation, with
minimal fallback when that validation cannot qualify. Import keeps `OriginalImport` for a ready
parser rejection and `OperationTerminal` for generation or terminal preparation failure.
Freeze interruption, destination rejection, central validation and supplemental control keep
their existing authority rules. No normal algorithm, semantic action weight, budget, second
ledger, stronger payload lifetime bound or unmetered ownership allocation is introduced.

## Frozen Source and Windows Evidence

Eleven Core/Import files change from the preceding closed Scene-edit component. The complete
1,406-file snapshot additionally contains the separately checked Operation candidate-body
development slice; this receipt does not claim complete workspace execution.

| Artifact | SHA-256 |
| --- | --- |
| Complete source set, UTF-8 sorted path + NUL + hash + LF | `b2ee093a4290b53a08fb53b4afe346d841e880592cfc7caa8c667a3d4a57412a` |
| Source manifest | `2754a43fc801afc0ab48423059d0040179cc4351a906eec5f169f10ba9441c14` |
| Actual Core/Import QA receipt | `7b2f47716bd58afbeb74ec341fb85d2d904a724fc27fc38436abc8274b3da158` |
| Independent Core/Import source and evidence review | `1e2d392bd1aadfa9d78d690bb61e852b30d318eb7b95553a02936566c9df71c6` |

Local source manifest: `target/review/convergence/closed-import-publication-formatted-source-1.json`.
The complete immutable tree is
`.git/baozi-review/performance-convergence/closed-import-publication-formatted-1-frozen`.
QA is `target/review/convergence/closed-import-publication-development-qa-4.json`; the
independent report is
`.git/baozi-review/performance-convergence/closed-import-publication-source-review-1.md`.
These paths identify retained local artifacts rather than files packaged with this export.

| Actual check | Result and scope |
| --- | --- |
| Complete Core/Import nextest, all features | **1,165 passed**, 4 existing skips; run `d1bd34ca-2c22-4624-af88-21ebe9d376ea`. |
| Core doctests | **7 passed**, including the private prepared-owner contracts. |
| Import doctests | **34 passed**: 7 positive and 27 compile-fail contracts. |
| Core/Import strict Clippy | PASS, all targets/features, `-D warnings`. |
| Core/Import formatting | PASS. |

All five QA commands return zero. Their complete 1,406-input before/after maps are unchanged
and match the frozen source; actual log hashes match the receipt. Initial compiler failures
remain retained: an unconstrained host root type, residual test-owner borrow scopes, sibling
root declaration order and one `err_expect` lint. Repairs change the affected fixtures and
their lexical scopes without removing assertions or changing production budgets.

The real regressions cover foreign recovery and either proof root before source invocation,
both parser outcomes, generation exhaustion, unused-source and generation panic, rich failure
versus invalid-recipe fallback, and independent sibling successes with exact producer prefixes.
The existing multiple-input destructor-panic oracle remains at Operation's actual recovery
admission boundary; it does not reintroduce publication callbacks into core.

Independent review finds no actionable introduced correctness defect in the eleven-file
Core/Import delta and verifies source/QA/log association. Some runtime-Control fallback,
Preparation-error and late joint branches are statically traced rather than separately injected
through the new fixed API. The earlier candidate-only Operation 944-test/58-doctest result
belongs to its own unchanged scoped inputs and is not added to this package run as a new
whole-workspace claim.

## Remaining Connected Work

Import's existing `ImportStoreTerminalEvidence::try_from_ledger` still performs potentially
rejecting projection checks after handoff. Its phase/dimension catalog, header and owner/cause
association need an owner-correct precommit guarantee; borrowing a prepared ledger afterward
would already be too late. This component neither removes nor proves that tail.

Both real import entries still need the actual outer retained body, reference-free pending
stage requests and session-cell cleanup before terminal work. Other connected obligations are
late joint-token cleanup, removing body-accessible legacy terminal authority, borrowed numeric
backing, current-state attachment with one start admission, shared-shell-before-state promotion
and deletion of superseded paths. Later Operation/Postprocess development is a different source
and requires its own exact verification. Full unchanged Windows/Linux timing and whole-child
memory qualification remain open; publication is HOLD.
