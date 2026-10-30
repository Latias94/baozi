# glTF Backend Notes

## Purpose

`baozi-format-gltf` uses `gltf-rs` 1.4.x as a private bootstrap dependency.
This note records backend hazards that affect Baozi's parser ownership, fuzzing, diagnostics, resource limits, or future fork work.
It is not a public API contract; ADR 0027 remains the architectural policy.

## Selected Profile Admission (2026-10-29)

The selected profile is source-admitted through `baozi/format-gltf` after integration `2dc2fc8`.
The normalized model, selected raw source mapping, generated/default lineage, exact limits, and
independent complex debug/release consumers pass on ordinary Windows stacks. A fresh immutable
archive passed six malformed JSON/GLB reproducers and a bounded 256-run Linux ASan campaign.
The archive SHA-256 is `558ff81b88cd38d8da78a609368dcfa19dff970f4f53e05453cd1bee72af6e39`.
The [profile document](../formats/gltf.md) defines supported semantics and explicit rejections;
the [progress record](../reviews/2026-10-29-delivery-progress.md) records execution scope.
Earlier quarantine and proposed correction sections below remain historical investigation evidence.
They do not describe current facade registration. Publication and Beta/Stable maturity remain
separate from source admission.

## Current Backend Boundary

- `gltf-rs` types are contained inside `baozi-format-gltf`.
- Public Baozi APIs expose owned `Scene`, `ImportReport`, diagnostics, and options only.
- `ParseScope` owns the metered primary snapshot, the origin-qualified external buffer/image graph,
  buffer data URI accounting, source-relative paths, and resource limits. GLB admission exposes
  borrowed JSON and BIN views over that snapshot.
- The backend receives only the JSON view and returns a private `gltf::Document`. It never receives
  the complete GLB container and cannot populate `gltf::Gltf::blob`; selected BIN-backed buffers are
  copied later through Baozi's metered buffer loader.
- Baozi deserializes `gltf::json::Root`, validates the global extension declarations under metered
  work, and only then calls `gltf::Document::from_json`. The selected `KHR_texture_transform`
  textureInfo payload is compiled into Baozi-owned binding semantics; unknown extension payloads
  remain outside the canonical IR.
- Selected external buffer snapshots stay alive through canonical authoring and are borrowed at the
  declared buffer prefix. The canonical profile records Buffer/Image source locators and attaches
  payload consumers to their dependency provenance, while the successful input manifest remains
  the authority for the complete acquired byte length, digest, relation role, and registered
  resolver stability. Canonical dependency resources remain session-opaque until import and core
  expose a sealed stable-evidence bridge.
- Baozi wraps many `gltf-rs` calls in `safe_gltf`, which uses `catch_unwind` in normal Rust builds to turn backend panics into `gltf.parser_panicked` format failures.

## E Backend Decision: Narrow Maintained Backport

**Investigation date:** 2026-10-29. **Decision:** retain the private `gltf` 1.4.1 reader API and
maintain a narrow `gltf-json` 1.4.1 backport, correcting the POSITION validation hook and camera
projection discriminator. Do not introduce another general validator ahead of the unchanged backend.
An owned bounded reader is not selected for this increment: replacing `Document` and its accessor,
appearance, skin, and animation handles would require a coordinated translator cutover, rather than
fixing the demonstrated validator defects at their source.

**Integration status:** the maintained packages are installed under `vendor/gltf` and
`vendor/gltf-json`, named `baozi-gltf` and `baozi-gltf-json`. Cargo dependency aliases preserve the
private reader API. The reader source is unchanged; its path-plus-version dependency selects the
corrected JSON package. Distinct package identities carry this selection into the separate fuzz
workspace and packaged manifests, where workspace-only registry patches would be lost. Both
original license files and a short change inventory are retained beside each package.

The initial unmodified-backend workspace run executed the acceptance tests: the two direct backend
checks and their two importer JSON/GLB checks failed as predicted. Installation is not sanitizer
admission or completion of ADR 0027's normalized-model and source-span boundary. The subsequent
2026-10-29 profile admission is recorded above.

### Evidence and Exact Failure Paths

The local Cargo cache contains the actual resolved `gltf`, `gltf-json`, and `gltf-derive` 1.4.1
sources. `gltf-json` records release-source revision `0f1e94f5e2307de7336be6d3a65c38c4d34aea08`
in `.cargo_vcs_info.json`. No `repo-ref/gltf` source checkout is present. The available
`repo-ref/khronos-gltf-selected/` files are sample descriptions, not backend source.

The local `fuzz/artifacts/gltf_import/` directory is empty. All six checked-in glTF corpus inputs
were inspected: four exercise triangle/skin imports, and two exercise node cycles/shared children.
None contains an out-of-range POSITION reference. The historical sanitizer panic remains recorded
below, but its original crash bytes, stack, and artifact hash are unavailable here. The new fixture
is a source-derived minimal reproducer of a confirmed abort-capable path; it is not presented as a
recovered or executed historical crash artifact.

| Input | Exact 1.4.1 path and result inferred from source | Required correction |
| --- | --- | --- |
| POSITION index 0 with no accessors | `parser::read_gltf_inner` -> `safe_gltf` -> `Document::from_json` -> `Document::validate` -> derived `Root::validate`/`Mesh::validate`/`Primitive::validate`. `Index<Accessor>::validate` first reports `IndexOutOfBounds`. Derived validation continues into `primitive_validate_hook`, which then evaluates `root.accessors[pos_accessor_index.value()]` and panics. The hook runs even though an earlier field reported an error. | Backport the checked lookup in the hook; retain the already reported validation error. |
| Perspective type with only an orthographic payload, or the converse | `camera_validate_hook` accepts either payload without checking which type selects it. `Document::from_json` can therefore succeed. Baozi's selected-camera path later reaches `append_cameras` -> `Camera::projection` -> `self.json.perspective.as_ref().unwrap()` or the orthographic equivalent. | Require the payload selected by the validated camera discriminator before a `Document` can be returned. |
| Invalid animation sampler index, interpolation, or input/output accessor reference | `Animation::validate` checks the channel's animation-local sampler bound and validates the sampler array. The sampler's derived validation checks its interpolation and root accessor references. These cases already return validation errors before `Channel::sampler`, `Sampler::interpolation`, or accessor reader entry. | Preserve these checks and add focused guards against regression; do not describe this as an unfixed sampler-validation panic. |
| Invalid animation target node or path | `Animation::validate` does not validate `channel.target`. Calling `Target::node` or `Target::property` directly would reach `unwrap`. Baozi's existing `parser/animation.rs::preflight_inner` instead reads the raw target, uses checked node membership and `checked_property`, and rejects it before reader calls. | Retain these existing format-semantic checks. Do not bypass them when integrating the backport. |

`catch_unwind` cannot establish safety for a panic-aborting sanitizer build. The original investigation used source evidence; the subsequent Windows workspace run reproduced
the POSITION panic and camera acceptance defect. Linux sanitizer execution remains a separate gate.

### Upstream Correction and Backport Scope

The inspected upstream `main` head was `50d65229477fe5f785c2c90df21eb59c93ea2261`, resolved using
`git ls-remote`. Its fixed-revision mesh source contains the checked POSITION lookup and
`primitive_with_oob_position_accessor_is_rejected` regression. The associated upstream patch series
identifies [the accessor fix, `36c8a5e564862014df19b8421e4dfeaac7e2aa52`](https://github.com/gltf-rs/gltf/commit/36c8a5e564862014df19b8421e4dfeaac7e2aa52)
and [tests, `e258937efe3077db4923a37e279ef0e9000deb43`](https://github.com/gltf-rs/gltf/commit/e258937efe3077db4923a37e279ef0e9000deb43).
The inspected [fixed-revision camera source](https://github.com/gltf-rs/gltf/blob/50d65229477fe5f785c2c90df21eb59c93ea2261/gltf-json/src/camera.rs)
still has the discriminator gap, and its animation validator still omits channel target validation.
Pointing Cargo at that upstream head alone does not close the demonstrated camera path.

Backport only the mesh correction and relevant regression into the maintained 1.4.1 source:

```rust
let Some(pos_accessor) = root.accessors.get(pos_accessor_index.value()) else {
    return;
};
```

Replace the old camera hook's "both absent" test with the discriminator-specific condition:

```rust
match camera.type_ {
    Checked::Valid(Type::Orthographic) if camera.orthographic.is_none() => {
        report(&|| path().field("orthographic"), Error::Missing);
    }
    Checked::Valid(Type::Perspective) if camera.perspective.is_none() => {
        report(&|| path().field("perspective"), Error::Missing);
    }
    _ => {}
}
```

The existing derived `Checked<Type>` validation already rejects an invalid discriminator. No broader
reader rewrite, generic reference checker, or duplicate primitive/accessor validator is needed for
these two fixes. Existing checked range/stride/count arithmetic, selected-source closure, work limits,
and cancellation remain Baozi responsibilities and are not superseded by the fork.

### Integration and Acceptance

1. The maintained licensed sources and both corrections are installed. `vendor/gltf/BAOZI_PATCHES.md`
   and `vendor/gltf-json/BAOZI_PATCHES.md` record the release revision, dependency changes, and the two
   source corrections. The shared Cargo registry cache is untouched.
2. The root dependency aliases `gltf` to `baozi-gltf`; that package explicitly selects
   `baozi-gltf-json`. The separate fuzz workspace reaches this same dependency through the format
   package, without a second patch declaration. Inspect both lockfiles and resolved metadata. The
   distinct package identities also keep the corrected closure in packaged artifacts.
3. **Required parser edit for these two fixes: none.** Keep `Root::from_slice` followed by the existing
   extension declaration admission and `Document::from_json`. That call must use the corrected
   dependency and return its validation error through the existing `InvalidDocument` mapping. Do not
   replace it with `from_json_without_validation` or add a parallel safety pass. No new private backend
   module was created merely to wrap this unchanged call. Keep animation's existing raw target checks.
4. Run the focused tests below, then the existing `limits`, `json_admission`, `node_graph`,
   `resource_closure`, `static_mesh`, `skinning`, and `animation` suites. Their existing limit,
   amplification, and cancellation cases remain part of the acceptance boundary.
5. Execute each checked-in malformed fixture through `gltf_import` under Linux AddressSanitizer,
   then run a bounded corpus campaign. Record the exact dependency revision, command, completion,
   and absence of panic/abort. That original execution gate closed for the selected profile in
   the admission record above; later source changes still require fresh qualification.
   `cargo fuzz check` remains compilation evidence only.

The new test target is `crates/baozi-format-gltf/tests/backend_malformed.rs`. Its seven tests cover
direct backend rejection without a panic-catching wrapper; structured importer failure for JSON and
GLB POSITION/camera cases; valid camera discriminators; invalid animation samplers/interpolation and
accessor references; and existing raw animation target checks. Import failures must use
`gltf.invalid_document`, release live accounting, and precede dependency requests and canonical node
or animation-cell consumption. Four POSITION/camera tests failed with the unchanged
1.4.1 dependency in the first Windows workspace run. The corrected dependency must make all seven
tests pass without weakening their direct-backend or structured-error assertions.

The source-derived sanitizer inputs are normal authored fixtures under
`crates/baozi-format-gltf/tests/fixtures/backend_malformed/`, not generated crash artifacts:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `position_oob.gltf` | 86 | `15d3fef032b34b9a1acf8bee8af63acc71481a68e2e8927fc36026b7f5a9658b` |
| `perspective_without_payload.gltf` | 178 | `ac3757ecb02e50cff9eb0e1b73216377d3b24d05d5b9ae3371f51ab1bec7815e` |
| `orthographic_without_payload.gltf` | 159 | `fb8d2643c5aa8d8524e339c15e6f451631b112365f4c86300a8a88aed17c2e58` |

The POSITION fixture contains the minimal candidate JSON (plus a trailing newline):

```json
{"asset":{"version":"2.0"},"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}
```

Suggested root-owned verification commands, **not executed here**:

```text
cargo +1.95 nextest run --locked -p baozi-format-gltf --test backend_malformed
```

From `fuzz/` on Linux, after verifying its maintained dependency resolution:

```text
cargo +nightly-2026-05-27 fuzz run gltf_import --sanitizer address -- -runs=1 ../crates/baozi-format-gltf/tests/fixtures/backend_malformed/position_oob.gltf ../crates/baozi-format-gltf/tests/fixtures/backend_malformed/perspective_without_payload.gltf ../crates/baozi-format-gltf/tests/fixtures/backend_malformed/orthographic_without_payload.gltf
cargo +nightly-2026-05-27 fuzz run gltf_import --sanitizer address -- -runs=256
```

Only the new Rust test file was formatted/parsed with standalone `rustfmt +1.95 --edition 2024
--config skip_children=true`; child modules were excluded. Fixture JSON decoding, byte lengths, and
digests were checked with Python. No Cargo command, commit, or index write was performed by this
investigation.

## Known Hazards

### Validation Panic Under Cargo Fuzz

- **Historical observed failure:** before the maintained correction, the selected malformed input
  could trigger a panic in `gltf-json` 1.4.1 validation during `cargo fuzz run`.
- **Why `catch_unwind` is insufficient:** cargo-fuzz/libFuzzer builds can abort on panic before unwinding reaches Baozi's `safe_gltf` wrapper.
- **Historical CI policy:** before the profile-admission record above, `gltf_import` was a
  non-admitted internal check-only target. The current target policy is documented in
  `docs/contributing/fuzzing.md` and `docs/formats/gltf.md`.
- **Resolved for the selected profile:** Baozi installs a narrow backend correction so the
  demonstrated malformed POSITION and camera payloads return structured errors before reader entry.
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
- selected `KHR_texture_transform` payloads: standard and material-specific textureInfo locations
  are normalized into the canonical binding transform and coordinate source, with declaration,
  finite-value, and UV-set bounds checked before dependency I/O

### Remaining Preflight Gaps

- Run `gltf_import` under Linux sanitizer after the new preflight checks and inspect whether the
  original abort class is gone.
- Add more real Khronos sample smoke coverage for interleaved and skinned assets before promoting
  the target back to sanitizer-run CI.
- Decide whether morph target iterator validation should become a fatal unsupported path rather
  than a warning after the core morph IR policy is exercised by fixtures.
- Admit additional entity-level extension schemas, reference closure, source-scope rows, and extras
  policy; the global declaration gate and the first transform schema slice do not yet provide
  complete extension provenance.
- Add a sealed import/core bridge for canonical stable dependency evidence if downstream Scene-only
  workflows must distinguish registered resolver identities from conservative session-opaque
  resources without consulting the successful input manifest.

## Fork Or Replacement Triggers

Fork or replace the relevant glTF backend slice if any of these remain true after Baozi-owned preflight work:

- malformed input can still abort under `cargo fuzz run gltf_import`
- required glTF 2.0 core features cannot be represented or rejected with correct diagnostics
- resource limits cannot be enforced before allocation or collection
- source context needed for diagnostics is unavailable through `gltf-rs`
- WASM, MSRV, license, or unsafe-code policy conflicts appear in the backend dependency chain

## Short-Term Policy

- Keep `gltf_import` in Linux sanitizer smoke and scheduled fuzz CI.
- Keep malformed glTF regression tests in normal Rust test suites where `safe_gltf` can catch unwindable panics.
- The bounded `2dc2fc8` campaign closes the selected-profile baseline quarantine; keep sustained
  campaign evidence separate from source admission.
- Preserve the narrow maintained corrections; do not grow another validator around the backend.

## References

- ADR 0027: `docs/adr/0027-gltf-backend-ownership-and-replacement-policy.md`
- glTF format docs: `docs/formats/gltf.md`
- Fuzz policy: `docs/contributing/fuzzing.md`
- Current parser entry point: `crates/baozi-format-gltf/src/parser.rs`
