# OBJ Document Token Ownership: Qualification and Separate Diagnostics

Status: source `7c1a7d9` is qualified on `optimize/obj-document-token-scratch`.
Independent source, final QA, instruction and heap reviews are complete. Separate
heap audit accepts the results with the retained summary/calibration limits. Ordinary
Windows/GNU time and whole-child memory qualification remain open. No main merge or
release acceptance follows. Release remains HOLD.

## Actual owner change

OBJ and each MTL document now own one existing metered borrowed-token vector and
truncate/refill it for each admitted line. `TextInputLine::text` exposes the decoded
document's existing borrow instead of restricting it to the temporary line view.
Tokens, grammar, ranges, column checks, recovery authorization and every token push
remain. The record cursor's separate per-record decoded owners are unchanged.

The retained capacity stays fully charged until document end. OBJ releases scratch
before loading material libraries; each MTL parser releases it before its final
material flush. No session cache, new token representation, unsafe operation or
capacity-fit admission bypass is introduced. The existing fixed OBJ scan scratch is
part of the same private token owner, with disjoint field borrows during refill.

The [plan](../plans/2026-11-06-0945-obj-document-token-scratch-plan.md) preserves the
first focused strict-Clippy rejection and its structural correction. Final tests
check the real importer with a 257-token early-wide statement, blank/comment records,
later recovery and geometry/material binding. Success at the candidate's exact live
peak and denial one byte below, exact final-token failure prefixes and zero residual
reservations pass. Allocation-dependent peaks and poll ordinals may change; the old
parser's peak is not promised. Existing actual-face cancellation and recovery/library
tests also pass. Wide-scratch allocator-failure/unwind and direct phase-by-phase charge
sampling remain bounded observation gaps, supported by existing source/RAII reasoning.

## Final serial qualification

Rust 1.95.0, one build job and one nextest thread; every build uses the current
`F:/SourceCodes/Rust/baozi/target`.

| Gate | Actual result |
| --- | --- |
| Workspace nextest, all features | 3,626 passed, one slow; six existing manual skips |
| Workspace doctests | 190 passed |
| Workspace strict all-target/all-feature Clippy | Passed |
| Asset-consumer nextest | 138 passed; one existing manual scale skip |
| Asset-consumer strict Clippy | Passed |
| Workspace and consumer formatting | Both passed |

The final receipt is `obj-token-scratch-final-qa-1/report.json` under
`.git/baozi-review/performance-convergence/`, SHA-256
`a1436a6517ce8a7039d2e68b4b90de815eb50dc847b92cb6c6743ae8ca8115dd`.
All 972 before/after/current Rust/Cargo inputs and the five frozen source hashes agree.
Independent audit checks all seven log hashes, totals, serial intervals and existing
skips. Expected panic stderr belongs to one passing postprocess unwind test. This is
Windows host/all-features qualification, not every feature combination or new wasm
coverage; environment attestation is limited to the recorded settings and inputs.

## Six original instruction observations

Candidate `7c1a7d9` uses a fresh GNU opt3/debug1/System build. The exact retained
`9744b35` binary and six prior profiles are reused as control; no fresh control run or
current paired native campaign is claimed. Recorded compiler, arguments, overrides,
workloads and manifest match; the candidate 972-input source map matches final QA and
differs from control at exactly the five reviewed source/test paths.

| Original pipeline/memory task | Control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| glTF shared 1 | 5,808,653 | 5,811,099 | +0.042110% |
| glTF shared 32 | 28,964,491 | 28,966,834 | +0.008089% |
| F64 PLY neighbors | 2,482,100 | 2,485,147 | +0.122759% |
| Binary STL repeated 1024 | 12,232,733 | 12,234,334 | +0.013088% |
| Repeated OBJ 16384 | 755,781,443 | 718,974,144 | -4.870098% |
| Grid OBJ 128 | 2,414,682,284 | 2,266,766,802 | -6.125671% |

Complete nonclock JSON, including work/output and original numeric spellings, agrees
in every pair; only `stage_ns` is excluded. The predetermined at-least-one-OBJ -1%,
no-case above +1%, equal-observations predicate passes independent review. Preserve
all four increases; do not discard them as noise or label them native regressions.

Compiled token-owner cleanup changes from 16,389/66,053 direct parse-path deallocations
to one token-buffer destructor/deallocation for repeated/grid OBJ. The corresponding
scan and record-child edges remain 16,389/66,053; token-parser edges remain
65,552/247,567. This locates the ownership effect, not every callback or allocation.
Overlapping inclusive costs are not savings estimates. Both grid profiles retain
Valgrind `brk segment overflow in thread #1` warnings and successful agreeing totals.
The debug-info-bearing binary grows by 4,720 bytes, from 76,417,000 to 76,421,720.
No stripped release-size or native memory conclusion follows.

## Eighteen fresh, separate heap workers

Each task runs once per exact Baozi control/candidate and pinned Assimp symbol worker
under heaptrack, separately from ordinary sampling. Counts and peaks cover the whole
worker, including setup, preload, validation, output and teardown. Peaks below are
heaptrack_print's displayed rounded decimal values, not exact byte measurements.

| Original task | Control allocation calls | Candidate allocation calls | Assimp allocation calls | Control / candidate displayed peak | Assimp displayed peak |
| --- | ---: | ---: | ---: | --- | --- |
| glTF shared 1 | 666 | 666 | 375 | 176.01K / 176.01K | 155.19K |
| glTF shared 32 | 1,116 | 1,116 | 845 | 416.53K / 416.53K | 174.36K |
| F64 PLY neighbors | 518 | 518 | 187 | 135.97K / 135.97K | 2.18M |
| Binary STL repeated 1024 | 488 | 488 | 1,186 | 456.56K / 456.56K | 317.09K |
| Repeated OBJ 16384 | 33,389 | 17,001 | 82,154 | 12.18M / 12.18M | 19.91M |
| Grid OBJ 128 | 99,425 | 33,373 | 360,697 | 28.82M / 28.82M | 28.69M |

The OBJ allocation-call reductions are 16,388 and 66,052. Control stack summaries
attribute 16,389/66,053 calls to token-vector growth, equal to the inputs' nonempty
record counts. The exact whole-worker saving is `record_count - 1` in each input;
candidate summaries still expose 16,384/32,768 face-related allocation calls. Ranked
summaries do not independently count a one-call candidate token branch or measure
MTL savings; both measured OBJ inputs contain no material library.
Equal displayed peaks do
not prove exact-byte equality or reduced peak memory. These are tracked heap results;
neither heaptrack time/RSS nor diagnostic `stage_ns` substitutes for ordinary metrics.

Independent audit verifies all 80 task/sanity stdout, stderr, analysis and compressed
trace files, every worker/analysis exit, complete nonclock/work pair equality and
eighteen analytic observations. Recorder stderr and saved analysis disagree on
temporary counts: analysis is one higher in nineteen executions and three higher for
F64 PLY Assimp. Both counts remain; the report adopts analysis values without claiming
an explanation. All Baozi task analyses also retain `total memory leaked: 544B`,
Assimp 4.10K, and the two sanity programs 896B/4.43K. These tool exit-live labels
remain unattributed; they are neither confirmed application leaks nor leak freedom.
Candidate/Assimp displayed peaks are unfavorable for both glTF tasks, STL and grid
OBJ, approximately 1.1342/2.3889/1.4398/1.0045. They remain rounded heap ratios.

Assimp F32/F64 symbol workers survive in WSL and are recovered only after matching
their historical exact binary hashes. Their runtime identity remains
`archive-declared-pin` for 6.0.5, not runtime Git verification. The surviving Rust/C++
sanity executables are also frozen and rerun: each reproduces historical checksum
17,293,312 and displayed 2.10M/8.19K/32.77K allocation sites. Their original source,
build receipts and old raw calibration traces were deleted. These fresh sanity runs
do not renew that missing source/build attestation or establish universal allocator
or dependency interception. Retain this limit alongside the new raw traces.

## Evidence and next disposition

Local evidence under `.git/baozi-review/performance-convergence/` includes frozen
`obj-token-scratch-source-2`, final source/QA reviews, `obj-token-scratch-probe-1`,
`obj-token-scratch-instructions-1`, its independent instruction review,
`heap-prerequisites-recovery-1`, and `obj-token-scratch-heap-1`.
The instruction report SHA-256 is
`39c8b94f8a8db5e41d4c6fea028263e69ba68831a58442071353e63f479a860b`.
The heap report SHA-256 is
`28eabbf429e13421b9429372d1714796a0f09ad198a3510ea1f830fd7f7246e9`;
`obj-token-scratch-heap-review-1.md` records the independent audit and every summary
discrepancy. Its disposition accepts bounded diagnostic evidence only. Ordinary
results remain a separate follow-up disposition.

Rebuild ordinary workers serially in the current target. Preserve both platforms'
original 14 cases/110 lanes, three warmup and twenty retained AB/BA pairs, all signs,
failures, startup limits and load records. No favorable diagnostic subset closes the
original release gates. The next independently reviewed source candidate is
[glTF owner-local source runs](../plans/2026-11-06-1030-gltf-owner-source-runs-plan.md);
its source work stays separate from these fixed workers and all measurement clocks.
