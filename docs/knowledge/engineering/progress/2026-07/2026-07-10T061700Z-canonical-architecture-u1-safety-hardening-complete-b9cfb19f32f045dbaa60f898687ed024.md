---
type: "Work Progress"
title: "Canonical architecture U1 safety hardening complete"
description: "U1 input, detection, parser, facade, and resource-boundary hardening is implementation-complete and review-clean."
timestamp: 2026-07-10T06:17:00Z
record_id: "b9cfb19f32f045dbaa60f898687ed024"
status: "complete"
producer_id: "codex-root"
run_id: "019f4919-ef38-7b32-a0b7-476f42d35dd1"
source_session: "019f4919-ef38-7b32-a0b7-476f42d35dd1"
related_plan: "docs/plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md"
git_branch: "refactor/baozi-canonical-architecture"
---

# Summary

U1 is complete on `refactor/baozi-canonical-architecture`. Detection, import failure evidence, final validation, sidecar accounting, parser termination, and generated Scene resource limits now form one enforced boundary.

# Details

- Detection uses immutable probe snapshots and typed `Match`, `NoMatch`, `Inconclusive`, and `Fault` evidence. Selection and ambiguity are deterministic across registration order, and importer metadata is frozen at registration.
- The facade returns one canonical `ImportResult<ImportReport>` contract. Successful reports and parse, panic, validation, strict-diagnostic, and postprocess failures preserve detection evidence.
- The finalizer validates every Scene and enforces graph, per-mesh and total attribute, topology, canonical string, mesh, vertex, face, and frozen postprocess limits. Empty morph targets are invalid.
- Sidecar denial happens before resolution or IO. Failed and oversized reads consume monotonic byte/open budgets; optional MTL failures preserve OBJ geometry while required glTF buffers remain fatal.
- glTF graph construction is iterative and budgets the synthetic root, selected nodes, root edges, depth, and work before Scene construction. Repeated generated names and canonical external texture URIs are preflighted; each referenced image URI is resolved once and reused.
- OBJ state names are interned and face segmentation compares stable identity in O(1). Generated vertices, topology, and repeated output strings are charged before materialization.
- PLY rejects zero-width binary records and duplicate face-topology aliases, binds custom scalar properties to O(1) attribute slots at header time, incrementally bounds canonical polygon topology, and emits schema diagnostics once. STL, OBJ, PLY, and glTF detection handle exact probe, CRLF, UTF-8 truncation, and weak-signature boundaries.
- Obsolete facade mapping helpers and core unsupported/ambiguous format errors were deleted rather than retained as compatibility layers.

Deferred by design: U3 owns typed resource open/read causes, unforgeable stage proofs, selected-format/source failure context, diagnostic bytes, reservations, and peak-live accounting. U6 owns glTF backend admission and full accessor/skin/animation budgets.

# Next Action

Commit U1 as one safety boundary, mark U2 active, then reconcile ADR authority and add executable ADR governance without preserving old API examples.

# Citations

- [Canonical architecture plan](../../../../plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md)
- [U1 verification](../../verification/2026-07/2026-07-10T061726Z-canonical-architecture-u1-verification-f54fee89d9ba41548b6ffd29304e3ef3.md)
