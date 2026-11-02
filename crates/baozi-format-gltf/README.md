# baozi-format-gltf

Selected glTF 2.0 / GLB importer backend for source builds, enabled through the
[baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md)'s `format-gltf` feature or `all-formats`.
It is outside `default-formats`. Its private Baozi document owns metered typed tables and
borrows admitted source ranges; custom registration uses the experimental importer API.

The [exact selected profile](https://github.com/Latias94/baozi/blob/main/docs/formats/gltf.md) governs support:

- Select one declared source scene through frozen `GltfImportOptions` and
  `GltfSceneSelection::SceneIndex`, or use `DocumentDefault` for a declared default,
  sole scene or zero-scene parentless-node projection. Multiple scenes without a default
  require an explicit index. Report evidence records the request and outcome, and generated
  root provenance binds both. Resources outside the selected closure are omitted;
  mixed-target animation filtering requires explicit semantic loss authorization.
- Preserve all seven point, line and triangle primitive modes, supported vertex streams including
  complete paired `JOINTS_n`/`WEIGHTS_n`, hierarchy and node transforms, skins, F32 VEC3
  POSITION/NORMAL/TANGENT morph deltas, node TRS and morph-weight animation,
  perspective/orthographic cameras, selected shared `KHR_lights_punctual` definitions and
  placements, and PBR metallic-roughness materials.
  Animation interpolation covers Step, Linear, and CubicSpline.
- Preserve every admitted influence lane in set/lane source order, including zero weights,
  with bounded variable rows and complete source provenance. Validate paired contiguous sets,
  nonzero-joint uniqueness and exact all-integer weight sums; preserve F32 sum discrepancies
  with a summary diagnostic instead of silently normalizing.
- Decode dense and sparse accessors throughout the admitted profile, with an optional base
  or implicit zero values, normalized conversion, bounded layout/index validation and complete
  base/indices/values buffer provenance.
- Preserve encoded external, data-URI, and buffer-view image payloads without
  pixel decoding. Selected dependencies use the origin-qualified session resolver;
  external URIs use the restricted relative-path profile and embedded data URLs
  require base64. URI schemes for remote fetching are unsupported.
- Distinguish authored material assignments from the shared generated PBR default used when a
  selected primitive omits its binding. Effective material reads require no source-format guessing.
- Preserve selected `extras` as exact `application/json` byte values owned by source identities,
  including JSON null, scalars, and nested sites. No source JSON is interpreted as scene behavior.
- Support typed `KHR_materials_unlit` and independently authored `KHR_materials_emissive_strength`
  at material objects and `KHR_texture_transform` at documented
  texture bindings. Unlit selects base color/vertex color and preserves alpha/sidedness; the original
  PBR fallback factors, bindings and complete resources remain retained. Reject unsupported selected
  extension payloads and unsupported required extensions. Unknown optional declarations without
  retained payloads produce a declaration-only diagnostic, with no semantic-loss claim.
- Lower selected punctual-light photometry through the checked canonical light type before
  dependency IO. Node scale affects placement, explicit unit conversion scales range, and shared
  camera/light definitions remain distinct from node attachments. No pixels are rendered here.
- Import retains source topology; the explicit `baozi-postprocess::list_vertices` view expands
  strips/fans/loops for list consumers without mutating the Scene. Resource-limit
  coverage is partial, canonical dependency identities remain session-opaque, and
  full glTF conformance is not claimed.

Declaration evidence is `Unverified`, with no assigned Baozi maturity. See the
[support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md),
[API tiers](https://github.com/Latias94/baozi/blob/main/docs/adr/0006-public-api-versioning-and-crate-stability-policy.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi-format-gltf/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi-format-gltf/LICENSE-APACHE), at your option.
