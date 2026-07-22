# Parser Tool Audit Checklist

This checklist operationalizes
[ADR 0018](../adr/0018-parser-tooling-and-format-owned-parser-policy.md) and
[ADR 0019](../adr/0019-parser-diagnostic-streaming-and-generated-code-contract.md). Use it before
adding a parser helper, parser combinator, lexer, generator, binary parsing macro, XML/JSON library,
archive reader, or optional backend to any `baozi-format-*` crate.

The decision remains per format. Passing this checklist does not make a tool a public Baozi API.

## Required Summary

| Field | Answer |
| --- | --- |
| Format crate |  |
| Tool crate and version |  |
| License |  |
| MSRV |  |
| Default dependency or optional feature |  |
| Target format subset |  |
| Why hand-written parsing is insufficient |  |
| Replacement plan if the tool becomes unsuitable |  |

## Architecture Boundary

- [ ] Tool types do not appear in public Baozi APIs, re-exports, diagnostics, or scene IR.
- [ ] Tool errors are converted through crate-private synchronous evidence producers admitted through
  the selected `ParseScope`, or to bounded Baozi diagnostics/loss facts.
- [ ] `ParseScope` copies dynamic codes, detail keys, string values, and scalars into Baozi-owned
  `FormatFailureEvidence`; the producer and every backend error are destroyed before return.
- [ ] Public outcomes expose no backend source chain, concrete downcast target, callback, vtable,
  native handle, or plugin-owned destructor.
- [ ] Producer code/detail/destructor callbacks cannot reenter `ParseScope` or derived metered
  capabilities; contract tests cover direct and pre-reserved storage paths.
- [ ] Session-terminal capability-produced `ScopeAbort` values are propagated unchanged and are never
  caught or reclassified as format failures.
- [ ] Permitted optional-dependency unavailability is handled only through the resolver's explicit
  non-abort outcome after bounded diagnostic/loss evidence is recorded; parser code never simulates
  optional continuation by catching `ParseFailure`.
- [ ] Tool ASTs are private, short-lived, and converted into a format-private normalized model or
  Baozi-owned `SceneDraft` data through the Scene construction capability.
- [ ] Parser code cannot return `ValidatedScene`; the import session always performs central
  validation after receiving `SceneDraft`.
- [ ] Public behavior is documented in `docs/formats/<format>.md`, not in tool-specific terms.
- [ ] Disabling the format or optional backend removes the tool from the dependency tree.

## License, MSRV, and Supply Chain

- [ ] License is compatible with Baozi's `MIT OR Apache-2.0` downstream intent, or the exception is
  documented.
- [ ] Tool MSRV does not exceed the workspace MSRV unless an MSRV ADR accepts the raise.
- [ ] Transitive dependencies are reviewed for native FFI, build scripts, proc macros, yanked
  versions, and unusual licenses.
- [ ] `cargo tree -e features` confirms feature isolation.
- [ ] `cargo deny --locked check` policy is updated when the dependency graph changes.
- [ ] Maintainer and release cadence risk is acceptable for the feature tier.

## WASM and Runtime Assumptions

- [ ] `wasm32-unknown-unknown` bytes import compiles with the relevant feature set.
- [ ] Default parser path does not require filesystem access, threads, sockets, process globals, or
  native dynamic libraries.
- [ ] Native filesystem candidate work, if needed, remains behind `native-fs-candidate`; release
      `native-fs` stays absent until a platform completes its no-skip promotion matrix.
- [ ] The tool does not assume platform endianness, pointer layouts, or locale-specific number
  parsing.

## Resource Limits and Memory Model

- [ ] Declared counts, offsets, lengths, and nested structures are checked before allocation.
- [ ] First-party/admitted parser work and allocation use checked work charges plus live
  reservations or a reviewed conservative allowance before opaque backend allocation.
- [ ] The trust model is explicit: an extension that may ignore cooperative accounting runs behind a
  process boundary with host-enforced limits.
- [ ] Known bounded counts use preallocation instead of repeated unbounded growth.
- [ ] Full-buffer parsing is justified for the format size and target use case.
- [ ] Large text/container formats have an incremental, event, or bounded-AST plan.
- [ ] Recursive grammar or include behavior has a depth limit.
- [ ] Diagnostic flooding is capped by `max_diagnostics`.
- [ ] Pathological inputs have tests or fuzz seeds for CPU and allocation behavior.

## Typed Byte Casting

- [ ] Any `bytemuck`, `zerocopy`, or equivalent casting use is private to the parser/backend.
- [ ] Endianness is checked or explicitly converted.
- [ ] Alignment and length preconditions are tested.
- [ ] Target type has a reviewed plain-data contract.
- [ ] Fallback explicit decoding exists when casting preconditions are not met.
- [ ] Casting does not create a public ABI, serialization, or cache-file layout promise.

## Diagnostics and Error Recovery

- [ ] Errors include byte offsets or line/column locations where the input representation allows it.
- [ ] Tool-specific error trees are collapsed into stable Baozi diagnostic codes/messages.
- [ ] Strict versus permissive behavior is documented for the format.
- [ ] Recoverable errors are intentionally chosen; unrecoverable structural errors return a stable
  format-owned cause through the selected `ParseScope`.
- [ ] Malformed fixture tests cover incomplete records, invalid numbers, invalid encoding, bad
  counts, and unexpected EOF.

## Generated Parser Workflow

Use this section for `lalrpop`, `pest`, code-generated lexers, schema-generated parsers, and similar
tools.

- [ ] Grammar files are committed and reviewed.
- [ ] Generated files are either not committed, or committed with a deterministic regeneration
  command.
- [ ] CI checks that generated files are current when they are committed.
- [ ] Grammar ambiguity, conflict, and precedence decisions are documented.
- [ ] Build dependencies do not leak into runtime feature sets.
- [ ] Generated code is covered by the same malformed tests and fuzz target as hand-written code.

## Tool-Specific Pain Points

| Tool family | Review focus |
| --- | --- |
| `binrw` | Hidden allocation from count fields, seek assumptions, endian declarations, offset validation, error mapping |
| `winnow` / `nom` | Span type choice, backtracking cost, error quality, lifetime complexity, complete-buffer assumptions |
| `pest` | Pair tree allocation, grammar ambiguity, line/column mapping, generated parser workflow |
| `lalrpop` | Build-time generator workflow, conflict resolution, generated code review, error recovery strategy |
| `logos` | Lexer-only boundary, invalid token policy, source span mapping, parser state machine ownership |
| XML/JSON libraries | Entity expansion, recursion depth, numeric precision, external references, data URI byte limits |
| Archive/container libraries | Path normalization, decompression bombs, entry count limits, nested archive policy |

## Verification Gate

Minimum gate before merging a parser-tool adoption:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p <format-crate> --tests
cargo check -p baozi --target wasm32-unknown-unknown --no-default-features --features <format-feature>
cargo tree -p <format-crate> -e features
```

For parser safety-sensitive changes, also run:

```text
cargo clippy -p <format-crate> --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic
cargo fuzz check <target>
```

If `cargo-fuzz` is unavailable, the format crate must include malformed-input regression tests that
exercise the same parser entry point.

## Fuzz and Sanitizer Evidence

- [ ] Fuzz target exists under `fuzz/fuzz_targets/`.
- [ ] Useful hand-authored seed corpus entries are committed under `fuzz/corpus/<target>/`.
- [ ] `fuzz/target` and `fuzz/artifacts` are ignored.
- [ ] `cargo check --manifest-path fuzz/Cargo.toml` passes.
- [ ] `cargo +nightly fuzz check <target>` passes when nightly is installed.
- [ ] Linux sanitizer CI includes the target before stable promotion.
- [ ] Sanitizer smoke run status is recorded:
  - command attempted
  - host OS and toolchain
  - pass/fail/environment failure
  - if environment failure, the missing runtime or tool is named
- [ ] Stable-promotion evidence includes a successful sanitizer fuzz smoke run on a supported CI
  environment, preferably Linux nightly.
