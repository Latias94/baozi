---
status: in_progress
date: 2026-11-01
---

# Complete glTF influence sets: execution review

This record covers the [influence-set execution plan](../plans/2026-11-01-006-feat-gltf-influence-sets-plan.md)
after `42ab367`. It implements the influence half of candidate 6; explicit scene selection remains
future work. Earlier sparse qualification belongs to its own implementation state.

## Behavior and decisions

The adapter reads all admitted paired contiguous JOINTS_n/WEIGHTS_n sets. Both semantic groups
are checked as ascending `0..S` sequences in the existing typed attribute map, then their counts
must agree. This proves exact pairing without allocating a table indexed by an untrusted suffix.
There is no format-specific four/eight-lane or eight-set cap; existing attribute/cell/work/storage
limits govern the admitted set count. Shared accessor validation covers every higher set.

The new private influence module streams each set through borrowed backend iterators into
Vertex-major CSR rows. The temporary iterator table is metered and contains inline RefCells for
local iterator advancement, accommodating MeteredVec's Copy-only mutable slice boundary without
expanding that capability or copying complete joint/weight streams. One reusable metered scratch
row checks joint uniqueness. Output order stays set index then lane index; every zero/signed-zero
lane and source-order ordinal `4*set + lane` is retained. The existing core variable influence
storage, all-instance palette validation and application extraction require no production changes.

Preflight checks `4*N*S` cells against the aggregate influence limit across selected primitive
definitions before dependency IO, with individual and cumulative U32 CSR endpoint/offset checks.
Shared instances do not multiply stored rows. The primitive work group additionally admits the
complete InfluenceCells count and conservative WorkUnits covering set lookups, decoding, writing,
per-row heap sorting and adjacent uniqueness checks. Allocations remain fallible/metered, and
reader loops, scratch comparisons and validation scans poll cancellation. Existing provenance
collectors already visit every attribute accessor and therefore retain every higher-set resource,
including complete sparse base/indices/values chains.

Source checks apply to single and multiple sets alike. Weights must be finite and non-negative;
positive-weight joints must be unique across a Vertex's complete row. Zero-weight duplicate joints
remain legal and every lane is still checked against each selected instance's palette by core.
All-integer weight sets require the exact raw sum `sum(U8*257) + sum(U16) == 65535`. Raw integers
are checked before conversion; canonical F32 conversion uses the backend's existing public Cast
implementation, without a duplicate normalization algorithm or changed rounding. Any F32 set
selects the source's floating sum recommendation instead of an integer-only subset check. Values
are preserved, and sums outside `2e-7 * positive_lane_count` produce one bounded summary warning
per primitive, after leaving the decode work group. No automatic repair, normalization, compaction
or loss authority is introduced. ADR 0031 now explicitly separates core raw-row semantics from
format-specific requirements and diagnoses preserved F32 discrepancies in the glTF profile.

The semantic source is the
[pinned official Khronos glTF specification](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/Specification.adoc#skinned-mesh-primitive-attributes),
including indexed semantic continuity, nonzero joint uniqueness, exact integer sums and per-skin
palette indices. The source was already retained locally by the sparse slice and read directly.
No Assimp implementation or fixture was copied.

## Review and verification

Three native Codex agents provided read-only architecture review, importer regression tests and
an independent application consumer test file. The main thread owns production integration and
serial Cargo execution. The first format-package run passed 271/271 tests
(`137b67c8-b41e-4ba6-94f3-1583a67b0d25`); the independent consumer's two-set and sparse third-set
cases passed 2/2 (`48111282-d5cc-4b8f-ab58-f9e15f65d782`). Final source identity, integrated results
and any subsequent corrections will be recorded after implementation freeze.

## Qualification boundary

Rust 1.95.0, locked/offline dependencies, one Cargo build job and the existing target directory
are used on x86_64-pc-windows-msvc. Fixtures are independently authored and bounded. This slice
does not change the canonical snapshot schema or runtime dependencies, implement an animation
evaluator, promote format maturity, or establish new runtime-platform/fuzz/performance results.
