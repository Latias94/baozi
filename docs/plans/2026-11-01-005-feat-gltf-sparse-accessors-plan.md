---
title: Shared glTF Sparse Accessor Decoding
type: feat
date: 2026-11-01
status: completed
---

# Shared glTF Sparse Accessor Decoding

## Outcome

Implement follow-on candidate 5 from the
[asset workflow plan](2026-11-01-001-feat-asset-workflow-expansion-plan.md). Preserve the logical
values of sparse glTF accessors everywhere the current profile accepts their dense equivalents:
geometry, indices, JOINTS_0/WEIGHTS_0, morph deltas, animation and inverse bind matrices. Retain
all contributing buffers in canonical provenance and the selected source closure.

The user authorized this execution and necessary breaking refactors on 2026-11-01. Reuse the
private backend's sequential accessor iterator and existing typed canonical authoring. No second
accessor/JSON model, core sparse rewrite, external/runtime dependency or compatibility layer is required.

## Design

- Validate the optional base layout and both tightly packed sparse ranges before dependency IO.
  Require nonzero sparse count bounded by the accessor count, valid U8/U16/U32 index encoding,
  separate component alignment of sparse offsets, and views without byteStride or target.
- A missing base initializes logical source components to zero. Apply replacements before the
  existing normalized/typed conversion; missing source COLOR VEC3 alpha still converts to one.
- After loading selected buffers, scan each selected sparse index stream once with admitted work
  and cancellation polling. Reject duplicate, decreasing and out-of-range entries before any
  canonical construction or accessor decoding. Do not materialize an extra index/overlay table.
- Admit the complete logical output through current vertex/attribute/topology/morph/animation/
  joint limits and metered collectors. A small sparse payload does not reduce decoded cardinality.
- Replace single-base accessor provenance with an inline set of up to three contributing buffer
  sources. Aggregate owner sources through the existing bounded sort/dedup collectors. Reserve
  capacity and work for all references; propagate complete sources to attributes, primitive,
  morph delta, animation channel/clip and skin authoring.
- Reuse the existing selected closure and source-object links for base/indices/values. Unselected
  sparse dependencies remain unopened. Move selected inverse-bind accessor preflight before IO.

## Execution

1. Inspect shared layout/reader/resource contracts and current Khronos sparse requirements.
2. Implement shared layout and index validation; integrate complete source aggregation.
3. Add independently authored typed regressions: zero and nonzero base, all sparse index widths,
   normalized values, admitted deformation/animation uses, and three external contributing buffers.
4. Verify malformed layouts/indices and logical output budget failures. Assert public provenance
   relations and selected dependency behavior.
5. Run focused nextest, affected workspace/consumer checks, format and strict Clippy serially.
   Record revision-qualified results, update behavior documents and commit the completed unit.

Native Codex agents review decoding assumptions, implement the disjoint provenance storage/caller
changes, and add a new sparse integration test file. Only the main thread runs Cargo.

Completed in `562e6da0f950683c33c063d603e52c4dbf354ba2`. The
[execution review](../reviews/2026-11-01-gltf-sparse-accessors.md) records the design decisions,
malformed-input and admission regressions, independent consumer, exact run identities and final
qualification: 2,608 workspace tests, 32 consumer tests, strict Clippy, formatting and a compile-only
wasm32 glTF library check.

## Sources and boundaries

Primary source: [Khronos glTF 2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#sparse-accessors),
including sparse indices/values and accessor alignment. The implementation uses the maintained
private gltf-rs backend already in this workspace. Assimp reference code remains read-only and
does not supply test fixtures or the semantic oracle.

Do not expand scene selection, extensions, additional joint sets, format breadth or postprocess
policy in this unit. Completion does not imply a new format maturity or target qualification.
