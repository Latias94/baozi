# glTF Format Support

## Summary

- Format: glTF 2.0 / GLB
- Crate: `baozi-format-gltf`
- Maturity: Experimental
- Default feature: not enabled by `default-formats`
- Parser backend: `gltf-rs` 1.4.x as an internal bootstrap dependency
- Supported extensions: `.gltf`, `.glb`
- Supported media types: `model/gltf+json`, `model/gltf-binary`
- Encoding: JSON plus binary buffers, or GLB container
- Sidecar policy: external buffers through `ImportContext`; GLB BIN payloads supported

## Current Status

`baozi-format-gltf` imports a static mesh MVP into Baozi's owned IR. It supports common `.gltf`
files with external binary buffers and `.glb` files with BIN payloads. External buffers are loaded
only through `ImportContext`, so resource limits, path resolution, diagnostics, and strict mode stay
under Baozi control.

The dependency on `gltf-rs` is intentionally hidden inside this crate. Baozi does not expose
`gltf-rs` types in public API, so the backend can later be forked or replaced by a Baozi-owned parser
without changing facade users. The ownership and replacement boundary is defined in
[`ADR 0027`](../adr/0027-gltf-backend-ownership-and-replacement-policy.md).

The crate remains `publish = false` until the ADR 0027 quality gates and a broader conformance corpus
are in place.

## Supported MVP

- Static primitives using points, lines, or triangles.
- Positions, normals, tangents, texture coordinates, colors, indices, `JOINTS_0`, and `WEIGHTS_0`
  streams when present.
- Node hierarchy, node transforms, mesh binding, and camera binding.
- Perspective and orthographic camera projection data.
- PBR metallic-roughness material factors.
- Base color, metallic-roughness, normal, occlusion, and emissive texture URI references.
- Y-up, right-handed, meters scene space metadata.
- Resource ledger accounting for primary assets, external buffers, GLB BIN payloads, and diagnostics.
- Quality gates for GLB import, snapshots, malformed external buffers, facade ledger stats, and a
  glTF fuzz target.

## Known Non-Support

- Buffer data URIs are explicitly unsupported for now.
- Embedded image buffer views and texture data URIs are diagnosed and skipped.
- Triangle strips, triangle fans, line strips, and line loops are not expanded yet.
- Skins, morph targets, and animation channels are diagnosed but not imported into final IR yet.
- The current backend is useful for bootstrapping, not the long-term parser ownership boundary.
