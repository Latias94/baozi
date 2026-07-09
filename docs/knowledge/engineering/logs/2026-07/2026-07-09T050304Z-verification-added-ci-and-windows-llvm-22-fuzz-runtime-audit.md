---
type: "Memory Event"
title: "Verification: Added CI fuzz gate and audited Windows LLVM 22 ASan runtime"
description: "Added Linux CI for Rust checks and STL sanitizer fuzz smoke. Installed LLVM 22.1.6 under F:\\MySoftware, but Windows MSVC fuzz run still failed with ASan entry-point mismatch."
timestamp: 2026-07-09T05:03:04Z
event_kind: "Verification"
related_plan: "docs/plans/2026-07-09-002-feat-stl-importer-vertical-slice-plan.md"
---
# Event

Added `.github/workflows/ci.yml` with a Linux Rust job for fmt, check, clippy, nextest, feature
checks, WASM checks, and cargo-deny, plus a Linux nightly `cargo-fuzz` smoke job for
`stl_import`.

Installed official LLVM 22.1.6 to `F:\MySoftware\LLVM-22.1.6` because `rustc +nightly -Vv` reports
LLVM 22.1.6. The expected ASan DLL exists at
`F:\MySoftware\LLVM-22.1.6\lib\clang\22\lib\windows\clang_rt.asan_dynamic-x86_64.dll`, but
`cargo +nightly fuzz run stl_import -- -runs=256` still exits with
`STATUS_ENTRYPOINT_NOT_FOUND`.

`llvm-objdump`/`llvm-readobj` comparison found the fuzz executable imports ASan/coverage names not
exported by the installed LLVM 22.1.6 DLL, including `__asan_new`, `__asan_delete`, and several
`__sanitizer_cov_*_cleanup__dll` symbols. Treat local Windows MSVC sanitizer run failure as
toolchain evidence, not parser evidence.

# Impact

The authoritative sanitizer run for stable promotion is now Linux CI. Local Windows development can
still use:

```powershell
cargo check --manifest-path fuzz\Cargo.toml
cargo +nightly fuzz check stl_import
```

and may retry ASan with a future Rust nightly/compiler-rt pairing, but Baozi should not block local
Windows contributors on this runtime mismatch.

# Citations

- [CI workflow](../../../../../.github/workflows/ci.yml)
- [Fuzzing guide](../../../../contributing/fuzzing.md)
- [ADR 0019](../../../../adr/0019-parser-diagnostic-streaming-and-generated-code-contract.md)
- [STL importer plan](../../../../plans/2026-07-09-002-feat-stl-importer-vertical-slice-plan.md)
