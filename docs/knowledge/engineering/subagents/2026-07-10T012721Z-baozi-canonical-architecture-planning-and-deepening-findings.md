---
type: "Subagent Finding"
title: "Baozi canonical architecture planning and deepening findings"
description: "Distilled architecture, security, performance, and Assimp comparison findings that changed the implementation plan."
timestamp: 2026-07-10T01:27:21Z
tags: ["baozi", "canonical-architecture", "assimp", "security", "performance", "ce-plan"]
status: "accepted"
source_session: "019f4919-ef38-7b32-a0b7-476f42d35dd1"
subagent_id: "deepen-architecture,deepen-security,deepen-performance"
related_plan: "docs/plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md"
git_branch: "refactor/baozi-canonical-architecture"
---

# Finding

- The mature-project comparison exposed five triggerable boundary defects before schema work: probe poisoning, zero-progress binary PLY records, unbounded glTF/Scene graph traversal, facade validation bypass, and format-local sidecar policy.
- ImportSession must exist before primary open and detection so one outcome builder, budget, diagnostic stream, and failure path cover the entire operation. Parsers receive a restricted ParseScope rather than raw AssetIo or report/stage constructors.
- Baozi can constrain untrusted asset data and first-party parsers, but cannot sandbox hostile in-process importer, AssetIo, or resolver code. Hosts must isolate genuinely untrusted plugins in another process.
- Resource governance needs monotonic attempted/consumed work, releasable live reservations, committed output, checked allocation sizes, and a peak-live limit spanning snapshots, backend decode, normalized data, canonical output, and scratch buffers.
- The canonical Scene IR needs explicit collection ownership/cardinality plus SceneBuilder to ValidatedScene proof transitions. Structural editing must be transactional and return remaps and derived-data invalidation.
- Graph work must be O(nodes + edges), postprocess must stage only affected replacement buffers, and snapshot digests must stream canonical values rather than materialize full normalized text.
- U5 and U6 remain parallel only because normalized format models stay private and their shared builder/invariant contract is frozen in U4.

# Evidence

- The repository audit reproduced the current failures in import detection, PLY parsing, glTF graph construction, facade report construction, and sidecar policy enforcement.
- Architecture, security, and performance deepening independently identified lifecycle ownership, trust-boundary, and peak-memory gaps in the first plan draft.
- The accepted plan now records these constraints in KTD16-KTD22 and strengthens U1-U8 plus the Verification Contract.

# Recommendation

Execute U1-U9 in dependency order. Keep U3 and U4 as exclusive shared-boundary units, permit only the file-disjoint U5/U6 format work to run in parallel, and capture each unit's commit and verification evidence as new sharded memory concepts.

# Disposition

Accepted into the 2026-07-10 canonical architecture plan during confidence deepening. Mandatory headless document review remains the final planning gate before ce-work execution.

# Citations

- [Canonical architecture implementation plan](../../../plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md)
- [Assimp-style owned Rust architecture decision](../decisions/2026-07-08T151044Z-adopt-assimp-style-rust-owned-import-architecture.md)
