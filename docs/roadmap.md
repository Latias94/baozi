# Baozi Roadmap

This roadmap states product direction and records the completed A–H library-delivery sequence. The
[delivery plan](plans/2026-10-29-001-refactor-library-delivery-plan.md) owns its scope and acceptance;
the [delivery progress](reviews/2026-10-29-delivery-progress.md#completed-ah-delivery-2026-10-31)
records exact executed target/profile evidence. The [2026-10-29 review](reviews/2026-10-29-library-state-review.md)
remains the starting observation. Format maturity, native release registration, and publication
remain separate decisions. The former U4 execution sequence is superseded history.

The [2026-11-01 Assimp comparison](research/2026-11-01-baozi-assimp-gap-analysis.md) reviews the
implementation baseline and its gaps. The adopted
[asset workflow plan](plans/2026-11-01-001-feat-asset-workflow-expansion-plan.md) prioritizes
current-profile fidelity and application consumption. Its
[implementation review](reviews/2026-11-01-asset-workflow-expansion.md) records the completed six-unit
initial tranche and exact local verification; it does not reopen A–H.

## Product Direction

Baozi is trusted, fidelity-preserving, engine-neutral 3D asset ingestion and offline compilation
infrastructure for Rust ecosystems. Its differentiation is not the number of filename extensions it
can open. It is the combination of:

- one usable, typed, immutable Scene lifecycle;
- bounded and attributable resource use for untrusted inputs;
- explicit loss, provenance, source scope, and successful-input evidence;
- explicit, deterministic postprocessing with useful geometry operations;
- format-profile maturity backed by reproducible evidence; and
- engine-neutral output suitable for runtime loaders, editors, and offline cookers.

Baozi may use narrow native or isolated adapters when a format cannot be implemented safely and
economically in pure Rust. Decoder ownership, failure containment, and canonical semantics remain
Baozi-owned. Assimp is a compatibility and differential oracle, not the production architecture.

## Completed Library Delivery Sequence

### 1. Repair Basic Execution and Source Fidelity

Normal-stack Windows triangle import, ordinary host-unwind cleanup, stable registered identity,
and executable examples establish the baseline. Specific reproductions and public behavior tests
cover the repaired OBJ/PLY data-discard paths, legal short reads, quadratic validation, repeated
cache-hit hashing, and live-memory accounting gaps.

The unpublished status permits replacing the responsible architecture. API compatibility and the
private shape of the current state machine do not justify retaining a defective design.

### 2. Deliver Typed Consumers and Simplify Scene Storage

Borrowed views expose geometry, material, hierarchy, instance, deformation, and animation values.
The independent consumer reads actual data; snapshots remain test/debug evidence. Plugin authoring
uses default document provenance and useful validation errors.

Private typed tables and immutable payloads replace generic Scene records and duplicate validation
projections. The cutover proceeded from node/geometry through each consumer's remaining domains,
deleting each replaced implementation. Operation mechanisms retain allocation admission and
lifetime-bound charging; core owns Scene semantics. The affected architecture decisions describe
the delivered implementation boundaries.

Node, geometry, profile, spatial, deformation, animation, and appearance now use retained typed
owners under one Scene root. The remaining private record directory contains revision/count fields
only. Fresh builds, central validation, public reads, snapshots, and unchanged edit sharing use those
same owners; each replaced codec and validation projection is deleted with its cutover. The
completed delivery plan and progress record carry the measured costs and final product acceptance.

### 3. Complete glTF/GLB and Useful Geometry Processing

The selected glTF/GLB profile is source-admitted through `format-gltf` and `all-formats`, using a
private maintained decoder. It includes selected-scene PBR assets in GLB and external-resource forms,
skins, morphs, and TRS animation. The [exact profile](formats/gltf.md) defines its supported slices
and explicit restrictions; source admission does not assign format maturity or publication readiness.

Concrete triangulation, Flat/Smooth normal generation, and explicit coordinate/unit conversion use
transactional edits. They preserve winding and attribute-domain remaps, require authority for
declared loss, and return the original immutable Scene on ordinary failure.

Whole-Scene space conversion now covers geometry, authored nodes, skin matrices, morph deltas,
TRS curves and derivatives, and camera/light definitions. Complete intrinsic camera/light axes
compose without changing node hierarchy. Supplemental material thickness and signed displacement
strength scale with units while unchanged encoded-image and text payloads remain shared.

The allocation-free `triangle_vertices` reader exposes one complete application tuple per draw
Corner, preserving Vertex/Corner/Face fields, influence rows, and primitive-qualified morph reads.
Applications select fields, allocate their concrete buffers, and may collect the implicit
`0..corner_count` indices; equal tuples may repeat. The independent consumer constructs and checks
owned vertex/index buffers through this workflow. Verified increments and target/profile evidence
are recorded in the [delivery progress](reviews/2026-10-29-delivery-progress.md).

### 4. Reduce Governance and Prove Distribution

Retain meaningful compiler-boundary, ownership, budget, failure, filesystem-race, and sanitizer tests.
Remove test-source hashes, AST-shape proofs, dead collector features, and repeated metadata scans.
Use a short behavior-to-test index, retained fuzz corpus, and a few explicit growth workloads. Do not
reintroduce retired `u4c-metrics` infrastructure.

Update dependencies in focused changes with relevant semantic/platform checks. Run independent
consumers against unpacked packages and supported targets. Finish licenses, notices, examples,
feature coverage, and exact profile claims before release. External attestation, signing, and cache
identity require a real distribution consumer; they do not precede basic library usability.

Formats gain maturity only through their exact tested profiles. Neither an ADR, a parser crate, nor
a passing snapshot alone constitutes supported format behavior.

Public facade features now describe only portable library capabilities. Native runtime IO and
repository capture live together in an excluded unpublished candidate workspace and compose through
the public `AssetIo` interface. Core adapters share an unconditional hidden integration namespace;
private constructors and single-use ownership enforce publication instead of Cargo feature union.
Historical internal execution rails that serve fault tests are compiled only for tests, and unused
projection/collector helpers are removed. The delivery progress records the corresponding source
and behavior qualification; this structural separation does not promote native support.

## Real-Asset Integration After A–H

The delivered follow-up increment validates the API against unchanged upstream assets through the
[standalone offline consumer](../tools/consumers/assets/README.md). Its pinned corpus covers static
texturing, interleaved storage, shared instances, skeletal TRS animation, and morph animation, plus
explicit selected-profile and numeric-policy rejections. The application owns extracted buffers
after Scene destruction; there is no second canonical Scene or automatic source repair.

Real assets exposed a false coupling between tangent construction and handedness encoding. Those
facts are now independent, and glTF declares its defined encoding without inventing a construction
algorithm. Consumer normal generation establishes CCW numeric winding first, then applies any
requested final winding. glTF's positive-Z asset front is declared independently from Baozi's
negative-Z-front target and camera-local viewing axes. These changes keep source facts and
operation preconditions explicit.

The [integration record](reviews/2026-10-31-real-asset-consumer.md) binds these changes to actual
Windows and Linux/WSL execution and separates advisory memory/timing measurements by scope.

Keep stage time, allocator requests, operation ledgers, and process RSS distinct and advisory. Extend
the corpus when a real caller supplies a missing profile, record its exact preservation/rejection
behavior, and minimize each discovered regression. Selected glTF extras now use source-object-owned
exact JSON metadata and borrowed public readers, including objects without one canonical entity.
The unchanged multi-primitive morph fixture covers their survival through edits and application
extraction, as recorded in the [metadata integration review](reviews/2026-10-31-source-object-metadata.md).
That increment has six successful assets and one named `SimpleSkin` numeric-policy rejection.
The subsequent expansion adds unchanged CesiumMan as a seventh successful asset, with source-derived
geometry, all 19 bind matrices, influence rows and every key of its 57 animation channels checked
after edits and Scene destruction. Latest verification and its platform/revision limits belong to
the corresponding review; the older A–H hosted result does not qualify later code automatically.

## Asset Workflow Expansion

The [asset fidelity and consumer plan](plans/2026-11-01-001-feat-asset-workflow-expansion-plan.md)
implements accepted PLY color semantics and MTL texture options, deterministic indexed application
buffers and public owner-to-source provenance reads. Recognized omissions in its bounded OBJ/MTL/PLY
branch list require explicit source-semantic loss authority. Clamp-on retains canonical Decal
coverage rather than an unspecified border color. Larger workloads and one bounded edit-lifecycle
fuzz target validate those workflows; exact completed observations and their limits are in the
[expansion review](reviews/2026-11-01-asset-workflow-expansion.md).

The initial tranche is complete: Windows workspace tests pass 2,482/2,482, Linux release scale
regressions include 98,304/393,216 Corner grids, five workloads have three fresh normal observations
plus separate allocation measurements, and the Linux ASan lifecycle target completes smoke,
1,803 seconds of campaigning and a 126-second saved-corpus resume. The current implementation has
no newly executed hosted/platform matrix or maturity/release promotion. Shared-primitive
measurements identify repeated single-primitive edit work to study when a concrete larger
consumer needs it, before choosing a batch API.

Tangent construction/encoding declarations now belong to each Tangent attribute, and explicit
single-primitive Triangle UV Derivative generation is implemented. Read-only base-pose world
transforms and static instance/Scene bounds are the next selected
[execution slice](plans/2026-11-01-003-feat-base-pose-world-queries-plan.md), preserving the existing
typed Scene architecture. Later selected-profile work should favor glTF primitive modes, sparse
accessors, additional influence sets and a concrete typed extension slice before starting another
broad format. Device draw limits remain a concrete consumer candidate.

Publication needs a separately chosen package set and resolvable decoder dependencies; native
distribution additionally needs a normal solution for its patched backend. Neither a local package
archive nor the native candidate's CI wiring makes these release decisions complete. Exporters,
a renderer, automatic recovery, native release registration and a generic asset compiler remain
separate work.

## Format Priorities

Format work begins only when it strengthens a current architecture or product goal.

| Priority | Format or profile | Role |
| --- | --- | --- |
| P0 | glTF 2.0 / GLB | Core game, editor, and offline-pipeline profile; GLB is the preferred single-file path. |
| P0 | OBJ / common MTL | Static-model, debugging, and legacy-toolchain profile without promising every MTL dialect. |
| P0 | PLY mesh and PLY point cloud | Two explicit scanning/tooling profiles rather than one ambiguous capability. |
| P0 | STL | Manufacturing and regression baseline; not the default game-content format. |
| P1 | VRM 1.0 | A glTF profile, not an independent parser architecture. |
| P1 | Binary FBX 7.x game profile | Optional isolated ingestion after the existing typed-consumer and processing workflows are usable. |
| P2 | Resolved USD stage | A composed Scene snapshot profile, not a claim of complete USD implementation. |
| P2 | BVH / PMX / VMD | Demand-driven motion-capture and avatar profiles. |
| P3 | Collada | Demand-driven legacy migration only. |

`.blend` is handled through a pinned, isolated Blender conversion workflow rather than direct parsing.
Alembic requires a separate time-sequenced geometry model and remains out of scope for the current
Scene IR. 3MF is considered only if Baozi deliberately enters manufacturing workflows.

## Later Product Work

Exporter infrastructure follows ADR 0038 only after registry identity, transactional sinks, loss
evidence, feature membership, and package gates are complete. Persistent caching, dynamic plugins,
third-party passes, C ABI support, out-of-core Scene storage, and standalone skeleton models remain
demand-driven. Each requires an Accepted decision only when it changes an existing public or durable
architecture contract.

## Success Metrics

| Metric | Target |
| --- | --- |
| Basic usability | Independent consumers run on normal platform stacks; normal drop and ordinary unwind release healthy owned allocations. |
| Canonical lifecycle | Typed public reads and central validation use one Scene path; completed cutovers delete replaced storage and adapters. |
| Resource safety | Actual work and overlapping live allocations follow the declared limits; terminal evidence remains bounded. |
| Evidence integrity | Direct behavior tests, bounded corpus growth, and explicit workload results are reproducible. |
| Semantic fidelity | Known source loss requires authorization and evidence; numeric precision and authored representation are preserved. |
| Format honesty | Support statements describe exact verified profiles and named evidence authority. |
| Engine neutrality | Packaged downstream consumers can ingest canonical output without a Baozi-specific engine runtime. |
| Release readiness | Locked package consumers, licenses, notices, and executed target/profile checks pass independently of ADR status. |

## Primary Risks

| Risk | Mitigation |
| --- | --- |
| Architecture work delays usable features | Deliver typed consumption first and migrate domains in bounded cuts tied to actual consumers. |
| Old and replacement paths diverge | Keep one production path per completed domain and delete obsolete implementations in that cutover. |
| Resource accounting diverges from live data | Bind reservations to actual allocation owners and test overlapping lifetimes, cache hits, short reads, and ordinary unwind. |
| Format breadth outruns evidence | Promote exact profiles only after corpus, fuzz, malformed, limit, facade, and provenance gates pass. |
| Infrastructure becomes architecture | Remove source-shape proof systems; require concrete risks and bounded behavioral checks for every replacement tool. |
