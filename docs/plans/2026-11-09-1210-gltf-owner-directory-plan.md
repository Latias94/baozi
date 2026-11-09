# Index the glTF source-owner directory by normalized owner domain

Status: architecture/work amendments and the bounded Stage1 A/B topology
clarification are adopted. Conditional Stage1 readiness review is closed; real
baseline population qualification is next. No dense algorithm edit or candidate
sampling is accepted yet.
This copy adopts the smaller coverage-derived shape and every bounded
architecture/work-contract amendment from the two closed reviews. Their
original AMEND reports remain immutable. Baseline population qualification
precedes the algorithm edit; comparator review precedes candidate sampling. Runtime
is exact qualified d81 at HEAD `5864b55`. The completion hint is retired; the
persistent comparison region and unary-kernel experiment are closed NO_GO/DEFER.
This plan changes a private data structure at its owner, without a new executor.

## Evidence and selected seam

`gltf-owner-directory-cost-study-1.md` independently binds original GLB1/32
profiles to d81 ELF372. GLB32 `finish` sorts 136 completed owner ranges for
634,004 inclusive Ir and checks uniqueness for 54,068. Directory searches add
a generous removable envelope of 214,576, but standalone lookup is below a
1% whole-worker opportunity. The combined 902,648 / 3.561% envelope is a loose
ceiling before replacement work, not predicted benefit. The 714 source-ordinal
conversions, each owner's ordinal sort/dedup, complete-owner coverage and all
Core validation remain outside it.

The actual normalized model already has compact, bounded owner IDs in nineteen
ProfileOwner classes. Node source indices differ from compact NodeLocalId
values: DFS construction assigns the latter. Association completes owners in
source/phase order, so requiring globally increasing owner keys is invalid.
Defaults and suspended sampler/animation runs also complete out of key order.

Selected design: preserve completed ranges in original append order and their
single flat ordinal vector, and build a direct index once after the existing
coverage pass. Use owner class and compact local index to locate a range index.
Keep existing entries/locators/ordinal relation storage; no new source catalog.
No Core trusted-input, adoption, provenance or public authoring API change.

## Alternatives

| Option | Assessment |
| --- | --- |
| Keep d81 | Default if measured gain does not cover replacement costs. |
| Cache lookup result per generated root/sampler/material | Saves at most three prefix scans elsewhere; does not remove this measured sort. Separate small hypothesis, not bundled. |
| Keep keys sorted while appending | Moves sort cost into insertion and can be quadratic on valid DFS/interleaved order. Reject. |
| General hash map or new tree | New hashing/allocation/resource order, unnecessary for closed compact owner domains. Reject. |
| Dense category index alongside append-order ranges | Selected: one admitted usize slot per possible canonical owner, O(n) build/query, no per-owner search or comparison sort. |
| Replace borrowed producer/Core contracts with owned adoption | Existing source study establishes different trust, nonce and allocator domains. Not justified by this directory cost. |

## Exact private representation

Keep `OwnerSourceRange { owner, start, length }` and `owners: MeteredVec<_>`
with their current allocation/transfer frontiers through `complete_owner`.
Add a private directory owner containing:

```text
OwnerDirectoryShape { offsets: [usize; 20] }
OwnerDirectory { shape, ranges: MeteredVec<usize> }
SelectedSourceProfile.directory: Option<OwnerDirectory>
```

Class order follows the exhaustive ProfileOwner match: Scene, SyntheticRoot,
Node, Camera, Light, Image, Sampler, Texture, TextureBinding, Material,
LogicalMesh, Primitive, Attribute, MeshInstance, Skin, MorphDefinition,
MorphDelta, AnimationClip, AnimationChannel. A present Scene/SyntheticRoot has
local index zero; other class lengths use the completed keys' checked maxima.
Missing classes have empty segments. Compact normalized IDs make successful
lengths equal actual class cardinalities, but the directory claims no model
completeness. A missing slot is `usize::MAX`, not uninitialized storage.
Each occupied slot is an index into the still-owned append-order `owners`.
No counter, work authority, storage allowance or escaping borrow is cached.

Derive shape while the existing contiguous coverage pass visits the actual
completed descriptors: after the original range check succeeds, a bounded
owner-key projection updates that class's checked maximum local index + 1.
Record checked local+1 overflow as a local invalid-domain flag; do not return
a domain error during coverage. Every existing range and covered-total
rejection must precede domain/size errors, including on a 32-bit target.
The same action still performs bounded work; no new model or JSON-table scan
is introduced. After covered-total validation, compute checked prefix offsets
for the at-most-19 keys and require totalSlots <= owners.len() inside that
same existing final covered-total action before allocation. No new
shape action or charge is added. This bounds new directory capacity by actual descriptors
even for a corrupt sparse/private key. It does not prove canonical owner
validity or completeness. The normalized producer's compact IDs satisfy the
bound; missing class segments and sentinel holes remain explicit. No new
finish argument, fixture shape constructor or model counter table is needed.

A single exhaustive private owner-key projection returns `(class, local)`;
Node uses NodeLocalId.index(). A shape method validates local < class length
before checked/indexed access. Only validated normalized owner IDs are expected
in production. A malformed private owner/domain is a clear internal format
error; no fallback to search or unbounded sparse allocation. Tests exercise
actual production inference and its explicit bound using known fixture facts;
they do not copy the maxima algorithm or supply a second model catalog.
The shape bounds indexing only; Core still validates canonical owners.
The original finish signature and complete_owner allocation frontier remain.
An out-of-domain or excessively sparse private graph fails internally before
allocation; it does not fall back to a quadratic/linear lookup or allocate
according to an attacker-sized raw index. A tiny valid shape test establishes
the zero-class, nonzero-offset and checked-overflow behavior independently.

## Construction, queries, and error/control ownership

`complete_owner` remains on its original path: open-run check, real ordinal
sort/dedup, range construction, ordinal reserve, owners reserve, actual
per-ordinal transfer, append descriptor, clear scratch. No early duplicate
rejection or directory-slot allocation is introduced there.

`finish` keeps its original entry poll and incomplete-run rejection, then the
actual append-order contiguous coverage pass and covered-total check. Replace
only directory sorting and the adjacent uniqueness comparator loop:

1. Admit actual directory capacity and initialize every actual slot to the
   missing sentinel under bounded metered mutation. Do not resize/set_len or
   populate slots outside the owned admitted storage. Resource failure drops
   this local directory before its reservation, leaving profile unfinished.
2. Traverse actual completed owner descriptors in append order. Admit one
   bounded registration action before reading/projecting/validating the owner
   and slot; reject duplicate occupied slots with the original duplicate-owner
   cause. Commit the range index only on normal success. Poll at entry, each
   64 registered descriptors, and short-scope exit. Do not defer actual work or
   prepay registrations never performed.
3. Preserve the final exit poll. Only after every check and poll succeeds move
   the local directory into the profile and set `finished=true`. Errors,
   cancellation, panic or denied work cannot expose a partial searchable map.

`for_owner` replaces the binary-search-bound lookup with one bounded direct
key resolution action, then keeps the original finished/nonempty/checked-range
validation at its existing second action. A missing/unconfigured/invalid key
returns the original missing-owner format cause; it cannot borrow data early.
No reference into a growing values vector is retained. The same lifetime ties
all borrowed ordinal slices to the owning profile. The profile's bool and
Option must be consistent in successful construction; tests may corrupt them
only to verify fail-closed behavior.

Duplicate detection now scans descriptors in physical append order rather than
sorted-key order, but returns the same parameter-free internal cause. Preserve
incomplete-run and range/coverage rejection before any indexing/duplicate work.
Declare new private key-domain errors separately. Earlier resource/denial and
cancellation counts at replaced sort/comparison frontiers necessarily follow
the new physical vocabulary; do not claim the old number of comparisons ran.

Delete the old directory-sort/uniqueness branch and directory binary search.
Keep ProfileComparisonWork solely for actual ordinal/extra sorting and dedup.
Do not retain a sorted fallback, compatibility mode or parallel test algorithm.

## Physical work vocabulary and public effects

The original goal expressly permits replacing getter-by-getter historical
units with bounded physical actions (original plan lines133-136). This is a
private algorithm change with externally observable work-limit diagnostics.
It must be described as a breaking work-vocabulary revision, not silently
normalized to old successful import_work totals.

Name the removed populations and replacements: directory sort comparisons,
owner-uniqueness comparisons and bounded binary searches become actual slot
initialization, one registration per actual range and one direct owner query.
Capacity sizing walks remain honestly metered. All other original actions,
resources, semantic validation and error/control ownership remain. No dummy
comparisons, literal charge padding or unmetered O(n) construction.

Before measurement, declare the only successful observation difference allowed
between d81 and this candidate: `import_work` reflects those actual physical
population changes. Every other complete non-clock JSON member, original
precision/fields/normal policy and geometry/output must remain literal-equal.
Let N be actual completed owner rows, T actual slots, S original directory-sort
comparisons, U original uniqueness comparisons, Q actual owner queries and
L(N)=bit_length(N)+1 the original prepaid search bound. The precise proposed
delta is `T + N - S - U + Q*(1-L(N))`: actual initialization/registration,
removed old comparisons and revised queries. The fixed at-most-19-class shape
calculation is part of the existing final covered-total action, not a new unit.
Coverage still charges its original one action per range, now also projecting
a bounded owner key. Retain other allocations/accounting actions; inspect their
source to confirm they charge storage/Internal rather than WorkUnits. If they
add WorkUnits, name that exact additional population in a reviewed amendment
before sampling. Do not use unexplained residuals or fit constants to results.

Freeze the control GLB1/8/32 action populations before any candidate sampling.
Original retained raw profiles bind GLB1 N=T=12,S=52,U=11,Q=13,L=5 and GLB32
N=T=136,S=1539,U=135,Q=137,L=9. Their declared deltas are -91 and -2498,
respectively: expected sampled import_work is 11820 and 67834 against the
original 11911 and 70332, once baseline-probe fidelity is qualified. Obtain GLB8 from an actual qualified-control production invocation
with existing test-only population instrumentation; do not guess its sort count
or run a copied reference algorithm. Qualify that fixture-only observation
separately from the immutable ordinary control binary and keep all raw logs.

Retain both actual work totals and exact independently checked population
derivation, not a blanket field exemption. A new collector may compare full
non-clock tagged JSON with only this predeclared checked integer replacement;
all other fields and all non-glTF work remain literal-equal. Do not rewrite old
collectors/receipts. Use real tests for admission, failure-prefix and capacity
accounting as well as successful delta verification. Extra directory capacity
is a real memory cost; no additional observation field exception is declared.
Any undeclared observation difference or unexplained work delta fails.
The ordinary comparison still runs the same task and full timing boundary;
this declaration changes no corpus, output, build, allocator or Assimp policy.

## Required tests and qualification

Use the existing real source-profile directory fixtures on the same finish
API, without deriving a new model shape in tests. Preserve arbitrary completion
order, suspended owners,
shared source relation content, missing-owner behavior, incomplete and duplicate
rejections, malformed/gapped/overflowing/overlapping ranges and unadmitted
ordinals. Private relation-test inspection may iterate the direct directory in
canonical key order; do not reintroduce a production sort solely for a test.

Replace old DirectorySort/OwnerUnique test instrumentation/oracles with actual
DirectoryInit/DirectoryRegister steps; keep every existing RunSort/RunDedup/
OrdinalTransfer/ExtrasSort test. Exercise denied current registration before
slot mutation, caught errors/unwind with no finished map, direct query bounds,
zero-length classes and nonzero offsets, DFS-vs-source node order, and missing
slots. Duplicate registration never overwrites the original map entry.

Test exact boundary cancellation at initialization, registration1/64/65 and
short scopes using actual production loops. Assert actual attempted/consumed
prefixes and that directory publication waits for final control. Derive memory
frontiers from actual capacity and retained reservations; no invented fixed
size or relaxed peak limit. Existing complete_owner capacity/transfer tests
must remain unchanged since their frontier is outside this replacement.

Affected glTF and Import tests, strict Clippy and docs, then original seven
workspace/consumer gates, exact frozen/current/staged/committed/archive hashes,
independent source/QA review. Rust1.95/jobs1/threads1/current repo target and
five protected notes remain. No simultaneous own Cargo or ordinary profiling.

## Prospective measurement and retirement

Fix exact d81 as control. Before observing candidate data: original seven
instruction tasks, one original GLB1/32 `100*N <= 99*B`, all seven
`100*N <= 101*B`; fresh processes/matched GNU opt3/debug1/System/genericCPU/
empty features. All complete literal non-clock observations must agree except
the explicitly declared and independently checked physical-work delta above.
Actual generated-code/owner evidence must show deleted directory sorting/search
and real admitted index construction; a renamed wrapper is insufficient.

Selection earns unchanged Windows/GNU fourteen cases/110 lanes C0/N0/N1/C1,
3 warmup+20 retained serial fresh AB/BA pairs, whole-child memory and separate
fresh representative tracked heap. Include index-memory growth in both
attribution and ordinary comparable peaks. All pipeline/endpoint <=3x/no
confirmed10x and whole-childmemory<=3x targets, signed regressions/noise/missing
cells remain. Keep source only with material ordinary improvement beyond
observed dispersion and no unexplained affected regression.

If either diagnostic or source/ownership contract fails, remove the new
representation by an exact reviewed inverse, retaining failed evidence and
useful behavior tests only with their own qualification. No additional cache,
format bypass or weakened validation to rescue a failing directory. No main
integration/native promotion/push/PR/publication/yank/cleanup follows here.

## Unresolved review questions

- Do normalized compact IDs satisfy the coverage-derived totalSlots<=N bound
  across defaults, selection gaps, interleaved geometry/morphs and animations?
  Preserve duplicate/range errors and bound corrupt sparse keys before allocation.
- Do new O(n) slot initialization/registration and one allocation plausibly
  leave >=1% GLB32 gain from the measured envelope without mispricing costs?
- Is the declared physical-work semantic delta faithful to original goal
  authorization, with a concrete complete-observation comparison/derivation
  rather than a blanket work-field exemption?

Independent architecture/correctness review must resolve these before Rust.

## Closed review amendments and staged execution

The exact private frozen experiment spec currently requires unchanged profile
algorithm/work count. Preserve its bytes in a named before snapshot, then amend
only that directory-specific clause for this candidate before Rust/probes.
Keep ordinal sorting/dedup, semantic results, all other observation members,
collector/corpus/comparator/build policies and every original release gate.
Use a separately reviewed standalone full-observation delta verifier; do not
edit frozen run_asset_profile.py or old experiment collectors/receipts.

Execution is staged, with one current checkout/target and serial Cargo:

1. Add only small cfg(test) population hooks to the actual old production
   directory/query paths and a real original-corpus GLB1/8/32 fixture in both
   file and memory mode. No copied sort, alternate model or forced benchmark
   path. Record source/input/options identities and baseline work, coverage,
   actual sort/uniqueness completions and query-bound population. A targeted
   probe run is not a new old-source full-QA/ordinary campaign. Freeze its logs
   and independently authenticate these actual populations before the dense
   algorithm replaces them. Candidate sampled binaries contain no hooks.
2. Implement only this private directory and migrate its production-loop tests.
   Keep the probe useful by observing actual new init/register/query events and
   comparing the frozen old-population remainder. For each original GLB1/8/32
   file/memory request, assert `Wb-(S+U+Q*L) == Wn-(T+N+Q)`. Counts are actual
   physical observations, never metering authority. A=H=0 is the fixed contract
   for storage-growth/sizing WorkUnits; the ordinary capacity reserve/resize
   path only changes storage accounting and control. Any deviation fails and
   requires a new prospective source-contract decision before sampling.
3. The probe imports the original bytes with the same FormatSet/request/layer/
   field/normal/precision and budget setup as the comparison worker. Its
   counters are read without changing policy or adding extra production
   queries. Bind the import_work snapshot boundary before stage-specific
   preparation/extraction, then reuse its qualified mapping across all 24
   glTF ordinary lanes. All 86 non-glTF lanes and five non-glTF diagnostic
   tasks retain exact delta zero. Unknown or differing configurations fail.
4. The new verifier rejects duplicate/missing keys, requires exact key sets and
   nonnegative u64 integer work, validates both raw work values and the frozen
   signed delta, then compares the entire tagged non-clock objects with only
   the verified candidate work token replaced in a temporary comparison copy.
   Preserve raw observations; report historical literal equality truthfully
   (false when work changes), separate all-other-member literal equality and
   exact-work-delta equality, and their named versioned conjunction. Exercise
   positive/negative/zero, off-by-one, clock-only, capacity/key/number drift,
   duplicate/missing work and wrong configuration cases before collection.
5. Independent exact source/QA review, actual probe review and verifier review
   precede measurement. No script framework, Rust-source proof engine or
   additional production metadata is needed. Freeze new source/binary/helper/
   work-derivation identities and retain all failed attempts.

The one new up-front empty-vector reserve requests the existing growth-policy
envelope (currently 16/256 usize slots for logical 12/136); actual returned
capacity and slack determine charges. S<=N bounds logical slots, not allocator
capacity. Include 20 inline offsets, the Option/vector header, original live
ranges/ordinals and added reservation in layout/peak accounting. Do not hide
these in output.capacity_bytes or claim no additional memory. Empty logical
directory has no new backing allocation. New reserve polls are explicit control
frontiers and remain inside the unchanged ordinary clock.

The original real shared-primitive fixture independently has one mesh, two
nodes/instances, one default material, p primitives and 3p attributes: 4p+8
owners and 22p+10 retained relations. Use these facts rather than reproducing
the production maxima fold as the test oracle. Preserve the real DFS, filtered
selection, implicit sampler/material and retained/omitted animation fixtures.
Add high/sparse-key and mixed range/domain-fault tests, ensuring later invalid
ranges win over a prior recorded domain overflow. Allocation/qualification
failure, denial and cancellation must retain no published partial map.

Amendment provenance: gltf-dense-owner-directory-plan-review-1.md and
gltf-directory-work-vocabulary-review-1.md; their exact hashes are bound in the
prospective decision receipt. They approve a bounded hypothesis with these
requirements, not a measured improvement, source retention or main/release.

Published-copy provenance: private draft3 SHA-256 `8587afbaa853fd1d2bf707dd5995c259b52c9235b2ee0fff9dcc3d697da8b46a`.
Only the initial status and this provenance paragraph differ. Actual closed
review, spec-before/after and data-free work-contract hashes are retained in
`gltf-dense-owner-directory-work-contract-1.json`.

## Stage1 topology clarification after bounded amendment review

This section supersedes the earlier identical-FormatSet/native-worker wording
for the test-only population probe. Adding other format or native-I/O dev
dependencies solely to instrument a glTF unit binary is unnecessary.

A: the cfg(test) probe uses the real GltfImporter and a single-format registry,
original exact GLB1/8/32 fixture bytes, original default request/options and
200,000,000 work bound. It invokes read_bytes and MemoryAssetIo.read_asset.
Only actual postselection directory sort/uniqueness/query/coverage populations
are mapped. This is neither full-builtin registry nor native filesystem I/O,
and its total import_work is never substituted for a worker total. Hooks are
test-only, use no new public feature, add no production query and contain no
copied sorting algorithm or alternative model graph. The three tiny original
analytic fixtures retain exact input hashes so the test runs without an
existing target corpus; register their bytes in frozen QA inputs.

B: six fresh original d81 ordinary-worker functional processes use the actual
full builtin formats and native file/memory paths and original task request.
They retain actual global import_work, source/input/executable/log identities,
and no accepted timing measurements. The completed control functional receipt
is gltf-directory-control-functional-1/report.json. It records:

| Original case | Native file work | Memory work | Directory delta |
| --- | ---: | ---: | --- |
| GLB1 | 29964489 | 11911 | -91, pending fresh A qualification |
| GLB8 | 29977342 | 24764 | UNAVAILABLE until actual A populations |
| GLB32 | 30022942 | 70332 | -2498, pending fresh A qualification |

Both registry registrations select GltfImporter::parse -> parser::read_gltf ->
the same selected-source/normalized owner path. Bind source and postselection
inputs/owner facts to map A's directory-only populations to B; registry and
I/O overhead remain distinct. GLB1/32 original raw profiles cross-check those
populations; GLB8 must run the actual old production algorithm. Unknown
configuration mapping fails.

Before performance sampling, repeat A on the candidate and the same six B
functional configurations, then require each full-worker exact work remainder
identity from the frozen vocabulary. File and memory absolute totals have
separate rows; only the verified directory population delta is shared when
postselection ownership matches. All stage clocks, tasks, original budgets,
output/capacity and every other observation member remain unchanged.

The bounded amendment review is gltf-owner-directory-amendment-closure-review-1.md.
Its conditional GO_STAGE1_READINESS follows this documented clarification,
not an assertion that A/B qualification or the dense implementation passed.
The versioned prospective vocabulary is now bound by
`gltf-dense-owner-directory-work-contract-2.json`; version1, draft3, both AMEND
reports and their original hashes remain historical inputs. No additional
dependencies, production feature or collector rewrite is required.
