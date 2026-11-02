# Parser Completion Borrow Consolidation

Implementation `bb6972c4490039e6835b3bc8934d7039367f324c` replaces two consecutive local
state borrows in `ParserWorkScope::complete` with one. Active-owner/role/channel validation and
the existing atomic numeric kernel execute in that short borrow; no physical action, allocator,
dimension comparison or user callback executes while it is held. Ordinary direct groups use
the same extracted kernel. All error ordering, pending-consumed checks, atomic duplicate-group
completion and exact remaining-prefix updates remain required.

This is the first small measured candidate after the
[current attribution](../research/2026-11-03-current-import-cost-attribution.md), not completion
of P5. It neither reduces the remaining number of sort comparisons nor changes cancellation
cadence or physical work evidence.

## Correctness Qualification

Operation nextest passes **852/852**, run `60bc1b47-482d-4a67-832a-c729a329f3ce`.
All-target/all-feature operation Clippy passes with `-D warnings`; package formatting passes.
The independent scoped source review reports no actionable finding. It checks the two exact
callers, privacy/owner identity, callback-free borrow, zero/empty groups, parent/child exclusion,
duplicate aggregation, overflow, tightened policy, pending headroom and failure cleanup.
Existing direct numeric oracles cover these obligations; no implementation-mirroring test is
added for the extraction. Broader dependent qualification belongs to final integrated U14.

## Ordinary Windows Measurement

Both workers use the same release/System allocator/no-instrumentation policy and the unchanged
ordinary collector/corpus. Each of six memory lanes retains twenty AB/BA pairs after three
warmup pairs: 120 retained pairs and eighteen warmup pairs, all semantically successful.

| Lane | Before Baozi median ms | After Baozi median ms | Before Assimp median ms | After Assimp median ms |
| --- | ---: | ---: | ---: | ---: |
| Repeated OBJ 16,384 import | 58.369 | 54.943 | 10.005 | 7.540 |
| Repeated OBJ preparation | 129.594 | 129.535 | 1.084 | 1.059 |
| Repeated OBJ pipeline | 191.835 | 189.636 | 9.540 | 9.432 |
| Grid OBJ 128 import | 300.984 | 287.995 | 23.541 | 23.298 |
| Grid OBJ preparation | 0.145 | 0.136 | 0.029 | 0.031 |
| Grid OBJ pipeline | 315.371 | 304.038 | 25.016 | 25.248 |

The grid import/pipeline improvements are about 4.3%/3.6%, with stable same-lane Assimp
controls. Preparation of repeated geometry is unchanged, consistent with this parser-only
change. Its import control moved substantially, so that lane does not isolate the numeric
change cleanly. The short no-change preparation lane is descriptive. No independent endpoint
medians are summed/subtracted as a CPU breakdown, and no cross-host comparison is made.

Retain the simpler single-borrow implementation. The remaining large gap justifies the
separately reviewed OBJ projection and normal-ordering candidates. Keep the bounded
physical-prefix/control contract intact while evaluating those algorithms.

The complete report is `target/assimp-comparison/Windows-p5-numeric/report.json`; its raw
samples include dispersion and whole-child working-set/commit observations. This is neither
stage heap nor Linux RSS. The immutable ordinary worker SHA-256 is
`3e095156c9397c4823cc403020a988b4dd8540562e0af298bf843ba86fad92ca`, with source/build
identity retained in `Windows-p5-numeric-build/build.json`. Source/test/review/report archive
identities are retained in `target/review/p5-numeric-evidence.json`.

| Artifact | SHA-256 |
| --- | --- |
| `Windows-p5-numeric/report.json` | `33466a1d018c7e2e49103ee8f97f85570d914fd18747c9430bb791983ab28b09` |
| `target/review/p5-numeric-evidence.zip` (581 files) | `6760bc2e79464bc57d5469eb90bd827da1c8e2b1b303c69789eb793e44885458` |
