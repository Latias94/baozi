---
type: "Memory Event"
title: "Verification: Attempted Windows fuzz run without sanitizer after ASan DLL failure"
description: "Attempted cargo +nightly fuzz run stl_import --sanitizer none -- -runs=256 after ASan DLL failure. Windows MSVC link failed with unresolved sanitizer coverage symbols."
timestamp: 2026-07-09T04:39:28Z
event_kind: "Verification"
---
# Event

Attempted cargo +nightly fuzz run stl_import --sanitizer none -- -runs=256 after ASan DLL failure. Windows MSVC link failed with unresolved __start/__stop __sancov symbols, so local fuzz execution remains an environment/toolchain limitation. Keep cargo +nightly fuzz check and malformed tests as local evidence; use Linux nightly sanitizer CI for canonical fuzz-run evidence.

# Impact

`--sanitizer none` is not a useful local fallback for Baozi's Windows MSVC fuzz target. Keep
malformed regression tests and `cargo +nightly fuzz check` as local parser evidence, and use Linux
nightly sanitizer CI as the canonical fuzz-run gate.

# Citations

- [ADR 0019](../../../../adr/0019-parser-diagnostic-streaming-and-generated-code-contract.md)
- [STL importer plan](../../../../plans/2026-07-09-002-feat-stl-importer-vertical-slice-plan.md)
