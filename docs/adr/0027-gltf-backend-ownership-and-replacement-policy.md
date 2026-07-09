---
status: accepted
date: 2026-07-09
authors:
  - Codex
decision_type: architecture
related:
  - docs/adr/0004-parser-backend-and-format-coverage-policy.md
  - docs/adr/0018-parser-tooling-and-format-owned-parser-policy.md
  - docs/adr/0019-parser-diagnostic-streaming-and-generated-code-contract.md
  - docs/adr/0022-per-import-resource-ledger-and-budget-accounting.md
  - docs/adr/0026-validation-snapshot-and-fuzz-gates.md
  - docs/formats/gltf.md
---

# ADR 0027: glTF Backend Ownership and Replacement Policy

## Context

glTF is strategically important because it is the most common modern interchange format for real-time
rendering pipelines. Baozi needs useful glTF support early, but the importer must still preserve the
project's clean-room, Rust-owned IR, `ImportContext`, resource-limit, diagnostics, and WASM
boundaries.

The current `baozi-format-gltf` crate uses `gltf-rs` 1.4.x as a private bootstrap dependency. That
lets Baozi ship static mesh coverage earlier, but it is not the long-term ownership boundary. glTF
ecosystem crates can lag the specification, extension ecosystem, or Baozi's diagnostic/resource
contracts, so the dependency must remain replaceable.

## Decision

Baozi will keep `gltf-rs` hidden inside `baozi-format-gltf` for the experimental static mesh stage,
while treating the glTF importer as Baozi-owned.

Rules:

- no `gltf-rs` type may appear in public Baozi API, `ImportReport`, diagnostics, options, snapshots,
  or IR fields
- all IO, external buffers, GLB payloads, buffer data URIs, and future image buffers must enter
  through `ImportContext`
- all output must be converted into Baozi `Scene` IR before leaving the format crate
- every supported glTF feature must have happy-path fixtures, malformed fixtures, snapshot coverage,
  resource-limit tests, and a fuzz path before it can move beyond experimental maturity
- if `gltf-rs` blocks required behavior, Baozi may fork it under `repo-ref/` for study and then
  vendor or replace only the necessary parsing layer inside `baozi-format-gltf`
- a fork or replacement is allowed to break `baozi-format-gltf` internals, but it must not break the
  facade API or core IR without a separate API/IR ADR

Replacement triggers:

| Trigger | Required action |
| --- | --- |
| Cannot enforce Baozi resource limits before large allocation | Replace or wrap the affected loader path |
| Cannot expose enough source context for diagnostics | Add Baozi-owned validation/diagnostic layer or replace parser slice |
| Dependency panic aborts under sanitizer fuzz before Baozi can return an error | Add Baozi-owned preflight, fork the affected validation path, or replace the parser slice before enabling sanitizer-run promotion |
| Unsupported required glTF 2.0 core feature blocks Beta maturity | Fork or self-write that slice |
| Dependency becomes incompatible with Baozi MSRV, WASM, license, or unsafe policy | Replace dependency or gate the affected backend |
| Extension support requires architecture not available upstream | Implement Baozi-owned extension parsing in the format crate |

## Alternatives Considered

### Option A: Expose `gltf-rs` as Baozi's glTF API

Pros:

- fastest to implement
- lets advanced users reach upstream details immediately

Cons:

- freezes Baozi to upstream type design
- leaks dependency release cadence into Baozi API
- bypasses Baozi-owned diagnostics, options, and resource contracts

Decision: rejected.

### Option B: Self-write the full glTF parser immediately

Pros:

- maximum ownership and clean-room control
- easiest to align every parse path with `ImportContext`
- avoids dependency maintenance risk

Cons:

- delays useful glTF support
- large upfront conformance burden
- distracts from IR, validation, postprocess, and fuzz infrastructure that all formats need

Decision: deferred until a concrete trigger justifies the cost.

### Option C: Private bootstrap dependency with planned replacement boundary

Pros:

- ships an early, useful glTF slice
- keeps public API and IR independent from backend choice
- lets tests, fuzzing, and resource ledger behavior define the replacement contract
- allows fork/self-written parser work to happen incrementally

Cons:

- requires discipline to prevent upstream types or assumptions from leaking
- some malformed-input behavior may need wrapper tests before a full replacement
- future contributors must understand the bootstrap boundary

Decision: chosen.

## Quality Gates

Before promoting glTF support beyond `Experimental`, Baozi must have:

- GLB BIN payload import fixture
- external `.gltf` buffer fixture through `ImportContext`
- base64 buffer data URI fixture with resource-ledger assertions
- skin fixture covering node-level mesh binding, joint streams, joint nodes, skeleton root, inverse
  bind matrices, and malformed skin validation
- snapshot coverage for hierarchy, mesh streams, material, texture reference, scene space, and
  diagnostics
- malformed fixtures for missing buffers, short buffers, invalid JSON/GLB, invalid accessor roots,
  invalid accessor component/type contracts, missing POSITION, unsupported primitive modes, and
  malformed skin data
- resource ledger assertions for primary bytes, sidecar bytes, total bytes, opened assets, generated
  vertices, generated faces, and data URI bytes
- fuzz target that mutates primary glTF/GLB bytes, external buffer bytes, buffer data URIs, and skin
  data
- successful Linux sanitizer fuzz smoke for `gltf_import`; `cargo fuzz check` alone is only an
  experimental-target compile gate while the bootstrap backend can abort on malformed validation
- documented unsupported features with diagnostics or fatal errors, not silent drops

## Consequences

Positive:

- Baozi can make glTF useful now without committing to `gltf-rs` forever.
- A future fork or self-written parser has a concrete compatibility contract.
- Public users keep the same owned `Scene` and `ImportReport` shape across backend changes.

Negative:

- Some early behavior is constrained by the bootstrap dependency.
- Baozi must maintain wrapper tests around upstream behavior until it owns more parsing code.
- Contributors need to add conformance and malformed fixtures with each glTF feature.

## Current Backend Risk Exception

As of 2026-07-09, `gltf_import` is excluded from mandatory CI sanitizer runs and kept as an
experimental `cargo fuzz check` target. A fuzz smoke found that malformed inputs can reach
`gltf-rs`/`gltf-json` validation paths that panic or abort under libFuzzer before Baozi's
`catch_unwind` boundary produces a normal parse error. This exception is a safety truthfulness
measure, not a parser success signal.

Re-enable the sanitizer run only after Baozi prevalidates the affected accessor, buffer, and layout
paths or replaces/forks the backend slice so malformed glTF inputs cannot abort the fuzz process.
The active backend notes live in
[`docs/research/gltf-rs-backend-notes.md`](../research/gltf-rs-backend-notes.md).
