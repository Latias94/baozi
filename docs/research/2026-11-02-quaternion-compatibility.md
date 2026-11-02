# SimpleSkin Quaternion Compatibility

Date: 2026-11-02. This adjudication belongs to U5 of the [pre-release evolution plan](../plans/2026-11-02-0923-refactor-pre-release-evolution-plan.md).

## Decision and Caller Consequence

Retain the unchanged SimpleSkin asset as an explicit Baozi numeric-policy rejection.
It passes the pinned Khronos validator, while eight authored rotation keys exceed Baozi's narrower squared-norm tolerance.
The selected Baozi profile therefore cannot import or evaluate this asset unchanged.
This restriction is visible in the [capability manifest](../../tools/consumers/assets/fixtures/expectations.json); it is not a general claim that the official asset is invalid.
RiggedSimple remains the admitted skeletal-workflow fixture.

No source quaternion is normalized on import, no global epsilon is widened, and no new quaternion representation is introduced by this decision.
The observed validator result does not establish one uniquely correct tolerance for Baozi's transform arithmetic.
Accepting a wider set later requires a coherent authored-versus-effective rotation contract across static transforms, animation, conversion, snapshots and application evaluation.
An importer-only exception would fail canonical validation or silently change retained source bits.

## Unchanged Source and Independent Arithmetic

The fixture is pinned to KhronosGroup/glTF-Sample-Assets commit `8f492390ddc7a2ae221a1369ef560589cf8c70fb`.
Its five files match the existing `SHA256SUMS` entries.
The animation output is twelve little-endian F32 XYZW tuples, starting at byte 48 of `SimpleSkin_animation.bin`, with a 16-byte stride.
Independent binary decoding and exact rational sums identify the following nonidentity families:

| Authored nonzero components | Squared norm | Absolute squared-norm error | Absolute length error |
| --- | ---: | ---: | ---: |
| `0.382999986410141`, `0.9240000247955322` | `1.0004650354123124` | `0.00046503541231235346` | `0.00023249068019803865` |
| `0.7070000171661377`, `0.7070000171661377` | `0.999698048545838` | `0.0003019514541620083` | `0.00015098712563710315` |

Keys 1, 2, 3, 4, 7, 8, 9 and 10 exceed `abs(norm2 - 1) <= 1e-4`; the remaining four keys are identity.
Negating the relevant components does not change admission.
The disagreement already exists in the three-decimal authored values, before binary decoding or Baozi arithmetic.

## Normative Semantics and Validator Policy

The [published glTF 2.0 specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html) describes unit XYZW quaternions and rotation interpolation.
The investigated published snapshot identifies version 2.0.1, source commit `8e798b02d254cea97659a333cfcb20875b62bdd4`.
The same relevant requirements were checked against [specification source `59ecbdba704206bb656965ab4e202881806066f1`](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/Specification.adoc).
Neither inspected source supplies a numeric quaternion epsilon.
Normalization of a cubic interpolation result does not authorize rewriting stored keys or derivatives.

The official [validator release `2.0.0-dev.3.10`](https://github.com/KhronosGroup/glTF-Validator/releases/tag/2.0.0-dev.3.10) is pinned to source `bcd52cc4ba5f333b2999a58f67cc05ddf28b4fb1`.
Its [threshold definition](https://github.com/KhronosGroup/glTF-Validator/blob/bcd52cc4ba5f333b2999a58f67cc05ddf28b4fb1/lib/src/errors.dart#L33) and [animation checker](https://github.com/KhronosGroup/glTF-Validator/blob/bcd52cc4ba5f333b2999a58f67cc05ddf28b4fb1/lib/src/base/animation.dart#L421) use an absolute **length** tolerance of `0.00769`, accommodating signed 8-bit quantization; that checker also receives FLOAT values.
This differs in both metric and threshold from Baozi's absolute **squared-length** tolerance of `0.0001`.
Passing the validator establishes its result under that policy, rather than requiring Baozi to adopt the same threshold.

The exact official npm artifact has SHA-256 `4e03dbdc3bc0d1342afd5d2d7ae341a7e3474502ccb872b02dbbaddaee0fdec6`.
The integration run verified that digest, npm SHA-1, SHA-512 integrity and `gitHead`, then invoked `validateBytes` with unlimited issues, no suppression/severity override and all four external buffers loaded.
The actual result was **zero errors, zero warnings, zero infos and one hint**, `BUFFER_VIEW_TARGET_MISSING` for `JOINTS_0`.
The full report, tool identity, source/resource digests and independent key arithmetic are retained under `target/quaternion-compatibility/`.

## Existing Semantic Boundaries

Core `Quat::is_unit` performs a finite check and sums F32 source components in F64.
Authored TRS, animation construction and published storage share that predicate.
Snapshots preserve authored component bits; coordinate conversion maps quaternion components without source normalization or hemisphere canonicalization.
Cubic derivatives require finiteness, not unit length.

Static TRS composition uses the raw admitted components, including their small admitted norm error.
The detached application evaluator normalizes temporary effective rotation samples, including exact/STEP endpoints, and preserves stored keys and derivatives.
These are existing, distinct contracts.
Widening admission would change raw static matrices as well as compatibility, so it cannot be treated as a harmless fixture fix.

Boundary regression vectors use adjacent representable F32 values with `q=(0,0,a,a)`:

| `a` bits | Independently calculated squared norm | Expected admission |
| --- | ---: | --- |
| `0x3f3502a2` | `0.99989999594711776` | Reject |
| `0x3f3502a3` | `0.99990016452608899` | Accept |
| `0x3f350744` | `1.0000999405930315` | Accept |
| `0x3f350745` | `1.0001001091888568` | Reject |

The additional value `0x3f3507c8` has squared norm `1.0001221953648383` but length error below `1e-4`; rejecting it protects the metric distinction.
Finite zero, subnormal-only and grossly scaled tuples remain rejected, as do nonfinite components.
The execution record owns fresh Baozi test results; the external validator pass alone does not qualify a Baozi workflow or produce a comparison ratio.
