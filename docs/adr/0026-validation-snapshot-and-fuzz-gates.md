---
status: accepted
date: 2026-07-09
authors:
  - Codex
decision_type: testing
related:
  - docs/adr/0005-testing-fuzzing-and-differential-oracle-strategy.md
  - docs/adr/0017-serialization-snapshot-and-cache-boundary.md
  - docs/testing/snapshot-and-fixture-policy.md
---

# ADR 0026: Validation, Snapshot, and Fuzz Gates

## Context

Parser projects often look healthy while hidden IR fields are unvalidated, invisible in snapshots,
or unreachable by fuzz targets. Baozi is adding broad IR surface before all formats are implemented,
so every new field needs a test discipline.

## Decision

Every new public IR field must answer these questions in the same change:

- What validator rule protects it from invalid references, non-finite values, or length mismatch?
- How does `SceneSnapshot` make it reviewable when parser output changes?
- Which focused malformed fixture or unit test proves the bad case?
- If the field is parser-reachable, which fuzz target can mutate it?

Current gates:

- Core validator tests cover geometry, materials, textures, skins, custom attributes, cameras,
  lights, and animation channels.
- `SceneSnapshot` prints counts and compact previews for omitted-heavy channels such as tangents,
  custom attributes, skins, cameras, lights, and animations.
- Fuzz smoke targets cover STL import, OBJ import with MTL sidecars, and OBJ import plus
  postprocess.
- Support-matrix rows are checked against `FormatInfo` capabilities.

## Consequences

Positive:

- IR growth is visible in code review.
- Parser fuzzing follows new reachable surfaces.
- Documentation and code descriptors are less likely to drift.

Negative:

- Adding a field costs more up front.
- Snapshot text must remain curated to avoid noisy reviews.
