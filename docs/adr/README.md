# Architecture Decision Records

This directory is the canonical index for Baozi architecture decisions. Only an `Accepted` ADR is
normative. `Proposed` ADRs are non-authoritative candidates; `Superseded` and `Rejected` ADRs are
historical context.

Run the structural governance gate from the repository root:

```console
cargo run -p baozi-xtask -- adr-check
```

## Metadata contract

ADR frontmatter is a strict schema defined by ADR 0028. Required fields are `status`, `date`,
`authors`, and `decision_type`. Optional fields are `updated`, `depends_on`, `related`, `supersedes`,
`superseded_by`, `evidence`, `tracers`, and `promotion_requires`. Lists use a two-space block list or
`[]`; unknown fields and duplicate keys fail the checker.

`depends_on` is normative. `related` is navigation only. Supersession uses reciprocal `supersedes`
and `superseded_by` lists. ADR 0037 and ADR 0038 cannot become Accepted without repository-local
implementation tracers.

A Proposed ADR may carry an explicitly isolated controlled-trial implementation and trial tracer.
That evidence creates no default, publication, compatibility, or production authority. Promotion may
atomically change the implementation, evidence, tracked tracer, and ADR status in one reviewed change;
only the resulting Accepted state makes the tracer authoritative.

## Decision ownership

Overlapping decisions must identify one canonical owner instead of relying on file number or date.
The key cross-cutting owners are:

| Concern | Canonical ADR |
| --- | --- |
| Overall Rust-owned import architecture and clean-room boundary | `0001` |
| Runtime neutrality and host concurrency admission | `0002` |
| Scene state, validation proof, and collection ownership | `0003` |
| Test layers, independent oracles, and external-oracle safety | `0005` |
| Public compatibility, canonical v1, lockstep release topology, and publish recovery | `0006` |
| Per-field validation, complete digest, preview, and fuzz evidence | `0026` |
| Format maturity and promotion evidence | `0011` |
| Workspace allowed/required/forbidden edges, feature composition, and CI membership | `0007` |
| Parser backend admission and replacement | `0004` |
| Math types, SceneSpace, units, numeric policy, and high-precision CAD exclusion | `0008` |
| Data ownership, borrowing, and memory layout | `0009` |
| Asset IO, capability roots, and path security | `0010` |
| Material, image, texture, and sampler identity | `0012` |
| Postprocess plan compilation and algorithm admission | `0013` |
| Parser security, panic, unsafe, and FFI admission | `0014` |
| Option envelopes, precedence, typed compilation scopes/store, and ordinary codec errors | `0016` |
| Serialization, snapshots, and cache boundaries | `0017` |
| Parser tooling and generated-parser ownership | `0018` |
| Parser diagnostics, input consumption, and generated code | `0019` |
| Import lifecycle, stage proof, outcome, and failure transport | `0020` |
| Extension registry lifecycle, descriptors, trust, unwind mapping, health, and permanent poison | `0021` |
| Resource ledger and phase accounting | `0022` |
| Custom material and attribute extension schema | `0023` |
| glTF backend ownership and replacement | `0027` |
| Detection, stable input, and candidate evidence | `0029` |
| Logical meshes, primitives, and attribute domains | `0030` |
| Animation, skin, morph, and deformers | `0031` |
| Edit transactions, remaps, and derived invalidation | `0032` |
| Runtime-neutral operation control and observability | `0033` |
| Supported targets and portability tiers | `0034` |
| Text decoding and recovery | `0035` |
| Source identity, provenance, and metadata | `0036` |
| ADR lifecycle, controlled trials, atomic promotion, and authority | `0028` |

ADRs 0037 and 0038 are Proposed future contracts. They do not own current archive or export behavior
and cannot become authorities until their declared implementation tracers exist.

## Requirement-to-authority traceability

The implementation plan may combine work across units, but it does not override an Accepted ADR.
Each product requirement has one explicit authority set and a delivery unit; a change that alters the
contract must update the owning ADR before changing the implementation plan.

| Requirement | Canonical authority | Delivery |
| --- | --- | --- |
| R1 - bounded, order-independent detection | [ADR 0029](0029-format-detection-stable-input-and-failure-contract.md) | U1, U3 |
| R2 - bounded parser and graph work | [ADR 0003](0003-core-scene-ir-and-material-model.md), [ADR 0019](0019-parser-diagnostic-streaming-and-generated-code-contract.md), [ADR 0022](0022-per-import-resource-ledger-and-budget-accounting.md) | U1, U3-U8 |
| R3 - object-bound dependency access | [ADR 0010](0010-asset-io-virtual-filesystem-uri-archive-and-path-security.md), [ADR 0020](0020-import-lifecycle-and-report-contract.md) | U3 |
| R4 - non-bypassable final validation | [ADR 0003](0003-core-scene-ir-and-material-model.md), [ADR 0020](0020-import-lifecycle-and-report-contract.md) | U1, U3-U4 |
| R5 - stable primary and dependency snapshots | [ADR 0010](0010-asset-io-virtual-filesystem-uri-archive-and-path-security.md), [ADR 0020](0020-import-lifecycle-and-report-contract.md), [ADR 0029](0029-format-detection-stable-input-and-failure-contract.md) | U3 |
| R6 - success and failure evidence | [ADR 0020](0020-import-lifecycle-and-report-contract.md), [ADR 0029](0029-format-detection-stable-input-and-failure-contract.md), [ADR 0036](0036-source-identity-provenance-and-metadata.md) | U3 |
| R7 - attempted, live, committed, and peak accounting | [ADR 0022](0022-per-import-resource-ledger-and-budget-accounting.md) | U3-U8 |
| R8 - configuration, registry-health, and runtime-control ownership | [ADR 0016](0016-import-options-presets-and-configuration-precedence.md), [ADR 0021](0021-extension-registry-and-format-descriptor-governance.md), [ADR 0033](0033-operation-control-and-observability.md), [ADR 0035](0035-text-decoding-conformance-and-recovery.md) | U3, U5-U6 |
| R9 - logical meshes and primitives | [ADR 0030](0030-logical-mesh-primitive-and-attribute-domain.md) | U4 |
| R10 - domain-aware attributes | [ADR 0030](0030-logical-mesh-primitive-and-attribute-domain.md), [ADR 0023](0023-material-and-custom-attribute-extension-model.md) | U4-U7 |
| R11 - variable skin influences and matrix spaces | [ADR 0031](0031-animation-skin-morph-and-deformer-contract.md) | U4, U6-U7 |
| R12 - morph and animation preservation | [ADR 0031](0031-animation-skin-morph-and-deformer-contract.md) | U4, U6 |
| R13 - image identities, scene space, and numeric-scope boundary | [ADR 0012](0012-material-texture-image-and-color-space-policy.md), [ADR 0008](0008-math-coordinate-units-and-numeric-policy.md) | U4, U6 |
| R14 - identity, provenance, proof, remap, and invalidation | [ADR 0003](0003-core-scene-ir-and-material-model.md), [ADR 0032](0032-postprocess-remap-and-derived-data-contract.md), [ADR 0036](0036-source-identity-provenance-and-metadata.md) | U4, U7 |
| R15 - first-party format semantic preservation | [ADR 0004](0004-parser-backend-and-format-coverage-policy.md), [ADR 0011](0011-format-support-tiers-and-compatibility-charter.md) | U5-U6 |
| R16 - compiled typed postprocess plans | [ADR 0013](0013-post-process-pipeline-semantics-presets-and-mutation-model.md), [ADR 0033](0033-operation-control-and-observability.md) | U7 |
| R17 - postprocess evidence and validated output | [ADR 0013](0013-post-process-pipeline-semantics-presets-and-mutation-model.md), [ADR 0032](0032-postprocess-remap-and-derived-data-contract.md) | U7 |
| R18 - robust bounded triangulation | [ADR 0013](0013-post-process-pipeline-semantics-presets-and-mutation-model.md), [ADR 0032](0032-postprocess-remap-and-derived-data-contract.md) | U7 |
| R19 - architecture lifecycle, release topology/recovery, and target governance | [ADR 0028](0028-adr-lifecycle-and-evidence-governance.md), [ADR 0006](0006-public-api-versioning-and-crate-stability-policy.md), [ADR 0034](0034-supported-targets-and-portability.md) | U2, U9 |
| R20 - executable quality and maturity evidence | [ADR 0005](0005-testing-fuzzing-and-differential-oracle-strategy.md), [ADR 0006](0006-public-api-versioning-and-crate-stability-policy.md), [ADR 0007](0007-workspace-crate-graph-feature-flags-msrv-and-ci-gates.md), [ADR 0011](0011-format-support-tiers-and-compatibility-charter.md), [ADR 0026](0026-validation-snapshot-and-fuzz-gates.md), [ADR 0034](0034-supported-targets-and-portability.md) | U5-U9 |
| R21 - progressive canonical public workflow | [ADR 0006](0006-public-api-versioning-and-crate-stability-policy.md), [ADR 0020](0020-import-lifecycle-and-report-contract.md) | U4, U9 |

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
| [0028](0028-adr-lifecycle-and-evidence-governance.md) | ADR Lifecycle and Evidence Governance | Accepted | Normative |
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
