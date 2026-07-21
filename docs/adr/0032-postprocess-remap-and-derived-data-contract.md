---
status: accepted
date: 2026-07-10
updated: 2026-07-20
authors:
  - Codex
decision_type: architecture
depends_on:
  - docs/adr/0003-core-scene-ir-and-material-model.md
  - docs/adr/0009-data-ownership-zero-copy-lifetimes-and-memory-layout.md
  - docs/adr/0022-per-import-resource-ledger-and-budget-accounting.md
  - docs/adr/0030-logical-mesh-primitive-and-attribute-domain.md
  - docs/adr/0031-animation-skin-morph-and-deformer-contract.md
  - docs/adr/0042-shared-canonical-storage-accounts-and-generation-lineage.md
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

ADR 0013 owns pipeline and preset policy. This ADR is authoritative for structural mutation,
transactionality, remaps, and derived-data validity.

ADR 0022 remains the normative authority for `Attempted`, `Consumed`, live attribution with its
peak, `CommittedCanonicalOutput`, and `PublishedOutcomeEvidence` accounting invariants. Import and
postprocess own their domain ledger semantics, policy, dimensions, phase vocabulary, and evidence.
This ADR owns the postprocess edit meanings for replacement, scratch, and remap reservations; it
cannot refund monotonic work or reinterpret either committed destination. ADR 0022 applies those
destination commitments during preparation. Exact postprocess wrappers privately own the already
qualified but domain-unexposed canonical and outcome-evidence owners; pure handoff only exposes/moves
them into report-facing values and performs no attribution transition.

The runtime-neutral leaf crate `baozi-operation` owns the reusable reservation/ledger state-machine
mechanism together with operation-control and effect-descriptor mechanics. It performs checked state
transitions but owns no import/postprocess policy, dimensions, phase semantics, evidence schema, or
Scene meaning. This ADR supplies the scene-specific read/write, remap, invalidation, loss, and edit
ledger semantics to that shared mechanism rather than defining a second mechanism.

This is the accepted postprocess target, not a current conformance claim. `SceneEdit` has not yet been
cut over to the operation-owned retained store and canonical block projections; its retained
transaction, report, and remap obligations become enforceable only through the atomic U4 workspace
switch and deletion of the legacy path.

## Decision

`SceneEdit` is the only canonical mutation capability. It starts from a
`ValidatedScene`, shares unchanged storage, and owns all replacement buffers and scratch state. The
input remains a valid generation throughout the operation. Final publication runs central validation,
then atomically reserves exactly one private never-reused generation nonce for the final Scene through
ADR 0003's `ReservedSceneGeneration` as the final fallible pre-freeze identity acquisition. Ordered
per-step report evidence does not consume generation nonces. Preparation attempts to freeze
canonical/remap/report attribution only after that one reservation succeeds and returns exact
`PreparedSuccess` only after the freeze and final control classification complete. Its pure handoff
consumes the one reservation into one fresh collision-free `SceneGenerationId`, constructs the new
`ValidatedScene`, and publishes the edit report/remaps. Branched edits from one input never share an
output generation token.
Failure, resource exhaustion, or cancellation drops the working set and publishes no scene.

Before a compiled postprocess plan opens `SceneEdit`, derived-operation start linearizes against the
input Scene account's shared ADR 0042 poison/counter state. Poison-first returns typed
`AccountPoisoned` before step work, attempted/consumed charging, share/refcount/range access, scratch,
remap, diagnostic, or replacement allocation. If start or retain/share admission linearizes first and
another account clone later poisons the account, admitted charge remains coupled, every later
retain/share attempt rejects, and final publication eligibility returns
`FinalizationContainmentError` with no exact prepared report, output generation, remap, or handoff.
Neither plan execution nor `SceneEdit` exposes reset, ignore, reuse, or account replacement.

The edit operation's pre-admitted fallback/evidence startup pair follows ADR 0042 before any exact
postprocess terminal can exist. A startup builder panic returns a start error with no accepted edit
invocation or terminal; successful startup instead preserves the exact baseline required by later
fallback preparation. A later retained candidate/report builder panic returns fatal containment and
never becomes a clean postprocess failure or minimal fallback. The unchanged input generation remains
valid and no report/remap is published. ADR 0042 exclusively owns physical pair admission, side and
pre-attribution containment, attribution coupling, cleanup evidence/precedence, and retained
allocation membership; this ADR owns only the edit-visible input/output and report consequences.

ADR 0013 decides whether a compiled postprocess plan requires a canonical write before opening a
`SceneEdit`. `NotRequested` and successfully executed `Unchanged` outcomes do not publish an edit and
therefore preserve the input proof, digest, and `SceneGenerationId`. `Changed` is the only successful
postprocess outcome that publishes; every such handoff still consumes a distinct reserved generation,
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

ADR 0013 compiles postprocess requests into a typed plan before an edit begins. This ADR requires
every operation descriptor consumed by that plan to declare entity/domain read sets, write sets,
invalidation sets, remap obligations, and structural loss classes. `SceneEdit` accepts writes only
through those declared effects and charges replacement/remap storage before constructing it.
User-facing edit methods are typed adapters over the same effect descriptors; there is no raw
mutable collection escape hatch that can omit remap or invalidation obligations.

After each operation constructs its affected replacements, the edit validates that operation's
postconditions, every affected canonical invariant, remap completeness, and derived-data outcome in
a private candidate state. Only a passing candidate is atomically swapped into the transaction's
working state for the next operation to observe. This is not public publication and constructs
neither a public generation nor a `ValidatedScene` proof; final preparation still performs full
central validation for the complete plan. A step failure discards its candidate and prevents all
later steps from running.

Each passing immutable private step state uses a transaction-local typed step path/ordinal, not a
generation nonce. Preparation seals those paths into report-branded `ReportStepStateId` values. A
step-state ID is meaningful only inside its owning report: it cannot be compared across reports,
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

### Remap Contract

Every successful structural operation records step-state-qualified private remaps for each affected
entity and attribute domain. Successful final edit handoff publishes those ordered per-step remaps
with report-scoped `ReportStepStateId` endpoints and separately publishes one composed
generation-qualified input-to-final remap. It does not relabel intermediate step states as Scene
generations. A value-only edit marks the corresponding identities unchanged while still reporting
invalidation. An entity remap is a relation, not a one-to-one map:

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

Replay evidence is invocation-local. An `Unchanged` replay publishes no generation, remap,
provenance, derived-data producer, or committed-output change; its scene digest, proof, and producer
records remain identical even when attempted work is non-zero. A replay that changes state creates a
fresh generation and fresh remaps from the immediately preceding input generation. Earlier remaps
are never copied under new endpoints; a cross-invocation relation exists only through explicit
composition.

Every executed step emits an invocation-local application record containing the full normalized
operation identity (operation ID, semantic version, and normalized parameter digest), stamp-schema
version, pre-write semantic-input stamp, post-write semantic-input stamp, and postcondition
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

An edit report contains both report-state-qualified ordered per-step remaps and one
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

Postprocess success reports its `Unchanged` or `Changed` outcome, ordered steps, diagnostics, losses,
resource statistics, entity remaps, domain remaps, and derived-data outcomes. `Unchanged` has no
committed output or remaps and names the preserved input generation. Resource semantics are owned by
ADR 0022: attempted and consumed work remain monotonic on success or failure; peak-live records the
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
| Postprocess starts or publishes through a poisoned account clone | Critical | derived start and final eligibility use ADR 0042's shared linearization state; poison-first has zero work/evidence/refcount/allocation effects, start/retain/share-admission-first retains charge but returns fatal containment before handoff, and no reset/reuse bypass exists |
| A retained edit/evidence builder panic is downgraded to an ordinary failure | Critical | startup panic returns a start error with side containment; later candidate/report panic returns fatal containment; fallback requires verified candidate cleanup and intact `StartupBaselineOwner` |
| A report step ID is mistaken for persistent Scene identity | Critical | Brand every typed step-state path to its owning report and expose no conversion, cross-report comparison, Scene lookup, persistence, or proof API. |

## Success Metrics

| Metric | Target | Measurement |
| --- | --- | --- |
| Atomicity | Every injected failure leaves input generation, digest, and proof unchanged and publishes zero remaps | failure/cancellation/resource matrix |
| Reserved generation atomicity | exactly one final reservation is attempted; exhaustion consumes no nonce and fails before freeze; any later freeze/ledger/product-graph preparation failure, cancellation/caught-unwind/prepared drop may burn that one gap; no failed path publishes a generation/proof/remap | reservation and publication typestate probes |
| Poisoned edit containment | all account/Scene clones reject new changed operations after poison; a pre-admitted invocation cannot publish when poison wins final eligibility | two-order start/retain/share/finalization barriers, work/evidence/refcount/allocator deltas, account snapshots, and no-bypass scans |
| Retained failure isolation | failed startup accepts no edit invocation; a later retained builder panic publishes no fallback, generation, proof, report, or remap; the input remains unchanged | postprocess integration against ADR 0042 terminal outcomes and input-preservation probes |
| Remap completeness | Per-step remaps cover every affected entity/domain with exact same-report step-state endpoints, and the composed remap has exact input/final generation endpoints | split/merge/delete/generate composition tests |
| Operation-local complexity | Private candidate validation and remap composition are `O(affected entities + affected edges + affected cells + remap cells)` | deterministic per-step scale counters |
| Publication validation complexity | Final central validation is `O(total graph edges + total entities + total references + total payload cells)` under ADR 0003 | deterministic full-validator work counters |

## Acceptance Criteria

- Injected failure, cancellation, and budget exhaustion preserve the input generation and full-data
  digest.
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
  0042's physical and attribution tests own the exact containment mechanics. The input generation,
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
- Coordinate/topology changes invalidate or recompute all dependent bounds, normals, and tangents.
- Published derived fields are either `Absent` or generation/input-stamped `Valid`; no private dirty
  marker or stale normal/tangent survives commit.
- Multi-step reports compose their input-to-final relation exactly, and sibling commits from one
  input use different output generation endpoints.
- Private step/candidate remaps refer only to transaction-local typed paths. Final handoff publishes
  ordered remaps with owning-report `ReportStepStateId` endpoints and one generation-qualified
  composed input-to-final remap. Step-state API tests reject Scene lookup, persistence, cross-report
  comparison/conversion, and proof construction; no intermediate state receives a generation.
- A compiled plan cannot open an edit unless every operation descriptor covers its writes, remaps,
  invalidation, and structural loss obligations.
- A second state-idempotent invocation with equal operation identity, a current pre-invocation stamp
  matching the prior record's post-write stamp, and a satisfied postcondition is `Unchanged` with
  identical digest, proof, generation, provenance, and derived producers; changing parameters,
  semantic version, or declared input reevaluates the operation. Tests include an operation that
  changes its own read set so replay cannot accidentally compare against its prior pre-write stamp.
- Prior-record lookup fixtures prove canonical traversal, equivalent-record collapse, and
  reevaluation after carrier deletion, missing evidence, or conflicting matching records; detached
  reports and recency never authorize a skip.
- Intentionally repeatable operations are not suppressed by prior history; when they change state,
  their remaps start at the immediately preceding input generation rather than relabeling old remaps.
- External code cannot publish a candidate scene or mark derived data valid without final validation,
  generation reservation, preparation freeze, and handoff.

## Amendment History

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
