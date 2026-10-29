# OBJ Format Support

## Summary

- Format: Wavefront OBJ with MTL sidecars
- Crate: `baozi-format-obj`
- Declaration evidence: Unverified
- Baozi maturity: None (no current `BaoziVerified` evidence record)
- Default feature: `format-obj`
- Parser backend: Baozi-owned hand-written parser
- Supported extensions: `.obj`
- Supported media types: `model/obj`, `text/plain`
- Primary references: Wavefront OBJ/MTL text conventions; Assimp OBJ/MTL behavior as a clean-room behavior reference only

## Capability Matrix

| Capability | Status | Notes | Tests |
| --- | --- | --- | --- |
| Geometry | Supported | Imports static face meshes from `v`, `vt`, `vn`, and `f`, preserving independent Corner-domain UV/normal indices and negative indices without duplicating shared positions. | `crates/baozi-format-obj/tests/geometry.rs` |
| Hierarchy and transforms | Partial | Preserves object/group changes by splitting mesh segments and attaching child nodes. OBJ has no general transform stack. | `geometry.rs`, facade tests |
| Materials | Partial | Loads `newmtl`, `Kd`, `d`, `Tr`, `Ke`, `Ns`, `Ni`, `illum`, `Ka`, and `Ks` where the current IR can express or preserve them. | `materials.rs` |
| Textures and sidecars | Partial | Resolves `mtllib` through `AssetIo`; records `map_Kd` as an external texture URI without decoding image bytes. | `materials.rs`, `crates/baozi/tests/obj_facade.rs` |
| Cameras and lights | Unsupported | OBJ/MTL support does not expose camera or light data. | not applicable |
| Animation | Unsupported | OBJ has no animation model in this importer slice. | not applicable |
| Skinning | Unsupported | OBJ has no skinning model in this importer slice. | not applicable |
| Morph targets | Unsupported | OBJ has no morph target model in this importer slice. | not applicable |
| Metadata | Partial | Stores descriptive OBJ/MTL context such as object/group names and MTL-only material values as namespaced metadata. Smoothing groups are typed Face-domain attributes rather than metadata. | geometry and material tests |
| Compression or containers | Unsupported | Raw `.obj` and sidecar `.mtl` assets only; archives are outside this crate. | not applicable |
| Coordinate and unit metadata | ParsedLossy | OBJ is imported as-authored. Baozi does not infer units, axis conventions, or winding changes in raw import. | docs/model conventions |
| Malformed input diagnostics | Supported | Structural geometry and strict text failures are fatal. Declared format warnings and explicitly authorized optional-dependency unavailability produce bounded diagnostics. | `malformed.rs`, `limits.rs`, `text_recovery.rs` |
| Resource limits | Supported | Enforces primary and dependency bytes, decoded/index/token/string bytes, vertices, faces, meshes, diagnostics, recovery attempts, and loss-evidence caps. | `limits.rs`, `text_recovery.rs` |

Status values: `Supported`, `Partial`, `ParsedLossy`, `IgnoredWithDiagnostic`, `Unsupported`, `Unknown`.

## Import Behavior

- Detection: `.obj` extension gives a positive signal, and lightweight content probes recognize common `v`, `vt`, `vn`, `f`, `o`, `g`, `s`, `mtllib`, and `usemtl` records.
- Text encoding: OBJ and MTL use UTF-8. `ObjTextDecodingMode::RejectInvalid` is the default; UTF-8 BOM and CRLF line endings are accepted.
- Raw coordinate behavior: Cartesian positions, normals, and texture coordinates retain their source values; four-component homogeneous positions are converted to Cartesian XYZ by dividing by their weight, as detailed below.
- Unit handling: OBJ is treated as unitless; Baozi does not infer meters, centimeters, or inches.
- Winding handling: face order is preserved as-authored.
- Topology handling: all-triangle mesh segments use `PrimitiveTopology::Triangles`; quads and N-gons use `PrimitiveTopology::Polygons` with `face_vertex_counts`.
- Smoothing handling: `s off` and `s 0` map to Face-domain smoothing group `0`; positive decimal group identifiers are preserved as dense Face-domain `u32` values. A segment with no face governed by an `s` statement has no smoothing attribute. Once an explicit smoothing state reaches a face, earlier unspecified faces in that segment are represented as `0` so the attribute remains dense, and the state continues across object, group, and material segment boundaries until another `s` statement changes it. Smoothing changes do not split a mesh segment by themselves.
- Post-process boundary: raw OBJ parsing preserves polygon topology. Canonical triangulation belongs
  in `baozi-postprocess`, but no transformation request is currently admitted while the retained
  edit, remap, and evidence rail is being completed.
- Attribute projection: positions are compacted per mesh segment by source position index, while UV and normal source tables are projected in face-corner order as Corner-domain attributes. UV or normal seams therefore remain distinguishable without duplicating shared positions. A segment that mixes present and missing values for one channel is rejected under `LossPolicy::preserve_or_fail()`. The existing sparse attribute representation supplies default values rather than preserving missing corner values, so partial channels require explicit loss authorization instead of synthesized defaults.
- Partial attribute loss: `LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss)` permits omitting each incomplete UV or normal channel while retaining positions, topology, and any complete channel. Each omitted channel records `SourceSemanticLoss` evidence with disposition `Applied` and its own code, `obj.partial_texcoord_channel` or `obj.partial_normal_channel`. Without authorization, the first incomplete channel records `RejectedByLossPolicy` evidence and terminates import. Diagnostics identify the affected channel at `/segments/{index}/attributes/texcoord` or `/segments/{index}/attributes/normal`; the typed loss subject is the source mesh. These are source-semantic omissions, independent of text recovery and conformance policy.
- Mesh splitting: material and object/group boundaries may split one OBJ file into multiple Baozi meshes. Polygon topology is preserved within each segment, and smoothing remains per-face data.

### Numeric Record Fidelity

Numeric record arity is checked before optional components are interpreted. This prevents a
vertex-color component from being treated as a homogeneous weight and prevents trailing UV or
normal values from being silently ignored.

| Record shape | Behavior |
| --- | --- |
| `v x y z` | Retains three Cartesian position components. |
| `v x y z w` | Retains `(x / w, y / w, z / w)`. Every source component must be finite, `w` must be nonzero, and the Cartesian result must be finite. Negative weights are accepted; zero, negative zero, invalid numeric tokens, and overflowing results are terminal format errors. |
| Six- or seven-component `v` | Recognized as an unsupported vertex-color extension shape. Every numeric component is validated. The default `LossPolicy::preserve_or_fail()` rejects omission through `SourceSemanticLoss`. Explicit authorization preserves the first three XYZ values without dividing by any suffix component and omits the entire suffix. Seven-component suffixes are not guessed to mean either RGBA or a weight plus RGB. |
| `vt u v` | Retains a two-lane UV attribute when every UV record referenced by the segment has two components. |
| `vt u v w` | Retains all three values in a three-lane UV attribute. A segment referencing both two- and three-component UV records uses three lanes, with the omitted third component of two-component records defaulting to zero. Width is selected per segment from its actual references, including an explicitly authored third component of zero; unrelated or unreferenced three-component UV records do not widen a two-lane segment. |
| `vn x y z` | Retains three normal components. |
| Other numeric record arities | Rejects with `obj.invalid_statement` and the reason `missing-components` or `unsupported-component-count`. One-component texture coordinates and additional record dialects are outside this bounded profile. |

Permitting `ImportLossKind::SourceSemanticLoss` records one `obj.vertex_color_omitted` loss and
warning per affected vertex record, with disposition `Applied`. Without that permission, the first
such record retains `RejectedByLossPolicy` evidence and terminates import before canonical output
is committed. Evidence identifies the primary source and the source mesh category. The diagnostic
object path `/records/{line}/vertex-color` and its `line`, `column`, `component_count`, and
`discarded_components` details identify the record and omission. Allowing `FormatRecovery` alone
does not authorize this loss, and semantic-loss permission never accepts malformed or non-finite
numeric values. Homogeneous conversion and complete UV preservation emit no loss evidence.

The hand-authored fixtures in `crates/baozi-format-obj/tests/record_fidelity.rs` assert typed
position and UV payloads, source/canonical vertex counts, segment widths, topology, and loss
evidence through the public Scene readers, without parsing snapshots.

### Text Decoding and Recovery

Invalid UTF-8 is rejected by default. A caller that needs declared legacy-text recovery must submit the OBJ-owned typed option `ObjImportOptions` with `ObjTextDecodingMode::ReplaceInvalidUtf8`. Recovery is not a global permissive-decoding switch: replacement is applied per Unicode maximal subpart and succeeds only when all four authorities allow it:

1. The typed OBJ option selects `ReplaceInvalidUtf8`.
2. The OBJ format descriptor advertises that decoding mode in its `TextProfile`.
3. The request conformance policy allows declared recovery, for example `ConformancePolicy::AllowDeclaredRecovery`.
4. The loss policy allows `ImportLossKind::FormatRecovery`.

Even with all four gates enabled, replacement is limited to these textual value ranges:

- OBJ values following `o`, `g`, `usemtl`, and `mtllib`.
- MTL values following `newmtl`, and the `map_Kd` path after its option operands have valid arity and required numeric syntax.

OBJ/MTL keywords, numeric fields, smoothing values, face references, `map_Kd` option operands, and all other required syntax remain strict. Invalid UTF-8 in those regions is terminal rather than replaced. When replacement is applied, the import report records a bounded `obj.invalid_utf8_replaced` diagnostic and `FormatRecovery` loss evidence with the original raw-byte range. Recovery count/raw bytes and loss-evidence count/bytes are charged through the import ledger; exhausting any applicable limit is terminal. In replacement mode, invalid bytes wholly inside a discarded `#` comment tail are ignored without a diagnostic, loss evidence, or recovery-ledger charge. In the default `RejectInvalid` mode, invalid UTF-8 is rejected before comment-tail recovery filtering.

## MTL Mapping

`mtllib` paths resolve relative to the OBJ asset through `AssetIo`. Texture paths inside MTL resolve relative to the MTL asset. Resolver behavior depends on the facade entry point:

- `Importer::read_bytes` admits only the caller-owned primary byte snapshot and uses `baozi_import::DenyDependencies`; it cannot load MTL sidecars.
- `Importer::read_asset` uses Baozi's registered same-authority resolver profile, so MTL sidecars are authorized against the same `AssetIo` authority as the primary OBJ and receive registered manifest identity evidence.
- `Importer::read_path` is available only with `native-fs-candidate`. It opens a native-directory capability rooted at the OBJ file's parent directory, then uses the same-authority behavior of `read_asset` for sidecars.
- `Importer::read_asset_with_resolver` accepts a prepared request and an explicit `DependencyResolver`. Use it when dependencies need a caller-defined authority policy, including `baozi_import::DenyDependencies` when every sidecar must be rejected before backend IO.

| MTL field | Baozi mapping |
| --- | --- |
| `newmtl` | `Material.name` and material lookup key |
| `Kd` | `Material.base_color` RGB |
| `d` | `Material.base_color.a`; alpha below 1 sets blend mode |
| `Tr` | inverse alpha; alpha below 1 sets blend mode |
| `Ke` | `Material.emissive` |
| `Ns`, `Ni`, `illum`, `Ka`, `Ks` | namespaced `Material.metadata` entries |
| `map_Kd` | `TextureSource::External` URI plus diffuse/base-color material texture slot |

MTL is optional enrichment only at the dependency-acquisition boundary. A missing, policy-denied, unreadable, or acquisition-budget-exhausted MTL may be represented as unavailable, emit structured optional-dependency diagnostics, and preserve valid OBJ geometry only when the request loss policy explicitly allows `ImportLossKind::OptionalDependencyUnavailable`. Without that authorization, the dependency failure is terminal.

Once an MTL snapshot is available, it is parsed under the same text, resource, and operation controls as the primary OBJ. Any failure returned while decoding or parsing that snapshot is terminal and cannot be reclassified as optional unavailability. In particular, invalid UTF-8 is terminal in `RejectInvalid` mode and invalid UTF-8 outside the narrowly recoverable `newmtl` and `map_Kd` path values is terminal in every mode. Statements that the MTL parser explicitly classifies as warnings may continue, but malformed sidecars do not receive a blanket optional-dependency waiver. Unknown `usemtl` references create a placeholder material with a warning so the mesh can keep a stable material binding.

## Known Limitations

- OBJ points, polylines, free-form curves, surfaces, parameter spaces, bevel/interp directives, and exporter behavior are not implemented in this slice.
- The importer does not repair degenerate faces, generate normals, normalize coordinates, infer units, flip winding, decode image files, or fetch remote assets.
- `map_Kd` option parsing is intentionally minimal: common option arities are skipped to find the texture path, but option semantics are not applied.
- MTL PBR extensions are preserved through namespaced material metadata/properties where possible, but are not promoted to first-class PBR fields yet.
- OBJ/MTL syntax is UTF-8 and strict by default. The only encoding recovery is the explicitly gated `ReplaceInvalidUtf8` behavior for the textual value ranges listed above.

## Security Notes

- Resource limits: primary asset bytes, sidecar bytes, line length, token length, string length, vertices, faces, meshes, and diagnostics are bounded by `ResourceLimits`.
- Path constraints: sidecars and textures resolve through `AssetIo` and `AssetPath`; the format crate does not call filesystem APIs directly.
- Dependency policy: passing `baozi_import::DenyDependencies` to `Importer::read_asset_with_resolver` rejects every MTL dependency before backend IO. The importer may preserve the OBJ geometry and report diagnosed optional unavailability only when the prepared request uses `LossPolicy::preserve_or_fail().allow(ImportLossKind::OptionalDependencyUnavailable)`; otherwise the denial is terminal. `read_bytes` applies the same resolver policy implicitly.
- Cancellation: operation cancellation is always terminal and is never converted into optional MTL unavailability. UTF-8/line scans poll in bounded byte chunks, and material/mesh comparison work polls in bounded work chunks.
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
