# Exclusive Invocation: Numeric Owner and Lifetime Proof

This is the first implementation slice of the
[exclusive invocation plan](../plans/2026-11-03-1120-refactor-exclusive-invocation-plan.md),
within the [Assimp convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
It proves the replacement execution seam against the actual operation owner. Real importer,
authoring and postprocess caller migration, lexical storage roots, publication changes and
performance qualification remain subsequent work.

## One Numeric Implementation

Work arithmetic now lives in one kernel over an immutable owner catalog and mutable numeric
rows/header. Existing adapters and the new exclusive runner use that implementation. Binding
resolves catalog positions, duplicate representatives and completion order once; successful
layouts retain no wide diagnostic-key array. Binding errors project supplied keys, while later
transaction errors project registered keys. Fixed arrays and dynamic rows run the same literal
duplicate/Internal/overflow traces.

The host lends a branded mutable invocation and retains action, parser and region guards.
Ordinary physical callbacks receive no execution authority. A normal semantic return, including
`Err`, completes its admitted action before the result is inspected. Parser pieces complete
only verified successful prefixes. Admission, partial completion, failure resolution and owned
result/capture destruction retain their required order. Caught leaf panic prevents further work;
the host resolves the outstanding suffix without inventing consumption.

Actual nested parser tests combine allocation, capacity reconciliation and Internal share work.
The allocated owner may escape the execution region and remains real live scratch until it is
destroyed. Allocation/final-control rejection destroys actual payload before releasing its
admission. Independently releasable storage keeps its genuine shared accounting. No numeric
lock or dynamic borrow guard spans a callback, allocator, observer or destructor.

The qualified weighted-action frontier remains at most 256 debt units. Numeric action amounts
and control costs stay distinct; invalid costs deny the callback, and a real checkpoint occurs
before the next action would exceed the bound. This slice changes no normal estimator,
contribution order, precision, reader semantics or benchmark task.

## Actual Source and Review

Application base is `0bc45ac7a712410b42b1e8f925a820a9713acb37`. The frozen candidate patch is
SHA-256 `4f6dc3ed501a568c2cd8ae069aadd2a2a5b774da1578a2cf8330a24a963631ca`; its operation overlay
was prepared from `57c33b2447a296c28f5f5817a36debd248bd4280`. Only its 25 operation paths were
applied, preserving the later qualified reader, normal intent and glTF profile changes.

Actual compiler/lint corrections locate the nested integration test, add the existing terminal
key bounds to the new split accessor, remove two unused forwarding imports, name an unchanged
private tuple, and make test error/import/header types explicit. Formatting changes no action
or assertion. An independent review compares all 25 actual files with the frozen candidate and
finds no actionable P1/P2 in this delta. It also checks 511 protected core/postprocess/glTF/
consumer paths against the application base; only three pre-existing line-ending differences
exist outside the overlay.

The final actual 25-file identity-set SHA-256 is
`4c1150217d66de89e2db1a65ae6a45e158bbac9570544beee66f8aa16e872b9d`. The set is hashed from sorted
UTF-8 `path + NUL + lowercase raw SHA-256 + LF`, with forward-slash paths. Per-file identities,
before/after checks, full commands and logs are retained under `target/review/convergence/`.
The source review is retained as
`.git/baozi-review/performance-convergence/exclusive-proof-actual-source-review.md`.

## Qualification

Windows qualification uses Rust 1.95.0, locked/offline dependencies, one Cargo job, the shared
target and nextest. The following checks use the same actual 25-file source set; filtered glTF
cases are distinguished from existing manual skips.

| Check | Result |
| --- | --- |
| Operation all-feature nextest | 905/905, no skips; `7bdcb5a3-2469-4497-98cc-34d35f3d9b61` |
| Core/import/postprocess all-feature nextest | 1420/1420, six existing skips; `dbfa7b7f-7a8d-4268-9b2a-4ab9cf0fc8c3` |
| Qualified glTF source-profile comparison tests | 9/9, 53 other unit tests filtered; `d0b40c31-446b-4ab7-a1c9-941cd5185367` |
| Facade control assurance | 9/9, no skips; `bc7f2805-ce34-4c0f-818e-c1d390348206` |
| Operation doctests | 54/54, including compiler diagnostics inspected below |
| Import doctests | 17/17 |
| Core/postprocess doctests | 6/6 and 4/4 |
| Independent asset consumer, all features | 138/138, one existing manual skip; `76c9f62d-8fbd-45f2-a420-081f2edd6e5c` |
| Strict affected and consumer Clippy | Passed, all targets/features with `-D warnings` |
| Affected formatting | Passed |

Nine new negative examples use valid replacement APIs and fail for the intended reason:
action reentry/parent overlap (`E0499`), context/layout/child escape (lifetimes), cross-owner
layout/context substitution (`E0521`), scoped-thread use of local authority (`E0277`), and a
child attempting the parent-only child operation (`E0599`). Five neighboring positive examples
compile, including owned result escape and thread transfer. The compiler stderr is inspected;
missing imports or retired symbols do not count as ownership evidence. Full observations are in
`exclusive-proof-compiler-contracts.json` and `exclusive-proof-docs-1.log`.

The new runtime tests require literal attempted/consumed/pending counters, original causes,
immediate cleanup and real payload Drop observations. They cover normal semantic/parser errors,
partial/unapplied completion failure, zero/exact/denied work, forgotten legacy shared/mutable
borrows, weighted cancellation, invalid costs, caught/unhandled/capture/result/idle unwind,
policy tightening and retained storage. They do not rely solely on equality between wrappers.

The initial formatting check and Clippy attempts 1--4 expose integration problems; their failed
receipts remain alongside the final passing `exclusive-proof-fmt-6` and
`exclusive-proof-clippy-5`. Later success does not erase those failures.

## Remaining Cutover and Performance Gate

The current real callers still use transitional adapters to the common kernel. Their removal
is required by the plan; this slice is neither full exclusive adoption nor a measured speedup.
No ordinary worker or profiler result is attributed to this source yet.

Before importer adoption, separate the execution state for a known semantic error from its
terminal cause classification. The first proof's provisional `domain(0)` is not the format's
owned cause and must not suppress its required error-exit poll. Ordinary actions, verified
parser pieces, backend Err/EOF, caught extension panic and central visitors have different
completion rules. Actual caller migration must preserve those distinctions and source/failure
ownership before deleting the old permits and whole-ledger aliases.

Independent phase-2 review identifies two concrete design revisions before that migration:
incomplete parser success and importer-boundary unwind must resolve prefixes and perform the
owner's prescribed exit poll before selecting their contract/extension cause; accounted format
failure construction inside parser pieces needs an exclusive host service for its real Internal
work, retained evidence and producer callbacks. A cold error view alone cannot provide that
service. These are pending caller-adaptation requirements, not failures of the scoped proof.

The owned geometry payload plan remains a separate change: Count/Build/Compare, existing
numeric predicates, physical accounting and selected-payload correspondence are unchanged here.
The parent plan's Windows/Linux 14-case/110-lane, all-endpoint/pipeline and whole-child-memory
3x gates remain open. No publication, push/PR or native target promotion is implied.
