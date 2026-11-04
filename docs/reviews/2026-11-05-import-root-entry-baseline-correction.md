# Import Root Entry: Correct the Poll Baseline

The original owned-error regression is real. The separate claim that ordinary Session
loans previously performed no entry poll is withdrawn. The corrected replacement must
preserve one entry checkpoint per successful ordinary loan. This is a correctness
checkpoint under the [lexical-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md),
not a performance keep or whole-workspace qualification. Isolated Rust remains unapplied
to main.

## Actual Qualified Call Chain

In qualified `borrowed-progress-source-2`, Import's `execution()` calls
`RetainedStoreSessionOutcomeOwner::execution()`, which forwards to
`RetainedStoreOperation::execution()`. The last method calls `services.checkpoint()`
before returning its work and services. Reading only the first two adapters misses that
real control effect.

For the bounded failing-open fixture, the original signal counts at authority lookup,
work-bound lookup and physical open are **3, 4 and 5**. The explicit Session boundary
includes its owning-loan entry checkpoint and its own poll. The separate open-attempt
transaction has another entry checkpoint; the open transaction and observed-call
admission reach the remaining prescribed checks. No checkpoint may be removed because
an intermediate adapter appears to perform only a borrow.

## Baseline and Regression Evidence

The same new public Session test was replayed against two retained sources. The corrected
test SHA-256 is `cc58713f628ceae2cd1785374601bfab1145ddd9215597bb3bd66be0a1f40662`.
Each replay preserves every original production file and uses its own build directory.

| Source | Corrected test result | Meaning |
| --- | --- | --- |
| Qualified borrowed-progress source2, set `bfd1b51233796b9e0081441da5ba2ef868b86cd1dd7891e2b89e79cb4d14fd98` | 3 passed, 0 skipped; run `c28cbfba-7a87-41cc-a1b1-0f5914d8ad1b` | Confirms the original owned cause, `3/4/5` frontiers and cancellation at check 5 after recording the open attempt |
| Full formats construction source2, set `3aaa887416d659eda11a76e722a039fa56749c54accc51b5887ad8610571c60d` | 2 passed, 1 failed; run `f328f4fd-5ba1-4c85-b58a-f6b90526558e` | Only the original `PrimaryBytes` error assertion fails; the two poll assertions already agree with the qualified source |

The owned-error failure comes from returning `Ok(Err(original_cause))` across the new
region. Successful close can then replace the original error with a generic stopped
error. The replacement root accepts the caller's actual error type and propagates a
genuine outer error through the existing guard; Session callbacks no longer wrap their
fallible transactions as successful values.

An earlier test expected `1/1/2` and cancellation at check 2. Those expectations fail
against the qualified original too. Their apparent repair-source success versus
construction-source failure cannot establish a regression or justify removing the
original entry checkpoints. The intermediate no-entry-poll repair is superseded.

The first construction replay also reused a shared-target binary from the replacement
workspace without recompilation. It is explicitly disqualified as old-source evidence.
The later replay logs show compilation from their actual retained source directories;
before/after maps and test hashes are preserved separately.

## Receipts and Remaining Work

Receipts under `target/review/convergence/` are
`import-root-entry-original-baseline-qa-2.json` and
`import-root-review-repair-red-qa-3.json`. The bounded independent correction is
`.git/baozi-review/performance-convergence/import-root-entry-poll-baseline-correction-1.md`.
Earlier reviews and failed or disqualified observations remain retained; this correction
supersedes their poll-baseline interpretation.

The current successor restores the ordinary entry checkpoint and separately preserves
the former loan's successful-return behavior after numeric cleanup. Actual entry gates,
quiescence, caught-unwind settlement, first-cause ownership and outer publication gates
remain required. Current successor qualification, connected detection/format and old
test migration, main integration, the complete Windows/Linux 14-case/110-lane comparison
and separate heap attribution remain open. Performance acceptance remains HOLD.
