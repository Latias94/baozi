---
type: "Work Registration"
title: "Baozi Assimp replication research"
description: "Registration for Baozi Assimp replication research."
timestamp: 2026-07-08T14:58:35Z
status: "active"
last_seen: 2026-07-08T15:19:06Z
producer_id: "codex-root"
related_plan: "docs\\adr\\0001-baozi-assimp-compatible-architecture.md"
git_branch: "main"
---

# Scope

Research how Baozi should replicate Assimp's multi-format model loading capability as a Rust-native workspace.


# Current Claim

Active research is documented in:

- [ADR 0001](../../../adr/0001-baozi-assimp-compatible-architecture.md)
- [Assimp replication study](../../../research/assimp-replication-study.md)

# Latest Links

- Assimp local reference: `repo-ref/assimp`
- Main architecture decision: `docs/adr/0001-baozi-assimp-compatible-architecture.md`
- Research notes: `docs/research/assimp-replication-study.md`

# Handoff

Current direction:

- Use an Assimp-style architecture: canonical scene IR, importer registry, virtual IO, ordered post-processing, exporter traits later.
- Keep Baozi Rust-native: owned scene model, id handles, typed options, structured errors and diagnostics.
- License boundary: Assimp is permissive BSD-3-Clause style, but copied/translated code needs retained notices; test assets must be curated, especially `test/models-nonbsd`.
- Dependency boundary: third-party crates may be internal parser/algorithm backends, but cannot define Baozi public API.
- User correction: remove the unrelated third-party Rust implementation as a reference; `asset-importer` is the user's existing Assimp Rust binding and may be used only as a bridge/oracle, not architecture input.


# Citations

- `repo-ref/assimp/LICENSE`
- `repo-ref/assimp/include/assimp/Importer.hpp`
- `repo-ref/assimp/include/assimp/BaseImporter.h`
- `repo-ref/assimp/code/Common/ImporterRegistry.cpp`
- `repo-ref/assimp/code/Common/PostStepRegistry.cpp`
- `repo-ref/assimp/doc/Fileformats.md`
