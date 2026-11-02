# Baozi native headless wgpu consumer

This excluded, unpublished workspace renders the existing detached
`baozi-asset-consumer::ApplicationAsset`. It adds no Scene, animation sampler,
runtime framework, portable facade feature, window or surface. It requires Rust
1.95 and exactly wgpu 30.0.1 / image 0.25.8. Production uses image's PNG feature
for PNG input/output and turbojpeg 1.5.1 for JPEG input, with turbojpeg-sys 1.1.1
pinning bundled libjpeg-turbo 3.1.0. Its native backends are DX12 and Vulkan;
compiling both does not qualify both.

The TurboJPEG dependency disables default features and enables `cmake`, so the
bundled native build requires CMake and a working C compiler, such as MSVC Build
Tools on Windows. NASM is optional; `require-simd`, `pkg-config` and `bindgen`
are not enabled. An explicitly substituted external JPEG library needs its own
source/version qualification. The image crate's JPEG feature is a dev-dependency
for test encoding; Cargo unifies it into test builds, but the consumer's only
JPEG decode path remains TurboJPEG.

## Run

Run these commands from the repository root. Select one backend, then copy the
exact adapter name and decimal vendor/device IDs from enumeration. Selection
requires exactly one match and never silently substitutes another GPU or backend.

```powershell
cargo run --manifest-path tools/consumers/render-wgpu/Cargo.toml --locked -- adapters --backend dx12
cargo run --manifest-path tools/consumers/render-wgpu/Cargo.toml --locked -- render --backend dx12 --adapter 'EXACT NAME' --vendor 1234 --device 5678 --root 'PATH TO ASSET DIRECTORY' --asset model.gltf --camera-node 2 --output 'PATH TO OUTPUT PREFIX' --source-revision 'EXACT SOURCE REVISION'
```

Choose exactly one of `--camera-node INDEX` or `--eye X,Y,Z`. The explicit eye
looks along world -Z with +Y up, perspective FOV pi/2, near 0.01 and infinite far.
It is appropriate for explicitly selected RH meter-space assets in front of that
camera. There is no implicit first camera, auto-framing or invented ambient light.
The file workflow explicitly converts to `TargetSpace::STANDARD`.
`--clip INDEX --time VALUE` selects a declared-domain time; `--width`/`--height`
default to 512. Imported camera/light placements use the same evaluated world
table as geometry. An asset without lights produces emission or Unlit output.

Output is `PREFIX.png`, `PREFIX.rgba32f` and `PREFIX.json`. The binary is raw
little-endian F32 RGBA, top-to-bottom, with width/height in the JSON. The PNG
preview applies exposure 1, `c/(1+c)` to nonnegative linear RGB and standard sRGB;
alpha stays linear. The JSON records actual adapter/backend/driver identity,
requested limits, camera/lights, counts and numerical policies. The caller supplies
the source revision. Qualification must additionally record toolchain, lock,
shader and fixture hashes; the example does not invent those provenance facts.
The output parent directory must already exist. All frame validation precedes
saving; separate host file writes are fallible, not a transactional filesystem API.

## Device profile

`DevicePlan::prepare(asset, evaluation, frames, limits, control)` borrows the asset
and evaluator until upload. The plan has private input references and cannot be
replayed with a replacement source. `Renderer::upload` returns a `GpuAsset` with
no asset/evaluation/Scene/IO borrow. Source arrays remain unchanged.

Each source instance/draw is visited in application order. Whole triangles split
stably on explicit vertex/index/byte/palette caps and orientation boundaries.
Default U16 has 65,536 legal values, including index 65,535, triangle lists,
no primitive restart and base vertex zero. U32 is an explicit alternative.
Degenerate triangles and transparent submission order are preserved. An element
that cannot fit an empty packet fails by name. Counts, expansion, bytes and work
are finite and checked; vectors use fallible reservations. Defaults are 2 million
output vertices, 8 million indices, 4,096 chunks, 4x vertex expansion, 4,096 full
CPU joints per instance, 512 MiB plan and GPU geometry limits and 200 million
planning work units. GPU creation intersects each buffer/texture requirement
with the requested device limits.

Positions are evaluator **world XYZ**, including the complete non-unit-weight
homogeneous skin equation; the shader uses W=1 once and never reapplies an
instance transform. F64-to-F32 conversion preserves exact signed zero and rejects
nonfinite values, overflow, nonzero underflow, F32 subnormals and excessive error.
Default position/UV absolute and relative tolerances are 1e-6; frame/color
tolerances are 2^-20. Distinct source ordinals never weld after narrowing.

Every chunk has `vertex_sources`, ordered indices, every retained UV stream,
optional-field flags, material assignment presence, complete palette and influence
remaps. Influence weights/order/duplicates/zero entries are unchanged. CPU full
rows is the default; bounded palette planning can fail or explicitly use
`CpuFullRows` for an indivisible element. It does not claim GPU skinning.
`source_draw`, `source_morph` and `source_influence_row` expose the original
complete rows/streams; unused source vertices remain in the borrowed authority.
Morph correspondence is a source map, not a second packed morph authority.

## Shading and frame contract

The ten fixed slots are BaseColor, MetallicRoughness, Normal, Occlusion, Emissive,
Clearcoat, ClearcoatRoughness, ClearcoatNormal, SheenColor and SheenRoughness.
Active bindings independently retain public swizzles, color interpretation,
UV set, `offset + rotation*(scale*uv)`, strength and sampler. Up to four distinct
active sets interpolate; all retained sets upload. UV1-only needs no UV0.
The stage interface uses five user locations: world position, normal, color and
two full-F32 UV pairs. Including the `FrontFacing` budget slot, it fits the
requested eight inter-stage variables. Tangent provenance remains in the
retained vertex stream; the fragment shader does not interpolate an unused
tangent. Packing preserves all four active UV sets and their interpolation.
Lower-left origin conversion follows the binding transform. Unknown sampled UV
origin fails. Decal disables the binding outside transformed unit coordinates;
ClampToBorder is unsupported. Unlit only activates base/alpha/sidedness and does
not decode retained fallback images. Unsupported active workflows, separate
Metallic/Roughness samples, Opacity and active displacement fail. Positive
transmission or thickness factors on lit materials fail with the named
`transmission or thickness` error. Their absent and explicit-zero groups keep
retained bindings inactive: those textures do not add images or UVs to the
active closure, even if the binding strength is nonzero. Source presence and
explicit-zero factors remain distinct and unchanged.

Base RGB multiplies typed factor, sampled linear RGB, binding strength and linear
vertex RGB. Base alpha multiplies factor, mapped alpha and vertex alpha separately.
MR samples use the public metallic/roughness swizzle; emission effective material
strength and binding strength apply exactly once. The lit model is Lambert plus
single-scatter GGX, Schlick Fresnel and height-correlated Smith visibility, with
roughness floor 0.045 for BRDF evaluation. Charlie sheen uses fitted visibility
and the explicit reciprocal normalization described below. A bounded 32x32 R16F
table is generated only for active sheen with fixed 32x64 midpoint quadrature
(2,097,152 samples),
charged separately; it is not a codec image or build script. This finite table
has CPU convergence and finite Vulkan readback vectors in the
[execution record](../../../docs/reviews/2026-11-02-pre-release-evolution.md).
HDR sheen above one is rejected. Clearcoat uses the
simple Fresnel mixture over sheen and emission, with an independent normal.
There is no IBL, shadowing or ambient term; AO is sampled and its indirect term
is zero. Directional lux and point/spot candela use scene meters, squared finite
range/spot attenuation and a 1 mm point-distance regularizer. Orthographic view
direction stays constant across the image. Nonrigid camera or light frames fail;
reflections are checked orthogonal frames.

Point/spot direction and perspective view normalize a vector divided by its
largest absolute component, so no unbounded scene-unit length is squared.
Point distance converts that component scale to meters before multiplying by
the bounded vector length; attenuation squares its reciprocal. A finite range
cutoff is checked before physical-distance arithmetic, and an outside-range
light contributes zero. A range above 2^64 scene units scales both quotient
operands by 2^-64 before division, keeping its reciprocal normal. A tiny scaled
numerator may become zero only where the F32 cutoff already rounds to one.
With a positive cutoff, a physical distance above
2^63 meters or an attenuation outside normal positive F32 fails with diagnostic
bit 512 instead of returning a successful black frame. The 1 mm regularizer
also covers distances too small for the intermediate meter product. Nonfinite
point deltas and nonzero subnormal deltas fail with bit 512. A zero, subnormal
or nonfinite viewing delta fails with bit 16. After scaling, both vector lengths
are checked against [1,sqrt(3)] with an F32 rounding margin. Loss of that invariant,
including reciprocal flushing during division at extreme scales, fails with the
same respective diagnostic bit. A component bound proving finite-range exclusion
skips normalization and physical-distance arithmetic entirely. The tests compare a point at
1e20 scene units with scale 1e-20 meters/unit against an ordinary one-meter
light and independently exercise a perspective eye at 1e20 scene units.

The raw fitted Charlie integral is retained without clamping. It can exceed one
at grazing angles: the independent 32x64 midpoint value at NoV=0.001 and
roughness=0.045 is 4.497436310601021. This consumer therefore uses a separately
named reciprocal normalization. For raw lookup energies Ev and El, let
`d=max(1,Ev,El)` and `C=max(sheen_rgb)`. Its sheen BRDF is `f_fit/d`; its substrate
weight is `A=min(1-C*clamp(Ev,0,1),1-C*clamp(El,0,1))`. For admitted C in [0,1], A is in
[0,1], and swapping V/L preserves both weights. This is a consumer approximation,
not a modification of source factors or the raw reference integral. Charlie NoV
and NoL use the lookup's [0.001,1] grazing domain; MR has its existing policy.

The reciprocal factor reduces the fitted lobe wherever either lookup exceeds
one; bounding A prevents negative substrate radiance. This finite R16F lookup
profile neither reproduces the exact Khronos layering equation nor establishes
full energy conservation over every angle. Tests retain independent raw-integral
convergence vectors and the grazing corner, then separately check the layer
weights and normalized grazing pixels against independent literals. At the
corner, raw E=4.497436310601021 stores as 4.49609375; white sheen has substrate
weight zero and a lobe multiplier of approximately 0.2224. For NoV=NoL=NoH=0.001,
the independent raw BRDF 5240.180562185577 gives normalized radiance
1.1654962848996593 under unit white directional-light intensity. Negative final RGB
is a diagnostic frame failure. The policy preserves finite HDR radiance; it does
not clamp output RGB to one.

Opaque/Mask precede Blend. Mask keeps alpha equal to cutoff. Blend uses straight
RGB SrcAlpha/OneMinusSrcAlpha and alpha One/OneMinusSrcAlpha, depth test on,
depth writes off, stable application triangle order. This is submission-order
transparency, not an intersecting-surface sorting solution. Raster front faces
account for instance/deformation and camera reflection. Backface shading negates
the final base and coat normals.

`DrawFrames` is the narrow U10 seam: F64 world normals, optional world SignInW
tangents and +/-1 world orientation per complete source draw vertex. The
`evaluated_frames` adapter borrows these arrays from the existing evaluator.
Static and animated frames use the evaluator's single checked-inverse policy;
the renderer does not implement a second inverse or normal transformation.
Every lit draw requires explicit evaluator frames; bind-pose normals never
establish deformed shading. Active normal maps use separate world/selected-UV
derivative frames for base and coat. This is not MikkTSpace or inferred authored
tangent/UV association. Active tangent morphs with derivative normal maps,
degenerate frames and mixed orientation elements fail explicitly. The CLI asks
the evaluator for explicit `FramePolicy::FrozenWeightNormalsOnly`, then checks
tangent-morph conflicts only on draws with active derivative normal maps.
Untextured tangent morphs do not receive an unrelated normal-map rejection.
The packed tangent attribute preserves source data when no world tangent was
requested; source-presence flags and `WORLD_TANGENT` distinguish these states.
It is not labeled a deformed tangent and the derivative shader does not use it.
Full `FrozenWeight` remains supported and strict; a degenerate requested tangent
still fails in the evaluator. The stricter whole-request `TriangleUvDerivative`
policy is also compatible with this consumer.

The GPU uses Rgba16Float, Depth32Float, one sample and 0..1 depth. A separate
R32Uint diagnostic attachment and persistent fragment atomic failure word
invalidate frame errors, including ones later overdraw would otherwise hide. The
device must support fragment writable storage. Readback aligns
rows to 256 bytes, waits on the exact submission with a finite 10-second deadline,
checks all three mapping callbacks and fallible mapped ranges, strips padding and
rejects nonfinite F16 results. Adapter/device/validation/mapping/timeout failures
are errors, never blank-frame success.

## Image admission and memory boundary

Only signature-routed PNG/JPEG still images with decoded 8-bit components are
accepted. Present MIME must match; 16-bit PNG, APNG and unsupported signatures
are rejected. PNG uses image 0.25.8. JPEG uses the pinned TurboJPEG decoder with
a 100 progressive-scan limit and requires the encoded slice to end in EOI.
This terminal check does not locate the decoder's first consumed EOI or prove
the absence of intervening or appended data when the slice ends in another EOI.
Codec errors and strict JPEG entropy warnings fail before texture creation. JPEG RGB output
accepts RGB/YCbCr/grayscale sources; CMYK/YCCK-to-RGB is unsupported and returns
a typed codec error. No filename routing, EXIF orientation, ICC/gamma management,
vertical flip or premultiplication occurs, and no CMYK/ICC fidelity is claimed.

Default encoded caps are 16 MiB/image and 128 MiB/active closure; maximum
dimension is min(4096, requested device dimension), at most 16,777,216 pixels,
and resident decoded/mip variants at most 256 MiB. JPEG borrows the encoded
input and writes RGB8 into an admitted consumer buffer; native/RGBA conversion
overlap is admitted before allocation and reconciled against actual capacities
before filling. Image dimensions and consumer-owned output/capacity admission
have explicit caps. `best_effort_decoder_alloc` sets image's PNG `max_alloc`
only, as a **best effort** limit. TurboJPEG has no corresponding scratch cap.
Codec-internal metadata, heap/scratch and allocator overhead remain outside
consumer capacity accounting; no total codec or process-memory cap is claimed.
Cancellation is checked around codec calls and within consumer-owned image/mip
loops, never within a codec call. No worker/OS isolation or hard within-call
deadline is claimed.

Active images decode once. Color and data uses build distinct mip variants;
samplers remain independent. Color mips average RGB in linear space and re-encode
sRGB; alpha/data are averaged directly. Area-box footprints include odd edges.
All mips are supplied explicitly; no driver mip generation or normal-map mip
renormalization is implied. Min/mag/mip/address modes map independently.

Renderer admission reports caller-owned array/copy/target/readback overlap and
logical GPU bytes. It excludes the already resident asset/evaluator, codec scratch,
allocator overhead, wgpu/driver internal allocations and physical VRAM/RSS.
The evaluator has its own explicit output/scratch/work limits. Neither account
establishes a whole-import/process peak-memory guarantee. Retain these distinct
scopes in qualification evidence.

## Verification

Main owns dependency resolution, formatting, all builds/tests and commits.
No execution receipt exists merely because these commands or tests are present.

```powershell
cargo nextest run --manifest-path tools/consumers/render-wgpu/Cargo.toml --locked --offline --jobs 1 --test-threads 1
$env:BAOZI_GPU_BACKEND = 'dx12'
$env:BAOZI_GPU_ADAPTER = 'EXACT ENUMERATED NAME'
$env:BAOZI_GPU_VENDOR = '1234'
$env:BAOZI_GPU_DEVICE = '5678'
cargo nextest run --manifest-path tools/consumers/render-wgpu/Cargo.toml --locked --offline --jobs 1 --test-threads 1 --run-ignored only
```

CPU tests assert scalar errors, the three U16 boundaries, exact full-row/morph
source reconstruction, palette/byte/cancellation failures, PNG/JPEG admission,
odd-edge transfer-aware mips and independent analytic BRDF/camera/light vectors.
GPU tests are explicitly ignored in ordinary CPU suites; the named qualification
command selects ignored tests across all test targets, including contracts,
upstream assets and any added quantization target. Missing adapter/environment
is a failure. Readback
asserts 4x4 interior linear regions, diagnostic zero, split/unsplit agreement,
asset/Scene/IO destruction ordering and actual uploaded U16/U32 index bytes.
The execution record binds the named backend, source, lock and fixture identities
for these checks, including unchanged RiggedSimple and AnimatedMorphCube frame
anchors and interior lighting changes. A screenshot or compile pass alone does
not establish those observations.
