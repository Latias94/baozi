---
title: "Converge maintained import workflows with Assimp before release"
type: perf
status: in_progress
date: 2026-11-03
baseline: e0a7b387371b71c28bc40f6e2cfd4c1ff5f91726
---

# Assimp Performance Convergence

The user requires comparable performance before the first usable release. The completed
pre-release evolution does not satisfy this new release condition. Necessary interface,
ownership, execution and representation changes may break compatibility. Publication remains
closed while this plan is incomplete.

## Evidence and Acceptance

Keep the fourteen existing analytic cases, 110 admitted lanes, successful semantic tasks,
precision, normal policies, allocator/build policy and complete-pipeline timing boundary in
[the comparison protocol](../testing/assimp-comparison.md). Frozen B0/B1 records remain history.
Use new immutable binaries, build manifests, source archives and artifact directories for every
candidate. No build or profiler runs concurrently with ordinary measurement.

The following are conservative **engineering targets**, adopted for development rather than
attributed to a numerical promise from the user:

- Every file/memory complete-pipeline median should be at most 3 times its same-campaign Assimp
  control, on both Windows and Linux. Report each lane; an aggregate cannot hide a failing case.
- Every admitted import, preparation and extraction endpoint should meet the same at-most-3x
  release target. Removing confirmed 10-fold gaps is an intermediate checkpoint. Very short
  endpoints require dispersion and absolute-time reporting and repeated confirmation, rather
  than an exemption or silently changed task. Unresolved noisy lanes remain open.
- Comparable whole-child peak memory should stay within 3 times the same-method control.
  Separately collected heap peaks and allocation counts are diagnostics with named lifetimes;
  they never substitute for missing RSS/working-set data. Investigate the old RSS breaches again.
- Timing parity is necessary, not sufficient for stability. Semantic, numerical, resource,
  cancellation, failure ownership and public consumer gates must still pass. No new producer
  breadth or untested platform claim follows from this finite comparison.

A retained experiment needs a material same-task ordinary improvement beyond dispersion,
without an unexplained regression in affected lanes or memory. A candidate that improves one
owner but leaves these release targets unmet is an intermediate result, not release readiness.
An independently reviewed candidate may remain the intermediate working source with a named
accepted tradeoff while its confirmed per-lane regression is repaired. That disposition is
explicitly conditional: it clears neither the regression nor any release gate and cannot be
reported as a general no-regression result. Preserve the failed lane and its immutable control.
After each major keep, refresh attribution before choosing the next representation change.

## Current Baseline

[Windows final](../reviews/2026-11-03-windows-final-comparison.md) and
[Linux B1](../reviews/2026-11-03-linux-b1-and-heap-comparison.md) each contain 2,200 retained pairs.
Runtime source is `52446ec8511b7dfba2e5352ce6941558d85e0ae2`. Windows memory pipelines include
90.450/9.406 ms repeated OBJ, 194.923/24.758 ms grid OBJ, 5.168/0.333 ms shared glTF and
4.488/0.228 ms STL (Baozi/Assimp). The normal-preparation endpoint remains 48.738/1.028 ms.
These independently measured endpoints are not additive pipeline components.

Current whole-worker Callgrind profiles are under `target/review/final-attribution/`. They
contain setup/checking/teardown and count instructions, not native CPU percentages. They show
repeated work binding, semantic-host validation and permit construction/completion across
formats. Parser completion is also substantial in grid OBJ. The existing stable buckets and
dense remaps already removed the previously demonstrated sorting waste; do not claim that gain
again or assume text allocation is now the leading cost.

## Work Sequence

1. **Locate and freeze.** Reconcile current exclusive profile rows with the executed source;
   inspect concrete call edges and record estimated opportunity bounds. Verify ordinary binary
   hashes and run a finite current Windows checkpoint. Keep the existing harness immutable.
2. **Bind semantic execution at its owner.** Investigate replacing per-unit producer-host
   construction/key validation/full control polls with one session-bound work/storage/control
   region. Admit each real action before execution, record each normally returned physical
   prefix immediately and poll at existing bounded/semantic/allocation boundaries. Retain
   panic containment, sticky errors, scoped receipts and rejection ownership. Delete the old
   repeated path when its replacement qualifies.
3. **Reduce numeric capability traffic.** Investigate carrying dense owner-bound positions
   through parser and observed-action loops, projecting full semantic identities only on cold
   error/evidence paths. There must remain one numeric authority. No shadow counters, precharged
   completion, guard across user code, unmetered cache or second interpretation of work policy.
   Measure this independently from step 2 where file scopes permit.
4. **Re-profile and choose the next owner.** Evaluate typed geometry read views for normals and
   extraction, borrowed conforming text, flat syntax staging and payload adoption only against
   the refreshed measured cost. Preserve source identity, signed zero, exact accumulation order,
   partial-channel policy, finite checks and bounded scratch. Replace a representation at its
   owner and remove superseded paths; avoid benchmark-only or format-specific escape hatches.
   Current normal call paths justify a versioned vocabulary of bounded face, smoothing-candidate,
   output-element and at-most-32-record scan actions. Preserve the numerical expressions/order;
   meter actual completed invocations and keep separate conservative control debt within the
   existing scheduling envelope. Borrow resolved fields through the existing core sampler rather
   than adding a second dense/sparse sampler to postprocess merely to avoid a by-value receiver.
   Apply the [normal standards research](../research/2026-11-03-normal-generation-standards.md)
   separately from this performance vocabulary change.
   Distinguish normative missing-field rendering behavior, authored normals/smoothing intent,
   and explicit generic weighting choices. The official glTF 2.0 specification requires flat
   normals when rendering missing NORMAL and ignores supplied tangents in that case; merely
   preserving absence during import is a separate contract. Audit the real asset consumer's
   unconditional default Smooth request. Require noncoplanar-face, authored-normal, hard-group,
   tangent and relevant morph/unlit regression evidence before a rendering-policy fix. Preserve
   the frozen comparison's explicitly selected normal algorithm and task at this cutover.
5. **Small-task lifecycle.** Locate setup/fallback/evidence cost separately from scaling work.
   A reused-plan workload may be added as a separately named experiment for both engines; it
   never replaces the frozen complete pipeline or moves setup outside its clock.
   The independent lifecycle review rejects deferring only retained assembly while keeping
   allocationful generic startup as the final architecture. Design one invocation owner with
   fixed inline state for the finite postprocess vocabulary, sharing the entire numeric work,
   storage and control implementation with dynamic import. Defer publication attachment until
   real writes are needed. First specify backing and release-token ownership and extract the
   consuming attachment/finish seam. Then integrate indexed classification in place of redundant
   full-Scene selection, attaching retained write resources to that same authority when needed
   without restarting counters or replaying validation.
   Adopt the reviewed [exclusive invocation plan](2026-11-03-1120-refactor-exclusive-invocation-plan.md):
   host-owned contexts over lexical fixed/dynamic roots replace unnecessary numeric reentry/mode
   checks while preserving real parser nesting, resource-release and failure obligations. Shared
   authority remains for genuinely escaping leases. That plan supersedes the initial promotion
   proposal with shell allocation before state movement and an explicitly named portable
   administrative payload/capacity metric; existing physical charges stay intact.
   Specify consuming attachment and cleanup-before-terminal before integrating selection; do
   not create a temporary second start/coordinator to bridge that seam.
   Empty/single unchanged default calls should need no new allocation, generation or retained
   terminal graph. Larger selections may use honestly admitted scratch. Replace mandatory
   retained-ledger report ownership with owned inline facts, and delete the superseded startup,
   selection and terminal representation paths when the consuming attachment qualifies.
   Preserve semantic validation and error order rather than getter-by-getter historical work
   units. Define and meter bounded physical actions before changing that vocabulary. No action
   denial may execute the action; normal returned prefixes, panic containment, current policy,
   same-account scratch and first-cause/control history remain mandatory. This is a substantial
   breaking owner refactor with separate design/source/measurement gates, not a no-op shortcut.
6. **Integrated convergence.** Run the complete ordinary Windows/Linux matrices, matching heap
   diagnostics for representative failures and all applicable regression gates. If a target is
   unmet, record the precise remaining owner and continue the next measured replacement. Do not
   classify an accepted historical semantic cost as passing this new release gate.

## Coordination and Verification

The main thread owns all Cargo/CMake/profiling/measurement, serialized with one build job and
the existing target. Native Codex research lanes cover OBJ construction, normal preparation and
small-task lifecycle. Implementation lanes receive disjoint file scopes; independent reviewers
inspect invariants and the combined diff before any keep. Preserve unrelated user notes.

Local execution state is `.git/baozi-review/performance-convergence/`. Retain hypotheses,
commands, exact source identities, failures, ordinary samples and decisions there or under new
`target/review/` and `target/assimp-comparison/` directories. Export reviewer-relevant conclusions
to tracked documents. No registry publication/yank, remote push/PR or native-target promotion is
part of this plan.

Validation is proportional to each owner: meaningful physical-prefix/admission/cancellation and
panic/failure tests first; affected nextest suites and strict Clippy/fmt; public comparison
consumer semantics; independent review and ordinary before/after measurements. Broad integrated
checks follow a retained runtime, not every exploratory edit. Use source-qualified local
Conventional Commits for complete slices.

Before retaining a borrowing/capability replacement, also run the affected packages' existing
compile-fail doctests separately with `cargo test --locked --doc --all-features`. Preserve the
negative contracts for region escape, cross-thread execution and linear publication. Add a new
negative case only for a genuinely new boundary, not for incidental private layout.

## Execution Record

- Initial source/workspace inspection: tracked paths clean; pre-existing untracked engineering
  notes preserved. No active build or prior optimization goal remains.
- Initial independent plan review: no P1 finding; both P2 corrections adopted (all endpoint
  release targets at most 3x, and affected compile-fail doctests before a capability keep).
- Three source/profile investigations independently identify semantic-session binding first;
  normal reader enum dispatch and borrowed text copying have much smaller current opportunity
  envelopes. Repeated glTF profile accounting is a separate subsequent measured candidate.
- Fresh Windows baseline: 23 memory-input lanes, 460 retained pairs and 69 warmup pairs;
  all successful, unchanged ordinary worker. Local report is
  `target/assimp-comparison/convergence-windows-baseline/report.json`, SHA-256
  `b0a155d1531068c06e0adc0a0a4dad87297076aee0d52d7c48531b481b4ff962`.
- R1 session-bound semantic execution is retained at
  `a54d39dcd0862b12d292e7541cab0bf5db92986c`. The
  [qualification receipt](../reviews/2026-11-03-bound-semantic-execution.md) records source and
  caller failures found/fixed, 2,262 affected tests, compiler contracts, strict lint, public
  control/asset consumers, fresh ordinary confirmation, short-endpoint cost dispositions and
  six reconciled current-source instruction profiles. This is an intermediate keep, not release
  convergence or a per-lane no-regression assertion.
- R2 compact owner-bound numeric capabilities and lazy diagnostic-key projection: independently
  source-reviewed and committed at `4010bd205c7f712278bcda7b9378cd5b8019375d` after 2,266 affected
  tests, compiler contracts, strict lint, control assurance and 120 asset-consumer tests passed.
  Four Windows campaigns and the targeted eight-lane Linux ordinary check are independently
  reviewed. Retain a modest intermediate gain with endpoint/control-drift/tail qualifications;
  the [receipt](../reviews/2026-11-03-compact-numeric-work.md) retains the grid instruction increase
  and unresolved repeated-extraction observation. Full release convergence remains open.
- Read-only lifecycle design review identifies two independent costs: allocationful invocation
  and mandatory retained terminal construction, plus redundant selection traversal. Delaying
  only terminal construction cannot establish endpoint convergence. Fixed invocation backing,
  one shared accounting authority, indexed semantic classification and consuming retained
  attachment are being designed before production implementation.
- R3 observed-envelope reuse is source-qualified at `ae4c77af891e3d92340535c48408a32ffbec34e9`.
  Independent four-campaign Windows review supports an intermediate keep: repeated normal
  preparation improves about 37% and its pipeline about 19–20%, while grid pipeline's positive
  medians remain an accepted unresolved tradeoff. The
  [receipt](../reviews/2026-11-03-observed-work-sequence.md) retains every positive-lane concern,
  memory scope and six current-source Linux instruction profiles. This is neither uniform
  improvement nor release convergence. Reader and bounded normal-vocabulary candidates remain
  separate, reviewed and unqualified until applied/tested/measured on their own source.
- R4 [bound attribute readers](../reviews/2026-11-03-bound-attribute-readers.md) are source-qualified
  at `1065e0469ad688bcfa1e2f7d204243be58d85d00`: 961 core/postprocess tests, compiler contracts,
  strict library/consumer Clippy and 124 asset-consumer tests pass. Independent ordinary review
  supports an intermediate keep with large repeated selected-extraction gains. F64-neighbor
  extraction is a confirmed +3.55/+3.10 us regression and remains a required repair; glTF/STL
  positive lanes also remain explicit. Six current-source Linux instruction profiles do not
  clear Windows native regressions. Full release convergence remains open.
- [Pinned Assimp principles and Rust ownership audit](../research/2026-11-03-assimp-principles-and-rust-ownership.md)
  distinguishes loader-specific buffering, different normal algorithms, genuinely escaping
  storage ownership and removable per-getter/lifecycle work. Independent design review requires
  correcting promotion and its footprint contract before implementation. A host-owned exclusive
  execution-context alternative is being assessed; no design is called optimal merely because
  it passes the borrow checker or has fewer allocations.
- R5 [bounded normal actions](../reviews/2026-11-03-bounded-normal-actions.md) are source-qualified
  at `57c33b2447a296c28f5f5817a36debd248bd4280`: 2,297 affected tests, compiler contracts,
  strict lint, 124 asset-consumer tests, control assurance and scene/fuzz compilation pass.
  Independent four-campaign ordinary Windows review supports an intermediate keep for the
  repeated-OBJ preparation gain of 26.48% in both orders and pipeline gains of 14.93% / 12.75%.
  Grid extraction's +5.36% confirmation cost, the original R4 F64 regression, noisy/positive
  small lanes and parent/tail observations remain explicit repair/confirmation obligations.
  Six exact reconciled Linux instruction profiles show repeated -15.14% and other workers
  +0.69% to +2.11%; they do not clear native-time or memory gates. Fifteen of 23 timing lanes
  and four of six pipelines still exceed 3x Assimp. Consumer normal policy and later owner
  candidates remain separately qualified sources, not part of these measurements.
- The [two-hint reader experiment](../reviews/2026-11-03-reader-composition-experiment.md)
  is rejected after four ordinary campaigns: F64 extraction rises 17.91% / 17.33% despite
  large extraction gains. Only those two attributes are removed at `8f89e4f`; qualified reader
  semantics remain. Linux executed inlining does not clear Windows native regressions.
- R7 [bound glTF profile comparisons](../reviews/2026-11-03-bound-gltf-profile-comparisons.md)
  are source-qualified at `ae11d58a6702a4f2a54910b277043d9aa9dbfbc2`, after 517 glTF and
  138 consumer tests, strict lint, formatting and actual-source review. Four eight-case,
  31-lane Windows campaigns support a named intermediate keep: glTF32 import improves
  24.97% / 23.79% and pipeline 22.72% / 22.79%. Repeated glTF1/8 and F64 extraction costs,
  glTF32's confirmation concern and small process-commit increases remain repair obligations.
  Eight exact-reconciled Linux instruction profiles remain separate evidence. Twenty-three
  of 31 timing lanes and six of eight pipelines still exceed 3x Assimp.
- The reviewed [owned geometry payload plan](2026-11-03-1255-refactor-owned-geometry-payload-plan.md)
  addresses the ownership erased by borrowed-only generated edits and global scalar pools.
  The audit distinguishes core and kernel validation predicates; immutable value/attachment
  proof and lease transfer require independent design review before implementation. Exclusive
  runner v3 restores compact catalog projection before actual first-proof qualification.
- The [actual exclusive invocation first proof](../reviews/2026-11-03-exclusive-invocation-proof.md)
  qualifies one numeric kernel and host-owned branded contexts: operation 905, affected 1420,
  glTF comparison 9, control 9, consumer 138, affected doctests and strict lint/formatting pass.
  Independent actual-source review closes the compiler/format delta. This is a structural
  proof, not caller cutover or a performance keep. Phase-2 design must preserve owner-specific
  incomplete-success/unwind exit polling and accounted format-error construction before
  replacing real parser services; the provisional first-proof cause is not an import classifier.
- [Query-owned node binding](../reviews/2026-11-03-query-owned-node-view.md) is source-qualified:
  972 core/postprocess and 138 consumer tests, affected compiler contracts, strict lint and
  formatting pass. One complete checked view and one shared row decoder preserve the query's
  existing admission, read, numeric and control order. Independent application review closes
  the formatting delta. Main rejects the ordinary experiment after four eight-case/31-lane
  campaigns: repeated OBJ extraction's initial gain does not reproduce and small F64/glTF
  extraction medians repeatedly increase. The four runtime paths are restored exactly to
  `45a09d1` at `24c533e`; all source and measured evidence remains. Independent reconstruction
  of all 5,704 children supports rejection and retains favorable pipelines, drift/tails and
  glTF32's 58--64 KiB extraction-commit cost. No further reader experiment is selected from
  this source-level opportunity; exclusive lifecycle and actual count-only work take priority.
- [Profile payload cardinality](../reviews/2026-11-03-profile-cardinality-actions.md) replaces
  the count-only per-cell loop with one bounded arithmetic action per positive call, preserving
  logical first-limit/overflow and all real metadata/data visits. The breaking work category
  records physical arithmetic rather than fictional visits. Core/import/postprocess 1430,
  all four formats 853, consumer 138, exact semantic-owner boundaries, doctests and strict
  lint/formatting pass. Independent source/application review and the source-derived 80-to-76
  test-boundary correction are closed; the initial failed check remains. Source is
  `0001bd8ab5ccf0c6c1b16754a1c2016ad8a76684`. Independent four-campaign Windows raw review
  supports an intermediate keep for repeated glTF import/pipeline gains of about 6--8%,
  retaining six extraction repairs, glTF8 prepare, glTF1 commit and named tails/parent costs.
  Twenty-three of 31 timing lanes and six of eight pipelines still exceed 3x Assimp.
- Exact matched Linux instruction workers separate the cardinality algorithm from the
  structural kernel extraction: glTF1/8/32 whole-worker Ir falls 17.74% / 14.07% / 10.91%
  against `45a09d1`. R7-to-45a adds about 4--9% on most cases while transitional callers remain;
  these whole-worker totals do not locate contributing call owners. Carry that structural
  revision cost as a caller-cutover investigation/repair obligation. Every raw/exclusive total reconciles. Neither
  instruction gain nor intermediate native keep clears Windows extraction costs or the
  final all-endpoint/pipeline/memory target.
- The [bounded call-owner investigation](../research/2026-11-03-common-work-kernel-costs.md)
  independently verifies a contributing unchanged-action route: repeated OBJ's typed runner
  still executes 262,189 bodies with identical exclusive instructions, while its authority/
  control child edges add 30,937,278 Ir. Preserve that shallow-cut scope and copy/inlining
  limits. It supports investigating real-region ownership and deleting obsolete adaptation;
  it is not a measured benefit of future exclusive adoption or Windows code-generation cause.
- [Actual import bridge qualification](../reviews/2026-11-03-exclusive-import-bridge.md) at
  `67b5ca609470268b124fd1915741ee81105c4a24` connects Dormant, real registry disposition and
  consuming submission to the same exclusive executor. Operation/core/import/postprocess 2346,
  formats 853, control 9 and independent consumer 138 pass with compiler examples, strict lint,
  formatting and independent production/oracle/application review. Exact Control causes,
  failure materialization before frame close, owned errors and stopped-state entry gates are
  qualified. This is an unmeasured structural slice; all prior performance costs remain open.
- Complete release-convergence qualification: pending; all final gates remain closed.
- The [binary STL exclusive caller](../reviews/2026-11-03-binary-stl-exclusive-caller.md)
  qualifies source `48d6f17e2f0829bdd5d50c8d7c48cea72ce343a1` on Windows: affected 2355,
  formats 855, control 9 and independent consumer 138 pass, with docs 30 + 54, strict lint,
  formatting and actual new-test/artifact association. One ordinary storage/growth owner and
  one guarded compound service replace duplicated bodies and binary's old adapter. No normal
  math, benchmark task or allowance changes. The subsequent
  [ordinary performance hold](../reviews/2026-11-03-binary-stl-performance-hold.md)
  independently reconstructs all 5704 children from four Windows campaigns: repeated OBJ
  import +7.66/+7.59%, grid import +5.13/+5.08%, with both pipelines positive twice.
  Preserve favorable extraction observations and all preparation/memory/tail costs.
  Performance keep is HOLD; the source remains only a structural prerequisite for correction.
  Exact matched Linux whole-worker Ir is +3.53% repeated OBJ, +3.62% grid and -0.84% STL;
  raw/exclusive totals reconcile without establishing Windows causation or native keep.
  Investigate per-call ordinary storage/control rebinding and common checkpoint state work
  at their actual owners. The [independent shallow-route review](../research/2026-11-03-ordinary-storage-binding-costs.md)
  separates 90 Ir per loan construction from scope checkpoint 93-to-104 Ir growth, with
  unchanged real poll counts, unchanged reserve kernel and unchanged vector-growth memcpy.
  It does not identify Windows causation or clear any cost. Twenty-three of 31 selected timing lanes and six of eight pipelines
  still exceed 3x; all earlier named costs and the complete release gate remain open.
  The exact caller archive also passes Linux operation/import/STL 1457/1457, no skips,
  docs 30 + 54 and strict all-target/all-feature lint. All archive-file identities match before
  and after each check; this is affected correctness evidence, separate from ordinary keep.
- The [normal source-value follow-up](../research/2026-11-03-normal-generation-standards.md#subsequent-selected-value-audit-and-windows-reproduction)
  reproduces selected FLOAT NORMAL `(0,0,2)` acceptance on Windows with a passing bit-exact
  unit control. This P2 requires an explicit glTF admission tolerance/policy and selected-owner
  regression coverage before claiming direction-length validation. Preserve quantized decoded
  magnitudes, signed morph deltas, generic core authored contracts and the frozen benchmark
  task. This is independent correctness work after the current bridge cut, not a normal
  estimator change or performance keep; the failed probe and source-qualified receipts remain.
- [Bound storage services](../reviews/2026-11-03-bound-storage-services.md) qualify
  `314d0fe3855cdd53159d2ffc0d6e10206d299b9f`: final Windows integrated 2357, formats 855,
  control 9, assets 138, actual loan compiler contracts and strict lint/formatting pass;
  exact-archive Linux affected 1459, docs 34 + 56 and strict lint pass. The old split
  storage/control views and arbitrary-sibling joins are removed, retaining one growth and
  numeric implementation. Independent [ordinary raw review](../reviews/2026-11-03-bound-storage-performance-hold.md)
  of all 5704 children keeps performance HOLD: repeated import's pilot gain does not repeat,
  both OBJ extraction costs repeat, and glTF confirmation/tails plus whole-child memory
  costs remain explicit. Preserve grid/STL pipeline and no-op preparation improvements.
  The source remains a structural prerequisite, with 23/31 timing lanes and 6/8 pipelines
  still above 3x; neither this slice nor small Linux Ir reductions clear the release gate.
  [Exact instruction attribution](../research/2026-11-03-bound-storage-instruction-costs.md)
  verifies constructor self cost 90-to-23 Ir per captured entry, unchanged real polls and
  numeric kernels, and whole-worker OBJ reductions of about 1.3%. It also identifies
  remaining canonical action owners and grid parser completion/active-borrow work.
  Preserve those distinct owners; a removed descriptor constructor is not a complete
  lifecycle, canonical payload or parser replacement.
- The [lexical invocation-host successor](2026-11-03-1955-refactor-lexical-invocation-host-plan.md)
  is being implemented in an isolated compiler workspace. Actual retained/assembly fields,
  output/proof and typed recovery causes are source-qualified in the outer host; the
  [recovery receipt](../reviews/2026-11-04-canonical-recovery-host.md) preserves real failing
  admission regressions and their repairs. The final successor passes 3,436 workspace tests,
  102 doctests, 138 consumer tests, strict lint/formatting and independent source/evidence review.
  These structural checkpoints do not apply that Rust successor to main, establish a new
  ordinary performance keep or clear any of the unchanged release targets.
- The isolated [closed Scene-edit publication component](../reviews/2026-11-04-closed-scene-edit-publication.md)
  removes the actual caller terminal callbacks/arbitrary R and moves rejecting handoff checks
  into the existing central proof before generation/commit. Actual operation and core/postprocess
  tests, affected compiler examples, consumer gates, strict lint/formatting and independent
  source/evidence review pass. Whole-invocation capture cleanup, import publication/cell migration,
  numeric backing, current-state attachment, promotion and deletion remain open. This source
  qualification supplies no new Windows/Linux ordinary timing or memory result.
- The isolated [canonical success-only joint component](../reviews/2026-11-04-canonical-joint-success.md)
  follows the connected Import body, actual payload cleanup and precommit catalog components.
  It removes unreachable joint failure outcomes, their three Core consumer branches and
  Postprocess's corresponding dead publication/owner path, while retaining actual ordinary
  rich/minimal preparation. The raw success finish rejects a non-success state before freeze.
  Independent review's after-Ready cancellation coverage request is implemented as a real
  outer-body regression; source2 passes that focused test and 2,424 affected tests (6 existing
  skips), 105 doctests, 138 consumer tests (1 existing skip), strict workspace/consumer lint
  and both format checks. Independent source and final evidence reviews close the identified
  gap and verify the exact frozen source/QA/log associations. The reviewed final-authority
  decision requires opaque body startup/
  operation views and private final state wrappers together with deleting `split`; that
  connected migration belongs to the successor below. Main Rust and ordinary performance
  remain unchanged by these isolated structural checkpoints.
- The isolated [final invocation-authority component](../reviews/2026-11-04-final-invocation-authority.md)
  deletes the old `split` and restricts body startup/operations, final failure builders and
  freeze issuance across actual Core/Import/Postprocess callers and legacy fixtures. The
  original outer host restores six actual final states after body/capture cleanup; no new
  numeric backend, account or mathematical algorithm is introduced. Frozen source4 passes
  3,469 whole-workspace runtime tests (6 existing skips), 142 affected doctests, 138 independent
  consumer tests (1 existing skip), workspace/consumer strict lint and both format checks.
  Complete source/QA/log checks match; independent source4 review closes the constructive
  and independent escape compiler gaps. Independent final source/evidence review also passes.
  Borrowed numeric backing, current-state single-start attachment, shared promotion, exact accounting,
  obsolete-owner deletion, main Rust application and the unchanged complete performance
  campaign remain open. No ordinary performance keep or release readiness follows.
- The isolated [borrowed geometry action component](../reviews/2026-11-04-borrowed-geometry-actions.md)
  replaces Postprocess's actual dynamic work/permit adapter with an outer-owned action host
  over the existing exclusive `WorkView` kernel. Actual normal/triangulation events, weights,
  scratch share work, completed prefixes and cancellation cadence remain. Source2 passes
  3,473 workspace runtime tests (6 existing skips), 144 affected doctests, 138 independent
  consumer tests (1 existing skip), strict workspace/consumer lint and both format checks.
  Independent connected-source review closes both private-oracle relocation gaps; independent
  final source/evidence association also passes. The semantic/Import/startup numeric consumers,
  complete borrowed backing, current-state attachment, shared promotion, exact accounting, main
  application and the unchanged ordinary Windows/Linux comparison remain open. No new
  performance measurement or release acceptance follows from this structural checkpoint.
- The isolated [host-owned semantic-work component](../reviews/2026-11-04-host-owned-semantic-work.md)
  replaces the real Core/Import/Postprocess owning-session callers with a retained callback
  borrowing the actual unique invocation. Client begin/finish methods and their old consumers
  are deleted; returned plans keep external input and outer reservation lifetimes after the
  numeric loan ends. Frozen source4 passes 3,478 workspace tests (6 existing skips), 149
  doctests, 12 focused semantic tests, 138 consumer tests (1 existing skip), both strict lints
  and both format checks, with exact source/map/log association. Independent source review
  identifies two direct host test gaps; production-identical source5 adds three tests/four
  executions and passes 15 focused tests, Operation strict lint and formatting. Final independent
  source/evidence review verifies both associations and closes both gaps without a finding.
  Import/detection/startup numeric consumers, attachment, promotion, deletion, main application
  and the unchanged Windows/Linux comparison remain open. This component supplies no new
  ordinary performance keep.
- The isolated [borrowed singleton parser facade](../reviews/2026-11-04-borrowed-parser-replenishment.md)
  forwards the existing replenishment kernel without resetting channels or control debt.
  Actual borrowed Operation and public Import comparison regressions pass within 1,450
  affected runtime tests (zero skips) and 140 doctests; strict workspace lint and formatting
  also pass. Independent source/evidence review verifies the actual probes and exact
  terminal association without a finding. This supplies a required
  comparison-loop capability, not complete Import migration, an ordinary performance
  improvement or release acceptance.
- The isolated [finite lexical-host Windows checkpoint](../reviews/2026-11-04-lexical-host-performance-checkpoint.md)
  completes four C,N,N,C campaigns with the unchanged eight selected cases, 31 memory
  lanes, three warmups and twenty retained pairs. Each campaign records external build/test
  contention after a quiet preflight, so ordinary performance acceptance remains HOLD.
  Report-derived repeated OBJ/STL pipeline decreases coexist with no-op preparation and
  extraction increases, higher whole-child working sets and six of eight pipelines still
  above 3x Assimp. Independent reconstruction verifies all 5,704 children and 3,596 summary
  slots with zero validation errors and unchanged reported work/output signatures. No
  performance keep, no-regression or complete Windows/Linux 110-lane claim follows. Full-service
  Import migration, actual backing/attachment/promotion, obsolete-owner deletion, main Rust
  application, uncontaminated confirmation and the complete final acceptance remain open.
- The isolated [trusted parser scan-close prerequisite](../reviews/2026-11-04-trusted-parser-scan-close.md)
  expresses a real text boundary separately from failure and exhausted success, while
  retaining the existing single parser host/kernel. Source1 passes 2,317 selected runtime
  tests, 142 doctests, strict workspace lint and formatting; its test-only source2 closes
  two review-requested direct branches with six focused tests and affected lint/formatting.
  Final independent review closes both gaps and verifies the separate terminal associations
  without a finding. The actual text/PLY/service/detection consumers and old-adapter
  deletion are still open; no new ordinary timing or release
  acceptance follows from this required migration mechanism.
- The isolated [borrowed progress component](../reviews/2026-11-04-borrowed-progress-storage.md)
  replaces the progress state's complete Ledger reference with a borrowed view of the
  same progress evidence storage, then binds actual metadata beside the unique work
  view. Operation/Import root and parser forwarding preserve the existing two polls,
  capacity release and first-cause behavior. The original source1 focused failure is
  retained; test-only source2 corrects a supplemental-cause assertion without changing
  production. It passes four focused tests, 142 affected doctests, 3,495 workspace
  runtime tests (six existing skips), 138 consumer tests (one existing skip), both strict
  Clippy gates and root formatting. Independent source review finds no new production defect
  and retains two non-blocking direct-coverage limits for the connected Import migration.
  Independent terminal evidence review verifies both source associations and all actual
  results without a mismatch; those coverage limits remain open.
  Real text/format/detection consumers, obsolete-owner deletion, attachment/promotion,
  main Rust integration and complete ordinary performance acceptance remain open.
- The isolated [current-state bound invocation](../reviews/2026-11-05-current-state-bound-invocation.md)
  binds the original account/source/control/start admission before prework, attaches against
  the same current ledger and connects actual Core, Import and Postprocess entries. Final
  source6 passes 3,551 workspace tests (six existing skips), 172 doctests, 138 consumer tests
  (one existing skip), both strict lints and both format checks, with exact source/map/log
  association. Independent source and repair reviews retain bounded coverage limits. The
  real source5 default-stack failure is repaired by separating initialization from the
  persistent parser frame; unchanged 1 MiB main-stack probes pass. This is a correctness
  prerequisite, without a new ordinary timing keep. Original-account lifetime through
  unattached finalization, honest unattached facts, actual pre-attachment preparation,
  backing/promotion, remaining owning-path deletion, main integration and unchanged full
  Windows/Linux time/memory and separate heap gates remain open.
- The isolated [shared action-region checkpoint](../reviews/2026-11-05-action-region-and-bound-prework.md)
  repairs actual ignored-error and caught-unwind reentry, using one guard/runner for
  attached and bound prework. Source2 passes 3,557 workspace tests (six existing skips),
  174 doctests, 138 consumer tests (one existing skip), both strict lints and both format
  checks. Independent source/evidence review verifies the original red run, source1
  fixture failure, exact Requested-41 correction and final frozen qualification. Actual
  late preparation, original-account unattached finalization, backing/promotion, owning
  path deletion, main integration and full ordinary time/memory/heap acceptance remain
  separate advancing work, without a new performance keep.
- The [actual preparation-before-attachment checkpoint](../reviews/2026-11-05-preparation-before-attachment.md)
  connects direct original-account completion and actual Postprocess prework before
  retained write attachment. Source3 passes 3,565 workspace tests (six existing skips),
  183 doctests, 138 asset-consumer tests (one existing skip), strict lints, formats,
  standalone Scene configurations and fuzz compilation/library regressions. Fresh
  retained compiler and default-stack artifacts close the bounded source/evidence cut.
  A demonstrated tangent preallocation breach is repaired by removing dead zero writes
  and allocating after the first computed face; original budget/memory/source assertions
  remain. Native convergence, allocation-free empty/single calls, backing/promotion,
  remaining owning-path deletion and indexing remain open. The exact reviewed 260-input
  overlay is applied to main after standalone checks and a pinned Linux registry ASan
  smoke; main-root strict compilation and formatting pass. A fresh exact-current control
  worker is retained; integrated-candidate native performance acceptance is still open.
- The integrated [outer-host Windows comparison](../reviews/2026-11-05-outer-host-windows-comparison.md)
  completes six original fourteen-case/110-lane campaigns. Independent reconstruction
  checks 30,360 native children and 60,720 output files without an integrity error or
  reported semantic/work-signature change. Large repeated-OBJ preparation improves
  about 31% and pipelines about 14%, but all twelve glTF import/pipeline lanes regress,
  103 initial whole-child elapsed medians rise and 98 peak working sets grow. N0 still
  has 58/110 timing lanes above 3x, 37 at least 10x, and nine of 28 pipelines above 3x;
  measured whole-child memory remains within 3x. Later rounds record tool overlap,
  so there is no complete clean confirmation. All six rounds and costs are retained;
  performance keep and release remain HOLD. Twelve exact-source Linux instruction
  profiles are collected for attribution, not the remaining full ordinary Linux gate.
- The [parse-metadata borrowing checkpoint](../reviews/2026-11-05-parse-metadata-borrowing.md)
  removes repeated by-value metadata/service expansion from the real Import root,
  child and parser roles, preserving the numeric owner and all existing work/control
  calls. Independent source review finds no P1/P2 defect; the exact source2 passes
  3,565 freshly compiled workspace tests (six existing skips), 188 doctests including
  five new service-loan contracts, 138 asset-consumer tests (one existing skip), both
  strict lints and both format checks. Independent final evidence review verifies
  all seven commands, 1,448-file maps and fresh main-root artifact association.
  An earlier cached isolated-workspace runtime result is explicitly excluded and
  preserved. The measured Linux copy edges motivate this candidate; ordinary timing,
  memory, backing/promotion, obsolete-owner deletion, indexing and full convergence
  remain open. A finite full-matrix four-campaign decision precedes candidate build.
- The metadata successor's [ordinary/instruction/heap report](../reviews/2026-11-05-parse-metadata-borrowing-performance.md)
  retains four complete Windows matrices and independent reconstruction of 20,240
  children / 40,480 logs. Grid import/pipeline costs fall descriptively, while small
  STL/F64 preparation and selected commit peaks grow. Tool observations 0/70/27/27
  prevent clean confirmation; candidate timing breaches remain 57/59 above 3x and
  nine pipelines above 3x in both rounds. All measured whole-child memory stays
  within 3x. Six exact-source Linux profiles independently verify reduced copying
  and 7.31% fewer grid whole-worker instructions, not Windows timing causation.
  Eighteen separate heap observations have identical control/candidate reported
  calls and rounded peaks; analysis/stderr temporary-count differences are retained.
  Performance and release remain HOLD; live typed-table producer migration and all
  remaining backing/promotion/deletion/indexing and final Windows/Linux gates persist.
- The [actual typed-table Producer migration](../reviews/2026-11-05-borrowed-typed-table-producer.md)
  borrows the unique numeric view and existing frame, deleting the former owning permit,
  facade, admission shim and unused checkpoint methods. Independent source reviews close
  actual prepare-share, prepare-cancellation and consumed-denial cases. Frozen source3
  passes 3,566 workspace tests (six existing skips), 188 doctests, 138 asset-consumer tests
  (one existing skip), both strict lints and both format checks. The interrupted QA1 retains
  its first three terminal gates; QA2 verifies that unchanged source/log association and
  completes only the remaining four, with actual terminal success and independent final
  evidence review. This source qualification supplies no new ordinary performance result.
  Internal/protected/share migration, obsolete parser deletion, backing/promotion/exact
  accounting/indexing and the unchanged full Windows/Linux/heap release gates remain open.
- The [obsolete aliasable parser chain](../reviews/2026-11-05-parser-compat-retirement.md)
  is deleted across its numeric, retained and concrete layers. Four integration traces
  now use the actual exclusive host; useful numeric oracles move to the existing kernel.
  Independent source reviews preserve raw unresolved-frame versus borrowed-view semantics
  and verify unchanged kernel/real-host bytes. The frozen integrated source passes 1,047
  Operation runtime tests, 128 Operation doctests, workspace strict lint/compilation,
  138 consumer runtime tests (one existing skip), consumer strict lint and both formats.
  Independent final evidence review verifies those affected gates; the preceding full
  workspace run is not reassigned to this source. This is a prerequisite deletion without
  a new performance result. Remaining retained-work/backing/promotion/accounting/indexing
  and full ordinary Windows/Linux/heap convergence stay open.
- The [typed-storage sharing leaf](../reviews/2026-11-05-borrowed-typed-storage-sharing.md)
  now borrows the unique numeric view through the existing clone/handle executor.
  Its discarded work-control construction and unused dimension argument are removed.
  Ordinary external quiescence, Internal nesting, physical prefixes and fail-closed
  abandonment remain. Independent source and test-only reviews find no blocking
  defect and retain bounded coverage limits. Final source3 passes 1,054 Operation
  tests, strict workspace/consumer lint, 138 consumer tests (one existing skip)
  and both formats; source2's 188 doctests qualify identical production bytes.
  QA1's test-only Clippy failure is preserved and fixed before successful QA2.
  This does not complete connected canonical/protected borrowing or remove LocalWork.
  There is no new performance result; the full convergence and release gates remain open.
- The [connected retained-work cut](../reviews/2026-11-05-connected-retained-work-borrowing.md)
  moves actual canonical creation/rewrite, inheritance, ordinary/protected evidence
  and controlled validation onto one unique numeric root beside shared storage.
  Deferred binding, current joint attribution, protected poll policy and completion
  before physical commit/proof acceptance remain connected at their real boundaries.
  The three aliasable owning permits and their remaining-count mirror are removed.
  Forgotten permissions reject success while the real host resolves its frame;
  numeric panic cleanup preserves physical poison and unverified charges.
  Independent production/test reviews find no blocking defect and retain the direct
  protected-panic coverage limit. Frozen final source passes 3,589 workspace tests
  (six existing skips), 188 doctests, 138 consumer tests (one existing skip), both
  strict lints and both formats. The seven terminal gates retain all 1,165 input
  identities. This is source qualification without new timing/memory/heap acceptance.
  Remaining actual backing/startup/share/cold users, obsolete execution-facade
  deletion, fixed/dynamic backing, shared promotion, exact accounting, indexing and
  the unchanged complete Windows/Linux comparisons remain required. Release stays HOLD.
- The [execution facade retirement](../reviews/2026-11-05-execution-facade-retirement.md)
  moves actual retained-backing admission to the unique numeric/storage split and
  deletes the old work/group/permit/services facade with its forwarding methods.
  Nineteen integration cases and the ignored manual diagnostic now use actual
  invocation/action hosts; all 23 parent and 19 child tests remain. Independent
  final production and test reviews find no blocking source issue. Frozen source2
  passes all seven serial gates: 3,589 workspace tests, 187 doctests, 138 consumer
  tests, both strict lints and both formats, with the six/one existing skips.
  The diagnostic-only Clippy failure is fixed locally and retained in QA1.
  Independent final evidence review reconciles all seven commands/logs and frozen
  source identities without a blocking mismatch. Next, measure the cumulative qualified
  successor against `308e9ab` using the prospectively fixed original Windows
  matrix before another representation change. Startup's four account shares,
  publication opening and both table creations form the next coherent owner cut;
  cold numeric users, fixed/dynamic backing, shared promotion, exact accounting,
  indexing and all full Windows/Linux/heap convergence gates remain open.
- The cumulative [retained-owner attribution and heap checkpoint](../reviews/2026-11-05-retained-owner-attribution-and-memory.md)
  measures immutable `05b7d9b` against `308e9ab`. Six GNU whole-worker profiles
  reduce instructions by 0.45% to 7.14%; eighteen fresh heap observations retain
  identical Baozi calls and rounded peaks in every selected case. These bounded
  diagnostics show no heap benefit or native timing conclusion. The fixed Windows
  C0/N0/N1/C1 sequence completes only C0 (110 lanes, no failures, 41 named-tool
  overlap snapshots); N0 cannot start within its 180-second quiet-window limit.
  The first unstarted C0 attempt and prospective reschedule remain recorded.
  Candidate ordinary timing, whole-child memory, confirmation and full ordinary
  Linux remain uncollected. Independent review reconciles GNU archives, profile
  totals and all eighteen heap identities/logs without a numerical mismatch;
  eleven historical Windows mixed-newline files retain only their earlier
  reconciliation record rather than newly reproduced raw bytes. Continue the
  real startup/publication/table owner cut before fixed/dynamic state and shared
  promotion; preserve the incomplete performance gates and release HOLD.
- The [startup/publication share implementation](../reviews/2026-11-05-startup-and-publication-share-borrowing.md)
  moves the four startup account shares, publication opening and both table
  creations to the existing numeric/storage split. Startup stores a deferred
  eight-entry layout, preserving pending consumption and live/work/commitment
  precedence. RetainedShareWorkPermit and the now-production-dead
  LinearWorkBatchPermit are deleted with their constructors; useful test oracles
  move to the actual kernel and hosts. Final source1 compiles without warnings,
  and independent production/test source reviews find no blocking issue.
  Final source2 passes seven serial gates: 3,592 workspace tests, 187 doctests,
  138 consumer tests, both strict lints and both formats, with existing skips.
  QA1's invalid test-fixture failure is retained and the one-row correction has
  independent source follow-up. Final evidence review reconciles all seven
  commands/logs and frozen/staged inputs without a blocking mismatch. This cut
  has no performance acceptance. Next remove the remaining numeric
  RefCell, LocalWork and cold BoundWorkGroup together with shared-write API
  migration, retaining the storage Arc identity and existing kernel. Fixed/dynamic
  backing, shared promotion, exact accounting, indexing and full convergence stay open.
- The [direct numeric WorkState implementation](../reviews/2026-11-05-direct-work-state.md)
  stores the original catalog/rows directly in Ledger and deletes LocalWork,
  BoundWorkGroup, numeric RefCell availability probes and recovery. Cold writes,
  policy/mode changes, control checkpoints and deferral sealing use mutable loans;
  range/evidence/snapshot readers and storage release keep shared access. Account
  preflight still covers failure-work transfer; startup defers work errors after
  live checks and snapshot allocates before checking work. Actual frame ownership,
  dirty terminal evidence and original storage Arc identity remain. Independent
  design and final production/test source reviews find no blocking issue. Workspace
  production/all-target compilation passes without warnings. QA1 retains its
  test-fixture type-complexity Clippy failure; source2 names that same tuple and
  passes all seven fresh serial gates: 3,588 workspace tests, 187 doctests,
  138 consumer tests, both strict lints and both formats, with existing skips.
  Independent final evidence review reconciles all seven original logs/commands,
  staged/frozen inputs and the sole source2 alias delta without a mismatch. No new
  timing or memory result is attributed to this source. Fixed/dynamic backing,
  promotion, exact accounting, indexing and all original convergence gates remain open.
- The [direct-state native/heap/placement checkpoint](../reviews/2026-11-05-direct-work-state-performance.md)
  completes all four original GNU campaigns, each with fourteen cases, 110 lanes,
  three warmups and twenty retained AB/BA pairs, without command failures.
  Candidate N0/N1 each retain sixteen of twenty-eight pipelines above 3x Assimp;
  all whole-child RSS ratios remain below 3x. C0/C1 overlap Windows build/test
  tools in 142/256 and 94/282 sampled instants; candidate windows have no named
  overlap. No clean source-change timing effect follows. Separate six-case
  whole-worker Ir changes are -0.369% to +0.041%; eighteen fresh heap observations
  retain identical Baozi allocation counts and rounded peaks. The Windows
  sequence reaches its start limit before C0 and has no ordinary samples.
  An unchanged-debug0-binary diagnostic then moves only executable placement
  from `/mnt/f` v9fs to the Linux filesystem: three small Baozi pipeline medians
  fall from 39–62 ms to 0.58–1.05 ms, with whole-worker median major faults
  204–266 to zero; Assimp also improves. This is environmental evidence, not a
  source optimization or acceptance substitute. Preserve all original samples
  and load records. Prospectively qualify the full original Linux matrix with
  both engines and corpus on the Linux filesystem before choosing a parser or
  algorithm rewrite from these ordinary times. Refresh the lexical-root design
  against current six-dimension/three-phase direct runtime; fixed/dynamic roots,
  shared promotion, exact accounting, terminal facts, cleanup/indexing and every
  original Windows/Linux release gate remain open.
- The [Linux-filesystem full-matrix checkpoint](../reviews/2026-11-05-linux-filesystem-convergence.md)
  completes all four original campaigns and both functional preflights with
  unchanged debug0 worker bytes, same input bytes, clocks and semantics. Each
  campaign has 110 lanes and 5,060 successful native children. Actual exec96388
  terminates zero; raw copies occur afterward and are excluded from child clocks
  and campaign windows. Candidate N0/N1 still have 69/70 stage lanes above 3x,
  34/38 at least 10x and fourteen of twenty-eight pipelines above 3x in both.
  Shared glTF 32 file and memory exceed 10x in both repeats; all whole-child RSS
  ratios remain below 3x (maxima 2.097/2.051). Windows named tools overlap C0,
  N0 and N1 in 26/54, 33/52 and 38/54 snapshots; C1 has 0/55. Preserve mixed
  contrasts, RSS increases, all tails and original DrvFS data. No clean source
  gain or release acceptance follows. Next address both finite initialization
  ownership and large-model per-element parser/numeric/execution costs; replacing
  initialization Vecs alone does not establish a scaling fix. Current-root design
  review requires one actual mutable storage authority with storage-free progress
  metadata, explicit unused/failed-start backing loans, movable storage source
  slots and genuine dirty containment distinct from clean finite facts. Main
  selected dispositions remain uncompiled; retain the existing invariant layout
  brand and one kernel. All original Windows/runtime/performance gates remain.
- The [parser/pipeline root-cause checkpoint](../reviews/2026-11-05-parser-pipeline-root-causes.md)
  completes two separate nine-case same-invocation diagnostics, each with three
  warmups and twenty observations, plus pinned Assimp source comparisons and
  independent source/task/work/statistical review. Large grid/glTF/STL pipelines
  spend about 99%/92%/91% in import. Internal spans locate grid record/token
  parsing at 44.78% of import, its two text passes at a combined 24.98%, and
  STL-1024 semantic preflight at 61.68% versus 3.78% in binary decode. glTF-32
  normalization is 33.56%; syntax/index and typed decode are about 8% each.
  These are instrumented locating intervals, not protocol CPU shares, expected
  savings, paired Assimp effects or release evidence. Next cut the real common
  semantic-unit execution owner and OBJ record/lexical boundary, preserving
  exact prefixes, resource/control/failure semantics and one numeric kernel.
  Address repeated glTF normalization before a whole JSON parser replacement.
  Finite roots, terminal facts, unchanged selection, pool-transfer ownership and
  all original integrated correctness/Windows/Linux performance gates remain.
- The [scalar/OBJ execution cut](../reviews/2026-11-05-semantic-copy-and-obj-lexical.md)
  implements Copy-only semantic units over one real reusable frame and binds
  OBJ/MTL lexical work once per invocation. Per-unit budgets, control frontiers,
  completed prefixes and owned-callback cleanup remain; scalar inner catches
  and successful per-character services acquisition are removed. Independent
  source/test reviews and seven integrated gates pass, followed by a test-only
  numeric-prefix regression update and focused/lint/format qualification.
  Native performance is not yet qualified. Next remove integer attribute no-op
  inspection and correct all four retained-work formulas together, including
  selection output lengths; keep actual copy, sparse/topology/range/schema work.
