---
title: Explicit glTF Scene Selection
type: feat
date: 2026-11-01
status: in_progress
---

# Explicit glTF Scene Selection

## Outcome and scope

Complete candidate 6 of the
[asset workflow plan](2026-11-01-001-feat-asset-workflow-expansion-plan.md), following complete
influence sets in `660a4f2`. The user authorized continuing this slice on 2026-11-01.
Expose a format-owned, frozen `GltfSceneSelection` through existing option envelopes and the
public facade. Explicit indices select one declared source scene; `DocumentDefault` retains
the existing declared-default, sole-scene, ambiguous-scene and parentless-projection rules.

Do not introduce a scene union, a second Scene model, a second option precedence chain,
node back-reference expansion, or a new scheduler. Selection does not authorize retained-domain
semantic loss or provenance collapse. Existing closure and loss-policy rules remain authoritative.

## Design and execution

1. Implement immutable `GltfImportOptions` and `GltfSceneSelection::{DocumentDefault,
   SceneIndex(u32)}` in the owning format. Register its bounded version-one codec through
   ADR 0016 and re-export the public input types behind `format-gltf` in the facade. Validate
   malformed transport before primary IO; resolve a structurally valid index against the
   document before dependency IO. Preserve default < preset < caller precedence, including
   an explicit reset to `DocumentDefault`.
2. Resolve the frozen selection while validating every declared scene's roots and the complete
   source node graph. Reuse the selected descendant/resource closure and synthetic-root rule.
   Reject out-of-range indices, including explicit zero for a document with no scenes, with
   structured format-owned evidence.
3. Extend the existing source-scope evidence with a typed requested selection. Validate that
   explicit request and actual outcome agree; include both in a newly versioned digest.
   Carry the request into the generated projection operation's options digest. Keep requested
   mode and outcome readable in complete/collapsed records and later terminal failures.
4. Add self-authored regression cases for explicit nonzero/default override, ambiguous documents,
   empty scenes, shared roots, omitted dependencies, complete structural validation, skin and
   animation closure, option precedence/freeze and provenance identity. Exercise the facade
   and copy the selected result into independently owned application buffers.
   Integration exposed two necessary corrections: the maintained decoder must default an omitted
   optional scene root list, and borrowed animation-channel metadata owners must use retained
   global rows rather than source ordinals. Correct both at their owners, with direct backend
   and filtered/repeated-source-ordinal regressions; keep the source identities intact.
5. Review changed API contracts, arithmetic/accounting and failure ordering. Run focused nextest,
   affected workspace and independent-consumer suites, strict Clippy and formatting serially;
   record the tested implementation identity and results, update profile/progress documentation
   and commit the completed slice.

Only the main thread runs Cargo using Rust 1.95.0, locked/offline dependencies, one build job
and the shared target directory. Native Codex agents own the format option codec and disjoint
new importer/consumer test files. The main thread owns selection, shared evidence and integration.
Existing untracked user notes remain untouched.

## References

- [ADR 0039: selection and closure](../adr/0039-source-document-scene-selection-and-import-closure.md)
- [ADR 0016: frozen options and precedence](../adr/0016-import-options-presets-and-configuration-precedence.md)
- [ADR 0036: provenance](../adr/0036-source-identity-provenance-and-metadata.md)
- [Pinned Khronos specification, scenes](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/Specification.adoc#scenes)

Khronos permits clients to choose a scene when no default is declared. Baozi's deterministic
`DocumentDefault` and zero-scene projection are the explicit library policies defined by ADR 0039.
No new extension profile, animation runtime, snapshot wire revision or release qualification is
part of this slice.
