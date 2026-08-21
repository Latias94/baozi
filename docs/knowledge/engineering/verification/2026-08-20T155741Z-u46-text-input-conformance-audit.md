---
type: "Verification Evidence"
title: "U4.6 text-input conformance audit passed"
description: "Repository-wide ownership audit and focused verification for the shared text-input conformance gate."
timestamp: 2026-08-20T15:57:41Z
verified_at: 2026-08-21T01:11:29Z
tags: ["baozi", "architecture", "text-input", "u4.6"]
status: "passed"
git_branch: "refactor/baozi-canonical-architecture"
verified_by: "Codex repository audit, focused nextest suites, and scoped Clippy"
related_plan: "docs/plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md"
---

# Audit Scope

The audit inspected every `baozi-format-*` production source for parser-owned UTF-8 decoding, BOM
classification, newline framing, raw-to-decoded span mapping, decoded-text copies, and line-index
state. It separately classified format detection, binary parsing, and the unregistered glTF
development candidate so those responsibilities were not forced through a line-oriented parser API.

The admitted first-party inventory is the exact `all-formats` set in `crates/baozi/Cargo.toml` and
the built-in registration set in `crates/baozi/src/lib.rs`: STL, OBJ/MTL, and PLY. glTF is a
workspace development candidate, but it has no facade feature, is not registered, and remains
`publish = false`.

# Ownership Matrix

| Surface | Shared text-input ownership | Format-owned responsibility | Result |
| --- | --- | --- | --- |
| OBJ primary | Source-bound `ConformedTextInput`, metered decoded text and line index, BOM/newline/span facts | Tokens, OBJ grammar, comment-sensitive recovery authorization, stable diagnostics | Pass |
| MTL dependency | The same conformance continuation bound to the dependency `DiagnosticSource` | MTL grammar, value/path recovery authorization, dependency-relative diagnostics | Pass |
| ASCII STL | `ConformedTextInput` after exact binary-first dispatch | STL token and facet state machine | Pass |
| PLY header and ASCII body | One source-bound `TextInputRecordCursor` with exact record ranges | Header grammar, typed properties, aggregate decoded-surface limit, record work | Pass |
| STL and PLY binary payloads | No text conformance, decoded copy, or line index | Checked binary cursors and typed endian decoding | Pass |
| glTF/GLB JSON | Not an admitted U4.6 surface | U6 borrowed JSON/GLB pre-decode admission | Deferred to U6 |

# Findings

- No admitted first-party parser retains a second UTF-8, BOM, newline, raw/decoded span, decoded-copy,
  or line-index state machine. The remaining format-local scans are tokenization, grammar, semantic
  recovery selection, or checked binary decoding.
- `TextInputRecordCursor` validates the selected format profile and complete raw range before any
  record scan. Boundary discovery is bounded and charged as decode work, CRLF lookahead remains
  transactional, and shared line starts retain raw offset, decoded offset, and replacement index so
  line and recovery locations are produced without a second source scan.
- Text diagnostics that claim a line and column now require an unconstructable
  `TextDiagnosticLocation` derived from a conformed primary-source line. The generic diagnostic API
  remains byte-range only, while dependency recovery diagnostics are emitted directly through the
  source-bound recovery tracker. There is no arbitrary diagnostic line-index cache or fallback scan.
- Record-boundary `Consumed` evidence ends at the bytes actually observed, including required CRLF
  lookahead. `Attempted` remains the ADR 0047 admission fact and may include an unused suffix of the
  final bounded work quantum; mixed text/binary assertions therefore use exact consumed work and
  zero decoded/line-index storage for the untouched binary payload rather than equating attempted
  work with semantic observation.
- OBJ's detector retains a bounded valid-prefix and complete-line heuristic because a detection probe
  may end inside a UTF-8 scalar or an unfinished OBJ record. It allocates no decoded text or line
  index, produces no parser span/recovery state, and remains detection-owned rather than a second
  formal parser.
- STL detection uses `DetectionScope::check_text_input_conformance`; PLY detection checks only its
  exact LF/CRLF magic. Neither owns parse-time text state.
- glTF's current `preflight_json_utf8`, minimal `glb_json_surface`, and heuristic detector are
  temporary development evidence. `ConformedTextInput` would allocate a complete decoded copy and
  line index while still failing to enforce JSON duplicate-key, nesting, token, object, array,
  string, number, GLB chunk, and padding limits. U6 therefore owns one borrowed, bounded JSON/GLB
  admission path with surface-relative BOM semantics and container-relative raw ranges.
- No ADR amendment was required. ADR 0035 already assigns generic text conformance to
  `baozi-import` while leaving grammar and recovery decisions format-owned; the remaining question
  was an implementation inventory classification, not a durable ownership ambiguity.

# Verification

- `cargo nextest run -j 1 --no-fail-fast -p baozi-import`: 334 passed.
- `cargo nextest run -j 1 --no-fail-fast -p baozi-format-stl`: 56 passed.
- `cargo nextest run -j 1 --no-fail-fast -p baozi-format-obj -E "all() - test(sidecar_open_asset_limit_is_terminal_after_recording_optional_loss_evidence)"`: 97 passed and the named stale assertion was the only skipped test.
- `cargo nextest run -j 1 --no-fail-fast -p baozi-format-ply`: 63 passed.
- `cargo nextest run -j 1 --no-fail-fast -p baozi --features all-formats --test stl_facade --test obj_facade --test ply_facade --test detection_facade`: 16 passed.
- `cargo nextest run -j 1 --no-fail-fast -p baozi-format-gltf --test text_preflight --test detect`: 15 passed as development-candidate evidence only.
- `cargo nextest run -j 1 --no-fail-fast -p baozi-xtask --test format_ownership`: 2 passed.
- `cargo clippy -j 1 -p baozi-import -p baozi-format-stl -p baozi-format-obj -p baozi-format-ply -p baozi-format-gltf --lib --no-deps -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

The focused OBJ command intentionally excludes the separately audited stale materials assertion
`sidecar_open_asset_limit_is_terminal_after_recording_optional_loss_evidence`. Its current terminal
ledger behavior predates and is independent of U4.6; correcting that expectation remains a separate
baseline-cleanup change.

# Result

Passed. U4.6 is complete: every admitted first-party text parser uses import-owned conformance, and
shared text locations, recovery evidence, record boundaries, and storage accounting no longer depend
on a format-local or diagnostic fallback scanner. The only remaining whole-document UTF-8 preflight
is correctly gated with the unregistered glTF candidate under U6.

# Evidence

- [Canonical architecture implementation plan](../../../plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md)
- [glTF development-candidate boundary](../../../formats/gltf.md)
- [Text decoding conformance and recovery](../../../adr/0035-text-decoding-conformance-and-recovery.md)
- [Operation ledger evidence and resource accounting](../../../adr/0047-operation-ledger-evidence-and-resource-accounting.md)
