# Reading a Validated Scene

`ImportReport::scene()` borrows the single immutable `ValidatedScene`. Its domain views do not copy
payloads or allocate. Keep the report alive while using its views, names, or scalar slices. Snapshot
capture is a separate bounded diagnostic operation and is not required to consume an imported model.

## Geometry and placements

Start with `scene.meshes()` for reusable geometry or `scene.mesh_instances()` for placements. Each
instance follows directly to its `mesh()` and `node()`. A logical mesh owns an ordered primitive
range; it can contain different topologies and material bindings.

`primitive.topology()` describes the retained stream. Import does not silently triangulate polygons,
strips, or fans. `polygon_face_sizes()` gives each polygon's number of corners. `indices()` returns
`Option<&[u32]>`: absence denotes the validated implicit stream, and `Some(&[])` preserves an
authored empty stream. Vertex, Corner, and Face attribute domains remain distinct.

Use `positions()`, `normals(domain)`, `texcoords(set, domain)`, or
`attribute(AttributeSemanticRef, domain)` to find a primitive attribute. Position uses the Vertex
domain; other lookups require the complete semantic/domain key because one primitive may retain
distinct Vertex and Corner streams for the same semantic. Lookup is linear in that primitive's
attribute count. Iteration retains borrowed namespaced custom semantics instead of allocating a
name for each read.

`AttributeData` distinguishes dense F32/F64/U32/I32 and sparse F32/U32/I32 payloads. Dense `values`
are tightly packed scalars with an explicit lane count. Sparse payloads expose sorted logical element
indices, one explicit element per index, and one default element. `AttributeRef::len()` includes
implicit defaults. Applications must choose how to consume these representations; readers do not
implicitly densify or reduce precision. In particular, F64 custom PLY data remains F64.

`attribute.element(index: usize)` selects one logical element as
`Option<AttributeElement<'scene>>`. `AttributeElement::F32`, `F64`, `U32`, and `I32` contain borrowed
scalar slices of exactly the attribute's lane count. Dense selection is O(1); sparse selection is
O(log explicit count), borrowing either the explicit value or the retained default. An out-of-range
index returns `None`. F64 precision, integer values, signed zeros, and scalar bits are unchanged.
The original `attribute.data()` remains available when source sparse representation matters; an
element read does not create or retain a dense expansion.

`attribute.tangent_conventions()` returns `Some(TangentConventionsRef { basis, encoding })` for a
Tangent attribute and `None` for every other semantic. Each semantic/domain stream owns its own
declaration. `TangentBasisRef` describes the construction algorithm; `TangentEncodingRef` describes
bitangent reconstruction. Both preserve explicit unknown values and borrow any namespaced schema
identity from the Scene. A missing Tangent attribute has no declaration; an existing Tangent with
unknown conventions still returns `Some` with both facts unknown. Readers do not infer either fact
from four-component values, normals, UVs, materials, or scene space.

Custom importers declare these facts on `GeometryAttributeAuthoring` with
`.with_tangent_conventions(TangentConventions { basis, encoding })`. Tangent constructors default
to `TangentConventions::UNKNOWN`; conventions belong to that descriptor rather than a Scene-wide
default. The selected glTF profile declares an unknown construction algorithm and `SignInW` on each
imported base Tangent attribute.

## Application list buffers

Use `baozi::postprocess::list_vertices(primitive)` to explicitly obtain list draw tuples from
Points, LineList, LineStrip, LineLoop, TriangleList, TriangleStrip or TriangleFan. It returns
`Result<ListVertices<'scene>, VertexLayoutError>`. The borrowed Scene retains its original topology.
Polygons require the existing explicit `triangulate_primitive` geometry operation first.
`triangle_vertices(primitive)` is the strict TriangleList entry and returns the same shared view
and tuple types; it rejects every other topology, even when empty. The former triangle-only public
type names are replaced by `ListVertices` and `ListVertex`.

The layout and its tuple views are `Copy` and borrow the current primitive:

| Layout method | Result |
| --- | --- |
| `primitive()` | The original borrowed `PrimitiveRef`, with source topology |
| `topology()` | Output Points, LineList or TriangleList |
| `len()`, `is_empty()` | Expanded application tuple count and emptiness |
| `vertex(index: usize)` | `Option<ListVertex>` with an out-of-range check |
| `vertices()` | An independent exact-size, double-ended, cloneable iterator |
| `indices()` | Allocation-free `Range<u32>` from zero to output tuple count |

Points and native lists retain their Corner order. For source Face `f` and source Corner count `n`,
assembly uses these exact Corner ordinals before resolving source indices:

| Source topology | Output topology | Source Corners per output element |
| --- | --- | --- |
| Points | Points | `[f]` |
| LineList | LineList | `[2f, 2f + 1]` |
| LineStrip | LineList | `[f, f + 1]` |
| LineLoop | LineList | `[f, (f + 1) % n]` |
| TriangleList | TriangleList | `[3f, 3f + 1, 3f + 2]` |
| TriangleStrip | TriangleList | even `f`: `[f, f + 1, f + 2]`; odd `f`: `[f, f + 2, f + 1]` |
| TriangleFan | TriangleList | `[f + 1, f + 2, 0]` |

Every output element follows source Face `f`. Degenerate elements remain present and occupy their
original Face, including strip parity and the closing loop edge. A two-Corner loop yields both
directions. Core-authored strips/fans/loops without a complete element yield zero tuples; their
source Vertex/Corner data remains in the Scene. glTF's format-specific minimum counts are stricter.
Checked expansion returns `VertexLayoutError::SizeOverflow` when the output count cannot fit u32.

`vertex.index()` is the expanded application ordinal. `source_corner()` gives the original Corner;
`source_vertex()` resolves that Corner through source indices or implicit indexing; `source_face()`
is the assembled source element. These are independent ordinals: a fan hub has the same source
Corner/Vertex in several Faces. Endpoints use the current generation-qualified `PrimitiveId`.
Earlier edit reports own any relation to a prior Scene. Vertex fields follow the source Vertex,
Corner fields follow the source Corner and Face fields follow the source Face.

Equal tuples may repeat. No deduplication, welding, precision conversion, normal generation,
coordinate conversion or numerical degeneracy test occurs in these readers. Unreferenced source
vertices remain in the Scene and are not appended to the draw layout.

`vertex.attributes()` iterates all primitive fields in retained order. For each `VertexAttribute`,
`field.attribute()` returns the original `AttributeRef`, including semantic, source domain, and
dense/sparse representation, while `field.element()` returns the selected borrowed `AttributeElement`.
`vertex.attribute(semantic, domain)` selects one complete key in O(attribute count). Vertex and
Corner normals or UVs with the same semantic remain separate; Face/custom fields are available
without shadowing another domain. Full-field iteration visits each descriptor once per tuple
instead of repeating keyed searches. No input-sized field inventory is allocated.

`vertex.influences()` borrows the complete row at `source_vertex().index`, preserving absence, empty
rows, every influence, weights, and source order. `vertex.morph_delta(target_id, semantic)` returns
`Result<Option<Vec3>, SceneLookupError>` after resolving the target through the current primitive.
A wrong generation or logical-mesh owner is an error. A missing semantic stream is `None`; an
implicit zero in a present sparse stream is `Some(Vec3::ZERO)`. All repeated tuples of one source
Vertex use that same Vertex for both base data and deformation. Skin palettes and effective morph
weights still come from the mesh instance, so sharing a layout does not select one instance's skin
or override vector. The original morph target/delta readers remain available for source presence
and representation.

Constructing and reading this layout allocates no heap storage, changes no Scene, and starts no
operation invocation or account/resource report. Tuple element slices borrow the Scene, so they
can outlive a temporary layout/iterator but cannot outlive that Scene. Application-owned buffers
copied from them have their own lifetime.

Allocate and size the application's concrete vertex/SoA buffers explicitly. A nonindexed draw can
consume the tuples directly; an indexed consumer may collect `layout.indices()` into its own
buffer. Buffer allocation, checked size products, fallible reservation, packing, field selection,
and interruption of the extraction loop belong to application code and are outside library
accounting. Repeated Corner data can produce larger buffers than a deduplicated layout; zero reader
allocations do not imply small application buffers. The helper does not truncate influences,
reduce precision, or authorize such losses.

## Hierarchy and spatial declarations

`nodes()` and `node_count()` contain source nodes. `roots()` gives the ordered actual roots; it does
not expose the implementation's synthetic root. Every node has independently iterable ordered
`children()` and a source-preserving `authored_transform()`. Matrix and TRS forms remain distinct;
`local_matrix()` derives a matrix when the application needs one.

Parent lookup is not currently exposed. Traverse from the roots and maintain application traversal
state when parent relationships are required. A future direct parent lookup must retain an admitted
index rather than scan the graph on every getter.

### Base-pose world transforms and static bounds

`scene.base_pose_world_transforms(limits, cancellation)` derives an owned
`BasePoseWorldTransforms` table in one linear forest traversal. Use
`SpatialQueryLimits::new(max_work, max_bytes)` and optionally borrow a `CancellationToken`.
The table preserves the originating generation and may outlive the Scene. Its
`node_transform(NodeId)` checks generation/range and returns a local-to-world `Mat4` in O(1).
Every ancestor contributes `parent_world * local`, including nodes without attached geometry.
Coordinates, units and unknown declarations remain those of the Scene.

The named pose is the authored node base pose. Clips may be present, but these queries do not
sample a time or interpolate animation. Node transforms are available even for nodes participating
in skinning or morph animation. Query a placement with
`world.instance_bounds(instance, limits, cancellation)` or explicitly aggregate all placements with
`world.scene_bounds(scene, limits, cancellation)`. The latter rejects a different Scene generation,
including an empty Scene from another publication. Recompute the table after a successful edit.

Bounds transform all logical Vertex Position elements for each placement, including unreferenced
vertices and sparse defaults. They do not depend on topology or index-stream emptiness. This set
can be larger than the draw tuples described above. An uninstanced mesh does not contribute to
Scene bounds. Actual positions are transformed rather than corners of a local box, so rotation
and shear do not introduce nonexistent vertices. No geometry is flattened or baked.

An instance with a skin or any nonzero effective morph weight returns a typed error identifying
that instance. A present instance override replaces mesh defaults; `-0.0` counts as zero. Morph
definitions/deltas and inert influences without an instance skin remain readable and do not
prevent a query. Other static instances can be queried individually. Whole-Scene aggregation
fails on any unsupported placement, without publishing a partial bound. No contributing vertices
returns `None`; an actual point returns `Some` with equal minimum and maximum.

TRS and matrix products use F64 intermediates. Local components, world components at each node,
and transformed vertex components round to F32 at their respective boundaries. The component dot
products add terms in increasing column order, with translation last for a point. Nonfinite
results and nonzero F64 values rounding to zero are errors, including for root transforms.
Bounds enclose these rounded F32 world vertices; they are not certified intervals over exact real
arithmetic. Negative, nonuniform and zero scales need no inverse and are supported.

World-table construction allocates fallibly under the explicit capacity-byte limit; traversal
needs no separate parent index or queue. Work counts initialized slots, evaluated nodes, visited
instances, effective morph weights, primitives, inspected attributes and logical Position elements.
Bounds queries allocate no heap storage and scan only the requested placement unless the caller
explicitly asks for the complete Scene. Cancellation is polled throughout and before success.
These derived allocations belong to the query, independently of import/postprocess retained-storage
accounting. The canonical Scene, metadata, provenance and snapshot do not change.

### Cameras, lights and spatial declarations

Node camera/light attachments follow to scene-owned definitions. `scene.space()` returns the
non-generic `SceneSpaceRef` with handedness, axes, units, winding, and UV origin, preserving unknown
facts. Tangent conventions are read from the actual attribute. Reading never assigns missing units
or performs an implicit coordinate conversion.

Match `camera.projection()` as `CameraProjection::Perspective(parameters)` or
`CameraProjection::Orthographic(parameters)`. Each payload includes its complete `axes()` alongside
FOV/aspect or half-extents and clipping distances. `light.definition()` returns the complete
`LightDefinition`, including kind, axes, canonical tint, and photometric intensity.
The old frame-free kind/tuple and light scalar getters are removed.

`LocalAxes::basis_matrix()` maps the fixed optical reference coordinates (+X right, +Y up,
-Z forward) into the attached node's local coordinates. Apply the node's evaluated placement
after this basis. All three signed axes are independent, so reflected frames remain reflected.
Coordinate conversion composes this intrinsic basis without adding nodes or changing attachments.

## Materials and images

`primitive.material()` follows its optional material. A material exposes its authored name, shading
model, alpha behavior, factor groups, and ordered bindings. Legacy diffuse factors retain their own
meaning; they are not reinterpreted as PBR base color by a getter.

A binding describes a material use: texture role, UV set, source-channel mapping, color
interpretation, transform, strength, blend operation, and optional source property key.
`binding.uv_set()` returns a direct `u16` in `0..=65535`; there is no coordinate-source enum or
generated spatial mapping. The index is independent of a primitive's UV stream count. Resolve it
with `primitive.texcoords(u32::from(binding.uv_set()), domain)`, choosing Vertex or Corner explicitly
and handling an absent stream. Reads do not generate coordinates or select another set implicitly.

Follow `binding.texture().image()` to encoded bytes and `texture.sampler()` to sampling state.
Binding, texture, sampler, and image identities remain distinct. Image decoding belongs to the
application. Factor groups and closed binding values are returned by value; names, optional source
keys, and encoded bytes borrow the Scene's actual storage. No getter allocates a replacement model.

Public authoring takes the same direct UV index as the third argument to
`MaterialBindingAuthoring::try_from_parts`. Convert a wider source index with a checked narrowing
conversion before calling it. Only the optional descriptive source key gives binding authoring its
borrowed lifetime; numeric and closed behavior values are owned.

Whole-Scene space conversion scales supplemental thickness and signed displacement strength by the
positive unit ratio. Other factors and optional groups, normal-map strengths, and UV indices and
transforms keep their exact bits. A changed appearance rebuilds its row tables while sharing the
encoded-image and text pools; destination byte slices and text borrow that same retained payload
owner and remain usable when the source Scene is dropped. If no appearance bits change, the row
owner is shared as well. See [appearance ownership and accounting](scene-ir.md#appearance-storage-and-accounting)
for the private storage boundary and its resource limits.

## Identities and iteration

Get typed IDs from views. Scene lookups check generation and collection range. A primitive ID also
qualifies its logical-mesh owner; `mesh.primitive(id)` rejects another mesh's ID. Related views from
the same borrowed Scene follow their already validated links directly.

Iterators are independent, exact-size, and may be stopped, restarted, or interleaved. Payload slices
borrow the Scene itself, so they outlive a temporary view or iterator. Dropping a view performs no
validation transaction and requires no `finish()` call.

## Metadata and source objects

`scene.metadata_entries()` is an independent exact-size, double-ended, cloneable iterator over
borrowed `MetadataEntryRef` values. `metadata_count()` gives its size. `key()` borrows the retained
namespaced key, and `value()` returns a closed `MetadataValueRef`: scalars, strings, byte payloads
with media type, vectors/colors/matrices, or typed dense arrays. Vector and matrix payloads expose
their validated packed scalar slices; array `len()` counts logical values. Reads allocate no heap
storage, parse no JSON, and preserve the stored scalar bits and byte spelling.

`owner()` distinguishes `MetadataOwnerRef::Entity` from `SourceObject`. An entity token includes the
current Scene generation. Compare it with `metadata_owner()` from the Scene or an entity view to
associate entries without constructing canonical numeric coordinates. A token's `descriptor()`
is descriptive; it is not a reusable cross-generation ID. Iteration places entity owners first,
then source-object owners, with each owner's keys in strictly increasing byte order.

Source metadata belongs to the original source object even if that object has no single canonical
entity. A buffer or animation sampler can contribute to many canonical owners, and a document
need not correspond to the selected Scene. Source metadata is not duplicated for those consumers
and survives edits that remove a derived attribute. Entity metadata follows canonical remaps and
is removed when its actual owner is deleted.

`scene.source_identities()` and `profile_resources()` expose the retained source and resource
inventories. `SourceIdentityRef` follows its sanitized `resource()`, format, optional display name,
locator, and optional content digest. A locator is either typed/indexed or qualified by its
namespaced schema and version. `ProfileResourceRef::stability()` keeps replay evidence separate
from the resource's identity text; a public identity grants no IO access. Source/resource lookup
uses a segment range search in O(log segment count); getters then borrow the resolved rows.
Numeric inventory indices are local to a Scene generation and are not persistence keys.
Equality of `SourceIdentityRef` compares its generation and inventory row. Correlate across edits or
imports through the retained resource, format, and locator fields, keeping digest and replay
stability separate.

`source.metadata_entries()` selects just that source object's entries in key byte order. Two binary
searches over the sorted metadata directory cost O(log metadata count); iterating the resulting
range costs O(1) per entry. An object without extras returns an explicit empty iterator. This reader
borrows the same descriptors and payloads as the whole-Scene inventory, without scanning unrelated
entries or copying the source JSON.

For selected glTF extras, match `SourceObject`, then `MetadataValueRef::Bytes` with media type
`application/json`. The payload includes the exact JSON value, even for an authored `null`, scalar,
or empty container. The site key distinguishes direct object extras from nested asset, material,
texture-transform, accessor, animation-target, and camera sites. See the
[glTF profile](../formats/gltf.md#descriptive-source-extras) for the complete key mapping.

All strings and slices borrow the Scene, so they may outlive a temporary entry or iterator but
cannot outlive the Scene. Copy only the application-owned fields needed after Scene destruction.
The [real-asset consumer](../../tools/consumers/assets/README.md) demonstrates that choice for
source JSON, including its resource and versioned locator.

## Retained provenance relationships

Start from `metadata_owner()` on the current Scene or entity view, including an attribute, binding,
morph delta, or animation channel. The existing `EntityMetadataOwner` token qualifies both
descriptive metadata and retained provenance; the entity need not have descriptive entries.
`scene.owner_provenance(owner)` checks the token's exact generation and borrows its direct
attachments. Each validated canonical owner has at least one attachment. Source objects remain
separate from those canonical owners: one source can feed several entities, and a display name or
numeric source index does not establish a relationship.

Each `ProvenanceRef` has an opaque `id()`, retained longest-path `depth()`, immediate `parents()`, and
`kind()`. Match `ProvenanceKindRef::Source(source)` to follow the actual `SourceIdentityRef`, its
resource and its exact metadata. Match `Operation(operation)` to inspect the borrowed
`ProvenanceOperationKindRef`, exact options digest and ordered `evidence()` summaries. An evidence
summary gives its namespaced code, occurrence count and optional digest. Namespaced operation
kinds, text, digests and metadata payloads borrow their original retained pools.

`scene.provenance_nodes()` exposes the finite retained inventory in topological order;
`provenance_node_count()` gives its size. `scene.provenance(id)` checks generation and range before
resolving one node. Attachments and parents have strictly increasing retained-node order. A parent
always precedes its child; Source leaves have depth one and no parents. A generated Operation can
also have no parents, and any Operation can have no evidence. Empty iterators preserve those facts
without inventing a source or an evidence row. Inventory nodes can remain retained after their last
direct owner attachment changes; a historical node does not prove that a canonical owner still
exists.

All these iterators are independent, exact-size, double-ended and cloneable. No reader allocates,
copies payloads, builds another graph, or automatically follows transitive ancestors. With `S`
retained lineage segments, `O` canonical owners and `M` metadata entries, their costs are:

| Read | Cost |
| --- | --- |
| Counts, resolved node depth/ID, Operation summary fields | O(1) |
| Select an owner's direct attachments | O(log O) |
| Construct a parent or evidence iterator | O(1) |
| Yield an attachment, parent, inventory node or evidence summary | O(log S) per item |
| Resolve a provenance ID, a Source kind or its resource | O(log S) |
| Select a source's metadata, then yield entries | O(log M), then O(1) per entry |

Traversal visits only the relations requested by the caller. Shared ancestors can be visited more
than once along different paths. Choose a visit budget, pending-work capacity and deduplication
policy for the application; depth bounds alone do not bound the number of paths in a DAG.

For example, this external consumer follows the selected entity's ancestry using public facade
types. Its `Vec` is application-owned traversal storage; its allocations are separate from the
allocation-free Scene reads. Returning `false` reports that the caller's visit or pending-work
budget stopped the walk. It deliberately retains repeated shared-ancestor visits.

```rust
use baozi::{
    EntityMetadataOwner, MetadataValueRef, ProvenanceKindRef, ProvenanceLookupError, ValidatedScene,
};

fn print_source_extras(
    scene: &ValidatedScene,
    owner: EntityMetadataOwner,
    max_visits: usize,
    max_pending: usize,
) -> Result<bool, ProvenanceLookupError> {
    let attachments = scene.owner_provenance(owner)?;
    if attachments.len() > max_pending {
        return Ok(false);
    }
    let mut pending: Vec<_> = attachments.rev().collect();
    let mut remaining = max_visits;
    while let Some(node) = pending.pop() {
        if remaining == 0 {
            return Ok(false);
        }
        remaining -= 1;
        match node.kind() {
            ProvenanceKindRef::Source(source) => {
                for entry in source.metadata_entries() {
                    if let MetadataValueRef::Bytes { media_type: "application/json", payload } =
                        entry.value()
                    {
                        println!("{:?} {} {:?}", source.locator(), entry.key(), payload);
                    }
                }
            }
            ProvenanceKindRef::Operation(_) => {
                let parents = node.parents();
                if parents.len() > max_pending - pending.len() {
                    return Ok(false);
                }
                pending.extend(parents.rev());
            }
        }
    }
    Ok(true)
}

// let entity = scene.meshes().next().expect("selected mesh");
// let complete = print_source_extras(scene, entity.metadata_owner(), 256, 256)?;
```

After a successful triangulation, space conversion or normal-generation edit, reacquire the owner
and provenance IDs from the replacement Scene. Tokens from another Scene or any older generation
return `ProvenanceLookupError::GenerationMismatch`, even if the same underlying lineage segments
were shared. Source-object metadata survives deletion of a derived Tangent while the removed
attribute and its entity metadata disappear. Newly retained operation parents describe the
dependencies actually recorded by that edit, which can include the Scene's source declarations.
The source identity fields still permit correlation across generations, with replay stability and
content digests considered separately.

These are the retained owner-level relationships. They do not promise per-Face or per-Corner
ancestry that the format path never recorded, detailed source-scope omission evidence, or
cross-generation remaps. Those reports retain their existing independent contracts. The
[public facade tests](../../crates/baozi/tests/provenance_readers.rs) exercise reordered and shared
source joins, exact extras, generation rejection and actual edit publication without the hidden
integration namespace or diagnostic snapshots.

## Deformation and animation

`scene.skins()` exposes ordered joint palettes, inverse-bind matrices, and explicit matrix-space
contracts. A mesh instance follows its optional skin; joint indices in `primitive.influences()`
address that skin's palette. Influence rows preserve empty rows, source order, and parallel joint,
weight, and source-order slices. Reading does not normalize, sort, or truncate weights.

Logical morph definitions belong to a mesh. Primitive morph targets expose dense or sparse deltas
without expanding them, and instances retain their authored weights. Names and optional payloads
preserve the distinction between absence and authored emptiness. `MorphDeltaSemantic::Tangent`
retains VEC3 delta semantics independently of the base Tangent attribute's construction and
handedness declarations; a morph delta does not own or override those conventions.

`scene.animation_clips()` exposes borrowed names, typed targets, key times, values, and tangents.
See [reading retained animation](reading-animation.md) for time domains and interpolation. These
readers expose retained data; curve evaluation and geometry transformations remain separate work.

The [independent consumer](../../tools/consumers/scene) exercises actual public geometry and hierarchy
values outside workspace feature unification. Its fixed feature cases run with
`python scripts/check_features.py`; `--check` records compile evidence only.
