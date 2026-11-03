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
- Complete release-convergence qualification: pending; all final gates remain closed.
- The [normal source-value follow-up](../research/2026-11-03-normal-generation-standards.md#subsequent-selected-value-audit-and-windows-reproduction)
  reproduces selected FLOAT NORMAL `(0,0,2)` acceptance on Windows with a passing bit-exact
  unit control. This P2 requires an explicit glTF admission tolerance/policy and selected-owner
  regression coverage before claiming direction-length validation. Preserve quantized decoded
  magnitudes, signed morph deltas, generic core authored contracts and the frozen benchmark
  task. This is independent correctness work after the current bridge cut, not a normal
  estimator change or performance keep; the failed probe and source-qualified receipts remain.
