# glTF Development Candidate

## Summary

- Format: glTF 2.0 / GLB
- Crate: `baozi-format-gltf`
- Admission: blocked; internal development candidate only
- Declaration evidence: Unverified
- Baozi maturity: None (no current `BaoziVerified` evidence record)
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
0027 baseline admission gates pass. A separate managed evidence inventory must verify an exact
admitted subject before Baozi can assign it any maturity.

The `gltf_import` development fuzz target currently compiles but is not accepted as parser evidence.
Malformed input can trigger a `gltf-rs` validation panic in cargo-fuzz builds where panic aborts
before Baozi's `catch_unwind` fallback can return an error. This known baseline failure is why the
format is disabled at the facade rather than assigned a maturity. The blocker and replacement
criteria are tracked in [glTF Backend Notes](../research/gltf-rs-backend-notes.md).

## Implemented Development Slice

- Static primitives using points, lines, or triangles.
- Positions, normals, tangents, texture coordinates, colors, indices, `JOINTS_0`, and `WEIGHTS_0`
  streams when present.
- Node hierarchy, node transforms, mesh binding, node-level skin binding, and camera binding.
- Deterministic document-default scene selection: a sole scene is selected when no default is
  declared, while multiple declared scenes without a default are rejected instead of silently
  selecting scene zero.
- Skins with joint node references, optional skeleton root, and optional inverse bind matrices.
- Perspective and orthographic camera projection data.
- PBR metallic-roughness material factors.
- Base color, metallic-roughness, normal, occlusion, and emissive texture URI references.
- Y-up, right-handed, meters scene space metadata.
- Node translation, rotation, and scale animation with source clip/channel identity, duration, time
  domain, interpolation, retained channel order, and non-contiguous source indexes preserved through
  canonical storage. Whole clips outside the selected scene are scope omissions; mixed-target clips
  require explicit `SourceSemanticLoss` authority before outside channels may be filtered.
- A bounded development `SourceScopeRecord` covering declared scenes, selected/omitted nodes,
  meshes/primitives, materials, textures/images/samplers, skins, accessors/buffer views/buffers,
  animations, and animation channels. The selected core resource closure is compiled before
  dependency IO; omitted external buffers/images are not authorized, and selected data-URI or
  buffer-view images are materialized as encoded payloads. This remains development evidence, not
  public admission evidence.
- Development resource-limit coverage for primary assets, decoded JSON text surfaces, external
  buffers, base64 buffer data URIs, GLB BIN payloads, diagnostics, and cooperative cancellation.
- Development gates for GLB import, snapshots, malformed external buffers/data URIs, skin validation,
  text preflight, bounded format failures, and a check-only glTF fuzz target. These are not facade
  admission evidence.

## Text Preflight, Resource Accounting, and Cancellation

This development preflight is intentionally outside U4.6's line-record migration. glTF is not in
the first-party format inventory, and JSON is a document grammar rather than a line-record grammar.
Materializing the complete JSON surface as `ConformedTextInput` would add a decoded copy and line
index without enforcing duplicate-key, nesting, token, object, array, string, number, GLB chunk, or
padding limits. U6 therefore owns replacing the bootstrap scan with one borrowed, bounded JSON/GLB
admission path that preserves container-relative raw ranges. The temporary UTF-8 preflight, minimal
GLB JSON-surface locator, and heuristic text detector are development evidence only and must be
deleted or reduced to adapters over that admitted path before facade registration.

- Plain `.gltf` counts the complete JSON document against `max_decoded_utf8_bytes`. GLB counts only
  its JSON chunk; the 12-byte container header, chunk header, and BIN chunks are excluded from the
  decoded-text surface.
- JSON UTF-8 validation is incremental and polls for every at-most-4096-byte window. An incomplete
  code point at a window edge carries its at-most-three-byte prefix into the next validation window.
  The JSON slice remains borrowed, so decoded-text admission creates no fictitious live allocation;
  the opaque `gltf-rs` document graph is covered separately by its conservative backend allowance.
- A leading UTF-8 BOM is forbidden on the JSON surface. Invalid UTF-8 and BOM evidence use raw
  container-relative offsets: zero-based within a plain `.gltf`, and based at the GLB JSON payload
  offset within `.glb`.
- Base64 data-URI whitespace/padding validation is a single forward scan with a control poll for every
  at-most-4096 encoded bytes. Decoding uses independent 4096-byte chunks, a multiple of four, and
  polls before each chunk. Offset-bearing decode errors add the chunk start to the library-local
  offset before producing format evidence.

These contracts are exercised by `crates/baozi-format-gltf/tests/text_preflight.rs` and
`crates/baozi-format-gltf/tests/limits.rs`. They are development safety evidence only and do not
promote glTF into the public facade inventory.

## Bounded Format Failure Evidence

`GltfFailure` is a crate-private synchronous evidence producer. Its human reason uses a sanitized
70-byte inline buffer rather than a heap-backed `String`. The selected `ParseScope` copies the stable
code, detail keys, string values, and scalar details into Baozi-owned `FormatFailureEvidence`, then
destroys the producer before returning the sealed envelope. A transient `std::str::Utf8Error` is used
only to compute the container-relative offset and bounded reason; neither it nor a glTF wrapper is
retained.

The public standard-error chain is exactly `FormatFailureEnvelope -> FormatFailureEvidence -> None`.
There is no backend downcast target, backend vtable, or plugin-owned destructor after packing. Inline
storage, sanitization, bounded packing, teardown, and source-chain termination are covered by the unit
and import-contract tests.

## Admission Blockers And Missing Semantics

- Embedded image buffer views and image data URIs are materialized as encoded image payloads without
  pixel decoding. Unsupported morph/default-weight semantics, required extensions, and the remaining
  backend/admission gaps are still fail-closed or explicitly deferred.
- Triangle strips, triangle fans, line strips, and line loops are not expanded yet.
- Morph-weight animation is unsupported retained-domain semantics. It is rejected by default and may
  be omitted only under explicit `SourceSemanticLoss` authority with typed loss evidence and a stable
  channel diagnostic until the morph animation slice is admitted.
- The current `SourceScopeRecord` still lacks ADR 0039's typed selection outcome, camera and
  animation-sampler kinds, extension rows, and complete dependency provenance. The bootstrap
  backend also materializes a GLB BIN chunk before the closure can filter it. These are admission
  blockers even though the selected core resource closure is now tested and bounded.
- The current backend is useful for bootstrapping, not the long-term parser ownership boundary.
- `gltf_import` is a non-admitted development target until known backend aborts are removed or
  process-isolated and the complete baseline admission matrix passes.
