# Fuzzing

Baozi parser fuzzing uses `cargo-fuzz` and libFuzzer. The canonical sanitizer
smoke run is the Linux CI job in `.github/workflows/ci.yml` because the Rust
nightly, compiler-rt, and sanitizer runtime are pinned and run in a predictable
Linux environment there.

Fuzz targets have two CI tiers:

- sanitizer-run targets: compile with `cargo fuzz check` and run a short sanitizer smoke in CI
- experimental check-only targets: compile with `cargo fuzz check`, but do not run under libFuzzer
  until their parser backend is known not to abort before Baozi can report a structured error

Longer fuzz campaigns run from `.github/workflows/fuzz.yml` on a schedule and
through manual dispatch. That workflow is Linux-only, has a bounded job timeout,
and uploads `fuzz/artifacts/<target>/**` only when the run fails.

Local fuzzing is still useful for quick parser work, but local platform failures
must be recorded as toolchain evidence rather than parser evidence.

## Common Commands

```powershell
cargo check --manifest-path fuzz\Cargo.toml
cargo +nightly-2026-05-27 fuzz check stl_import
cargo +nightly-2026-05-27 fuzz run stl_import -- -runs=256
cargo +nightly-2026-05-27 fuzz check obj_import
cargo +nightly-2026-05-27 fuzz run obj_import -- -runs=256
cargo +nightly-2026-05-27 fuzz check obj_postprocess
cargo +nightly-2026-05-27 fuzz run obj_postprocess -- -runs=256
cargo +nightly-2026-05-27 fuzz check ply_import
cargo +nightly-2026-05-27 fuzz run ply_import -- -runs=256
cargo +nightly-2026-05-27 fuzz check gltf_import
```

## Windows MSVC Setup

On Windows, `cargo +nightly-2026-05-27 fuzz run` may fail before executing the target if the
AddressSanitizer runtime DLL is missing. Match the LLVM major/minor version from
the active nightly:

```powershell
rustc +nightly-2026-05-27 -Vv
```

The CI fuzz job currently pins `nightly-2026-05-27` and `cargo-fuzz` 0.13.2.
That nightly reports LLVM 22.1.6. A matching local LLVM install can be placed
outside the repo:

```powershell
$installRoot = 'F:\MySoftware\LLVM-22.1.6'
$installerPath = 'F:\MySoftware\LLVM-22.1.6-win64.exe'
$downloadUrl = 'https://github.com/llvm/llvm-project/releases/download/llvmorg-22.1.6/LLVM-22.1.6-win64.exe'

Invoke-WebRequest -Uri $downloadUrl -OutFile $installerPath
Start-Process -FilePath $installerPath -ArgumentList @('/S', "/D=$installRoot") -Wait -WindowStyle Hidden

$env:PATH = "$installRoot\bin;$installRoot\lib\clang\22\lib\windows;$env:PATH"
cargo +nightly-2026-05-27 fuzz run stl_import -- -runs=256
cargo +nightly-2026-05-27 fuzz run obj_import -- -runs=256
cargo +nightly-2026-05-27 fuzz run obj_postprocess -- -runs=256
cargo +nightly-2026-05-27 fuzz run ply_import -- -runs=256
```

Known Windows outcomes:

- `STATUS_DLL_NOT_FOUND`: the sanitizer DLL is not on `PATH`, or the installed
  LLVM/compiler-rt version does not match the active Rust nightly closely enough.
- `STATUS_ENTRYPOINT_NOT_FOUND`: the loader found a DLL, but the executable and
  runtime disagree on exported sanitizer or coverage symbols. Treat this as a
  Windows MSVC toolchain incompatibility and use the Linux sanitizer CI result as
  the authoritative gate.
- `--sanitizer none` is not a replacement for sanitizer evidence. On Windows
  MSVC it may also fail to link coverage symbols, and even when it links it does
  not exercise ASan.

## Evidence Policy

Experimental parser slices need:

- a compiling fuzz target
- committed seed corpus entries
- malformed regression tests through the same parser entry point
- a recorded local fuzz check or environment failure

Stable promotion additionally needs a successful sanitizer fuzz smoke run on a
supported Linux CI runner.

`gltf_import` is currently an experimental check-only target. It can compile and is useful for local
parser work, but malformed inputs can still reach a `gltf-rs` 1.4.x validation panic in a fuzz build
where the panic aborts before Baozi's `catch_unwind` fallback can convert it to `BaoziError`. Do not
treat a skipped `gltf_import` sanitizer run as parser evidence. The re-enable criteria and known
backend hazards live in [glTF Backend Notes](../research/gltf-rs-backend-notes.md).

## CI Tool Pinning

The fuzz workflows intentionally pin:

- `RUST_FUZZ_NIGHTLY=nightly-2026-05-27`
- `CARGO_FUZZ_VERSION=0.13.2`

Dependency automation is currently disabled, so review these pins manually when changing fuzz
infrastructure or promoting a parser support tier. Broader CI policy lives in [CI Policy](ci.md).

## Targets

| Target | Format crate | Coverage focus |
| --- | --- | --- |
| `stl_import` | `baozi-format-stl` | Sanitizer-run target for binary and ASCII STL facade import. |
| `obj_import` | `baozi-format-obj` | Sanitizer-run target for OBJ facade import plus optional MTL sidecar bytes split by the first NUL byte. |
| `obj_postprocess` | `baozi-format-obj` | Sanitizer-run target for OBJ facade import followed by triangulation and bounding-box postprocess. |
| `ply_import` | `baozi-format-ply` | Sanitizer-run target for PLY ASCII and binary facade import. |
| `gltf_import` | `baozi-format-gltf` | Experimental check-only target for glTF/GLB facade import. Inputs beginning with the `glTF` magic are treated as whole GLB bytes; other inputs use primary `.gltf` bytes plus external `buffer.bin` bytes split by the first NUL byte. |
