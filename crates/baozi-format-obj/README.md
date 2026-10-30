# baozi-format-obj

Baozi-owned Wavefront OBJ/MTL importer backend. Applications normally use the
[baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md)'s `format-obj` feature, included in
`default-formats`; custom importer registries use the experimental extension API.

The profile covers static face meshes, triangles and preserved polygons,
independent Corner UV/normal indices, negative indices, smoothing groups, and
object/group/material segments. MTL support maps a bounded subset of material
fields. MTL sidecars and `map_Kd` diffuse image bytes are acquired through the
session dependency resolver; image payloads remain encoded without pixel decoding.
Byte-only imports deny dependencies. Optional-dependency omissions require explicit
loss-policy permission.

Text is UTF-8 and strict by default; the narrowly gated recovery mode is documented
in the [OBJ/MTL profile](https://github.com/Latias94/baozi/blob/main/docs/formats/obj.md). Points, polylines, free-form
surfaces, cameras, lights, skinning, morphs, animation, and export are outside this
profile. Raw import preserves coordinates and winding without inferring units or
generating normals; triangulation is an explicit postprocess operation.

Declaration evidence is `Unverified`, with no assigned Baozi maturity. See the
[support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md),
[API tiers](https://github.com/Latias94/baozi/blob/main/docs/adr/0006-public-api-versioning-and-crate-stability-policy.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi-format-obj/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi-format-obj/LICENSE-APACHE), at your option.
