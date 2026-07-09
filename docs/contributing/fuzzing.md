# Fuzzing

Baozi parser fuzzing uses `cargo-fuzz` and libFuzzer. The canonical sanitizer
smoke run is the Linux CI job in `.github/workflows/ci.yml` because the Rust
nightly, compiler-rt, and sanitizer runtime are pinned and run in a predictable
Linux environment there.

Longer fuzz campaigns run from `.github/workflows/fuzz.yml` on a schedule and
through manual dispatch. That workflow is Linux-only, has a bounded job timeout,
and uploads `fuzz/artifacts/stl_import/**` only when the run fails.

Local fuzzing is still useful for quick parser work, but local platform failures
must be recorded as toolchain evidence rather than parser evidence.

## Common Commands

```powershell
cargo check --manifest-path fuzz\Cargo.toml
cargo +nightly-2026-05-27 fuzz check stl_import
cargo +nightly-2026-05-27 fuzz run stl_import -- -runs=256
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

## CI Tool Pinning

The fuzz workflows intentionally pin:

- `RUST_FUZZ_NIGHTLY=nightly-2026-05-27`
- `CARGO_FUZZ_VERSION=0.13.2`

Dependabot does not update these shell-level pins. Review them when changing
fuzz infrastructure or promoting a parser support tier. Broader CI policy lives
in [CI Policy](ci.md).
