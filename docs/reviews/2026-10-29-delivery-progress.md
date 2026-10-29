# Library Delivery Progress — 2026-10-29

This is implementation evidence for the [active A–H plan](../plans/2026-10-29-001-refactor-library-delivery-plan.md),
not release admission or whole-plan completion. The baseline reviews remain historical observations.
Source changes below were exercised in the working tree on Windows x86_64 with Rust 1.95.0 unless
another toolchain is named. Commands use one Cargo build job and reuse `target`.

## Delivered increments

| Unit | Implemented behavior | Runtime evidence |
| --- | --- | --- |
| A1 | Compact references, admitted execution-plan storage, borrowed proof access, smaller phase transitions, and one owned terminal ledger remove cumulative stack movement. | Facade child-process regression imports, captures a snapshot, rejects malformed input, and returns cancellation on a 1 MiB thread. Standalone default consumer runs on the ordinary main stack. |
| A2 | Healthy final owners release during host unwind. Actual payload/Cause destructor faults retain fail-closed containment. | Operation 700 tests passed on Rust 1.99; allocation probe after warmup observed zero net allocations/bytes for normal drop and host unwind. Intermediate unpublished attribution states remain a separate audit. |
| A3 | Registered proof schemes reside in one named static array rather than relying on promoted wrapper addresses. | Four focused IO identity tests passed on 1.95 debug and 1.99 debug/release. IO 47 and native-fs checker 50 tests passed before this increment's lockfile refresh. |
| A4 | Executable sidecar example owns its PNG at the correct MTL-relative path; README and commands match real APIs. | All four examples and README quick start ran using the repaired 1.95 library. Typed-reader examples are rechecked at C's integration boundary. |
| B fidelity | Partial OBJ channels require specific loss authority; homogeneous positions divide by finite nonzero W; three-component UVs retain per-segment width; unsupported color suffix omission is explicit. PLY F64 custom values remain F64. STL multiword names preserve full trimmed text. | OBJ/facade integration 182/182 passed. PLY custom-attribute suite 5/5 passed, including ASCII/LE/BE exact values and nonfinite/budget precedence. STL name/diagnostic regressions passed in its subsequent suite. |
| B resource work | Short reads reuse admitted byte credit across serial resources; EOF probes are bounded read work; payload ceilings and live buffer capacity are independent. Hashes use 64 KiB control boundaries and dependency snapshots retain one digest. Geometry caches group slot identities without prefix scans. | Core/import/PLY/STL integration ran 804 tests: 801 passed and three old error/test-fixture expectations failed; those were corrected. Import's full portion, public reader allocation tests, five geometry-cache tests, eleven hashing tests, and short-read cases passed. Final combined rerun remains the integration gate. |
| C reading | One immutable Scene exposes borrowed nodes/roots, geometry, scalar payloads, placements, materials/bindings, encoded images/samplers/textures, space declarations, cameras/lights, and checked typed IDs. | Dense/sparse/presence/spatial tests passed, including zero counted allocations during repeated reads. Facade geometry and appearance tests passed in 182/182. Independent default OBJ/STL consumer passed outside workspace feature unification. |
| C authoring/errors | Static plugins use a borrowed builder with metered default alignment and primary-document provenance. Node, geometry, appearance, deformation, animation, spatial, profile, and aggregate errors preserve bounded semantic detail through finalization. | Nine static-plugin and eight submission-error tests passed, including domain/rule/value cases, later terminal cancellation, nested material rules, and digest reuse. Required-dependency context retains the declaring origin, role, original reference, normalized path, and resolver-authorized target. Nested missing/denied/redirected/rejected-reference tests passed in the integrated workspace run. |
| G feature/modules | Empty `std` and unused tracing forwarding are removed. Facade errors and importer workflows have separate modules; compiled-in registration is named `builtin`. Typed readers have private domain modules. | Existing facade and tracing tests passed; independent feature matrix is the follow-up gate. Native candidate extraction and hidden integration bridge migration remain separate future cutovers. |

## Independent consumer

[`tools/consumers/scene`](../../tools/consumers/scene) is a checked-in standalone workspace using the
public facade for consumption and a deliberate `baozi-import` dependency for plugin authoring.
Its expected format set is supplied independently of registration.
[`scripts/check_features.py`](../../scripts/check_features.py) executes a fixed list of empty,
default, single-format, admitted-all-format, tracing, and mint cases serially. It does not generate
Rust, resolve Rust names, or inspect source shape. A target check is compile evidence only.

## Recovery integration and storage pilot

The continuation after source session `01a1834e-5379-76b0-b412-01aa80c80fc6` reverified operation
unwind/slot behavior (24/24) and glTF animation/name handling (20/20) on Rust 1.95.0. Animation names
now share the admitted animation work group instead of opening an invalid nested work scope.
Deformation and animation readers expose borrowed influences, morph payloads, names, key times, and
tangents; an independent focused reader review reported no substantive findings.

The expanded independent consumer executed all three admitted formats, repeated nested sidecar
references, a custom hierarchical plugin, Matrix/TRS preservation, sparse attributes, and precise
invalid-index/parent errors on the ordinary Windows main stack. It exposed two stack costs missed
by the initial triangle: recursive cache partitions and success-path space reserved for rejection
branches. A fixed partition array and separately prepared rejection branches address those costs
without increasing the caller's stack.

Resource-graph depth planning now keeps its five temporary vectors accounted. Pending depth updates
retain their capacity lease, repeated edges fit an admitted traversal stack, and graph-owned paths
retain their backing-capacity reservations. Successful manifest terminal ownership remains open.

Three ignored complete-pipeline scale fixtures plus the normal control fixture and five cache tests
passed (9/9). The manual scale timeout is bounded at 180 seconds; these fixtures remain ignored in
ordinary runs. The deterministic work baseline for distinct indexed primitive buffers is:

| Primitive buffers | Consumed construction work | Peak requested allocation bytes |
| --- | ---: | ---: |
| 256 | 109,701 | 1,159,986 |
| 1,024 | 435,333 | 4,539,186 |
| 4,096 | 1,737,861 | 18,055,986 |

The 10,000-node edit shared its unchanged 8 MiB payload but requested 19,609,003 total allocation
bytes, with a 19,604,867-byte peak. For 100,000 vertices with four influences and 8 MiB metadata,
construction peaked at 27,228,207 requested bytes; a transform edit peaked at 12,033,084. These are
allocator-request observations, not RSS or exact record-copy measurements. They establish targets
for removing full influence/metadata projections and unnecessary node-table rebuilding in D.

Commit `34208ce`, the first D increment, now borrows immutable influence row-offset, joint, and source-order slices
only after the existing metered validation visit and completion succeed. The three full scratch
vectors, their allocations, and their zero-initialization work are removed. The operation/core suite
passed 1,012/1,012 tests, and the repeated influence/metadata scale fixture passed all four size
combinations in 60.49 seconds. At 100,000 vertices with four influences and 8 MiB metadata:

| Observation | Baseline | Borrowed validation |
| --- | ---: | ---: |
| Build peak requested bytes | 27,228,207 | 23,628,203 |
| Edit peak requested bytes | 12,033,084 | 8,433,080 |
| Build allocation requests | 161 | 158 |
| Edit allocation requests | 123 | 120 |
| Build consumed work | 49,345,465 | 48,445,464 |
| Edit consumed work | 20,179,178 | 19,279,177 |

Each path removes 3,600,004 bytes of immutable-payload scratch and 900,001 initialization work
units. Persistent physical live bytes and manually expected semantics are unchanged. The remaining
8 MiB edit scratch still tracks metadata; this increment does not complete D1's typed node/geometry
cutover or D2's metadata work.

The runtime check now uses a single locked Cargo metadata query. Source/AST checks for runtime
scheduling, format ownership scans, conformance receipts, and test-source hash binding were retired
in favor of the [behavior index](../testing/behavior-gates.md). Native-fs AST/path/cfg/test-hash analysis is also retired. Its metadata graph checks still enforce
target/feature partitions, source/checksum identity, reviewed dependency basis, and snapshot drift;
compiler and filesystem behavior tests carry availability and behavior checks. The locked dependency
baseline was refreshed using the existing snapshot functions, with the repository-only dependency
delta unchanged. Commit `d70b7fe` records the locked dependency refresh and matching runtime snapshot. All 65 xtask tests passed with all features after the cleanup; commit `e1a35c1` records this governance unit.

The first recovered all-feature workspace run executed 1,940 tests: 1,928 passed, 12 failed, and
three scale fixtures remained ignored. Four failures reproduced the two backend defects; the rest
identified exact work-prefix and unequal-source-path calibration assertions after accounting changes,
plus an old numeric-only error expectation. Focused fixes preserve exact budgets and typed causes;
the full rerun executed 1,945 tests: 1,944 passed and one old backend-panic expectation failed.
That expectation now requires `gltf.invalid_document`; the complete affected glTF package rerun
passed 167/167, including all seven direct-backend/importer regressions. No other full-run failure
remained. All-target/all-feature clippy with `-D warnings` and workspace doctests passed on 1.95.0.
The maintained backend is installed in commit `5a1a687`, with no facade promotion. The independent default and all-features consumers both passed on the ordinary Windows main
stack after this integration. Workspace rustdoc with `-D warnings` and formatting also passed.
Linux sanitizer execution remains separate and pending.

## Remaining acceptance

- Complete the final combined acceptance after the remaining architectural units; current clippy and doctests pass, while documentation/independent-feature and sanitizer runs continue.
- Finish B's successful manifest retained ownership and glTF fidelity paths; validate the
  isolated STL color-composition fix after integration.
- Preserve C's verified typed reads and required-dependency context through the storage cutover; retain the single Scene owner.
- Execute D's measured typed-storage cutovers and delete replaced records/projections.
- Complete E's backend decision and real Linux sanitizer admission before facade glTF registration.
- Deliver F's triangulation, normals, target-space conversion, and application vertex lowering.
- Finish G's behavioral governance cleanup and H's dependency, package, and executed-target proof.

No change here asserts exact RSS from ledger counters, glTF sanitizer qualification, a public
postprocessing workflow, or publication readiness.
