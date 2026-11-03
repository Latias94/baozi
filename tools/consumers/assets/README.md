# Offline Real-Asset Consumer

This unpublished standalone application exercises the public Baozi API with unchanged upstream
assets and actual capability-bound filesystem access. It imports a model and calls
`prepare_triangles` once for all primitive IDs acquired from the imported Scene. Preparation lowers
topology and preserves authored normals or generates missing normals together in one selected transaction. The application
then converts the complete prepared Scene to the requested target space once, copies buffers and
destroys the Scene before returning those buffers and detached import evidence.
Before extraction it walks the retained provenance of each primitive through the public borrowed
readers, sharing one visited set across draws. This reports reached source objects without
inventing a persistent application provenance graph.

`process_file` and `process` use the shared triangle preparation workflow: TriangleList primitives
are accepted, TriangleStrip/TriangleFan primitives are lowered to lists, and polygons are triangulated.
Points and line modes, including LineLoop, are rejected during preparation. The normal policy is
`NormalOptions::default()` with `NormalOrientation::SceneFrontFace`, using `Options::source_assumptions`
when supplied: generated normals follow the source Scene's front-face declaration or explicit source
assumption, and existing normals are preserved. The default tangent policy preserves existing tangents and rejects invalidating normal
changes unless the caller chooses an explicit replacement/removal policy through the preparation API.
`ApplicationAsset::from_scene` is a separate direct extraction path for an already validated Scene.
It explicitly uses `list_vertices`
to accept all seven glTF modes without editing the Scene, generating attributes, converting units
or coordinates, or baking instance transforms. Callers choose any processing needed before direct
extraction; polygons still require explicit triangulation. Direct extraction also accepts points
and lines, which remain outside the triangle processing workflow.

The target is left-handed, positive Y up, negative Z front, centimeters, with counter-clockwise front
faces. UVs remain in their declared source convention. Formats with unknown spatial facts, including
OBJ, need an explicit caller assumption. The native dependency remains an unregistered candidate;
this tool does not add a filesystem backend to the portable library or its release target list.
Library callers choosing a clockwise target prepare using the source winding and then convert
directly to that target, transporting generated normals and Corner tuples together. There is no
temporary CCW processing space or second winding conversion.
glTF explicitly declares a +Z asset front, distinct from camera-local -Z viewing axes; conversion
to this application's default target maps `(x, y, z)` to `(100x, 100y, -100z)`.

Run from the repository root, reusing its target directory:

```text
cargo +1.95.0 run --locked --release --manifest-path tools/consumers/assets/Cargo.toml --target-dir target -- tools/consumers/assets/fixtures upstream/Models/BoxTextured/glTF/BoxTextured.gltf
cargo +1.95.0 run --locked --release --manifest-path tools/consumers/assets/Cargo.toml --target-dir target -- /path/to/assets model.obj --assume-standard-space
cargo +1.95.0 nextest run --locked --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --all-features --build-jobs 1 --test-threads 1
cargo +1.95.0 nextest run --locked --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --test primitive_modes --build-jobs 1 --test-threads 1
cargo +1.95.0 nextest run --locked --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --test material_output --test indexing --test unlit_materials --test deformation_seams --build-jobs 1 --test-threads 1
cargo +1.95.0 nextest run --locked --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --test spatial_output --test evaluation --build-jobs 1 --test-threads 1
```

`ROOT` is an ambient directory used to acquire the native capability; the asset path is relative to
it. Dependencies resolve through the same capability. A successful invocation emits one JSON
summary. Failures emit a cause chain on stderr and return a nonzero exit code. There is no automatic
retry, hidden coordinate assumption, or automatic budget increase. `--max-work-units N` explicitly
sets only the import work allowance through `Options::max_work_units`; its default is 200,000,000,
including filesystem accounting. `--postprocess-actions N` and `--postprocess-bytes N` set
`Options::postprocess_budgets.max_work_actions` and `.storage_bytes` for the single `prepare_triangles` call.
The default `PostProcessBudgets` allows 100,000,000 bounded work actions and 1,073,741,824 accounted storage
bytes. These preparation budgets are separate from import limits and from application buffer
allocations. Coordinate conversion retains its documented default operation limits; the two
postprocess flags apply to triangle preparation, not conversion or a combined pipeline ledger.

`bounded-normal-actions/v2` identifies the postprocess action vocabulary. One normally returned
bounded normal operation, including a semantic error, consumes one action; a denied admission
increments representable attempted actions without executing that operation. Face evaluation,
one smoothing-candidate visit and one output element are separate operations. Concrete Copy
initialization/order scans process at most 32 records per action. Delegated selection, core
construction, real retained-share and protected terminal work retain their defined units on the
same authority. Conservative control costs are separate from consumed actions and preserve the
bounded cancellation frontier. There is no numerical conversion from the retired getter counts.

The processing order is one selected triangle-preparation transaction followed by one whole-Scene
space conversion. Lowering first lets polygons/strips/fans enter a conversion that reverses winding.
Generated Normal orientation uses the source winding or the explicitly supplied source assumption;
the conversion then transports those normals and all Corner tuples together.

## Application layout

This is a concrete extraction example, not a new canonical representation or a general exporter:

- Each primitive becomes a separate indexed draw with a public `DrawBuffer::topology` of Points,
  LineList, or TriangleList. Direct extraction retains the Corner order of source lists and points;
  LineStrip/LineLoop expand to LineList and TriangleStrip/TriangleFan expand to TriangleList.
  For source face `f`, strip triangles use `[f, f + 1, f + 2]` on even faces and
  `[f, f + 2, f + 1]` on odd faces; fan triangles use `[f + 1, f + 2, 0]`.
  Loop edges use `[f, (f + 1) % n]`, retaining both directed edges for a two-point loop.
  Degenerate elements remain in the output and still advance strip parity. Source primitive
  topology, indices, and shared mesh/instance relationships remain unchanged.
- Reuse requires the same `PrimitiveVertex` identity and identical `to_bits()` values in every
  selected Position, Normal, Tangent (including W), every retained UV set and COLOR0 lane.
  These application fields use F64, with exact F32 widening. RGB colors
  acquire an explicit alpha of one; RGBA and absent colors remain distinct. Indices follow first
  expanded tuple encounter order, independently of hash map iteration, and retain list element
  order and triangle winding. Distinct source vertices, signed-zero seams, and separate draw/material
  boundaries stay distinct; there is no epsilon comparison or position welding. Multiple domains
  for the same Normal, Tangent, UV set or COLOR0 require an explicit policy and are rejected here.
  `Vertex` contains Position, optional Normal/Tangent, and optional COLOR0. Ordered
  `DrawBuffer::uv_streams` contain every retained Texcoord set, including unreferenced UV0,
  its original domain, and F64 values in the same vertex order. `draw.uv_stream(set)` returns
  exactly that set or `None`; it never substitutes UV0. Every stream participates in tuple
  equality before index reuse, including signed-zero bits and UV1-only seams.
- Skin influences use variable-length CSR rows without truncation. Each representative copies its
  complete joint/weight row once, preserving empty rows, all lanes, their sequence, and F32 bits.
  The core's separate source-order ordinals remain outside this selected application layout.
  Every authored Position, Normal, and Tangent morph delta stream uses the same indexed vertex
  order. Missing sparse entries use the public logical value read, and no new stream is synthesized.
  Representatives record expanded output tuple ordinals, so morph reads address that same tuple
  even when strips, fans, and loops repeat or reorder source Corners.
  Influences and morph deltas are Vertex-domain values, so source identity guarantees their
  equivalence within a key. Any future selected Corner/Face deformation fields must also extend
  that key or keep their tuples separate.
- Owned node hierarchy and authored transforms, separate mesh default morph weights and optional
  instance overrides, joint references, inverse bind matrices and skin matrix-space/correction
  declarations, animation time domains, channel extrapolation, samples and cubic derivatives
  survive Scene destruction. Authored matrices remain matrices.
  Opaque Scene IDs are mapped to application indices only after the final edit.
- Independent `cameras` and `lights` tables own complete `CameraProjection` and `LightDefinition`
  values. Each node has optional `camera` and `light` application indices, alongside any mesh
  instances. Multiple nodes can share either definition without duplicating it. Camera/light-only
  scenes require no geometry or default draw. Camera and Light names come from their own retained
  `org.baozi.gltf:source-name` metadata; absent and explicitly empty names stay distinct.
- Materials retain names, shading model, alpha mode (including Mask cutoff), sidedness and the
  complete `MaterialFactors` value: raw emission with independently optional strength,
  metallic/roughness, specular/glossiness, legacy, and independent optional clearcoat, transmission,
  thickness and sheen groups. Presence and factor bits survive extraction; effective emission is
  derived in F64 without overwriting the authored values. Every ordered binding retains texture identity, role, UV
  set, channel mapping, color interpretation, transform, strength, blend operation and an owned
  optional source key. Unlit keeps all lighting fallback factors and bindings.
- Independent `textures`, `samplers` and `images` tables close every binding. A texture holds an
  image index and sampler index; equal links do not merge texture identities. A sampler retains
  the complete `SamplerState`, including independent minification/magnification, mipmap and U/V/W
  addressing. An image owns `encoded_bytes` and an optional declared `media_type`; an unknown
  media type stays unknown even when its filename or byte signature suggests a codec.
  Image, Texture and Sampler names come only from their respective owners' retained
  `org.baozi.gltf:source-name` metadata. Absent and explicitly empty names remain distinct;
  no image-name or path fallback is copied into another owner.
- `ApplicationAsset::space` copies the complete lifetime-free `SceneSpaceRef`: handedness, up/front
  axes, optional unit scale to meters, front-face winding, and UV origin. Unknown facts stay unknown;
  UV origin is read through `asset.space.uv_origin`. Each draw's
  `tangent_conventions` belongs to its selected tangent attribute and owns any namespaced basis
  and encoding identity/version. The copier neither flips UVs nor infers a MikkTSpace convention.
- Source-object glTF JSON metadata is copied explicitly with sanitized resource identity, format,
  versioned namespaced locator, site key, media type, and exact payload bytes. This subset omits
  entity metadata beyond the source names described above, non-JSON values, and other formats'
  metadata. It does not parse or rewrite extras.
  The unchanged `MorphPrimitivesTest` checks all 147 original `/asset/extras` bytes after edits and
  Scene destruction, together with both primitives, shared default morph weights, and its JPEG.
- This application selects one domain for each normal, tangent, retained UV set and COLOR0.
  It does not retain other color sets, custom attributes or the complete provenance
  graph. It is not a round-trip serializer or renderer. Image decoding, shading, tangent generation
  and GPU packing remain outside this example. `mesh_local_bounds` aggregates base
  mesh-local positions; it does not apply instances, node transforms, skinning, or morph weights.

Extraction checks every retained binding against each draw's selected UV streams, including
Unlit fallback uses. Missing or ambiguous referenced sets return `UvExtractionError`, carrying
mesh/primitive, material-local binding ordinal, set and issue kind. Unreferenced ambiguous sets
also reject, with no binding ordinal. Unsupported UV scalar/width/domain profiles reject explicitly;
this per-draw vertex layout accepts Vertex, Corner and Face UVs, and rejects Instance-domain UVs.
The copier validates this closure before reading tuples and returns no partial `ApplicationAsset`.
UV transforms, channel selection, color transfer and factors remain separate retained declarations;
none is baked into the image or coordinates. Resolve a binding through `binding.texture`, then
`asset.textures[...].image` and `.sampler`; resolve its UV stream with
`draw.uv_stream(u32::from(binding.uv_set))`.

`DrawBuffer::material` retains one `MaterialAssignment<usize>`: Unspecified, Authored, or
ImplicitDefault. Its `.authored()` and `.effective()` accessors derive both views from that same
assignment. Bindings and UV closure checks use the effective material; callers can still distinguish
authored absence when an implicit default supplies shading data. The copier maps the canonical
assignment directly, without inferring its origin from format identity or material values.

For glTF `KHR_materials_unlit`, the owned material records `ShadingModel::Unlit`; its retained PBR
base color, BaseColor binding and COLOR0 supply color, while alpha coverage and sidedness remain
explicit. Lighting factors and bindings remain authored fallback data. The example copies these
facts without rendering or silently applying PBR lighting to Unlit. The self-authored
`unlit_materials` integration tests cover optional/required declarations, texture transforms,
fallback resources, space edits and detached owned values.
`material_output` adds self-authored shared-image/different-sampler and equal-link/distinct-texture
fixtures, UV1-only and unreferenced UV0 streams, nondefault filtering/addressing, full ordered binding
semantics, source-owner names, declared/unknown image media type, F64 PLY UVs, legacy factors and
named missing-UV failures for Unlit fallback. All full appearance assertions read owned output after
the Scene, import evidence, importer, IO and fixture buffers are destroyed. The UV transform vector
maps `(1/4,1/8)` to `(1/16,3/4)` using scale, 90-degree rotation, and translation, without decoding pixels.
Clearcoat/sheen cases also retain explicit zero-valued groups, all five additional texture roles,
signed coat-normal scale, independent UV transforms and exact nested extras. Shared images with
different samplers remain shared through coordinate conversion and source-owner destruction;
missing sheen UVs report their exact material-local binding ordinal.

Camera projections retain their intrinsic axes, perspective field of view and optional aspect/far
planes, or orthographic magnifications and depth range. Light definitions retain category, intrinsic
axes, normalized linear tint, photometric intensity, optional range, and spot half-cones in radians.
Directional intensity is in lux; point/spot intensity is in candela. Ranges and camera distances use
the current `asset.space` length unit. The copier preserves these canonical values without deriving
a second emission representation or applying attenuation.

Node hierarchy and authored transforms remain the sole placement authority. The existing detached
`evaluate` operation supplies `world_transforms[node_index]` even for a scene without draws. The
node's world origin locates a point/spot emitter, while the definition's intrinsic axes identify its
reference directions; glTF cameras and directional/spot lights use local -Z before conversion.
Parent scale affects inherited position, but node scale does not rescale intensity, range, camera
parameters, or cone angles. Explicit Scene unit conversion scales distances and ranges and converts
intrinsic axes before extraction; emission and cone bits remain unchanged. These copied transforms
can retain scale/shear. A renderer that needs a rigid photometric frame must check it explicitly;
the copier and position evaluator do not silently orthogonalize it.

The self-authored `spatial_output` tests specify camera/light-only scenes, shared definitions with
omitted source ordinals, absent/empty owner names, exact light extras, and nodes carrying geometry,
camera and light together. Independent source vectors give world origin `(12,26,42)` from parent
translation `(10,20,30)`, scale `(2,3,4)` and child translation `(1,2,3)`. Conversion to left-handed
Y-up centimeters gives `(1200,2600,-4200)` and range `8 -> 800`, with unchanged photometry and cones.
All successful output assertions run after Scene, import/conversion reports, importer, IO and fixture
buffers leave scope. A missing material UV returns an error while preserving the source Scene.

The separate `evaluate` reference in `src/evaluation.rs` consumes only these owned buffers. Its
fourth argument is an explicit `EvaluationRequest` with frame policy, `EvaluationLimits` and an
optional cancellation token. Select one clip
with `Some(index)` or the authored pose with `None`; `SampleTime::Declared(f64)` uses the clip's
time domain and `SampleTime::Seconds(f64)` requires a known positive frame/tick rate. It samples
Step, Linear and CubicSpline TRS/morph curves with F64 math, normalized shortest-path slerp for
linear rotations, and component Hermite followed by result-only normalization for cubic rotations.
Derivatives remain in declared time units and are multiplied by the key interval. SourceDefault
uses immutable authored components or instance-then-mesh weights; Constant uses the endpoint.
Linear/Repeat extrapolation and TRS animation of matrix-authored nodes return named errors.
Morph deltas precede canonical skin position evaluation, with no influence truncation, weight
clamping or renormalization. Explicit skin corrections, singular instance transforms and zero
quaternion results return named failures. Inversion tests numerical rank relative to the linear
3x3 scale, allowing well-conditioned tiny or large uniform scales and affine shear. It checks
both inverse multiplication residuals and translation cancellation before publishing positions;
an inverse that cannot meet the F64 reference accuracy limit returns a named numeric failure.
Output owns F64 local/world transforms, effective morph weights and per-instance draw positions.
`PositionOnly` returns no shading frames. `FrozenWeight` additionally applies authored normal and
tangent morph deltas before the complete per-vertex weighted linear skin map; normals use its
checked inverse transpose, tangents are orthogonalized, and tangent W includes reflection parity.
This frozen-weight approximation does not differentiate spatial weight gradients. Missing active
morph shading deltas, singular maps, degenerate frames and mixed element orientation fail the
requested result without a bind-pose fallback. Source influence rows and samples are unchanged.

`FrozenWeightNormalsOnly` explicitly requests normals/orientation without certifying unused authored
tangents. The unchanged untextured AnimatedMorphCube needs this policy: one source tangent is
parallel to its normal, so a full frame still returns `DegenerateTangent`. `TriangleUvDerivative`
also requests normals/orientation, rejects active nonzero tangent morph conflicts and leaves each
binding's UV derivative frame to the separate [renderer consumer](../render-wgpu/README.md).
No policy silently downgrades after a failure. RiggedSimple and Cube tests compare positions and
changing normals at a source key and interval midpoint with independently decoded source values.

Limits admit output vertices/draws, both transform tables, nested output capacities, optional frames,
per-instance palette matrices, cumulative scratch capacities and work. Allocation is fallible;
cancellation is checked between bounded visits/kernels. `EvaluationAccounting` reports capacities
and work for that operation. Cumulative scratch is a conservative admission account, not a peak
live heap or RSS measurement; borrowed input, allocator overhead and system internals are excluded.
Playback clocks, blending and animated bounds remain outside this reference. See
`tests/evaluation.rs` and `tests/evaluation_frames.rs` for analytic and reflected-space vectors.

`BasePoseSpatialSnapshot::from_scene` is a separate, opt-in query before Scene destruction. It
returns an owned `BasePoseWorldTransforms` table and scene-wide `Option<Aabb>` in the Scene's current
coordinate convention and units. It composes authored base poses as `parent_world * local`, without
sampling animation. Bounds transform every Position vertex per instance, including vertices omitted
from the draw indices; uninstanced meshes do not contribute, and no vertices produce `None`.
Bound skins and effective nonzero morph weights return a `SpatialQueryError`. The existing
`ApplicationAsset::from_scene` extraction path remains available for those assets. The caller supplies
`SpatialQueryLimits` and optional cancellation; the same limits apply independently to transform
construction and bounds traversal. Both outputs survive Scene destruction. `world_bounds` tests use
a self-authored shared-instance glTF with ancestor transforms, nonuniform/negative scale, shear and
an unreferenced vertex, plus the unchanged rigged, morph and animated upstream models.

Preparation preserves mesh and primitive order. Handles belong to one Scene generation, so the
application gathers the full selection from the imported Scene and submits it in one call before
coordinate conversion.
It does not reacquire targets between per-primitive edits. The two-polygon integration fixture
exercises multiple primitives in that selection. The unchanged real
models and their independent expected facts are documented in [fixtures/README.md](fixtures/README.md).

The borrowed `list_vertices` reader is a zero-allocation view of `ListVertex` tuples. Its `index()`
is the expanded output tuple ordinal; `source_corner()`, `source_vertex()`, and `source_face()`
retain the original endpoints for field and deformation reads. `triangle_vertices` is the strict
TriangleList convenience returning the same `ListVertices` view; it does not expand other modes.
Indexing is private application policy; the consumer reserves its index buffer from the expanded
`tuples.len()`, and its buffer/map allocations use fallible reservation. The library checks the
expanded tuple count against u32 and returns `VertexLayoutError::SizeOverflow` on overflow.
Allocation or count failures propagate through the application `Result`, including that typed
error and structured `TryReserveError` and `TryFromIntError` causes. Application allocations are
outside the library's operation ledger.

The independent `primitive_modes` tests directly extract self-authored glTF data covering all seven
modes with indexed and nonindexed streams, exact source Corner and Face order, a degenerate strip
triangle followed by an odd-parity triangle, fan order, and both directed edges of a two-point loop.
They verify unchanged source topology and indices, first-encounter deduplication, F64 attribute bits,
complete influence rows and morph deltas, shared geometry with instance-specific morph weights and
skin palette references, and owned buffers after Scene destruction. They exercise direct extraction
without calling the separate triangle processing workflow.

Indexing tests reconstruct every original draw Corner through the indices and compare all selected
F64 fields and exported F32 deformation values by `to_bits()`. They cover UV/Normal seams, same-source
signed-zero UV and Tangent/W-only seams, five-influence and empty rows, coincident source vertices,
repeat ordering, and owned data after Scene destruction. The quad produces 4 vertices / 6 indices;
both `BoxTextured` and `BoxInterleaved` produce 24 / 36. The existing workflow assertions retain
independent source counts: `RiggedSimple` is 160 / 564, `AnimatedMorphCube` is 24 / 36, and the two
`MorphPrimitivesTest` draws are 21 / 72 and 9 / 24.

## Advisory profiling

The thin Python driver builds one release executable and launches a fresh process for each
successful fixture. Python 3.10 or newer is required. These measurements are observations without
performance thresholds:

```text
python scripts/run_asset_profile.py --toolchain 1.95.0 --timeout 1800
python scripts/run_asset_profile.py --toolchain 1.95.0 --allocation-metrics --timeout 1800
python scripts/run_asset_profile.py --toolchain 1.95.0 --root /path/to/assets --asset model.obj --assume-standard-space --timeout 1800
python scripts/run_asset_profile.py --toolchain 1.95.0 --root target/asset-workloads-v1 --asset static-quads-128.obj --asset static-quads-256.obj --assume-standard-space --max-work-units 200000000 --repetitions 3 --timeout 1800
```

`--asset` can be repeated. `--artifact-root NEW_DIR` selects a new output directory; the default is
`target/asset-profiles/<UTC timestamp>`. An existing directory is rejected to preserve prior evidence.
`--timeout SECONDS` is required and applies to every tool/build/worker child. Choose a finite
positive value that accommodates the release build; timeouts retain logs and allow other cases to run.
Reports include Git revision/dirty state, `rustc -vV`, actual argv, return codes, elapsed process time,
per-stage consumer summaries, and separate stdout/stderr logs. On a failure, the partial report is
retained and the nonzero result is propagated. CI exercises the consumer on the three exact native
candidate hosts and retains both Linux release profile variants.
`--repetitions N` starts a new consumer process for every selected asset in each repetition;
it does not reuse an imported Scene or average away individual observations. Normal timing and
the separately built `--allocation-metrics` variant belong in different artifact directories.

Consumer summaries separate import, postprocess, provenance traversal and buffer extraction time.
`postprocess_work_actions` sums the coordinate-conversion and triangle-preparation reports,
with `postprocess_work_action_contract` identifying their bounded vocabulary;
`changed_operations` counts changed reports from those two calls.
`selected_geometry_reconstructions` comes from the preparation report's
`geometry_reconstruction_count()` and records the actual selected transaction count. It is separate
from the number of selected primitives, emitted draws and coordinate-conversion changes.
`traced_primitive_owners`, `traced_provenance_nodes` and `traced_source_objects` count the owners
and distinct retained ancestors actually reached. Library ledgers cover their existing operation
scopes; the application's visited sets and indexed buffers use application allocation accounting.
The V1 families and semantic expectations are described in
[fixtures/README.md](fixtures/README.md#v1-workload-definitions-and-regressions); actual executed
measurements belong to the revision-qualified
[expansion review](../../../docs/reviews/2026-11-01-asset-workflow-expansion.md).

The separate geometry-only [Assimp comparison protocol](../../../docs/testing/assimp-comparison.md)
uses `--comparison` in this executable. It measures explicit import/preparation/extraction/pipeline
lifetimes in source space and copies only selected F64 fields plus independent draw placements.
It does not perform the full material/image/animation extraction or provenance tracing above.
Generate its original single-file inputs before collecting measurements, supply the two prebuilt
probes and retain their exact build metadata:

```text
python scripts/generate_asset_workloads.py --comparison --output-dir target/assimp-comparison/corpus
python scripts/run_asset_profile.py --comparison-cases target/assimp-comparison/corpus/comparison-cases.json --baozi-probe /path/to/baozi-asset-consumer --assimp-probe /path/to/baozi-assimp-probe --assimp-double-probe /path/to/double/baozi-assimp-probe --timeout 120 --repetitions 20 --artifact-root target/assimp-comparison/B0
```

The comparator builds separately through [its CMake consumer](../assimp/README.md).
Three fresh warmups precede the retained observations, serial AB/BA order balances engine order,
and semantic/precision/IO failures remain visible without contributing ratios.
Process RSS and optional independently verified heap observations remain distinct from stage time.
Generated-case counts or a working invocation are not comparative performance results.

The memory measurements answer different questions:

| Field | Scope |
| --- | --- |
| `import_peak_live_bytes` | Peak library-accounted import reservations, including declared allowances; not an exact allocator or process measurement. |
| `max_postprocess_peak_live_bytes` | Maximum of individual postprocess operation peaks, not their sum or an aggregate physical peak. |
| `allocations.requested_peak_bytes` | Optional calling-thread allocator-request peak during root open, import, edits, and extraction; excludes other threads and allocations outside the Rust allocator. |
| `allocations.requested_total_bytes` / `allocation_requests` | Cumulative requested bytes and allocation requests in that same instrumented scope. |
| `allocations.requested_retained_bytes_after_scene_drop` | Requested bytes still retained for application buffers and original detached import evidence after Scene destruction; not a leak assertion. |
| `peak_rss_bytes` in the Python report | Individual child peak RSS from Linux GNU `/usr/bin/time -f %M`, converted from KiB; includes the executable/runtime and process pages. Other hosts report null with an explicit reason. |

The optional `allocation-metrics` feature adds allocator instrumentation and changes timings.
Uninstrumented runs emit `allocations: null`. Stage clocks exclude some setup and teardown; Python
elapsed time includes process startup. These small fixtures establish a reproducible functional and
measurement baseline, not a throughput benchmark or a production memory bound. Keep the two variants
separate, and record the host, source revision, toolchain, and source assets when comparing runs.
