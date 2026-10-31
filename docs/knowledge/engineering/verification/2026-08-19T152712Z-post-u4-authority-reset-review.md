---
type: "Architecture Review Record"
title: "Post-U4 architecture authority reset approved"
description: "Approval and structural review record for ADRs 0045-0047 and the related authority migration."
timestamp: 2026-08-19T15:27:12Z
tags: ["baozi", "architecture", "adr", "u4"]
status: "passed"
git_branch: "refactor/baozi-canonical-architecture"
approved_by: "repository owner"
verified_by: "Codex architecture review and repository ADR/conformance checks"
related_plan: "docs/plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md"
---

# Review Scope

The repository owner authorized continued execution of the canonical architecture plan and the
atomic Post-U4 authority reset on 2026-08-19. The reviewed decision scope is:

- accept ADR 0045 as the sole canonical Scene ownership, construction, validation, publication, and
  generation authority, superseding ADRs 0003 and 0025;
- accept ADR 0046 as the sole import-session lifecycle and terminal-outcome authority, superseding
  ADR 0020;
- accept ADR 0047 as the sole operation ledger-evidence and resource-accounting authority,
  superseding ADR 0022;
- redirect active ADR dependencies, the ownership registry, requirement traceability, and
  conformance inventory to those successors;
- resolve ADR 0009's stale ownership questions against the canonical public owner; and
- deepen ADR 0041 behind one session-owned resource-graph and manifest module before U4.5 begins.

# Review Result

Passed. The replacement ADRs preserve the durable guarantees of their predecessors while removing
private typestate names, exhaustive implementation sequences, and migration-era evidence from
architectural authority. ADR 0043 remains Proposed. ADR 0042 receives no semantic amendment.

The review confirmed:

- each replacement owns one coherent architectural concern and contains genuine alternatives,
  diagrams, risks, measurable success criteria, and acceptance criteria;
- supersession is reciprocal and every normative dependency targets an Accepted ADR;
- the ADR dependency graph is acyclic;
- the decision-ownership registry and stable requirement traceability identify the replacements;
- active implementation and conformance references no longer treat the superseded ADRs as current
  authority; and
- implementation conformance remains a separate axis and is not inferred from Accepted status.

# Verification

- Parsed all numbered ADR frontmatter and verified 46 records, Accepted-only dependencies, an
  acyclic dependency graph, and reciprocal supersession.
- Scanned current Accepted and Proposed ADR text before amendment history for stale normative
  references to ADRs 0003, 0020, and 0022.
- Ran `cargo +1.95.0 nextest run -j 1 --locked -p baozi-xtask --test conformance`: 11 tests
  passed.
- Ran `cargo +1.95.0 run -j 1 --locked -p baozi-xtask -- repository-conformance-check`: the
  inventory was structurally valid with 12 contracts, 12 gates, and zero declared conforming.
- Ran repository diff and Markdown path checks for the authority-reset change.

# Evidence

- [Canonical architecture implementation plan](../../../plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md)
- [ADR governance policy](../../../contributing/adr-governance.md)
- [ADR ownership registry](../../../adr/README.md)
- [Architecture conformance inventory](../../../conformance/contracts.toml)
