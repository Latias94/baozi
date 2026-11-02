# Owned glTF Conformance and Test Maturity

Date: 2026-11-02. This supplements the [evolution plan](../plans/2026-11-02-0923-refactor-pre-release-evolution-plan.md), especially R18/R19 and U14. Research and in-flight implementation are distinguished below; this is not a complete glTF conformance certificate.

## Current Ownership and Claim

[ADR 0027](../adr/0027-gltf-backend-ownership-and-replacement-policy.md) replaced the maintained gltf-rs distribution fork with Baozi's private metered syntax index and selected-profile document. `baozi-format-gltf` has no production `gltf`/`gltf-json` dependency. Its public API registers a format importer; consumers obtain canonical Baozi Scene values. It does not currently expose a general public glTF document/accessor library, export API or gltf-rs-compatible object graph.

The reason for owning the decoder was input-controlled allocation/work admission and source-range ownership. That decision also transfers responsibility for grammar, schema, defaults, reference/layout validation and extension-site behavior to Baozi. Passing its own round trips or snapshots cannot establish independent correctness. API extraction cannot resolve a parser defect by itself.

Three comparisons serve different purposes:

| Reference | Evidence it can provide | Limit |
| --- | --- | --- |
| Pinned Khronos specification, schemas and extension registry | Normative meaning, required validation and extension status | Optional extensions and drafts are not automatically required features of a finite importer profile. |
| Khronos Validator | Independent checks on original fixtures and unchanged model files | Its supported checks/extensions are finite; zero errors does not prove rendering, application ownership or every importer policy. |
| Published gltf-rs, separately labelled newer source, and issue regressions | An independent implementation, ecosystem expectations and useful failure scenarios | Acceptance, an open issue or an unchecked API panic is not the normative answer. Native layouts and API scope differ. |

Assimp is an additional scenario source. The test audit uses development pin `a2a586d9370a493651256ec5d03524710ce37a5f`; the benchmark still uses released v6.0.5 at `392a658f9c271be965271f45e7521a1b80ea4392`. Neither pin is silently substituted for the other.

## Existing Independent Evidence

The audit found meaningful existing coverage, rather than a suite of load-only smoke tests:

- glTF primitive-mode tests cover all seven core modes, indexed/nonindexed inputs, minimum sizes, restart sentinels and retained degenerates. Assimp's bad-face dropping is not adopted as an expected Baozi repair.
- Sparse tests cover malformed indices/values, normalized components, layout and resource limits. Influence tests retain more than four weights, independent joint palettes, zero lanes and ordering. Animation tests spell cubic derivatives and morph widths independently.
- The detached asset consumer reads material factors, texture channels, UV transforms and independent image/texture/sampler identities after original owners are destroyed. Numeric normal-generation and triangulation tests already exceed the corresponding Assimp allocation/count smoke assertions.
- Eight unchanged Khronos sample models are pinned in the [fixture inventory](../../tools/consumers/assets/fixtures/README.md). Source-derived counts, binds, channels, weights and selected position values are separate from Baozi snapshots. SimpleSkin remains a documented stricter quaternion-policy rejection, although the official Validator accepts it.

These observations do not establish complete extension coverage, low-level document API parity, deformed shading or successful independent use on an unexecuted target.

## Bounded Gaps and Planned Checks

| Gap within the current or already planned profile | Independent observation | Existing owner / unit |
| --- | --- | --- |
| Imported camera values and optional defaults | Asymmetric magnifications, FOV/planes/aspect, absent versus authored values, shared attachments and centimeter conversion | Asset `spatial_output` tests, U7 |
| Owned core-field and syntax validation | Valid controls plus wrong-type/default matrices; malformed transitions, supplementary-key identity and distinct Unicode normalization forms | glTF document/syntax tests, U14 |
| Nonidentity sparse inverse binds | Literal asymmetric MAT4 replacements and evaluated meaning, not only identity or dense/sparse agreement | glTF document compatibility, U14/U10 |
| GLB logical buffer versus physical padding | Admit legal BIN padding; prohibit buffer views/accessors from using the padding beyond declared `byteLength` | glTF document compatibility, U14 |
| Fuzz feature reachability and failure handling | Original sparse/cubic/camera/extension seeds reach named semantics; malformed/limited failures release ordinary state and avoid internal-contract outcomes | Existing glTF fuzz lane, U14 |
| Actual playback of admitted samples | Independently decode source bytes and calculate positions at a key and interval midpoint; retain weights, binds and source bits | Existing evaluator and render acceptance, U10 |
| Geometry edits composed with deformation/color | Resolve authored corner identities independently, then check complete skin/morph rows and Corner/Face color seams after detachment | Asset semantic composition, U9/U10 |
| Cross-format record fidelity | Unequal OBJ index tables at record time; distinct STL facet scalar bits; PLY list-skip alignment; absent/reused material assignments | Existing format and asset targets, U14 |
| Direct single-format use | Public importer registration and owned semantic reads outside facade feature unification, with pinned external comparisons | Excluded conformance consumer, U14/R18 |

The current assurance amendment preserves the fifteen-unit development objective and frozen B0/B1 corpus. New optional material/codec/animation features require complete producer, validation, retained meaning and consumer evidence through their named slice; a registry entry alone does not imply support.

## Executed Test-Sensitivity Corrections

The first implemented additions strengthen existing OBJ/PLY/STL targets with original in-memory data:

| Execution | Result |
| --- | --- |
| OBJ/PLY `geometry` targets, Rust 1.95, Windows x86_64 MSVC | 27/27 pass, nextest `a78832de-9dd4-4ae6-9f81-b122306ab242` |
| STL `binary`, including independent ASCII/binary two-facet payloads | 10/10 pass, nextest `8b418200-b878-468c-9185-efc4cadeefda` |
| C++ F32 semantic suite on Linux/WSL | 10/10 pass after the independent status/CLI expectations; the final explicit missing-option addition also passes the `json` baseline below |
| C++ F64 semantic suite on Linux/WSL | 7/7 pass with the final strengthened test source |
| Three localized C++ fault injections | Overflow guard removal, duplicate guard removal and semantic-status mislabelling each fail their intended new assertion; unmodified `json` baseline passes |

The fault-injection run at `2026-11-02T04:17:40Z` uses test SHA-256 `c4e950beeb19575c040b4a3adab7e04899d57701bd5ea603862f408f56932236` and unchanged production-probe SHA-256 `0bb9d9fd2eadc6f49297b634b9d31aac951560a29a3c3c4aa90d8d73cf30049b`. Temporary mutated translation units and binaries are isolated under `target/review/comparison-test-sensitivity/`; production sources and B0 artifacts are untouched. `result.json` retains the exact failing assertions and logs. This establishes sensitivity to those three faults, not a general mutation score.

The fixture inventory audit also found one stale local `expectations.json` checksum. Its current bytes, including U5's documented quaternion interpretation, hash to `74a73e41dbbb45d0b6a4960274f21281b08a55e7eedef11f8f16d3fb38e02092`. Refreshing that inventory entry makes all 63 entries match. No upstream sample bytes or independent expected values were changed to obtain passing tests.

Independent review found no defect in the bounded format-test/protocol/inventory changes and
verified the three temporary fault sources and their failure logs. Execution remains main-owned;
`target/review/test-maturity/verification.json` binds those test source hashes and run identities.
The format/import strict Clippy and format checks also pass. The reviewer separately identified a
plan gap: an excluded conformance consumer would not be reached by root workspace tests or the
existing facade package cases. The verification table and Definition of Done now explicitly require
its own manifest, separate Validator execution and direct version-only archive-consumer execution.

## Public API Direction

A direct single-format importer consumer is the immediate useful check. A separately usable raw glTF document/accessor crate is a distinct API commitment: it must specify validation versus unchecked reads, lifetimes, IO/resource ownership, precision, extension representation, errors and bounded execution. A gltf-rs drop-in interface is another compatibility promise again. Do not label these interchangeable.

Any justified extraction must move the existing single production parser behind its reviewed boundary and remove the superseded path. It must not introduce a second decoder, expose private `ParseScope`-bound internals as a shortcut, or add reference loaders/pixel codecs to the public production feature closure. The architecture review and current normative/issue inventory refine this route before extraction work.

## Sources and Provenance

- [Owned backend decision](../adr/0027-gltf-backend-ownership-and-replacement-policy.md), [current selected glTF profile](../formats/gltf.md), [fixture policy](../testing/snapshot-and-fixture-policy.md), [behavior gates](../testing/behavior-gates.md).
- [Pinned Assimp glTF tests](https://github.com/assimp/assimp/blob/a2a586d9370a493651256ec5d03524710ce37a5f/test/unit/utglTF2ImportExport.cpp) and the same pin's OBJ/PLY/STL, material, scene and postprocess unit tests supply scenario research. No upstream test code or model bytes are copied.
- [Khronos glTF specification](https://github.com/KhronosGroup/glTF/tree/main/specification/2.0), [extension registry](https://github.com/KhronosGroup/glTF/blob/main/extensions/README.md), [Validator](https://github.com/KhronosGroup/glTF-Validator), [gltf-rs source](https://github.com/gltf-rs/gltf) and [issues](https://github.com/gltf-rs/gltf/issues). Moving links identify the projects; current-source conclusions require the retrieved exact revisions recorded by the ecosystem audit.

Further owned-document regressions, the current standards/issues comparison, direct conformance-consumer execution and integrated portable/package qualification remain in progress. This record does not assign Beta/Stable maturity or authorize publication.
