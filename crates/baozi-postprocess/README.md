# baozi-postprocess

Controlled operations on immutable validated Scenes, exposed to applications as
`baozi::postprocess` by the [baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md)'s optional
`postprocess` feature. Scene operations return a Scene and retained operation
evidence on success, or the original Scene on ordinary failure.

Current operations triangulate one polygon primitive, generate flat or smooth
Corner normals on one triangle-list primitive, and convert a whole Scene's
coordinate basis and units. Triangulation requires simple planar rings without
holes and retains Corner/Face origins. Normal generation preserves authored
normals by default.

The allocation-free `triangle_vertices` reader exposes one complete application tuple
per draw Corner after those operations. Vertex, Corner, and Face fields, influences,
and morph deltas follow the same source-vertex relation. It preserves scalar types and
source identity; it does not deduplicate or allocate output buffers. Applications select
their fields and own the concrete buffers and any collected `0..corner_count` indices.

Scene-space conversion covers geometry, node transforms, skin inverse binds and
corrections, dense/sparse morph deltas, and TRS animation values and cubic derivatives.
Camera/light intrinsic axes compose with the target basis; camera distances and light
ranges scale with units while photometric emission stays unchanged.
Material thickness and signed displacement strength also scale with units, sharing
unchanged encoded-image and text payloads.
Missing source conventions require explicit assumptions. UV-origin changes and
tangent generation are not part of this conversion.

See the [coordinate contract](https://github.com/Latias94/baozi/blob/main/docs/model/coordinate-and-render-conventions.md),
[API tiers](https://github.com/Latias94/baozi/blob/main/docs/adr/0006-public-api-versioning-and-crate-stability-policy.md),
[format support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi-postprocess/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi-postprocess/LICENSE-APACHE), at your option.
