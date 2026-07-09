---
type: "Work Progress"
title: "Assimp replication research draft"
description: "Work Progress for Assimp replication research draft."
timestamp: 2026-07-08T15:10:44Z
tags: ["baozi", "assimp", "research"]
status: "active"
related_plan: "docs\\research\\assimp-replication-study.md"
git_branch: "main"
---

# Summary

Drafted the initial Baozi research and ADR for an Assimp-like Rust-native asset import library.

# Details

- Created or updated `docs/adr/0001-baozi-assimp-compatible-architecture.md`.
- Created or updated `docs/research/assimp-replication-study.md`.
- Analyzed local `repo-ref/assimp` architecture: `Importer`, `BaseImporter`, `aiScene`, importer registry, post-process registry, format list, tests, fuzzing, and license.
- Subagent finding retained: Assimp has about 51 registered importers, an ordered post-process pipeline, per-format test assets, unit tests, and fuzz targets.
- Added license policy: Assimp is permissive BSD-3-Clause style, but copied/translated source needs retained notices and assets need separate license review.
- Added dependency policy: ecosystem crates are allowed as internal backends, but Baozi owns public IR, traits, diagnostics, and post-process semantics.
- User correction applied: removed the unrelated third-party Rust implementation as a reference; `asset-importer` is the user's Assimp Rust binding and only an optional oracle/bridge.

# Next Action

Discuss and choose the first implementation slice:

1. Scaffold Rust workspace crates (`baozi-core`, `baozi-io`, `baozi-import`, `baozi-postprocess`, facade).
2. Define `Scene`, id handles, `AssetIo`, `FormatImporter`, diagnostics, and `ValidateScene`.
3. Implement first parser loop with STL or OBJ using Baozi-owned IR.

# Citations

- `docs/adr/0001-baozi-assimp-compatible-architecture.md`
- `docs/research/assimp-replication-study.md`
- `repo-ref/assimp/LICENSE`
- `repo-ref/assimp/include/assimp/Importer.hpp`
- `repo-ref/assimp/include/assimp/BaseImporter.h`
- `repo-ref/assimp/code/Common/ImporterRegistry.cpp`
- `repo-ref/assimp/code/Common/PostStepRegistry.cpp`
- `repo-ref/assimp/test/unit`
- `repo-ref/assimp/fuzz`
