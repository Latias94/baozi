# Remove Integer Attribute Validation No-ops

This cut removes two value scans that cannot reject any U32/I32 attribute
scalar: empty semantic callbacks over borrowed input, and always-true predicates
over retained integer values/defaults. There is no integer analogue of NaN or
infinity. Shape, type/schema, lane/cardinality, typed-range coverage, sparse index
order/range, topology indices and polygon counts still require real checks.

The baseline is `9f3a739`. This cut follows the
[semantic/OBJ execution change](2026-11-05-semantic-copy-and-obj-lexical.md), whose
native performance has not yet been qualified. Correctness checks are not
performance evidence. The next ordinary comparison must identify the cumulative
source changes since the measured `56eecd9` worker.

## Physical Work and Stored Data

For valid retained geometry, let `D` count all explicit integer attribute value
scalars plus stored defaults. Count every lane once, including default-only
sparse attributes. Exclude sparse indices, primitive indices and polygon counts.
The retained validation recipe is:

```text
rows = logical_meshes + 2 * primitives + attributes + mesh_instances
validation_work = rows + F32_pool + F64_pool + U32_pool + I32_pool - D
```

This subtracts only the deleted integer predicates. Actual copying, pool
capacities, scalar cells, names and bytes remain. Semantic preflight independently
records its own remaining real actions; its work is not replaced by this retained
formula. No aggregate completion or padding stands in for a deleted loop.

The input validator accumulates one checked `integer_attribute_scalar_count`
fact during its existing attribute dispatch. Immutable retained geometry keeps
that same fact so source work queries remain constant-time. The field contributes
to the actual `size_of::<GeometryStorage>()` header footprint; the change does not
claim memory savings.

Four work owners change together: the import storage plan, existing retained
geometry, selected/reconstructed output, and coordinate-converted output. The
selection Count pass records integer data from actual output attribute metadata;
triangulation can change those lengths. Space conversion preserves this count
and accounts for any synthesized topology indices separately. The retained
validator checks the stored count against metadata encountered in its existing
range/cardinality traversal, without reading integer data values. Exact envelope
completion remains required.

The semantic integer inspection helpers are deleted. Sparse generic integer
inspection returns immediately; floating inspection still performs the same
finite predicates. Retained dense/sparse dispatch invokes value predicates only
for floating types. Integer sparse indices retain their real controlled loop.
Cancellation, denial-before-effect and completion apply to the remaining actual
actions; the removed no-ops are no longer artificial cancellation boundaries.

## Verification

Independent production review finds no actionable correctness or unnecessary
complexity issue. Test review finds one stale triangulation precision fixture:
its integer-to-F64 replacement must remove the old integer metadata quantity.
The corrected fixture keeps its original precision and correspondence assertions.
The follow-up review closes that finding and the default-only materialization
coverage gap. The added regression retains a two-lane SparseU32 default
`[0, u32::MAX]` with no explicit items; coordinate conversion preserves those
bits and accounts for three real synthesized topology indices, changing retained
validation work from 16 to 19 without integer-value padding.

The four existing fixture changes are source-derived: semantic work 73 -> 71
and 69 -> 64, retained mixed geometry 35 -> 34, and triangulated output 106 -> 96.
The last uses output integer data 10, not source data 8. Existing sparse-index,
topology-index, range-coverage, nonfinite F32/F64, cancellation and exact-envelope
regressions remain. Corrupting the cached metadata count is also rejected.

The final Core source passes 697 tests with four existing skips. Earlier failures
remain in their original receipts: the new semantic fixture initially had
out-of-order custom names; normal/tangent integer-to-F64 fixtures and the reviewed
triangulation fixture initially retained stale counts. Those are corrected test
inputs, not waived semantic checks. The first full workspace run completes 3,592
tests with 3,591 passes, one stale-fixture failure and six existing skips.

The final source then passes all seven fresh serial gates: 3,593 workspace runtime
tests with six existing skips, 188 documentation tests, 138 external asset-consumer
tests with one existing skip, both strict all-target/all-feature Clippy checks,
and both format checks. All 908 recorded Rust/TOML source inputs remain identical
before and after qualification. The actual terminal result is zero; the retained
receipt is `target/review/convergence/integer-noop-qa-2.json`.

| Evidence | SHA-256 |
| --- | --- |
| Final seven-gate QA | `d58a64eec3749ec12f7ebbfd54540cd8b51e707d378e1184008d0bef21fec472` |
| Production source review | `1fe4d10d4f7e063573d4afe4792ecffa941e3d3662b60fadf801fe6c20888dfa` |
| Final test follow-up review | `f1a8cbf215df548beeb261b3a640ea284134764b56017f40fcb2f7d9da1e65f5` |

The private reviews live under `.git/baozi-review/performance-convergence/` with
the `integer-noop-*review*` names. Final identity checks match all seven reviewed
production files and both follow-up test files. No native timing, allocation or
RSS result is attributed to the source qualification.

## Construction Invariant Follow-up

The separate Interface investigation confirms a narrower opportunity beyond
integer no-ops. `ValidatedGeometryBatch` holds actual immutable source slices
and compiled metadata. Direct and sparse floating data are checked finite there,
copied without floating arithmetic by the initial typed builder, then checked
finite again in retained storage. The same finite predicate is repeated for
those unchanged values. Projected F32 is different: its source scan also checks
unselected source tuples, while construction can repeat selected tuples.

The current output remains mutable before retention, and edits construct new
values and layouts. A capacity-only `GeometryStoragePlan` also accepts its batch
separately. These Interfaces do not yet establish an immutable finite-output
invariant; a blanket switch that skips the final finite pass would be premature.
Retained range coverage, cardinality, topology and cross-domain checks protect
newly constructed output, independently of source immutability.

The preliminary preferred direction is a private geometry construction Module
that binds validated input to its plan, closes numeric mutation before freezing,
and guarantees finite pools through its construction Interface. Initial typed
copy/gather can transfer that invariant; edit output must obtain it through real
numeric checks on its produced values. The central pass would still check actual
layout and references. The existing exact envelopes must retain one coherent
interpretation across initial construction and edits. No separate proof engine,
payload hash, per-scalar provenance map or second numeric ledger is proposed.

This is a source-grounded design recommendation, not an implementation, measured
saving or accepted replacement. Qualify and measure the current cut first, then
attribute the remaining source inspection/copy/retained-finite intervals before
choosing the next change.

## Remaining Work

Measure the cumulative candidate with immutable ordinary binaries against the
original fourteen cases and 110 lanes, three warmup pairs and twenty retained
AB/BA pairs. Keep all failed lanes and host-load observations. Windows and Linux
release targets remain open.

The subsequent [finite measurement attempt](2026-11-05-semantic-integer-performance-attempt.md)
builds both ordinary workers and passes both versions' original functional
preflights on Windows and Linux. Fixed load gates stop Windows before candidate
sampling and Linux before ordinary sampling. No performance effect is established.

Subsequent removal of repeated validation must identify the input/output owner
and the exact invariant being transferred. A type or construction proof can
replace an internal runtime scan when it covers the actual produced storage;
a predicate that can detect malformed indices, nonfinite results, schema errors
or exceeded resources remains necessary until an equivalent owner contract is
implemented and qualified. Avoid adding a second ledger or generic proof engine
merely to retain historical work quantities.
