# Assimp Replication Study for Baozi

**Date**: 2026-07-08
**Status**: Draft for discussion
**Reference source**: `repo-ref/assimp`

## Executive Summary

Assimp's strength comes from four architectural commitments:

1. A broad but stable common scene IR.
2. A registry of independent format importers.
3. A pluggable IO layer that lets importers resolve related files.
4. An ordered post-processing pipeline shared by every format.

Baozi should replicate these commitments in Rust instead of replicating Assimp's C++ memory layout. The recommended direction is a Rust workspace with a safe owned scene model, id-based cross references, importer traits, virtual IO, and deterministic post-processing.

## License Assessment

Assimp's root `LICENSE` is a modified BSD / BSD-3-Clause style license. Its own README summarizes the practical effect as permissive use with license text retention, and explicitly notes that static linking is allowed. For Baozi this means:

- Studying Assimp architecture is fine.
- Comparing Baozi behavior against Assimp is fine.
- Porting concrete source code or algorithms is possible only if the derived files keep the required Assimp copyright and license notices.
- Baozi should not use Assimp's project name or contributors as endorsement.
- Assimp test assets must be handled separately from source code. `repo-ref/assimp/test/models-nonbsd` contains non-BSD model assets with additional source-specific requirements.

Recommended project policy: keep Baozi code independently authored by default, use Assimp as an oracle and reference, and create `THIRD_PARTY_NOTICES.md` before importing any Assimp-derived code or fixtures.

For crate licensing, the recommended default is `MIT OR Apache-2.0` for clean-room Baozi crates. That remains viable as long as Baozi does not copy, translate, or mechanically derive Assimp implementation code. If a crate includes Assimp-derived code, express that crate as mixed-license, for example `(MIT OR Apache-2.0) AND BSD-3-Clause`, and keep the Assimp notice with the derived code and in third-party notices.

## Assimp Core Abstractions

### Scene IR

`aiScene` is the common product of every importer. It owns or references:

- root hierarchy: `aiNode`
- geometry: `aiMesh`
- materials: `aiMaterial`
- animation: `aiAnimation`
- embedded textures: `aiTexture`
- lights and cameras
- metadata and skeletons

Evidence:

- `repo-ref/assimp/include/assimp/scene.h`
- `repo-ref/assimp/code/Common/scene.cpp`

### Node Graph

`aiNode` stores a local transform, parent pointer, child pointers, mesh indices, name, and metadata. Mesh references are indices into the scene mesh list.

Baozi should keep the same graph concept, but replace pointers with ids and owned vectors.

Evidence:

- `repo-ref/assimp/include/assimp/scene.h`

### Mesh

`aiMesh` uses one material index per mesh. Multi-material source meshes are split. It supports positions, normals, tangents, bitangents, up to 8 color sets, up to 8 UV channels, faces, bones, morph targets, bounding boxes, and texture coordinate names.

Baozi should keep the one-material-per-mesh invariant because it simplifies post-processing and rendering-oriented consumers.

Evidence:

- `repo-ref/assimp/include/assimp/mesh.h`

### Material

Assimp materials are property tables keyed by name, semantic texture type, and texture index. This lets old Phong-style data and modern PBR data coexist.

Baozi should expose typed common material fields, but keep an extensible property table so format-specific data is not discarded.

Evidence:

- `repo-ref/assimp/include/assimp/material.h`
- `repo-ref/assimp/code/Common/material.cpp`

### Importer Facade

`Assimp::Importer` owns IO, progress, configuration properties, importers, post-process steps, the current scene, and error state. It can read from files or memory, register custom loaders, apply post-processing, list importers, and inspect supported extensions.

Baozi should expose a facade with similar capability, but return `Result<Scene, BaoziError>` instead of tying scene lifetime to the importer object.

Evidence:

- `repo-ref/assimp/include/assimp/Importer.hpp`
- `repo-ref/assimp/code/Common/Importer.cpp`

### Format Importers

`BaseImporter` defines:

- `CanRead`
- `SetupProperties`
- `GetInfo`
- `InternReadFile`

`ReadFile` wraps `InternReadFile`, catches errors, constructs the scene, and applies shared file-system filtering.

Evidence:

- `repo-ref/assimp/include/assimp/BaseImporter.h`
- `repo-ref/assimp/code/Common/BaseImporter.cpp`

### IO

`IOSystem` abstracts file existence, open/close, path comparison, directory stack, and directory/file operations. Importers use it instead of direct filesystem access.

Baozi should use an `AssetIo` trait returning `Read + Seek` streams, plus source identity and related-file resolution.

Evidence:

- `repo-ref/assimp/include/assimp/IOSystem.hpp`
- `repo-ref/assimp/include/assimp/IOStream.hpp`

### Post-Processing

Post-processing is activated by flags but executed in registry order. Examples include triangulation, normal generation, vertex joining, optimization, validation, coordinate conversion, UV transforms, bone limits, and bounding boxes.

Baozi should encode ordering as part of the pipeline, because user flag order is not enough.

Evidence:

- `repo-ref/assimp/include/assimp/postprocess.h`
- `repo-ref/assimp/code/Common/PostStepRegistry.cpp`
- `repo-ref/assimp/code/Common/BaseProcess.h`

## Import Lifecycle

Assimp's file import flow:

1. Delete any previous scene.
2. Check file existence through `IOSystem`.
3. Find possible importers by file extension.
4. If multiple importers match, call `CanRead` for signature-based detection.
5. If extension matching fails, try signature-based detection across all importers.
6. Call the selected importer's `ReadFile`.
7. Attach source-format metadata.
8. Optionally validate first.
9. Preprocess the scene.
10. Apply requested post-process steps in registry order.
11. Store errors if import failed.

Evidence:

- `repo-ref/assimp/code/Common/Importer.cpp`

Baozi should preserve this logic, but make it explicit in a Rust `Importer` session:

- candidate discovery
- probe
- import
- build
- validate
- post-process
- return owned scene

## Format Ecosystem Observations

`ImporterRegistry.cpp` registers a large list of format workers, including USD, X, OBJ, AMF, 3DS, M3D, MD2/MD3/MD5, PLY, STL, LWO, DXF, OFF, AC3D, BVH, Collada, Ogre, Blender, IFC, FBX, Assbin, glTF1, glTF2, 3MF, X3D, MMD, and IQM.

The simple and complex format importers differ sharply:

- STL is a compact geometry importer with ASCII/binary detection and a generated default material.
- OBJ exercises text parsing, MTL sidecars, relative path handling, groups, objects, materials, normals, and UVs.
- PLY exercises structured header parsing and binary/text variants.
- glTF2 exercises modern PBR, buffers, images, skins, animations, and extensions.
- FBX has extensive format-specific settings and reads the file into a dedicated parser/converter pipeline.

Evidence:

- `repo-ref/assimp/code/Common/ImporterRegistry.cpp`
- `repo-ref/assimp/code/AssetLib/STL/STLLoader.cpp`
- `repo-ref/assimp/code/AssetLib/Obj/ObjFileImporter.cpp`
- `repo-ref/assimp/code/AssetLib/Ply/PlyLoader.cpp`
- `repo-ref/assimp/code/AssetLib/glTF2/glTF2Importer.cpp`
- `repo-ref/assimp/code/AssetLib/FBX/FBXImporter.cpp`

## Rust Ecosystem Survey

The current Rust ecosystem helps, but does not remove the need for a Baozi-owned IR and parser policy.

### Existing Assimp bindings

Crates found: `assimp`, `russimp`, `russimp-ng`, `assimp-sys`, `russimp-sys`.

Use them only as optional compatibility or differential-test tools. They wrap C/C++ Assimp and therefore do not satisfy the goal of a Rust-native reimplementation.

### Format-specific parser crates

| Format | Crates observed | Practical recommendation |
| --- | --- | --- |
| glTF2/GLB | `gltf` 1.4.1 | Strong candidate as first backend or reference; still normalize to Baozi IR immediately |
| OBJ/MTL | `tobj` 4.0.4, `wavefront-obj-io` | `tobj` is useful; a Baozi self-written streaming parser is also reasonable for diagnostics and MTL control |
| STL | `stl_io` 0.11.0 | Good accelerator; STL is small enough to self-write if diagnostics or exact behavior matter |
| PLY | `ply-rs` 0.1.3, `ply-rs-bw` | Evaluate carefully; PLY's flexible property model may be better served by a Baozi parser |
| Collada | `collada` 0.17.0, `dae-parser` | Useful references, but Collada normalization is large enough that Baozi should own the conversion layer |
| FBX | `fbxcel`, `fbx`, `fbx_direct` | Treat as low-level references; Assimp-like FBX support likely requires Baozi's own tokenizer/DOM/converter |
| USD | `rust-usd`, `usd`, `oxideav-usdz` | Defer. Prefer a backend abstraction before choosing OpenUSD FFI or pure Rust crates |
| 3MF | `lib3mf-core`, `threemf` | Evaluate, but a direct `zip` + XML implementation is realistic |

### Infrastructure and post-processing crates

- `quick-xml`: strong XML reader/writer candidate for Collada, 3MF, X3D, and XML-heavy sidecars.
- `zip`: natural infrastructure for 3MF, USDZ, and future archive-backed assets.
- `nom`, `binrw`, `byteorder`: useful parser building blocks.
- `mikktspace`: useful tangent-space generation backend.
- `meshopt`: useful for cache optimization and mesh simplification behind optional features.
- `draco-core`: possible pure Rust Draco backend for glTF `KHR_draco_mesh_compression`, but should be feature-gated.

### Parser ownership stance

Baozi should be comfortable writing parsers when crate maturity is weak or format behavior matters. The decision rule is:

1. Self-write small or behavior-sensitive parsers: STL, OBJ scanner, PLY, custom binary chunks.
2. Wrap mature spec-driven parsers when they accelerate coverage: glTF2 is the best candidate.
3. Use low-level crates for complex formats, but own the semantic conversion: FBX, USD, Collada, IFC.
4. Never expose third-party parser types from Baozi's public API.
5. Record every format backend decision with license, feature coverage, and replacement cost.

## Recommended Baozi Roadmap

### Phase 0: Workspace and Contracts

Goal: create the architecture before chasing format count.

Deliverables:

- Rust workspace with crates described in ADR 0001.
- `Scene`, `SceneBuilder`, ids, mesh/material/node basics.
- `AssetIo` trait and filesystem/memory implementations.
- `FormatImporter` trait and registry.
- `Importer` facade with `read_file` and `read_memory`.
- Structured errors and diagnostics.
- `ValidateScene` processor.

Validation:

- `cargo fmt`
- `cargo nextest run`
- Unit tests for ids, builder invariants, IO, registry selection, and validation failures.

### Phase 1: First Closed Import Loop

Goal: prove bytes can become a validated `Scene`.

Formats:

- STL: first geometry-only importer.
- OBJ plus MTL: first multi-file text importer.
- PLY: first header-driven text/binary importer.

Post-process:

- ValidateScene
- Triangulate
- GenerateBoundingBoxes
- GenerateNormals

Validation:

- Golden snapshots for small fixtures.
- Virtual IO test for OBJ plus MTL.
- Error snapshots for unsupported format and malformed files.

### Phase 2: Modern Asset Baseline

Goal: become useful for modern engines.

Formats:

- glTF2 `.gltf`
- glTF2 `.glb`

Capabilities:

- PBR materials
- external and embedded buffers
- images/textures
- node hierarchy
- skins and basic animations
- extension-preserving metadata

Validation:

- Compare against Khronos sample assets.
- Snapshot scene summaries, not raw floating-point dumps.

### Phase 3: Post-Processing Depth

Goal: make imported output ergonomic for renderers and tools.

Processors:

- JoinIdenticalVertices
- SortByPrimitiveType
- SplitLargeMeshes
- LimitBoneWeights
- OptimizeMeshes
- TransformUVCoords
- Coordinate handedness conversion

Validation:

- Processor-level unit tests.
- Pipeline-order tests.
- Property-based tests for index validity after mutations.

### Phase 4: Complex Legacy Formats

Goal: expand toward Assimp breadth after core invariants are proven.

Candidate order:

1. Collada
2. FBX
3. 3DS
4. MD2/MD3/MD5/IQM
5. BVH
6. 3MF
7. X3D

These should not block the first useful release.

### Phase 5: Export, FFI, and Tooling

Goal: complete the ecosystem.

Deliverables:

- Exporter traits and selected exporters.
- `baozi-ffi` C ABI compatibility surface.
- CLI inspection tool.
- Differential comparison tooling against Assimp for supported fixtures.

## Minimal Closed Loop

The first milestone should be:

```text
read_file("triangle.stl", options)
  -> select STL importer
  -> parse geometry
  -> build SceneBuilder
  -> validate Scene
  -> generate bounding boxes
  -> return Result<Scene, BaoziError>
```

The second milestone should be:

```text
read_file("materialized.obj", options)
  -> resolve sibling .mtl through AssetIo
  -> produce nodes, meshes, materials, UVs, normals
  -> validate material and mesh indices
  -> return Scene with diagnostics
```

This pair exercises both single-file and multi-file import without entering the complexity of glTF2 or FBX too early.

## Discussion Questions

1. Is Baozi's goal API compatibility with Assimp, behavior compatibility, or "same breadth with Rust-native API"?
2. Should first-party format crates be allowed to wrap existing parser crates when that accelerates coverage?
   Recommendation: yes, but only behind Baozi-owned traits and only after recording license, maturity, and replacement risk.
3. Should `baozi-core` depend on a math crate, or define minimal vector/matrix types to avoid dependency lock-in?
4. Should material keys intentionally mirror Assimp names for familiarity?
5. Should `repo-ref/assimp` remain in the repository as a local reference, and should it be added to `.gitignore` if it is not meant to be committed?

## Source Map

| Topic | Evidence |
| --- | --- |
| Public facade | `repo-ref/assimp/include/assimp/Importer.hpp` |
| Import lifecycle | `repo-ref/assimp/code/Common/Importer.cpp` |
| Format trait | `repo-ref/assimp/include/assimp/BaseImporter.h` |
| Format read wrapper | `repo-ref/assimp/code/Common/BaseImporter.cpp` |
| Importer registry | `repo-ref/assimp/code/Common/ImporterRegistry.cpp` |
| Post-process registry | `repo-ref/assimp/code/Common/PostStepRegistry.cpp` |
| Post-process trait | `repo-ref/assimp/code/Common/BaseProcess.h` |
| Scene IR | `repo-ref/assimp/include/assimp/scene.h` |
| Mesh IR | `repo-ref/assimp/include/assimp/mesh.h` |
| Material system | `repo-ref/assimp/include/assimp/material.h` |
| IO abstraction | `repo-ref/assimp/include/assimp/IOSystem.hpp` |
| Post-process flags | `repo-ref/assimp/include/assimp/postprocess.h` |
| STL importer | `repo-ref/assimp/code/AssetLib/STL/STLLoader.cpp` |
| OBJ importer | `repo-ref/assimp/code/AssetLib/Obj/ObjFileImporter.cpp` |
| PLY importer | `repo-ref/assimp/code/AssetLib/Ply/PlyLoader.cpp` |
| glTF2 importer | `repo-ref/assimp/code/AssetLib/glTF2/glTF2Importer.cpp` |
| FBX importer | `repo-ref/assimp/code/AssetLib/FBX/FBXImporter.cpp` |
