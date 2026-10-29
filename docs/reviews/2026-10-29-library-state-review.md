# Baozi Library State Review

Date: 2026-10-29 (Asia/Shanghai)

Scope: unpublished library; breaking API, dependency, and architecture changes are permitted.

Recommendation: use a new delivery plan, preserve the sound scene semantics, and simplify their storage
and publication implementation.

## Review basis and conclusion

The reviewed state is commit `14fe5b3` plus the 15 modified files present at review start, principally
glTF resource closure, animation, and canonical profile changes. The local Assimp reference is
`a2a586d9370a493651256ec5d03524710ce37a5f`. Assimp informed behavioral comparisons; its implementation
and fixtures were not copied. Five independent reviewers examined core, IO/import/operation, formats,
public workflows, and engineering/planning. The coordinating review checked representative findings
and performed the verification below. This is a sampled deep review, not exhaustive certification.

Baozi has useful domain modeling and substantial defensive engineering. Logical meshes, primitives,
instances, attribute domains, variable influences, authored transforms, separate appearance identities,
immutable publication, central validation, and resource policy deserve preservation. The format crate
boundaries and frozen importer registry are reasonable.

The library is not yet usable for normal model-consumption workflows. Public users cannot read
imported geometry. Postprocessing has no nonempty operation. The simplest Windows example exhausts
the default main-thread stack. Existing parser success paths discard data despite the loss policy.
Infrastructure adds its own costs: quadratic validation, duplicate projections, broad edit rebuilding,
and source-shape governance checks.

This calls for selective architectural change. Retain semantics and concrete security behavior;
reconsider generic retained-record machinery, mandatory authoring ceremony, and speculative evidence
systems according to actual consumers.

Rust text under `crates/` totals 367 files and 318,141 lines, including tests, examples, comments, and
blank lines. Core accounts for 109,958 lines and operation 101,519: about 66% together. The four format
crates total 34,590 lines. Xtask has 8,681 source plus 4,276 integration-test lines. `docs/` contains
166 files and 29,999 lines. These are scope indicators, not production-code counts or quality scores.

## Verification

Host: Windows 11, `x86_64-pc-windows-msvc`. Cargo compilation used `CARGO_BUILD_JOBS=1`, reusing the
workspace `target`; nextest execution used two test threads.

| Check | Result |
| --- | --- |
| Rust 1.99.0: `cargo nextest run --locked --workspace --all-features --test-threads 2 --status-level fail --final-status-level fail --no-fail-fast` | 1,904 run; 1,901 passed; 3 failed; 0 skipped. |
| Rust 1.95.0: focused IO/import semantic-identity tests, `--all-features --lib` | Both passed; the identity failure depends on build/toolchain behavior. |
| `cargo +1.95.0 run --locked -p baozi --example import_bytes` | `0xc00000fd`, `STATUS_STACK_OVERFLOW`, with default features. |
| Rust 1.99.0 all-feature debug `import_bytes` and `import_memory_sidecars` | Both stack overflow outside the test harness. |
| Same 1.99 libraries and exact example sources linked with an 8 MiB stack | Triangle import succeeds; sidecar example returns `NotFound`. |
| Exact README Rust quick start compiled against built library | `E0599`: no `meshes` method on `&ValidatedScene`. |
| Public-facade partial-channel OBJ and custom-double PLY probes, 8 MiB stack | Success with zero loss records while discarding source data. |
| Allocation-counter probe after warmup: normal report drop versus ordinary host unwind, 8 MiB stack | Normal drop: 0 net allocations/bytes; unwind: 64 allocations and 27,877 bytes remain, repeated twice. |
| Hint-length custom IO, same 32-byte OBJ and 33-byte backend limit, ordinary versus one-byte short reads | Ordinary read succeeds; legal short reads fail with `asset-bytes exceeds limit 33`. |
| OBJ homogeneous coordinates and STL multiword solid name | OBJ ignores `w` with zero loss records; STL rejects the multiword closing name. |
| Internal glTF importer registered in an isolated development consumer | `asset.version` 1.0, 2.0, and 3.0 all succeed; `%20` external-buffer URI fails when the decoded resource exists. |

The full-suite failures were:

- Import: `resource_graph::manifest::encoding::tests::registered_asset_io_semantic_vectors`,
  `InvalidCanonicalId`.
- IO: `semantic_identity::tests::same_authority_profile_issues_the_registered_stable_identity`,
  proof-scheme pointer identity assertion.
- Xtask: `format_and_import_production_sources_use_only_backend_io`, because native-fs dependency
  allowlists and closure baselines omit the existing `sha2` dependency.

The two identity tests were rerun with:

```text
cargo +1.95.0 nextest run --locked -p baozi-io -p baozi-import --all-features --lib -E 'test(registered_asset_io_semantic_vectors) | test(same_authority_profile_issues_the_registered_stable_identity)' --test-threads 1 --status-level fail --final-status-level fail --no-fail-fast
```

Linux sanitizer execution, release-profile performance, a full MSRV workspace run, and macOS/WASM/WASI
runtime execution were not performed. Complexity and memory estimates below are static analysis.
Ephemeral reproduction files are in ignored `target/review/`.

## Findings

P1 denotes a basic-workflow blocker or significant correctness/availability problem. P2 denotes a
substantial design, performance, or maintenance issue. Explicit future features are not presented as
existing algorithm defects.

### R1 — P1: Minimal Windows consumers exhaust the default stack

**Runtime reproduction.** The [triangle example](../../crates/baozi/examples/import_bytes.rs) aborts
on Rust 1.95 default features and Rust 1.99 all features. Its PE stack reserve is 1,048,576 bytes.
Relinking the same 1.99 library and source with `/STACK:8388608` succeeds. Hostile input recursion is
not required.

The path passes through [facade read_bytes](../../crates/baozi/src/lib.rs#L254),
[import_bytes](../../crates/baozi-import/src/session.rs#L140), and retained scene/outcome construction.
PE unwind/symbol inspection of a reviewed debug executable identified large individual frames:
`prepare_scene` about 405 KiB, `submit_scene` about 290 KiB, `finalize_success` about 248 KiB, and
`import_bytes` about 160 KiB. These are individual frame sizes, not a claim that all four are
simultaneously active. Public sizes in the 1.99 build are: `Error` 5,608, `ImportFailure` 5,600,
`ImportReport` 11,184, `ValidatedScene` 5,416 bytes. The existing per-type 64 KiB check does not test
return slots, by-value transition temporaries, and cumulative calls.

**Direction:** reduce cumulative frames and large state movement, with startup-admitted indirection
where needed. Test a normal-stack subprocess consumer. Raising every host application's stack is not
the library remedy.

### R2 — P1: Public scenes cannot be consumed; postprocessing is empty-only

**API inspection and compile reproduction.** [ValidatedScene](../../crates/baozi-core/src/canonical_scene/store/api/scene.rs#L14)
offers generation, block count, and snapshots. [Semantic views](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/semantic.rs#L82)
remain internal. Applications cannot read vertices, indices, hierarchy, material parameters, or
animation tracks. Snapshot previews are bounded diagnostic text, not a consumer API.

[PostProcessRequests](../../crates/baozi-postprocess/src/pipeline.rs#L28) permits only empty requests.
This is explicitly unfinished functionality. The README also names an absent `postprocess` example;
its quick start does not compile. The sidecar example references
`models/materials/textures/red.png` without inserting that asset and returns `NotFound` once the stack
failure is bypassed.

**Direction:** expose borrowed typed views/lookups and direct counts, then complete an independent
import-to-application-data consumer. Deliver real transformations in concrete increments.

### R3 — P1: Loss policy does not govern all successful data-discard paths

**Runtime reproduction and source inspection.** [OBJ partial attributes](../../crates/baozi-format-obj/src/mesh_builder.rs#L167)
emit warnings and discard complete UV/normal channels without loss authorization. The
[existing test](../../crates/baozi-format-obj/tests/geometry.rs#L210) accepts this under a strict helper.
One fully attributed face followed by an unattributed face succeeds with only position data, two
warnings, and **zero loss records**.

PLY allocates custom `double` properties as [F32 storage](../../crates/baozi-format-ply/src/parser.rs#L2205)
and uses [as f32](../../crates/baozi-format-ply/src/parser.rs#L2082), checking only finiteness. A custom
`measurement=16777217` becomes canonical `0x4b800000` (`16777216`), with no diagnostic or loss record.
Core custom attribute input only supports [F32/U32/I32](../../crates/baozi-core/src/canonical_scene/store/production/input/geometry.rs#L46).

**Direction:** use the existing loss-policy mechanism for all known discards. Preserve partial OBJ
data through representation or primitive splitting, or fail when loss is forbidden. Support F64
custom attributes and make precision lowering explicit.

### R4 — P1: Central geometry validation contains unmetered quadratic searches

**Static control-flow analysis.** Every primitive performs
[prefix searches](../../crates/baozi-core/src/canonical_scene/store/validation/node_scene/geometry.rs#L2290)
over earlier primitive scratch rows for polygon/index buffer summaries. Distinct buffers cause
`P(P-1)/2` comparisons. [Sparse attributes](../../crates/baozi-core/src/canonical_scene/store/validation/node_scene/geometry.rs#L1711)
use the same pattern. Each whole search spends only one validation unit, so actual comparisons and
cancellation latency diverge from the work budget. At 100,000 independent indexed primitives, the
index search alone approaches five billion comparisons; this is a static worst case.

**Direction:** use budgeted summaries keyed by complete buffer identity, with direct indexing where
possible. Validate near `O(entities + references + inspected payload cells)`. Test distinct-buffer
growth as well as shared geometry.

### R5 — P1: Registered semantic identity relies on wrapper addresses

**Failing 1.99 tests, passing 1.95 reruns, and production call sites.**
[Proof-scheme wrappers](../../crates/baozi-io/src/semantic_identity.rs#L918) are held in a referenced
slice and returned through const accessors. Consumers compare their addresses, including the
[test semantic-subject encoder](../../crates/baozi-import/src/resource_graph/manifest/encoding.rs#L567).
The tested 1.99 build disagrees about the canonical wrapper address; 1.95 passes the focused tests.
The observed `InvalidCanonicalId` is in a test-only encoder, so it does not establish that every
production import fails. The production [negative-proof encoder](../../crates/baozi-import/src/resource_graph/manifest/encoding.rs#L132)
also compares outer wrapper addresses and needs the same audit.

**Direction:** base identity on a sealed registered entry/identifier or one actual static allocation.
Audit wrapper-address comparisons and cover cross-crate immutable-memory manifests on MSRV and
current stable. A toolchain pin alone is not a correctness repair.

### R6 — P2: Record indirection and validation projections amplify costs

**Static allocation and call-path analysis.** A [node-transform edit](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/edit/graph.rs#L89)
rebuilds appearance, geometry, deformation, and animation tables through
[mapped record copies](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/edit/graph.rs#L1026).
Large payload leaves can share, but unrelated records are copied. [RecordCell](../../crates/baozi-operation/src/concrete_store.rs#L822)
mixes ordinary fields and physical references in a dynamic representation.

[Validation scratch](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/scratch.rs#L267)
duplicates influence offsets, joints, and source orders: approximately 36 MB for a million vertices
with four influences each, before other scratch. [Profile validation](../../crates/baozi-core/src/canonical_scene/store/validation/node_scene/profile.rs#L374)
also projects metadata/provenance payloads. Metering this memory does not remove its cost.

**Direction:** private typed tables, immutable contiguous payloads, typed logical references, and
generation qualification at public handles. Validate borrowed data using necessary marks/summaries.
Share unchanged tables/pages during edits; remove the replaced production representation.

### R7 — P1 delivery gap: glTF remains outside the usable facade

**Feature inspection and documented backend evidence.** [Facade features](../../crates/baozi/Cargo.toml#L14)
exclude glTF; [backend notes](../research/gltf-rs-backend-notes.md) record a panic/abort class and
check-only fuzz policy. Quarantine is appropriate. Current closure/animation work is useful but does
not close sanitizer admission. The wrapper already has about 10,000 source lines across its modules;
overlapping preflight/backend validation needs a clear endpoint.

**Direction:** timebox a maintained narrow backend fix/fork versus an owned bounded slice. Run the
known reproducer and malformed corpus under Linux sanitizer before choosing. Admit one exact profile
with readable geometry, appearance, hierarchy, skin, morph, and animation before broadening formats.

### R8 — P2: Plugin authoring ceremony is high and validation errors lose detail

**Contract and error-mapping inspection.** [Complete authoring batches](../../crates/baozi-core/src/canonical_scene/store/production/input.rs#L273)
require manual provenance coverage for every owner. Even [minimal importers](../../crates/baozi-import/tests/support/mod.rs#L160)
manually construct generated-operation lineage. Format producers repeat owner tables for shared
document sources. [Submission errors](../../crates/baozi-import/src/scope.rs#L637) are collapsed to
codes such as `canonical-scene-request-rejected`, making invalid fields hard to locate.

**Direction:** a small typed builder that supplies document-level provenance by default and permits
precise overrides. Preserve bounded domain/rule/object details. Keep activation and accounting private.

### R9 — P2: Some generic-model choices are current-producer constraints

**Representability analysis, mostly future-scope decisions.** Built-in material roles permit one
binding and `Replace`; [planning](../../crates/baozi-core/src/canonical_scene/store/production/plan/appearance.rs#L1211)
rejects namespaced behavior. [Animation input](../../crates/baozi-core/src/canonical_scene/store/production/input/animation.rs#L38)
uses one interpolation kind per channel and the planner rejects duplicate targets. Layered FBX
textures have a concrete Assimp importer path. The reference's per-key interpolation representation
also motivates a synthetic mixed-interpolation test; this review has not traced such a track to an
actual Assimp importer/fixture. Neither case justifies a universal material graph or animation
interpreter now.

Image authoring requires owned bytes, preventing complete material bindings with deferred external
image loading. Custom resolvers receive references only after portable filesystem-style
[normalization](../../crates/baozi-import/src/scope.rs#L2427), restricting asset-database URI use.
These are product/security tradeoffs to decide with consumers, not automatic vulnerabilities.

**Direction:** test a few non-glTF representability cases; decide image/reference policy explicitly.
Keep filesystem authorization bound to the opened object.

### R10 — P1 planning issue / P2 engineering issue: Active plans and gates retain obsolete shapes

**Inventory/source inspection.** The [old verification contract](../plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md#verification-contract)
still requires `u4c-metrics`, although its command/specs/jobs were
[retired](../conformance/gate-retirements.md#u4c-metrics-and-_u4c-collector). The roadmap schedules an
old pre-cutover sequence. Following it can resurrect removed migration infrastructure.

Native-fs checks [test source digests and AST shape](../../crates/baozi-xtask/src/native_fs/candidate.rs#L831),
including module placement, signatures, calls, and allowed macros. This restricts harmless refactoring
without independently proving behavior. Dependency-baseline drift caused the third actual suite failure.

The conformance catalog has 12 contracts and 13 gates: 10 `partial`, 2 `nonconforming`. The checker
rejects `conforming` until a future trusted receipt adapter exists. This is honest but has no current
completion consumer.

**Direction:** replace the execution plan; retain threat-model/domain decisions and a short behavior
coverage index. Delete source-shape proof systems and orphan collector features. Preserve actual
filesystem races, ownership, budget, and failure tests. Defer external receipts until a real consumer.

### R11 — P2: Evidence underrepresents consumers and sustained exploration

**Workflow/fixture inspection.** CI packages only operation/IO leaves. WASM/WASI are compile checks.
The README and every documented example are not executed, allowing basic workflow failures to survive.
Scheduled [fuzz campaigns](../../.github/workflows/fuzz.yml#L53) do not retain successful corpus growth.
Five targets have 19 checked-in parser seeds; registry detection has no checked-in corpus.

The 1,926-line [scale runner](../../scripts/run-scale-profile.ps1) measures one repeated-face OBJ case,
missing distinct buffers, unique vertices, nodes, and resource growth. Runtime-admission's nested
Cargo metadata call omits `--locked`.

**Direction:** independent public consumers, normal-stack execution, retained fuzz corpus, and a few
orthogonal scale cases. Use a thin cross-platform Python measurement runner and Rust assertions.
Keep sanitizer smoke, meaningful fault injection, compiler boundary checks, dependency audit, and
platform filesystem behavior tests.

### R12 — P1: Ordinary host unwind leaks healthy published retained data

**Runtime allocation reproduction and ownership-path analysis.**
[release_retained](../../crates/baozi-operation/src/retained/allocation.rs#L215) forgets the value
whenever `std::thread::panicking()` is true, even after the unique final owner is established.
[RetainedLeaseGuard::drop](../../crates/baozi-operation/src/retained/allocation.rs#L129) then forgets
the lease. The path applies to published canonical stores and outcome evidence, not only a partially
constructed or internally uncertain owner.

The public reproduction imports a triangle inside a `catch_unwind` closure, keeps the report in a
local variable, then triggers an unrelated host panic. After registry/panic warmup, allocation-counter
records zero remaining allocations for normal drop, versus 64 allocations and 27,877 bytes after
unwind. Repeating each case gives the same result. This measures leaked library/allocator bytes, not
exact process RSS, and does not attribute each measured allocation to a concrete retained block.
Static ownership analysis independently shows retained charges remain coupled to forgotten
leases; the public reproduction does not independently measure those account counters.

Existing injected destructor-panic tests exercise a different situation: the retained payload's own
destructor fails. They do not justify leaking healthy values merely because the host is unwinding.

**Direction:** release known healthy unique owners during ordinary unwind. Reserve containment and
poison for explicit internal ownership uncertainty or actual destructor failure. Test both cases.

### R13 — P1: Cache hits rehash whole dependency payloads without byte work charging

**Static call-path analysis.** [Existing resource targets](../../crates/baozi-import/src/resource_graph/mod.rs#L653)
recompute `ManifestDigest::of(bytes)` on every available reference. That is a full SHA-256 pass with
no byte-proportional charge or chunked cancellation inside the call. One 128 MiB dependency referenced
16,384 times implies approximately 2 TiB of extra hashing despite one physical read. This illustrates
the amplification; other limits may reject some concrete asset constructions before that endpoint.

**Direction:** compute a metered, chunked digest once when creating an immutable snapshot; reuse its
sealed identity/index/digest on cache hits. Rehashing immutable cached bytes adds no useful guarantee.

### R14 — P1: Resource-graph memory is not fully covered for its live duration

**Static allocation/lifetime analysis.** [plan_existing_target](../../crates/baozi-import/src/resource_graph/mod.rs#L1052)
uses ordinary dynamically allocated traversal vectors without covering storage reservations.
Identity/reference path clones also allocate string backing storage beyond inline graph-row charges.

More directly, [manifest sealing](../../crates/baozi-import/src/resource_graph/mod.rs#L876) releases
live reservations and stores the still-live manifest. The [release function](../../crates/baozi-import/src/resource_graph/manifest.rs#L669)
does not destroy the vectors. [Scene submission](../../crates/baozi-import/src/scope.rs#L632) seals
before Scene construction, letting construction reuse memory allowance while manifest allocations
remain. [TerminalVec conversion](../../crates/baozi-import/src/metered_storage.rs#L460) likewise drops
capacity reservations while retaining the data, with no equivalent physical owner transfer there.

Thus `max_live_bytes` does not cover all Baozi-owned allocations or all overlapping lifetimes.
This finding is not a claim of exact measured peak-memory excess.

**Direction:** use the existing metered storage for scratch and path ownership, or intern paths into
accounted storage. Keep leases with data until real deallocation or explicit transfer to an accounted
outcome-evidence owner. Moving a vector into another Rust struct does not end its memory lifetime.

### R15 — P2: Short reads exhaust requested-capacity admission before actual byte limits

**Runtime reproduction and control-flow analysis.** A valid 32-byte OBJ and a 33-byte backend limit
succeed with an ordinary hint-length reader but fail when the same reader legally returns one byte
per call. [Unknown/hint-length reads](../../crates/baozi-import/src/scope.rs#L4267)
permanently charge requested buffer length into `attempted_precharged`; unused admission from short
reads is not reusable. With a 4-byte limit and a valid reader returning one byte per call, the first
request asks for 3 and returns 1; the second asks for 1 and returns 1, then
[rejects](../../crates/baozi-import/src/scope.rs#L4319) after only 2 actual bytes. Unknown-length assets
exactly at the byte limit also lack a successful EOF probe. Native filesystem lengths use hints, so
this affects more than contrived custom readers.

**Direction:** distinguish actual bytes, read-call work, and reusable read admission. Specify EOF
probing at the limit. Cover legal short reads, exact limits, and over-limit streams separately.

### R16 — P2: Additional format paths need focused fidelity and interoperability checks

**Mixed evidence.** Homogeneous coordinates, STL names, glTF version acceptance, and escaped-URI
failure were runtime-reproduced. The glTF cases use deliberate registration of the internal importer
in a development consumer; they do not imply that the production facade currently exposes glTF.
Color composition, clip-name loss, and provenance complexity remain static findings.

| Path | Finding and concrete input | Direction |
| --- | --- | --- |
| [OBJ positions](../../crates/baozi-format-obj/src/obj.rs#L211) | Fixed-prefix parsing ignores homogeneous `w`, common vertex-color fields, and the third `vt` component. For `v 2 0 0 2`, output remains x=2 while homogeneous interpretation gives x=1. | Distinguish supported record shapes; preserve, explicitly lower with loss, or reject. |
| [ASCII STL endsolid](../../crates/baozi-format-stl/src/ascii.rs#L276) | `solid part one` accepts the multiword name; `endsolid part one` rejects excess tokens. | Use consistent remainder-of-line name handling. |
| [Binary STL colors](../../crates/baozi-format-stl/src/binary.rs#L181) and [material](../../crates/baozi-format-stl/src/parser.rs#L87) | Header color can populate fallback vertex colors and material factors together when any facet colors exist, duplicating tint in a multiplying consumer. | Jointly specify/test material and vertex-color composition; use neutral factors where appropriate. |
| [glTF admission](../../crates/baozi-format-gltf/src/parser.rs#L188) | JSON `asset.version`/`minVersion` pass as strings without a Baozi semantic compatibility check; GLB container version checks do not cover them. | Validate syntax and supported versions before backend work/IO. |
| [glTF resources](../../crates/baozi-format-gltf/src/parser.rs#L1460) | `triangle%20data.bin` is passed as a literal path instead of URI-decoded `triangle data.bin`. | Decode/validate URI references once, then apply path authorization. |
| [glTF clips](../../crates/baozi-format-gltf/src/parser/animation.rs#L1280) | Clip names are not read or attached, even though named animation fixtures succeed. | Preserve bounded names with the existing clip metadata owner. |
| [glTF provenance](../../crates/baozi-format-gltf/src/parser.rs#L1143) | New `contains`-based provenance deduplication can scan clip sources quadratically and then performs an unmetered sort. | Use a source-index mark table and ordered metered output. |

These should become small reproducers and semantic fixes, not separate architecture frameworks.

## Dependency assessment

Official crates.io APIs were checked on 2026-10-29. This is an upgrade candidate list, not a tested
dependency set; this review did not change requirements or lockfiles.

| Dependency | Current requirement | Latest stable | Treatment |
| --- | --- | --- | --- |
| Criterion | `0.5` | `0.8.2` | Focused benchmark upgrade. |
| SHA-2 | `0.10` | `0.11.1` | Require identical canonical digest vectors. |
| TOML (xtask) | `0.8.23` | `1.1.8+spec-1.1.0` | Reduce tooling first; intentionally check syntax changes. |
| Triomphe | `=0.1.16` | `0.1.17` | Recheck required fallible/final-owner operations if retained. |
| cap-std | `=4.0.2` | `4.0.3` | Review capability behavior with cap-fs-ext and target tests. |
| rustix | `=1.1.4` | `1.1.6` | Review native resolution and lock closure. |
| glTF | `1.4.1` | `1.4.1` | A version bump cannot close the backend issue. |

Sources: [Criterion](https://crates.io/api/v1/crates/criterion),
[SHA-2](https://crates.io/api/v1/crates/sha2), [TOML](https://crates.io/api/v1/crates/toml),
[Triomphe](https://crates.io/api/v1/crates/triomphe), [cap-std](https://crates.io/api/v1/crates/cap-std),
[rustix](https://crates.io/api/v1/crates/rustix), [glTF](https://crates.io/api/v1/crates/gltf).

## Decision and adoption

The user adopted the accompanying [development plan](../plans/2026-10-29-001-refactor-library-delivery-plan.md)
on 2026-10-29 and explicitly authorized necessary architecture refactoring, API replacement,
dependency updates, and removal of obsolete unpublished implementations. It is now the active
execution plan; the 2026-07-10 plan remains superseded history. This adoption records the approved
direction and does not change the observed test results or mark any finding resolved.

Start with normal-stack consumption, truthful loss handling, usable
typed access, and bounded validation; then simplify storage, complete glTF and concrete postprocessing,
and prove release behavior through the facade. Each abstraction should justify its cost through a
visible behavior, security boundary, or measured workload. Format, test, and ADR counts are not
completion criteria.
