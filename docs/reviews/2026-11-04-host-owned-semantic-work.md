# Host-Owned Semantic Preflight

This component follows [borrowed geometry actions](2026-11-04-borrowed-geometry-actions.md)
in the isolated [lexical-host refactor](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
Production callers and legacy fixtures use the new host-owned semantic pass. Source4's
complete serial qualification passes. Independent source review identified two direct host
regression gaps; the test-only source5 successor covers both and passes focused runtime,
strict affected lint and formatting. Final independent source/evidence review closes both
gaps and finds no actionable defect or association mismatch. The main Rust checkout has
not received the isolated refactor.

## Actual Interface and Ownership

`CanonicalPlanning::with_semantic_work` retains one `FnMut` callback and owns the actual
`CanonicalSemanticWorkSession` around it. Callers receive a mutable loan. The public
`begin_semantic_work`, `finish_exact` and `finish_rejected` route is removed, including its
real production and fixture consumers. The library alone decides exact/rejected finish
after the callback and its captures end.

The session owns a short `StoreInvocation`, its bound work layout, and references to the
existing first-fault/semantic state. That invocation lends the existing unique `WorkView`;
each action uses the existing `ActionHost` and `WorkFrame` kernel. Action admission,
physical-prefix completion and partial resolution have one implementation. Ordinary and
semantic storage validation share the existing owner/active-reservation checks. No new
ledger, account, numeric backend or mathematical kernel is introduced.

The return value is `(R, CanonicalSemanticCompletion<'scope>)` on qualified nonempty success.
`CanonicalSemanticWorkError` distinguishes a runtime entry/success-finish error from the
callback's owned domain rejection. A rejected body retains its original domain owner; the
driver's preserved runtime/contract fault still decides the subsequent `planning.reject`.
External input borrows and outer-scoped reservations can travel in R after the numeric loan
ends. Completion keeps its existing outer receipt brand, so sealing and activation occur
outside the semantic callback.

## Cleanup, Control and Real Callers

Semantic actions retain their own `FnMut` captures outside the numeric leaf. A normally
returned value, including a domain `Err`, is physically accounted before capture cleanup.
A panic remains attempted-only for that action; explicit partial resolution precedes
controlled capture destruction and resumption of the original unwind. Completion failures
retain their actual applied prefix and original cause before disposing a discarded value.

The new pass callback has its own controlled cleanup boundary. Captures end before the final
semantic checkpoint and completion issuance. Entry rejection disposes callback captures
while preserving the first runtime cause. Body and capture panics are handled separately;
the actual host remains available to mark rejection and preserve cleanup. A legal forgotten
client borrow leaves that host available to finish. Independently leaked scratch remains
charged under the existing containment rules.

The three production entries are Import Scene activation, Postprocess Scene edit activation,
and Core node-transform edit activation. Each returns its existing source-bound plan or
facts, then seals outside the numeric callback. Import retains `transaction.finish` after
activation. Postprocess retains PreparedEdit, replacement/input and mapping loans through
postflight validation and their existing cleanup-before-terminal boundary. The authoring
and typed edit-control adapters preserve the existing reservation scope and input lifetime.
The normal, triangulation, storage-plan and transformation algorithms and weights remain.

## Regression and Compiler Coverage

Legacy operation, Core, Import and Postprocess fixtures use the same real host entry and
retain exact numeric prefixes, first causes, poll counts, storage release and physical
account assertions. The former forgotten owning-session setup now exercises an empty
forgotten borrow; the actual nonempty forgotten-borrow success is covered separately.

Twelve focused Operation semantic tests include seven existing cadence, domain-error,
scratch and action-capture oracles plus five new host-boundary regressions:

- A forgotten mutable loan preserves the host, completed prefix and returned outer reservation.
- Callback capture cleanup can cancel the final checkpoint before completion is issued.
- A second semantic entry runs neither its callback nor an additional poll.
- Denied entry catches callback-capture failure while retaining the original Control cause.
- Callback body/capture panic combinations preserve the literal completed prefix and unwind.

Public examples use the actual exported session-owner entry to infer the opaque planning
type. Positive cases retain external input/outer reservations and owned facts; corresponding
negatives reject semantic-loan escape, planning reentry and client-side final preparation.
The earlier constructive canonical-host positive/negative pairs are migrated together.

## Frozen Source and Current Evidence

Source4 contains 1,415 files; its complete source-set SHA-256 is
`2f9da8ed342e44830addb4f46ee8029ee712d04875775ede3a7a9d54ceb51dc3`.
The manifest and serial receipt are
`target/review/convergence/semantic-work-development-source-4.json` and
`semantic-work-development-qa-4.json`. Frozen trees and independent reviews are under
`.git/baozi-review/performance-convergence/`. Set encoding is sorted workspace-relative UTF-8
path + NUL + lowercase SHA-256 + LF.

The exact source4 passes **149 affected doctests** and **12 focused semantic tests**, run
`fbc6399c-100e-435c-9993-f60d0a88fd5b`. That focused filter excludes 570 other tests; they
are not manual ignores or a complete workspace result. Its complete serial QA also passes:

| Gate | Actual result |
| --- | --- |
| Whole-workspace all-feature nextest | 3,478 passed, 6 existing skips; run `83b5e547-6d33-4861-86dd-0e5e1e9eeebd` |
| Independent asset consumer | 138 passed, 1 existing skip; run `87561d6b-26a3-4a6a-a975-366ba2dd77c7` |
| Workspace and consumer all-target/all-feature Clippy | Both pass with `-D warnings` |
| Workspace and consumer format checks | Both pass |

Main re-enumerates both complete live/frozen path sets and hashes every file. All 1,415
files match source4, all nine command log hashes match their receipt, and all sixteen
qualification before/after maps plus the preparatory formatter's two maps equal source4.
The terminal association is
`target/review/convergence/semantic-work-source-4-terminal-association-1.json`.

Both bounded independent source reviews report no confirmed production defect. The owner
review identifies missing direct tests for returned-value destruction after a real final
checkpoint failure and nontrivial rejection ownership across finish/capture failures.
The source5 successor changes only private `preflight/tests.rs`, preserving every production,
compiler-contract and consumer source from source4. Its 1,415-file source set is
`278c2ca98ddb13b0ee10bcd1a7f18e4560eb96c57bafce47d5f93deeb9170dde`.

Three additional test functions execute four actual host paths:

- A callback capture signals final cancellation; a returned R then panics on destruction.
  The original Signal cancellation, completed prefix, single drop and contract mark survive.
- Rejection finish observes cancellation but returns the original nontrivial E unmodified.
  Explicit E disposal occurs once; the outer driver publishes healthy Control failure evidence.
- Callback capture and returned E destruction both panic, with and without a real earlier
  cancellation observed through semantic reservation. Cleanup preserves the first actual
  cause, completed prefix, single E destruction and final containment.

Every new path uses the actual public-host transaction and private semantic entry. External
completion markers prove the immediate ownership/cause/prefix assertions ran despite the
activation entry's panic containment. No helper-only injected first fault substitutes for
these paths. Source5 passes **15 focused semantic tests**, run
`2cd4683e-83f7-4139-bb6b-e1ceee7a0e87`, plus Operation all-target/all-feature strict Clippy
and the workspace format check. The 570 other tests are filtered. These are affected
successor checks, not a claim that source5 reran the complete workspace or consumer suite.

The source5 manifest, four-command receipt and terminal association use the same naming
with `source-5`, `qa-5` and `semantic-work-source-5-terminal-association-1.json`.
Both live/frozen trees, exact single-file delta and all four logs match. Its preparatory
formatter changes the newly written test file; its after map equals frozen source5.
All three subsequent checks have unchanged before/after maps equal to source5. The final
independent source/evidence review verifies the complete source4/source5 association,
actual result counts, unchanged skip declarations and each new regression's intended branch.
It closes both earlier test gaps with no actionable correctness finding. The three source
and evidence reports remain under `.git/baozi-review/performance-convergence/`:
`semantic-work-owner-review-1.md`, `semantic-work-callers-review-1.md` and
`semantic-work-final-evidence-review-1.md`. Earlier reports remain unchanged; the final
report records the subsequent closure and exact evidence boundary.

Historical failures remain associated with their actual predecessors. Source1 strict lint
rejects two explicit drops of non-Drop invocation permissions; source2 removes those calls.
Source2 then exposes a large inline domain-error lint in one Core fixture; source3 adds a
scoped test-function reason while retaining that original owner. Source3 passes strict
workspace lint but its two new positive compiler examples fail because they name an opaque,
unexported planning type. Its three corresponding negative passes are not qualified for
their intended expressions. Source4 changes only the new Markdown examples to use the
actual public entry. Earlier results do not substitute for source4 qualification.

## Remaining Work

This removes the real semantic dynamic numeric consumer. Import's other execution paths,
startup/share consumers and the underlying `LocalWork`/`BoundWorkGroup` route remain open,
along with current-state single-start attachment, shared-shell-before-state promotion, exact
accounting and final obsolete-owner deletion. Main Rust application and the unchanged
Windows/Linux 14-case/110-lane ordinary timing/memory campaign also remain open. This
component supplies no new performance keep; release remains HOLD.
