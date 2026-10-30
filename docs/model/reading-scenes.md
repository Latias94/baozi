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

## Application triangle buffers

Use `baozi::postprocess::triangle_vertices(primitive)` after any explicit triangulation, normal
generation, and space conversion that the application needs. It returns
`Result<TriangleVertices<'scene>, VertexLayoutError>` and accepts only `TriangleList` topology.
Other topologies, even when empty, return `VertexLayoutError::UnsupportedTopology { topology }`.
An empty TriangleList gives an empty layout. No transformation runs inside this helper.

The layout and its tuple views are `Copy` and borrow the current primitive:

| Layout method | Result |
| --- | --- |
| `primitive()` | The current borrowed `PrimitiveRef` |
| `len()`, `is_empty()` | Application tuple count and emptiness; count equals source Corner count |
| `vertex(index: usize)` | `Option<TriangleVertex>` with an out-of-range check |
| `vertices()` | An independent exact-size, double-ended, cloneable iterator |
| `indices()` | An allocation-free `Range<u32>` from zero to source Corner count |

There is exactly one application tuple per draw Corner, including repeated source vertices. For
tuple index `c`, Vertex fields sample the source index at `c` or implicit Vertex `c`, Corner fields
sample `c`, and Face fields sample `c / 3`. Triangle order, winding, UV seams, Flat/Smooth normal
values, and Face boundaries therefore follow the already processed primitive. Equal tuples are
allowed to repeat: the helper performs no deduplication, welding, minimum-count optimization, or
precision conversion. Unreferenced source vertices remain in the Scene and are not appended to
the draw layout.

`vertex.index()` is that application/Corner ordinal. `source_vertex()` returns
`PrimitiveVertex { primitive: PrimitiveId, index: u32 }`; `source_corner()` and `source_face()` return
the corresponding qualified endpoints. `vertex.primitive()` follows the exact current primitive.
These sources refer to the current Scene; earlier edit reports own any relation to a prior Scene.

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

Node camera/light attachments follow to scene-owned definitions. `scene.space()` preserves unknown
axes, units, winding, UV origin, and tangent conventions. Reading never assigns missing units or
performs an implicit coordinate conversion.

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

## Deformation and animation

`scene.skins()` exposes ordered joint palettes, inverse-bind matrices, and explicit matrix-space
contracts. A mesh instance follows its optional skin; joint indices in `primitive.influences()`
address that skin's palette. Influence rows preserve empty rows, source order, and parallel joint,
weight, and source-order slices. Reading does not normalize, sort, or truncate weights.

Logical morph definitions belong to a mesh. Primitive morph targets expose dense or sparse deltas
without expanding them, and instances retain their authored weights. Names and optional payloads
preserve the distinction between absence and authored emptiness.

`scene.animation_clips()` exposes borrowed names, typed targets, key times, values, and tangents.
See [reading retained animation](reading-animation.md) for time domains and interpolation. These
readers expose retained data; curve evaluation and geometry transformations remain separate work.

The [independent consumer](../../tools/consumers/scene) exercises actual public geometry and hierarchy
values outside workspace feature unification. Its fixed feature cases run with
`python scripts/check_features.py`; `--check` records compile evidence only.
