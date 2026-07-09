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
Stable Rust CI uses the `dtolnay/rust-toolchain@1.95.0` action ref instead.

## Local Checks

Run the core Rust gates before opening a PR:

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo test --doc --workspace --all-features
$env:RUSTDOCFLAGS = '-D warnings'
cargo doc --workspace --all-features --no-deps
cargo deny check
```

Run workflow lint locally when Go is available:

```powershell
go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.12
```

If Go is not installed locally, rely on the `Workflow lint` GitHub Actions job rather than using an
unpinned installer.

## Fuzz Gates

Normal PR CI runs short Linux sanitizer smoke targets for `stl_import`, `obj_import`, and
`obj_postprocess`. Experimental fuzz targets that are not yet sanitizer-run safe still compile in a
separate check-only job; `gltf_import` is in that tier until the private `gltf-rs` bootstrap backend
no longer aborts under fuzz panic settings. The scheduled/manual fuzz workflow follows the same
split: stable targets run longer Linux-only campaigns, while experimental targets run `cargo fuzz
check`.

Windows fuzzing is useful for local parser work, but Windows sanitizer setup failures are toolchain
evidence, not parser evidence. Linux GitHub Actions remains the promotion gate for sanitizer fuzz
smoke results.
