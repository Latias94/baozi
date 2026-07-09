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

| Format | Crate | Maturity | Geometry | Materials | Textures | Animation | Skinning | Cameras/lights | Morph targets | Resource limits | Sidecars/archives | Diagnostics | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| STL | `baozi-format-stl` | Experimental | Supported | Partial | Unsupported | Unsupported | Unsupported | Unsupported | Unsupported | Supported | Unsupported | Supported | Binary and ASCII triangle meshes; see [STL details](stl.md) |
| OBJ/MTL | `baozi-format-obj` | Experimental | Supported | Partial | Partial | Unsupported | Unsupported | Unsupported | Unsupported | Supported | Partial | Supported | Static face meshes and MTL texture URI references; see [OBJ details](obj.md) |
| PLY | `baozi-format-ply` | Experimental | Unknown | Unknown | Unsupported | Unsupported | Unsupported | Unsupported | Unsupported | Unknown | Unsupported | Unknown | Planned parser shell for flexible vertex properties |
| glTF2/GLB | `baozi-format-gltf` | Experimental | Supported | Partial | Partial | IgnoredWithDiagnostic | Partial | Partial | IgnoredWithDiagnostic | Supported | Partial | Supported | Mesh/material/camera/skin MVP for `.gltf` external buffers, base64 buffer data URIs, and GLB BIN payloads; morph targets and animation are deferred |

Promotion to `Stable` requires the gate defined in [ADR 0011](../adr/0011-format-support-tiers-and-compatibility-charter.md).
