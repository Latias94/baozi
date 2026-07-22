# Format Onboarding Checklist

Use this checklist before adding or promoting a `baozi-format-*` crate.

## Before Coding

- Read [ADR 0004](../adr/0004-parser-backend-and-format-coverage-policy.md).
- Read [ADR 0011](../adr/0011-format-support-tiers-and-compatibility-charter.md).
- Read [ADR 0014](../adr/0014-parser-security-unsafe-ffi-and-panic-boundary-policy.md).
- Read [ADR 0018](../adr/0018-parser-tooling-and-format-owned-parser-policy.md) and
  [ADR 0019](../adr/0019-parser-diagnostic-streaming-and-generated-code-contract.md).
- Read [ADR 0020](../adr/0020-import-lifecycle-and-report-contract.md).
- Before adding a parser helper, generator, decoder library, or optional backend, complete the
  [Parser Tool Audit Checklist](../architecture/parser-tool-audit-checklist.md).
- Read [CI Policy](ci.md) before adding parser jobs, fuzz targets, or workflow tooling.
- Copy `docs/formats/_template.md` to `docs/formats/<format>.md`.
- Decide whether the first backend is Baozi-owned, wrapped, or temporary.

## Parser Contract

- Use the selected `ParseScope` resolver for every sidecar or dependency; format crates receive no
  raw `AssetIo` or unrestricted path authority.
- Charge input-derived work and reserve live storage through `ParseScope` before allocation, or
  document and test a conservative allowance for an opaque trusted backend.
- Convert backend errors into crate-private synchronous evidence producers admitted through the
  selected `ParseScope`, or bounded diagnostics/loss facts. `ParseScope` must copy every code, detail
  key, string value, and scalar into Baozi-owned evidence and destroy the producer before returning.
- Never retain or publicly expose a backend error, source chain, concrete downcast target, callback,
  vtable, native handle, or plugin-owned destructor in an import outcome.
- Evidence-producer code, detail, and destructor callbacks must not reenter `ParseScope` or a derived
  metered-storage capability. Reentry is a terminal descriptor-contract failure.
- Propagate session-terminal capability-produced `ScopeAbort` unchanged; never catch it to continue
  or reclassify it as a format failure.
- Continue after optional dependency unavailability only through the resolver's explicit non-abort
  outcome, after it records bounded diagnostic/loss evidence under descriptor and caller policy.
- Return `SceneDraft` only. The import session owns central validation and publication of
  `ValidatedScene`.
- Do not expose backend parser types in public Baozi APIs.
- Do not silently normalize coordinates, UVs, winding, materials, normals, or tangents.
- Keep facade examples public-API-only. Examples must import from `baozi`, not from private format
  crates or internal workspace crates.
- Keep sanitizer fuzz claims truthful. A target with a third-party backend that can abort under
  `cargo fuzz run` may stay as check-only, but must not be listed as stable sanitizer evidence.
- Treat in-process parsers and callbacks as trusted extension code. Hostile plugins require process
  isolation with host-enforced time and memory limits.

## Required Evidence by Stable Promotion

- Valid fixtures for declared supported capabilities.
- Malformed fixtures for expected failure modes.
- Golden scene snapshots.
- Resource-limit tests.
- Fuzz target.
- Support matrix row.
- Dependency and license notes.
- Passing GitHub Actions CI gates for workflow lint, docs, WASM, dependency policy, and fuzz smoke.
- A facade example or README snippet showing the supported public import path.
- A Criterion baseline or an explicit note explaining why this format is excluded from import
  benchmarks.
- Oracle comparison or documented reason it is not useful yet.
- Successful sanitizer smoke run on Linux CI for stable promotion.

## Security Review

- No unchecked count-to-allocation paths.
- No unbounded recursion or includes.
- No path traversal through sidecars.
- No `unsafe` without a local safety comment and tests.
- No FFI dependency in default features.
- Windows sanitizer failures are recorded as toolchain evidence, not parser evidence.

## Facade and Benchmark Checklist

- `cargo check -p baozi --examples --all-features`
- `cargo bench -p baozi --bench import_baseline --no-run --all-features`
- If the format needs sidecars, add an in-memory capability/resolver example or test.
- If the format participates in post-processing, add a pipeline example or benchmark case that
  exercises the supported pass sequence.
