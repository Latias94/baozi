---
status: accepted
date: 2026-07-09
authors:
  - Codex
decision_type: architecture
related:
  - docs/adr/0003-core-scene-ir-and-material-model.md
  - docs/adr/0015-mesh-topology-vertex-attributes-skinning-and-animation-semantics.md
---

# ADR 0025: Scene IR Invariants and Validation Boundaries

## Context

Baozi's `Scene` is an owned Rust IR, not a C pointer graph. Users still need to inspect and adapt
scene data after import, so making the entire IR immutable would make legitimate tooling harder.
The risk is that public fields can be edited into invalid states unless validation boundaries are
clear.

## Decision

`Scene` remains an editable owned data package, with validation enforced at construction and import
boundaries:

- ID newtypes have private tuple fields and expose `as_u32()` / `index()` accessors.
- `SceneBuilder::finish()` validates before returning `Scene`.
- Facade import returns only validated scenes.
- Facade postprocess helpers validate output before returning `PostProcessed`.
- Direct user mutation is allowed, but callers must run `validate_scene` before handing scenes back
  to Baozi processing.

The validator owns IR invariants:

- root node range and parent rules
- node parent/child consistency and acyclic reachability
- mesh/material/texture/camera/light/skin references
- topology and index consistency
- SoA channel lengths for built-in streams, joints, morph targets, and custom attributes
- finite numeric payloads for geometry, materials, cameras, lights, skins, and animations
- animation target ranges, sorted times, and value counts

## Consequences

Positive:

- Tooling can edit scenes without fighting opaque builders.
- Import/postprocess boundaries remain safe.
- Invalid future fields should fail tests as soon as they are exposed.

Negative:

- Downstream users can still construct invalid scenes manually.
- New IR fields must update validator and snapshot code in the same change.
