---
type: "Work Registration"
title: "OBJ MTL importer"
description: "Active implementation workstream for the OBJ/MTL importer vertical slice."
timestamp: 2026-07-09T08:38:06Z
status: "active"
last_seen: 2026-07-09T08:38:06Z
producer_id: "codex-root"
related_plan: "docs/plans/2026-07-09-005-feat-obj-mtl-importer-plan.md"
git_branch: "main"
base_commit: "a522020"
---

# Scope

Implement the OBJ/MTL importer plan: replace the OBJ shell with a Baozi-owned parser, add minimal core material/texture affordances, support common static OBJ face geometry, load MTL sidecars through `AssetIo`, add diagnostics and limits, expand facade/WASM/fuzz/CI/docs coverage, and land on `main`.

# Current Claim

Goal-mode execution started from canonical plan `docs/plans/2026-07-09-005-feat-obj-mtl-importer-plan.md`.
The working tree also contains an untracked duplicate-looking plan `docs/plans/2026-07-09-005-feat-obj-mtl-importer-vertical-slice-plan.md`; do not treat it as canonical unless the user explicitly asks to adopt it.

# Latest Links

- [Plan](../../../plans/2026-07-09-005-feat-obj-mtl-importer-plan.md)
- [Planning findings](../subagents/2026-07-09T083806Z-obj-mtl-importer-planning-findings.md)
- [Start log](../logs/2026-07/2026-07-09T083806Z-progress-started-obj-mtl-importer-goal.md)

# Handoff

Next action is to execute U1 from the plan: core material/texture and snapshot support, starting with failing tests for `SceneBuilder::add_texture`, material metadata, texture slot validation, and snapshot visibility.

# Citations

- Plan: `docs/plans/2026-07-09-005-feat-obj-mtl-importer-plan.md`
- Base commit: `a522020`
- Goal thread: `019f4236-ffd3-76c0-9fb8-89d7d8607c3b`
