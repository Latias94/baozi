# Single parser pieces execute through their original host

Status: source-only implementation `2bdc37a28bd00fffe8c70b4bd4b8ad28a769a227`
from qualified baseline `0452a73`.
Independent five-file production source review closes GO before formatting.
Frozen source2 passes affected all-target/all-feature checking and all 1,403
Operation tests with zero skips, all 665 Import/OBJ tests with zero skips, and
affected strict Clippy. Independent test/format binding review closes GO,
including its initially unknown original-ParseFailure fixture after actual
runtime evidence. All original seven integrated gates pass on the same source2
inputs. Independent terminal-receipt and exact staged-byte review closes GO.
The committed archive matches all 1,061 qualified inputs; independent narrow
commit-association review remains pending. No measured source-retention,
codegen, native or release acceptance is claimed at this checkpoint.

## Actual owner and consumer change

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
note identities remain unchanged. The narrow independent commit/archive
association review is the remaining source prerequisite.

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
  staging/commit/archive identities; independent commit review pending).

## Remaining measured decision

Complete independent narrow commit/archive review for the source-qualified
implementation and passed original seven integrated gates.
Then inspect actual generated code for the migrated call sites and run the
fixed seven-task diagnostic screen: all literal non-clock pairs equal, at
least one original OBJ Ir decrease of at least 1%, and no task increase above
1%. Source structure or passed tests do not establish that screen.

Original Windows/GNU fourteen-case/110-lane 3+20 native comparisons, comparable
whole-child memory and separate heap remain necessary after a selected
candidate. All source/builds use current-repository target with Rust1.95,
jobs1 and nextest threads1. The rejected `d62cf67` experiment remains isolated
on its earlier branch; none of its source or acceptance is inherited. Main
runtime integration and release remain HOLD.
