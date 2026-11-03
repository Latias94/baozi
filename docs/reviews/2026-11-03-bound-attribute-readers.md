# Bound Attribute Readers and Public Consumer Adoption

This experiment follows the retained
[observed-work sequence](2026-11-03-observed-work-sequence.md) in the
[Assimp convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
It moves selected-field discovery outside application tuple loops and repairs borrowing at
the existing core reader. It changes no normal algorithm, geometric data or timing boundary.

## Ownership and Single Sampling Implementation

Core `AttributeRef` inspection getters and `element` borrow `&self`. Element dispatch matches
the borrowed payload and uses the existing dense/sparse helpers, whose bodies are unchanged.
Returned typed slices retain the Scene lifetime after a temporary descriptor is dropped.
Explicit descriptor/data inspection still returns its existing Copy value. This receiver change
is intentionally breaking for explicit UFCS/function-pointer callers; the one repository UFCS
caller is migrated without a compatibility alias.

`ListVertices::attribute(semantic, domain)` binds a `ListAttribute` to the exact owning layout
and descriptor once. `attributes()` enumerates bound fields lazily in retained order. A bound
stream stores only that layout and original attribute; its constructor remains private.
`element(&self, draw_index)` uses the existing checked topology/source-coordinate mapping and
the same core element reader. `elements(self)` owns the borrowed view for an exact-size,
double-ended, cloneable iterator. Metadata and source identity remain available separately.
The tuple convenience reader uses the same source-domain mapping and core sampler.

An initial isolated proposal duplicated dense/sparse sampling in postprocess to avoid a
by-value core receiver. Main rejected it and replaced the receiver at its owner. That sampler
is absent from production; no alternate payload representation, index cache, lookup table,
unsafe rebinding or second topology implementation was introduced.

Bindings and sampling allocate nothing. Vertex/Corner/Face domains, all seven supported list
assemblies, strip parity, loop closure, repeated/degenerate indices, missing versus present-empty
fields, sparse explicit/default slice identity and F32/F64/U32/I32 bits remain intact. These
readers start no work ledger; the application still owns copying and interruption of its loop.

## Real Applications and Measurement Scope

The actual asset application's `buffers.rs` binds Position, selected Normal/Tangent/Color and
every selected UV stream once per draw. Existing UV selection storage now holds its bindings;
no second UV-binding Vec is allocated. Records are larger borrowed views, so this is not a
claim of smaller metadata memory. Source-Vertex/bitwise indexing, all selected UV sets,
influences, morph representatives, material/tangent declarations, numeric conversions and
detached output ownership remain unchanged. The replaced tuple-level exact-key lookups are
removed from those selected-field loops.

The comparison adapter adopts the same ordinary public API **only inside `visit_draw`**.
Domain ambiguity, missing-field policy, topology admission, per-tuple Position/Normal/UV
conversion order and visitor errors remain. Binding performs no scalar preflight and does
not reject a missing Position on an empty layout before any tuple is read. There is no zip
truncation. Native setup/cleanup, selected detached F64 output, placements, corpus, precision,
normal policy, work allowances, clocks and collector are unchanged. New workers must identify
this adoption rather than relabel R1/R2/R3 evidence.

## Independent Review and Correctness Checkpoint

Independent library and adoption reviews reconstruct both patches against their pinned bases
and establish no open P1/P2 at the inspected snapshots. A subsequent nine-file review finds
only formatting changes and the reviewed UFCS adaptation in actual production. The two
consumers and newly added test file are included in the post-format source freeze.

Tests use real public import and central validation. They cover dense/sparse values and
original pointer/bit identity, 16-lane fields, every list topology, source domains/seams,
empty/missing distinctions, iterator direction/clones, temporary wrapper lifetimes and
zero-allocation reads. Consumer probes retain literal error ordering, visitor interruption,
non-F32-representable F64 values and negative-zero output after Scene destruction.

| Check | Result at this checkpoint |
| --- | --- |
| Core/postprocess all-feature nextest | 961/961; six existing skips; `e0b1e0f3-2ab1-4646-8f08-791703a91c05` |
| Affected compiler contracts | Core six negative; postprocess one positive plus three negative; passed |
| Strict affected all-target/all-feature Clippy | Passed with `-D warnings` |
| Strict standalone asset-consumer Clippy | Passed with `-D warnings` |
| Package and standalone-consumer formatting | Passed |
| Asset-consumer nextest | 124/124; one existing skip; `0037a4f1-8b6e-419c-ae53-c99b02e9a3a1` |
| Ordinary performance | Not yet measured for this candidate |

Commands use Rust 1.95.0, locked/offline dependencies, one build job and the shared target.
The first reader check's wrapper records only changed existing crate files; the supplemental
`target/review/convergence/r4-source-freeze.json` records all nine actual files during that
check, including the new source and both consumers. Independent review reconciles that freeze
with the reviewed copies. No source changed after formatting. Following checks record both
source roots and new source before/after execution. This is an evidence-scope correction,
not a second run relabelled as the first or a claim that its initial wrapper covered more files.

Exact commands/logs are `target/review/convergence/r4-*.json` and matching logs. Isolated v1/v2
copies, adoption scope, patches and the independent reviews remain under
`.git/baozi-review/performance-convergence/`. Source-level removal of repeated lookup and
descriptor transport does not establish optimized ABI traffic, native speed, memory savings
or Assimp convergence. Ordinary immutable workers decide the performance result separately.
