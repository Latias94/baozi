# Lexical Root Fields and Connected Callers

This isolated checkpoint moves actual retained operation and publication-assembly state into the
caller's lexical root and connects the existing core, postprocess, import and format callers. It is
a bounded implementation step in the [lexical host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
It does not complete that plan. No isolated Rust changes have been applied to the main checkout,
and there is no new ordinary performance keep or release-readiness claim.

## Actual Ownership Change

`InvocationRoot` owns the real fallback baseline, terminal plans/capability, permit flags, publication
tables, attribution batches, account and validation metadata. Operation and assembly permissions
borrow these fields. An assembly's remaining nested operation value holds references rather than
the final physical owners. Forgetting a permission leaves those owners in the root.

Root cleanup reuses the checked table-before-attribution path. Containment stops and seals ordinary
work and detaches physical baseline owners, preserving uncertain charges without forgetting the
borrowed invocation. Startup installs its actual coordinator/control before invoking the fallback
and evidence builders. Invocation permissions are explicitly neither Send nor Sync; independently
released physical storage keeps its separate thread-safe contract.

Both import entries hold their actual roots outside `ImportSession`. `ParseContext<'session, 'inv>`
keeps the root borrow distinct from request/source lifetimes. Core, postprocess, four formats,
facade, standalone consumer and fuzz callers are adapted without changing parsing, normal
mathematics, benchmark tasks or clocks. The three qualified glTF FLOAT-direction files retain their
exact `0484dcb` hashes.

## Source Identity and Qualification

Both complete isolated snapshots contain 1,404 files, with 180 changed paths relative to qualified
checkpoint 5. Source-set hashes encode sorted UTF-8 relative slash path, NUL, lowercase file SHA-256
and LF. Source3 differs from source2 only by six explicit root-drop statements in five core test
files; all production and other test bytes are identical. Independent review reproduces that exact
delta and confirms the unchanged assertions, budgets and test attributes.

| Snapshot | Source-set SHA-256 | Manifest SHA-256 |
| --- | --- | --- |
| Source2 | `ebb575702cf6bfe6b09573288a50d1e8e0d99d9eb0022b6a0b92138e9efc0411` | `3d041bfa09a0f84bb84f62d418ea41faff509e56222eec41572104f3276f5094` |
| Final source3 | `0e8e22e18c27cf663009808471c5c784eb8e9b71460a79c0c54a778edf3eeb88` | `a509a99f1dc2af52937339eafc574278f4dc6278d14ab1afe294f674424ff7c5` |

Windows checks use Rust 1.95.0, locked/offline dependencies, one build job and one nextest thread.
Every mapped runtime, rustdoc, consumer and format-check receipt verifies all 1,404 source hashes
before and after execution.

| Actual check | Source and result |
| --- | --- |
| Workspace, all targets/features, compiler | Source2: PASS, zero warnings. |
| Complete workspace nextest | Source2: **3,413 passed**, 6 existing skips; run `d3036d69-4886-4972-ab9b-495821f33d42`. |
| Entire changed core lib nextest, all features | Source3: **683 passed**, 4 existing skips; run `eaf034d1-52da-41e6-9639-ba2e34c61ffb`. |
| Workspace strict Clippy, all targets/features | Source3: PASS with `-D warnings`. |
| Workspace rustdoc, all features | Source3: **100 passed**: core 6, import 34, operation 56, postprocess 4. |
| Standalone assets nextest | Source3: **138 passed**, 1 existing manual skip; run `b084b7c7-e010-4963-b82b-a733780463c7`. |
| Standalone assets strict Clippy, all targets/features | Source3: PASS. |
| Workspace, assets and fuzz `cargo fmt --check` | Source3: all PASS. |
| `registry_detection` fuzz target | Windows compile PASS after its interface-only adaptation; no fuzz campaign claimed. |

The 3,413-test run belongs to source2. Qualification of final source3 combines that broad result
with the actual complete core rerun over its only changed test files and the final lint, rustdoc,
consumer and formatting checks. It does not claim another complete workspace runtime on source3.
Existing skips remain explicit and are not counted as passing tests.

Five real root oracles cover forgotten operation and assembly permissions, reference-free
containment, rejected startup with root-owned control and non-Send/non-Sync permissions. During
actual canonical/evidence destruction, probes observe the original ledger's exact live bytes and
active reservations before either attribution batch releases. Final physical-zero checks remain.
The two eligibility races keep preparation on the host thread and move only independent poisoners,
account references, observation probes and barriers to the other thread.

Five extracted rustdoc contracts also verify actual compiler reasons: owned output compiles;
non-Copy proof, cross-binder substitution, repeated movement and cloning fail with E0277, E0521,
E0382 and E0599 respectively. Missing-lifetime and missing-API errors do not satisfy these probes.

## Evidence and Preserved Failures

Execution receipts live under `target/review/convergence/`; frozen source and independent reviews
live under `.git/baozi-review/performance-convergence/`. The ten-check aggregate is
`lexical-root-checkpoint-6-qualification-1.json`, SHA-256
`e40beae182a81ca41d9573d5d982907485d0a0a62456d04b63599b3689a8fc2b`.
Final source-delta review is `lexical-root-checkpoint-6-final-delta-review-3.json`, SHA-256
`31e23e600822666c81495d6f86a8c8847e4eeaf45c4e23a42016d5a19a5a533f`.
Independent raw-qualification review is `lexical-root-checkpoint-6-qualification-review-4.json`,
SHA-256 `f056751c707a83506b31d7fe31bace17a8722e908fc6f4de48796d1314ef9769`.
The final bounded qualified-source receipt is `lexical-root-checkpoint-6-qualified-source-1.json`,
SHA-256 `c38c8542cc5141cf16597f8984f7f96d6fdbf46adbdee5d229849b05678a5745`.

The exact source3 archive is `lexical-root-checkpoint-6-source-3.zip`, SHA-256
`bd0fd94b06798743c4458e46e0fa486ea07e535a260ab4b6a7cbaea950e18a89`.
All 1,405 members, including its source manifest, were read back and rehashed. Source2 has its own
independent archive and manifest; source identities are never relabeled after a fix.
The combined local bundle `lexical-root-fields-and-callers-evidence-1.zip` contains 189 members,
including both complete source archives, raw failures/results and reviews. Its SHA-256 is
`ad33b0c3c0be21815c5bf3669aa82b88bf41d0743fc74a127f66b61304a88c3b`; every member was read back
and rehashed.

Preserved failures include the interrupted 51-file source, initial caller-signature errors, 57
import and 17 public-store cleanup-boundary failures, the real startup oracle's initial absent-common
failure, three private-store test failures, 18 then 12 core compiler errors and the six/four
test-helper lint failures. Cleanup fixtures now destroy the actual root before unchanged exact-zero
assertions. The startup ordering defect was fixed in production; no test budget or numeric algorithm
was weakened. The four-error Clippy log after a partially applied UTF-8 editing attempt also remains
retained.

## Remaining Work

Actual canonical output/proof/recovery causes still need outer-host slots. Terminal, freeze and
joint authority still need the real callback-and-capture disposal boundary. `LocalWork` and its
live aliasing consumers, borrowed numeric backing, current-state attachment, shared-storage
promotion and their replacement/deletion gates remain unfinished. The existing terminal common
extraction is not made safe merely by the new permission lifetime.

Complete those connected paths before applying the complete root cut to the primary checkout. Then
qualify the unchanged Windows/Linux 14-case/110-lane performance and memory targets. The last ordinary result
remains HOLD; this checkpoint supplies correctness and ownership evidence without a new benchmark,
heap attribution, Linux qualification, promotion, publication, push or PR claim.
