---
title: "Keep lexical invocation authority in an outer host"
type: refactor
status: in_progress
date: 2026-11-03
parent: 2026-11-03-1120-refactor-exclusive-invocation-plan.md
---

# Lexical Invocation Host and Consuming Attachment

This is the connected root/attachment replacement from the
[exclusive invocation plan](2026-11-03-1120-refactor-exclusive-invocation-plan.md).
The [bound-services result](../reviews/2026-11-03-bound-storage-performance-hold.md)
remains HOLD: reduced construction instructions did not establish an ordinary performance
keep, and canonical action traffic, parser state access and small-operation startup remain
measured owners. The unchanged 14-case/110-lane Windows/Linux release gates remain open.

The user authorizes breaking interfaces, ownership changes and removal of obsolete code.
There is no publication, remote push/PR, native promotion, benchmark task change or numerical
algorithm change in this cut.

## Reviewed Ownership Decision

Fixed postprocess arrays and dynamic import Vecs live in lexical roots. An actual outer host
owns the one coordinator/control, unique work view, storage selector, start admission, failure
cause, progress/debt and publication lineage. It also owns the real retained baseline, tables,
candidate output, proof and rejection/fallback stage data. Callbacks borrow short permission
views; consuming a view changes those host-owned stages. No view owns the final recovery
authority or exposes a raw-owner extractor, finalizer or `DerefMut` into the host.

This corrects a rejected by-value-owner design: `mem::forget` of that actual owner could lose
control and cleanup authority. A disposition bit, poison anchor, owning-view Drop or HRTB
alone cannot recover it. Forgetting a permission leaves the actual state in the outer host.
An unresolved exit fails or contains; forgetting an inert view after a valid disposition
request can be harmless. Leaked genuine scratch remains charged and prevents clean finish.

Keep the qualified `work + StoreExecutionServices` split and short `services.ordinary()`
loans. Numeric mutation uses the existing `WorkView` kernel over unique row/header slices.
Replace `LocalWork` and its real permit/parser consumers rather than introducing a permanent
borrowed/owned compatibility backend. No full-Ledger alias accompanies exclusive work.
True independently released storage remains synchronized and may escape or move across
threads; invocation, recovery and candidate permission views may not escape the root.

The concrete outer entry calls a retained `FnMut` callback exactly once under controlled
unwind handling and accepts no arbitrary returned `R`. Captures remain owned outside that
call, then are explicitly disposed while the real host is armed. Postprocess/import supply
their existing concrete output/proof/cause specializations; operation gains no dependency
on either crate. Owned-only final outcomes leave the entry after private driver finalization.
Inner bounded actions can still return values under their existing private host guards.

## Attachment, Storage and Finish

- Acquire `PendingOperationStartAdmission` once, before execution/storage effects. Attachment
  accepts no replacement source, account, control, limits or domain. Preserve account/source
  checks and recheck actual lineage/poison eligibility without another operation-start action.
- Qualify attachment against actual current work, pending competition, live scratch, tightened
  policies and phases. Use the current-state startup predicates; zero-state qualification or
  replay into a new Ledger/coordinator is incorrect. Preserve the real six startup reservation
  specs, four account-share steps, manifest and protected-work eligibility.
- Split common from existing retained/assembly/failure/freeze fields. A private attachment
  frame stages their actual owners while common stays in its host field. Ready installs only
  fully qualified fields in a nonallocating/non-callback transition. Record clean unattached
  failure, eligible fallback or unresolved containment before returning a permission result.
  Ignored errors latch the real first cause and stop ordinary work before returning `Err`.
- At the first genuine escaping reservation, admit and allocate an unissued vacant shared shell
  and required fixed-to-owned row capacity before moving authority. Qualify actual capacity
  and unique ownership, install complete state, then issue tokens. Dynamic Vecs/charges move
  once. Preserve every counter, peak, policy and phase. No token observes vacancy or a second
  account. Zero-byte tokens are still real tokens; no no-op allocation claim follows yet.
- Rename administrative limits/evidence to explicit payload/capacity bytes. Cover known
  payloads, Vec capacities and overlap once, and report inline root/fact layout separately.
  Opaque standard Arc headers, runtime and allocator overhead remain unmeasured. Existing
  retained requested-layout/strong-owner charges remain intact. Replace forced-zero metadata
  projections with exact allowed-coverage validation, preserving all live-scratch, pending,
  account and publication gates.
- Split scene-edit validation from private outer finish. Store actual success, rejection,
  interruption or live `PendingMinimalFallback` state in the host. End transaction/postflight
  data loans, destroy replacements/prepared/session scratch, then return from the callback and
  dispose its captures before terminal progress/freeze/joint publication. Requesting a final
  disposition stops ordinary execution. Remove the activation `Fallback` variant with no
  production constructor; keep the real central fallback and its cleanup-before-finish route.
- Containment must detach only reference-free physical owners/charges after stopping work.
  Existing whole-operation `forget` cannot retain a borrowed common/root. Reuse value-before-
  storage cleanup and conservative unresolved-charge semantics, with no fabricated release.
  No client callback, destructor under our control or fallible allocation follows irreversible
  publication. Process abort and double-panic abort have no recovery promise.

Keep all six direct dimensions, three phases, four possible progress pairs and the current
normal action weights/control debt. Attached changed-direct and import paths still request
their actual terminal bundle in this cut. Indexed selection, the five-row follow-up, changed-
direct terminal removal and the [owned-payload work](2026-11-03-1255-refactor-owned-geometry-payload-plan.md)
remain separate changes.

## Connected Implementation and Deletion Boundary

1. **Operation root and actual hosts.** Migrate ledger/work/exclusive/storage/snapshot/startup
   and coordinator, then retained operation/execution/startup/assembly/terminal and concrete
   activation/payload/fallback/joint owners. Real private transitions borrow common and own
   stage fields; they cannot call the former consuming constructor under a borrowed wrapper.
2. **Core and postprocess.** Core alone binds the actual canonical source/account. Preparation,
   space, scene-edit validation/finish, resource facts and normal-action instrumentation use
   that same host and kernel. Preserve numeric expressions/order and the original source Scene,
   remaps, typed failure ownership and distinct known-error control behavior.
3. **Actual import entry and formats.** Construct root/account/actual canonical host outside
   `ImportSession` in both byte and asset/resolver entry paths. The existing accounted session
   cell holds a short host permission, vocabulary and recipe; pop/drop it before outer finish.
   `AccountedVec<T>` and both `release_storage<T>` paths already permit borrowed T. Keep the
   unrelated fallback `F: Send + Sync + 'static` and truly shared payload bounds unchanged.
   Migrate real detection, dependency graph, IO/hash/text and OBJ/PLY/STL/glTF parser adapters.
4. **Delete replaced owners.** Remove LocalWork's dynamic numeric alias route, caller-owned
   permit/parser compatibility, fresh Ledger startup/from_started_ledger, nested common owners,
   callback finalizers and stale eager direct terminal branches once real callers have moved.
   Internal staging may be incomplete; completion requires this connected production path.

Use an isolated compiler workspace for the large signature migration. Main exclusively owns
Cargo, profiling and ordinary measurements. Workers have explicit file ownership and cannot
modify benchmark clocks/tasks, unrelated user notes or high-context instructions. Apply to
main only a source-qualified, independently reviewed connected result.

## Meaningful Qualification

The [ordinary-entry baseline correction](../reviews/2026-11-05-import-root-entry-baseline-correction.md)
withdraws a false no-entry-poll premise found during Import migration. The qualified owning
adapter's complete call chain includes one real checkpoint. Preserve that checkpoint and
the original explicit frontiers; the separately reproduced original-error loss remains a
real construction regression. Intermediate passes against false poll expectations do not
qualify a replacement.

- Compile actual changed/unchanged edits, space declaration changes and both import entries
  through attachment, central success/rejection/fallback, caller cleanup and outer finish.
  Owned Scene/facts/failure and independent storage leases escape successfully. Valid-method
  negative probes reject permission/recovery/candidate escape, cross-root substitution,
  reentry and scoped-thread movement; pair them with adjacent valid uses.
- Forget actual permissions at each stage; ignore real semantic/resource/attach errors, then
  attempt reuse/publication. Exercise body panic, caught leaf panic, denied-entry capture Drop,
  post-request capture Drop and rejected inner-result Drop. Assert the actual host/state survives,
  exact completed prefixes/first cause, drop-once storage, no generation change and honest
  unresolved charges. Preserve importer extension and geometry/parser exit-poll distinctions.
- Use literal fixed/dynamic accounting and current-state attachment with real prework, live
  scratch, policy tightening and pending Internal competition. Cover zero/exact/one-short,
  overflow/order, shell-before-move, spare/overlap capacity, zero-byte lease and one shared
  identity through later independent/thread release. No mock-only owner proves this cut.
- Verify scratch/cell/capture destruction precedes every final success/rejection/fallback gate.
  Check allowed positive administrative coverage and unexplained residue rejection. Run affected
  nextest/compiler/consumer/strict Clippy/fmt gates, then immutable ordinary Windows/Linux
  comparisons under the unchanged acceptance protocol. Record regressions individually.

The concrete v2 sidecar and independent source review are frozen under
`.git/baozi-review/performance-convergence/`: design SHA-256
`19b11dea41615507fe9a1438355d29fba8ad8561aae6abd3d8786f245bf9d119`, review
`b658c285010878c462065ab5fa0f84715b66edab6f973b97f99d1c83aa10102c`.
They support design code-go for this connected implementation, without claiming compiler
feasibility, source qualification, an optimization keep or release readiness. No new user
permission is needed to execute the reviewed work.

## Isolated Implementation Checkpoint, 2026-11-04

The [borrowed detection and OBJ checkpoint](../reviews/2026-11-05-borrowed-import-detection-and-obj.md)
connects actual mutable detection and OBJ/MTL to the same invocation and deletes Import's
retired owning execution adapters. Its explicitly glTF-excluded source passes 994 Operation
tests, 107 Operation doctests, 338 OBJ/PLY/STL tests, three corrected root-entry tests,
17 detection/cancellation-stage tests, six Detection compiler examples and the scoped
strict lint/format checks. Independent review covers the bounded detection/root production
seams. Import's old private callers, glTF, later storage-transition support, final integrated
qualification, main application and performance acceptance remain open; this is a construction
checkpoint, not completion of the connected plan.

Actual retained/assembly fields now live in the caller-owned root, and core, postprocess, both import
entries and their format/consumer callers compile through that root. The [source-qualified receipt](../reviews/2026-11-04-lexical-root-fields-and-callers.md)
records source2's complete 3,413-test runtime and final source3's six test-only cleanup statements,
683-test core rerun, strict lint, rustdoc and consumer gates. The isolated Rust changes are not
applied to main. Canonical payload/recovery slots, outer terminal/capture authority, borrowed numeric
backing, current-state attachment and ordinary performance qualification remain open; this plan is
still in progress.

The subsequent [output/proof host receipt](../reviews/2026-11-04-canonical-output-proof-host.md)
qualifies actual outer-owned canonical values and an opaque startup permission that prevents root
extraction after a forgotten permission. Its final 1,405-file source runs all 3,426 workspace tests,
102 doctests and 138 consumer tests successfully, with strict lint and format checks. That isolated
checkpoint also remains unapplied to main. That receipt does not qualify its recovery successor or
close the remaining recovery/outer-driver work.

The subsequent [recovery-host receipt](../reviews/2026-11-04-canonical-recovery-host.md) records the
bounded central-fallback change and the actual typed activation/validation/generation cause slots.
Independent review found and reproduced two successive admission defects: foreign recovery slots
were checked too late, then aggregate cleanup of independently panicking callback captures could
abort. Source4 implements early admission and separate controlled callback cleanup; all 3,436
workspace tests, 102 doctests, 138 consumer tests, strict lint and format checks pass on its exact
1,405-file source. Independent review closes both findings and confirms final source/QA association.
Successful terminal cause detachment, actual outer driver/capture cleanup, borrowed numeric
backing, current-state attachment, shared storage promotion, retired-path deletion and ordinary
performance qualification remain open. Main Rust is still unchanged by this isolated successor.

The [closed Scene-edit publication component](../reviews/2026-11-04-closed-scene-edit-publication.md)
then removes the actual caller rejection/preparation callbacks and arbitrary result R. Central
validation retains reference-free proved handoff facts; both roots are admitted before generation,
and private success handoff only moves the exact owners. The isolated successor passes 938
operation tests and 58 doctests on unchanged scoped inputs, 986 complete core/postprocess tests,
10 affected doctests, 138 consumer tests and workspace/consumer strict lint/formatting. Independent
operation and core/caller reviews confirm the source/evidence association within that component.
This is not a whole-workspace-runtime or performance result. Late joint-failure token cleanup,
the actual outer FnMut/capture boundary and import's corresponding publication/cell migration
remain connected work; the isolated Rust successor is still not applied to main.

The subsequent [closed import-publication component](../reviews/2026-11-04-closed-import-publication.md)
migrates the actual core/import generation consumer to fixed prepared success/rich/minimal
results, admits recovery and both roots before generation, and couples the private success
owners without a fallible pairing tail. Its complete frozen source passes 1,165 Core/Import
tests (4 existing skips), 41 doctests and affected strict lint/format checks. Independent review
finds no actionable introduced defect within that component and verifies exact source/QA/log
association. Existing posthandoff import ledger projection, import's outer body/session cell,
late joint-token cleanup and the connected owner/performance work remain open. This is not
current-successor, whole-workspace-runtime or performance qualification; main Rust remains
unmodified by the isolated refactor.

The subsequent [actual outer-host/Postprocess checkpoint](../reviews/2026-11-04-outer-host-postprocess-checkpoint.md)
implements finite reference-free root stages, typed concrete requests/restoration and retained
FnMut/capture cleanup. Both production P entries use that boundary; their old borrowed
`PendingEdit` and eager body finalizers are deleted. A real never-activated rich-failure RED is
repaired by a terminal-only failure-opening marker without reopening ordinary work. Exact final
Operation inputs pass 951 tests, 59 doctests and strict lint/formatting; the unchanged executable
P implementation passes 295 tests (2 existing manual skips) and O/P strict lint. Independent
review verifies the two frozen sources and separate package evidence without a whole-source
claim. Import migration, removing alternative terminal authority, precommit projection, late
joint cleanup and all subsequent ownership/performance gates remain open. Main Rust is unapplied.

The [connected Import outer-host checkpoint](../reviews/2026-11-04-connected-import-outer-host.md)
then moves both physical import entries onto the same retained body. Outside session data
preserve report/manifest/error owners; the accounted session cell and temporary loans end
before the disposition request. Old eager session finalizers and unused terminal slots are
deleted. Its frozen six-file Import delta passes 471 Import tests and strict C/I/P lint, and
independent review finds no actionable introduced defect. The exact complete source also
passes 3,457 workspace tests (6 existing skips), 104 doctests, 138 independent consumer tests
(1 existing skip), strict workspace/consumer lint and both format checks, with complete source
maps stable across all seven integrated commands. Precommit terminal projection, late joint
cleanup, final-only authority, numeric backing/attachment/promotion and all performance gates remain open. This
isolated Rust successor is still not applied to main.

The [precommit joint-payload checkpoint](../reviews/2026-11-04-precommit-joint-payload-cleanup.md)
then retains actual output/proof inside the scoped joint. A raw unqualified classification
precedes discarded-payload cleanup, account recheck and failure finishing. Core constructs
tokens only with prepared success; late token drops and the old edit joint wrapper are deleted.
Frozen source2 passes 958 Operation tests, 1,461 Core/Import/Postprocess tests (6 existing skips),
105 doctests, 138 consumer tests (1 existing skip), strict workspace/consumer lint and applicable
format checks. Independent source review finds no introduced defect, while direct preparation
error/payload-panic and actual shared-storage refund/failure compositions need additional
regressions. Those tests are a separate moving successor. Import schema admission/projection,
old terminal authority, numeric backing/attachment/promotion and ordinary performance gates
remain open. Main Rust remains unapplied.

The same receipt's composed source6 then closes those two regression gaps with actual
attributed shared-storage refunds/release faults and three real prepare-failure phases across
both unwind states and all payload-panic masks. It passes 961 Operation tests, 1,461 connected
caller tests (6 existing skips), 105 doctests, 138 consumer tests (1 existing skip), strict
workspace/consumer lint and both format checks. Independent final source/evidence review
closes the earlier test-oracle findings without relabeling any historical RED. This bounded
checkpoint does not qualify a whole-workspace runtime or ordinary performance; all remaining
schema/authority/backing/attachment/promotion work and main Rust application are still open.

The [precommit Import catalog receipt](../reviews/2026-11-04-import-precommit-catalog.md)
then admits the complete schema against the actual finish root before restoration/generation,
and removes fallible post-handoff report projection. Independent review finds a secondary
cleanup-panic escape during an existing unwind; an actual new regression aborts on the old
gate. Source2 preserves the original unwind and passes that same regression, 2,423 affected
tests (6 existing skips), 105 doctests, 138 consumer tests (1 existing skip), strict workspace/
consumer lint and both format checks. Independent final review verifies the exact RED/GREEN,
all source/QA identities and F1 closure. Old terminal authority, unreachable canonical API
arms, numeric backing, attachment/promotion and ordinary performance gates remain open;
main Rust is still unapplied.

The [success-only canonical joint receipt](../reviews/2026-11-04-canonical-joint-success.md)
then removes the exact joint's unreachable rich/minimal outcomes, redundant concrete
classification wrapper/helper, three Core dead consumer branches and the corresponding
direct Postprocess publication/owner path. Actual ordinary failure classifications and
storage-cleanup oracles remain at their raw owner. Source2 adds real after-Ready cancellation
coverage through the outer body; it passes that focused regression, 2,424 affected tests
(6 existing skips), 105 doctests, 138 independent consumer tests (1 existing skip), strict
workspace/consumer lint and both format checks. Independent source review finds no introduced
defect and closes its identified test gap; final evidence review verifies all frozen sources,
QA/log associations and exact result counts with no finding. Final-only
authority, numeric backing, attachment/promotion, main Rust application and all ordinary
performance gates remain open.

## Final Authority Removal Decision, 2026-11-04

The bounded independent analysis of the qualified Import catalog source identifies three
connected removal requirements. Deleting `split` alone is insufficient: a body can start the
raw retained operation and prepare an ordinary terminal directly, and a body-owned rejection
builder can internally prepare fallback. Close all three routes together:

- Replace body parts with `CanonicalBodyParts`, containing only opaque `CanonicalBodyStart`,
  output/proof slots and activation/validation slots. Generation and failure-fallback slots
  are issued only after the retained body and captures have been disposed.
- Start an opaque `CanonicalBodyOperation` through that nominal permission. It forwards
  bounded execution, reservation, progress, phase and failure-work operations, activation and
  disposition requests. It exposes no raw startup conversion, publication assembly, direct
  terminal preparation, recovery extractor or mutable dereference to the retained owner.
  Ordinary raw publication remains a separate existing API for its real callers.
- Restore each actual deferred canonical stage into privately constructed `CanonicalFinal<S>`.
  Only its concrete implementations expose freeze issuance or rejection/interruption/fallback
  preparation. Generation-rejection descendants retain that permission. Body states retain
  inspection, activation/validation and consuming disposition requests, with no terminal
  escape through their failure builders.

Keep the current finite host fields and same-root restoration; these views create no new
account, ledger, domain, allocation or numeric backend. Migrate production Import/Postprocess
and Core's same-account edit constructor, then legacy split-based fixtures to `run_body`.
Tests may inspect inside a body or return an actual recorded finish; a test-only `split` or
public final-wrapper constructor would retain the defect. Preserve the inner format/parser
callbacks' metered results and borrowed authoring data. Their bounded callbacks have a
different purpose from the fixed-unit outer body.

Qualification must pair valid outside-body finalization with external compiler rejections
for body-side terminal methods, raw startup conversion, final-wrapper construction, escaped
body state and the deleted `split`. Exercise real candidate/failure dispositions and legal
`mem::forget` of body/final permissions: actual host payloads, graph and original account must
remain available for cleanup or containment. This decision is a source-grounded design cut;
it does not claim this removal has been implemented or measured. The static report is retained
locally as `final-authority-removal-cut-1.md` with its frozen-source and lane association.
Its immutable SHA-256 is `f93c9514ac034bb91eed9c1ad9fda155f904176d139930556b8404bb94920124`.

## Final Authority Implementation Checkpoint, 2026-11-04

The [final-authority receipt](../reviews/2026-11-04-final-invocation-authority.md) implements
the preceding decision across the actual Operation/Core/Import/Postprocess path and legacy
fixtures. `split` and its parts type are deleted. Opaque body startup/operation views expose
bounded work and disposition requests; only the six real restored `CanonicalFinal<S>` states
expose final freeze/failure preparation. The original host retains real output/proof/cause,
account and cleanup authority. Actual inner authoring/parser callbacks remain.

Frozen source4 passes all 3,469 workspace runtime tests (6 existing skips), 142 affected
doctests, 138 independent consumer tests (1 existing skip), workspace/consumer strict Clippy
and both format checks. Complete live/frozen sources and all seven verification commands'
14 maps match the 1,412-file source set; the separate preparatory formatter's maps also
match. Independent source4 review closes the constructive/escape compiler gaps without a
correctness finding. Independent final review verifies the exact source, all command/log
associations, actual totals and unchanged skip declarations without a finding.

The remaining implementation is borrowed numeric backing and the actual permit/parser
consumer migration, current-state attachment using one start admission, shared-shell-before-
state promotion, exact accounting and superseded-owner deletion. Main Rust application and
all unchanged Windows/Linux timing/memory gates remain open. This structural qualification
does not establish an ordinary performance improvement.

## Borrowed Geometry Action Checkpoint, 2026-11-04

The [geometry-action receipt](../reviews/2026-11-04-borrowed-geometry-actions.md) migrates
Postprocess's actual `ActiveRuntime` onto `StoreActionExecution`, which holds the existing
exclusive `WorkView`, actual pending `WorkFrame` and captured cost outside client loans.
The old Postprocess work/group/services/owning-permit adapter and its cleanup dispatcher
are deleted. Begin/Complete events, singleton replenishment, physical-prefix accounting,
scratch Internal share work and control behavior retain the existing kernel and cadence.

Frozen source2 passes 3,473 whole-workspace tests (6 existing skips), 144 affected doctests,
138 independent consumer tests (1 existing skip), strict workspace/consumer lint and both
format checks. Independent connected-source review closes the two relocated owner-oracle
gaps without an introduced defect. Independent final review verifies exact source/QA/log
association, the actual new owner-test inclusion and unchanged skips without a finding.

This migrates the geometry action consumer; it does not yet remove `LocalWork` from the
real semantic, Import and startup consumers. The next semantic cut has three production
entries and must return source-bound plans/outer-branded reservations only after the unique
numeric loan and controlled callback captures end. Current-state attachment, shared
promotion, exact accounting, main Rust application and ordinary performance remain open.

## Host-Owned Semantic Work Checkpoint, 2026-11-04

The [semantic-work receipt](../reviews/2026-11-04-host-owned-semantic-work.md) connects the
actual Import, Postprocess and Core node-transform semantic callers to retained
`CanonicalPlanning::with_semantic_work`. The real session holds the existing unique
`StoreInvocation`; clients borrow it. Callback captures end before final control and
completion issuance. Public begin/client-finish methods and all their old callers are
removed. Returned input-bound plans and outer-scoped reservations survive the shorter
numeric loan; sealing remains outside that callback. The existing action kernel,
physical-prefix accounting, first-cause and mathematical behavior remain.

Frozen source4 passes all 3,478 workspace runtime tests (6 existing skips), 149 affected
doctests, 12 focused semantic tests, 138 independent consumer tests (1 existing skip),
workspace/consumer strict lint and both format checks. Both exact 1,415-file trees, all
nine logs and sixteen qualification maps plus both preparatory formatter maps match.
Bounded independent source reviews find no confirmed production defect and identify two
direct host regression gaps. A test-only source5 successor adds three tests/four executions
for returned-value cleanup and nontrivial rejection ownership. Its 15 focused semantic tests,
Operation strict lint and format check pass. All production/docs/consumer files remain
identical to source4; this is affected successor evidence, not another full-workspace run.
Final independent review verifies both exact source/QA/log associations, closes the two
testing gaps and confirms the new external completion markers without a finding.

Remaining real Import/detection and startup/share numeric consumers still block removal
of `LocalWork`/`BoundWorkGroup`. Current-state single-start attachment, shared-shell-before-
state promotion, exact accounting/deletion, main Rust application and the unchanged
Windows/Linux timing/memory campaign remain open. This component establishes no new
ordinary performance improvement.

The subsequent [borrowed singleton replenishment receipt](../reviews/2026-11-04-borrowed-parser-replenishment.md)
adds the missing thin Operation/Import forwarding required by actual comparison loops.
Three real Operation host tests preserve literal prefixes, sticky denial and the 257th
checkpoint; an actual custom importer performs six comparisons and submits only after the
numeric loan ends. Its frozen four-file delta passes 1,450 Operation/Import runtime tests
(zero skips), 140 affected doctests, strict workspace lint and formatting. Independent
source/evidence review verifies the exact delta, actual probes and terminal association
without an actionable finding. This is the first facade prerequisite; full-service
roles, detection, format migration, old-adapter deletion and all subsequent gates above
remain open.

The [finite Windows source checkpoint](../reviews/2026-11-04-lexical-host-performance-checkpoint.md)
then measures this exact isolated source against the retained bound-services worker.
Four completed campaigns retain all selected eight-case/31-memory-lane observations, but
each records external Cargo/rustc/test activity and cannot establish an uncontaminated
ordinary keep. Observed repeated OBJ/STL pipeline reductions coexist with no-op and
extraction costs; all remaining implementation and complete Windows/Linux timing/memory
gates persist. Independent raw reconstruction verifies all 5,704 children and 3,596
summary slots with zero validation errors and unchanged reported work/output signatures;
the 25 actual interference observations and all costs/tails remain explicit.

The [trusted scan-close prerequisite](../reviews/2026-11-04-trusted-parser-scan-close.md)
then adds a typed Exhausted/Boundary outcome over the existing parser host. Ordinary
parent/child results stay exact; a trusted text boundary preserves the verified prefix,
releases unused admission and skips successful-channel debt without clearing earlier
failure. Source1 passes four focused tests, 142 doctests, all 2,317 selected Operation/
Import/four-format tests (zero skips), strict workspace lint and formatting. Independent
review identifies two direct scan-branch test gaps; a production-identical source2 appends
both and passes six focused tests, Operation strict lint and formatting. Final independent
review closes both gaps and verifies the separate source/QA/log associations without a
finding. Actual Import text exit polls, service/children loans, returned-prefix execution,
PLY and remaining format/detection migration remain required;
this mechanism is not connected Import completion or a new performance result.

The [borrowed progress component](../reviews/2026-11-04-borrowed-progress-storage.md)
then removes the progress state's complete Ledger alias. Real metadata and poll fields
bind to the same storage-only evidence authority beside exclusive numeric work; one
existing algorithm retains its capacity/release, two-poll and first-cause behavior.
Source1's failed new supplemental-cause assertion is preserved. Test-only source2
corrects that assertion and passes four focused tests, 142 affected doctests, all
3,495 workspace runtime tests (six existing skips), 138 consumer tests (one existing
skip), both strict Clippy gates and root formatting. Independent source review finds
no new production defect and records two non-blocking direct-coverage limits. Successful
root event delivery and remaining active-prefix control failures remain follow-up work
with the actual Import migration. Independent terminal evidence review verifies both
source associations and actual results without a mismatch; those limits remain open.
This is a bounded service prerequisite; the concrete
services/children projections, returned-prefix runner, actual text/PLY and other format
consumers, old-adapter deletion and all subsequent integration/performance gates remain
required.
