---
title: First-Release Correctness and Application Workflows
type: feat
date: 2026-11-01
status: superseded
---

# First-Release Correctness and Application Workflows

Superseded by the reviewed [first-release maturation refactor](2026-11-02-001-refactor-first-release-maturation-plan.md).
The audit remains the problem evidence; the replacement plan owns implementation choices, sequencing and acceptance.

## Objective

Make the first canonical 0.1.0 useful for a real Rust asset application: import the
documented four format profiles, inspect all retained semantics, prepare common
geometry with explicit policies, and retain complete application data after the
Scene is dropped. The
[first-release audit](../reviews/2026-11-01-first-release-usability-audit.md)
is the source of findings and evidence limits; the
[preparation slice](2026-11-01-010-chore-portable-first-release-plan.md)
records the registry observation and completed packaging preparation.

This is the next implementation sequence, not a statement that these changes or
release gates have passed. It retains the twelve-package portable candidate,
STL/OBJ defaults, PLY/selected-glTF opt-ins and explicit postprocessing. The native
adapter remains a separate unpublished/unregistered candidate. Actual registry
publication and retirement of 0.0.1 require separate execution authorization.

## Implementation order

Complete the correctness tranche before widening accepted inputs or enabling
publication. Implement each unit with its public reproducer and focused regression
cases, then commit a coherent slice. Existing API shape is not a compatibility
constraint; delete replaced caches, duplicate models and false invariants.

### C1 — Close the glTF backend allocation boundary (A1)

Turn the dense-empty-node counterexample into a regression. Measure concrete
schema sizes and capacities separately from ledger charges; do not confuse either
with process RSS. Establish the allocation boundary before owning deserialization.

Evaluate the smallest correct choice: a proved schema/container envelope using
existing admission counts, or budget-controlled decoding into the selected
profile. Keep the maintained validation fixes and malformed regressions. An
unproven larger JSON multiplier is not acceptance; neither is a second general
glTF validator around the same uncontrolled allocation path.

**Acceptance:** dense Node/Material/nested-collection/extras/string inputs cannot
exceed admitted backend storage through the identified container expansion;
overflow and tight budgets fail before the affected allocation; decoded semantics,
typed failures, cancellation and the known malformed reproducers remain correct.
Document exactly what is bounded, including allocator/container overhead scope.

### C2 — Make PLY numeric precision an explicit owned contract (A2)

Choose the supported geometry precision through parser, canonical validation,
readers and numerical operations before changing a cast. Source-preserving F64
geometry is a justified core refactor if the first scanning/tooling profile
requires it; the current F32 Position invariant must not silently decide that
product contract. Existing F64 custom attributes are not proof that Position or
every postprocess operation supports F64.

**Acceptance:** test all three PLY encodings with large adjacent coordinates,
integer precision boundaries, subnormal/underflow values and signed zero.
Preserved values stay exact at the declared scalar width. Any narrower output
requires explicit loss authority and exact evidence; default imports never report
unmodified success after an inexact projection. Numerical operations either honor
the chosen precision or return an explicit capability error before publication.
Do not add a second Scene or implicit F32/F64 fallback chain.

### C3 — Make derived geometry respect morph dependencies (A3)

Add the missing normal-edit capability checks at the owning transaction seam.
Initially reject replacing a base Normal with retained Normal morph deltas and
removing a base Tangent with retained Tangent deltas. A future rebasing operation
requires its own semantics and evidence.

**Acceptance:** dense/sparse affected deltas reject with the original generation,
snapshot and payload unchanged; Position-only morph fixtures remain supported;
preserve-existing-normal no-ops do not acquire an unnecessary rejection.

### C4 — Remove the remaining confirmed projection/cleanup defects (A4–A7)

- Derive MTL alpha mode from final opacity instead of a sticky duplicate cache;
  cover repeated/interleaved `d`/`Tr` and complete material redefinition.
- Classify custom reader destruction at the IO ownership boundary; preserve
  earlier read/control failure and keep a healthy format registration usable.
- Append structurally unique PLY metadata keys and order them once with bounded
  cancellation/work; test several comment counts and semantic ordering.
- Keep Image/Texture/Sampler names on their actual source-object owners, including
  shared-image and post-edit cases.

These fixes may be separate commits. Do not bundle an unrelated material model,
new fault framework or generalized parser collection layer into them.

### U1 — One useful geometry preparation interface

Build explicit operations over selected primitives with one plan, reconstruction
and generation publication. Reuse the existing edit transaction, borrowed topology
assembly and remap evidence. Add strip/fan list lowering, then a limited triangle
preparation recipe that owns the necessary winding/normal/tangent order. Expose
postprocess limits through the current budget mechanism.

**Acceptance:** indexed/nonindexed strips/fans preserve parity, winding,
degenerates and every selected attribute/deformation/source relation; batch output
matches equivalent single-target semantics; any failed target returns the original
Scene. Use 32/128/512 fixed-size primitives to check bounded near-linear work and
copying. Cover CW/CCW source and target conventions, reflection, custom tight
budgets and cancellation. Callers do not loop over full-Scene rebuilds or invent
intermediate CCW transforms. No generic pass DAG or new Scene representation.

### U2 — Close the application output profile

Complete the independent consumer's selected material/texture contract: every
referenced UV stream, sampler, channel mapping, color interpretation, strength,
blend and media type is retained or explicitly rejected by that output profile.
Distinguish source material absence from effective glTF default material semantics.

Add bounded parent/node-to-instance navigation where the real consumer needs it,
consistent facade format options and runnable memory/sidecar integration examples.
Document costs and authority; no hidden allocating getters or ambient IO promise.

**Acceptance:** implicit versus explicit empty glTF materials have equivalent
effective factors with distinguishable authored provenance; UV1 and shared-image/
different-sampler fixtures produce complete output or a named rejection. Output
remains usable after Scene/evidence teardown. Forest/shared-mesh navigation and
generation errors are covered through public interfaces.

### U3 — Prove the animation consumption contract

Use a small CPU-only reference consumer with hand-calculated Step/Linear/CubicSpline,
quaternion, instance morph override and two-joint skin results. Verify that source
and converted-space evaluations correspond. This proves how applications combine
the existing data without adding an engine, blend graph or renderer to core.

### D — Qualify the actual first-release subject

After correctness and chosen workflows stabilize:

- Correct maintained backend MSRV declarations and installation examples. Decide
  the retained upstream default/all-feature/docs.rs scope and verify it separately
  from the facade's selected features. Root workspace all-features does not cover
  excluded backend packages' optional image/import features.
- Make the excluded backend package/lock inputs explicit in the real publication
  procedure. Keep repository-only test helpers out of normalized manifests; do
  not publish them just to satisfy dev-dependency resolution.
- Bind PLY/glTF publication eligibility to a finite reviewed profile record with
  semantic fixtures, failures/limits, relevant sanitizer evidence and actual
  artifact/target results. Keep `publish = false` until that gate is met; no Stable
  maturity promotion or generic managed-evidence framework is inferred.
- Reuse unchanged exact-source evidence and rerun affected tests. Execute the
  current MSRV/debug and stable/release artifact matrices on intended real hosts,
  current WASI runtime and the declared browser compile scope. Include representative
  new glTF semantics and cross-target canonical digest vectors. Record unavailable
  hosts and profile promotion separately, following ADR 0034.
- Review docs, examples, licenses/notices and changelog against the actual final
  feature set. The twelve-package closure must include all optional production
  edges and correct development-edge publication order.

The current registry observation is preparation only. A future authorized train
must use its reviewed expected state, clean tagged source, normal verified
packaging and matching registry artifact hashes. Publish the facade last, run a
fresh registry-only consumer, then yank 0.0.1 and verify retirement. Do not infer
upload or credential success from local rehearsals.

## Bounded expansion candidates

After C and the chosen U units, prefer one complete `KHR_lights_punctual` or
`KHR_materials_emissive_strength` slice when a representative asset calls for it.
Typed references, units, selected closure, source metadata, conversion and public
consumption must all pass; changing an extension-name list is insufficient.

FBX/USD/Collada/VRM breadth, compression backends, a general mutable Scene builder,
exporters, pixel decoding, runtime animation, mesh/cache optimization and device
partitioning are independent later slices. They are not acceptance requirements
for the documented four-profile first candidate.
