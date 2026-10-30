---
status: accepted
date: 2026-07-10
updated: 2026-10-31
authors:
  - Codex
decision_type: architecture
depends_on:
  - docs/adr/0045-canonical-scene-ownership-construction-and-generation.md
  - docs/adr/0009-data-ownership-zero-copy-lifetimes-and-memory-layout.md
  - docs/adr/0047-operation-ledger-evidence-and-resource-accounting.md
  - docs/adr/0030-logical-mesh-primitive-and-attribute-domain.md
  - docs/adr/0031-animation-skin-morph-and-deformer-contract.md
  - docs/adr/0044-retained-physical-storage-ownership-and-account-lineage.md
supersedes:
  - docs/adr/0025-scene-ir-invariants-and-validation-boundaries.md
related:
  - docs/adr/0013-post-process-pipeline-semantics-presets-and-mutation-model.md
  - docs/adr/0036-source-identity-provenance-and-metadata.md
---

# ADR 0032: Postprocess Remap, Edit Transaction, and Derived Data Contract

## Context

Triangulation, vertex joining, mesh splitting, influence compaction, and coordinate conversion can
change several identity domains at once. Mutating vectors in place and validating only at the end
cannot tell callers which IDs survived, whether corner/face attributes followed topology, or which
cached bounds and tangents became stale. It also risks publishing a partial scene after failure.

ADR 0013 owns direct-operation, composition, and preset policy. This ADR is authoritative for
structural mutation, transactionality, remaps, and derived-data validity.

ADR 0047 remains the normative authority for `Attempted`, `Consumed`, live attribution with its
peak, `CommittedCanonicalOutput`, and `PublishedOutcomeEvidence` accounting invariants. Import and
postprocess own their domain ledger semantics, policy, dimensions, phase vocabulary, and evidence.
This ADR owns the postprocess edit meanings for replacement, scratch, and remap reservations; it
cannot refund monotonic work or reinterpret either committed destination. ADR 0047 applies those
destination commitments during preparation. Exact postprocess wrappers privately own the already
qualified but domain-unexposed canonical and outcome-evidence owners; pure handoff only exposes/moves
them into report-facing values and performs no attribution transition.

The runtime-neutral leaf crate `baozi-operation` owns the reusable reservation/ledger state-machine
mechanism together with operation-control and effect-descriptor mechanics. It performs checked state
transitions but owns no import/postprocess policy, dimensions, phase semantics, evidence schema, or
Scene meaning. This ADR supplies the scene-specific read/write, remap, invalidation, loss, and edit
ledger semantics to that shared mechanism rather than defining a second mechanism.

## Decision

`SceneEdit` is the only canonical mutation capability. It starts from a
`ValidatedScene`, shares unchanged storage, and owns all replacement buffers and scratch state. The
input remains a valid generation throughout the operation. Final publication runs central validation,
then atomically reserves exactly one private never-reused generation nonce for the final Scene through
ADR 0045's `ReservedSceneGeneration` as the final fallible pre-freeze identity acquisition. Ordered
per-step report evidence does not consume generation nonces. Preparation attempts to freeze
canonical/remap/report attribution only after that one reservation succeeds and returns exact
`PreparedSuccess` only after the freeze and final control classification complete. Its pure handoff
consumes the one reservation into one fresh collision-free `SceneGenerationId`, constructs the new
`ValidatedScene`, and publishes the edit report/remaps. Branched edits from one input never share an
output generation token.
Failure, resource exhaustion, or cancellation publishes no candidate Scene. The direct operation's
failure value returns the exact original immutable `ValidatedScene`. Verified candidate cleanup and
ordinary failure evidence follow the existing operation protocol; uncertain retained ownership
remains fatal containment rather than a clean fallback.

Concrete executors keep activation, central validation, and terminal publication in separate
private phase functions. Their linear owners still follow this single protocol. Mutually exclusive
failure continuations must not force every successful caller to reserve all their debug stack
temporaries. Caller-owned scratch remains alive through correspondence validation and is released
before ordinary terminal preparation. Ordinary-stack acceptance runs the complete public workflow
in debug and release, including the unpacked-package consumer; isolated operation tests alone do
not establish that composed caller footprint.

Before a direct operation or compiled postprocess plan opens `SceneEdit`, derived-operation start
linearizes against the input Scene account's shared ADR 0044 poison/counter state. Poison-first
returns typed `AccountPoisoned` before step work, attempted/consumed charging, share/refcount/range access, scratch,
remap, diagnostic, or replacement allocation. If start or retain/share admission linearizes first and
another account clone later poisons the account, admitted charge remains coupled, every later
retain/share attempt rejects, and final publication eligibility returns
`FinalizationContainmentError` with no exact prepared report, output generation, remap, or handoff.
Neither plan execution nor `SceneEdit` exposes reset, ignore, reuse, or account replacement.

The edit operation's pre-admitted fallback/evidence startup pair follows ADR 0044 before any exact
postprocess terminal can exist. A startup builder panic returns a start error with no accepted edit
invocation or terminal; successful startup instead preserves the exact baseline required by later
fallback preparation. A later retained candidate/report builder panic returns fatal containment and
never becomes a clean postprocess failure or minimal fallback. The unchanged input generation remains
valid and no report/remap is published. ADR 0044 exclusively owns physical pair admission, side and
pre-attribution containment, attribution coupling, cleanup evidence/precedence, and retained
allocation membership; this ADR owns only the edit-visible input/output and report consequences.

ADR 0013 decides whether a direct operation or compiled postprocess plan requires a canonical write
before opening a `SceneEdit`. `NotRequested` and successfully executed `Unchanged` outcomes do not
publish an edit and therefore preserve the input proof, digest, and `SceneGenerationId`. `Changed`
is the only successful postprocess outcome that publishes; every such handoff still consumes a distinct reserved generation,
including a value-only edit whose entity identities remain logically unchanged. A fresh generation is
evidence of a new immutable state, not a side effect of merely invoking postprocess.

```mermaid
sequenceDiagram
    participant Caller
    participant Input as ValidatedScene G
    participant Edit as SceneEdit
    participant Validator
    Caller->>Input: begin edit
    Input-->>Edit: shared unchanged storage
    Edit->>Edit: build affected replacements and remaps
    Edit->>Validator: generation-free candidate
    alt valid
        Validator->>Edit: reserve one final token; freeze publication
        Edit-->>Caller: pure handoff as ValidatedScene G' + report
    else error or cancellation
        Validator-->>Caller: failure evidence
        Caller->>Input: continue using unchanged G
    end
```

ADR 0013 permits a direct typed operation to derive its concrete plan before replacement construction;
a general dependency compiler is needed only when implemented composition requires one. This ADR
requires every operation to declare entity/domain read sets, write sets, invalidation sets, remap
obligations, and structural loss classes. A concrete closed edit plan may carry those effects
without a generic descriptor registry. `SceneEdit` accepts writes only through those declared
effects and charges replacement/remap storage before constructing it.
User-facing edit methods are typed adapters over the same effect descriptors; there is no raw
mutable collection escape hatch that can omit remap or invalidation obligations.

After each operation constructs its affected replacements, the edit validates that operation's
postconditions, every affected canonical invariant, remap completeness, and derived-data outcome in
a private candidate state. Only a passing candidate is atomically swapped into the transaction's
working state for the next operation to observe. This is not public publication and constructs
neither a public generation nor a `ValidatedScene` proof; final preparation still performs full
central validation for the complete plan. A step failure discards its candidate and prevents all
later steps from running.

In a multi-step edit, each passing immutable private step state uses a transaction-local typed step
path/ordinal, not a generation nonce. Preparation seals those paths into report-branded
`ReportStepStateId` values. A step-state ID is meaningful only inside its owning report: it cannot be compared across reports,
persisted as Scene identity, converted into `SceneGenerationId`, used for Scene lookup, or used as a
validation proof. The report brand plus typed path prevents staged per-step remap endpoints from being
confused while avoiding public identities for states that never had a queryable Scene.

After final central validation, preparation reserves exactly one `ReservedSceneGeneration` for the
final Scene. Exhaustion or collision rejection leaves the generation domain unchanged and precedes
attribution freeze; the postprocess operation preserves the input and classifies ordinary rich
failure with no candidate output generation. Only failure to prepare that rich evidence requires its
minimal fallback. Only after that reservation succeeds may preparation freeze attribution. Exact
`PreparedSuccess` owns the single reservation and staged report. Cancellation, caught unwind, or prepared
drop after that point burns one invisible never-reused gap and publishes no generation, proof, or
remap. A freeze, ledger, or product-graph preparation failure after successful reservation has the
same burn-without-publication result. Final handoff converts the one reservation into the only public
output generation, constructs the only new `ValidatedScene` proof, and atomically exposes the report.
No API exposes a private candidate Scene or a partial generation identity.

### First F Increment: Direct Single-Primitive Triangulation

`triangulate_primitive` consumes an immutable input Scene and targets one generation- and
logical-mesh-qualified `PrimitiveId`. It converts all faces of that `Polygons` primitive into a
`TriangleList`; a triangle list or empty polygon returns the input as `Unchanged`. Other topologies
are rejected in this increment. Repeated caller invocations may address other primitives using
the preceding result's generation; no selection scheduler is required.

Postprocess owns one same-account derived runtime, admitted scratch/origins, geometric preflight
and independent postflight, and its report projection. Core extends the existing closed `SceneEdit`
program and rebuilds the existing typed `GeometryStorage` owner. Unchanged nodes and other eligible
storage are shared through existing retained-owner mechanisms. The current geometry owner is a
whole table, so this increment does not promise primitive-granular sharing. It introduces no second
Scene representation or roundtrip through fresh authoring.

The report identifies `PostProcessOperation::Triangulate { target }` and its input/output
generations directly. The removed empty request compiler, compiled-plan/step wrappers, separate
coordinator ledger, and unused diagnostic/loss placeholders have no role in this edit boundary.
`Unchanged` returns the input owner with real invocation evidence and no remap; failure returns the
original input with the existing typed direct-operation cause and available real terminal evidence.

The replacement keeps Vertex identity/order, primitive and mesh ownership slots, material binding,
and unrelated primitive values. It remaps every present supported Corner/Face attribute through the
actual origin relation, retaining dense/sparse shape, scalar kind and bits, and explicit sparse
entries/defaults. Each affected primitive or attribute retains its own lineage parents; unrelated
owners do not acquire a common fabricated parent set.

Success evidence is one rooted four-member graph: the edit root, Scene-count child, flattened
source-corner origins, and source-face output-triangle prefix offsets. The operation admits capacity
for that complete graph before writing it. Ordinary rich failure still has its actual two-member
root/count payload and absent origin references; a capacity bound does not change that failure
schema or authorize partial successful remaps. Construction and validation work must cover the
actual graph and origin sizes under the shared operation authority.

Before publication, core compares the retained destination indices and attribute values against
the exact source/origin relation and centrally validates the actual candidate and rooted evidence.
The existing joint generation/freeze/handoff pairs the one output generation with that validated
evidence. Describing an intended relation in a report is not a substitute for this comparison.

This is the first operation increment of plan F. Flat/smooth normal generation, explicit
coordinate/unit conversion, corner-to-application-vertex unification, general scheduling, and
cross-invocation replay are not implemented by it. The direct triangulation increment passed its
workspace, extracted-package consumer, native release, and WASI integration checks on 2026-10-30;
the delivery progress record identifies their scope and evidence. This does not complete plan F.

The operation reports published domain-evidence bytes separately from terminal ledger/fallback
record counts. Each commitment uses the dimension with its own declared unit. Frozen live/evidence
counters describe the terminal operation boundary; they are not dynamic measurements of later
Scene or report teardown.

### Direct Normal Generation

`generate_normals` targets one `TriangleList` primitive and produces a dense F32, three-lane
Corner `Normal` attribute. It borrows dense F32 Vertex positions directly. Its oriented face cross
is `(b - a) × (c - a)` in the stored numeric basis; it does not infer a front-facing direction from
unknown Scene conventions. Smooth mode shares area-weighted normals by Vertex identity and
optional dense U32 Face smoothing groups; group zero keeps each face hard. Coincident positions
are not welded, and Corner UV seams do not split shared Vertex identities. Degenerate faces,
non-finite results, and cancelling smoothing sums reject the operation rather than inventing a
fallback normal. Unsupported position or smoothing-group encodings return a typed error.
Face crosses expand each determinant into six products of original F32 coordinates. Those
products are exact in F64 throughout the finite F32 exponent range; a fixed six-entry TwoSum
expansion retains cancellation residuals before estimating each component. This avoids losing
direction when rounded edge differences erase a small coordinate. Normalization scales before
computing length, and smoothing uses compensated sums of unnormalized face crosses. Determinant
products use a fixed Vertex-ID order, restoring the input winding by an exact sign change, so
cyclic permutations have identical estimates and opposite faces cancel bit-for-bit.
Canonical `(semantic, domain)` keys may legally provide multiple authored Normal or Tangent fields.
When generation would need to select, merge, or remove multiple such fields, this increment returns
an explicit capability error before replacement. It does not silently select a domain or merge
owner-local metadata. Typed attribute reads require both semantic and domain.

`NormalOptions` makes source-field changes explicit. The default `ExistingNormals::Preserve`
returns the exact input unchanged when a Normal exists. `Replace` authorizes replacement. If a
Tangent would be invalidated, the default rejects; `TangentHandling::Remove` authorizes deleting
that attribute and only its owner-local metadata. No tangent generation is implied. Vertex,
Corner, and Face identities, indices, other attributes, and unrelated primitive values remain
unchanged, including dense/sparse encoding and scalar bits.

The concrete attribute mapping inserts a new Normal in canonical semantic order, maps a replacement
to the old Normal, removes a deleted Tangent, and reindexes all later Scene-wide attribute owners.
Profile owners come from the final typed Scene layout. The generated Normal's lineage parents are
the deduplicated union of the target Primitive, Position, replaced Normal, and, for Smooth, consulted
SmoothingGroup attachments. Other owners retain their own parents. Metadata descriptors follow the
same mapping. Pure reindexing shares immutable payload pools; actual deletion compacts surviving
pools under separately admitted storage and work. Removed metadata entry counts describe real
deleted descriptors.

Postprocess retains generated values in charged scratch until core compares every candidate value
with that exact borrowed replacement and centrally validates the candidate. It then drops scratch
before final preparation. The operation reuses triangulation's existing geometry transaction,
generation reservation, and publication protocol. Its evidence graph has only the edit root and
Scene-count child; no empty remap sequences are constructed. The core handoff returns a closed enum
with operation-specific evidence. Each variant owns its actual
rooted store and exposes only the applicable facts; a Normal result cannot be used as a remap.
Public `NormalGenerationReport` owns the latter directly, without duplicating decoded effects.

### Direct Whole-Scene Spatial Conversion

`convert_scene_space` consumes one immutable Scene and a validated complete `TargetSpace`, with
optional explicit complete source assumptions. Resolution borrows the exact Scene used to derive
the plan. No transaction API accepts an independently replaceable input alongside that resolved
value. Its closed program extends the existing transaction, central validation, fallback,
generation, and publication path rather than defining a second edit protocol.

The conversion transports every F32 Position, Normal, and Tangent across every authored
domain, including sparse defaults and explicit entries. ADR 0008 supplies the arithmetic. Dense
and sparse Corner attributes follow triangle index-lane reversal; sparse indices remain sorted
without input-sized sorting scratch. A nonindexed triangle gains explicit indices so Vertex
order and identity remain unchanged. Vertex, Face, Primitive, logical-mesh, and instance order
remain unchanged. The report states the complete lane relation `[0, 2, 1]` when reversal is
required; it does not expose an unbounded primitive lookup masquerading as a checked remap.
Points and line streams retain their Corner order. Nonempty Polygons, TriangleStrip, and
TriangleFan require a separately implemented lowering before a conversion needing reversal.

Node conversion preserves authored Matrix/TRS form, hierarchy, names, and attachments. The new
spatial owner changes the target's five facts and converts complete camera/light definitions;
UV origin and tangent basis/encoding remain preserved. Declaration-only and winding-only edits share
unaffected typed tables. Scene provenance records the resolved conversion. Changed Node,
Primitive, Attribute, Skin, MorphDelta, AnimationChannel, Camera, Light, Material, and TextureBinding
owners retain their own attachments together with the input Scene's attachments as operation
parents. An appearance owner is selected only when its own converted value changes bits; copying
an unchanged row into a rebuilt table does not change that row's provenance. Metadata descriptors
and immutable payloads retain their owners and values. Central validation compares the actual
replacement tables with the exact borrowed source plan, including preserved values, and validates
all published conversion facts.

One closed `SceneSpacePlan` constructs node, geometry, deformation, animation, spatial, and
appearance plans against the same immutable `SceneTables` and conversion. Its private fields
prevent replacement of an individual domain plan; binding it to another Scene root fails by
pointer identity before profile planning even when both Scenes have the same generation.
Preflight covers every numeric payload before retained output admission. Its fixed correspondence
and rebuilt-name costs are reused by execution and validation rather than reconstructed later.
Inverse binds and skin corrections use matrix conjugation, morph deltas follow their semantic
linear map, and animation values and cubic derivatives follow their target map under ADR 0008.
Quaternion signs, zero/nonunit derivatives, names, f64 times/rates, source order, sparse presence,
influence rows, and morph weights are preserved.

Unchanged deformation and animation bits share their complete retained owners. Profile selection
exhausts each relevant skin/morph/channel payload, including unchanged curve streams, without
stopping after the first difference. Lineage and state construction each admit their own complete
selection pass. Winding-only and declaration-only edits bypass these selection scans while still
checking actual shared-domain correspondence.

Camera/light intrinsic reference axes compose as `B' = Q B`, including reflections. Camera
clipping distances and orthographic half-extents, and finite light ranges, scale with units.
Node counts, hierarchy, attachments, and logical references remain unchanged, including through
repeated conversions. FOV/aspect, cone angles, canonical tint, and photometric intensity retain
their bits. Rounded camera planes must still satisfy their strict ordering. The same source-bound
spatial plan preflights and checks actual correspondence; no separate declaration-only validator
can approve an unconverted definition.

Supplemental material thickness and signed Displacement binding strength are Scene-unit lengths.
For positive unit ratio `s`, each becomes `f32(f64(value) * s)` through the checked length map.
Displacement does not acquire a handedness sign. The conversion preserves signed zero and rejects
overflow or nonzero underflow to zero with the affected material/binding location. All other factor
bits and optional groups, alpha cutoffs, non-Displacement binding strengths, UV indices and
transforms, names, source keys, ordering, and identity links remain unchanged. Thickness binding strength stays one;
normal-map strengths remain dimensionless. The canonical binding model admits only a direct
`uv_set: u16`; it contains no spatial coordinate-generation enum or implied evaluation contract.

Appearance preflight visits every binding and material, including unchanged values, before any
retained replacement. Changed bits require a new `AppearanceRows` owner with the same five row
counts; unchanged bits share that owner. Both paths share the exact `AppearancePayloads` owner for
the two byte pools. Correspondence first checks payload-owner pointer identity, then compares every
row with its expected converted or preserved value. Byte equality in a newly copied payload owner
does not satisfy the plan. Row construction and correspondence each admit one action per row in
the five tables, with no image/text copy or byte scan. The appearance contribution to rebuilt
name-byte admission is zero. This does not remove the pools from the candidate's canonical-byte
footprint or bypass central semantic validation. Fixed-size Material/TextureBinding provenance
selection fits its existing profile-row action and adds no payload traversal or selection cache.

The remaining capability boundary concerns unknown spatial semantics. Custom attributes reject
numeric conversion because their spatial meaning is unknown; a winding-only conversion can retain
and permute opaque Corner tuples.
Reflection with a present Tangent requires `SignInW`. Each rejection retains its object location
and typed reason. No rejected domain is silently inherited under a new target declaration.

The public report directly owns `SceneSpaceConversionEvidence`, whose input/output generations,
source/target facts, assumption mask, and winding effect are backed by the validated evidence
root. It creates no empty Corner/Face origin sequences. The common root schema is revision 4;
non-conversion evidence has zeroed conversion fields. Runtime success/failure capacity and work
requirements cover the real schema. No independently cached decoded report facts are authoritative.

Complete spatial conversion for the remaining domains and application vertex unification remain
open F work. This conversion increment does not complete plan F.

### Remap Contract

Every successful structural operation records private remaps for each affected entity and
attribute domain. A direct operation owns its actual input-to-final relation without a synthetic
report step, intermediate Scene identity, or general composition engine. For a multi-step edit,
successful final handoff publishes ordered per-step remaps
with report-scoped `ReportStepStateId` endpoints and one composed generation-qualified input-to-final
remap. It does not relabel intermediate step states as Scene generations. A value-only edit marks
the corresponding identities unchanged while still reporting invalidation. An entity remap is a
relation, not a one-to-one map:

- one old ID may map to zero IDs when deleted
- one old ID may map to many ordered IDs when split
- many old IDs may map to one ID when merged
- a generated ID has no old ID but carries derivation provenance

Domain remaps cover vertex, corner, face, and instance elements. In ordered per-step evidence, an
endpoint is a sealed typed tuple of `(ReportStepStateId, complete owner-local typed identity)`; the
primitive/logical-mesh ownership chain remains explicit, but no component is a public Scene handle.
In the composed input-to-final remap, Vertex, Corner, and Face endpoints are exactly
`(PrimitiveId, domain_index)`, where `PrimitiveId` contains the public Scene generation,
logical-mesh owner, and primitive-local index. A composed Instance endpoint is exactly the owning
generation-qualified `MeshInstanceId`; the domain has one element for that instance and does not add
an unqualified numeric component. These shapes provide both old-to-new relations for external
references and deterministic new-to-old origins for copying attributes and diagnostics.
Interpolated/generated elements include a typed derivation record rather than a fake source index.
Identity remaps for unaffected kinds are omitted only when prepared report staging explicitly marks
that kind unchanged between its input and final output state; the marking becomes public only with
final handoff.

Every ordered per-step remap envelope names exact `from_step_state` and `to_step_state` values from
the same report brand. Those endpoints are report evidence only. The one composed remap envelope names
the exact public `from_generation` and `to_generation`. Its Scene-wide entity endpoints contain typed
dense IDs; primitive and morph-target endpoints also contain their logical-mesh owner. Vertex,
corner, and face endpoints additionally identify their owning primitive through `PrimitiveId`. A
numeric index without the applicable report step state or public generation and complete required
owner chain is not a remap endpoint.

For direct triangulation, the changed report owns `TriangulationRemap`, backed by core's published
`SceneTriangulationEvidence`. The retained root owns the actual origin leaves, independently of both
input and output Scene lifetimes. Borrowed relation views borrow that evidence owner, not either
Scene or temporary kernel scratch. Dropping either Scene, or both Scenes, must leave the retained
relation queryable; dropping the report/evidence releases its physical ownership through ADR 0044.

Corner origins contain one input corner ordinal per output corner in triangle order, including
repeated occurrences. Face offsets have one entry per source face plus the final endpoint and
identify each face's contiguous output-triangle range. Complete `PrimitiveId` endpoints bind these
ordinals to the input and final output. Foreign-generation/owner queries and out-of-range indices
are rejected. The report's concrete operation identifies this relation directly; no generic step-ID
allocator or inverse-map allocation is required.

Replay evidence is invocation-local. An `Unchanged` replay publishes no generation, remap,
provenance, derived-data producer, or committed-output change; its scene digest, proof, and producer
records remain identical even when attempted work is non-zero. A replay that changes state creates a
fresh generation and fresh remaps from the immediately preceding input generation. Earlier remaps
are never copied under new endpoints; a cross-invocation relation exists only through explicit
composition.

When an executor implements cross-invocation replay, every executed step emits an invocation-local
application record containing the full normalized operation identity (operation ID, semantic version,
and normalized parameter digest), stamp-schema version, pre-write semantic-input stamp,
post-write semantic-input stamp, and postcondition
version/result. The post-write stamp is recomputed over the same declared read set on the step's
validated output; it equals the pre-write stamp for `Unchanged`. Cross-invocation replay compares the
current pre-invocation stamp with the prior record's post-write stamp. The pre-write stamp records
what the earlier execution consumed and is audit evidence, not skip authority.

Prior-record lookup is conservative and deterministic. It scans records attached to extant owners
in the descriptor's declared provenance-carrying write domains in canonical owner order; detached
reports and deleted owners are not lookup authority. A candidate must match the complete normalized
operation identity, stamp-schema version, postcondition version, and current pre-invocation stamp to
recorded post-write stamp. Matching records may collapse only when their normalized records are
identical. No candidate, conflicting matching candidates, or an unavailable former owner forces
reevaluation; generation order, timestamps, insertion order, and a notion of "latest" never select a
winner. Even one matching record is only replay evidence: the executor recomputes the current
postcondition before returning `Unchanged`.

`Unchanged` retains this record only in the report. For a changed invocation, the edit report retains
it and only actually changed or generated owners attach it to provenance; this is the comparable
history carrier for non-derived structural and value writes. Recomputed derived values receive the
same full producer identity plus their post-write input stamp; preserved derived values retain their
producer while following the generation-rebinding rule below.

The direct first F operation reevaluates the live target instead of consulting prior application
records. A triangle list or empty polygon has a live unchanged postcondition. History lookup,
application-stamp machinery, and generic multi-step remap composition are not prerequisites for
that behavior.

A multi-step edit report contains both report-state-qualified ordered per-step remaps and one
generation-qualified composed input-to-final remap for every affected kind/domain. Composition is
relational in execution order: deletion terminates an old origin, split order remains stable, merges
retain all ordered old origins, and generated elements remain originless while carrying typed
derivation provenance. An explicitly unchanged kind composes as the identity relation across the
input and final generation tokens. Private failed-step or prepared-drop remaps are never included in
either published form.

An operation that writes topology must either remap every present affected-domain attribute in the
same transaction or fail with an unsupported-domain error. It cannot drop a corner or face stream
merely because the algorithm originally handled only vertex attributes.

### Derived Data

Published optional derived fields use one state model:

```text
Derived<T> =
    Absent { reason }
  | Valid { value, generation, input_stamp, producer }
```

`generation` equals the owning `ValidatedScene` generation. `input_stamp` is a deterministic digest
of the field's declared semantic inputs and dependency-schema version; `producer` distinguishes a
source-preserved value from the complete normalized operation identity including semantic version
and parameter digest. `Absent` contains no stale value.
An edit may use a private dirty/invalidated staging marker, but final preparation must turn it into
  `Absent` or a new `Valid` staged against the one reserved final output generation; `Invalidated` is an edit-
report outcome, not a third publishable value-bearing state. When an unchanged derived value is
preserved, preparation verifies its input stamp and stages the small state wrapper for output-
generation rebinding; pure handoff exposes that binding, while immutable value storage may remain
shared.

The bounds owners are fixed: each primitive may own primitive-local bounds, each logical mesh may own
logical-mesh-local bounds under an explicit morph-bounds policy, and Scene may own scene-space bounds
over node transforms and mesh instances. No owner reuses another owner's cache as if the spaces were
equal. Derived fields carry validity tied to scene generation and declared inputs. The minimum
dependency rules are:

| Derived data | Invalidated by |
| --- | --- |
| primitive bounds | positions, morph bounds policy, topology/vertex deletion |
| logical-mesh bounds | primitive bounds, primitive membership, logical-mesh morph bounds policy |
| normals | positions, topology, smoothing/crease data |
| tangents | positions, topology, normals, selected UV stream, tangent-basis convention |
| scene bounds | logical-mesh/primitive bounds, node transforms, mesh instances, skin/morph bounds policy |
| renderer influence palette | skin palette, influences, compaction options |

Operations may preserve, invalidate, or recompute a derived field. Preservation requires a proof
declared by that operation; otherwise a matching write set invalidates it. A validated output may
contain an explicitly absent optional cache, but never a stale value marked valid. Normals and
tangents are canonical attributes rather than hidden caches: when their dependencies change, an
operation must prove preservation, remap or recompute them, or remove them with structured
invalidation/loss evidence before final preparation. A validated scene never exposes an attribute marked
invalid for ordinary reads.

The first triangulation operation declares source-field preservation. Authored normals and tangents
remain canonical attributes: Vertex values keep their identity/order, and Corner/Face values are
copied through their exact origins with their supported encoding and scalar bits. This is not
normal/tangent generation, normalization, or a claim that each triangle's geometric normal equals an
authored value. A future producer-stamped derived field whose dependency contract cannot prove this
preservation must be rejected, recomputed, or explicitly removed with evidence before publication.
Affected bounds are reported as invalidated; the absence of a value-bearing bounds cache is not a
new valid cache or an invented computed value.

Postprocess success reports its `Unchanged` or `Changed` outcome, ordered steps, diagnostics, losses,
resource statistics, entity remaps, domain remaps, and derived-data outcomes. `Unchanged` has no
committed output or remaps and names the preserved input generation. Resource semantics are owned by
ADR 0047: attempted and consumed work remain monotonic on success or failure; peak-live records the
transaction high-water; final preparation applies canonical and report/remap destination commitments
with operation-live attribution at zero; and pure handoff only exposes/moves the already qualified
owners as committed/published evidence. Failure evidence includes the last completed step plus attempted, consumed,
peak-live, zero newly committed canonical output, and bounded published outcome evidence without
presenting private candidate remaps as committed. A failure may identify the already published input
generation that remains valid, but it exposes no candidate output generation or generation-qualified
candidate remap.

## Considered Options

### Option A: In-place mutation with validation at pipeline end

This minimizes temporary storage, but rollback is unreliable, IDs become ambiguous, and a failed
step can poison the caller's only scene. Rejected.

### Option B: Clone the entire scene before every step

This gives simple rollback but violates peak-live goals and still leaves remap/invalidation rules
implicit. Rejected.

### Option C: Affected-set transaction with explicit remaps and invalidation

This preserves the input proof, bounds temporary storage, and makes structural effects consumable by
tools. Chosen.

## Consequences

- All postprocess implementations and user edits use the same commit boundary.
- Algorithms must be domain-complete or reject unsupported present data before mutation.
- Dense IDs never silently survive a generation change; provenance persists separately.
- Reservation rejection or exhaustion consumes no nonce. Any later freeze/ledger/product-graph
  preparation failure, cancellation, caught unwind, or prepared drop after the one successful reservation
  may burn one private generation nonce gap, but never publishes or reuses it.
- A retained-builder panic is a pre-prepared containment result, not a caught-unwind fallback; it
  preserves the input Scene while retaining the exact affected physical/accounting owners.
- Some operations need extra remap storage, which is budgeted as required output evidence rather
  than optional debug data.

## Risks and Mitigations

| Risk | Severity | Mitigation |
| --- | --- | --- |
| Copy-on-write still exceeds peak-live budget | High | Reserve affected replacement and remap capacities before execution and release step scratch promptly. |
| Remap relations become too large | Medium | Use compact ranges/CSR representations while preserving complete semantics and limits. |
| Operation forgets a derived dependency | High | Central dependency registry plus mutation tests that compare declared write sets to cache outcomes. |
| Failure reports imply partial commit | High | Separate attempted step evidence from committed remaps; a failure may retain its public input identity but exposes no candidate output generation. |
| Replay fabricates a new generation or provenance without a write | High | Make unchanged replay invocation-local and assert bit-identical scene/proof/producer state. |
| Generation exhaustion occurs after attribution freeze | Critical | Reserve exactly one final-Scene nonce as the final fallible pre-freeze identity acquisition; intermediate step states use report-scoped typed paths and never consume nonces. |
| Prepared drop exposes a private remap endpoint or reuses its nonce | Critical | Keep the one reservation and staged remaps private and linear; drop burns the one gap, while only pure final handoff converts the final identity and publishes the report. |
| Postprocess starts or publishes through a poisoned account clone | Critical | derived start and final eligibility use ADR 0044's shared linearization state; poison-first has zero work/evidence/refcount/allocation effects, start/retain/share-admission-first retains charge but returns fatal containment before handoff, and no reset/reuse bypass exists |
| A retained edit/evidence builder panic is downgraded to an ordinary failure | Critical | startup panic returns a start error with side containment; later candidate/report panic returns fatal containment; fallback requires verified candidate cleanup and intact `StartupBaselineOwner` |
| A report step ID is mistaken for persistent Scene identity | Critical | Brand every typed step-state path to its owning report and expose no conversion, cross-report comparison, Scene lookup, persistence, or proof API. |

## Success Metrics

| Metric | Target | Measurement |
| --- | --- | --- |
| Atomicity | Every injected failure leaves input generation, digest, and proof unchanged and publishes zero remaps | failure/cancellation/resource matrix |
| Reserved generation atomicity | exactly one final reservation is attempted; exhaustion consumes no nonce and fails before freeze; any later freeze/ledger/product-graph preparation failure, cancellation/caught-unwind/prepared drop may burn that one gap; no failed path publishes a generation/proof/remap | reservation and publication typestate probes |
| Poisoned edit containment | all account/Scene clones reject new changed operations after poison; a pre-admitted invocation cannot publish when poison wins final eligibility | two-order start/retain/share/finalization barriers, work/evidence/refcount/allocator deltas, account snapshots, and no-bypass scans |
| Retained failure isolation | failed startup accepts no edit invocation; a later retained builder panic publishes no fallback, generation, proof, report, or remap; the input remains unchanged | postprocess integration against ADR 0044 terminal outcomes and input-preservation probes |
| Remap completeness | A direct operation owns its complete input/final relation; multi-step remaps additionally cover exact same-report step-state endpoints and compose to exact input/final generations | direct Corner/Face relation and split/merge/delete/generate composition tests |
| Operation-local complexity | Private candidate validation and remap composition are `O(affected entities + affected edges + affected cells + remap cells)` | deterministic per-step scale counters |
| Publication validation complexity | Final central validation is `O(total graph edges + total entities + total references + total payload cells)` under ADR 0045 | deterministic full-validator work counters |

## Acceptance Criteria

- Injected failure, cancellation, and budget exhaustion preserve the input generation and full-data
  digest. Direct-operation failures return that original immutable Scene, including startup failure
  and containment, without exposing candidate remaps.
- Direct triangulation targets exactly one qualified polygon primitive. Triangle lists and empty
  polygons return `Unchanged` without new generation, lineage, or fabricated remaps.
- Published Corner/Face origins match the actual retained destination and remain queryable after
  dropping the input and output Scenes in either order; final evidence drop releases its storage.
  Relation queries reject foreign-generation/owner endpoints and out-of-range indices.
- Poisoned-account tests force both linearization orders across sibling Scene/account clones.
  Poison-first changed-operation start returns typed `AccountPoisoned` with zero step, ledger,
  share/refcount/range, scratch/remap/diagnostic, and allocator effects. Start/retain-first keeps its
  charge, rejects later retain/share, and final preparation returns `FinalizationContainmentError`
  with no exact wrapper, generation, proof, derived binding, report, remap, or handoff. Every clone's
  account snapshot exposes the same poison/count/evidence; source/API gates find no reset, ignore,
  reuse, or account-replacement path.
- Retained failure integration tests prove startup failure returns no accepted invocation or
  postprocess terminal, while successful startup preserves the baseline required by later terminal
  downgrade. A later candidate/report builder panic publishes no failure/fallback/report/remap. ADR
  0044's physical and attribution tests own the exact containment mechanics. The input generation,
  proof, and digest remain unchanged throughout.
- Exact/exhausted generation-domain tests prove exactly one final reservation is attempted, atomic
  reservation failure consumes no nonce, and the failure precedes attribution freeze. Separate later-
  preparation-failure, cancellation, caught-unwind, and prepared-drop tests prove any path after that one
  successful reservation may burn one never-reused gap, while
  no failed path publishes a `SceneGenerationId`, `ValidatedScene`, derived-data binding, or remap.
- An invalid affected replacement fails private step validation before any later operation can read
  it, while the original validated generation remains unchanged.
- Triangulation remaps face and corner attributes; vertex joining remaps vertex attributes and skin
  influences; split/merge publishes one-to-many/many-to-one entity relations.
- Every operation with present unsupported domains fails before allocating its output replacement.
- Coordinate/topology changes prove source-field preservation, invalidate, or recompute affected
  dependent values. Direct triangulation preserves authored normals/tangents through their exact
  domain origins and reports bounds invalidation; it does not generate replacement normals.
- Published derived fields are either `Absent` or generation/input-stamped `Valid`; no private dirty
  marker or stale normal/tangent survives commit.
- Multi-step reports compose their input-to-final relation exactly, and sibling commits from one
  input use different output generation endpoints.
- In multi-step edits, private step/candidate remaps refer only to transaction-local typed paths.
  Final handoff publishes ordered remaps with owning-report `ReportStepStateId` endpoints and one
  generation-qualified composed input-to-final remap. Step-state API tests reject Scene lookup, persistence, cross-report
  comparison/conversion, and proof construction; no intermediate state receives a generation.
- A direct or compiled plan cannot construct replacements unless its concrete operation effects
  cover writes, remaps, invalidation, preservation, and structural loss obligations; no generic
  scheduler or descriptor registry is needed for the one-operation case.
- In a history-based executor, a second state-idempotent invocation with equal operation identity, a
  current pre-invocation stamp matching the prior record's post-write stamp, and a satisfied postcondition is `Unchanged` with
  identical digest, proof, generation, provenance, and derived producers; changing parameters,
  semantic version, or declared input reevaluates the operation. Tests include an operation that
  changes its own read set so replay cannot accidentally compare against its prior pre-write stamp.
- History-based prior-record lookup fixtures prove canonical traversal, equivalent-record collapse, and
  reevaluation after carrier deletion, missing evidence, or conflicting matching records; detached
  reports and recency never authorize a skip.
- Intentionally repeatable operations are not suppressed by prior history; when they change state,
  their remaps start at the immediately preceding input generation rather than relabeling old remaps.
- External code cannot publish a candidate scene or mark derived data valid without final validation,
  generation reservation, preparation freeze, and handoff.

## Amendment History

### 2026-10-31: Convert appearance lengths with independently shared payloads

- Previous rule: unit changes rejected nonzero thickness or displacement, and a complete appearance
  owner coupled numeric row replacement to encoded-image and text ownership. Non-UV coordinate
  variants had no defined generation/evaluation contract.
- New rule: the same source-bound `SceneSpacePlan` converts thickness and signed displacement,
  rebuilds only changed appearance rows, and requires exact payload-owner sharing. Material and
  TextureBinding lineage follows actual changed bits. Bindings use direct bounded UV indices.
- Compatibility impact: the coordinate-source enum and its error/getter are removed without
  aliases. The UV snapshot framing is unchanged. Public authoring/read callers use `u16` directly.
- Required review: checked length arithmetic and preserved bits/groups, exact source binding,
  payload-owner identity, independently admitted row/pool lifetimes, and canonical versus physical
  accounting. This amendment records the contract, not execution or release qualification.
- Rationale: changing a material length does not change image or text payloads and must not copy or
  charge those immutable allocations again.

### 2026-10-30: Remove empty pipeline and synthetic report steps

- Previous rule: the direct operation's report reused a compiled-plan/step facade maintained beside
  the empty request coordinator and its separate resource/error model.
- New rule: one concrete operation descriptor names the generation-qualified target. The existing
  retained runtime, central publication, actual origin owner, and input-preserving failure boundary
  supply all real operation evidence.
- Compatibility impact: empty pipeline APIs, synthetic step outcomes, unused diagnostic/loss
  placeholders, and the legacy coordinator ledger/error hierarchy are removed without adapters.
- Required review: direct-operation generation binding, cancellation/deadline ownership, actual
  remap and resource lifetime, thread transfer, and tracing result equivalence.
- Rationale: direct edit evidence should describe the operation that actually executed.

### 2026-10-30: Qualify direct triangulation and retained origin evidence

- Previous rule: edit/remap text assumed general compiled execution, replay records, and multi-step
  composition before any concrete postprocess operation could land.
- New rule: a direct single-primitive operation uses the existing closed `SceneEdit` and central
  publication boundary. Its one report-owned relation is also its composed input-to-final relation,
  backed by the actual rooted origin storage independently of either Scene lifetime.
- Compatibility impact: a pre-release clarification permits the first F increment without general
  scheduling. Failures return the original immutable Scene, and authored normals/tangents are
  preserved rather than generated. Space conversion and vertex unification remain unimplemented.
- Required review: actual candidate correspondence, source-field preservation, four-member success
  versus two-member ordinary failure evidence, independent relation lifetime, endpoint rejection,
  and joint generation/freeze/handoff.
- Rationale: direct geometry work needs complete domain and ownership semantics, while multi-step
  scheduling and composition should follow demonstrated requirements.

### 2026-07-27: Follow retained physical-ownership authority supersession

- Affected headings: frontmatter `depends_on`, edit startup/finalization, containment risks, metrics,
  and acceptance criteria.
- Previous rule: current edit and postprocess text cited ADR 0042, which combined durable retained
  ownership invariants with U4-specific implementation and measurement details.
- New rule: ADR 0044 is the sole retained physical-ownership and lineage authority. Edit
  transaction, remap, invalidation, and input-preservation semantics are unchanged. Older amendment
  entries preserve the ADR identifier in effect when they were recorded and do not restore ADR 0042
  authority.
- Compatibility impact: none; this is an atomic authority-navigation update.
- Required review: reciprocal supersession, Accepted dependency graph, current-text reference scan,
  and direct ADR review.
- Rationale: postprocess should consume durable physical outcomes without freezing the U4 retained
  store's temporary construction vocabulary.

### 2026-07-20: Reuse operation destination commitment and owner exposure

- Affected headings: `Context`, `Decision`, and postprocess outcome semantics.
- Previous rule: this ADR repeated private prepared-attribution types and described handoff as their
  accounting reclassification.
- New rule: ADR 0022 alone applies canonical and outcome-evidence destination commitments during
  preparation. Exact postprocess wrappers privately own the already qualified but domain-unexposed
  Scene/report/remap owners. Handoff only exposes/moves those owners while publishing the one final
  generation and report; it performs no attribution transition.
- Compatibility impact: private pre-release transaction vocabulary changes; remap, generation, and
  derived-data semantics do not.
- Required review: postprocess-to-operation typestate mapping, prepared-drop preservation of the
  input Scene, pure-handoff counters, and source gates rejecting duplicate attribution typestates.
- Rationale: SceneEdit owns semantic replacement and remap truth, while operation owns accounting
  commitment. Reusing that seam prevents a second ledger state machine inside postprocess.

### 2026-07-19: Correct the storage-to-edit dependency direction

- Affected headings: frontmatter dependency metadata and implementation status.
- Previous rule: ADR 0009 depended on this higher-level postprocess contract while this ADR treated
  storage policy only as related navigation, creating a cycle through ADR 0042.
- New rule: postprocess normatively consumes ADR 0009's lower-level storage representation and ADR
  0042's retained physical-ownership protocol. Neither lower-level authority depends on postprocess
  semantics.
- Compatibility impact: none; this corrects architectural authority metadata before release.
- Required review: ADR dependency-cycle gate and postprocess/storage ownership review.
- Rationale: `SceneEdit` is a consumer of storage and retained-publication primitives, not their
  dependency root.

### 2026-07-19: Supersede retained-builder rollback with startup containment

- Affected headings: `Decision`, `Consequences`, `Risks and Mitigations`, `Success Metrics`, and
  `Acceptance Criteria`.
- Previous rule: the earlier 2026-07-19 amendment delegated caught-unwind cleanup but still permitted
  a retained-builder unwind to be read as a verified rollback/fallback path.
- New rule: failed retained edit startup accepts no invocation or terminal, while successful startup
  supplies the baseline needed by later downgrade. A later retained-builder panic publishes no
  postprocess fallback, report, generation, proof, or remap, and the original input Scene remains
  valid. ADR 0042 exclusively owns the physical admission, containment, cleanup evidence,
  attribution coupling, membership, and release protocol below these postprocess consequences.
- Compatibility impact: pre-release postprocess failure and containment typestates change; no edit or
  remap compatibility adapter is retained.
- Required review: startup-versus-later builder mapping, input preservation, zero published remaps,
  prepared boundary, ADR 0042 integration, and ADR review.
- Rationale: transactional edit atomicity protects the input Scene, but it cannot turn an uncertain
  retained ownership graph into a clean published failure.

### 2026-07-19: Close poisoned-edit and in-process containment

- Affected headings: `Decision`, publication lifecycle, `Consequences`, `Risks and Mitigations`, `Success Metrics`, and
  `Acceptance Criteria`.
- Previous rule: “unwind” could be read as including abort, double panic, exit, or OOM, and a changed
  postprocess invocation did not define derived-start/final-publication behavior when another clone
  poisoned the input Scene's physical account.
- New rule: generation-gap and in-process rollback claims apply to a caught unwind. ADRs 0020, 0022,
  and 0042 separately govern verified-cleanup fallback, fatal containment when release is uncertain,
  and process isolation for failures that cannot unwind. Derived start and final publication
  eligibility use ADR 0042's shared poison/counter linearization state: poison-first rejects before
  any edit/evidence/refcount/allocation effect, while start/retain/share-admission-first retains charge but cannot
  prepare or hand off after poison. Every clone shares the latch/count/evidence and there is no
  reset/reuse/account-replacement path.
- Compatibility impact: no remap or generation shape changes; the pre-release start and failure
  contracts gain typed `AccountPoisoned`/fatal-containment behavior.
- Required review: caught-unwind rollback/gap probes, fatal-containment integration, two-order
  changed-operation start/retain/finalization barriers, clone/reuse/zero-side-effect/no-bypass probes,
  process-isolated abort fixtures, and direct ADR review.
- Rationale: this ADR owns edit/remap semantics, not a stronger in-process containment guarantee than
  the shared operation/import authorities can provide. A poisoned physical lineage cannot safely
  authorize another edit or publish newly derived state.

### 2026-07-18: Reserve one final edit generation and keep step identity report-scoped

- Affected headings: accounting authority, `Decision`, sequence diagram, private step validation,
  `Remap Contract`, `Derived Data`, consequences, risks, success metrics, and acceptance criteria.
- Previous rule: final edit commit and each passing private step were described as directly assigning
  a `SceneGenerationId`, leaving fallible nonce exhaustion and prepared-drop/remap visibility
  undefined.
- New rule: private candidates use transaction-local typed step paths. After final validation,
  preparation reserves exactly one linear `ReservedSceneGeneration` for the final Scene as the final
  fallible pre-freeze identity acquisition. Rejection or exhaustion consumes no nonce; any later
  freeze/ledger/product-graph preparation failure, cancellation, caught unwind, or prepared drop burns at
  most that one invisible never-reused gap. Pure final handoff converts the reservation into the sole
  output generation, constructs `ValidatedScene`, publishes ordered per-step remaps with report-
  scoped `ReportStepStateId` endpoints, and publishes one generation-qualified composed input-to-
  final remap. Intermediate states never receive public Scene identity. `Unchanged` reserves and
  publishes nothing, and failure exposes no candidate output generation.
- Compatibility impact: pre-release edit/remap typestates and tests change; no stable public edit API
  or serialized generation representation is preserved, and no failed candidate endpoint remains
  observable.
- Required review: one-token generation exhaustion/never-reuse and sibling concurrency tests,
  report-branded multi-step remap composition, step-state lookup/persistence/cross-report rejection,
  cancellation/caught-unwind/prepared-drop probes, derived-data rebinding,
  postprocess publication accounting, compile-fail authority gates, and direct ADR review.
- Rationale: remap identity is truthful only when every endpoint is published with the same atomic
  proof boundary, and a fallible generation allocator cannot remain inside a pure handoff.
