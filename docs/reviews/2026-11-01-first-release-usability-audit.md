---
title: First-Release Usability and Assimp Comparison
date: 2026-11-01
status: source-audited
---

# First-Release Usability and Assimp Comparison

## Decision and evidence boundary

Prepare a useful 0.1.0 around complete, documented import-to-application workflows.
Do not freeze a compatibility facade around today's awkward interfaces merely
to publish sooner. Preserve the typed immutable Scene and its explicit ownership,
attribute domains, source evidence and transactional edits; change a specific
interface when a real caller exposes a false invariant or duplicated work.

This audit follows the user's explicit request to reassess functionality and
interfaces against Assimp before the first canonical release. The
[preparation plan](../plans/2026-11-01-010-chore-portable-first-release-plan.md)
owns its execution. Actual publication, native registration, format maturity and
target promotion remain separate decisions.

| Subject | Audited identity |
| --- | --- |
| Baozi source baseline | `f08d39d37ae44baae856cc05a918efaf31613324`; current portable implementation is unchanged from the `ec2958f` unlit slice, apart from subsequent packaging/governance work. |
| Assimp source | Clean local `repo-ref/assimp` at `a2a586d9370a493651256ec5d03524710ce37a5f`, described as `v6.0.5-179-ga2a586d93`. This is a development revision, not a claim about a later release. |
| Comparison method | Source/interface and existing-consumer review. No Assimp binary execution, new differential corpus or measured speed comparison. |
| New changes in this slice | Repository-only dev-dependency packaging and the user-directed placeholder retirement policy. Functional findings below describe the baseline unless marked resolved here. |

The earlier [gap analysis](../research/2026-11-01-baozi-assimp-gap-analysis.md)
is historical. Tangent ownership/generation, base-pose world queries, all seven
glTF primitive modes, sparse accessors, complete influence sets, explicit scene
selection, unlit materials and normal maintained native dependency resolution
are now implemented. They must not be counted again as unresolved gaps.

## First candidate package and format set

All packages below are 0.1.0. This is the complete portable candidate and a valid
dependency order, not a registry publication receipt:

| Order | Package | Role and current eligibility |
| --- | --- | --- |
| 1 | `baozi-operation` | Required supporting mechanisms; not a second end-user facade. |
| 2 | `baozi-io` | Portable byte, memory and custom asset IO. |
| 3 | `baozi-core` | Immutable Scene, typed readers and central validation. |
| 4 | `baozi-import` | Import orchestration and experimental parser-authoring extension surface. |
| 5 | `baozi-format-stl` | Default format; binary/ASCII triangle profile. |
| 6 | `baozi-format-obj` | Default format; documented OBJ/MTL subset and scoped sidecars. |
| 7 | `baozi-postprocess` | Optional explicit processing; publish after its versioned OBJ development dependency and before the facade. |
| 8 | `baozi-format-ply` | Opt-in ASCII/binary geometry and point-cloud profile; `publish = false` pending release qualification. |
| 9 | `baozi-gltf-json` | Maintained upstream JSON model/validator; preserves original authorship/licenses. |
| 10 | `baozi-gltf` | Maintained upstream reader; hidden behind the Baozi format adapter. |
| 11 | `baozi-format-gltf` | Opt-in selected glTF/GLB profile; `publish = false` pending release qualification. |
| 12 | `baozi` | Application facade, published last. |

`default-formats` remains STL and OBJ/MTL. `all-formats` adds PLY and the
[selected glTF/GLB profile](../formats/gltf.md). `postprocess`, `tracing` and
`mint` retain their explicit feature behavior. Optional production dependencies
still belong to the release closure even when the default build disables them.
Retain path-plus-version production edges and the exact maintained glTF sibling
edge; no consumer patch is part of this candidate.

Excluded: test-support, xtask, fuzz, consumers, reference trees and all four native
packages (`baozi-native-io-candidate` and the three `baozi-cap-*` helpers).
Native packages remain `publish = false`; no native target is registered. The
backend reader's optional upstream `import`/image features are distinct from
Baozi's selected profile and do not imply a Baozi pixel decoder or ambient IO.

## Comparative capability map

Assimp references are pinned to the audited revision. Features in either project
must be evaluated as actual mappings and workflows, not merely retained fields.

| Workflow or capability | Baozi now | Assimp comparison and first-release implication |
| --- | --- | --- |
| Format breadth | Four explicit profiles: STL, OBJ/MTL, PLY and selected glTF/GLB. | Assimp's [format inventory](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/doc/Fileformats.md) includes substantially more importers. FBX/Collada/USD/VRM breadth is a separate product slice, not evidence that existing profiles are unusable. |
| Application import | `Importer::new`, bytes, memory bundles and custom `AssetIo`; typed reports and explicit per-format options. | Assimp's [Importer](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/Importer.hpp) provides path/memory entry points and configurable IO. The ordinary local-file/sidecar installation story deserves explicit first-user acceptance. |
| Scene and geometry | Validated immutable owners; generation-qualified borrowed reads; logical meshes, primitive instances and Vertex/Corner/Face domains. | Assimp's [Scene](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/scene.h) and [Mesh](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/mesh.h) use a mutable pointer graph and primarily vertex channels. Preserve Baozi's identities/domains rather than flattening to that representation. |
| Geometry processing | Explicit triangulation, flat/smooth normals, attribute-owned Triangle UV Derivative tangents, whole-Scene space/unit conversion, borrowed list/triangle tuples. | Assimp's [postprocess flags](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/postprocess.h) additionally expose joining/splitting/optimization and many lossy transformations. Prioritize common caller workflows; do not replicate its flag scheduler. Baozi's tangent algorithm does not claim MikkTSpace parity. |
| Application buffers | Independent consumer performs deterministic tuple packing, retains application-owned data after Scene drop, and preserves skin/morph streams. | Distinguish demonstrator packing from a reusable public packing contract. Source identity and every selected stream constrain deduplication. |
| Materials and images | Typed PBR/unlit and legacy factors, distinct texture/image/sampler identities, UV transforms, encoded image bytes and explicit unsupported selected extensions. | Assimp's [material keys](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/material.h) cover more source models. A property bag or extension-name whitelist does not establish correct shading semantics. |
| Skin/morph/animation | Variable influence rows, instance palettes/corrections, morph deltas/defaults/overrides, f64 channel time and retained cubic derivatives. | Assimp's [animation model](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/include/assimp/anim.h) has different representations. Retention and extraction are distinct from evaluated poses; neither importer API alone is a complete playback engine. |
| Spatial queries | Read-only base-pose world transforms and static instance/Scene bounds with explicit unsupported evaluated/deformed cases. | Keep the pose scope visible. Current static bounds must not become an animated culling promise. |
| Loss, diagnostics and provenance | Explicit loss authority, structured failures, selected source scope, source/operation lineage readers and resource manifests. | Retain these distinctions. More accepted files are not an improvement if unsupported semantics become silent successful output. |
| Distribution and targets | Patch-free local artifact rehearsal; exact registry names observed; source and artifact runs have revision-specific scopes. | Installation from public crates.io and current host qualification still require separate evidence. A successful local archive is preparation. |

## Public registry observation

The [normalized observation](../research/2026-11-01-portable-release-registry-observation.json)
records all twelve names, HTTP statuses, UTC timestamps and response hashes. On
2026-11-01 from `15:29:27Z` through `15:29:54Z`, eleven names returned HTTP 404.
Only `baozi` was present, with 0.0.1, `yanked = false`, and observed owner
`(kind = user, id = 179538, login = Latias94)`; separate user/team endpoints agreed
on one user and zero teams. None of the twelve had a candidate 0.1.0 version.

This is a documentary observation, not proof of ownership by the acting session,
a namespace reservation or ADR 0006's approved publication-state manifest.
No credentials or Trusted Publisher configurations were inspected. An initial
capture stopped on TLS EOF; a complete recapture had one TLS EOF and succeeded on
one retry after two seconds. Failed transports were never interpreted as absence.
Raw responses and transport receipts are retained locally under
`.git/baozi-review/portable-first-release/`.

The release owner explicitly chose to retire 0.0.1 **after** the complete canonical
train is verified through downloaded archives and a fresh registry-only facade
consumer. The [ADR amendment](../adr/0006-public-api-versioning-and-crate-stability-policy.md#2026-11-01-retire-the-placeholder-after-a-usable-replacement)
and [release policy](../contributing/release-policy.md) now agree. A partial train
leaves retirement pending; a failed yank is a separate retry, not a rollback.
No publication, yank, owner change, push or PR occurred in this slice.

## Correctness findings

Severity here describes first-release impact. A source-confirmed path is not an
executed reproducer; individual entries state the evidence held. Missing breadth
and explicitly rejected profiles are not counted as silent-loss defects.

### A1 — P1: glTF backend allocation allowance can undercount the decoded graph

[`read_normalized_gltf`](../../crates/baozi-format-gltf/src/parser.rs:194)
reserves `JSON bytes * 64 + 64 KiB` before deserializing the owning upstream
object graph. The formula in
[`gltf_backend_allocation_allowance`](../../crates/baozi-format-gltf/src/parser.rs:464)
does not account for the concrete schema-type size or container growth. Graph
node admission occurs later in `preflight_node_graph`, after backend allocation.

A document with 65,536 empty nodes and an empty default scene is a counterexample:
196,669 JSON bytes reserve 12,652,352 bytes, while executed backend decoding on
Windows stable/release produced capacity 65,536 of 264-byte Nodes, or 17,301,504
bytes for the node array alone. Small JSON objects expand into the optional fields of
[`json::scene::Node`](../../vendor/gltf-json/src/scene.rs:17). Even the node array
alone exceeds the entire opaque allowance. A smaller 4,096-node case also passed
the full public import: 1,081,344 node-array bytes versus an 855,872-byte allowance.
This finding concerns an incorrect
accounting envelope; it does not establish an executed OOM, abort or allocator-wide
resident-memory measurement.

**Release gate:** establish an allocation model that covers the actual retained
types, capacities and temporary allocations before backend decode, or replace
that decode path with budget-controlled allocation. Reuse the existing syntax
admission where appropriate, but do not close the finding by increasing an
unproven byte multiplier. Cover dense empty schema objects, nested collections,
extras, strings, invalid input and tight-budget refusal. Include the known
malformed-input backend regressions. The selected glTF profile stays ineligible
for publication until this admission issue is resolved.

### A2 — P1: PLY built-in attributes silently narrow source precision

[`ScalarValue::to_f32`](../../crates/baozi-format-ply/src/parser.rs:2160)
casts I32/U32/F64 and checks only finiteness. Positions, normals and UVs use this
path. Distinct double positions `16777216` and `16777217` collapse to one F32
position; `1e-100` underflows to zero. Unlike the already corrected color path,
this projection does not consult loss authority or emit loss evidence. An executed
public import of the two adjacent doubles returned two `16777216.0` positions
and zero loss records under the default policy.

**Release gate:** exact projections pass; inexact projections fail under the
default preserve-or-fail policy. Explicitly authorized narrowing records the
source attribute/record and the actual change. Cover ASCII, little-endian and
big-endian input, large adjacent values, integer precision, underflow and signed
zero. Current canonical Position requires F32; promising F64 positions requires
a separate complete precision contract through validation, readers and affected
operations. A parser-only type substitution is insufficient.

### A3 — P1: normal edits can orphan Tangent morph deltas

The [normal transaction](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/edit/transaction.rs:829)
changes base geometry without checking Normal/Tangent morph dependencies. The
[rewrite](../../crates/baozi-core/src/canonical_scene/store/construction/node_scene/edit/graph.rs:147)
shares deformation storage unchanged outside space conversion. The tangent
transaction already has a dedicated morph capability check.

Replacing a base Normal changes the interpretation of retained Normal deltas;
removing a base Tangent can leave Tangent deltas without their corresponding
base field. Structural validation alone does not establish preservation of the
old deformed shape. An executed dense glTF fixture with Normal/Tangent morph
deltas successfully published after Replace/Remove: the base Tangent was absent
while both Normal and Tangent deltas remained. Sparse variants and independent
pose evaluation were not newly executed. The Normal-only branch still needs an
independent case and an explicit rebasing policy; retention by itself is not
reported as a newly executed Normal-only defect.

**Release gate:** initially reject affected combinations with a typed capability
error and return the original Scene. Cover dense/sparse Normal deltas and Tangent
removal with Tangent deltas; assert unchanged generation, snapshot and payload
on failure. Preserve valid Position-only morph cases. Rebase/rebuild semantics
are a later explicit operation, not an implicit normal-generation side effect.

### A4 — P2: MTL alpha mode retains stale derived state

[`d` and `Tr` parsing](../../crates/baozi-format-obj/src/mtl.rs:448)
sets Blend when opacity falls below one but does not reset it. Final
[`material_from_parsed`](../../crates/baozi-format-obj/src/mesh_builder.rs:1110)
can therefore publish alpha one with Blend after `d 0.25` followed by `d 1`.
The public memory-sidecar reproducer returned exactly opacity 1 and Blend.

**Fix:** derive alpha mode once from final admitted opacity and relevant authored
semantics; remove the redundant sticky cache. Test repeated/interleaved `d`/`Tr`
and complete material redefinitions through public factors and alpha-mode reads.

### A5 — P2: custom reader destruction escapes the IO fault classification

[`read_opened_asset`](../../crates/baozi-import/src/scope.rs:4362)
contains read-call panics, but reader destruction can occur outside that catch.
A custom reader with a panicking `Drop` can escape a primary import as a host
panic; the same sidecar fault can reach the outer parser catch and poison the
format registration as an importer fault. This is a trusted host-extension fault,
not an arbitrary asset-byte exploit. No new destruction-panic test ran here.

**Fix:** put cleanup at its owning IO fault boundary, preserve the original
read/control failure and classify destruction consistently. Test EOF, read error,
cancellation and identity-rejection cleanup, followed by a healthy import using
the same registry. Do not weaken the hostile-input parser boundary to accommodate
a host callback failure.

### A6 — P2: PLY metadata construction creates unnecessary quadratic work

[`metadata_insertion_work`](../../crates/baozi-format-ply/src/parser.rs:3144)
charges a triangular sum and
[`insert_metadata_parts`](../../crates/baozi-format-ply/src/parser.rs:3204)
scans all preceding keys. Indexed comment/obj-info keys are structurally unique.
20,000 short comments create 20,002 entries and 200,050,003 insertion work units,
already above the default 200,000,000 total work limit for a roughly 200 KB file.
The rejection is fail-closed; the avoidable cost is the defect.
The executed 200,106-byte fixture failed with attempted WorkUnits 200,250,122
after including the other admitted parsing work. This is an admission observation,
not evidence of executing two hundred million comparisons, a timing benchmark
or a resident-memory measurement.

**Fix:** append unique generated keys and use one bounded, cancellable final
ordering step. Verify stable metadata semantics and growth across several input
sizes using ledger work, rather than a timing threshold or a larger default cap.

### A7 — P2: image and texture source names are conflated

[`texture.name().or_else(image.name)`](../../crates/baozi-format-gltf/src/parser.rs:2218)
is retained under a single
[Texture metadata owner](../../crates/baozi-format-gltf/src/parser/normalized.rs:977).
An authored image name can disappear when its texture also has a name; without
a texture name the image name is assigned to the texture owner. Source indices
remain distinct, so this is descriptive source loss, not broken object identity.

**Fix:** retain each Image/Texture/Sampler name on its own existing source-object
owner. Leave display-name fallback to applications. Test two differently named
textures sharing one independently named image, and preservation through edits.

## Interfaces and workflows to complete before first release

These are usability decisions, not claims that the underlying retained data is
missing or that every corresponding Assimp operation should be copied.

| Priority | Caller obstacle and evidence | Recommended interface and acceptance |
| --- | --- | --- |
| High | Per-primitive normal/triangulation/tangent edits repeatedly reconstruct whole geometry and require new-generation IDs. The [application loop](../../tools/consumers/assets/src/lib.rs:158) exposes this directly; the [earlier measured workload](2026-11-01-asset-workflow-expansion.md:212) recorded 9.201x cumulative normal-edit work when shared primitive count rose from 8 to 32. | Add operations over a selected primitive set: one bounded plan, one reconstruction, one publication and per-target relations. Cover 32/128/512 fixed-size primitives, semantic equivalence, unchanged unselected fields, cancellation and all-or-nothing failure. Do not introduce a general pass scheduler. |
| High | All modes import and borrowed `list_vertices` assembles strips/fans, but normal/tangent generation and winding-changing conversion require lists. `triangulate_primitive` handles polygons, leaving a legal import-to-processing gap. | Add explicit strip/fan list lowering using existing assembly rules and transactional remaps. Preserve parity, winding, degenerates, Corner/Face fields, influences, morphs and source metadata. Then complete conversion and field generation on the lowered result. |
| High | Correct normal generation can require the application's intermediate CCW processing space; [`PostProcessControl`](../../crates/baozi-postprocess/src/operation.rs:80) also lacks configurable work/byte limits, while the runtime uses fixed caps. | Build a small named triangle-preparation recipe on batch/lowering operations, with explicit conventions, normal/tangent choices and limits. Cover source/target CW/CCW and reflection; an application should not reproduce the hidden ordering rule. Reuse existing budgets and failure ownership. |
| High | The [copy-out consumer](../../tools/consumers/assets/src/buffers.rs:331) copies UV0 while binding `uv_set` may refer to UV1. It omits sampler/channel/color/strength/blend/media-type information that is already publicly readable. Its README correctly declares a subset. | Define an application output profile that either preserves every selected binding's referenced UV and sampling semantics or rejects that binding explicitly. Test shared image/different samplers, UV1, normal/occlusion strengths and complete output after Scene drop. No canonical material rewrite or pixel decoder is required. |
| Medium | A glTF primitive without an authored material keeps `None`; consumers must know glTF's implicit PBR default, while an explicit empty material has typed factors. Existing unlit tests deliberately assert absence. | Decide and document source binding versus effective material. Prefer a generated default with provenance or an equivalent format-neutral effective-material query. Preserve authored absence and source-scope counts; implicit default and explicit `{}` must have equivalent effective shading factors. This is a contract decision, not a proven parser regression. |
| Medium | Nodes expose children, but parent and node-to-instance queries require an application index or repeated inventory scans. | Add a bounded relations view or retained relations at the existing seam. Document construction/storage costs and keep getters allocation-free. Cover forests, multiple placements, shared meshes and stale generations; avoid a hidden full-scene scan per getter. |
| Medium | glTF options are facade-exported, whereas ordinary OBJ text options require another format dependency. Portable local-file/sidecar loading also needs a clear first-user recipe because native remains separate. | Make common format options consistently reachable and provide runnable bytes, memory-sidecar and host-file integration examples with explicit IO authority and limits. Do not replace scoped IO with a misleading ambient path helper. |
| Medium | Animation/skin/morph extraction is implemented, but no small consumer proves their combined evaluation contract. | Add a CPU-only reference consumer with hand-calculated Step/Linear/CubicSpline, quaternion, morph-override and two-joint skin results, including space-conversion equivalence. Keep evaluation owned by the application; a general playback engine is outside this release. |

The batch geometry operation is the clearest justified refactor: it moves repeated
whole-Scene work and generation bookkeeping behind one useful interface while
preserving the existing storage and transaction invariants. General mutable Scene
construction, an Assimp compatibility facade and a second Scene representation
are not justified by these findings.

## Distribution findings and bounded fix

The prior archive rehearsal successfully built downstream consumers, but an
archived facade used as a root package could not resolve the versioned
`baozi-test-support` dev-dependency. That helper deliberately remains unpublished.
The new `dfd512f` fix makes the workspace development edge path-only, so Cargo
omits it from normalized manifests. All five affected extracted roots now resolve
without the helper. No production edge was removed or replaced by a patch.

Remaining distribution work before the real train:

- Both maintained backend manifests still claim Rust 1.61, while maintained JSON
  source uses `let ... else` and resolved dependencies require later Rust. Align
  the declared minimum with verified support (the Baozi train uses 1.95), or
  independently prove a lower contract; the upstream-MSRV maintenance wording
  must agree.
- The maintained reader's crate documentation still installs upstream `gltf`
  version 1. Correct its package identity and direct applications to the facade.
  Verify the chosen docs.rs feature scope; default facade docs do not show every
  optional operation.
- Excluded backend packages retain upstream default `import` and all-feature
  docs.rs settings, including optional image/base64/urlencoding dependencies.
  The facade's selected feature union does not exercise them. Choose and verify
  the maintained scope explicitly; no compile failure is inferred merely from
  this missing evidence.
- Root `-p` publication templates do not select excluded vendor packages. Define
  their `--manifest-path` and reviewed lock inputs before the locked real train.
  The local checker stages a workspace and does not settle that procedure.
- Keep PLY/glTF `publish = false` until the correctness findings and a finite
  profile-specific release record close. Existing fixtures, limits, fuzz and
  artifact evidence are useful and must be reused by scope. Do not require Stable
  maturity as an invented condition for an explicitly experimental 0.1.0.

## Executed checks and target evidence

New runs use x86_64-pc-windows-msvc, stable Rust 1.99.1
(`58cf8f9c35b355199727627d07ae34a4e2846d67`), one Cargo build job and release
optimization. Their tested production source is baseline `f08d39d` plus the root
manifest patch now committed in `dfd512f`. There are no new production Rust
changes. Actual archives retain their local dirty rehearsal identity; they are
not clean-tag publication artifacts.

| Newly executed check | Result and boundary |
| --- | --- |
| Complete portable archive rehearsal | 12 actual archives; 12/12 fixed consumer feature cases build and execute, exit 0, 542.791 seconds. Fresh artifact outputs under `target/package-check-ou4ck17o`; no native packages included. |
| Previously failing archive root | Before: metadata resolution fails on missing `baozi-test-support`. After: facade and four format roots all resolve from unmodified normalized manifests, and the helper is absent. |
| Independent diagnostic consumer | Five issue probes reproduce backend allowance undercount, PLY precision loss, stale MTL mode, PLY comment-work rejection and retained morph deltas after Tangent removal. Version-only roots use the actual staged archives. These probes expose defects; they are not passing correctness regressions. |
| Review | Five read-only native Codex lanes covered package closure, target evidence, public semantic flow, failure/IO behavior and geometry design. Two subsequent independent cross-checks confirmed the highest findings and evidence boundaries; no blocking report/policy finding remained. All agents were closed. |
| Source/format revisions | Existing MSRV/source functional results are reused only for unchanged code and retain their original revision identities. No full workspace rerun or new sustained fuzz campaign is claimed. |

Executed package command:

```text
python scripts/check_packages.py --toolchain stable --release --offline --target x86_64-pc-windows-msvc
```

Diagnostic source, fixture, exact commands, logs, public registry responses and
the twelve archive digests are retained under
`.git/baozi-review/portable-first-release/`. The local reproduction consumer is
small and disposable; each production correction should move its relevant case
into the existing focused test target, not grow a new audit framework.

The local `evidence.zip` is 2,311,131 bytes, SHA-256
`f8c8a22d199852e91abdd908274d8d5f3cd1981944fc2d138d26b1ec689022a5`.
It retains public API receipts, logs, diagnostic source/fixture/lock, actual
maintained archives, the old failing facade archive and the staged consumer
manifest/lock. It is local evidence, not a published CI artifact or registry
release bundle.

| Target | Existing or new evidence | Still missing for this first release |
| --- | --- | --- |
| Windows x86_64 MSVC | `ec2958f`: 1.95.0 all-feature workspace 2,677 passes, four existing manual skips; asset consumer 43 passes, one existing manual skip. `e7e9b00`: 12/12 MSRV/debug artifact cases. This slice adds 1.99.1/release 12/12. | Final corrected-subject regression/package evidence and release qualification. Existing successes did not catch the confirmed semantic defects. |
| Linux x86_64 GNU | `e7e9b00`: 1.95.0/debug 12/12 artifact cases through unprivileged Ubuntu 26.04 WSL. Earlier focused geometry and sanitizer evidence has its own revision. | Current corrected-subject stable/release and affected feature/runtime gates; WSL evidence is not a hosted receipt. |
| Apple ARM64 | Historical `0bb29f1` hosted run `41201133748`: actual host tests/docs and MSRV/debug plus 1.99.1/release package matrices. | Later features and the current release subject have no new Apple execution receipt. CI configuration remains present; no current green result is inferred. |
| Browser wasm32 | `ec2958f`: 1.95.0 glTF library compile; earlier complete consumer compile has narrower source coverage. | Current per-feature compile and browser runtime if claimed. No browser execution claim exists. |
| WASI preview 1 | Historical `0bb29f1` 1.95.0/release empty/all source consumers executed on Wasmtime 50.0.0; later primitive-mode slice compiled. | Current sparse/influence/selection/unlit and corrected-subject runtime/artifact evidence, plus selected canonical digest vectors. |
| Other targets | ADR 0034 Build Monitored candidates, dependency-only audits and negative native compile checks. | These are not positive runtime support evidence. No new supported target is selected here. |

ADR 0034 still has no release-qualified target. Keep current-source host evidence,
canonical golden consistency, Portable Byte scope and profile promotion separate.
Because correctness changes are now required, defer remaining broad target runs
until the corrected subject stabilizes; do not spend another full matrix to
qualify code already known to need replacement.

## Recommended completion order

The [next implementation plan](../plans/2026-11-01-011-feat-first-release-usability-plan.md)
defines finite units and acceptance cases:

1. Close the three P1 admission/precision/morph findings and the smaller confirmed
   alpha/cleanup/metadata defects, with focused public regressions.
2. Deliver selected-primitive batch preparation, strip/fan lowering, explicit
   winding order and configurable limits using the existing transaction.
3. Complete the application material/UV/sampling profile, effective defaults,
   navigation/options and a small reference animation consumer.
4. Decide any additional bounded light/material extension slice from a real asset,
   then qualify the corrected twelve-package subject and final target claims.
5. Under future publication authorization, verify the registry-only installation
   path for the complete train and retire 0.0.1 afterward.

The first release is not ready on the audited baseline. It has a useful core and
substantial implemented functionality, but the confirmed correctness findings
and the ordinary application seams above deserve priority over more format names
or additional governance infrastructure.

## Breadth and deliberate limits

For a broader first glTF workflow, the next two useful bounded extension slices
are `KHR_lights_punctual` and `KHR_materials_emissive_strength`. Each needs typed
mapping, references, units, selected closure, provenance, conversion, public reads
and independent fixtures. Current selected unsupported payloads are rejected;
they are not silently discarded. A release may retain that documented restriction
instead of claiming either extension prematurely.

FBX, USD, Collada, VRM, compression backends, IOR/volume material expansion,
exporters, pixel decoding, animation playback, CPU geometry baking, cache
optimization, cross-source welding and device draw/palette splitting remain
separate scope decisions. MikkTSpace-compatible generation is a distinct content
profile. Select these with a concrete asset or device requirement; none replaces
the correctness gates above.
