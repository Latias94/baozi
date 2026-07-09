---
type: "Decision"
title: "Reviewed animation, diagnostics, feature-gate, and binding ADR suggestions"
description: "Mapped external ADR suggestions 12-15 onto Baozi's existing ADR set. No new ADR number was needed; ADR 0014 and ADR 0015 were clarified."
timestamp: 2026-07-09T05:10:15Z
event_kind: "Decision"
---
# Decision

External suggestions about animation/skinning, non-fatal diagnostics, Cargo features, and
cross-language bindings were reviewed against Baozi's current architecture docs.

No new ADR was created:

- animation and skinning are covered by ADR 0015 and `docs/model/scene-ir.md`
- non-fatal diagnostics are covered by ADR 0002, ADR 0016, ADR 0019, and `ImportReport`
- Cargo feature gates are covered by ADR 0007, ADR 0006, and ADR 0004
- FFI safety is covered by ADR 0014

Two clarifications were added:

- ADR 0015 now explicitly rejects making fixed four-influence skinning or evaluated animation tracks
  canonical raw IR.
- ADR 0014 now explicitly says foreign-language bindings should use dedicated wrapper crates such as
  `baozi-c-api`/`baozi-ffi`; `baozi-core` should remain Rust-native and must not promise `#[repr(C)]`
  layout for its scene structs.

# Context

The accepted direction is source-preserving, owned, Rust-native IR. Runtime-friendly views are useful,
but they belong in helper APIs, post-process outputs, or binding crates rather than the raw import
contract.

# Consequences

Future glTF/FBX/Collada work should preserve animation keyframes, interpolation declarations, source
timing metadata, skins, and morph targets without implementing playback. Future C/C#/Python/Unity
integration work needs a dedicated binding ADR before introducing stable ABI crates.

# Citations

- [ADR 0015](../../../../adr/0015-mesh-topology-vertex-attributes-skinning-and-animation-semantics.md)
- [ADR 0014](../../../../adr/0014-parser-security-unsafe-ffi-and-panic-boundary-policy.md)
- [Scene IR](../../../../model/scene-ir.md)
