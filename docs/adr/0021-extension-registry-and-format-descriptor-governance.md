---
status: accepted
date: 2026-07-09
authors:
  - Codex
decision_type: architecture
related:
  - docs/adr/0006-public-api-versioning-and-crate-stability-policy.md
  - docs/adr/0007-workspace-crate-graph-feature-flags-msrv-and-ci-gates.md
  - docs/adr/0011-format-support-tiers-and-compatibility-charter.md
---

# ADR 0021: Extension Registry and Format Descriptor Governance

## Context

`FormatImporter` is public from `baozi-import`, but the stable facade no longer re-exports it.
Third-party importers can still implement it deliberately by depending on `baozi-import`, but the
main `baozi` crate should not accidentally freeze this trait as a casual facade API. The registry
and `FormatInfo` shape must be intentional now, or every new format will hard-code support claims
differently.

## Decision

Baozi supports importer registration as an experimental extension point. `FormatImporter` and
`ImporterRegistry` remain available from `baozi-import`, while the facade only exposes `Importer`
registration and report accessors. Extension stability is carried by docs and versioning policy, not
by pretending the first trait shape is final.

Registry rules:

- Built-in importers are registered by facade features.
- Third-party importers register through `Importer::register` or `ImporterRegistry::register`; both
  return `Result<()>`.
- Detection conflicts are resolved by `ReadConfidence`; equal top confidence remains an ambiguity error.
- Format IDs must be stable lowercase identifiers and are rejected when duplicated in the same
  registry.

Format descriptor direction:

- `FormatInfo` remains the code-level descriptor and is constructed through methods rather than
  public fields.
- It includes media types, binary/text/container kind, sidecar policy, support docs link, maturity,
  capabilities, and notes. Security profile and fuzz/oracle evidence can be added without changing
  external struct literals.
- `docs/formats/support-matrix.md` is a human summary and must be checked against `FormatInfo` until
  it can be generated.

## Consequences

Positive:

- Baozi keeps plugin-like extensibility without committing to a C ABI or dynamic plugin system.
- Support claims have a single code-side anchor.
- Registry conflict behavior remains deterministic.

Negative:

- The public trait is a real pre-1.0 compatibility surface.
- Descriptor fields will need additive migration and tests.
