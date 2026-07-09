# Release and Publish Policy

Baozi is currently foundation-first. Breaking changes are allowed when they protect the long-term
IR, parser security model, or public API boundary.

## Published Crates

Crates intended for eventual publication:

- `baozi`
- `baozi-core`
- `baozi-io`
- `baozi-import`
- `baozi-postprocess`
- `baozi-format-stl`
- `baozi-format-obj`

Crates intentionally not published yet:

- `baozi-test-support`: test-only helpers
- `baozi-format-ply`: descriptor-only shell
- `baozi-format-gltf`: descriptor-only shell
- `baozi-fuzz`: cargo-fuzz workspace

Shell format crates must stay `publish = false` until they import representative fixtures into
`Scene` with validation, snapshots, malformed tests, resource limits, and fuzz coverage.

## Release Checklist

Before any crate publish:

- `cargo fmt --all -- --check`
- `cargo check --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace`
- `cargo test --doc --workspace --all-features`
- `cargo doc --workspace --all-features --no-deps` with `RUSTDOCFLAGS=-D warnings`
- `cargo deny check`
- fuzz smoke for public parser crates in Linux CI
- `CHANGELOG.md` updated
- support matrix and format docs updated
- security-impacting parser changes called out in release notes

## MSRV

The workspace currently uses Rust 1.95. Lowering MSRV can be considered later after format coverage
and public API shape stabilize. Until then, implementation velocity and language ergonomics take
priority.
