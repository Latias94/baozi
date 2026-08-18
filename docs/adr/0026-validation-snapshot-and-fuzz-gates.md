---
status: accepted
date: 2026-07-09
updated: 2026-08-04
authors:
  - Codex
decision_type: testing
depends_on:
  - docs/adr/0005-testing-fuzzing-and-differential-oracle-strategy.md
  - docs/adr/0017-serialization-snapshot-and-cache-boundary.md
related:
  - docs/adr/0003-core-scene-ir-and-material-model.md
  - docs/adr/0025-scene-ir-invariants-and-validation-boundaries.md
evidence:
  - docs/plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md
tracers:
  - crates/baozi-core/src/canonical_scene/store/construction/node_scene/snapshot.rs
  - crates/baozi-core/src/canonical_scene/store/construction/node_scene/snapshot/tests.rs
  - fuzz/fuzz_targets/stl_import.rs
  - fuzz/fuzz_targets/obj_import.rs
  - fuzz/fuzz_targets/ply_import.rs
---

# ADR 0026: Validation, Snapshot, and Fuzz Evidence Contract

## Status and Authority

This accepted ADR is the sole authority for the evidence that must accompany each behavior-bearing
canonical field and transformation result. ADR 0005 owns the broader test portfolio, fixture
provenance, and external-oracle safety policy. ADR 0017 owns the separation between the Rust API,
test snapshots, and future cache schemas. This ADR owns the non-bypassable field evidence contract.

Acceptance defines required architecture, not current completion. Test counts, currently reachable
formats, fuzz campaign results, and pass/fail status belong in verification records rather than this
durable decision.

## Context

Parser output can be structurally plausible while values, references, transforms, domains, or loss
semantics are wrong. Counts and short previews reveal gross shape but do not prove that data beyond
the preview participates in regression evidence. Full normalized text makes every value visible but
can duplicate a hostile-size Scene in memory and produce unreviewable goldens. Fuzz targets can also
create false confidence when they exercise a parser helper instead of the facade/session path or do
not validate successful output.

Baozi therefore needs evidence that is complete for machines, bounded for reviewers, reachable
through production boundaries, and updated atomically with the field it protects.

## Decision

Every new or changed behavior-bearing canonical field must add or update four forms of evidence in
the same implementation unit:

1. **Validator evidence.** A central, bounded rule checks references, domain cardinality, numeric
   validity, ordering, storage shape, and resource limits applicable to the field. Invalid cases have
   stable issue codes and object paths.
2. **Snapshot evidence.** The canonical snapshot traversal includes the complete semantic value in
   a versioned, domain-separated digest and emits a bounded human preview.
3. **Direct evidence.** A focused unit, malformed, property, or independently authored semantic
   expectation proves both a valid value and its most important invalid or lossy boundary.
4. **Reachability evidence.** A production-path fuzz target can mutate the field through the facade,
   import session, validator, or postprocess entry point that publishes it. Every successful fuzz
   result is centrally validated.

A field is not complete because it appears in a struct, count, debug print, parser-private test, or
compile-only fuzz harness. Producerless concepts remain abstract rather than adding concrete public
variants with no way to satisfy this contract.

```mermaid
flowchart LR
    Change[Canonical field or transformation change] --> Validator[Bounded validator rule]
    Change --> Snapshot[One-pass canonical snapshot]
    Change --> Direct[Independent direct test]
    Change --> Reach[Production-path fuzz reachability]
    Snapshot --> Digest[Complete versioned digest]
    Snapshot --> Preview[Bounded review preview]
    Validator --> Gate[Evidence gate]
    Digest --> Gate
    Preview --> Gate
    Direct --> Gate
    Reach --> Gate
    Gate --> Publish[Eligible to publish]
```

### Complete Digest Contract

The snapshot schema has an explicit schema version, domain tag, and digest-algorithm identifier.
The digest traversal covers every canonical value exactly once, including omitted-from-preview
values, collection lengths and order, enum discriminants, option presence, integer signedness and
width, string/byte lengths and contents, references, transforms, metadata values, material
properties, embedded payloads, skin matrices and influences, morph data, animation values,
provenance, remaps, losses, and structured diagnostics that belong to the selected snapshot domain.

Encoding is canonical rather than derived from `Debug`, memory layout, or host endianness. Maps use
their specified deterministic key order. Finite floats use a field-defined canonical encoding; any
normalization of signed zero or precision is explicit in the schema and never inherited from a
display epsilon. A change to field encoding, traversal meaning, or digest algorithm follows the
schema lifecycle below and always updates known-vector tests in the same change.

Raw import, postprocessed output, validation, and report evidence use distinct domain tags. Equal
bytes in two domains therefore do not create an accidental equivalence claim.

### Snapshot Schema Lifecycle

Every snapshot domain is in exactly one of two lifecycle states:

1. **Draft.** Before Baozi publishes an explicit compatibility baseline for that domain, its latest
   canonical encoding is the only retained implementation. An incompatible design improvement may
   replace the draft encoding in place while keeping the intended first revision number. The same
   change deletes the superseded writer, reader, vectors, and compatibility language; updates every
   validator, direct expectation, digest, and preview total; and makes no claim that previously
   generated draft bytes remain readable or comparable. Parallel `V2` types, compatibility decoders,
   and migration shims are forbidden for an unpublished draft.
2. **Frozen.** A domain becomes frozen only through an explicit reviewed baseline that names the
   domain, revision, digest algorithm, known vectors, compatibility scope, and first release or
   artifact contract that depends on it. After that event, revision numbers are never reused or
   reset. Any incompatible encoding, traversal, or digest change advances the revision
   monotonically; compatible implementation changes retain the revision and must preserve every
   known vector.

The lifecycle state and freeze baseline belong in the repository conformance inventory rather than
an implicit code comment or a transient branch name. Until an explicit reviewed freeze record exists
there, a domain is Draft. A normal CI artifact, passing vector, tag, package, or publication event
cannot freeze a domain by implication. Updating a draft vector proves only the current
implementation; it does not retroactively create a compatibility guarantee. Any Draft digest or
evidence persisted or compared across commits binds the exact source-closure digest (or commit
identity when the closure is exactly the tracked source) and vector-set digest in addition to
domain, draft revision, and algorithm. The shorter tuple is not a stable cross-commit cache or
comparison key.

Physical storage topology is not snapshot semantics. Block identities, capacities, allocation
owners, profile segment ordinals, and process-local nonces do not enter a semantic domain digest
unless a separate domain explicitly defines them as its subject. Two centrally validated Scenes
with equal canonical meaning but different retained segment partitioning must produce the same
domain digest and bounded semantic preview.

```mermaid
stateDiagram-v2
    [*] --> Draft
    Draft --> Draft: Replace unpublished encoding and vectors
    Draft --> Frozen: Record reviewed compatibility baseline
    Frozen --> Frozen: Preserve vectors for compatible implementation changes
    Frozen --> Frozen: Advance revision for incompatible changes
```

### Bounded Preview and Traversal

Snapshot generation traverses canonical values once and feeds two sinks:

- a constant-state digest sink for complete machine evidence;
- a preview sink capped by configured bytes and per-collection item limits.

Preview truncation is explicit and includes total counts, but it never truncates digest coverage.
The implementation must not construct a full normalized string or clone the Scene. Allocation,
traversal work, optional ordering scratch, output bytes, and cancellation checks are charged before
growth under snapshot-specific limits. Ordering scratch is bounded and documented when canonical
storage does not already provide deterministic order.

Scale tests use 1x, 10x, and 100x inputs, plus selected million-cell or 1000x cases, to prove linear
visit counts, bounded preview text, and auxiliary memory that does not grow with omitted payload
bytes except for explicitly capped ordering scratch.

### Direct and Differential Evidence

Checked-in goldens are review artifacts, not their own only oracle. Critical format semantics have
independently authored expected values or specification-derived fixtures. Importer-generated output
must not be accepted merely by regenerating its snapshot.

Mutation tests alter a value after the visible preview boundary and must change the complete digest.
Direct tests also cover invalid references, non-finite values, domain/count mismatches, unsupported
loss, and budget exhaustion relevant to the field. Tolerated external-oracle differences remain
explicit under ADR 0005 and cannot weaken Baozi's validator or source expectation.

### Fuzz Reachability

Parser fuzz targets invoke the same detection, stable-input session, dependency policy, conversion,
and final validation path used by the facade. Format-private decoder targets may supplement this
coverage but cannot replace it. Generic validation and postprocess targets cover canonical edits or
transformations that no current source format can construct directly.

Fuzz assertions require termination within budgets, no panic/abort/invalid success, bounded failure
evidence, and central validation of every success. Compilation proves harness compatibility; the
declared sanitizer platform and bounded execution campaign provide runtime evidence.

## Alternatives Considered

### Option A: Counts and compact previews only

Pros:

- small, readable snapshots
- inexpensive implementation

Cons:

- mutations outside the preview can remain invisible
- equal shapes can hide wrong transforms, indices, weights, metadata, or animation values
- adding a field can appear covered without its payload entering any oracle

Decision: rejected because shape-compatible semantic regressions remain undetectable.

### Option B: Full normalized text snapshots

Pros:

- every value is directly inspectable
- simple text comparison

Cons:

- duplicates large canonical data and can exceed memory/output budgets
- creates noisy, impractical reviews for large buffers
- couples correctness evidence to display formatting

Decision: rejected as the complete representation. Small focused fixtures may still print all
values inside the same bounded schema.

### Option C: Complete domain-separated digest plus bounded preview

Pros:

- every value affects deterministic machine evidence
- reviewer output and auxiliary memory remain bounded
- schema and domain changes are explicit
- supports large-scale mutation and complexity tests

Cons:

- a digest difference identifies that data changed but the preview may not locate it
- canonical encoding and known vectors require maintenance
- digest evidence must be paired with independent expected values to avoid self-oracles

Decision: chosen.

### Option D: Monotonic revisions from the first prototype

Pros:

- every historical experiment has a distinct number
- no explicit freeze event is needed

Cons:

- unpublished implementation churn creates dead compatibility versions with no consumer
- historical experiment numbers do not prove that any pair of those experiments is compatible
- a large revision number can be mistaken for maturity or compatibility evidence

Decision: rejected before the first compatibility baseline. Monotonic revisions are mandatory after
the domain is frozen.

### Option E: Allow undocumented revision resets until release

Pros:

- minimal process during early development

Cons:

- reviewers cannot tell whether a changed vector is a draft replacement or a broken promise
- stale artifacts can silently compare unequal under the same claimed schema identity
- the freeze boundary becomes a convention rather than auditable evidence

Decision: rejected. Draft replacement is permitted only under the explicit lifecycle and atomic
evidence rules above.

## Consequences

Positive:

- hidden canonical data cannot silently bypass regression evidence
- snapshot output stays reviewable for large assets
- validators, direct tests, and fuzz reachability evolve with the IR
- successful parser and postprocess fuzz cases cannot publish invalid Scene state
- unpublished schema experiments do not create compatibility scaffolding, while the first frozen
  baseline creates a precise monotonic contract

Negative:

- every semantic field has a higher implementation and review cost
- snapshot schema changes require explicit versioning and golden updates
- digest-only differences need focused fixtures or diagnostic tooling for localization
- freezing a domain requires an explicit reviewed baseline and prevents later revision reuse

## Risks and Mitigations

| Risk | Severity | Likelihood | Mitigation |
| --- | --- | --- | --- |
| Snapshot code forgets a newly added field | High | Medium | Require the four-evidence checklist in the same change and exhaustive snapshot/validator review at the owning type boundary. |
| Digest schema changes accidentally | High | Medium | Version and domain-separate the encoding; maintain known vectors and cross-platform goldens. |
| Digest collision masks a change | Medium | Very low | Use an admitted collision-resistant algorithm and pair digests with direct semantic expectations. |
| Preview or sorting recreates input-sized memory | High | Medium | Cap both sinks and scratch reservations; enforce million-cell scale and peak-memory tests. |
| Fuzz target exercises only a private parser helper | High | Medium | Require facade/session reachability; treat private targets as supplemental evidence only. |
| Snapshot regeneration becomes the only oracle | High | High | Require specification-derived or independently authored expectations for critical semantics. |
| Compile-only fuzz checks are mistaken for runtime safety | High | Medium | Record sanitizer execution separately and prohibit maturity promotion on compilation alone. |
| A draft reset is mistaken for a compatibility guarantee | High | Medium | Record Draft or Frozen state explicitly, prohibit parallel draft decoders, and bind the freeze event to reviewed vectors and a release or artifact contract. |
| Physical retained-layout changes perturb semantic evidence | High | Medium | Exclude allocation and segment identities from semantic domains and test equal meaning across different segment partitioning. |

## Acceptance Criteria

| Contract | Measurable evidence |
| --- | --- |
| Complete coverage | Mutating any selected canonical value, including one beyond the preview limit, changes the domain digest. |
| Schema stability | Known vectors pin schema version, domain tags, type/length encoding, float policy, and digest algorithm. |
| Lifecycle discipline | Draft replacement deletes superseded encodings atomically; a frozen domain names one reviewed baseline and every later incompatible change advances its revision. |
| Storage independence | Equal canonical meaning under different retained segment partitioning produces the same semantic digest and preview. |
| Bounded review output | Million-cell scenes remain within configured preview bytes and do not materialize full normalized text. |
| Linear traversal | 1x/10x/100x scale tests report one bounded visit per encoded value plus documented capped ordering work. |
| Field discipline | Each behavior-bearing field has validator, snapshot, direct, and parser- or generic-reachable fuzz evidence. |
| Invalid-state rejection | Direct and fuzz tests prove invalid references, cardinalities, values, and budgets cannot become successful validated output. |
| Production reachability | Public parser fuzz targets execute detection/session/dependency/finalization boundaries, and every success passes central validation. |
| Oracle independence | Critical supported semantics have source-derived expectations or explicit tolerated-difference records, not only regenerated snapshots. |

## Implementation Evidence

Implementation and verification status change over time. Consult the canonical architecture plan,
test-support snapshot implementation, format/facade test suites, fuzz workspace, and engineering
verification records. Those artifacts demonstrate progress without changing this accepted decision.

## Amendment History

### 2026-08-04: Define the snapshot compatibility freeze boundary

- Affected: `Complete Digest Contract`, `Snapshot Schema Lifecycle`, alternatives, consequences,
  risks, and acceptance criteria.
- Previous: every encoding or traversal change required a revision change, but the ADR did not say
  when an unpublished draft first acquired a compatibility promise or whether retained storage
  partitioning belonged to semantic evidence.
- New: unpublished domains replace their sole Draft encoding and vectors atomically; only an
  explicit reviewed conformance-inventory baseline freezes compatibility; Frozen revisions advance
  monotonically; Draft evidence is source- and vector-set-qualified; and physical segment topology
  is excluded from semantic digests.
- Impact: Baozi can converge the current unpublished implementation on one final v1 without
  compatibility shims, while no stale Draft artifact may impersonate a stable cross-commit result.
  Frozen consumers gain an auditable monotonic revision promise and storage-independent semantics.
- Review: 2026-08-04 architecture review and Codex adversarial document review.
- Rationale: this closes an ambiguity inside the existing snapshot evidence concern and strengthens
  its compatibility boundary without moving ownership, reversing the selected digest design, or
  requiring a replacement ADR.
