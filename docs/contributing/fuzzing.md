# Fuzzing

Baozi parser and contract fuzzing uses `cargo-fuzz` and libFuzzer. Parser targets exercise
production import paths, central Scene validation, and bounded semantic snapshot capture, while the
generic registry target covers format detection independently. The
canonical sanitizer smoke run is the Linux CI job in `.github/workflows/ci.yml`
because the Rust nightly, compiler-rt, and sanitizer runtime are pinned and run
in a predictable Linux environment there.

Parser fuzz targets compile with `cargo fuzz check` and run a short sanitizer smoke in CI.

Longer fuzz campaigns run from `.github/workflows/fuzz.yml` on a schedule and
through manual dispatch. That workflow is Linux-only, has a bounded job timeout,
and retains both `fuzz/artifacts/<target>/**` logs and the resulting `fuzz/corpus/<target>/**`
after successful or failed runs. Downloaded campaign corpora can seed a later bounded run; a
retained artifact records progress without declaring a format admitted or a fuzz campaign complete.

Local fuzzing is still useful for quick parser work, but local platform failures
must be recorded as toolchain evidence rather than parser evidence.

## Common Commands

```powershell
cargo check --manifest-path fuzz\Cargo.toml
cargo +nightly-2026-10-28 fuzz check stl_import
cargo +nightly-2026-10-28 fuzz run --sanitizer address stl_import -- -runs=256
cargo +nightly-2026-10-28 fuzz check obj_import
cargo +nightly-2026-10-28 fuzz run --sanitizer address obj_import -- -runs=256
cargo +nightly-2026-10-28 fuzz check ply_import
cargo +nightly-2026-10-28 fuzz run --sanitizer address ply_import -- -runs=256
cargo +nightly-2026-10-28 fuzz check registry_detection
cargo +nightly-2026-10-28 fuzz run --sanitizer address registry_detection -- -runs=256 -max_len=4106
cargo +nightly-2026-10-28 fuzz check gltf_import
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
cargo +nightly-2026-05-27 fuzz run ply_import -- -runs=256
cargo +nightly-2026-05-27 fuzz run registry_detection -- -runs=256 -max_len=4106
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

Every admitted parser slice, including Experimental, needs:

- a compiling fuzz target
- committed seed corpus entries
- malformed regression tests through the same parser entry point
- a successful baseline sanitizer smoke on the authoritative Linux runner

Stable promotion additionally needs sustained fuzz/security campaign evidence for the declared
support subset. A local toolchain failure is environment evidence only and never substitutes for the
baseline Linux sanitizer gate.

`gltf_import` is included in sanitizer smoke and scheduled runs for the selected profile. Its
maintained backend rejects the demonstrated POSITION and camera defects before reader entry.
The immutable `2dc2fc8` archive passed six malformed JSON/GLB reproducers and a bounded 256-run
Linux ASan campaign; the [delivery progress](../reviews/2026-10-29-delivery-progress.md) records
the exact source/toolchain scope. This short campaign does not establish sustained fuzz coverage.
Known backend hazards and unsupported slices remain in [glTF Backend Notes](../research/gltf-rs-backend-notes.md).

## CI Tool Pinning

The fuzz workflows intentionally pin:

- `RUST_FUZZ_NIGHTLY=nightly-2026-10-28`
- `CARGO_FUZZ_VERSION=0.13.2`

Dependency automation is currently disabled, so review these pins manually when changing fuzz
infrastructure or promoting a parser support tier. Broader CI policy lives in [CI Policy](ci.md).
Cargo-fuzz 0.13.2 does not accept `--locked`. Workflows resolve the fuzz manifest with locked Cargo
metadata first and require an unchanged fuzz lockfile after execution.

## Targets

| Target | Primary crate | Coverage focus |
| --- | --- | --- |
| `stl_import` | `baozi-format-stl` | Sanitizer-run target for binary and ASCII STL facade import; every success traverses the canonical snapshot. |
| `obj_import` | `baozi-format-obj` | Sanitizer-run target for OBJ facade import plus optional MTL sidecar bytes split by the first NUL byte; every success traverses the canonical snapshot. |
| `ply_import` | `baozi-format-ply` | Sanitizer-run target for PLY ASCII and binary facade import; every success traverses the canonical snapshot. |
| `registry_detection` | `baozi-import` | Sanitizer-run contract target for bounded Automatic, Preferred, and Forced detection through compiled requests and the public import session. It checks that registration order cannot change selection or detection evidence. |
| `gltf_import` | `baozi-format-gltf` | Sanitizer-run selected glTF/GLB profile. Inputs beginning with the `glTF` magic are whole GLB bytes; other inputs use primary `.gltf` bytes plus external `buffer.bin` bytes split by the first NUL byte. Every success traverses the canonical snapshot. |
