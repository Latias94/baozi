# Fuzzing

Baozi parser fuzzing uses `cargo-fuzz` and libFuzzer. The canonical sanitizer
smoke run is the Linux CI job in `.github/workflows/ci.yml` because Rust nightly,
compiler-rt, and sanitizer runtime availability are predictable there.

Local fuzzing is still useful for quick parser work, but local platform failures
must be recorded as toolchain evidence rather than parser evidence.

## Common Commands

```powershell
cargo check --manifest-path fuzz\Cargo.toml
cargo +nightly fuzz check stl_import
cargo +nightly fuzz run stl_import -- -runs=256
```

## Windows MSVC Setup

On Windows, `cargo +nightly fuzz run` may fail before executing the target if the
AddressSanitizer runtime DLL is missing. Match the LLVM major/minor version from
the active nightly:

```powershell
rustc +nightly -Vv
```

For the 2026-05-27 nightly currently used by this repo, Rust reports LLVM
22.1.6. A matching local LLVM install can be placed outside the repo:

```powershell
$installRoot = 'F:\MySoftware\LLVM-22.1.6'
$installerPath = 'F:\MySoftware\LLVM-22.1.6-win64.exe'
$downloadUrl = 'https://github.com/llvm/llvm-project/releases/download/llvmorg-22.1.6/LLVM-22.1.6-win64.exe'

Invoke-WebRequest -Uri $downloadUrl -OutFile $installerPath
Start-Process -FilePath $installerPath -ArgumentList @('/S', "/D=$installRoot") -Wait -WindowStyle Hidden

$env:PATH = "$installRoot\bin;$installRoot\lib\clang\22\lib\windows;$env:PATH"
cargo +nightly fuzz run stl_import -- -runs=256
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
