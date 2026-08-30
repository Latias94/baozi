# glTF Backend Notes

## Purpose

`baozi-format-gltf` uses `gltf-rs` 1.4.x as a private bootstrap dependency.
This note records backend hazards that affect Baozi's parser ownership, fuzzing, diagnostics, resource limits, or future fork work.
It is not a public API contract; ADR 0027 remains the architectural policy.

## Current Backend Boundary

- `gltf-rs` types are contained inside `baozi-format-gltf`.
- Public Baozi APIs expose owned `Scene`, `ImportReport`, diagnostics, and options only.
- `ParseScope` owns the metered primary snapshot, external buffers, buffer data URI accounting,
  source-relative paths, and resource limits. GLB admission exposes borrowed JSON and BIN views over
  that snapshot.
- The backend receives only the JSON view and returns a private `gltf::Document`. It never receives
  the complete GLB container and cannot populate `gltf::Gltf::blob`; selected BIN-backed buffers are
  copied later through Baozi's metered buffer loader.
- Baozi deserializes `gltf::json::Root`, validates the global extension declarations under metered
  work, and only then calls `gltf::Document::from_json`. No extension schema is currently admitted.
- Baozi wraps many `gltf-rs` calls in `safe_gltf`, which uses `catch_unwind` in normal Rust builds to turn backend panics into `BaoziError::Parse`.

## Known Hazards

### Validation Panic Under Cargo Fuzz

- **Observed:** `cargo +nightly-2026-05-27 fuzz run gltf_import -- -runs=256` can trigger a panic in `gltf-json` 1.4.1 validation while parsing malformed glTF input.
- **Why `catch_unwind` is insufficient:** cargo-fuzz/libFuzzer builds can abort on panic before unwinding reaches Baozi's `safe_gltf` wrapper.
- **Current CI policy:** `gltf_import` is a non-admitted internal check-only target. CI may run
  `cargo fuzz check gltf_import`, but the target is not facade, sanitizer, or maturity evidence.
  The public facade has no glTF feature while this baseline abort remains reproducible.
- **Re-enable condition:** Baozi must prevent this abort class before the backend validation call, or fork/replace the affected parser/validation slice so malformed input returns a structured error.
- **Local artifact policy:** crash artifacts under `fuzz/artifacts/gltf_import/` are gitignored. Record the hash/name in issue or verification notes when useful, but do not commit generated crash files.

### Reader API Panic Boundaries

Baozi currently treats these `gltf-rs` reader calls as untrusted backend boundaries:

- document parse: `gltf::json::Root::from_slice`, Baozi-owned global extension admission, then
  `gltf::Document::from_json`
- primitive readers: `read_positions`, `read_normals`, `read_tangents`, `read_indices`, `read_tex_coords`, `read_colors`, `read_joints`, `read_weights`
- skin reader: `read_inverse_bind_matrices`
- node traversal helpers: mesh, camera, skin, children, joint, and skeleton accessors
- morph target iteration

The long-term target is to make predictable malformed cases fail in Baozi-owned preflight before these reader calls.
`safe_gltf` remains a last-resort boundary, not the first line of parser validation.

### Closed Preflight Checks

The 2026-07-09 format beta hardening slice added Baozi-owned checks for:

- attribute counts for NORMAL, TANGENT, TEXCOORD, COLOR, JOINTS, and WEIGHTS matching POSITION count before collection
- index accessor count enforcing `max_faces` before index reader collection
- inverse bind matrix accessor type/count checks before collection
- accessor and bufferView `byteOffset`, `byteStride`, element size, alignment, declared buffer length, and `byteLength` coverage
- explicit sparse accessor rejection before reader behavior decides the outcome
- integer TEXCOORD, COLOR, and WEIGHTS normalized-value policy before conversion
- JOINTS/WEIGHTS pairing before reader calls
- core scene validation rejecting joint-stream meshes that are also referenced by unskinned mesh bindings
- GLB JSON/BIN ownership separation: only JSON enters the backend, BIN remains borrowed from the
  primary snapshot, and equivalent JSON/large-BIN imports consume identical backend `ScratchBytes`
- global extension declarations: empty and duplicate list entries are rejected, every required name
  must be present in `extensionsUsed`, unsupported required extensions return typed failure before
  backend document validation, and unsupported optional extensions emit a stable warning while the
  core fallback is retained

### Remaining Preflight Gaps

- Run `gltf_import` under Linux sanitizer after the new preflight checks and inspect whether the
  original abort class is gone.
- Add more real Khronos sample smoke coverage for interleaved and skinned assets before promoting
  the target back to sanitizer-run CI.
- Decide whether morph target iterator validation should become a fatal unsupported path rather
  than a warning after the core morph IR policy is exercised by fixtures.
- Admit entity-level extension schemas, reference closure, source-scope rows, and extras policy; the
  global declaration gate does not preserve or validate extension payloads.

## Fork Or Replacement Triggers

Fork or replace the relevant glTF backend slice if any of these remain true after Baozi-owned preflight work:

- malformed input can still abort under `cargo fuzz run gltf_import`
- required glTF 2.0 core features cannot be represented or rejected with correct diagnostics
- resource limits cannot be enforced before allocation or collection
- source context needed for diagnostics is unavailable through `gltf-rs`
- WASM, MSRV, license, or unsafe-code policy conflicts appear in the backend dependency chain

## Short-Term Policy

- Keep `gltf_import` compiling in CI with `cargo fuzz check`.
- Keep malformed glTF regression tests in normal Rust test suites where `safe_gltf` can catch unwindable panics.
- Do not claim glTF sanitizer fuzz promotion until `cargo fuzz run gltf_import` passes on Linux CI without hitting backend aborts.
- Prefer small Baozi-owned preflight validators before considering a broader fork.

## References

- ADR 0027: `docs/adr/0027-gltf-backend-ownership-and-replacement-policy.md`
- glTF format docs: `docs/formats/gltf.md`
- Fuzz policy: `docs/contributing/fuzzing.md`
- Current parser entry point: `crates/baozi-format-gltf/src/parser.rs`
