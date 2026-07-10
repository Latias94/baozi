# glTF Development Candidate

## Summary

- Format: glTF 2.0 / GLB
- Crate: `baozi-format-gltf`
- Admission: blocked; internal development candidate only
- Public facade feature: none
- Parser backend: `gltf-rs` 1.4.x as an internal bootstrap dependency
- Candidate extensions: `.gltf`, `.glb`
- Candidate media types: `model/gltf+json`, `model/gltf-binary`
- Encoding: JSON plus binary buffers, base64 buffer data URIs, or GLB container
- Sidecar policy: external buffers through the selected `ParseScope` dependency resolver; base64
  buffer data URIs and GLB BIN payloads supported by the current development harness

## Current Status

`baozi-format-gltf` contains an internal mesh, material, camera, and skinning development slice, but
it is not a first-party inventory entry and is not registered by the public `baozi` facade. The
crate remains in the workspace so its preflight, normalized-model, and sanitizer work can proceed
without presenting the backend as admitted support.

The dependency on `gltf-rs` is intentionally hidden inside this crate. Baozi does not expose
`gltf-rs` types in public API, so the backend can later be forked or replaced by a Baozi-owned parser
without changing facade users. The ownership and replacement boundary is defined in
[`ADR 0027`](../adr/0027-gltf-backend-ownership-and-replacement-policy.md).

The crate remains `publish = false`. A public facade feature may be reintroduced only after the ADR
0027 baseline admission gates pass; broader non-safety maturity evidence then determines whether the
admitted format is Experimental, Beta, or Stable.

The `gltf_import` development fuzz target currently compiles but is not accepted as parser evidence.
Malformed input can trigger a `gltf-rs` validation panic in cargo-fuzz builds where panic aborts
before Baozi's `catch_unwind` fallback can return an error. This known baseline failure is why the
format is disabled at the facade rather than labeled Experimental. The blocker and replacement
criteria are tracked in [glTF Backend Notes](../research/gltf-rs-backend-notes.md).

## Implemented Development Slice

- Static primitives using points, lines, or triangles.
- Positions, normals, tangents, texture coordinates, colors, indices, `JOINTS_0`, and `WEIGHTS_0`
  streams when present.
- Node hierarchy, node transforms, mesh binding, node-level skin binding, and camera binding.
- Skins with joint node references, optional skeleton root, and optional inverse bind matrices.
- Perspective and orthographic camera projection data.
- PBR metallic-roughness material factors.
- Base color, metallic-roughness, normal, occlusion, and emissive texture URI references.
- Y-up, right-handed, meters scene space metadata.
- Development resource-limit coverage for primary assets, external buffers, base64 buffer data URIs,
  GLB BIN payloads, and diagnostics.
- Development gates for GLB import, snapshots, malformed external buffers/data URIs, skin validation,
  and a check-only glTF fuzz target. These are not facade admission evidence.

## Admission Blockers And Missing Semantics

- Embedded image buffer views and texture data URIs are diagnosed and skipped.
- Triangle strips, triangle fans, line strips, and line loops are not expanded yet.
- Morph targets and animation channels are diagnosed but not imported into final IR yet.
- The current backend is useful for bootstrapping, not the long-term parser ownership boundary.
- `gltf_import` is a non-admitted development target until known backend aborts are removed or
  process-isolated and the complete baseline admission matrix passes.
