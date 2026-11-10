# Actual-row text emission: qualified source and instruction experiment

The bounded instruction-selection predicate passes. Grid OBJ128 decreases
7.14259617% and repeated OBJ16384 decreases1.59745099% in whole-worker
Callgrind instructions. All four other cases increase, by0.06374037% to
0.38063229%; every sign remains below. Native convergence, local-main runtime
integration and release remain **HOLD**. This is not an ordinary-time result.

The experiment follows the reviewed
[text-emission plan](../plans/2026-11-08-0820-closed-text-emission-plan.md)
after the [fixed finite-consumer heap qualification](2026-11-08-finite-consumer-attribution.md).
The original Windows and GNU partial campaigns retain their missing controls
and GNU candidate intervals. They do not measure this new source cut.

## Exact change and correctness

Candidate `3da88939a91326371a5634b2421fd22e0153aafe` changes exactly
`crates/baozi-import/src/text_input.rs`. Its immediate parent is the precommit
QA base `f127bb66d68b0d9cf3edc98d6dc64438d678faa9`; that parent adds only
documentation above the earlier finite-consumer source. The diagnostic
control is the retained qualified `b76fbc0bfdb83739fdfef8425702c7b5c5d4472a`
worker. No other production source changed between the compared input maps.

The emission pass now borrows the existing actual singleton accounting row
between control frontiers. Each classified raw width is immediately committed
to the original row/remainder before arithmetic or output writes. Stable
channel/layout checks occur at loan entry. Numerical consumed/pending/
attempted/limit checks, original output owners and the armed host guard remain.
The retired per-unit generic runner path and its two unused WorkCursor polling
wrappers are deleted. No public API, provider, copied counter, deferred
completion, allocation policy or generalized multi-lane executor is added.

Raw/debt exits remain distinct; debt-only polling does not advance the raw
frontier. Keep the original4096-byte lookahead/poll order, UTF-8 and CRLF
widths, BOM offset handling, exact-EOF poll, final valid suffix and exact
output/index checks. Actual Import uses `Some(importer_boundary)`; shared
Operation's direct-mode tests remain unchanged. No direct Import entry was
invented solely for this private caller test.

The new actual-emission unwind oracle panics after writing the first line of
`a\nb\n`, with the row still active and an unread suffix. It catches outside
the builder, rejects later physical work, preserves DecodeWork attempted8 /
consumed6 and the original runtime-contract cause, and verifies actual live
storage/reservations/canonical members return to zero. Existing cancellation,
copy-failure, conformance and direct-row tests remain.

Frozen source1 has1,058 input hashes. Scoped Import qualification passes487
tests, strict all-target/all-feature Clippy and format. Original final seven
gates then pass:3,957 workspace tests (two slow, six existing manual skips),
218 doctests,138 independent consumer tests (one existing manual scale skip),
both strict Clippy gates and both format checks. All1,058 before/after/current
inputs and committed archive bytes match. The source and actual quality
reviews close with GO, including all six existing1MiB stack tests and the
unchanged consumer PASS set. The extra workspace PASS is the new unwind oracle.
Expected caught-panic stderr and repeated final success output are retained
and counted once.

QA ran on the frozen patch before the single-file source commit. The build
enforces the exact parent/path relationship and complete archive/QA/checkout
digest equality; it makes no postcommit test-execution claim. GNU general
runtime and wasm qualification are outside these Windows quality receipts.

## Matched symbol workers and tasks

| Role | Source | GNU symbol-worker SHA-256 |
| --- | --- | --- |
| Control | `b76fbc0bfdb83739fdfef8425702c7b5c5d4472a` | `e8fa229f90e334e506a7c2141ced9d2980457ed81c7db82125c8009f95ca3993` |
| Candidate | `3da88939a91326371a5634b2421fd22e0153aafe` | `ccb81771bc7b4fcca62f1d12fce9b86c54ec8c2ca45afb57fa51c2bbf686e333` |

Both workers use matched Rust1.95.0, opt3/debug1, generic CPU and System
allocator, empty consumer features and no allocation instrumentation or
CPU/LTO flag overrides. Actual compiler identity, build argv, environment
overrides and consumer profiles are equal. The candidate builds in
`/mnt/f/SourceCodes/Rust/baozi` and its existing target; Windows source/build
work remains in the same `F:/SourceCodes/Rust/baozi` repository. Neither
symbol worker is an ordinary timing input.

Twelve fresh processes run the same six original pipeline/memory tasks,
with24 retained Self/inclusive annotations and the original process-group
timeout helper. Corpus, fields, normal policies and work bounds remain
unchanged. Complete six-pair JSON matches after removing only `stage_ns`,
preserving integer/float spelling and types, work, output and metadata.

## Actual instruction results

| Case | Fresh control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| Shared glTF1 | 5,861,004 | 5,868,901 | +0.13473801% |
| Shared glTF32 | 25,530,543 | 25,590,778 | +0.23593309% |
| F64 PLY neighbors | 2,499,201 | 2,500,794 | +0.06374037% |
| Repeated binary STL1024 | 12,172,115 | 12,218,446 | +0.38063229% |
| Repeated OBJ16384 | 723,375,934 | 711,820,358 | -1.59745099% |
| Grid OBJ128 | 2,269,692,689 | 2,107,577,706 | -7.14259617% |

Change is `100*(candidate-control)/control`. All comparisons use these fresh
same-campaign control executions. Small differences from earlier profiles
of the same retained control binary are preserved; older instruction totals
are not substituted into this table.

The fixed predicate uses positive integer instruction counts: at least one
OBJ has `100*candidate <= 99*control`, and every case has
`100*candidate <= 101*control`. Both OBJ tasks satisfy the first test and all
six satisfy the second. Selection also requires all pairs completed with
full non-clock equality. Descriptive floating-point percentages do not decide
the boundary, and collection completion remains separate from selection PASS.

Both grid profiles retain `brk segment overflow in thread #1`: control at
`0x5417000` and candidate at `0x5419000`, together with the manual-limitations
notices. Every raw stderr stream remains available. Whole-worker Ir includes
setup, validation, observation, serialization and teardown. Counts supply no
stage clock, Assimp instruction comparison, RSS, native speed or heap result.

The [complete12-attempt CSV](2026-11-08-closed-text-emission-instructions.csv)
retains both roles, exact source/binary identities, all instruction counts,
candidate deltas and per-pair selection/equality flags. Its line endings are
LF. Independent actual raw/profile/config/output review closes with GO:
all12 raw Self sums agree with summary/footer/stderr and both annotation
totals; all72 streams and source/archive/binary associations match; all six
complete literal non-clock pairs and exact integer gates reproduce. This
qualification remains separate from source-retention or native-release
decisions.

## Evidence and next disposition

Private receipts remain under `.git/baozi-review/performance-convergence` and
raw data under `target/assimp-comparison/closed-text-emission-instructions-1`.
Both source/quality review and the measurement-driver amendment are closed.
The latter enforces source-only precommit association and exact integer gates;
the original reviewed driver versions and earlier AMEND receipt remain.

| Evidence | SHA-256 |
| --- | --- |
| Source1 manifest | `49e03ebdc16d81cd8d035879307dbc656f22bf13a5dcce72ef5c28cf91844d31` |
| Actual final seven-gate receipt | `0cdac9ff6cf9e996c89e3705afae00f647ec7a02c64339bb60d006d04246af23` |
| Independent actual QA review | `94667d6def605bb62be19c78e761534fbc34b23ff70ba83c63b11b03a830e34c` |
| New GNU symbol build | `b5ef126da9fab6298552767d3380a3aa9ac26caae9f5e08cbff69815944d7c01` |
| Actual instruction receipt | `4ccea4da353bf4a5d66aac3094bd63fca75a48525ef06f4bb992ccdd51adcbd0` |
| Independent actual instruction review | `ad346561e1493dcd30d1ca776361df68c066b4ae97196d7a6d3a78186411df54` |
| Complete instruction CSV (LF) | `a5baadccf929b4b7100104a689178dd9cc320a0bad6682ddba43a704646c21eb` |

Next qualify new ordinary workers for the original14-case/110-lane Windows/GNU
protocol: three warmup
pairs plus twenty retained fresh AB/BA pairs, all campaigns/load/missing cells
preserved. No new ordinary result for3da is available here. Earlier finite
tracked-heap observations concern b76, not this new candidate; no fresh3da
heap result is claimed. Main/runtime integration and release continue to
require the active convergence evidence.
