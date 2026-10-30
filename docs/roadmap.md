# Baozi Roadmap

This roadmap states product direction and implementation order. The
[active delivery plan](plans/2026-10-29-001-refactor-library-delivery-plan.md) owns execution scope and
acceptance. The [2026-10-29 review](reviews/2026-10-29-library-state-review.md) records the starting
evidence. This roadmap does not claim implementation completion, format maturity, or publication
readiness. The former U4 execution sequence is superseded history.

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

## Current Delivery Sequence

### 1. Repair Basic Execution and Source Fidelity

Start with normal-stack Windows triangle import, ordinary host-unwind cleanup, stable registered
identity, and executable examples. Close successful data-discard paths in OBJ/PLY, legal short-read
handling, unmetered quadratic validation, repeated cache-hit hashing, and live-memory accounting
gaps. Use specific reproductions and public behavior tests.

The unpublished status permits replacing the responsible architecture. API compatibility and the
private shape of the current state machine do not justify retaining a defective design.

### 2. Deliver Typed Consumers and Simplify Scene Storage

Expose borrowed geometry, material, hierarchy, instance, deformation, and animation views. A small
independent consumer must read actual data; snapshots remain test/debug evidence. Simplify plugin
authoring with default document provenance and useful validation errors.

Replace generic Scene records and duplicate validation projections with private typed tables and
immutable payloads. Start with node/geometry, then migrate other domains alongside their consumers.
Keep one production representation per completed domain cutover and delete the replaced code.
Operation mechanisms retain allocation admission and lifetime-bound charging; core owns Scene
semantics. Revise the affected architecture decisions together with the implementation boundary.

Node, geometry, profile, spatial, deformation, animation, and appearance now use retained typed
owners under one Scene root. The remaining private record directory contains revision/count fields
only. Fresh builds, central validation, public reads, snapshots, and unchanged edit sharing use those
same owners; each replaced codec and validation projection is deleted with its cutover. The active
delivery plan and progress record carry execution evidence and the remaining product acceptance.

### 3. Complete glTF/GLB and Useful Geometry Processing

Timebox the glTF backend fix/fork/replacement decision around its known malformed-input reproducer.
Keep glTF outside facade registration until baseline sanitizer admission passes. Deliver selected
static PBR assets in GLB and external-resource forms, then the declared skin/morph/animation profile,
with readable output and precise support claims.

Implement triangulation, normals, and explicit coordinate/unit conversion through transactional edits.
Preserve winding and attribute-domain remaps, authorize loss, and retain the input on failure.
Deliver the first concrete operation without requiring an extensible scheduling/replay framework.
Add corner-to-application-vertex lowering so consumers can build actual vertex/index buffers.

Whole-Scene space conversion now covers geometry, authored nodes, skin matrices, morph deltas,
TRS curves and derivatives, and camera/light definitions. Complete intrinsic camera/light axes
compose without changing node hierarchy. Material spatial behavior and application vertex
lowering remain open; verified increments and target/profile evidence are recorded in the
[delivery progress](reviews/2026-10-29-delivery-progress.md).

The node/geometry storage cutover enables the first processing operation. Other domain migrations
must not force every product increment to wait for a whole-library rewrite.

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
