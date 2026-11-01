# Baozi Triangle UV Derivative v1

Research date: 2026-11-01. This note describes the private, allocation-free triangle kernel and
the selected operation profile. It does not qualify a completed Scene operation or report test
execution. The convention is the built-in `TangentBasisConvention::TriangleUvDerivative` with
`SignInW`, described as **Baozi Triangle UV Derivative v1**. It is not MikkTSpace.

## Backend decision

Use the independently authored per-triangle UV derivative kernel for the initial profile.
It has fixed-size inputs, output, and stack storage, with no allocation, recursion, welding,
cross-face accumulation, callback, or new backend dependency. The operation owns storage
admission, domain selection, cancellation, work accounting, and atomic publication.

The verified maintained MikkTSpace candidate is `bevy_mikktspace` **1.0.0**, published on
2026-02-17 from commit `9de78a281bca0505142da521eb5152146da28656`. The local Cargo registry
source's `.cargo_vcs_info.json` agrees with that revision. Its package declares
`Zlib AND (MIT OR Apache-2.0)`, Rust 1.85.0, no runtime dependencies, and default feature `std`.
The README states Rust 1.85.1. Its source forbids unsafe code.

Its public interface is `Geometry<O>` plus
`generate_tangents(&mut I) -> Result<(), GenerateTangentSpaceError>`. The geometry interface
reads position, normal, and UV tuples and writes `Option<TangentSpace>` for each face corner.
`TangentSpace::tangent_encoded()` returns `[f32; 4]`. The optional grouping threshold API
accepts a linear cosine threshold in `[-1, 1]`, rather than the C API's angular threshold.

The implementation internally uses infallible `Vec` allocation and growth, potentially
allocating stable sorts, recursive grouping and default vertex welding, and nested subgroup
scans. Geometry callbacks cannot return a resource or cancellation error, and the public
error enum currently has no variants. The `corrected-vertex-welding` feature uses an internally
allocating `BTreeMap`; neither correction feature supplies caller-owned scratch or cooperative
control. A wrapper reservation cannot account for actual internal capacity and lifetime, and
callback panic containment cannot turn allocation aborts into typed resource failures.
Chunking input changes welding and smoothing across chunk boundaries. Consequently the crate
cannot directly satisfy Baozi's strict operation contract. A future private adaptation must
address those internal boundaries and retain its license notices. Default-feature byte
compatibility claims are qualified by architecture and math backend in the upstream docs.

The older `mikktspace` **0.3.0**, published on 2022-02-12 from
`c1e6f163ec0ac9e77bffcf6144c9b09ab75dc412`, exposes `Geometry`,
`generate_tangents(&mut I) -> bool`, and `set_tangent_encoded([f32; 4], ...)`.
The published VCS revision was read from the crate archive in memory. Its repository head at
verification was `6275cc4f15cff8be29819fb34ae8be3b9129dae1`, dated 2022-02-27; that is not the
published revision. The implementation contains unsafe Rust and infallible allocation, and
requires an optional math backend, defaulting to `nalgebra`. Package metadata says
MIT/Apache-2.0, but `generated.rs` also retains Mikkelsen's Zlib notice. It is not the preferred
basis for a safe private backend.

No Assimp or Mikk implementation or third-party fixture was copied into this kernel or its
tests. The determinant expansion follows Baozi's existing normal kernel. `repo-ref/assimp`
is study-only under ADR 0001. The selected Khronos reference files contain asset documentation,
not a reusable Rust tangent implementation. The fixtures below are independently authored.

## Algorithm and sampling convention

For the selected tuples at the three triangle corners, define:

```text
e1 = P1 - P0
e2 = P2 - P0
(du1, dv1) = UV1 - UV0
(du2, dv2) = UV2 - UV0
D = du1*dv2 - dv1*du2
G = cross(e1, e2)
A = e1*dv2 - e2*dv1
Tu = A / D
Tv = (e2*du1 - e1*du2) / D

N = normalize(selected_normal)
T_ideal = normalize(Tu - N*dot(N, Tu))
W_analytic = sign(D) * sign(dot(N, G))
```

These are analytic definitions. The kernel approximates `T_ideal` with the specified F64
projection, rounds its components once to the emitted F32 tangent `T`, and records the analytic
orientation as W using its conditioned F64 sign tests. Reconstruction uses
`B = W * cross(N, T)`. A decoded tangent-space normal `m` is transformed with
`normalize(m.x*T + m.y*B + m.z*N)`. The bitangent equation agrees with glTF's `SignInW`
encoding; this agreement does not imply MikkTSpace construction or compatibility with a
MikkTSpace normal-map bake. The latter also depends on welding, grouping, and weighting.

The kernel evaluates `normalize(sign(D)*A)` instead of dividing by `D`; positive scaling
does not change tangent direction. It uses `cross(cross(N, direction), N)` for Gram-Schmidt
projection to avoid subtracting nearly equal vectors. Since
`cross(Tu, Tv) = G / D`, it obtains handedness as
`sign(D) * sign(dot(N, G))`, avoiding cancellation between nearly parallel UV derivatives.
Analytically this equals `sign(dot(cross(N, T_ideal), Tv))`. Using only `sign(D)` would be
incorrect for reversed selected normals.

Handedness records the orientation of the analytic UV derivative frame, evaluated using the
documented F64 arithmetic and conditioning checks. The emitted tangent is rounded to F32.
For sufficiently ill-conditioned UV mappings, the reconstructed bitangent is not guaranteed
to have a positive dot product with the unprojected V derivative after rounding. W is never
flipped in response to that rounded dot product. The algorithm does not reject all poorly
conditioned UV maps or promise correctly rounded analytic tangent components.

Coordinates are used as stored. Scene handedness, front-face declarations, and UV origin add
no implicit flip. An explicit `v -> 1-v` change naturally changes the derivative and W.
Normal texture channel conversion belongs to its declared texture convention. The algorithm
does not inspect images, texture transforms, wrap modes, or select a material's normal UV set.

## Numeric and work contract

All input components must be finite. A zero geometric cross or zero UV determinant rejects
the triangle. Each selected normal is normalized in a local F64 value; source values remain
unchanged. With `tau = 64 * f32::EPSILON`, evaluated in F64 as exactly
`2^-17 = 0.00000762939453125`, the kernel rejects:

- `abs(dot(unit_normal, unit_geometric_cross)) <= tau`;
- a projected tangent length `<= tau` relative to the normalized U derivative;
- different W signs among the triangle's three corners.

These are relative frame-conditioning tests, not area or UV determinant epsilon cutoffs.
The inclusive cutoff uses the F32 output precision to limit amplification of F64 projection
error when the U derivative almost follows the selected normal. Unit length and orthogonality
alone cannot detect an unstable direction within the tangent plane. Nonzero subnormal F32
geometry and UV coordinates remain admissible when the relative frame checks pass. Handedness is exactly
`+1` or `-1`. F64 normalization scales by the maximum absolute component before computing
length. Tangent XYZ is rounded to F32 once, with generated zeros canonicalized to positive
zero. The rounded squared length and normal dot product are checked against
`8 * f32::EPSILON`; an invalid rounded frame returns `InvalidOutput`.

The kernel orders Position tuples lexicographically with `f32::total_cmp`, using three fixed
compare/swap steps, and applies the same permutation to UVs. Selected normals and returned
corner locations retain original order. This fixes determinant evaluation order under all
face permutations. Coincident Position tuples are geometrically degenerate, so they need
no secondary ordering key.

Each determinant is expanded into six products of original F32 coordinates promoted to F64.
Those products are exact and within the F64 exponent range. A six-entry TwoSum expansion
retains cancellation residuals before an ordered F64 estimate. Merely promoting rounded
edge subtraction to F64 is insufficient. The same routine evaluates three geometric cross
components, one UV determinant, and three Position/V numerator components: seven calls,
forty-two exact products, and at most 105 TwoSum inner iterations. There are at most eight
scaled normalizations. Every bound is independent of scene or primitive size.

The caller checks `face <= (usize::MAX - 2) / 3` before entry. Diagnostic corners are the
original global indices `face * 3 + local_corner`. No additional issue variants are required.
The caller charges and polls once per face plus descriptor and tuple loads. Output scratch,
actual vector capacity, retained replacement storage, overlapping lifetimes, metadata/remap
work, and final validation remain the operation's responsibility. The kernel returns values
by value and never publishes partial output.

## Selected operation restrictions

The operation selects a complete Normal semantic/domain key and Texcoord set/domain key.
The current core schema admits Normal and Texcoord only on Vertex or Corner domains. The
public operation accepts those two domains independently for Normal and UV selection and
rejects Face or Instance options with `UnsupportedDomain`; this profile does not expand the
schema. Vertex reads `indices[corner]` (or the non-indexed vertex), and Corner reads the
corner. The standalone kernel takes already selected tuples without domain descriptors.
Three identical UV tuples, such as a hypothetical Face UV broadcast, are a useful kernel-only
`DegenerateUv` fixture, not a publicly admitted Face-domain attribute or operation option.

The initial result is dense Corner F32x4. It preserves independently generated values on
mirrored seams, including shared source Vertex identities. Adjacent faces with continuous
UVs may still have different tangent directions, because this profile does not smooth.
The convention belongs to the generated Tangent attribute, rather than unrelated attributes
or a scene-wide assertion about all authored tangents.

The kernel evaluates only the supplied base geometry. Morph support must be admitted by the
operation before output allocation. In particular, existing Vertex-domain Tangent morph
deltas cannot silently be reused with a regenerated Corner-domain tangent or different basis,
including explicit zero and sparse payloads. Morph tangent deltas contain XYZ only; W is not
morphed. The conservative initial profile rejects affected Position, Normal, or Tangent morph
delta payloads until deformation-aware generation and domain mapping are supported. This
note does not imply that the standalone kernel inspects or enforces morph restrictions.

## Independently checkable fixtures

Unless specified otherwise, use `P=[O,X,Y]`, `N=+Z`, and
`UV=[(0,0),(1,0),(0,1)]`. Values apply to all three corners.

| Case | Changed input | Expected F32x4 or rejection |
| --- | --- | --- |
| Basic | None | `(1,0,0,+1)` |
| U mirror | `UV1=(-1,0)` | `(-1,0,0,-1)` |
| V mirror | `UV2=(0,-1)` | `(1,0,0,-1)` |
| Reversed normal | `N=-Z` | `(1,0,0,-1)` |
| Tilted corner normal | `N=normalize(1,0,1)` | `(1/sqrt(2),0,-1/sqrt(2),+1)` |
| Extreme UV cancellation | `H=2^100; UV=[(H,H),(-H,-H),(1,0)]` | `D=2H; (-1/sqrt(5),2/sqrt(5),0,+1)` |
| Extreme Position cancellation | `P=[(H,H,0),(-H,-H,0),(1,0,0)]` | `G.z=2H; (-1/sqrt(2),-1/sqrt(2),0,+1)` |
| Analytic orientation after F32 rounding | `UV=[(0,0),(1048576,1048575),(1048575,1048574)]` | `D=-1; T=(-0.7071064710617065,0.7071071267127991,0); W=-1` |
| Non-axis-aligned unstable projection | `d=2^-43; P=[(d,0,0),(1,2,3),(0,1,0)]; N=(1,2,3)` | `DegenerateFrame` at the first original corner |
| Non-axis-aligned admitted projection | Same shape with `d=2^-13, 2^-14, 2^-15` | Direction `(-13,2,3)/sqrt(182)` within `8*f32::EPSILON` radians; `W=-1` |
| Zero area | `P2=(2,0,0)` | `DegenerateFace` |
| Collinear UV | `UV2=(2,0)` | `DegenerateUv` |
| Constant UV broadcast (kernel only) | All UVs `(2,3)` | `DegenerateUv` |
| Zero normal | One `N=(0,0,0)` | `ZeroNormal` at that original global corner |
| In-plane normal | One `N=(0,1,0)` | `DegenerateFrame` |
| Mixed orientation | `N=[+Z,-Z,+Z]` | `MixedHandedness` |

Expected positive `1/sqrt(2)` bits are `0x3f3504f3`; positive `1/sqrt(5)` and `2/sqrt(5)`
bits are `0x3ee4f92e` and `0x3f64f92e`. Generated zero bits are `0x00000000`.
For the extreme UV fixture, naive F64 edge subtraction instead produces a zero determinant.
For the analytic-orientation fixture, tangent bits are
`[0xbf3504ee, 0x3f3504f9, 0x00000000, 0xbf800000]`. Its exact raw V derivative is
`(1048575,-1048576,0)`, and reconstructed B dotted with that derivative is
`-328953/16777216`, despite the analytically correct W. This is an explicit output precision
limitation, not a reason to flip W. For the non-axis-aligned shape, exact Gram-Schmidt before
normalization is `d/14 * (-13,2,3)`. Both relative checks pass for the three admitted deltas;
`d=2^-16` and `d=2^-43` are rejected by the new conditioning threshold. The latter previously
passed a `64*f64::EPSILON` cutoff while incurring about 0.00204 radians of direction error.
Kernel tests also cover all six face permutations, normal magnitudes, F32 exponent extremes,
threshold boundaries, non-finite inputs, original corner diagnostics, and allocation counters
on successful and rejected calls. These are test definitions, not a claim of executed checks.

## Primary sources

- [bevy_mikktspace 1.0.0 package and publication history](https://docs.rs/crate/bevy_mikktspace/1.0.0).
- [Exact 1.0.0 manifest, license expression, features, and dependencies](https://github.com/bevyengine/bevy_mikktspace/blob/9de78a281bca0505142da521eb5152146da28656/Cargo.toml).
- [Exact 1.0.0 public API, unsafe prohibition, and compatibility conditions](https://github.com/bevyengine/bevy_mikktspace/blob/9de78a281bca0505142da521eb5152146da28656/src/lib.rs).
- [Exact 1.0.0 allocation, grouping, and recursion implementation](https://github.com/bevyengine/bevy_mikktspace/blob/9de78a281bca0505142da521eb5152146da28656/src/mikktspace.rs).
- [Exact corrected vertex-welding implementation](https://github.com/bevyengine/bevy_mikktspace/blob/9de78a281bca0505142da521eb5152146da28656/src/mikktspace/weld_vertices.rs).
- [mikktspace 0.3.0 package and publication history](https://docs.rs/crate/mikktspace/0.3.0).
- [Published 0.3.0 API and unsafe entry point](https://github.com/gltf-rs/mikktspace/blob/c1e6f163ec0ac9e77bffcf6144c9b09ab75dc412/src/lib.rs).
- [Published 0.3.0 generated implementation and Mikkelsen license notice](https://github.com/gltf-rs/mikktspace/blob/c1e6f163ec0ac9e77bffcf6144c9b09ab75dc412/src/generated.rs).
- [Official MikkTSpace interface, algorithm description, and Zlib notice](https://github.com/mmikk/MikkTSpace/blob/3e895b49d05ea07e4c2133156cfa94369e19e409/mikktspace.h).
- [glTF geometry, SignInW, and same-triangle handedness](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#meshes-overview).
- [glTF morph target component and handedness restrictions](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#morph-targets).
- [glTF texture coordinate origin](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#images).
- [Baozi normal kernel determinant arithmetic](../../crates/baozi-postprocess/src/normals/kernel.rs).
- [Baozi source-study boundary](../adr/0001-baozi-assimp-compatible-architecture.md#assimp-and-clean-room-boundary).
- [Baozi morph contract](../adr/0031-animation-skin-morph-and-deformer-contract.md#morph-targets-and-instances).
