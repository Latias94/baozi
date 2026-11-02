# Baozi and Assimp after the Pre-Release Evolution

Date: 2026-11-03, Asia/Shanghai. Source: `088d7176d5982d1ae9e5045c0250dc6f608f66c1`,
with production runtime `52446ec8511b7dfba2e5352ce6941558d85e0ae2`.

This updates the development baseline in the [November 2 comparison](2026-11-02-baozi-assimp-comparison-and-roadmap.md).
The [evolution plan](../plans/2026-11-02-0923-refactor-pre-release-evolution-plan.md) and
[architecture continuation](../plans/2026-11-02-1700-perf-import-operation-architecture-plan.md)
own the delivered tranche. Its [final qualification](../reviews/2026-11-03-final-pre-release-qualification.md)
owns actual execution and any remaining gaps. This document proposes subsequent sequencing;
it does not authorize another implementation goal or registry operations.

## Current Comparison

Assimp comparison observations use release 6.0.5 at
`392a658f9c271be965271f45e7521a1b80ea4392`. Broader source capabilities in the older report
retain that report's separately identified development pin. Neither pin is described as the
latest release by this update.

| Area | Current Baozi result | Remaining difference or boundary |
| --- | --- | --- |
| Import breadth | Four documented STL, OBJ/MTL, PLY and selected glTF/GLB profiles | Assimp has much broader producer coverage. FBX, Collada, BVH and avatar workflows are not Baozi support claims. |
| Public import design | Typed format options, self-executing plans, borrowed execution contexts and private validated scene publication | This is a usable Rust importer seam, not C/C++ API compatibility or an out-of-core parser. |
| Work and ownership | One local work owner, independently surviving storage owners, immediate physical completion and bounded control checks | These contracts have costs. They do not prove universal security, speed or memory superiority. |
| glTF materials and lights | Metallic-roughness, Unlit, texture transforms, emissive strength, clearcoat, sheen and punctual lights through public reads and consumers | Transmission, IOR, volume and further source mappings remain finite future slices. Retained core factors alone are not producer support. |
| Geometry and animation | Declared F32/F64 geometry, seven glTF modes, sparse/implicit-zero accessors, admitted mesh quantization, complete influences, skin/morph/TRS evaluation | Geometry/texture compression and MikkTSpace remain unsupported. The existing tangent algorithm keeps its own name and restrictions. |
| Application output | Detached assets, complete concrete device remaps and an excluded wgpu PNG/JPEG rendering example | The concrete profile is not a universal renderer ABI, physically complete PBR implementation or browser rendering claim. |
| Rendered feedback | Named RTX 4090/Vulkan static and supported animated readback, with analytic expectations | One tested device/backend does not establish Apple, browser or every GPU/driver combination. |
| Cancellation and progress | Public cancellation token, deadline and observer cancellation; real-format control tests; honest unknown totals | Cooperative checks cannot interrupt an opaque blocking call. Decode/Normalize vocabulary does not imply detailed progress from every parser. |
| Owned glTF implementation | One metered production decoder, normative/reference/issue regressions and a direct public single-format importer consumer | The private document is not a general public glTF document library or a drop-in gltf-rs replacement. |
| Export and ecosystem | Import, selected processing and detached application output | Assimp has exporters and established C/C++ integration. Baozi has no interchange exporter or C ABI. |
| Distribution | Ten-package portable `0.1.0` train, with source-qualified local qualification | Registry availability, PLY/glTF publication flags, ownership and native target admission remain separate release work. |

The detailed format restrictions remain in the [format documentation](../formats/support-matrix.md).
The support matrix's `Unverified` declaration and unassigned maturity are unchanged; test count,
feature presence or local packaging does not promote them.

## Measured Costs

These are ordinary memory-input complete-pipeline medians in milliseconds for matching successful
semantic tasks. Windows and Linux are separate campaigns. Full reports retain file-input cases,
stage clocks, dispersion, failures and identities. Assimp uses an independently built F64 control
for the F64 PLY task.

| Task | Windows Baozi / Assimp | Linux Baozi / Assimp |
| --- | ---: | ---: |
| Repeated OBJ 16,384 | 90.450 / 9.406 | 88.782 / 8.817 |
| Grid OBJ 128 | 194.923 / 24.758 | 189.241 / 20.269 |
| Shared glTF 32 | 5.168 / 0.333 | 4.352 / 0.191 |
| Binary STL repeated 1,024 | 4.488 / 0.228 | 3.930 / 0.177 |
| PLY points 1,024 | 1.827 / 0.734 | 1.196 / 0.781 |

The [complete Windows report](../reviews/2026-11-03-windows-final-comparison.md) and
[corrected Linux B1/heap report](../reviews/2026-11-03-linux-b1-and-heap-comparison.md) each
have 110 lanes and 2,200 retained pairs. Current Linux repeated/grid pipelines improve 74.3%/65.3%
against immutable Baozi B0, while the Assimp time gap remains substantial on those tasks.
Those gains justify the retained changes, not a claim that optimization is finished.

The remaining stage costs differ by workload. Windows repeated OBJ 16,384 has
36.679/48.738/4.724 ms import/preparation/extraction medians; grid OBJ 128 has
179.001/0.151/14.966 ms. These independent endpoint medians are not added as an exact pipeline
decomposition. They do show why text/construction work, normal preparation and output reads
need separate hypotheses rather than one uniform streaming diagnosis.

Memory has no single winner. Whole-child Linux grid RSS falls from Baozi B0's 39.867 MiB to
34.625 MiB, compared with Assimp's 33.916 MiB. Separate heaptrack grid peaks are 28.83M/28.69M
decimal bytes for Baozi/Assimp; repeated OBJ is 12.18M/19.91M. These are different lifetimes
and methods. They cannot be interchanged with Windows working set, private commit or internal
admission counters.

All final Linux stage medians fit the frozen limits. Six parent-elapsed breaches do not recur
in same-lane repeat medians. Fifty-seven RSS breaches remain above their original limits and
receive explicit reviewed cost acceptance; their values and limits remain unchanged. Small-worker
exit maps support a resident-image contribution without explaining every lane's lifetime peak.
The initial Linux B1 controls were overwritten by heap builds; those runs remain disqualified.
The corrected full series and the binary-identity guard preserve this distinction.

## Next Development Order

| Order | Bounded slice | Start condition and acceptance |
| --- | --- | --- |
| 1 | Ordinary external adoption | Exercise installation, sidecar ownership, cancellation and detached output in one real caller. Keep unsupported outcomes explicit; turn actual failures into minimized public regressions. Distribution follows its separate registry policy. |
| 2 | Remaining demonstrated import costs | Select representative OBJ/glTF/STL workloads from that caller. Measure borrowed text, flat syntax staging, payload adoption or typed reader views independently. Retain a design only when ordinary timings/heap improve and semantic, admission and cancellation tests pass. |
| 3 | One workflow blocker | Choose one demanded material mapping, MikkTSpace or one geometry/texture codec. Deliver complete parsing, resource closure, numeric policy, validation, public reads and consumer interpretation with independent vectors. Do not bundle unrelated codecs or all PBR extensions. |
| 4 | One new producer audience | Choose binary FBX 7.x game exchange for DCC users, or VRM 1.0 for avatars. Start from licensed unchanged assets and one caller; write a finite preserve/loss/reject contract before changing canonical data. |
| 5 | Outbound use | If a converter/editor needs it, implement one GLB exporter under proposed ADR 0038: representability/loss preflight, complete buffered output and independent re-import expectations. Atomic native replacement remains a separate host capability. |

A public standalone glTF document crate is a separate product decision. Its first gate is a
reviewed consumer-facing API and an actual independent caller. If extracted, it must share the
existing parser, not create a second production JSON/schema implementation. R18/R19 reference
and issue coverage continues to distinguish normative conformance, deliberate policy, unsupported
extensions and gltf-rs API compatibility.

## When to Refactor Again

The operation and format interfaces were replaced because real measurements exposed repeated
hot-loop interpretation and ownership costs. Dense bounded OBJ remaps and stable normal buckets
then removed demonstrated algorithmic waste. Obsolete execution paths and aliases were removed.
This is the precedent for future breaking changes: identify the violated contract or measured
cost, design at its owner, qualify one replacement and delete the retired path.

Whole-Scene storage replacement, persistent caches, async scheduling and out-of-core parsing
need representative workloads that exceed the current lifecycle after simpler lifetime/copy
changes. Assimp is not uniformly streaming; its complete import path is not parser-only work.
Different aggregate timing alone does not identify a trait dispatch, allocation, IO or storage
cause. Keep the same successful semantic task and exact worker identity while testing a candidate.

A caller that reuses prepared import plans can justify a separate reuse workload. The existing
complete-pipeline contract includes importer/request setup, whereas the import endpoint prepares
them before its clock. Measure equivalent reuse on both engines and name the new series rather
than changing the frozen B0/B1 task or treating setup as proven parser cost.

Cargo packages remain `0.1.0`; Baozi-owned draft revision families are canonical v1. External
standards, dependency versions and historical records were not reset. The existing registry
`baozi 0.0.1` placeholder requires a separately authorized replacement/yank sequence.
