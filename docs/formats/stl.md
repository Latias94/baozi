# STL Format Support

## Summary

- Format: STL
- Crate: `baozi-format-stl`
- Maturity: Experimental
- Default feature: `format-stl`
- Parser backend: Baozi-owned hand-written parser
- Supported extensions: `.stl`
- Supported media types: none declared by this crate
- Primary references: STL de facto ASCII and binary encodings; Assimp STL behavior as a clean-room behavior reference only

## Capability Matrix

| Capability | Status | Notes | Tests |
| --- | --- | --- | --- |
| Geometry | Supported | Imports triangle facets as `PrimitiveTopology::Triangles` with duplicated per-facet normals. | `crates/baozi-format-stl/tests/ascii.rs`, `crates/baozi-format-stl/tests/binary.rs` |
| Hierarchy and transforms | Partial | Emits one child node per non-empty ASCII solid and one child node for binary STL. STL has no transforms. | `ascii.rs`, `binary.rs` |
| Materials | Partial | Emits one default material. Binary Materialise `COLOR=` header maps to material base color. | `binary.rs` |
| Textures and sidecars | Unsupported | STL import does not resolve sidecars or textures. | facade and parser tests |
| Cameras and lights | Unsupported | STL has no camera or light data. | not applicable |
| Animation | Unsupported | STL has no animation data. | not applicable |
| Skinning | Unsupported | STL has no skinning data. | not applicable |
| Morph targets | Unsupported | STL has no morph target data. | not applicable |
| Metadata | Partial | Adds `stl.storage` and `stl.source` mesh metadata. | snapshot tests |
| Compression or containers | Unsupported | Raw `.stl` bytes only; archive/container handling is outside this crate. | not applicable |
| Coordinate and unit metadata | ParsedLossy | STL is unitless and has no reliable up/front axis. Baozi preserves coordinates as-authored. | docs/model tests by convention |
| Malformed input diagnostics | Supported | Parse errors include line/column or byte offsets where available; empty ASCII solids emit diagnostics. | malformed, ASCII, binary tests |
| Resource limits | Supported | Enforces primary bytes, line/token/name bytes, solids, meshes, faces, vertices, and diagnostics caps. | ASCII/binary limit tests |

Status values: `Supported`, `Partial`, `ParsedLossy`, `IgnoredWithDiagnostic`, `Unsupported`, `Unknown`.

## Import Behavior

- Raw import behavior: binary STL is detected by exact `84 + 50 * facet_count` length; ASCII STL is detected by a leading `solid` token after optional UTF-8 BOM and whitespace. Binary detection wins when a binary header starts with `solid`.
- Default coordinate behavior: coordinates are imported unchanged.
- Unit handling: STL is unitless; Baozi does not infer meters, millimeters, or inches.
- UV origin handling: STL has no UV data.
- Winding handling: Baozi preserves facet vertex order as-authored.
- Material mapping: all STL meshes reference one default material. Binary `COLOR=` header default RGBA maps to `Material.base_color`.
- Texture color-space rules: not applicable.
- Sidecar resolution: STL does not open sidecars; bytes imports remain self-contained.

## Binary STL

Binary STL is parsed as:

- 80-byte header
- 4-byte little-endian `u32` facet count
- one 50-byte record per facet

Each facet record contains a normal, three vertices, and a 16-bit attribute field. Baozi expands each
facet into three vertices and three indices.

Color handling is experimental:

- a `COLOR=` token in the 80-byte header maps four following bytes to default material RGBA
- if facet attribute bit `0x8000` is set, Baozi expands a 15-bit facet color into `Mesh.colors[0]`
- with `COLOR=` present, facet colors use Materialise ordering
- without `COLOR=`, facet colors use Magics/VisCAM-style ordering

## ASCII STL

ASCII STL is the text encoding of the same triangle-facet model. Baozi parses this shape:

```stl
solid name
  facet normal 0 0 1
    outer loop
      vertex 0 0 0
      vertex 1 0 0
      vertex 0 1 0
    endloop
  endfacet
endsolid name
```

Baozi supports multiple top-level `solid ... endsolid` blocks. Each non-empty solid becomes one mesh
and one child node. Empty solids emit a warning diagnostic; if every solid is empty, import fails
because there is no renderable STL geometry.

## Known Limitations

- STL remains experimental and is not yet promoted under ADR 0011.
- ASCII parsing is intentionally strict about facet structure: every facet must contain exactly
  three vertices.
- The parser does not repair degenerate triangles, unify duplicate vertices, infer units, generate
  missing normals, or normalize coordinates.
- Binary color conventions are de facto and conflicting; Baozi documents the supported mappings but
  does not claim universal STL color compatibility.

## Security Notes

- Resource limits: primary asset bytes, line length, token length, name length, solids, meshes,
  faces, vertices, and diagnostics are bounded by `ResourceLimits`.
- Path and sidecar constraints: STL does not open external resources. `Importer::read_bytes` uses
  memory IO and denies external references by default.
- Archive constraints: raw STL only; archive handling is outside this crate.
- Fuzz targets: `fuzz/fuzz_targets/stl_import.rs`.
- Malformed fixtures: `crates/baozi-format-stl/tests/malformed.rs` and parser integration tests.

## Compatibility Notes

- Assimp comparison: Baozi follows the important behavior lesson that binary STL can have a header
  beginning with `solid`, so exact binary length detection must win over ASCII-prefix detection.
- Known tolerated differences: Baozi emits Baozi-owned diagnostics and scene IR rather than Assimp
  types, flags, materials, or metadata names.
- Known Baozi deviations: Baozi validates scenes before returning them and fails non-finite floats
  through scene validation.

## Dependencies and Licenses

| Dependency | Role | License | Notes |
| --- | --- | --- | --- |
| none | parser backend | not applicable | STL parser is Baozi-owned and hand-written |

## Fixtures

| Fixture | Purpose | License | Snapshot |
| --- | --- | --- | --- |
| hand-authored in tests | binary/ASCII triangles, colors, malformed input, limits | Baozi project license | `SceneSnapshot` assertions |
