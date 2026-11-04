# Prepare Before Attaching Retained Write Resources

The connected successor runs actual triangle preparation and space-conversion
classification on the original invocation before attaching retained write resources.
Unchanged and early-failure paths finish through their actual original-account snapshot;
changed paths attach to that same work/control history and keep the existing canonical
validation, publication and fallback route.

This is an intermediate implementation of the
[lexical-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
The complete [performance convergence goal](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md)
remains active and HOLD. No native timing or comparative-memory result follows from the
correctness checks below.

## Original Account, Cleanup and Direct Facts

`CanonicalBoundInvocationFinish` keeps independent host and original-account lifetimes.
An unattached request moves the original admission metadata outside the retained body,
while common/control and numeric state stay in the physical root. The real callback and
its captures are disposed before final authority is returned. An attached-only consuming
conversion ends the external account loan without weakening the host loan.

Direct completion checks the original account before terminal control and again after
control-extension disposal. It then consumes the existing coordinator and `Ledger::finish`.
`StoreLedgerEvidence` owns either genuine retained evidence or the actual unattached
snapshot. Dirty work/storage facts are preserved, including failures; they are not
normalized to zero to manufacture success. No replacement account, start admission,
ledger or numeric backend is introduced.

Postprocess now executes real `input::prepare` and checks the returned prepared values
before `attach`. The unchanged path drops prepared scratch before requesting direct
completion; failure retains its actual typed cause and resource evidence. The space
entry similarly resolves and classifies the real conversion before attaching. Changed
preparation keeps the same real scratch across current-state attachment and releases it
through the existing outer cleanup/publication route.

## Tangent Allocation Repair

Moving preparation exposes an actual tight-budget defect: tangent generation allocates
its full output and fills it with zeros before reading/computing its first triangle.
The preserved construction run reports attempted work 25 against ordinary limit 24,
consumed work 23, and staging peak 16,778,120 bytes. This exceeds the existing fixture's
14,545,488-byte bound; its explicit total allowance remains 150.

The repair removes the redundant output-zero pass. The first face's existing reads and
numeric kernel must succeed before output reservation/allocation; each face then appends
its three computed tangents in original corner order with the existing metered writes.
F32/F64 source selection, scaling, expressions and read/kernel order remain unchanged.
Deleted physical zero writes honestly reduce work; no future work is precharged and no
allowance is raised. The original memory, source, generation and full-snapshot assertions
now pass with the exact nested tangent work-limit cause.

## Frozen Qualification

Final source3 contains 1,421 files, source-set SHA-256
`3f6abf2ab64c2a2be9973b10eb283216a52559839a9de9cccf517074cab4cdd1`.
Its manifest is `target/review/convergence/pre-attachment-postprocess-source-3.json`.

| Final source3 check | Actual result |
| --- | --- |
| Whole-workspace runtime | 3,565 passed; six existing skips; run `06ff9931-178a-4d89-bb35-0cf18fa11c79` |
| Whole-workspace doctests | 183 passed, including 128 Operation cases |
| Workspace strict Clippy | Passed, all targets/all features, warnings denied |
| Independent asset consumer | 138 passed; one existing skip; run `6eb2d8c8-8cd4-4745-9871-a59e8a95927f` |
| Asset-consumer strict Clippy | Passed, all targets/all features, warnings denied |
| Workspace and asset-consumer formatting | Both passed |
| Independent Scene consumer | Public contract passes with all features; empty and OBJ/Postprocess isolated runs pass; strict Clippy and formatting pass |
| Fuzz package on Windows | All six targets compile; 16 deterministic library regressions pass; formatting passes |
| Registry detection on Linux | GNU target check and 256-run AddressSanitizer smoke pass on the exact pinned nightly commit |
| Explicit compiler probes | Four positive examples pass; five fail only with their intended coded errors |
| Original default-stack CLI probes | All eight stage/IO cases pass; retained executable preserves 1 MiB PE stack reserve |

All Windows Cargo checks run serially with Rust 1.95.0, locked/offline dependencies, one
build job and one nextest test thread. Complete before/after source maps and original
logs are retained. The seven-check terminal association recomputes all frozen/live hashes,
fourteen maps and seven logs; the actual parent `exec83752` exits zero.

The compiler probes use separately retained rlib/dependency copies, with no shared-target
search path. Their exact rejections are E0505, E0499, E0277, E0505 and E0597 for host drop,
host replacement, thread movement, account drop before attached conversion, and account
scope escape. The earlier receipt's shared-target rlib was later replaced by builds;
that historical identity is preserved rather than silently relabeled. Fresh retained
copies supply the present proof.

Default-stack probes use a separately retained consumer executable. All non-timing JSON
fields equal the earlier qualified eight `gltf-shared-8` stage/IO observations, and PE
stack reserve/commit remain 1,048,576/4,096 bytes. These are functional build observations,
not portable stack bounds or ordinary performance measurements.

The Linux registry smoke uses cargo-fuzz 0.13.2, nightly commit
`92ffebeceaa0a071066f233873c2438e73d70fe7` and LLVM 23.1.4, matching the repository pins.
The initial command selects an unavailable musl target and fails before target execution;
that environment failure remains recorded. An explicit `x86_64-unknown-linux-gnu` retry
passes type checking and all 256 ASan runs, with every source and lockfile hash unchanged.
This is a bounded smoke from an empty registry corpus, not sustained fuzz coverage or
qualification of all format fuzzers.

## Failure History, Review and Remaining Scope

The original compiler failure, 293-pass/four-failure construction run and source2's
successful runtime/docs followed by `drop_non_drop` lint failure remain retained.
Source3 removes only that redundant explicit drop from a reference/identity-only type;
it has its own complete runtime/docs/lint results. Retained-terminal record expectations
become exact zero only for actual direct outcomes, preserving the original cause,
source-chain, nonzero peaks and cleanup assertions.

Independent reports under `.git/baozi-review/performance-convergence/` cover raw
unattached ownership, actual callers/compiler docs, the tangent/fixture delta, the
one-line lint repair and final source/evidence association. The final evidence report is
`pre-attachment-postprocess-final-evidence-review-3.md`; it verifies the complete
workspace/asset cut and retained compiler/default-stack artifacts without a mismatch.
`pre-attachment-main-integration-readiness-1.md` permits intermediate integration once
the separately identified standalone gates qualify. Neither report claims a performance
keep or completion of the final architecture.

Direct coverage remains bounded for combined account quarantine/destructor causes,
forgotten finishes, derived/cross-root completion, dirty pending/admin projections,
second-region control rejection and partial tangent-append denial. Existing tests and
source traces are not promoted into execution evidence for those combinations.

Final progress can still allocate dynamic storage; empty scratch still requests a real
zero-byte token, and single selection still allocates temporary vectors and walks the
Scene. Fixed/dynamic borrowed backing, shell-before-state shared promotion, exact
administration accounting, remaining LocalWork/owning-path deletion and indexed
selection remain open. The unchanged full Windows/Linux 14-case/110-lane time,
whole-child peak memory and separate heap gates still determine release acceptance.

## Main Integration for Measurement

After the standalone checks and Linux registry smoke pass, the reviewed overlay is
applied to main: 258 Rust files and two required embedded compiler-documentation files.
All 260 transferred files match frozen source3 byte-for-byte. No file is deleted and
no Cargo dependency/feature policy changes. Current main's glTF FLOAT direction repair,
newer plans/reviews and unrelated user notes are preserved. Main-root all-target/all-feature
strict Clippy and formatting checks pass; the complete runtime/consumer evidence above
belongs to the identical frozen code inputs, not a second main-root runtime execution.

The exact pre-integration control is `4c22eae585ee1e151e7521333d4d5a0a4d163638`, including
the current FLOAT direction check. Its ordinary uninstrumented Windows worker and source
archive are retained under
`target/assimp-comparison/convergence-windows-late-attachment-control-build/`.
Historical `314d0` workers remain historical. The next step builds and measures the
integrated candidate before selecting further representation changes; integration itself
does not approve a performance keep or complete the lexical-host plan.
