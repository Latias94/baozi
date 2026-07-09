---
type: "Subagent Finding"
title: "OBJ MTL importer planning findings"
description: "Distilled read-only subagent findings for the OBJ/MTL importer plan."
timestamp: 2026-07-09T08:38:06Z
tags: ["obj", "mtl", "parser", "planning", "subagent"]
related_plan: "docs/plans/2026-07-09-005-feat-obj-mtl-importer-plan.md"
---

# Finding

Read-only subagents converged on the same critical risks: OBJ must implement detection, tuple remapping, negative-index semantics, mesh splitting for material changes, MTL sidecar loading through `AssetIo`, and bounded diagnostics before it can honestly move from planned to experimental.

# Evidence

- `baozi-format-obj` currently has no `can_read` and `read` always returns unsupported.
- Current `Mesh` has one `material`, so `usemtl` changes must flush/split mesh builders.
- Current `Material` lacks metadata and `SceneBuilder` lacks `add_texture`, so MTL `illum`/legacy fields and `map_Kd` require a small core affordance before parser work can be complete.
- Current snapshots do not print texcoords, texture URIs, or texture slots, so OBJ tests must either expand snapshots or assert those fields directly.
- Assimp can be used only as a clean-room behavior checklist because copying BSD-3-Clause implementation material would contaminate Baozi's `MIT OR Apache-2.0` distribution posture.

# Recommendation

Implement in this order: core material/texture affordances, OBJ detection and parser shell, geometry tuple remap, MTL sidecars, diagnostics/limits, facade/postprocess/WASM, fuzz/CI, and docs/memory.
Use hand-authored fixtures and direct assertions for UV and texture fields.

# Disposition

Integrated into `docs/plans/2026-07-09-005-feat-obj-mtl-importer-plan.md`.
