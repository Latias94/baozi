---
status: accepted
date: 2026-07-09
updated: 2026-11-02
authors:
  - Codex
decision_type: architecture
depends_on:
  - docs/adr/0045-canonical-scene-ownership-construction-and-generation.md
related:
  - docs/adr/0006-public-api-versioning-and-crate-stability-policy.md
  - docs/adr/0011-format-support-tiers-and-compatibility-charter.md
  - docs/adr/0031-animation-skin-morph-and-deformer-contract.md
  - docs/adr/0032-postprocess-remap-and-derived-data-contract.md
  - docs/adr/0039-source-document-scene-selection-and-import-closure.md
---

# ADR 0008: Math Types, Scene Space, Units, and Numeric Policy

## Context

Coordinate conventions are part of the asset's meaning. A model can have numerically valid
positions and still render incorrectly when handedness, front-face winding, UV origin, tangent
basis, units, or matrix conventions are implicit. Animation time also needs more precision than
renderer-oriented geometry buffers.

## Decision

Baozi exposes small Baozi-owned math types. Public IR types do not expose a parser backend, SIMD
type, `glam`, `nalgebra`, or `cgmath`. Optional ecosystem adapters are conversions, not IR owners.

Public math semantics are fixed independently from memory layout:

| Topic | Canonical meaning |
| --- | --- |
| vectors | right/left handedness comes from `SceneSpace`, not the vector type |
| matrices | column vectors; indexed accessors name row and column; a matrix's domain defines its source and destination spaces |
| authored node transforms | local-to-parent; `p_parent = M_local_to_parent * p_local`; TRS composes as `T * R * S` |
| quaternion | `(x, y, z, w)` component meaning, finite unit rotation within documented tolerance; named constructors avoid positional ambiguity |
| angles | radians |
| normals | inverse-transpose under non-uniform scale |
| bounds | closed axis-aligned minimum/maximum in the declared owner space |

No `repr(C)`, SIMD alignment, column-major byte storage, or binary ABI follows from these semantic
rules; ADR 0009 keeps memory layout private.

Numeric storage is deliberate:

| Value class | Canonical scalar | Rule |
| --- | --- | --- |
| positions, normals, tangents, and evaluated UV attributes | per-stream `f32` or `f64` | preserve admitted width and finite values; explicit narrower output owns loss/error policy |
| authored transforms and material/color factors | `f32` | retain their source-domain contract; reject non-finite values and diagnose admitted narrowing |
| encoded color and custom attribute storage | ADR 0030 `ComponentType` | preserve admitted width/normalization; evaluated color retains its F32 profile |
| derived base-pose world matrices and bounds | `f64` arrays | one query/result family; no per-node F32 projection |
| animation key time and clip duration | `f64` | preserve authored time domain; do not silently resample |
| animation TRS samples, morph weights, and morph deltas | `f32` | retain renderer-independent values while keeping source time in `f64` |
| source unit scale and timing rates | `f64` | finite and positive when known |
| metadata numeric value | typed `i64`, `u64`, or `f64` | no string coercion |

The raw scene preserves source numbers and records a complete `SceneSpace`:

- handedness
- signed up and front axes
- unit scale to meters
- clockwise or counter-clockwise front-face winding
- upper-left or lower-left UV origin, or unknown

Each Tangent attribute separately owns its construction convention and handedness encoding.
These independent facts are absent from `SceneSpace`: either may remain unknown on an authored
Tangent descriptor, and a primitive's Vertex and Corner streams may differ. Non-Tangent attributes
cannot carry tangent declarations. Declaring MikkTSpace requires `SignInW`. An importer may know
that tangent W encodes a sign without knowing the construction algorithm. In particular, glTF
declares `SignInW` for each authored Tangent and an upper-left Scene UV origin, without guaranteeing
MikkTSpace construction. A scene without Tangents has no tangent declarations.

glTF declares a positive-Z asset front. This source convention is distinct from Baozi's
negative-Z-front standard target and from camera-local negative-Z viewing axes. Import records the
source front without conversion; the explicit Scene conversion transports every affected domain.

Column vectors and local-to-parent authored node transforms are fixed canonical API semantics,
not source facts or target options. `SceneSpace` does not carry a duplicate transform-convention
tag. Inverse bind, correction, and other matrices retain their domain-specific source/destination
spaces; their meaning is not redefined as a node transform. The earlier removal of the redundant
tag changed the then-current deterministic snapshot schema to revision 4. The 2026-11-02 amendment
below records the current draft-v1 cutover.

Unknown is an explicit value, never a guessed default. The standard spatial target is right-handed,
positive Y up, negative Z front, meters, and counter-clockwise front faces. `TargetSpace` requires
these five known spatial facts together. Coordinate conversion preserves UV origin and tangent
basis declarations. A separate implemented operation is required to change those conventions;
generated tangents use the documented Baozi basis. Reaching any target requires an explicit
postprocess operation.

Validation rejects signed up/front axes on the same underlying axis. When both are known, they and
the declared handedness must derive one finite orthonormal side axis; a complete-space claim cannot
retain `Unknown`. Front-face winding remains an explicit independent convention. Reflection checks
each Tangent attribute's encoding; any unsupported stream rejects the operation atomically.
Every Baozi
quaternion field, including future static TRS fields, obeys the same finite-unit rule; normalization
outside tolerance is an explicit diagnosed recovery, not a constructor side effect.

The canonical node base transform preserves one of two authored forms:

- `Matrix`: a finite affine local-to-parent matrix in the convention above; and
- `Trs`: finite translation, finite unit rotation, and finite scale, with format-defined omitted
  components materialized as translation zero, rotation identity, and scale one.

For `Trs`, the canonical local matrix is derived as `T * R * S` under the column-vector convention.
For a conforming `Matrix`, it is the preserved authored matrix. Neither form is silently rewritten
into the other: format rules decide whether a source matrix must be decomposable or exclude shear,
and a conversion from matrix to TRS requires an explicit format contract plus provenance and
authorized representation loss. A derived matrix view never changes which form the source authored.

`Mat4::from_trs` derives components with F64 intermediates and one final F32 rounding. As an
unchecked math helper it may produce a nonfinite result for an unrepresentable component. The
read-only `BasePoseWorldTransforms` query checks local derivation and world composition, including
roots, rather than assuming a valid authored TRS guarantees a representable derived matrix.
It composes local/world components in F64, without projection at node boundaries, and rejects
nonfinite F64 output or nonzero products underflowing to zero. Authored Matrix/TRS fields retain
their F32 representation. Matrix and point dot products accumulate in increasing column order;
points add translation last. This is deterministic floating-point evaluation. Static bounds use
F64 min/max arrays over computed F64 world vertices, with no exact-real interval guarantee. The
[Scene reader contract](../model/reading-scenes.md#base-pose-world-transforms-and-static-bounds)
specifies the all-Position-vertex scope, pose/deformation limits and query resource boundary.

The canonical `Matrix` invariant is format-independent: every cell is finite and the affine bottom
row compares exactly to `[0, 0, 0, 1]` under the convention above. Signed zero compares equal to zero
and its authored bit remains preserved. Canonical construction never stores a near-affine bottom row.
Format-owned numeric predicates may impose stricter decomposition, orthogonality, singular-scale,
reflection, or reconstruction rules, but their tolerances cannot relax this canonical affine
invariant.

A non-glTF format may define an explicit recovery that accepts a near-affine source value only when
it normalizes the bottom row before canonical construction and records the original source values,
recovery provenance, and policy-authorized loss. Such output is a recovered canonical matrix, not a
bit-preserved authored matrix. A conforming admission path never clamps, rewrites, or changes the
stored authored matrix bits. ADR 0027 rejects the corresponding invalid glTF input instead of
recovering it.

The unit-quaternion rule applies to rotation sample values, not cubic interpolation derivatives.
Rotation cubic tangents are finite `Vec4<f32>` derivatives in authored component order and are not
normalized or interpreted as rotations. ADR 0031 owns channel cardinality and tangent layout.

```mermaid
flowchart LR
    Source[Source numeric values and conventions] --> Raw[Validated raw scene]
    Raw -->|inspect| Caller[Source-preserving consumer]
    Raw -->|explicit conversion| Plan[Coordinate and unit operation]
    Plan --> Geometry[Geometry and topology]
    Plan --> Hierarchy[Nodes, cameras, lights]
    Plan --> Deform[Skins, morphs, animation]
    Plan --> Derived[Invalidate or recompute bounds, normals, tangents]
    Derived --> Normalized[Validated normalized scene]
```

Coordinate conversion is atomic across every affected domain. It updates base positions, normals,
tangents, morph position/normal/tangent deltas, spatial mesh-instance attributes, winding, node
transforms, inverse bind and correction matrices, cameras, lights, animation values, material
displacement strengths, material thickness factors, every other explicitly Scene-unit material or
binding parameter, and scene-space metadata together. The conversion preserves each value's physical
meaning before and after the unit-scale change and invalidates derived data it cannot recompute. An
operation that cannot convert a present semantic fails or reports policy-approved loss; it never
returns a scene whose metadata claims a conversion that only base geometry received.

A basis transform with negative determinant reverses topology winding when required to preserve the
declared front face and updates tangent handedness consistently. The operation validates the output
space descriptor and geometric orientation together before commit.

For signed cardinal axes, define the basis columns as `(side, up, front)`, where
`side = handedness_sign * cross(front, up)`. Conversion from source to target uses the signed
permutation `Q = B_target * transpose(B_source)` and positive unit ratio
`s = source_units / target_units`. Positions and translations use `s * Q`, normals and tangent
directions use `Q`, tangent `w` uses `det(Q)`, and a local-to-parent matrix uses
`H * M * inverse(H)` with `H = diag(s * Q, 1)`. TRS scale components are permuted without the
basis signs or unit factor; quaternion vector components use `det(Q) * Q` and retain `w`.
Matrix/TRS authored form is preserved, including shear and singular affine matrices.

Inverse binds and optional skin corrections use the same `H * M * inverse(H)` map without
changing their declared source/destination spaces. Position morph deltas use `s * Q`; Normal
and Tangent morph deltas use `Q`. Sparse omissions and explicit entries keep their representation.
Animation values and cubic derivatives use the linear map of their target: `s * Q` for translation,
unsigned axis permutation for scale, and `(det(Q) * Q * xyz, w)` for quaternion components.
Quaternion derivatives remain arbitrary finite Vec4 values, including zero/nonunit derivatives;
neither samples nor derivatives receive sign canonicalization or normalization. Key times and
time-domain rates are unchanged.

Camera/light reference coordinates retain a complete `LocalAxes` signed permutation, with
reference-to-node matrix `B = [right, up, -forward]`. Conversion composes `B' = Q B` while nodes
continue to use `H M H^-1`. It does not append compensation nodes or derive one intrinsic axis
from a fixed-handedness cross product. Camera lengths and finite light ranges scale by `s`;
photometric intensity remains independent of Scene length units. ADR 0040 owns the complete
projection/emission reference contract and post-rounding plane constraints.

Triangle lanes reverse exactly when `(det(Q) < 0) XOR (source_winding != target_winding)`.
Index and every Corner tuple follow the same `[0, 2, 1]` permutation. A winding-only change does
not multiply tangent `w` again. Transported normals retain their physical direction; independently
generating normals from index-order cross products follows that operation's numeric-basis contract.

Unknown source facts require explicit source assumptions. A complete assumption may fill unknown
fields and repeat matching known fields, but cannot override any known field. The target never
supplies implicit source facts. A changed conversion reports exactly which fields were assumed.
Numeric conversion preserves each geometry attribute's retained width. F32 values use F64
intermediates and one final F32 rounding; F64 values use F64 arithmetic directly. Signed-axis
permutation and copy/remap paths preserve admitted bits except the requested sign change.
Non-finite output or a nonzero component underflowing to zero rejects the transaction.

Floating-point snapshots use deterministic ordering and per-field documented absolute/relative or
angular tolerances. Tolerances are test comparison policy, not permission to clamp imported data.

When a format contract uses a numeric predicate beyond the exact canonical affine check, that format
ADR must freeze the predicate, promotion precision, operation order, inclusive/exclusive
comparisons, absolute/relative tolerances, zero-scale behavior, and reflection/negative-scale
behavior. The predicate is Baozi-owned and runs before canonical construction. A backend
decomposition routine or its test epsilon is not normative. A conforming predicate never rewrites or
clamps the authored matrix; an explicit recovery follows the separate loss/provenance rule above.
ADR 0027 supplies the concrete glTF predicate.

### Precision And Geometric Scope

The [reviewed maturation plan](../plans/2026-11-02-001-refactor-first-release-maturation-plan.md)
reopens the earlier F32-only geometry decision for the existing scanning/asset workflow. PLY float
and up-to-16-bit integer built-ins select F32; double and I32/U32 select F64. Mixed components use
the wider declared width before allocation, independently of their current values. Position,
Normal, Tangent and Texcoord share the existing typed attribute/storage family, including sparse
F64. They do not require a separate Scene or a universal scalar parameter.

Preserving finite F64 source data establishes a storage contract. Numerical operations have their
own bounded arithmetic contract: ordinary F64 geometry must be supported through the maturation
work, while ill-conditioned or unrepresentable computations reject with an explicit numeric cause.
An F64 payload does not by itself prove robust orientation, exact topology or a manufacturing error
budget. Derived fields use the widest dependency or replaced-field width; applications own any
requested F32 packing and its explicit loss/error policy.

Normal generation uses the declared Position width for its determinant specialization and the
widest Position/replaced-Normal width for output. Tangent output joins Position, the selected
Normal/UV fields and a replaced Tangent; each determinant still selects its algorithm from its
own operand declarations. F32 factors retain the exact-product expansion. Binary64 arithmetic
uses exact common power-of-two scaling, FMA residual/error bounds and compensated area sums;
it rejects unresolved directions and frames rather than normalizing a rounding residual.

The F64 path uses one Position scale per selected primitive, and tangent preparation uses a separate common
scale for its selected UV field. Dense scalar scans and sparse explicit/default scans cover the
same logical magnitudes without expanding sparse storage. Scaling that loses a nonzero value
or significand bits is a typed numeric failure. F64-derived directions require a conservative
32-epsilon relative component-error gate; tangent projection and alignment retain their explicit
error budgets even when output is widened solely by a replaced F64 field. These policies bound
this numerical operation graph; they do not provide exact predicates or tolerance-based repair.

STEP/IGES/B-rep, exact surfaces and manufacturing-tolerance workflows require a separate concrete
producer and an accepted domain contract. Such a proposal must define evidence for:

- target domains and formats, dimensional/unit semantics, model extents, and an explicit absolute,
  relative, and topology-sensitive error budget;
- how the existing per-attribute storage represents its values and where a different domain model
  is actually required;
- robust-predicate and tolerance semantics for validation, topology, triangulation, transforms,
  derived data, snapshot/digest, serialization, and postprocess;
- measured memory, throughput, and cross-platform determinism costs; and
- independent precision, round-trip, adversarial, and large-coordinate fixtures proving that values
  which matter to the declared CAD contract do not collapse or change topology.

Format maturity, facade and release claims must name that concrete profile and its evidence.
The present four-profile ingestion contract supplies no exact-surface or manufacturing guarantee.

## Considered Options

### Option A: Expose one ecosystem math crate

This gives rich operators and SIMD quickly, but turns that crate's versions and feature choices into
Baozi's public compatibility burden. Rejected.

### Option B: Normalize every format while parsing

This simplifies renderer examples, but loses source truth and duplicates conversion logic across
importers, especially for skins and tangent bases. Rejected.

### Option C: Baozi math, explicit source space, explicit conversion

This keeps parser output auditable and lets tools choose source-preserving or normalized workflows.
Chosen.

## Consequences

- Geometry remains efficient for common graphics consumers while animation retains `f64` timing.
- Importers must populate known space fields and leave unknown fields explicit.
- Matrix and tangent conventions become normative and tested rather than comments on individual
  importers.
- Source precision belongs to each admitted geometry attribute; source width, computed output
  width and an application's packed buffer width remain explicit decisions.

## Risks and Mitigations

| Risk | Severity | Mitigation |
| --- | --- | --- |
| A narrow projection collapses source geometry | High | Preserve declared F32/F64 attribute widths; require explicit loss/error authority for narrower output. |
| Partial coordinate conversion corrupts animation or skinning | High | Use operation read/write sets and integration fixtures containing all spatial domains. |
| Unknown source conventions invite caller guesses | Medium | Preserve `Unknown`, allow explicit caller-supplied source overrides, and record them in provenance. |
| Platform float behavior changes snapshots | Medium | Use scalar reference tests, deterministic ordering, and field-specific tolerances. |
| Format breadth language implies unsupported exact-surface guarantees | Critical | Require a concrete producer, topology/tolerance contract and independent conformance evidence. |

## Success Metrics

| Metric | Target | Measurement |
| --- | --- | --- |
| Time fidelity | Distinct authored `f64` times never collapse through `f32` conversion | long-time and close-key fixtures |
| Space completeness | A complete-space claim has no unknown or contradictory convention | exact/excess field validation tests |
| Conversion completeness | Every declared spatial domain is converted or the transaction fails | all-domain coordinate fixture and invalidation report |
| Precision fidelity | Adjacent F64 source values remain distinct through storage, reads and applicable operations | all-encoding PLY, sparse, query and numerical fixtures |
| Scope truthfulness | Claims distinguish finite F64 retention from exact-surface/manufacturing semantics | roadmap, format-matrix, facade-doc, and release-doc checks |

## Acceptance Criteria

- Public API inspection finds no third-party math types.
- Raw fixtures preserve declared handedness, axes, units, winding, UV origin, and tangent basis.
- Coordinate conversion fixtures cover nodes, geometry, topology, skins, animation, cameras, lights,
  material/binding Scene-unit parameters, and derived-data invalidation in one transaction, proving
  equal physical meaning before and after the unit-scale change.
- Animation times that differ below `f32` precision remain distinct after import.
- NaN, infinity, invalid unit scales, and invalid quaternions fail validation deterministically.
- Matrix-authored and TRS-authored nodes retain distinct authored forms while producing the same
  documented local-matrix view where their numeric transforms are equivalent.
- Every canonical Matrix has an exact affine bottom row; format-owned stricter admission publishes
  one backend-independent numeric predicate with fixed orthogonality/decomposition, zero-scale, and
  reflection fixtures, while explicit non-glTF recovery records normalized source loss/provenance.
- CAD-oriented format/workflow admission fails governance review unless an accepted update to this
  ADR supplies the complete precision and migration evidence above.

## Amendment History

### 2026-11-02: Preserve numeric semantics through the draft-v1 cutover

U12 rebases the current core-owned Scene snapshot from observed revision 8 to draft revision 1,
consumed through `ValidatedScene`. The earlier revision-4 transition remains historical. Numeric
widths, float bits, transform conventions and explicit conversion rules retain their current
semantics. [ADR 0026](0026-validation-snapshot-and-fuzz-gates.md#snapshot-schema-lifecycle) governs
draft evidence identity; historical evidence remains bound to its original source and revision.
