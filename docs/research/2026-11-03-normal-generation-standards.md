# Normal Generation: Format Requirements and Algorithm Choices

Research date: **2026-11-03**. This supports the
[performance convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
It authorizes no implementation change and supplies no new performance measurement.

## Conclusion

There is a format requirement for missing glTF triangle normals: **flat shading**, rather than
an unspecified smooth-normal estimator. There is no single smooth-normal weighting rule shared
by the primary implementations inspected here. Format interpretation, smoothing boundaries,
weighting and numerical evaluation are separate decisions. MikkTSpace defines tangent-space
generation using input normals; it does not select the geometric normal field.
[glTF, Meshes][gltf-meshes]; [libigl][igl-cpp]; [Blender manual][blender-weighted];
[MikkTSpace interface][mikk].

Baozi's explicit generic `Smooth` policy is documented as area weighted. That policy need not
change. The concrete default-policy gap is the asset consumer's unconditional request for
`NormalOptions::default()`, including glTF primitives without normals. If its output is used
for format-faithful lit rendering, that request can smooth a crease that glTF defines as flat.
An importer preserving an absent attribute is not, by that fact alone, violating the rendering
semantics. The frozen comparison separately requests Smooth only when its
`generate_missing_normals` option is enabled; its workload must remain unchanged.
See [Baozi policy and proposed corrections](#baozi-policy-and-proposed-corrections).

Subsequent implementation at `509b80de4fff2494d5315f7a17e086cb7afe2f26` corrects that consumer
decision with explicit Preserve, SourceFormat and GeometryEdit intents. The
[implementation receipt](../reviews/2026-11-03-consumer-normal-intent.md) records 138 consumer
tests, strict lint and CLI checks. SourceFormat selects Flat for supported static glTF inputs
and returns owned typed failures for its unsupported tangent/morph/bound-skin cases. It does
not implement MikkTSpace or animated implicit-flat evaluation. The source audit and proposed
oracles below retain their original inspection scope; they are not claims of remaining default
Smooth behavior or of full rendering conformance after that correction.

## glTF: Normative Behavior and Its Scope

The published registry page retrieved on 2026-11-03 identifies itself as **2.0.1**, dated
**2021-10-11**, generated from `8e798b02d254cea97659a333cfcb20875b62bdd4`.
Sections **3.7.2.1, Meshes / Overview**, and **3.7.2.2, Morph Targets**, are the published
references used below. The current Khronos source, commit
`59ecbdba704206bb656965ab4e202881806066f1` dated **2026-10-30**, reorganizes this material under
`meshes-usage` and `morph-targets-usage`. Its own introduction distinguishes repository source
from published registry specifications. The newer wording expresses the same missing-normal
meaning as implicit flat normals; it must not be quoted as the older page's exact requirement.
[Published specification][gltf]; [current source, publication note][gltf-current-note];
[current primitive usage][gltf-current-usage].

| Situation | Published requirement or guidance | Consequence for a consumer |
| --- | --- | --- |
| Triangle primitive has no `NORMAL` | Section 3.7.2.1 requires clients to calculate flat normals and disregard supplied tangents. | One geometric normal per triangle; shared position indices do not authorize smoothing. A client may represent the result per face/corner, split output vertices, or calculate it at rendering time. The current source explicitly discusses possible vertex duplication. |
| `NORMAL` is present | Its attribute meaning is a normalized XYZ normal. | Use the authored field for format interpretation. Replacing it with geometric normals is an editing policy, not completion of a missing field. |
| `TANGENT` is absent | Section 3.7.2.1 recommends default MikkTSpace using positions, normals and the UV set associated with the normal texture. | This is tangent construction after normal selection, not permission to generate smooth normals. The current source explicitly scopes the calculation to materials with a normal texture. |
| Base primitive has no normals and has morph targets | Section 3.7.2.2 requires flat normals for each target and disregards provided tangents and tangent deltas. | Generating a static base normal field alone does not establish correct shading as position morphs change the surface. |
| An attribute has morph deltas | The corresponding base attribute must exist; geometric deltas precede skinning and node transforms. | Preserve authored normal deltas when their base normals remain valid. Do not invent an absent normal-delta field or silently retain deltas after replacing their base. |

Sources: [Meshes, including the attribute table][gltf-meshes]; [Morph Targets][gltf-morphs];
[current source, implicit normals and morph application][gltf-current-morphs]. The representation
alternatives above are implementation advice consistent with these semantics, not additional
mandated storage layouts. In particular, neither vertex splitting nor an allocated normal array
is a requirement on a bare import API.

`KHR_materials_unlit` is **ratified** and defines color independently of lighting. It uses base
color, vertex color and alpha, with alpha coverage and double-sided behavior retained; lighting
properties are fallback data. Thus an application consuming only the supported unlit model does
not need normals to compute that color. This is not a reason to discard authored attributes or
fallback resources. Normal generation can still be requested for another application task or a
PBR fallback. Likewise, position-only extraction need not materialize a shading field merely
because a later renderer might need one. [Extension, Extending Materials / Definition /
Extension compatibility][unlit].

Do not extend the triangle rule to all topology. Published section **3.9.7, Point and Line
Materials**, gives separate lighting recommendations based on which normal/tangent attributes
exist. [Point and Line Materials][gltf-lines].

## OBJ: Authored Normals and Smoothing Boundaries

The original Wavefront **Advanced Visualizer, Appendix B1, Object Files**, is available as a
transcription linked by the Library of Congress. It describes the 3.0-era format; the retained
transcription has no precise publication date. LOC dates the original manual copy to roughly
1992. The Utah PDF link currently returns 404, so the transcription, not an unread PDF, is the
textual evidence here. [Original Appendix B1][obj]; [LOC provenance and original-document links][loc].

The relevant sections say:

- **Vertex data / `vn i j k`:** supplied vertex normals replace facet normals for shading and
  take precedence over smoothing groups.
- **Elements / Referencing vertex data / `f`:** positions, texture vertices and normals have
  independent index spaces. Each face corner can select its own `v/vt/vn` tuple; `v//vn` omits
  only the texture index. Consequently, one position can have several authored corner normals.
- **Grouping / `s group_number`:** smoothing state applies to subsequent elements; `off` and
  `0` turn it off, while group numbers distinguish polygonal smoothing groups. The adjoining
  squares examples explicitly demonstrate smoothing across a shared edge.

These establish authored-normal precedence and discontinuities, not a specific area/angle/equal
weight formula. Do not treat object/group names (`o`/`g`) as aliases for smoothing state, or
replace independent position identity with approximate coordinate equality. Those would be
extra import/edit policies, not consequences of the cited syntax. [Appendix B1, Vertex data,
Elements and Grouping][obj].

Baozi preserves `Unspecified`, `Off` and nonzero group state in the OBJ parser. A completely
unspecified stream can remain absent; when a smoothing stream is activated, earlier unspecified
faces are backfilled with zero. This research does not infer a universal missing-`s` estimator
from that representation. Explicit `s off`, differing groups and authored `vn` precedence
should be tested independently. See `crates/baozi-format-obj/src/obj.rs:49`,
`src/mesh_builder.rs:338`, and `tests/geometry.rs:643` in that crate.

## Smooth-Normal Estimators Are Choices

For a nondegenerate triangle at a corner, let `e1` and `e2` be its two outgoing edges,
`c = e1 × e2`, `n = c / |c|`, and `theta` the corner angle. A common family is
`normalize(sum(w * n))`, **after selecting which incident corners may contribute**.
The following formulas summarize the cited implementations; they do not choose those
discontinuity boundaries.

| Estimator | Contribution before the final normalization | Verified primary implementation |
| --- | --- | --- |
| Equal / uniform | `n` | libigl's `UNIFORM` uses weight 1. |
| Area | `c`, equivalently `2 * area * n` | libigl's `AREA` and `DEFAULT` use double area; Baozi sums unnormalized crosses. |
| Corner angle | `theta * n` | libigl's `ANGLE`; VCGLib's `PerVertexAngleWeighted`. |
| Area times angle | `area * theta * n` before any application-specific bias | Blender exposes this separately from its area and angle modes. |
| Nelson Max style | `c / (|e1|^2 * |e2|^2)`, equivalently `sin(theta) * n / (|e1| * |e2|)` | VCGLib's `PerVertexNelsonMaxWeighted`; this is the inspected implementation's formula, not an independently verified transcription of the original paper. |

Sources: [libigl weighting and accumulation][igl-cpp]; [VCGLib normal updates][vcg];
[Blender 4.5, Weighting Mode][blender-weighted]. The factor two in area weighting cancels at
final normalization when applied consistently. VCGLib's update routines shown here accumulate
vectors; callers must also request normalization when they require unit vertex normals.

Two relevant original papers have verified publisher-deposited bibliographic records:

- Grit Thürmer and Charles A. Wüthrich, **Computing Vertex Normals from Polygonal Facets**,
  *Journal of Graphics Tools* **3(1), 43–46 (1998)**,
  DOI [10.1080/10867651.1998.10487487][thurmer]. VCGLib explicitly attributes its angle-weighted
  routine to this paper. The similarly numbered DOI ending in `87484` is a different paper and
  must not be used. [Publisher-deposited metadata][thurmer-meta]; [VCGLib attribution][vcg].
- Nelson Max, **Weights for Computing Vertex Normals from Facet Normals**,
  *Journal of Graphics Tools* **4(2), 1–6 (1999)**,
  DOI [10.1080/10867651.1999.10487501][max]. An author manuscript is catalogued at UC's
  [eScholarship record][max-record]. [Publisher-deposited metadata][max-meta].

**Access limitation:** publisher full texts returned HTTP 403; the Max institutional PDF also
returned 403. The paper metadata and implementation attributions are verified, but the papers'
derivations, experimental rankings and claimed exactness/convergence properties were not read.
No such properties are asserted here. In particular, VCGLib's comment about spherical surfaces
is not adopted as independently verified paper evidence. This limitation does not prevent
comparing the explicitly implemented estimators, but it prevents recommending a replacement on
the authority of either paper's uninspected results.

Changing an estimator can change a visible result on irregular tessellations. Choosing angle
weights, Max weights, or Assimp's grouping merely because another tool uses them would be a
semantic change requiring its own application goal and geometric oracles. None is mandated for
generic smoothing by the glTF or OBJ sources above.

## What Mature Implementations Actually Do

| Owner and pinned revision | Observed behavior | What it establishes |
| --- | --- | --- |
| Blender glTF importer, `c05ed6cedd76acf8929a6dcd276a6594040ad48f`, 2026-10-29 | `set_poly_smoothing` marks primitives without `NORMAL` flat in both the `NORMALS` and `SMOOTH` import branches. Authored normals are decoded in the `NORMALS` branch and installed as custom split normals. | Format handling is separate from the application's general smoothing machinery. This is source inspection, not a conformance test of every importer option. |
| Blender 4.5 Weighted Normal modifier manual, accessed 2026-11-03 | Describes custom face-corner normals, area/angle/product modes, sharp boundaries and artist controls; identifies angle weighting as Blender's ordinary default combination. | A mature authoring application intentionally offers several estimators. Its modifier defaults do not replace glTF import rules. |
| libigl, `b41f1944a703653ae0da81e6588650ba14c270ea`, 2026-10-29 | Exposes uniform, area and angle weighting; default selects area. | Even mature geometry libraries need an explicit estimator contract. |
| Assimp 6.0.5, `392a658f9c271be965271f45e7521a1b80ea4392` | glTF2 importer copies normals if present; tangent extraction is nested under normal presence. It does not synthesize missing normals at that location. Explicit `GenVertexNormalsProcess` normalizes face crosses, finds nearby positions and sums their unit-normal slots, with an angle-dependent route. | Importing absent data and explicitly smoothing it are distinct stages. The measured 175-degree Smooth route is a postprocess choice, not the glTF missing-normal rule. |

Sources: [Blender mesh import, normal installation][blender-import-normals] and
[smoothing branches][blender-import-smoothing]; [Blender manual][blender-weighted];
[libigl enum/default][igl-h]; [Assimp glTF2 import][assimp-import];
[Assimp explicit normal process][assimp-normals]. Assimp's position-neighbor accumulation must
not be described as source-identity area weighting. This research makes no claim that all
geometries or public import pipelines in these projects have been audited.

MikkTSpace's own API reads **position, normal and UV per face corner**, and returns tangent data
per corner. It warns against overwriting those results through an incompatible existing index
list. This reinforces the need to preserve corner discontinuities and choose the normal map's
UV set. It is not evidence for changing the input normal estimator or welding source vertices
in Baozi. [MikkTSpace interface, callbacks and output contract][mikk].

## Baozi Policy and Proposed Corrections

The relevant files below match audited R2 source
`4010bd205c7f712278bcda7b9378cd5b8019375d`. Concurrent R3 work is under qualification elsewhere;
this is not a receipt for that runtime delta.

| Boundary | Actual code | Assessment |
| --- | --- | --- |
| Generic normal operation | `crates/baozi-postprocess/src/normals/options.rs:3`: `Smooth` defaults to area-weighted shared source identities and face groups; `ExistingNormals` defaults to `Preserve`. | A concrete, documented editing policy. Retain its meaning. |
| Generic triangle preparation | `crates/baozi-postprocess/src/prepare/options.rs:49`: derived default leaves `normals` as `None`. | Preparation does not silently request a normal estimator. |
| Numeric grouping | `crates/baozi-postprocess/src/normals/kernel.rs:364`, `:496`, `:757`: group by source vertex and smoothing group, with group zero separated by face; accumulate original-order area contributions. | Neither coordinate welding nor UV seams decide normal continuity. No crease-angle or connected-fan partition is promised. |
| Real asset workflow | `tools/consumers/assets/src/lib.rs:116-135`: collects all primitive targets and unconditionally requests default normals. | **P1 policy gap for format-faithful lit glTF consumption:** absent normals can become Smooth across noncoplanar adjacent triangles. This does not indict the bare importer or the explicit generic operation. |
| Frozen comparison | `tools/consumers/assets/src/comparison.rs:235-240`: requests default normals only when `generate_missing_normals` is true. | Explicit Smooth workload. Preserve it for historical and new R1/R2/R3 comparisons. |

The simplest correction belongs at the consumption boundary: make the caller's intent explicit
as preserving source fields, satisfying format-defined shading, or requesting a named geometric
edit. For format-faithful lit glTF triangles, absent normals select Flat while authored normals
are preserved. Resolve the source format from the import contract, not a filename heuristic;
absence alone in an arbitrary `ValidatedScene` does not identify glTF. Group heterogeneous
targets by the selected policy when necessary. An unlit-only or position-only request may leave
normals absent unless another requested operation needs them. This does not require replacing
the numeric owner or globally changing `NormalMode::default()`.

Two related **P2 integration gaps** need explicit boundaries and tests before claiming a complete
format-aware rendering path:

1. **Tangent policy:** generic preparation correctly rejects normal generation that would retain
   an invalidated tangent (`prepare/input.rs:197`). A glTF consumer must instead disregard
   tangents when base normals are absent, and use the appropriate missing-tangent behavior when
   normal mapping is actually requested. Baozi's `tangents/options.rs:14` and `kernel.rs:1`
   explicitly name **Triangle UV Derivative v1**, not MikkTSpace. Do not claim Mikk compatibility
   for that algorithm, or silently change the generic tangent-preservation contract.
2. **Morph evaluation:** keep generic replacement's rejection of dependent authored normal or
   tangent deltas. Separately define how implicit flat normals follow the consumed morphed
   geometry. A typed unsupported result is honest where this is not implemented; a static
   generated base field is not proof of animated shading support. Existing tests already cover
   dependent-delta rejection and frame requests requiring normal-delta coverage:
   `crates/baozi-postprocess/tests/space_deformation_animation.rs:626` and
   `tools/consumers/assets/tests/evaluation_frames.rs:332`.

These are scoped consumer-policy recommendations, not permission to weaken source ownership,
original-scene failure returns, cancellation, resource bounds, or independent reports. Removing
unrequested shading work from a new application policy is distinct from accelerating the
existing measured Smooth workload.

## Regression Cases and Numerical Contracts

The core distinction is already tested by
`crates/baozi-postprocess/src/normals/tests.rs:110`; area weights, group zero, differing groups,
corner UV seams and coincident-but-distinct source IDs also have dedicated cases. Public
normal-preservation and precision cases exist in `tests/normals_boundary.rs` and
`tests/normals_precision.rs`. The asset consumer already has a folded glTF fixture and an
**explicit Flat** extraction test in `tools/consumers/assets/tests/deformation_seams.rs:38-147`.
That does not exercise the default `process()` policy. The default workflow's missing-normal
orientation case at `tests/workflow.rs:581` contains just one triangle.

Add focused consumer tests rather than duplicate those kernel tests:

- **Default lit glTF, two adjacent noncoplanar triangles, no normals.** Reuse or reduce the
  folded fixture: `p0=(0,0,0)`, `p1=(1,0,0)`, `p2=(0,1,0)`, `p3=(0,0,1)`, triangles
  `(0,1,2)` and `(1,0,3)`. Source-space face normals are `+Z` and `+Y`. A format-faithful path
  must keep those corner directions across the shared edge, rather than `(0,1,1)/sqrt(2)`.
  Assert the actual configured target-space transform, detached output indexing and independent
  report disposition. Use a separate explicit Smooth call to retain its existing averaged oracle.
- **Authored normals on the same crease:** preserve their selected values and seams even if a
  geometric estimator would disagree. Cover independent OBJ `vn` indices, `s off`/`0`, matching
  and differing nonzero groups, and distinct source vertices at equal coordinates at the
  consumer boundary only where current public tests do not already provide that coverage.
- **Unlit and field selection:** a supported unlit-only or position-only policy need not create
  an unused normal field; explicit geometry-processing requests still do. Retain material,
  fallback resources and authored streams. Mixed lit/unlit targets must not share one accidental
  Smooth default.
- **Normal-map and morph boundaries:** missing base normals with a supplied tangent must not
  retain that tangent as valid; missing tangents must use the normal texture's UV set in a
  claimed Mikk path. A position morph that changes a face direction must update implicit flat
  shading or produce the documented typed unsupported result. Include a mixed-weight pose,
  rather than testing only static base and target endpoints.

Robust arithmetic is a further contract, not another estimator. Baozi validates finite input,
retains relative face areas through a common scale, preserves contribution order, and rejects
degenerate/uncertain directions with typed errors. Compensated accumulation, wider intermediates
and bounded numerical errors support that definition; normalizing each face first, adding an
arbitrary fallback normal, or welding nearby coordinates changes it. See `normals/kernel.rs:452`,
`:628`, `:744`, and the F32/F64 precision tests cited above. A performance refactor may reduce
admission/callback traffic around bounded actions while preserving the mathematical sequence,
error projection and control bounds. This research supplies no basis to retune them, and no
evidence that a semantic policy change closes the all-endpoint 3x performance goal.

## Subsequent Selected-Value Audit and Windows Reproduction

A separate audit at `77a8aaba850acf4e51d3fe38e36ee436e69b493e` finds a **P2 glTF
source-direction admission/assurance gap**. The selected base-attribute collector checks
finite values, actual bounds and TANGENT.w, but does not check NORMAL/TANGENT.xyz length.
The storage-form check rejecting implicit zero directions does not reject an explicitly
stored zero. The format-independent core deliberately accepts finite authored directions;
the stronger generated-normal predicate is a different producer contract.

Main reproduced the NORMAL case on Windows with Rust 1.95.0, locked/offline dependencies,
one build job and one test thread. The probe uses the existing triangle document, binary and
PNG. It changes only the three normal-z F32 cells at byte offsets **44, 56 and 68** from
`1.0` to `2.0`; no accessor, position, index or resource descriptor changes.

| Probe | Actual result |
| --- | --- |
| Original unit normals, read through the public attribute API | Passed; all three tuples retain bits `00000000,00000000,3f800000`. |
| Same asset with NORMAL `(0,0,2)`, requiring `InvalidAccessor` | Failed with `expected glTF import to fail`: the current importer accepts the asset. |

Nextest run **`4d98a903-046a-4765-b8b2-dd9bc59b4244`** runs both probes with no skips.
This is a deliberately failing contract probe, not a passing regression qualification or
a repaired parser. Its source SHA-256 is
`618a76b1473268eef43130c21e817ca7cd25e338deaf9de53eecf79fdb168f52`.
Exact inputs, command and output are retained in
`target/review/convergence/normal-format-reproduction-1.{json,log}` and the isolated
`normal_format_contract_probe.rs`. The glTF production source matches the audited revision;
the complete isolated tree additionally contains the independently qualified import-bridge
checkpoint 4. The probe is outside that bridge's 18-path scope and is not applied to main.

The specification defines normalized base directions, but does not require every tolerant
importer to reject every malformed asset. Baozi does not claim full conformance. The concrete
follow-up is to specify its FLOAT source-direction tolerance and rejection/diagnostic policy,
then enforce that policy at the existing selected sample-collection owner. Do not infer a
normative epsilon from the generated-normal kernel or the official Validator's tool policy.
[Published attribute definitions][gltf-meshes].

Keep quantized directions and morph deltas separate. Quantization explicitly allows length
error and expects lighting consumers to normalize; retained decoded magnitudes must not be
rewritten. Morph NORMAL/TANGENT values are signed displacements, including zero and nonunit
values. Dense and sparse base counterexamples, valid quantized controls, tangent w priority,
signed/zero morph controls, failure cleanup and exact work/control prefixes belong in the
follow-up. It must use the existing pass, preserve authored bits, and avoid a global core
unit-vector restriction. [Quantization extension][quantization]; [Morph Targets][gltf-morphs].
No generation algorithm, benchmark task, admission allowance or performance gate changes in
this audit/reproduction. MikkTSpace and animated implicit-flat evaluation remain separate
capability work; this finding does not justify replacing the area-weighted Smooth estimator.

The selected FLOAT follow-up is subsequently implemented at
`0484dcb581869afef106f43721ffbb1589e1b631`, using an explicitly named Baozi `1e-4` squared-length
tolerance in the existing sample collector. It preserves admitted bits and the separate
quantized/morph/core contracts. Windows qualification passes all 525 glTF tests and all 138
asset-consumer tests, strict lint and fmt; the earlier deliberate failures remain history.
The [source-qualified receipt](../reviews/2026-11-03-gltf-base-directions.md) records exact
scope, original logs, corrected cancellation/fixture oracles and remaining limits. It is not
a new performance or full-conformance result.

## Retrieval and Source Identity

The research skill's primary-source rule was applied without additional agents. General web
search was unavailable under the reported allowance limit; it was not retried. Sources were
read directly over HTTPS through the configured local HTTP proxy. Crossref supplied publisher
metadata; a DOI-keyed OpenAlex lookup was used only to locate the author's institutional copy,
not as evidence for an algorithm. Unavailable URLs and their failures are retained alongside
successful receipts. That initial retrieval ran no Rust, Cargo, tests, benchmark or profiler;
the later Windows reproduction is qualified separately above.

Evidence is retained under `target/review/convergence/normal-standards/` in
`reviewer-primary-fetch-{1..9}.json` and their downloaded files. The registry body fetched by
main has SHA-256 `f080045a2c38e5f97b3b3407e59c113f546427bb4346604e2b38f94d88a91dc3`
at `2026-11-03T02:45:49Z`. The independent registry fetch is a separate body/receipt and is not
claimed byte-identical. Key independent source identities are:

| Retained source | Revision / access date | SHA-256 |
| --- | --- | --- |
| Registry HTML | 2.0.1 / 2026-11-03 | `891bdde79a2bd24b75f458dc643dfbf7d1d9a17513ad7eda37ae2ba0a5f03b7f` |
| Current Khronos specification source | `59ecbdba704206bb656965ab4e202881806066f1` / 2026-11-03 | `8951acd6f3299e2ca28b4017e128626731049932ef316556e320e05266e73f41` |
| Ratified unlit extension | Same Khronos commit / 2026-11-03 | `a07fc5833a2b93c6570dc6eda7baa794a2c0873c4453e9dd75815a6f3b0ef384` |
| Wavefront Appendix B1 transcription | Original date unspecified / 2026-11-03 | `60e398c4ce8fa91dcba2cf414914da03a030e1379a82620e9d3a7b0ea813263f` |
| Blender glTF mesh importer | `c05ed6cedd76acf8929a6dcd276a6594040ad48f` / 2026-11-03 | `6714838eccf9f82bf8c5a3a28b35b190773995ef9176295c27146f61521be8e1` |
| Blender weighted-normal manual | 4.5 / 2026-11-03 | `f2f56e4fc420cf3bb712794e122ec6fdc4c00c0f9f15c0964e2e51b4cfc6326b` |
| libigl normal implementation | `b41f1944a703653ae0da81e6588650ba14c270ea` / 2026-11-03 | `90a5059d0250f38ffdb9ba24b1a71983ad354ae6d9a547740d904db8a011605e` |
| VCGLib normal updates | `5cae2abc2f9056785b0537dcbe156c40da2aea20` / 2026-11-03 | `6d2c139257e4fb18ad951b5afd981ff624cfcb354247baec7e56899ab59e3c48` |
| MikkTSpace header | `3e895b49d05ea07e4c2133156cfa94369e19e409` / 2026-11-03 | `17fc433894f24c73753d548086cc4d8c5c0379f4a6edfb98b5da243e4f0bc3d0` |
| Assimp glTF2 importer | Measured 6.0.5 commit / 2026-11-03 | `cd852a4e1b52b1482cf4cc8c8eb1c687be26026017594355eff41c621efd770f` |
| Assimp normal process | Measured 6.0.5 commit / 2026-11-03 | `6822e98c5c9c88fb0c933d519eb385d1e32e8527f0f868912f415e8faf5a2db8` |

Exact Baozi hashes and equality to R2 are in `reviewer-baozi-source-receipt.json` in that evidence
directory. These are source-reading identities, not fresh correctness qualification results.

[gltf]: https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html
[gltf-meshes]: https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#meshes-overview
[gltf-morphs]: https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#morph-targets
[gltf-lines]: https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#point-and-line-materials
[gltf-current-note]: https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/Specification.adoc#L30-L39
[gltf-current-usage]: https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/Specification.adoc#L1808-L1825
[gltf-current-morphs]: https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/Specification.adoc#L1961-L2002
[quantization]: https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/extensions/2.0/Khronos/KHR_mesh_quantization/README.md
[unlit]: https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/extensions/2.0/Khronos/KHR_materials_unlit/README.md#extending-materials
[obj]: https://www.martinreddy.net/gfx/3d/OBJ.spec
[loc]: https://www.loc.gov/preservation/digital/formats/fdd/fdd000507.shtml
[blender-weighted]: https://docs.blender.org/manual/en/4.5/modeling/modifiers/normals/weighted_normal.html#options
[blender-import-normals]: https://github.com/KhronosGroup/glTF-Blender-IO/blob/c05ed6cedd76acf8929a6dcd276a6594040ad48f/addons/io_scene_gltf2/blender/imp/mesh.py#L652-L662
[blender-import-smoothing]: https://github.com/KhronosGroup/glTF-Blender-IO/blob/c05ed6cedd76acf8929a6dcd276a6594040ad48f/addons/io_scene_gltf2/blender/imp/mesh.py#L1005-L1068
[igl-h]: https://github.com/libigl/libigl/blob/b41f1944a703653ae0da81e6588650ba14c270ea/include/igl/per_vertex_normals.h#L15-L23
[igl-cpp]: https://github.com/libigl/libigl/blob/b41f1944a703653ae0da81e6588650ba14c270ea/include/igl/per_vertex_normals.cpp#L40-L103
[vcg]: https://github.com/cnr-isti-vclab/vcglib/blob/5cae2abc2f9056785b0537dcbe156c40da2aea20/vcg/complex/algorithms/update/normal.h#L87-L165
[mikk]: https://github.com/mmikk/MikkTSpace/blob/3e895b49d05ea07e4c2133156cfa94369e19e409/mikktspace.h#L62-L114
[assimp-import]: https://github.com/assimp/assimp/blob/392a658f9c271be965271f45e7521a1b80ea4392/code/AssetLib/glTF2/glTF2Importer.cpp#L575-L606
[assimp-normals]: https://github.com/assimp/assimp/blob/392a658f9c271be965271f45e7521a1b80ea4392/code/PostProcessing/GenVertexNormalsProcess.cpp#L104-L227
[thurmer]: https://doi.org/10.1080/10867651.1998.10487487
[thurmer-meta]: https://api.crossref.org/works/10.1080/10867651.1998.10487487
[max]: https://doi.org/10.1080/10867651.1999.10487501
[max-meta]: https://api.crossref.org/works/10.1080/10867651.1999.10487501
[max-record]: https://escholarship.org/uc/item/7657d8h3
