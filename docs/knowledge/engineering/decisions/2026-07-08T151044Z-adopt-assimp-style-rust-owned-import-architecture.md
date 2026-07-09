---
type: "Decision"
title: "Adopt Assimp-style Rust-owned import architecture"
description: "Decision for Adopt Assimp-style Rust-owned import architecture."
timestamp: 2026-07-08T15:10:44Z
tags: ["baozi", "assimp", "architecture"]
status: "proposed"
related_plan: "docs\\adr\\0001-baozi-assimp-compatible-architecture.md"
git_branch: "main"
---

# Decision

Baozi should adopt Assimp's proven architecture pattern but implement it with Rust-owned data and APIs:

- canonical scene IR
- importer registry
- virtual asset IO
- typed options and diagnostics
- ordered post-processing pipeline
- exporter traits after import and validation are stable

Baozi should not mirror Assimp's C/C++ pointer layout as its primary API.

# Context

The project goal is Assimp-level breadth in Rust. Local `repo-ref/assimp` shows that Assimp's durability comes from shared scene structures, independent importers, unified IO, and common post-processing, not just parser count.

The user also clarified:

- Existing ecosystem crates may be used if useful, but immature crates are not a blocker because Baozi can self-write parsers.
- The unrelated third-party Rust implementation should not be used as a reference.
- Assimp bindings are only possible oracles or migration bridges, not architecture input.

# Alternatives

1. Exact Assimp C ABI clone: rejected for the primary API because it would import pointer lifetime constraints into Rust.
2. Thin binding to Assimp: rejected as Baozi core because it does not create a Rust-native implementation.
3. Single crate with ad hoc parsers: rejected because it will not scale to Assimp-like breadth.
4. Rust-native Assimp-style workspace: chosen.

# Consequences

Implementation should start with architecture and invariants, then formats:

- P0: workspace, IR, registry, IO, validation
- P1: STL, OBJ/MTL, PLY
- P2: glTF2/GLB
- P3: deeper post-processing
- P4: Collada, FBX, 3MF, USD, IFC, legacy formats

Self-written parsers are acceptable and often preferable when diagnostics, control, or crate maturity require it.

# Citations

- `docs/adr/0001-baozi-assimp-compatible-architecture.md`
- `docs/research/assimp-replication-study.md`
- `repo-ref/assimp/include/assimp/scene.h`
- `repo-ref/assimp/include/assimp/mesh.h`
- `repo-ref/assimp/include/assimp/material.h`
- `repo-ref/assimp/code/Common/Importer.cpp`
- `repo-ref/assimp/code/Common/ImporterRegistry.cpp`
