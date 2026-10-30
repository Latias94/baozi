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
| G feature/modules | Empty `std` and unused tracing forwarding are removed. Facade errors and importer workflows have separate modules; compiled-in registration is named `builtin`. Typed readers have private domain modules. | Existing facade/tracing tests and the independent source feature matrix pass, including the selected glTF profile. Native candidate extraction and hidden integration bridge migration remain separate future cutovers. |

## Independent consumer

[`tools/consumers/scene`](../../tools/consumers/scene) is a checked-in standalone workspace using the
public facade for consumption and a deliberate `baozi-import` dependency for plugin authoring.
Its expected format set is supplied independently of registration.
[`scripts/check_features.py`](../../scripts/check_features.py) executes a fixed list of empty,
default, single-format, admitted-all-format, tracing, and mint cases serially. It does not generate
Rust, resolve Rust names, or inspect source shape. A target check is compile evidence only.

## Typed deformation owner cutover — 2026-10-30

Deformation now uses one retained owner containing seven typed row tables and ten payload buffers.
Skin joints, whole inverse-bind matrices, CSR influence columns, morph weights/deltas, sparse
indices, and names retain their source bits and presence. Logical owner/range coordinates do not
depend on the published Scene generation. Unchanged edits share the complete owner. Fresh builds,
central validation, public readers, and semantic snapshot traversal use the same actual tables;
the old deformation record schema, writer, decoder, edit rewrite, and seven descriptor scratch
tables are deleted. Central validation retains palette maxima and bounded uniqueness marks only.

The remaining animation records no longer encode a redundant target-context cell or a deformation
record reference. Target kind and logical index resolve against borrowed node/geometry/deformation
owners. The private Scene root is revision 6 with nine fields; the transitional animation codec is
revision 4 with 22 channel fields. The semantic snapshot remains revision 4, and existing golden
vectors passed unchanged in the 204-test focused run. Appearance and animation storage cutovers
remain open; this increment does not complete D2 or the A–H plan.

Authoring canonical-byte limits count the real initialized header, used rows, and payloads. Inline
skin corrections add no separate allocation or footprint. Initial scratch admission reads only
top-level lengths; nested morph plans and uniqueness buffers grow through the existing admitted
capacity mechanism during controlled validation. Reader cursors borrow proof authority when
needed; typed geometry/deformation lengths come from actual proof-qualified tables.

Finite affine matrix constraints are consistent across public correction construction, authoring,
typed central validation, and glTF normalization. Shear, reflection, singular linear parts, and
signed zero remain source-preserving values; projective rows and nonfinite values are rejected.
The focused 204/204 run includes these cases, exact work, per-action interruption/unwind,
presence/identity, sparse values, random-row reads, and zero-allocation reading/validation.
Independent read-only reviews found no concrete defects in materialization/edit/count integration
or authoring scratch/builder paths. Integrated workspace, manual scale, consumer, and package
results are recorded after execution below.

The workspace run passed 2,254/2,254 tests with the three manual pilots skipped. A subsequent
serial core run passed all three manual pilots, their normal admission/cancellation fixture, and
the added eight-class post-construction corruption test (5/5). Strict workspace all-target,
all-feature Clippy and workspace/standalone-consumer formatting passed. No source mutation was
made to update the existing semantic snapshot vectors.

Compared with the immediately preceding static-conversion source state (`4832101`), the same
100,000-vertex, four-influence, 8 MiB metadata fixture measures:

| Observation | Before deformation cutover | Typed deformation |
| --- | ---: | ---: |
| Build peak requested allocation bytes | 15,220,047 | 15,210,563 |
| Edit peak requested allocation bytes | 19,732 | 12,320 |
| Edit allocation requests | 72 | 58 |
| Build consumed work | 40,055,804 | 38,355,474 |
| Edit consumed work | 10,889,338 | 9,289,056 |

All four 10k/100k-vertex and 1/8 MiB metadata combinations retain the same 12,320-byte edit peak.
The fixture checks complete owner identity and borrowed influence-column addresses, including
survival after source release. With 4,096 independent indexed primitive buffers, build requested
peak falls from 4,053,838 to 2,083,618 bytes and consumed work from 442,991 to 324,111; 256/1,024
cases remain linear. These are requested allocator bytes and deterministic work, not process RSS.
Logs are `target/review/d2-deformation-workspace-nextest.log`,
`d2-deformation-scale-and-central.log`, and `d2-deformation-clippy.log` in the same directory.

The standalone all-feature consumer executed all four formats and the complete postprocess chain
on the ordinary Windows main stack in debug and release. Fresh `.crate` archives unpacked under
`target/package-check-nlicuk4y` built and executed the postprocess and glTF consumers; Cargo source
verification rejected checkout substitution. The release `wasm32-wasip1` all-feature consumer
executed through Wasmtime 50.0.0 with the same four-format expectations. Browser
`wasm32-unknown-unknown` passed an all-feature compile check only. Workspace doctests and rustdoc
with `-D warnings` passed. Logs use the `target/review/d2-deformation-` prefix with suffixes
`native-debug.log`, `native-release.log`, `packaged-consumers.log`, `wasi-consumer.log`,
`browser-wasm-check.log`, `doctests.log`, and `rustdoc.log`. No new sanitizer execution or sustained
fuzz campaign is claimed for this source state.

## Typed animation owner cutover — 2026-10-30

Animation now uses one retained owner with two typed row tables and five contiguous pools. Closed
targets identify canonical nodes or logical mesh instances, while actual time/value ranges
determine key count and value width. Whole-Scene construction, unchanged edits, central validation,
public reads, and semantic snapshots use that one owner. The animation record schema, writer,
edit rewrite, scalar decoder, and clip/channel validation descriptors are removed. Central
validation borrows the actual node/geometry/deformation dependencies and initializes only duplicate
target marks, excluding the synthetic root and allocating no marks for a channel-free domain.

`AnimationTimeDomain`, `AnimationInterpolation`, and `AnimationExtrapolation` are common domain
values rather than separate authoring, storage, semantic, and public-read enums. Producer APIs use
those same types; the former `*Authoring` enum names are removed without compatibility aliases.
Only resolved target coordinates and widths remain in retained authoring scratch. Canonical-byte
admission counts actual initialized rows/pools and the owner header, with exact/one-short tests.
The private Scene directory is revision 7 with eight fields; appearance is the sole remaining
record-backed semantic domain. Semantic snapshot encoding remains revision 4.

The first targeted run passed 213/213 tests, including ten post-construction corruption classes,
public exact payload reads, existing snapshot vectors, real target resolution, cubic derivative
shape, and per-action interruption/unwind. Read-only cross-reviews found a draft-only duration
restriction: keys beyond declared clip duration passed authoring but failed the new validator.
The restriction was removed to preserve the existing source contract. Finite negative pre-roll and
keys beyond source duration remain valid and bit-preserved; duration itself stays finite and
nonnegative. Dedicated tests pin both sides of that range. No unresolved review finding remains.
The final integrated execution results are recorded after their gates complete below.

The all-feature workspace nextest run passed 2,283/2,283 with three manual pilots skipped. After
adding the complete-pipeline source-duration regression, the serial core rerun executed every
unit test and all manual pilots: 447/447 passed with no skips. It explicitly exercises arbitrary
finite keys through authoring, central validation, publication, and the public reader. Strict
workspace all-target/all-feature Clippy passed after removing a redundant count conversion and
naming the corruption-case tuple type; workspace formatting passed.

For the unchanged 100,000-vertex/four-influence/8 MiB metadata workload, allocator-requested edit
peak falls from 12,320 bytes at `79c91cb` to 10,792 bytes and requests from 58 to 55. All four
vertex/metadata size combinations keep that same peak. This fixture has no animation channels;
it measures removal of the empty animation directory and its edit/validation overhead, not an
animated-sample benchmark. Deterministic work remains linear for the independent primitive
buffers. Logs are `target/review/d2-animation-workspace-nextest.log`,
`d2-animation-core-and-scale.log`, and `d2-animation-clippy.log` in the same directory. As before,
allocator-requested bytes are not process RSS.

All-feature standalone Windows consumers executed all four formats and the complete postprocess
chain in debug and release on the ordinary main stack. Separate empty-feature and curated-default
release consumers also executed successfully, outside workspace feature unification. Fresh
archives under `target/package-check-b25bhbjq` built and executed postprocess and glTF consumers
with dependency origins verified against the unpacked package closure. The all-feature release
WASI consumer executed through Wasmtime 50.0.0; browser WASM passed compilation only. Workspace
doctests, rustdoc with `-D warnings`, and workspace/standalone-consumer formatting passed.
Logs use `target/review/d2-animation-` with suffixes `native-debug.log`, `native-release.log`,
`native-feature-subsets.log`, `packaged-consumers.log`, `wasi-consumer.log`,
`browser-wasm-check.log`, `doctests.log`, and `rustdoc.log`. No new sanitizer or sustained fuzz
result is claimed for this source state.

## Typed appearance owner cutover — 2026-10-30

The final appearance domain now uses one retained owner with five row tables and two pools: images,
samplers, textures, material-owned bindings, materials, encoded image bytes, and combined UTF-8 text.
Fresh construction, central validation, public reads, snapshots, and unchanged edits use that same
owner. The former record writer/schema/decoder, edit rewrite, three validation descriptor arrays,
and inert record-leaf inheritance plan are deleted. All seven Scene semantic domains now use typed
owners. The private revision-8 directory has seven scalar revision/count fields, no domain references,
and no record-store payload leaves. Semantic snapshot encoding remains revision 4.

Shared compact behavior enums replace the duplicate `ShadingModelKind`, `TextureRoleKind`, and
`ColorInterpretationKind` enums. Channel APIs use `TextureChannelSwizzle` directly, without a
single-variant wrapper. The seven producerless appearance `Namespaced` behavior variants and their
schema-construction/error paths are removed. No admitted producer could publish those variants.
Descriptive metadata and other domains' extension identities remain supported; a future rendering
descriptor requires complete production, storage, validation, read/snapshot, and remap support.
ADRs 0012, 0023, and 0045 record that boundary without pretending metadata preserves unknown behavior.

Authoring retains only a metered resolved-image borrow table, preserving the same-session resource
snapshot authority; binding and material descriptions stay in original input. Canonical-byte limits
count the actual initialized header, rows, and pools. Semantic cells count present numerical values
and bytes rather than retired codec padding. Central appearance validation allocates no scratch: it
checks actual references, complete image/text/binding coverage, bounded numeric values, material
workflow requirements, and single-role authority with fixed cursors and a bounded role set. Public
random access reads only the selected row, and image identity does not follow byte equality.

The initial core run passed 473/473 ordinary tests, with three manual pilots skipped. This includes
fourteen post-construction appearance corruption classes through the complete central/publication
pipeline, requiring zero generation reservations, zero canonical commits, bounded rich failure, and
complete release. New builder, validator, and semantic tests check per-action interruption/unwind,
exact footprint/work, absent versus empty names, source bits, channel order, and allocation-free
validation/reading. Existing snapshot golden vectors passed without changing their expected bytes.
Two independent read-only reviews found no concrete issue in their assigned builder/validation and
reader/snapshot/sharing scopes. These static reviews do not replace executed tests.

The integrated all-feature workspace run passed 2,311/2,311 with the three then-existing manual
pilots skipped. After adding the explicit encoded-image pilot, the serial scale run passed all five
selected tests: four manual pilots plus the ordinary admission/cancellation control. Strict
all-target/all-feature Clippy passed. The same 100,000-vertex/four-influence/8 MiB metadata node edit
now requests a 9,240-byte allocation peak in 53 requests, compared with 10,792 bytes and 55 requests
at `95acffd`. All four influence/metadata size combinations retain that same peak. This workload has
an empty appearance domain and measures removal of its legacy directory and projection overhead.

The new 1 MiB and 8 MiB arbitrary encoded-image workloads both measure an 8,444-byte edit allocation
peak in 50 requests. The complete appearance owner and public byte-slice addresses are shared;
every byte remains correct after the authoring input and original generation/report are destroyed.
Final retained bytes and allocations return to zero. No image decoding is required. Independent
primitive-buffer work remains linear at 256/1,024/4,096. These are allocator-requested observations
and deterministic work, not process RSS. Logs are `target/review/d2-appearance-workspace-nextest.log`,
`d2-appearance-scale.log`, and `d2-appearance-clippy.log` in the same directory. Consumer and artifact
execution evidence follows after those gates finish.

That pilot also exposed redundant per-byte authoring and central-validation actions for opaque
encoded images. Neither action inspected content or established an invariant. Both no-op scans are
removed; per-image bounded range/nonempty/coverage validation remains, byte/cell admission still
counts the full payload, and real materialization copies and snapshot hashing retain their actual
metering. The source reader borrows an already-authorized snapshot, so no extra content pass is
needed before copying. Text and numeric semantics retain their existing validation work.
The final core run passed 475/475 ordinary tests with four manual pilots skipped. The image pilot
then passed independently and now requires identical charged edit work across both payload sizes:
224 units for 1 MiB and 8 MiB, with the same 8,444-byte requested peak and complete lifetime checks.
Logs are `d2-appearance-core-final.log` and `d2-appearance-image-scale-final.log`. This measured
change removes both payload-sized temporary growth and the otherwise linear opaque-content walk
from a node edit; it does not claim constant work for unrelated semantic domains.

The final production-workspace run, excluding the separately changing repository-only xtask, passed
2,248/2,248 with four manual pilots skipped. Strict workspace all-target/all-feature Clippy and
workspace/standalone-consumer formatting passed. Ordinary-stack Windows all-feature consumers
executed all four format workflows and the complete postprocess chain in debug and release.
Before the no-op scan removal, all twelve fixed package cases built and executed from fresh
archives under `target/package-check-t9xocm93`. The final source then repeated the postprocess and
selected glTF artifact workflows from new archives under `target/package-check-_pgds135`, with
dependency origins verified against the extracted package closure. No checkout source substitution
was accepted. Logs are `d2-appearance-workspace-final.log`, `d2-appearance-clippy-final.log`,
`d2-appearance-native-debug-final.log`, `d2-appearance-native-release-final.log`,
`d2-appearance-packaged-consumers.log`, and `d2-appearance-packaged-final.log` in `target/review`.
Workspace doctests and strict rustdoc passed before that bounded work-scheduling change; their
public interfaces are unchanged. No new sanitizer or sustained fuzz campaign is claimed.
The final all-feature release WASI consumer executed the same four-format and postprocess workflows
under checksum-qualified Wasmtime 50.0.0. Browser WASM passed compilation only. Logs are
`d2-appearance-wasi-final.log` and `d2-appearance-browser-wasm-final.log`.

## Appearance semantic value ownership — 2026-10-31

The follow-up API cutover removes unnecessary references from appearance authoring. Closed behavior
values (`ShadingModel`, `AlphaMode`, `TextureRole`, coordinate source, swizzle, color interpretation,
and blend operation) are copied into the private semantic values. Only the optional material name and
descriptive texture source key retain producer lifetimes. `MaterialSemantics` has no lifetime;
`TextureBindingSemantics` borrows only its optional source key. Public authoring constructors now take
the closed values by value, so a producer can pass temporaries without allocating or keeping dummy
borrowed arrays alive. Existing source keys, names, numeric bits, validation rules, and snapshot
encoding remain unchanged.

The authoring lifetime regression constructs owned semantics from inner scopes and asserts that only
name/key borrows constrain the result. The core target run passed **477/477** tests with four manual
pilots skipped; strict workspace Clippy and formatting passed. Logs are
`target/review/appearance-owned-values-core.log`, `appearance-owned-values-check-final.log`, and
`appearance-owned-values-clippy.log`. No compatibility alias or parallel borrowed semantic model was
retained.

## Production package contents and fixed artifact matrix — 2026-10-31

All ten self-owned production packages now include a local English README plus byte-identical
copies of the root MIT and Apache-2.0 licenses. Each package declares that README explicitly and
uses its own description and docs.rs URL. Repository/document links in package READMEs are absolute
so the registry rendering does not depend on an unpacked workspace layout. The maintained glTF/JSON
packages identify their Baozi package names, repository, and integration boundary while preserving
upstream authors and original license files. Backend features do not expand Baozi's selected profile.

`check_packages.py` checks the extracted normalized manifest's README declaration and all three
nonempty documents before executing a consumer. It retains its one metadata resolution, exact
archive identity, checked extraction, source-origin verification, unchanged consumer lockfile, and
fresh build directory. No Rust source parser, generated consumer, or second release registry was
introduced. Public documentation now describes this already-implemented simulation and the actual
five-target sanitizer workflow, and OBJ's support description reflects owned encoded-image snapshots.

The complete twelve-case Rust 1.95.0 debug artifact matrix passed under
`target/package-check-t9xocm93`: empty, default, default-formats, each of OBJ/PLY/STL/glTF,
postprocess, all-formats, tracing, mint, and all. All twelve extracted production packages, including
both maintained dependencies, contained the declared README and both licenses. A later new archive
closure under `target/package-check-_pgds135` again passed postprocess and glTF after the bounded
opaque-image validation-work change. Logs are `target/review/d2-appearance-packaged-consumers.log`
and `d2-appearance-packaged-final.log`. These results establish package content and the executed
consumer cases; current-stable/release full matrices and native filesystem promotion remain separate
acceptance, and no crate was uploaded or publication flag changed.

## Featureless xtask and manual growth-gate check — 2026-10-31

The unused `repository-input-candidate` xtask feature and optional `baozi-io` dependency are removed.
There was no capture command or source consumer behind that edge. Repository/native IO mechanisms
remain owned by `baozi-io`; the metadata checker has no runtime IO authority. Its production graph
now rejects direct or indirect `baozi-io` and native filesystem dependencies, including normal/build
edges while excluding dev-only edges. The former special-case dependency exception and its exact
feature-routing machinery are deleted. One xtask metadata query replaces equivalent empty/all
profiles, and CI runs its featureless suite once. ADRs 0007/0010 and the behavior index follow that
implemented boundary; no target candidate was promoted.

The real central typed geometry path has no legacy buffer cache. Its existing complete-pipeline
256/1,024/4,096 pilot measures controlled traversal and rejects more than fourfold work growth for
fourfold input. A one-time manual negative check inserted a test-only quadratic prefix walk. The
first probe was rejected earlier by exact work admission, confirming that unplanned traversal could
not publish. With matching temporary admission, 256/1,024 work rose to 53,295/605,103 units and the
existing growth assertion failed at 1,024 primitives as intended. Both temporary edits were removed
by precise patches; the two original SHA-256 values matched, with no source diff. The unchanged pilot
then passed, recording 20,655/81,327/324,015 units. Logs are `target/review/g-prefix-negative.log`,
`g-prefix-negative-admitted.log`, and `g-prefix-restored.log`. No persistent mutation runner,
comparison collector, or deliberate regression was added.

The final featureless xtask run passed 66/66 tests with no skips, including the real repository
baseline check, direct/indirect IO rejection, normal/build versus dev-only closure cases, and
renamed-feature forwarding rejection. Strict xtask all-target Clippy and workspace formatting
passed. The lockfile change removes only xtask's obsolete `baozi-io` dependency entry. Logs are
`target/review/g-xtask-featureless-final.log` and `g-xtask-featureless-clippy.log`.

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

## Selected glTF Profile Admission

The immutable `2dc2fc891fcacbd33f3c541b768b59ea575a3297` archive passed the updated Linux
AddressSanitizer gate on Ubuntu 26.04 x86_64. Its SHA-256 is
`558ff81b88cd38d8da78a609368dcfa19dff970f4f53e05453cd1bee72af6e39`. A fresh per-commit target
directory compiled the maintained backend closure. The executed 112,096,088-byte binary contains
`__asan_init` and `__asan_report_load1`. Three malformed JSON reproducers and their three GLB
equivalents completed, followed by 256 runs over the six checked-in seeds without panic or abort.
The campaign used a 10-second input timeout, 4,096 MiB RSS cap, 65,536-byte maximum input, and
120-second campaign cap; the final 147 MiB RSS is an observation of that short run only.

Both root and fuzz lockfiles remained byte-identical. Cargo-fuzz 0.13.2 has no `--locked` option:
locked Cargo metadata resolved the closure first, then the build/run used offline mode and
checked both lockfiles after each invocation. The stored archive and original corpus remain
unchanged; the mutation campaign ran against an evidence-directory copy. Local output is
`target/review/linux-gltf-asan-2dc2fc8.log`.

This closes the baseline quarantine for the exact selected profile and permits facade registration
through `format-gltf` and `all-formats`. The curated default remains OBJ/MTL and STL. The support
matrix keeps partial capabilities and no Beta/Stable maturity; package publication remains disabled.
The profile explicitly rejects unsupported selected extras/extensions and sparse/strip/fan slices.

## Direct Triangulation and Edit Ownership (2026-10-30)

The first F operation, `triangulate_primitive`, now runs against the exact input account before
target inspection, temporary allocation, or kernel work. It preserves authored attributes, source
winding, and Corner/Face origins; triangle lists return the original Scene unchanged. Changed
results publish a new generation together with the actual retained origin relation. A detached
report keeps that relation alive after both the output Scene and the import evidence are dropped.
Ordinary failures return the original immutable Scene.

The textured quad consumer exposed two multi-owner provenance defects: operation digest ranges
reused one offset, and each local provenance index was added twice to its segment's global base.
The fixed path retains a distinct digest range per operation. Address construction now belongs
to `ProfileSegmentAddressDomain::address(local)`, which owns the exact nonce, extent, and base.
The old four-argument address constructor is removed. Regression assertions read exact UVs,
authored normals, and duplicated Face smoothing groups through the actual published Scene.

The edit program now contains its source-bound geometry plan. Materialization accepts that closed
program directly; it no longer takes a separate optional plan or reconstructs its validation work
from a caller-supplied scalar. The execution cell no longer stores a second geometry-plan copy.
Unbound transform edits retain only their transform rewrite and source identity.

The obsolete empty request compiler, compiled-run/step types, separate coordinator/ledger, and
empty diagnostic/loss placeholders have been removed. Public reports identify the concrete
`PostProcessOperation::Triangulate { target }`, input/output generations, resource evidence, and
the actual remap. Cancellation, deadline, original-input, generation, thread-transfer, tracing,
and drop-lifetime checks now exercise the direct operation instead of the empty pipeline.

Resource evidence now separates published domain-evidence bytes from terminal ledger/fallback
record counts. Their private dimension vocabulary records those units explicitly. A terminal
ledger snapshot is not reported as one byte of domain evidence, and frozen operation-live
counters are not presented as dynamic retained-owner measurements.

The combined all-feature workspace suite passed 2,097 tests with three planned skips. After the
final lint cleanup, all 37 direct-operation/kernel tests passed again. Strict all-target,
all-feature Clippy, doctests, rustdoc, formatting, and the edited workflow syntax passed.
The native release `all` consumer ran on the normal Windows main-thread stack. Fresh extracted
`.crate` builds executed both the selected glTF and textured-triangulation consumer cases.
The release `wasm32-wasip1` all-feature consumer executed under Wasmtime 50.0.0, and the
`wasm32-unknown-unknown` all-feature consumer passed compilation only. The WASI runner was
downloaded from the fixed official release and verified against its published SHA-256;
it is a transient verification artifact, not a repository runtime dependency.

Evidence is in `target/review/fgh-workspace-current.log`, `final-postprocess.log`,
`final-workspace-clippy.log`, `final-workspace-doctests.log`, `final-workspace-rustdoc.log`,
`final-packaged-consumers.log`, `final-native-release-consumer.log`,
`final-wasi-release-consumer.log`, and `final-browser-wasm-check.log`. The independent static
ownership review found no concrete P0-P2 issue; it did not replace dynamic verification.
Normal generation, target-space conversion, and application vertex unification remain unimplemented.

## Dependency Closure Refresh (2026-10-30)

The current source resolves triomphe 0.1.17, cap-std/cap-fs-ext/cap-primitives 4.0.3,
sha2 0.11.1, criterion 0.8.2, and xtask toml 1.1.8. Runtime and repository native-filesystem
snapshots were regenerated through the existing snapshot functions from ten serial locked
metadata profiles. The exact critical package pins and fixtures now match the lockfile.
All 65 xtask tests, including the actual repository dependency check, passed.
The standalone fuzz lockfile now resolves the same sha2/triomphe requirements, while retaining
its existing arbitrary/cc/find-msvc-tools versions. Locked fuzz metadata resolved successfully;
this is dependency-resolution evidence, not a new sanitizer campaign.

The previously reviewed Windows directory-open source files are byte-identical between the
checksum-qualified cap-primitives 4.0.2 and 4.0.3 archives. Their source hashes and the limits of
that continuity observation are recorded in the native-filesystem admission research. This is
dependency and behavior evidence, not promotion of a new runtime target.

## Typed Profile and Provenance Cutover (2026-10-30)

Fresh construction, edit planning/materialization, central validation, semantic reading, and snapshot
traversal now use one typed profile representation. `SceneTables` owns nodes, geometry, and profile.
Profile state owns the final-layout owner/attachment and metadata descriptors; immutable lineage
segments and metadata payload pools have independent retained owners. Node-transform and
triangulation edits share their unchanged allocations through operation's existing physical owner.
Each changed owner receives its own operation node with its actual parents.

Provenance edges store only segment ordinal, nonce, and local node. Checked lookup derives the global
index from validated segment prefixes. Digests and wide values are typed inline values, and text
pools are `String` values that remain valid UTF-8 through interrupted construction. Owner maximum
depth caches are verified against actual attachment depths. Profile canonical-byte planning counts
initialized typed headers/tables and used payloads under the target ABI; allocator capacity and
retained-owner infrastructure remain separately admitted by operation.

Central validation borrows actual retained profile data and allocates only provenance depths.
The generic profile schema, scalar codecs, record writers, dynamic inherited-profile leaf scan,
record rewrite, and full lineage/descriptor projection buffers have been deleted. The remaining
record-store block ceiling counts only domains that still use those blocks. Ordinary handle
sharing now consumes import `WorkUnits`, independently of that smaller Scene-record ceiling.
Callback, hash, read, and cancellation tests use actual action-entry baselines while retaining
exact completed-prefix assertions. Hash-reuse tests deny another full hash chunk while allowing
the unrelated ownership work.

The combined all-feature workspace suite passed **2,103/2,103 tests**, with three manual scale tests
skipped in that invocation. Strict all-target/all-feature Clippy, doctests, rustdoc with `-D warnings`,
and formatting passed. The maintained consumer built and executed the glTF and textured
triangulation workflows against freshly unpacked `.crate` dependencies, without resolving checkout
paths. An independent read-only review of typed storage, prepared admission, profile edits, grammar,
work, and branch qualification found no outstanding concrete P0-P2 issue; it did not replace runtime
verification.

All three manual scale tests passed separately. For both 10,000 and 100,000 four-influence vertices,
the node edit's allocator-request peak remained **21,284 bytes** when metadata grew from **1 MiB to
8 MiB**. The edit retained shared lineage and metadata payloads after source destruction, and final
account bytes/allocations returned to zero. Independent primitive-buffer workloads at
256/1,024/4,096 and node-edit workloads at 1,000/10,000 nodes also passed. These are requested
allocation and deterministic work observations, not exact process-RSS claims.

Logs are `target/review/d2-workspace-nextest.log`, `d2-storage-scale.log`,
`d2-workspace-clippy.log`, `d2-workspace-doctests.log`, `d2-workspace-rustdoc.log`,
and `d2-packaged-consumers.log`. This increment does not provide normal generation, target-space
conversion, application vertex unification, or a new sanitizer campaign.

## Direct Normal Generation and Exact Attribute Keys (2026-10-30)

`generate_normals` now provides direct Flat and area-weighted Smooth operations on TriangleList
primitives. Smooth buckets use Vertex identity and optional U32 Face smoothing groups; zero is
hard per face, and UV seams do not split a shared Vertex. The operation borrows dense F32
positions, produces dense F32 Corner normals, and preserves topology and unrelated attribute
encodings, defaults, sparse forms, and scalar bits. Authored normals remain unchanged by default;
replacement and invalidated Tangent deletion require explicit options. Multiple authored Normal
or Tangent domains that would require a field-selection policy return typed capability errors.

Each face determinant uses six exact products of original F32 coordinates in F64 and a fixed
six-entry TwoSum expansion. This fixes the concrete `2^-60` coordinate-cancellation case where
widened edge subtraction still lost the normal's Z component and changed direction under a cyclic
vertex permutation. Tests also cover reverse winding, exact collinearity, and the smallest F32
subnormal. The contract remains `(b - a) × (c - a)` in the stored numeric basis; it does not infer
a front-facing direction from Scene declarations. Scaled normalization and compensated Smooth
accumulation retain the existing finite-extreme and opposing-area behavior.
An independent numerical review also found a one-ulp estimate difference between opposite
permutations of one face. The determinant now fixes its product order by Vertex ID and restores
winding by an exact sign change. Its concrete regression checks identical cyclic estimate bits,
opposite signs, and an exact Smooth cancellation error with or without the face cache.

The source-bound geometry plan owns the concrete added/replaced/deleted attribute mapping. Profile
owners derive from the final layout, lineage uses the deduplicated Primitive/Position/replaced
Normal/consulted SmoothingGroup parents, and metadata follows the same mapping. Reindexing shares
immutable payload pools; actual Tangent metadata deletion compacts surviving payloads under its own
admission. Published evidence is a closed operation-specific enum. Triangulation owns its real
remap; a normal report owns its actual root and effects independently of either Scene, without
duplicate cached effect fields or inapplicable remap accessors. Both operations use one concrete
geometry publication transaction.

Typed attribute lookup now requires the complete `(semantic, domain)` key. `normals(domain)` and
`texcoords(set, domain)` read distinct Vertex and Corner streams without an implicit selection.
A real Scene regression uses unequal cardinalities and different values and verifies allocation-free
lookup. All maintained callers, including the separate source/package consumer, use explicit domains.

Recoverable central evidence or controlled-validation failures now return `PendingMinimalFallback`.
Callers end their borrows and release scratch before terminal preparation. Import retains that
continuation through parser cleanup, and geometry edits release generated values before freezing.
The regression injects a real final-owner allocation failure with both ordinary and retained-output
reservations and checks unfrozen evidence during the borrow, preserved cause, and zero live bytes,
reservations, and canonical commits afterward. The pre-fix all-poll cancellation test already passed;
these changes do not claim to have reproduced a cancellation-containment failure.

The final all-feature workspace suite passed **2,168/2,168 tests**, with three manual scale tests
skipped in that invocation. Strict all-target/all-feature Clippy, workspace doctests, rustdoc with
`-D warnings`, and workspace/consumer formatting passed. The numerical review's two concrete
counterexamples have executed Rust regressions; the final read-only review found no outstanding
concrete P0-P2 issue in its reviewed scope and does not replace runtime evidence.

The normal-stack Windows release all-feature consumer passed the glTF/OBJ/PLY/STL workflows.
Freshly unpacked `.crate` dependencies built and executed the postprocess and selected glTF cases
without checkout-path substitution. The postprocess case verifies preserved authored normals,
explicit replacement, exact UVs, and reports that remain readable after Scene destruction. The
release `wasm32-wasip1` all-feature consumer passed the same format workflows under the previously
checksum-qualified Wasmtime 50.0.0 runner. The `wasm32-unknown-unknown` all-feature consumer passed
compilation only; no browser runtime result is claimed.

Logs are `target/review/f-normals-workspace-final.log`, `f-normals-numerical-final.log`,
`f-normals-clippy-final.log`, `f-normals-doctests.log`, `f-normals-rustdoc.log`,
`f-normals-native-consumer.log`, `f-normals-packaged-consumers.log`,
`f-normals-wasi-consumer.log`, and `f-normals-browser-wasm-check.log`. This increment provides no
new sanitizer/fuzz campaign, target-space conversion, or application vertex lowering.

## Typed Spatial Storage and Initialized Footprint Admission (2026-10-30)

`SceneTables` now owns spatial storage through the existing retained owner. It contains the actual
inline `SceneSpace`, camera projections, and typed light definitions. `NamespacedName` and
`ExtensionSchemaId` already store their bounded names inline, so the cutover needs no name pool,
range descriptors, duplicate convention enums, or scalar codec. Fresh construction admits both row
capacities before building. Node-transform, triangulation, and normal edits share the existing
spatial owner and leave declarations and reusable camera/light definitions intact.

Central validation borrows actual values, checks exact row and inline-name extents, and reuses the
existing node placement scratch for dangling references and orphan detection. The root directory
revision is five with ten fields; spatial is no longer a record reference. Its old schema,
materializer, validator/decoder, edit rewrites, and inherited name leaves have been deleted. The
remaining record-store ceiling and terminal attribution envelope exclude spatial rows and names.
The independent spatial snapshot transcript and golden digests retain their semantic encoding.

The integration regression corrupts twelve real stored components after construction: row counts,
units/axes/tangent declaration, inline-name extent, light tint/zero emission, dangling node
attachments, and orphaned definitions. Every case reaches a build-invariant rich failure, reserves
no generation, commits no canonical output, and releases all storage. Invalid raw tags and
incompatible category/payload combinations disappeared with the codec because the closed domain
types cannot represent them. Public reads cover both camera projections, all light kinds,
namespaced conventions, and shared placements after authoring input destruction without allocating.
Edit tests check actual owner identity, either generation's first release, and ordinary unwind.

The review also found that node and geometry canonical-byte limits still used retired record,
reference-row, and transform-leaf formulas. Those formulas and the empty-geometry-directory
placeholder are deleted. All migrated domains now count initialized typed headers, rows, and used
scalar/name pools under the target ABI. Matrix/TRS variants occupy the same node row; sparse
defaults and explicit values/indices are counted once without expansion. Fresh capacity plans
check the same sealed footprint. The current Scene directory contributes forty logical bytes
independently once; remaining record domains retain their current representation until their D2
cutovers. Physical capacity, retained-owner infrastructure, and process RSS remain separate concepts.

Geometry footprint planning reads only top-level lengths before scratch admission, then accumulates
actual payload/name counts during existing semantic traversal. The first shape read is a controlled
action, preserving zero-work rejection before scratch allocation. Removing the old descriptor
prepass reduces ordinary work; cancellation tests use actual action baselines and preserve bounded
polling, rejected-prefix accounting, and storage-first cleanup. The focused core suite passed
**354/354 tests**. The complete all-feature workspace suite passed **2,179/2,179 tests**, with three
manual scale tests skipped in that invocation. Strict all-target/all-feature Clippy, doctests,
rustdoc with `-D warnings`, and workspace/consumer formatting passed.

All three scale tests passed separately. The node edit's allocator-request peak was **19,732 bytes**
for both 10,000 and 100,000 four-influence vertices, at both 1 MiB and 8 MiB metadata sizes; the
preceding profile checkpoint recorded 21,284 bytes. Independent buffers at 256/1,024/4,096 and
shared-payload edits at 1,000/10,000 nodes passed their existing work, sharing, and release assertions.
These are observed requested-allocation and ledger results, not process-RSS measurements.

The Windows release all-feature consumer passed glTF/OBJ/PLY/STL workflows on the ordinary main
stack. Freshly unpacked `.crate` dependencies built and executed postprocess and selected glTF
consumers without checkout-path substitution. The release WASI all-feature consumer passed the
same format workflows under Wasmtime 50.0.0. Browser WASM passed compilation only. Read-only review
identified the canonical-byte inconsistency and missing initial work admission described above;
both fixes have executed regressions, and the follow-up review found no additional concrete P0-P2
issue in the reviewed scope. No new sanitizer/fuzz campaign or coordinate-conversion result is claimed.

Logs are `target/review/d2-spatial-workspace-nextest.log`, `d2-spatial-footprint-core-final.log`,
`d2-spatial-storage-scale.log`, `d2-spatial-workspace-clippy.log`,
`d2-spatial-workspace-doctests.log`, `d2-spatial-workspace-rustdoc.log`,
`d2-spatial-native-consumer.log`, `d2-spatial-packaged-consumers.log`,
`d2-spatial-wasi-consumer.log`, and `d2-spatial-browser-wasm-check.log`.

## Whole-Scene Static Spatial Conversion (2026-10-30)

The direct postprocess boundary now supports `convert_scene_space` with a complete validated
`TargetSpace`: handedness, signed up/front axes, meters per unit, and front-face winding. Explicit
source assumptions fill only unknown declarations. Matching known facts are accepted; a conflicting
fact or missing source convention returns a typed reason and the original Scene. The target is never
used to guess source units. Filling unknown declarations publishes a change; an already satisfied
target with no filled assumptions preserves the exact input generation.

The redundant `TransformConvention` tag is removed. Column-vector matrices and local-to-parent
authored node transforms are fixed canonical semantics. Matrix domains such as inverse bind retain
their own source/destination meaning. The deterministic snapshot schema is revision 4. Handwritten
vectors and byte totals remove only the two-byte tag/value and update the revision word. The OBJ
capability fixture additionally reconstructs its previous pinned transcript by restoring exactly
those bytes, constraining the changed digest to the declared schema revision.

The signed-permutation arithmetic transports positions with a positive uniform unit ratio, normals
and tangent directions without normalization, and tangent `w` by the basis determinant. Node
conjugation preserves authored Matrix/TRS form, shear, singular scales, hierarchy, and attachments.
Triangle reversal follows determinant XOR source/target winding parity; indices and every Corner
tuple move together. Nonindexed triangles gain explicit indices while retaining Vertex identity.
Sparse defaults, explicit entries, and explicit values equal to defaults retain their representation.
The geometry plan rebuilds compact scalar pools in canonical order without input-sized sorting scratch.

The former geometry-only transaction is now a closed Scene-edit transaction with an explicit
whole-Scene target. Resolution borrows the exact input publication. Existing activation, central
validation, failure preparation, generation reservation, and joint handoff remain the sole protocol.
Declaration-only edits share node/geometry owners; winding-only edits share nodes. The replacement
spatial owner changes only the five target facts. Profile owner indices and metadata values remain
stable; each affected owner derives from its own attachments and the original Scene's attachments.
Reports own the actual validated evidence root and read conversion facts directly from it.

The current capability boundary rejects numeric conversion with actual skins, morph deltas,
animation channels, cameras, lights, or custom attributes. It also rejects unhandled spatial texture
coordinate sources, nonzero Scene-unit thickness/displacement during a unit change, and reflected
tangents with an encoding other than `SignInW`. Nonempty polygon/strip/fan topology needing reversal
requires separately implemented lowering. Winding-only conversion can preserve otherwise unaffected
domains and opaque custom Corner tuples. These are explicit current limitations, not completion of F.

Independent read-only review checked sparse remapping, source binding, owner provenance, matrix and
quaternion arithmetic, and build/correspondence/validation work. It found an additional limit gap:
the transaction's caller-provided scratch ceiling covered the sealed plan but omitted central
validation scratch. Preflight now checks the actual rebound scratch envelope before sealing.
The shared allocator/ledger admission remains unchanged.

The complete all-feature workspace suite passed **2,226/2,226 tests**, with three manual scale
tests skipped in that invocation. All three scale tests then passed separately. The four-influence
node-edit pilot retained its **19,732-byte allocator-request peak** at 10,000 and 100,000 vertices
with either 1 MiB or 8 MiB metadata. This is requested-allocation evidence, not process RSS.
Strict all-target/all-feature Clippy, workspace doctests, and rustdoc with `-D warnings` passed.
Public conversion tests cover the complete triangulation/normal/conversion chain, sparse
explicitness, multiple Normal/Tangent domains, reflection versus independent winding, precise F64
opaque tuples under winding-only conversion, source assumptions/conflicts, unchanged results,
one-MiB caller stacks, detached reports, cancellation during planning/validation/publication, and
ordinary host-unwind release. Source-bound unit tests reject mutated actual descriptors/payloads
and check exact build/correspondence/central validation work.

The first freshly unpacked debug consumer exposed an additional ordinary-stack failure even though
the release consumer and individual one-MiB-stack operations had passed. Debugger tracing located
it in import's final typed-ledger detachment, reached from the complete caller workflow. The public
consumer functions were left intact and the Windows main-stack reservation remained 1 MiB.
Postprocess activation, central validation, and publication now have separate concrete private
phase boundaries. Import's central rejection, interruption, and deferred-fallback preparation also
have separate frames, so a successful import does not reserve their mutually exclusive temporary
owners. The linear ownership protocol, source Scene, scratch-before-preparation order, control-cause
precedence, and allocation-free rejection path are preserved.

PE unwind/PDB inspection measured `execute_changed` at **211,240 → 94,360 bytes** and import's
`prepare_scene` at **106,056 → 67,144 bytes** in the diagnostic debug consumer. The complete source
debug postprocess consumer then passed on the unchanged default main stack. These measurements are
specific compiled-frame observations, not a portable stack bound or a new generic instrumentation
framework. Temporary progress probes were removed before that successful run.

After the phase refactor, the complete workspace suite again passed **2,226/2,226** and strict
Clippy passed. The unchanged full Windows consumer passed in both debug and release with all
features and all four format workflows. A new unpacked artifact run built and executed the
postprocess and selected glTF cases from `.crate` dependencies, including the maintained vendor
packages; it verified that no local dependency resolved back to checkout source. The release WASI
all-feature consumer executed the same four format workflows under Wasmtime 50.0.0. Browser WASM
passed compilation only. The standalone empty-feature release consumer also passed. These checks
do not admit the currently rejected spatial domains or finish application vertex lowering.

Final logs are `target/review/f-space-workspace-nextest-final.log`, `f-space-clippy-final.log`,
`f-space-storage-scale.log`, `f-space-doctests-final.log`, `f-space-rustdoc-final.log`,
`f-space-native-debug-all.log`, `f-space-native-release-final.log`,
`f-space-packaged-consumers-final.log`, `f-space-wasi-consumer.log`, and
`f-space-browser-wasm-check.log`. The diagnostic stack trace, compiled-frame observations, and
unchanged PE stack reservation are retained as `f-space-stack-trace.log`,
`f-space-stack-frames-final.txt`, and `f-space-stack-header.log`. The initial failed package run
remains in `f-space-packaged-consumers.log` to distinguish the discovered defect from the repaired
artifact verification. No new sanitizer or sustained fuzz campaign is claimed by this increment.

## Remaining acceptance

- Complete final acceptance after the remaining architectural units. The current combined suite
  closes the B/D1/E integration checks recorded above; later changes need their own validation.
- Preserve B's verified retained manifest/evidence ownership and fidelity behavior through the
  remaining storage and postprocessing cutovers.
- Preserve C's verified typed reads and required-dependency context through the storage cutover; retain the single Scene owner.
- Preserve all seven completed typed domain cutovers and normal generation's concrete
  attribute-owner mapping. Keep validation borrowed and unchanged owner sharing intact through the
  remaining postprocessing work; do not reintroduce retired record/projection paths.
- Retain E's exact selected-profile restrictions and sanitizer coverage through later storage changes.
- Preserve F's triangulation, normal generation, and static target-space conversion while closing
  the remaining spatial domains and application vertex lowering.
- Finish G's remaining behavioral growth checks and H's current-stable/release package matrix,
  CI execution wiring, and native candidate acceptance. Dependency upgrades, package-content fixes,
  and the recorded fixed artifact matrix already have their distinct implementation/evidence above.

No change here asserts exact RSS from ledger counters, sustained fuzz coverage, completion of
the full geometry workflow, or publication readiness.
