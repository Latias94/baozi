---
type: "Work Progress"
title: "Canonical architecture U2 authority contracts complete"
description: "U2 canonical authority, ADR governance, Scene IR contracts, and glTF quarantine are complete and review-clean."
timestamp: 2026-07-10T12:40:41Z
record_id: "2cc663e9dac544c4ac0100eacdea427a"
status: "complete"
producer_id: "codex-root"
run_id: "019f4919-ef38-7b32-a0b7-476f42d35dd1"
source_session: "019f4919-ef38-7b32-a0b7-476f42d35dd1"
related_plan: "docs/plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md"
git_branch: "refactor/baozi-canonical-architecture"
---

# Summary

U2 is complete on `refactor/baozi-canonical-architecture`. The repository now has one explicit architecture authority, executable ADR governance, and canonical contracts for the breaking implementation work in U3-U9.

# Details

- Thirty-eight ADRs now have explicit status, authority edges, implementation owners, requirements, and evidence. ADR 0006 is the sole public compatibility authority and fixes the pre-release contract at canonical v1 without compatibility shims.
- `CONTEXT.md` maps requirements R1-R21 to owners and evidence. The selected architecture fixes the operation leaf, Scene identity and ownership, validation boundary, mesh/primitive/instance model, attribute domains, animation precision, image payloads, provenance, registry snapshots, option codecs, parse scope, and native filesystem admission.
- The canonical Scene IR documentation and U4 contract freeze generation-qualified IDs, central `SceneDraft -> ValidatedScene` validation, Scene-owned mesh instances, and domain-specific remap endpoints.
- The ADR checker enforces canonical frontmatter, authority acyclicity, the exact index, local links and fragments, tracers, evidence, path casing, and reparse-point containment. Forty-four adversarial tests cover known silent-bypass classes.
- glTF is quarantined as an unpublished development harness until U6 admission. It is absent from the facade registry and `all-formats`; PLY is included in `all-formats`, and CI exercises workspace all-features.
- Independent architecture, documentation, implementation, and adversarial reviews found no remaining reproducible P0/P1 issue after follow-up fixes.

Deferred by design: U3 implements the operation leaf and import/runtime boundaries; U4 implements the canonical Scene IR; U5-U8 migrate postprocess and formats; U9 completes release evidence and cleanup.

# Next Action

Commit U2 as the architecture authority baseline, mark U3 active, then introduce `baozi-operation` and cut errors, IO, registry/options, sessions, and `ParseScope` over to the selected contracts while deleting replaced APIs.

# Citations

- [Canonical architecture plan](../../../../plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md)
- [U2 verification](../../verification/2026-07/2026-07-10T124045Z-canonical-architecture-u2-verification-6b124e2bbe514eb8a66867bb87e3b00a.md)
