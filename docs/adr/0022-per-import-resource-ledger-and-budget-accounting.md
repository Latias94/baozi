---
status: accepted
date: 2026-07-09
authors:
  - Codex
decision_type: architecture
related:
  - docs/adr/0010-asset-io-virtual-filesystem-uri-archive-and-path-security.md
  - docs/adr/0014-parser-security-unsafe-ffi-and-panic-boundary-policy.md
  - docs/security/parser-threat-model.md
---

# ADR 0022: Per-Import Resource Ledger and Budget Accounting

## Context

`ResourceLimits` defines caps, but complex formats need cumulative accounting. glTF external
buffers, embedded data URIs, images, archives, decompression, and nested sidecars can each stay under
a per-file limit while exceeding the caller's total import budget.

## Decision

Baozi will introduce a per-import resource ledger before adding archive-heavy or buffer-heavy
formats. The ledger records consumption against the active `ImportContext` and should be included in
future `ImportReport` stats.

Budgets to track:

- primary asset bytes
- sidecar asset bytes
- total opened sidecars
- sidecar/include depth
- archive entries opened
- compressed bytes and decompressed bytes
- data URI decoded bytes
- embedded image bytes
- generated vertices, faces, meshes, materials, textures, animations
- diagnostics emitted and diagnostics dropped by cap

Ledger accounting must happen before large allocation or decompression wherever possible.

## Policy

- Per-resource limits stay as fast local checks.
- The ledger enforces aggregate limits across the import.
- Denied or over-budget optional resources produce diagnostics when geometry can still load safely.
- Over-budget primary or required structural resources are fatal errors.
- Tests for new sidecar/archive/data URI features must assert both per-resource and aggregate limits.

## Consequences

Positive:

- Security behavior scales beyond simple single-file parsers.
- Import reports can explain why optional resources were skipped.
- Fuzzing gets a clearer oracle for resource exhaustion paths.

Negative:

- `ImportContext` gains mutable accounting responsibilities.
- Some existing limit tests will need migration to check ledger stats.
