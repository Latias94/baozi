# Reuse OBJ and MTL Tokens Within Their Immutable Document

Status: reviewed source experiment integrated on `optimize/obj-document-token-scratch`,
based on reserved-row source `9744b35`. The reserved-row diagnostic control/candidate
are already archived separately; this delta is not part of either worker. Final
production sources pass affected-package runtime/doctest/lint/format qualification;
the strengthened failure-prefix test also passes. Final integrated qualification
passes with 972 stable Rust/Cargo inputs. Measured acceptance remains pending.
Release remains HOLD.

## Cost and ownership

Every current OBJ/MTL line allocates a new metered token buffer although tokens borrow
the same immutable decoded document. The earlier root-cause investigation identifies
66,053 grid OBJ record buffers; the later heap stacks agree on that allocation branch.
Those historical diagnostics locate the cost, not the saving from this experiment.

`TextInputLine<'input, 'scope>` already stores `&'input str`, but its elided getter
returns a reference limited to the wrapper borrow. Expose the existing input lifetime
without changing ownership. Each full-document OBJ/MTL parse then owns one existing
`MeteredVec` of existing borrowed tokens and truncates/refills it for each admitted line.
Reset `comment_start` on every line. Keep the existing slice grammar, raw ranges,
columns, token pushes and error projection.

No public token pool, offset-only representation, new token view, unsafe operation,
session cache or generic allocation framework is needed. The existing incremental
record cursor is a real boundary: each callback owns a different conformed record,
so its decoded borrows cannot populate this outer document token buffer. Leave that
cursor and its higher-ranked callbacks unchanged.

## Boundaries to preserve

Records admission remains before tokenization; chunk DecodeWork and every Tokens and
TokenBytes child remain. Capacity-fit pushes keep their existing identity, preflight
and control checks. The same scan handles NUL, Unicode, comments and the later chunk
frontier. Tokenization completes before recovery authorization, which completes before
grammar or material mutation. A failed partial token sequence never reaches grammar.

OBJ scratch dies at the end of its parser before library loading. Each MTL document
owns and destroys its own scratch; explicitly drop it before the final material flush.
Tokens contain inert borrowed text and metadata, so ordinary lexical destruction
continues to release backing storage before its reservation on errors and unwinds.
Material names, paths and semantic metadata remain owned by their existing outputs.

Capacity retention is a real resource change. `truncate(0)` does not shrink the buffer
or release its reservation. An early wide statement can keep its high-water capacity
live alongside later geometry/material growth or blank records. Peak live bytes,
budget success/failure and growth-dependent poll ordinals can change. Keep the actual
allocation fully charged; do not promise the predecessor's peak or hide bytes by
releasing its token early. Avoided growth operations are physical changes; no invented
Internal WorkUnits reduction is attached to them.

## Source and verification

The initial implementation is five files: the public text getter and existing contract
test, private OBJ/MTL parsing, and OBJ limits coverage. Independent correctness review
finds no confirmed source defect and permits serial compiler/runtime qualification.
It does not certify borrow-checker acceptance, downstream compatibility, cancellation,
allocation failures, memory peaks, speed or main integration.

The positive contract test consumes line views and retains their text slices within
the actual document callback. A wide-first integration fixture then uses blank/comment
records, a later textual recovery and real OBJ geometry or MTL binding. It measures the
candidate's live peak dynamically, requires success at the exact bound and failure one
byte below, and checks later token rejection and reservation cleanup. These observations
must execute; source reasoning does not establish their results.

Reuse existing record/token rejection, comment-chunk cancellation, text recovery,
multiple-library/origin, live-byte handoff and actual-face cancellation tests. Qualify
the lifetime interface through Import, OBJ and unchanged STL/cursor callers. Inspect
all source changes from formatting; never discard unrelated user edits.

Main owns all Cargo work, serial jobs1 and nextest threads1, using Rust 1.95.0 and
`F:/SourceCodes/Rust/baozi/target`. The parallel worker edited only an isolated source
worktree and ran no build. First run affected nextest/doctests, strict lint and formatting;
final selected source still requires the complete workspace/consumer gates. Preserve
any compiler/test failure and correct its actual cause rather than weakening an oracle.

Compare the same six diagnostic pipeline/memory tasks against `9744b35`: at least one
OBJ whole-worker Ir reduction of 1%, no case above +1%, identical complete non-clock
observations and reported work. Attribute actual token-growth calls and fully charged
high-water memory in separate heap observations. Diagnostics remain distinct from
native elapsed time and ordinary process memory. Any accepted candidate retains the
original 14-case/110-lane Windows/GNU campaign, 3 warmup/20 retained pairs, source/binary
identities, every unfavorable sign/load failure/missing round and the original release
targets. No source review, allocation reduction or favorable subset grants acceptance.

Local source opportunity/review/frozen copies remain under
`.git/baozi-review/performance-convergence/obj-token-*.md` and
`obj-token-scratch-source-1.json`. No push, PR, publication/yank or native promotion is
part of this experiment.

## Final source checkpoint

The first focused run passes 656 Import/OBJ runtime tests and their doctests, then
strict Clippy rejects two unnecessary outer lifetime names and an eighth `parse_line`
argument. The correction elides only those outer lifetimes and moves the existing
fixed `TokenScanScratch` arrays into the existing OBJ token owner. The tokenizer
borrows its fields separately, and token pushes still receive the actual metered
vector. No lint allowance, extra allocation, public interface or tokenizer is added.
The second focused run passes all four gates, including 656 runtime tests with no
skips and strict all-target/all-feature lint.

The later test-only strengthening locks independent source-derived token totals
(275 OBJ / 283 MTL), final rejected-token attempted/consumed counts, completed
Records/Vertices/Faces prefixes, and the exact ordered diagnostic/source/raw-range
sequence. That selected test passes separately after formatting, exercising both
document kinds. Its 41 filtered tests are not newly executed or manual skips.

Frozen source 2, including the correction and strengthened assertions, is recorded in
`.git/baozi-review/performance-convergence/obj-token-scratch-source-2.json`.
The bounded final source review compares it directly with source 1, verifies all
five current/frozen hashes, and reuses the original ownership review. It finds no
actionable defect. It verifies that the focused run covers the final production
source while the later prefix run covers the final assertions. Full selected-source
workspace/consumer qualification then completes with the same frozen five hashes:

| Final serial gate in the current repository target | Actual result |
| --- | --- |
| Workspace nextest, all features | 3,626 passed (one slow); six existing manual skips |
| Workspace doctests | 190 passed |
| Workspace all-target/all-feature Clippy, `-D warnings` | Passed |
| Asset-consumer nextest | 138 passed; one existing manual scale skip |
| Asset-consumer strict Clippy | Passed |
| Workspace and consumer formatting | Both passed |

The receipt is `obj-token-scratch-final-qa-1/report.json` under the same local evidence
directory, SHA-256
`a1436a6517ce8a7039d2e68b4b90de815eb50dc847b92cb6c6743ae8ca8115dd`.
All seven logs and all 972 before/after/current Rust/Cargo hashes agree. The existing
passing postprocess unwind test's intentional panic stderr remains in the log. This
is Windows-host source qualification; it is not new wasm, skipped-fixture or native
performance coverage. Independent final QA receipt review confirms all seven gates,
logs, totals/skips and source association with no actionable findings.

Existing actual-face cancellation, later comment/token rejection, recovery-frontier
and multiple-library/origin tests exercise the retained document owner. High-water
allocation failure, panic/unwind and direct charge sampling at each library boundary
remain bounded observation limits; they are not new runtime claims. Actual allocation,
instruction, elapsed-time and process-memory effects still require measurement.

The next diagnostic comparison reuses the exact retained `9744b35` GNU opt3/debug1
control binary and its six completed original profiles, whose source/build/raw
associations are independently reviewed. Only candidate observations are fresh in
that comparison. Record that reuse explicitly; do not label it a fresh control run or
paired native campaign. The unchanged ordinary protocol still requires fresh serial
campaigns with all original lanes and sampling.

The [completed diagnostic checkpoint](../reviews/2026-11-06-obj-document-token-scratch.md)
retains both OBJ Ir reductions (-4.870098%/-6.125671%), all four non-OBJ increases
(+0.008089% to +0.122759%), equal full nonclock observations/work, 4,720 additional
diagnostic binary bytes and both grid profiler warnings. Independent instruction
review accepts the predetermined diagnostic predicate only. Eighteen separate fresh
heap workers report 16,388/66,052 fewer OBJ allocation calls and unchanged rounded
displayed peaks. Independent heap audit verifies all 80 task/sanity artifacts and
retains recorder/analysis temporary-count discrepancies, exit-live labels and missing
historical calibration source/build evidence. Native qualification remains open.
