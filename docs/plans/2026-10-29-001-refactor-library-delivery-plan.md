---
title: Baozi Usable Import Library and Core Simplification
type: refactor
date: 2026-10-29
status: active
---

# Baozi Usable Import Library and Core Simplification

## Decision and starting point

This is the active execution plan for the remaining unpublished-library work, based on
the [2026-10-29 review](../reviews/2026-10-29-library-state-review.md). The working-tree glTF/profile
changes reviewed there are part of the baseline. The user adopted this direction on 2026-10-29 and
explicitly authorized necessary architecture refactoring without preserving unpublished compatibility.
Plan adoption does not mark implementation complete, admit a format, or publish a package.

This plan replaces the [2026-07-10 execution plan](2026-07-10-001-refactor-baozi-canonical-architecture-plan.md),
which remains at its original path as superseded history. The [roadmap](../roadmap.md) follows this
sequence. Never restore retired collectors to satisfy obsolete plan commands. Historical checkboxes
and commands are evidence to inspect, not outstanding instructions to execute.

Breaking API changes, deleting obsolete code, changing crate boundaries, and dependency upgrades are
authorized. This includes replacing the generic record store and publication architecture when their
design causes the reviewed defects. A local patch is sufficient only when it resolves the underlying
cause; obsolete internal APIs and state machines do not have a compatibility requirement.

Each increment must remain buildable and delete the implementation and tests it replaces, retaining
their meaningful behavioral coverage. Use one production Scene representation after each completed
cutover. Temporary development branches may compare implementations, but no duplicate public model,
compatibility facade, or permanent migration adapter is retained.

Architecture decision changes are part of the corresponding implementation unit. Update or replace
the affected ADR, its ownership/index references, and any directly affected conformance record in the
same change as the new boundary. Existing ADRs record the current contract; they are not a reason to
preserve a rejected architecture or seek the same refactoring authorization again. Keep retained
guarantees explicit and never declare conformance from a documentation change alone.

## Product completion criteria

Independent consumers using public APIs must complete three workflows:

1. Import supported OBJ/PLY/STL and admitted glTF/GLB profiles; read positions, topology, indices,
   material bindings, authored transforms, hierarchy, and mesh instances. For the selected glTF
   animated profile, also read skins, morphs, and animation. Handwritten expected values establish
   semantics rather than merely asserting that a snapshot can be generated.
2. Import memory/custom-IO sidecars and a custom format plugin. Obtain actionable validation errors
   and inspect resource/loss reports without naming activation or physical-account internals.
3. Import a polygonal asset, triangulate it, generate requested normals, convert to an explicit
   target space, and construct application vertex/index data. Preserve Corner/Face attributes
   through transformations; authorize lossy lowering explicitly.

The normal platform stack must work in debug and release. Ordinary host unwind must release healthy
owned data. Work and memory evidence must cover the library's own retained and temporary data for
the lifetime claimed. Opaque decoder allowances remain allowances, not hard RSS guarantees.

## Architecture to retain and simplify

Retain logical mesh/primitive/instance separation; Vertex/Corner/Face domains; separate image,
sampler, texture, and binding identities; variable influences; authored matrix/TRS; f64 time;
immutable validated output; scoped IO; stable input snapshots; central validation; explicit loss;
and cooperative control. Preserve source identity separately from scene-local identity.

The selected target is a private typed `SceneStorage` containing typed tables and immutable contiguous payloads.
The scene root owns tables; records use typed logical indices. Public borrowed views hide storage
layout. Scene-generation qualification belongs at lookup/edit boundaries. Validators borrow the
actual tables, allocating only marks, traversal state, and necessary summaries. Edits share unchanged
tables or pages where the complexity is justified by measured use.

Core owns typed tables and payloads through a private owner that binds their lifetime to centrally
managed leases. The common release mechanism destroys storage before releasing its charge; neither
payload ownership nor its charge can be detached independently.

Operation code should own budgeting, cancellation, fallible storage admission, and lifetime-bound
leases. It should not become a generic schema engine for Scene semantics. Initialize heavy state
once and move small handles or borrow it. Keep any required no-allocation terminal path through
preallocated bounded storage; avoid moving the complete state through every typestate wrapper.

Keep crate boundaries initially. Merge or privatize operation components as their concrete use
shrinks; crate count is not an objective. A plain private typed table is preferable to a new generic
framework. Preserve proven behavior and tests during deletion, not the exact internal API.

### Architecture changes included in the plan

| Boundary | Target and required decision work | Preserved behavior |
| --- | --- | --- |
| Core storage / physical allocation | D1 replaces dynamic Scene records with core-owned typed tables. Rework ADR 0044's opaque-aggregate-only seam; use a replacement decision if the allocation/payload ownership boundary changes, with reciprocal index/reference updates. Operation retains the single admission and release mechanism; core does not implement a second allocator/account protocol. | Charge before allocation; bind charge to live storage; one charge per shared allocation; storage-first final release; no semantic identity from physical topology. |
| Import and final publication | A removes large by-value state transitions in favor of startup-admitted state and small handles/borrows. ADRs 0045/0046 do not require the current typestate layout. Revise any actual publication seam that must change with its implementation. | One session/finalizer; central validation before success; paired ownership/evidence; no partial public Scene; bounded failure reporting. |
| Validation and resource evidence | B/D validate borrowed typed data and attach leases to actual graph/manifest/evidence ownership. ADR 0047 governs accounting meaning, not projection copies or a prescribed terminal container. | Monotonic attempted/consumed work; truthful live/peak lifetimes; cancellable metered traversal; explicit loss. |
| Concrete postprocessing | F starts with one typed operation prepared against the input, using SceneEdit, remaps, and final validation. ADRs 0013/0032 must permit a single-operation implementation without first building a general dependency scheduler, replay framework, or plugin system. | Parameter/loss preflight; unchanged input on failure; correct domain remaps and invalidation; deterministic outcomes. |

Public read views and automatic default provenance can be implemented under the existing Scene
ownership rules. Do not create a new architecture decision for ordinary accessors or builder helpers.
Likewise, neither generic records nor an allocation-heavy typestate chain is a security invariant.

The [API ergonomics review](../reviews/2026-10-29-api-ergonomics-review.md) informs C's concrete
consumer and producer boundaries. The [feature/module review](../reviews/2026-10-29-feature-design-review.md)
extends G/H: remove empty `std` and unused tracing switches, keep independently selectable format
dependencies, and distinguish compiled-in built-ins from Cargo's curated default set. Private
modules should own one responsibility; facade errors, importer workflows, typed readers, acquisition,
and storage accounting need not share one file. Splitting a file must not widen constructor authority
or create domain-specific crates without a dependency or ownership reason.

Implement the portable feature cleanup and facade module split first. Treat native IO candidate
extraction and the hidden cross-crate integration bridge as separate cutovers with behavioral and
target checks; review recommendations do not constitute their implementation. Use the checked-in
`tools/consumers/scene` package for both C and H, without generating or inspecting its Rust source
from xtask. Its expectations come from an explicit test case, outside workspace feature unification.

## Execution units

At adoption the implementation units below remain open; review reproductions establish the baseline
and do not count as fixes. The first implementation increment is **A1: normal-stack triangle import**.
Keep it focused on the shared cause in the import/construction/finalization call chain and its actual
public smoke test. Continue with A2 ordinary-unwind cleanup, A3 identity, and A4 example/baseline repair.
No new format or full-domain storage migration is required to close A1.

### A — Establish a usable and truthful baseline

Fix before model expansion:

- Default-stack Windows triangle import. Investigate cumulative frames and admit heavy session
  state once; do not solve the problem by requiring larger host stacks.
- Healthy retained data cleanup during host unwind, distinguishing it from an internal ownership
  failure or a payload destructor that itself panics.
- Proof-scheme identity across MSRV/current-stable builds; remove dependence on wrapper addresses.
- Stale native-fs dependency baselines and the missing sidecar-example asset. Make every documented
  command and quick start truthful; mark unavailable product functionality clearly.

**Acceptance:** default-feature and all-feature independent consumers run on normal Windows stacks;
examples execute, rather than only compile; focused identity/manifest tests pass on both toolchains;
normal drop and ordinary unwind both return net live allocations and observable physical accounts to
zero after the caught panic payload is released. Keep existing genuine poison and destructor-failure
tests. Run the workspace suite after the targeted changes.

**Scope limit:** no new formats, no attestation implementation, no new terminal evidence framework.

### B — Close fidelity and resource-accounting defects

- Preserve or explicitly reject partial OBJ UV/normal data under `preserve_or_fail`.
- Preserve PLY F64 custom attributes. Add value assertions using non-F32-representable finite data.
- Audit known record shapes: OBJ homogeneous coordinates, vertex colors, and three-component UVs;
  STL multiword solid names and material/vertex-color composition; glTF version, URI, clip-name,
  and provenance aggregation paths. Resolve demonstrated defects rather than treating these as
  automatic new feature commitments.
- Eliminate geometry validation prefix searches and meter input-dependent inner work.
- Hash each immutable resource snapshot once with chunked control/work checks; reuse its digest.
- Account for resource-graph scratch and owned path storage. Keep evidence reservations alive until
  actual deallocation or transfer into an accounted retained owner.
- Define short-read and EOF behavior independently of requested read-buffer capacity.

**Acceptance:** strict imports cannot succeed with an unauthorized known loss; allowed loss has a
specific report. Distinct-buffer validation and provenance aggregation grow near linearly in
deterministic work counters. Repeated references do not rehash payloads. At-limit and one-byte-at-a-time
readers work, over-limit readers fail, and account snapshots include overlapping manifest/Scene work.

**Scope limit:** reuse existing policy and metered storage; do not create allocation/source-code proof
languages or a second budget implementation.

### C — Deliver typed consumption and simpler authoring

- Publish borrowed views for roots/nodes/transforms/instances, meshes/primitives/topology/attributes,
  materials/bindings/images/samplers, and current deformation/animation data.
- Provide direct collection counts and checked typed lookup. Slice access where contiguous data is
  appropriate; explicit iterators/views for sparse or domain-specific representations.
- Give plugin authors a small builder with automatic document provenance and precise overrides.
  Preserve bounded validation details: domain, object index, rule, and necessary values.
- Add an independent consumer outside workspace feature unification. It must use only supported
  public exports. Geometry/material/hierarchy assertions must read typed data.

Illustrative API shape, not a prescribed signature contract:

```rust,ignore
let report = importer.read_bytes("model.obj", source)?;
for mesh in report.scene().meshes() {
    for primitive in mesh.primitives() {
        consume_geometry(primitive.topology(), primitive.positions(), primitive.indices());
    }
}
```

**Acceptance:** currently available formats support typed geometry/material/hierarchy consumption,
custom IO/plugin use, and useful error reporting without snapshots, raw stores, or authoring integration
imports. Failed custom-plugin submissions identify the violated rule. Borrowing does not allocate or
recopy the full asset. E extends the same consumer with admitted glTF; F adds transformations; H tests
packaged artifacts. Keep snapshots as separate test/debug evidence.

### D — Replace generic Scene records and duplicate validation projections

**D1: node/geometry cutover.** Start with the path used by the consumer. Compare the existing implementation against
private typed tables using identical manually expected output, budget/cancellation fixtures, and the
distinct-buffer/large-attribute workloads from B. Confirm the typed design reduces stack movement,
unnecessary record rebuilding, or scratch duplication for a concrete target cost, without unacceptable
regression in the other measures. A already closes the stack failure; D need not improve every metric.

**D2: additional domain increments.** Migrate appearance, deformation/animation, and profile ownership
according to the consumer and measured pilot results. Keep
central validation and source semantics. Use typed access internally and share unchanged tables or
pages where justified. Delete each replaced record/projection/conversion rail in the same cutover.
Remove unused alternate validator entry points and abstract APIs that no longer have callers.
E and F can deliver after their required domain cutovers; they do not wait for every D2 domain.

The next D2 increment is the complete profile/provenance domain, including owner attachments and
metadata. Normal generation can add, replace, or remove attributes, so its output owner catalog
must come from the final typed domain layout rather than a scan of input owners. Keep immutable
metadata payloads separate from owner-bearing descriptors so owner reindexing does not copy large
payloads. Share unchanged lineage segments through the existing retained-storage mechanism.
Cut fresh build, edit, central validation, semantic reads, and snapshot traversal over together;
delete their replaced profile schemas, scalar codecs, record rewrites, and projection scratch.
This is the prerequisite for normals' owner changes, not a new generic patch or scheduler layer.

The private root becomes `SceneTables`, retaining node, geometry, and profile owners through the
existing storage-first release mechanism. Profile state owns typed owner/attachment and metadata
descriptors; immutable lineage segments and metadata payloads have independent retained owners.
Store operation digests as typed values, not offset/length rows. A provenance link identifies
its exact segment and local node; derive other coordinates at checked lookup boundaries instead
of asking callers to maintain duplicate indices. Preserve sibling-branch isolation and source
identity. Derive owner additions/removals/reindexing from the concrete operation's final layout,
and remap metadata descriptors without copying unchanged payloads.

The cutover removes `schema/profile.rs`, old profile lineage/metadata record writers,
`edit/graph/profile.rs`, profile-specific inherited-leaf machinery, scalar-decoding semantic
readers, and full lineage/descriptor validation projections. Legacy block-count deltas are
replaced by actual typed table capacities and semantic counts. Existing corrupt-reference,
branch-nonce, metadata-value, cancellation, teardown, and large-payload tests remain behavioral
acceptance; obsolete physical row/stride assertions do not constrain the replacement.

**Acceptance:** identical semantic outputs; all invalid-reference/cardinality/control cases remain
rejected; simple node edits avoid copying unrelated tables; validation does not duplicate full
immutable influence/metadata payloads. Record requested allocation and observed process memory
separately. Require measured improvement on representative cases; never infer exact RSS from a ledger.

Limit the pilot to these workloads, using existing helpers or small local probes:

| Workload | Sizes | Observation |
| --- | --- | --- |
| Independent indexed primitive buffers | 256, 1,024, 4,096 primitives | Actual cache comparisons and charged work; no prefix-scan growth. |
| One node-transform edit with unchanged geometry | 1,000 and 10,000 nodes with one shared 8 MiB payload | Unrelated record-copy volume and temporary allocation. |
| Influence and metadata validation | 10,000 and 100,000 vertices with four influences; 1 and 8 MiB metadata | Temporary allocation/peak and absence of full immutable-payload copies. |

Use focused test counters or allocation-counter where needed. Observed process memory and timing are
advisory pilot data. Do not build a shared instrumentation protocol, collector, or evidence schema.

**Decision boundary:** if a necessary security invariant cannot be preserved by the proposed typed
design, document the specific failure and adjust that component before migrating it. Do not redesign
all accounting at once or weaken established limits silently.

### E — Complete the glTF/GLB vertical slice and make the backend decision

Timebox backend evaluation to one implementation investigation with a concrete reproducer and
acceptance suite. Compare a narrow maintained fix/fork against an owned bounded parser slice. Choose
one and remove superseded overlapping validation. Avoid building a second general glTF validator
around an unchanged aborting backend indefinitely.

First deliver selected-scene static PBR assets: GLB and external-resource forms, multiple primitives,
interleaved accessors, materials/bindings/samplers/images, correct reference interpretation, and
equivalent typed output. Then complete the declared skin/morph/TRS animation profile, including names,
time/interpolation semantics, and source closure. Unsupported sparse/strip/fan/extension behavior is
explicitly rejected or loss-authorized; it is not implied by a broad format label.

**Acceptance:** the known malformed reproducer returns an error under Linux sanitizer; malformed,
resource-amplification, cancellation, and limit regressions pass. Retain the fuzz quarantine until
that result exists. Enable facade registration only for the admitted profile, with a real consumer
and an exact support description. Passing `cargo fuzz check` is not admission evidence.

### F — Deliver concrete geometry postprocessing

Implement robust triangulation, flat/smooth normals, and explicit coordinate/unit conversion in
that order. Preserve polygon winding and Corner/Face remaps; reject unsupported malformed polygons.
Each operation declares the data it needs and the derived data it changes. Start with direct typed
operation implementations; add shared plan compilation only for demonstrated dependency/conflict
requirements. Keep distinct unchanged/changed results, useful diagnostics, and failure access to the
original immutable scene.

Add an explicit helper or operation for unifying corner attributes into application vertex tuples.
Do not assume triangles alone form a GPU-ready vertex/index layout. Lossy influence truncation and
precision conversion require caller authorization and specific evidence.

**Acceptance:** independent input -> typed read -> triangulation -> normals -> target-space ->
application-buffer workflow. Test winding, UV seams, smoothing boundaries, reflection/normal handling,
degenerate inputs, cancellation, and unchanged results. Keep these cases bounded and semantic.

### G — Reduce governance and accumulate useful evidence

This cleanup can proceed alongside C-F once A's baseline is recorded:

- Replace test-source hashes, AST shape checks, dead collector features, and duplicate metadata scans
  with compiler boundaries and actual behavioral tests. Use `--locked` for nested Cargo commands.
- Keep a short behavior-to-test/CI index. Defer trusted receipt systems until a real external consumer.
- Preserve sanitizer smoke; retain bounded successful fuzz corpus and logs; turn crashes into minimized
  regressions. Add lifecycle fuzz only where it covers a concrete construction/edit boundary.
- Replace the large local PowerShell scale runner with a thin Python driver. Workloads/assertions stay
  in Rust. Cover unique vertices, distinct primitive buffers, node depth/width, and resource sharing.
- Run the independent semantic consumer on native targets and WASM/WASI where support is claimed.

**Acceptance:** deleting a governance implementation does not remove its concrete risk coverage.
Ordinary test refactors do not require new source digests. Campaign progress can be reused and scale
growth checks cover the specified workloads. Perform one bounded manual negative check of the
distinct-buffer comparison assertion, then remove the deliberate regression; do not build a mutation
runner. A persistent blocking gate needs its own deterministic upper bound or justified threshold.
Timing trends remain advisory unless measurement noise and thresholds are justified.

### H — Update dependencies and prove first publication

Upgrade tooling dependencies in focused changes: Criterion, TOML, and SHA-2, preserving digest vectors.
Review triomphe/capability/rustix changes together with the operations/platform behavior they support.
Choose MSRV from actual consumer needs and test it with current stable; no compatibility requirement
forces retaining old dependency major versions. A glTF version bump alone does not fix backend admission.

Before release, build consumers from unpacked `.crate` artifacts, not only workspace paths. Cover each
advertised format feature independently and a curated default set. Align licenses/notices, support
matrix, examples, security claims, changelog, and lock/release policy. Add signing/SBOM/external
attestation according to real distribution needs after the package path is proven.

**Acceptance:** packaged consumers complete the same semantic workflows as source consumers. Only
executed target/profile claims are published; native filesystem candidate status is closed by platform
behavior evidence rather than a metadata allowlist alone.

## Ordering and verification discipline

Initial critical path: A -> B -> C -> D1 -> the first F operation. E backend investigation can overlap
C/D, and E's consumer integration can start after C. Schedule D2 domain cutovers alongside the E/F
features that need them. H integrates completed supported profiles and their actual storage paths.
G cleanup is independent once the behavioral baseline exists. Dependency-only tooling updates can be
separated from architecture changes.

Use one Cargo build at a time locally when resource pressure exists; reuse `target`, prefer nextest,
and use `cargo fmt`. Run targeted behavioral checks during each unit, then the required workspace,
doctest, lint, and feature checks at its integration boundary. Repeat broad checks only for changed
code or unresolved evidence. Commit coherent increments with Conventional Commit messages.

## Explicitly deferred

FBX/VRM/USD breadth, exporters, persistent cache/build identity, dynamic plugins, C ABI, standalone
skeletons, universal material graphs, arbitrary animation interpreters, and out-of-core storage are
deferred until one of the three consumers needs them. A few hand-authored representability tests for
layered materials and mixed interpolation can inform boundaries without implementing those systems.

## Completion

This plan is complete when normal-stack independent consumers, truthful fidelity/resource behavior,
typed consumption, simplified single storage, admitted glTF, useful postprocessing, and packaged
target/profile evidence are delivered. No unit completes merely by adding an ADR, helper framework,
source digest, or a larger count of passing internal tests.
