# Real glTF asset fixtures

Eight unchanged models (seven small and one medium) from the official
[Khronos glTF-Sample-Assets repository](https://github.com/KhronosGroup/glTF-Sample-Assets),
pinned to commit
[`8f492390ddc7a2ae221a1369ef560589cf8c70fb`](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/8f492390ddc7a2ae221a1369ef560589cf8c70fb).
The 59 vendored files total 645,140 bytes. All primary files use the ordinary
`glTF/` variant with original external resources; every primitive uses TRIANGLES
(mode 4, sometimes implicit).

| Model and primary file | Coverage | Expected outcome |
| --- | --- | --- |
| [BoxTextured](upstream/Models/BoxTextured/glTF/BoxTextured.gltf) | Static cube, UVs, external buffer and PNG, material/texture binding | Success |
| [BoxInterleaved](upstream/Models/BoxInterleaved/glTF/BoxInterleaved.gltf) | POSITION and NORMAL share a buffer view with 24-byte stride and different offsets | Success |
| [SimpleMeshes](upstream/Models/SimpleMeshes/glTF/SimpleMeshes.gltf) | Two nodes instance one indexed triangle mesh; the second node is translated | Success |
| [SimpleSkin](upstream/Models/SimpleSkin/glTF/SimpleSkin.gltf) | Two-joint skin, inverse bind matrices, LINEAR joint rotation animation, four external buffers | Reject quaternion values under Baozi policy |
| [RiggedSimple](upstream/Models/RiggedSimple/glTF/RiggedSimple.gltf) | Two-joint skin, explicit skeleton root, inverse bind matrices, mixed influences, LINEAR joint TRS animation | Success |
| [AnimatedMorphCube](upstream/Models/AnimatedMorphCube/glTF/AnimatedMorphCube.gltf) | Two morph targets with position/normal/tangent deltas and LINEAR weight animation | Success |
| [MorphPrimitivesTest](upstream/Models/MorphPrimitivesTest/glTF/MorphPrimitivesTest.gltf) | Two textured primitives, one shared morph slot, default weight 0.5, preserved asset extras | Success |
| [CesiumMan](upstream/Models/CesiumMan/glTF/CesiumMan.gltf) | Medium textured character, 19-joint skin, 57 LINEAR TRS channels, external buffer and JPEG | Success |

The [selected profile](../../../../docs/formats/gltf.md) expects six small
successes and one rejection in [expectations.json](expectations.json), plus one
medium success in [workload-expectations.json](workload-expectations.json).
These expectations come from source inspection; no Baozi importer was run to
produce them.

`MorphPrimitivesTest` preserves `/asset/extras` as source-owned `Bytes` metadata
under `org.baozi.gltf:asset-extras`, with media type `application/json`. The
payload retains the exact original JSON value bytes, including whitespace.
General object extras use `org.baozi.gltf:extras` on their source owners. This
metadata uses a source-object owner independently of canonical Scene entity metadata.
The recorded title, author,
and literal license value are source facts; upstream attribution and provenance
remain unchanged.

`SimpleSkin` is retained unchanged as a rejection fixture for Baozi's stricter
canonical quaternion policy: `abs(norm2 - 1) <= 1e-4`. Direct binary decoding
finds rotation keys approximately `(0, 0, 0.383, 0.924)` with squared norm
`1.0004650354123124` and `(0, 0, 0.707, 0.707)` with squared norm
`0.999698048545838`. Both deviations exceed that tolerance, so the expected
failure is `gltf.invalid_document` with a non-unit animation rotation reason.
This documents Baozi's admission policy, not a general claim that the official
model is invalid. Neither the upstream bytes nor Baozi's tolerance are changed.

`RiggedSimple` provides successful skeletal animation coverage within the
selected profile. Its five source nodes contain one mesh instance at node 2,
bound to skin 0 with joint nodes `[3, 4]` and skeleton root 3. The mesh has
160 vertices and 564 indices. All three LINEAR channels target joint node 4
(translation, rotation, scale), each with 50 keys over the authored, nonzero
time domain `[0.04166661947965622, 2.0833330154418945]`. All 50 rotation keys
satisfy the quaternion tolerance; the maximum absolute squared-norm deviation
is about `1.00018e-7`. Independent inspection found no extras, extension
payloads, or sparse accessors. The six added upstream files total 24,109 bytes.

[expectations.json](expectations.json) records independently readable source
counts, accessor bounds, names, instance/skin relationships, and animation data
with JSON Pointer evidence. `RiggedSimple` additionally records every input
time, representative directly decoded channel values and vertex influences,
both column-major inverse bind matrices, quaternion norm statistics, and the
pinned tree's blob sizes and Git SHA-1 values. Node counts include all selected **source** nodes
and exclude any importer-generated root. Primitive definitions are counted once
per mesh, not once per instance. Morph slots are counted once per mesh, not once
per primitive. Position bounds describe the base mesh in local coordinates,
before node transforms, skinning, or morph weights. Compare floating-point
values with an appropriate tolerance. `SimpleMeshes` actually contains POSITION
and NORMAL only, despite the upstream summary mentioning texture coordinates.

Files under `upstream/` retain their upstream paths and exact bytes. Each model
includes its original `README.md`, `README.body.md`, `LICENSE.md`, and
`metadata.json`. Repository `README.md`, `REUSE.toml`, and the applicable full
license texts are also retained. Screenshots and alternative format variants
are omitted. Upstream Markdown links remain unchanged and may reference those
omitted files or the moving `main` branch; use the pinned commit above for source
inspection. The copied `REUSE.toml` describes the original upstream tree.

Model attribution and license details are in those original notices:

- BoxTextured, BoxInterleaved, and RiggedSimple: Cesium, CC-BY-4.0; the existing
  [full license text](upstream/LICENSES/CC-BY-4.0.txt) also covers RiggedSimple.
  BoxTextured additionally
  retains the Cesium logo notice in
  [LicenseRef-LegalMark-Cesium.txt](upstream/LICENSES/LicenseRef-LegalMark-Cesium.txt).
- CesiumMan: 2017 Cesium, CC-BY-4.0, with the original Cesium logo notice
  (2015 Cesium) and the same retained full license and mark texts. Model metadata
  describes the logo as non-copyrightable; the mark notice remains applicable.
  Original notices are retained byte for byte alongside the model. The model
  license does not grant logo usage rights in other contexts.
- SimpleMeshes and SimpleSkin: Marco Hutter; model content CC0-1.0.
- AnimatedMorphCube: Microsoft; model content CC0-1.0.
- MorphPrimitivesTest: ft-lab, with Frank Galligan's upstream Draco credit
  preserved; CC-BY-4.0. Only the uncompressed variant is included.
- Metadocumentation: CC-BY-4.0 as stated in each original `LICENSE.md`; repository
  notices additionally credit the Khronos Group.

On 2026-10-31, `gh api` verified the exact commit and complete Git tree
`936af0f6e5b471fefd6fc1d84a42328b9ce70978` through the official GitHub REST API.
Every downloaded blob's size and Git blob SHA-1 matched that tree before writing.
The commit is **unsigned**; this is content/provenance verification, not a claim
of signature verification. The integrating fixture owner maintains
[SHA256SUMS](SHA256SUMS) as the inventory changes. The workload snapshot separately
records byte lengths, Git blob SHA-1 and SHA-256 for all seven CesiumMan files;
each size and Git blob SHA-1 was independently matched against the supplied
pinned tree. The CesiumMan files total 464,728 bytes.
The unchanged `.gitattributes` rule `upstream/** -text` prevents Git newline
conversion of all upstream bytes, including every CesiumMan file.

## V1 workload definitions and regressions

[workload-expectations.json](workload-expectations.json) defines exactly three
families. The first two are original generated project test content; the third
uses the unchanged vendored CesiumMan files and their upstream notices.

| Family | V1 sizes | Independent source expectations |
| --- | --- | --- |
| Static quad grid OBJ | 128 and 256 cells per side | `(N+1)^2` source and indexed draw vertices; `N^2` quads; `6*N^2` triangulated corners: 98,304 and 393,216 |
| Shared-resource glTF primitives | 8 and 32 primitives | Fixed four positions and six indices per primitive; one mesh, two instances, two materials sharing one image; `4*P` draw vertices and `6*P` draw indices |
| Unchanged CesiumMan | One medium model | 3,273 referenced vertices, 14,016 indices, 22 nodes, one mesh instance, one skin with 19 joints, one clip with 57 channels and 48 keys per channel, one material and one JPEG |

The OBJ grid is planar at Z=0, uses integer row-major positions and CCW quad
faces, declares smoothing group 1, and omits normals, UVs and materials. OBJ
does not encode units or axes. The caller must explicitly assume right-handed,
+Y up, -Z front, meters and CCW fronts (`TargetSpace::STANDARD`). Conversion to
the consumer's default space reflects X and scales positions by 100; generated
normals are +Z. The workflow exercises triangulation, space conversion, normal
generation and indexed buffer extraction. Its indexed vertex counts are 16,641
and 66,049; the much larger corner counts are index counts, not vertex counts.

The generated glTF cases keep the same 92-byte geometry payload at both sizes.
Every primitive shares POSITION, UV and index accessors and deliberately omits
normals, requiring a separate normal edit for each primitive. Alternating
materials bind one texture and one original 1x1 PNG. Two nodes instance the same
mesh; the second has an authored two-meter X translation. glTF supplies its own
axis/unit conventions, so no source assumption is needed. Default conversion
reflects Z; generated normals are -Z, and the translated node moves to X=200 cm.
Expected source opens stay at three, independently of primitive count. Draw
counts and mesh-local bounds do not multiply by the two instances.

Generate inputs into a new or empty directory from the repository root:

```text
python scripts/generate_asset_workloads.py --output-dir target/asset-workloads-v1
```

Use `python3` if that is the interpreter name on your platform. The generator
uses only Python's standard library, makes no network requests and does not
build, import or measure anything. It emits OBJ/glTF resources and
`manifest.json` with sizes, construction-based expected counts, explicit source
assumptions and per-input SHA-256. Paths are relative, output is deterministic
and existing output files are never replaced. The manifest excludes its own
checksum. CesiumMan is referenced in the checked-in workload expectations and
is not copied into generated output.

Repeat `--grid-size` or `--primitive-count` to override the two defaults for a
family. Small test generation uses `--grid-size 2 --grid-size 4`; shared-resource
CI cases use the default 8/32 primitives. If later profiling needs more
per-primitive overhead, `--primitive-count 64 --primitive-count 256` increases
primitive count while retaining the exact same geometry and image bytes per
case. This adds sizes within the same family, not another workload family.

[scale_workflows.rs](../tests/scale_workflows.rs) runs small generated cases and
CesiumMan as ordinary semantic regressions. The tests invoke Python (`python`
on Windows, `python3` elsewhere); `BAOZI_WORKLOAD_PYTHON` may name a different
interpreter executable. Generated inputs live in an exclusively created
temporary directory. The tests remove that directory and drop detached import
evidence before reading retained application buffers, exercising buffer and
image ownership after Scene destruction.

```text
cargo nextest run --locked --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --test scale_workflows --test-threads 1
```

The 128/256 grid regression is ignored by default and has no timing gate. Run
it manually, with one test at a time:

```text
cargo nextest run --locked --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --test scale_workflows --test-threads 1 --run-ignored all -E 'test(generated_v1_scale_workflows)'
```

CesiumMan expectations use its source JSON and directly decoded little-endian
U16/F32 accessors, including byte offsets and byte strides. All 3,273 source
vertices are referenced. There are four influence slots per vertex (13,092
slots total, including zero weights); 7,645 weights are positive. Active
influence counts per vertex are 458/1,678/717/420 for one/two/three/four active
weights. No influences or animation values were normalized or truncated.
The 19 input-time accessors contain identical 48-key sequences over
`[0.04166661947965622, 2.0]`; the authored start time is retained. Direct binary
inspection finds all 912 rotation keys within `abs(norm2 - 1) <= 1e-4`.

The snapshot records complete input times, source accessor IDs, joint order,
representative influence rows, bind matrices and animation keys, plus JSON
Pointers. The Rust regression directly compares every influence row, all 19
inverse bind matrices and every key of all 57 channels to the source data,
applying independently specified spatial arithmetic. It reconstructs each
indexed triangle after the reflection, preserving positions, normals, UVs,
material/image bindings and JPEG bytes after Scene and import evidence drop.
Inverse binds use `C*M*C^-1` for `C=diag(100,100,-100,1)`. Translation keys scale
and reflect; quaternion keys map `(x,y,z,w)` to `(-x,-y,z,w)` without shifting
times or changing scale keys.

These are workload definitions and semantic regressions, with no performance
thresholds. Executed observations, including their exact implementation and
measurement scopes, are recorded in the
[expansion review](../../../../docs/reviews/2026-11-01-asset-workflow-expansion.md).
Planar grids do not cover arbitrary polygons or hard
edges. Shared quad primitives grow primitive overhead, not unique geometry or
image content. CesiumMan covers one LINEAR skeletal clip; it does not add morph,
sparse, compression, cubic spline, sampled playback or rendering coverage.
Encoded image preservation does not test image decoding. The small source
decoder intentionally supports only this dense U16/F32 fixture layout.
The production workflow also traverses each primitive's retained provenance
through the public reader, and regressions require reached owners and source
objects after edits. `SHA256SUMS` covers these workload definitions and the
unchanged upstream files.
