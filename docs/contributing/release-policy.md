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
- `baozi-format-gltf`: static mesh MVP, held back until snapshots, fuzz, and broader fixtures land
- `baozi-fuzz`: cargo-fuzz workspace

Shell format crates must stay `publish = false` until they import representative fixtures into
`Scene` with validation, snapshots, malformed tests, resource limits, and fuzz coverage.

## Publish Order

The facade crate `baozi` depends on workspace crates that do not exist on crates.io before the first
release. A direct `cargo package -p baozi` cannot pass until those dependencies have already been
published at the matching version.

First release order:

1. `baozi-core`
2. `baozi-io`
3. `baozi-import`
4. `baozi-postprocess`
5. `baozi-format-stl`
6. `baozi-format-obj`
7. `baozi`

Package leaf crates first with `cargo package -p <crate> --allow-dirty --no-verify`. Package the
facade only after the dependency versions are available from the registry. Optional format crates
with `publish = false` must not be included in release feature sets.

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
- workspace path dependencies include matching `version` fields
- security-impacting parser changes called out in release notes

## MSRV

The workspace currently uses Rust 1.95. Lowering MSRV can be considered later after format coverage
and public API shape stabilize. Until then, implementation velocity and language ergonomics take
priority.
