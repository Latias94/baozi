# Import Catalog Admission Before Terminal Restoration

This isolated successor to [joint payload cleanup](2026-11-04-precommit-joint-payload-cleanup.md)
moves Import's explicit terminal-schema admission before restoration, final progress,
generation and publication. The former fallible report projection after handoff is deleted.
The [lexical-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md) remains
in progress; this component does not complete numeric backing, attachment or performance work.

## Actual Guarantee and Owner

The raw invocation root lends a crate-private cold view of its installed coordinator's actual
ledger. The concrete root checks exact dimension/phase counts, strict catalog order and every
expected semantic identity against that view. It creates no snapshot, key authority, ledger,
account, reservation, callback framework or compatibility backend.

`CanonicalInvocationFinish::admit_terminal_catalog` consumes the final permission. A mismatch
uses the existing owning host cleanup: stop work, dispose actual typed payloads/causes under
their existing panic boundaries, and contain the original physical graph. A rejected finish
cannot be resumed. Success returns that same permission; no raw owner reference escapes.

Import derives the expected eight phases and all 62 dimensions from its existing semantic
definitions: 58 ordinary dimensions and four terminal dimensions. Its private `prepare_scene`
requires admission before `finish.resume()`, so all candidate, rejection, never-activated,
interruption and fallback branches pass the gate. Immutable startup catalogs and terminal
evidence qualification preserve the admitted mapping through final handoff.

Live accounting need not be zero at this gate. Existing typed terminal qualification proves
that actual live bytes/reservations are covered by the owned batches and retained output.
The terminal projection describes their post-commit zero state while actual cover remains
live until the exact account-bound commit releases it. The existing ownership and cause rails
pair success/failure payloads with their own evidence. This repair reuses those guarantees;
it does not replace them with an early zero assertion or a second accounting interpretation.

`ImportStorePreparedScene::handoff` now returns its owned outcome directly. Its report copies
the frozen scalar header and retains the same immutable ledger for lazy row access. There is
no post-commit `Result`, catalog rejection, owner/cause mismatch branch, allocation or new
terminal decision. The operation's actual finite terminal-cause/failure-class enums are
exhaustive, so adding a future variant requires compile-time review of Import's mapper.

The obsolete `ImportStoreTerminalProjectionError`, `ImportStoreFinalizationError`, lookup-only
schema validators, and late `try_from_ledger` path are removed. Test callers use the direct
handoff; their semantic, error, resource and cancellation assertions remain.

## Actual Regression and Source

The former missing-row lookup mock is replaced with a real finish-root test. Both an actual
candidate and an unsubmitted import reject missing phase/dimension rows and same-size wrong
semantic rows. The test checks unchanged final-progress count, owning containment, honest
remaining physical allocations and unchanged contained state after root destruction. Existing
successful and failed import tests exercise the adjacent valid catalog and direct report
handoff, including retained context with zero terminal ledger totals but live physical leases.

Source1 contains 1,409 files, frozen before runtime qualification. Its identity encoding is
sorted workspace-relative UTF-8 path + NUL + lowercase SHA-256 + LF.

| Artifact | SHA-256 |
| --- | --- |
| Complete source1 set | `0a06b93a93d6135936fa707ebb1e32a2bae7f225da3f92a9df5d6b682b7009e8` |
| `import-precommit-catalog-development-source-1.json` | `37f49f149964abdee50eda2a7bc1b7426bc43f851095d51bd24a91078ce2f94b` |
| Seven-command exact-source QA6 | `9130005ee7ba30d7b182937d8791c9a5841217e9913d9e03869b4ccab658e26b` |

The source tree is retained as `.git/baozi-review/performance-convergence/import-precommit-catalog-source-1-frozen`;
QA manifests/logs are under `target/review/convergence/`. These identify local evidence,
not packaged release artifacts. Compiler/lint failures retain their original logs: an
unnecessary activation-error variant, now-unreachable closed-enum test arms, a remaining old
`Result` expectation, and a non-Drop test request. The actual gate/error mapping uses the
existing contained-operation vocabulary; no new ordinary producer failure state is needed.

Actual source1 passes **2,422 Operation/Core/Import/Postprocess tests** with 6 existing skips,
run `a348107f-c842-4bd7-a062-e367f8d34a6e`, across 49 binaries. It also passes **105 affected
doctests**, **138 independent consumer tests** with 1 existing skip (run
`be9acace-657a-4e07-b9eb-5b6d71a46e02`), strict workspace/consumer all-target/all-feature Clippy
with `-D warnings`, root format check and exact consumer format check. All seven QA6 commands
exit zero; every complete source map matches the same frozen source1 before/after each command,
and all retained log hashes match their receipt. Independent review nevertheless identifies
one introduced panic-composition defect in the new rejection gate, described below. No
result from the earlier joint source6 is transferred to this successor.

## Rejection During an Existing Unwind

Independent source1 review finds that a rejected catalog can resume a caught payload-cleanup
panic while an unrelated outer unwind is already active. A destructor invoking the actual
finish gate would then abort instead of preserving that original panic. The normal catalog
tests and seven passing QA6 commands do not cover this composition.

The added actual host test obtains a real candidate finish through `run_body`, moves that
permission into a Drop callback, and rejects its actual nonempty catalog while the original
panic unwinds. It exercises normal/unwinding calls and all four output/proof panic masks.
On unchanged source1 production, `import-precommit-catalog-panic-red-1.log` records an actual
aborted test child and nextest exit 100, not a compiler failure. Its immutable QA maps and
the separately frozen new regression source retain this RED.

Source2 changes only the rejection gate and that regression. The gate captures the incoming
unwind state before actual host cleanup. It preserves the existing first-panic behavior for
a normal call, and discards a secondary cleanup panic during an existing unwind after both
payloads and the original graph have been resolved. It uses the existing host Drop policy
without weakening containment or adding another callback/ledger.

| Source2 artifact | SHA-256 |
| --- | --- |
| Complete 1,409-file set | `df16c0c95d2bdea216dc7d8c98f4f1ba1ecc197de918261d107edc9ad2255909` |
| `import-precommit-catalog-development-source-2.json` | `8aad14c0d831c9f6523762764e33d0ef50f41adfa5d065df3a3f786cb7525fc9` |
| Retained RED regression source | `b52b96baea4a25c9ae897524ddb2fcba34fc546414823708826c33faf1ec10e7` |
| Final source2 QA7 | `37cc2d466981faa138ce9c9933c3c73782d8d1653994cd82880d0be75eb52974` |
| Independent source1 review retaining F1 | `fddda37c10ce1707e68d49a44e94a90aa236aba0ce95e571fb2406c413534b42` |
| Independent final source2 repair review | `b0d68aa7d1af1166fa1134822679b6a359df4f1f2e9815076099f7716e62a774` |

Actual source2 passes the unchanged focused regression, run
`10379a17-d1a5-4363-bdad-9773c86af646`, across all eight combinations. Its complete affected
runtime passes **2,423 tests** with 6 existing skips, run `e461464b-831c-4be9-af2a-ee53e0624d2c`.
It also passes **105 doctests**, **138 consumer tests** with 1 existing skip (run
`f0676b56-e5e1-44b8-a584-12235467135e`), workspace/consumer strict lint and both format checks.
All nine QA7 commands exit zero with all 18 complete source maps matching source2 and every
retained log hash verified. The focused test's filtered cases are not reported as the complete
Operation suite. Independent `import-precommit-catalog-source-review-2.md` verifies the exact
two-file repair, actual unchanged RED/GREEN test, all source/QA/log associations, and closes F1
with no remaining finding or testing gap in this component. Source1's earlier passing totals
are not substituted for the successor checks, and its original finding/report remain immutable.

## Remaining Work

This component leaves the old split/client terminal authority, canonical unreachable failure
API arms, borrowed numeric backing, current-state consuming attachment with one start
admission, shared-shell-before-state promotion and retired-path deletion open. Main Rust is
still unapplied. There is no new ordinary benchmark or memory campaign; the unchanged
Windows/Linux 14-case/110-lane performance requirements and release remain HOLD.
