# Own Finite Invocation State at the Actual Root

Status: architectural direction and contract amendments independently reviewed GO.
The bounded `owned-finite-invocation-plan-amendments-review-1.md` closes all three P1
terminal/error omissions and the P2 disjoint-loan issue at the plan-contract level.
The first dynamic compiler cut passes all seven integrated gates: 3,664 workspace
tests, 191 doctests, 138 assets-consumer tests, both strict Clippy checks and both
format checks. Its [implementation report](../reviews/2026-11-06-owned-dynamic-invocation.md)
preserves failed attempts, source-review scopes and the limited layout observation.
Independent final-receipt review is GO for that source; the real finite compiler cut
remains required.
No finite root implementation, allocation improvement or native keep is claimed.
The [preparation producer-result slice](2026-11-06-1225-prepare-consuming-result-plan.md)
is separately recorded work. The parent convergence goal remains active and release HOLD.

## Decision and source authority

Use two concrete owned invocation roots: finite postprocess owns fixed arrays; dynamic
Import owns its actual admitted buffers. Share the work, storage, control, progress and
terminal algorithms through short borrowed views. Do not propagate an external backing
lifetime through every continuation, add a collection/provider parameter to Import, or
put the large finite payload in a common owning enum. An owning finite/dynamic enum
would pad dynamic values; boxing its finite variant would add mandatory allocation.

The source-design report `finite-invocation-consuming-interface-1.md` and independent
`finite-invocation-owned-interface-review-1.md` under
`.git/baozi-review/performance-convergence/` inspect runtime `6e7c1c0`. Neither finds an
actual borrowing edge requiring an independent external array owner. This supersedes
the old external-loan representation in the lexical-backing draft while preserving
its ownership, accounting and cleanup duties. The old whole-Ledger terminal transfer
is an internal API to replace, not a reason to keep external backing.

The current glTF-1/STL-1024 instruction profiles locate approximately 42% of unchanged
preparation Ir in planning/binding. These inclusive contexts include required checks;
they are not expected savings or native CPU percentages. The producer slice removes
one output owner and rediscovery. Fixed startup, two nonempty selection scratch owners
and terminal evidence still require their own changes.

## Concrete state and interfaces

The finite invocation owns six initialized dimension/work slots, three ledger phase
slots and four initially empty optional progress slots. Only semantic constants and
shape facts are static. The actual domain, fresh request, current budgets/policies,
work header, storage/publication state, progress occupancy, debt and first cause belong
to each invocation. Keep one common control/cause/lifecycle payload; no second ledger,
counter copy, mutable global cache or alternate numerical backend is introduced.

Finite and dynamic storage each own an Inline movable `Option` payload or the actual
Shared handle. Short storage selectors contain references to these owners. `WorkView`
continues to borrow the real catalog/rows/header; storage arithmetic takes corresponding
short header/row/phase views. Progress fields hold no persistent second storage owner.
A report temporarily borrows the one storage authority and disjoint progress/poll
fields, preserving both existing polls, insertion order and observer delivery.

At outer entry, split the finite/dynamic root into its invocation-state slot and the
existing separate assembly/retained/host payload fields. The inner state selector
borrows only that slot. It must not retain `&mut whole_root` alongside mutable loans of
the same root's physical fields, or carry physical F/C/E payload types through numerical
views unnecessarily. Existing invariant execution brands and short `'inv`/`'op` loans
remain; no field points into another field of its owning host.

Finite plan construction returns a domain, fixed key/policy descriptors and phase keys
without first constructing `LedgerLimits`/`StoreLedgerPlan` Vecs. Qualify immutable
catalog order/membership/shape at their owner boundary. Retain current source/account,
start-plan relationships, irreversible start latch, failure-plan qualification, start
admission, actual construction, terminal catalog and protected-work configuration in
their established order. Current tightened policies and mutable state cannot be
replaced by initial facts. Starting/Running/FailedStart use one exclusive owned state
slot; failure never recreates unused start authority.

Core's finite scene-edit bridge stays generic over existing operation owner/progress
types and accepts operation-layer hosts/plans/control. Postprocess specializes those
types; Core must not import postprocess aliases. Dynamic Import's public concrete
constructors, error and evidence types acquire no finite owning variant or backing
generic.

## Mandatory terminal and error amendments

The independent review identifies three concrete P1 omissions and one disjoint-loan
issue in the earlier sketch. Adopt these corrections as part of the first relevant
integration, rather than shipping adapters around the old ownership.

1. **Retain actual state when a terminal permission is forgotten or rejected.** Arm
   the real root state slot as terminal-outstanding before lending the short terminal
   permission. Its successful detach or actual containment consumes/disarms that slot
   exactly once. A forgotten permission leaves the slot armed, so root cleanup takes
   the same fail-closed disposition. On uncertainty, quarantine the physical graph
   before retiring its accounting cover, then consume the actual finite/dynamic state
   into owned residue or retain that actual owned payload under the existing quarantine
   protocol. Forgetting a borrowed proof alone is insufficient: it would leave the
   root's actual work/storage/progress/domain to ordinary field destruction. Do not
   allocate, promote or replay admission during unwind. Preserve the first cause.
2. **Finite Start/Body errors carry actual residue.** Give the finite body boundary a
   distinct concrete error carrying the existing typed cause and optional owned finite
   containment. Transfer it after constructor-candidate, body/capture and host-payload
   disposal, while the original account admission still covers cleanup. Move residue
   into postprocess's private failure owner; public resources remain absent when no
   qualified facts exist. A Copy containment cause or scalar Start error is not an
   owner. Clean early failures need no invented payload. Dynamic Import's common error
   remains free of a finite owning variant. Every finite entry and every Start/Body
   return must use this carrier; no caller-specific optional host drain is accepted.
3. **Migrate all three whole-Ledger transfers and the pre-freeze classifier.** The
   actual transfers are `retained/operation.rs:2673`, `:2745`, and sibling
   `retained.rs:3237` at the inspected baseline. The third path reaches real changed
   joint success as well as rich failure/assembly fallback. Thread the short terminal
   state loan through the pre-freeze classification, primary/fallback attribution,
   typed qualification and discard guards until actual detach. Detached proof,
   publication and output stay owned. Do not satisfy an old signature by constructing
   another coordinator/Ledger, copying live counters or restarting work. All guard
   rejection/Drop routes use correction 1.

The disjoint state-slot/physical-field split described above resolves the P2 aliasing
issue. An outer root selector may exist at entry; an inner whole-root borrow may not
overlap retained/assembly permissions. These are concrete owner transitions, not new
proof/provider frameworks.

## Promotion, terminal facts and administration

At the first genuine escaping storage lease, complete current key/control/work/account
admission and admit known Shared payload/required capacities and overlap. Allocate an
unissued vacant Shared shell and finite-to-owned row capacity while Inline remains
intact. Qualify source presence/shape, destination vacancy/uniqueness/poison and all
capacities before taking the owned source Option. The subsequent move/install/select
transition performs no allocation, callback, fallible conversion, reserve or user Drop.
Only then issue the first Arc/Weak/reservation. Dynamic buffers move without replay;
later tokens use the same Shared identity. Independent zero-byte tokens still promote;
tokenless empty containers do not. Reject candidates drop before admission refunds.

Consuming finite close returns independent clean facts or actual dirty containment.
Clean means structurally clean ownership, including a possible semantic failure. Facts
own the one original domain, qualified header/cause/key mapping and truthful optional
six dimension/three phase observations. Earlier startup failure preserves absent or
partial rows. Dirty zero-counter open frames are still dirty; actual work/header,
Inline/Shared storage, progress and unresolved candidates move to containment, with
no execution/retry API or outgoing reference to the expired host.

Changed edits retain genuine canonical/remap/outcome/fallback owners and the same joint
generation/account commit. Freeze finite observations at the qualified terminal
boundary. Postprocess resources project from their one finite facts/containment owner;
delete eleven duplicated counters plus the generic ledger retained only for getters.
Dynamic Import keeps its genuine variable evidence and real physical owners.

Rename the existing evidence-byte observation to known administrative payload/capacity.
Count actual owned Vec capacities, the known Shared T payload and live candidate
overlap exactly once; exclude opaque Arc/runtime/allocator overhead explicitly. Report
inline root/fact/error layout separately. Observe before real teardown, and release
capacity credit only after its actual buffer dies. Replace forced-zero snapshot facts
and update the actual retained freeze validator (`retained/evidence.rs:742`), which
currently rejects all nonzero metadata. Accept only exact structural coverage of
permitted remaining metadata while keeping physical, work, reservation, pending and
publication gates. Positive metadata alone is neither cleanliness nor a leak finding.

## Two integrated compiler cuts

All source changes and builds stay in the current repository; all build output uses
`F:/SourceCodes/Rust/baozi/target`, Rust 1.95.0, jobs1 and nextest threads1. Main serializes
all Cargo/profiling/ordinary work. Source workers receive disjoint exclusive files.

| Cut | Actual integration and mandatory deletion |
| --- | --- |
| Dynamic owner and complete borrowed terminal rail | Common slice storage kernels, one mutable storage authority, storage-free progress/candidate admission, actual start-error disposition and every borrowed terminal/classifier/guard path. Remove old progress storage bindings and whole-Ledger transfer at all three callers. Preserve concrete dynamic Import types. |
| Real finite scene-edit owner and every exit | Fixed plan/root/host, once-only Starting/FailedStart, promotion, distinct finite facts/containment/error and actual Core/postprocess entry/finish/private error plumbing. Remove Vec-backed finite startup/terminal ownership and temporary adapters; keep the one shared operational implementation. |

The first cut's inseparable operation scope includes `ledger.rs`, relevant `ledger/**`,
`coordinator.rs`, **`retained.rs`**, relevant `retained/**`, `concrete_store.rs` and its
host/driver/export glue. Split numerical/storage files from lifecycle files only after
the exact shared view interfaces agree; neither half alone is a claimed compiler
checkpoint. Mechanical observation consumers may follow that signature. The second
cut assigns Core entry/terminal glue and postprocess runtime/resource/private errors to
separate writers only after operation interfaces exist. Do not mix selection or the
current producer algorithm into these ownership cuts.

Qualify actual parser nesting/storage, all three changed/failure terminal rails,
unattached finish, pre-detach rejection/Drop and owned detach in the first cut. The
finite cut must instantiate unchanged, semantic rejection, partial constructor failure,
Shared scratch, changed joint success, rich/minimal fallback, fatal body and forgotten
permissions. Test literal current-policy/budget/overflow/first-cause traces, four-slot
occupancy and delivery, promotion denial/identity/poison, actual candidates dropping
before refunds, exact dirty zero-pending frame state and facts/residue after host/source
exit. Migrate real positive/compile-fail brand/context/parser/thread contracts; missing
symbols are not ownership proofs. Observe concrete dynamic root/context/output/error
layouts; separate owners alone do not prove size isolation.

Review, simplify and run affected gates, then all seven integrated gates for each
selected complete source cut. Freeze source/worker identities before diagnostic and
ordinary comparisons; retain failed attempts, capacity changes and every unfavorable
sign. The original Windows/GNU 14-case/110-lane, three warmup/twenty retained AB/BA pair
protocol and all release targets remain. A structural compiler cut is not a native
performance keep. Allocation-free empty/single unchanged preparation additionally
requires the later indexed selection/zero-single scratch owner cut; this plan does
not claim that fixed arrays alone eliminate all escaping scratch.
