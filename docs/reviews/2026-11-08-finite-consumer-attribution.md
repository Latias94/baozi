# Finite consumer diagnostic attribution

Release, native convergence and local-main runtime integration remain **HOLD**.
The finite Core/Postprocess ownership integration has no instruction reduction
on the six measured whole-worker tasks. This diagnostic checkpoint accompanies
the [Windows ordinary results](2026-11-08-finite-consumer-windows.md). Ordinary
wall time, whole-child RSS/Windows memory and tracked heap have separate scopes.

## Fixed source and instruction scope

Control is `00b54f46ec22795377dcbc9df0f931cd93368f25`; candidate is
`b76fbc0bfdb83739fdfef8425702c7b5c5d4472a`. The original corpus and collector
remain unchanged. Both GNU workers are fresh matched Rust1.95 opt3/debug1,
System allocator, generic CPU builds with empty consumer features and no
allocation instrumentation or CPU/LTO flag overrides. Source archives match
qualified Final27/control and Final29/source172 candidate inputs. All builds
remain in this repository's existing target.

| Symbol worker | SHA-256 |
| --- | --- |
| Baozi control | `448a0ddb8a3f9d4d269c7baa3f2a3360da0011785b5022ff68973e5896a23761` |
| Baozi candidate | `e8fa229f90e334e506a7c2141ced9d2980457ed81c7db82125c8009f95ca3993` |

Twelve fresh Callgrind executions measure six original pipeline/memory tasks,
one process per Baozi role/case. All succeed; complete paired non-clock JSON
matches after removing only `stage_ns`, preserving numeric spelling, numeric
types, work and output. Twenty-four retained annotations cover inclusive and
Self views. Only instruction events are recorded. These are whole-worker
counts including setup, validation, observation, serialization and teardown;
they are neither stage clocks nor an Assimp instruction comparison.

| Case | Control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| Shared glTF1 | 5,857,340 | 5,860,998 | +0.06245156% |
| Shared glTF32 | 25,526,610 | 25,530,543 | +0.01540745% |
| F64 PLY neighbors | 2,494,605 | 2,499,456 | +0.19445964% |
| Repeated binary STL1024 | 12,166,997 | 12,172,171 | +0.04252487% |
| Repeated OBJ16384 | 718,991,859 | 723,375,934 | +0.60975308% |
| Grid OBJ128 | 2,269,688,171 | 2,269,692,865 | +0.00020681% |

Changes are `100*(candidate-control)/control`; every adverse sign remains.
Both grid runs retain `brk segment overflow in thread #1`, at `0x5404000`
and `0x5417000`, plus manual-limitations and suppressed-further-instances
notices. No source-retention or native-speed decision follows from this batch.

Independent actual evidence review verifies twelve raw profile hashes,
summary/footer/Self/annotation/stderr totals, all72 command streams, matched
ELF/build/source identities and all observations. No arithmetic discrepancy
is found. Do not sum overlapping inclusive costs or duplicate inline contexts.

## Bounded allocator calibration

Fresh Rust System and C++ controls exercise malloc/realloc, alignment, threads,
operator new and a shared dependency. Exact actual sizes and selected collapsed
allocation-stack weights are independently verified:

| Function | Allocation sizes, bytes | Expected/observed allocation weight |
| --- | --- | ---: |
| Rust malloc/realloc | 1,048,576 to2,097,152 | 2/2 |
| Rust aligned | 1,048,576, alignment128 | 1/1 |
| Rust thread | 524,288 | 1/1 |
| C++ malloc/realloc | 1,048,576 to2,097,152 | 2/2 |
| C++ operator new | 524,288 | 1/1 |
| C++ aligned new | 1,048,576, alignment128 | 1/1 |
| C++ shared dependency/thread | 786,432 | 1/1 |

Attempt1 remains failed: default merged analysis hides the Rust thread's
function label, so its required stack assertion fails and the C++ profile
does not start. Offline replay of the same Rust trace with explicit
`--merge-backtraces 0` restores all three expected weights2/1/1. Global25
allocations and rounded2.17M peak display stay unchanged. Disassembly retains
the thread's actual malloc524288. This analysis correction does not relabel
the failed attempt.

Fresh attempt2 passes all seven function paths with unmerged analysis. Control
sources, build flags and expected counts remain unchanged; both executable
files and the dependency ELF are byte-identical across attempts. Native and
profiled JSON are byte-identical, with checksums444/Rust and778/C++. Independent
review reproduces all seven selected collapsed-stack exports and verifies
all44 command streams across both attempts. Printed global footers/histograms
are not assumed filtered to the selected function.

This qualifies these finite paths in this GNU environment. It establishes
neither universal allocator/dependency coverage nor exact general heap,
stage heap, RSS or native speed. Actual Baozi/Assimp worker stack and sanity
qualification remains separate from these controls.

## Actual heap worker collection

All18 fresh heaptrack attempts complete: six original pipeline/memory cases
times control, candidate and precision-matched Assimp. All36 worker/analysis
commands succeed with explicit unmerged analysis; all six complete Baozi
non-clock JSON pairs match. These executions follow the terminal ordinary
attempts and passed bounded calibration. Each row is one whole-worker trace,
including setup, observation, serialization and teardown.

Assimp symbols use the same pinned6.0.5 source, static GNU15.2.0
`-O3 -g1 -DNDEBUG`, generic CPU/platform allocation and separate F32/F64
builds. All1,652 source and seven consumer hashes remain unchanged across
builds; F32 ten and F64 seven semantic CTests pass. These debug1 symbols are
distinct from ordinary debug0 workers and are ineligible for native timing.

| Assimp symbol worker | SHA-256 |
| --- | --- |
| F32 | `9ac136f1856a419118ec904f2084d5499ca026fce41047b802583ed32dc00f9e` |
| F64, PLY | `f0d45a71409f9870e8329ebdc979d60a4834e1df22f48c1531adc9a3422470ac` |

The next table retains every role and adverse comparison. Peaks are the
tool's rounded display strings, not exact byte counts or RSS. Equal control/
candidate displays do not prove equal underlying peaks. The candidate has
more tracked allocation calls than Assimp for both glTF cases and F64 PLY;
its displayed peak is higher for both glTF cases, STL and grid OBJ.

| Case | Control calls | Candidate calls | Assimp calls | Control peak | Candidate peak | Assimp peak |
| --- | ---: | ---: | ---: | --- | --- | --- |
| Shared glTF1 | 670 | 645 | 355 | 174.50K | 174.50K | 155.19K |
| Shared glTF32 | 1,125 | 1,100 | 825 | 389.93K | 389.93K | 174.36K |
| F64 PLY neighbors | 517 | 492 | 175 | 136.00K | 136.00K | 2.18M |
| Repeated binary STL1024 | 487 | 462 | 1,174 | 456.58K | 456.58K | 317.09K |
| Repeated OBJ16384 | 17,001 | 16,970 | 82,142 | 12.18M | 12.18M | 19.91M |
| Grid OBJ128 | 33,372 | 33,347 | 360,685 | 28.82M | 28.82M | 28.69M |

Candidate/control tracked allocation changes are -25 calls in five tasks
and -31 in repeated OBJ. They establish neither zero-allocation execution
nor a native-time gain. The [complete18-row CSV](2026-11-08-finite-consumer-heap.csv)
also retains temporary allocations, source/binary identity and exact count
changes. Per-row stack qualification notes reflect collection time; they
explicitly require independent actual stack/sanity review. That review is
now closed with GO for all18 finite tracked-heap observations. The original
collection notes remain unchanged and are superseded by that separate review.
Positive counts and completed collection alone did not establish this result.

The reviewer rehashes all traces/binaries and72 streams, reproduces all18
retained analyses byte-for-byte, and checks full unmerged collapsed stacks.
For every worker, all stack weights sum exactly to raw allocation events,
worker allocation-call stderr, analyzer footer and receipt count. Actual
Rust ELF allocation backing reaches malloc/posix_memalign/realloc/calloc;
all six Assimp tasks show native importer/library/memory-IO paths. This
qualifies these observations beyond the earlier bounded controls.

One Baozi allocation and17 Assimp allocations per task contain unresolved
loader/filesystem/runtime frames; they remain in totals. Exact internal
attribution is unavailable. Worker stderr and analyzer temporary-allocation
classifications differ, mostly by one and by14 versus17 for Assimp F64 PLY.
That classification difference remains unresolved; the CSV retains the
analyzer's temporary counts. Neither temporary count substitutes for actual
allocation calls or proves coverage. Complete recorded-event accounting
cannot detect allocations missing from a trace, prove universal interception
or turn these results into exact application heap/RSS/native-time evidence.

## Source opportunities and limits

One disjoint classification of the retained candidate Self annotations assigns
38.83% to65.70% of whole-worker Ir to Operation-related namespace/source labels
across these six tasks. Each physical Self row is counted once; the groups sum
exactly to the original total. Inlined callbacks and business work can inherit
these labels. The percentages identify source to inspect and do not measure
removable framework overhead or native-time proportions.

The text planning pass already borrows its actual singleton accounting row.
The emission pass still calls the generic returned-prefix runner per unit.
Grid OBJ128 has1,830,715 `TextEmission::read` calls and the same number of
`ParserFrame::complete` calls in that caller block, equal to its ASCII input
byte count. A bounded next design can reuse the existing private row loan
between control frontiers, retaining immediate per-unit numeric commits,
reserved-copy checks, UTF-8 boundaries, raw4096-byte frontiers and direct-mode
action debt. Neither two-pass reservations nor numeric pending/limit checks
can be removed on the strength of successful attempted admission.

A separate glTF source exploration finds one existing child per binary
attribute stream, followed by per-sample generic `run` calls. Schema numeric
tokens separately bind/open children, while JSON syntax scanning already has
one enclosing scan child. These are different seams; a multi-lane accessor
loan needs a separate atomicity/API design. No such refactor is implemented
or performance accepted by this diagnostic checkpoint.

The native GNU CPU advertises SHA/SSE4.1 and sha2-0.11.1 has runtime hardware
dispatch; this is capability/source inspection, not a selected-backend trace.
Callgrind executes software SHA in these profiles. Its SHA percentages do not
establish native hashing cost or justify replacing the digest. Pinned Assimp
glTF/STL whole-buffer and OBJ initial-block facts remain as qualified in the
Windows report; multi-block streaming is not exercised by these OBJ fixtures.

## Retained evidence

Private immutable receipts and raw data remain under
`.git/baozi-review/performance-convergence` and `target/assimp-comparison`.
Both ordinary envelopes are terminal before diagnostic execution. Failed
calibration1, same-trace diagnosis, fresh calibration2 and all instruction
regressions remain separately retained.

| Evidence | SHA-256 |
| --- | --- |
| Baozi symbol association | `ccf594beae18caf89f70512dce23dad508063aa75703d6846f7cabe666852e51` |
| Instruction receipt | `15294efdc8178f8808e18d74aaf7595843d3f0620be8ffd4af6216cf8b3f7d45` |
| Passed bounded calibration2 | `4e819807b70f3f9e198516dea4fac2566dd629ea2d179f89a1950548194fa543` |
| Assimp symbol build receipt | `2cf4bfb9f1a5442a24c81959a595112bea486e284a45b67c24ec8a3240e53a32` |
| Actual18-worker heap collection | `c286336777d788e4f66cdb08c0bf59b21f1ebb8072f3ca5c35e4877a72cb407e` |
| Complete heap CSV (LF) | `835d5f395155d554b0f02ca8b2afbaa4f60293d97915707eed306f7ccf249004` |
| Independent actual heap review | `71b33c786db4e917df6d91e2ce77ea29c4b2ad904dc980406f40f0b62b62e8fc` |

No release or native convergence acceptance is implied. Actual finite heap
qualification is closed; GNU ordinary candidate intervals and final native
source/main disposition remain unresolved. The next private caller experiment
is specified in the reviewed
[text-emission plan](../plans/2026-11-08-0820-closed-text-emission-plan.md);
its fixed-candidate heap prerequisite is now satisfied.
