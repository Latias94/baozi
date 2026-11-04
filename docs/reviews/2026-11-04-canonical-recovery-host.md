# Canonical Recovery Causes in the Lexical Host

This continues the [lexical invocation plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md)
after the [output/proof host checkpoint](2026-11-04-canonical-output-proof-host.md).
The implementation remains in the isolated compiler workspace. Main Rust, benchmark tasks,
numerical algorithms and the 14-case/110-lane Windows/Linux acceptance gates are unchanged.
There is no new ordinary performance result or release qualification.

## Central Pending Fallback

The host owns the actual `Option<StoreRuntimeError>` for pending central fallback. Its opaque
cause permission travels with the proof slot and must be vacant and belong to the same root
before a callback runs. Both evidence-build failure and central-validation fallback install
the original selected error. `PendingMinimalFallback` holds a short `PayloadLoan` with its
existing storage permission. Forgetting that wrapper leaves the cause in the host.

Real tests cover allocation failure with ordinary and retained scratch, both preparation and
forgotten pending permissions, and a distinct forgotten central-validation panic fallback.
Preparation still waits for caller cleanup. Host cleanup clears the cause before disposing
canonical values and their physical storage. The candidate/frozen disposal orders and original
body-unwind panic remain intact.

| Frozen central-fallback source | SHA-256 |
| --- | --- |
| Source1, complete 1,405-file set | `67b7112dd500d1ae77e09bbfc33ede7cac255771ae99de41f9d609849d957af2` |
| Final source2, complete 1,405-file set | `dda4e08637ab8684f15b519d0754670c9d23091620127c2d0acddde2a31ba300` |
| Final source2 manifest | `4a310633b8672eb398cbf9a281028a4746d0d05449f2f2cc3b8e2e47eccaac70` |
| Final source2 QA receipt | `b619482c2a91d1ae0503c063682fc923bae1aaa48b51ecb087e82e9e96d07cfb` |

Source1 changes five operation files from the qualified output/proof source. Final source2
changes only its concrete cause cleanup in `host.rs`: a runtime error with no destructor is
cleared directly, removing an unnecessary panic-capture path and its strict Clippy failure.
The old failure and actual source1 runtime remain separate evidence.

| Actual check | Source | Result |
| --- | --- | --- |
| Complete affected operation/import/postprocess nextest | Source1 | **1,696 passed**, 2 existing skips; run `856a5315-cdee-4e34-adbc-68b4372d559b`. |
| Affected operation/import/postprocess strict Clippy | Source2 | PASS, all targets/features, `-D warnings`. |
| Focused operation host/fallback nextest | Source2 | **37 passed**; run `1e85ae6a-3dde-4672-8e7b-5b296a241b96`. Other tests were filtered. |
| Operation doctests | Source2 | **58 passed**, 7 merged and 51 separate. |
| Operation `cargo fmt --check` | Source2 | PASS. |

All 1,405 source hashes match before and after each corresponding QA receipt. One orchestration
stdout encoding error occurred after the successful source2 lint and runtime child processes;
the continuation verified their logs and completed docs/fmt without source mutation. It did
not turn the source1 1,696-test result into a source2 full-runtime claim. Independent frozen1
and frozen2 source/evidence review reports no introduced findings in this bounded change.

## Typed Recovery Successor

The actual successor adds distinct activation, validation and generation error slots to the
same host. Named invocation parts forward opaque recovery permissions through the actual core,
import and postprocess consumers. Never-activated and unrooted/rooted/validated/rejected failure
states borrow their original causes; no error type is erased or cloned to repair ownership.
The old unused product-graph fallback preparation helper is deleted.

Frozen typed source2 passes all 3,431 workspace tests (6 existing skips), 102 doctests and 138
standalone consumer tests (1 existing manual skip), plus workspace/consumer strict Clippy and
format checks. All 1,405 source hashes match before and after its complete QA sequence. These
results belong to source set `30b82122790bdcd33d0658af28daa6b7e547ed8f21ee5785066b13fc08ff574d`,
not to any subsequent repair.

Independent caller review nevertheless finds an introduced P2, F1: the two core generation drivers
accept an independent typed rejection slot but check its invocation only when selecting rejection.
The successful path can reserve a generation and invoke preparation with a foreign host's slot;
exhaustion detects that mismatch only after calling the generation source. Two actual regressions
fail at runtime on source2 production: foreign success is accepted, and the exhausted source is
called once instead of zero times. The source2 test pass does not qualify that implementation.

## Generation Admission and Callback Cleanup

Source3 introduces operation-owned same-root/vacancy admission before either core driver can
inspect parser disposition, call its generation source or invoke a result callback. Matching
admission returns the original permit and inputs. Denial disposes inputs while canonical values
and their physical storage remain live, then contains that same physical owner. Four actual
node/edit regression tests pass, including real Scene edit success/exhaustion and one panicking
source capture. Independent caller review closes F1 for this exact source3.

Source3 also passes all 3,435 workspace tests (6 existing skips), 102 doctests and 138 standalone
consumer tests (1 existing manual skip), workspace/consumer strict Clippy and both formatting
checks. All 1,405 inputs match before and after its eight-command sequence. Its complete source
set is `054492266351e62b79c4d741b50b9e8126e4baa31190b5eb30c02a37fcbd7796`; QA receipt SHA-256 is
`624e0ecef60173e53e4ff31ed099e9d978236062352d5e9095e988f1807fd7a1`.

A separate operation review then finds F2: source3 drops the three independently owned callbacks
as one tuple under a single unwind catcher. If two callback captures panic in Drop, tuple cleanup
starts the second Drop during the first unwind and aborts before the catcher can return. A new
actual regression reproduces that test-process abort with source3 production unchanged. Nextest
returns exit 100; the stack identifies the tuple Drop and admission method. The RED log SHA-256 is
`2e457bfba426aafc61cd7107a36b41f7616f834b2f6d976c6e29561a4bb17b72`.
Consequently source3 is also withheld despite its complete ordinary QA pass.

The source4 repair accepts the known three input types as a tuple, destructures them on denial,
and calls the existing controlled Drop helper separately for generation, rejection and preparation
callbacks. All three cleanup attempts finish before canonical output/proof/store containment.
Valid admission still returns the original permit and tuple without invoking them. No callback
disposer, provider framework, erased error, stronger lifetime bound or new lint allowance is added.
This does not extend recovery to aborting runtimes or nested double panics inside an individual
user-controlled destructor.

The enhanced real node fixture gives all three callbacks separate capture sentinels and checks
that each is disposed while the account is unpoisoned and canonical allocations remain live.
The new regression covers every two-capture panic combination and all three together, with
successful/exhausted generation sources and accepted/rejected parsing. All source/result call
counters remain zero on foreign admission. Matching nonpanicking cases retain their original
success/rejection behavior. No old regression is disabled.

| Frozen typed source4 | SHA-256 |
| --- | --- |
| Complete 1,405-file source set | `9f604363c58083d9500780b5fb746358d901989118a4d96627b1f668019b4ee1` |
| Source manifest | `f91f3c2c052c5b197c260b02eecfce7708ebf1a5ebd4d19079910d323c0d5581` |
| Complete QA receipt | `3fa122419fcf78c8d32af59c0ef8d1cd251a4377ff5fa0eb0bfa2926c0804d09` |
| Independent F1 caller closure, source3 | `b255190f294dfe998a38fd94dd71457dd13fb32aa64ade7cee70251ae1692182` |
| Independent F2 repair and final source4 QA review | `da9e26c80b195643eb684571487da0d3a9605daca623ce2bb2d4fee649a76529` |

Source4 differs from source3 in exactly the operation admission method and the node regression
fixture. Its aggregate source identifier uses the explicitly recorded compact sorted JSON encoding;
the complete per-file maps remain authoritative across the differing predecessor encoding.

| Actual source4 check | Result |
| --- | --- |
| Focused generation regressions | **5 passed**; run `04c890ae-6b9e-4ee5-8597-23cc37073d59`. Other core tests were filtered. |
| Complete workspace nextest, all features | **3,436 passed**, 6 existing skips, 151 binaries; run `470d4292-d49c-4b9c-b5cf-31bc58e8f6c5`. |
| Workspace doctests, all features | **102 passed**, 0 failed or ignored; includes the existing negative compiler contracts. |
| Standalone asset consumer nextest, all features | **138 passed**, 1 existing manual skip; run `d4043b28-3631-48f8-9a78-2fe3010ce9e6`. |
| Workspace and standalone consumer strict Clippy | PASS, all targets/features, `-D warnings`. |
| Workspace and standalone consumer `cargo fmt --check` | PASS. |

All eight commands return exit zero. All 1,405 source hashes match the source4 manifest before
and after the complete sequence, and every recorded log hash matches its actual bytes. The
earlier source2/source3 passes are not transferred to source4. The independent source review
confirms the bounded F2 repair without introduced P1/P2 and independently associates the final
source4 manifest, complete before/after maps, command exits and actual log hashes. F2 is closed;
the typed recovery source4 checkpoint is qualified within this receipt's ownership boundary.
The unchanged source3 caller admission and its F1 closure remain in that final source. Neither
this qualification nor either review closes the remaining connected work below.

## Remaining Connected Work

Successful public preparation still detaches owned causes with terminal outcomes. Replacing
that client terminal authority requires the concrete outer driver to retain actual stage
facts, dispose callback captures and caller scratch while armed, then perform terminal control,
freeze and joint publication. No root-borrowing wrapper may be stored back into its own host.
Borrowed numeric backing, one current-state attachment/start admission, storage promotion,
retired-path deletion and ordinary performance qualification remain required.
