---
title: "Define selected glTF FLOAT base-direction admission"
type: fix
status: completed
date: 2026-11-03
parent: 2026-11-03-0900-perf-assimp-convergence-plan.md
---

# Selected FLOAT Base Directions

The [normal standards audit](../research/2026-11-03-normal-generation-standards.md)
reproduces selected FLOAT NORMAL `(0, 0, 2)` being accepted. The selected attribute collector
checks finite values, stored bounds and TANGENT.w, but lacks a base-direction length policy.
This correctness slice accompanies the performance work. It changes neither the measured
Smooth estimator nor the comparison tasks, work vocabulary, timing boundary or release target.

## Decision

For selected FLOAT base NORMAL and TANGENT.xyz, widen the three decoded F32 components to F64,
calculate `(x*x + y*y) + z*z`, and accept an absolute squared-length error at most **1e-4**.
This is a named Baozi format-admission tolerance, not a normative glTF constant or a claim that
all conforming clients must reject malformed assets. It admits ordinary F32 rounding around a
unit direction, while rejecting explicit zero, substantially nonunit, overflowing-F32-square
and subnormal directions. Widening before multiplication keeps every finite F32 input within
the F64 arithmetic range. Preserve all admitted authored F32 bits, including signed zero.

Enforce the rule inside `parser/accessor.rs::collect_mesh_attribute`, after existing finite
and TANGENT.w checks, after logical sparse replacement, and before storing/completing the
sample. Do not inspect replaced dense values or add a second payload traversal. Keep the
current fixed mesh-sample work envelope and polling boundaries; an invalid sample completes
no attribute cell and retains all earlier completed cells.

Quantized signed-normalized directions keep their distinct decoded-magnitude policy. They
may have length error and are not subject to this FLOAT tolerance. Morph NORMAL/TANGENT values
are signed displacement vectors: zero and nonunit values remain valid. The format-independent
core's authored finite-direction contract and generated-normal predicates are unchanged.

This is a stricter pre-release input contract. There is no tolerance option, normalization,
compatibility decoder or new source representation. The supported-profile documentation must
state the policy and its scope explicitly; it must not claim full glTF conformance.

## Implementation and Verification

1. Preserve the Windows red reproduction and add public dense/sparse regressions, including
   full sparse replacement with and without a base, replacement of an invalid dense value,
   zero/nonunit/extreme F32 values and exact completed-sample prefixes.
2. Review the selected policy and actual collector/morph ownership independently. Add adjacent
   F32 controls on both tolerance boundaries, authored-bit preservation, finite/sign error
   priority and cancellation before direction validation, storage, and completion. The iterator
   may already have decoded that sample before the loop's control poll.
3. Add only the format-local constant and same-pass predicate. Run focused regressions, all
   glTF tests including existing quantization and morph controls, strict glTF lint and fmt.
   Correct unrelated invalid test fixtures only if their original purpose requires a valid
   direction; retain their substantive assertions and record those corrections explicitly.
4. Review the actual source/test diff independently and verify the maintained public asset
   consumer. Record exact source identities, failures, command outcomes and test run IDs, then
   make a scoped local Conventional Commit. No publishing, remote push/PR or native promotion.

Main alone runs Cargo, profiling and ordinary measurements. Root/attachment design review is
read-only in separate agent scopes. The all-110-lane Windows/Linux performance goal remains
open; this policy correction is not a speedup or a release qualification.

## Result

The same-pass predicate, eight public regression tests and purpose-preserving legacy fixture
repairs are complete. Windows Rust 1.95.0 locked/offline qualification passed all 525 glTF
tests, all 138 maintained asset-consumer tests (one existing manual skip), strict Clippy for
both scopes and glTF fmt. The source-qualified command receipt is
`target/review/convergence/gltf-base-direction-qualification-1.json`; source review accepted
the three Rust paths and two supported-profile documents. Its plan wording finding is corrected
above. The final receipt records exact source hashes and retained failed checkpoints.
