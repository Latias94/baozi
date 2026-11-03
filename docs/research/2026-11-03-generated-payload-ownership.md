# Generated normal payload ownership: design audit

2026-11-03. Read-only architecture investigation; implementation is not authorized.
Source references use `8f89e4fafc287fc785a90bb25aa0b6f87d16f3a7`. The owner paths cited here
are unchanged from measured R5 `57c33b2447a296c28f5f5817a36debd248bd4280` (checked with Git).
This proposal is independent of exclusive v3, node-view binding, codegen hints and glTF work.
No Cargo, formatter, build, test, profiler or measurement was run; no candidate packet changed.

## Recommendation

**Adopt the allocation through a core-owned immutable typed payload module.** The current
borrowed replacement interface and owner-global scalar pools force reconstruction; neither is
a fundamental normal-generation requirement. Keep the existing numeric kernel and its checks.
Have core validate the generated values at the current semantic-preflight position, freeze
their exact owner, and let central validation compose that value proof with actual attachment,
source correspondence, schema, graph and evidence checks. This is a storage/interface refactor,
not permission to trust a producer's assertion that its values are valid.

Timeless requirements are typed validity, exact correspondence, source binding, truthful leases,
actual work prefixes and controlled publication. Replaceable interfaces are borrowed-only edits,
global scalar pools and scalar replay. Coarse actions primarily reduce authority traffic.

## Actual route and why it walks three times

Under `crates/`: `core/` = `baozi-core/src/canonical_scene/store/`, `pp/` = `baozi-postprocess/src/`, `op/` = `baozi-operation/src/`.

| Stage | Source evidence | Actual ownership/obligation |
| --- | --- | --- |
| Generate once | `pp/normals/input.rs:37-51`, `:167-186` | `GeneratedNormals` owns `ScratchVec<[f32;3]>` or F64; other kernel scratch is dropped. |
| Final orientation | `pp/normals/input.rs:54-76`; `pp/prepare/input.rs:340` | Mutates generated values; freezing before the last write would invalidate a proof. |
| Erase ownership | `pp/prepare.rs:295-302`; `core/typed_storage/edit/selected.rs:37-47` | Replacement retains only borrowed `VectorValues` (`SceneVectorValues` integration alias), not its allocation or lease. There is no existing `GeneratedVectorValues` owner. |
| Plan | `core/typed_storage/edit/selected.rs:115-217` | Checks target/dependencies/local values, then `reconstruct(Mode::Count)` obtains nine capacities, mappings and exact planned work. |
| Build | Same file `:263-308`; `construction/node_scene/edit/graph.rs:92-100` under `core/` | Admits new geometry storage, allocates nine Vecs, reconstructs into them, verifies counts/work/capacities. |
| Correspondence | Same selected file `:311-353` | Reconstructs with `Mode::Compare` against actual output, including relocated scalar spans. |
| Central validity | `core/construction/node_scene/edit/transaction/validation.rs:65-115` | Checks brand/generation/facts/rooted evidence, correspondence, then actual canonical graph. |

`generated_normal`/`generated` (`selected.rs:1018-1081`) flatten borrowed triples and call
`Writer::f32/f64` for every scalar in all three modes. `selected/writer.rs:157-194` counts,
pushes or compares through the shared traversal; `:241-244` compares floating-point **bits**.
Count plans work; it neither writes physical output nor runs the numeric kernel again.

There are additional value checks, outside those three generated-payload walks. Local generated
normal validation (`selected.rs:555-570`) checks finite components and sequential F64
sum-of-squares, with tolerance `4*f32::EPSILON` or `8*f64::EPSILON` (`edit.rs:56-68`).
Central geometry validation independently checks schema/cardinality and finite payload scalars
(`typed_storage/validation.rs:183-212`, `:376-450`, `:468-478`). It does not require every
authored Normal to be unit length. Preserve that distinction.

The kernel's `Output::publish` (`pp/normals/kernel.rs:320-344`) uses a different, fused
`mul_add` length expression and `8*epsilon`, after width rounding and zero canonicalization.
**Its success cannot substitute for the existing core normal predicate.** Neither predicate,
orientation, rounding, accumulation order nor precision should change in this owner refactor.

## Concrete storage obstacles and usable authority

`core/typed_storage.rs:214-239` stores only scalar-pool ranges in dense/sparse payloads.
`GeometryStorage` (`:333-342`) owns one Vec per scalar kind containing positions, generated
normals and other attributes together. Moving a standalone normal Vec cannot append its
allocation into an already occupied Vec. `semantic/geometry.rs:177-224` under
`core/construction/node_scene/` resolves those ranges into the public borrowed reader.

`ScratchVec` (`pp/operation/runtime.rs:330-400`) admits before allocation, qualifies actual
capacity with `CapacityPlan::finish`, shrinks accounting, and destroys storage before its lease.
It has no consuming handoff or retained plan; plain `into_vec` would sever the physical lease.

Existing operation mechanisms make a narrow handoff plausible:

- `op/concrete_store.rs:1910-1923`: `RetainedStoreReservation::bind_storage` consumes backing
  plus physical lease into `RetainedStorage<T>`, preserving operation attribution without a
  new owner-header allocation. This trusted-adapter method is not a typed capacity proof.
- `op/concrete_store/execution.rs:345-381`: live-owner validation and `transition` preserve
  physical backing while moving attribution; they reject foreign domains or excess bytes.
- `op/concrete_store/activation/typed_storage.rs:136-152`: a prepared aggregate may own
  separately retained children without charging their backing a second time. Existing profile
  storage uses this composition (`core/construction/node_scene/edit/profile.rs:883-905`).
- `op/retained/allocation/storage.rs:25-44`: retained owners deliberately prohibit extracting
  or mutably borrowing payloads independently of their lease; shared immutable owners exist.

`try_retain_storage` requires zero-source plans (`op/concrete_store/activation/typed_storage.rs:182-195`).
It admits another allocation; a role-checked canonical adoption operation is still required.
Existing storage methods alone neither certify a Scene nor permit publication.

## Proposed small interface and proof composition

The following names are conceptual, not existing types or a proposed generic framework.

1. **One concrete generated-normal buffer owner.** Move the normal-only `ScratchVec` allocation
   responsibility into a core integration owner carrying its exact capacity qualification,
   physical reservation and issuing invocation. Lend the same typed mutable triple slice to
   the existing kernel; keep order/cross scratch as scratch. Borrow it read-only for tangents.
   This preserves the allocation from birth through publication without accepting a public
   unrelated-Vec-plus-reservation constructor.
2. **Freeze mutation; validate in the current order.** Consume mutable access after generation,
   orientation and dependent preparation. At the existing preflight normal-validation point,
   core runs the exact existing predicate, under admitted bounded element actions. Only complete
   success can mint an immutable value certificate covering the whole initialized slice.
   Preserve target/dependency/precision/origin-check order (`transaction/selected.rs:61-164`,
   `typed_storage/edit/selected.rs:458-570`). An empty slice cannot prove target cardinality:
   current local loop is vacuous when empty; central cardinality must still reject a mismatch.
3. **One payload representation.** Replace implicit global-pool references with typed immutable
   block ownership plus checked ranges. Imported pools may each become a block; a generated
   normal allocation becomes another block of the same scalar kind. Dense and sparse shapes,
   indices/defaults and all four scalar kinds use the same resolver and public core sampler.
   Keep one reader, contiguous payload borrows and direct allocation owners, not parent-Scene
   chains. No special generated-normal reader or per-attribute block allocation is required.
4. **Move, do not copy, scalar storage.** Safe `Vec<[T;3]>::into_flattened()` can retain the
   allocation for F32/F64. Installed Rust 1.95 `alloc/src/vec/mod.rs:3566` implements this;
   workspace MSRV is 1.95 (`Cargo.toml:15`). Preserve tuple/scalar unit conversion explicitly:
   capacity `c` triples becomes `3*c` scalars with identical bytes, not a fresh allocator result
   or a new rounded envelope. No Baozi raw-parts cast, spare-capacity read or unsafe conversion.
5. **Linear canonical attachment.** A consuming selection transaction owns frozen payloads;
   replace plans' borrowed generated slices with sealed attachment records. At admission verify
   exact invocation/account, request, source generation/primitive, output slot, Normal/Corner,
   width/lanes, full range and length. Pre-admit any handle table and shared header. Transfer
   attribution through the existing authority projection; move each owner once, after admission.
   Postflight retains a private expected-owner receipt, not a second value buffer or mutable alias.
6. **Central proof remains authoritative.** Central validation checks the actual attachment
   against that receipt and the immutable owner's certificate, then the real schema/cardinality,
   dependencies, remap/profile/evidence and graph. Identity must be the allocation owner plus
   range/type/binding, not a data pointer (especially for empty buffers), hash alone, index alone
   or caller-supplied flag. A certificate of value validity is not a whole-Scene proof.
   The exact core predicate supplies finite-value facts once; central validation composes those
   facts instead of rescanning those same immutable scalars. This must be an explicit checked
   proof rule, not `if generated { skip_validation }`. Uncertified values still need the shared
   validator before a certificate exists; no second validator or numeric implementation.
7. **Correspondence becomes ownership evidence where valid.** Current correspondence proves
   requested bits survived reconstruction; it does not recompute the normal algorithm.
   Attaching that exact immutable full-range owner proves those bits without a Compare replay.
   This proves nothing about unchanged fields copied/remapped elsewhere: retain their source,
   origin, sparse/default and bit correspondence checks. Relocation of handles must remain valid;
   substituting another owner/range requires its own established correspondence, even if values
   happen to compare numerically equal. Never infer correspondence just from schema validity.

## Work, storage, failure and control contract

Derive scalar cardinality in O(1) with checked `len*3`; value checking remains O(M) for M
normals. Retain one `ValidateNormalElement` action (three scalars, existing finite/unit predicate).
Attachment moves one prepared owner into one reserved slot; allocation, user callbacks and
destruction occur outside it. No payload/selection scan becomes one unit. A later 32-record
action needs a literal finite bound and conservative control debt; adoption does not need it.

Admission precedes each physical action; a normal return, including a returned validation
failure, completes that action immediately. Denial completes nothing; unwind mints no
certificate and fabricates no completed work. Retain actual earlier prefixes, original fault
priority, policy tightening, role/mode revocation, and no ledger guard across action/control/
allocator/drop. Check cancellation on entry, at the existing finite cadence and final freeze.
Removed scalar actions disappear from a newly versioned semantic catalog, budget meaning,
evidence work fields and reports; never pre-complete their old counts or widen budgets.

Transfer must keep actual capacity continuously charged. Validate the live owner before
erasing its producer type; preserve same-invocation/source binding beyond account equality.
Move scratch attribution to its permitted retained destination, admitting that destination
and any headers first. Check live and published limits without refund/reacquire fiction.
`pp/operation/runtime.rs:137-147` gives ScratchBytes no published allowance and CanonicalBytes
an explicit one: a retained scratch lease alone is not publishable canonical storage.
Charge each child allocation once, the outer metadata separately, and every shared handle's
required attribution. Preserve original envelope policy and actual spare capacity; do not
charge only `len`, double-charge physical bytes, or report a temporary lease as released.
Sharing coarse original blocks can retain obsolete scalar ranges: account their entire live
capacity and do not promise universal memory reduction or add unbounded retained ancestry.

Only new generated payloads move; `prepare::run` retains the immutable source Scene. Ordinary
failure destroys candidate children before releasing leases and returns that original Scene,
generation and bits (`pp/prepare.rs:380-414`, `:501` onward). Known rollback releases verified storage; unresolved
panic/drop ownership retains conservative charges/contract faults, as existing retained owners
do. Keep activation, central rejection and rich/minimal terminal routes. Final control and
freeze/handoff remain mandatory (`pp/scene_edit.rs:304-384`); a value certificate grants none
of these permissions. Merely binding/promoting a payload is not a successful public operation.

## Opportunity bounds and comparison

R5 repeated raw SHA-256: `3937f0d4df55b8c517cb02ffa67cfa4d7314067c04ce518dc59c4256c24d5ef4`.
`target/review/convergence/r5-attribution/obj-repeated-16384.both.txt:1470-1477` confirms
three `generated_normal` invocations under Count/Build/Compare. Existing reconciled attribution:

| Measured owner | R5 instructions / observed calls | Interpretation |
| --- | ---: | --- |
| Scene edit | 286,570,373 Ir | Includes other tables, evidence and validation. |
| Three generated-normal walks | 159,613,541 inclusive; 18,432,414 exclusive Ir | Upper owner envelope, not zero-cost-removable instructions. |
| Their semantic/build/compare children | 100,147,044 / 28,608,304 / 12,425,779 Ir; 147,458 calls each | 147,456 scalar visits plus metadata per pass; nested in preceding rows. |
| Entire core semantic adapter | 180,475,848 Ir; 262,189 fresh lifecycles | Also handles selection, indices and profile work; overlaps the payload row. |
| Its admit/complete/finish children | 135,289,524 Ir combined | Lifecycle opportunity belongs partly to separate exclusive-v3 work. |

The fixture has 49,152 F32 triples = 147,456 scalars = 589,824 logical payload bytes; scratch
and copied output coexist (`pp/prepare.rs:380-395`). Adoption targets three scalar replays and
duplicate backing; core value validation and attachment/proof/metadata work remain. Additional
central finite checks are outside that row; no per-normal measured saving is assigned to them.
Block/header/table capacities determine net memory. Linux Ir does not predict native savings
or allocate the Windows gap to a time percentage; normal-math asymptotics are unchanged.

| Choice | Removes | Leaves / scope cost |
| --- | --- | --- |
| Closed fixed-triple actions only | Many per-scalar authority calls; checked-length planning can separately avoid Count scalar enumeration | Build/Compare value walks and duplicate backing remain; smaller writer/catalog change. |
| Owned payload + immutable proof | Count scalar replay, generated copy and generated Compare replay; repeat validation via explicit proof composition | One actual core value check plus graph/attachment checks; deeper storage, ownership and accounting migration. |

## Bounded implementation and meaningful qualification scope

Required owners: `pp/normals/input.rs`, `prepare.rs`/`scene_edit.rs` ownership; core integration
exports/generated buffer, `typed_storage.rs`, selected plan/writer/storage, production geometry planner/builder,
`construction/node_scene/semantic/geometry.rs`, edit graph/transaction/postflight and central
validation; every typed-geometry edit producer, including space conversion, must resolve the
same block representation. Operation needs only narrow typed adoption/attribution composition
beside existing retained storage; retain its single admission projection. Migrate direct tests.
Delete borrowed generated-normal reconstruction/Compare routes once replaced; retain borrowed
public readers and unrelated source-remap validation. No legacy generated-payload backend.

Qualification must include literal F32/F64 bits and signed zero, exact existing normal tolerance
boundaries, nonfinite/invalid length, empty/absent distinction, and proof refusal after partial
validation. Reject wrong invocation/source/target/slot/domain/width/range and owner substitution;
allow handle relocation; prove mutable access and duplicate attachment are unavailable after freeze.
Preserve sparse indices/default bits and pointers, all
scalar kinds, topology origins and tangent/morph dependencies through the one public reader.
Inject denial/cancellation/policy tightening before validation, attachment, central proof and
final freeze; verify immediate prefixes, no certificate on panic, original Scene on error, and
no optimistic charge release. Test actual allocation/capacity reuse, spare-capacity envelopes,
header/transfer denial, child drop exactly once, source-first/output-first drops and sharing.
Evolve `selected/tests.rs:204`, `:292`, `pp/prepare/cancellation_tests.rs:27` and
`pp/normals/runtime_tests.rs:97-143` around these contracts, not old scalar work totals.
Review the proof/lease transition before implementation; only later source-qualified ordinary
timing and refreshed attribution can establish a keep. No current performance result clears it.
