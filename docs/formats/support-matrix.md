# Baozi Format Support Matrix

This matrix tracks Baozi's declared support, not the full theoretical feature set of each format.

Status values:

- `Experimental`: selected fixtures only; behavior may change.
- `Beta`: common assets work; known gaps documented.
- `Stable`: supported subset is documented, tested, fuzzed, and release-gated.
- `Deprecated`: present but not recommended.

Capability values:

- `Supported`
- `Partial`
- `ParsedLossy`
- `IgnoredWithDiagnostic`
- `Unsupported`
- `Unknown`

| Format | Crate | Maturity | Geometry | Materials | Textures | Animation | Sidecars/archives | Diagnostics | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| STL | `baozi-format-stl` | Experimental | Supported | Partial | Unsupported | Unsupported | Unsupported | Supported | Binary and ASCII triangle meshes; see [STL details](stl.md) |
| OBJ/MTL | `baozi-format-obj` | Planned | Unknown | Unknown | Unknown | Unsupported | Unknown | Unknown | First text and sidecar format |
| PLY | `baozi-format-ply` | Planned | Unknown | Partial | Unsupported | Unsupported | Unsupported | Unknown | Exercises flexible vertex properties |
| glTF2/GLB | `baozi-format-gltf` | Planned | Unknown | Unknown | Unknown | Unknown | Unknown | Unknown | First modern PBR scene format |

Promotion to `Stable` requires the gate defined in [ADR 0011](../adr/0011-format-support-tiers-and-compatibility-charter.md).
