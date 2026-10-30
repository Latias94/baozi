# baozi-postprocess

Controlled operations on immutable validated Scenes, exposed to applications as
`baozi::postprocess` by the [baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md)'s optional
`postprocess` feature. This implementation layer returns a Scene and retained
operation evidence on success, or the original Scene on ordinary failure.

Current operations triangulate one polygon primitive, generate flat or smooth
Corner normals on one triangle-list primitive, and convert a whole Scene's
coordinate basis and units. Triangulation requires simple planar rings without
holes and retains Corner/Face origins. Normal generation preserves authored
normals by default. Application vertex unification is not implemented.

Scene-space conversion covers geometry, node transforms, skin inverse binds and
corrections, dense/sparse morph deltas, and TRS animation values and cubic derivatives.
Camera and light conversion remains outside the supported profile.
Missing source conventions require explicit assumptions. UV-origin changes and
tangent generation are not part of this conversion.

See the [coordinate contract](https://github.com/Latias94/baozi/blob/main/docs/model/coordinate-and-render-conventions.md),
[API tiers](https://github.com/Latias94/baozi/blob/main/docs/adr/0006-public-api-versioning-and-crate-stability-policy.md),
[format support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi-postprocess/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi-postprocess/LICENSE-APACHE), at your option.
