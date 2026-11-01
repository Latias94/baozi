---
status: completed
date: 2026-11-01
type: feat
---

# Base-pose world transforms and static bounds

This slice adopts follow-on candidate 3 from the completed
[asset workflow expansion plan](2026-11-01-001-feat-asset-workflow-expansion-plan.md).
The implementation baseline is `6c0b1ea`. Tangent ownership and generation remain completed;
this work does not select another format, deformation evaluator, or Scene ownership rewrite.

Completed on 2026-11-01 in `947a92b`. The
[execution review](../reviews/2026-11-01-base-pose-world-queries.md) records independent review,
the corrected 20-test core query suite, 2,565 passing Windows workspace tests, 29 passing standalone
asset-consumer tests, strict Clippy, 58 doctests, rustdoc, formatting, native consumer execution and
two wasm compile-only checks. Later follow-on candidates remain unselected.

## Contract and implementation

- Derive an owned, exact-generation world-transform table by traversing the existing retained
  node forest once. Compose column-vector transforms as `parent_world * local`, including
  ancestors without mesh attachments. Canonical parent-before-child order avoids a traversal
  queue; authoring IDs need not be topological. Do not recurse.
- Name the pose explicitly: authored node base transforms in the Scene's existing coordinates
  and units. Animation clips may be present, but this API has no time-sampling request.
- Keep node lookup constant time. Bound a requested instance by transforming its actual base
  Position elements; include all vertices, including unreferenced vertices and sparse defaults.
  Do not transform the corners of a local AABB or bake geometry. Aggregate the whole Scene only
  through an explicit query; exclude uninstanced meshes.
- Reject any queried instance with a skin or a nonzero effective morph weight. An instance
  override replaces mesh defaults. Empty geometry returns `None`. Other static instances remain
  queryable even when another instance requires unsupported deformation.
- Use checked F64 arithmetic for derived transforms/positions and finite F32 results. Document
  node-by-node rounding and the precise numeric meaning of bounds. Reject unrepresentable
  results with the responsible node or instance/primitive/vertex identity.
- Bound allocation and traversal work explicitly, reserve fallibly, and poll the existing
  cancellation token during traversal. These are read-only derived queries, with no retained
  Scene mutation, publication, operation account, snapshot change, or second graph.
- Add facade exports, API documentation, and an opt-in independent asset-consumer use that
  preserves its existing animation and deformation extraction path.

## Acceptance

Use hand-calculated assertions for non-topological node order, multiple roots, shared instances,
ancestors, rotation, shear, nonuniform/negative/zero scale, actual-vertex tightness, sparse/default
and empty geometry. Check effective morph precedence, skin rejection, animated base poses,
foreign and stale handles, overflow, exact limits, cancellation, and unchanged Scene data.
Review API/resource boundaries and numerical edge cases independently before completion.

Run focused Rust 1.95.0 nextest first with one Cargo build at a time, then the affected core,
facade and independent consumer suites, strict Clippy, fmt, doctests and portable consumer checks
as justified by the integrated change. Record executed commands and counts in a review document;
earlier qualification does not substitute for checks of the new implementation.
