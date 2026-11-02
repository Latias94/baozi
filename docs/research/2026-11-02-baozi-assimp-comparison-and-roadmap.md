# Baozi versus Assimp: current capabilities and development route

Date: 2026-11-02, Asia/Shanghai.

## Decision and scope

Assimp has substantially broader format, processing, export and integration reach. Baozi now has a
usable Rust import/processing foundation for four explicit profiles, with a different contract for
ownership, source fidelity, resource admission and immutable Scene publication. Deliver and exercise
that foundation, then extend workflows that real callers need. Current evidence does not require
another wholesale replacement of the Scene architecture.

This is recommended sequencing, not a new implementation mandate or an expanded first-release
gate. It does not authorize publishing, yanking, native promotion or a new development Goal. The
completed U1-U8 work is not reopened.

- Baozi baseline: `36e4c6d9cf2a6f04e5076c47a43d8d7ac46f1690`; qualified implementation:
  `6fb9112cb75e1dc942f6d7e605aa9e7b23ec1559`.
- Assimp: local `repo-ref/assimp` at `a2a586d9370a493651256ec5d03524710ce37a5f`, described as
  `v6.0.5-179-ga2a586d93`. The official [v6.0.5 release page][a-release] was marked Latest during
  this review. The inspected development revision is later; its behavior is not automatically
  attributed to the tag.
- Method: source inspection, primary-source research and two independent native Codex review lanes.
  No Cargo checks, renderer, Assimp executable or comparative benchmark ran for this review.
  Existing results retain their scope in the [maturation record](../reviews/2026-11-02-first-release-maturation.md)
  and [portable candidate](../release/portable-0.1.0-candidate.md).

## Current comparison

Implementation existence is not complete conformance. Optional Assimp backends/tools depend on its
build. Baozi core vocabulary does not count as importer support without mapping and validation.

| Area | Baozi now | Assimp at the pin | Consequence |
| --- | --- | --- | --- |
| Import breadth | STL, OBJ/MTL, PLY and selected glTF/GLB profiles. | Dozens of import formats, including FBX, Collada, BVH and PMX; USD is experimental and opt-in. [Registry][a-import], [inventory][a-formats]. | A substantial gap; choose a demanded profile rather than a format-count target. |
| Scene/ownership | One validated immutable Scene, generation-qualified typed reads, meshes/primitives/instances and owned relations; explicit Vertex/Corner/Face domains. | Public C/C++ node/mesh pointer/array structures. Importer-owned scenes can be orphaned to transfer ownership. [Mesh][a-mesh], [ownership][a-owner]. | Basic ownership/navigation are delivered. Adapt foreign layouts at the edge if needed. |
| Geometry/precision | Seven glTF modes, sparse accessors, declared F32/F64 geometry, F64 spatial queries and precision-preserving edits. | Face-based output; build-wide `ai_real`, normally float with a double option. [Scalars][a-scalar], [option][a-cmake]. | Neither universal F64 nor a numerical-quality ranking follows; preserve source-width contracts. |
| glTF materials/lights | Metallic-roughness, unlit and texture transforms. Core light types exist; punctual-light import and other PBR extension mappings do not. | Broader material mappings, including clearcoat, sheen, transmission, volume, IOR and emissive strength; punctual lights. [Implementation][a-gltf]. | Useful common mappings are a near-term feature lane. |
| Images/compression | Encoded images and sampler interpretation survive extraction; no pixel decoding or compressed-geometry extensions. | Optional Draco decode and BasisU/WebP image-source routing. Routing is not pixel transcoding. [Implementation][a-asset]. | Treat geometry decode, image routing and pixel decode separately. |
| Animation/skin/morph | Full influences, skin/morph/TRS curves, explicit time/interpolation/extrapolation and an application-owned CPU reference evaluator. | Broad producer coverage; this pin includes TRS interpolation tags and cubic triplets. [Types][a-animation], [lowering][a-gltf]. | Evaluation is not missing; extend semantics for a selected producer or consumer. |
| Processing | Selected triangle preparation, triangulation, normals, Triangle UV Derivative tangents and whole-Scene space conversion. | Also joining, draw/bone splitting, cache optimization, UV operations, hierarchy baking and influence limiting. [Passes][a-post]. | Device/output preparation is a gap; basic batching is delivered. |
| Application use | Chosen material/UV/image/sampler profile, indexed F64 output and detached evaluation. The example omits lights, cameras, other color sets, custom fields and full provenance. | C APIs and command-line tooling alongside its mesh projection. [C API][a-capi], [CLI][a-cli]. | Prove a real renderer workflow; the example is not a universal renderer ABI. |
| IO/control | Portable memory/custom IO, explicit dependencies, limits/cancellation and a host-bundle example. Native filesystem IO is an unpublished unregistered candidate. | File import, seekable custom IO and progress cancellation. [Importer][a-owner], [IO][a-io]. | Improve ordinary host onboarding without weakening authority; no security ranking follows. |
| Export | No exporter; snapshots/application copies are not interchange output. ADR 0038 remains proposed. | Multiple file/blob exporters for supported subsets. [Implementation][a-export]. | A genuine gap for conversion/editor users, conditional on that audience. |
| Distribution | Ten-package portable 0.1.0 candidate has local evidence. PLY/glTF publication is disabled; managed declarations remain `Unverified`, without assigned maturity. | Existing releases and a wider integration surface. | Complete actual distribution separately from architecture expansion. |

Baozi's useful distinction is an explicit semantic pipeline: retain source facts, admit bounded work,
publish a validated immutable result, then derive caller output with declared loss. That is a design
direction, not measured proof of higher speed, safety or compatibility. Assimp remains an external
compatibility/differential oracle, not the production backend or a schema to reproduce.

## Recommended sequence

This order is a product judgment for Rust game/editor/offline tooling, not calendar estimates or a
requirement that all stages block release. Thin renderer work can start with today's profile while
release preparation or small glTF slices proceed.

### R0: deliver the current portable profile

Finish a short installation-to-output path and existing release gates. Reuse U8 archive/host
evidence rather than restarting qualification without a source change. Document features, one GLB,
an explicit sidecar bundle, unsupported outcomes and owned output after Scene destruction. Distinguish
portable memory/custom IO from the native candidate.

The current train is `0.1.0`; existing `baozi 0.0.1` is a placeholder, not a version to overwrite.
The [release policy](../contributing/release-policy.md) requires approved registry state/preflight,
deliberate PLY/glTF eligibility, dependency order, archive identity and a fresh registry-only consumer.
That consumer must download, build and execute without paths, patches or source replacement. Only
then consider separately authorized placeholder retirement. Local archives already pass; registry
availability is a different result.

Keep the first release finite. Lights, more materials, MikkTSpace, compression, a renderer, FBX and
native promotion are follow-ups, not newly imposed R0 blockers. Describe exact profile/target evidence
and promote only claims whose specified evidence requirements have actually been met.

### R1: common glTF workflows and rendered feedback

Start a thin independent renderer with current textured PBR/unlit assets. Its image codecs, clocks,
graphics devices and renderer dependencies stay in the application, outside core and the facade. Exercise
static texturing, instances, a skinned clip and a morph clip, including UV selection, texture
transforms, channel/color interpretation, samplers, alpha and sidedness. Retain named-target readback
or image observations with analytic expectations. The CPU reference evaluates positions; deformed
normal/tangent frames and shading require their own policy and evidence.

The first two bounded importer additions should be:

1. **`KHR_lights_punctual`.** Reuse core directional/point/spot types. Add root-definition and selected
   node-reference closure, source identities/scope rows, defaults and mapping before dependency IO.
   Cover shared/omitted lights, axes, photometric units, finite range and cones through reads,
   conversion and the chosen output. Current source-scope inventories do not yet have Light; an
   allowlist addition alone is insufficient. Follow the [normative extension][k-light].
2. **`KHR_materials_emissive_strength`.** Decide authored versus effective emission first. Prefer
   retaining independently authored factor and strength as typed data rather than silently changing
   the existing emissive getter or hiding behavior in metadata. Cover defaults, zero, HDR values,
   textures and representability. The [extension][k-emissive] prohibits combining it with unlit;
   test that constraint explicitly.

Each slice closes ownership/work accounting, malformed/limit/cancellation cases, provenance,
selection, canonical validation, reads, snapshots, edits and downstream use. The current application
omits lights, so consume them deliberately for a light demonstration. Renderer smoke need not wait
for either extension.

Adjudicate named compatibility rejections. `SimpleSkin` exceeds Baozi's quaternion normalization
tolerance; compare source values with the specification, [independent validation][k-validator] and
reference behavior before retaining the restriction or changing representation/policy. Silently
normalizing data or calling the official sample generally invalid is not justified by this rejection.

### R2: useful device output before optimizers

Use the renderer to select fields/domains, packed scalar types, index width, draw size and joint
palette limits. These belong outside canonical Scene. Current F64/U32/full-influence application
output is a reference choice, not a universal GPU contract.

Prioritize lossless splitting and complete vertex/palette remaps: U16 boundaries, shared meshes with
different instance skins, all selected UV/color/tangent streams, morphs, material boundaries and
signed-zero seams. Precision narrowing, influence truncation and renormalization need explicit policy
and quantitative loss where meaningful. Bound output expansion/application allocations separately;
import accounting does not cover GPU buffers. Ordinary failure returns no partial successful result.

Keep the first adapter concrete, for example a wgpu sample. Extract a reusable package after callers
demonstrate a common interface. Add cache/vertex-fetch optimization only with measured benefit.

Choose further glTF work by a named asset blocker:

- **MikkTSpace:** a separately selected algorithm with independent [reference vectors][k-mikk],
  UV seam/mirror/degenerate cases, morph restrictions and declared convention. Never relabel today's
  Triangle UV Derivative generator.
- **Quantization:** [KHR_mesh_quantization][k-quant] expands attribute encodings and uses existing
  transforms for dequantization; it is not a compressed-buffer decoder.
- **Compression:** choose the required mesh or texture codec from the corpus. Admit decoded sizes,
  input/output/scratch overlap, work and failure behavior before integration. Compare with an
  independent uncompressed reference and requalify dependencies/targets.
- **More materials:** implement full behavioral factors, bindings, resource closure, conversion,
  validation and consumer interpretation, not just enum names or retained metadata.

These are separate increments, not a single complete-glTF project. Device splitting need not wait
for them.

### R3: one new producer profile

For general DCC/game/editor exchange, investigate the documented **binary FBX 7.x game profile**.
For an avatar product, choose **VRM 1.0** first. Do not run both merely to increase format count;
the roadmap's P1 labels do not decide which actual audience goes first.

Require a downstream caller, licensed unchanged assets and a finite preserve/loss/reject matrix.
Investigate backend maintenance, licenses/isolation, transforms, pivots, inheritance, curves and
references before committing. Reuse canonical authoring/validation. VRM reuses glTF parsing but
requires its own behavior; generic glTF success is not VRM support.

If current types cannot represent a required producer semantic, change those precise types or narrow
the profile explicitly. Do not silently resample/decompose to fit current TRS curves or invent a
generic curve/property framework before real source assets require it.

### Later: outbound workflows and measured scale

A converter/editor caller can justify one GLB exporter tracer: representability/loss preflight and
complete buffered output first, under the [proposed export contract](../adr/0038-export-transaction-and-loss-contract.md).
Native atomic replacement and multi-resource guarantees are separate capabilities. An inspect/
validate CLI can be useful earlier without becoming an asset-compiler framework.

C ABI, dynamic plugins, resolved USD, durable caches, async parsing and out-of-core storage remain
demand-driven. Synchronous chunk IO plus a primary snapshot is not streaming parsing. Change storage
when measured workloads exceed today's model and simpler copying/batching cannot meet the need.
Durable caches also require stable cross-session closure identity, not session-opaque IDs.

Native IO distribution is independent. If host-file convenience blocks adoption, qualify the smallest
adapter on an exact demanded target through existing admission checks and normal package edges.
Keep portable IO and repository capture separate; another host's tests or an archive rehearsal do
not establish a native release target.

## Fearless refactoring boundaries

Replace a wrong invariant when a real capability exposes it. A working architecture need not be
discarded on a schedule.

| Boundary | Change when needed | Preserve |
| --- | --- | --- |
| Materials | Add missing typed behavior. As several lobes become real, replace the growing supplemental-factor constructor with coherent typed optional groups. IOR/volume attenuation are not represented today. | One semantic authority, explicit source/effective meaning; avoid untyped bags and extension-combination enum variants. |
| Codecs | Private adapters with decoded-output/scratch admission. Native/isolated codecs require a separate boundary and decision; disabling them by default does not admit them to the portable facade. | Backend ownership, central validation and portability of the complete public facade/IO feature union under ADR 0007. |
| Device output | Concrete profiles and complete draw/vertex/palette correspondence. | Full canonical influences, source precision/identity and attribute domains. |
| New-format animation | Producer-backed curve/transform/target changes with validation, reads and edit semantics in the same slice. | Explicit time/interpolation/extrapolation; no silent baking during import. |
| Large assets | A measured storage/lifetime change if representative workloads require it. | One Scene lifecycle; no speculative second canonical model or scheduler. |

F64 geometry, owned glTF decoding, relations, selected transactions, material assignment distinctions,
chosen material extraction and reference evaluation are completed foundations, not current gaps.

## Evidence with every feature

Extend existing licensed fixtures, semantic tests, minimized regressions and bounded fuzz/scale runs
with each slice. Use pinned Assimp in a subprocess for selected differential cases, plus independent
expected values, official validation and rendered feedback. Classify disagreement as a Baozi bug,
Assimp bug, unsupported semantics or deliberate policy. Assimp output alone is not ground truth.
No general differential framework, source-shape proofs or script-based semantic compiler is required.

For performance comparison, match input features, precision, processing, output, builds and hardware.
Keep stage time, allocator requests, Baozi ledgers and process RSS separate. Unequal test counts and
unmatched workloads establish no parity percentage or reliability/performance winner.

The candidate records Windows/Linux tests and unchanged archives, WASI execution/snapshots, browser
compilation and bounded Linux ASan campaigns. It does not establish current Apple execution, browser
runtime, registry availability, native promotion or exhaustive correctness. Decoder changes need fresh
relevant evidence at their own source revision.

[a-release]: https://github.com/assimp/assimp/releases/tag/v6.0.5
[a-import]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/Common/ImporterRegistry.cpp
[a-formats]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/doc/Fileformats.md
[a-mesh]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/mesh.h
[a-owner]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/Importer.hpp
[a-scalar]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/defs.h
[a-cmake]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/CMakeLists.txt
[a-gltf]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/glTF2/glTF2Importer.cpp
[a-asset]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/AssetLib/glTF2/glTF2Asset.inl
[a-animation]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/anim.h
[a-post]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/postprocess.h
[a-capi]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/cimport.h
[a-cli]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/tools/assimp_cmd/code/Main.cpp
[a-io]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/IOSystem.hpp
[a-export]: https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/code/Common/Exporter.cpp
[k-light]: https://github.com/KhronosGroup/glTF/blob/main/extensions/2.0/Khronos/KHR_lights_punctual/README.md
[k-emissive]: https://github.com/KhronosGroup/glTF/blob/main/extensions/2.0/Khronos/KHR_materials_emissive_strength/README.md
[k-quant]: https://github.com/KhronosGroup/glTF/blob/main/extensions/2.0/Khronos/KHR_mesh_quantization/README.md
[k-validator]: https://github.com/KhronosGroup/glTF-Validator
[k-mikk]: https://github.com/mmikk/MikkTSpace
