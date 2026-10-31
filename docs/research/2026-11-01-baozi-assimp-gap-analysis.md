---
title: Baozi Progress and Assimp Gap Analysis
date: 2026-11-01
status: source-audited
---

# Baozi Progress and Assimp Gap Analysis

## Conclusion and evidence boundary

Baozi has completed its first usable-library delivery. Its next stage should improve the fidelity
and usefulness of existing asset workflows, then expand selected format profiles. The evidence
does not justify replacing the typed Scene architecture or pursuing Assimp-wide feature parity.
It does identify smaller ownership boundaries that should change before the next related feature,
especially the Scene-wide declaration of tangent construction and encoding.

Five independent read-only audits covered core/readers, format profiles, postprocessing/consumers,
import/IO/distribution, and validation/scale. This report consolidates their source findings. No
Assimp binary was built or executed, no differential corpus was run, and no new Baozi test or
performance result is claimed. Recommendations are input to the
[proposed execution plan](../plans/2026-11-01-001-feat-asset-workflow-expansion-plan.md).

| Baseline | Exact identity and use |
| --- | --- |
| Baozi | `e9c6b8780e8e8708547363a7b781d401ff93e1a0`; includes `ffb4fba` source-object metadata and exact glTF extras preservation. |
| Assimp source audit | Clean local `repo-ref/assimp` at `a2a586d9370a493651256ec5d03524710ce37a5f`, 2026-10-28; `v6.0.5-179-ga2a586d93`. All source comparisons below use this revision. |
| Assimp latest release, checked 2026-11-01 | [v6.0.5](https://github.com/assimp/assimp/releases/tag/v6.0.5), published 2026-04-30; peeled commit `392a658f9c271be965271f45e7521a1b80ea4392`. The audited development revision is later than this release. |
| Assimp remote master, checked 2026-11-01 | `a4643cbd30d3088bbff312d3cc2652f514fb97c0`, 2026-10-30. Its [PLY uint color correction](https://github.com/assimp/assimp/commit/a4643cbd30d3088bbff312d3cc2652f514fb97c0) is a separate recent-regression input, not an audited release capability. |

The [July Assimp study](assimp-replication-study.md) remains historical. In particular, its
one-material-per-mesh recommendation has been replaced by Baozi's logical mesh / primitive /
instance model. Neither that study nor an unimplemented ADR section establishes current behavior.

## Current progress

| Area | Delivered behavior | Qualification boundary |
| --- | --- | --- |
| A–H library delivery | Typed immutable Scene storage/readers, bounded import/IO, selected formats, three concrete postprocess operations, independent consumers, and package/platform checks. | [Completion record](../reviews/2026-10-29-delivery-progress.md#completed-ah-delivery-2026-10-31): hosted revision `0bb29f15`, run `41201133748`, 15/15 jobs. This completed sequence stays closed. |
| Real asset consumption | Unchanged pinned glTF assets, application-owned buffers after Scene drop, full Scene conversion, skeletal/morph data extraction. | [Initial integration](../reviews/2026-10-31-real-asset-consumer.md) and [metadata increment](../reviews/2026-10-31-source-object-metadata.md): six successes and one explicit `SimpleSkin` numeric-policy rejection. |
| Latest local verification | Windows workspace 2,389/2,389; standalone application 10/10 on Windows and Linux/WSL release; two Linux release profile variants each complete six assets. | Exact implementation and commands belong to the linked records. These observations do not extend the older hosted/Apple result to the later metadata changes. Four heavy storage pilots are manual cases, not ordinary-suite failures. |
| Format claims | Source-admitted STL, OBJ/MTL, PLY, and selected glTF/GLB. | [Support matrix](../formats/support-matrix.md): declarations remain `Unverified`, maturity remains None. Admission, maturity, package publication, and native release registration are distinct. |

Existing growth tests already vary geometry, primitive buffers, hierarchy shape, shared resources,
influences, metadata, and resource-cache behavior. The missing evidence is larger end-to-end
application workloads and exploration of consecutive Scene edits, rather than the absence of all
scale testing.

## Core and consumer capability comparison

Assimp references in this section are fixed to the source-audit revision above. A retained field,
a public reader, an importer mapping, and an evaluator are four different capabilities.

| Capability | Baozi evidence and assessment | Assimp comparison |
| --- | --- | --- |
| Ownership and identity | [Generation-qualified borrowed readers](../../crates/baozi-core/src/canonical_scene/store/api/read.rs) expose immutable validated data. Preserve this boundary; generation is not a persistent asset key. | [Scene pointer graph](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/scene.h#L273) is not a replacement model for Rust. |
| Geometry identity and domains | [Logical meshes, primitives, placements and typed attribute readers](../../crates/baozi-core/src/canonical_scene/store/api/read/geometry.rs) retain Vertex/Corner/Face domains, sparse storage and custom F32/F64/U32/I32 streams. | [Mesh structure](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/mesh.h#L620) uses one material per mesh and primarily per-vertex channels. Baozi need not flatten its model to match. |
| Hierarchy navigation | [Nodes](../../crates/baozi-core/src/canonical_scene/store/api/read/node.rs) expose roots, ordered children, and authored Matrix/TRS; placements are enumerable. Parent and node-to-instance convenience reads are absent. | [Node fields](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/scene.h#L107) include parent and mesh indices. This is a navigation gap, not lost hierarchy. |
| Materials and textures | [Typed factors](../../crates/baozi-core/src/appearance.rs) and [binding readers](../../crates/baozi-core/src/canonical_scene/store/api/read/appearance.rs) cover legacy and PBR groups, separate image/sampler/texture identity, UV transforms, channel interpretation and strengths. IOR and volume attenuation are concrete missing fields. | [Material keys](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/material.h#L995) cover more shading and mapping models. A generic property bag would not establish typed consumer semantics. |
| Camera/light placement | [Spatial readers](../../crates/baozi-core/src/canonical_scene/store/api/read/spatial.rs) and direct node references avoid name matching; complete intrinsic axes survive conversion. | [Camera](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/camera.h#L105) has node-relative offsets. A future source mapping must preserve such offsets explicitly. |
| Light representation | [Current definitions](../../crates/baozi-core/src/entity.rs) represent directional/point/spot, range/cones and tint/intensity. Area/ambient lights, legacy independent emission colors and arbitrary attenuation coefficients are outside this typed model. | [Light kinds and attenuation](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/light.h#L63) are broader. Extend only with a selected producer and independent mapping fixtures. |
| Skin and morph retention | [Deformation readers](../../crates/baozi-core/src/canonical_scene/store/api/read/deformation.rs) expose variable influence rows, instance-selected palettes, bind/correction spaces, logical morph identity, per-primitive deltas, and instance overrides. | [Bones and animation meshes](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/mesh.h#L265) include other vertex-animation methods. Those are future producer requirements, not missing current skin readers. |
| Animation retention | [Animation readers](../../crates/baozi-core/src/canonical_scene/store/api/read/animation.rs) expose f64 time, time domains, extrapolation, values and cubic derivatives. Current targets are node TRS and instance morph weights; interpolation is channel-wide, and TRS channels on matrix-authored nodes are rejected. | [Animation model](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/anim.h#L276) has different matrix-replacement and interpolation assumptions. Neither model is a complete animation runtime. Test these differences before adding another animation format. |
| Metadata and provenance | [Metadata/source/resource readers](../../crates/baozi-core/src/canonical_scene/store/api/read/metadata.rs) preserve exact selected extras. [Stored provenance](../../crates/baozi-core/src/canonical_scene/store/typed_storage/profile.rs) also contains owner attachments, Source/Operation nodes and parent edges, but the consumer reader does not expose that relation chain. | [Metadata](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/metadata.h#L66) is directly attached but has no equivalent source/operation DAG. Baozi's gap is making retained relationships usable. |

The provenance gap was checked against crate-root exports, authoring and validation types, hidden
integration wrappers and all `ValidatedScene` implementations. Public authoring inputs are not
readers for a published Scene. Existing source-scope reports and postprocess remaps remain useful
implemented evidence; neither substitutes for owner-to-retained-provenance navigation.

The node hierarchy is an ordered single-parent forest, with a hidden synthetic root, rather than
a general shared-node DAG. A parent reader is semantically unambiguous. Mesh definitions may be
shared by several instances, and a node may own several placements; do not collapse those rows
into one `(node, mesh)` association. Navigation convenience should reuse retained structure where
possible and document its cost, instead of requiring a new persistent index by assumption.

## Existing format profiles

Assimp's [importer registry](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/Common/ImporterRegistry.cpp#L237)
has much broader format coverage. Baozi's more urgent gaps are inside its current four profiles.

| Area | Baozi gap or deliberate boundary | Assimp source and implication |
| --- | --- | --- |
| glTF primitive modes | [`map_topology`](../../crates/baozi-format-gltf/src/parser.rs) accepts points, lines and triangles; line strip/loop and triangle strip/fan are rejected, although [core topology authoring](../../crates/baozi-core/src/canonical_scene/store/production/input/geometry.rs) and validation already represent them. | [Mode lowering](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/glTF2/glTF2Importer.cpp#L891) exists. Baozi should first preserve authored modes in import; explicit list lowering for a consumer is a separate operation with strip-parity/winding and degenerate tests. |
| glTF sparse accessors | Shared accessor validation rejects sparse geometry as well as sparse morph. Format-side `accessor_provenance`, [attribute storage](../../crates/baozi-format-gltf/src/parser/model.rs) and [morph storage](../../crates/baozi-format-gltf/src/parser/morph.rs) assume one base-buffer source. Existing core sparse storage does not make the importer support it. | [Accessor population](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/glTF2/glTF2Asset.inl#L818) handles base/zero initialization and substitution. Extend shared decoding, selected closure and bounded base/indices/values source aggregation together; removing the rejection alone is insufficient. |
| glTF influence sets | Normalized UV/color/weight forms are admitted, but only `JOINTS_0` / `WEIGHTS_0` are read; additional sets are rejected. The core's variable influence rows are already sufficient for a broader importer slice. | [Weight mapping](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/glTF2/glTF2Importer.cpp#L1108) iterates additional sets. No core fixed-four redesign is needed. |
| glTF extensions | Only selected `KHR_texture_transform` payload sites are admitted. Unlit, punctual lights and advanced material payloads are absent despite some corresponding core types. Selected unsupported optional payloads are rejected; only unsupported declarations without such selected payloads can retain fallback with a warning. | [Lights](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/glTF2/glTF2Importer.cpp#L1016) and [material extensions](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/glTF2/glTF2Asset.inl#L1374) cover more slices. Resolve explicit fallback/loss policy before widening admission. |
| glTF compression and quantization | Draco, meshopt, quantization and alternative image extension payloads are outside the selected profile. Encoded image retention is not pixel decoding. | [Draco path](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/glTF2/glTF2Asset.inl#L1591) is build-option dependent. No meshopt or `KHR_mesh_quantization` implementation was found in the audited glTF2 directory; do not attribute every modern extension to Assimp. |
| glTF scene selection | Default/sole-scene selection is deterministic; multiple scenes without a default are rejected. Caller selection of a nondefault scene is absent. Duplicate animation targets are rejected. | [Default scene fallback](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/glTF2/glTF2Asset.inl#L2172) differs. Do not copy implicit scene-zero choice or overwrite duplicate channels. |
| OBJ/MTL semantics | [`texture_path_from_map`](../../crates/baozi-format-obj/src/mtl.rs) skips recognized `map_Kd -o/-s/-clamp` options; [authoring](../../crates/baozi-format-obj/src/mesh_builder.rs) always emits identity texture transform and default sampler. `Ka/Ks` survive as metadata but not corresponding typed factors. OBJ `p/l` records are warned and skipped. | [MTL options](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/Obj/ObjFileMtlImporter.cpp#L604) and [OBJ primitives](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/Obj/ObjFileParser.cpp#L333) demonstrate useful missing semantics. Recognized semantic loss must be mapped or rejected/authorized, rather than mistaken for successful fidelity. |
| PLY semantics | U32/signed/out-of-range float color mapping needs correction below. Built-in stream narrowing, duplicate semantic aliases, face-corner UV and recognized property omissions also need an explicit profile. Custom F64 preservation is already implemented and tested. | [PLY face UV mapping](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/Ply/PlyLoader.cpp#L620) writes onto vertex channels; it is not a seam-preservation oracle. |
| STL dialects | Existing face/material color composition and ASCII multi-solid tests should remain. The supported binary color dialect is deliberately narrow. | [STL color behavior](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/STL/STLLoader.cpp#L500) is another implementation, not manufacturer authority for Magics/Materialise validity-bit semantics. Verify a named dialect before broadening it. |

For glTF sparse layout, optional-extension fallback, quaternion tolerances, PLY color conventions
and STL dialects, primary source semantics must resolve disagreements. No Assimp acceptance or
rejection alone establishes conformance. Current selected-profile restrictions are documented
limitations; the PLY and MTL successful-import mismatches are higher-priority fidelity problems.

## Postprocessing and application outputs

| Workflow | Current behavior and remaining gap | Fixed Assimp source |
| --- | --- | --- |
| Triangulation and normals | [Triangulation](../../crates/baozi-postprocess/src/triangulate/input.rs) and [Flat/Smooth normals](../../crates/baozi-postprocess/src/normals/input.rs) are transactional, domain-aware operations. Their dense F32/simple-planar-polygon input boundaries are real restrictions. Strip/fan lowering is absent. | [Operation ordering](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/Common/PostStepRegistry.cpp#L225). |
| Coordinate/unit conversion | [Whole-Scene conversion](../../crates/baozi-postprocess/README.md) covers geometry, hierarchy, skin, morph, animation, spatial definitions and material lengths. Consumer ordering already resolves numeric CCW before generating normals. | [Postprocess operations](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/postprocess.h). No evidence supports reimplementing this as disconnected flags. |
| Tangent generation | Authored tangents are readable and convertible; generation is absent. [Scene-wide tangent declarations](../../crates/baozi-core/src/scene_space.rs) cannot precisely describe independently generated and authored streams. Resolve ownership before generation. | [Tangent implementation](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/PostProcessing/CalcTangentsProcess.cpp#L103) is not evidence of MikkTSpace compatibility. |
| Indexed application vertices | [`triangle_vertices`](../../crates/baozi-postprocess/src/vertices.rs) is a zero-allocation Corner tuple reader. The [application](../../tools/consumers/assets/src/buffers.rs) copies all tuples and emits sequential indices. Deterministic reuse within each source Vertex can reduce buffers while preserving seams and deformation. | [Join vertices](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/PostProcessing/JoinVerticesProcess.cpp). Application packing and canonical vertex welding are different operations. |
| Bounds and transforms | Application `bounds()` combines untransformed base draw coordinates. It is not an instance/world bound. Public world-transform, animation and skin evaluators are absent. Static read-only instance bounds are a bounded next consumer slice. | [Bounding boxes](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/PostProcessing/GenBoundingBoxesProcess.cpp#L57) are mesh-local; [pretransform](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/PostProcessing/PretransformVertices.cpp#L572) can remove animation/bones. |
| Draw/device limits and optimization | No draw chunking, palette-limited output, cache reordering or cross-mesh merging. Reuse indices before measuring cache optimization; keep top-K influences and static baking explicitly lossy. | [Large mesh splitting](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/PostProcessing/SplitLargeMeshes.cpp), [cache optimization](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/PostProcessing/ImproveCacheLocality.cpp#L171). |
| UV conversion/baking | Baozi retains UV origin and binding transforms; no UV conversion or transform-baking operation is delivered. A binding-aware workflow is needed when a real output consumer cannot retain those values. | [Texture transforms](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/PostProcessing/TextureTransform.cpp). The recent coordinate-space fix is a warning against treating UV flipping as a spatial-axis change. |

## Import, IO and distribution

| Area | Current state and next boundary | Comparative reference |
| --- | --- | --- |
| Public workflow | [Facade](../../crates/baozi/src/lib.rs), reusable prepared requests, deterministic detection, structured outcomes, custom IO and [memory sidecars](../../crates/baozi/examples/import_memory_sidecars.rs) already exist. Forced selection still rejects a contradictory `NoMatch`. Disabled known builtins can appear unknown because their unavailable declarations are not registered. | [Assimp Importer](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/Importer.hpp#L215). Improve concrete examples and disabled-feature explanations without adding another config/report framework. |
| IO capability | [`AssetReader`](../../crates/baozi-io/src/resource.rs) provides synchronous sequential chunks; import snapshots complete primary bytes before parse. Bytes-only import denies dependency access. Custom resolvers receive normalized `AssetPath` values, whose components reject colons. | [IOStream](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/IOStream.hpp#L90) includes seek. Baozi does not currently promise streaming parsing, arbitrary URI schemes, interruptible blocking IO, async or archives. Hosts can map allowed logical names to remote resources. |
| Native adapter | [Native IO](../../candidates/baozi-native-io/Cargo.toml) is an excluded unpublished candidate with explicit directory capability, three candidate hosts and no release-registered targets. Its patched `cap-primitives` dependency also requires a root patch in the asset consumer. | [Default filesystem IO](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/Common/DefaultIOSystem.cpp#L121) is readily available in Assimp. Baozi has implementation, but not a normally distributable published native adapter. |
| Registry dependency closure | Facade optional production dependencies include `publish = false` PLY/glTF packages and a maintained decoder chain. [Package checks](../../scripts/check_packages.py) validate unpacked local artifacts through patches and `--no-verify` packaging; they do not prove registry resolution. | Cargo documents [root patch behavior](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html#the-patch-section) and [publication dependency locations](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#multiple-locations). Choose the release package set and resolve its full dependency closure before publication; turning off a feature does not remove the manifest obligation. |
| Target evidence | CI already has Windows/Apple unpacked consumers, browser-WASM compile checks, WASI execution and three native candidate jobs. Linux lacks the corresponding complete unpacked-consumer matrix. | [Current CI](../../.github/workflows/ci.yml) is the actual boundary. Add the missing Linux cases by reusing the existing script; do not rebuild checks that already exist. Qualify later changes at their own revision. |

The native dependency issue is source-inferred; no fresh external consumer without the root patch
was built in this audit. Cargo only recognizes a patch at the consuming workspace root, so native
publication needs a normal dependency solution. Repository capture is a separate capability and
must not become a prerequisite for basic native file import. First publication also needs an
explicit decoder-maintenance and exact target/profile decision. These are real delivery gaps,
but publication is not an implied action in this planning task.

## Numeric fidelity and evidence gaps

The [PLY color conversion](../../crates/baozi-format-ply/src/parser.rs) divides U8 by 255 and U16
by 65535, then clamps all scalar types to `[0,1]`. For a common full-range U32 color profile,
`2147483648` consequently becomes 1 instead of approximately 0.5. Signed integers and out-of-range
floats also reach the clamp. The [PLY support document](../formats/ply.md) does not restrict these
advertised color properties by scalar type. This is a concrete admitted-value/policy mismatch to
resolve first, not a reason to copy Assimp's converter. PLY type syntax alone does not establish
one universal color interpretation; publish the chosen profile and reject unsupported semantics
or require explicit recovery authority instead of silently saturating them. This finding is
source-derived and has not yet been reproduced by a newly executed test.

`SimpleSkin` is a different question: its rotations exceed Baozi's current unit-quaternion
tolerance. It remains a truthful named rejection. Decide the admission/recovery contract from
source semantics, numerical error bounds and consumer need; neither Assimp acceptance nor an
official sample's presence automatically warrants silent normalization. Import-time repair,
preservation of admitted authored samples, and normalization of an evaluated interpolation result
must remain distinct. The [glTF animation specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#animations)
is the primary semantic reference.

The pinned success corpus's largest source mesh currently has 160 vertices / 564 indices, as
recorded in [independent expectations](../../tools/consumers/assets/fixtures/expectations.json).
It is strong functional evidence but weak evidence for large application pipelines. Add a few
bounded larger workloads while retaining the existing small semantic regressions. Keep normal
release stage times, instrumented allocations, operation ledgers and per-process RSS separate.

Current [fuzz targets](../../fuzz/Cargo.toml) import and inspect snapshots; they do not exercise
the delivered edit chain. The [scheduled workflow](../../.github/workflows/fuzz.yml) uploads corpus
and logs, but runs short 120-second campaigns and does not restore a previous corpus automatically.
Assimp's [generic](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/fuzz/assimp_fuzzer.cc#L60)
and [postprocess](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/fuzz/assimp_postprocess_fuzzer.cc#L74)
targets suggest a useful missing test surface: successful edits, failure atomicity, cancellation
and current-generation reads through one bounded public workflow. A general Scene generator or
arbitrary pass scheduler is unnecessary.

## Architecture decisions and priority

Preserve the typed Scene root, borrowed views, central validation, logical mesh/primitive/instance
identity, attribute domains, variable influences, authored Matrix/TRS, f64 time, separate texture
identities, scoped IO and transactional edits. The completed replacement of generic record storage
does not need a second rewrite based on this comparison.

Change a boundary when the next concrete workflow exposes a false invariant:

1. Move tangent construction/encoding from `SceneSpace` to the specific Tangent attribute
   descriptor before adding local tangent generation. Primitive scope is insufficient when
   Vertex and Corner Tangent coexist. Carry the typed declaration through authoring, storage,
   borrowed reads, validation, snapshots and per-attribute reflection. Delete the replaced global
   fields and authority in the same cutover; do not retain defaults/overrides for unpublished
   compatibility. A descriptive metadata key cannot control conversion behavior.
2. Expose existing provenance relationships as generation-checked borrowed readers; do not create
   a second DAG or make callers infer source identity from canonical indices or display names.
3. Add IOR/volume, additional light semantics, interpolation segments or other vertex animation
   only with a producer that needs them. Map, validate, read and test one slice completely.
4. Keep application packing, world bounds and draw constraints outside source-preserving Scene
   mutation unless a real reusable library consumer establishes that boundary.

The first implementation unit should close PLY's admitted-color policy mismatch, followed by
accepted MTL texture semantics and explicit handling of recognized OBJ/PLY omissions. Then deliver
deterministic indexed application buffers and public source tracing; add larger workload and
edit-lifecycle evidence alongside these changes. Tangent ownership must precede generation.
Core glTF mode/accessor coverage is a better next format investment than starting FBX or USD.

Do not add a renderer, exporter framework, arbitrary animation runtime, generic asset compiler,
receipt system, source-shape proof machinery or Assimp compatibility facade to complete this plan.
High-precision CAD, independent skeleton assets, archives/async adapters, FBX, VRM and USD remain
separate product decisions under the roadmap. Assimp is a comparative input and optional external
oracle; independent source-semantic assertions remain the correctness authority.
