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
The main Stable Rust job uses the `dtolnay/rust-toolchain@1.95.0` action ref.

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
cargo run --locked -p baozi-xtask -- repository-conformance-check
```

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

Browser WASM and WASI jobs are compile-only today. They prove that the byte/custom-IO feature graph
builds without enabling the native-filesystem candidate, but they do not execute an import, snapshot,
or digest golden and therefore cannot satisfy ADR 0034 Portable Byte promotion or release evidence.
U9 must select and pin executable browser-WASM and WASI runners, then run representative byte and
custom-IO facade imports plus canonical digest goldens before either target is promoted. CI also
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
package once with default features and once with all features, then invokes the required repository
checks directly; ADR review remains an authoring concern rather than a default Rust gate. The
`repository-conformance-check` command reads only `docs/conformance/contracts.toml`,
`docs/conformance/gates.toml`, and the repository-relative files explicitly named there. It does not
scan ADR prose, source prose, Git history, or the repository tree, and it does not execute the
cataloged commands. Focused retained-store and canonical-Scene commands are cataloged as behavioral
evidence while the corresponding contracts remain partial; CI executes their code through the
workspace test job rather than through a source-layout comparator.

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

Normal PR CI runs short Linux sanitizer smoke targets for `stl_import`, `obj_import`, and
`ply_import`. Non-admitted candidate targets compile in a separate check-only job; `gltf_import`
stays in that tier until the private `gltf-rs` bootstrap backend no longer aborts under fuzz panic
settings. The scheduled/manual workflow follows the same split: admitted targets run Linux-only
sanitizer campaigns, while non-admitted candidates run `cargo fuzz check` only.

Windows fuzzing is useful for local parser work, but Windows sanitizer setup failures are toolchain
evidence, not parser evidence. Linux GitHub Actions remains the promotion gate for sanitizer fuzz
smoke results.
