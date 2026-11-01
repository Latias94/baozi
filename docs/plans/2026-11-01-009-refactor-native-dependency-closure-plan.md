---
title: Normally Resolvable Maintained Native Dependency Closure
type: refactor
date: 2026-11-01
status: in_progress
---

# Normally Resolvable Maintained Native Dependency Closure

## Outcome and boundary

Continue the separate release track of the
[asset workflow plan](2026-11-01-001-feat-asset-workflow-expansion-plan.md) after
the unlit-material slice in `ec2958f`. The user authorized this dependency and
package-verification slice on 2026-11-01. Remove the independent asset consumer's
root patch requirement at the dependencies that own the affected edges.

This slice prepares and rehearses distribution. The native adapter remains an
unpublished, unregistered candidate; runtime target qualification, repository
capture admission, registry publication and the first release format selection
remain independent decisions. No registry write, push or PR is authorized here.

## Decisions

- Maintain the reviewed upstream 4.0.3 source as three distinct packages:
  `baozi-cap-primitives`, `baozi-cap-std` and `baozi-cap-fs-ext`, initially 0.1.0.
  Preserve library aliases, original copyrights/licenses, complete source and
  explicit upstream archive provenance. The upper two packages need only
  dependency-identity changes; the existing bounded manual-link implementation
  remains in the primitives package.
- Each maintained edge uses its own package identity, a relative development
  path and an exact distribution version. Remove both native-candidate and
  asset-consumer `[patch.crates-io]` workarounds. Keep optional production edges
  normally resolvable, including repository capture and upstream helper features.
- Audit all three local identities, exact paths/versions and reviewed upstream
  provenance. Continue to reject foreign/duplicate backends and preserve feature,
  target and portable-facade isolation. Upstream checksums identify original
  archives, never the modified maintained source.
- Reuse the package checker with standard Cargo directory sources for a local
  unpublished-registry rehearsal. Build actual `.crate` archives, extract their
  normalized manifests, and resolve a version-only independent consumer against
  these exact artifacts and its registry dependencies. Do not use root patches,
  rewrite artifact dependency paths or substitute checkout source. This proves
  package closure, not that unpublished packages already exist on crates.io.
- Add native default/optional-closure checks and the missing Linux packaged
  consumer job. Keep the existing scene-consumer feature matrix and runtime versus
  compile-only claims explicit. Avoid a registry server or generic registry tool.

## Execution and acceptance

1. Copy and byte-verify the original upper dependency archives, establish distinct
   maintained identities, wire the normal closure and regenerate only affected
   lockfiles. A source consumer with no patch must resolve and build both native
   feature modes.
2. Update the target-filtered dependency contract and negative regressions, then
   compare checked-in closure baselines with Cargo's actual graph. Target sets and
   the empty release registry remain unchanged.
3. Archive and extract the complete local production closure. Resolve independent
   consumers with version-only roots and no patch, verify every maintained source
   comes from extracted artifacts, then build/run the selected feature cases.
   Exercise Windows locally and Linux through the available WSL environment;
   record missing hosted targets as unexecuted.
4. Run focused native/consumer/audit regressions, formatting and strict Clippy
   appropriate to the changed boundaries. Main thread alone runs Cargo with one
   build job and shared target directories; native Codex agents own disjoint audit
   and package-checker files and a read-only architecture review.
5. Update active ownership/ADR/package documentation, commit the implementation,
   record revision-qualified evidence and complete this plan. Preserve unrelated
   user notes and do not publish.

Execution results will be recorded in
`docs/reviews/2026-11-01-native-dependency-closure.md`.
