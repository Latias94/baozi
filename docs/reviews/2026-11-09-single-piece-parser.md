# Single parser pieces execute through their original host

Status: candidate `2bdc37a28bd00fffe8c70b4bd4b8ad28a769a227` is source-qualified
but fails the original instruction screen. Retirement `84f68b8` removes the
new route and its dedicated fixtures, restoring all 1,059 qualified baseline
`0452a73` inputs exactly. No ordinary or heap worker runs for this candidate.
Independent five-file production source review closes GO before formatting.
Frozen source2 passes affected all-target/all-feature checking and all 1,403
Operation tests with zero skips, all 665 Import/OBJ tests with zero skips, and
affected strict Clippy. Independent test/format binding review closes GO,
including its initially unknown original-ParseFailure fixture after actual
runtime evidence. All original seven integrated gates pass on the same source2
inputs. Independent terminal-receipt and exact staged-byte review closes GO.
The committed candidate archive matches all 1,061 qualified inputs and its
independent narrow commit-association review closes GO. Actual instruction
fidelity, generated-code condition and retirement identity reviews also close
GO; the original instruction selection remains FAIL. No measured source
retention, native or release acceptance is claimed.

## Candidate owner and consumer change

The [reviewed plan](../plans/2026-11-09-0400-single-piece-parser-plan.md)
identifies OBJ/MTL token sinks that open a child solely to execute one returned
storage piece. That route formerly constructed the original ParserHost and
a second resumable ParserActionHost for a body that never resumed.

Operation now shares parser admission and terminal settlement through the
same actual ParserHost helpers. The single-piece route invokes its one
physical callback directly under that host, commits its literal returned
amounts through the unchanged kernel and performs the original exact close.
Compound parsers retain their separate per-piece guard because they can
catch a piece panic while the outer body remains alive. Both routes use one
shared numeric-completion/error-resolution helper. No work-kernel arithmetic,
alias handling or numerical ownership is replaced.

Import exposes `ParseChildren::run_piece_with_storage`; OBJ and MTL migrate
only their single-piece token insertions. Original token amounts, source
spans, checked columns and storage pushes remain. Scanner, text conformance,
mesh construction, multi-piece parser paths and per-token exit polls remain.
The old single-use child wrapper is deleted at those two sinks, rather than
wrapped by another adapter.

The original physical callback stays in an Import-owned Option borrowed
through admission. Runtime/item-limit errors reach the existing Import
projection before uncalled captures are disposed. Admission/close remain
Runtime-class failures; inner runtime, ParseFailure and format-evidence
materialization retain the Owned path. Ordinary storage growth/Internal work
still occurs before actual numeric completion. A partial successful return
commits only its actual prefix and fails exact close; it cannot become a
healthy scan boundary.

The original host remains responsible for callback/projection/materializer
unwind, with registry UnwindPending distinct from direct abandonment. Two
new doc examples exercise an owning result and rejection of a storage-loan
escape. Both pass in the completed documentation gate.

## Current source-qualified checks

All production and test writes are limited to nine paths: Operation's
exclusive host, child-admission forwarding, test-module declaration and new
single-piece fixture file; Import's parser role, test-module declaration and
new public fixture file; the two OBJ/MTL token sinks.

Frozen source1's affected check fails only in a new public fixture: calling
`expect_err` requires Debug for its successful `MeteredVec` result. Source2
uses an explicit `let Err` match with the same original-error identity
assertion. Production bytes do not change for this test correction. The
failed source1 receipt remains; it is not assigned source2's pass.

Source2 has 1,061 captured inputs. Affected all-target/all-feature checking
passes in 19.98 seconds. Operation nextest then runs all 1,403 tests:
1,403 pass, zero skipped. This includes all eight new direct boundary
functions; seven compare original child/prefix and single-host paths on
independent real fixtures. They cover alias completion, partial success,
overrun, output and callback panic, original error identity/materialization,
uncalled capture disposal, direct/registry debt/control order, pending
Internal work and tighter consumed limits. No original assertion is weakened.

Five additional public Import fixture functions cover actual storage/results,
exact work, incomplete/cancel precedence, original ParseFailure identity,
accounted format-producer failures and denied-capture Drop. All five pass in
the 665-test Import/OBJ run. The original error's eager construction was an
explicit static-review UNKNOWN; both cancellation branches pass unchanged
identity, producer counts, accounting and final-cause assertions. Independent
review closes that specific unknown using the raw result without a rerun.

Affected strict all-target/all-feature Operation/Import/OBJ Clippy passes in
44.30 seconds. All 1,061 source2 inputs remain identical before/after each
completed affected run. The complete workspace/consumer sequence also passes
all original seven gates in one serial invocation:

| Gate | Actual result |
| --- | --- |
| Workspace nextest | 3,976 PASS, one slow, six existing manual skips |
| Workspace doctests | 220 PASS, including both new loan examples |
| Workspace strict all-target/all-feature Clippy | PASS |
| Public asset-consumer nextest | 138 PASS, one existing manual scale skip |
| Public asset-consumer strict Clippy | PASS |
| Workspace format | PASS |
| Public asset-consumer format | PASS |

Session 52205 is terminal0. Its final receipt SHA-256 is
`4f8eb15599cd7eb98870d040a4935b2726cb8936ea1232577d601ab50fb7a074`.
All 1,061 before/after/current input hashes match source2. The passing
ordinary-host unwind test retains expected panic stderr and a repeated printed
PASS tail; that tail is not a second test. Independent terminal log/count/source
review closes GO, and no affected result substitutes for an integrated gate.

The source-only commit contains exactly the nine reviewed Rust paths and
immediately follows QA HEAD `a30c2c72929a7799c9636cccca1ca8cd05034a40`.
Every committed intended blob equals its staged/frozen/current bytes. The
retained source archive has SHA-256
`03fb4dc00861587406a6990872e847fc26933853cd758f1d16e87560832cc90d`;
all 1,061 archive/frozen/QA/current hashes match. QA ran before committing
those exact bytes; it is not relabeled a postcommit run. All five current user
note identities remain unchanged. Independent narrow commit/archive review
closes GO for these actual committed bytes, without another QA invocation.

Local records under `.git/baozi-review/performance-convergence/` are:

- `single-piece-parser-source-{1,2}.json` and
  `single-piece-parser-before-format-1/manifest.json`.
- `single-piece-parser-check-source1-attempt1/report.json` (failed fixture
  compilation) and `single-piece-parser-check-source2-attempt1/report.json`
  (PASS).
- `single-piece-parser-test-source2-attempt1/report.json` (1,403 PASS).
- `single-piece-parser-integration-source2-attempt1/report.json` (665 PASS).
- `single-piece-parser-lint-source2-attempt1/report.json` (affected strict PASS).
- `single-piece-parser-final-source2-attempt1/report.json` (all seven PASS).
- `single-piece-parser-plan-review-1.md` and
  `single-piece-parser-production-source-review-1.md` (bounded source GO).
- `single-piece-parser-test-and-format-review-1.md` (test/format binding GO,
  with the original Parse fixture's narrow actual-runtime closure).
- `single-piece-parser-final-qa-review-1.md` (terminal exact-WIP/staged-byte
  QA GO, preserving the earlier pending checkpoint).
- `single-piece-parser-staged-source-1.json` and
  `single-piece-parser-source-commit-association-1.json` (actual nine-path
  staging/commit/archive identities).
- `single-piece-parser-source-commit-review-1.md` (actual committed association
  GO; inherited QA is not relabeled a postcommit run).

## Actual fixed instruction screen

Fresh matched Rust1.95 GNU opt3/debug1/System workers are built serially in
the current repository's existing target. Both actual compiler, argument and
environment vectors match. Empty consumer features, generic CPU and absence
of instrumentation/flag overrides remain verified. Symbol workers are not
ordinary timing workers.

| Role | Source | Actual ELF SHA-256 |
| --- | --- | --- |
| Control | `0452a73e66bd5d8e002d3b2850502423efd7875a` | `959f95f5f5fbe6b8fe5c43ee061cdc5d9e8a5b59f8040f91a3aa9aa46c6d35bf` |
| Candidate | `2bdc37a28bd00fffe8c70b4bd4b8ad28a769a227` | `6d763eedfbace25176204f7cbbc521a5a9a7cf8fc5c1599f748e88f28c638477` |

All fourteen fresh workers and twenty-eight annotations complete successfully:
42 commands, seven original pipeline/memory tasks. All seven complete literal
non-clock output pairs match after removing only `stage_ns`, including work,
output and numeric spelling/type.

| Original task | Control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| glTF1 | 5,621,852 | 5,652,018 | +0.53658474% |
| glTF32 | 25,281,590 | 25,430,035 | +0.58716639% |
| F64 PLY neighbors | 2,389,705 | 2,386,566 | -0.13135513% |
| Binary STL1024 | 12,107,371 | 12,133,432 | +0.21524904% |
| Repeated OBJ16384 | 711,709,601 | 710,699,216 | -0.14196591% |
| Grid OBJ128 | 2,107,466,996 | 2,100,255,910 | -0.34216840% |
| Binary STL1 | 2,030,327 | 2,025,468 | -0.23932106% |

The unchanged integer predicate fails: neither original OBJ satisfies
`100*candidate <= 99*control`. All seven independently satisfy
`100*candidate <= 101*control`, and literal equality passes. Those two passes
do not substitute for the required gain. Collector exit0 and collection
`completed` therefore coexist with `instruction_predicate_passed=false`.
The [complete fourteen-row export](2026-11-09-single-piece-parser-instructions.csv)
preserves both identities and every increase. Whole-worker Ir includes setup,
validation, observation, serialization and teardown; it is not native time,
stage CPU, RSS or an Assimp instruction comparison.

Both grid workers retain their `brk segment overflow in thread #1` warnings,
at `0x5418000` and `0x53b0000`, plus the manual limitations and suppressed
further-instance notices. Matching warning classes do not establish equal
native allocation behavior. Raw unresolved annotation labels remain retained.
Independent actual evidence review closes fidelity GO and selection FAIL.
All fourteen profiles, 42 actual commands, 84 streams, seven literal pairs,
112 CSV data cells and 99 archive members are verified. Every annotation's
46 rows containing `???` remain.

Independent matched generated-code review also closes its separate condition
with GO. In the exact candidate ELF, the actual OBJ and MTL storage callbacks
execute in their original host's piece bodies at `0x364c60` and `0x3658e0`.
Their literal returned amounts reach the original frame, then common finish
and exact close; the former nested single-piece guard/runner is absent from
those concrete paths. The remaining compound-route guard is legitimate.
Every token still performs original admission, physical metered insertion,
numeric prefix settlement, exact close and prescribed exit control. A real
layer removal therefore does not establish a large removable cost. This
witness does not quantitatively attribute the measured deltas to individual
sites and cannot rescue the failed selection predicate.

After all collection clocks end, all 99 artifact files are retained unchanged
both in the original target directory and in the private compressed archive
`single-piece-parser-instructions-1-raw.tar.gz` (9,044,029 bytes, SHA-256
`7733e813a37db160895b1eb10b440c19b31a2a33e891640b9fbf5231f1662390`).
The original report SHA-256 is
`0981f5e68a2f4d4a59260b53c0bc30974e9e321da48857808da1edf6c0f53b10`.

## Retirement and remaining convergence

The gain does not earn the new route and public adapters. Retirement commit
`84f68b8e1482bd4d63d10fdadcbc6363ff9aa5b3` applies the exact inverse of the
nine candidate Rust paths, including the candidate-only API fixtures. All
original baseline source and assertions remain byte-identical. Every one of
the 1,059 actual committed-archive inputs equals qualified `0452a73` and its
`late-edit-error-slot-final-1/report.json` input map; no additional crate Rust
or TOML input remains. Its existing seven-gate qualification is reused by
exact source identity, not claimed as a new post-retirement test run. That
baseline invocation has 3,963 passing workspace tests with six existing manual
skips and 138 passing consumer tests with one existing manual skip.
Independent retirement review confirms the exact inverse nine-path blob/mode
delta, all 1,059 committed/current input identities, the 760 crate Rust/TOML
paths with no additions or omissions, and all five user-note identities.

The final bounded review identities under the private evidence root are:

| Actual review | SHA-256 |
| --- | --- |
| Instruction/equality/export/archive fidelity | `d225f632751df61ddbac2c99d0135435cbcf9b87fb285d1697e0fa0d936e46c6` |
| Matched generated-code condition | `4243286129e513bb391f2be2377c6f58d01f61a32ccb5e07e77158ec817a82e3` |
| Retirement identity/association | `9e952dd77713a66ba5c73014cefeac5ec7f79b4b74da102501b0c7ea6ceaefa8` |

The candidate commits, its passed source qualification and failed screen remain
history. Prepared ordinary/heap/export scripts are unexecuted, unused private
artifacts; the native-launch prerequisite is never created. No ordinary or
heap result is inferred for this failed experiment. The next cost owner must
be selected independently, without changing this screen or preserving the
unhelpful route merely to support another optimization.

Original Windows/GNU fourteen-case/110-lane 3+20 native comparisons, comparable
whole-child memory and separate heap remain necessary after a selected
candidate. All source/builds use current-repository target with Rust1.95,
jobs1 and nextest threads1. The rejected `d62cf67` experiment remains isolated
on its earlier branch; none of its source or acceptance is inherited. Main
runtime integration and release remain HOLD.
