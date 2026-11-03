# Bounded Normal Actions

This is the next independently qualified slice of the
[Assimp convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md), after
[bound attribute readers](2026-11-03-bound-attribute-readers.md). It changes the accounting
vocabulary around normal generation. It does not change the normal estimator, source identity,
precision, mathematical evaluation order or the frozen comparison task.

## One Authority and Two Distinct Units

Normal generation now admits one bounded semantic invocation at a time: logical-position
validation, face evaluation, one smoothing-candidate visit, one output element, or a Copy-record
scan of at most 32 records. Face reading, canonicalization, component scaling and the cross
product run inside the admitted face action. A rejected admission cannot enter that body.
A normally returned semantic error still completes the invocation; an unwind does not.
The existing observed envelope is reused, and each actual next action must replenish one unit
before execution. There is no precharged batch completion or second numeric ledger.

Work actions and control cost have separate purposes. Actions determine attempted/consumed
work. A conservative bound for each action advances invocation-owned control debt after actual
completion. The scheduler polls before the next action would exceed 256 credits. Copy scans
reserve 32 credits even for a shorter tail; the worst sparse F64 face requires 118. Sparse
lookup retains a literal 33-iteration bound. Allocation, entry, region completion and the
existing semantic checkpoints still perform full control checks. Grouping compound work
therefore does not silently lengthen cancellation to 256 compound faces.

The adapter retains the admitted action's cost until completion. Numeric or cadence completion
failure immediately resolves its envelope while retaining the first cause. Known idle errors
close the envelope without a cleanup poll; pending actions and unwind keep abandonment
containment. Scratch capacity admission, actual-capacity adjustment and storage release order
are unchanged. The operation/import forwarding callers explicitly use cost one; the retired
no-argument control interface has no compatibility alias.

## Geometry and Public Contract

The same pure face helpers retain ascending validation of every logical Position, common-scale
selection, original-corner checks, canonicalization parity, cross-product expressions and
compensated accumulation order. Cache publication, wide versus narrow validation, group
lookahead and write-before-output-validation order remain. Smooth still means area weighting
over shared source identities and Face smoothing groups. No coordinate weld, unit-face
weighting, reassociation, fallback normal, precision reduction or algorithm retry is added.

`PostProcessBudgets::max_work_actions` replaces `work`. Resource evidence exposes
`attempted_work_actions()`, `consumed_work_actions()` and
`work_action_contract() == "bounded-normal-actions/v2"`; the fixed runtime catalog becomes
`baozi-postproc-2`. Delegated selection, core construction, real retained-share and protected
terminal work retain their declared units on the same authority. The default allowance remains
100,000,000 actions and the byte allowance is unchanged. Historical getter counts have no
numerical conversion to this vocabulary. Reconstruction work evidence has its separate
existing meaning.

The real asset CLI uses `--postprocess-actions`, and JSON uses `postprocess_work_actions` with
the adjacent `postprocess_work_action_contract`. The scene consumer and lifecycle fuzz target
are migrated at their actual budget literals. Old flags and accessors are removed. Preparation
still uses one budget transaction; conversion and application buffers retain their documented
separate limits. The comparison adapter, clocks, corpus, selected output, normal policy and
allowances are unchanged.

The [normal standards research](../research/2026-11-03-normal-generation-standards.md)
separately identifies the real consumer's format-policy correction. Neither this action
vocabulary nor a performance improvement establishes glTF rendering conformance. The separate
SourceFormat/Preserve/GeometryEdit consumer candidate is not part of this slice.

## Source Review and Qualification

Independent design and source reviews inspect the bounded bodies, weighted scheduler,
physical prefixes, panic/error paths and unchanged numerical owner. The actual formatted
integration receives a separate delta review, including the borrowed core-reader UFCS call,
all mechanical callers and current API documentation. No actionable P1/P2 remains at that
reviewed snapshot. The isolated patch and all reviews remain under
`.git/baozi-review/performance-convergence/normals-semantic-actions-*`.

New concrete oracles distinguish denied face reads from mid-face unwind, retained output
sentinels and semantic versus completion error priority. Literal ordering vectors and the
0/1/31/32/33-row frontier cases check chunk denial. Real tiny runtime totals are Flat 18,
Smooth 39 and hard-zero-group Smooth 41. Weighted cancellation tests use literal completed
prefixes rather than interpreting credits as consumed actions. Existing geometric, precision,
hard-group, seam, source-bit, failure ownership and pending-permit cases remain.

| Check on Windows, Rust 1.95.0 | Result |
| --- | --- |
| Operation/core/import/postprocess all-feature nextest | 2,297/2,297; six existing skips; `a5e476e8-a8ea-4262-a8da-859965f2a16b` |
| Import and operation compiler contracts | Import 17 negative; operation one positive plus 39 negative; passed |
| Core and postprocess compiler contracts | Core six negative; postprocess one positive plus three negative; passed |
| Strict affected all-target/all-feature Clippy | Passed with `-D warnings` |
| Strict standalone asset-consumer Clippy | Passed with `-D warnings` |
| Public facade control assurance | 9/9; `ae43ee03-3588-48ae-b11f-22ec49ef8b57` |
| Asset-consumer all-feature nextest | 124/124; one existing skip; `1e9e5834-a99e-4fee-a861-362482903b21` |
| Standalone scene consumer | All-feature/all-target compilation passed |
| Lifecycle fuzz target | `edit_lifecycle` compilation passed; no new fuzz campaign claimed |
| Affected packages, asset/scene consumers and fuzz formatting | Passed |

Commands use locked/offline dependencies, one build job and the shared target. The integrated
run records an unchanged source freeze. Two unused test constants are then deleted;
independent review reconstructs that exact non-executable delta and confirms every other
recorded hash. Strict lint, compiler contracts, control assurance and consumer checks run on
the cleaned source. This delta is disclosed rather than attributing the initial test run to
later bytes. Exact commands, logs, source hashes and check reconciliation remain in
`target/review/convergence/r5-*.json` and matching logs, including `r5-tested-source.json`.

Ordinary performance evidence remains a separate pending gate. Neither a speedup nor release
readiness follows from the smaller work count. The previously confirmed R4 F64 extraction
regression and the full Windows/Linux all-endpoint convergence target remain open.
