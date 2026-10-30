# baozi-format-gltf

Selected glTF 2.0 / GLB importer backend for source builds, enabled through the
[baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md)'s `format-gltf` feature or `all-formats`.
It is outside `default-formats`. The maintained decoder based on `gltf-rs` remains
private and replaceable; custom registration uses the experimental importer API.

The [exact selected profile](https://github.com/Latias94/baozi/blob/main/docs/formats/gltf.md) governs support:

- Select the document-default scene, or its sole scene when no default is declared.
  Multiple scenes without a default are rejected. Resources outside the selected
  closure are omitted; mixed-target animation filtering requires explicit semantic
  loss authorization.
- Import points, lines, and triangles, supported vertex streams including
  `JOINTS_0`/`WEIGHTS_0`, hierarchy and node transforms, skins, dense F32 VEC3
  POSITION/NORMAL/TANGENT morph deltas, node TRS and morph-weight animation,
  perspective/orthographic cameras, and PBR metallic-roughness materials.
  Animation interpolation covers Step, Linear, and CubicSpline.
- Preserve encoded external, data-URI, and buffer-view image payloads without
  pixel decoding. Selected dependencies use the origin-qualified session resolver;
  external URIs use the restricted relative-path profile and embedded data URLs
  require base64. URI schemes for remote fetching are unsupported.
- Support only the typed `KHR_texture_transform` slice at documented texture
  bindings. Reject selected extras, unsupported selected extension payloads,
  unsupported required extensions, and sparse morph accessors. Unsupported optional extension
  declarations without selected payloads may retain the core fallback with a
  diagnostic.
- Triangle strips/fans and line strips/loops are not expanded. Resource-limit
  coverage is partial, canonical dependency identities remain session-opaque, and
  full glTF conformance is not claimed.

Declaration evidence is `Unverified`, with no assigned Baozi maturity. See the
[support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md),
[API tiers](https://github.com/Latias94/baozi/blob/main/docs/adr/0006-public-api-versioning-and-crate-stability-policy.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi-format-gltf/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi-format-gltf/LICENSE-APACHE), at your option.
