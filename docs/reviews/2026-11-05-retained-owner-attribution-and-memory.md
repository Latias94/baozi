# Retained Owner Migration: Instruction and Heap Checkpoint

The cumulative owner migration from `308e9ab` to `05b7d9b` reduces whole-worker
Linux instructions by 0.45% to 7.14% in six retained cases. It has identical
reported allocation calls and rounded heap peaks in the six separately measured
heap cases. Current-source ordinary timing and whole-child memory qualification
remain incomplete: the finite Windows comparison completes only its control
campaign before external build/test activity prevents the candidate from starting.
Performance keep and release remain HOLD.

This checkpoint covers typed-table producer borrowing, obsolete parser removal,
typed-storage sharing, connected retained work, retained backing and execution
facade retirement together. It cannot attribute the cumulative change to any one
of those steps. The final source's correctness gates are recorded in
[execution facade retirement](2026-11-05-execution-facade-retirement.md).

## Immutable Workers

| Identity | Control | Candidate |
| --- | --- | --- |
| Runtime revision | `308e9ab890b9e490c3f2bf3a444ca155d9244cd5` | `05b7d9b8dd2bf4158319e202ba68963cfb64e01d` |
| Source archive SHA-256 | `e581f1278a2bad1a6b8d9fdd25f99c36f392ce5b1f1397b064b3346800614a46` | `4c1170f3e41a09035ac5fdc5220b95b33cda7df6fc86113a2eb8630d1880f30e` |
| Windows executable SHA-256 | `b3eb661bb9e99c4b18f53734d67b0d814648b571d3c2d726aa1369b9b0b6cc71` | `af5c906a9c503c0c7f97f06f8c1bc72397f5c7876756bcca5c8fe9ee9f2c3eb2` |
| Linux symbol executable SHA-256 | `71d9470f3655d81af68c40d75cbcfe06f712ff790227910022f547da36c9d2d1` | `7f266268c4e3c06400d61c993e2e709d3a1d88a6e4bc83e0245d47362461e1e7` |

Both Windows workers use Rust 1.95.0, generic x86-64 MSVC, opt3/debug0,
dynamic CRT, System allocation and no allocation metrics. The fresh candidate
build finishes successfully in actual exec99858, retaining its source archive,
before/after input maps, compiler artifacts and Import dep-info. Its PE stack
reserve/commit remains 1,048,576/4,096 bytes. Actual exec49379 passes the original
110-lane functional preflight with zero warmups and one pair. Those functional
observations are not retained ordinary timing or memory samples.

The GNU workers use the same Rust version and generic opt3/System policy with
debug1 symbols, no worker allocation metrics and external Callgrind/heaptrack.
The candidate is built from its committed archive using the existing Linux target
cache. Actual exec79302 completes the build and six profiles with unchanged source
inputs. Symbolized and Windows workers have separate binary identities and scopes.

Independent review reproduces both committed archives and the exact GNU input
maps. All 1,165 qualified candidate inputs match its archive and Windows build
maps. The candidate's remaining archive/map differences are seventeen
documentation files, reconciled as newline-only. The historical Windows control
has 56 raw archive/map differences: the current review directly reconciles 45
but cannot reproduce eleven original mixed-newline byte sequences, including
four Cargo manifests and seven documentation files. No Rust source file differs.
The [earlier metadata comparison](2026-11-05-parse-metadata-borrowing-performance.md)
and its retained raw-review receipt record exact before/after/then-observed hash
matches and canonical equivalence for those files. That historical attestation
remains distinct from the present review's direct-verification limit; it is not
replaced by a blanket claim of byte equality. Original paths and hashes remain in
the independent receipt. These Windows limitations do not weaken the separately
reconstructed GNU archive associations or qualify a missing ordinary comparison.

## Incomplete Ordinary Windows Sequence

Before measurement, the finite decision fixes C0, N0, N1, C1, each using the
original fourteen cases, 110 lanes, three warmup pairs and twenty retained AB/BA
pairs. A 20-second named-tool quiet window is required before each campaign, with
a 180-second start limit. Approximately two-second process snapshots continue
during each campaign. The source, tasks, fields, normal policies, precision,
allowances, allocation policy and timing boundaries do not change.

The first launch reaches its start limit without starting C0. After those
external processes end, a second prospective decision allows one rescheduled
start with the same conditions and no measured outcomes to select. It starts C0;
that control completes all 110 lanes with 5,060 native children and no command
failure. It records 41 named-tool overlap snapshots. N0 then reaches the same
start limit because external Cargo/tests remain active. Actual collector
exec24744 terminates at that point; N0, N1 and C1 have no ordinary samples.

The interrupted sequence is retained rather than filled with preflight data or
historical candidate medians. There is no current candidate-versus-control native
contrast, complete confirmation, or candidate whole-child peak-memory conclusion.
C0's load record also precludes calling that campaign clean. The monitor covers
named processes at sampled instants, not every source of host load.

Decisions and both unstarted records are retained under
`.git/baozi-review/performance-convergence/retained-owner-windows-decision-{1,2}.json`
and `target/review/convergence/retained-owner-{C0,N0}-not-started-*.json`.
The successful control receipt is `retained-owner-C0-command-1.json`.

## Separate Instruction Attribution

Each candidate profile is a pipeline/memory task with the unchanged case manifest.
Controls are the retained earlier `308e9ab` profiles, not fresh simultaneous
control runs. Callgrind counts the whole worker, including setup, checking,
serialization and teardown outside the endpoint clock. It does not measure native
CPU time or establish Windows causation.

| Case | Control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| Shared glTF 1 | 6,071,746 | 5,928,267 | -2.36% |
| Shared glTF 32 | 30,277,355 | 29,687,737 | -1.95% |
| F64 PLY neighbors | 2,505,293 | 2,494,023 | -0.45% |
| Repeated STL 1024 | 27,370,260 | 25,416,724 | -7.14% |
| Repeated OBJ 16384 | 843,396,527 | 819,153,676 | -2.87% |
| Grid OBJ 128 | 2,998,843,880 | 2,937,988,768 | -2.03% |

All six candidate raw `summary`, `totals` and stderr `Collected` counts agree.
Grid retains Valgrind's `brk segment overflow` warning. Raw profiles, stderr,
observations and annotations remain under
`target/review/convergence/retained-owner-attribution/`.
Inclusive parents must not be added to their exclusive children, and line-level
annotation rows alone do not establish the share of an endpoint's native cost.

## Separate Heap Observations

Actual exec75284 completes eighteen fresh heaptrack observations: six cases each
for `308e9ab`, `05b7d9b` and the retained symbolized Assimp F32/F64 workers. Each
task is pipeline/memory with its original fields and normal policy. Instrumented
elapsed time and heaptrack-overhead RSS are not ordinary measurements.

Values below are extracted from original analysis and stderr logs. K/M heap peaks
are rounded decimal units. Allocation calls cover the whole child, including
setup, preload, validation, serialization and teardown. Prior interception
calibration covers named allocator paths rather than every possible allocator.

| Case | Both Baozi calls | Assimp calls | Both Baozi temporary A/S | Assimp temporary A/S | Both Baozi peak | Assimp peak |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Repeated OBJ 16384 | 33,389 | 82,154 | 10/9 | 9/8 | 12.18M | 19.91M |
| Grid OBJ 128 | 99,425 | 360,697 | 33,262/33,261 | 10/9 | 28.82M | 28.69M |
| Shared glTF 32 | 1,116 | 845 | 14/13 | 16/15 | 416.52K | 174.36K |
| Repeated STL 1024 | 488 | 1,186 | 8/7 | 10/9 | 456.54K | 317.09K |
| F64 PLY neighbors | 518 | 187 | 10/9 | 17/14 | 135.96K | 2.18M |
| PLY points 1024 | 482 | 4,281 | 9/8 | 1,041/17 | 188.38K | 2.23M |

`A/S` retains both the analysis and stderr temporary-allocation counts. Their
differences, including Assimp points' 1,041/17, are not reconciled by choosing one
count. All reported calls and rounded peaks agree between the two Baozi sources.
Exit-live reports are 544B for each Baozi observation and 4.10K for Assimp; these
whole-worker exit observations do not by themselves identify a product leak.
There is no observed heap-count or rounded-peak benefit from this cumulative cut.

All eighteen traces, original stdout/stderr and analysis logs remain in
`target/assimp-comparison/convergence-linux-retained-owner-heap/`.
Independent bounded review reconciles all eighteen trace identities, 54 original
logs, actual task/geometry/reported-work data, raw allocation-event counts and
both GNU source associations. It also checks all twelve instruction-profile
totals and original observations, both committed archives, the functional
preflight and incomplete Windows sequence records. It does not audit every C0
child log or re-execute a compiler/profiler. No blocking numerical mismatch is
found; the historical Windows source and grid-warning limits above remain.
The receipt is
`.git/baozi-review/performance-convergence/retained-owner-attribution-and-heap-review-1.md`.

## Consequences for the Next Owner

The instruction savings are useful but modest, while the retained heap structure
is unchanged. Removing wrappers alone has not demonstrated convergence of the
historical small-task setup gaps. The next coherent prerequisite joins startup's
four account shares, publication opening and canonical/evidence table creation
through the existing unique numeric/storage views. It must retain deferred current
policy, pending consumption, error precedence, exact completed physical work and
failure cleanup. Cold/shared numeric users still prevent deleting LocalWork.

Fixed/dynamic backing, allocation-free finite startup, shared promotion, exact
administrative accounting and indexed selection remain required by the
[convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
Current candidate Windows timing/whole-child memory, complete ordinary Linux
comparison and final integrated qualification remain open. No broad performance
acceptance or release readiness follows from this checkpoint.
