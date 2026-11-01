---
title: Complete glTF Influence Sets
type: feat
date: 2026-11-01
status: in_progress
---

# Complete glTF Influence Sets

## Outcome and scope

Implement the influence-set part of candidate 6 in the
[asset workflow plan](2026-11-01-001-feat-asset-workflow-expansion-plan.md), following completed
shared sparse decoding in `562e6da`. The user authorized continuing this slice on 2026-11-01.
Explicit scene selection remains a separate execution unit.

Preserve every admitted JOINTS_n/WEIGHTS_n pair using the existing variable influence rows.
Do not introduce a fixed-eight limit, truncate or merge lanes, silently normalize weights, or
replace core deformation storage. Keep source order as ascending set index followed by lane index,
including zero and signed-zero weights. Source order is the bounded row ordinal `4*n + lane`.

## Design and execution

1. Replace the format's set-zero rejection and boolean pairing check with paired contiguous set
   validation. Count actual attributes rather than allocating by the largest untrusted suffix.
   Reuse shared accessor type, layout, sparse and normalized conversion checks.
2. Preflight full influence count `4 * vertex_count * set_count` across selected primitive
   definitions before dependency IO. Check row width, offsets, endpoint and byte/count arithmetic.
   Admit decode/assembly work and storage before constructing the complete rows.
3. Decode each set through a metered table of borrowed backend iterators and write Vertex-major
   CSR rows directly. Keep only O(set count) temporary iterator state; do not create complete
   per-set joint/weight copies. Retain complete primitive source provenance through existing
   accessor/source collectors.
4. Preserve the core's all-instance palette interpretation, including zero-weight lanes. Check
   every higher-set input, selected closure and source order through public readers. Resolve
   format-specific weight validation against the pinned Khronos source without adding a generic
   numeric-policy or repair framework.
5. Verify two/three or more sets, sparse/strided/normalized higher sets, malformed pairs/gaps,
   logical budgets, palette and value failures, complete typed provenance and an independent
   application consumer. Use legal self-authored fixtures and exact bit comparisons.
6. Run focused tests, affected workspace/consumer nextest, strict Clippy and formatting serially;
   record implementation identity and results, update profile/progress documents and commit.

Only the main thread runs Cargo, with Rust 1.95.0, locked/offline dependencies, one build job and
the existing target directory. Native Codex agents own read-only design review and disjoint new
importer/consumer integration test files. Existing user notes and unrelated edits remain untouched.

## Normative reference

Use the [official pinned Khronos glTF specification](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/Specification.adoc#skinned-mesh-primitive-attributes),
including indexed semantic continuity, paired skin attributes, weights and palette-local indices.
The maintained backend and core variable rows are existing implementation dependencies. No new
external dependency, snapshot schema, animation runtime or release qualification is required.
