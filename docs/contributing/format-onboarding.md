# Format Onboarding Checklist

Use this checklist before adding or promoting a `baozi-format-*` crate.

## Before Coding

- Read [ADR 0004](../adr/0004-parser-backend-and-format-coverage-policy.md).
- Read [ADR 0011](../adr/0011-format-support-tiers-and-compatibility-charter.md).
- Read [ADR 0014](../adr/0014-parser-security-unsafe-ffi-and-panic-boundary-policy.md).
- Read [ADR 0018](../adr/0018-parser-tooling-and-format-owned-parser-policy.md) and
  [ADR 0019](../adr/0019-parser-diagnostic-streaming-and-generated-code-contract.md).
- Read [ADR 0046](../adr/0046-import-session-lifecycle-and-outcome-ownership.md).
- Before adding a parser helper, generator, decoder library, or optional backend, complete the
  [Parser Tool Audit Checklist](../architecture/parser-tool-audit-checklist.md).
- Read [CI Policy](ci.md) before adding parser jobs, fuzz targets, or workflow tooling.
- Copy `docs/formats/_template.md` to `docs/formats/<format>.md`.
- Decide whether the first backend is Baozi-owned, wrapped, or temporary.

## Parser Contract

- Implement `Format` with one concrete associated `Options` type. Compile bounded defaults once at
  registry freeze; use `NoFormatOptions` for formats without options. A format with serialized
  options declares its `FormatOptionSchema` and compiles the submitted layers against typed defaults
  before any import session or IO begins.
- Execute a frozen `ImportPlan` directly. The plan retains its registry and compiled invocations;
  callers do not pair a prepared request with another importer.
- Use the selected `ParseExecution` resolver for every sidecar or dependency; format crates receive no
  raw `AssetIo` or unrestricted path authority.
- Enter `ParseContext::with_execution` to borrow work, storage and control. Bind repeated work groups
  once outside their loops, admit work before each physical action, and record its actual completion
  before the next observable boundary. Reserve live storage before allocation, or document and test
  a conservative allowance for an opaque trusted backend. Poll at documented bounded frontiers and
  before IO, allocation, external callbacks and success.
- Metered containers and text/source cursors may retain session input, values and storage reservations.
  Pass execution explicitly when they grow or perform accounted work; do not retain execution, a work
  group, a permit or a mutable context inside them. Region-bound work cannot leave the closure.
- Convert backend errors into crate-private synchronous evidence producers admitted through the
  selected `ParseExecution`, or bounded diagnostics/loss facts. It must copy every code, detail
  key, string value, and scalar into Baozi-owned evidence and destroy the producer before returning.
- Never retain or publicly expose a backend error, source chain, concrete downcast target, callback,
  vtable, native handle, or plugin-owned destructor in an import outcome.
- Evidence-producer code, detail, and destructor callbacks must not reenter execution or a derived
  metered-storage capability. Reentry is a terminal descriptor-contract failure.
- Propagate session-terminal capability-produced `ScopeAbort` unchanged; never catch it to continue
  or reclassify it as a format failure.
- Continue after optional dependency unavailability only through the resolver's explicit non-abort
  outcome, after it records bounded diagnostic/loss evidence under descriptor and caller policy.
- Stage authoring values in execution regions, then call outer `ParseContext::submit` with a
  `StaticSceneBuilder`, or `submit_complete` with a complete `SceneAuthoringBatch`. Return
  `ParseResult<()>`. The session retains the candidate and owns central validation, final control
  reconciliation and publication of `ValidatedScene`. A submission followed by error or panic is
  still a failed import; catching a terminal capability failure cannot restore success.
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
