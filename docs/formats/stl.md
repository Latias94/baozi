# STL Format Support

## Summary

- Format: STL
- Crate: `baozi-format-stl`
- Declaration evidence: Unverified
- Baozi maturity: None (no current `BaoziVerified` evidence record)
- Default feature: `format-stl`
- Parser backend: Baozi-owned hand-written parser
- Supported extensions: `.stl`
- Supported media types: `model/stl`, `application/sla`
- Primary references: STL de facto ASCII and binary encodings; Assimp STL behavior as a clean-room behavior reference only

## Capability Matrix

| Capability | Status | Notes | Tests |
| --- | --- | --- | --- |
| Geometry | Supported | Imports triangle facets as `PrimitiveTopology::Triangles` with duplicated per-facet normals. | `crates/baozi-format-stl/tests/ascii.rs`, `crates/baozi-format-stl/tests/binary.rs` |
| Hierarchy and transforms | Partial | Emits one root node per non-empty ASCII solid and one root node for binary STL. STL has no transforms. | `ascii.rs`, `binary.rs` |
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
| Resource limits | Supported | Enforces primary and decoded UTF-8 bytes, line/token/name bytes, solids, meshes, faces, vertices, diagnostics, and bounded cancellation checkpoints. | `crates/baozi-format-stl/tests/limits.rs` |

Status values: `Supported`, `Partial`, `ParsedLossy`, `IgnoredWithDiagnostic`, `Unsupported`, `Unknown`.

## Import Behavior

- Raw import behavior: binary STL is detected by exact `84 + 50 * facet_count` length; the detector
  recognizes an ASCII candidate only when its probe conforms to the STL text profile and has a
  leading `solid` token after ASCII whitespace. The profile forbids a UTF-8 BOM. Binary detection
  wins when a binary header starts with `solid`.
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
solid part one
  facet normal 0 0 1
    outer loop
      vertex 0 0 0
      vertex 1 0 0
      vertex 0 1 0
    endloop
  endfacet
endsolid part one
```

Baozi supports multiple top-level `solid ... endsolid` blocks. Each non-empty solid becomes one mesh
and one root node, both carrying the solid name. Empty solids emit a warning diagnostic; if every
solid is empty, import fails because there is no renderable STL geometry.

Names are the complete remainder of the `solid` or `endsolid` line after trimming surrounding ASCII
whitespace. Internal spacing is preserved. The opening and closing names must match exactly,
including case and internal whitespace; an unnamed `solid` requires an unnamed `endsolid`. This
matching rule also applies to empty solids before their warning is emitted. Keywords remain
case-insensitive. Canonical scene name validation still rejects embedded control characters
(including tabs within a name) and names longer than 1024 bytes; tabs around a name are accepted.

Blank lines and surrounding spaces or horizontal tabs are accepted. Comments are not recognized:
`#` and `//` in a name are literal name content, and standalone comment lines fail the grammar.
The text profile remains ASCII-only with LF or CRLF newlines; BOMs, bare CR, NUL, and other
forbidden control bytes are rejected before parsing, as are malformed UTF-8 and non-ASCII text.
Facet records retain their strict structure. An open solid or incomplete facet at EOF fails;
the final `endsolid` line does not need a newline. After a complete solid, only whitespace or
another complete solid block is accepted, so trailing garbage fails.

## Resource Accounting and Cancellation

- The complete ASCII STL input is the decoded text surface. Its exact byte length is admitted against
  `max_decoded_utf8_bytes`, materialized into an owned `String`, and retained under a live
  `DecodedText` reservation while the ASCII parser uses it. Allocator slack is reserved separately.
- ASCII STL line starts are retained through the import-owned text-input line index. Line limits count
  content bytes only, so neither LF nor the CRLF terminator reduces the admitted content budget.
- Solid-name words on both opening and closing lines remain individually tokenized and charged
  against token count and per-token byte limits. The complete trimmed opening name is admitted
  against the string-byte limit before copying; closing-name comparison borrows the decoded text.
- Binary STL has no decoded text surface or line index and consumes zero corresponding attempted,
  consumed, or live evidence, including when the binary header begins with `solid`.
- ASCII text-input planning and decoded-text construction each poll operation control for every
  at-most-4-KiB chunk. Owned solid-name and other string copies use UTF-8-boundary-safe chunks with
  the same bound. Matching solid names are compared in at-most-4-KiB chunks with control checks.
- Binary facet parsing checks control at each 50-byte record. The optional facet-color expansion pass
  checks control at least once per 4096 facets.

Exact decoded-text and line-index admission, one-byte-under rejection, binary-zero accounting, CRLF
content limits, multiword name string/token limits, distant profile failure cancellation, long
solid-name copying and comparison, per-facet cancellation, and color-expansion bounds are covered
by `crates/baozi-format-stl/tests/limits.rs`. Name preservation in node/mesh snapshots, mismatches,
unnamed solids, comments, trailing garbage, and EOF behavior are covered by
`crates/baozi-format-stl/tests/ascii.rs`.

## Known Limitations

- STL remains experimental and is not yet promoted under ADR 0011.
- ASCII parsing is intentionally strict about facet structure: every facet must contain exactly
  three vertices.
- The parser does not repair degenerate triangles, unify duplicate vertices, infer units, generate
  missing normals, or normalize coordinates.
- Binary color conventions are de facto and conflicting; Baozi documents the supported mappings but
  does not claim universal STL color compatibility.

## Security Notes

- Resource limits: primary asset bytes, decoded UTF-8 bytes, line length, token length, name length,
  solids, meshes, faces, vertices, diagnostics, and cooperative work intervals are bounded by
  `ResourceLimits` and the parser's fixed checkpoint quanta.
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

## Canonical Binary Color Composition

The experimental binary mappings above use the following explicit canonical composition policy.
`COLOR=` supplies an object fallback RGBA. A facet with attribute bit `0x8000` set overrides that
fallback with its decoded 5-bit RGB and alpha `1`. A facet without that bit uses the object RGBA,
or opaque white when the header has no complete `COLOR=` value. Header bytes are divided by `255`
and facet components by `31`, preserving the existing linear numeric interpretation without a
color-space conversion.

| Input colors | Material base-color factor | Vertex color channel | Alpha mode |
| --- | --- | --- | --- |
| No explicit facet colors, no object color | Opaque white | Absent | `Opaque` |
| Object color with no explicit facet colors | Object RGBA | Absent | `Blend` if object alpha is below `1`; otherwise `Opaque` |
| Explicit colors on some facets | Opaque white | Facet RGBA for explicit colors; object RGBA or opaque white for other facets | `Blend` if a used object fallback has alpha below `1`; otherwise `Opaque` |
| Explicit colors on every facet | Opaque white | Facet RGBA with alpha `1` | `Opaque`, including when the unused object fallback is transparent |

The final RGBA is the component-wise product of the typed material base-color factor and the vertex
color, with an absent vertex channel contributing opaque white. An object fallback appears in
exactly one of those factors. For mixed colors, transparent fallback faces retain their source alpha
while explicitly colored faces remain opaque; both share a `Blend` material. This avoids applying
the object tint twice or squaring its alpha, including when an object component is zero.

This policy follows the material-versus-vertex-color separation in the local Assimp reference at
`repo-ref/assimp/code/AssetLib/STL/STLLoader.cpp`: its binary loader selects the object color as the
material color only when there is no vertex color channel, fills missing facet colors from the
object fallback, and assigns explicit facet colors alpha `1`. Baozi retains its existing channel
order: low/middle/high 5-bit components map to RGB with `COLOR=`, and BGR without it. The existing
set-bit color-valid convention applies in both cases. Other Materialise/Magics validity-bit
conventions are outside this supported dialect; no heuristic switches conventions.

All RGBA combinations in this supported policy are representable using the existing material and
attribute schema, including mixed opacity, so strict `preserve_or_fail` imports need no semantic-loss
authorization. A future source combination that cannot be represented requires a specific
source-semantic-loss diagnostic and caller authorization, or rejection; silently duplicating a tint,
clamping colors, or approximating an unsupported composition does not preserve source semantics.

`crates/baozi-format-stl/tests/color_composition.rs` reads actual typed material factors and dense RGBA
attributes and evaluates their final product for every imported vertex. Fixtures cover uncolored
geometry, object-only colors, mixed colors and fallback, fully colored meshes, nonwhite facet
components in both channel orders, zero components, transparent and opaque object alpha, and
transparent fallbacks overridden on every facet. They use the default strict loss policy and require
no loss evidence.
