# Baozi Scene Import

Baozi turns heterogeneous model assets into one loss-aware scene domain that can be inspected,
validated, and deliberately transformed without inheriting a source format's object model.

## Scene Structure

**Scene**:
The complete imported asset aggregate, including hierarchy, geometry, appearance, deformation,
animation, spatial conventions, and provenance.

**Logical Mesh**:
A named or source-identifiable geometry object that groups one or more primitives and their shared
morph target definitions. _Avoid_: Primitive, geometry buffer

**Primitive**:
A homogeneous draw or topology unit with one topology, optional material binding, index stream,
attribute set, and bounds. _Avoid_: Mesh, face

**Mesh Instance**:
A placement of a logical mesh owned by exactly one scene node, with optional skin and morph-weight
overrides. _Avoid_: Mesh copy, node mesh

**Attribute**:
A typed, named stream whose values apply to one declared attribute domain.

**Attribute Domain**:
The identity set over which an attribute is defined: vertex, corner, face, or mesh instance.
_Avoid_: Attribute rate, channel kind

## Appearance

**Image**:
An owned immutable encoded-image payload, independent of how it is sampled or used. A sanitized
reload hint may accompany it as provenance, but scene usability never depends on a live resolver.
_Avoid_: Texture

**Sampler**:
Filtering and address behavior used when sampling an image.

**Texture**:
The reusable pairing of an image and a sampler. _Avoid_: Image, material slot

**Texture Binding**:
A material use of a texture with a role, color interpretation, coordinates, transform, channels,
and strength. _Avoid_: Texture

## Deformation And Time

**Skin**:
An ordered joint palette and bind-space contract used by mesh instances for skeletal deformation.
_Avoid_: Skeleton

**Morph Target**:
A logical deformation target whose per-primitive deltas can be sparse or dense and whose weight can
come from mesh defaults, instance overrides, or animation. _Avoid_: Animation mesh, shape key

**Animation Clip**:
A collection of time-domain channels that target scene objects and declare interpolation and
out-of-range behavior. _Avoid_: Timeline, action

## Identity And Change

**Dense ID**:
A generation-qualified scene-local handle whose meaning lasts only for one validated scene
generation. Primitive and morph-target IDs are additionally qualified by their owning logical mesh.
_Avoid_: Source ID, persistent ID

**Scene Generation ID**:
An opaque, collision-free token for one immutable scene state. Only a validated result exposes its
token publicly; concurrent or branched edits always receive different tokens even when equal.

**Source Identity**:
A stable, sanitized locator for a source resource or source object across scene generations.
_Avoid_: Dense ID, host path

**Provenance**:
The bounded acyclic lineage of source objects and transformations from which an entity was derived.
_Avoid_: Metadata

**Validated Scene**:
A scene generation accompanied by an unforgeable proof that all canonical invariants hold.

**Scene Edit**:
An isolated change transaction that either yields a new validated scene generation or publishes
nothing.

**Remap**:
The explicit relation between identities or attribute-domain elements before and after a scene
transformation.

**Derived Data**:
Optional data such as bounds, normals, or tangents whose validity is proven against one scene
generation and declared input set. Invalidated values are removed or recomputed before publication.
_Avoid_: Source data
