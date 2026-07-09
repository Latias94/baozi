# Format Onboarding Checklist

Use this checklist before adding or promoting a `baozi-format-*` crate.

## Before Coding

- Read [ADR 0004](../adr/0004-parser-backend-and-format-coverage-policy.md).
- Read [ADR 0011](../adr/0011-format-support-tiers-and-compatibility-charter.md).
- Read [ADR 0014](../adr/0014-parser-security-unsafe-ffi-and-panic-boundary-policy.md).
- Read [CI Policy](ci.md) before adding parser jobs, fuzz targets, or workflow tooling.
- Copy `docs/formats/_template.md` to `docs/formats/<format>.md`.
- Decide whether the first backend is Baozi-owned, wrapped, or temporary.

## Parser Contract

- Use `AssetIo` for all sidecar and archive access.
- Respect `ResourceLimits`.
- Convert parser errors into `BaoziError` and diagnostics.
- Do not expose backend parser types in public Baozi APIs.
- Do not silently normalize coordinates, UVs, winding, materials, normals, or tangents.

## Required Evidence by Stable Promotion

- Valid fixtures for declared supported capabilities.
- Malformed fixtures for expected failure modes.
- Golden scene snapshots.
- Resource-limit tests.
- Fuzz target.
- Support matrix row.
- Dependency and license notes.
- Passing GitHub Actions CI gates for workflow lint, docs, WASM, dependency policy, and fuzz smoke.
- Oracle comparison or documented reason it is not useful yet.
- Successful sanitizer smoke run on Linux CI for stable promotion.

## Security Review

- No unchecked count-to-allocation paths.
- No unbounded recursion or includes.
- No path traversal through sidecars.
- No `unsafe` without a local safety comment and tests.
- No FFI dependency in default features.
- Windows sanitizer failures are recorded as toolchain evidence, not parser evidence.
