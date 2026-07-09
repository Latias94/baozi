---
type: Work Progress
title: glTF quality gates and backend ownership policy
timestamp: 2026-07-09T12:28:58Z
tags:
  - baozi
  - gltf
  - fuzz
  - adr
status: completed
---

# Summary

Baozi's glTF MVP now has explicit quality gates around GLB import, snapshot visibility, malformed
external buffers, facade resource-ledger stats, and fuzz entry coverage.

# Changes

- Added a generated in-memory GLB fixture for `baozi-format-gltf` tests.
- Added glTF snapshot assertions covering hierarchy, mesh streams, material, texture URI, scene
  space, and diagnostics.
- Added malformed glTF tests for missing external buffer, short external buffer, and unsupported
  primitive mode.
- Added a facade test proving `ImportReport` exposes glTF primary bytes, sidecar bytes, total bytes,
  opened assets, generated vertices, and generated faces.
- Added `gltf_import` cargo-fuzz target and a committed external-buffer corpus seed.
- Added ADR 0027 to keep `gltf-rs` as a private bootstrap backend with explicit fork/replace
  triggers.
- Updated CI and scheduled fuzz workflows to include `gltf_import`; CI package smoke is limited to
  `baozi-core` until internal dependencies are published in order.

# Citations

- [ADR 0027](../../../adr/0027-gltf-backend-ownership-and-replacement-policy.md)
- [glTF format docs](../../../formats/gltf.md)
- [glTF tests](../../../../crates/baozi-format-gltf/tests/static_mesh.rs)
- [glTF facade ledger test](../../../../crates/baozi/tests/gltf_facade.rs)
- [glTF fuzz target](../../../../fuzz/fuzz_targets/gltf_import.rs)
