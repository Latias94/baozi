# Invocation-bound Observed Work Sequence

R3 of the [Assimp convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md)
reuses one producer envelope across a nonempty bounded postprocess execution region. This
receipt qualifies the implementation and correctness. Ordinary before/after measurement is
still required before retaining it as a performance improvement.

## Replacement and Ownership

The first real action admits one unit; subsequent actions replenish exactly one only after the
preceding action has completed. Each normally returned action, including a semantic error,
completes immediately through the original numeric authority. There is no pre-admitted future
action, buffered completion, shadow ledger or relaxed work/storage allowance. Initial admission
and singleton replenishment share the same extracted arithmetic/check/commit implementation.
Current policy, mode, protected grant, duplicate ordering and pending headroom remain live.

`ActiveRuntime` independently tracks an in-flight action and ownership of the idle envelope.
Zero remaining is not closure. Successful region return explicitly closes the envelope and
checks quiescence before the final checkpoint. A normally returned known error explicitly
resolves its idle ownership without another poll and preserves its cause. An unmatched Begin
or actual unwind retains abandonment through the existing Drop path. Completion failure
immediately resolves the owner while preserving any already applied physical prefix and the
original failure; callers may check quiescence inside that failure boundary.

Actual Internal retained-share work can occur between leaves on the same authority; it does
not reopen the Producer lane or overwrite another owner's pending suffix. No numeric guard
or storage lock spans an action, allocation, control callback or capture/result destruction.
The old per-leaf open/finish path is removed from this runtime. The existing raw multi-unit and
parser protocols remain for their actual distinct consumers. This experiment changes no normal
math, precision, work vocabulary, selected fields, parser layout or comparison clock.

## Review and Qualification

Independent source review initially requested two missing destructor-driven unwind probes.
The added tests exercise an actual owned FnOnce capture dropping at callback return and a
completed leaf's returned value dropping while the region remains idle. Both require one
literal physical write, one drop, the exact attempted/consumed prefix, ProductGraph failure
evidence and no cleanup poll. They are not replaced by an explicit body-panic test.
The reviewer closed that finding and rechecked all seven applied/formatted files.

Sixteen new tests cover refill ownership, rejection/overflow, attempted-versus-consumed
precedence, live policy and role revocation, competition with real Internal pending work,
scratch admission/share/release, idle success/error/unwind and the destructor probes.
Existing failure, no-poll, immediate completion-error cleanup, zero/full-forget and
cancellation-bound assertions remain. Cancellation cases include a final short region and
0/1/31/32/33/255/256/257 completed leaves.

The first strict Clippy run rejected a large-error closure in explicit region cleanup.
Replacing `closed.and_then(...)` with an equivalent `match` retains inline errors and the
same evaluation order, with no new allocation or lint exemption. Source review accepted that
exact adjustment. Its failed log is retained. The final source passed:

| Check | Result |
| --- | --- |
| Operation/core/import/postprocess nextest | 2,282/2,282; six existing skips; `0939b408-d82a-4877-a38b-19269ffe6920` |
| Affected compiler contracts | Import 17; operation one positive plus 39 compile-fail; passed |
| Strict affected all-target/all-feature Clippy | Passed with `-D warnings` |
| Formatting | Passed |
| Real-format facade control assurance | 9/9; `2278b68d-6602-4d2d-8df9-ff2e5c79c9a7` |
| Asset consumer, all features | 120/120; one existing skip; `7226f75f-4b0c-422d-8593-a0d908dd8ed9` |
| Independent source review | No open P1/P2 at the final inspected hashes |
| Ordinary performance | Pending; no measured R3 gain or convergence claimed |

Commands use Rust 1.95.0, locked/offline inputs, one build job and the shared target. Compiler
contracts ran before the closure-to-match correction; that correction changes no exposed
signature, lifetime or capability. Integrated tests and strict lint were rerun afterward.
Exact source hashes and command logs are `target/review/convergence/r3-*.json` and matching logs.
The preformat v2 patch, implementation handoff and independent source review remain under
`.git/baozi-review/performance-convergence/`.

The subsequently designed bounded normal-action vocabulary and exclusive invocation/reader
changes are not included. Source-level removal of envelope churn is an opportunity, not
evidence of native gain. Ordinary immutable workers must preserve the existing task and all
samples; the complete Windows/Linux release-convergence gates remain open.
