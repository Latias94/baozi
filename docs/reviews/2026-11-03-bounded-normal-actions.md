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

This table records correctness qualification. Neither a speedup nor release readiness follows
from the smaller work count; ordinary evidence is assessed separately below. The previously
confirmed R4 F64 extraction regression and the full Windows/Linux convergence target remain open.

## Ordinary Windows Evidence and Retained Costs

Qualified runtime source is `57c33b2447a296c28f5f5817a36debd248bd4280`. The ordinary Windows
worker has SHA-256 `b583f6a58191fb8f5e8ba00351c5f4f08e3d8d4081b3b9a5736f7f7ba311e053`,
and its source archive is
`3f4d6acf8611064720b748b632eaeeba2e4b70153de9ed47c2c8831d018fe2b9`.
Four serial six-case, 23-lane memory-input campaigns compare retained R4 and R5 in both version
orders, each with twenty retained alternating AB/BA engine pairs after three warmup pairs.
All 4,232 children succeed. Build/profiling runs do not overlap ordinary collection.

Independent review reconstructs every raw observation, engine/output statistic and paired
summary, with unchanged collector, corpus, comparator, selected F64 output and comparison
adapter. Its intermediate keep basis is the repeated-OBJ preparation and pipeline gains,
which have disjoint old/new Baozi sample ranges in both rounds and persist in half/order
splits. Source semantics, correctness and the observed gain support retention while the
explicit costs below remain repair obligations. This is not a general no-regression result.

Times are milliseconds; changes are R5/R4 minus one:

| Endpoint | Initial R4 / R5 | Change | Reverse R4 / R5 | Change | Reverse R5 / Assimp |
| --- | ---: | ---: | ---: | ---: | ---: |
| Repeated OBJ prepare | 24.10805 / 17.72435 | -26.48% | 23.88190 / 17.55690 | -26.48% | 16.21x |
| Repeated OBJ pipeline | 59.02360 / 50.20880 | -14.93% | 57.20780 / 49.91380 | -12.75% | 5.16x |
| Grid OBJ pipeline | 155.05620 / 154.99765 | -0.04% | 154.31165 / 154.93725 | +0.41% | 6.00x |
| Shared glTF pipeline | 4.55935 / 4.42050 | -3.05% | 4.64800 / 4.65705 | +0.19% | 13.46x |
| Repeated STL pipeline | 3.00455 / 2.89505 | -3.64% | 3.08250 / 2.99200 | -2.94% | 12.00x |
| PLY points pipeline | 1.66450 / 1.56945 | -5.71% | 1.69775 / 1.65490 | -2.52% | 2.14x |
| F64 neighbors pipeline | 1.38835 / 1.33595 | -3.77% | 1.39440 / 1.36370 | -2.20% | 2.25x |
| Grid OBJ extract | 2.30050 / 2.03095 | -11.72% | 1.96200 / 2.06710 | +5.36% | 1.96x |
| F64 neighbors extract | 0.01285 / 0.01065 | -17.12% | 0.01295 / 0.01085 | -16.22% | 9.86x |

The grid-extraction confirmation increase is 105.10 us. Its Q1/Q3 bands are separated, and
half/order splits retain the increase. The first pilot instead improves alongside substantial
control movement. Preserve this as a localized confirmation cost, not a reproducible
two-round causal regression, and keep its source-qualified explanation/correction and
ordinary confirmation open. Neither the pilot nor pipeline median clears it. Repeated
extraction also changes sign, glTF pipeline is effectively flat, and overlapping small-lane
distributions do not establish independent material gains.

F64 extraction improves in both current comparisons, but its archived public reader is
unchanged. These observations do not establish a reader repair or a fresh matched R3/R5
comparison. Historical R3 medians cannot close R4's confirmed regression by comparison across
campaigns. Both the original evidence and its repair obligation remain. Assimp drift is
reported without causal subtraction; repeated-OBJ pipeline control changes -21.88% initially
and -1.64% in confirmation. The initial candidate improvement alone is not the reason to keep.

The six pipeline medians sum to -4.13% / -3.07%. Fifteen of 23 timing lanes and four of six
pipelines still exceed 3x Assimp in both rounds. Maximum endpoint ratios are 102.59x / 93.37x;
maximum pipeline ratios are 13.18x / 13.46x. Short no-op preparation remains a major cost.
Parent process timing and tails are retained separately: a candidate grid pipeline tail and
440.58 ms preparation parent observation, and a 2,441.49 ms R4 extraction parent observation,
prevent a whole-process or tail-safety claim. No outlier is trimmed and no p95 SLO is inferred
from twenty samples.

Comparable whole-child Windows working-set and commit high-water marks include setup, earlier
stages, observation and cleanup. Median changes stay within 0.67% / 1.07%, with maximum
same-campaign Baozi/Assimp working-set ratio 1.66782x. There is no material confirmed memory
regression in these selected lanes. This is not stage heap, retained storage or Linux RSS,
and small process-level decreases are not attributed to a storage optimization.

| Report under `target/assimp-comparison/` | SHA-256 |
| --- | --- |
| `convergence-windows-r5-r4-control/report.json` | `8cf75f995f1acd9403ae5a9095e64a4203967a5894d3b244165c8150f5ab5901` |
| `convergence-windows-r5/report.json` | `b56a47fbe19255c3336093454f859b499ddae3dc465eca2b53af115eb34a5378` |
| `convergence-windows-r5-confirm/report.json` | `671a941628727add3721a75bd5ed1f8c1047f82e137b047c4344cc0ba4e7f458` |
| `convergence-windows-r5-r4-control-confirm/report.json` | `09869405dce12df26b130676939a785a820b86e6d8cefeedd7f97b5c581920c1` |

The detailed independent disposition is
`.git/baozi-review/performance-convergence/normal-actions-measurement-review.md`. These
artifacts qualify R5 alone; later consumer-policy, reader-codegen, glTF-profile and exclusive
invocation candidates need their own source and evidence identities.

## Refreshed Linux Instruction Attribution

Six separate whole-worker Callgrind profiles run after ordinary collection, using the same
R5 source archive and symbol-worker SHA-256
`d3d7c1ed09e191101c89701b2b241ec35d6c049a4ac2975db7176020842ef391`.
Main reconciles every flat exclusive sum with raw totals and reproduces the previous owner's
classification exactly on all six R4 profiles. Repeated OBJ Ir falls 15.14%; grid, glTF, STL,
points and F64 instead increase 1.26%, 2.11%, 2.11%, 1.66% and 0.69%. Those increases remain
explicit costs for investigation, not native-time conclusions. Operation-owned exclusive
instructions still account for about 64% of the repeated worker and 56% of grid. These are
whole-worker instruction owners, including setup/checking/teardown, not stage CPU fractions.

Raw profiles, annotations and the exact summary remain under
`target/review/convergence/r5-attribution/`; summary SHA-256 is
`deecd8faf2f1920f273737d9af87a4b5d782e3a499f5139fee291a503185e985`.
Further investigation must use actual call subtrees before choosing the next replacement.
Neither this attribution nor the intermediate Windows keep clears the full fourteen-case,
110-lane ordinary Windows/Linux endpoint, pipeline, memory and correctness gates.

The subsequent read-only call-edge investigation independently rechecks all twelve R4/R5 raw
hashes and exclusive sums. In the repeated R5 worker, the `prepare_triangles` subtree is
384,159,274 Ir: normal input/generation contributes 97,468,246 and Scene edit contributes
286,570,373. These are nested function costs, not elapsed-stage fractions. Measured normal
Begin calls fall from 573,463 to 121,870; the previous design estimate is now corroborated by
actual successful call edges. Within that normal subtree, Begin/Complete children still cost
54,078,410 Ir. The emitted generic wrapper also contains numerical work and cannot be counted
as pure dispatch overhead.

Scene edit's canonical semantic adapter still performs 262,189 fresh permit lifecycles. Its
admit/complete/finish children contribute a disjoint 135,289,524 Ir at that caller. Planning,
materialization and correspondence each traverse generated normal payload through their real
writer modes; this is one normal computation followed by three required transaction
traversals. The next owner refactor must address that execution/writer boundary while retaining
admission, correspondence, validation and immediate physical prefixes. Replacing the normal
formula alone cannot remove this larger remaining route.

Other workers' instruction increases also have a concrete common route: their unchanged
cost-one `before_action` calls now pay twelve additional Ir per emitted call for the checked
weighted contract. Full control-poll counts are unchanged on those five profiles. Newly
emitted completion functions partly replace previously inlined work, so their full cost is
not all net new work. Preserve one checked control owner while simplifying its composition;
a second unweighted backend or relaxed cancellation is not justified. Exact edges and inline
limitations are in `.git/baozi-review/performance-convergence/r5-normal-cost-investigation.md`.
