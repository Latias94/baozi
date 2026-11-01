---
status: in_progress
date: 2026-11-01
type: feat
---

# Attribute-owned tangent conventions and explicit generation

This execution slice adopts follow-on candidates 1 and 2 from the completed
[asset workflow expansion plan](2026-11-01-001-feat-asset-workflow-expansion-plan.md).
Its initial A1/A2/B1/B2/V1/V2 tranche remains completed. The input implementation is
`4ecc5d4`; new qualification applies to this slice's actual implementation and executed checks.

## Ownership cutover

Move construction and bitangent reconstruction declarations from `SceneSpace` to each
Tangent attribute. Keep the declarations independent: an unknown construction method may have
known `SignInW` encoding. MikkTSpace requires its defined encoding. Non-Tangent descriptors may
not carry these declarations. Default authored Tangents carry explicit unknown/unknown facts.

Thread the descriptor through authoring, admission, retained geometry storage, borrowed reads,
central validation, profile closure, snapshots and every existing edit. Namespaced schema text
belongs to the same geometry owner and resource accounting. Delete the global fields, borrowed
types' obsolete spatial lifetimes, schema authority and replaced tests. Retain no override chain.

The same primitive may retain Vertex and Corner Tangents with different declarations. Reflection
checks each present stream and rejects atomically if any encoding is unsupported. Normal-driven
Tangent removal removes that descriptor's declarations and owner-local metadata; removing one or
the last Tangent must leave no orphan names or schema requirement.

## Generation profile

Expose one `generate_tangents` operation using the existing SceneEdit admission, central validation,
same-account publication and generation-bound evidence. Select the Normal domain and Texcoord set
and domain explicitly. The selected fields must be F32x3 Normal and F32x2 UV, dense or sparse.
The existing canonical Normal/UV schemas admit Vertex and Corner domains;
reject Face and Instance selection without expanding those schemas in this slice.

Use **Baozi Triangle UV Derivative v1**, declared as `TriangleUvDerivative` plus `SignInW`.
The [algorithm research](../research/2026-11-01-tangent-generation-algorithm.md) records the verified
MikkTSpace candidates and why the initial private fixed-stack kernel is selected. The selected
algorithm neither welds nor averages across triangles and does not claim MikkTSpace equivalence.

Generate dense Corner F32x4 values under either policy when absent. Preserve any existing Tangent
by default; explicit replacement of authored values requires exactly one stream. Preserve
unrelated fields and owner identities. Reject
degenerate geometry, UVs, zero/ill-conditioned frames, mixed per-triangle handedness and any
authored morph delta on the target, including explicit zeros and sparse payloads. Empty logical
morph targets remain admissible. Preserve-existing no-op does not need generation prerequisites.

Meter domain reads and sparse lookup bounds; admit actual scratch capacity and overlapping
retained storage through the current runtime. Poll control at bounded face/read checkpoints.
Return the original Scene generation, digest and readable values on ordinary failure or
cancellation. Bind options and the selected Position/Normal/UV provenance parents to the new
Tangent owner. Keep the report usable independently of the Scene owner.

## Acceptance and execution

- Ownership: independent stream declarations, namespaced closure/text accounting, borrowed reads,
  snapshot participation, remap preservation, deletion and reflection rejection.
- Numeric kernel: hand-derived ordinary/mirrored/reversed-normal and seam cases, permutation
  invariance, normalized orthogonal frames, positive zeros, exact handedness, extreme F32
  cancellation, degeneracy, fixed allocation-free execution and threshold boundaries.
- Operation: selected Vertex/Corner combinations and UV sets, preserve/replace, insertion and
  unaffected values, stale handles, source provenance/options digest, limits, cancellation,
  morph rejection, post-generation reflection and independent report lifetime.
- Integrated checks: format producers, facade and standalone consumers; serial Rust 1.95.0
  nextest, strict Clippy, formatting, doctests and relevant target checks. Regenerate the lockfile
  through Cargo when adding the existing workspace SHA-256 dependency edge.
- Record actual commands, toolchain/profile, counts and limitations in the review record. Old
  campaign/target results are historical context and do not qualify the new code.

Many-primitive batch editing, MikkTSpace adaptation, smoothing, deformation-aware generation,
UV conversion, world bounds and additional format admission remain separate future candidates.
