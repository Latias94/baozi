---
title: glTF Resource Ledger and Skin MVP - Plan
type: feat
date: 2026-07-09
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# glTF Resource Ledger and Skin MVP - Plan

## Goal Capsule

| Field | Value |
| --- | --- |
| Objective | Move `baozi-format-gltf` from a static-mesh-only experimental importer toward a trustworthy glTF MVP by wiring buffer data URIs into the resource ledger, importing node-level skin bindings correctly, hardening malformed fixture coverage, and updating fuzz/docs/CI evidence. |
| Authority | User authorization for fearless breaking refactors, ADR 0001, ADR 0002, ADR 0004, ADR 0005, ADR 0012, ADR 0014, ADR 0015, ADR 0022, ADR 0026, ADR 0027, existing `ImportContext`/`ResourceLimits`, and the 2026-07-09 read-only skin review. |
| Execution profile | Break public APIs where the current IR is structurally wrong; prefer owned Rust IR over compatibility shims; keep `gltf` crate private and replaceable; commit logical slices on `main`; preserve WASM/custom-IO compatibility. |
| Stop condition | Stop only if the change would require copying third-party implementation material, introducing native dependencies, silently bypassing `ImportContext`, or pretending unsupported glTF domains are imported. |
| Tail ownership | The executor owns implementation, docs, engineering memory, local gates, commits, push to `main`, and CI follow-up. |

---

## Product Contract

### Problem

The current glTF importer is useful for simple static meshes, but it has three foundation gaps that will become expensive if more formats land first.
First, glTF buffer data URIs are declared unsupported even though `ResourceLimits::max_data_uri_bytes` already exists, so the security contract is ahead of the parser.
Second, skinning is represented as `Mesh.skin`, but glTF skin is a node-level mesh binding: the same mesh can be instantiated by multiple nodes with different skins.
Third, detector and malformed-input coverage still rely on format-local shortcuts rather than the registry/import lifecycle as the single boundary.

This plan fixes the architecture now, while Baozi can still break APIs, instead of adding a skin MVP on top of a wrong mesh-level binding.

### Requirements

**IR and scene invariants**

- R1. Replace mesh-level skin binding with node-level mesh bindings:
  - add `MeshBinding { mesh: MeshId, skin: Option<SkinId> }`
  - move node mesh references from `Node.meshes: Vec<MeshId>` to `Node.mesh_bindings: Vec<MeshBinding>`
  - remove `Mesh.skin` or stop exposing it as an authoritative binding.
- R2. Validator enforces mesh binding invariants:
  - binding mesh IDs are in range
  - binding skin IDs are in range
  - joint streams are valid only when at least one binding references the mesh with a skin
  - joint indices are less than the bound skin's joint palette length.
- R3. Snapshot output shows node mesh bindings and enough skin/joint detail to catch incorrect node-level binding or inverse bind matrix import.
- R4. Existing STL/OBJ/glTF tests and postprocess logic are migrated to the new node mesh binding API.

**glTF buffer resource handling**

- R5. glTF buffer data URIs are supported for base64-encoded buffers through an importer-local decoder and `ImportContext` ledger methods.
- R6. Decoded data URI bytes count against both `max_data_uri_bytes` and `max_total_asset_bytes`.
- R7. Data URI decoding must reject oversized payloads before unbounded allocation when the encoded length makes the decoded upper bound knowable.
- R8. Malformed data URI syntax, unsupported non-base64 data URI payloads, invalid base64, and declared `buffer.byteLength` mismatches return structured `BaoziError`s without panic.
- R9. `ImportStats` exposes data URI byte counts so facade reports prove which byte budget was consumed.

**glTF skin MVP**

- R10. Import `gltf::Skin` into existing owned `Skin` IR:
  - `name`
  - ordered joint palette as `Vec<NodeId>`
  - optional `skeleton_root`
  - optional inverse bind matrices, kept empty when absent.
- R11. Preserve glTF joint palette semantics: `JOINTS_0` values index into the skin's joint list, not directly into glTF node indices.
- R12. Bind skins at node mesh instance level, not geometry level.
- R13. Support multiple nodes instantiating the same glTF mesh with different skins without corrupting either instance.
- R14. Import `JOINTS_0` and `WEIGHTS_0` as SoA streams and validate their length/range against the bound skin.
- R15. Remove `gltf.skin_ignored` and `gltf.skins_ignored` diagnostics when the importer successfully imports the skin domain; retain diagnostics for domains still not implemented, such as animations and morph targets.

**glTF parser hardening**

- R16. Remove hidden detector probe constants so format detection honors the registry's `DetectionOptions::max_probe_bytes` boundary.
- R17. Malformed fixture coverage includes invalid JSON, invalid GLB, missing `POSITION`, missing/short buffers, unsupported primitive modes, data URI errors, and skin/joint validation failures.
- R18. The importer remains private-backend-first: no public API mentions `gltf-rs` types and no downstream code depends on `gltf-rs` staying available.

**Fuzz, CI, docs, and memory**

- R19. `gltf_import` fuzz corpus includes data URI and skinned fixtures plus at least one malformed buffer/skin seed.
- R20. CI fuzz matrices continue to include `gltf_import`; no reintroduction of Dependabot during rapid development.
- R21. `docs/formats/gltf.md`, `docs/formats/support-matrix.md`, and ADR 0027 are updated to state current data URI and skin capability honestly.
- R22. Engineering wiki memory records the node-level mesh binding decision and the glTF resource ledger work so future agents do not reintroduce mesh-level skin binding.

### Acceptance Examples

- AE1. Given a glTF JSON buffer with `uri: "data:application/octet-stream;base64,..."`, Baozi imports the mesh, the report shows nonzero `data_uri_bytes`, and `primary_asset_bytes + data_uri_bytes <= max_total_asset_bytes`.
- AE2. Given `max_data_uri_bytes` is lower than the decoded payload upper bound, import fails with `BaoziError::LimitExceeded { limit: "max_data_uri_bytes" }` before decoding into a large vector.
- AE3. Given two glTF nodes instantiate the same glTF mesh with two different skins, the Baozi scene contains node mesh bindings that point at the correct skin for each node.
- AE4. Given a skinned primitive has `JOINTS_0 = [2, 0, 0, 0]` but its skin has only two joints, validation fails instead of silently wrapping or treating `2` as a glTF node ID.
- AE5. Given a valid skinned triangle with inverse bind matrices, snapshot output shows one skin, the expected joint node IDs, one bound mesh instance, joint streams, and inverse bind matrix evidence.
- AE6. Given `DiagnosticOptions::strict = true` and a glTF animation is ignored, import fails even when diagnostic storage is capped.
- AE7. Given `DetectionOptions::max_probe_bytes` is too small to include a content signature, registry detection does not bypass that limit through a format-local read cap or full read.
- AE8. Given a malformed GLB header or invalid JSON document, import returns a parse error and fuzz smoke can run without panic.

### Scope Boundaries

- This plan does not implement animation import, morph target import, image decoding, full glTF conformance, Draco/KTX2 extensions, or a fork of `gltf-rs`.
- This plan may add a small pure-Rust base64 dependency to `baozi-format-gltf`, but it must remain feature-gated behind `format-gltf` and work on WASM.
- This plan does not stabilize `FormatImporter` as a third-party plugin API.
- This plan does not change the internal coordinate standard: imported glTF remains Y-up, right-handed, meters, with negative-Z front metadata.
- This plan does not re-enable Dependabot.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **Node-level mesh binding is the IR contract.** `Mesh.skin` is structurally wrong for glTF and should be removed or made non-authoritative now. The durable shape is `Node.mesh_bindings: Vec<MeshBinding>`.
- KTD2. **Meshes stay geometry/material streams.** Skin is an instance binding. Joint streams can remain on `Mesh`, but validation must evaluate them through the bindings that reference that mesh.
- KTD3. **glTF parser uses two-phase scene assembly.** Build a glTF node-index to Baozi `NodeId` map first, create skins from that map, then attach mesh bindings to nodes. This avoids cloning geometry just to express per-node skin.
- KTD4. **Out-of-scene joint nodes are not ignored silently.** If a skin references a joint/skeleton node not imported into the selected scene, the importer must either import that node under the Baozi root with a diagnostic or fail with a parse error. MVP should choose the smallest correct implementation and document it.
- KTD5. **Data URI bytes are ledger-owned.** Format code may decode, but budget checks and stats recording live on `ImportContext`; parser code should not update stats directly.
- KTD6. **Data URI MVP supports base64 buffers.** Non-base64 data URIs may return a named unsupported/parse error until percent-decoding is intentionally implemented.
- KTD7. **`gltf-rs` remains a private backend.** Tests assert Baozi behavior and owned IR, not `gltf-rs` type names or quirks.
- KTD8. **Strict diagnostics uses the existing strict-violation path.** New recoverable glTF warnings must go through `ctx.push_diagnostic`, not ad hoc logs.

### Dependencies and Constraints

- Existing pure-Rust crates may be used when mature and small. `base64` is acceptable for base64 decoding because it is a narrow, maintained dependency and stays behind the glTF feature.
- All new parsing logic must be safe Rust; parser crates should keep `#![forbid(unsafe_code)]`.
- The repository targets Windows local development plus Linux CI; fuzz run behavior on Windows may be limited by sanitizer runtime availability, so `cargo fuzz check` is the local minimum.
- WASM support matters. Avoid `std::fs`, native codecs, threads, or OS-only assumptions in `baozi-format-gltf`.

### Risks

- RISK1. Migrating `Node.meshes` to `Node.mesh_bindings` touches core, snapshots, OBJ/STL/glTF, postprocess, examples, and tests.
  - Mitigation: do this as the first implementation slice and run narrow core/parser tests before deeper glTF work.
- RISK2. Joint stream validation depends on binding context, not only a mesh-local field.
  - Mitigation: build a `mesh -> bound skins` index in validator and require exactly one compatible bound skin for joint-index range checks unless all compatible skins share enough joint capacity.
- RISK3. glTF skins may reference nodes outside the selected scene.
  - Mitigation: make the importer create required joint nodes under root or fail loudly; do not produce invalid `Skin.joints`.
- RISK4. Base64 decoded-size estimation can be subtly wrong for padding/whitespace.
  - Mitigation: validate encoded input shape, use decoder APIs for actual decode, and keep both pre-decode upper-bound checks and post-decode ledger debit.
- RISK5. Public API churn can hide accidental compatibility shims.
  - Mitigation: remove stale fields/tests instead of maintaining duplicated `meshes` and `mesh_bindings`.

---

## Implementation Units

### U1 - Core IR Mesh Binding Refactor

**Files**

- `crates/baozi-core/src/scene.rs`
- `crates/baozi-core/src/validation.rs`
- `crates/baozi-test-support/src/snapshot.rs`
- `crates/baozi-postprocess/src/*`
- `crates/baozi-format-stl/src/*`
- `crates/baozi-format-obj/src/*`
- `crates/baozi-format-gltf/src/*`
- affected tests under `crates/*/tests`

**Work**

- Add `MeshBinding`.
- Replace `Node.meshes` with `Node.mesh_bindings`.
- Update `SceneBuilder` helpers and callers.
- Remove mesh-level `skin` from `Mesh` if feasible; otherwise mark it internal-deprecated and do not use it in validation/importers.
- Update validation to check node bindings and joint streams through bound skins.
- Update snapshots to print mesh bindings and skin/joint previews.

**Tests**

- Core validation tests for binding mesh out of range, binding skin out of range, joint streams without bound skin, joint index exceeding skin palette, inverse bind matrix mismatch, inverse bind non-finite, skeleton root out of range, joint weights non-finite.
- Existing STL/OBJ/glTF tests compile against the new binding API.

### U2 - glTF Detector and Malformed Parser Gates

**Files**

- `crates/baozi-format-gltf/src/detect.rs`
- `crates/baozi-format-gltf/tests/detect.rs`
- `crates/baozi-format-gltf/tests/static_mesh.rs`
- `crates/baozi-format-gltf/tests/common/mod.rs`
- `crates/baozi/tests/gltf_facade.rs`

**Work**

- Remove the format-local `PROBE_BYTES` cap and rely on the registry-provided bounded reader.
- Add focused malformed fixtures for invalid JSON/GLB, missing `POSITION`, unsupported primitive mode, missing external buffer, short buffer, and strict ignored animation behavior.
- Ensure tests assert structured error classes/codes where possible rather than only string containment.

**Tests**

- Registry/facade detection with constrained `DetectionOptions::max_probe_bytes`.
- Direct format detector rewind behavior.
- Malformed inputs do not panic.

### U3 - Data URI Resource Ledger

**Files**

- `crates/baozi-import/src/context.rs`
- `crates/baozi-format-gltf/Cargo.toml`
- `Cargo.toml`
- `Cargo.lock`
- `crates/baozi-format-gltf/src/parser.rs`
- `crates/baozi-format-gltf/tests/static_mesh.rs`
- `crates/baozi/tests/gltf_facade.rs`

**Work**

- Add or expose `ImportContext` APIs for bounded data URI accounting.
- Add `ImportStats::data_uri_bytes()` if not already present; keep total bytes inclusive.
- Implement base64 buffer data URI loading for glTF buffers.
- Enforce `max_string_bytes`, `max_data_uri_bytes`, and `max_total_asset_bytes`.
- Keep texture/image data URIs as referenced/ignored per current texture policy unless explicitly implemented later.

**Tests**

- Valid base64 buffer data URI imports.
- `max_data_uri_bytes` failure.
- `max_total_asset_bytes` failure with primary + data URI.
- Malformed data URI syntax.
- Invalid base64.
- Non-base64 data URI unsupported/parse error.
- Declared `buffer.byteLength` larger than decoded bytes.
- Facade report stats expose data URI bytes.

### U4 - glTF Skin MVP

**Files**

- `crates/baozi-format-gltf/src/parser.rs`
- `crates/baozi-format-gltf/tests/common/mod.rs`
- new or existing `crates/baozi-format-gltf/tests/skinning.rs`
- `crates/baozi/tests/gltf_facade.rs` if facade-level evidence is needed
- `docs/formats/gltf.md`

**Work**

- Refactor glTF scene assembly to map glTF node indices to Baozi `NodeId`.
- Import skins after node IDs exist.
- Attach mesh bindings with the skin referenced by each glTF node.
- Preserve `JOINTS_0`/`WEIGHTS_0` streams already read by primitive import.
- Import inverse bind matrices through `skin.reader(...)`.
- Remove skin-ignored diagnostics for successfully supported Skin MVP.
- Add diagnostics or errors for unsupported skin edge cases, especially missing joint nodes.

**Tests**

- Skinned triangle with skeleton root, two joints, inverse bind matrices, `JOINTS_0`, and `WEIGHTS_0`.
- Skin with absent inverse bind matrices keeps an empty matrix list.
- Joint index outside palette fails validation.
- Inverse bind matrix count mismatch fails.
- Same glTF mesh under two nodes with different skins produces two correct node-level bindings.

### U5 - Fuzz, CI, Docs, and Memory

**Files**

- `fuzz/fuzz_targets/gltf_import.rs`
- `fuzz/corpus/gltf_import/*`
- `.github/workflows/ci.yml`
- `.github/workflows/fuzz.yml`
- `docs/contributing/fuzzing.md`
- `docs/formats/gltf.md`
- `docs/formats/support-matrix.md`
- `docs/adr/0027-gltf-backend-ownership-and-replacement-policy.md`
- `docs/knowledge/engineering/*`

**Work**

- Add data URI and skin corpus seeds.
- Keep `gltf_import` in CI/scheduled fuzz matrices.
- Update format docs and support matrix capability rows.
- Update ADR 0027 gates to reflect the new data URI and Skin MVP evidence.
- Add engineering wiki memory for the node-level mesh binding and resource ledger decision.

**Tests**

- Support matrix validation still passes.
- Fuzz check for `gltf_import` passes locally or the documented Windows sanitizer/toolchain limitation is recorded.

### U6 - Verification, Review, Commit, Push

**Work**

- Run formatting, checks, nextest, clippy, docs, deny, WASM checks, and fuzz checks.
- Ask one or more read-only review subagents to review the final diff if there is enough time and context.
- Fix review findings that materially affect correctness.
- Commit logical slices with Conventional Commit messages.
- Push `main` and watch CI if network/GitHub access is available.

---

## Verification Contract

Run these gates unless a platform/tooling limitation is hit and recorded:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets --all-features`
- `cargo nextest run --workspace --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --doc --workspace --all-features`
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`
- `cargo deny check`
- `cargo check -p baozi --target wasm32-unknown-unknown --no-default-features --features format-gltf`
- `cargo +nightly-2026-05-27 fuzz check gltf_import`

If `nextest` is unavailable locally, install it or fall back to `cargo test --workspace --all-features` only after recording the reason.
If Windows sanitizer DLLs block `cargo fuzz run`, keep `cargo fuzz check` local and rely on Linux CI for sanitizer execution.

---

## Definition of Done

- `Node.mesh_bindings` is the only authoritative scene-level mesh instance binding.
- glTF buffer data URIs import through `ImportContext` and are visible in `ImportReport` stats.
- glTF Skin MVP imports skins, joint palettes, skeleton roots, inverse bind matrices, joint streams, and node-level mesh skin bindings.
- Detector and malformed parser tests lock the registry/import lifecycle boundaries.
- Fuzz corpus includes glTF data URI and skin seeds.
- Format docs, support matrix, ADR 0027, and engineering memory match the implemented behavior.
- Local verification gates pass or have an explicit, dated limitation note.
- Work is committed with Conventional Commit messages and pushed to `origin/main`.
