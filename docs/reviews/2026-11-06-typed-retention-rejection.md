# Typed Retention: Owned Rejection and Bounded Completion

This slice prepares the error and control Interface required by the [finite geometry payload plan](../plans/2026-11-05-2330-refactor-finite-geometry-payload-plan.md). Typed retention can now transport destructor-free domain rejection facts separately from runtime failure, close the rejected lane, explicitly settle failed work and observe normal completion within the 256-action validation bound. Geometry float pools and all existing finite scans remain unchanged. This is a source-qualified structural prerequisite, not a performance keep or release result.

## Error transport and one retention owner

The hidden integration Interface introduces `CanonicalStorageBuildError<R: Copy>` with `Runtime(StoreRuntimeError)` and `Rejected(R)`. Core will interpret future geometry coordinate facts; operation contains no float policy. Current runtime-only callers instantiate `Infallible` and recover runtime errors exhaustively. Preparation values and successful output do not acquire Copy bounds and retain their existing destruction rules. The preparation callback remains runtime-only and continues supporting edited-profile child sharing.

The two entry forms delegate to one canonical retention implementation. All 24 actual Core factory calls are migrated, together with Operation's private and scoped-plan callers. `map_storage_build` only wraps the existing typed-build runtime mapping. It adds no action, admission, scan, allocator or counter. No parallel legacy factory, new dependency, unsafe conversion, arbitrary-error destructor framework or proof/prefix protocol is added.

A clean domain rejection sets one lane-local pending bit before returning its facts. Cleanup and terminal `lane.reject` remain available. Any later admitting action or success completion records an existing contract fault, so ignoring Rejected cannot produce an alternative valid root. Correct immediate rejection does not fabricate an internal invariant error for malformed input.

## Failed work and physical ownership

Each normally returned physical action still completes one work unit. Rejected semantic facts can therefore accompany completed work without an initialized output cell. A panic does not complete its interrupted action; earlier completed units remain consumed.

Sticky action errors are recorded before callback runtime errors and before cleanup. Ordinary failed finish explicitly resolves its unexecuted suffix even after an earlier fault; Drop is fallback containment. Missing work/string checks apply only to apparent success without a prior fault. Later resolution errors preserve the first observed runtime and update existing history. A Ledger cause is not assumed to set contract severity automatically; actual ledger integrity/quiescence and separately observed contract faults still enforce containment.

Preparation and build run under the existing physical reservation. Partial values are destroyed with their admission live. A nonclean physical release retains attribution fail closed and records the existing ownership/invariant fault; it cannot become a clean domain-only rejection or refund unverified backing. The failed paths bind no shared output. Panic preserves the existing unresolved-ownership containment and completed prefix.

## Bounded control completion

Typed construction reuses its existing action counter with interval 256, replacing 4096 for the stronger validation contract needed by future fused writes. The first action is checked; another checkpoint precedes action 257. Normal success and domain rejection explicitly resolve/drop the work frame before a final control observation. Short or exact-length builds cannot avoid that observation merely because no next action occurs. Runtime, resolution-failure and panic paths add no normal final observer callback. This slice adds no separate control/progress counter.

## Actual qualification

All Cargo execution is serialized with one build job, one nextest thread and existing target directories. Source inputs are recorded before and after each actual run.

| Run | Actual result | Scope |
| --- | --- | --- |
| Focused first proof | 10 typed-storage-related tests pass; Core all-target/all-feature check passes | 908 stable Rust/TOML inputs; filtered test selection, not a whole-suite pass |
| Full integration | 3,595 workspace tests pass, 6 existing skips; 188 doctests; 138 external consumer tests pass, 1 existing skip | Both strict all-target/all-feature Clippy checks and both formats pass; 908 inputs stable |
| Fault-test follow-up | 1,080 Operation tests pass, no skips; 128 doctests | Operation strict all-target/all-feature Clippy and workspace format pass; 908 final inputs stable |

The full integration completes before adding the private fault tests. The follow-up adds one `#[cfg(test)]` module and fifteen narrow visibility changes inside an existing test harness. Every other 906 recorded input remains byte-identical. The original typed-storage file prefix matches its full-QA hash; removing only those known harness visibility additions in memory and formatting with installed Rustfmt reconstructs the prior activation file hash. This is bounded test-delta association, not a custom Rust analyzer. No source is rolled back or written by that check.

Thus the earlier full integration qualifies the unchanged runtime, while the Operation follow-up actually executes the new private cases. A whole-workspace run including those four new cases is not claimed. The original receipts and all source maps remain retained.

## Independent review and meaningful fault coverage

Initial independent combined review finds no confirmed implementation defect but requests direct coverage of explicit resolution/first-cause collisions and nonclean owned-release branches. Four private tests add five scenarios through the actual factory and existing invocation harness:

- Admit two units, complete one and corrupt the private remaining suffix so actual resolve returns `FinalSnapshotShapeMismatch`. Exercise both Rejected and an earlier runtime deadline; check the observed resolution failure, retained runtime first cause, actual integrity containment, unchanged completed prefix and no further observer callback.
- Cancel after preparing a non-Copy value whose destructor fails. Check that the build does not execute, destruction sees live admission, the control cause remains first, ownership failure is recorded and physical/logical attribution is retained fail closed.
- Return an output whose destructor fails after final cancellation is observed. Check completed work, no shared output, live-admission destruction, preserved control cause and containment without extra cleanup callbacks.
- Panic after a completed builder action. Check its completed prefix and absence of post-panic control callbacks.

Existing meaningful tests retain whole-envelope denial, extra-action suppression, incomplete success/string work, forgotten borrow, panic/prepared-child cleanup and prepared sharing/order. Domain-prefix tests cover zero/partial/full completion and final cancellation collision. Ignored-Rejected tests attempt both retry and successful completion of an already valid root. Cancellation covers 255/256/257 actions. No enum-layout, Copy-bound mirror or generic proof test is added.

Independent final fault-test and receipt review closes both P2 gaps with no new implementation or test-delta finding. It checks the actual tests, assertions, unchanged-runtime association, completed gates and this report's qualification boundaries. The final private review SHA-256 is `1a91cd69e2ae4e7b49d62218b3b89db2a5a96d2246d946018283877c09cbdb7c`.

## Next construction work and limits

The next cut must close the initial, selected and converted float writers, preserve full-source projected-value rejection and stronger normal/tangent/conversion rules, then atomically remove ordinary source-finite and retained-finite visits with every surviving work formula. Central actual-output layout, range, sparse-index, topology and reference validation remains. This slice does not implement that finite invariant or delete those scans.

No ordinary benchmark or heap observation is collected for this transport-only prerequisite, and no performance improvement is inferred. The previous text cleanup's mixed native results, endpoint regressions, tails, GNU missing confirmation and release gaps remain open. Every final Windows/GNU endpoint, pipeline and comparable process-memory target is unchanged. No push, PR, publication or yanking occurs.

## Local evidence

- `target/review/convergence/finite-retention-focused-1.json`, `finite-retention-integration-qa-1.json`, `finite-retention-fault-qa-1.json` and their original logs.
- `target/review/convergence/finite-retention-fault-delta-source-1.json`.
- `.git/baozi-review/performance-convergence/finite-retention-source-review-1.md`, `finite-payload-plan-review-1.md` and implementation/fault-test lane records.

The scoped source is committed as `5e3aff55b0dd9ad1113740fac2346ea0a79b5836` after final review, exact eleven-path staging and current-source comparison with the passing follow-up receipt. The implementation commit includes the hidden integration's breaking error/cadence contract. Local-main integration is recorded in the execution state after its actual fast-forward; no remote action is inferred. The active convergence goal remains unfinished.
