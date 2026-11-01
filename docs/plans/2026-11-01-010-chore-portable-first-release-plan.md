---
title: First Portable Release Candidate Preparation
type: chore
date: 2026-11-01
status: completed
---

# First Portable Release Candidate Preparation

## Outcome and boundary

Continue the separate release track of the
[asset workflow plan](2026-11-01-001-feat-asset-workflow-expansion-plan.md)
after the dependency-closure slice in `e7e9b00` and its evidence in `f08d39d`.
The user approved portable first-release preparation on 2026-11-01, then asked
for a broader functionality and interface audit against Assimp before selecting
the remaining implementation work. First-release usability is the priority.

Select the complete 0.1.0 portable package/format candidate, account for every
optional production dependency, observe current public registry names and
owners, reconcile manifest eligibility with evidence, and identify or execute
the missing local qualification gates. Produce a reviewable preparation result
and scoped commits. Registry publication, yanking, owner changes, credentials,
pushes, PRs, native registration and a general publication framework are outside
this slice.

## Initial decisions

- Keep STL and OBJ/MTL as default formats; retain opt-in PLY and the documented
  selected glTF/GLB profile. Evaluate all twelve intended production packages
  together, including `baozi-gltf-json` and `baozi-gltf`. A disabled feature does
  not remove its dependency from the publication requirement.
- Treat manifest eligibility, local archive qualification, public-registry
  availability, format maturity and target promotion as separate decisions.
  Existing native packages stay `publish = false` and outside this train.
- Capture public registry observations with exact timestamps, status codes and
  stable owner IDs. An observation is neither a reservation nor the approved
  expected-state authority required for actual publication by ADR 0006.
- Reuse revision-qualified results. Fill bounded local gaps where practical;
  list unavailable hosted/target evidence explicitly. Do not infer a run from
  workflow configuration or promote a target using historical partial results.
- Use existing Cargo/package/consumer tooling. Add a check only when it proves a
  concrete missing boundary, without implementing a registry service or source
  analysis framework.

## Parallel lanes

Base: `f08d39d37ae44baae856cc05a918efaf31613324`,
`feat/real-asset-consumer`. The main thread owns every edit and serial Cargo
execution with one build job and the existing target directory.

| Lane | Ownership | Output | Verification |
| --- | --- | --- | --- |
| Main | Plan, public registry observation, implementation integration and commits | Exact package/format decision, registry evidence, bounded fixes and execution review | Public API receipts, manifest/artifact inspection and relevant local checks |
| Package review | Read-only manifests, normalized artifacts and release policy | Production/optional closure, eligibility and development-dependency hazards | File references and existing qualified evidence; no Cargo |
| Target review | Read-only ADRs, reviews, consumers and CI | Revision-specific target evidence and feasible gaps | Executed receipts separated from configured jobs; no Cargo |
| Final review | Read-only completed diff | Correctness and claim-boundary findings | Independent review after implementation |

All lanes preserve unrelated user files and may not modify AGENTS/CLAUDE or
environment settings. Native Codex agents only.

## Acceptance

1. A durable release-candidate record names the twelve packages, versions,
   dependency order, default/optional format scopes and exclusions. Any change to
   this set or eligibility flags is justified against existing authority.
2. All twelve names receive timestamped public registry observations. Present
   packages include observed stable owner identities and exact relevant version
   state. Errors stay errors; absent names are never treated as owned/reserved.
3. Production and optional dependency declarations are publishable in form;
   local artifacts and consumer evidence are traced to exact revisions. Any
   development-only packaging hazards are fixed or explicitly retained as release
   blockers rather than hidden by the local rehearsal.
4. The target evidence inventory names actual target, toolchain/profile, scope,
   source revision and missing release gates. New executions identify their
   tested source; unexecuted hosts and browser runtime remain explicit gaps.
5. Applicable focused checks and an independent review pass. The completed slice
   is committed with Conventional Commits and leaves a concrete next step. No
   registry mutation or user-facing maturity/target promotion occurs.

## Usability audit extension

Compare Baozi's current implementation and actual application workflows with the
pinned local Assimp reference. Distinguish confirmed defects, missing interfaces,
deliberate profile restrictions, and optional breadth. Trace each important gap
through public entry points, retained data and an application consumer; do not
infer support solely from descriptors or private representability tests.

Additional read-only lanes share the base and restrictions above:

| Lane | Ownership | Output |
| --- | --- | --- |
| Public interfaces and semantic flow | Import, Scene readers, ownership, material/animation/provenance access and real-asset consumption | Concrete caller obstacles, silent-loss defects and Assimp interface comparison |
| Hostile-input and failure behavior | Limits, cancellation, IO, parser failures and exposed unsafe/panic boundaries | Evidence-backed first-release safety or recovery gaps |
| Geometry and integration design | Postprocess, vertex/index extraction, hierarchy/animation workflows and module placement | Missing common workflows, justified redesigns and bounded acceptance cases |

The main thread owns synthesis, Assimp reference verification, release-policy
updates, focused reproductions and the next implementation sequence. Do not start
new format families or a wholesale Scene rewrite from a comparison checklist.

The user explicitly changed the placeholder retirement sequence: record its
noncanonical status before the train; publish and verify the canonical facade
and its complete dependency unit first; yank `baozi 0.0.1` afterward. Update the
policy and ADR consistently. This decision does not authorize any registry write.

Acceptance also requires a ranked, source-linked audit and a concrete first-release
implementation plan. A completeness claim must name its supported workflows and
restrictions, rather than claim feature parity with Assimp.

## Result

Completed preparation and the usability audit, not publication readiness. The
[audit](../reviews/2026-11-01-first-release-usability-audit.md) fixes the twelve-package
candidate, records all public name/owner observations, separates actual target
evidence from configured jobs, and identifies the remaining correctness/interface
work. Five read-only native Codex lanes and two cross-checks support the review.

`dfd512fdc7d2de9408ea06aa53e8c9702724efb6` removes the repository-only versioned
test helper from normalized manifests. Windows stable 1.99.1/release passes all
twelve actual artifact-consumer cases; all five affected archive roots resolve.
Independent diagnostic consumers reproduce five issue classes, including the
three P1 release gates. These successful reproductions are not evidence that the
underlying defects are fixed. Remaining broad target runs are deferred until that
corrected subject exists.

The [next implementation plan](2026-11-01-011-feat-first-release-usability-plan.md)
owns correctness, selected-primitive batch processing and complete application
consumption. PLY/glTF eligibility stays disabled, native registration stays empty,
and no registry mutation, push or PR occurred. The placeholder retirement policy
now follows the user's post-publication sequence.
