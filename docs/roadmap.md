# Baozi Roadmap

This roadmap keeps implementation order aligned with the architecture decisions. It is not a promise
that every listed format is already supported.

## Product Direction

Baozi should become a Rust-native importer with Assimp-class breadth over time. The path is not to
bind to another importer as the primary design. The path is to define a durable scene IR, importer
contract, IO policy, post-process pipeline, test oracle, and format promotion process, then add
formats through repeatable vertical slices.

## Milestone 0: Workspace Foundation

Status: in progress.

Exit criteria:

- workspace crate graph exists
- facade compiles with default features and `all-formats`
- core scene IR has IDs, materials, diagnostics, IO limits, and format registry contracts
- architecture ADRs cover the major stability boundaries
- CI-equivalent local commands pass

Key docs:

- ADR 0001 through ADR 0017
- `docs/model/scene-ir.md`
- `docs/security/parser-threat-model.md`

## Milestone 1: First Real Parser Slice

Recommended target: binary and ASCII STL.

Why STL first:

- small format surface
- no material sidecars
- good parser security exercise
- clear geometry-only snapshots
- useful baseline for post-process validation

Exit criteria:

- parser handles ASCII and binary STL detection
- malformed fixtures return structured errors, not panics
- resource limits are enforced before large allocation
- raw snapshot format exists in `baozi-test-support`
- support matrix records exact capability status
- fuzz target exists before beta promotion

## Milestone 2: Text Format and Sidecar Slice

Recommended target: OBJ with MTL.

Status: first experimental slice implemented.

Why OBJ second:

- separate indices force vertex remapping decisions
- polygon topology exercises triangulation boundary
- MTL exercises material and texture path policy
- sidecars exercise `AssetIo` and path security

Exit criteria:

- OBJ geometry, groups, smoothing metadata, and MTL references are represented for static face meshes
- importer preserves polygons before requested triangulation
- core `Mesh` face-boundary data and triangulation postprocess remain sufficient for OBJ polygons
- sidecar loading obeys `AssetIo` scope rules
- material mapping is documented against ADR 0012 and the OBJ format page
- snapshots or direct field assertions cover raw and post-processed scenes

## Milestone 3: Flexible Vertex Properties

Recommended target: PLY.

Why PLY third:

- property schema varies by file
- point clouds and vertex colors are common
- helps prove custom attribute policy from ADR 0015

Exit criteria:

- ASCII and binary little-endian PLY are handled first
- common position, normal, color, and UV properties map to typed channels
- unknown bounded properties map to namespaced custom attributes or diagnostics
- support matrix distinguishes point cloud and mesh capabilities

## Milestone 4: Modern Scene Format

Recommended target: glTF2 and GLB.

Why glTF fourth:

- validates PBR material model
- validates textures and color space policy
- validates skins, morph targets, and animation channels
- validates buffer and URI policy

Exit criteria:

- GLB and external-buffer glTF work through `AssetIo`
- metallic-roughness material mapping is documented
- skins, morph targets, and basic animation channels map without IR redesign
- strict and permissive import options are tested
- fixtures cover coordinates, cameras, lights, and animation timing

## Milestone 5: Post-Process Depth

Exit criteria:

- validation pass is implemented
- triangulation pass is implemented
- bounding box generation is implemented
- coordinate/unit normalization is implemented
- normal and tangent generation have documented math and tests
- presets expand to explicit pipelines
- destructive steps report diagnostics or stats

## Milestone 6: Format Promotion System

Exit criteria:

- every format has a capability document under `docs/formats/`
- stable promotion requires fixtures, fuzzing, differential evidence, and docs
- support matrix is generated or checked against crate metadata
- release notes distinguish parser maturity from crate existence

## Later Milestones

Potential later work:

- Collada parser
- FBX strategy and parser feasibility study
- IQM, MD5, MD2/MD3, and MMD family formats
- image decoding helpers behind optional features
- exporter crates
- persistent cache crate
- C ABI or dynamic plugin strategy

Each of these should get a design note or ADR before implementation if it changes public contracts.

## Success Metrics

| Metric | Target | Measurement |
| --- | --- | --- |
| Build health | workspace check, clippy, nextest, and deny pass | local/CI commands |
| Parser safety | malformed fixtures never panic | parser tests |
| Format honesty | support matrix never claims unimplemented support | docs review and tests |
| IR stability | STL, OBJ, PLY, and glTF map without replacing core scene types | milestone reviews |
| Post-process clarity | raw and processed snapshots are separately reviewable | golden tests |
| License hygiene | default dependency tree remains compatible with MIT/Apache-2.0 consumers | `cargo deny check` |

## Risks and Mitigations

| Risk | Severity | Likelihood | Mitigation |
| --- | --- | --- | --- |
| Parser count grows before core invariants are enforced | High | Medium | Keep milestones vertical and validator-first |
| Complex formats expose missing animation or material semantics | High | Medium | Use ADR 0012 and ADR 0015 before glTF stabilization |
| Snapshot format becomes noisy | Medium | Medium | Normalize snapshots separately from public API |
| Support claims outrun implementation | High | Medium | Keep support matrix conservative |
| Optional dependencies bloat default builds | Medium | Medium | Keep codecs and FFI out of default features |
