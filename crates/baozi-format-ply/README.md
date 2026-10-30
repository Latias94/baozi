# baozi-format-ply

Baozi-owned PLY importer backend for source builds. Applications normally select
the [baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md)'s `format-ply` feature, also included in
`all-formats` and outside `default-formats`. Backend registration uses the
experimental importer extension API.

The PLY 1.0 profile covers ASCII and binary little- and big-endian vertex/face
geometry: positions, optional normals, common vertex colors and UVs, point clouds,
triangles, and polygons. Supported custom scalar vertex properties retain typed
attributes, including `f64` values; comments and `obj_info` become mesh metadata.

Materials, textures, sidecars, cameras, lights, skinning, morphs, and animation are
unsupported. Non-scalar custom properties and non-geometry elements are skipped
with diagnostics. Coordinates and units remain `Unknown`; text framing is strict
ASCII with no BOM, LF/CRLF newlines, and horizontal tabs allowed.

Declaration evidence is `Unverified`, with no assigned Baozi maturity. See the
[exact PLY profile](https://github.com/Latias94/baozi/blob/main/docs/formats/ply.md),
[support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md),
[API tiers](https://github.com/Latias94/baozi/blob/main/docs/adr/0006-public-api-versioning-and-crate-stability-policy.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi-format-ply/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi-format-ply/LICENSE-APACHE), at your option.
