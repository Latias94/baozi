# Success-Only Canonical Joint Preparation

This component follows the [Import catalog admission](2026-11-04-import-precommit-catalog.md)
in the isolated [lexical-host refactor](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
It removes terminal outcomes that the frozen canonical joint cannot produce. Actual ordinary
rich/minimal failure APIs and their ownership regressions remain separate.

## Actual Contract and Deletions

`CanonicalJointCommit::prepare` returns `CanonicalJointSuccess`, containing the actual output,
proof and prepared store, or owning containment. Rejection and final control interruption
are decided before a ready freeze permit. After that boundary, the frozen classifier rejects
invalid roots, exact commitment shape, binding faults or a late terminal cause by containing
the original graph; it cannot publish rich failure or minimal fallback.

The raw success finish checks the selected unqualified state before transferring physical
ownership or finishing attribution. A non-success state is contained before qualification,
freeze or publication. Both the ordinary dispatcher and the exact canonical path use the
same success implementation. The general ordinary dispatcher still finishes genuine rich
and minimal classifications, preserving their original control cause.

The concrete contribution now directly classifies, finishes success and performs the exact
account-bound commit. Its former `StorePreFreezeClassification` wrapper, redundant result
classification and `finish_classified_joint` helper are deleted. `PayloadPair` keeps the
actual output/proof loans alive through preparation and independently guards their destructors
on failure; no output/proof copy or new storage/account owner is introduced.

Three Core publication consumers destructure the exact success and construct their tokens
only afterward. Their unreachable joint-failure arms are removed. Direct Scene edit's rich
publication variant, unused optional control fields, Postprocess rich publication helper and
its sole rich failure-owner variant are deleted. Genuine generation exhaustion retains its
minimal fallback, and ordinary activation, rejection, destination and final-control failure
paths remain usable.

## Regression Scope

The existing borrowed non-Copy success, dropped/forgotten joint, preparation-error and
destructor-panic matrices remain. Ordinary shared-storage refund/release-fault tests now
explicitly invoke their real raw failure classifier, dispose actual payloads, recheck the
original account and finish that ordinary owner. They no longer imply canonical joint-failure
reachability or require a production helper used only by test scaffolding.

The raw classification regression additionally sends real rich and minimal failures to the
success-only finish. It checks owning containment, an unqualified original ledger, zero
published destinations and unchanged unresolved physical charges. Adjacent ordinary finish
cases still publish their actual evidence/fallback and original control cause.

Independent source1 review finds no introduced correctness defect, but identifies missing
direct coverage for cancellation after `Ready`. Source2 adds a real outer-body test: request
the candidate, dispose the retained body, resume the exact stage, then issue `Ready`. The test
records cancellation polls, progress calls and both extension drops; only then does it cancel
the retained external token. Exact joint preparation must still return success, commit the
correct destinations and perform no further extension call or destructor. Its adjacent
pre-permit cancellation test continues to require an interruption.

Source2 changes only this new test and two ordinary-failure test names relative to source1;
all production source bytes are identical. The source-qualified regression passes one test,
run `4de734df-849a-481b-a333-dacd3cc69934`, with 962 filtered tests. That focused run is not
the complete Operation suite.

## Source and Verification

Frozen source1 has complete set
`2bdb656d9dd8b98dc065727a0cedaa9408b21cab06ffb1a67c50c719a39b5ef4`.
Its affected runtime passes 2,423 tests with 6 existing skips, run
`6d98392e-1445-4bd2-9d02-adb064fb115f`; 105 doctests, 138 independent consumer tests
with 1 existing skip, workspace/consumer strict lint and root/consumer format checks also
pass. These are retained predecessor results, not substituted for source2 qualification.

Source2's complete 1,409-file set is
`64e8d825e2ed6a991bbb8f49cbfb12dbf714e350529a38ebfde3312c8cb7b4c5`.
Both source manifests, exact before/after maps and logs are under `target/review/convergence/`;
full frozen trees and independent reviews are retained under
`.git/baozi-review/performance-convergence/`. Source-set encoding is sorted workspace-relative
UTF-8 path + NUL + lowercase SHA-256 + LF. Source2's focused
regression and complete affected runtime pass: **2,424 tests**, 6 existing skips, run
`cf1c7b72-5e4f-485d-9e78-d1add6d47885`, across 49 binaries. Its **105 doctests** and
**138 independent consumer tests**, 1 existing skip, run
`b37682c7-0486-4559-91f9-a658d3f2d642`, also pass, together with workspace/consumer
all-target/all-feature strict Clippy and root/consumer format checks.

| Artifact | SHA-256 |
| --- | --- |
| Source2 manifest | `5b7525ad70f55a5664bf494753d0e82fcdc5b27504750b4c60cb73d0bff5c28f` |
| Source2 QA3, formatting/focused/runtime/workspace lint | `bf560585c69f5fe077c8daef8f41419a319398866385692e0bd94b9225ecf05b` |
| Source2 QA4, docs/consumer/lint/format | `0cc1f71da22f2ee81624cb358720cdc206fbfed921630608d697135f408101d3` |
| Independent source1 review | `f28920392d4bc3dc31adf5db9d399c986619c1a06e0b7b72637eb0026d8ade93` |
| Independent source2 gap-closure review | `be78bc14a5224caabb43e312ef5539654594c9d6ee5b627bb53f793e300d1395` |
| Independent final evidence association | `92698887bab5ed13d3b43fef136746352b224356b803b12f2e85f6675ef1c610` |

QA3's initial formatter records its actual preformatted input and final output; it precedes
the source2 freeze. The following eight verification commands preserve all 16 complete
before/after maps against source2, and all nine retained command logs match their recorded
hashes. The current live compiler workspace also matches all 1,409 frozen file hashes.
Independent source2 review closes the after-Ready coverage gap with no new finding or
remaining test scenario in that delta. Independent final evidence review checks all frozen
files, source/QA maps, recorded commands, exits, actual log hashes, counts and immutable review
associations, with no finding or remaining evidence gap. These results are not a
whole-workspace-runtime or performance qualification.

## Remaining Work

The old `split` and body-accessible finalization, borrowed numeric backing, single-start
current-state attachment, shared-shell-before-state promotion and connected obsolete-owner
deletion remain open. The main Rust tree has not received this isolated refactor. No new
ordinary timing or memory campaign belongs to this component; Windows/Linux 14-case/110-lane
performance requirements and release remain HOLD.
