# Semantic Execution: Fresh Instruction and Heap Attribution

The cumulative `e5ca93b` candidate reduces whole-worker instructions for large OBJ
and STL tasks, while three small tasks increase. Allocation-call counts do not
decrease. These observations narrow the next implementation target; they do not
establish native speed, RSS, an isolated source-change effect or release readiness.

The preceding [ordinary attempt](2026-11-05-semantic-integer-performance-attempt.md)
could not start candidate sampling. This follow-up retains that limitation and
collects separate bounded diagnostics from the exact source archive.

## Source and Method

Candidate `e5ca93b82f2041e82e6bbaa6913462be551bd69f` is compared with retained
control `56eecd985693cba1826326169e6c673a7b36e3ff`. Both symbol workers use Rust
1.95, generic GNU x86-64, opt3/debug1, System allocation and no allocation-metrics
feature. The candidate's 1,470 source inputs remain unchanged across its successful
build. Its archive is the same committed archive used for the ordinary builds.

Six candidate Callgrind observations use the original memory/pipeline tasks and
their unchanged fields, normals, precision and work ceilings. The controls are
historical same-policy profiles, not simultaneous source pairs. Whole-worker Ir
includes setup, preload, checking, serialization and teardown. Every source/task
has one observation; no distribution or native-time claim follows.

| Original task | Control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| glTF shared 1 | 5,920,175 | 5,967,110 | +0.793% |
| glTF shared 32 | 29,677,439 | 29,982,679 | +1.029% |
| F64 PLY neighbors | 2,484,828 | 2,512,824 | +1.127% |
| STL repeated 1024 | 25,427,021 | 23,642,326 | -7.019% |
| OBJ repeated 16384 | 819,357,617 | 810,329,922 | -1.102% |
| OBJ grid 128 | 2,938,588,473 | 2,761,416,772 | -6.029% |

Independent review reconciles all twelve raw profiles, their summary/totals and
stderr Collected values, all twenty-four annotations and their exclusive totals,
worker/archive identities, task metadata and aggregate geometry observations.
Both grid stderr logs retain the `brk segment overflow` warning. This instrumented
allocator qualification remains visible; it is not treated as a native result.

## Separate Heap Observations

Eighteen fresh heaptrack 1.5.0 tasks cover six cases and the two Baozi versions plus
Assimp. Assimp uses the retained B1 symbol workers, including the separate F64
worker; it is not substituted for the B0 ordinary comparator. Each task has one
whole-worker observation. Rounded decimal peak strings include the complete
worker lifetime and are not endpoint heap or comparable whole-child RSS.

| Task | Baozi calls, both versions | Assimp calls | Control peak | Candidate peak | Assimp peak |
| --- | ---: | ---: | ---: | ---: | ---: |
| OBJ repeated 16384 | 33,389 | 82,154 | 12.18M | 12.18M | 19.91M |
| OBJ grid 128 | 99,425 | 360,697 | 28.82M | 28.82M | 28.69M |
| glTF shared 32 | 1,116 | 845 | 416.52K | 416.53K | 174.36K |
| STL repeated 1024 | 488 | 1,186 | 456.54K | 456.56K | 317.09K |
| F64 PLY neighbors | 518 | 187 | 135.96K | 135.97K | 2.18M |
| PLY points 1024 | 482 | 4,281 | 188.38K | 188.40K | 2.23M |

`K` and `M` are the tool's rounded decimal units. The small displayed increases
are not independently established exact-byte differences or stable population
effects. No allocation-call reduction is demonstrated.

Every trace hash, raw allocation-record count, analysis footer and stderr
allocation total agrees. All eighteen temporary-allocation pairs differ, however:

| Task | Baozi control, analysis/stderr | Baozi candidate, analysis/stderr | Assimp, analysis/stderr |
| --- | ---: | ---: | ---: |
| OBJ repeated 16384 | 10/9 | 10/9 | 9/8 |
| OBJ grid 128 | 33262/33261 | 33262/33261 | 10/9 |
| glTF shared 32 | 14/13 | 14/13 | 16/15 |
| STL repeated 1024 | 8/7 | 8/7 | 10/9 |
| F64 PLY neighbors | 10/9 | 10/9 | 17/14 |
| PLY points 1024 | 9/8 | 9/8 | 1041/17 |

These original values remain separate. No inferred explanation or corrected
temporary count replaces them. Independent heap review finds no artifact, task,
allocation-call or main-summary mismatch within this bounded scope.

## Remaining Owners and Next Experiment

The STL source investigation distinguishes source finite checks, real typed
copies and retained finite checks. For the 1,024-triangle fixture, each phase
handles 18,432 F32 scalars and 3,072 structural indices. A private finite-geometry
construction invariant could remove the retained scalar predicates, reducing
that geometry validation recipe from 21,510 to 3,078 actions. It would still
require closing numeric mutation and qualifying edited output.

Fresh call-chain observations place the source finite-check chain at 35.574% Ir
and the retained finite-check chain at 5.954%. These chains include shared calls;
their percentages cannot be added or interpreted as predicted elapsed savings.
The complete WorkFrame and ParserFrame exclusive counts remain unchanged in the
two STL profiles. The source scalar execution owner is therefore a stronger
immediate target than deleting only the final numeric pass.

Main's separate OBJ/glTF inspection also retains large completion/dispatch costs:
grid OBJ's ParserFrame completion source row is 9.73% of whole-worker Ir, and
glTF-32 contains repeated parser entry, replenishment and binding. Source text
planning/building invokes a general returned-prefix operation per text unit;
for ASCII this is usually once per byte. Token containers still allocate per
line. These facts justify investigating the execution owner rather than assuming
that JSON syntax or scalar arithmetic itself dominates.

The next bounded experiment specializes immutable empty/singleton WorkLayout
mapping facts in the existing numeric kernel. It retains policy, arithmetic,
ownership, exact-prefix and physical-work semantics. Independent source review
finds no invariant counterexample; 1,074 Operation tests, 128 doctests, strict
Operation Clippy and workspace format checks pass. Its performance and integrated
qualification are pending. No second kernel or generic proof framework is added.

## Evidence Locations

Fresh instruction artifacts are under
`target/review/convergence/semantic-integer-attribution/`; the main summary is
`semantic-integer-attribution-main-summary-1.json`. Heap originals are under
`target/assimp-comparison/convergence-linux-semantic-integer-heap/` and the main
summary is `target/review/convergence/semantic-integer-heap-main-summary-1.json`.

Private independent receipts under `.git/baozi-review/performance-convergence/`
are `semantic-integer-instruction-review-1.md`, `semantic-integer-heap-review-1.md`,
`semantic-integer-stl-numeric-owner-1.md` and
`work-layout-arity-design-review-1.md`. Ordinary Windows/Linux qualification and
every original release target remain open.
