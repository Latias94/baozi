# Real glTF asset fixtures

Seven small, unchanged models from the official
[Khronos glTF-Sample-Assets repository](https://github.com/KhronosGroup/glTF-Sample-Assets),
pinned to commit
[`8f492390ddc7a2ae221a1369ef560589cf8c70fb`](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/8f492390ddc7a2ae221a1369ef560589cf8c70fb).
The 52 vendored files total 180,412 bytes. All primary files use the ordinary
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

The [selected profile](../../../../docs/formats/gltf.md) expects six successes
and one rejection, based on source inspection; no Baozi importer was run to
produce these expectations.

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
of signature verification. [SHA256SUMS](SHA256SUMS) lists per-file SHA256 for
every file in this fixture directory except the checksum manifest itself.
The unchanged `.gitattributes` rule `upstream/** -text` prevents Git newline
conversion of all upstream bytes, including every new RiggedSimple file.
