# Feature and Module Design Review — 2026-10-29

**Recommendation:** keep a small, additive capability surface; retain curated STL/OBJ defaults;
remove misleading switches; isolate native IO development from the public dependency graph; and
verify features through independent consumers. Use private modules for implementation responsibilities
and crates for independently selectable dependencies or shared ownership boundaries.

This is a source review of the unpublished canonical product, with breaking redesign authorized.
All 14 first-party `Cargo.toml` files, feature `cfg` sites, relevant ADRs/format/contributing docs,
the three workflows, scale commands, and feature-related xtask code were inspected. The initial review
made no code changes. The subsequently authorized bounded source patch is recorded below. No Cargo
command, build, test execution, external lookup, or commit was performed. The shared worktree is
changing; references are not a verified release revision. The root task owns the
`canonical_scene/store/api/read/` node/geometry/appearance split and resource-accounting correctness.

## Bounded Implementation Status

The later 2026-10-31 cutover implements the separately reviewed native package and core bridge
changes below. Native code/tests now live in the excluded unpublished candidate workspace, and
core uses an unconditional hidden `__scene_integration` namespace with actual constructor/privacy
and linear-ownership checks. The earlier bounded-patch description remains historical; current
integration evidence belongs in the [delivery progress](2026-10-29-delivery-progress.md).

Ready for root validation: removed facade/core `std`, unused core tracing and forwarding; moved the
facade's direct tracing dependency to dev-dependencies; declared OBJ requirements for all four examples
and both benches; extracted private `error.rs` and `importer.rs` with unchanged public type exports;
renamed built-in registry methods; preserved existing Scene-reader exports and added the requested
core appearance vocabulary. Tracing now has an independent local importer and a separate OBJ-gated
case. The native-fs checker's exact facade feature expectation and fixtures were updated for removing
`std`; native IO code was not moved.

**Explicit separate units:** native candidate extraction remains the selected design direction, but
requires a demonstrated dependency/API boundary before extraction. The hidden integration rename and
removal of `_scene-integration` are also deferred pending proof of the minimal cross-crate boundary;
do not move or expose internals wholesale. Authoring remains `baozi_import::authoring`. Scope/storage
modularization and the consumer/CI matrix are recommendations for later units, not completed work.
Cargo resolution, compilation, nextest, and executable consumer validation remain with the root task.

## Initial Review Evidence and Gaps

| Inspected surface | Evidence at initial review | Consequence |
| --- | --- | --- |
| [`baozi` features](../../crates/baozi/Cargo.toml#L13) | `default = ["std", "default-formats"]`; defaults select STL/OBJ; `all-formats` additionally selects PLY; optional formats use `dep:`. | The format dependency boundaries are useful. Keep defaults curated rather than growing automatically. |
| [`baozi-core` features](../../crates/baozi-core/Cargo.toml#L13) | Empty `std`; `mint`, `tracing`, `_scene-integration`. Facade `std` is also empty. Neither crate declares `no_std`; IO directly uses `std::io`, `Arc`, and panic contracts in [`resource.rs`](../../crates/baozi-io/src/resource.rs#L1). | Disabling defaults does not remove the standard library. [ADR 0034](../adr/0034-supported-targets-and-portability.md#L91) explicitly excludes `no_std`. Delete both `std` switches. |
| [`import`](../../crates/baozi-import/Cargo.toml#L13) / [`postprocess`](../../crates/baozi-postprocess/Cargo.toml#L13) | Each defaults to no features and enables real tracing spans, while forwarding `baozi-core/tracing`. No tracing use or tracing `cfg` was found anywhere in core source/tests. Facade source also does not use its direct tracing dependency; its tracing test does. | Preserve actual instrumentation, remove unused core tracing and forwarding, and put facade's test-only tracing dependency in dev-dependencies. Dependency activation alone does not justify this unused core switch. |
| [`baozi-io`](../../crates/baozi-io/Cargo.toml#L13) | Empty defaults; `fs-candidate` selects cap-std/rustix, `repository-fs-candidate` adds cap-fs-ext. [`lib.rs`](../../crates/baozi-io/src/lib.rs#L69) rejects non-candidate targets and reports an empty release-registered target set. | Candidate switches are real but expose development capabilities through a publish-intended supporting crate and facade `--all-features`. |
| [`baozi-xtask`](../../crates/baozi-xtask/Cargo.toml#L10) | `repository-input-candidate` activates an optional IO dependency. Source has no actual `baozi_io` consumer or corresponding executable command; references are manifest/closure checks and fixtures. | This switch currently changes the dependency graph without supplying a tool capability. Remove it rather than preserve a future collector. |
| Remaining manifests | Root has resolver `3` and no feature table. `baozi-operation`, all four `baozi-format-*` crates, `baozi-test-support`, and `fuzz` have no feature tables. `fuzz/Cargo.toml` is a separate workspace. | No reason to invent per-format `std`, runtime, or backend switches. External dependency feature selections are implementation details. |
| Public format membership | [Built-in registration](../../crates/baozi/src/importer.rs) registers **every enabled** format. [`gltf.md`](../formats/gltf.md#L19) documents glTF as non-admitted; its manifest is `publish = false`. PLY is facade-admitted but also `publish = false`. | Excluding glTF from `all-formats` is correct admission separation. PLY's packaging gap is independent of default membership. |
| CI and tooling | [CI](../../.github/workflows/ci.yml#L101) runs workspace default/all-feature tests, a no-default facade **check**, aggregate WASM checks, and native candidate suites. No independent consumer or single-format test matrix appears. [`crate_graph.rs`](../../crates/baozi-xtask/src/crate_graph.rs#L137) models dependencies, not format-feature algebra. | Existing configuration does not establish independent no-default/per-format behavior or all of ADR 0007's feature criteria. Workflow presence is not evidence of a green execution. |

[ADR 0007](../adr/0007-workspace-crate-graph-feature-flags-msrv-and-ci-gates.md#L144) already specifies
useful feature algebra and isolated tests, but explicitly describes a target contract rather than
completed implementation. [`support-matrix.md`](../formats/support-matrix.md#L9) labels every current
format declaration Unverified with no managed maturity. Default inclusion must not be advertised as
verification or platform qualification. Likewise, historical filesystem research is not proof of the
current adapter's behavior; current qualification is explicitly empty in
[ADR 0034](../adr/0034-supported-targets-and-portability.md#L76).

## Two Plausible Designs

| Design | Benefit | Cost / remaining problem |
| --- | --- | --- |
| A. Clean the existing feature graph in place | Remove `std`, unused tracing, and the dead xtask flag; retain candidate features in facade/IO. Small migration. | Public `--all-features` still selects unregistered platform code, fails on byte-only targets, and unifies runtime/repository candidate dependencies during workspace builds. Candidate naming communicates status but does not isolate it. |
| **B. Capability facade plus a separate native IO candidate package — choose this** | Public features describe usable format/integration capabilities; portable public builds have no native candidate edge. Candidate experiments and their optional repository dependency have their own graph and tests. | One implemented adapter package must move, its lockfile/baselines must be updated, and candidate path callers migrate to explicit `AssetIo` composition. |

B uses an existing seam: native IO already implements `AssetIo`, while `Importer::read_asset` accepts
that trait. It needs neither a second importer framework nor more feature aliases. The extra crate
separates platform dependencies and admission status; it is not a split to satisfy a line-count rule.
Revise the affected ADRs with the migration rather than retaining obsolete forwarding for an
unreleased API.

## Recommended Capability Contract

The intended facade feature table is:

```toml
[features]
default = ["default-formats"]
default-formats = ["format-stl", "format-obj"]
all-formats = ["format-stl", "format-obj", "format-ply"]
format-stl = ["dep:baozi-format-stl"]
format-obj = ["dep:baozi-format-obj"]
format-ply = ["dep:baozi-format-ply"]
tracing = ["baozi-import/tracing"]
mint = ["baozi-core/mint"]
```

- **Always available:** standard-library-based byte/memory/custom IO, registry composition, requests,
  diagnostics, limits, cancellation, and Scene reading. These are foundations, not optional policies.
  `default-features = false` means no built-in parsers, while preserving custom registration.
- **Format switches:** one parser dependency and one registration each. Keep STL/OBJ as the explicit
  initial convenience set; PLY remains opt-in. This is a product recommendation based on current
  wiring/examples and separate publication work, not a measured performance or maturity ranking.
  `all-formats` means every admitted format, currently STL/OBJ/PLY. Add glTF only after admission.
- **`tracing`:** optional spans with unchanged semantic results and no installed subscriber/runtime.
  Import and postprocess each retain only `tracing = ["dep:tracing"]`. Postprocess currently has no
  facade dependency or admitted transformations; retain its separate package/API and defer a facade
  feature until it exposes a real user workflow.
- **`mint`:** retain because it adds observable conversions. Document the actual current coverage:
  only `Vec3` to/from `mint::Vector3<f32>` in [`math.rs`](../../crates/baozi-core/src/math.rs#L218).
  Do not imply a complete matrix/quaternion/vector interoperability layer.
- **No placeholder switches:** remove `std` without a deprecated empty alias. Do not add `full`,
  `experimental`, parser backend selectors, serialization, async, parallel, or security-policy flags
  before they have real consumers and independent evidence. Runtime policy belongs in typed requests.

Core defaults are now `[]`, retaining the real `mint` integration. For the later visibility unit,
review whether `_scene-integration` should remain a feature:
[`baozi-import`](../../crates/baozi-import/Cargo.toml#L18) enables it unconditionally, so virtually every
import build already exposes that bridge. An unconditional `#[doc(hidden)]`
`baozi_core::__scene_integration` is a possible destination only after positive and negative API tests
demonstrate exactly which names and constructors the actual import consumer needs. Keep authoring
values at `baozi_import::authoring`. Hidden public APIs remain public Rust APIs; constructor privacy
and linear capabilities must enforce invariants. Feature resolution is not a friend-visibility or
authority mechanism. The current bounded patch deliberately retains the existing gate and names.

Keep test fault injection under `cfg(test)` and helpers in unpublished `baozi-test-support`. Do not
introduce a production `test-support` feature. A cross-crate compile-fail test should test the actual
available bridge, not pass because its entire feature is disabled. The separate visibility unit must
establish inaccessible-constructor/illegal-transition evidence before replacing absent-bridge doctests.

## Native IO Placement and Status

In the separate native IO unit, first demonstrate the narrow `AssetIo` dependency boundary and the
adapters/tests that need it, then move the implemented candidate into an unpublished standalone package at
`candidates/baozi-native-io/Cargo.toml` named `baozi-native-io-candidate`. Give it its own `[workspace]`
and lockfile, and explicitly exclude it from the root workspace. Its production edge is to
`baozi-io`; neither the facade nor import/core/postprocess depends on the candidate.

Move `crates/baozi-io/src/fs.rs`, `repository.rs`, and `repository/` there. Native opening stays one
private owner shared by the adapter and repository capture. Retain a local
`repository-capture = ["dep:cap-fs-ext"]` capability: it has actual API/dependency impact and is tested
both off and on. Move the candidate/registered status constants with the implementation. Remove
facade path helpers, candidate error variants/re-exports, and the three old candidate features rather
than publish aliases to an unpublished dependency. Candidate callers construct its IO and call
`Importer::read_asset`; any path convenience belongs beside the adapter.

Preserve the current exact runtime-candidate triples: `x86_64-unknown-linux-gnu`,
`x86_64-pc-windows-msvc`, and `aarch64-apple-darwin`, including ABI predicates. Keep explicit failures
elsewhere and separate FreeBSD dependency-audit resolution from runtime claims. Move existing
same-handle, nonblocking, race/link/reparse, and repository tests with the implementation. Preserve
reviewed native dependency closures; a module move does not constitute requalification.

Delete xtask's unused `repository-input-candidate` and optional dependency. Its checks can inspect
candidate metadata without linking the implementation. Do not turn repository capture into a parser
or facade capability. Browser WASM/WASI remain byte/custom-IO paths with compile-only CI evidence;
no first-party WASI preopen adapter or native filesystem release support is established here.

## Focused Module and Visibility Changes

Keep operation, core, import, IO, and format crates: they respectively own shared mechanisms, canonical
Scene invariants, import policy/lifecycle, IO contracts, and optional parsers. Operation is used by both
core and postprocess; merging it into import creates the wrong dependency direction. A re-export of
owned canonical authoring values is a useful adapter; forwarding an unused tracing switch is not.

| Exact area | Implementable action and boundary |
| --- | --- |
| `crates/baozi-core/src/canonical_scene/store/api/` | Build on the root task's `read/` node/geometry/appearance modules. Keep `scene.rs` responsible for the Scene owner/lifecycle and route typed read views through `read/`. Read modules borrow the owner and expose no writers, activation, or proof constructors. Keep construction/schema/validation private, with `pub(super)` or existing narrow ancestor visibility; expose only required names through `__scene_integration`. Do not create per-domain crates or Cargo switches for nodes, materials, or geometry. |
| `crates/baozi/src/lib.rs` | The bounded patch keeps the public composition surface and extracts the composite error/cause projection into private `error.rs`, with builder/workflow implementation in private `importer.rs`. The three registration statements stay together. The methods are now `with_builtin_formats`, `builder_with_builtin_formats`, and `register_builtin_formats`: they operate on every compiled-in format, independently of the Cargo default subset. |
| `crates/baozi-import/src/scope.rs` | Convert to `scope/mod.rs` owning `ParseScope`, construction, and callback state. Put work scopes in `scope/work.rs`, diagnostic/loss emission in `scope/diagnostics.rs`, and dependency authorization/acquisition in `scope/dependencies.rs`. Child implementations may access parent-private state; do not make fields public to perform the split. Move shared `read_opened_asset` and its read helpers into crate-private `acquisition.rs`, used by session and dependency acquisition. These responsibilities are currently interleaved in one scope implementation. |
| `crates/baozi-import/src/metered_storage.rs` | Merge the `StorageMeter` and `Metered*` implementations currently in `scope.rs` with their type definitions here. Keep `AccountedVec` internal and only the necessary abort/meter adapter crate-visible. This removes a real ownership split between definitions and behavior, without inventing a generic allocation framework or a new crate. |
| `crates/baozi-import/src/session.rs` and `store.rs` | Retain one lifecycle owner; keep selection/parse/finalization in session and storage/activation in store. Their size alone is not a reason for more crates, lifecycle feature flags, or public mutable context. Defer further internal movement until the root resource-accounting changes settle. |
| `crates/baozi-io/src/semantic_identity.rs` | Keep the registry entries and constructors private and its minimum cross-crate bridge hidden. Do not split proof schemes into selectable features or publish constructors to ease movement. Native extraction needs no new semantic-identity crate. |
| `crates/baozi-xtask/src/native_fs.rs` and `native_fs/` | Retain dependency/target checks for the moved candidate. Remove obsolete activation expectations and test-source/AST-shape proofs as the behavioral suites take over; do not merely relocate that complexity. Feature coverage belongs to the checked-in consumer matrix. Do not add xtask crate generation, consumer-source checking, or another format-admission database. |

Splitting a source file does not change its public API or dependencies. Extraction into a crate is
justified here only for optional parsers, shared mechanism ownership, or independently evolving native
IO. The read-module split and scope-module split must preserve existing ownership and evidence; this
review does not re-open those state machines.

## Unification Hazards and Required Evidence

Cargo features are additive across a dependency instance. `default-features = false` on one edge
cannot disable features enabled on another; resolver `3` does not eliminate ordinary dependency
unification. Format crates currently reach the same core/import instances, and import always enables
the core integration bridge. Public API visibility therefore cannot depend on its absence.

Workspace all-feature tests also select packages that an application never depends on. In particular,
xtask's current all-feature graph enables IO's repository candidate in addition to facade runtime IO.
Passing that union cannot establish either smaller closure. A lockfile entry is not proof that a
package is compiled; inspect target-filtered normal/build reachability from the consumer.

The initial [`tests/tracing.rs`](../../crates/baozi/tests/tracing.rs) required only `tracing`, but
unconditionally imported OBJ. The bounded patch uses a local importer for format-independent
instrumentation and retains a separate `tracing + format-obj` semantic-equivalence case. It does not
make tracing enable OBJ. All four facade examples and both benches exercise OBJ; the patch declares
`required-features = ["format-obj"]` for those targets. These source changes still need execution.

Use the root task's **`tools/consumers/scene/`** for both C's typed-read acceptance and H's feature/package
acceptance; do not create a parallel feature-consumer crate. At the layout rescan, `tools/` existed
without consumers and root `tests/` did not exist. No established independent-consumer convention was
found. Workspace examples and xtask's generated checker fixtures are not independent public consumers.

Exclude `tools/consumers/scene` in root `Cargo.toml`; give it its own `[workspace]`, checked-in lockfile,
and exactly one dependency, `baozi`. Declare edition/MSRV explicitly rather than inheriting from the
parent. A concrete manifest skeleton is:

```toml
[package]
name = "baozi-scene-consumer"
version = "0.0.0"
edition = "2024"
rust-version = "1.95"
publish = false

[workspace]
resolver = "3"

[dependencies]
baozi = { path = "../../../crates/baozi", default-features = false }

[features]
default = ["baozi/default"]
default-formats = ["baozi/default-formats"]
all-formats = ["baozi/all-formats"]
format-stl = ["baozi/format-stl"]
format-obj = ["baozi/format-obj"]
format-ply = ["baozi/format-ply"]
tracing = ["baozi/tracing"]
mint = ["baozi/mint"]
```

The named `default` forwarding tests actual upstream defaults; hard-coding OBJ/STL in the consumer's
default would hide upstream drift. `--no-default-features` now selects an empty facade graph, and
`--features format-ply` explicitly forwards the desired public capability. Consumer `--all-features`
unions only these named forwards: until candidate features leave the facade, it must not be reported
as testing every current facade feature. Compare the forwarding list with the public contract through
manifest metadata, not consumer-source scans.

Keep handwritten fixtures and typed Scene assertions in `src/lib.rs`; use `src/main.rs` to execute
the same workflow on the normal process stack and `tests/contracts.rs` for registry/feature cases.
Assert node/geometry/appearance values through the root task's read API, never snapshots, block counts,
or internal bridges. Pass expected descriptor IDs from the case independently of registration `cfg`.
Run semantic fixtures for that explicit expected profile; the empty case still runs registry and
unsupported-input assertions. Do not infer upstream defaults from consumer `cfg(feature = "format-obj")`:
forwarding `baozi/default` does not enable the consumer's local `format-obj` flag.
Use only standard library test support: no test-support, xtask, lower Baozi, native candidate, or mint
dependency in this consumer. Actual mint conversion tests belong in core, which already owns that
optional dependency; custom-parser/instrumentation tests belong in import's extension tier.

C runs this checked-in package against the source facade. H stages the same source files against
unpacked `.crate` artifacts, changes only the staged dependency location/resolution, and proves no
dependency points back to workspace source. Package staging is not permission to generate new Rust
test bodies. CI invokes the consumer directly; xtask neither creates it nor inspects its source.

| Independent case | Required behavior |
| --- | --- |
| No defaults, no features | `Importer::new` has no built-ins; known format input returns structured unsupported-format failure; request/IO vocabulary remains usable. Test custom parser registration separately in the import extension tier. |
| Facade defaults; explicit `default-formats` without defaults | Exactly OBJ/STL; import both; PLY absent. |
| Each of `format-stl`, `format-obj`, `format-ply` alone, no defaults | Exactly that descriptor; representative import succeeds; other parser dependencies are absent. |
| `all-formats` without defaults | Exactly OBJ/STL/PLY; import each. No glTF/native/runtime/subscriber dependency is reachable. |
| `tracing` alone; `mint` alone | Empty built-in set and usable facade; no accidental formats or filesystem edge. Import tests exercise instrumentation with a local parser; core tests exercise mint conversions. |
| Each individual format plus `tracing`; forwarded public feature union | Same canonical results with instrumentation. After graph cleanup, this union equals public `--all-features` and adds only admitted formats and real integrations. |

Run native consumer tests and representative debug/release binaries on the normal host stack. Add
WASM/WASI compile checks for empty/default/individual/all-format/public-all-feature cases; record these
as compile evidence until executable target runners exist. Keep candidate IO in its separate three-host
matrix, repository-capture off/on, and preserve its negative-target cases.

Example **future** commands, not executed during this review:

```text
cargo nextest run --locked -j 1 --manifest-path tools/consumers/scene/Cargo.toml --no-default-features --test-threads 2
cargo nextest run --locked -j 1 --manifest-path tools/consumers/scene/Cargo.toml --no-default-features --features format-ply --test-threads 2
cargo check --locked -j 1 --manifest-path tools/consumers/scene/Cargo.toml --target wasm32-unknown-unknown --no-default-features --features all-formats
```

Use serial Cargo invocations and explicitly reuse the repository's `target` for this standalone
workspace via `--target-dir` or `CARGO_TARGET_DIR`. A small cross-platform Python runner at
`scripts/check_features.py` may enumerate these fixed cases and inspect target-filtered Cargo metadata;
reuse metadata per case. CI still runs package/workspace regression suites, supplementary to this
consumer matrix. Do not dynamically generate consumers, build a general feature powerset engine, or
infer behavior by scanning Rust test bodies.

## Patch Checklist

- [x] **Bounded public graph source patch:** update
  `crates/{baozi,baozi-core,baozi-import,baozi-postprocess}/Cargo.toml`: remove `std` and core
  tracing/forwarding; move facade's tracing test dependency to dev-dependencies. Preserve format
  defaults, native candidate features, `_scene-integration`, and `dep:` format wiring.
- [x] **Facade source patch:** extract private error/importer modules, re-export the public names,
  rename built-in registry APIs, and preserve the root reader exports. Authoring stays external.
- [ ] **Separate visibility/module unit:** prove the minimum core/import bridge before renaming or
  removing its feature. Preserve root read modules; scope/storage co-location waits for root
  resource-accounting validation and must preserve behavior.
- [ ] **Candidate extraction:** create `candidates/baozi-native-io/`, move the native/repository
  implementation and tests, and update root `Cargo.toml` exclusion. Remove candidate wiring and path
  helpers from facade/IO; delete xtask's unused optional dependency/feature. Retarget native admission
  baselines in `docs/research/native-filesystem{,-repository}-resolved-closure.json` and
  `crates/baozi-xtask/src/native_fs{.rs,/contract.rs}`; update its fixtures. Preserve exact targets and
  security-sensitive closure checks while replacing obsolete source-shape expectations.
- [ ] **Executable features:** use `tools/consumers/scene/{Cargo.toml,Cargo.lock,src/lib.rs,src/main.rs,tests/contracts.rs}`
  and root workspace exclusion for C/H. Add named forwarding and the fixed matrix above, optionally
  driven by `scripts/check_features.py`. Check aggregate sets, curated subset, one optional dependency
  per format, public forwarding completeness, and forbidden dependency reachability using manifest
  metadata; assert registration and Scene semantics by executing the consumer. No dynamic xtask
  consumer generation or source checking.
- [x] **Tracing/target source patch:** decouple `crates/baozi/tests/tracing.rs` from OBJ and add
  `required-features` for `import_bytes`, `import_memory_sidecars`, `diagnostics`, `wasm_memory`,
  `import_baseline`, and `import_scale_probe`. The existing registry assertion exercises the renamed
  built-in builder API.
- [ ] **Execution evidence:** add mint conversion evidence and run the per-format facade suites,
  no-default/tracing-only cases, doctests, and locked build checks. No Cargo execution is claimed here.
- [ ] **CI:** update `.github/workflows/ci.yml` with the independent cases and moved native matrix.
  In `.github/workflows/scale.yml` and `scripts/run_scale_profile.py`, pin the measured OBJ case to
  `--no-default-features --features format-obj` instead of all features; update its duplicated command
  expectations if the root task has not already replaced that runner. Keep `.github/workflows/fuzz.yml`
  and `fuzz/Cargo.toml` consistent with admission; glTF check-only remains development evidence.
- [ ] **Docs/package truth:** amend ADRs `0006`, `0007`, `0010`, `0034`, README, format pages, and
  `docs/contributing/{ci,format-onboarding,release-policy}.md`; reconcile affected conformance paths.
  Publish exact dependencies for every admitted optional format, including PLY, before the facade, or
  explicitly remove that format from the release inventory. A disabled optional feature does not
  bypass its registry dependency requirement. Record current unverified/candidate status separately
  from the redesigned API contract.
