# Changelog

All notable Baozi changes should be recorded here.

Baozi currently publishes no stable compatibility promise. The project may make breaking API and IR changes while the importer foundation is being built.

## Unreleased

- Added allocation-free `AttributeRef::element` reads and `postprocess::triangle_vertices` for
  building application buffers from complete draw-order tuples. Vertex/Corner/Face fields keep
  their original keys, scalar types, sparse defaults, and Scene borrows; influences and morph
  deltas follow the same source vertex. The helper preserves every draw corner without welding
  or allocating a second Scene or owned map. Applications own buffer allocation and packing.
- Extended whole-Scene unit conversion to material thickness and signed displacement strength,
  preserving all other factor bits, optional groups, and binding values. Appearance now has
  independently retained rows and immutable image/text pools, so changing material lengths copies
  no encoded images or text. Unchanged values share both owners. Replaced the undefined generated
  coordinate-source variants with a direct `uv_set: u16` across authoring, typed reads, and format
  adapters; existing UV snapshot bytes remain unchanged at revision 5.
- Removed bootstrap record-block estimates for retired typed-domain layouts. Fresh Scene startup
  and materialization now use the same single-directory shape; typed owners keep independent
  physical admission instead of inflating record-store limits.
- Added complete intrinsic `LocalAxes` to camera projections and light definitions. Camera/light
  space conversion composes the discrete reference frame, scales distances/ranges, and preserves
  hierarchy, attachments, and photometric emission. Replaced frame-free camera kind/tuple APIs
  with typed projection payloads and light scalar readers with `LightDefinition`. Removed redundant
  camera/light authoring wrappers and the separate retained light representation. Snapshot
  encoding is revision 5 and includes all three intrinsic axes. Static plugin builders now accept
  camera/light definitions with automatic document provenance.
- Extended whole-Scene space conversion to skin inverse binds/corrections, dense/sparse morph
  deltas, and TRS animation values and cubic derivatives. One complete borrowed plan binds all
  domains to the exact input root and conversion; unchanged deformation/animation owners stay
  shared. Quaternion signs, zero/nonunit derivatives, source identities, times, and sparse presence
  remain preserved. Numeric failures carry precise locations and return the original Scene.
- Made appearance authoring own compact material and binding behavior values directly. Constructor
  lifetimes now apply only to optional material names and descriptive source keys; small enum and
  swizzle temporaries no longer need caller-owned reference scaffolding.
- Removed xtask's unused `repository-input-candidate` feature and IO dependency. Native dependency
  checks now reject asset/native IO in the tool's production closure and query its featureless graph
  once; CI no longer repeats an identical all-feature xtask test run.
- Added package-local README and dual-license files to all ten production crates, with specific
  descriptions and documentation links. Maintained glTF backend packages now identify their Baozi
  role and preserve upstream authorship and licenses. The artifact-consumer checker verifies those
  files in the extracted archives before building the fixed feature matrix.
- Replaced the final appearance records with one retained typed owner for images, samplers,
  textures, bindings, and materials. Unchanged edits share the complete owner, validation allocates
  no appearance scratch, and public reads borrow actual payloads. Removed duplicate `*Kind` enums,
  the `TextureChannelMapping` wrapper, and producerless appearance `Namespaced` behavior variants;
  channel APIs use `TextureChannelSwizzle` directly. All Scene semantic domains now use typed
  storage; the private directory contains counts only. Semantic snapshot encoding remains revision 4.
  Authoring and central validation check opaque image ranges without redundant content walks;
  unchanged node edits retain constant image-related allocation and validation work.
- Replaced animation record encoding and validation descriptors with one retained typed owner.
  Clips and channels directly borrow f64 key times, values, and cubic tangents; unchanged edits
  share that owner. Unified time-domain, interpolation, and extrapolation declarations across
  authoring, storage, and public reads, removing the duplicate `*Authoring` enums. Finite negative
  pre-roll and existing semantic snapshot bytes remain preserved.
- Replaced deformation records with one retained typed owner for skins, variable influences,
  morph definitions/deltas, and instance overrides. Unchanged edits share that owner, public reads
  borrow its payloads directly, and validation retains only algorithm scratch. Removed redundant
  animation target-context references and proof copies from semantic cursors. Semantic snapshot
  encoding remains revision 4.
- Made the public skin-correction constructor enforce the same finite affine contract as central
  validation. glTF inverse-bind normalization reports malformed matrices at the format boundary;
  affine shear, reflection, singular linear parts, and signed-zero bits remain preserved.
- Added whole-Scene static `convert_scene_space` with validated complete target facts and explicit
  source assumptions. Nodes retain authored Matrix/TRS form; dense/sparse spatial attributes and
  triangle Corner data convert together through the shared Scene-edit transaction. Unsupported
  present semantics return typed locations and preserve the input. Reports own centrally validated
  conversion evidence independently of Scene lifetimes.
- Removed the redundant `TransformConvention` declaration. Column-vector math and local-to-parent
  authored node transforms are fixed API semantics. Scene snapshot schema is now revision 4.
- Fixed a debug Windows stack overflow when importing inside the complete postprocess workflow.
  Activation, central validation, publication, and mutually exclusive terminal rejection paths now
  use separate concrete phases while preserving the same ownership and failure protocol.
- Replaced spatial records and scalar codecs with a retained typed owner for inline `SceneSpace`,
  camera projections, and light definitions. Edits share unchanged spatial storage; central
  validation borrows its actual values and checks placement closure.
- Aligned node and geometry canonical-byte admission with initialized typed headers, rows, and used
  payloads. Matrix/TRS variants share one row footprint, sparse storage stays sparse, and the Scene
  directory is counted independently once.
- Added direct flat/smooth `generate_normals` operations with area-weighted smoothing by vertex
  identity and Face smoothing groups. Authored normals are preserved unless replacement is explicit;
  invalidated tangents and their owner-local metadata require explicit removal. Changed reports own
  validated operation-specific evidence independently of either Scene.
- Made typed attribute lookup require the complete `(semantic, domain)` key. `normals(domain)` and
  `texcoords(set, domain)` distinguish simultaneously authored Vertex and Corner streams without
  selecting one implicitly.
- Replaced generic profile/provenance records with retained typed tables. Edits share immutable
  lineage segments and metadata payloads; central validation borrows their actual storage and
  keeps only provenance depth scratch. Segment-local references derive their global coordinates
  from validated prefixes.
- Counted ordinary retained-handle work in the import operation's `WorkUnits` budget, independently
  of the number of remaining Scene record blocks.
- Added direct, controlled `triangulate_primitive` operations with centrally validated immutable
  output, retained Corner/Face remaps, and the original Scene returned on failure. Triangle lists
  preserve their generation and return `Unchanged`. Removed the empty request compiler and its
  separate coordinator, ledger, and placeholder report types.
- Enabled the selected glTF/GLB profile through `format-gltf` and `all-formats` after its
  malformed-input Linux sanitizer check. The curated default formats remain OBJ/MTL and STL.
- Hardened importer registration, format descriptors, import reports, diagnostics, and postprocess lifecycle boundaries.
- Added OBJ and STL experimental importers with support-matrix checks, fuzz smoke targets, and facade coverage.
- Added core IR extension fields for skins, morph targets, custom vertex attributes, cameras, lights, animation channels, material properties, texture samplers, and texture transforms.
