# CI Policy

Baozi uses GitHub Actions as the authoritative project gate for formatting, Rust checks,
documentation, dependency policy, WASM compileability, and parser fuzz smoke runs.

## Action Version Policy

Read-only PR CI may use maintained major action tags when all of these are true:

- the workflow has no repository secrets
- the workflow does not request write permissions
- the workflow does not use `pull_request_target`
- the workflow does not consume privileged `workflow_run` output
- the workflow does not run on self-hosted runners
- the workflow is not a release, publish, signing, or artifact attestation workflow

Any workflow that crosses one of those boundaries must re-evaluate full-length commit SHA pinning,
token permissions, and trigger safety before it lands.

Checkout steps must set `persist-credentials: false` unless a job documents why git credentials are
required after checkout.

## Manually Pinned Tools

Dependency automation is intentionally disabled while Baozi is in rapid foundational development.
GitHub Actions, Cargo manifests, and the workspace-adjacent `fuzz/` package are reviewed manually to
avoid noisy update PRs while parser/API churn is high.

Shell-level workflow pins also need manual review because they do not live in a Cargo manifest.

Review these pins when changing CI, investigating tooling failures, or promoting parser maturity:

- `ACTIONLINT_VERSION` in `.github/workflows/ci.yml`
- `GO_VERSION` in `.github/workflows/ci.yml`
- `CARGO_FUZZ_VERSION` in `.github/workflows/ci.yml` and `.github/workflows/fuzz.yml`
- `RUST_FUZZ_NIGHTLY` in `.github/workflows/ci.yml` and `.github/workflows/fuzz.yml`

The dated fuzz nightly intentionally uses `dtolnay/rust-toolchain@master` with a `toolchain` input.
The workspace job tests MSRV through `dtolnay/rust-toolchain@1.95.0`. A separate current-stable
job runs strict workspace Clippy, all-feature tests, and doctests. The Windows packaged-consumer
matrix additionally tests current stable in release mode. Both stable jobs record `rustc -Vv`,
so a rolling stable result has an exact compiler identity.

## Local Checks

Run the core Rust gates before opening a PR:

```powershell
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo nextest run --locked --workspace --exclude baozi-xtask
cargo nextest run --locked --workspace --all-features --exclude baozi-xtask
cargo test --locked --doc --workspace --all-features
$env:RUSTDOCFLAGS = '-D warnings'
cargo doc --locked --workspace --all-features --no-deps
cargo deny --locked check
cargo deny --locked --manifest-path fuzz/Cargo.toml check
cargo check --locked -p baozi --target x86_64-pc-windows-msvc --no-default-features --features all-formats,native-fs-candidate
cargo nextest run --locked --target x86_64-pc-windows-msvc -p baozi-io --features fs-candidate
cargo check --locked -p baozi --target wasm32-unknown-unknown --no-default-features --features all-formats
cargo check --locked -p baozi --target wasm32-wasip1 --no-default-features --features all-formats
cargo run --locked -p baozi-xtask -- crate-graph-check
cargo run --locked -p baozi-xtask -- native-fs-check
cargo nextest run --locked -p baozi-xtask
cargo nextest run --locked -p baozi-xtask --all-features
cargo run --locked -p baozi-xtask -- runtime-admission-check
python scripts/check_features.py --toolchain 1.95.0
python scripts/check_packages.py --toolchain 1.95.0
python scripts/check_packages.py --toolchain stable --release
```

The package matrix creates fresh archives for the complete production dependency closure, checks
each package's normalized metadata, README and licenses, and resolves local dependencies only from
their extracted contents. It builds and executes all twelve fixed consumer cases in a fresh
artifact build directory: no features, curated defaults, each format independently, postprocessing,
all formats, tracing, mint, and all features. The MSRV/debug and current-stable/release Windows
jobs execute the same maintained semantic consumer, including the ordinary main-stack import and
application-buffer workflow.

There is currently no release-registered `native-fs` or low-level `fs` feature. The explicitly
non-release `native-fs-candidate`/`fs-candidate` features preserve implementation and test access
while `NATIVE_FS_REGISTERED_TARGETS` remains empty. This is a breaking migration: callers must not
replace `native-fs` with the candidate feature in production or treat successful candidate tests as
a support promise.

GitHub Actions binds each runtime candidate to one fixed runner and exact target:
`ubuntu-24.04` / `x86_64-unknown-linux-gnu`, `windows-2025` /
`x86_64-pc-windows-msvc`, and `macos-15` / `aarch64-apple-darwin`. Each job first compares the
`rustc -vV` host with its matrix target, then passes that target explicitly to every Cargo and
nextest command. Candidate fixture setup is fail-closed; the suite contains no `SKIP` path.
Unix candidate tests require an unprivileged runner so the search-permission fixture can verify
an actual host denial. Root or another DAC bypass fails that fixture rather than passing it silently.

Browser WASM remains compile-only. WASI executes the independent no-feature and all-feature release
consumers under Wasmtime 50.0.0, including byte/custom-IO imports, typed semantic value assertions,
and the application-buffer workflow where compiled in. The workflow pins the
Wasmtime setup action by commit and its runtime by version. These executed WASI cases do not
establish browser runtime support or promote any native-filesystem candidate. Portable Byte release
promotion still follows ADR 0034 and the exact executed target/profile evidence. CI also
requires the candidate feature to fail with the same explicit Baozi diagnostic on
`wasm32-unknown-unknown`, `wasm32-wasip1`, Build Monitored `x86_64-apple-darwin`, and audit-only
`x86_64-unknown-freebsd`, plus ABI-distinct `x86_64-unknown-linux-gnux32`, after first proving each
target is installed. The `native-fs-check` command separately compares normal/build dependency
closures for the three runtime candidates plus the FreeBSD audit profile against schema 3 of
[`native-filesystem-resolved-closure.json`](../research/native-filesystem-resolved-closure.json),
whose `runtime_candidate_targets`, `dependency_audit_profiles`, and
`release_registered_targets` fields keep those universes distinct. It also checks package sources,
lockfile checksums, resolved features, direct target edges, and the version-bound review basis for
the Windows directory share-mode boundary. Native filesystem
dependencies remain private to `baozi-io`. The checker also rejects `cap-std`, `cap-primitives`,
`rustix`, or `cap-fs-ext`, and rejects activation of either registered or candidate native filesystem
features in byte-only WebAssembly graphs. The separate `crate-graph-check` traverses every workspace
normal/build dependency edge, including optional, renamed, and target-conditional declarations. It
enforces the complete ADR 0007 production direction table, rejects unclassified production crates,
support/tool dependencies, reverse paths, and production cycles, while allowing dev-only test edges.
The workspace nextest runs exclude `baozi-xtask`. Its repository-scanning fixture suites duplicate
whole-tree work and can dominate otherwise parallel workspace tests. CI runs the complete xtask
package once, then invokes the required repository checks directly. The dead repository-input
feature and optional IO edge are removed, so a second all-feature run would repeat the same graph.
ADR review remains an authoring concern rather than a default Rust gate.
`runtime-admission-check` makes one locked Cargo metadata query and checks normal/build dependency
reachability. Host scheduling, cancellation, release ordering, and publication are covered by the
[behavior gate index](../testing/behavior-gates.md). The former conformance receipt and test-source
hash checks have been retired; a handwritten receipt never substituted for an executed test.

A green dependency or candidate runner gate is not filesystem-conformance evidence. The three exact
runtime candidates still require the complete platform-local race, link/reparse,
special-file, and same-handle acceptance matrix from
[`native-filesystem-bound-handle-admission.md`](../research/native-filesystem-bound-handle-admission.md).
The FreeBSD closure is resolved in CI, but it is not a runtime candidate and no hosted FreeBSD
runtime job is claimed by this workflow.
Windows unknown-reparse construction is likewise still a promotion blocker; it is not represented by
an environment-dependent passing or skipped test.

Run workflow lint locally when Go is available:

```powershell
go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.12
```

If Go is not installed locally, rely on the `Workflow lint` GitHub Actions job rather than using an
unpinned installer.

## Fuzz Gates

Normal PR CI runs short Linux AddressSanitizer smoke targets for `stl_import`, `obj_import`,
`ply_import`, `gltf_import`, and `registry_detection`. The selected glTF profile uses the maintained
backend corrections and passed its recorded baseline admission; it is no longer a check-only
candidate. The scheduled/manual workflow runs bounded Linux sanitizer campaigns for the same five
targets and retains their corpus and logs. A compile check alone does not establish sanitizer or
format admission, and a historical successful source state does not prove a later revision.

Windows fuzzing is useful for local parser work, but Windows sanitizer setup failures are toolchain
evidence, not parser evidence. Linux GitHub Actions remains the promotion gate for sanitizer fuzz
smoke results. Both PR smoke and scheduled campaigns retain successful or failed corpus/log
artifacts; their presence preserves progress without implying sustained coverage.
