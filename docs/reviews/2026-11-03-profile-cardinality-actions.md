# Constant-Time Profile Payload Cardinality

`ProfileMetrics::add_payload` receives an already derived cardinality but previously loops over
that count, increments a metric and checks a limit once per cell. It reads or validates no cell
in that loop. Replace it with one bounded arithmetic action for a positive count. This is an
owner-level algorithm change within the
[Assimp convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md), separate from
exclusive caller adoption, generated-payload ownership and the rejected node-view experiment.

## Arithmetic and Work Contract

Logical payload-cell totals and limits retain their meaning. First check the next successor
for overflow and limit, then compute the justified remaining room. A fitting increment succeeds
in one checked update. A crossing retains the exactly derivable logical prefix through the
maximum and reports its first rejected successor. This is logical cardinality arithmetic,
not a claim that those cells were visited.

Checking only the final `p + count` is incorrect: with `p=MAX-3`, maximum `MAX-1` and increment
10, the old first semantic failure is limit actual MAX, before a later overflow. Conversely,
maximum MAX can accept a representable prefix and then overflow. The replacement retains both
outcomes, exact error dimensions and unchanged public projection. Zero keeps the old no-op
behavior even for a directly tested already excessive metric.

The breaking `SceneValidationWork::PayloadCardinality` category means one fixed-step metric
update. Positive calls enter the existing semantic runner once; zero enters neither the runner
nor a poll. Real metadata/value visits still use `PayloadCell`; source/reference, ordered
parent/evidence, finite checks, capacities, canonical bytes and scratch work remain. Numeric
budget values are unchanged. Removed dummy work and its old poll/work ordinals are not replayed
as N fabricated completed units.

The unchanged runner preserves local work eligibility before external admission, real body
before immediate completion, local successful-return accounting, due poll, then semantic-error
conversion. A normally returned limit/overflow error completes the arithmetic action. Denial
executes no body; an unwound body completes none. A due control error still wins over that same
returned semantic error, preserving any completed metric write. Entry/final and genuine long-walk
checkpoints remain, with at most 256 actual bounded actions between cooperative checks.

## Scope and Qualification

The frozen isolated five-file patch is SHA-256
`1eac8372378242a1b5537032a21a555c475d38e9f8fb23206a2e8c86b33437ed`, based on
`45a09d19d948600d9b2a4261eb2e3394fe594ef1`. Independent design and source reviews find no
actionable P1/P2. Its application base is `24c533e`, after the four-path node experiment is
reversed and the runtime is checked byte-identical to `45a09d1`. No older whole-tree copy
replaces later operation/glTF/consumer work. The old count-only loop is removed completely;
the existing metric arithmetic/error helpers and semantic runner remain the owners.

Ten new tests and two extended existing tests provide literal MAX, inclusive/crossing,
first-limit-before-overflow, zero-control, denied-prefix, action-256 semantic-error/due-poll,
reference-before-count, logical-cell and actual metadata-visit oracles. Huge input is tested
at the arithmetic owner without allocating a huge source. Existing real operation-session
admission, returned-error, panic and capture-Drop tests verify the common execution boundary.
No substitute ledger, general counting framework or increased allowance is introduced.

Actual formatting/compiler integration adds one existing test-file correction. Its node-only
profile has a Generated operation with no parents/evidence: kind 1 plus fixed 4 cells formerly
uses five dummy actions and now one cardinality action. The completed authoring prefix is
therefore 80-4=76. At the stale limit 80, later materialization gets through 79 actions before
its next batch cannot be admitted; changing the expected result to 79 would move the intended
test boundary. The correction changes the named prefix to 76 with a derivation comment,
preserving every test body/assertion, rich Ledger failure and complete release requirement.
Independent delta review accepts this sixth test file without runtime or allowance changes.

The first affected run is retained as 1429/1430 passed, one failure and six existing skips,
`253f2ab1-40bd-41f3-8b5a-03c99fa907d3`. It is not relabeled as a pass. Windows follow-up checks
use Rust 1.95.0, locked/offline dependencies, one Cargo job and the shared target:

| Check | Result |
| --- | --- |
| Profile arithmetic/control tests | 30/30, 657 other unit tests filtered; `371760e3-75fa-4b30-bd9a-1ee5174a3bde` |
| Real operation semantic-session boundaries | 4/4, 901 other tests filtered; `469a5f36-8950-4487-bca9-a97ef02e0577` |
| Corrected materialization-entry boundary | 1/1, 686 other unit tests filtered; `cd6d177c-0219-4ff8-8fcd-b3a2da035f68` |
| Core/import/postprocess all-feature nextest | 1430/1430, six existing skips; `66db3a4a-3c45-4d51-98bb-91e430d3d6bb` |
| Core/postprocess doctests | 6/6 and 4/4 |
| Independent asset consumer, all features | 138/138, one existing manual skip; `09b2d4ee-365f-4f51-924d-2dd151221e9f` |
| Strict affected and consumer Clippy | Passed, all targets/features with `-D warnings` |
| Core formatting | Passed |
| Complete glTF/OBJ/PLY/STL suites | 853/853, no skips; `7f99d84c-f160-4519-b8ed-6fc9f44be624` |

Focused runs are subsets of their complete suites, not additional tests to add to the total.
Five-file post-format review and the separate boundary delta review find no actionable P1/P2;
the former frozen report is not overwritten. Evidence is retained under
`target/review/convergence/profile-cardinality-*`; source design/packet/review artifacts are under
`.git/baozi-review/performance-convergence/profile-cardinality-*`.

The final six-path identity-set SHA-256 is
`9bb7159f4fa1dc1602f4e72750a9bd4ea6607651627ac7bb0ec8bd53973305f3`, using sorted UTF-8
`path + NUL + lowercase raw SHA-256 + LF` with forward-slash paths. The eight final-source
checks pin those same six paths. The focused profile/operation checks precede the test-only
boundary correction and pin the unchanged five paths; the receipt distinguishes these scopes.
The ten-check aggregate `profile-cardinality-qualification.json` has SHA-256
`b88c27b47330b0488520af011b5f3999b1e598e8d47a8eaa35dac69155d059ed`.

## Measurement Gate

The earlier glTF profile attributes 1,646/2,381/4,925 count-only actions to 33/47/95 calls at
scales 1/8/32. This source predicts one action per positive call while keeping all real visits.
Those owner envelopes overlap exclusive-runner overhead and cannot be added as savings. No
ordinary speedup, instruction reduction, memory gain or release convergence is claimed until
the actual qualified source receives its own immutable same-task measurements and review.
The parent Windows/Linux 14-case/110-lane all-endpoint/pipeline/memory 3x gates remain open.
