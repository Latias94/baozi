---
type: "Work Progress"
title: "glTF resource skin and fuzz hardening"
description: "Work Progress for glTF resource skin and fuzz hardening."
timestamp: 2026-07-09T14:10:26Z
tags: ["gltf", "fuzz", "skin", "resource-ledger", "ci"]
related_plan: "docs/plans/2026-07-09-007-feat-gltf-resource-and-skin-plan.md"
---

# Summary

Completed the glTF resource-ledger, accessor hardening, fuzz protocol, and Skin MVP implementation
from the July 9 plan. The local `main` branch contains the relevant commits and is ahead of
`origin/main` pending push.

# Details

- Added pre-reader glTF JSON accessor contract validation and panic boundaries around `gltf-rs`
  reader calls so malformed inputs return `BaoziError` instead of unwinding through fuzz.
- Routed base64 buffer data URIs through `ImportContext` data URI byte accounting, including total
  asset byte limits.
- Moved skin binding semantics to node-level `MeshBinding { mesh, skin }` and imported glTF skins
  with joint node references, optional skeleton root, optional inverse bind matrices, and
  `JOINTS_0`/`WEIGHTS_0` streams.
- Updated `gltf_import` fuzz input handling: GLB magic inputs are treated as whole GLB bytes, while
  non-GLB inputs keep the legacy `.gltf` plus NUL-delimited `buffer.bin` protocol.
- Added data URI and skinned data URI fuzz corpus seeds.
- Updated glTF format docs, support matrix, and ADR 0027 quality gates to match implemented support.

# Next Action

Push `main` after final local verification. The CI run that originally failed should be rechecked
for the Linux fuzz smoke result after push.

# Citations

- Plan: [2026-07-09 glTF resource and skin plan](../../../plans/2026-07-09-007-feat-gltf-resource-and-skin-plan.md)
- Commits: `7c5f9e6`, `becf2bd`, `5ba8c9d`
- CI failure link: <https://github.com/Latias94/baozi/actions/runs/29019454481/job/86122242072>
