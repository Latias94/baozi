# Changelog

All notable Baozi changes should be recorded here.

Baozi currently publishes no stable compatibility promise. The project may make breaking API and IR changes while the importer foundation is being built.

## Unreleased

- Added direct, controlled `triangulate_primitive` operations with centrally validated immutable
  output, retained Corner/Face remaps, and the original Scene returned on failure. Triangle lists
  preserve their generation and return `Unchanged`. Removed the empty request compiler and its
  separate coordinator, ledger, and placeholder report types.
- Enabled the selected glTF/GLB profile through `format-gltf` and `all-formats` after its
  malformed-input Linux sanitizer check. The curated default formats remain OBJ/MTL and STL.
- Hardened importer registration, format descriptors, import reports, diagnostics, and postprocess lifecycle boundaries.
- Added OBJ and STL experimental importers with support-matrix checks, fuzz smoke targets, and facade coverage.
- Added core IR extension fields for skins, morph targets, custom vertex attributes, cameras, lights, animation channels, material properties, texture samplers, and texture transforms.
