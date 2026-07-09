---
status: accepted
date: 2026-07-09
authors:
  - Codex
decision_type: architecture
related:
  - docs/adr/0006-public-api-versioning-and-crate-stability-policy.md
  - docs/adr/0021-extension-registry-and-format-descriptor-governance.md
---

# ADR 0024: Public API Hardening and Stability Tiers

## Context

Baozi is expected to iterate quickly and may make breaking changes while the importer foundation is
being built. That does not mean every public field or trait should accidentally become a de facto
promise. Parser libraries become expensive to refactor once downstream users construct internal
types directly.

## Decision

Baozi separates API surfaces into tiers:

- Facade tier: `baozi` re-exports core scene types, facade import helpers, report accessors, IO
  traits, and post-process entry points.
- Extension tier: `baozi-import` exposes `FormatImporter` and `ImporterRegistry` for format crates
  and deliberate third-party importer experiments.
- Internal/test tier: `baozi-test-support` and unpublished shell crates are not compatibility
  promises.

Rules:

- Report and descriptor types use constructors/accessors where future fields are expected.
- `Importer::register` returns `Result<()>`; duplicate format IDs are not panic paths.
- Facade does not re-export `FormatImporter`, `ImportContext`, `ImporterRegistry`, or detection
  internals.
- Empty feature flags are avoided; a public feature must have observable dependency, API, or
  behavior impact.
- Shell format crates are marked `publish = false` until they import fixtures with validation,
  snapshots, malformed tests, and fuzz coverage.

## Consequences

Positive:

- Future descriptor/report fields can be added without forcing downstream struct literal churn.
- Third-party importer work is possible but clearly experimental.
- The facade remains smaller and harder to accidentally freeze.

Negative:

- Advanced users must depend on `baozi-import` for custom format work.
- Some construction is less direct than public-field structs.
