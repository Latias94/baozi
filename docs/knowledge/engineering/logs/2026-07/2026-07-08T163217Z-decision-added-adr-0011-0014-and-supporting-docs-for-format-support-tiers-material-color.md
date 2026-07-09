---
type: "Memory Event"
title: "Decision: Added ADR 0011-0014 and supporting docs for format support tiers, material/color"
description: "Added ADR 0011-0014 and supporting docs for format support tiers, material/color-space policy, postprocess semantics, parser security, forma"
timestamp: 2026-07-08T16:32:17Z
event_kind: "Decision"
---
# Event

Added ADR 0011-0014 and supporting docs for format support tiers, material/color-space policy, postprocess semantics, parser security, format template, support matrix, coordinate conventions, and parser threat model.

# Impact

Baozi now has proposed semantic gates for declaring format support, material and color-space mapping,
post-process behavior, and parser security. The supporting format template, support matrix, coordinate
conventions, and threat model should guide the first workspace scaffold and parser implementations.

# Citations

- [ADR 0011](../../../../adr/0011-format-support-tiers-and-compatibility-charter.md)
- [ADR 0012](../../../../adr/0012-material-texture-image-and-color-space-policy.md)
- [ADR 0013](../../../../adr/0013-post-process-pipeline-semantics-presets-and-mutation-model.md)
- [ADR 0014](../../../../adr/0014-parser-security-unsafe-ffi-and-panic-boundary-policy.md)
- [Format template](../../../../formats/_template.md)
- [Support matrix](../../../../formats/support-matrix.md)
- [Coordinate conventions](../../../../model/coordinate-and-render-conventions.md)
- [Parser threat model](../../../../security/parser-threat-model.md)
