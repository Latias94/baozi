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

Use `positions()`, `normals()`, `texcoords(set)`, or `attribute(AttributeSemanticRef)` to find a
primitive attribute. Lookup is linear in that primitive's attribute count. Iteration retains borrowed
namespaced custom semantics instead of allocating a name for each read.

`AttributeData` distinguishes dense F32/F64/U32/I32 and sparse F32/U32/I32 payloads. Dense `values`
are tightly packed scalars with an explicit lane count. Sparse payloads expose sorted logical element
indices, one explicit element per index, and one default element. `AttributeRef::len()` includes
implicit defaults. Applications must choose how to consume these representations; readers do not
implicitly densify or reduce precision. In particular, F64 custom PLY data remains F64.

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

## Materials and images

`primitive.material()` follows its optional material. A material exposes its authored name, shading
model, alpha behavior, factor groups, and ordered bindings. Legacy diffuse factors retain their own
meaning; they are not reinterpreted as PBR base color by a getter.

A binding describes a material use: texture role, coordinate source, source-channel mapping, color
interpretation, transform, strength, blend operation, and optional source property key. Follow
`binding.texture().image()` to encoded bytes and `texture.sampler()` to sampling state. Binding,
texture, sampler, and image identities remain distinct. Image decoding belongs to the application.

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
