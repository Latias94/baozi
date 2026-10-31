# Library Delivery Progress — 2026-10-29

This is implementation evidence for the [active A–H plan](../plans/2026-10-29-001-refactor-library-delivery-plan.md),
not release admission or whole-plan completion. The baseline reviews remain historical observations.
Source changes below were exercised in the working tree on Windows x86_64 with Rust 1.95.0 unless
another toolchain is named. Commands use one Cargo build job and reuse `target`.

## Delivered increments

| Unit | Implemented behavior | Runtime evidence |
| --- | --- | --- |
| A1 | Compact references, admitted execution-plan storage, borrowed proof access, smaller phase transitions, and one owned terminal ledger remove cumulative stack movement. | Facade child-process regression imports, captures a snapshot, rejects malformed input, and returns cancellation on a 1 MiB thread. Standalone default consumer runs on the ordinary main stack. |
| A2 | Healthy final and unpublished owners release during host unwind. Callback output/proof/cause owners isolate actual destructor failure while preserving the original caller panic. | The final-owner allocation probe observed zero net allocations/bytes. Four new intermediate-owner regressions first failed on the old guards, then passed; the integrated operation suite passes 721 tests with the original destructor/poison cases retained. Final workspace and consumer qualification follows below. |
| A3 | Registered proof schemes reside in one named static array rather than relying on promoted wrapper addresses. | Four focused IO identity tests passed on 1.95 debug and 1.99 debug/release. IO 47 and native-fs checker 50 tests passed before this increment's lockfile refresh. |
| A4 | Executable sidecar example owns its PNG at the correct MTL-relative path; README and commands match real APIs. | All four examples and README quick start ran using the repaired 1.95 library. Typed-reader examples are rechecked at C's integration boundary. |
| B fidelity | Partial OBJ channels require specific loss authority; homogeneous positions divide by finite nonzero W; three-component UVs retain per-segment width; unsupported color suffix omission is explicit. PLY F64 custom values remain F64. STL multiword names preserve full trimmed text. | OBJ/facade integration 182/182 passed. PLY custom-attribute suite 5/5 passed, including ASCII/LE/BE exact values and nonfinite/budget precedence. STL name/diagnostic regressions passed in its subsequent suite. |
| B resource work | Short reads reuse admitted byte credit; EOF probes are bounded read work; payload ceilings and live buffer capacity are independent. Hashes use 64 KiB control boundaries and immutable dependency snapshots retain one digest. Typed geometry validation reads actual compact ranges without the retired record-buffer cache. | The complete cache increment meters every physical-cache probe/replacement and graph lookup, with original-cause preservation, finite growth budgets, and transactional rejection tests. Current dual-toolchain workspace and integrated evidence are recorded below; the early 804-test calibration failures remain historical observations in their original sections. |
| C reading | One immutable Scene exposes borrowed nodes/roots, geometry, scalar payloads, placements, materials/bindings, encoded images/samplers/textures, space declarations, cameras/lights, and checked typed IDs. | Dense/sparse/presence/spatial tests passed, including zero counted allocations during repeated reads. Facade geometry and appearance tests passed in 182/182. Independent default OBJ/STL consumer passed outside workspace feature unification. |
| C authoring/errors | Static plugins use a borrowed builder with metered default alignment and primary-document provenance. Node, geometry, appearance, deformation, animation, spatial, profile, and aggregate errors preserve bounded semantic detail through finalization. | Nine static-plugin and eight submission-error tests passed, including domain/rule/value cases, later terminal cancellation, nested material rules, and digest reuse. Required-dependency context retains the declaring origin, role, original reference, normalized path, and resolver-authorized target. Nested missing/denied/redirected/rejected-reference tests passed in the integrated workspace run. |
| F application workflow | Triangulation, normal generation, complete admitted Scene-space conversion, and borrowed draw-order tuples compose without a second Scene model or owning layout runtime. Applications select their concrete fields and own the final buffers. | Six typed-element and nine public tuple tests pass. The independent consumer builds vertex/index buffers after the geometry operations, checks them after dropping the Scene, and executes in native debug/release, fresh packages, and WASI. See the final increment evidence below. |
| G feature/modules | Empty `std` and unused tracing forwarding are removed. Facade errors and importer workflows have separate modules; compiled-in registration is named `builtin`. Typed readers have private domain modules. The later cutover extracts native IO into its own unpublished workspace and replaces the core bridge feature with actual private/linear contracts. | The earlier portable feature matrix and the newer focused core/candidate checks have distinct evidence below. Candidate extraction and hidden integration are implemented; their final integrated target/package qualification follows their changed source. |

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

## Deformation and Animation Space Conversion (2026-10-31)

Whole-Scene conversion now includes skin inverse binds/corrections, dense and sparse morph
Position/Normal/Tangent deltas, and translation/rotation/scale animation values and cubic
derivatives. The former separate domain-plan argument list is replaced by one closed
`SceneSpacePlan`. It binds all four domain recipes to the exact same `SceneTables` and conversion,
performs complete numeric preflight, and retains only borrowed plans and fixed costs.
Materialization and correspondence use that same plan. Same-generation sibling roots are
rejected before profile traversal; individual domain plans cannot be replaced independently.

Inverse binds and corrections use `H M H^-1`; morph positions and translation curves/derivatives
use `sQ`, morph directions use `Q`, scale curves permute unsigned axes, and quaternion values and
arbitrary derivatives use `(det(Q) Q xyz, w)`. Source order, f64 times/rates, interpolation and
extrapolation, names, sparse presence/indices, influences, and weights are preserved. The operation
does not normalize quaternion derivatives or choose a canonical quaternion sign.
Numeric overflow/underflow reports skin-correction, inverse-bind, morph tuple, or channel/stream/key
coordinates before admitting retained output. Failure returns the original immutable Scene.

Only deformation/animation owners with changed scalar bits are rebuilt. Winding-only and
declaration-only edits share populated owners. Changed Skin, MorphDelta, and AnimationChannel
owners derive their lineage from their own attachments plus the input Scene's attachments.
Lineage and state construction each admit their own complete selection pass. The selected glTF
shape exercises exactly 44 additional actions per pass; unchanged weights and curves are still
visited, and a first change never short-circuits the remaining work.

Local plan tests check exact construction/correspondence work, every interrupted action, caught
unwind cleanup, actual-table corruption, and late numeric failures. Retained-graph tests check
source binding, changed-owner parent unions, actual sharing, exact correspondence, and interrupted
candidate cleanup back to the source account's live-byte/allocation baseline. Six public
integration tests read handwritten matrix/morph/curve expectations, preserve q/-q and signed-zero
bits, reject seven numeric failure cases, run populated conversion on a 1 MiB stack, and measure
zero net allocations/bytes after an ordinary host unwind through published Scene/report owners.
The postprocess suite passed 98/98 tests after correcting an incomplete tangent declaration in the
new test fixture. Independent read-only correctness review found no additional concrete defect
in exact-source binding, selection work, conditional rebuilding, or shared-owner correspondence.

The complete all-feature production workspace suite passed **2,290/2,290 tests**, with four
manual scale pilots skipped. Strict all-feature/all-target workspace Clippy, workspace doctests,
rustdoc with `-D warnings`, and workspace/standalone-consumer formatting passed. The full Windows
all-format consumer executed in both debug and release on the ordinary main stack, including new
handwritten assertions over the admitted glTF Scene's converted binds, morphs, animation values,
and cubic derivatives.

A fresh all-feature archive consumer built and executed from
`target/package-check-dc2zfq27`, with its local dependency closure verified to resolve only to
newly extracted `.crate` artifacts. The checker also verified package README/license content.
The release WASI all-format consumer executed the same workflows under the previously
checksum-qualified Wasmtime 50.0.0 installation. Browser WASM passed compilation only.
These results do not establish a new sustained fuzz/sanitizer campaign or final current-stable
publication matrix.

Logs are `target/review/f-space-domains-workspace-final.log`,
`f-space-domains-clippy-final.log`, `f-space-domains-doctests.log`,
`f-space-domains-rustdoc.log`, `f-space-domains-postprocess-final.log`,
`f-space-graph-cancel.log`, `f-space-domains-native-debug-final.log`,
`f-space-domains-native-release-final.log`, `f-space-domains-packaged-final.log`,
`f-space-domains-wasi-final.log`, and `f-space-domains-browser-wasm-final.log`.

Camera/light local-frame conversion, material Scene-unit/spatial-texture behavior, and application
vertex unification remain open F work. This increment does not complete the A–H plan.

## Complete Camera/Light Values and Spatial Conversion (2026-10-31)

Camera/light conversion now uses complete intrinsic `LocalAxes` rather than generated compensation
nodes. The fixed optical reference remains right +X, up +Y, forward -Z, with reference-to-node
matrix `B = [right, up, -forward]`. All 48 independent signed-axis frames, including reflections,
are admitted. Conversion composes `B' = Q B` while node placement uses `H M H^-1`. Node counts,
child order, attachments, geometry references, and animation targets remain structurally stable
through repeated conversions. Source-local authored extrinsic poses still belong to nodes.
ADR 0040 is revised with this distinction and its physical evaluation equation.

`CameraProjection` now exposes typed Perspective/Orthographic payloads containing their axes and
parameters. `LightDefinition` couples kind, axes, and canonical emission. Frame-free camera
Kind/tuple getters and independent light scalar readers are removed; public light readers return
the complete definition. Authoring and spatial storage use these same compact owned values,
deleting `CameraAuthoring`, `LightAuthoring`, their error wrapper, and `StoredLight`. Static plugin
builders accept camera/light slices and create document provenance for both, using the existing
bounded builder and single central publication path.

One source-bound `SpatialSpacePlan` replaces the declaration-only rewrite and separate spatial
correspondence path. It scales all camera distances/orthographic half-extents and finite light
ranges, preserves FOV/aspect/cutoff absence/cone angles and photometric emission bits, and rejects
overflow, underflow, or rounded plane-order collapse before retained output admission. The complete
Scene plan includes its fixed work and names; profile selection records actual Camera/Light
changes with their own and Scene parents, using the existing fixed-size owner actions.

Semantic snapshot encoding is revision 5. Three tagged axes follow each camera/light reference.
Five hand-written transcript vectors change only the revision word and those declared fields;
independent SHA-256 calculation verifies every new digest and reproduces its revision 4 predecessor
after reversing precisely those bytes. The spatial vector is 1,322 bytes and 140 snapshot actions.
The OBJ capability fixture independently restores its revision 4 transcript digest. Axis mutation
tests cover all three axes for perspective, orthographic, and light definitions.

The core suite passed 532/532 tests before the final additional retained-profile regression;
that regression then passed independently. It checks unchanged node layout, actual Camera/Light
parents, preserved emission bits, exact complete correspondence work, and final physical release.
The 17 spatial-plan tests cover handwritten values, all 48 frames, actual-storage corruption,
post-rounding numeric constraints, exact work, and interrupted/unwound buffer cleanup. Five public
camera/light integration tests passed as part of the 103/103 postprocess suite: shared definitions,
off-axis reference-point evaluation, repeated inverse conversion without node growth, eight typed
numeric failures returning the original Scene, a 1 MiB caller stack, and host-unwind allocation
cleanup. Independent read-only review found no concrete P0–P2 fault in the reviewed production paths.

The integrated all-feature production workspace suite passed **2,317/2,317 tests**, with four manual
scale pilots skipped. Strict workspace all-feature/all-target Clippy, doctests, rustdoc with
`-D warnings`, and workspace/standalone-consumer formatting passed. The all-format consumer ran
on the ordinary Windows main stack in debug and release. Its custom plugin uses the public static
builder to publish and read complete camera/light definitions, executes the reflected/permuted
unit conversion, and checks inverse axes and fixed node count.

The same all-feature consumer built and executed from fresh `.crate` archives in
`target/package-check-97rl88tx`; normalized metadata, README/licenses, and the extracted-only local
dependency closure were checked before compilation. The release WASI consumer executed under the
previously checksum-qualified Wasmtime 50.0.0 installation. Browser WASM passed compilation only.
No sustained fuzz/sanitizer result, current-stable publication matrix, or remaining material/vertex
workflow completion is inferred from these checks.

Logs are `target/review/f-camera-workspace-final.log`, `f-camera-clippy-final.log`,
`f-camera-core-final.log`, `f-camera-profile-check.log`, `f-camera-postprocess-final.log`,
`f-camera-handwritten-goldens.txt`, `f-camera-doctests-final.log`, `f-camera-rustdoc-final.log`,
`f-camera-native-debug-final.log`, `f-camera-native-release-final.log`,
`f-camera-packaged-final.log`, `f-camera-wasi-final.log`, and `f-camera-browser-wasm-final.log`.

## Material Lengths and Independent Appearance Ownership (2026-10-31)

The appearance root now contains two independently retained children. `AppearanceRows` owns the
five image/sampler/texture/binding/material tables. `AppearancePayloads` owns the encoded-image
and UTF-8 pools. `AppearanceStorage` is inline in `SceneTables`, without another retained header.
Fresh construction admits and builds the two children separately. Central validation and semantic
reads borrow those actual rows and pools. The initialized canonical footprint counts the inline
root, both data headers, and the seven used buffers; operation separately admits the physical
shared headers, backing capacities, and infrastructure. Shared payloads keep one physical charge
until their own final handle is destroyed.

`AppearanceSpacePlan` is part of the same exact-source `SceneSpacePlan`. It scales supplemental
material thickness and signed Displacement strength by the positive unit ratio, with a binary64
intermediate and one checked binary32 rounding. Reflection does not change displacement sign.
Signed zeros and representable subnormals survive; overflow and nonzero underflow to zero return
typed material/binding locations before retained output admission. Other factors, independent
optional groups, alpha, UV transforms, binding strengths, names, source keys, and order keep their
bits and presence. Unknown custom-attribute spatial meaning remains an explicit capability error.

When converted appearance bits change, execution retains only new rows and shares the exact
original payload owner. It copies no image/text bytes and adds zero rebuilt-name bytes. If no bits
change, both owners remain shared. Correspondence checks payload-owner identity in constant work
and exhausts the expected rows, rejecting an equal-byte pool from a different owner. Material and
TextureBinding provenance changes only for rows whose own converted bits differ, with the union of
their own and Scene parents; copying unchanged descriptors does not change their lineage.

The undefined generated-coordinate variants and their enum/error/wrapper paths are removed.
Authoring, storage, and public reads directly use `uv_set: u16`. glTF checks its effective base or
transform-override index before narrowing; OBJ uses zero. UV 0, ordinary sets, and 65535 remain
explicit, independently of whether a particular primitive has that stream. Existing UV snapshot
framing remains kind zero plus a `u32` index, so revision 5 and its UV transcript bytes do not change.
ADRs 0012, 0032, and 0045 and the model/read documentation describe these boundaries.

Read-only review also found a stale bootstrap estimate for deleted appearance/deformation/animation
record layouts. The former maximum of 41 record blocks and presence-based formulas are removed.
Fresh startup and materialization now use the one remaining directory-record shape; core terminal
attribution needs its directory plus one terminal record. Typed-owner retention remains separately
admitted. Top-level input checks still reject count/limit overflow without walking nested payloads;
large invalid nested fixtures and zero-allocation checks cover that distinction.

The final all-feature workspace suite passed **2,406/2,406 tests**, including xtask, with four
manual scale pilots skipped. Strict all-target/all-feature Clippy, workspace doctests, rustdoc
with `-D warnings`, formatting, and diff checks passed. The retained appearance regressions verify
changed/unchanged owner identity, exact correspondence work, distinct own+Scene parent markers,
source-before-output destruction, and every cancellable retention checkpoint. The full glTF
physical-allocation quota sweep continues through every reachable rejection until first success;
each failed quota commits no canonical output, reserves no generation, and leaves the released
account healthy. This sweep does not identify a particular rejected shared header by phase.

All six public appearance conversion tests passed as part of the **109/109** postprocess suite.
They check factor-group presence and bits, signed displacement under reflection, zero/subnormal
boundaries, typed original-input numeric failures, repeated inverse conversion, and ordinary host
unwind with zero remaining counted allocations. The 1/8 MiB image test requires identical payload
and text addresses plus equal requested allocations/bytes/peak, operation resource peaks, committed
canonical bytes, and attempted/consumed work. The separate manual shared-image node-edit pilot also
passed: both sizes used a requested-allocation peak of **8,444 bytes**, ledger peak **9,124 bytes**,
and **226** attempted/consumed work items. These are measured allocation/accounting values, not RSS.

The independent all-format consumer executed on the ordinary Windows main stack in debug and
release. Its static plugin authors material lengths, direct UV indices, and encoded image bytes,
then checks reflected conversion, inverse values, optional group/zero bits, and image/name/key
address sharing through public APIs. The same consumer built and executed from fresh archives in
`target/package-check-6l3x5ff_`, with normalized metadata, README/licenses, and extracted-only local
dependency closure checks. The release WASI consumer executed under the existing checksum-qualified
Wasmtime 50.0.0 installation; browser WASM passed compilation only. No new sustained fuzz/sanitizer
or current-stable publication qualification is inferred from this increment.

Logs are `target/review/f-material-workspace-final.log`, `f-material-retention-final.log`,
`f-material-clippy-final.log`, `f-material-doctests-final.log`, `f-material-rustdoc-final.log`,
`f-material-image-pilot-final.log`, `f-material-native-debug-final.log`,
`f-material-native-release-final.log`, `f-material-packaged-final.log`,
`f-material-wasi-final.log`, and `f-material-browser-wasm-final.log`.

## Borrowed Application Triangle Tuples (2026-10-31)

`AttributeRef::element` now returns typed F32/F64/U32/I32 element slices borrowing the actual
Scene. Dense selection is constant time; sparse selection uses its sorted explicit indices and
borrows either the value or retained default. Logical bounds are checked before indexing, including
`usize::MAX`. The original descriptor and dense/sparse representation remain available without an
owned expansion, precision conversion, or scalar normalization.

`postprocess::triangle_vertices` supplies one complete application tuple per draw Corner of an
already validated TriangleList. Application index `c` uses source Corner `c`, source Face `c / 3`,
and the source Vertex addressed by that corner, or implicit Vertex `c`. All Vertex/Corner/Face
fields retain their complete semantic/domain keys and scalar types. Full-field iteration visits
the descriptor sequence once per tuple rather than repeating keyed scans. Whole influence rows
and requested morph deltas follow that same source Vertex. Morph targets resolve through the
current primitive; generation/logical-mesh qualification and per-primitive stream differences
remain intact. Skin palettes and effective morph weights remain instance choices.

The layout, tuple, and field views have only Scene borrows and fixed-size coordinates. Their
construction, checked selection, and full traversal allocate no heap storage. There is no new Scene,
owned map, deduplication kernel, artifact runtime, invocation, or resource report. Equal tuples may
repeat, and unreferenced source vertices remain untouched in the Scene. The application owns its
buffer sizing, fallible allocation, extraction loop, selected fields, and index packing. This
concrete reader satisfies tuple lowering without imposing a premature owning-artifact architecture.
ADRs 0030/0032, Scene IR/read documentation, and the crate README describe the boundary.

The independent consumer now completes import, typed reads, triangulation, normal generation,
target-space conversion, and concrete application vertex/index buffer construction. It chooses
position, normal, and UV source domains explicitly, then destroys the Scene and compares both
owned buffers with their complete expected contents. The implicit application index stream is
`0..corner_count`, without a claim of minimum vertex count, welding, truncation, or quantization.

Six core element tests and nine public tuple tests passed. Coverage includes original scalar bits
and backing addresses, complete semantic/domain order, UV seams, Face fields, simultaneous Vertex
and Corner normals/UVs, all supported scalar and sparse cases, empty and implicit draws, repeated
indices, degenerate faces, unreferenced source vertices, and qualified source endpoints. Deformer
fixtures check different streams for two primitives of one logical target, complete influence rows,
two instance palettes/weight overrides, and foreign-generation/mesh rejection. Borrowed elements,
influences, and iterators survive temporary layout destruction. Full tuple/deformer traversal
allocates zero bytes. Independent read-only reviews found no concrete P0–P2 fault in the reviewed
source-ownership and domain-selection paths.

The final all-feature workspace suite passed **2,421/2,421 tests**, including xtask, with four
manual pilots skipped. Strict workspace all-target/all-feature Clippy, doctests, rustdoc with
`-D warnings`, formatting, and diff checks passed. The all-format consumer executed on the ordinary
Windows main stack in debug and release. It also built and executed from fresh archives in
`target/package-check-719ag18s`, after README/license, normalized metadata, and extracted-only local
dependency closure checks. The release WASI consumer executed the same final-buffer workflow under
the existing checksum-qualified Wasmtime 50.0.0 installation; browser WASM passed compilation only.
These checks establish the implemented F workflow, not sustained fuzz coverage or final H release
qualification.

Logs are `target/review/f-vertices-elements.log`, `f-vertices-layout-final.log`,
`f-vertices-workspace-final.log`, `f-vertices-clippy-final.log`, `f-vertices-doctests-final.log`,
`f-vertices-rustdoc-final.log`, `f-vertices-native-debug-final.log`,
`f-vertices-native-release-final.log`, `f-vertices-packaged-final.log`,
`f-vertices-wasi-final.log`, and `f-vertices-browser-wasm-final.log`.

## Indexed Resource Lookup and Local Depth Planning (2026-10-31)

Resource-graph identity lookup now reuses the physical dependency cache's authority-qualified
index and its existing cache-slot/node binding. The primary remains one direct complete identity
comparison; available dependencies require an available bound cache record. No second identity map
or full node scan remains. The shared probe loop checks control and charges actual examined slots.
Resolving/failed/unbound records and foreign authorities cannot alias an available graph node.

Existing-target depth planning uses the strict committed-edge depth invariant to accept an already
deep-enough target without scratch or descendant traversal. A changed leaf needs one update. Other
connections walk only the reachable region iteratively and merge longest depths in reverse
postorder. Epoch-tagged marks are part of the admitted graph row; local stack/order/update owners
retain their actual capacity charges. Whole-graph scratch initialization and its helper are removed.
Cycle, depth-limit, work, and cancellation failures preserve committed depths and release scratch.

Completion checks the relation count captured by target preparation and rejects stale plans after
interleaved commits. All validation, path allocation, accounting, and final control checks precede
the write phase. Available node/relation consumption is grouped before publishing nodes, cache
bindings, links, and depth updates. Primary and unavailable commits follow the same final-check
discipline. Tests cover stale source depth/back edges and late rejection without partial state.
ADR 0041 records these internal contracts.

The final all-feature workspace suite passed **2,434/2,434 tests**, with four manual pilots skipped;
the full importer portion separately passed **401/401**. Strict Clippy, doctests, rustdoc with
`-D warnings`, formatting, and diff checks passed. Independent read-only reviews found two concrete
P2 gaps in cancellation and prepared-plan freshness; both were fixed and their regressions passed.
The all-format independent consumer also executed on the ordinary Windows debug main stack,
including external-resource inputs and the final application-buffer workflow.

The new default node growth test builds and edits reverse-ID deep chains and ordered wide trees at
256/1,024/4,096 nodes. Deep build work was **16,496 / 64,880 / 258,416**, wide build work
**16,750 / 65,902 / 262,510**, and edit work for each shape **7,377 / 28,881 / 114,897**. Each
fourfold input satisfies increasing work bounded by four times the previous value. Complete
published structure, child order, edited transforms, source-first destruction, and healthy release
also passed. This bounded fixture runs in under one second locally and no longer needs manual
selection.

The resource test executes 64/256/1,024 unique resources followed by N references to the final leaf.
One recorded run used graph work **888 / 3,486 / 14,159** for unique insertion and
**832 / 3,584 / 15,360** for reuse. It checks independent finite probe budgets, exact fixed-action
plus measured-probe reuse work, and manifest node/occurrence/multiplicity/depth semantics. These
RandomState-dependent observations are finite regression evidence, not worst-case bounds for every
seed. Manifest sorting has its separate work dimension.

This increment does not close all cache processing accounting: review identified pre-existing
uncontrolled acquisition-state, insertion/transition/removal, and index-rebuild paths. Their
storage admission and one-unit physical-open budget do not cover their actual loops. The next
increment must meter those paths through the same index/control mechanism and revise growth
formulae from the actual additional work. No full-cache metering or new fuzz/target claim is made
from this graph-specific test.

Logs are `target/review/g-resource-focused-final.log`, `g-resource-import-final.log`,
`g-node-growth.log`, `g-resource-workspace-final.log`, `g-resource-clippy-final.log`,
`g-resource-doctests-final.log`, `g-resource-rustdoc-final.log`, and `g-resource-native-debug.log`.

## Complete Physical Dependency-Cache Work Accounting (2026-10-31)

Every production cache operation now uses the same controlled identity-probe loop: acquisition
state lookup, resolving insertion, available/failed transition, removal, and graph target lookup.
The cache methods accept the concrete import host and return work/control errors separately from
missing or unusable records. Constant-time index-addressed views remain host-free. No second map,
work dimension, or cache-control framework is introduced.

Capacity planning remains constant time. Applying a replacement admits and initializes candidate
index storage in at most 256-slot controlled chunks, charges every scanned record slot and
reinsertion probe, and qualifies the known-length bulk record move before its final control check.
The previous duplicate sentinel fill is removed. Live entries/index/free-list/tombstones remain
unchanged throughout fallible construction; successful completion swaps the qualified candidates.
The graph wrapper restores the prior reservations on candidate failure.

Scope handling preserves the first terminal ledger/control cause. A failed available transition
ends its mutable graph borrow before cleanup. Further cleanup and unavailable-relation attempts
cannot replace the original operation error, and terminal non-optional acquisition failures skip
failed-cache insertion instead of letting another probe overwrite that cause. Optional
acquisition-local budgets keep their existing unavailable-input contract. Graph work errors cannot
be relabeled as a missing transition or consumed by optional-input fallback. Refused cleanup leaves
the resolving row owned by the session until terminal destruction, without an unmetered scan.

Growth evidence now separates physical-cache warm-up from graph connections. For 64/256/1,024
fixed-size identities, warm-up accounts for initializations, moves, rebuild scans and all miss,
insertion, and transition probes. Independently bounded acquisition lookups then determine the
expected query component: unique graph connections use `12N + 2 * sum(probes)`, and N references to
one existing leaf use `12N + 2N * hot_key_probes`. Logical nodes, relation occurrences, multiplicity,
maximum depth, and release are checked independently. The finite randomized-table budgets are
regression thresholds, not universal worst-case complexity claims. These supersede the preceding
increment's graph-only work formulae.

The final all-feature workspace suite passed **2,441/2,441 tests**, with four manual pilots
skipped. Strict all-target/all-feature Clippy, workspace doctests, rustdoc with `-D warnings`,
formatting, and diff checks passed. The all-format independent consumer executed the full typed
read and application-buffer workflow on the ordinary Windows debug main stack.

The completion regression covers available and unavailable cache transitions, a real
snapshot-sharing live `ScratchBytes` rejection, and acquisition rejection on Required and strict
Optional requests, each followed by exhausted graph work. Every case retains
the original dimension, counter class, value, and limit, publishes no optional/loss evidence or
Scene, and releases all owned storage without poisoning. Cache tests also interrupt candidate
initialization/rebuild and reject the final commit checkpoint without modifying live cache state.
These results qualify the integrated cache change; final package, current-stable, and Linux
sanitizer checks remain separate H work.

Logs are `target/review/g-cache-first-cause-final.log`, `g-cache-workspace-final.log`,
`g-cache-clippy-final.log`, `g-cache-doctests-final.log`, `g-cache-rustdoc-final.log`, and
`g-cache-native-debug-final.log`. The focused cache/growth run is retained in
`target/review/g-cache-scope-growth.log`.

## Final Integrated Linux ASan Smoke (2026-10-31)

The immutable `34b12dd5a9bb40c9e719badbae42f071b1485deb` Git archive executed all five
current targets on Ubuntu 26.04 x86_64 under WSL2: `stl_import`, `obj_import`, `ply_import`,
`gltf_import`, and `registry_detection`. Each completed **256 runs**, with a 10-second input
timeout, 4,096 MiB RSS limit, and 120-second campaign limit. Format inputs used a 65,536-byte
maximum; registry detection used 4,106 bytes. Format targets started from the archive's committed
seeds; registry detection started from an empty corpus. All resulting corpus entries and logs are
retained separately from the tracked seeds.

The actual compiler was `rustc 1.101.0-nightly (92ffebece 2026-10-28)`, LLVM 23.1.4,
with cargo-fuzz 0.13.2. Locked offline metadata succeeded before execution, and the archive's
fuzz lockfile remained byte-identical afterward. All five executed binaries contain `__asan_init`
and `__asan_report_load1`. There was no parser panic, abort, or sanitizer finding in these bounded
runs. This qualifies the exercised parser, validation, and snapshot paths in the completed typed
domain and integrated graph/cache source state. It does not exercise every postprocessing operation
or establish sustained fuzz coverage or native-filesystem promotion.

The local verification helper initially stopped before registry startup because that target has no
committed corpus directory. Registry execution then completed from an explicitly empty corpus
against the same archive, without repeating or replacing the four successful format results.
The source/toolchain, instrumentation, individual target logs, and resulting corpus are retained
in `target/review/h-linux-asan-34b12dd5a9bb-jpoh52qb/`; orchestration logs are
`target/review/h-linux-asan-final.log` and `h-linux-asan-registry.log`.

The later immutable `10495f31d1aaa32478d78cc80018160ec14efae2` archive repeated all five
targets successfully with the same compiler, sanitizer, locked fuzz dependencies, and per-target
bounds after the MSRV API migration and native test/CI changes. This time all five targets ran in
one sequence, with an explicitly empty registry corpus. All five binaries again contain the ASan
symbols, and the lockfile remained unchanged. Logs, instrumentation, and resulting corpus are in
`target/review/h-linux-asan-10495f31d1aa-07iyk1u8/`; orchestration is in
`target/review/h-linux-asan-current.log`. This is evidence for that exact archive, not subsequent
ownership changes discovered during the remaining acceptance audit.

## Native Capability Replacement Races (2026-10-31)

Three default candidate tests each run 64 cooperating replacement rounds over prewritten immutable
files: final-file exchange, intermediate-directory exchange with an escaping directory link, and
file-symlink exchange between an in-root relative link and an outside link. A returned handle stays
alive until replacement completes; successful reads must match complete authorized bytes. Stable
positive and negative controls run between rounds. Windows directory replacement uses a real
junction. Only sharing violation 32 may defer exchange until handle release, after which that
exchange must succeed. This complements the existing deterministic same-handle identity test.

Unix additionally checks a relative multi-link chain with an in-root parent step, then changes only
its final link to escape. Linux opens a real `/proc/self/fd` magic link through a capability and
rejects it after a host read verifies the outside sentinel. These tests need no new dependency,
unsafe code, production hook, or alternate resolver. Coordination timeouts detect stalled test
communication, but cannot interrupt an OS call or establish a hard open deadline.

Rust 1.95.0 Windows candidate runtime IO passed **40/40**, repository IO **50/50**, and the
all-format candidate facade **55/55**, all without skips. The Windows repository result includes
its explicit `UnsupportedIdentityBackend` rejection before traversal, not operational capture.
On Ubuntu 26.04 x86_64 under WSL2, candidate runtime IO passed **43/43**, operational repository IO
**61/61**, and the candidate facade **55/55**, all without skips. The locked native dependency check
passed all 14 profiles. Strict IO all-target/all-feature Clippy, formatting, and diff checks passed.

These are candidate development results. Alias and permission additions are recorded below.
Windows unknown/volume-mount reparse fixtures, exact backend retry/fallback evidence, and actual
Apple ARM64 execution remain required before the complete promotion matrix can close.
The race schedules are bounded samples rather than a proof of every interleaving. Release
registration remains empty.

Logs are `target/review/h-windows-native-runtime.log`, `h-windows-native-repository.log`,
`h-windows-native-facade.log`, `h-linux-native-admission.log`, `h-linux-native-runtime.log`,
`h-linux-native-repository.log`, `h-linux-native-facade.log`, `h-native-clippy.log`, and
`h-native-fmt.log`.

The subsequent Unix permission fixture first verifies authorized reads, removes search permission
from an intermediate directory, and requires both actual host denial and typed capability
`PermissionDenied`. It restores permissions before assertions and verifies a final positive read.
An account that bypasses the restriction fails fixture setup rather than skipping. This executed
under Linux UID/GID 1000 after the current-stable API migration. The complete Linux rerun passed
**44/44 runtime IO, 62/62 repository IO, and 55/55 facade tests**, without skips, plus strict IO
all-target/all-feature Clippy and all 14 locked native dependency profiles. The current
`h-linux-native-*.log` files contain that newer run; `h-linux-native-environment.log` records the
actual user, kernel, and Rust 1.95.0 identity. The permission fixture still needs Apple execution.

The later Windows case fixture verifies real lower/upper-case host aliases and a successful
original-spelling capability read before checking its authorized-content-or-open-error oracle.
It passed in **41/41 runtime IO and 51/51 repository IO tests**, without skips, with current-stable
strict IO Clippy. Logs are `h-windows-native-case-runtime.log`,
`h-windows-native-case-repository.log`, and `h-windows-native-case-clippy.log`. Read-only 8.3 volume
queries on C: and F: returned access denied; no volume configuration was changed or 8.3 behavior
inferred from that environment result.

A later non-privileged file query returned the real F: short name `LONGAU~1.BIN` for a newly
written long-name fixture. A small temporary native consumer linked against the existing Rust
1.95 candidate IO library then verified both host spellings, successful original-name capability
reads, and exact `ShortNameAlias` rejection before adapter IO. No 8.3 volume setting was changed.
This qualifies the exercised local volume case only. The fixture, temporary Rust source, alias
metadata, and successful build/run logs are retained as `target/review/windows-alias-probe/`,
`native_alias_probe.rs`, `h-windows-8dot3-alias-probe.json`, `h-windows-8dot3-build.log`, and
`h-windows-8dot3-runtime.log`. The portable path firewall remains the maintained automated check;
this probe is not a new hosted CI result or universal volume claim.

## MSRV and Current-Stable Integration (2026-10-31)

Current stable is `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, LLVM 23.1.1, on Windows x86_64.
Its first full run passed 2,444 tests, but strict Clippy then exposed deprecated atomic APIs and
new constant-chunk lints. Six checked atomic issuers now use `try_update`, stable since the
existing Rust 1.95 MSRV, with unchanged closures, return mapping, and memory ordering. The
maintained glTF reader uses equivalent integer `MAX` constants. Constant-sized animation/morph,
hex, example, and test traversals borrow array chunks; dynamic attribute widths keep their
existing slice traversal and validation. No compatibility wrapper or lint suppression is added.

After these changes, both Rust **1.95.0 and 1.99.1 passed 2,444/2,444 all-feature workspace
tests**, with four manual pilots skipped. Strict all-target/all-feature Clippy passed on both
toolchains. Current-stable workspace doctests and rustdoc with `-D warnings` passed. Read-only
review found no changed closure/order, tuple/cardinality, control-unit, remainder, or borrowing
semantics in this migration. The package README also describes the already delivered material
length and borrowed application-tuple workflow.

Logs are `target/review/h-stable-toolchain.log`, `h-msrv-workspace.log`,
`h-stable-workspace.log`, `h-msrv-clippy.log`, `h-stable-clippy.log`,
`h-stable-doctests.log`, and `h-stable-rustdoc.log`. The earlier warning-producing source run is
retained as `h-stable-workspace-before-api-rename.log` and
`h-stable-clippy-before-api-rename.log`; it is not the final lint result. Final packaged-consumer
and target execution evidence remains separate below.

## Complete Packaged-Consumer Matrix (2026-10-31)

The maintained consumer built and executed all twelve fixed cases from freshly unpacked archives
on the ordinary Windows main stack under both required toolchain/profile combinations:

| Toolchain / profile | Retained artifacts | Result |
| --- | --- | --- |
| Rust 1.95.0 / debug | `target/package-check-rxbi7xjg/` | 12/12 built and executed |
| Rust 1.99.1 / release | `target/package-check-psw5mj6c/` | 12/12 built and executed |

Cases are empty, default, default-formats, OBJ, PLY, STL, glTF, postprocess, all-formats,
tracing, mint, and all. Each run creates all twelve production package archives, validates their
normalized identity/README/licenses, and resolves local dependencies only from extracted contents.
Fresh artifact build directories prevent checkout build products from satisfying these checks.
The unchanged maintained consumer lockfile and `--locked` apply to resolution, build, and execution.
There is no unlocked retry, generated Rust consumer, or checkout-source substitution.

These cases execute typed geometry/appearance/hierarchy, custom IO/plugin behavior, selected glTF
deformation/animation, and the complete postprocess/application-buffer workflow where enabled.
The stable archives additionally contain the subsequently added Unix-only permission test. A
bounded comparison of 553 packaged Rust files found that test's `fs.rs` to be the sole source
difference between the two runs; production paths are unchanged. This local artifact simulation
uses the documented dirty/no-verify archive phase and does not establish exact-tag registry
packaging, package ownership, authentication, publication, or target promotion.

Logs are `target/review/h-package-msrv-debug-final.log`,
`h-package-stable-release-final.log`, and `h-package-source-comparison.log`.

## Executed Target, Example, and Dependency Gates (2026-10-31)

The final production paths passed the four public examples (`import_bytes`,
`import_memory_sidecars`, `diagnostics`, and `wasm_memory`) under Rust 1.99.1 with all features.
The diagnostics example exercises its expected format-failure path and exits successfully; it is
not an unexpected parser failure. Both the root and fuzz workspace dependency checks passed their
advisory, ban, license, and source policies. Existing duplicate-version/wildcard warnings remain
visible. The checked lockfiles did not change.

The independent no-feature and all-feature release consumers executed under
`wasmtime 50.0.0 (ebe5e8df1 2026-10-20)` on `wasm32-wasip1`, using Rust 1.95.0. Their typed semantic
assertions and the enabled application-buffer workflow passed. The corresponding
`wasm32-unknown-unknown` consumers passed compilation only; no browser execution is claimed.

CI now contains MSRV/debug and current-stable/release Windows package matrices, an executable
WASI empty/all matrix under pinned Wasmtime 50.0.0, strict current-stable workspace
lint/tests/doctests, and always-retained PR fuzz corpus/logs. Actionlint 1.7.12 passed locally.
No GitHub workflow was dispatched and no branch was pushed as part of this qualification, so these
are local executed results plus verified workflow wiring, not a claim of hosted CI results.

Logs are `target/review/h-deny-workspace.log`, `h-deny-fuzz.log`, `h-example-*.log`,
`h-wasmtime-version.log`, `h-wasi-release-final.log`, `h-browser-check-final.log`, and
`h-ci-actionlint.log`.

## Separate Linux Process-Memory Observation (2026-10-31)

The four existing manual storage pilots executed from the immutable `10495f31d1aa` archive under
Linux Rust 1.95.0, debug libtest, and one test thread. Each semantic/work/allocation assertion
passed. `/usr/bin/time` measured the individual test process after compilation, so compiler memory
is excluded. Requested allocator counts/bytes and ledger work/peaks remain in each Rust test log;
the following values are separately observed maximum resident set sizes:

| Complete test process | Maximum RSS (KiB) |
| --- | ---: |
| Independent indexed primitive buffers, 256/1,024/4,096 | 20,584 |
| Shared 8 MiB points, 1,000/10,000 node edits | 41,368 |
| 10,000/100,000 four-influence vertices and 1/8 MiB metadata | 48,768 |
| Shared encoded images, 1/8 MiB node edits | 34,056 |

The same executable's list-only startup measured 4,192 KiB. These process observations include the
test harness, input/output overlap, allocator behavior, and OS residency. They are advisory and
are neither per-size RSS measurements nor exact Scene storage. No baseline subtraction, comparison
with earlier Windows allocation counts, or exact-RSS inference from a ledger is made.

The existing Rust assertions and workloads are unchanged. A temporary local driver selects the
compiled test executable from Cargo JSON and wraps only those four tests with GNU time; it adds
no maintained measurement framework. Source/toolchain scope, `report.json`, raw RSS values, and
allocation/work logs are retained in `target/review/d-final-rss-10495f31d1aa-p3gobih4/`.

## Unpublished Ownership and Actual Callback Destruction (2026-10-31)

The final ownership audit found four remaining blanket unwind guards in detached/attributed
retained parts, startup fallback owners, and unprepared publication assemblies. All four new
regressions failed against those guards: an unrelated host panic prevented the healthy payload's
destructor from running. Exact ownership, attribution, and account relationships are now checked
through the existing normal cleanup path during unwind. The thread panic flag is no longer used
as evidence of a physical ownership fault.

This required making a previously implicit activation boundary explicit. Seven concrete stages
own callback output, proof, or rejection cause together with their current store through a private
`PayloadOwner`. It adds no allocation, account handle, ledger, or public extension protocol.
Successful value destruction permits ordinary storage release. An actual destructor panic invokes
the existing concrete store containment method while keeping all uncertain charges. Nested owners
destroy proof and output through separate catches, so two destructor failures do not become an
accidental process abort. The original callback panic is resumed during normal drop; an already
active host panic is preserved instead. Consuming state transitions take payload and store together.
The freeze-issue containment branch also destroys both detached values through separate catches
before returning its original error.
Core generation-source panic handling is explicit at its invocation boundary while the frozen
permit remains owned. Its existing burn-the-reserved-ID and contain-the-operation regression is
preserved; the generic physical destructor no longer needs to infer that fault from thread state.
The corresponding old node-transform edit entry now uses the same explicit boundary. Its new
regression first failed on missing containment, then passed with the burned identity, original
panic, retained operation, and unchanged readable input checked together.

The integrated operation suite passed **721/721 tests**. Its original candidate/failure/freeze
destructor-fault and corrupt-graph tests remain intact; simply removing the guards would have
failed eight of them. The new public cases cover direct activated-output failure, proof/output
failure together, healthy candidate/freeze host unwind, and both callback destructors failing under
an outer panic. Healthy cases require zero physical/factory accounts without poison; fault cases
require both destructors, preserved original panic, and complete retained charges. The four
intermediate tests additionally observe storage destruction while its physical and attribution
charges are still live. Current-stable strict operation Clippy passed.

Before/after evidence is in `target/review/a2-intermediate-before-all.log`,
`a2-intermediate-after.log`, `a2-intermediate-operation-all-before.log`,
`a2-payload-owner-operation-final.log`, `a2-payload-owner-clippy.log`,
`a2-edit-generation-before.log`, and `a2-edit-generation-after.log`.

Final Rust **1.95.0 and 1.99.1 workspace suites each passed 2,454/2,454**, with the four manual
pilots skipped in their ordinary runs. A separate current-stable storage selection passed all
**6/6** cases, including those four pilots and both ordinary controls. Strict workspace
all-target/all-feature Clippy, doctests, and rustdoc with `-D warnings` passed. The image pilot
retains the same 8,444-byte edit allocation peak and 226 consumed work units for 1/8 MiB inputs;
distinct-buffer growth, source-first sharing, influence/metadata behavior, and normal release
assertions passed. Pilot timing remains advisory (the metadata case was reported slow).

Logs are `target/review/a2-owner-workspace-msrv.log`, `a2-owner-workspace-stable.log`,
`a2-owner-core-pilots.log`, `a2-owner-clippy.log`, `a2-owner-doctests.log`, and
`a2-owner-rustdoc.log`. Package and sanitizer verification for this newer owner boundary remains
separate from the preceding archive evidence.

## Packaged and Sanitized Ownership Boundary (2026-10-31)

The committed `59cf026f9fc9` ownership implementation completed both fresh extracted-artifact
consumer matrices on the ordinary Windows main stack: **12/12 under Rust 1.95.0 debug** and
**12/12 under Rust 1.99.1 release**. Each matrix builds and executes the maintained public consumer
against all twelve extracted production packages. A bounded comparison found all **555 Rust
files** in both staging trees equal to the committed source. No checkout dependency or unlocked
retry substitutes for those artifacts. The operation and IO leaf packages additionally passed
ordinary `cargo +1.95.0 package --locked` verification without dirty/no-verify flags.

The empty/all release consumers also executed under Wasmtime 50.0.0 on `wasm32-wasip1`; their
handwritten semantic assertions passed. The corresponding browser-WASM empty/all consumers
compiled successfully. Browser execution is not claimed.

Five Linux ASan targets then executed **256 runs each** from an immutable Git archive of the
same commit: STL, OBJ, PLY, selected glTF, and registry detection. The locked fuzz metadata and
execution used the existing checksum-qualified offline dependency cache. All five binaries
contain `__asan_init` and `__asan_report_load1`; the fuzz lockfile digest remained unchanged.
Per-input timeout, memory cap, campaign cap, seeds, logs, resulting corpus, and artifacts follow
the bounded settings recorded above. This is sanitizer smoke for the ownership boundary, not
sustained fuzz coverage or native-filesystem qualification. Native candidate IO is not enabled
in these portable parser targets.

Artifact directories are `target/package-check-oh8pc0oc/` and
`target/package-check-_p23czn6/`. Logs are `target/review/a2-owner-packages-msrv-debug.log`,
`a2-owner-packages-stable-release.log`, `a2-owner-package-source-comparison.log`,
`a2-owner-package-operation-verify.log`, `a2-owner-package-io-verify.log`,
`a2-owner-wasi.log`, and `a2-owner-browser.log`. Sanitizer evidence is retained in
`target/review/h-linux-asan-59cf026f9fc9-xjy_l8lk/`, with orchestration in
`target/review/a2-owner-linux-asan.log`. The subsequently discovered Windows reparse defect and
its candidate-only fix have their own behavioral qualification.

The actual Ubuntu 26.04 x86_64 host then passed **2,465/2,465 all-feature workspace tests** under
Rust 1.95.0, with four manual pilots skipped in that ordinary invocation. Strict workspace
all-target/all-feature Clippy and workspace doctests also passed. The workspace test binaries
were built before the separate unique-vertex target was added; that new workload and the latest
native predicate receive their own Linux follow-up below. The newer predicate preserves Unix
symlink semantics. These Linux commands used the existing target directory and locked offline
dependencies; no networking or system configuration change was made. Logs are
`a2-owner-linux-workspace.log`, `a2-owner-linux-clippy.log`, and `a2-owner-linux-doctests.log`.

## Actual Windows Reparse Rejection and Rename Controls (2026-10-31)

An actual non-Microsoft reparse fixture exposed a runtime defect beyond the lexical path firewall.
For file tag `0x42`, the capability backend returned a handle whose metadata reported a regular
file, even though ordinary host reading rejected the object with Windows error 1920. A temporary
native consumer first failed its required rejection assertion. Runtime IO now checks the returned
handle's reparse attribute before accepting its ordinary file type or constructing an `OpenedAsset`.
The same crate-private predicate replaces repository capture's duplicate check. No pathname
preflight/reopen, unsafe Rust, new dependency, or alternate resolver is introduced.

The maintained regression uses Python/Win32 only to create and read back the actual tag/GUID
fixtures. Both root and nested file objects require exact `InspectObject` /
`NonRegularFile(SymlinkOrReparsePoint)` errors. A directory object is rejected; an ordinary file
inside the nonredirecting directory tag `0x10000042` must return its complete authorized bytes or
an open error. The direct ordinary file remains a positive control. This preserves the runtime
contract's in-root link semantics; repository capture has the separately stricter no-reparse
component contract. Test fixture failures cannot skip the gate, and the Windows candidate CI job
now explicitly installs its Python prerequisite.

The integrated rerun also exposed a false error-code restriction in the earlier replacement
fixture: a directory containing the reader's still-open child can reject rename with error 5,
while holding the directory itself produces error 32. A controlled local `MoveFileExW` fixture
confirmed both observations and immediate success after the corresponding handle was closed.
The race now accepts only those two exact codes while a read is in flight; the same exchange
must still succeed immediately after release. A deterministic held-child control requires one
of those exact denial codes, complete original bytes, then successful rename and restoration.
There is no blanket permission-error acceptance or retry loop. The failing round-12 log is
retained separately.

Rust 1.95.0 passed **42/42 runtime IO and 52/52 repository-candidate tests**. Rust 1.99.1 passed
**52/52 all-feature IO tests**. All executed without skips, including the new actual-tag and
held-child controls. Read-only correctness review found no predicate or storage/handle-lifetime
regression. Actual helper creation succeeded on the workspace F: volume and system-temp C:
volume; maintained tests left no `baozi-io-unknown-reparse-*` directory behind. These observations
do not qualify every filter driver or volume mount.

The separate real-volume mount attempt failed in fixture setup with `ERROR_ACCESS_DENIED` (5);
no mount was created. Apple ARM64 execution is still outstanding. Release registration remains
empty and Windows repository capture retains its typed pre-traversal rejection.

Logs are `target/review/h-windows-unknown-reparse-runtime.log` (before),
`h-windows-unknown-reparse-after.log`, `a2-final-windows-directory-rename-before.log`,
`h-windows-rename-denial.json`, `a2-final-windows-msrv-runtime.log`,
`a2-final-windows-msrv-repository.log`, and `a2-final-windows-stable-repository.log`.
Fixture reports include `h-windows-unknown-reparse-fixture.json`,
`h-windows-unknown-reparse-fixture-directory.json`, and `h-windows-volume-fixture.json`.

## Unique Vertices and Final Growth Coverage (2026-10-31)

The older OBJ scale fixture repeats faces over only three vertices. The new independent
`unique_vertex_growth_contract` complements it with 256/1,024/4,096 triangles and three distinct,
nonreused positions per triangle. Public typed reads verify all coordinates, all indices, exact
geometry counts, terminal counter values, and zero live bytes/reservations. Fixed-width OBJ
records make input bytes grow exactly fourfold. Actual consumed pipeline work is bounded by
fourfold previous work plus the larger input's deterministic position-sort comparison bound;
the gate has no timing threshold or inferred RSS claim.

The Windows Rust 1.95.0 run passed this new case together with the existing repeated-face scale
and one-MiB stack controls (**3/3**):

| Distinct vertices | Input bytes | Consumed work | Peak declared bytes |
| ---: | ---: | ---: | ---: |
| 768 | 14,336 | 41,907 | 251,110 |
| 3,072 | 57,344 | 178,739 | 945,382 |
| 12,288 | 229,376 | 763,059 | 3,722,470 |

The maintained thin Python scale driver now runs both Rust contract targets. Fixture generation,
semantic expectations, and the work bound remain in Rust; the driver adds only the fixed test
target argument. Syntax/CLI checks and Actionlint passed locally. Distinct primitive buffers,
node depth/width, and repeated resource sharing retain their separate existing Rust growth
checks. The log is `target/review/a2-final-windows-msrv-growth-stack.log`.

Current-stable Clippy then required the new test's fixed-width position traversal to borrow
`as_chunks::<3>()` arrays. The MSRV already supports this API. Both Rust 1.95.0 and 1.99.1 reran
the affected test successfully with identical semantic/work/allocation observations. The actual
Linux MSRV run used only `--no-default-features --features format-obj`, matching the scale driver's
feature boundary, and passed the unique-vertex, repeated-face, and one-MiB stack cases (**3/3**).
Linux consumed exactly the same work totals; its declared peaks were 250,624 / 944,896 / 3,721,984
bytes. These are ledger observations rather than RSS comparisons between hosts.

The final Windows current-stable all-feature workspace suite passed **2,456/2,456**, with the four
manual pilots skipped as before. After the fixed-array adjustment, both targeted toolchains and
strict all-target/all-feature workspace Clippy passed. The current-source MSRV all-feature IO suite
also passed **52/52**. All four public examples executed successfully again, and formatting,
Python syntax/CLI, Actionlint, and diff checks passed.

The committed `aa44f81` IO package additionally passed `cargo +1.95.0 package --locked -p baozi-io
--all-features --jobs 1` without dirty/no-verify flags. The archive contains the committed native
object check, repository predicate use, and Python fixture. The earlier complete portable
artifact matrices remain scoped to the ownership commit; this newer native-only repair has its
own executed candidate and verified-package evidence.

Logs are `a2-final-windows-msrv-growth-final.log`, `a2-final-windows-stable-growth-final.log`,
`a2-final-linux-growth-stack.log`, `a2-final-windows-stable-workspace.log`,
`a2-final-windows-msrv-io-final.log`, `a2-final-windows-stable-clippy.log`,
`a2-final-windows-integration-resumed.log`, `a2-final-windows-example-*.log`,
`h-owner-native-actionlint.log`, and `a2-final-package-io-all-features.log` under `target/review/`.
The initial lint failure is retained as `a2-final-windows-stable-clippy-before-typed-chunks.log`.

## Forced Linux Backend Fallback (2026-10-31)

The current-source Rust 1.95.0 all-feature IO libtest binary passed **62/62 without skips** in its
ordinary environment and in each of three forced backend states: `openat2` returns `ENOSYS` (38),
`EPERM` (1), or persistent `EAGAIN` (11). Each isolated child first installs a process-local seccomp
filter under `NO_NEW_PRIVS`, verifies the actual syscall result, and then executes that same binary
with one test thread. The fixture's filter allows every other syscall; no parent filter, machine
configuration, networking change, or root privilege is involved. The native tests include authorized reads,
escaping/magic links, replacement races, permission failures, special objects, and operational
repository capture. All three child processes returned zero.

This is actual fallback behavior under backend rejection. It does not count the internal retry
attempts, measure a backend deadline, or establish every platform's retry/work bound. The fixture
is a temporary local qualification helper, not another maintained resolver or mutation framework.
Cargo JSON supplies the actual built executable; no artifact filename is guessed.

Evidence is `target/review/a2-native-fallback-build.jsonl`, `a2-native-fallback-build.log`,
`a2-native-fallback-baseline.log`, `a2-native-fallback-enosys.log`,
`a2-native-fallback-eperm.log`, `a2-native-fallback-eagain.log`, and
`a2-final-linux-followup.log`. These complement rather than replace the native candidate matrix.

## Native Package and Core Integration Cutover (2026-10-31)

A further requirement audit found two explicitly selected source cutovers missing from the prior
acceptance summary: standalone native IO development and the actual core/import integration seam.
They are implemented together with their dependency/compiler boundaries, rather than treating
previously passing portable behavior as proof that those units had been completed.

The unpublished `baozi-native-io-candidate` now owns its own excluded workspace and lockfile,
runtime adapter, optional `repository-capture`, status constants, and all former filesystem and
repository fixtures. Its only production Baozi edge is portable `baozi-io`. Facade native features,
path helpers, error variants, status exports, and IO feature aliases are deleted. Native integration
tests construct a capability and pass the adapter to the same public `read_asset` seam as other IO;
they read typed triangle data and exact nested-sidecar image bytes. A deliberately different
primary-relative decoy cannot satisfy the expected image. The maintained public consumer forwards
the facade's entire real feature set, so its `all` case has no native exception.

The candidate's four target runtime/repository graphs and root Linux/WASM/WASI graphs use eleven
locked metadata queries. Native package versions, sources, checksums, runtime/audit target sets,
and the Windows reviewed share-mode basis are unchanged. Feature isolation removes only the
workspace-inherited Unix `libc/default` alias and Windows Console/SystemInformation flags; the
new full closure adds the explicit local IO dependency. Runtime schema 4 and repository-delta
schema 3 record the new manifest/lockfile ownership. The root/fuzz/consumer locks are reconciled
offline without upgrading registry packages. The candidate remains unregistered and its Windows
repository API continues to reject before traversal.

Core now exposes one unconditional hidden `__scene_integration` namespace. Before changing it,
an actual enabled packaged core library passed a positive naming compile and four independent
negative compiles for private draft/construction/proof fields and moved handoff ownership
(`E0616` / `E0382`, with missing-import errors forbidden). Those same guarantees are maintained
as default-core public API tests and compile-fail examples. The obsolete bridge feature/module
and the compile-only node-transform plan exports are deleted; plugin authoring keeps its existing
`baozi_import::authoring` interface.

Removing the blanket canonical-module dead-code allowance exposed unused production projections,
duplicate metadata/count accessors, retired collector-only observations, and a standalone test
execution rail. Unused values are deleted; the meaningful standalone fault harness is now
`cfg(test)`. Current production triangulation/normal/space transactions keep one path. RAII tokens
remain after their payload allocations, including zero-byte tokens. Deformation's skin-plan table
is still used for palette validation, then destroyed with uniqueness scratch before the live
reservation shrinks to the actual retained materialization plans. Stable identity semantics retain
their explicit sealed-authority restriction; no constructor becomes public merely to suppress a
warning. Read-only correctness review found no changed production capability, control-cause,
storage-release-order, or retained-reservation defect.

The Windows focused checks passed **558/558 core tests**, with four manual pilots skipped;
strict core Clippy covers production and tests without the blanket allowance. The moved candidate
passed **18/18 default/runtime** and **28/28 repository-enabled** tests, including four public
facade integration tests in each run, and strict all-target/all-feature Clippy. Those counts exclude
portable IO tests, which remain in the root workspace. All five installed negative targets fail
with the candidate's own compiler diagnostic, not a missing standard library. The initial migrated
xtask selection passed **56/56** and strict Clippy; the final suites also verify the updated snapshot
schema. No hosted CI execution or platform promotion is implied.

The integrated Windows workspace passed **2,419/2,419 tests on both Rust 1.95.0 and stable
1.99.1**, with four manual pilots skipped in each broad run. Strict stable all-target/all-feature
workspace Clippy, MSRV workspace doctests, default-core public API tests, and default-core
compile-fail doctests all pass. These checks exercise the actual standalone-candidate/core-bridge
cutover; they do not borrow qualification from an older packaged source archive.
The subsequent serial scale selection passed all six storage pilots and admission/cancellation
controls with no selected test skipped. It covers the changed scratch-release lifetime as well
as independent primitive buffers, hierarchy shape, and retained-owner sharing. Requested allocator
bytes and deterministic work remain the measured quantities; process RSS is not inferred.

Security review caught an independent dependency-policy gap: moving the package would remove its
native dependencies from both old `cargo deny` roots. CI now scans the complete candidate workspace
with `deny.toml` as well. That actual scan exposed the checksum-pinned `winx@0.36.4` package's single
`Apache-2.0 WITH LLVM-exception` license. After inspecting its full package license and manifest,
an exact package/version exception was added; no global license allow-list expansion or checksum
change was used. The candidate advisory/ban/license/source checks all pass. Existing duplicate
dependency warnings remain visible.

Evidence includes `target/review/h-bridge-before.log`, `h-bridge-public-api.log`,
`h-bridge-default-doctests.log`, `h-extraction-core-clippy.log`, `h-extraction-core.log`,
`h-extraction-candidate-runtime.log`, `h-extraction-candidate-repository.log`,
`h-extraction-candidate-clippy.log`, `h-native-extraction-negative-*.log`,
`h-native-extraction-xtask.log`, `h-native-extraction-xtask-clippy.log`, and
`h-native-extraction-deny.log`. The integrated checks add `h-extraction-workspace-msrv.log`,
`h-extraction-workspace-stable.log`, `h-extraction-workspace-clippy.log`,
`h-extraction-doctests.log`, `h-extraction-core-public.log`, and
`h-extraction-core-default-doc.log`, with manual-scale output in `h-extraction-pilots.log`.
Earlier rejected lint/license/source states are retained
separately.

## Bounded Asset-Open Admission (2026-10-31)

The native-work gap identified after extraction is now implemented across the public IO contract
and both import entry points. `AssetIo` requires a positive pure `open_work_bound`, with no
compatibility default. Memory backends cover path retention and bounded binary-search comparisons;
the mutable implementation remains valid after concurrent namespace growth. Custom wrappers
explicitly account for their own work. Import owns the observed-work permit and completes a
normally returned opaque call's declared weight, including returned errors; panic leaves attempted
work only. `AssetOpen` retains its separate successful-object meaning. Bound errors, cancellation,
and global-work denial cannot be softened into optional-input omission or replaced by later graph
accounting. Cache hits and direct bytes do not enter open planning.

The pinned registry implementation did not itself provide a practical finite manual-link envelope:
Unix link reads used allocation-growth retries, and Windows called opaque ambient-path helpers.
The standalone candidate therefore records an explicit local `cap-primitives` 4.0.3 patch, retaining
the complete upstream package and licenses. Fixed-buffer link reads and target limits precede the
existing resolver's component expansion. Windows validates native reparse bytes and uses its held
parent directly; Unix uses a single raw sentinel-buffer read. Existing platform expansion limits,
four Linux fast attempts, final handle inspection, nonblocking options, and root/parent share modes
remain. The checked envelope bills wrapper text/component/helper actions and includes complete
manual fallback; it does not claim exact syscall counts, kernel iteration limits, RSS or wall time.

Runtime schema 5 and repository-delta schema 4 admit only the actual local backend path/version
with distinct upstream provenance. All other registry identities stay unchanged. Actual offline
resolution initially selected a different Windows binding version; the patch now explicitly pins
the reviewed `windows-sys` 0.61.2, leaving only the two necessary binding features as closure changes.
The candidate lock adds the direct local edge and removes its former registry source/checksum;
the original archive checksum remains labeled as provenance, never as the patched directory's hash.

The new six-case primary/dependency contract selection passes, followed by the complete
**445/445 import/IO** suite with no skips and strict all-target/all-feature Clippy. Windows candidate
runtime **19/19**, repository-enabled **29/29**, and the actual vendored backend's **21/21** unit tests
all pass without skips. Native decode cases include malformed headers, native-unit limits, unpaired
surrogates, odd/out-of-range name spans, unsupported tags/flags, and absolute target containment.
Strict candidate Clippy and **35/35** native metadata tests pass, including actual repository
baseline comparison and adversarial local-path/provider substitution. These focused results do not
replace the remaining complete-workspace, fresh-package, or committed Linux qualification.
Independent read-only security review found no actionable defect in the changed low-level sources
or wrapper envelope. It inspected relative-parent access/share modes, native reparse bounds and
conversion, reachable userspace retry helpers, and the complete fast/manual allowance. The review
does not dynamically count every abstract action or supply an unexecuted platform result.
Independent read-only correctness review likewise found no actionable defect in the shared import
admission, primary/dependency cause handling, memory bounds or new contract tests.

The first complete workspace run exposed an old glTF calibration defect: the two runs used
different path lengths, which no longer have equal billed open work. They now use the same exact
path and bytes without relaxing the one-unit-short assertion. The related resource-closure test
was strengthened to include actual snapshot-storage setup in its calibrated pre-hash prefix and
to require complete primary bytes, one successful open, and the intended completed work prefix.
It can no longer pass merely because the new open cost causes an earlier failure. All **22/22**
JSON-admission/resource-closure tests pass after those repairs; rejected runs remain in separate logs.
The no-fail-fast full run then completed **2,437/2,438** and isolated the same path-prefix issue in
STL color-expansion cancellation. Its four observations now use one exact source path while
retaining the original facet-count, scan-chunk and completed-prefix assertions. No production
budget or parser behavior was weakened to satisfy either calibration.
The subsequent complete STL limits selection passed **11/11** with no skips, including both
color-expansion chunk boundaries. Its evidence is `h-open-work-stl-calibration-final.log`;
the glTF repair is `h-open-work-gltf-calibration-final.log`.

The bounded-open implementation and calibration repairs are committed as `faff556`. A fresh
Windows Rust 1.95.0 all-feature workspace run from that commit passed **2,438/2,438**, with four
manual core pilots skipped; those unchanged core fixtures have their six-case execution recorded
in the extraction increment. `h-open-work-workspace-msrv-committed.log` is the complete green
run, distinct from the earlier rejected calibration runs. Stable/package/Linux qualification
continues against this committed production source.
The matching stable **1.99.1** all-feature workspace run also passed **2,438/2,438** with the
same four manual core pilots skipped (`h-open-work-workspace-stable.log`). Neither toolchain
requires a compatibility implementation or an alternate IO-open path.
Strict stable workspace all-target/all-feature Clippy, MSRV workspace doctests, and all five
installed negative-target compiler checks pass. Root, fuzz and standalone-candidate advisory,
license, ban and source policy scans pass, followed by strict MSRV `baozi-io` packaging. Evidence
uses `h-open-work-workspace-clippy.log`, `h-open-work-doctests.log`,
`h-open-work-negative-targets.log`, `h-open-work-negative-*.log`, `h-open-work-deny-*.log`,
and `h-open-work-leaf-io.log`. Candidate dependency warnings remain visible; no target is promoted.
Fresh `.crate` consumers for the new interface passed all twelve fixed cases in both MSRV/debug
(`target/package-check-_5hv0wbd`) and stable/release (`target/package-check-us1wrjzf`). Every
production dependency was built from its verified unpacked package, and both maintained consumer
closures used their unchanged lockfiles. The complete custom-IO/plugin, typed-format and
application-buffer workflows execute successfully. Logs are `h-open-work-packages-msrv-debug.log`
and `h-open-work-packages-stable-release.log`. The later Windows-volume fixture is test-only and
is not part of these portable package claims.

Evidence uses `target/review/h-open-work-focused.log`, `h-open-work-import-io-final.log`,
`h-open-work-import-io-clippy.log`, `h-open-work-candidate-runtime.log`,
`h-open-work-candidate-repository.log`, `h-open-work-vendored-backend.log`,
`h-open-work-candidate-clippy.log`, `h-open-work-xtask.log`, and
`h-open-work-governance-final.log`. The exact syscall-trace fixture was first qualified against
the older binary: `ENOSYS`/`EPERM` each showed one `openat2`, persistent `EAGAIN` showed four, and
all reached successful manual fallback. That fixture qualification is historical until repeated
against the new committed backend.

The extraction's fresh artifact matrices both passed all twelve cases: Rust 1.95.0 debug in
`target/package-check-lyhpmks8` and stable release in `target/package-check-yvahp7g_`. They exercise
the source archived before the new bound API and do not qualify that later interface by implication.
All four examples, empty/all-feature executable WASI, and empty/all-feature browser-WASM checks
subsequently passed with the new IO/import seam in the working tree. The initial formatting check
then observed the still-unformatted new increment; formatting was applied before focused tests.

## Actual Windows Volume-Mount Containment (2026-10-31)

The earlier `SetVolumeMountPointW` attempt failed before creating any mount. A later direct
`FSCTL_SET_REPARSE_POINT` probe succeeded inside a new owned fixture directory. Raw native data
readback, `GetVolumeNameForVolumeMountPointW`, and known bytes reached through that mount verified
a real volume-GUID target. The probe cleared its exact reparse object and removed only its known
owned paths. This creates no machine-wide drive mapping or volume-policy change.

The maintained candidate fixture now exercises the same verified object under the actual adapter.
Ordinary Win32 access reaches a known root-external sentinel, including a non-ASCII filename;
capability-relative `AssetIo` rejects that logical escaping path and still reads an in-root positive
control. Explicit UTF-8 prevents host code-page dependence. Normal cleanup checks the exact reparse
clear result; unwind cleanup cannot issue a second panic. Read-only review identified those two
fixture concerns before execution and both were repaired. The default/runtime candidate passed
**20/20**, repository-enabled passed **30/30**, and strict candidate Clippy passed with no fixture
skip. Evidence is `target/review/h-windows-volume-reparse-probe.json`,
`h-windows-maintained-volume-fixture.json`, and `h-volume-candidate-{runtime,repository,clippy}.log`.
The new fixture is candidate-only; it does not change the portable production source qualified at
`faff556`, or qualify arbitrary filter drivers and other mount targets.

The final portable chain at `faff556` also completed all four examples, executable empty/all WASI,
empty/all browser-WASM checks, root/candidate/vendor formatting, actionlint and strict rustdoc.
Current logs use the `h-open-work-` prefix with their respective command names. Linux qualification
was deliberately deferred until this additional Windows fixture could be verified and committed;
no Linux execution is claimed by that deferral.

## Committed Linux Qualification (2026-10-31)

Linux qualification now consumes only `git archive` of `5de735e561a9a775c059ee2f7df848ac5c071d68`
under `target/review/h-open-work-linux-5de735e561a9-k1gcmnfp/source`. It uses the actual unprivileged
UID/GID 1000 host on Ubuntu 26.04/WSL2, Rust 1.95.0, offline locked dependencies, one build job,
two test threads, and the existing `fuzz/target`. The all-feature workspace passed
**2,438/2,438**, with the same four manual core pilots skipped. This is current committed-source
evidence for the extraction, hidden bridge and bounded-open changes; it does not reuse an earlier
source archive. Subsequent candidate/fallback/sanitizer results are recorded only after completion.
The archive retains `source-and-toolchain.log`, `source.tar`, and `workspace.log` in that directory.
Strict all-target/all-feature Linux Clippy and workspace doctests then passed. The standalone
native candidate passed **21/21 runtime** and **39/39 repository-enabled** tests without skips,
including actual Unix permissions, FIFO/device/link behavior and public facade sidecars. Those
results are `clippy.log`, `doctests.log`, `candidate-runtime.log` and `candidate-repository.log`.

The subsequent vendored backend unit-test compile exposed an upstream packaging omission:
the registry archive includes a Unix test calling `cap_tempfile`, but its published manifest
omits that path-only dev dependency. The repair retains all five original link-content cases and
uses a uniquely created standard-library temporary directory; it changes only `#[test]` code and
adds no dependency. Its Linux selection passed **18/18** without skips
(`target/review/h-linux-backend-test-fixture-repair.log`). Prior complete workspace/native results
still describe unchanged production source; the remaining backend/fallback/ASan stages use a new
immutable commit archive after the test-only repair. The rejected original compile remains in the
first archive's `candidate-backend.log` rather than being erased or reported as a passing stage.

The follow-up immutable archive is `bdd7c800c463` under
`target/review/h-open-work-linux-bdd7c800c463-ddigqztn`. Its backend unit tests passed **18/18**
and strict candidate Clippy passed. One repository-enabled candidate libtest executable passed
**35/35** in the ordinary environment and again under each child-local `openat2` rejection state:
`ENOSYS`, `EPERM` and persistent `EAGAIN`. No fixture skips or parent/machine syscall-policy changes
were used. Logs are `candidate-backend.log`, `candidate-clippy.log`, `candidate-baseline.log` and
`candidate-fallback-{enosys,eperm,eagain}.log` in that archive.

Actual `strace` observation now closes the formerly unmeasured retry count for this binary.
`ENOSYS` and `EPERM` each produce exactly **one** `mesh.stl` `openat2` attempt; persistent `EAGAIN`
produces exactly **four**. Each reaches **one** successful manual `openat` of the fixture, and the
same-handle byte assertion passes. The fixture independently verifies the forced errno before
executing the test. `exact-retries-*.log` and `exact-retries-*.strace` retain the full evidence.
The locally extracted strace 6.19 archive was checked against the registered Ubuntu package SHA-256;
it was not installed system-wide. These observations qualify the tested fallback/retry branches,
not kernel duration or every abstract action in the conservative work envelope. The pre-open
budget regression and bounded backend implementation supply the independent work-admission checks.

All five Linux ASan smoke targets then completed **256 actual runs each**: `stl_import`,
`obj_import`, `ply_import`, `gltf_import` and `registry_detection`. They use the same immutable
`bdd7c800c463` source, installed nightly 1.101.0 (`92ffebece`, 2026-10-28), cargo-fuzz 0.13.2,
single-job builds and committed corpus seeds. Input deadlines are 10 seconds, campaign ceilings
120 seconds, RSS ceilings 4096 MiB, and maximum lengths 65536 bytes (4106 for registry detection).
The logs' `Done 256 runs` lines were checked rather than inferred from successful builds. All five
binaries expose both `__asan_init` and `__asan_report_load1`; `instrumentation.log` records those
checks and binary sizes. `fuzz/Cargo.lock` remains byte-identical. Logs, corpora and artifact
directories remain in the follow-up archive; completion is 2026-10-31T09:12:11Z.

These are bounded sanitizer smoke runs, not sustained fuzz coverage or a Windows-native FFI
sanitizer claim. Nightly's redundant-homepage manifest warnings remain visible. Current Windows
dual-toolchain workspace, packaged consumer and WASI results, committed Linux workspace/native
results, exact fallback traces, and current immutable ASan evidence now cover the delivered
cutovers. Apple ARM64 runtime/permission execution remains the concrete outstanding native
platform item; candidate registration is unchanged and the A–H goal is not marked complete.

## Apple ARM64 Acceptance Wiring (2026-10-31)

A further completion audit found that Apple CI exercised the native candidate but did not run
the standalone packaged semantic consumer. The existing `package-consumers` job now has four
explicit native-host cases: Windows x86_64 and Apple ARM64 each execute all twelve cases in
MSRV/debug and current-stable/release. Each job records the compiler identity and rejects a
host/target mismatch before packaging. The Apple native candidate job additionally executes the
portable all-feature workspace tests and doctests on its verified ARM64 host, with one Cargo build
job and two workspace test threads. Existing read-only PR/main triggers remain unchanged.

All three workflow files pass local actionlint. The new host-check script was executed locally:
it accepts the actual Windows host and rejects the Apple triple on that host. Independent read-only
configuration review found no actionable defect. These are configuration and host-check results,
not Apple execution evidence; no broad Rust rerun is inferred or required for the workflow-only
change.

Read-only GitHub API inspection found only old-main results in the five most recent runs, all at
`2743f0028c37e5790538eb319997e11ab3f3ea37`, and no remote
`refactor/baozi-canonical-architecture` branch. Those results cannot qualify current local source.
Running the updated Apple jobs requires publishing the current branch and opening a PR (or a
separately authorized real-host runner). No branch push, PR creation or hosted workflow execution
is claimed by this wiring change. Actual Apple runtime/permission and artifact results remain open.

Before remote execution, a Windows-hosted Rust 1.95.0 cross-check of `e1dd050` installed the actual
`aarch64-apple-darwin` standard-library component and checked the workspace library/binary targets
with all features. The standalone native candidate also passed all-feature/all-target checks and
strict Clippy for that Apple target, covering the Darwin production branches and candidate test
compilation. All twelve standalone consumer feature cases passed target checks outside workspace
feature unification. These operations reuse `target`, use one Cargo job and `--locked`, and leave
all maintained lockfiles unchanged. No Rust source change was required.

The broader workspace `--all-targets` cross-check reached an external toolchain limitation:
dev dependency `alloca` needs a Darwin C compiler, while the available MinGW `cc` rejects `-arch`
and `-mmacosx-version-min=11.0`. That rejected run remains `target/review/h-apple-workspace-check.log`.
The positive results are `h-apple-production-check.log`, `h-apple-candidate-check.log`,
`h-apple-candidate-clippy.log` and `h-apple-independent-consumers-check.log`. They prove compilation
only: no Apple binary was linked or executed, no permission/race fixture ran, and no cross-check is
counted as native Apple qualification. Publishing the prepared branch/PR or providing a real host
is still required to complete the pending runtime jobs.

## A–H Acceptance Audit (2026-10-31)

Earlier entries describe their own increments and source revisions. This audit replaces the stale
list of unfinished typed-domain and postprocess migrations; it does not turn historical execution
into qualification of later changes. The active remaining work is behavioral acceptance and native
candidate qualification, rather than another Scene representation or a speculative release system.

| Unit | Delivered implementation | Acceptance and remaining scope |
| --- | --- | --- |
| A | Startup-admitted session state; normal-stack import; static proof-scheme identities; healthy published/unpublished cleanup; explicit callback-destructor containment; repaired examples. | Both Windows toolchains, ordinary-stack extracted consumers, the full Linux suite, current-source one-MiB controls, and all four executable examples pass. Real poison/destructor failures retain their distinct containment behavior. |
| B | OBJ partial-channel/color loss decisions and homogeneous positions; exact PLY F64 values; STL names; single snapshot hashing; retained manifest/evidence; bounded short reads; complete cache/graph work and original-cause propagation. | Existing semantic, budget, cancellation, growth, and lifetime regressions run in the integrated suites. No separate fidelity/storage implementation remains from this unit. |
| C | Borrowed typed readers, checked IDs/counts, simple static-plugin construction with default provenance, bounded semantic errors and dependency context, and one independent public consumer. | Fresh extracted consumers execute the supported workflows without snapshots or private ownership/account APIs. No second public Scene or compatibility facade remains. |
| D | All seven semantic domains use typed retained owners; obsolete record codecs/projections are removed; validation borrows actual storage; unchanged tables/payloads share owners; structural edits remap final owner provenance. | Corruption, interruption, source-first destruction, and sharing remain covered. The latest six-case pilot/control selection preserves measured allocation/work results. Earlier Linux process RSS is explicitly scoped to its immutable archive and remains advisory. No domain migration remains. |
| E | Maintained glTF/GLB backend, exact selected PBR/deformation/animation profile, normalized source ownership and explicit unsupported-profile rejection. | Typed artifact consumers and the newer immutable ownership ASan archive pass. The six historical backend reproducers remain committed corpus. This does not admit additional formats/extensions or claim sustained fuzz coverage. |
| F | Triangulation, Flat/Smooth normals, complete admitted target-space conversion, and borrowed triangle tuples into application-owned buffers. | The independent artifact/WASI workflow checks actual values and buffers after Scene destruction. Corner/Face semantics, seams, deformation, animation, camera/light axes, and material lengths retain focused regression coverage. Global tuple deduplication and removed undefined generated-coordinate sources are not unfinished requirements. |
| G | Compiler/behavior boundaries replace source-hash/AST governance; metadata checks stay locked; the scale driver is thin Python; Rust owns fixtures/assertions; fuzz logs/corpus are retained. Native development is an excluded workspace; the core integration uses actual privacy/linearity rather than a feature gate. | The deliberate quadratic growth regression was rejected and precisely removed. Distinct buffers, depth/width, resource sharing, and unique vertices have executed work checks. The newer core/candidate cutover has separate focused and integrated evidence; earlier package/ASan archives do not cover later source by implication. Driver timing/RSS stay advisory. |
| H | Updated dependencies/MSRV checks, package-local documentation/licenses, extracted-only twelve-case consumer matrix, executable WASI and current-stable CI wiring. | Both fresh ownership-boundary artifact matrices and verified leaf packages pass; the newer native repair also passes all-feature package verification. Browser-WASM is compile-only. Linux forced fallback passes all three backend-error states. No hosted CI dispatch, upload, or registry publication is claimed. Native candidates retain the concrete remaining platform matrix below. |

Native release registration remains empty. Actual Apple ARM64 runtime/permission execution remains
outstanding before promotion; the committed-source backend retry/work evidence above now passes.
The actual Windows volume-mount case above now passes. The accounting trace exposed a genuine
implementation gap at `71a81e0`; the bounded-open increment implements that seam and backend limit.
The completed package/Linux/Windows evidence above qualifies its changed source independently. The
dependency acquisition unit retains its distinct snapshot-acquisition meaning after open and does
not substitute for pre-open `WorkUnits` admission.
The original volume-mount fixture attempt returned access denied before creating a mount; its
later replacement supplies actual adapter acceptance for that exercised case. Unknown-tag payload
handling has a candidate-only repair, fixture, and separate regression gate. Windows repository capture continues
to return `UnsupportedIdentityBackend` before traversal; passing that test does not claim working
Windows capture.

The maintained [`scale-profile-policy.md`](../testing/scale-profile-policy.md) documents the Python
driver and Rust workload boundaries. [`behavior-gates.md`](../testing/behavior-gates.md) maps the
actual checks, and [native admission](../research/native-filesystem-bound-handle-admission.md)
records the platform distinction. Exact RSS, sustained fuzz coverage, full target promotion, and
registry publication readiness are not inferred from these local results. The delivery goal remains
active until its required remaining behavior has actually been qualified.

## Public Report Consumption and First Hosted CI (2026-10-31)

The completion audit identified one remaining C/H public boundary: report getters returned loss,
source-scope and resource-dimension values that an application could inspect through inference but
could not name through `baozi`. The facade now exports the ordinary report-reading vocabulary,
including the independent `ImportEvidence` owner, typed loss subjects/dispositions, diagnostics,
source selection/counts, text-profile policies and `ImportDimensionSnapshot`. The latter is also
exported from `baozi-import`; its private fields and read-only snapshot semantics are preserved.

The maintained independent consumer now requires a strict partial-UV OBJ rejection, explicitly
authorizes `SourceSemanticLoss`, and verifies the applied loss's exact code, source, subject and
disposition. Typed reads require preserved positions, indices and complete Corner normals with
no UV channel. After dropping the Scene, it reads the separately retained loss/diagnostic and
resource evidence. The glTF/GLB consumer additionally checks selected source scene 1 and five
selected/one omitted nodes. Its application-owned error boundary uses a boxed standard error;
the library's allocation-free terminal error transport is unchanged.

With explicit user authorization, branch `refactor/baozi-canonical-architecture` was pushed and
[draft PR #3](https://github.com/Latias94/baozi/pull/3) opened. The first hosted
[run 41196335613](https://github.com/Latias94/baozi/actions/runs/41196335613) records `b8ab176` results:
all four twelve-case Windows/Apple package matrices execute successfully, including actual Apple
ARM64 Rust 1.95.0/debug and 1.99.1/release. Linux native runtime/backend/repository pass 21/18/39
tests with no skips, and each of the five ASan jobs records `Done 256 runs`. Those results precede
the added public-report assertions above and do not qualify them by implication.

That run also exposed four concrete test/runner defects: a Darwin device-path spelling assertion,
a Windows temporary-root alias assumption, a ten-second timeout for a repeated-import cancellation
fixture, and an expected negative Cargo exit leaking out of a successful PowerShell assertion loop.
The repairs retain object-identity checks (including distinct `/dev/null` and `/dev/zero` controls),
exercise the real volume mount through both direct and junction paths, give only the existing
4,097-item cancellation case a thirty-second ceiling, and explicitly return success only after all
negative-target assertions pass. No test is ignored and no production containment policy changes.
The repaired Apple backend and complete hosted acceptance still require the next current-source
run; draft creation and the first run alone do not close that item or promote native support.

Local Rust 1.95.0 checks after these repairs execute independent `empty`, `obj` and `all` consumers,
and pass strict facade and standalone-consumer Clippy. The updated nextest configuration runs all
24 `node_graph` cases without skips; the large cancellation case completes in 4.421 seconds.
Windows native runtime/repository pass 20/30 without skips, including both actual mount spellings.
The corrected vendored backend's Apple library/tests cross-check passes; this remains compilation
evidence only. Six controlled PowerShell subprocess probes preserve success and each rejection
route, and actionlint, formatting and `git diff --check` pass. Actual hosted negative-target Cargo
checks and the remaining Apple execution still belong to the following run.

The second hosted [run 41197892544](https://github.com/Latias94/baozi/actions/runs/41197892544),
at `514861b`, confirms the repaired Windows candidate (runtime/backend/repository 20/21/30,
zero skipped), stable workspace 2,438/2,438 and the new twelve-case Apple MSRV consumer assertions.
The large cancellation case takes 10.570 seconds on that stable runner, demonstrating why its
previous ten-second ceiling rejected valid completed-prefix work. Apple runtime 20/20 and backend
18/18 now pass. Its repository suite reaches an additional fixture defect: the runner's long
temporary path plus `socket-root/input.socket` exceeds Darwin's `sun_path` before binding a socket.
That one test now creates an exclusive `0700` directory under `/tmp`; the complete ASCII socket
path is at most 87 bytes. Existing socket/type rejection and FIFO deadlines remain intact. Its
all-feature Apple test cross-check and formatting pass locally; actual completion still requires
the next native run. The final hosted result below must establish that execution independently.
