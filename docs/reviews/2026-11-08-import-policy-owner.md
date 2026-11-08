# Reuse the original import policy owner

The bounded six-task instruction predicate passes. All six whole-worker
instruction counts decrease, with glTF1 down 1.72953523% and F64 PLY down
3.97271493%. Complete paired non-clock output matches. Native convergence,
source retention, main/runtime integration and release remain **HOLD**.

This experiment follows the [policy-owner plan](../plans/2026-11-08-1240-import-policy-owner-plan.md)
and [tiny-STL fixed-cost attribution](2026-11-08-small-stl-fixed-cost-attribution.md).
Earlier [glTF native gaps](2026-11-08-gltf-field-layout-native.md), including
missing Windows N1, all GNU ordinary intervals and the monitor interruption,
remain independent limitations. Instruction selection does not repair them.

## Actual owner change

Candidate `bdc92b3fa5ca523a99920ff980b909ecfe6d61e9` changes eleven Rust
paths. Production changes are confined to Import `session.rs` and `store.rs`;
the other nine files migrate existing test callers/imports only. Its immediate
parent is precommit QA base `36d1e19f8702a2686e591cdab86e0423e1f34a2f`.
Control is `7b8c5f3d2559085967dcdfd4098130c77bd877ce`; intervening commits
add documentation rather than production changes.

Previously, entry constructed and validated a 58-dimension import policy
owner, then discarded it. Requirements and bootstrap each constructed it again.
Entry now retains the original successful `LedgerLimits<ImportDimension>`,
borrows it for checked requirements, and moves it into bootstrap. The two
reconstruction paths and their obsolete private error variant/Display/source
arms are deleted. No compatibility wrapper, cloned policy owner, request cache,
counter mirror, provider, public API or allocation instrumentation is added.

The final mapped 62-dimension `LedgerLimits<StoreDimensionKey>` remains a
genuine separate owner. Both missing-policy checks, checked aggregate work,
structural/protected budgets and exact partition validation remain. Startup
outcome policy reads the original owner's live-byte getter, which equals the
entry's original raw limit rather than the physical account's minimum clamp.

Initial typed policy validation still precedes physical account creation,
requirements, bootstrap and bound operation start. Surviving initialization
codes, typed source chains, shared-account quotas, selected-format tightening,
runtime failure evidence, publication and physical cleanup remain. The import
temporary now overlaps account creation and loses two independent construction
failure opportunities. Exact OOM occurrence, allocation timing and transient
peak memory are not assumed equivalent; separate evidence remains required.

## Correctness and source association

Source1 freezes 1,059 inputs and eleven changed files. Affected checks pass
488 Import tests, zero skips, 49 doctests (15 ordinary and 34 compile-fail),
strict all-target/all-feature Clippy and format. The original seven integrated
gates pass 3,959 workspace tests, 218 doctests, 138 independent consumer tests,
both strict Clippy gates and both formats. Six workspace and one consumer
existing manual skips remain; the workspace has one slow pass.

The existing aggregate-overflow fixture now asserts requirements rejection
before consuming the same real owner into direct bootstrap with independently
specified requirements. The new incomplete-owner fixture omits the final
DependencyAcquisitionWork dimension and requires both interfaces to return its
exact typed missing-policy failure before mapping. Existing attempted/consumed
partition, initialization/no-side-effects, publication and unwind tests remain.
No original test or stack bound is removed or weakened.

Independent source review and actual QA/source-commit review close with GO.
All 1,059 frozen/before/after/current/archive/committed input hashes and eleven
snapshots agree. All six explicit 1 MiB stack fixtures pass. Expected caught
panic stderr and repeated success tails are counted once. Five current user
note identities match their existing baseline. There are no failed numbered
QA attempts in this experiment; earlier glTF failures remain retained.

QA ran before the source-only commit, whose parent is the recorded QA HEAD.
Archive equality substantiates qualification of those committed bytes; no
postcommit rerun, general GNU runtime, wasm or memory-peak qualification is
claimed. Individual recorded command exits/logs are verified independently;
outer historical session terminal observations come from the coordinator.

## Matched instruction experiment

| Role | Source | GNU symbol-worker SHA-256 |
| --- | --- | --- |
| Control | `7b8c5f3d2559085967dcdfd4098130c77bd877ce` | `14c41c745814ee2116b55691d1b649e86f1120f2dfef5b22733bfa0048a14ef4` |
| Candidate | `bdc92b3fa5ca523a99920ff980b909ecfe6d61e9` | `bdd276067e86505b6cf45ac20c2bef917649b925dd3aae098d6cae2b85ddfb1f` |

Both workers retain equal Rust 1.95 compiler, argv, environment overrides,
opt3/debug1 profiles, empty features and generic CPU/System policy. Source
archives match qualified inputs. Source/builds stay in this repository and
its existing target, including GNU `/mnt/f`. Debug1 workers are ineligible
for ordinary native timing.

Twelve fresh workers run the same six original pipeline/memory tasks, with
twenty-four Self/inclusive annotations. Original corpus, fields, normals and
work bounds remain. All six complete non-clock pairs match after removing
only `stage_ns`, preserving numeric spelling/types, work, output and metadata.

| Case | Fresh control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| Shared glTF1 | 5,732,754 | 5,633,604 | -1.72953523% |
| Shared glTF32 | 25,392,417 | 25,293,349 | -0.39014797% |
| F64 PLY neighbors | 2,500,708 | 2,401,362 | -3.97271493% |
| Repeated binary STL1024 | 12,218,397 | 12,119,318 | -0.81090015% |
| Repeated OBJ16384 | 711,820,613 | 711,721,271 | -0.01395604% |
| Grid OBJ128 | 2,107,578,055 | 2,107,479,306 | -0.00468543% |

Selection uses exact integers: one glTF1/glTF32 must satisfy
`100*candidate <= 99*control`, and all six must satisfy
`100*candidate <= 101*control`. glTF1 meets the first condition and every
case meets the second; completion/equality remains independently required.
Descriptive percentages do not determine the 1% boundary. The
[complete twelve-row CSV](2026-11-08-import-policy-owner-instructions.csv)
retains both roles, exact identities/counts/deltas, equality and integer flags.

Both grid profiles retain `brk segment overflow in thread #1` at `0x5419000`,
manual-limitations and suppressed-further-instances notices. Whole-worker Ir
includes setup, validation, observation, serialization and teardown. It gives
no native time, stage time, Assimp Ir, RSS or heap result.

## Separate tiny-STL attribution

An additional fresh candidate run of the original 134-byte STL1 pipeline /
memory task records 2,041,865 whole-worker Ir. The earlier control locating
run recorded 2,140,762, a descriptive difference of -4.61971018%. This is not
a fresh paired selection or native-time comparison and adds no selection case.

The raw candidate call graph has exactly one incoming `import_ledger_limits`
call, from `session::import_bytes`, with 50,958 inclusive Ir. Earlier control
had that call plus requirements 49,480 and bootstrap 49,280 Ir, three calls
totaling 149,718. The two demonstrated reconstruction calls disappear; the
genuine Store-keyed policy construction remains. Inclusive values are not
summed with overlapping inline views or promoted to predicted native savings.

Build/profile wrapper output also reports failure to start the systemd user
session for frankorz. Coordinator tool exits are zero and recorded compiler /
worker commands succeed. This host warning remains distinct from Rust failure
or qualification of the user's service session; no native inference follows.

## Next disposition

Independent actual six-task/tiny-STL evidence review closes with GO. All
thirteen raw profiles, twenty-six annotations, seventy-eight streams, literal
output comparisons, exact integer selection and constructor call arcs agree.
Unknown-symbol labels remain in totals; matching grid warning addresses do
not establish allocation equivalence. Supervisor exits and wrapper warnings
remain coordinator observations rather than independently retained exit proof.

A static review also finds that the tiny runner's terminal barrier still
names the previous glTF campaigns. Future execution is corrected to require
both current policy-owner ordinary terminal decisions; its one-line amendment
closes with STATIC GO. The exact historical executed script is retained.
Actual tiny collection followed the six-task profile and preceded the ordinary
builds, with no own-heavy-task overlap. No historical trace is rerun or relabeled;
the future guard correction does not create a new performance observation.

Ordinary debug0 workers and their unchanged Windows/GNU full-matrix protocol
are being qualified separately. No new native
or heap result is available here. Original 14-case/110-lane three-warmup /
twenty-retained-pair comparisons and separate representative calibrated heap
attribution remain required, preserving all signs, failures, memory limits,
missing campaigns and source/ownership obligations.

The later [native and tracked-heap checkpoint](2026-11-08-import-policy-owner-native-heap.md)
supersedes that pending collection status. Windows completes C0/N0/N1/C1;
GNU completes C0/N0 only and terminates N1/C1 not started. Both Windows
candidates fail the pipeline target and share thirty endpoints at least 10x.
All eighteen heap workers complete with 56 fewer candidate allocation calls
per task and equal rounded control/candidate peak displays. Independent actual
Windows/GNU evidence reviews close with GO; heap stack/interception review closes
with bounded GO, locating the reduction in policy-construction stacks while
retaining unknown frames and temporary-classification differences. No source-speed
acceptance, main integration or release follows.
