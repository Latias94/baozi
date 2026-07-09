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
| OBJ/MTL | `baozi-format-obj` | Experimental | Supported | Partial | Partial | Unsupported | Partial | Supported | Static face meshes and MTL texture URI references; see [OBJ details](obj.md) |
| PLY | `baozi-format-ply` | Experimental | Unknown | Unknown | Unsupported | Unsupported | Unsupported | Unknown | Planned parser shell for flexible vertex properties |
| glTF2/GLB | `baozi-format-gltf` | Experimental | Unknown | Unknown | Unknown | Unknown | Unknown | Unknown | Planned parser shell for the first modern PBR scene format |

Promotion to `Stable` requires the gate defined in [ADR 0011](../adr/0011-format-support-tiers-and-compatibility-charter.md).
