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

## Ordinary preflight and Windows startup attempt

Four fresh uninstrumented opt3/debug0 Baozi workers are built serially in the current
repository target. Per-platform control/candidate compiler descriptions, arguments
and recorded overrides agree; candidate 972-input maps match final QA. Controls use
fixed main capacity source `3cd9997`; cumulative candidates use `7c1a7d9`. The independent
diagnostics above compare the narrower OBJ delta against `9744b35`, so do not substitute
one comparison's source effect for the other. Windows Assimp F32/F64 are rebuilt from
the exact pinned local Git archive and pass 10/7 semantic ctests. GNU ordinary Assimp
uses the recovered exact optimized debug0 workers and original build manifests.

Windows and GNU each complete two original 110-lane functional preflights, with zero
comparison-task failures. Each records 222 commands: 220 comparison children plus two
Git context probes. The frozen source archive is located inside the repository's
`.git` evidence tree: `source-head` succeeds but `source-status` exits 128. Those context
failures remain in raw reports; worker identity is established by exact build/source
archives and binaries, not the collector's enclosing Git HEAD/status. A successful
functional preflight does not provide ordinary sampling evidence.

Windows C0/N0/N1/C1 all remain **not started**: none obtains the required 20-second
named-tool quiet interval within its prospective 180-second deadline. Actual recorded
waits are 181.016/181.112/181.048/181.149 seconds, preserving polling overrun and every
sampled Cargo/nextest/rustc/link observation. There are no Windows ordinary samples
for this attempt. GNU C0/N0/N1 likewise do not start within 180 seconds, with actual
waits of 180.493/180.505/180.620 seconds. GNU C1 completes 110 lanes and 5,060
successful comparison children, plus its two context commands, with the original
three warmup and twenty retained pairs. Both campaign envelopes remain partial.

There is no ordinary candidate sample on either platform. GNU C1 is old capacity
control `3cd9997` only. It cannot establish candidate convergence, a source gain or
control drift across the missing bracket. Every original lane and missing campaign
is retained in the [complete availability/ratio tables](2026-11-06-obj-document-token-scratch-lanes.md).

Independent ordinary review identifies a start-qualification defect: the launcher's
quiet duration starts at the last busy snapshot, crediting the next sampling interval.
C1 reports 20.135 seconds quiet, but the first all-clear joint snapshot to release is
only 18.114 seconds. Its required 20-second observed start window is therefore
unqualified. Preserve the complete C1 as a **descriptive old-control recording**;
do not repair, replace or relabel it. All 46 C1 load snapshots subsequently contain
Windows Cargo, with 43 also containing rustc. Empty Linux tool lists and low Linux
load averages do not establish host isolation.

For future attempts only, version 2 of the launcher starts at the first fresh all-clear
snapshot with zero quiet time, resets on busy/stale evidence, and checks the original
180-second deadline before acceptance. Independent bounded source review accepts
that correction; it is prepared but not executed. The original launcher, decisions,
C1 and every wait/load record remain unchanged. The 20-second criterion and original
task/sampling protocol are not relaxed.

The descriptive GNU C1 paired medians leave 70/110 worker endpoints above 3x Assimp,
28 at least 10x, and 14/28 complete pipelines above 3x. Examples include repeated OBJ
memory pipeline 4.931860x, grid OBJ 6.624102x and shared-32 glTF 12.009943x; their
parent elapsed ratios are separate 3.680762x/4.956565x/1.901210x. Whole-child RSS
has a maximum lane median ratio of 2.057177 and maximum individual pair ratio 2.141463.
These are old-control observations under recorded load and the unqualified start
guard, not successful candidate thresholds or new release evidence.

The independent audit reads 5,940 complete worker JSON objects, all named command
stdout/stderr, 5,500 GNU RSS records, 661 wait records, build associations and every
table cell. Pair inversion precedes median/IQR calculation; failures and unavailable
metrics remain explicit. The audit accepts functionality, descriptive statistics
and retained availability, with the quiet-guard correction above. It grants no
runtime keep, main integration or release acceptance.

Local association and receipts are
`obj-token-scratch-ordinary-build-association-1.json`,
`obj-token-scratch-ordinary-windows-campaigns-1/`,
`obj-token-scratch-ordinary-linux-campaigns-1/`, and
`obj-token-scratch-ordinary-linux-monitor-1/` in the same evidence directory.
GNU workers/corpus are copied to its native filesystem outside worker clocks; no
build takes place there. The unchanged protocol remains 14 cases/110 lanes, three
warmup and twenty retained AB/BA pairs, 60-second child timeout, and the original
release targets. No startup limit is widened and no unavailable sample is imputed.

The ordinary summary SHA-256 is
`e1dd6a75b1518d886cbee8845af760ec88904c2ee2898a0545e4a6aaac72cf7b`;
the descriptive C1 report is
`2e43c9c909cc8ea33f6045a9786f4b14c7d3cfb16562dff054fdff32a5e18c16`.
Independent audit is `obj-token-scratch-ordinary-review-1.md` in the same evidence
directory; future-guard preparation/review are `quiet-window-first-clear-fix-1.json`
and `quiet-window-first-clear-fix-review-1.md`. Source development proceeds separately
through the reviewed glTF owner-run candidate; the release goal remains active.
