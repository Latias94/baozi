# Architecture Decision Records

This directory is the canonical index for Baozi architecture decisions. Only an `Accepted` ADR is
normative. `Proposed` ADRs are non-authoritative candidates; `Superseded` and `Rejected` ADRs are
historical context.

## Metadata contract

ADR frontmatter is a strict schema defined by the
[ADR governance policy](../contributing/adr-governance.md). Required fields are `status`, `date`,
`authors`, and `decision_type`. Optional fields are `updated`, `depends_on`, `related`, `supersedes`,
`superseded_by`, `evidence`, `tracers`, and `promotion_requires`. Lists use a two-space block list or
`[]`; unknown fields and duplicate keys are review failures. The policy also governs reviewable
amendment history for authority-affecting updates; frontmatter alone is not an approval record.

`depends_on` is normative. `related` is navigation only. Supersession uses reciprocal `supersedes`
and `superseded_by` lists. Promotion requirements are stable gate IDs interpreted by the policy; a
tracer is always one exact tracked regular file, never a directory or glob.

A Proposed ADR may carry an explicitly isolated controlled-trial implementation and trial tracer.
That evidence creates no default, publication, compatibility, or production authority. Promotion may
atomically change the implementation, evidence, tracked tracer, and ADR status in one reviewed change;
only the resulting Accepted state makes the tracer authoritative.

## Review checklist

Before accepting an ADR change, reviewers manually verify the following structural and lifecycle
invariants:

- the numbered file, heading, title, status, and index row agree, and each active ADR has exactly one
  canonical index entry;
- frontmatter uses only the fields defined by the governance policy, with valid scalar/list syntax,
  unique keys, and no duplicate normalized paths;
- active and retired IDs are disjoint and exhaustive through the current maximum, retired IDs are
  permanent and well-formed, and a new ID is greater than that maximum;
- every `depends_on` target is Accepted and the dependency graph is acyclic; `related` is reviewed as
  navigation only;
- `supersedes` and `superseded_by` are reciprocal and legal for the involved lifecycle states;
- local links, fragments, fenced examples, and referenced paths resolve with exact casing and remain
  repository-contained; and
- evidence, tracers, promotion gates, and conformance claims are directly relevant and backed by
  review records or the applicable executable gates.

These checks support the index; they do not replace semantic review. Reviewers still decide whether
an ADR has a single owner, whether prose conflicts with another Accepted ADR, whether an amendment
should be a supersession, and whether implementation evidence actually demonstrates conformance.

## Decision ownership

This table is the canonical concern-to-owner registry. The living governance policy defines how
registry changes are reviewed. The table assigns ownership but does not restate the owned decision:
the linked Accepted ADR remains the sole authority for that
decision's semantics. Overlapping decisions must update this registry and identify one owner instead
of relying on ADR number, file date, or merge order.

| Concern | Canonical owner | Ownership boundary |
| --- | --- | --- |
| Top-level architecture and crate-family responsibility, including the shared `baozi-operation` foundation | ADR 0001 | Assigns implementation families; detailed semantics remain with their domain ADRs. |
| Runtime neutrality, host session admission, executor capacity, and subscriber ownership | ADR 0002 | ADR 0033 governs control inside one admitted operation. |
| Canonical Scene aggregate, construction/draft/proof/edit states, and scene-local identity | ADR 0003 | Geometry, deformation, mutation, and persistent identity are delegated below. |
| Parser backend admission, isolation, and replacement | ADR 0004 | Does not own format maturity or facade features. |
| Test layers, fixture provenance, independent oracles, and external-oracle safety | ADR 0005 | Per-field evidence mechanics belong to ADR 0026. |
| Public compatibility, stability tiers, breaking changes, lockstep release topology, publication recovery, and versioning | ADR 0006 | Domain ADRs define semantics but cannot create a second compatibility or release policy. |
| Production crate graph allowed/required/forbidden edges, facade feature algebra, MSRV, and baseline CI membership | ADR 0007 | Format maturity and target promises remain separate decisions. |
| Math semantics, numeric storage, units, transforms, `SceneSpace`, and the high-precision CAD exclusion/reopening gate | ADR 0008 | Memory layout remains private under ADR 0009. |
| Ownership representation, borrowing/lifetimes, zero-copy boundaries, and memory layout | ADR 0009 | Does not redefine semantic Scene ownership. |
| Asset IO capabilities, logical paths, object-bound authorization, physical resource opening, resolver semantic identity, and atomic failure-plus-negative-observation caching | ADR 0010 | Public/canonical proof semantics belong to ADR 0041; public source identity belongs to ADR 0036. |
| Format maturity, managed `BaoziVerified` evidence/issuer, capability status, promotion, demotion, and joined support claims | ADR 0011 | Joins ADR 0021 declaration identity/authority to exact managed tested implementation/build evidence outside registry freeze; it does not define `Unverified`/`SelfAttested` or issue runtime producer, closure, cache, or build identity. |
| Material, image, sampler, texture, binding, and color interpretation | ADR 0012 | Source-specific bounded extensions belong to ADR 0023. |
| Import/postprocess separation, typed plan compilation, postprocess outcome vocabulary, presets, orchestration, replay policy, and triangulation admission | ADR 0013 | Transaction/remap mechanics belong to ADR 0032. |
| Parser trust boundaries, panic containment, unsafe code, native FFI, security-sensitive dependency admission, and isolation claims | ADR 0014 | Hostile in-process code is never treated as sandboxed; native filesystem mechanics remain under ADR 0010. |
| Common/format option envelopes, precedence, codec compilation semantics/scopes, typed compiled store, and ordinary compile failures | ADR 0016 | Registry lifecycle, unwind mapping, and permanent poison belong to ADR 0021; live control remains outside frozen options. |
| Rust API serialization, test snapshot, and negative prerequisites for any future persistent cache | ADR 0017 | Snapshot encoding and coverage mechanics belong to ADR 0026; this ADR authorizes no persistent-cache key, schema, owner, feature, or implementation. |
| Format-owned parser tooling and generator selection | ADR 0018 | Parser collections use ADR 0022's `baozi-import` metered-storage owner; backend admission remains under ADR 0004. |
| Diagnostic shape, parser input-consumption modes, recovery delivery, and generated-code contract | ADR 0019 | Outcome finalization belongs to the owning operation. |
| Import lifecycle, `ParseScope`, detection permit, inert format-failure packing, phase evidence, `ImportReport`, and `ImportFailure` | ADR 0020 | Postprocess has a distinct lifecycle and result. |
| Registry/descriptor/extension lifecycle, declaration identity, declaration `Unverified`/issuer-scoped `SelfAttested` schema and issuer validation, dependency-role contracts, snapshot/unload semantics, callable trust, unwind mapping, shared health latch, and permanent poison | ADR 0021 | Managed `BaoziVerified` maturity evidence belongs exclusively to ADR 0011 and is not stored in the registry; exact detector/parser invocation belongs to ADR 0020; options belong to ADR 0016; future producer/build identity remains non-authoritative under ADR 0043. |
| Attempted/consumed/live-attribution/committed operation semantics, ledger dimensions, and unique ParseScope-bound metered temporary storage | ADR 0022 | `baozi-operation` provides ledger mechanisms; `baozi-import` owns parser storage wrappers; operations own phase vocabularies; retained physical allocation lifetime belongs to ADR 0044. |
| Material/custom-attribute descriptors and bounded namespaced extensions | ADR 0023 | Canonical typed fields remain owned by their semantic ADRs. |
| Per-field validation, stable digest, bounded preview, direct tests, and fuzz reachability | ADR 0026 | Broader portfolio/oracle policy remains under ADR 0005. |
| glTF semantic ownership and private backend replacement boundary | ADR 0027 | General backend admission remains under ADR 0004. |
| Format detection, shared stable primary input, direct-byte snapshot ownership, fresh session authority isolation, selection, and detection failure evidence | ADR 0029 | Direct bytes grant no dependency authority; general authorization remains under ADR 0010 and import finalization under ADR 0020. |
| Logical mesh, primitive topology, attribute domains, and their cardinality | ADR 0030 | Deformer semantics belong to ADR 0031. |
| Skin, influence, morph, animation, and deformer semantics | ADR 0031 | Structural edit/remap behavior belongs to ADR 0032. |
| `SceneEdit` transactionality, entity/domain remaps, replay publication effects, and derived-data validity | ADR 0032 | Plan compilation/orchestration remains under ADR 0013. |
| Operation cancellation, deadlines, progress, callback safety, and tracing semantics | ADR 0033 | Hosts still own concurrent-session admission and trace subscribers. |
| Supported target profiles, portability promises, and promotion/demotion | ADR 0034 | Feature inventory and CI wiring remain under ADR 0007. |
| Text decoding, source mapping, conformance, and deterministic recovery | ADR 0035 | Format options authorize only format-declared recovery. |
| Persistent source identity, declaring-origin-relative dependency identity, provenance, lineage, aliases, redaction, and metadata | ADR 0036 | Dense scene-local identity remains under ADR 0003; authorization remains under ADR 0010. |
| Source-document scene selection, projection closure, cross-scene references, and source-scope evidence semantics | ADR 0039 | Format ADRs own selector identities; ADR 0020 owns sealed report transport; ADR 0036 owns identity/digest/provenance/loss representations. |
| Camera and light projection/emission semantics, attachment, validation, conversion, and loss | ADR 0040 | ADR 0003 owns aggregate membership; ADR 0008 owns generic math and `SceneSpace`. |
| Resource dependency graph, public semantic quotient, origin-relative resolution, negative-input replay classification, and canonical successful-manifest transcript | ADR 0041 | ADR 0010 owns authorization and atomic physical failure-plus-proof caching; ADR 0020 owns outcome transport; ADR 0021 owns dependency roles; ADR 0036 owns public identity/digest types; ADR 0039 owns source-document projection. |
| Retained physical allocation ownership, physical storage accounts, final-owner deallocation, and cross-generation account lineage | ADR 0044 | `baozi-operation` owns the deep retained-allocation protocol; domain crates receive semantic construction capabilities and non-owning projections; ADR 0022 owns operation attribution and logical committed evidence; ADR 0003/0032 own Scene generations and edit semantics. |
| Archive/container support | None while ADR 0037 is Proposed | ADR 0037 owns no crate, feature, capability, or public promise until Accepted promotion. |
| Export transactions and export loss | None while ADR 0038 is Proposed | ADR 0038 owns no crate, feature, capability, or public promise until Accepted promotion. |
| Producer attestation, closed-input qualification, and import build identity | None while ADR 0043 is Proposed | ADR 0043 owns no production/default/cache/cooker/release authority until an atomic Accepted promotion satisfies its gates. |

ADRs 0015, 0024, 0025, and 0042 are historical and own no active concern. Their reciprocal
supersession links point to ADRs 0030/0031, ADR 0006, ADRs 0003/0032, and ADR 0044 respectively.

## Requirement-to-authority traceability

These stable traceability labels locate the authoritative records for each architectural or
governance requirement. Accepted ADRs own architecture; the contribution policy owns ADR process.
The labels do not give implementation plans authority, and they deliberately omit delivery units
such as U1-U9. Plans may reference these labels and schedule work, but cannot change their meaning or
override the linked records.

| Requirement | Canonical authority |
| --- | --- |
| R1 - bounded, order-independent detection | [ADR 0029](0029-format-detection-stable-input-and-failure-contract.md) |
| R2 - bounded parser and graph work | [ADR 0003](0003-core-scene-ir-and-material-model.md), [ADR 0019](0019-parser-diagnostic-streaming-and-generated-code-contract.md), [ADR 0022](0022-per-import-resource-ledger-and-budget-accounting.md), [ADR 0041](0041-resource-dependency-graph-and-successful-input-manifest.md) |
| R3 - object-bound dependency access | [ADR 0010](0010-asset-io-virtual-filesystem-uri-archive-and-path-security.md), [ADR 0020](0020-import-lifecycle-and-report-contract.md) |
| R4 - non-bypassable final validation | [ADR 0003](0003-core-scene-ir-and-material-model.md), [ADR 0020](0020-import-lifecycle-and-report-contract.md) |
| R5 - stable primary and dependency snapshots | [ADR 0010](0010-asset-io-virtual-filesystem-uri-archive-and-path-security.md), [ADR 0020](0020-import-lifecycle-and-report-contract.md), [ADR 0029](0029-format-detection-stable-input-and-failure-contract.md) |
| R6 - success and failure evidence | [ADR 0020](0020-import-lifecycle-and-report-contract.md), [ADR 0029](0029-format-detection-stable-input-and-failure-contract.md), [ADR 0036](0036-source-identity-provenance-and-metadata.md), [ADR 0039](0039-source-document-scene-selection-and-import-closure.md) |
| R7 - attempted, consumed, live, committed, peak, and retained physical accounting | [ADR 0022](0022-per-import-resource-ledger-and-budget-accounting.md), [ADR 0044](0044-retained-physical-storage-ownership-and-account-lineage.md) |
| R8 - configuration, registry health, source selection, and runtime-control ownership | [ADR 0016](0016-import-options-presets-and-configuration-precedence.md), [ADR 0021](0021-extension-registry-and-format-descriptor-governance.md), [ADR 0033](0033-operation-control-and-observability.md), [ADR 0035](0035-text-decoding-conformance-and-recovery.md), [ADR 0039](0039-source-document-scene-selection-and-import-closure.md) |
| R9 - logical meshes and primitives | [ADR 0030](0030-logical-mesh-primitive-and-attribute-domain.md) |
| R10 - domain-aware attributes | [ADR 0030](0030-logical-mesh-primitive-and-attribute-domain.md), [ADR 0023](0023-material-and-custom-attribute-extension-model.md) |
| R11 - variable skin influences and matrix spaces | [ADR 0031](0031-animation-skin-morph-and-deformer-contract.md) |
| R12 - morph and animation preservation | [ADR 0031](0031-animation-skin-morph-and-deformer-contract.md) |
| R13 - appearance identities, camera/light semantics, scene space, and numeric-scope boundary | [ADR 0012](0012-material-texture-image-and-color-space-policy.md), [ADR 0008](0008-math-coordinate-units-and-numeric-policy.md), [ADR 0040](0040-camera-and-light-canonical-semantics.md) |
| R14 - identity, provenance, proof, remap, invalidation, and generation storage lineage | [ADR 0003](0003-core-scene-ir-and-material-model.md), [ADR 0032](0032-postprocess-remap-and-derived-data-contract.md), [ADR 0036](0036-source-identity-provenance-and-metadata.md), [ADR 0044](0044-retained-physical-storage-ownership-and-account-lineage.md) |
| R15 - first-party format semantic preservation | [ADR 0004](0004-parser-backend-and-format-coverage-policy.md), [ADR 0011](0011-format-support-tiers-and-compatibility-charter.md), [ADR 0039](0039-source-document-scene-selection-and-import-closure.md), [ADR 0040](0040-camera-and-light-canonical-semantics.md) |
| R16 - compiled typed postprocess plans and replay | [ADR 0013](0013-post-process-pipeline-semantics-presets-and-mutation-model.md), [ADR 0033](0033-operation-control-and-observability.md) |
| R17 - postprocess evidence and validated output | [ADR 0013](0013-post-process-pipeline-semantics-presets-and-mutation-model.md), [ADR 0032](0032-postprocess-remap-and-derived-data-contract.md) |
| R18 - robust bounded triangulation | [ADR 0013](0013-post-process-pipeline-semantics-presets-and-mutation-model.md), [ADR 0032](0032-postprocess-remap-and-derived-data-contract.md) |
| R19 - architecture authority, compatibility, target governance, and current conformance | [ADR governance policy](../contributing/adr-governance.md), [ADR 0006](0006-public-api-versioning-and-crate-stability-policy.md), [ADR 0007](0007-workspace-crate-graph-feature-flags-msrv-and-ci-gates.md), [ADR 0034](0034-supported-targets-and-portability.md) |
| R20 - executable quality and maturity evidence | [ADR 0005](0005-testing-fuzzing-and-differential-oracle-strategy.md), [ADR 0006](0006-public-api-versioning-and-crate-stability-policy.md), [ADR 0007](0007-workspace-crate-graph-feature-flags-msrv-and-ci-gates.md), [ADR 0011](0011-format-support-tiers-and-compatibility-charter.md), [ADR 0026](0026-validation-snapshot-and-fuzz-gates.md), [ADR 0034](0034-supported-targets-and-portability.md), [ADR 0041](0041-resource-dependency-graph-and-successful-input-manifest.md) |
| R21 - progressive canonical public workflow | [ADR 0006](0006-public-api-versioning-and-crate-stability-policy.md), [ADR 0020](0020-import-lifecycle-and-report-contract.md) |
| R22 - complete resource dependency graph and successful input manifest | [ADR 0041](0041-resource-dependency-graph-and-successful-input-manifest.md) |

## Retired ADR IDs

This unique, four-digit, numerically sorted table is the machine-readable registry for permanently
unavailable IDs. Every row uses the canonical `Permanently unassigned` disposition and a nonempty
reason. Active and retired IDs are disjoint and exhaustive from 0001 through their current maximum.
A retired ID has no ADR authority, cannot appear in the index, and is never reused; a new ADR takes
an ID above the maximum.

| ADR ID | Disposition | Reason |
| --- | --- | --- |
| 0028 | Permanently unassigned | Removed during the initial pre-release baseline because the process-policy record was not an architectural decision. |

## Index

| ADR | Title | Status | Authority |
| --- | --- | --- | --- |
| [0001](0001-baozi-assimp-compatible-architecture.md) | Rust-Owned Canonical Import Architecture | Accepted | Normative |
| [0002](0002-runtime-concurrency-observability.md) | Runtime Neutrality, Host Concurrency, and Execution Backends | Accepted | Normative |
| [0003](0003-core-scene-ir-and-material-model.md) | Canonical Scene IR Ownership and State Model | Accepted | Normative |
| [0004](0004-parser-backend-and-format-coverage-policy.md) | Parser Backend and Format Coverage Policy | Accepted | Normative |
| [0005](0005-testing-fuzzing-and-differential-oracle-strategy.md) | Testing, Fuzzing, and Differential Oracle Strategy | Accepted | Normative |
| [0006](0006-public-api-versioning-and-crate-stability-policy.md) | Public API, Versioning, and Crate Stability Policy | Accepted | Normative |
| [0007](0007-workspace-crate-graph-feature-flags-msrv-and-ci-gates.md) | Workspace Crate Graph, Feature Membership, MSRV, and CI Gates | Accepted | Normative |
| [0008](0008-math-coordinate-units-and-numeric-policy.md) | Math Types, Scene Space, Units, and Numeric Policy | Accepted | Normative |
| [0009](0009-data-ownership-zero-copy-lifetimes-and-memory-layout.md) | Data Ownership, Zero-Copy, Lifetimes, and Memory Layout | Accepted | Normative |
| [0010](0010-asset-io-virtual-filesystem-uri-archive-and-path-security.md) | Asset IO, Virtual Filesystem, URI, and Path Security | Accepted | Normative |
| [0011](0011-format-support-tiers-and-compatibility-charter.md) | Format Maturity and Evidence Charter | Accepted | Normative |
| [0012](0012-material-texture-image-and-color-space-policy.md) | Material, Image, Texture, Sampler, and Color-Space Policy | Accepted | Normative |
| [0013](0013-post-process-pipeline-semantics-presets-and-mutation-model.md) | Compiled Post-Process Pipelines and Triangulation Boundary | Accepted | Normative |
| [0014](0014-parser-security-unsafe-ffi-and-panic-boundary-policy.md) | Parser Security, Unsafe, FFI, and Panic Boundaries | Accepted | Normative |
| [0015](0015-mesh-topology-vertex-attributes-skinning-and-animation-semantics.md) | Mesh, Attribute, Skinning, and Animation Semantics | Superseded | Historical |
| [0016](0016-import-options-presets-and-configuration-precedence.md) | Import Options, Presets, and Configuration Precedence | Accepted | Normative |
| [0017](0017-serialization-snapshot-and-cache-boundary.md) | Serialization, Snapshot, and Cache Boundary | Accepted | Normative |
| [0018](0018-parser-tooling-and-format-owned-parser-policy.md) | Parser Tooling and Format-Owned Parser Policy | Accepted | Normative |
| [0019](0019-parser-diagnostic-streaming-and-generated-code-contract.md) | Parser Diagnostics, Input Consumption, and Generated Code | Accepted | Normative |
| [0020](0020-import-lifecycle-and-report-contract.md) | Import Lifecycle, ParseScope, and Outcome Contract | Accepted | Normative |
| [0021](0021-extension-registry-and-format-descriptor-governance.md) | Extension Registry, Format Descriptor, and Codec Governance | Accepted | Normative |
| [0022](0022-per-import-resource-ledger-and-budget-accounting.md) | Per-Import Resource Ledger and Phase Budget Accounting | Accepted | Normative |
| [0023](0023-material-and-custom-attribute-extension-model.md) | Material and Custom Attribute Extension Model | Accepted | Normative |
| [0024](0024-public-api-hardening-and-stability-tiers.md) | Public API Hardening and Stability Tiers | Superseded | Historical |
| [0025](0025-scene-ir-invariants-and-validation-boundaries.md) | Scene IR Invariants and Validation Boundaries | Superseded | Historical |
| [0026](0026-validation-snapshot-and-fuzz-gates.md) | Validation, Snapshot, and Fuzz Evidence Contract | Accepted | Normative |
| [0027](0027-gltf-backend-ownership-and-replacement-policy.md) | glTF Backend Ownership and Replacement Policy | Accepted | Normative |
| [0029](0029-format-detection-stable-input-and-failure-contract.md) | Format Detection, Stable Input, and Failure Evidence | Accepted | Normative |
| [0030](0030-logical-mesh-primitive-and-attribute-domain.md) | Logical Mesh, Primitive, and Attribute Domains | Accepted | Normative |
| [0031](0031-animation-skin-morph-and-deformer-contract.md) | Animation, Skin, Morph, and Deformer Contract | Accepted | Normative |
| [0032](0032-postprocess-remap-and-derived-data-contract.md) | Postprocess Remap, Edit Transaction, and Derived Data Contract | Accepted | Normative |
| [0033](0033-operation-control-and-observability.md) | Operation Control and Observability | Accepted | Normative |
| [0034](0034-supported-targets-and-portability.md) | Supported Targets and Portability | Accepted | Normative |
| [0035](0035-text-decoding-conformance-and-recovery.md) | Text Decoding Conformance and Recovery | Accepted | Normative |
| [0036](0036-source-identity-provenance-and-metadata.md) | Source Identity, Provenance, and Metadata | Accepted | Normative |
| [0037](0037-archive-and-container-security.md) | Archive and Container Security | Proposed | Non-authoritative |
| [0038](0038-export-transaction-and-loss-contract.md) | Export Transaction and Loss Contract | Proposed | Non-authoritative |
| [0039](0039-source-document-scene-selection-and-import-closure.md) | Source Document Scene Selection and Import Closure | Accepted | Normative |
| [0040](0040-camera-and-light-canonical-semantics.md) | Camera and Light Canonical Semantics | Accepted | Normative |
| [0041](0041-resource-dependency-graph-and-successful-input-manifest.md) | Resource Dependency Graph and Successful Input Manifest | Accepted | Normative |
| [0042](0042-shared-canonical-storage-accounts-and-generation-lineage.md) | Shared Canonical Storage Accounts and Generation Lineage | Superseded | Historical |
| [0043](0043-producer-attestation-closed-input-qualification-and-import-build-identity.md) | Producer Attestation, Closed-Input Qualification, and Import Build Identity | Proposed | Non-authoritative |
| [0044](0044-retained-physical-storage-ownership-and-account-lineage.md) | Retained Physical Storage Ownership and Account Lineage | Accepted | Normative |
