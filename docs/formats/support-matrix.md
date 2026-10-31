# Baozi Format Support Matrix

This matrix is the generated or mechanically checked status view for Baozi's declared support, not
an authority. [ADR 0021](../adr/0021-extension-registry-and-format-descriptor-governance.md) owns
declaration evidence schema and issuer validation;
[ADR 0011](../adr/0011-format-support-tiers-and-compatibility-charter.md) owns capability
vocabulary/statuses, managed Baozi maturity, evidence gates, and the joined support claim.

Presence in this matrix never waives first-party admission safety. A path that fails baseline safety
is disabled, rejected, or isolated and removed from the first-party inventory. Until the managed
evidence inventory is implemented, every declaration is `Unverified` and has no Baozi maturity. A
descriptor cannot promote either value.

| Format | Crate | Declaration evidence | Baozi maturity | Geometry | Hierarchy | Materials | Textures | Cameras/lights | Animation | Skinning | Morph targets | Metadata | Compression/containers | Coordinates/units | Resource limits | Sidecars/archives | Diagnostics | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| STL | `baozi-format-stl` | Unverified | None | Supported | Partial | Partial | Unsupported | Unsupported | Unsupported | Unsupported | Unsupported | Partial | Unsupported | ParsedLossy | Supported | Unsupported | Supported | Binary and ASCII triangle meshes; see [STL details](stl.md) |
| OBJ/MTL | `baozi-format-obj` | Unverified | None | Supported | Partial | Partial | Partial | Unsupported | Unsupported | Unsupported | Unsupported | Partial | Unsupported | ParsedLossy | Supported | Partial | Supported | Static face meshes and MTL texture URI references; see [OBJ details](obj.md) |
| PLY | `baozi-format-ply` | Unverified | None | Supported | Partial | Unsupported | Unsupported | Unsupported | Unsupported | Unsupported | Unsupported | Partial | Unsupported | Unknown | Supported | Unsupported | Supported | ASCII and binary vertex/face geometry with custom scalar vertex attributes; see [PLY details](ply.md) |
| glTF/GLB | `baozi-format-gltf` | Unverified | None | Partial | Supported | Partial | ParsedLossy | Partial | Partial | Partial | Partial | Partial | Unsupported | Supported | Partial | Supported | Supported | Selected-scene PBR, skin, morph, and TRS animation profile; see [exact glTF restrictions](gltf.md) |
