# Borrow Immutable Parse Metadata from the Invocation Root

This candidate keeps immutable parse metadata in `ParseRoot` and lends it to child,
parser and service roles. It removes the private metadata's `Clone`/`Copy` path and
the fifteen expanded metadata fields in `ParseServices`. Numeric invocation ownership,
work/resource arithmetic, cancellation and exit checkpoints remain unchanged.

The exact source2 passes the seven current-root correctness/consumer gates below,
and final independent source/evidence association finds no actionable mismatch.
No ordinary measurement of this candidate or performance keep follows from those
results. The complete
[convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md) and
[lexical-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md)
remain active and HOLD.

## Recorded Cost and Scope of the Hypothesis

The preceding [full Windows comparison](2026-11-05-outer-host-windows-comparison.md)
retains the repeated-OBJ benefit and unresolved glTF, whole-child time and memory
costs. Fresh Linux instruction attribution independently checks twelve matched
whole-worker profiles for control `4c22eae` and integrated source `67e9d1c`.

| Case | Control whole-worker Ir | Integrated whole-worker Ir | Change |
| --- | ---: | ---: | ---: |
| Shared glTF 1 | 6,213,424 | 6,244,333 | +0.50% |
| Shared glTF 32 | 32,858,682 | 31,833,318 | -3.12% |
| F64 PLY neighbors | 2,658,476 | 2,514,877 | -5.40% |
| Repeated STL 1024 | 34,229,972 | 27,440,619 | -19.83% |
| Repeated OBJ 16384 | 997,741,583 | 875,739,369 | -12.23% |
| Grid OBJ 128 | 3,190,339,399 | 3,235,431,365 | +1.41% |

Grid's memcpy exclusive Ir grows by 102,388,000 while all other exclusive Ir falls
57,296,034, giving the actual net increase of 45,091,966. Its largest direct memcpy
caller is `scan_token_ranges`: 3,529,324 calls and 105,879,720 callee-edge Ir. The
memcpy leaf has no outgoing calls in these profiles, so its incoming edges sum to
its exclusive cost. That equality cannot be applied to arbitrary callees or added
again to their parents' inclusive costs. Calls and instructions are not copied bytes.

The source repeatedly constructs child/parser/service roles containing the same
metadata references and policies. The candidate replaces those aggregate copies
with a root metadata loan. The attribution does not establish the exact Rust value
behind every libc copy, or prove that this change explains every observed cost.
Fresh matching ordinary measurements are required to evaluate the hypothesis.

The independent Linux report is
`.git/baozi-review/performance-convergence/outer-host-linux-attribution-review-1.md`,
with a complete companion JSON. Raw totals, exclusive sums, `Collected`, `I refs`
and both annotations reconcile for all twelve profiles; complete raw and outgoing
edges agree. Incoming annotation trees are incomplete except for the separately
verified memcpy edges. Both grid stderr files report Valgrind `brk segment overflow`;
successful exits and matching semantics do not make these native allocator or
memory-pressure measurements. Whole-worker Ir includes setup, observations and
teardown beyond the endpoint clock, and cannot explain Windows glTF native timing
by itself.

## Ownership and Preserved Behavior

`ParseRoot` owns one non-Copy `ParseViews`. `ParseChildren`, `ParseParser` and
`ParseServices` carry a shared metadata reference beside their existing unique numeric
loan. The aggregate contains immutable references, existing shared-state references
and the selected Copy policies; it contains no numeric owner or physical allocation.
No cache, allocation, `Arc`, unsafe code or second accounting implementation is added.

Service methods extract the original issuer, terminal, callback cell, failure storage,
source, policies and evidence cells through that reference. Session data retains its
original session lifetime. The metadata loan shortens with its role, while the numeric
brand and mutable invocation loan still exclude capability escape and reentry.
`StoreInvocationParser` continues to contain only a mutable `ParserHost` reference.

Child and parent admission, callback slots, work amounts and item limits, preflights,
prescribed exit checkpoints, owned error mapping and format-failure capture order
are unchanged. The former copied metadata had no destructor; replacing its copies
with references adds no destructor action around caller captures or rejected values.

## Frozen Source and Public Contracts

The two-file candidate is frozen at
`.git/baozi-review/performance-convergence/parse-metadata-borrow-source-2/`.
Baseline is `c8c374909497ef0931e539e086b15e0faabdaf9a`, whose runtime is `67e9d1c`.

| Source2 input | SHA-256 |
| --- | --- |
| Manifest | `0d74a0707524826af6bced27fb533045600828c1379830bd22fe87a70c774524` |
| `crates/baozi-import/src/scope.rs` | `537c962fd2057eb6772e3fab8e2eb815e9b10aa7797a2c754115ebfbf31c0984` |
| `crates/baozi-import/src/scope/parser_execution.rs` | `cc91a925835e56c8d573cd0224b8e076fd1c1b059da2e8663f80ec07024dfd23` |

Independent source review 1 finds no concrete P1/P2 defect in source1's production
delta. Source2 adds only 52 documentation lines with three compiler contracts;
all non-documentation production lines remain byte-identical. The five new public
examples cover root session-byte return, root-service escape rejection, parent/child
session-byte return after short service loans, and exclusion of parent and child
reentry while each service is live. Compiler execution and source association remain
separate requirements from the static review.

Reports are retained as `parse-metadata-borrow-source-review-1.md` and
`parse-metadata-borrow-source2-qualification-review-1.md` under
`.git/baozi-review/performance-convergence/`.

## Qualification History and Current Status

QA1 rejects the nextest CLI before any runtime test: its `--jobs` alias duplicates
`--test-threads`. QA2 uses `--build-jobs 1 --test-threads 1` and returns zero for seven
commands, but root-workspace test artifacts still reference the old isolated source
directory. Its 3,565-pass runtime result is excluded from candidate qualification.
New rustdoc text against that cached library is also insufficient for this candidate.
The unchanged tracked-file maps do not repair the artifact-source mismatch.

QA2's independent asset consumer is narrower actual evidence: it freshly compiles
current main into its existing consumer target and passes 138 tests with one existing
skip, run `2cfe8a6e-2b19-422d-8226-6c2908366323`. This is not source2 workspace
qualification and is not relabeled as such.

After QA2's parent exits zero, 2,233 Baozi-only debug fingerprint directories are
moved to `target/review/convergence/parse-metadata-borrow-fingerprint-history-1/`.
All target paths are checked, all 8,696 files are preserved with hashes and independently
verified, and no source or retained worker is removed. QA3 explicitly uses the root
target for both workspaces, forcing current-root compilation while retaining dependency
artifacts. Newly produced Import dep-info names main's parser sources and contains no
old isolated-source marker.

QA3's parent session `74942` actually terminates with exit zero. It uses Rust 1.95.0,
locked/offline dependencies, one build job and one nextest thread. The receipt is
`target/review/convergence/parse-metadata-borrow-qa-3.json`; all 1,448 tracked input
hashes remain unchanged through all seven gates.

| Source2 gate | Actual result |
| --- | --- |
| Whole-workspace runtime | 3,565 passed; six existing skips; run `bf7f1fcb-8383-4c8a-ac19-ac9429c1fb7d` |
| Whole-workspace doctests | 188 passed, including all five new service contracts |
| Workspace strict Clippy | Passed, all targets/all features, warnings denied |
| Independent asset consumer | 138 passed; one existing skip; run `bf951e13-52d1-49fc-bd0b-a598a58a69d3` |
| Asset-consumer strict Clippy | Passed, all targets/all features, warnings denied |
| Workspace and consumer formatting | Both passed |

The workspace test build takes 16m22s before running the newly associated binaries.
The consumer also freshly compiles current main against the root target. New Import
dep-info and actual library artifacts supply current-root association, rather than
the existence of unrelated historical files in the same target directory. The five
new rustdoc entries occur at source2 lines 604, 615, 883, 906 and 920. This proves
their positive acceptance or negative rejection; no separate captured exact diagnostic
code is claimed from the fence text. Original QA1/QA2 logs and receipts remain retained.

The source2 static follow-up draft verifies the documentation-only delta and historical
fingerprint preservation, but its reviewer stops before final QA association. It is
not promoted into a completed evidence review. The separate final audit,
`.git/baozi-review/performance-convergence/parse-metadata-borrow-final-evidence-review-1.md`,
checks the actual terminal receipt, all seven log hashes, the complete input maps,
five service contracts and fresh compiler association without an actionable mismatch.
The QA3 receipt SHA-256 is
`c1c18de35b8ca56d94c718bdc8651110557925e14a0a831b755407ec520084ec`.
The audit is bounded to the executed configurations and retains all existing skips;
it is not a compiler/code-generation attestation or arbitrary consumer qualification.

## Measurement and Remaining Architecture

Before candidate build or measurement, a finite C0/N0/N1/C1 campaign decision fixes
the original fourteen cases, 110 lanes, three warmups and twenty retained AB/BA pairs
per lane. Control is the immutable ordinary `67e9d1c` worker; the candidate must be
the qualified committed source2. All campaigns, costs, outliers and observations are
retained. Tool overlap remains descriptive/HOLD, without an outcome-selected rerun.
The 20-second watched-tool quiet start has a prospective 180-second wait cap.

Ordinary release construction must also retire stale Baozi release fingerprints and
verify fresh Import compiler output/dep-info before retaining the executable. This
prevents debug cache repair alone from being mistaken for ordinary build identity.
No new ordinary result or performance keep exists yet.

Fixed/dynamic borrowed backing, shared-shell-before-state promotion, exact administrative
accounting, remaining `LocalWork`/owning-path deletion and indexed selection remain open.
The concrete remaining-owner map identifies typed-table producer envelopes as one live
connected migration/deletion boundary, while escaping storage leases remain distinct
from numeric work permits. Full current-source Windows/Linux convergence, separate heap
attribution and all final correctness/consumer gates remain required before release.
