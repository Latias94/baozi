---
status: in-progress
date: 2026-11-01
type: feat
---

# glTF primitive preservation and explicit list consumption

This slice adopts follow-on candidate 4 from the
[asset workflow plan](2026-11-01-001-feat-asset-workflow-expansion-plan.md), after the completed
base-pose queries in `3267064`. Sparse accessors and later format extensions remain separate.

## Source preservation

Map all seven glTF primitive modes directly into the existing canonical topology vocabulary.
Preserve primitive ordering, optional indices, source Vertex identity, attributes, deformation,
materials, instances and source provenance. Import never implicitly lowers or drops a primitive.
Remove the replaced partial-mode mapping and unsupported-mode branches.

The [Khronos glTF 2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#meshes)
defines the draw count from the index accessor when present, otherwise from the attributes.
Preflight the format-specific nonempty/minimum/divisibility rules before dependency I/O. Canonical
element counts are N for Points/LineLoop, N/2 for LineList, N-1 for LineStrip, N/3 for TriangleList,
and N-2 for TriangleStrip/TriangleFan. Charge the existing face/element limits and work for points
and segments as well as triangles. Count stored index cells separately from implicit draws.

Reject out-of-range indices and the forbidden maximum value of the original unsigned source
component type while decoding, before widening loses that distinction. Reuse admitted buffers,
bounded collection, polling, closure selection and source-profile ownership.

## Explicit consumer expansion

Add `list_vertices` as an allocation-free borrowed view. Return Points, LineList or TriangleList
draw tuples while preserving the Scene's original topology. Keep `triangle_vertices` as a strict
TriangleList entry using the same implementation. Use the shared `ListVertices`/`ListVertex`
types and remove replaced triangle-only type names rather than introducing aliases.

Separate output tuple index, source Corner, source Vertex and source Face. Vertex fields,
influences and morph deltas follow the original Vertex; Corner fields follow the original Corner;
Face fields follow the assembled source element. Preserve degenerates and strip parity, fan hub
reuse and every loop closure edge. Fix the exact corner order to the Khronos assembly formulas.
Short core-authored strips/fans/loops have zero assembled elements and produce an empty view;
glTF's stricter minimum counts do not change core's general authoring contract.

Construction and random access are O(1), with checked output counts and a typed overflow error.
The caller owns final-buffer allocation, packing and cancellation of its extraction loop. No Scene
publication, retained remap, operation account or generic scheduler is needed for this borrowed
consumer view. Canonical materialization, independent remap lifetime and polygon geometry
triangulation retain their existing explicit boundaries.

Update the asset consumer's direct extraction path to choose this view and retain each draw's
output topology. Its separate canonical triangle edit/normal/space workflow keeps its documented
scope. Do not silently pass lines through triangle processing.

## Acceptance and qualification

- Indexed and implicit draws for all seven modes, boundary counts, invalid modes and restart
  sentinels; explicit point/line element budgets; pre-I/O metadata rejection.
- Multiple modes in one mesh, shared instances/accessors, retained source identity/resources and
  exact source indices. No mode-dependent discard or guessed provenance joins.
- Handwritten list order, strip parity through degenerates, two-point and duplicate-endpoint loops,
  fan Face fields, Corner/Face sparse fields, complete influences and morph streams.
- Empty/short core topology, checked expansion, forward/reverse/random access, no allocation,
  exact borrowed source slices and Scene generation/data unchanged.
- Independent application extraction, indexed reconstruction and output data after Scene drop.
- Rust 1.95.0, locked/offline focused nextest followed by affected suites, strict Clippy, fmt,
  doctests/rustdoc and portable consumer checks. Serialize Cargo builds and reuse the root target.
  Record the actual source identity, results and limitations in a separate execution review.
