---
type: "Memory Event"
title: "Decision: Added ADR 0007-0010 covering workspace crate graph and CI gates, Baozi-owned mat"
description: "Added ADR 0007-0010 covering workspace crate graph and CI gates, Baozi-owned math with mint interop, owned public Scene data, and root-scope"
timestamp: 2026-07-08T16:12:48Z
event_kind: "Decision"
---
# Event

Added ADR 0007-0010 covering workspace crate graph and CI gates, Baozi-owned math with mint interop, owned public Scene data, and root-scoped secure AssetIo.

# Impact

Baozi now has proposed decisions for the workspace scaffold, Rust version and CI policy, math
interop boundary, owned public scene memory model, and secure virtual asset IO. Implementation work
should use these ADRs before creating the workspace crates or the first parser crates.

# Citations

- [ADR 0007](../../../../adr/0007-workspace-crate-graph-feature-flags-msrv-and-ci-gates.md)
- [ADR 0008](../../../../adr/0008-math-coordinate-units-and-numeric-policy.md)
- [ADR 0009](../../../../adr/0009-data-ownership-zero-copy-lifetimes-and-memory-layout.md)
- [ADR 0010](../../../../adr/0010-asset-io-virtual-filesystem-uri-archive-and-path-security.md)
