# Baozi Roadmap

This roadmap states product direction and implementation order. It is not an implementation-status
ledger, a format-maturity claim, or a substitute for Accepted ADRs. Current progress belongs in the
engineering-state record and repository conformance inventory.

## Product Direction

Baozi is trusted, fidelity-preserving, engine-neutral 3D asset ingestion and offline compilation
infrastructure for Rust ecosystems. Its differentiation is not the number of filename extensions it
can open. It is the combination of:

- one canonical, immutable Scene lifecycle;
- bounded and attributable resource use for untrusted inputs;
- explicit loss, provenance, source scope, and successful-input evidence;
- deterministic, typed post-process compilation;
- format-profile maturity backed by reproducible evidence; and
- engine-neutral output suitable for runtime loaders, editors, and offline cookers.

Baozi may use narrow native or isolated adapters when a format cannot be implemented safely and
economically in pure Rust. Decoder ownership, failure containment, and canonical semantics remain
Baozi-owned. Assimp is a compatibility and differential oracle, not the production architecture.

## Current Architecture Sequence

### 1. Complete the Canonical Scene Cutover

The immediate priority is the U4 sequence defined by the canonical architecture plan:

1. complete the profile of the opaque core-owned Scene construction continuation;
2. make import, every format producer, post-process, snapshot, fuzz, and facade use that one path in
   an atomic workspace switch;
3. delete the legacy Scene, `SceneBuilder`, `SharedStorage`, bridge code, and migration-only seams;
4. remove the unused BZIR request wire; and
5. record the resulting implementation state in the focused repository conformance inventory.

The cutover is incomplete while a public or production path can choose between legacy and canonical
storage. No compatibility adapter is retained after the switch.

Before the atomic switch, the U4 lifecycle and production comparators must match the current ledger
and report vocabulary, run in CI, and fail closed on source-closure or evidence drift. The minimum U4
contract and gate records must also exist so the cutover cannot silently bypass its architecture
evidence.

### 2. Close Resource and Text Input Boundaries

After the single Scene lifecycle is established:

- centralize the origin-qualified resource graph and complete successful-input manifest;
- centralize BOM, UTF-8, newline, and source-span mapping while keeping each grammar format-owned;
- preserve bounded role, alias, cycle, depth, digest, and source identity evidence; and
- keep producer/build qualification separate until its Proposed decision is accepted and has a real
  consumer.

### 3. Prove the Canonical Profile with Existing Formats

Existing STL, OBJ/MTL, PLY, and glTF work is used as proof material for the canonical contracts, not
as four competing Scene models. OBJ/MTL proves text, sidecars, topology, materials, and unchanged
typed views. glTF/GLB proves scenes, authored transforms, PBR appearance, external resources, skins,
morphs, and animation.

No format receives a maturity promotion merely because its parser crate exists. Promotion follows
the exact profile and evidence rules in ADR 0011.

### 4. Replace Fixed Post-Process Flags

Implement ADR 0013 as a compiled, typed, loss-aware dependency plan rather than a fixed bitmask and
global execution order. Each pass must declare prerequisites, invalidation, mutation, validation,
loss, diagnostics, outcome, and generation/remap behavior. Tessellation remains behind its admitted
adapter and resource envelope.

### 5. Build the Evidence and Release System

Complete the architecture conformance inventory, gate catalog, canonical lifecycle fuzzing, packaged
facade consumer, provenance-qualified regression corpus, digest snapshots with bounded previews,
feature/target/package matrices, and cross-platform evidence. Release work then closes locked tools,
licenses, SBOM, attestation, signing, and reproducibility without conflating publication readiness
with ADR acceptance or format maturity.

## Format Priorities

Format work begins only when it strengthens a current architecture or product goal.

| Priority | Format or profile | Role |
| --- | --- | --- |
| P0 | glTF 2.0 / GLB | Core game, editor, and offline-pipeline profile; GLB is the preferred single-file path. |
| P0 | OBJ / common MTL | Static-model, debugging, and legacy-toolchain profile without promising every MTL dialect. |
| P0 | PLY mesh and PLY point cloud | Two explicit scanning/tooling profiles rather than one ambiguous capability. |
| P0 | STL | Manufacturing and regression baseline; not the default game-content format. |
| P1 | VRM 1.0 | A glTF profile, not an independent parser architecture. |
| P1 | Binary FBX 7.x game profile | Optional isolated ingestion path after U4 and evidence infrastructure are stable. |
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
| Canonical lifecycle | Every producer and consumer uses one Scene path; legacy storage and adapters are absent. |
| Resource safety | Hard caller limits include protected terminal evidence and all input-dependent work is bounded, cancellable, or isolated. |
| Evidence integrity | Contract, gate, snapshot, corpus, and format evidence is reproducible and CI-enforced. |
| Semantic fidelity | Loss, provenance, source scope, authored representation, and deterministic digests remain explicit. |
| Format honesty | Support statements describe exact verified profiles and named evidence authority. |
| Engine neutrality | Packaged downstream consumers can ingest canonical output without a Baozi-specific engine runtime. |
| Release readiness | Locked builds, package consumers, licenses, SBOM, attestation, signatures, and supported target matrices pass independently of ADR status. |

## Primary Risks

| Risk | Mitigation |
| --- | --- |
| Accepted decisions outrun implementation | Maintain explicit `unassessed`, `partial`, `conforming`, and `nonconforming` contract records with executable gates. |
| Legacy and canonical paths diverge | Make U4e an atomic switch and U4f immediate deletion, with no compatibility adapter. |
| Resource accounting leaks through helpers | Keep ownership and terminal policy in deep modules; expose bounded capabilities rather than raw ledger or retained-storage types. |
| Format breadth outruns evidence | Promote exact profiles only after corpus, fuzz, malformed, limit, facade, and provenance gates pass. |
| Infrastructure becomes architecture | Keep current progress, hashes, commands, and implementation type names out of durable ADR semantics. |
