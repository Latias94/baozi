# baozi-format-stl

Baozi-owned binary and ASCII STL importer backend. Applications normally use the
[baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md)'s `format-stl` feature, included in
`default-formats`; custom registration uses the experimental importer extension API.

The profile imports triangle facets and authored facet normals. Multiple ASCII
solids become separate meshes and root nodes; binary STL uses one root. Binary
header and facet color mappings are experimental and do not imply universal
compatibility with conflicting STL color conventions.

The importer preserves source coordinates and winding without inferring units,
repairing degenerate triangles, welding vertices, or generating missing normals.
ASCII facets require exactly three vertices. Textures, sidecars, cameras, lights,
skinning, morphs, animation, and archive handling are unsupported.

Declaration evidence is `Unverified`, with no assigned Baozi maturity. See the
[exact STL profile](https://github.com/Latias94/baozi/blob/main/docs/formats/stl.md),
[support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md),
[API tiers](https://github.com/Latias94/baozi/blob/main/docs/adr/0006-public-api-versioning-and-crate-stability-policy.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi-format-stl/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi-format-stl/LICENSE-APACHE), at your option.
