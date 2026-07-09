# OBJ Format Support

## Summary

- Format: Wavefront OBJ with MTL sidecars
- Crate: `baozi-format-obj`
- Maturity: Experimental
- Default feature: `format-obj`
- Parser backend: Baozi-owned hand-written parser
- Supported extensions: `.obj`
- Supported media types: `model/obj`, `text/plain`
- Primary references: Wavefront OBJ/MTL text conventions; Assimp OBJ/MTL behavior as a clean-room behavior reference only

## Capability Matrix

| Capability | Status | Notes | Tests |
| --- | --- | --- | --- |
| Geometry | Supported | Imports static face meshes from `v`, `vt`, `vn`, and `f`, including OBJ tuple remapping and negative indices. | `crates/baozi-format-obj/tests/geometry.rs` |
| Hierarchy and transforms | Partial | Preserves object/group changes by splitting mesh segments and attaching child nodes. OBJ has no general transform stack. | `geometry.rs`, facade tests |
| Materials | Partial | Loads `newmtl`, `Kd`, `d`, `Tr`, `Ke`, `Ns`, `Ni`, `illum`, `Ka`, and `Ks` where the current IR can express or preserve them. | `materials.rs` |
| Textures and sidecars | Partial | Resolves `mtllib` through `AssetIo`; records `map_Kd` as an external texture URI without decoding image bytes. | `materials.rs`, `crates/baozi/tests/obj_facade.rs` |
| Cameras and lights | Unsupported | OBJ/MTL support does not expose camera or light data. | not applicable |
| Animation | Unsupported | OBJ has no animation model in this importer slice. | not applicable |
| Skinning | Unsupported | OBJ has no skinning model in this importer slice. | not applicable |
| Morph targets | Unsupported | OBJ has no morph target model in this importer slice. | not applicable |
| Metadata | Partial | Stores OBJ/MTL context such as group names, smoothing flags, and MTL-only material values as namespaced metadata. | geometry and material tests |
| Compression or containers | Unsupported | Raw `.obj` and sidecar `.mtl` assets only; archives are outside this crate. | not applicable |
| Coordinate and unit metadata | ParsedLossy | OBJ is imported as-authored. Baozi does not infer units, axis conventions, or winding changes in raw import. | docs/model conventions |
| Malformed input diagnostics | Supported | Structural geometry errors are fatal; recoverable sidecar/material/unsupported-statement issues produce diagnostics. | `malformed.rs`, `limits.rs` |
| Resource limits | Supported | Enforces primary bytes, sidecar bytes, line/token/string bytes, vertices, faces, meshes, and diagnostics caps. | `limits.rs` |

Status values: `Supported`, `Partial`, `ParsedLossy`, `IgnoredWithDiagnostic`, `Unsupported`, `Unknown`.

## Import Behavior

- Detection: `.obj` extension gives a positive signal, and lightweight content probes recognize common `v`, `vt`, `vn`, `f`, `o`, `g`, `s`, `mtllib`, and `usemtl` records.
- Text encoding: this experimental slice expects UTF-8. UTF-8 BOM and CRLF line endings are accepted.
- Raw coordinate behavior: positions, normals, and texture coordinates are imported unchanged.
- Unit handling: OBJ is treated as unitless; Baozi does not infer meters, centimeters, or inches.
- Winding handling: face order is preserved as-authored.
- Topology handling: all-triangle mesh segments use `PrimitiveTopology::Triangles`; quads and N-gons use `PrimitiveTopology::Polygons` with `face_vertex_counts`.
- Post-process boundary: triangulation is performed by `baozi-postprocess`, not by raw OBJ parsing.
- Attribute remapping: OBJ's separate position/UV/normal indices are remapped into Baozi's vertex-indexed SoA mesh streams by unique `(position, texcoord, normal)` tuples.
- Mesh splitting: material, object/group, and topology boundaries may split one OBJ file into multiple Baozi meshes to preserve the one-material-per-mesh contract.

## MTL Mapping

`mtllib` paths resolve relative to the OBJ asset through `AssetIo`. Texture paths inside MTL resolve relative to the MTL asset. `Importer::read_bytes` uses memory IO and denies external references by default; use `read_asset` or `read_path` with an explicit IO policy when sidecars should load.

| MTL field | Baozi mapping |
| --- | --- |
| `newmtl` | `Material.name` and material lookup key |
| `Kd` | `Material.base_color` RGB |
| `d` | `Material.base_color.a`; alpha below 1 sets blend mode |
| `Tr` | inverse alpha; alpha below 1 sets blend mode |
| `Ke` | `Material.emissive` |
| `Ns`, `Ni`, `illum`, `Ka`, `Ks` | namespaced `Material.metadata` entries |
| `map_Kd` | `TextureSource::External` URI plus diffuse/base-color material texture slot |

MTL remains optional enrichment. Missing, denied, malformed, or oversized sidecars emit diagnostics while valid OBJ geometry continues when safe. Unknown `usemtl` references create a placeholder material with a warning so the mesh can keep a stable material binding.

## Known Limitations

- OBJ points, polylines, free-form curves, surfaces, parameter spaces, bevel/interp directives, and exporter behavior are not implemented in this slice.
- The importer does not repair degenerate faces, generate normals, normalize coordinates, infer units, flip winding, decode image files, or fetch remote assets.
- `map_Kd` option parsing is intentionally minimal: common option arities are skipped to find the texture path, but option semantics are not applied.
- MTL PBR extensions are preserved through namespaced material metadata/properties where possible, but are not promoted to first-class PBR fields yet.
- OBJ and MTL input must be UTF-8 for this experimental support tier.

## Security Notes

- Resource limits: primary asset bytes, sidecar bytes, line length, token length, string length, vertices, faces, meshes, and diagnostics are bounded by `ResourceLimits`.
- Path constraints: sidecars and textures resolve through `AssetIo` and `AssetPath`; the format crate does not call filesystem APIs directly.
- External reference policy: `ExternalReferencePolicy::Deny` prevents MTL sidecar reads and records a warning.
- Archive constraints: raw OBJ/MTL only; archive handling is outside this crate.
- Fuzz targets: `fuzz/fuzz_targets/obj_import.rs`.
- Malformed fixtures: `crates/baozi-format-obj/tests/malformed.rs`, `limits.rs`, and parser integration tests.

## Compatibility Notes

- Assimp comparison: Baozi uses Assimp only as a behavior checklist for common OBJ/MTL edge cases such as separate indices, negative indices, optional MTL files, and material changes.
- Clean-room boundary: no Assimp source code, comments, macros, fixture files, or test assets are copied into Baozi.
- Known Baozi deviations: Baozi validates scenes before returning them, exposes Baozi-owned diagnostics, preserves polygons during raw import, and keeps triangulation in the post-process pipeline.

## Dependencies and Licenses

| Dependency | Role | License | Notes |
| --- | --- | --- | --- |
| none | parser backend | not applicable | OBJ/MTL parser is Baozi-owned and hand-written |

## Fixtures

| Fixture | Purpose | License | Snapshot |
| --- | --- | --- | --- |
| hand-authored in tests | triangles, quads, N-gons, separate indices, materials, sidecars, malformed input, limits | Baozi project license | direct assertions and `SceneSnapshot` support |
| `fuzz/corpus/obj_import/*` | seed inputs for sanitizer fuzzing | Baozi project license | fuzz target corpus |
