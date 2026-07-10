# Baozi Format Support Matrix

This matrix is the generated or mechanically checked status view for Baozi's declared support, not
the authority for maturity or capability vocabulary. The meanings and gates for every displayed
value are defined solely by
[ADR 0011](../adr/0011-format-support-tiers-and-compatibility-charter.md).

Presence in this matrix never waives first-party admission safety. A path that fails baseline safety
is disabled, rejected, or isolated and removed from the first-party inventory rather than retained as
Experimental. Missing non-safety maturity evidence may keep an admitted row Experimental.

| Format | Crate | Maturity | Geometry | Hierarchy | Materials | Textures | Cameras/lights | Animation | Skinning | Morph targets | Metadata | Compression/containers | Coordinates/units | Resource limits | Sidecars/archives | Diagnostics | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| STL | `baozi-format-stl` | Experimental | Supported | Partial | Partial | Unsupported | Unsupported | Unsupported | Unsupported | Unsupported | Partial | Unsupported | ParsedLossy | Supported | Unsupported | Supported | Binary and ASCII triangle meshes; see [STL details](stl.md) |
| OBJ/MTL | `baozi-format-obj` | Experimental | Supported | Partial | Partial | Partial | Unsupported | Unsupported | Unsupported | Unsupported | Partial | Unsupported | ParsedLossy | Supported | Partial | Supported | Static face meshes and MTL texture URI references; see [OBJ details](obj.md) |
| PLY | `baozi-format-ply` | Experimental | Supported | Partial | Unsupported | Unsupported | Unsupported | Unsupported | Unsupported | Unsupported | Partial | Unsupported | Unknown | Supported | Unsupported | Supported | ASCII and binary vertex/face geometry with custom scalar vertex attributes; see [PLY details](ply.md) |
