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
- [Pinned Assimp principles and Rust ownership audit](../research/2026-11-03-assimp-principles-and-rust-ownership.md)
  distinguishes loader-specific buffering, different normal algorithms, genuinely escaping
  storage ownership and removable per-getter/lifecycle work. Independent design review requires
  correcting promotion and its footprint contract before implementation. A host-owned exclusive
  execution-context alternative is being assessed; no design is called optimal merely because
  it passes the borrow checker or has fewer allocations.
- Complete release-convergence qualification: pending; all final gates remain closed.
