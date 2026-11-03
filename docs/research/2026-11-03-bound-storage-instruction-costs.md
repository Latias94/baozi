# Bound Storage Services: Matched Linux Instruction Costs


Date: 2026-11-03. Tracked export of the independent offline audit, SHA-256
`d30e19571d67975e7e8a0d22f0f2c558a05024730411c750e56ba9531b9cebbb`. It supports the
[ordinary performance HOLD](../reviews/2026-11-03-bound-storage-performance-hold.md)
and [convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
Reviewer sidecars named below are retained under
`.git/baozi-review/performance-convergence/`; their hashes identify local evidence,
not files distributed with this Markdown export.

**Verdict:** the six retained raw profiles reconcile completely. The bound owner removes a measurable repeated construction cost on both OBJ tasks: whole-worker Ir falls 1.3318% and 1.2934%. STL is effectively unchanged, with a retained increase of 1,341 Ir. This supports the bounded Linux construction-cost explanation, not a Windows performance keep or a general speed claim. No attribution-integrity blocker was found.

## Evidence and method

Control is **48d6f17e2f0829bdd5d50c8d7c48cea72ce343a1**, symbols SHA256 `ad4c62f0f26fea5f995f7b3e12ec07e867e4c785ea15e307ae3df52692bd16bb`: `target/review/convergence/binary-stl-attribution/candidate-{obj-repeated-16384,obj-grid-128,stl-repeated-1024}.*`. The old 67b5 control was not used.

Candidate is **314d0fe3855cdd53159d2ffc0d6e10206d299b9f**, symbols SHA256 `56a2ba3e28777e39c3fe9e929482648e56459e934396b1276d1ec9f349e013cf`: `target/review/convergence/bound-owner-attribution/{obj-repeated-16384,obj-grid-128,stl-repeated-1024}.*`.

The new offline audit copies the four pure readers from the frozen previous audit; it never runs that script's driver. For every profile it independently checks raw `summary`, `totals`, exclusive sum and stderr `Collected`; all exclusive function and file/function rows against **both** annotations; and every raw caller/callee edge's count and inclusive Ir against **both directions** of the `both` annotation. All six pass. The JSON retains artifact hashes, full receipts, row/edge counts, focused source-file attribution and caller/callee details. Main's summary was not a numeric input.

Both build receipts are successful Cargo 1.95.0 release/debug=1 builds without instrumentation or allocation metrics. Retained binaries, archive SHA/PAX revisions, input byte lengths/hashes and build/profile receipt identities match. Pipeline/memory argv, field selections, normal policies, expected observations and all observation fields except instrumented `stage_ns` match across each pair. This is observation equality, not an independent proof of every scalar bit or failure path. Profiles cover the whole worker, including checking and teardown; they are not endpoint timings.

| Task | Control Ir | Bound-owner Ir | Delta Ir | Delta |
|---|---:|---:|---:|---:|
| repeated OBJ 16384 | 1,011,204,654 | 997,737,279 | −13,467,375 | −1.3318% |
| grid OBJ 128 | 3,232,140,919 | 3,190,335,809 | −41,805,110 | −1.2934% |
| repeated STL 1024 | 34,224,811 | 34,226,152 | +1,341 | +0.0039% |

## Construction owner: 90 to 23 Ir per independent entry

48d6 `ParseExecution::storage` has **60 self Ir/entry**, and its `ImportStoreExecution::ordinary` child has **30 self Ir/entry**. These are disjoint exclusive costs, totaling 90 per successful measured construction. At 314d, `storage` has **23 self Ir/entry and no outgoing edge**; the ordinary helper has no independent row or call edge.

| Task | Old/new independent storage entries | Old two-owner self Ir | New storage self Ir | Difference |
|---|---:|---:|---:|---:|
| repeated OBJ | 147,509 / 147,506 | 13,275,810 | 3,392,638 | −9,883,172 |
| grid OBJ | 444,732 / 444,729 | 40,025,880 | 10,228,767 | −29,797,113 |
| STL | 22 / 22 | 1,980 | 506 | −1,474 |

The three fewer independent entries on each OBJ task are exactly two old incoming edges from `conform_text_input` and one from `MeteredString::try_reserve`. Other caller counts match. Their absent edges do not prove three fewer semantic actions. Remaining helper code may be folded into callers or eliminated; absence of a symbol alone cannot separate those effects.

Source provides the ownership explanation: 48d6 `concrete_store/execution.rs:470` joins separate storage/control, checks `shares_invocation`, then raw `retained/operation/execution.rs:424` reconstructs `storage_view` and `reborrow_poll`. At 314d `concrete_store/execution.rs:478`, `ordinary()` borrows `&mut self.raw.ordinary`; `import/store/execution.rs:232` wraps that already bound service. The old sibling join is structurally gone. This is stronger evidence than treating a renamed or absent symbol as the gain.

Other disjoint self changes include `ParseExecution::with_work` −2,114,233 / −7,672,504 Ir and legacy `MeteredVec::try_push` −1,261,567 / −2,523,161 Ir (repeated/grid). These are caller attribution, not proof of fewer payload bytes copied. Whole `memcpy` self actually **increases** by 11,690 / 11,471 / 7,322 Ir (repeated/grid/STL). The JSON preserves every memcpy caller; new entry/owner transfers remain visible. Do not add inclusive edges to their enclosing flat costs.

The complete nonoverlapping symbol-prefix delta partition is retained in JSON. In particular import contributes −13,619,833 / −40,574,007 Ir on OBJ, while operation contributes +188,073 / −535,840. Inlining is charged to the containing symbol, so these prefixes are accounting partitions, not precise semantic ownership boundaries.

## Renames, real polls, allocations and numerical work

The audit normalizes only the actual `ExecutionControl`→`ExecutionServices` and `StoreExecutionControl`→`StoreExecutionServices` type segments. Old symbols disappearing are not counted as savings.

| `before_action` exclusive delta | repeated OBJ | grid OBJ | STL |
|---|---:|---:|---:|
| Concrete Store layer | +450,085 | +412,723 | +21,798 |
| Raw retained layer | −390,996 | −1,816,715 | −43,480 |
| These two layers' net | **+59,089** | **−1,403,992** | **−21,682** |

Both layers' invocation counts match across each pair. Concrete `completed_action` also retains its call counts and self Ir: 449,815/8,546,485; 411,104/7,810,976; 21,700/412,300. This subtotal excludes other import wrappers and nested work. An inlined raw completion is not free because it lacks an independent symbol.

The following paired values are **unchanged**; each cell is calls / exclusive Ir:

| Actual owner | repeated OBJ | grid OBJ | STL |
|---|---:|---:|---:|
| `poll_operation_control` | 384,751 / 11,157,779 | 1,216,850 / 35,288,650 | 1,462 / 42,398 |
| `Ledger::reserve_core_in` | 32,922 / 4,642,002 | 98,990 / 13,957,590 | 67 / 9,447 |
| `Ledger::resize_core_in` | 32,886 / 1,809,482 | 98,964 / 5,443,215 | 43 / 2,789 |
| `ParserFrame::complete` | 311,335 / 36,392,327 | 1,727,857 / 149,598,822 | 4,104 / 1,591,501 |
| `WorkFrame::complete_batch_with_outcome` | 730,535 / 63,558,822 | 1,086,371 / 94,512,596 | 43,428 / 3,779,098 |
| `WorkLayout::admit_observed` | 328,016 / 29,854,460 | 411,140 / 37,416,536 | 21,731 / 1,980,317 |

Outer libc malloc/free counts are unchanged at 33,385/33,385; 99,433/99,433; 496/496, with four reallocs and one calloc on every task. This does not prove identical allocator internals or bytes: repeated `_int_malloc` changes 16,722→16,721, and small internal instruction differences remain. The archived `ledger/work/kernel.rs`, `ledger/work.rs`, `metered_storage.rs`, typed-edit transaction/writer and comparison sources are byte-identical across versions. The numeric kernels' measured calls/costs above corroborate unchanged work; error/control correctness still belongs to the separate tests and source reviews. No state-gate removal is established or recommended here.

## Remaining actual canonical action owners

Current canonical roots and their direct children are already large after the constructor reduction. `TypedStorageBuildControl`'s adapter at `core/.../construction/node_scene/edit/transaction.rs:1137` invokes the **same** `CanonicalSemanticWorkSession::run_unit` implementation (`operation/.../activation/preflight.rs:1654`); compiler specialization accounts for the separate typed-edit symbol. It is not a second numerical backend.

| Current root | Calls | Root self Ir | Root inclusive Ir |
|---|---:|---:|---:|
| repeated OBJ typed edit | 262,189 | 30,462,585 | 212,736,359 |
| repeated OBJ generic validation | 65,729 | 7,264,040 | 53,354,472 |
| grid OBJ generic validation | 411,084 | 46,745,613 | 334,997,482 |
| STL generic validation | 21,682 | 2,425,719 | 17,630,875 |

Every root's calls and self Ir are unchanged from control. Current direct child edge Ir is below; these are sibling subtrees **inside** the preceding inclusive totals, not additional totals. Admission/completion/finish edge costs also match control exactly.

| Root | `admit_observed` | permit `complete` | permit `finish` | `before_action` | `completed_action` |
|---|---:|---:|---:|---:|---:|
| repeated typed | 66,596,006 | 51,389,044 | 38,017,405 | 16,307,942 | 8,652,237 |
| repeated generic | 17,944,017 | 12,422,781 | 9,267,789 | 3,956,540 | 2,169,057 |
| grid generic | 112,225,932 | 77,694,876 | 57,962,844 | 24,745,290 | 13,565,772 |
| STL generic | 5,919,186 | 4,097,898 | 3,057,162 | 1,305,120 | 715,506 |

The typed repeated root is reached by `generated_normal` 147,458 times, `GeometrySelectionPlan::try_new` 49,158 times and `Writer::u32` 49,152 times. Grid generic callers include `GeometryValidationContext::run_unit` 196,617 times, `validate_primitive` 98,314 and `inspect_f32_values_from` 83,208. Thus the remaining semantic-owner question is concrete: persistent canonical action admission/completion/finish through the existing shared kernel, including the typed writer/selection and validation callers. It is not another per-push storage constructor fix.

These root self costs also contain inlined useful semantic actions. The entire inclusive root or its admission/complete/finish subtrees cannot be declared removable. Any later owner proposal must preserve admission before the callback, actual immediate completion on normal return including semantic `Err`, completion/finish failure precedence before dropping returned values, callback/capture panic containment, final checkpoint and the existing weighted control frontier. No skipped validations, invented work, budget increase or second kernel is justified by this audit.

Nor is canonical work the largest universal remaining flat row. Grid still has `ParserWorkScope::complete` **415,386,531 self Ir / 5,389,287 calls**, `ParseExecution::with_work` **378,782,398 / 496,456**, and `BoundWorkGroup::borrow_active` **212,349,192 / 5,588,133**. Repeated has 43,442,993 / 573,557, 49,780,068 / 132,900 and 17,468,928 / 459,705 respectively. Parser completion and active borrow costs/counts are unchanged. Grid's text/parser work remains a distinct, larger investigation with its own exact-prefix contract; this report does not infer a safe classifier or completion batching change.

## Limits and frozen sidecars

Both old and new grid stderr retain Valgrind's `brk segment overflow ... can't grow` limitation note. Both workers exit zero with expected observations and exactly reconciled totals. Preserve this shared diagnostic; do not turn it into a native allocator claim or conceal it. One retained profile per source/task provides exact cost attribution for these executions, not statistical native speed or memory evidence. Windows ordinary disposition remains the separate 5704-child audit. No Cargo, WSL, compiler, profiler, worker, source edit or measurement was run for this review.

Sidecar SHA256 pins:

- `bound-owner-linux-attribution-audit.py`: `245da1ffc176f544b50d1b74cef130f20be5ff7137283739707e0fdaf3fc3432`.
- `bound-owner-linux-attribution-summary.json`: `836996e76121d4bc26418665128d73181e5e69446cf6caffb459638be0623ff9`.
- Frozen previous parser source: `1ed3bc91df28b9e27dd05430247ca35d11d76a6c0c4fcab16644c848470f21cc`.
- Separate Linux QA review: `885368007130ceb822c0f72856fc891a18c231969f08115a5d3832d45c002b4d`; it covers 1459 runtime tests, import 34 + operation 56 doctests and affected strict Clippy, not this instruction attribution.

All raw/profile/annotation/build/archive hashes and the exhaustive prefix partitions are in the summary. This report and the two new sidecars are now frozen; earlier reports, scripts and raw evidence were left unchanged.
