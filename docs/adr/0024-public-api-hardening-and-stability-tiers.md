---
status: superseded
date: 2026-07-09
updated: 2026-07-10
authors:
  - Codex
decision_type: architecture
superseded_by:
  - docs/adr/0006-public-api-versioning-and-crate-stability-policy.md
related:
  - docs/adr/0021-extension-registry-and-format-descriptor-governance.md
---

# ADR 0024: Public API Hardening and Stability Tiers

## Status

Superseded by
[ADR 0006: Public API, Versioning, and Crate Stability Policy](0006-public-api-versioning-and-crate-stability-policy.md).

This document is retained as a historical record of why Baozi introduced explicit API tiers. It
contains no active compatibility rules. ADR 0006 is the sole authority for the canonical v1 reset,
stability tiers, crate publication, feature compatibility, and release versioning.

The status, index, link, and reciprocal supersession shape of this historical record are maintained
by direct architecture review. Reviewers must keep this record historical-only; no structural check
can determine whether later prose is normative.

## Historical Context

The initial importer foundation exposed public fields and extension traits before Baozi had enough
format and validation evidence to know which contracts deserved stability. This ADR identified the
need to keep the common facade smaller than deliberate extension and workspace-internal surfaces.

That concern overlapped ADR 0006 rather than defining an independent decision. Maintaining both as
accepted authorities would allow their tier definitions and pre-release rules to drift.

## Supersession Decision

Move the complete public compatibility policy into ADR 0006 and leave this file as a non-normative
tombstone with machine-readable `superseded_by` metadata.

## Alternatives Considered

### Option A: Keep ADRs 0006 and 0024 accepted

Pros:

- preserves the original documents without editing their roles
- allows each ADR to focus on a smaller subset of API concerns

Cons:

- creates two authorities for the same tiers and facade boundary
- makes conflicts in release and extension policy impossible to resolve mechanically

Decision: rejected.

### Option B: Make ADR 0024 the sole authority

Pros:

- starts from the newer document
- requires fewer changes to this file's original status

Cons:

- discards ADR 0006's broader versioning, publication, feature, and release scope
- breaks existing references that already identify ADR 0006 as the compatibility policy

Decision: rejected.

### Option C: Consolidate into ADR 0006

Pros:

- establishes one complete compatibility authority
- preserves this ADR's history without duplicating normative text
- supports mechanical supersession and link checks

Cons:

- readers following an old link must make one additional hop

Decision: chosen.

## Consequences

Positive:

- public compatibility questions have one answer
- lifecycle tooling can resolve supersession without interpreting prose

Negative:

- this file no longer provides a standalone summary of active API tiers

## Risks and Mitigations

| Risk | Severity | Likelihood | Mitigation |
| --- | --- | --- | --- |
| New normative text is added here later | Medium | Low | Treat this file as historical-only in review; manual supersession review cannot classify prose as normative. |
| An old link is mistaken for current authority | Low | Medium | Keep the supersession notice first and link directly to ADR 0006 |

## Acceptance Criteria

- the ADR index marks 0024 as `Superseded` and resolves its `superseded_by` target
- compatibility policy searches point to ADR 0006, not to duplicated rules in this file
- accepted ADRs that discuss API compatibility link to ADR 0006
