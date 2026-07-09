---
status: accepted
date: 2026-07-09
authors:
  - Codex
decision_type: architecture
related:
  - docs/adr/0010-asset-io-virtual-filesystem-uri-archive-and-path-security.md
  - docs/adr/0013-post-process-pipeline-semantics-presets-and-mutation-model.md
  - docs/adr/0016-import-options-presets-and-configuration-precedence.md
---

# ADR 0020: Import Lifecycle and ImportReport Contract

## Context

Baozi now has a facade, registry, import context, diagnostics, validation, and post-process pipeline.
Without a lifecycle contract, each importer or facade helper can decide differently whether import
means raw parsing, validation, post-processing, stats collection, or all of the above.

## Decision

The default facade lifecycle returns a raw imported scene that has passed structural validation.
Post-processing is explicit and caller-selected. A later convenience helper may combine import and a
post-process preset, but it must expose that choice in the API name or options.

Lifecycle stages:

1. Detect format.
2. Read raw bytes and sidecars through `AssetIo`.
3. Parse into a format-owned private model where useful.
4. Build owned Baozi `Scene`.
5. Validate imported scene before returning success.
6. Return `ImportReport` with raw scene, selected format info, diagnostics, stage, and stats.
7. If the caller requested post-processing through a `*_with_postprocess` facade helper, run a
   `PostProcessPipeline`, validate the output, refresh scene counts, and return stage
   `PostProcessed`.

`ImportReport` is the boundary for import-time evidence. It should grow in additive fields before
1.0 rather than forcing format crates to invent per-format return types.

## Contract

- Importers return raw scenes. They do not run renderer convenience steps.
- Structurally unsafe scenes are errors, not warning-only reports.
- Recoverable source loss is recorded as diagnostics and should not block geometry when safe.
- `ImportReport::format()` is the selected importer descriptor, not a speculative support claim.
- `ImportReport::stats()` distinguishes primary bytes, sidecar bytes, opened assets, generated
  meshes, vertices, faces, materials, textures, and diagnostic accounting.
- Facade helpers that post-process must name the choice, for example
  `read_bytes_with_postprocess`, rather than changing `read_bytes` semantics.

## Consequences

Positive:

- Raw and runtime-ready workflows stay separate.
- Parser tests can compare source-preserving output.
- Post-process failures are not hidden behind import success.

Negative:

- Users wanting renderer-ready data must opt into a second step or explicit helper.
- `ImportReport` will need careful additive evolution before 1.0.
