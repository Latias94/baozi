---
status: accepted
date: 2026-07-09
authors:
  - Codex
decision_type: architecture
related:
  - docs/adr/0003-core-scene-ir-and-material-model.md
  - docs/adr/0012-material-texture-image-and-color-space-policy.md
  - docs/adr/0015-mesh-topology-vertex-attributes-skinning-and-animation-semantics.md
---

# ADR 0023: Material and Custom Attribute Extension Model

## Context

OBJ/MTL already needs material metadata. PLY needs flexible vertex properties. glTF needs richer
PBR material fields, texture transforms, sampler behavior, skins, morphs, and extension data. If
every format adds ad hoc fields, the core IR will be repeatedly reshaped by the next importer.

## Decision

Baozi will use a layered extension model:

1. Typed common fields for behavior every runtime understands.
2. Typed descriptor structs for cross-format concepts such as texture slots, samplers, UV transforms,
   and custom vertex attributes.
3. Namespaced metadata for source-specific values that are useful for inspection, diagnostics, or
   future export but not yet stable typed fields.

Namespaces use lowercase source prefixes such as `obj:`, `gltf:`, `ply:`, and `fbx:`. Public typed
fields take precedence over metadata when both represent the same stable concept.

## Material Direction

Material support should grow toward:

- typed PBR metallic-roughness fields
- typed legacy Phong/Blinn fields where needed for source preservation
- texture slots with role, color space, UV set, sampler, transform, scale, and source key
- external and embedded texture sources without mandatory image decoding
- metadata for source-only material properties

## Mesh Attribute Direction

Meshes should grow from fixed channels into typed custom attributes:

- built-in streams remain SoA for positions, normals, tangents, UVs, colors, joints, and weights
- custom streams use namespaced semantics and explicit scalar/vector element types
- attribute lengths must be validated against vertex count or documented as face/primitive data
- unknown PLY properties and glTF extension attributes should map to custom streams when bounded and
  useful; otherwise emit diagnostics

## Consequences

Positive:

- PLY and glTF can land without replacing the core scene model.
- Renderers get typed common data while tools can inspect source-specific data.
- Extension metadata remains bounded and namespaced.

Negative:

- The IR has more descriptor types before every field has an importer.
- Promotion from metadata to typed fields requires migration tests.
