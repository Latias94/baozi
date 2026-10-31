# Selected glTF 2.0 / GLB Profile

## Summary

- Format: glTF 2.0 / GLB
- Crate: `baozi-format-gltf`
- Admission: source builds, for the exact profile below
- Declaration evidence: Unverified
- Baozi maturity: None (no current `BaoziVerified` evidence record)
- Public facade feature: `format-gltf` (also included in `all-formats`)
- Parser backend: maintained `baozi-gltf` / `baozi-gltf-json` packages based on `gltf-rs` 1.4.1
- Extensions: `.gltf`, `.glb`
- Media types: `model/gltf+json`, `model/gltf-binary`
- Encoding: JSON plus binary buffers, base64 buffer data URIs, or GLB container
- Sidecar policy: selected external buffers and images through the origin-qualified `ParseScope`
  dependency resolver; base64 buffer data URIs and GLB BIN payloads remain embedded in the
  enclosing primary resource

## Current Status

`baozi-format-gltf` imports the selected-scene PBR, skin, morph, and TRS animation profile below.
Enable `baozi/format-gltf` to register it through `Importer::new()`, or register `GltfImporter`
in a custom registry. The curated default format set remains OBJ/MTL and STL.

The dependency on `gltf-rs` is intentionally hidden inside this crate. Baozi does not expose
`gltf-rs` types in public API, so the backend can later be forked or replaced by a Baozi-owned parser
without changing facade users. The ownership and replacement boundary is defined in
[`ADR 0027`](../adr/0027-gltf-backend-ownership-and-replacement-policy.md).

The crate remains `publish = false` until package/release checks are complete. Source admission
does not assign Beta/Stable maturity or a full glTF conformance claim.

The `gltf_import` fuzz target executed the six known malformed JSON/GLB reproducers and
a bounded 256-run Linux ASan campaign on the immutable `2dc2fc8` source archive. The maintained
validator corrections reject the demonstrated out-of-range POSITION reference and missing camera
projection payload without a panic fallback. That integration also passed the complete workspace
suite, normalized/source-mapping regressions, and independent debug/release typed consumers on
ordinary Windows main stacks. Exact execution scope is recorded in
the [delivery progress](../reviews/2026-10-29-delivery-progress.md). The short campaign establishes
baseline malformed-input safety for that source state; it is not sustained fuzz evidence.

## Supported Profile

- Static primitives using points, lines, or triangles.
- Positions, normals, tangents, texture coordinates, colors, indices, `JOINTS_0`, and `WEIGHTS_0`
  streams when present.
- Node hierarchy, node transforms, mesh binding, node-level skin binding, and camera binding.
- Deterministic document-default scene selection: a sole scene is selected when no default is
  declared, while multiple declared scenes without a default are rejected instead of silently
  selecting scene zero.
- Skins with joint node references, optional skeleton root, and optional inverse bind matrices.
- Dense F32 VEC3 POSITION/NORMAL/TANGENT morph deltas, with equal target width across morph-bearing
  primitives, zero deltas for empty targets or primitives without targets, zero mesh defaults when
  weights are absent, and complete node weight overrides. Delta streams require the corresponding
  base semantic. Cardinality, finite values, ordered POSITION bounds, and morph-cell admission are
  checked before canonical publication; unsupported morph semantics are rejected explicitly.
- Perspective and orthographic camera projection data.
- PBR metallic-roughness material factors.
- Base color, metallic-roughness, normal, occlusion, and emissive texture URI references.
- The typed `KHR_texture_transform` slice for selected base color, metallic-roughness, normal,
  occlusion, and emissive texture bindings, including the extension `texCoord` override. Its
  finite transform and canonical UV-set bounds are checked before dependency I/O, and the values
  participate in the canonical material-binding snapshot.
- Y-up, right-handed, meters scene space metadata.
- Node translation, rotation, and scale animation with source clip/channel identity, duration, time
  domain, interpolation, retained channel order, and non-contiguous source indexes preserved through
  canonical storage. Whole clips outside the selected scene are scope omissions; mixed-target clips
  require explicit `SourceSemanticLoss` authority before outside channels may be filtered.
- Morph-weight animation targets the mesh instance attached to the authored source node, even when
  several nodes share a logical mesh. Step, Linear, and CubicSpline retain key-major weight vectors;
  cubic input/value/output parts each contain a complete target-width vector. Output is SCALAR F32
  or normalized I8/U8/I16/U16. Non-normalized integers, U32, invalid counts, non-finite weights,
  duplicate targets, and weights on a node without morph targets are rejected.
- A bounded development `SourceScopeRecord` covering declared scenes, selected/omitted nodes,
  meshes/primitives, materials, textures/images/samplers, skins, accessors/buffer views/buffers,
  animations, and animation channels. The selected core resource closure is compiled before
  dependency IO; omitted external buffers/images are not authorized, and selected data-URI or
  buffer-view images are materialized as encoded payloads.
- Selected external buffers and images are recorded in the canonical source profile after the
  primary document, with formal Buffer/Image locators and owner provenance for geometry,
  attributes, skins, animation clips/channels, and buffer-view images. External buffer snapshots
  are borrowed at their declared prefix instead of copied. The Scene digest therefore reflects
  only consumed canonical meaning, while the successful input manifest still hashes and measures
  the complete acquired dependency, including any unconsumed tail bytes.
- The canonical profile conservatively classifies these dependency identities as session-opaque.
  Registered resolver stability remains authoritative in the successful input manifest until the
  import/core boundary provides a sealed stable-evidence bridge; format code must not forge that
  evidence from URI strings.
- Development resource-limit coverage for primary assets, decoded JSON text surfaces, external
  buffers, base64 buffer data URIs, GLB BIN payloads, diagnostics, and cooperative cancellation.
- GLB admission retains borrowed JSON and BIN views over the primary snapshot. Only the JSON view is
  deserialized into a private `gltf::Document`; selected BIN-backed buffers are copied later through
  Baozi's metered buffer loader, and BIN size does not expand the opaque backend allowance.
- Global `extensionsUsed` and `extensionsRequired` declarations are checked after JSON-root decode
  and before backend document validation. Empty or duplicate names and a required name missing from
  `extensionsUsed` are invalid; unsupported required extensions return typed format failure, while
  unsupported optional extensions retain the core fallback and emit `gltf.extension_ignored`.
  Duplicate detection uses metered name storage and bounded `WorkUnits` sorting.
- Gates cover GLB import, snapshots, malformed external buffers/data URIs, skin validation,
  text preflight, bounded format failures, and the Linux sanitizer-run glTF fuzz target.

## Text Preflight, Resource Accounting, and Cancellation

This preflight uses document admission because JSON is a document grammar rather than a
line-record grammar.
Materializing the complete JSON surface as `ConformedTextInput` would add a decoded copy and line
index without enforcing duplicate-key, nesting, token, object, array, string, number, GLB chunk, or
padding limits. U6 therefore uses one borrowed, bounded JSON/GLB admission path that preserves
container-relative raw ranges and exposes the validated BIN chunk separately. The parser enters the
bootstrap backend through `gltf::json::Root` and `gltf::Document`, so the backend never receives the
GLB container or materializes its BIN payload.

- Plain `.gltf` counts the complete JSON document against `max_decoded_utf8_bytes`. GLB counts only
  its JSON chunk; the 12-byte container header, chunk header, and BIN chunks are excluded from the
  decoded-text surface.
- JSON UTF-8 validation is incremental and polls for every at-most-4096-byte window. An incomplete
  code point at a window edge carries its at-most-three-byte prefix into the next validation window.
  The JSON slice remains borrowed, so decoded-text admission creates no fictitious live allocation;
  the opaque `gltf-rs` document graph is covered separately by a conservative JSON-sized backend
  allowance. GLB BIN bytes remain part of the metered primary snapshot and selected-buffer copy, not
  that opaque allowance.
- A leading UTF-8 BOM is forbidden on the JSON surface. Invalid UTF-8 and BOM evidence use raw
  container-relative offsets: zero-based within a plain `.gltf`, and based at the GLB JSON payload
  offset within `.glb`.
- Base64 data-URI whitespace/padding validation is a single forward scan with a control poll for every
  at-most-4096 encoded bytes. Decoding uses independent 4096-byte chunks, a multiple of four, and
  polls before each chunk. Offset-bearing decode errors add the chunk start to the library-local
  offset before producing format evidence.

These contracts are exercised by `crates/baozi-format-gltf/tests/text_preflight.rs` and
`crates/baozi-format-gltf/tests/limits.rs`.

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

## Explicit Restrictions

- Embedded image buffer views and image data URIs are materialized as encoded image payloads without
  pixel decoding. Sparse morph accessors, typed extension payloads beyond
  `KHR_texture_transform`, and selected extras are rejected explicitly.
- Triangle strips, triangle fans, line strips, and line loops are not expanded yet.
- Selected `extras` and unsupported extension payloads are explicitly rejected with original raw byte
  ranges. A supported extension at an unsupported owner/site is rejected as well. Unselected entity
  payloads remain scope omissions. Declaring an unsupported optional extension without a selected
  payload retains the core fallback with `gltf.extension_ignored`.
- The current `SourceScopeRecord` includes ADR 0039's typed selection outcome, camera rows, and
  animation-sampler rows. Entity-level extension rows, typed extension-reference closure for
  reference-bearing schemas, and a sealed bridge for stable canonical dependency evidence remain
  outside this profile. Unsupported selected payloads are rejected. Selected dependency provenance and the complete successful
  input manifest are now compiled and tested, global extension declarations are gated, and GLB BIN
  data is isolated from the bootstrap backend. Canonical dependency identities remain
  session-opaque; the successful input manifest retains resolver stability evidence.
- The maintained backend remains private and replaceable. Canonical authoring reads the bounded
  normalized model after backend data and its allowance have been released.

## Version Metadata and Resource URIs

Version and URI rules follow the
[glTF asset and URI specification](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/Specification.adoc)
and the [asset schema](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/schema/asset.schema.json),
with the explicit restrictions listed below.

- The JSON document and `asset` must be objects. `asset.version` is a required string;
  `asset.minVersion` may be absent but must be a string when present, including in a GLB JSON chunk.
  Neither arrays accepted by a backend struct visitor nor explicit null version properties are
  admitted. Version components are ASCII decimal integers in `0..=999999999`, without leading
  zeros, separated by exactly one dot. Comparisons are numeric, and `minVersion > version` is an
  invalid document.
- The supported asset profile requires major version 2. Future 2.x minor versions remain compatible
  when no minimum is declared, or when the declared minimum is `2.0`. A different major version,
  including one paired with `minVersion: "2.0"`, or any other well-formed minimum is an unsupported
  profile, reported as `gltf.unsupported_feature`. Invalid syntax, types, or minimum ordering use
  `gltf.invalid_document`. Reasons identify `asset.version` or `asset.minVersion` as applicable.
  Schema/version scanning is charged to `WorkUnits` before backend decoding and dependency IO;
  its temporary storage is covered by the existing JSON-sized backend allowance.
- Selected external buffers and images share one relative IRI path decoder. Literal non-ASCII
  characters and their UTF-8 percent encodings identify the same resource. Escapes are decoded
  exactly once; `+` stays a literal plus sign, and `%252F` identifies literal `%2F` filename text.
  An escaped `#` is filename text rather than a fragment. Malformed escapes, invalid decoded UTF-8,
  control characters, and unescaped ASCII whitespace are `gltf.invalid_reference` failures.
- URI schemes other than the supported embedded `data:` profile, authorities, absolute paths,
  query/fragment parameters, and backslashes are unsupported. Percent-encoded separators, colons,
  and complete `.` or `..` path segments are also explicitly unsupported; decoding cannot silently
  introduce path structure. These failures use `gltf.unsupported_feature`, with a `buffer-uri` or
  `texture-uri` subject and the source buffer/image index. Filename restrictions enforced by
  `AssetPath` and the host resource policy continue to apply.
- Literal `.` and `..` path segments remain subject to the existing logical path normalization,
  root-escape checks, and dependency authorization. The resolver and input graph receive the
  once-decoded logical reference. Every dependency is still acquired through the primary origin
  capability and its declared role; decoding grants no resource authority. Primary input hashing
  continues to cover the original JSON bytes. Only selected resources enter this URI policy.
- Embedded data URLs retain the bounded base64-only profile. The case-insensitive `;base64` marker
  must be the final metadata item immediately before the comma; a media type merely named `base64`
  is not that marker. Non-base64 and percent-encoded data URL spellings are explicitly unsupported
  and retain the source buffer/image index. Data URL grammar is defined by
  [RFC 2397](https://www.rfc-editor.org/rfc/rfc2397#section-3).

The narrow regression cases are in `crates/baozi-format-gltf/tests/version_uri.rs`. Existing animation
and source-profile semantics are outside this bounded version/URI change.

Canonical provenance resource labels reversibly escape literal `%`, `#`, and `?` in normalized
filenames. This label is distinct from the dependency path and manifest identity: a file named
`base#.png` is opened under that exact filename, while its canonical logical label contains
`base%23.png`. Label allocation remains admitted for the lifetime of its authoring owner.
