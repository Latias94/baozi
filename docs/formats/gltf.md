# Selected glTF 2.0 / GLB Profile

## Summary

- Format: glTF 2.0 / GLB
- Crate: `baozi-format-gltf`
- Admission: source builds, for the exact profile below
- Declaration evidence: Unverified
- Baozi maturity: None (no current `BaoziVerified` evidence record)
- Public facade feature: `format-gltf` (also included in `all-formats`)
- Parser: private Baozi selected-profile document with metered tables and borrowed source ranges
- Extensions: `.gltf`, `.glb`
- Media types: `model/gltf+json`, `model/gltf-binary`
- Encoding: JSON plus binary buffers, base64 buffer data URIs, or GLB container
- Sidecar policy: selected external buffers and images through the origin-qualified `ParseExecution`
  dependency resolver; base64 buffer data URIs and GLB BIN payloads remain embedded in the
  enclosing primary resource

## Current Status

`baozi-format-gltf` imports the selected-scene PBR/unlit, skin, morph, and TRS animation profile below.
Enable `baozi/format-gltf` to register it through `Importer::new()`, or register `GltfImporter`
in a custom registry. The curated default format set remains OBJ/MTL and STL.

The private document borrows admitted JSON/BIN and unescaped strings, owns metered typed tables,
and admits escaped-string storage before decoding. Accessor reads use checked immutable buffer
layouts. Document/index owners are destroyed after normalization and before canonical submission;
no owning generic JSON tree or opaque allocation multiplier participates in production decoding.
The ownership and replacement boundary is defined in
[`ADR 0027`](../adr/0027-gltf-backend-ownership-and-replacement-policy.md).

The crate remains `publish = false` until package/release checks are complete. Source admission
does not assign Beta/Stable maturity or a full glTF conformance claim.

The previous decoder's `gltf_import` fuzz target executed the six known malformed JSON/GLB reproducers and
a bounded 256-run Linux ASan campaign on the immutable `2dc2fc8` source archive. The maintained
validator corrections reject the demonstrated out-of-range POSITION reference and missing camera
projection payload without a panic fallback. That integration also passed the complete workspace
suite, normalized/source-mapping regressions, and independent debug/release typed consumers on
ordinary Windows main stacks. Exact execution scope is recorded in
the [delivery progress](../reviews/2026-10-29-delivery-progress.md). The short campaign establishes
baseline malformed-input safety for that source state; it does not qualify the replacement
document. Fresh target/sanitizer and archive-consumer evidence belongs to the
[maturation execution record](../reviews/2026-11-02-first-release-maturation.md).

## Supported Profile

- All seven authored primitive modes: POINTS, LINES, LINE_LOOP, LINE_STRIP, TRIANGLES,
  TRIANGLE_STRIP and TRIANGLE_FAN. Canonical topology, primitive order and absent versus present
  indices are preserved. Import performs no mode expansion or primitive discard. Consumers may
  explicitly use [`list_vertices`](../model/reading-scenes.md#application-list-buffers) for borrowed
  Points/LineList/TriangleList draw tuples.
- Positions, normals, tangents, texture coordinates, colors, indices, and complete paired
  `JOINTS_n` / `WEIGHTS_n` streams when present. Influence set indices start at zero without gaps;
  the existing attribute, cell, work and storage limits bound their count.
- Selected FLOAT base NORMAL and TANGENT.xyz must have an absolute squared-length error at
  most `1e-4`, evaluated from decoded F32 components in F64. This is Baozi's admission tolerance,
  not a normative glTF epsilon. Invalid directions return `InvalidAccessor`; admitted values
  retain their authored bits. Existing finite-value and TANGENT.w checks precede the length
  check. It runs in the existing sample traversal after sparse replacement. Quantized directions
  retain their separate decoded-magnitude policy, and morph directions are signed displacements
  rather than unit vectors. Import preserves a missing normal stream; format-faithful flat
  shading is a separate consumer policy.
- Dense and sparse accessors for every admitted accessor use, including geometry, indices,
  influences, morph deltas, animation inputs/outputs and inverse bind matrices. Sparse accessors
  accept an optional base bufferView or implicit zero source components and apply replacements
  before normalized integer conversion. Canonical materialization uses the existing bounded
  dense collectors; source sparsity does not reduce logical cardinality or allocation admission.
- Accessors with neither a buffer view nor a sparse descriptor read as implicit zeros without
  allocating a source zero buffer. Positive logical counts and semantic constraints still apply;
  zero normals, tangents or rotation keys do not become valid through this storage form. An
  authored `byteOffset` requires a view. Pure implicit-zero descriptors retain the specification's
  min/max exception; stored dense/sparse morph positions retain actual-data bounds validation.
- Known schema fields distinguish absence from authored `null`. Null is rejected where another
  type is required, including in structurally checked omitted objects; `extras: null` remains
  exact descriptive JSON. Unsigned integer fields accept mathematically integral JSON decimal
  and exponent spellings without an F64 intermediate. Fractional or negative nonzero values and
  values outside the U64/reference/host domains fail explicitly.
- Normalized I8/U8/I16/U16 reads use the nearest, ties-to-even F32 value of the exact rational,
  with signed minima clamped before division. Accessor and influence reads share this rule;
  source influence rows are not renormalized. This stronger Baozi rule can differ by one ULP
  from a reference implementation without establishing a Khronos conformance defect there.
- `KHR_mesh_quantization` admits additional base POSITION I8/U8/I16/U16, signed normalized
  NORMAL/TANGENT, and signed or unnormalized unsigned TEXCOORD encodings. POSITION may be
  normalized or unnormalized. Morph POSITION admits signed I8/I16 with either normalization;
  morph NORMAL/TANGENT admit signed normalized I8/I16. All existing FLOAT encodings remain.
  COLOR, JOINTS and WEIGHTS keep their core component profile. Both `extensionsUsed` and
  `extensionsRequired` must declare quantization, even for a recognized extra encoding in an
  omitted mesh. A used-only declaration is invalid even when all attributes happen to be FLOAT.
  Canonical values remain F32; source integer widths do not reduce logical admission or change
  node/skin/UV transform semantics. Quantized directions retain their decoded magnitudes.
  Selected UV/color morphs remain outside the admitted morph profile.
- Mesh bounds describe stored components, including normalized integer endpoints. Integer
  bounds must be integral and in the source component range before any F32 conversion. Optional
  bounds on supported streams, including JOINTS/WEIGHTS, are checked in their existing single
  materialization traversal after sparse replacement. POSITION bounds are required; pure implicit
  zero descriptors keep the exception above. No second full buffer pass or dequantization bake
  is introduced.
- Node hierarchy, node transforms, mesh binding, node-level skin binding, and camera binding.
- Every declared node observes matrix/TRS mutual exclusion, even for an authored identity matrix
  or omitted node. Matrix-authored transforms require an exact affine bottom row, nonzero spatial
  columns and the frozen orthogonality tolerance in ADR 0027. Reflections and nonzero subnormal
  scales retain source bits. Explicit zero TRS scales and inverse-bind matrices have their separate
  contracts; the node-matrix test does not tighten them.
- Frozen format-owned scene selection through `GltfImportOptions`: `DocumentDefault` uses a
  declared default, otherwise a sole scene or a validated parentless-node projection when no
  scenes are declared. Multiple declared scenes without a default require an explicit
  `GltfSceneSelection::SceneIndex`; no declaration-order fallback is applied.
- Skins with joint node references, optional skeleton root, and optional inverse bind matrices.
- VEC3 POSITION/NORMAL/TANGENT morph deltas in the FLOAT or quantization encodings above, with equal target width across morph-bearing
  primitives, zero deltas for empty targets or primitives without targets, zero mesh defaults when
  weights are absent, and complete node weight overrides. Delta streams require the corresponding
  base semantic. Cardinality, finite values, ordered POSITION bounds, and morph-cell admission are
  checked before canonical publication; unsupported morph semantics are rejected explicitly.
- Perspective and orthographic camera projection data.
- Typed `KHR_lights_punctual` directional, point and spot definitions, selected by node
  attachments. Shared definitions retain their source ordinals, names, exact extras and owner
  provenance; unreferenced definitions remain omitted. Light-only scenes need no geometry or
  sidecar IO. The existing checked core photometry and spatial conversion remain authoritative.
- Selected `extras` values, including document/asset, scene, buffer/view/accessor, animation sampler,
  and nested schema sites, retained as exact `application/json` bytes under source-object metadata
  owners. Source objects without a canonical entity keep their own identity and metadata.
- PBR metallic-roughness material factors.
- Typed `KHR_materials_emissive_strength`: raw emission RGB and independently authored strength
  survive reads, snapshots, conversion and owned application output. An absent extension retains
  `None`; an empty object retains explicit default strength one. Effective emission is derived in
  F64. Selected invalid, negative, overflowing or nonzero-underflowing strength and the forbidden
  Unlit combination reject before dependency IO; image bytes and binding strength are unchanged.
- Typed `KHR_materials_unlit` materials through the existing `ShadingModel::Unlit`, with base color,
  vertex colors, alpha coverage and double-sided state. Original PBR fallback factors, bindings and
  their resources remain retained independently of the active shading model.
- Typed `KHR_materials_clearcoat` and `KHR_materials_sheen` groups retain authored absence versus
  an explicit empty/default group. Their source unit-range factors are checked before narrowing;
  zero factors do not remove authored bindings or resources. Both extensions exclude Unlit;
  specular-glossiness remains outside this producer profile.
- Clearcoat intensity samples red as data, coat roughness green as data, and the independent
  coat normal RGB as data with its finite signed scale. Sheen color samples RGB as sRGB color;
  sheen roughness samples alpha as data. Each slot retains its own UV selection, texture transform,
  exact nested extras, texture/image/sampler identity and provenance. Binding order is the existing
  five core slots followed by coat, coat roughness, coat normal, sheen color and sheen roughness.
  A clearcoat normal binding requires authored NORMAL and TANGENT attributes or a base normal
  texture, as the extension specifies. Import does not silently generate the missing frame.
- Base color, metallic-roughness, normal, occlusion, and emissive texture URI references.
- The typed `KHR_texture_transform` slice for selected base color, metallic-roughness, normal,
  occlusion, emissive, clearcoat and sheen texture bindings, including the extension `texCoord` override. Its
  finite transform and canonical UV-set bounds are checked before dependency I/O, and the values
  participate in the canonical material-binding snapshot.
- Y-up, right-handed, +Z asset front, meters scene space metadata, counter-clockwise fronts, and
  upper-left UV origin.
- Each imported base `TANGENT` attribute owns `TangentConventions` with an unknown construction
  algorithm and `SignInW` handedness encoding. Read these facts through that attribute's
  `tangent_conventions()`, independently of its primitive's other streams and the Scene's spatial
  declarations. An absent base tangent has no convention declaration. Morph `TANGENT` deltas
  retain their separate VEC3 delta semantics; importing authored tangents does not claim MikkTSpace
  generation.
- Node translation, rotation, and scale animation with source clip/channel identity, duration, time
  domain, interpolation, retained channel order, and non-contiguous source indexes preserved through
  canonical storage. Whole clips outside the selected scene are scope omissions; mixed-target clips
  require explicit `SourceSemanticLoss` authority before outside channels may be filtered.
  Rotation values use Baozi's canonical `abs(norm2 - 1) <= 1e-4` policy, computed in F64 from
  retained F32 components. This differs from the Khronos validator's length-tolerance policy.
  The unchanged official SimpleSkin fixture passes the pinned validator but remains an explicit
  Baozi policy rejection; see the [source and boundary adjudication](../research/2026-11-02-quaternion-compatibility.md).
  Accepted source bits remain unchanged. Temporary application evaluation can normalize an
  effective interpolation sample under its documented policy; it does not rewrite stored keys.
- A core animation channel without `target.node` contributes no runtime channel, duration or
  channel-only resource closure. Its structure, present references and extension mechanisms remain
  checked; later retained channels preserve their original source ordinals. This core ignore rule
  neither grants semantic-loss authority nor implements `KHR_animation_pointer`.
- Morph-weight animation targets the mesh instance attached to the authored source node, even when
  several nodes share a logical mesh. Step, Linear, and CubicSpline retain key-major weight vectors;
  cubic input/value/output parts each contain a complete target-width vector. Output is SCALAR F32
  or normalized I8/U8/I16/U16. Non-normalized integers, U32, invalid counts, non-finite weights,
  duplicate targets, and weights on a node without morph targets are rejected.
- A bounded development `SourceScopeRecord` covering declared scenes, selected/omitted nodes,
  meshes/primitives, materials, textures/images/samplers, skins, accessors/buffer views/buffers,
  animations, animation channels, cameras and light definitions. The selected core resource closure is compiled before
  dependency IO; omitted external buffers/images are not authorized, and selected data-URI or
  buffer-view images are materialized as encoded payloads.
- Typed source-scope selection evidence retains the requested mode and actual source scene or
  parentless projection. Generated root provenance includes both the request and resolved
  projection in its versioned options digest, even when default and explicit requests resolve
  to the same scene.
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
- GLB admission retains borrowed JSON and BIN views over the primary snapshot. The JSON view feeds
  the metered structural index and typed document; selected BIN-backed buffers are copied later
  through the metered buffer loader. BIN size does not expand document storage.
- Global `extensionsUsed` and `extensionsRequired` declarations are checked after JSON-root decode
  and before selected closure interpretation. Empty or duplicate names and a required name missing from
  `extensionsUsed` are invalid; unsupported required extensions return typed format failure, while
  unsupported optional declarations without retained payloads emit
  `gltf.extension_uninterpreted_declaration` after selected-payload validation. Unsupported retained
  payloads fail explicitly; a loss opt-in alone does not define a fallback mapping.
  Duplicate detection uses metered name storage and bounded `WorkUnits` sorting.
- Gates cover GLB import, snapshots, malformed external buffers/data URIs, skin validation,
  text preflight, bounded format failures, and the Linux sanitizer-run glTF fuzz target.

### Selecting one declared scene

Enable the facade's `format-gltf` feature. The input type and locator belong to the glTF format;
they use the same option envelopes, preset/caller precedence and immutable import plans as
other format options:

```rust
use baozi::{
    AssetPath, GltfImportOptions, GltfSceneSelection, ImportConfigOverlay, ImportOperationControl,
    ImportReport, ImportRequest, ImportRequestLayer, Importer, OptionCompileLimits,
};

fn import_preview(document: &[u8]) -> Result<ImportReport, Box<dyn std::error::Error>> {
    let importer = Importer::new()?;
    let caller = ImportRequestLayer::new(ImportConfigOverlay::default())
        .try_with_format_option(
            GltfImportOptions::default()
                .with_scene_selection(GltfSceneSelection::SceneIndex(1))
                .to_envelope()?,
        )?;
    let plan = importer.plan(
        &ImportRequest::new().with_caller_layer(caller),
        OptionCompileLimits::default(),
    )?;
    Ok(plan.read_bytes(
        AssetPath::new("preview.gltf")?,
        document,
        ImportOperationControl::default(),
    )?)
}
```

Use `plan.read_asset` with an `AssetIo` namespace when the selected scene needs external buffers or
images. The plan retains its registry and compiled options and can be reused after the creating
importer is dropped. A caller's explicit `DocumentDefault` overrides an explicit preset selection.

`SceneIndex` is a zero-based source index, independent of names and compact canonical IDs. It
selects exactly one declared scene, including an empty scene. Explicit zero for a document with
no scenes is out of range; the parentless-node rule belongs only to `DocumentDefault`. Malformed
option transport fails during request compilation before primary IO. Out-of-range selection
fails with `gltf.invalid_reference`, subject `scene-selection` and the requested index, before
dependency IO or source-scope publication. An ambiguous default fails with
`gltf.scene_selection_required` and the same subject.

All scene roots, source indices and node-graph safety are still structurally validated before
selection. The chosen roots and descendants determine the resource closure; an omitted resource
cannot pull otherwise unselected nodes into the Scene. A selected skin's joints and skeleton
must already belong to the structural closure. Mixed-target animation retains the existing
`SourceSemanticLoss` requirement. Each projection gets one identity-TRS synthetic root with
selected children in source order, including an empty child list for an empty scene. Import
different projections separately when several source scenes are needed.

`source_scope_record().selection_request()` exposes `DocumentDefault` or
`ExplicitScene { scene_index }`; `selection_outcome()` exposes the resolved source identity.
Both participate in the `baozi.source-scope.v1\0` digest and survive optional-detail collapse or a
later import failure. The generated root operation uses `org.baozi.gltf:projection-root-v1`;
its source parents identify the actual document and scene, while its options digest distinguishes
the requested modes. Its 18-byte request/resolution frame retains big-endian u64 index lanes and
the default request's zero index lane.

**2026-11-02 U12 amendment:** These owned draft-v1 identities supersede the earlier source-scope v3
and projection v2 names. Source-scope request indices remain little-endian u64; the selection frame,
ordered observations including Light, actual source parents and selected resource closure are
unchanged. Persisted draft evidence binds the exact source closure and vector set under
[ADR 0026](../adr/0026-validation-snapshot-and-fuzz-gates.md#snapshot-schema-lifecycle). Earlier
revision-qualified evidence retains its original source binding; the import-options codec remains
its separate version-one family.

This follows [ADR 0039](../adr/0039-source-document-scene-selection-and-import-closure.md) and
the [pinned Khronos scenes contract](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/Specification.adoc#scenes).
The deterministic default-selection rules and parentless-node projection are Baozi policies.

### Unlit materials and optional extensions

A selected material's `extensions.KHR_materials_unlit` object selects `ShadingModel::Unlit` through
the existing material contract, normalized row and public `MaterialRef::shading_model()`. The name
must appear in `extensionsUsed`; an optional or required declaration has the same material meaning.
Malformed values and unsupported owner/sites fail before dependency IO. An empty object and admitted
descriptive `extras` are supported; empty nested `extensions` is allowed. Unknown direct properties
and nested extension payloads return `gltf.unsupported_feature` with their original byte ranges.
Khronos permits additional object properties with undefined interpretation; rejecting them is this
selected profile's restriction.

For glTF Unlit, evaluate the retained MR group's base color and `BaseColor` texture together with
vertex color. Alpha mode/cutoff and double-sided state still apply. Read `shading_model()` before
interpreting the complete `factors()` and `bindings()` values. Metallic, roughness, normal, occlusion
and emissive data remain source fallback truth and do not contribute to active unlit evaluation.
Their texture/image/sampler dependencies remain in the retained resource closure and are required
when authored. The importer does not render pixels, rewrite factors or silently discard fallback data.

Unsupported required extensions fail globally. Unsupported payloads on retained owners/sites fail
even with `SourceSemanticLoss` authority; accepting a fallback requires a schema-owned mapping and
appropriate loss evidence. Optional payloads on omitted owners remain scope omissions. An unknown
optional declaration without retained payload produces only the declaration diagnostic above, with
no semantic-loss record. This replaces the earlier misleading `gltf.extension_ignored` diagnostic.

See the [pinned Khronos unlit contract](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/extensions/2.0/Khronos/KHR_materials_unlit/README.md)
and [extension schema](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/extensions/2.0/Khronos/KHR_materials_unlit/schema/material.KHR_materials_unlit.schema.json).

### Punctual lights and spatial consumption

Root `extensions.KHR_lights_punctual.lights` is a definition inventory. Only attachments on
selected nodes select definitions; several nodes may share one light, and a node can also carry
a camera and geometry. Declaration/root-array safety is checked globally, while omitted light
payloads are not interpreted. Selected light preparation precedes dependency IO and uses one
checked `LightDefinition` construction per definition. Public node readers hide the internal
synthetic projection root, as for other glTF scenes.

Directional intensity is illuminance in lux; point/spot intensity is luminous intensity in candela.
glTF linear-sRGB color times intensity is converted through the existing canonical unit-luminance
tint/intensity contract in [ADR 0040](../adr/0040-camera-and-light-canonical-semantics.md).
Defaults are white, intensity one, no range and the standard local frame. Emission points along
local `-Z`. A spot requires a `spot` object; its defaults are inner zero and outer `pi/4`.
Source cones satisfy `0 <= inner < outer <= pi/2` before narrowing and must remain strictly
ordered in canonical F32. Invalid colors/intensities/ranges, authored nulls, overflow and nonzero
underflow reject explicitly. Unsupported selected properties/sites and nested behavior reject
under both loss policies; descriptive extras retain their exact source bytes.

Node scale changes placement through hierarchy composition without scaling range, photometry or
cone angles. Explicit Scene unit conversion scales range and camera clipping/extents, transports
intrinsic axes and keeps emission/cone bits. The detached asset example owns camera/light
definitions, dense node references and the full `SceneSpaceRef` after original Scene/importer/IO
destruction. Its actual renderer is a separate application and qualification boundary.

The [pinned Khronos extension](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/extensions/2.0/Khronos/KHR_lights_punctual/README.md)
defines the source semantics. The finite canonical cutoff policy remains ADR 0040; import does
not promise pixel identity with every other renderer.

### Primitive counts and index validation

Draw Corner count comes from the index accessor when present, otherwise from the vertex attributes.
glTF requires nonempty points, complete nonempty line/triangle lists, at least two Corners for line
strips/loops and at least three for triangle strips/fans. These format rules are checked before
dependency I/O. Canonical `face_count` and the corresponding import limit/work count assembled
points or segments as well as triangles; stored topology cells count only explicit indices.

Decoded indices must address the Position domain and cannot equal the maximum value of their
original U8/U16/U32 component type. The check runs before source-width information is discarded.
These rules follow the [Khronos mesh contract](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#meshes).
Repeated indices and degenerate elements remain present; import does not perform geometric repair.

### Sparse accessor validation and provenance

The shared accessor preflight checks the base and both sparse layouts before dependency I/O.
Short-component base vertex streams require explicit four-byte-multiple `byteStride`;
distinct vertex accessors sharing one view also require a stride. The stride must cover an
element and cannot exceed the view length. Each accessor's view-relative offset is four-byte
aligned; view and accessor offsets independently retain component alignment. These vertex
requirements do not pad tightly packed sparse replacement values or non-vertex streams.
Sparse count must be positive and no greater than the logical accessor count. Sparse index encoding
is U8, U16 or U32; both sparse views must omit `byteStride` and `target`, fit their declared buffers,
and cover all replacements. Relative offsets and view offsets must each align to the corresponding
component size. An accessor without a base view must omit `byteOffset`. Logical decoded size is
checked independently of stored sparse size, and existing semantic count limits still apply.
Base vertex relative offsets and explicit strides also require four-byte alignment. Animation
accessors use tightly packed base views without `byteStride`; integer indices and JOINTS must not
set `normalized`, while supported normalized UV/color/weight/animation values retain their conversion.

After selected buffers are loaded, a single admitted scan of each selected sparse index stream
rejects duplicates, decreasing order and indices outside the accessor count before decoding.
This scan borrows source bytes, charges DecodeWork by sparse index count, and polls between
at-most-4096-index chunks. It creates no auxiliary index table. Source-width primitive restart
checks and semantic value checks then operate on the final accessor values as for dense input.

The accessor's provenance includes all distinct contributing base, index and value buffer sources,
including the primary resource for embedded bytes. The selected closure and source-object links
retain the same full chain. Attribute, primitive, morph delta, skin and animation owners propagate
these sources through the existing public provenance readers. Buffer sources are sorted and
deduplicated with admitted capacity/work; even a completely replaced base remains a declared source.

These rules follow the [Khronos sparse accessor contract](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#sparse-accessors).
They do not add new component types, extension-provided element sources or matrix forms.

### Complete influence sets

Each selected primitive must provide both JOINTS_n and WEIGHTS_n for each set, with contiguous
set indices starting at zero. JOINTS uses U8/U16 VEC4 without normalized conversion; WEIGHTS uses
F32 or normalized U8/U16 VEC4. Every accessor has the primitive's complete Position count and
passes shared dense/sparse layout checks. All contributing accessor/buffer sources remain in
primitive provenance, including higher-set sparse dependencies.

The importer writes all `4 * set_count` lanes per Vertex to existing variable influence rows.
Order is ascending set index, then source lane; `source_order` is `4 * set + lane`. Zero and
signed-zero weights remain present. Import performs no compaction, truncation, joint merging,
sorting of the output rows or weight normalization. Every lane must address every selected skin's
palette for instances sharing that logical mesh, including lanes with zero weight.

Weights must be finite and non-negative, and positive-weight joint indices must be unique across
all sets of each Vertex. If all weight sets use normalized integer storage, the complete raw sum
must be exactly one: U8 components contribute `value * 257` and U16 components contribute `value`,
for a total of `65535`. If any set uses F32, preserve the exact converted values; rows whose F64
sum differs from one by more than `2e-7 * positive_lane_count` produce one summary warning per
primitive (`gltf.influence_weight_sum`). A warning does not normalize or authorize semantic loss.

Preflight checks the aggregate logical influence count `4 * vertex_count * set_count` across
selected primitive definitions against `max_influence_cells` before dependency IO; shared instances
do not multiply stored rows. Row widths, individual and aggregate CSR endpoints/offset counts must
fit canonical U32 domains. Materialization admits that complete InfluenceCells count and bounded
WorkUnits before storage reservation. Temporary storage consists of borrowed iterators per set and
one reusable row scratch for joint uniqueness; no full per-set joint/weight copies are retained.
The scratch sort is metered and cancellable, while canonical row order stays unchanged.

These rules follow the [Khronos skinned mesh attribute contract](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/Specification.adoc#skinned-mesh-primitive-attributes).

## Text Preflight, Resource Accounting, and Cancellation

This preflight uses document admission because JSON is a document grammar rather than a
line-record grammar.
Materializing the complete JSON surface as `ConformedTextInput` would add a decoded copy and line
index without enforcing duplicate-key, nesting, token, object, array, string, number, GLB chunk, or
padding limits. U6 therefore uses one borrowed, bounded JSON/GLB admission path that preserves
container-relative raw ranges and exposes the validated BIN chunk separately. The private typed
document borrows these surfaces and the source index; it never materializes an extra GLB BIN copy.

- Plain `.gltf` counts the complete JSON document against `max_decoded_utf8_bytes`. GLB counts only
  its JSON chunk; the 12-byte container header, chunk header, and BIN chunks are excluded from the
  decoded-text surface.
- JSON UTF-8 validation is incremental and polls for every at-most-4096-byte window. An incomplete
  code point at a window edge carries its at-most-three-byte prefix into the next validation window.
  The JSON slice remains borrowed, so decoded-text admission creates no fictitious live allocation;
  the structural index, typed tables, nested collections and decoded escaped strings have their
  actual metered owners. GLB BIN bytes remain part of the admitted primary snapshot and selected
  buffer copy, with no opaque expansion allowance.
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
70-byte inline buffer rather than a heap-backed `String`. The selected `ParseExecution` copies the stable
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
  pixel decoding. Only extension payloads at the owner/sites listed in Supported Profile above
  are admitted; other typed extension payloads are rejected explicitly.
- Unsupported selected extension payloads are explicitly rejected with original raw byte
  ranges. A supported extension at an unsupported owner/site is rejected as well. Unselected entity
  payloads remain scope omissions. Declaring an unsupported optional extension without a selected
  payload emits `gltf.extension_uninterpreted_declaration` without claiming a retained-payload loss
  or an arbitrary safe fallback.
- The current `SourceScopeRecord` includes ADR 0039's typed selection outcome, camera rows, and
  animation-sampler rows. Entity-level extension rows, typed extension-reference closure for
  reference-bearing schemas, and a sealed bridge for stable canonical dependency evidence remain
  outside this profile. Unsupported selected payloads are rejected. Selected dependency provenance and the complete successful
  input manifest are now compiled and tested, global extension declarations are gated, and GLB BIN
  data stays borrowed until selected buffer materialization. Canonical dependency identities remain
  session-opaque; the successful input manifest retains resolver stability evidence.
- The selected-profile document remains private. Canonical authoring reads the bounded normalized
  model after document tables, decoded strings and syntax indexes have been released.

## Descriptive Source Extras

Each selected `extras` property creates one source-object-owned metadata value. The key names its
schema site, and `MetadataValueRef::Bytes` exposes media type `application/json` and the original
JSON value bytes. The [upstream extras schema](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/schema/extras.schema.json)
allows any JSON type. There is no parse/reserialize step: null, booleans, strings, numbers, empty
containers, Unicode escapes, number spelling, and internal whitespace remain distinct. An absent
property produces no entry. The source object's versioned locator addresses the original object
range in primary bytes, including the GLB JSON chunk offset.

Direct object extras use `org.baozi.gltf:extras`; asset extras use
`org.baozi.gltf:asset-extras` on the document source identity. Other nested sites use these suffixes
under `org.baozi.gltf:`:

| Source owner | Nested-site key suffix |
| --- | --- |
| Material | `pbr-metallic-roughness-extras`, `unlit-extras`, `emissive-strength-extras`, `clearcoat-extras`, `sheen-extras` |
| Material, for each admitted binding prefix below | `<binding>-texture-extras`, `<binding>-texture-transform-extras` |
| Document punctual-light inventory | `punctual-lights-extras` |
| Node punctual-light attachment | `punctual-light-extras` |
| Light spot payload | `spot-extras` |
| Accessor | `sparse-extras`, `sparse-indices-extras`, `sparse-values-extras` |
| Animation channel | `target-extras` |
| Camera | `orthographic-extras`, `perspective-extras` |

Binding prefixes are `base-color`, `metallic-roughness`, `normal`, `occlusion`, `emissive`,
`clearcoat`, `clearcoat-roughness`, `clearcoat-normal`, `sheen-color` and `sheen-roughness`.
Extension-specific nested extras require their admitted parent owner/site.
Unknown extension objects remain subject to the explicit extension rejection policy; data nested
inside extras, including a property named `extensions`, is inert JSON. Extras do not create
canonical material, animation, or reference behavior.

Only the selected source closure is retained. Extras belonging to omitted objects are scope
omissions, and shared accessors/buffers or other source objects are not duplicated onto all their
canonical consumers. Source metadata survives canonical edits even when an associated derived
attribute is deleted. See [metadata readers](../model/reading-scenes.md#metadata-and-source-objects)
for lifetimes and ownership.

Before dependency IO, the selected extras lower-bound preflight checks entry count against
`max_records`, each key against `max_string_bytes`, and summed key, MIME, and payload bytes against
`max_canonical_string_bytes`. Core separately admits the complete metadata batch, including other
source properties, with those existing limits; there is no additional metadata quota or duplicate
output commitment. Descriptors and retained payload storage remain metered and centrally validated.

## Version Metadata and Resource URIs

Version and URI rules follow the
[glTF asset and URI specification](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/Specification.adoc)
and the [asset schema](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/schema/asset.schema.json),
with the explicit restrictions listed below.

- The JSON document and `asset` must be objects. `asset.version` is a required string;
  `asset.minVersion` may be absent but must be a string when present, including in a GLB JSON chunk.
  Arrays and explicit null version properties are rejected. Version components are ASCII decimal
  integers in `0..=999999999`, without leading
  zeros, separated by exactly one dot. Comparisons are numeric, and `minVersion > version` is an
  invalid document.
- The supported asset profile requires major version 2. Future 2.x minor versions remain compatible
  when no minimum is declared, or when the declared minimum is `2.0`. A different major version,
  including one paired with `minVersion: "2.0"`, or any other well-formed minimum is an unsupported
  profile, reported as `gltf.unsupported_feature`. Invalid syntax, types, or minimum ordering use
  `gltf.invalid_document`. Reasons identify `asset.version` or `asset.minVersion` as applicable.
  Schema/version validation is charged to `WorkUnits` before dependency IO; typed tables,
  decoded strings and validation scratch use their actual metered owners.
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
