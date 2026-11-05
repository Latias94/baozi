# Finite Geometry Payload Construction

Source commit `b3a748c` closes all three geometry float writers and removes the ordinary
source-finite and retained-finite passes. F32/F64 data remains contiguous and directly
borrowable. Integrated source qualification is complete. Native measurements remain pending;
this record does not establish a performance keep or release readiness.

## Cause and bounded change

Ordinary geometry previously checked each source float, copied it under another admitted action,
then checked the stored float again. The final check was necessary while raw mutable Vec fields
escaped the constructor. Initial construction, selected reconstruction and spatial conversion
all wrote those fields, so changing only the first constructor could not close the invariant.

One private Vec-backed `FiniteScalarPool<T>`, sealed to F32/F64, now owns initialized floats.
It exposes immutable slices, capacity and checked scalar/bounded-tuple writes. There is no
mutable slice, raw-Vec conversion, per-value wrapper, parallel validity buffer or new allocation.
Each write qualifies the final-width value inside its existing admitted action. A rejected
attempt completes its actual work without appending a cell; prior completed actions remain
consumed. Capacity and exact finish still use the admitted plans.

Initial construction transports only Copy attribute/scalar facts through the previously qualified
retention envelope. Core maps them to `NonFiniteAttributeValue`, rather than an internal invariant
failure. Selected and converted writes retain their deterministic stronger preflights and treat
unexpected invalid output as construction failure. Unit-normal/tangent checks, conversion
overflow/underflow and exact output correspondence remain.

The private `PreparedGeometry*` family now names the source preparation phase accurately.
Ordinary dense/sparse values are qualified when written. Projected F32 input still scans its
entire source, including unselected values and present-empty projections, before checking its
projection indices. Sparse-default coordinates follow the explicit values. Resource, structural
or control rejection can precede an ordinary invalid float that has not yet been observed;
actually observed causes retain their existing precedence and cleanup rules.

## Surviving validation and physical work

Central geometry validation still checks actual ranges, complete pool/name coverage, lane and
domain shape, topology, sparse index order/bounds, tangent declarations and cross-domain
references. Only the float finite visits disappear. Test fixtures use the checked writer too;
post-build NaN injection is removed, while structural faults and changed finite bits remain
testable by constructing another checked pool.

The initial plan, actual storage and selected-output work queries independently compute:

```text
index work = (U32 pool cells + I32 pool cells) - integer attribute scalar cells
geometry validation work = meshes + 2 * primitives + attributes + instances + index work
```

The checked integer subtraction occurs before adding rows. Spatial conversion derives output
validation from source work plus synthesized indices. Sealed central sums, aggregate limits and
edit rebinding consume the same convention. Float capacity, semantic cells, storage bytes,
copy actions and correspondence work remain charged. No deleted scan receives synthetic credit.

## Verification checkpoint

Before deleting the scans, the closed-writer checkpoint passes all-target Core checking and
702 Core tests, with four existing skips and 909 stable Rust/TOML inputs. Its independent
production review finds no confirmed P1/P2. This checkpoint still includes both old scans;
it does not qualify the later scan deletion.

The first integrated run executes 3,609 workspace tests: 3,604 pass, five fail and six existing
cases are skipped. Four failures stop in a shared new test helper because it supplies absent
roots for empty nodes; the fifth retains the old rewrite validation expectation of 96 instead
of the 27 surviving row/index actions. Two test-only corrections supply explicit empty roots
and derive `1 mesh + 2*2 primitive visits + 7 attributes + 1 instance + 14 indices = 27`.
All 35 other frozen review inputs remain identical. An independent bounded review accepts
these corrections without weakening runtime behavior or assertions. The original failed run
and source map remain retained.

Independent final correctness and simplicity reviews find no confirmed P1/P2 or required
simplicity correction. They confirm all three writers and the actual removal of both ordinary
finite passes. The simplicity review records one optional pre-existing preparation forwarding
function; it has no extra controlled work and no native benefit is claimed. A suggested fused
geometry/retention cancellation-frontier regression remains an additional composition test;
the reviewed source and separate writer/retention frontier tests show no concrete defect.

The corrected formatted candidate completes all seven serial qualification gates:

| Gate | Actual result |
| --- | --- |
| Workspace nextest | 3,609 pass, six existing skips |
| Workspace doctests | 188 pass |
| Workspace strict Clippy | All targets/features pass with warnings denied |
| External asset consumer | 138 pass, one existing skip |
| Consumer strict Clippy | All targets/features pass with warnings denied |
| Workspace and consumer formats | Both pass |

All 909 recorded Rust/TOML inputs remain byte-identical before and after this run and at
precise 36-path source staging. The five pre-existing untracked user notes remain unchanged.
The source is committed as `b3a748c`; the branch remains `refactor/finite-geometry-writes`
until measured disposition. Local main remains at the qualified prerequisite `19c0a42`.

Final receipts are `finite-pool-final-qa-3.json`, the two final source manifests, the
independent correctness/simplicity reviews and the bounded final test-delta review. The
earlier 702-test checkpoint and failed 3,609-test run retain their original distinct scope.

## Assimp source comparison and measurement boundary

The local clean Assimp reference at `a2a586d9370a493651256ec5d03524710ce37a5f` loads STL
through `TextFileToBuffer`, then allocates output arrays and fills facet tuples. Its importer
executes the optional `aiProcess_ValidateDataStructure` before preprocessing and explicitly
removes that flag from the subsequent postprocessing call. This source evidence does not support
assuming that Assimp's STL advantage comes from streaming file input, nor does it establish
the behavior of every format or the flags used by a particular measurement.

Native qualification must retain the original 14 cases, 110 lanes, three warmup pairs and
20 retained AB/BA pairs on Windows and GNU. Diagnostic instructions, native endpoint times and
whole-child memory remain distinct measurements. No savings are forecast by adding inclusive
callchain percentages. Every release convergence target remains open until measured.

Local source/QA/review receipts remain under `target/review/convergence` and
`.git/baozi-review/performance-convergence`; original failed compiler runs and intermediate
snapshots are retained. No push, PR, registry publication or yanking is part of this slice.
