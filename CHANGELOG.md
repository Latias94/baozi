# Changelog

All notable Baozi changes should be recorded here.

Baozi currently publishes no stable compatibility promise. The project may make breaking API and IR changes while the importer foundation is being built.

## Unreleased

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
