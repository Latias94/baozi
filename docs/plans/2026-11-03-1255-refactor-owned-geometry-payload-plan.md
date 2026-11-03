---
title: "Preserve generated geometry ownership through validation and publication"
type: refactor
status: in_progress
date: 2026-11-03
parent: 2026-11-03-0900-perf-assimp-convergence-plan.md
---

# Owned Geometry Payloads

The [convergence plan](2026-11-03-0900-perf-assimp-convergence-plan.md) permits breaking
representation and ownership changes before release. The
[measured normal-action slice](../reviews/2026-11-03-bounded-normal-actions.md) removes
repeated getter-level execution but leaves a larger Scene-edit route. This plan addresses
the storage/interface cause of its generated-payload replay. It complements the
[exclusive invocation plan](2026-11-03-1120-refactor-exclusive-invocation-plan.md); it creates
neither another numerical authority nor a publication path around that owner.

## Evidence and Decision

The [source audit](../research/2026-11-03-generated-payload-ownership.md) identifies the actual
loss of ownership: `GeneratedNormals` owns capacity-qualified triple storage, but selected
replacement retains only borrowed `VectorValues`. Global scalar pools then force Count,
Build and Compare reconstruction of those values. A standalone generated Vec cannot donate
its allocation to an occupied scalar Vec.

At R5 `57c33b2447a296c28f5f5817a36debd248bd4280`, the repeated preparation function subtree
is 384,159,274 Linux Ir. Scene edit accounts for 286,570,373; three generated-normal walks
account for a nested 159,613,541. Their 147,458 controlled calls per pass include 147,456
scalars plus metadata. These owners overlap the exclusive-runner opportunity. They are not
additive savings or native-time percentages. The measured fixture holds 49,152 F32 triples,
589,824 logical bytes, with scratch and copied canonical output coexisting.

Choose one core-owned immutable typed payload representation with checked ranges. Preserve
the generated allocation from birth, freeze its last mutation, validate with the actual core
predicate, then consume its owner into the changed Scene. Central validation composes that
value fact with attachment, schema, source correspondence and graph checks. The result must
use the same reader as imported data. A generated-field flag is not a validity proof.

| Alternative | Advantage | Limitation / decision |
| --- | --- | --- |
| Owned typed blocks and consuming attachment | Removes generated scalar replay/copy and duplicate backing; permits ownership to establish exact-bit correspondence. | Selected; requires coordinated storage, proof and lease changes. |
| Keep global pools; use closed triple/chunk actions | Smaller writer/counter change; removes many per-scalar authority boundaries. | Useful independent control, but preserves Build/Compare walks, borrowed-only replacement and duplicate backing. It is not the final ownership design. |
| Bulk-copy borrowed fields after checked-length planning | Avoids Count enumeration and reduces writer overhead. | Still allocates/copies and needs independent bit correspondence. Do not maintain it as a second generated-payload backend. |

No new dependency or general proof/allocator framework is proposed. One concrete core module
owns this boundary. Necessary public/integration/snapshot interfaces may break; remove retired
implementations rather than add aliases. Preserve old measurement artifacts under their real
source identities.

## Ownership and Validation Boundary

```mermaid
sequenceDiagram
    participant H as Invocation host
    participant B as Core buffer owner
    participant N as Normal kernel
    participant T as Consuming Scene transaction
    participant V as Central validator
    H->>B: Admit capacity; allocate and qualify actual backing
    B->>N: Lend mutable initialized triple slice
    N-->>B: Finish generation and final orientation
    H->>B: End mutation; validate exact core predicate
    B-->>T: Move immutable owner, value facts and live lease
    T->>T: Admit metadata/destination; verify binding; attach once
    T->>V: Actual attachment, owner receipt, schema and graph
    V-->>T: Checked candidate or owned rejection
    H->>T: Final control, freeze and joint publication
    T-->>H: Owned changed Scene and independent evidence
```

The source Scene remains immutable and owned throughout. Ordinary rejection returns that
original Scene/generation/bits. Candidate destruction precedes verified lease release;
uncertain cleanup retains conservative charges and containment. An attached buffer or value
fact grants no publication permission.

Core owns allocation, actual-capacity qualification and issuing invocation from the start.
The normal kernel receives a typed mutable slice. Order/cross workspace remains scratch.
Tangents may borrow values after orientation; no certificate is minted while any mutation
or dependent preparation can still change them. Do not expose an unrelated Vec plus an
arbitrary reservation as a trusted constructor.

At the current semantic-preflight position, core runs the exact existing normal predicate,
including width, finite checks, sequential sum-of-squares and tolerance. Kernel validation
is insufficient: it uses fused `mul_add` and F32 `8*epsilon`, whereas core uses sequential
arithmetic and F32 `4*epsilon`. Preserve both semantics and their error order. Partial
validation, denial, unwind or a failed final check cannot mint whole-buffer validity.

Only complete validation consumes mutation and yields the immutable owner plus core-issued
value facts. These are facts about that initialized typed range, not a whole-Scene proof.
Empty value validation does not establish target cardinality. Attachment and central schema
checks must still reject missing/extra rows and distinguish absent from present-empty fields.

## One Storage and Reader Representation

Replace implicit global scalar-pool references with typed immutable blocks and checked
ranges for all four scalar kinds. An existing imported pool may be one block; do not allocate
one block per attribute. Generated F32/F64 triples become a block through the safe installed
Rust 1.95 `Vec::into_flattened` operation, preserving allocation and bytes. Convert triple
length/capacity to scalar units with checked arithmetic; do not invent a new allocator envelope.

Dense/sparse data, indices and sparse defaults resolve through one core implementation and
the existing scalar sampler. Keep each payload's contiguous borrowed slices, scalar bits,
source/domain semantics and Scene lifetime. Resolve a block at field binding; do not clone
shared ownership or search a block table per scalar. Revisit descriptor borrowing if needed
at its actual owner rather than preserve a wide Copy representation for compatibility.

Do not add `LegacyPool` versus `GeneratedOwned` reader branches, a postprocess sampler, a
hidden payload cache or an unvalidated transform-only view. Space conversion and every typed
geometry producer must migrate to the same representation. Genuine immutable sharing may
outlive an invocation; mutable continuations may not.

Retain each referenced block once in the output inventory. Repeated edits must not accumulate
dead generated blocks or unbounded retained ancestry. A referenced coarse imported block can
retain obsolete ranges: charge its whole live capacity and disclose that tradeoff. Compaction
would be a separately admitted copy, not free memory recovery.

## Consuming Attachment, Proof and Capacity

Before implementation, specify the concrete Rust owner/state types, error ownership and
lease transition. Use existing issuance and retained-storage mechanisms where possible.
The trusted attachment verifies invocation/account, request, source generation/primitive,
output slot, Normal/Corner domain, scalar width, lanes, initialized full range and length.
Account equality alone is insufficient. Identity cannot be a payload pointer, especially for
empty buffers, a hash alone, an index alone or a caller-supplied validity flag.

The consuming transaction retains a private expected-owner receipt. Central validation
checks the actual installed owner/range/type/binding against it and composes core value facts
with real schema, dependencies, remaps, lineage, profile and graph checks. Handle relocation
is allowed when those identities remain established. A different equal-valued owner or range
requires its own correspondence; schema validity cannot substitute for source correspondence.

Owned attachment proves that generated bits were not changed by reconstruction, so delete
generated-payload Compare replay. Preserve unchanged/remapped fields' actual source/sparse/
bit correspondence checks. Uncertified values use the same authoritative value validator;
do not add `if generated { skip_validation }` or a second normal predicate.

Capacity stays continuously charged. Admit destination/header/table obligations before
consuming the transfer, and use one atomic authority transition without refund/reacquire
gaps. Scratch has no canonical publication allowance; merely retaining its lease does not
make it publishable. Charge actual spare capacity, each child once and outer metadata
separately. Retain existing physical obligations and the exclusive plan's honest known
payload/capacity metric; opaque standard Arc/allocator overhead is not a portable zero.

Use the same numeric work kernel, storage admission, sharing and joint publication owner.
Validate one normal triple per bounded action, complete actual returned prefixes immediately
and poll at the qualified cadence and semantic boundaries. Attachment moves one prepared
owner into an admitted slot; allocation/control/user Drop occurs outside that action. No
payload traversal becomes one unit. Removed physical work disappears from a newly identified
work vocabulary; do not pre-complete historical scalar counts or increase limits.

## Implementation Sequence and Deletions

1. Independently review this plan and the concrete ownership/proof/lease design before Rust
   edits. Qualify the exclusive first proof and define its consuming attachment seam. Preserve
   current compact owner-catalog projection and weighted control; no second accounting owner.
2. Implement the single typed block/range resolver and migrate imported pools plus every
   geometry reader/producer. Preserve meaningful source, sparse, topology and snapshot tests.
   Any snapshot layout break has an explicit new contract; no unpublished legacy decoder is
   retained only to keep obsolete tests passing.
3. Introduce the concrete core normal-buffer owner, lend it to the existing kernel, and
   freeze/validate only after the final mutation. Add the consuming typed attachment through
   the existing transaction; qualify actual backing and lease reuse with real storage.
4. Compose central value/attachment proof and ownership correspondence. Delete borrowed
   generated-normal planning/build/Compare replay and duplicated scalar validation only where
   the immutable proof justifies replacement. Metadata and other-field checks remain.
5. Remove superseded pools, generated-only adapters, constructors and compatibility paths.
   Version work evidence honestly. Finish temporary scratch cleanup before terminal freeze,
   then qualify all actual import/postprocess/consumer paths.
6. Build immutable ordinary workers and repeat the same semantic comparison, including
   current per-endpoint repair items, actual heap attribution and whole-child memory. Refresh
   profiles before selecting further numerical or representation changes.

## Qualification and Measurable Success

| Obligation | Required evidence |
| --- | --- |
| Allocation adoption | Real nonempty F32/F64 backing, bits and actual capacity survive the move without a payload copy; empty-owner identity is checked independently of dangling payload pointers. |
| Single authoritative value check | Exact current tolerance/finite/length errors and complete-prefix accounting; no value fact after partial validation, cancellation or panic. |
| Binding and linear ownership | Wrong invocation/source/target/slot/domain/width/range and owner substitution reject; relocation succeeds; valid-API compiler probes prohibit post-freeze mutation and duplicate attachment. |
| Reader fidelity | Dense/sparse values, defaults, all scalar kinds, source identities/domains, topology, tangent/morph dependencies and detached public consumers keep independent literal/bit oracles. |
| Failure and release | Original Scene on rejection; denial at validation/transfer/central/final boundaries; exact capacity/metadata charges; drop once, source-first/output-first sharing and bounded repeated-edit retention. |
| Removed work | Actual profiles/source show generated Count/copy/Compare replay removed, rather than moved outside the clock. Remaining validation/metadata work is reported. |
| Ordinary performance | Material same-task gain beyond dispersion with independently reviewed per-lane regressions and comparable memory; no predicted percentage substitutes for measurement. |
| Release | Parent plan's full fourteen cases/110 lanes on Windows and Linux, every endpoint/pipeline and comparable whole-child memory at most 3x Assimp, plus integrated correctness/lint/compiler/consumer gates. |

Use existing selected-edit, cancellation, normal-precision, sparse-reader and consumer suites;
add only the new ownership and proof boundaries. No layout-mirror tests or script-based Rust
reasoning. Frozen R5/R7 evidence cannot qualify this future implementation.

## Risks and Review Gates

| Risk | Mitigation / implementation gate |
| --- | --- |
| An invalid value fact bypasses central validity | One exact core predicate; mutation ends before issuance; actual owner/range/binding composition; reject partial and foreign facts. |
| Capacity disappears or is charged twice during transfer | Concrete same-invocation transition and actual spare-capacity tests before installation; never detach backing from its lease. |
| Block sharing retains obsolete storage | Referenced-block inventory and repeated-edit/drop tests; whole-capacity accounting and no universal memory-saving claim. |
| Broad representation cutover leaves two readers | Migrate all scalar kinds/producers together through one resolver; delete retired pool paths before keep. |
| A numerical change is hidden as ownership work | Kernel expressions, orientation, rounding and both existing predicates remain; independent literal bits/errors and source review. |
| Overlapping opportunity is double-counted | Re-profile after exclusive caller migration and before adoption measurement; separate actual owners and never add nested Ir. |

Independent plan and final bounded concrete-design reviews find no open P1/P2 at their
reviewed revisions. Fresh/edit issuance, single-leaf consolidation, publication-local
inventory and typed capacity rejection before structural freeze are specified below. The
root-bearing issuer and protected storage transition must still be implemented and qualified
before adoption; the exclusive first proof does not supply those APIs. User authorization
covers necessary breaking development; the remaining gates are source, compiler, accounting
and measured qualification, not a new conversational approval requirement.

## Execution Record

- Source audit exported exactly from the read-only investigation, SHA-256
  `9a5f1a5673ea64dd60b909a4478b8ba0a683c8dcbce5c321d79efdd7c75a9a6b`.
- Independent plan review checks the proposal against seventeen actual owner/validator/reader
  paths and accepts the bounded module/representation replacement without a compatibility
  layer. It emphasizes authored-versus-generated predicates, actual attachment identity,
  same-invocation transfer, retained source correspondence and the concrete design gate.
- The initial 315-line concrete design, SHA-256
  `61c18166dc30389e5c04013d5746d885b91f5c713f441d5c50fc7dc5ef73e3cd`, is superseded during
  review by the final revision below; it has no separate completed acceptance receipt. The
  concrete direction has core own independently leased typed blocks
  from birth, including the imported pools; no published aggregate yields a child Vec.
  `WritableNormals -> ImmutableNormals -> CheckedNormals` separates final mutation, tangent
  borrowing and the original core-check position. One private block/inventory identity and
  consuming attachment receipt establish actual owner/range/binding correspondence; central
  schema/source/graph checks remain. Finite facts do not strengthen authored normals.
- The proposed retained-build and adoption methods reuse existing storage authority. The
  additional shared header and inventory are admitted separately, while original backing
  stays continuously leased through Scratch-to-Canonical transition. Both methods depend
  on the real root/protected-role cutover. Private finite coverage needs ordered block cursors,
  not a general certificate, per-scalar bitmap or interval engine. Detailed design/review are
  retained under `.git/baozi-review/performance-convergence/generated-payload-ownership-concrete-*`.
- The first frozen 300-line revision, SHA-256
  `2c7ece13b37de6db4a5b825a9c20ad7b3eb1d8c0dcad252ade50dcb3dc4cb9fe`, identifies that existing
  shared-storage admission covers the outer allocation, not automatically the nested child
  backing. Independent review requires a concrete durable pair/source-coverage record, both
  capacities' final eligibility and explicit nested-release outcomes, or a fully specified
  consolidation alternative. Matching accounts and the old outer share call do not suffice.
  It also requires a fresh-admission issuer without a source Scene, sharing the same private
  request/ordinal authority with source-bound edits. The v2/v3 decisions below supersede
  those owner/signature proposals; there is no payload implementation or qualified adoption API.
- Exclusive first-proof source qualifies at `45a09d19d948600d9b2a4261eb2e3394fe594ef1`;
  real caller/root migration, payload adoption and performance qualification remain pending.
- Concrete v2, 213 lines, SHA-256
  `6ca865fce5bb4f54f9e1b21fadcd645604024a2fbf4b58162b7351b0b83ce0e8`, chooses one ordinary
  shared leaf: atomic backing/header consolidation preserves physical bytes/allocations,
  combines classified logical tokens without refund/re-admission, and uses the existing
  final-owner release outcome. Fresh and source-bound edit issuers share one private counter.
  Independent review accepts those old P2 corrections and requires two further actual-caller
  closures: declaration-only space conversion cannot share an immutable inventory while
  restamping its children, and configured late capacity denial must retain its typed resource
  cause rather than become `TerminalEvidenceFreezeUnverified` containment. No payload Rust
  or measured gain is claimed.
- Concrete v3, 126 lines, SHA-256
  `425f29a157b39381cda0463ed17ecb7c67accf4681e821d9057ebb16fb54068d`, closes both v2 findings
  in independent bounded review. `SceneTables.geometry` owns shareable descriptors plus a
  publication-local inventory by value. One assembly path admits a new inventory for every
  changed publication, including declaration-only space conversion, and shares each exact
  source leaf once with current full-G attribution. Identity descriptors may be shared only
  with identity slot mapping. Source wrappers are never restamped.
- V3 moves the new capacity check into the existing commitment-pair staging transaction.
  Healthy denial retains the exact typed ledger cause and intact ordinary failure route;
  successful staging seals ordinary work/policy before final control and structural freeze.
  This preserves the input Scene and distinguishes configured limits from malformed-graph
  containment. No extra final poll, fabricated commitment bytes or general policy framework
  is introduced. V3 supersedes those specific base/v2 owner/failure proposals; all unchanged
  mathematical, value/correspondence, cancellation and cleanup obligations remain.
