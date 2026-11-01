---
title: Typed glTF Unlit Materials
type: feat
date: 2026-11-01
status: in_progress
---

# Typed glTF Unlit Materials

## Outcome and scope

Deliver candidate 7 of the [asset workflow plan](2026-11-01-001-feat-asset-workflow-expansion-plan.md)
through `KHR_materials_unlit`, following explicit scene selection in `82c976d`. The user authorized
continuing this slice on 2026-11-01. Resolve unsupported optional-extension policy explicitly before
admitting the material schema. Use the existing canonical `ShadingModel::Unlit`, material factors,
bindings, source profile and snapshot framing; do not introduce another material model or renderer.

## Decisions

- Unsupported required extensions fail globally. An unsupported payload on a retained source owner
  fails before dependency IO even when `SourceSemanticLoss` is allowed: that authority alone does
  not define an unknown extension's fallback, references or interactions. A future lossy fallback
  requires a concrete schema-owned mapping and durable per-owner evidence.
- Unsupported optional declarations without retained payloads produce a bounded declaration-only
  diagnostic after selected-payload validation. They create no retained-domain semantic loss. Replace
  the misleading `gltf.extension_ignored` wording/code rather than maintaining an alias.
- Selected material-level unlit objects require `extensionsUsed`; required and optional declarations
  both select Unlit. Wrong sites, malformed values, unknown behavior properties and unsupported nested
  extensions fail. Preserve admitted `extras` through the existing source-object metadata path.
- Unlit uses base color, vertex color, alpha coverage and double-sided state. Original PBR lighting
  factors and texture bindings remain retained fallback source truth, including their full forward
  resource closure. The active `ShadingModel` distinguishes those values from unlit evaluation inputs.
  Retention is complete; no automatic factor rewrite, fallback texture discard or loss authorization
  is introduced.

## Execution and acceptance

1. Extend the existing admission source map for unlit payload/site identity, nested extras/extensions
   and unsupported direct properties, preserving escaped-key matching and raw byte ranges. No second
   JSON tree or schema interpreter is needed.
2. Enable the maintained backend's existing unlit feature for required-declaration validation. Compile
   shading and texture bindings into one selected material contract before dependency IO, then carry
   the closed shading enum across decoder teardown into existing canonical authoring/public readers.
3. Keep selected closure/provenance source indices and fallback resources complete. Reject unsupported
   behavior before source-scope publication/dependency IO; omitted materials do not acquire resources.
   Document the exact extension profile and independent loss authorities in the existing ADRs.
4. Copy shading model, alpha mode and double-sided state into the independent application's owned
   material alongside its existing factors/bindings. Extend the selected vertex layout and reuse key
   with COLOR0 so unlit color inputs survive extraction. Verify mixed lit/unlit, base factor/texture and
   UV transforms, vertex colors, default/alpha variants, provenance/snapshots and Scene teardown.
5. Add self-authored malformed/declaration/site/nested/omission regressions and bounded work/storage
   checks appropriate to the changed path. Run focused and integrated nextest, strict Clippy and
   formatting serially, qualify the implementation commit, update evidence and commit the slice.

Only the main thread runs Cargo using Rust 1.95.0, locked/offline dependencies, one build job and the
existing target. Native Codex agents own disjoint scanner, importer-test and consumer slices plus
read-only material/policy review. Existing user notes and unrelated changes remain untouched.

## References

- [Pinned Khronos extension](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/extensions/2.0/Khronos/KHR_materials_unlit/README.md)
- [Pinned extension schema](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/extensions/2.0/Khronos/KHR_materials_unlit/schema/material.KHR_materials_unlit.schema.json)
- [ADR 0012 material semantics](../adr/0012-material-texture-image-and-color-space-policy.md)
- [ADR 0023 extension authority](../adr/0023-material-and-custom-attribute-extension-model.md)
- [ADR 0027 private backend](../adr/0027-gltf-backend-ownership-and-replacement-policy.md)
- [ADR 0039 selection and closure](../adr/0039-source-document-scene-selection-and-import-closure.md)

Additional extension-object properties are allowed by Khronos but have undefined interpretation;
this selected profile rejects unknown behavioral properties. Package release, extension-reference
schemas, punctual lights and other material models remain separate work.
