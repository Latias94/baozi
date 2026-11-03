# Ordinary Storage Binding: Matched Linux Instruction Costs

2026-11-03. Independent offline audit of six completed Callgrind profiles: control `37e800541be6cae31585e40c01828aa64b5809ef` (runtime `67b5ca609470268b124fd1915741ee81105c4a24`) versus candidate `48d6f17e2f0829bdd5d50c8d7c48cea72ce343a1`.


Tracked export of the independent offline review, SHA-256
`d18390f243b2de24d2e8611c8cbf6b2e12c36e46bd0a4b70e033858f5afd0c79`.
It supports the [ordinary performance HOLD](../reviews/2026-11-03-binary-stl-performance-hold.md)
and the [performance convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
Local reviewer files named below live under `.git/baozi-review/performance-convergence/`.
The companion summary has SHA-256
`5fcc616daa57d9b75eb75a9159f913d79ba901534c12d289e27f055cc6642a41`;
the reconstruction program has SHA-256
`1ed3bc91df28b9e27dd05430247ca35d11d76a6c0c4fcab16644c848470f21cc`.
They identify retained local evidence, not files distributed with this Markdown export.

## Findings and disposition

**The profiles support a concrete Linux execution-cost investigation, not performance keep. Windows performance acceptance remains HOLD.** All six raw totals reconcile with every exclusive flat row, both retained annotation variants, stderr's collected Ir, and source/binary/task receipts. No integrity P1/P2 was found. The three Linux whole-worker changes are repeated OBJ **+34,506,586 Ir (+3.5330%)**, grid **+112,919,612 (+3.6201%)**, STL **-291,166 (-0.8436%)**.

**P2 — ordinary-loan construction and checkpoint wrappers are distinct repeated costs.** The new legacy adapter constructs storage loans 147,509 times for repeated OBJ and 444,732 times for grid, charging **13,275,810 / 40,025,880 exclusive Ir** in two construction functions. Those functions perform no poll. Separately, the corresponding normal scope checkpoint routes grow **93 -> 104 Ir per entry**, with the same underlying poll count. These are useful owner-local targets; they do not authorize removing issuer checks, terminal/state handling, allocation frontiers, cancellation, error precedence or destructor containment. Any correction needs the separately qualified semantic contracts and fresh ordinary evidence.

**P2 — gross owner growth is not a net-cost explanation.** New `ExclusiveStorage::reserve` grows 15.85M/47.70M Ir, but replaces old lookup/reserve owners. A matched six-function replacement bundle grows only **297,733 / 892,485 Ir**. Likewise, parser completion wrappers redistribute deeply inlined work, and memcpy's extra 3.52M/10.56M Ir is concentrated in specific adapter callers. Do not label the gross comparisons as duplicated catalogue work or all memcpy as extra geometry payload. Exact local routes and limits follow.

The Windows repeated-OBJ/grid import and pipeline regressions, all named endpoint/memory/tail costs, and earlier R4/R5/R7/R8 evidence remain as frozen in `binary-stl-ordinary-measurement-review.md`. This review neither clears them nor estimates Windows time from Linux Ir. A small Linux STL whole-worker reduction does not establish a native STL import win.

## Complete reconstruction and scope

The reviewer-owned offline parser `binary-stl-linux-attribution-audit.py` reads the original compressed Callgrind name tables, inline file switches (`fi`/`fe`), exclusive cost lines and `calls` edge costs. Edge-inclusive costs are excluded from flat totals. It independently reconstructs all function and source-file/function flat costs, then matches every such row against both `none.txt` and the starred self rows in `both.txt`. It also reconstructs every aggregated caller/callee call count and inclusive edge cost, matching both the incoming and outgoing annotations exactly. No selected-hotspot sum is used as the whole total.

| Whole-worker pipeline | Control Ir | Candidate Ir | Difference Ir / % |
| --- | ---: | ---: | ---: |
| obj-repeated-16384 | 976,698,068 | 1,011,204,654 | +34,506,586 / +3.5330% |
| obj-grid-128 | 3,119,221,307 | 3,232,140,919 | +112,919,612 / +3.6201% |
| stl-repeated-1024 | 34,515,977 | 34,224,811 | -291,166 / -0.8436% |

All raw `summary`, `totals`, the complete self-cost sums, both annotation totals and profiler stderr agree exactly. Profiles record only `Ir`, line positions, Callgrind/Valgrind 3.26.0 and program-termination collection. All six receipt return codes are 0. Raw stdout matches each receipt's observation; input hashes, analytic facts, widths, fields, IO, Smooth/preserve policy, import work and output bytes match across versions. The command is the same public `pipeline` task. Captured `stage_ns` values are instrumented elapsed values and are not used for performance conclusions.

The companion `binary-stl-linux-attribution-summary.json` contains all input hashes, full profile/annotation reconciliation counts, an exhaustive nonoverlapping function-prefix partition, leading exclusive deltas, shallow call edges, reservation net and memcpy caller deltas. Its validation-error list is empty. Edge-inclusive amounts are explicitly labeled and are never added to exclusive buckets. Function-prefix ownership includes inlined instructions charged to that function even when their source file is Rust core; the residual `other` bucket includes libc, allocator, loader, traits/drop glue and anonymous symbols. This partition is exact bookkeeping, not a complete semantic module classification.

| Exact function prefix, exclusive Ir delta | Repeated OBJ | Grid | STL |
| --- | ---: | ---: | ---: |
| `baozi_operation` | -5,340,442 | -54,586,827 | +53,992 |
| `baozi_import` | +36,031,295 | +155,367,111 | -130,536 |
| `baozi_format_obj` | -48,418 | +403,918 | +344 |
| `baozi_format_stl` | -266 | -266 | -190,528 |
| `baozi_core` | +48,905 | +293,322 | +2,995 |
| `baozi_postprocess` | +12 | +6 | +6 |
| `baozi_asset_consumer` | +0 | +0 | +0 |
| `other` | +3,815,500 | +11,442,348 | -27,439 |

Thus the import-function prefix is where much of the extra exclusive work is charged, while the operation prefix falls overall. Calling every new concrete/ledger symbol an independent addition would reverse that net picture. Other buckets and their exact totals remain in the JSON; all prefix buckets sum to the whole-worker result with no overlap or omission.

## Binding versus common checkpoint: actual shallow routes

Candidate source locations below refer to the frozen candidate archive, not an assumed later working tree.

- `crates/baozi-import/src/scope.rs:792` constructs `ParseStorage` after `preflight_active`, obtains `storage_identity`, and calls `ImportStoreExecution::ordinary`. Legacy `MeteredVec::try_reserve` / `try_push` at `:3643` / `:3651` call this on each invocation; string reserve/append also use the shared owner.
- `crates/baozi-import/src/store/execution.rs:227` joins sibling storage/control through `StoreExecutionStorage::ordinary`. `crates/baozi-operation/src/concrete_store/execution.rs:470` checks the actual sibling identity and constructs the loan; `retained/operation/execution.rs:424` borrows the existing storage/poll/debt/state fields. Construction itself does not checkpoint.
- Candidate `scope/storage.rs:93` polls through `ImportOrdinaryStorage::poll` (`store/execution.rs:44`). That checkpoint calls `check_execution_state` and `poll_execution` at `retained/operation/execution.rs:77` / `:89`; ordinary reserve and resize use the same actual state/control owner. The old `ExecutionControl::checkpoint` was at control `retained/operation/execution.rs:285`; it did not perform the newly common execution-state settlement.

| Shallow measured component | Repeated OBJ | Grid |
| --- | ---: | ---: |
| New `ParseExecution::storage` entries | 147,509 | 444,732 |
| Its exclusive Ir, 60/entry | 8,850,540 | 26,683,920 |
| `storage -> ImportStoreExecution::ordinary` calls | 147,509 | 444,732 |
| `ordinary` exclusive Ir, 30/entry, no outgoing calls | 4,425,270 | 13,341,960 |
| Two binding self costs combined, 90/entry | 13,275,810 | 40,025,880 |
| Old scope poll entries | 346,125 | 1,113,293 |
| New `ParseExecution::poll_active` entries | 165,763 | 569,620 |
| New `ParseStorage::poll` entries | 180,362 | 543,673 |
| Old/new scope-route checkpoint Ir per entry | 93 -> 104 | 93 -> 104 |
| Local scope-route increment, 11/entry | 3,807,375 | 12,246,223 |
| All `poll_operation_control` calls, control = candidate | 384,751 | 1,216,850 |
| Its exclusive Ir, control = candidate, 29/call | 11,157,779 | 35,288,650 |

The old scope route is self 32 (`ParseExecution::poll_active`) + self 32 (`StoreExecutionControl::checkpoint`) + poll primitive 29 = **93**. The new conventional route is self 33 + self 39 (`ImportStoreExecution::poll`) + self 3 (`settle_caught_unwind`) + poll primitive 29 = **104**. The new storage route is self 32 (`ParseStorage::poll`) + self 40 (`ImportOrdinaryStorage::poll`) + settlement 3 + poll primitive 29 = **104**. These named paths' outgoing edges have no additional callees in the captured success execution. The new two entry counts sum exactly to the old scope count. The primitive's whole-worker count and Ir remain unchanged, including non-scope callers.

This isolates observed constructor work from observed checkpoint work on these Linux success paths. It does not isolate the source-level cost of each inlined conditional, specify failure-path cost, or establish the marginal saving from deleting one check. The 90-Ir construction includes terminal/reentrancy preflight and result/view handling, not merely pointer assignment. Its direct callers are `ParseExecution::with_work` (81,937 / 296,977), legacy vector `try_push` (49,152 / 98,305), `try_reserve` (16,397 / 32,781), and smaller parse/context routes. These are actual executed edges, not a static callback guess.

`scope::preflight_terminal` itself is unchanged at **3,424,828 / 19,006,570 Ir**. The new storage preflight has partly inlined checks charged to its containing methods, so this unchanged standalone function does not mean total terminal checking is unchanged. `StoreExecutionState::settle_caught_unwind` grows from **312 -> 1,246,887 Ir** (104 -> 415,629 calls) and **303 -> 3,939,159** (101 -> 1,313,053 calls). It is **3 Ir per captured entry**, with no outgoing calls. Its calls arise from conventional/storage polls, ordinary reserve/resize and other checkpoint owners. These state costs overlap the scope-route decomposition above and must not be added to it again. `check_execution_state` is inlined; absence of its own row does not imply it did not execute.

Construction costs alone equal about 38.47%/35.45% of the *net whole-worker increase*. That arithmetic does not make them the sole cause or a guaranteed removable fraction: new helper bodies replace old ones, and compensating decreases elsewhere matter. A useful next design is to shorten repeated legacy loan construction at the real disjoint owner while retaining one growth implementation and every required poll/error/Drop boundary. The data does not justify a second backend, counter restart, weakened physical budget, or removing state settlement.

## Reservation comparisons: account for migrated owners

The same 32,878 / 98,965 ordinary reservation entries migrate from old `ExecutionStorage::reserve` to new `ExecutionOrdinaryStorage::reserve -> ExclusiveStorage::reserve`. The actual `Ledger::reserve_core_in` exclusive cost is unchanged at **4,642,002 / 13,957,590 Ir**. Old dimension lookup edges disappear on exactly that route. New `ExclusiveStorage::reserve` contains the catalogue binary search and phase scan visible at candidate `ledger/exclusive.rs:105`; it is not evidence that another independent payload reservation was added.

| Exclusive symbol delta, candidate minus control | Repeated OBJ Ir | Grid Ir |
| --- | ---: | ---: |
| `ExclusiveStorage::reserve` | +15,847,121 | +47,701,250 |
| Old `Ledger::bound_dimension_position` | -10,981,117 | -33,054,310 |
| Old `ExecutionStorage::reserve` | -6,082,490 | -18,308,645 |
| New `ExecutionOrdinaryStorage::reserve` | +2,169,949 | +6,531,690 |
| `StoreExecutionStorage::reserve` | -3,154,382 | -9,498,840 |
| New `StoreOrdinaryStorage::reserve` | +2,498,652 | +7,521,340 |
| **Net of these six disjoint self buckets** | **+297,733** | **+892,485** |

This bundle is an explicitly bounded replacement comparison, not the complete allocator/storage cost. Its new checkpoint descendants are separately attributed, and other import wrappers are outside the bundle. The underlying phase lookup row is unchanged as well. Consequently the large new `cmp` / `slice::cmp` source-file charges under `ExclusiveStorage::reserve` must be read alongside vanished old lookup/phase/wrapper charges; they do not independently explain the 34.5M/112.9M whole-worker increase. Do not double-count the common reserve kernel or edge-inclusive amounts.

## Completion wrappers and memcpy: recorded callers, not a payload hypothesis

Three disjoint self rows, `ImportParserWorkScope::complete`, `StoreParserWorkScope::complete` and `StoreParserWorkScope::finish_exact`, combine **4,785,550 -> 13,782,997 Ir (+8,997,447)** for repeated OBJ and **63,232,546 -> 114,241,014 (+51,008,468)** for grid. For example, grid's import wrapper grows 73,228,600 Ir while the concrete complete wrapper falls 35,956,530 Ir. These rows are charged to the parser completion/finish projection route and deserve investigation alongside binding. Their downstream numeric kernel costs are excluded from these figures.

The import complete wrapper's source body is unchanged apart from position (control `store/execution.rs:490`, candidate `:557`). Different outlined/inlined instances and call routes become visible in these builds. The public work/output signatures remain identical. Do not infer more semantic completions, a new numeric algorithm, or a 73M independent regression from that single positive row; the paired decreases and other monomorphs must remain visible. The summary preserves both top exclusive changes and exact incoming/outgoing edge counts.

Libc `__memcpy_avx_unaligned_erms` is a leaf in the inspected routes. Its total self cost changes **16,241,258 -> 19,764,893 Ir (+3,523,635)** and **38,376,876 -> 48,933,015 (+10,556,139)**. The complete caller-edge deltas sum to exactly those changes. Important matched caller deltas are:

| Caller into memcpy | Repeated OBJ delta Ir | Grid delta Ir |
| --- | ---: | ---: |
| `ParseExecution::with_work` | +2,458,331 | +8,425,901 |
| `MeteredVec::try_push_reserved` | +1,063,169 | +2,126,075 |
| Old `MeteredVec::try_reserve` -> new `ParseStorage::reserve_vec` pair | 0 | 0 |
| Old `reserve_storage_cause` -> new `ParseStorage::reserve` pair | 0 | 0 |
| Old `MeteredVec::try_push` plus new `ParseStorage::push_vec` pair | +1,827 | +3,879 |

The first two callers account for **3,521,500 / 10,551,976 Ir** of the respective increase. `with_work` memcpy calls change **49,157 -> 131,095** and **98,309 -> 379,166**; `try_push_reserved` changes **2 -> 16,388** and **4 -> 32,774**. Conversely the source-qualified replacement/growth route `Vec::append_elements -> memcpy` remains **3,093,182 / 4,821,550 Ir** unchanged, and the reserve-vector pair retains the same calls and cost. Thus the observed extra libc instructions point principally to adapter/result/value movement caller contexts, not newly demonstrated large geometry-buffer copying. Callgrind records neither memcpy byte arguments nor copied-object identity here. Identifying a specific enum/result/loan layout as the object requires generated-code or type-layout evidence, which this audit did not obtain. No payload-size, allocator traffic or Windows code-generation cause is asserted.

## Build/task association and limitations

Both Linux symbol workers were freshly built with Rust 1.95.0, release opt3 and debuginfo 1, one job, no allocation metrics. Cargo artifact JSON records operation/import/STL/consumer as `fresh: false`, with correct separate source/target roots, optimized profiles, no assertions/tests and no consumer features. The helper clears external Rust flags/wrappers/preload and selects debug level 1. Retained stderr ends successfully. The roots are fresh isolated Linux trees justified by the prior concrete shared-target association failure; no reviewer accessed WSL or rebuilt them.

The two build receipts identify the exact previously audited ordinary source archives; these archives were rehashed again here. Source-level statements above were read from those archives, with all inspected file hashes in the summary. Original comparison implementation, requested fields, F64 output, source geometry, explicit repeated-OBJ missing Smooth policy, clocks and cleanup are unchanged. Control/candidate profiled observations have identical non-timing fields for all three cases. Three cases and one profile each per build are not ordinary statistical confirmation or full format qualification.

Inline source attribution is a compiled-debug view. A single generic symbol name may aggregate several monomorphs and callback bodies; inline callees may have no edge or self row. A file row named Rust core may execute inside a Baozi wrapper. Symbol-size or exclusive shifts do not prove a source-only isolated optimization. Costs in nested edges are inclusive and may overlap; only disjoint self rows and full partitions are summed here. There is no import/prepare/extract/teardown CPU boundary in these whole-program profiles, so none of the percentages is a stage share, cycle count, native latency, memory result, Linux ordinary speedup or Windows causal explanation.

## Exact artifact hashes

| Identity | Control | Candidate |
| --- | --- | --- |
| Source revision | `37e800541be6cae31585e40c01828aa64b5809ef` | `48d6f17e2f0829bdd5d50c8d7c48cea72ce343a1` |
| Source archive SHA-256 | `7e29fab78351316ae775a0c489ca86f43fb40717b445d5bbfee6db59c00a937a` | `169d3fda4c70ab084264c0dda986e7e20bdc78fb251b69ebe4fecb36e3aedca4` |
| Linux worker SHA-256 | `538abbbd81cb10c404b76c46c6fd5e191b82a8595ca9e1eb4b0883e44c6cb645` | `ad4c62f0f26fea5f995f7b3e12ec07e867e4c785ea15e307ae3df52692bd16bb` |
| Build manifest SHA-256 | `6f3f731a0426b0ff4b90ddb9723be608a54c13457869d2b1c61b7183de91cfa4` | `8dea2efab05a34354da19e34d38e686e5ac05610d4c3279f40dbfc825ee126c4` |
| Build stdout SHA-256 | `7d6a8a899c4a50c500adc06fe78cb06bf5ccc728c5421f76b60781acecaf6ad1` | `a34fa0e72a85dcf012c8511c10c9eb59a81cc08c2ac7406fc1d359247da547b1` |
| Build stderr SHA-256 | `f24a058d26435c68bb2d8bcb4b5726adb5dc813741a463c256b15825064924c1` | `af12c4294cfa9d291e14ecd0a8ba4220687d25e5276ed11f218c601108bc62f3` |

Both build directories additionally retain `stdout.jsonl` and `stderr.log`; their hashes and every profile's stdout/stderr/annotation-stderr are included by full relative path in the small companion JSON. The next table pins the six receipts and the three principal profile representations explicitly.

| Profile | Receipt JSON SHA-256 | Raw Callgrind SHA-256 | none.txt SHA-256 | both.txt SHA-256 |
| --- | --- | --- | --- | --- |
| control-obj-repeated-16384 | `3c110398f46a9727d193a56ff235314ec0721ebbd59380d081dfb770b6bcce7a` | `677e9cda22469d3efd3f863a4cddcb3e3e1134e32acf6502568f655af462a38c` | `87990aed27c3b76b65a20f0fa40f98415855885d7d3c31f3566903fc8860477a` | `3553c8f5b9bfc0f607905c40ea611f0e20283b65bfc728163a45c505a4658f07` |
| control-obj-grid-128 | `2d4f5ec99b64c7443818241dad714642bd8c7af1878ec4752a7c80df95d588ae` | `44657e252b463db854db43ae26db5ab4df193ca12f09484a398308b81ab36695` | `6fa604b45b7c37f5d720856f2d736d37a641042c7d0b1b5bfd57235247777f07` | `bcc235367c487de26e1582704a8b2b6b35cad94b3d6981bdff4ac51ab7b043ab` |
| control-stl-repeated-1024 | `52ff1cc6b593d2267c8d9220f973f881dd4510edb66e994722c25d96f4974a45` | `78bc526a17a170c55b1167429883fc0cf6153e3419033a4cb0c304b8f8dd979f` | `e739592add200f7cf539e9aee6dcc9fbaddf7577fbdcbde0207c7f01173e1069` | `72728554f537c40845cabfe6aea5ba1c00b3251a3749ffe0de7601083238e974` |
| candidate-obj-repeated-16384 | `6ce6132ceee238782a4801dca1cffb4f4142864cb249065c503cc12a6db504a5` | `d27a9b074127799a42bcde03b06d3c4f51645008aa4d2d980cdcd5136cef4bdf` | `6b4ec2f27ec2f284ec4029cbba240f2ffdfb7c48ac758fa61d24530e1614b8b2` | `fc3f08dd9ffd2d71011a9b3e7511b8902bc9e4a5081413ef1aa4d7ba45cf2c6d` |
| candidate-obj-grid-128 | `c41c8d3222de00b804922f0c4650792e53ecba5f660945704b02b3d76f37f7da` | `cc711a7dd04a24604833c82bf2437603436b4bf978b10fd6a9904fd83d3740ba` | `83c5032563de2aa467f6aaf684c2da71a54ee149b384455d437292243a9e3322` | `3a169210795adcf877c629b018530bfe62335ed5361bc8881424309e2cf25edd` |
| candidate-stl-repeated-1024 | `6684482f80dc85bf435162b8116a4a38bcd74f5fbc1ae46bd26cadf92cdedca1` | `ba5d0a5b08f6e45b738a65a42bb3d1e71320325dcd55a7636161ddfea0f5cb36` | `78c4b9851ee68a7aefd1cdb249c7db04bebdccf46cebc849497a1207159323cb` | `20723c605baa5de9263a9483eeef6b104856217f702124cea4786dd211a80483` |

The reviewer launched only local offline Python and read-only Git/file inspection during this lane, with no Cargo, compiler, worker, profiler, WSL, benchmark or outside agent. Only new reviewer artifacts were written. Primary raw/annotation/build files, production source, ordinary receipts and prior reviews remain unchanged. All final reviewer hashes are reported with the delivery; the program hash is also embedded in the summary.
