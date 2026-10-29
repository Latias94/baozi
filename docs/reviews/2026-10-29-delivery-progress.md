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
Linux sanitizer execution is recorded separately below.

## Linux sanitizer baseline and glTF continuation

The immutable `04d6874` source archive executed on Ubuntu 26.04 x86_64 with
`rustc 1.101.0-nightly (92ffebece 2026-10-28)`, cargo-fuzz 0.13.2, and AddressSanitizer.
The maintained backend closure resolved to `baozi-gltf` and `baozi-gltf-json`; the fuzz lockfile
was byte-identical before and after execution. Each of the three malformed JSON reproducers and
their three GLB equivalents executed successfully. A subsequent bounded campaign completed
256 runs over the six checked-in corpus seeds without panic or abort, with a 10-second input
timeout, 4,096 MiB RSS cap, 65,536-byte maximum input, and 120-second campaign cap. The executed
117,394,712-byte binary contains `__asan_init` and `__asan_report_load1`. The campaign's final
145 MiB RSS observation describes that short run only.

Local logs are `target/review/linux-gltf-asan-04d6874.log` and
`target/review/linux-asan-instrumentation.log`. They are transient execution artifacts, not packaged
evidence or qualification for subsequent edits. This closes the demonstrated baseline abort
reproduction on that source state; E's semantic, normalized ownership, source mapping, independent
consumer, and updated sanitizer gates remain required before registration.

A continuation snapshot based on that archive passed all 174 glTF tests after the static morph
increment and the first normalized ownership cutover. Seven new tests assert dense
POSITION/NORMAL/TANGENT values, multiple primitive projections, zero-delta empty and absent targets,
mesh defaults, node overrides, external-buffer equivalence, pre-IO cardinality and cell limits,
non-finite rejection, and explicit unsupported semantics. Two obsolete unsupported-morph tests were
replaced with successful typed consumption and invalid weights-without-targets coverage. This is
isolated glTF evidence while the retained-owner and typed-core workers continue; it does not claim
that their combined working tree has passed.

The expanded isolated snapshot then passed 198/198 glTF tests. Morph-weight animation now retains
Step, Linear, and CubicSpline channels with complete vector-width cardinality, instance-qualified
targets, normalized integer conversion, and correct vector-major cubic tangent ordering. Selected
source entities retain scanner-derived raw ranges through normalization. Tests cover original UTF-8
bytes, escaped property names, GLB offsets, nested source ordinals, selected payload rejection, and
unselected payload omission. A hand-built `NormalizedGltf` fixture reaches canonical authoring
without a decoder; its primary JSON intentionally has no glTF asset declaration. Backend-owned data
and its opaque allowance are explicitly dropped before the authoring module runs. These results
still precede the B/D combined tree, final facade consumer, and updated Linux ASan campaign.

## Typed node/geometry and retained-owner integration

The current working tree replaces node and geometry records with one private typed
`SceneStorage` ownership path. Node-transform edits share the unchanged geometry owner; they
rebuild the typed node owner and the remaining publication-local domain records. Metadata
validation now borrows immutable text, bytes, and scalar payloads after their complete metered
visits. It retains descriptors and necessary summaries without full metadata copies.

The operation/core all-feature suite passed 1,023/1,023 tests, with the three manual scale fixtures
ignored in that run. The scale fixtures and their normal budget/cancellation control test then
passed 4/4 in a separate serial run. Distinct-buffer construction now measures:

| Primitive buffers | Consumed construction work | Peak requested allocation bytes |
| --- | ---: | ---: |
| 256 | 40,937 | 546,042 |
| 1,024 | 160,745 | 2,075,898 |
| 4,096 | 639,977 | 8,195,322 |

For a 10,000-node transform edit with unchanged shared 8 MiB geometry, total requested allocation
bytes fell from 19,609,003 to 10,390,606 and peak requested bytes from 19,604,867 to 10,387,326.
At 100,000 vertices with four influences and 8 MiB metadata, construction peaked at 15,236,923
requested bytes and the edit at 33,870, compared with the original 27,228,207 and 12,033,084.
All four influence/metadata size combinations passed. These are allocation-request observations;
no exact process RSS claim follows from them.

A complex independent glTF/GLB consumer exposed another ordinary-main-stack overflow during
publication. CDB first identified cumulative activation/normalization frames, then a second
failure in terminal ledger freeze after those frames were split. The import session now keeps its
large state and semantic vocabulary in one startup-admitted cell using the existing `AccountedVec`
storage owner. The cell allocation is destroyed and its charge released before terminal freeze;
no final-pivot allocation is introduced. The parser also returns its normalized model before
canonical authoring begins.

The independent consumer subsequently passed on ordinary Windows debug and release main stacks, covering
GLB and external resources, interleaved/multiple primitives, PBR/image/texture/sampler identities,
Matrix/TRS hierarchy, skins, static morphs, and Step/Linear/CubicSpline animation. Its manually
expected node enumeration was corrected to the existing breadth-first order; parent/child
assertions continue to verify hierarchy independently. This consumer uses explicit public plugin
registration while glTF remains quarantined in the facade.

Selected glTF source declarations now cross normalization as original source ordinals and raw
ranges, with nested parent identity and namespaced locators in the canonical profile. The complex
consumer passed in both debug and release; updated sanitizer qualification remains required.
Current transient logs are `target/review/typed-storage-scale.log`,
`bd-operation-core-full.log`, `gltf-normal-stack-cdb-current.log`, and
`gltf-independent-consumer.log`.

Stronger import fixture teardown checks subsequently exposed an older dormant-owner leak: dropping
a session before activation unconditionally contained its healthy operation and retained 7,368
physical bytes. Dormant owners now distinguish intact session ownership from an explicitly marked
transient activation transfer. Ordinary drop and unrelated unwind release the former; abandoned
transfers and active-driver failures keep containment. Tests continue to require zero final physical
bytes/allocations and no poison after healthy teardown. Startup scratch assertions measure the
actual session baseline rather than assuming zero or hard-coding its layout.

The glTF profile also records synthetic-root generation and implicit-sampler defaults as operation
nodes, retaining source parents and versioned durable instruction digests. Provenance declaration
limits use the entity envelope independently of hierarchy node limits. External image lookup uses
bounded binary search, and per-owner animation source aggregation uses controlled sort/dedup with
admitted work and reusable scratch.

Hierarchy depth and provenance DAG depth now have separate limits. An empty glTF projection fits
its exact hierarchy depth of one while retaining the generated root's two-level provenance. Core
tests independently reject excess provenance entities and depth; import continues to derive that
envelope from the canonical payload budget, with actual storage and traversal admission enforced.

The final combined all-feature workspace run passed 2,071/2,071 tests, with three manual scale
fixtures skipped; those fixtures have the separate 4/4 result above. This includes the retained
manifest, diagnostic, loss, dependency-context and error-payload lifetimes, healthy dormant-owner
teardown, typed storage, glTF source mapping, and exact depth regressions. The all-target/all-feature
Clippy run also passed with `-D warnings` after the depth-limit split. Workspace doctests,
public rustdoc with `-D warnings`, formatting, and whitespace checks passed. Local logs are
`target/review/bde-workspace-final.log`, `bde-workspace-clippy-final.log`,
`bde-workspace-doctests-final.log`, and `bde-workspace-rustdoc-final.log`.

## Remaining acceptance

- Complete final acceptance after the remaining architectural units. The current combined suite
  closes the B/D1/E integration checks recorded above; later changes need their own validation.
- Preserve B's verified retained manifest/evidence ownership and fidelity behavior through the
  remaining storage and postprocessing cutovers.
- Preserve C's verified typed reads and required-dependency context through the storage cutover; retain the single Scene owner.
- Extend D1's measured typed node/geometry cutover to the remaining D2 appearance,
  deformation/animation, and profile domains; delete each replaced record/projection path.
- Complete E's backend decision and real Linux sanitizer admission before facade glTF registration.
- Deliver F's triangulation, normals, target-space conversion, and application vertex lowering.
- Finish G's behavioral governance cleanup and H's dependency, package, and executed-target proof.

No change here asserts exact RSS from ledger counters, glTF sanitizer qualification, a public
postprocessing workflow, or publication readiness.
