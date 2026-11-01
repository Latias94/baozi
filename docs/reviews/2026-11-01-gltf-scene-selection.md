---
status: in_progress
date: 2026-11-01
---

# Explicit glTF scene selection: execution review

This record covers the [scene-selection execution plan](../plans/2026-11-01-007-feat-gltf-scene-selection-plan.md)
after `01fe7e1`. Together with complete influence sets in `660a4f2`, this completes candidate 6
of the [asset workflow plan](../plans/2026-11-01-001-feat-asset-workflow-expansion-plan.md).
Implementation identity and integrated verification are recorded after source freeze below.

## Behavior and decisions

`GltfImportOptions` owns `GltfSceneSelection::{DocumentDefault, SceneIndex(u32)}`. Its schema is
`org.baozi.gltf` / `import-options`, version 1. The codec uses ADR 0016's existing metered
readers, registry defaults, preset/caller layers and frozen request store. A complete default
payload is `[1, 0]`; an explicit index is `[1, 1, <u32 LE>]`; a dynamic `[0]` layer inherits.
Unknown presence bits/tags, truncation and trailing bytes fail request compilation. Explicit
`DocumentDefault` overrides a preset selection. Retained option bytes are the actual inline
`size_of::<GltfImportOptions>()`; there is no side configuration channel or new dependency.
The public facade exposes these types behind `format-gltf`.

After global schema/index, graph and declared-root validation, the selector resolves the frozen
request before dependency IO. Explicit indices override a valid declared default and may select
any declared scene, including an empty scene. Out-of-range indices return `gltf.invalid_reference`
with subject `scene-selection` and the source index. `DocumentDefault` still resolves a declared
default, a sole scene, or a zero-scene parentless projection. Ambiguous multi-scene documents
return `gltf.scene_selection_required`. Explicit zero never invokes the zero-scene projection.

The existing descendant and forward resource closure is reused. Shared roots are retained once;
selected root and child ordering remain source ordering. Unselected payloads are not opened,
while invalid source indices/graphs and scene-root membership still reject the entire document.
Skins cannot add otherwise unselected joints or skeletons. Animation projection retains the
existing source-semantic-loss authority, clip identity/time domain/original duration, channel
ordering and source indices. No union, node cloning, alternate Scene or runtime is introduced.

Shared report evidence now requires a `SourceScopeSelectionRequest` together with the resolved
`SourceScopeSelectionOutcome`. This enum describes the resolved format request in a report;
the caller's input remains format-owned. The builder checks document identity and explicit
request/outcome agreement before adopting evidence storage. The v3 source-scope digest frames
the request before the existing outcome/document and ordered observations. Per-entity detail,
digest-byte and item budgets retain their semantics. The fixed request is inline; complete and
collapsed records, success and later failure all retain it with the same full observation digest.
The old builder signature and v2 digest domain are replaced directly.

Normalized glTF meaning carries the frozen request across backend teardown. The synthetic root's
`org.baozi.gltf:projection-root-v2` operation hashes the request and actual projection in a fixed
18-byte frame; its source parents remain the actual document and selected scene. Two requests
resolving to the same source scene therefore have identical source parents and different options
digests. The implicit sampler's v1 instruction stays unchanged. This changes provenance content
without changing canonical snapshot wire layout or ownership.

The semantic reference is the
[pinned official Khronos scenes specification](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/Specification.adoc#scenes),
already retained locally by the sparse slice. Deterministic defaults and the parentless-node
projection follow Baozi's [ADR 0039](../adr/0039-source-document-scene-selection-and-import-closure.md).
No Assimp implementation or fixture was copied.

## Review and verification

Three native Codex agents implemented the option codec and two disjoint new importer/consumer
test files. The option worker also reviewed the integrated production contracts read-only.
The main thread owns selection/evidence integration and all serial Cargo execution.
The production contract review found no additional confirmed defect beyond the empty-scene
decoder correction. A separate audit confirmed that Attribute and MorphDelta borrowed owners
already use canonical global descriptor rows; the animation identity correction does not
require changing those readers.

New tests cover six option cases, sixteen importer cases, one direct backend case, three
shared-evidence cases and three independent consumer cases. They exercise preset/caller precedence and request reuse; malformed
transport and exact option budgets; explicit nonzero, JSON/GLB, ambiguous, empty and zero-scene
behavior; maximum index bounds; shared roots and source ordering; omitted resource closure;
skin membership; mixed animation losses, duration and stable identity; request/outcome mismatch;
v3 digest vectors and exact evidence budgets; collapse/later-failure transport; and generated
root request-sensitive options with unchanged actual source parents.

The consumer fixtures use two projections with a shared logical mesh, distinct buffers/materials,
unavailable omitted resources and selected source JSON metadata. They import through the public
facade, copy into `ApplicationAsset::from_scene`, destroy the report/Scene/IO/importer/request,
inspect detached selection evidence, destroy that evidence and reconstruct owned draw tuples,
indices, transforms, material factors and source metadata. Consumer production code is unchanged.

The initial host compile required constructing non-exhaustive `ResourceLimits` through its
default and public fields. The first option test run exposed a test wrapper that did not forward
format detection; it now delegates detection as well as options/parsing. These were test-fixture
integration corrections, with no production validation relaxation.

The first complete format run passed 291/292 cases and exposed an actual public-reader defect:
`AnimationChannelRef::metadata_owner()` used an authored channel source ordinal as a global
canonical owner row. Filtering made the owner invalid or selected another retained channel.
The borrowed view now retains its actual global row, independently of its source provenance.
Regressions cover both non-contiguous filtered ordinals and repeated source ordinal zero across
two retained clips, through random access and iteration.

Source review also found that the maintained decoder required the optional scene `nodes` field.
The positive empty-scene fixture had used an explicit empty array, while the
[official scene schema](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/schema/scene.schema.json)
permits omission and requires a nonempty array when present. A direct backend and explicit-selector
red run both reproduced `missing field nodes` with a legal omitted field. The existing backend
field now has Serde's default empty root list, matching its existing omission on serialization;
the maintenance inventory records this correction. The direct regression validates JSON/GLB and
serialization roundtrip without Baozi's panic containment. The existing model and validator are
reused. All three targeted repair cases then passed (`3114c562-1ab5-4d56-b508-f4f7891a06ad`).

| Executed check | Result | Run identity |
| --- | --- | --- |
| Shared source-scope tests | 19/19 pass | `d94e0629-cb32-4223-a05f-cb42b1554315` |
| New independent consumer cases | 3/3 pass | `af41cd51-59b7-457b-8d02-3192049ea238` |
| Focused option/selection/direct-backend tests | 30/30 pass | `29f3b8a9-1ca4-4527-b1ad-03b28eb7118f` |
| glTF format package | Pending | Pending |
| Windows workspace, all features | Pending | Pending |
| Independent asset consumer, all features | Pending | Pending |
| Workspace strict Clippy | Pass, all targets/features, `-D warnings` | Local log |
| Consumer strict Clippy and formatting | Pending | Pending |

## Qualification boundary

Rust 1.95.0, locked/offline dependencies, one Cargo build job and the existing target directory
are used on x86_64-pc-windows-msvc. Fixtures are independently authored and bounded. Cancellation
checkpoints are inherited from the existing graph/closure path; this slice does not claim a new
stage-targeted cancellation campaign, runtime platform result, sanitizer/fuzz campaign,
performance result, format maturity promotion or package release qualification.
