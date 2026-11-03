# Real Import Bridge to Exclusive Execution

This slice connects the actual retained import-session owner to the exclusive executor
qualified in the [first proof](2026-11-03-exclusive-invocation-proof.md). A registered importer
can use branded root/parent/child roles without lending an independently usable work permit.
It qualifies real registry, submission, failure and control behavior before format callers
migrate. It does not establish a performance keep or complete exclusive adoption.

## Actual Ownership and Failure Behavior

The Dormant `RetainedStoreSessionOutcomeOwner` forwards its existing operation into the same
`RegionHost`, ledger `WorkView`, physical account and weighted control-debt owner. Direct and
import execution share one numeric kernel. The import adapter retains its real vocabulary and
original error owner; the first proof's provisional `domain(0)` does not classify import errors.

Execution state records Running, KnownError, IncompleteSuccess or UnwindPending separately
from terminal cause selection. Actual registry disposition is sibling session data that remains
available after consuming activation into Pending. No numeric root borrow spans submission.
Caught numeric unwind denies both execution entries and both submission routes before mutation.
An unwind reaching the real registry catch retains importer panic/poison behavior. Incomplete
success and eligible semantic returns preserve their prescribed exit polling; the unchanged
session still performs its real post-parse poll and cause selection. Already recorded Control
causes are projected directly on denied reentry without polling again.

Normal parser errors retain only verified pieces and return their original owned error.
Ordinary returned actions and opaque reader calls complete actual returned work, including
backend Err and EOF; a caught reader panic resolves its unconsumed work. Host guards retain
numeric cleanup authority across physical callbacks and destructor/unwind paths.

Old and replacement import roles use one accounted FormatFailure materializer. It constructs
the real retained envelope/sink, performs existing Internal clone/handle work, reconciles
capacity, contains code/details/producer Drop callbacks, and transfers failure storage only on
its final seal. Materialization occurs while the failed parser frame remains active, followed
by frame resolution. No second ledger, allocation bypass or synthetic error replaces that path.
Necessary inline Runtime-versus-Owned error carriers have four method-scoped lint reasons;
there is no additional boxed allocation on an already rejected path.

## Discriminating Oracles and Preserved Failures

The real `Format`/`RegistryBuilder`/`ImportPlan`/`read_bytes` tests cover successful submission,
semantic rejection, incomplete success, caught/uncaught numeric panic, late cancellation,
registration poison, accounted format evidence, producer destructor combinations and opaque
reader return/error/EOF/panic. Literal work prefixes, physical writes, owner identity, poll
counts and final storage cleanup remain asserted. A distinct marker after successful submission
and the outer Validated phase prove that the post-submit panic case actually reached Pending.
Existing parallel-poison-after-submit coverage remains separate.

One private unit test constructs a real coordinator, account and `StoreInvocation`, then calls
the actual parent/child and compound failure service. It checks that the same child frame is
active with remaining `[1]` during materialization and inactive when that method returns.
Parent metadata is observed only before/after the child borrow. Actual quiescence, original Rc
error identity, physical `[17,29,0]`, child `(2,1,0)`, parent `(1,0,0)` and Internal `(0,0,0)`
provide independent state and numeric checks. No public test hook is introduced.

The first attempted timing oracle incorrectly interpreted row `pending_consumed` as a parser
frame's unverified suffix. Main's checkpoint-5 run passed 320/321 and exposed that error. The
entire new invalid integration test is deleted; one private frame test replaces it. Ordinary
parser channels correctly reserve no pending-consumed envelope. Production close order and
accounting were unchanged. The failed packet/log and the reviewer's explicit correction remain.

Earlier qualification also retains the original full-suite Control-reentry regression:
checkpoint 3 passed 1369/1370 because its new gate collapsed the existing typed Control cause
into Stopped. The common retained gate now reads the original coordinator cause. The baseline
test is unchanged; both entry routes additionally assert the exact cancellation, no callback
and no extra poll. Checkpoint 4 passes all 1370 operation/import tests. Invalid diagnostic-code
and wrong detection-stage poison fixture expectations were corrected without relaxing their
production validators or exact final oracles. Earlier compiler/lint failures remain available.

New compiler examples pair valid owned/separate-owner/scoped-thread use with root/layout
escape, invariant cross-owner substitution, exclusive reentry and cross-thread negatives.
Main inspected actual compiler stderr: the five new import negatives fail for borrow,
lifetime/brand or Send/Sync reasons, with no unintended missing symbol. Existing intentional
retired-symbol and child-role-gate examples are not relabeled as ownership proofs.

## Source Identity and Qualification

Qualified source: **`67b5ca609470268b124fd1915741ee81105c4a24`**.
Runtime base: `0001bd8ab5ccf0c6c1b16754a1c2016ad8a76684`. The final frozen packet contains
exactly 18 source paths and has full-patch SHA-256
`dfe1adfb61d6ff7f0f16de3ea1a622382f016df519f42dedb4ab5454155b854a`.
Actual main application is byte-identical to that packet; main formatting changes no bytes.
Its sorted `path + NUL + SHA-256 + LF` identity-set hash is
`cf4ab34fa3574e761d2f753f79105eec91806a1fe5ae00f0c1625f17ac654acb`.
The separate glTF direction probe, helpers, manifests and unrelated user notes are excluded.

Independent production review, final oracle review and actual application review close with
no actionable P1/P2. Their SHA-256 identities are respectively
`c676328b1581347e5edfcada7c3722b6ad00741654648a9451fd5fe75598e67a`,
`5ad88cfc1a69c703f666530a645f7e87e382f5927290f5dc8aef40c867a53102`
and the application receipt retained with the final evidence bundle. Main verifies the
committed Git source against all 18 qualified file identities after committing.

Windows uses Rust 1.95.0, locked/offline dependencies, one build job, the shared target and
two nextest threads. Isolated final-source tests pass 321/321 with strict all-target Clippy;
unchanged production checkpoint 4 also passes import 25 and operation 54 doctests. These
preparatory results remain separate from actual integrated main qualification below.

Actual integrated main qualification passes on that exact source:

| Check | Actual result |
| --- | --- |
| Operation/core/import/postprocess nextest | 2346/2346, six existing skips; `3e5738be-2d05-48fe-94a1-c75a4c89d472` |
| glTF/OBJ/PLY/STL nextest | 853/853, no skips; `be587aaf-8257-4bb8-b88f-624133e5eadb` |
| Facade control assurance | 9/9, no skips; `3bb327d1-be3b-446f-aed7-2f1fabb5a565` |
| Independent all-feature asset consumer | 138/138, one existing manual skip; `d85e697e-36b9-4e80-91dd-a6a3851a549f` |
| Import/operation doctests | 25 + 54 passed |
| Affected and consumer strict all-target/all-feature Clippy | Both pass with `-D warnings` |
| Affected formatting | Passed with no source change |

All eight checks retain identical 18-path before/after identities. Their aggregate receipt is
`target/review/convergence/bridge-main-qualification-1.json`, SHA-256
`af1e9ca774662fb24ec40808aa308a156c1b11e2aa4aaaefed85534298fed563`.
Individual commands/logs remain under `bridge-main-*`; `bridge-source-commit-1.json` ties the
passing precommit source to the committed Git tree. These are Windows correctness/compiler
results, not a Linux runtime campaign, ordinary performance result or repaired glTF direction
admission policy. The separately [reproduced direction-value P2](../research/2026-11-03-normal-generation-standards.md#subsequent-selected-value-audit-and-windows-reproduction)
remains open.

The same committed source subsequently passes **Linux operation/import 1371/1371**, with
no skips, nextest run `5625317f-b404-41ee-a4e0-52e8566bbbf2`. Import 25 + operation 54
doctests and strict all-target/all-feature Clippy also pass using Rust 1.95.0, locked/offline
dependencies and one build job. All 18 file identities remain identical before/after each
check. The five new import negative examples again fail for their intended borrow, lifetime,
invariant-owner or thread-trait reasons, without unintended missing symbols. Aggregate receipt
`target/review/convergence/bridge-linux-qualification-1.json` has SHA-256
`fd0f347560b011a2eadf8780e939510adcc01eee67151ecb9b00dfe006078ba8`.
This is an additional Linux bridge qualification, not the full Linux formats/consumer matrix
or an ordinary performance/memory campaign. The preceding Windows receipt remains unchanged.

## Remaining Caller Cutover

Formats, metered ordinary storage/text services, typed canonical writers and postprocess still
have real transitional callers of the common kernel. Their cluster migrations must remove the
superseded implementations and public roles at their actual last use. Binary STL decode and
one ordinary vector/string growth owner are the next design under independent review.

Lexical roots/promotion, honest administrative metrics, consuming attachment, indexed unchanged
selection and the [owned geometry payload plan](../plans/2026-11-03-1255-refactor-owned-geometry-payload-plan.md)
remain separate. No ordinary-worker speedup is attributed to this bridge. Previous extraction,
startup and structural revision costs remain open, as do the parent plan's full Windows/Linux
14-case/110-lane timing and whole-child-memory 3x gates. No release readiness follows here.
