# Baozi Rust Model Importer Ecosystem Report

Date: 2026-07-09

## Executive Summary

Baozi should keep the public importer architecture, scene IR, diagnostics, validation, and postprocess
semantics Baozi-owned. Existing Rust crates are useful as references, optional backends, or test
oracles, but they should not define the facade API or the canonical scene model.

The first implementation slice should be an owned STL importer. STL is small enough to parse in
Baozi while still proving detection arbitration, binary parsing, ASCII parsing, diagnostics,
resource limits, validation, snapshots, facade wiring, and WASM byte-buffer import.

## License and Clean-Room Boundary

Assimp's behavior can be studied as a compatibility reference, but Baozi must not copy Assimp source
or test assets. A Rust-native reimplementation of public behavior can remain `MIT OR Apache-2.0` as
long as Baozi-owned code, fixtures, and documentation are independently authored and every third-party
dependency or fixture has an audited license.

## Crate Usage Guidance

| Crate | Suggested role | Reason |
| --- | --- | --- |
| `stl_io` | Reference or dev oracle only | MIT and current enough, but STL is small and owned parsing gives better diagnostics and WASM control |
| `tobj` | Future OBJ reference or optional backend | MIT, active, and useful for OBJ behavior comparison |
| `ply-rs` | Prior art only until deeper audit | MIT but old; PLY's flexible property model likely needs a Baozi-owned converter anyway |
| `gltf` | Strong future glTF parser candidate | MIT OR Apache-2.0 and mature; still normalize immediately into Baozi IR |
| `winnow` / `nom` | Future text parser infrastructure | Useful for OBJ/MTL/PLY-style parsing after the STL slice proves contracts |
| `binrw` | Future binary parser candidate | Useful, but macro-generated parsing needs security and audit review |
| `image` | Optional future image decoding | Latest metadata observed MSRV 1.88, which now fits Baozi's Rust 1.95 policy; keep optional because decoding should remain separate from core import |
| `meshopt` | Optional postprocess backend | License fits, but FFI/native risk keeps it behind an explicit feature |
| `mikktspace` | Optional tangent generation backend | Useful for glTF-style tangent basis after mesh attributes and validation are stable |

## WASM Implications

WASM support must be part of the first importer slice:

- expose bytes and memory import APIs before path-only convenience APIs
- keep filesystem helpers behind facade feature `native-fs`
- keep `baozi-io` filesystem adapters behind `fs`
- make `wasm32-unknown-unknown` compile checks target the bytes path
- keep WASI filesystem checks separate with `native-fs`
- avoid mandatory threads, native SIMD, FFI, or process-global runtimes in parser crates

## Plan Impact

The STL vertical slice should implement:

- registry detection arbitration where content confidence beats extension hints
- `ImportOptions`, diagnostics, resource limits, and import reports before parser internals spread
- `SceneBuilder::finish() -> Result<Scene>` with shared validation
- deterministic scene snapshots in `baozi-test-support`
- owned binary and ASCII STL parsers with no unsafe code and no malformed-input panics
- support docs that mark STL experimental until fuzzing, fixtures, and compatibility evidence mature
