# PLY Format Support

## Summary

- Format: PLY
- Crate: `baozi-format-ply`
- Maturity: Experimental
- Default feature: not enabled by `default-formats`
- Parser backend: Baozi-owned parser
- Supported extensions: `.ply`
- Supported media types: `model/ply`
- Encoding: ASCII, binary little-endian, binary big-endian
- Sidecar policy: none

## Current Status

`baozi-format-ply` imports common vertex and face geometry into Baozi's owned IR. It is marked
`publish = false` while fixture coverage, fuzzing, and format maturity evidence are still growing.

## Supported MVP

- ASCII, binary little-endian, and binary big-endian PLY.
- Vertex positions from `x`, `y`, `z`.
- Optional normals from `nx`, `ny`, `nz`.
- Optional vertex colors from `red/green/blue/alpha` or `r/g/b/a`.
- Optional texture coordinates from `s/t`, `u/v`, or `texture_u/texture_v`.
- Point clouds when no face element is present.
- Triangle faces and polygon faces from `vertex_indices` or `vertex_index` list properties.
- Unknown scalar vertex properties preserved as `ply:<name>` custom vertex attributes when they map
  to Baozi's current typed attribute model.
- Comments and `obj_info` lines preserved as mesh metadata.
- Resource-limit checks for primary bytes, header line/token/string sizes, vertex count, face count,
  list lengths, mesh count, and diagnostics.

## Known Non-Support

- Materials, textures, cameras, lights, skinning, morph targets, and animation.
- Non-scalar custom properties and non-geometry elements are skipped with diagnostics.
- Coordinate system and unit metadata are not standardized by the format and remain `Unknown`.
- The parser is still Experimental until fuzz coverage and broader fixture evidence land.
