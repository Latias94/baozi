# Finite Geometry Payload Construction

Source commit `b3a748c` closes all three geometry float writers and removes the ordinary
source-finite and retained-finite passes. F32/F64 data remains contiguous and directly
borrowable. Integrated source qualification and the four ordinary Windows campaigns are
complete. GNU completes C0/N0/N1; its C1 never starts within the original load gate.
Independent ordinary, instruction, heap and final integration reviews are complete. The
accepted disposition retains the qualified construction invariant as an intermediate
development-source improvement, permitting a local main fast-forward with the explicit
unresolved native costs attached. No release readiness or general performance convergence
is established.

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
At this source-qualification checkpoint, `b3a748c` is committed on
`refactor/finite-geometry-writes` and local main remains at prerequisite `19c0a42`.
The final integration disposition below follows the separate measurement reviews.

Final receipts are `finite-pool-final-qa-3.json`, the two final source manifests, the
independent correctness/simplicity reviews and the bounded final test-delta review. The
earlier 702-test checkpoint and failed 3,609-test run retain their original distinct scope.

## Ordinary Windows observations

The fixed C0/N0/N1/C1 sequence completes all four original 110-lane campaigns. Each uses
three warmup pairs, twenty retained AB/BA pairs and 5,062 successful commands. Sampled
nonempty named-tool observations are 83/79/91/22; these are not quiet-host measurements.
The independent raw review reconstructs all 20,240 worker stdout objects, 440 campaign
rows, 220 separate-campaign version changes and 220 Windows table rows without discrepancy.
Warmups are checked and excluded from retained statistics. Every executable and recorded
semantic task retains its original identity.

| Memory-input lane | C0 median us | N0 median us | N1 median us | C1 median us | N0/C0 change | N1/C1 change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| STL1024 import | 1,845.100 | 1,478.400 | 1,466.600 | 1,840.750 | -19.87% | -20.33% |
| STL1024 pipeline | 2,124.200 | 1,784.250 | 1,772.050 | 2,116.000 | -16.00% | -16.25% |
| Grid128 pipeline | 136,887.900 | 131,099.150 | 131,855.000 | 134,909.800 | -4.23% | -2.26% |
| Shared glTF8 prepare | 84.400 | 89.800 | 97.700 | 84.600 | +6.40% | +15.48% |
| F64 PLY pipeline | 1,348.450 | 1,336.600 | 1,388.450 | 1,323.900 | -0.88% | +4.88% |

These changes compare separate campaign medians, not paired source versions or isolated
causal effects. STL1024's candidate pipeline B/A pair medians remain 7.535/7.625, above
the release target. Preparation has 22/26 increases in each comparison and 21 in both;
47 lanes increase in both comparisons across all stages. File F64 PLY preparation and
extraction, point-PLY extraction and some file pipelines also retain positive changes.
Small absolute durations and load confounding do not clear these unresolved costs.

| Candidate campaign | Lanes B/A >3 | Lanes B/A >=10 | Pipelines B/A >3 | Maximum lane median peak-WS B/A |
| --- | ---: | ---: | ---: | ---: |
| N0 | 56/110 | 34/110 | 9/28 | 1.802278 |
| N1 | 57/110 | 35/110 | 9/28 | 1.804237 |

These are descriptive retained-pair counts, not confirmed population classifications or
acceptance. Working set is the whole child process, distinct from commit and endpoint heap.
The independent review retains all commit growth and parent-wall outliers too. The
[all-lane export](2026-11-06-finite-geometry-payload-lanes.md) preserves absolute medians,
IQRs, both change signs, same-campaign comparator ratios, candidate maxima and process memory.

Recorded STL1024 pipeline work drops from 116,328 to 79,464 in the functional and every
retained ordinary observation: 36,864 actions, matching the removed 18,432 source-finite
and 18,432 retained-finite visits. Complete recorded semantic observations and output
logical/capacity bytes remain identical between sources, excluding clock and work fields.
No functional-preflight clock contributes to the native results above.

At the Windows-only review checkpoint, the bounded disposition retains a source-qualified
intermediate candidate, while deferring measured-performance acceptance, local-main
integration and release until the GNU/native and diagnostic evidence informs disposition.

## Ordinary GNU observations

The native-filesystem GNU sequence completes C0/N0/N1, each with all original 110 lanes,
three warmup pairs, twenty retained AB/BA pairs and 5,062 successful commands. Independent
raw review reconstructs all 15,180 ordinary worker stdout objects, including 13,200 retained
and 1,980 warmup observations, all 330 campaign rows and all 220 Linux table rows without
discrepancy. The 220 Windows table rows remain mathematically identical to the unchanged,
previously reviewed Windows summary; that limited check does not repeat Windows raw review.

C1 never starts: all 91 blocked observations retain Windows build/test tools during the
original 180-second joint gate. The wrapper terminates with code 2 for that gate expiration.
There is no C1 campaign or N1/C1 version comparison. The summary contains only N0/C0
changes, and the all-lane export leaves C1 and N1/C1 unavailable.

STL1024 memory-pipeline medians are 1,327.383/991.424/1,005.586 us for C0/N0/N1,
with IQRs 89.493/58.862/94.039 us. N0/C0 is -25.31%; N1 is a second candidate
observation, not reverse-control confirmation. Its same-campaign B/A medians remain
6.554436/6.688865 for N0/N1. All original memory-pipeline work observations record
116,328 -> 79,464 actions, and complete recorded semantic/output observations remain
unchanged after excluding clocks and work.

Other source-median observations remain mixed. N0/C0 has 67 increases in 110 lanes,
including 19/26 preparation lanes and 20/28 pipelines; 50 increases exceed 3% and 25
reach 10% using unrounded changes. STL1024 memory preparation rises 46.13%, F64 PLY
file preparation 31.86%, and triangle-PLY memory pipeline 36.40%. These unresolved
costs remain attached to the candidate; they are not cleared by the STL improvement.

| Candidate campaign | Lanes B/A >3 | Lanes B/A >=10 | Pipelines B/A >3 | Maximum lane median RSS B/A |
| --- | ---: | ---: | ---: | ---: |
| N0 | 70/110 | 29/110 | 14/28 | 2.080854 |
| N1 | 69/110 | 29/110 | 14/28 | 2.074086 |

All observed individual retained candidate RSS pair ratios also remain below 3. RSS is
whole-child lifetime memory, separate from endpoint heap. These descriptive observations
do not establish stable population bounds or clear the outstanding timing gates.

Linux watched-tool snapshots are empty, but the external Windows monitor records
20/27 nonempty observations within the N0/N1 command intervals, including 20/26
within their actual ordinary-child spans. C0 has no watched-tool detections in its
sampled interval; none of these sparse observations establishes a quiet host. Candidate
load overlap, control drift and missing C1 prevent isolated source-effect acceptance.

At the GNU ordinary-review checkpoint, the bounded disposition retains an isolated
source-qualified intermediate working candidate with named unresolved costs, deferring
measured-performance acceptance, local-main integration and release. Separate instruction/heap
diagnostics do not fill the missing control or clear endpoint/pipeline release targets.

## Separate instruction attribution

Two exact-source opt3/debug1 GNU workers complete six original pipeline/memory tasks each.
All twelve raw summaries, totals, stderr instruction counts and exclusive annotation sums
reconcile exactly. Independent review also checks all twelve stdout/stderr pairs, 24
annotations, source archives and binary identities. Whole-worker instructions include
startup, preload, checking, serialization and teardown; they are not native CPU time or
isolated stage costs.

| Original task | Control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| glTF shared 1 | 5,964,450 | 5,955,363 | -0.15235% |
| glTF shared 32 | 29,962,882 | 29,556,893 | -1.35497% |
| F64 PLY neighbors | 2,515,899 | 2,516,063 | +0.00652% |
| STL repeated 1024 | 23,650,389 | 14,074,422 | -40.48968% |
| OBJ repeated 16384 | 809,003,721 | 798,708,899 | -1.27253% |
| OBJ grid 128 | 2,747,283,456 | 2,692,357,267 | -1.99929% |

Raw action streams confirm the exact deleted visits. STL source-finite and retained-finite
actions each go from 18,432 to zero. Grid source-finite actions go from 83,208 to 33,285,
preserving projected input; its 541,443 retained-finite visits disappear. Every observed work
delta matches the corresponding removed actions. Recorded semantic summaries and output
logical/capacity bytes remain identical between sources after excluding clock and work.

The replacement itself has attributed cost. STL's printed `append_payload` exclusive Ir
increases by 55,310 and its forwarded typed-build `run_unit` by 184,176; grid's corresponding
increases are 3,983,637 and 5,410,198. These differences include qualification, capacity and
code-generation effects and do not isolate the cost of `is_finite`. The F64 net increase of
164 Ir remains visible. No overlapping inclusive subtrees are added into predicted savings.

Remaining candidate grid exclusive routine families include parser completion (380,268,941
Ir, 14.1240%), guarded prefix execution (222,562,597, 8.2665%), child scanning (221,515,838,
8.2276%) and child admission (209,986,422, 7.7994%). glTF32 retains guarded prefixes,
layout binding and parser completion as leading routine families. These are concrete next
investigation owners, not a forecast that all their work can disappear.

The glTF32 `prepare_triangles` physical root owns 462,779 inclusive Ir, 1.565723% of this
candidate worker. A source audit finds three scratch vectors and a second `changed()` pass
inside unchanged preparation, but those are only part of that root. This smaller ownership
opportunity cannot explain the entire pipeline gap. Parser admission/completion remains a
higher-priority investigation from this evidence; separate prepare endpoint costs stay open.

Both grid profiles retain `brk segment overflow` warnings despite successful exits and exact
totals. Software SHA-256's exclusive share under emulation is not its native CPU-time share:
the locked implementation dispatches using CPU features, whose native result was not measured
here. Each profile is a single observation, all controls preceding all candidates. These limits
remain attached to the instruction evidence.

## Separate whole-worker heap observations

Six original pipeline/memory tasks complete once per control, candidate and Assimp engine:
18 worker and 18 heap-analysis exits are zero. Independent review checks all 18 stdout,
stderr, analyses and trace identities, 74 primary evidence files and 144 analytic facts.

| Original task | Baozi calls, control = candidate | Baozi displayed peak, control = candidate | Assimp calls | Assimp displayed peak |
| --- | ---: | ---: | ---: | ---: |
| OBJ repeated 16384 | 33,389 | 12.18M | 82,154 | 19.91M |
| OBJ grid 128 | 99,425 | 28.82M | 360,697 | 28.69M |
| glTF shared 32 | 1,116 | 416.53K | 845 | 174.36K |
| STL repeated 1024 | 488 | 456.56K | 1,186 | 317.09K |
| F64 PLY neighbors | 518 | 135.97K | 187 | 2.18M |
| PLY points 1024 | 482 | 188.40K | 4,281 | 2.23M |

Baozi's aggregate calls, temporary-allocation counts and displayed peaks match between
sources in every task. The closed pool introduces no additional observed allocation layer.
Equality of rounded decimal displays does not prove exact peak-byte equality or every site's
behavior. This is instrumented whole-worker heap, including preload/setup/checking/serialization
and teardown; it is neither native RSS, endpoint heap nor an elapsed-time comparison.

The 544B/4.10K tool-reported exit-live values are not attributed to Core as leaks without
separate stack evidence. Prior interception calibration covers only its documented allocator
paths. No universal coverage, repeated population peak or rejection/edit qualification is
inferred from this successful finite corpus.

## Accepted source disposition and remaining release work

Retain the closed construction owner as a qualified intermediate source change. It replaces
the mutable float escape with one checked writer, deletes two observed redundant passes and
their charges, preserves successful semantic tasks and passes full integrated correctness
gates. Independent native and diagnostic review finds no evidence corruption. STL has
repeated ordinary improvement in Windows and a compatible GNU/Ir result, with no observed
heap regression in the six diagnostic tasks.

This decision accepts the recorded native preparation/extraction increases and mixed small
pipelines as unresolved working-source costs; it does not dismiss them as noise or prove
that the source caused every increase. Windows load overlap and GNU's missing final control
prevent general isolated performance acceptance. The ordinary reviewers recommend keeping
the isolated candidate while deferring main integration. The separate final integration
review accepts a local development-main fast-forward for the closed ownership invariant,
actual scan deletion and qualified source, with all named costs retained. It finds no
blocking source-correctness, evidence-integrity or convergence-policy counterexample. This
judgment does not upgrade the ordinary reviewers' conclusions or grant release approval.

The intermediate-source provision already exists on control `19c0a42`; this disposition
does not introduce it after measurement. The final review checks all four frozen evidence
documents, their pinned summary hashes and the 909-input QA association. Its local receipt
is `.git/baozi-review/performance-convergence/finite-pool-final-integration-review-1.md`;
the reviewed evidence manifest is
`0ffba98674a066266f9aea5c12327c0cafd0ab421408e1fdd98d170678addfef`.
The current documentation-only disposition records that decision; the measured runtime
remains `b3a748c` within candidate `a1064ff`.

Every original endpoint/pipeline timing and comparable-process-memory convergence target
remains active. The next work should attribute the existing parser admission/completion
routes before changing algorithms or introducing another execution framework. Keep the
unchanged-prepare producer decision as a bounded secondary hypothesis, preserve all original
measurement tasks and record a fresh exact-source comparison for any subsequent code change.

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
