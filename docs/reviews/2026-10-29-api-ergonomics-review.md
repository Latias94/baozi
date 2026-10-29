# Baozi API, Abstraction, and Ergonomics Review

Date: 2026-10-29, Asia/Shanghai.

Recommendation: make one validated Scene directly readable, make ordinary producer input small,
and preserve validation locations across the import boundary. Keep ownership, central validation,
resource admission, and explicit loss. Remove ceremony that does not enforce one of those guarantees.

## Review basis

This is an additional design review for execution unit C of the
[active delivery plan](../plans/2026-10-29-001-refactor-library-delivery-plan.md), with consequences
for D-F. It complements the [library state review](2026-10-29-library-state-review.md).

I inspected the current facade, public exports, authoring input, construction/rejection path,
semantic readers, IO and dependency contracts, report ownership, postprocessing, examples, and
representative format producers. The shared worktree was changing during inspection; HEAD was
`033e491` at `2026-10-29T09:30:08+08:00`, with additional local changes. Findings describe the inspected
API, not a claim that every concurrent implementation change is represented by that commit.

This review changes only this document. No Cargo command, executable consumer, benchmark, or test
was run. Code under “current” is source-grounded; proposed signatures and “target” examples are
illustrative contracts for implementation, not claims that the proposed API compiles today.
Line anchors identify the inspected state and may move during the shared work.

The local Assimp headers were used only to compare consumer capabilities:
[Importer.hpp](../../repo-ref/assimp/include/assimp/Importer.hpp),
[scene.h](../../repo-ref/assimp/include/assimp/scene.h), and
[mesh.h](../../repo-ref/assimp/include/assimp/mesh.h). Assimp provides immediate access from a scene
to nodes, meshes, geometry, and materials, and explicitly directs multifile memory imports to custom
IO. That is a useful capability baseline. Its importer-owned scene lifetime, raw pointer arrays,
postprocess flags, and error strings are not the recommended Rust design. No reference code or
fixtures were copied.

## Material findings

| Priority | Finding | Consumer consequence | Recommended correction |
| --- | --- | --- | --- |
| P1 | E1. A successful Scene has no typed read surface. | An application can count or hash geometry through a snapshot but cannot use it. | Borrowed domain views and slices/iterators over the existing owner. |
| P1 | E2. Submission discards actionable validation details. | A plugin cannot locate an invalid parent, index, attribute, or cardinality. | A bounded semantic error preserved through terminal finalization. |
| P2 | E3. Ordinary authoring requires publication bookkeeping. | A static triangle needs synthetic-root provenance and empty deformation alignment rows. | A small borrowed batch builder; session-derived defaults; explicit overrides. |
| P2 | E4. Required dependency errors lose resource context. | A nested sidecar failure can report only an open stage/kind and the primary source. | Retain bounded origin, role, and normalized target context with the failure. |
| P2 | E5. Public vocabulary is incomplete and poorly separated. | Common consumers see storage/evidence terms, while constructors return types they cannot name. | Complete read/constructor exports; document the deliberate plugin tier; hide physical metrics. |
| P2 | E6. The public postprocess framework precedes a useful operation. | The only constructible request compiles an empty plan. | Implement a direct typed operation before expanding planning abstractions. |

These are capability and abstraction findings. They do not reopen A's stack/unwind work, B's fidelity
fixes, or glTF admission. In particular, the current worktree already contains F64 custom-attribute
authoring/storage work; C must expose that precision, rather than repeat the old review's claim that
the authoring vocabulary supports only F32/U32/I32.

## What the boundary actually has to guarantee

| Necessary capability or invariant | Appropriate boundary | Accidental requirement to remove |
| --- | --- | --- |
| No partially valid public Scene | One session submission and central validation before publication | A consumer navigating activation, sealing, handoff, or proof types |
| Charge before allocation and retain the charge with live ownership | Private operation/core storage ownership | Public physical block counts as a substitute for semantic access |
| Cancellation and bounded input-dependent work | Parse scope, materialization, validation, and transformations | Metering every subsequent application read of immutable data |
| Cross-scene and stale-handle rejection | Checked public ID lookup and edit boundaries | Generation/owner checks on every relationship followed within the same borrowed Scene |
| Source provenance and truthful stability | Session-observed input plus producer source overrides | Plugins inventing `Generated` provenance and zero option digests for imported documents |
| Complete domain correspondence | Core compilation and validation | Every static producer allocating empty morph/deformation rows |
| Preserve authored and sparse semantics | Typed domain views and explicit conversion operations | Flattening every input into one GPU vertex format during import |
| Explicit loss authority | Import/operation policy and specific loss evidence | A convenience builder silently discarding data or making unknown units “meters” |

An opaque immutable owner is valuable. An owner without useful readers is not sufficient. Similarly,
an unconstructible publication token is valuable; seven mandatory domain batches do not make a
plugin more trustworthy when several batches merely restate counts already present in geometry.

## Workflow 1: import and read geometry, materials, and hierarchy

### E1: snapshots are the only ordinary observation path

[ValidatedScene](../../crates/baozi-core/src/canonical_scene/store/api/scene.rs#L12) exposes
`generation`, `canonical_block_count`, and `capture_snapshot`. The facade
[import_bytes example](../../crates/baozi/examples/import_bytes.rs) imports successfully and then
requests a snapshot merely to print mesh/index totals. At review intake the README called the
unavailable `scene().meshes()` API; A4 has since corrected the quick start to use a snapshot.
The missing typed-read capability remains C's implementation task.

The current public consumer can write this:

```rust
use baozi::{Importer, Result, SceneSnapshotLimits};

fn main() -> Result<()> {
    let report = Importer::new()?.read_bytes(
        "triangle.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
    )?;
    let snapshot = report.scene().capture_snapshot(
        SceneSnapshotLimits::try_new(16_384, 64, 1_000_000).unwrap(),
    ).unwrap();
    assert_eq!(snapshot.logical_mesh_count(), 1);
    assert_eq!(snapshot.primitive_index_count(), 3);
    // There is no supported call here to read the three positions.
    Ok(())
}
```

This is not a cheap count API. The
[snapshot implementation](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/snapshot.rs)
traverses semantic data, hashes it, constructs a bounded preview, and can fail for work/allocation
bounds. It is appropriate test/debug evidence. Preview text is also intentionally incomplete.
Parsing it to recover application data would couple a consumer to a diagnostic rendering and lose
the remainder of a sufficiently large asset.

The target consumer should be approximately this small:

```rust,ignore
use baozi::{AttributeData, AttributeDomain, Importer, Result, TopologyKind};

fn main() -> Result<()> {
    let report = Importer::new()?.read_bytes(
        "triangle.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
    )?;
    let scene = report.scene();
    assert_eq!(scene.mesh_count(), 1);
    let mesh = scene.meshes().next().unwrap();
    let primitive = mesh.primitives().next().unwrap();
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(primitive.indices(), Some(&[0, 1, 2][..]));
    let positions = primitive.positions().unwrap();
    assert_eq!(positions.domain(), AttributeDomain::Vertex);
    match positions.data() {
        AttributeData::DenseF32 { lanes, values } => {
            assert_eq!(lanes, 3);
            assert_eq!(values, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        }
        other => panic!("unexpected fixture representation: {other:?}"),
    }
    let instance = scene.mesh_instances().next().unwrap();
    assert_eq!(instance.mesh().id(), mesh.id());
    let local = instance.node().authored_transform().local_matrix();
    assert_eq!(local, baozi::Mat4::IDENTITY);
    Ok(())
}
```

The triangle is an example with a known dense representation, not a claim that all POSITION
attributes must be dense. Sparse F32 positions are representable in the current authoring validator.
An application that supports arbitrary admitted attributes must handle both payload forms. A
convenience logical iterator can do that without densifying the entire asset.

### Minimum read contract for C

Keep the actual owner named `ValidatedScene` for this increment. Do not introduce a `Scene` alias,
a parallel owned `ReadableScene`, or an export-only mirror of the asset. Public view constructors
remain private. Small views may be `Copy`; the retained owner need not be cloneable.

| Owner/view | Minimum useful methods | Return/semantic contract |
| --- | --- | --- |
| `ValidatedScene` | `node_count`, `root_count`, `mesh_count`, `primitive_count`, `mesh_instance_count`, material/image/sampler/texture counts; `nodes`, `roots`, `meshes`, `mesh_instances`, `materials`, `images`, `samplers`, `textures`; `space` | O(1) counts; independent exact-size iterators; scene-local data only |
| `NodeRef<'s>` | `id`, `name`, `parent`, `children`, `authored_transform`, optional camera/light | `Option<&'s str>`; borrowed relationships; preserve Matrix versus TRS |
| `LogicalMeshRef<'s>` | `id`, `name`, `primitives` | One logical mesh can have multiple material/topology primitives |
| `MeshInstanceRef<'s>` | `id`, `node`, `mesh`, later `skin` and morph weights | Geometry is not duplicated per placement |
| `PrimitiveRef<'s>` | `id`, `topology`, `vertex_count`, `corner_count`, `face_count`, `indices`, `polygon_face_sizes`, `attributes`, `positions`, `material` | Indices are `Option<&'s [u32]>`; absent means the validated implicit stream; polygon entries are per-face corner counts |
| `AttributeRef<'s>` | `semantic`, `domain`, `lanes`, `len`, `data` | Built-ins and borrowed namespaced custom names; count is logical domain elements |
| `MaterialRef<'s>` | `id`, `name`, `shading_model`, `alpha_mode`, `double_sided`, `factors`, `bindings` | Reuse existing semantic factor values; do not reinterpret legacy diffuse as PBR base color |
| `MaterialBindingRef<'s>` | `texture`, `role`, `coordinate_source`, `channel_mapping`, `color_interpretation`, `transform`, `strength`, `blend`, `source_key` | Material-owned binding semantics remain distinct from texture/image identity |
| `TextureRef<'s>` | `id`, `image`, `sampler` | Follow existing validated links directly |
| `ImageRef<'s>` / `SamplerRef<'s>` | `id`, `encoded_bytes`, optional `media_type`; `id`, `state` | Borrow encoded bytes; image decoding remains outside this read API |

`AttributeData<'s>` should be one discriminated borrowed payload enum: `DenseF32`, `DenseF64`,
`DenseU32`, `DenseI32`, `SparseF32`, `SparseU32`, and `SparseI32`, matching the currently retained
forms. Dense variants contain lanes and a typed scalar slice. Sparse variants contain lanes,
logical count, sorted element indices, explicit scalar values, and a default element slice.
Do not add sparse F64 simply for enum symmetry before a producer needs it. Do not expose dynamic
record cells, canonical block references, sequence schemas, or lane-by-lane fallible cursors.

Use a borrowed semantic discriminant such as `AttributeSemanticRef<'s>` with `Custom(&'s str)`
if the existing owned `AttributeSemantic::Custom(CustomAttributeSemantic)` would require allocating
a name on each read. Share conversions with authoring; do not manufacture an owned custom name
inside iteration. This owned/borrowed distinction has a real lifetime purpose.

For dense F32 XYZ data, a slice of scalars plus `chunks_exact(3)` is already useful. A safe helper
that yields `[f32; 3]` values can also handle sparse defaults/overrides, with O(N+K) sequential work
and no allocation. Do not promise `&[Vec3]` over a scalar allocation by casting its layout, or
allocate a new vector to satisfy a getter. Random sparse lookup should state its search cost.

Preserve absence and authored emptiness where meaningful. `None` indices and `Some(&[])` must not
be collapsed by the reader. `Option<&[T]>` already expresses that distinction. A convenience
effective-index iterator may generate an implicit stream; it must not mutate the source topology
or pretend strips/fans are triangle lists.

Counts and iteration must document the synthetic root: `nodes()` and `node_count()` address actual
retained source nodes; `roots()` is the ordered set of actual roots. The internal synthetic root is
not a surprise extra application node. Do not require its transform or provenance to be authored by
an ordinary plugin.

### Lookup, borrowing, and access order

Reuse the existing opaque handles and
[SceneLookupError](../../crates/baozi-core/src/identity.rs#L116). Expose `NodeId`, `LogicalMeshId`,
`PrimitiveId`, `MeshInstanceId`, `MaterialId`, `ImageId`, `SamplerId`, and `TextureId` as needed.
`scene.node(id)` and `scene.mesh(id)` return `Result<...Ref<'_>, SceneLookupError>`; a mesh's
primitive lookup checks its logical-mesh owner as well as generation. IDs come from views, rather
than caller construction from physical indices. Authoring local IDs remain a separate namespace.

Within an already borrowed validated Scene, `instance.mesh()` and `primitive.material()` should
return a view or `Option<view>`, without requiring the user to perform the same lookup manually.
Missing material is a semantic option. A stale ID is a lookup error. A mistyped physical cell in a
supposedly published Scene is an internal invariant failure, not an optional material or EOF.
Establish/fail that invariant at publication; do not add a per-scalar user-visible error protocol.

Returned slices and child views borrow the Scene for `'s`, not the temporary wrapper or iterator:

```rust,ignore
impl<'s> LogicalMeshRef<'s> {
    pub fn primitives(self) -> impl ExactSizeIterator<Item = PrimitiveRef<'s>> + 's;
}

impl<'s> PrimitiveRef<'s> {
    pub fn indices(self) -> Option<&'s [u32]>;
}
```

A consumer must be able to keep a material view while iterating another mesh, start two independent
iterators, stop early, and iterate again. Borrow checking should prevent dropping/moving the Scene
while its data remains borrowed. Requiring `finish()` on a read iterator would add no validation
guarantee: the Scene has already been validated.

Do not implement `node.instances()` by silently scanning the entire instance table on every call.
The initial contract can expose all scene instances plus direct instance-to-node links. Add a
metered private adjacency table only when reverse lookup is implemented with a documented cost.

The same caveat applies to `parent()`: the current `node_scene_node_field` in
[schema.rs](../../crates/baozi-core/src/canonical_scene/store/schema.rs#L65) retains child ranges but
no parent column. An O(1) parent getter therefore needs a compact parent index retained once during
construction/validation under the same Scene owner and charge, later folded into D1's typed node
table. Do not rescan the child graph on every read. Roots/children are sufficient for the first
hierarchy consumer if adding that metadata would widen the initial increment; document that parent
lookup is still pending instead of presenting it as an existing direct field.

### Feasibility with today's storage and D's cutover

The implementation already has borrowed payloads:
[geometry leaf cursors](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/semantic/geometry.rs#L1062)
hold `&[T]`, and `SequenceView::as_slice` exists in
[concrete_store.rs](../../crates/baozi-operation/src/concrete_store.rs#L773). Appearance readers hold
image bytes, names, binding scalars, and factor scalars as borrowed slices. No full-asset snapshot or
second owned model is necessary to expose these capabilities.

However, do not simply make `NodeSceneSemanticCursor` public. It enforces roots/nodes/appearance/
geometry/deformation/animation/profile traversal order and subcursor completion. Geometry also
requires mesh/primitive/attribute/instance phases. That is a transcript/snapshot protocol, not a
natural application API. Repeatedly seeking through it would turn random access into prefix scans.

For C, factor direct row readers from the existing decode logic: `node_at`, `logical_mesh_at`,
`primitive_at`, `attribute_at`, `mesh_instance_at`, and appearance equivalents. Resolve table and
payload references under the existing immutable owner/proof. Use recorded offsets for names and
ownership ranges. Getters must not create a full-scene `semantic_view()` just to read one domain;
the current constructor resolves every domain, including profile.

Keep these readers private and concrete. Public views can contain a Scene borrow plus a logical
index. They do not own another authoritative graph. D1 then replaces their record lookup bodies
with direct typed-table lookup and deletes the replaced schemas/decoders in that same cutover.
There should be no public backend trait, persistent record-to-typed mirror, or permanent migration
adapter. If implementing direct access reveals that D1 is equally small, combine that narrow
node/geometry change with C rather than building a second production representation.

### Deformation/animation extension of the same surface

C's full target also includes the current retained domains, even though the first bounded increment
below can stop at static OBJ/MTL. Expose `skins()`, logical-mesh morph definitions/default weights,
primitive morph deltas, instance overrides, and `animations()` through the same ownership pattern.

Skin views need ordered joint NodeIds/views, inverse-bind matrices, and the existing explicit matrix
space/correction contract. Influence views need per-vertex rows containing variable-length joint,
weight, and source-order data; do not truncate to four influences. Since current storage uses
parallel arrays, a zipped borrowed row iterator is appropriate; a `&[Influence]` would require a
different physical layout. Morph deltas retain semantic, sparse/dense representation, and logical
target identity. Animation clips/channels expose names, targets, time domain, interpolation,
extrapolation, `&[f64]` times, value width, values, and optional cubic tangents. Preserve absent versus
present authored instance weights. A reader is not an evaluator or animation interpreter.

These getters do not admit the glTF backend. E must add its independent semantic consumer and
admission evidence before a facade glTF feature claims this profile.

## Workflow 2: custom IO and sidecars

### What already works at the API boundary

The existing `read_asset(&dyn AssetIo, AssetPath)` is the right operation for a primary asset and
sidecars. `read_bytes` has no sidecar namespace; its source string is a logical path, not authority
to read host files. The currently edited
[memory-sidecar example](../../crates/baozi/examples/import_memory_sidecars.rs) now inserts the PNG
payload as well as OBJ and MTL. This review does not repeat the earlier missing-PNG finding.

The input-side consumer can already be this simple:

```rust
use baozi::{AssetPath, Importer, MemoryAssetIo, Result};
use std::sync::Arc;

fn main() -> Result<()> {
    let io = MemoryAssetIo::new();
    io.try_insert(AssetPath::new("models/a.obj")?, Arc::from(
        b"mtllib materials/a.mtl\nusemtl red\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".as_slice(),
    ))?;
    io.try_insert(AssetPath::new("models/materials/a.mtl")?, Arc::from(
        b"newmtl red\nKd 1 0 0\n".as_slice(),
    ))?;
    let report = Importer::new()?.read_asset(&io, AssetPath::new("models/a.obj")?)?;
    assert_eq!(report.format().id().as_str(), "org.baozi.obj");
    assert!(report.loss_evidence().is_empty());
    Ok(())
}
```

The missing capability is to follow that report through typed material/texture/image links and
assert real values. Under the target API, the existing three-resource example should also check:

```rust,ignore
let primitive = report.scene().meshes().next().unwrap().primitives().next().unwrap();
let material = primitive.material().unwrap();
assert_eq!(material.factors().legacy().unwrap().diffuse(), baozi::Vec3::new(1.0, 0.0, 0.0));
let binding = material.bindings().next().unwrap();
assert_eq!(binding.role(), &baozi::TextureRole::Diffuse);
let texture = binding.texture();
assert_eq!(texture.image().encoded_bytes(), RED_PNG);
let _sampler_state = texture.sampler().state();
```

The existing OBJ producer uses `TextureRole::Diffuse`; preserve that distinction from PBR base
color. `RED_PNG` is the complete constant in the existing example. Image bytes and sampler state
should be read without decoding a pixel or constructing a snapshot.

A custom `AssetIo` needs one stable `AuthorityDomain` for its namespace and must return
`OpenedAsset::new(domain.clone(), path, reader, length)` with the requested normalized path. The
existing [resource contract](../../crates/baozi-io/src/resource.rs#L268) and
[open/read error constructors](../../crates/baozi-io/src/error.rs) already provide this. Clones must
share the domain. Returning a newly constructed domain on every call is an identity violation.
Retain `AssetLength::Exact` versus `Hint` and the standard short-read/EOF contract; no new async
runtime, VFS framework, or IO-specific builder is required for C.

### E4: required failures lose the context needed to repair an asset

[abort_dependency_context](../../crates/baozi-import/src/scope.rs#L3156) completes the unavailable
resource relation, then drops dependency identity/path evidence after recording the generic scope
cause. `AssetOpenError` carries stage and kind, while `ImportFailure::source_identity()` names the
primary resource. The complete successful manifest is not available on failure. Optional dependency
evidence has a normalized path, but that does not repair required-dependency error reporting.

For a required nested dependency that fails in `open`, the consumer cannot reliably distinguish
which sidecar failed from this cause alone. Do not manufacture a runtime reproduction here: this
finding follows the inspected ownership/drop path. Verify it with a custom plugin declaring a
required role, since an optional OBJ texture is not an adequate required-failure test.

Retain one bounded dependency-failure context: origin path, declared role, normalized target path,
and the existing typed open/read/policy cause. Preserve the relation's host-owned evidence rather
than copying arbitrary backend messages. `failure.dependency_context()` can borrow that context;
its lifetime and charge belong to the actual failure owner. Error formatting should make the target
path and role visible. If raw hostile spelling is included, apply the existing text bounds and
sanitization. A missing normalized path after normalization failure must remain explicit.

The target diagnostic is equivalent to:

```text
dependency role image failed: models/materials/a.mtl -> models/materials/textures/red.png
asset open failed during Open: NotFound
```

Do not require a persistent failure-manifest framework merely to report one failed relation.
Continue applying the current authorization/normalization policy. Custom resolvers currently receive
portable normalized references; supporting `asset://...` or arbitrary database keys is a separate
policy/product decision, not something this ergonomics increment should silently enable.

The plugin tier is a reasonable home for custom resolver signature vocabulary. If the facade
continues to advertise `DependencyResolver`, either re-export its signature types
`AuthorizedDependency` and `DependencyReference` and its role constructor error as well, or direct
resolver authors consistently to `baozi-import`. A facade trait name alone does not make a complete
extension API. `AssetReader` is also missing from the facade for backends that implement the chunk
contract directly rather than `std::io::Read`; re-export it if custom IO is advertised there.

## Workflow 3: a custom format plugin with useful failures

### E3: complete producer input contains avoidable ceremony

[submit_minimal_scene](../../crates/baozi/tests/support/mod.rs#L21) constructs a Generated operation,
an all-zero options digest, provenance IDs, and owner coverage for Scene and SyntheticRoot before
submitting an empty node graph. The
[point helper](../../crates/baozi/tests/support/mod.rs#L104) additionally creates logical morph,
primitive deformation, instance deformation, and provenance rows for every geometry owner.
First-party [STL](../../crates/baozi-format-stl/src/parser.rs) and
[OBJ](../../crates/baozi-format-obj/src/mesh_builder.rs) repeat this alignment work in production.

The validator's
[GeometryTableCardinality](../../crates/baozi-core/src/canonical_scene/store/production/plan/deformation.rs#L65)
requirement is legitimate for the final internal representation. It does not imply that a static
plugin should supply those rows. Likewise, the existing
[ProfileOwnerCatalog](../../crates/baozi-core/src/canonical_scene/store/production/plan/profile.rs)
already knows canonical owner coverage; every parser should not maintain an independent copy.

Use `SceneAuthoringBatch` itself as a small borrowed builder. A new general owning `SceneBuilder`
would add another mutable model, lifetime/allocator policy, and future migration burden. The
minimum target is:

```rust,ignore
impl<'a> SceneAuthoringBatch<'a> {
    pub fn new(nodes: NodeAuthoringBatch<'a>) -> Self;
    pub fn with_spatial(self, spatial: SpatialAuthoringBatch<'a>) -> Self;
    pub fn with_appearance(self, appearance: AppearanceAuthoringBatch<'a>) -> Self;
    pub fn with_geometry(self, geometry: GeometryAuthoringBatch<'a>) -> Self;
    pub fn with_deformation(self, deformation: DeformationAuthoringBatch<'a>) -> Self;
    pub fn with_animations(self, animations: AnimationAuthoringBatch<'a>) -> Self;
    pub fn with_profile(self, profile: ProfileAuthoringBatch<'a>) -> Self;
}
```

The descriptors borrow parser-owned arrays only until `scope.submit_scene(...)` returns. They own
no Scene, ledger, validation proof, or publication token. Keep one submission method and the current
opaque `SceneDraft` parser return; do not add `BuiltScene`, `PreparedScene`, or public activation
steps. The existing parser still submits a single complete logical input, with defaults resolved
inside its owning session.

Precise default rules matter more than fluent spelling:

- Omitted spatial input is unknown space and no cameras/lights. It does not imply handedness or
  a unit scale. Omitted appearance, geometry, and animation are empty.
- Omitted deformation means no skin/morph data, with alignment rows generated from the submitted
  geometry. Explicit deformation is validated exactly as supplied; do not repair a partial table.
- Default provenance is the observed primary document, selected format, and existing snapshot
  digest. Keep its real stable/session-opaque classification. Use
  `SourceObjectLocator::indexed(SourceObjectKind::Document, 0)` for document-level source identity.
  Do not rehash the primary payload and do not label imported data Generated.
- `with_profile` supplies explicit resources/sources/provenance and owner overrides. Append a
  document default without rebasing the caller's existing local IDs. Fill only owners with no
  supplied override; an explicitly supplied empty, duplicate, or invalid attachment is an error.
  Defaults never replace a producer's more precise source location or aggregate provenance.
- Build the owner coverage once, using the existing catalog, in bounded controlled work. Keep all
  temporary rows charged until submission finishes. Reuse the current storage/admission mechanism.

Keep the implementation split precise: import supplies the already observed default document
facts; core resolves missing semantic rows during controlled construction preflight. The existing
`ProfileOwnerCatalog::from_validated` needs validated domain counts and the local-to-canonical node
mapping, so it cannot simply be called from `ParseScope` before preflight. Resolve absent deformation
after geometry validation, and fill provenance coverage when the validated owner catalog is available.
Use a small hidden source-default input if necessary to carry the document facts across the existing
seam; it carries no publication or physical-account authority. Do not create a second public profile
builder or a virtual-table framework. Keep the initial input envelope O(1), with conservative bounds
for generated rows, before any input-dependent generation under semantic work/storage admission.

Preserve stable classification only through the existing registered identity evidence. A manifest
digest does not make a caller-authored resource label replay-stable. If the first default path has
no registered translation into canonical resource identity, retain an explicit session-opaque reason
instead of inventing stable authority or starting an attestation project.

This preserves a fast flat batch interface for real parsers. `AuthoringRange` describes logical
ownership of borrowed rows, not a physical account. It can remain for the initial C increment;
a nested mutable mesh builder is not a prerequisite. If later consumer evidence justifies nested
builder methods, replace the superseded public construction path rather than retaining two complete
authoring APIs indefinitely.

With those changes, the full submission body for a small triangle plugin can be:

```rust,ignore
use baozi_import::authoring::{
    AttributeDomain, AttributeSemantic, AuthoredTransform, AuthoringRange,
    GeometryAttributeAuthoring, GeometryAuthoringBatch, GeometryLogicalMeshAuthoring,
    GeometryMeshInstanceAuthoring, GeometryPrimitiveAuthoring, GeometryTopologyAuthoring,
    LogicalMeshLocalId, NodeAuthoringBatch, NodeAuthoringNode, NodeLocalId, Presence,
    SceneAuthoringBatch,
};
use baozi_import::{ParseResult, ParseScope, SceneDraft};

fn submit_triangle(scope: &ParseScope<'_>) -> ParseResult<SceneDraft> {
    let positions = [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let indices = [0, 1, 2];
    let roots = [NodeLocalId::new(0)];
    let nodes = [NodeAuthoringNode::new(
        Presence::Absent, AuthoredTransform::identity_trs(), Presence::Absent, Presence::Absent,
    )];
    let attributes = [GeometryAttributeAuthoring::f32(
        AttributeSemantic::Position, AttributeDomain::Vertex, 3, &positions, Presence::Absent,
    )];
    let primitives = [GeometryPrimitiveAuthoring::new(
        GeometryTopologyAuthoring::TriangleList, Presence::Present(&indices),
        Presence::Absent, AuthoringRange::new(0, 1),
    )];
    let meshes = [GeometryLogicalMeshAuthoring::new(AuthoringRange::new(0, 1))];
    let instances = [GeometryMeshInstanceAuthoring::new(NodeLocalId::new(0), LogicalMeshLocalId::new(0))];
    scope.submit_scene(
        SceneAuthoringBatch::new(NodeAuthoringBatch::new(Presence::Present(&roots), &nodes))
            .with_geometry(GeometryAuthoringBatch::new(&meshes, &primitives, &attributes, &instances)),
    )
}
```

This deliberately reuses today's per-domain constructors. A complete plugin implements the existing
`FormatImporter::descriptor`, `detect`, and `parse`, registers with `Importer::builder().register`,
and calls this body from `parse`. The external consumer below must exercise that entire chain;
calling a test helper directly is insufficient.

`Presence<T>` duplicates `Option<T>`'s absent/present semantics, including present empty slices.
It does not prevent a distinct illegal state. Replace it with `Option` when the affected constructor
families are next cut over, without adding an alias. In particular, the node validator rejects
`RootsAbsent`, so a complete node batch should simply require a root slice. Original source-field
absence belongs in source evidence where needed, rather than a mandatory constructor argument that
can only be rejected. This cleanup is not a reason to delay useful reads or expand the first C
increment into every animation/material constructor.

### E2: useful errors exist, but the public boundary throws them away

The private validators already know enough to diagnose real mistakes:

- [ParentOutOfRange](../../crates/baozi-core/src/canonical_scene/store/production/plan.rs#L181):
  authored node, referenced parent, and node count.
- [VertexIndexOutOfRange](../../crates/baozi-core/src/canonical_scene/store/production/plan/geometry.rs#L201):
  primitive, corner, index value, and vertex count.
- Attribute cardinality/schema, polygon boundary, material reference, and deformation alignment
  variants in the same domain planners preserve comparable details.

The information is reduced through
[NodeSceneConstructionRejection](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene.rs#L307),
`SceneActivationRejection::stable_code`, the boolean authoring test in
[scene_activation.rs](../../crates/baozi-import/src/store/scene_activation.rs#L184),
`ImportStoreActivationError::AuthoringRejected`, and the mappings in
[ParseScope::submit_scene](../../crates/baozi-import/src/scope.rs#L624). Initial construction errors
are discarded with `map_err(|_| ...)` too. Finalization may make the public cause
`ImportCause::OperationTerminal`, whose available facts are class and code. The current
[boundary test](../../crates/baozi/tests/importer_boundary.rs) asserts precisely that generic result.

Add one semantic error value in core, rather than exposing all generic/control/storage planner
errors. A feasible bounded shape is:

```rust,ignore
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SceneValidationError {
    ParentOutOfRange { node: u32, parent: u32, node_count: u32 },
    VertexIndexOutOfRange { primitive: u32, corner: u64, index: u32, vertex_count: u32 },
    AttributeCardinality { attribute: u32, domain: AttributeDomain, expected: u64, actual: u64 },
    // Add concrete semantic variants for the existing supported validators.
}
```

Each variant determines its domain and rule; expose `domain()` and a stable `code()` such as
`geometry.vertex-index-out-of-range` without storing duplicate discriminants or an unbounded text
map. Use producer-local row indices for submission errors, not generated canonical node order or
unpublished Scene IDs. Range/limit failures must retain actual and expected/maximum values where
available. Do not cast unchecked collection lengths into these fields: reject count-domain overflow
separately. Negative/NaN data errors can retain scalar position and rule without formatting arbitrary
floating-point values into terminal storage.

Map the existing domain failures once while they still contain their concrete fields. Carry that
small value through the current private construction/activation error and terminal owner. Keep
control, budget/allocation, and internal materialization errors classified separately; not every
rejected activation is a producer validation error.

A small `SceneValidationDomain` enum can name Scene/Spatial/Node/Geometry/Appearance/Deformation/
Animation/Profile for `domain()`. Do not expose a generic schema path or physical table ID to provide
that information. The concrete semantic variants supply object indices and rule-specific values.

Expose `ImportCause::SceneValidation(SceneValidationError)` for the ordinary primary cause, or an
equally direct `ImportFailure::scene_validation_error()` accessor backed by that same cause value.
Do not require downcasting private errors or reverse lookup from magic numbers. The terminal ledger
still supplies the authoritative failure class/code. The semantic value is its bounded diagnostic
detail, not a second finalizer or independently authored terminal outcome. Preserve it when a later
terminal step succeeds; preserve primary plus cleanup information if terminal cleanup also fails.

The negative consumer changes from this current assertion:

```rust
// Current shape after extracting Error::Import(failure):
assert!(matches!(failure.cause(), baozi::ImportCause::OperationTerminal(_)));
```

to an assertion that actually helps a plugin author:

```rust,ignore
let detail = failure.scene_validation_error().unwrap();
assert_eq!(detail.code(), "geometry.vertex-index-out-of-range");
assert_eq!(detail, &SceneValidationError::VertexIndexOutOfRange {
    primitive: 0, corner: 2, index: 3, vertex_count: 3,
});
assert!(failure.to_string().contains("primitive 0"));
assert!(failure.to_string().contains("corner 2"));
```

Change the plugin triangle's last index from `2` to `3` to trigger this case. Separately retain the
existing invalid-parent scenario (`node=1`, `parent=2`, `node_count=2`). A snapshot assertion or a
generic terminal-code assertion cannot replace either test.

### Small exact ergonomics patch for the root to apply

Simple format failures currently require a custom type implementing
[FormatFailureEvidenceProducer](../../crates/baozi-import/src/error.rs#L788). Keep that producer
extension for structured details, but add this method next to `ParseScope::format_failure` in
`crates/baozi-import/src/scope.rs`:

```rust
/// Captures a format-owned code and message using the existing bounded evidence path.
pub fn format_error(&self, code: &str, message: &str) -> ParseFailure {
    struct Message<'a> {
        code: &'a str,
        message: &'a str,
    }

    impl crate::FormatFailureEvidenceProducer for Message<'_> {
        fn code(&self) -> &str {
            self.code
        }

        fn visit_details(&self, sink: &mut dyn crate::FormatFailureDetailSink) {
            sink.string("message", self.message);
        }
    }

    self.format_failure(Message { code, message })
}
```

Then a small parser can return
`Err(scope.format_error("example.invalid-header", "expected BZTR header"))` without another public
error framework. The existing capture path validates codes, bounds/sanitizes details, meters storage,
and copies borrowed text synchronously. Keep those semantics, including fail-closed behavior when
the bounds are exceeded. Do not add a generic `From<any error>` that discards cancellation/budget
ownership or retains a plugin-owned error object. Test one ordinary message, invalid code/oversized
detail using the existing limit behavior, and survival after the plugin's borrowed text is dropped.

Also update `FormatFailureEnvelope::Display` in `baozi-import/src/error.rs` to include a captured
`message` detail when present, alongside format and code. Today it prints only format and code.
Use the already sanitized bounded detail and write it to the formatter directly; do not allocate a
second error string. Keep all other structured details available through their existing getters.

This patch is described here instead of applied because this worker owns only the review artifact.
It does not fix E2 by itself: a format-owned message cannot substitute for structured core validation.

## E5: complete and narrow the vocabulary deliberately

The facade currently re-exports a large evidence vocabulary while the only Scene metric exposed
directly is physical block count. Meanwhile, the custom importer in
[importer_api.rs](../../crates/baozi/tests/importer_api.rs) needs `baozi-import` for `ParseScope`,
`SceneDraft`, detection types, and failure producers even though `FormatImporter` is visible at the
facade. This can be an intentional extension tier, but examples must state it.

Recommended C packaging: keep ordinary read types and their complete semantic vocabulary exported
from `baozi-core`, and re-export the supported read vocabulary from `baozi`. Keep plugin authoring at
the existing `baozi_import::authoring` path, with parser traits/scopes at `baozi-import`. An external
plugin consumer deliberately depends on `baozi-import`; the main import/read consumer does not.
Document both dependency sets. Do not add a parallel `baozi::plugin` authoring facade just to shorten
imports, and do not expose `_scene-integration` to consumers. A future module grouping can be a
normal packaging change; it is not the architecture fix.

Audit the public signature closure. In
[api/authoring.rs](../../crates/baozi-core/src/canonical_scene/store/api/authoring.rs), current exports
include constructors and getters whose related error/value types are hidden by private modules:
`AuthoredTransformError`, `AuthoredTransformKind`, `TrsTransform`, `EntityNameError`,
`NamespacedNameError`, `MaterialFactorError`/`MaterialFactorField`, and the texture coordinate,
channel mapping, transform, strength, and source-key error types are concrete examples. Re-export
the types needed to name return values and match errors. Do the same for skin space/correction and
other new read signatures. `?` into `Box<dyn Error>` is not adequate evidence that a typed API is
complete.

Make authoring-local IDs consistent too. `NodeLocalId::index()` and the appearance local-ID indices
are public, but `LogicalMeshLocalId::index()` in `production/input/geometry.rs` and several
deformation/lineage local-ID accessors are restricted to core. These are explicit producer row
ordinals accepted by public `new(u32)` constructors, not physical store indices. Make their `index()`
accessors public when exposed as supported authoring types; it helps correlate validation errors
with a plugin's arrays. Keep Scene handle internals and physical indices private.

Retain the useful registry/request distinction: registration is mutable, publication freezes claims,
and `PreparedImportRequest` belongs to its frozen registry and is checked before IO. The existing
default `read_bytes`/`read_asset` methods already hide preparation for a simple application. Do not
replace this with another per-stage public typestate chain, and do not remove request qualification
merely to shorten the advanced controlled-import call. Detection, option codecs, and structured
failure producers remain opt-in plugin capabilities rather than prerequisites for application reads.

Make `ValidatedScene::canonical_block_count` private/integration/test-only and remove its uses from
public examples and postprocess Debug output. Keep semantic counts and resource evidence as
separate concepts. `ImportReport::into_parts()` already separates the unique Scene owner from its
import evidence and is useful for F; preserve that capability. Re-export `ImportEvidence` and loss
evidence/value types where facade users must name them, rather than inventing new report wrappers.
Do not add another generic report/telemetry layer for basic application use.

## E6: F should expose a concrete operation before another plan abstraction

[PostProcessRequests](../../crates/baozi-postprocess/src/pipeline.rs#L34) only constructs an empty
request. Compile/run/plan/step objects and multiple failure wrappers are already public, but no
triangulation, normals, or coordinate conversion can be requested. This is an explicit unfinished
capability, not a faulty implementation of triangulation.

Keep the useful owning behavior: failure returns the unchanged input; unchanged success can retain
its generation; a changed Scene is centrally validated; reports expose cancellation/resources/loss.
Avoid requiring callers to understand plan construction for a single operation. The target can be:

```rust,ignore
let (scene, import_evidence) = report.into_parts();
let outcome = Triangulate::default().run(scene, control)?;
let (scene, triangulation_report) = outcome.into_parts();
```

`run` can perform private preflight/compilation internally. If callers later need inspectable
preflight, expose that concrete prepared operation based on a real use case. Do not add a general
descriptor registry, dependency scheduler, replay protocol, or postprocess plugin framework first.
Fold empty-only plan/step wrappers into the actual implementation or delete them when replacing
that public path; do not preserve an unpublished empty framework as an obligatory front door.

C must keep Corner/Face attributes visible so F can remap them honestly. A triangulated primitive
still need not have a single index for position/UV/normal tuples. Deliver an explicit tuple-unifying
helper/operation, with seam preservation, after triangulation and requested normal/space operations.
Precision lowering, influence truncation, or other losses require specific caller authorization.
Do not make `positions()` or material reading perform those transformations implicitly.

## Competing designs and decision

| Choice | Benefit | Cost/risk | Decision |
| --- | --- | --- | --- |
| Public owned fields and public typed `Vec` tables | Immediately familiar, direct slice access | Freezes layout; invites unchecked mutation or another validated wrapper; couples accounting to consumer ownership | Keep the typed tables private; public borrowed views provide the needed access. |
| Publish the existing sequential semantic cursors | Small export-only patch | Exposes phase/finish protocol; awkward lifetimes/access order; random lookup risks rescanning | Reject as the application API; keep snapshot traversal private. |
| Rebuild a second public Scene from snapshots/records | Easy isolated implementation | Duplicates memory and validity authority; creates a permanent adapter | Reject. |
| Borrowed views over one owner, private direct lookup | Small consumer model; preserves immutability; permits D's layout change | Requires concrete row readers now and their deletion/replacement during D | Recommended for C, with measured direct access and no full-asset projection. |
| Fully owning general Scene builder | Convenient incremental authoring | More allocation/lifetime policy and another mutable semantic model; large implementation surface | Defer until a workflow needs it. |
| Existing borrowed batches with defaults at submission | Reuses actual parser data; removes provenance/alignment ceremony | Flat ranges remain an advanced authoring concept; defaults need precise merge/error rules | Recommended first authoring increment. |
| Serialize all planner error internals into a generic detail tree | Maximum apparent flexibility | More storage/schema machinery; leaks runtime types; difficult stable error matching | Reject; use bounded semantic variants from real validators. |

The recommendation retains real semantic concepts: mesh/primitive/instance; attribute domain and
sparse storage; image/sampler/texture/binding; source identity versus Scene ID. It removes redundant
state transitions and user-authored restatements. Fewer type names alone is not the objective.

## Bounded next implementation increment for the root

After the active A/B work, implement **one static OBJ/MTL + custom-plugin consumer increment**.
It may be split into the following buildable commits; it does not claim all of C, E, or F complete.
Do not add glTF admission, a complete D migration, an owning builder, or new dependencies to it.

### 1. Static typed reads on the sole Scene owner

- Edit `crates/baozi-core/src/canonical_scene/store/api/scene.rs`; add concrete view definitions in
  `api/scene/{node,geometry,appearance}.rs` or a similarly small module split. Add the static view
  types, `AttributeData`, borrowed custom semantics, typed IDs/lookups, direct counts, and the
  getters in the minimum read table. Export through `baozi-core/src/lib.rs` and `baozi/src/lib.rs`.
- Factor direct private row/payload readers from `construction/node_scene/semantic.rs` and
  `semantic/{geometry,appearance}.rs`. Reuse `identity.rs`, proof counts, and payload slices.
  Do not reach `semantic_view().into_cursor()` for a getter and do not allocate a read cache.
  If including `parent()`, retain its compact validated index at construction; otherwise exercise
  hierarchy through roots/children and add reverse lookup with D1.
- Remove `canonical_block_count` from the supported public surface; update its postprocess Debug
  callers and any external tests rather than adding an alias metric.
- Update `baozi/examples/import_bytes.rs` and `import_memory_sidecars.rs` to assert typed values and
  relationships; retain snapshots only in examples/tests specifically about snapshots.

### 2. Small authoring input and preserved errors

- Change the constructor/builder in `core/.../production/input.rs` and its authoring exports. Reuse
  the domain batches; add optional/default state privately. Update existing production callers in
  STL/OBJ/PLY and the internal glTF candidate to the same constructor path. Do not keep the old
  seven-argument constructor or add a legacy adapter.
- In `baozi-import/src/scope.rs`, obtain default source facts from the existing session/resource
  graph and pass only those facts through the existing hidden construction seam. Resolve missing
  deformation/profile rows in core's controlled preflight, using
  `production/plan/{deformation,profile}.rs` and the validated owner catalog. Adjust
  `production/scene.rs` envelope bounds for generated defaults without adding pre-admission scans.
  Defaults and explicit input enter the same central construction and use the same admission path.
- Add `SceneValidationError` and a domain discriminant/code method in a core public error module;
  map the existing node/geometry semantic failures first. Make material/deformation and other
  existing-domain failures equally identifiable before claiming full C error coverage. Preserve
  the value through `core/.../construction/node_scene.rs`, `node_scene/materialization.rs`,
  `baozi-import/src/store.rs`, `store/scene_activation.rs`, `error.rs`, `outcome.rs`, and final
  cause selection in `session.rs`. Carry a small optional detail through the existing owner.
- Add `ParseScope::format_error` exactly as described above. Complete public constructor error/value
  exports in `api/authoring.rs`; no generic error registry or new dependency is necessary.

### 3. Independent consumer and focused verification

Use a standalone Cargo package under an existing consumer area, or a small new
`tests/consumers/api_ergonomics/` directory with its own `[workspace]`. It is outside workspace feature
unification, with only `baozi` for application reads and a deliberate `baozi-import` dependency for
its plugin/resolver. Root owns manifest changes, execution, and the repository's normal verification.

The consumer must exercise these behaviors, using handwritten expected values:

1. Import a triangle and a polygon OBJ; inspect exact positions, explicit/implicit indices where
   applicable, polygon face sizes, Corner UVs and Face smoothing data. Check mesh/primitive/instance
   relationships, authored transform representation, root order, and parent/child links. Use a
   custom hierarchical plugin fixture for facts OBJ does not author. A roots/children-only first
   increment asserts the edges directly; it does not fake a constant-time parent getter.
2. Import OBJ -> nested MTL -> image through memory IO and a custom IO wrapper with a shared stable
   domain and open log. Check exact material factors, binding semantics, encoded bytes, sampler,
   three opened resources, no invented losses, and session-owned snapshot reuse. Use a fixture
   with repeated image references to verify one resource acquisition. Keep the existing IO policy.
3. Register a custom `FormatImporter`, use the small builder to submit geometry, and read its typed
   output through the facade. No `baozi-core::scene::integration`, operation-store type, custom stack
   size, or snapshot is allowed in this consumer.
4. Reject its out-of-range vertex and invalid parent with exact semantic details through the facade.
   Reject explicit deformation cardinality errors rather than filling them. Check default provenance
   covers every owner, keeps the observed document identity, and honors a more precise override.
5. Read sparse defaults/overrides without dense allocation; retain a non-F32-representable custom
   F64 value exactly after the concurrently owned F64 change is integrated. Check absent versus
   present empty payloads where supported. Do not write a second F64 implementation in this slice.
6. Verify two independent iterators, early stopping/restarting, and a retained payload borrow after
   its temporary view is gone. Add one compile-fail lifetime example and one foreign-scene ID lookup
   test. Keep owner/generation mismatch tests at the actual lookup boundary.
7. Add focused core assertions/counters showing repeated counts/getters allocate nothing, do not
   capture snapshots, and do not walk earlier rows. A deterministic row-read counter at first/last
   rows is sufficient; do not create a shared instrumentation framework. Keep normal-stack success
   and error checks with the root's A1 coverage.

For E4, the smallest additional test uses a custom plugin with a declared required sidecar role and
an intentionally missing nested resource. Retain/assert origin, role, target, and open kind through
finalization. This can be the next focused commit if retaining path ownership would otherwise widen
the initial C increment. It is not a reason to delay the typed-read consumer.

Use `cargo fmt`, focused `cargo nextest` runs, external consumer check/run and a doc test for the new
public examples, with Cargo serialized as appropriate to host load. Broaden verification according
to the touched modules and repository requirements. This review has not executed those checks.

### Delete the code and tests that the increment replaces

- Remove the seven-argument `SceneAuthoringBatch::new` API after updating its callers; keep one
  submission path. Do not retain a second “legacy” Scene input or conversion facade.
- Delete manual default Generated/zero-digest provenance assembly in minimal/plugin helpers once
  document defaults exist. Delete repeated empty deformation-row construction in migrated static
  format producers. Keep explicit source/provenance and actual deformation authoring.
- Replace the detail-erasing `map_err(|_| ...)` for construction/semantic errors and the
  `AuthoringRejected` unit payload with semantic detail carriage. Keep distinct control/runtime
  rejection behavior. Numeric terminal evidence remains available for accounting.
- Replace snapshot-count-only acceptance assertions for typed-read workflows. Keep snapshot digest,
  truncation, semantic traversal, cancellation, and corruption tests that actually test snapshots.
- Remove public physical block count and its consumer expectations. Do not replace it with another
  public physical-store proxy.
- Delete now-unused duplicate row decoders/validators as direct readers are factored; retain the
  cursor only while snapshots or real internal consumers use it. D deletes each obsolete record
  schema/materialization/projection path at the corresponding typed-table cutover.
- When `Presence` constructor families are converted, delete the custom enum/helpers and update
  callers to `Option`; do not add `type Presence<T> = Option<T>`. This is a separate bounded cleanup
  if full-domain churn would distract from the static consumer.
- In F's actual replacement, remove unused empty-only plan/step APIs and tests that merely assert
  their placeholder shape. Retain unchanged/failure-owner/resource/control behavior tests.

This increment is reviewable when a downstream program can read the static asset and repair an
invalid plugin submission from the reported domain/rule/values. An increase in exported type count,
another internal snapshot helper, or more plan objects does not establish that result.
