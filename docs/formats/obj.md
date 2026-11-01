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
- Primary references: original Wavefront [OBJ appendix](https://paulbourke.net/dataformats/obj/) and [MTL manual](https://paulbourke.net/dataformats/mtl/), reproduced by Paul Bourke; Assimp OBJ/MTL behavior as a clean-room behavior reference only

## Capability Matrix

| Capability | Status | Notes | Tests |
| --- | --- | --- | --- |
| Geometry | Supported | Imports static face meshes from `v`, `vt`, `vn`, and `f`, preserving independent Corner-domain UV/normal indices and negative indices without duplicating shared positions. | `crates/baozi-format-obj/tests/geometry.rs` |
| Hierarchy and transforms | Partial | Preserves object/group changes by splitting mesh segments and attaching child nodes. OBJ has no general transform stack. | `geometry.rs`, facade tests |
| Materials | Partial | Loads `newmtl`, `Kd`, `d`, `Tr`, `Ke`, `Ns`, `Ni`, `illum`, and typed legacy RGB `Ka`/`Ks`; retains source metadata and replaces duplicate materials as complete definitions. | `materials.rs`, `mtl_semantics.rs` |
| Textures and sidecars | Partial | Resolves `mtllib` and `map_Kd` through the origin-qualified dependency resolver; maps two-dimensional `-o`/`-s` and `-clamp` into typed binding/sampler state; owns encoded image bytes without pixel decoding. | `materials.rs`, `mtl_semantics.rs`, `crates/baozi/tests/obj_facade.rs` |
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
- Post-process boundary: raw OBJ parsing preserves polygon topology. Call
  `baozi::postprocess::triangulate_primitive` with the optional `postprocess` feature for an explicit
  single-primitive edit. The operation preserves winding and authored Corner/Face attributes,
  including UV/normal seams and smoothing groups, through retained origin relations.
- Attribute projection: positions are compacted per mesh segment by source position index, while UV and normal source tables are projected in face-corner order as Corner-domain attributes. UV or normal seams therefore remain distinguishable without duplicating shared positions. A segment that mixes present and missing values for one channel is rejected under `LossPolicy::preserve_or_fail()`. The existing sparse attribute representation supplies default values rather than preserving missing corner values, so partial channels require explicit loss authorization instead of synthesized defaults.
- Partial attribute loss: `LossPolicy::preserve_or_fail().allow(ImportLossKind::SourceSemanticLoss)` permits omitting each incomplete UV or normal channel while retaining positions, topology, and any complete channel. Each omitted channel records `SourceSemanticLoss` evidence with disposition `Applied` and its own code, `obj.partial_texcoord_channel` or `obj.partial_normal_channel`. Without authorization, the first incomplete channel records `RejectedByLossPolicy` evidence and terminates import. Diagnostics identify the affected channel at `/segments/{index}/attributes/texcoord` or `/segments/{index}/attributes/normal`; the typed loss subject is the source mesh. These are source-semantic omissions, independent of text recovery and conformance policy.
- Mesh splitting: material and object/group boundaries may split one OBJ file into multiple Baozi meshes. Polygon topology is preserved within each segment, and smoothing remains per-face data.
- Point/line handling: `p` and `l` are recognized source geometry omissions. They fail by default; explicit `SourceSemanticLoss` authority can omit them while retaining imported faces. This permission works under `ConformancePolicy::Strict` and does not require text recovery. Point records require at least one valid position index. Line records require at least two valid position or position/texture-coordinate references. Positive and negative indices are validated against their source tables before omission is authorized. Missing vertices, zero/out-of-range indices, malformed integers, point texture suffixes, empty line texture suffixes, and normal-index suffixes remain format errors. Other line/free-form dialects are outside this bounded profile. A file with no retained faces still fails with `obj.empty_geometry`, including an authorized point/line-only file.

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
- Native candidate callers create `NativeDirectoryCapability` and `FileSystemAssetIo` from the unpublished `baozi-native-io-candidate` package, then pass that adapter and a relative primary path to `Importer::read_asset`. Sidecars use that same authority; no facade path helper or native feature is required.
- `Importer::read_asset_with_resolver` accepts a prepared request and an explicit `DependencyResolver`. Use it when dependencies need a caller-defined authority policy, including `baozi_import::DenyDependencies` when every sidecar must be rejected before backend IO.

| MTL field | Baozi mapping |
| --- | --- |
| `newmtl` | `Material.name` and material lookup key |
| `Kd` | `Material.base_color` RGB |
| `d` | `Material.base_color.a`; alpha below 1 sets blend mode |
| `Tr` | inverse alpha; alpha below 1 sets blend mode |
| `Ke` | `Material.emissive` |
| `Ka`, `Ks` | `LegacyMaterialFactors.ambient` / `.specular`, plus exact source value text in `obj:ka` / `obj:ks` metadata |
| `Ns` | legacy shininess plus `obj:ns` metadata |
| `Ni`, `illum` | namespaced `obj:ni` / `obj:illum` metadata |
| `map_Kd` | encoded image/texture dependency plus a typed `TextureRole::Diffuse` binding, UV transform and sampler; exact option/path text in `obj:map_kd` metadata |

`Ka` and `Ks` accept one to three finite RGB components. Omitted green/blue components use red,
as specified by the original manual's scalar RGB shorthand. Nonnegative values above one remain
unchanged: legacy factors are not unit-clamped. Negative source components are syntactically valid
but cannot be represented by Baozi's nonnegative legacy factors; omitting that whole factor requires
explicit `SourceSemanticLoss` permission. Authorized omission retains the source metadata and uses
zero for the omitted ambient/specular factor. Nonnumeric, nonfinite, missing or extra RGB operands
are terminal format failures. Spectral and CIEXYZ color forms are outside this RGB profile.

Every `newmtl` begins a fresh definition with its own defaults. When names repeat within a sidecar
or across `mtllib` sidecars, the last complete definition wins, with
`obj.mtl_duplicate_material`. Fields, textures, sampler state and metadata from an earlier
definition are not merged into the replacement. Missing ambient/specular factors default to zero.

### Diffuse Texture Options

The **Baozi OBJ/MTL UV Sampling Profile** selects the common ecosystem interpretation of authored
offset and scale as a direct sampling transform. The [original MTL manual](https://paulbourke.net/dataformats/mtl/)
supplies the option arities and defaults, but its shifted/scaled-logo examples do not uniquely
specify a sampling equation or the composition order. The following equation is an explicit Baozi
profile decision, not a claim of an unambiguous normative formula:

```text
q = offset + scale * p
p = authored OBJ UV
q = texture sampling UV
```

Baozi retains authored U/V offset and scale directly in `TextureTransform`, with rotation zero.
The profile scales the UV coordinates about `(0, 0)`, then adds the offset, independently of the
options' textual order. For example, `-o 0.25 -0.5 -s 2 0.5` maps authored UV `(0.25, 0.75)` to
sampling UV `(0.75, -0.125)`. There is no V flip or center pivot. With repeating addressing,
`-s 1.2` covers 1.2 horizontal texture periods per unit source-U interval in this selected profile.
The complete authored option/path text remains available as `obj:map_kd` metadata. Tests assert
typed transform values and this profile's equation without treating the manual's examples as
independent proof of the equation.

As comparative compatibility evidence, [three.js r180's MTL loader reads authored `-s`/`-o`
directly into scale/offset](https://github.com/mrdoob/three.js/blob/r180/examples/jsm/loaders/MTLLoader.js#L547)
and [copies those values to texture repeat/offset](https://github.com/mrdoob/three.js/blob/r180/examples/jsm/loaders/MTLLoader.js#L364).
This supports the selected ecosystem profile without claiming that the original manual uniquely
requires it. No implementation code is copied.

| Option | Accepted operands and defaults | Typed behavior |
| --- | --- | --- |
| `-o u [v [w]]` | One to three finite values; omitted components are zero. | Retains authored U/V as canonical sampling offset. Neutral `w=0` needs no loss permission. |
| `-s u [v [w]]` | One to three finite values; omitted components are one. | Retains authored U/V as canonical sampling scale, including signed and zero values. Neutral `w=1` needs no loss permission. |
| `-clamp on\|off` | Exactly `on` or `off`; default `off`. | `on` sets U/V to `Decal`; `off` sets U/V to `Repeat`. W and filtering retain the canonical defaults. |

The manual's clamped texture is a single copy with the underlying material unchanged outside its
extent. The canonical `Decal` coverage rule expresses that behavior: if transformed sampling U or V
is outside the closed unit interval `[0, 1]`, the binding contributes no sample and is skipped,
retaining the underlying material before filtering, color interpretation, or alpha evaluation.
Coordinates inside the interval sample with `ClampToEdge`; the endpoints zero and one remain
covered. This is an explicit binding-coverage rule, separate from `ClampToBorder` and any border
color. The built-in Diffuse role multiplies an active sample by the legacy diffuse factor under
the canonical role contract; the binding retains `Replace` as required by that contract.

Options may appear in either order before the path. Repeating an option replaces its entire tuple,
including omitted-component defaults; the last value for each supported option is effective.
Paths use the remaining source text, preserving internal spaces. A purely numeric path token is
ambiguous with an extra operand and must use a prefix such as `./`. Option operands stay strict
even when path text recovery is authorized. Missing operands/path, extra numeric operands, nonfinite
numbers, invalid clamp/type values, negative boost, and nonpositive/noninteger/out-of-u32 texture
resolution are terminal format failures before semantic omission is considered.

The third offset/scale components describe three-dimensional behavior that the two-dimensional
binding cannot retain. This profile conservatively requires explicit semantic-loss permission for
effective nonneutral W values, including an explicitly authored scale W=0. Authorized omission
keeps both representable UV components. Zero U/V scale, finite subnormal components, and all other
finite F32 transform components are retained without semantic loss. No reciprocal transform is
constructed. Numeric operands still follow the existing F32 OBJ/MTL parsing contract, including
rejection of nonfinite results.

### Recognized Semantic Omissions

Each branch below rejects under `LossPolicy::preserve_or_fail()`. Adding
`.allow(ImportLossKind::SourceSemanticLoss)` permits the specified omission and records
`ImportLossDisposition::Applied`, including under strict conformance. Rejected omissions retain
`RejectedByLossPolicy` evidence. `FormatRecovery` permission alone never substitutes for this
authority, and neither loss permission nor declared-recovery conformance accepts malformed syntax.
Even apparently neutral operands on the six unimplemented recognized map options require this
authority; they are not silently interpreted as supported no-ops.

| Branch | Loss code | Authorized result |
| --- | --- | --- |
| `map_Kd -t` | `obj.mtl_texture_turbulence_omitted` | Omits turbulence; validates one to three finite operands. |
| `map_Kd -mm` | `obj.mtl_texture_range_omitted` | Omits range adjustment; validates two finite operands. |
| `map_Kd -bm` | `obj.mtl_texture_bump_multiplier_omitted` | Omits the recognized compatibility option; validates one finite operand. |
| `map_Kd -boost` | `obj.mtl_texture_boost_omitted` | Omits sharpness adjustment; validates one finite nonnegative operand. |
| `map_Kd -type` | `obj.mtl_texture_type_omitted` | Omits projection; validates `sphere` or one of `cube_top`, `cube_bottom`, `cube_front`, `cube_back`, `cube_left`, `cube_right`. |
| `map_Kd -texres` | `obj.mtl_texture_resolution_omitted` | Omits image resolution selection; validates a positive U32 integer. |
| Effective nonzero offset W | `obj.mtl_texture_offset_w_omitted` | Keeps the UV transform, omits W. |
| Effective scale W other than one | `obj.mtl_texture_scale_w_omitted` | Keeps the UV transform, omits W. |
| Negative `Ka` / `Ks` RGB | `obj.mtl_ambient_omitted` / `obj.mtl_specular_omitted` | Retains source text, uses zero for that whole legacy factor. |
| OBJ `p` / `l` | `obj.point_geometry_omitted` / `obj.line_geometry_omitted` | Omits the validated point/line record, retains faces. |

MTL omissions use a typed source-material subject and diagnostic paths
`/dependencies/{mtl-path}/records/{line}/{keyword-or-option}`, with dependency path, line, column,
option and reason details. The current public semantic-loss recorder binds the evidence's
`source` field to the primary OBJ; the diagnostic path/details identify the actual MTL snapshot.
OBJ point/line omissions use a typed source-primitive subject and `/records/{line}/p` or
`/records/{line}/l`. Each omitted record/option has its own bounded evidence entry. Source metadata
is descriptive evidence alongside typed fields and authorized omissions, not a substitute for them.

MTL is optional enrichment only at the dependency-acquisition boundary. A missing, policy-denied, unreadable, or acquisition-budget-exhausted MTL may be represented as unavailable, emit structured optional-dependency diagnostics, and preserve valid OBJ geometry only when the request loss policy explicitly allows `ImportLossKind::OptionalDependencyUnavailable`. Without that authorization, the dependency failure is terminal.

Once an MTL snapshot is available, it is parsed under the same text, resource, and operation controls as the primary OBJ. Any failure returned while decoding or parsing that snapshot is terminal and cannot be reclassified as optional unavailability. In particular, invalid UTF-8 is terminal in `RejectInvalid` mode and invalid UTF-8 outside the narrowly recoverable `newmtl` and `map_Kd` path values is terminal in every mode. Statements that the MTL parser explicitly classifies as warnings may continue, but malformed sidecars do not receive a blanket optional-dependency waiver. Unknown `usemtl` references create a placeholder material with a warning so the mesh can keep a stable material binding.

## Known Limitations

- OBJ points/polylines are covered by the explicit omission contract above; free-form curves, surfaces, parameter spaces, bevel/interp directives, and other exporter dialects remain unimplemented with their existing warning behavior.
- The importer does not repair degenerate faces, generate normals, normalize coordinates, infer units, flip winding, decode image files, or fetch remote assets.
- The completed recognized MTL option list is exactly `-o`, `-s`, `-clamp`, `-t`, `-mm`, `-bm`, `-boost`, `-type`, and `-texres`. Unknown options keep the earlier warning and one-operand skipping behavior; other MTL maps and syntax remain outside this bounded audit. This does not claim that all unknown source semantics are loss-gated.
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
| `mtl_semantics.rs`, `geometry_loss.rs` | typed UV transforms, sampler/factor defaults, named UV sampling profile, finite zero/extreme transform components, full-definition replacement, and each recognized omission/conformance branch | Baozi project license; independently authored fixture bytes | borrowed typed Scene assertions and bounded report evidence |
| `fuzz/corpus/obj_import/*` | seed inputs for sanitizer fuzzing | Baozi project license | fuzz target corpus |
