# Baozi-maintained capability filesystem extensions

## Provenance

The full crates.io `cap-fs-ext` 4.0.3 archive was copied on 2026-11-01. All 20
original package files were verified byte-for-byte against the cached archive
before maintenance changes. No original source, build script, license or
bundled lockfile was pruned. Registry cache markers are not package contents.

- Upstream: https://github.com/bytecodealliance/cap-std
- Upstream commit: `b7acf8e8807fe3fab991884d2208b7e03d35a409`
- Upstream package path: `cap-fs-ext`
- Upstream version: `4.0.3`
- Upstream source: `registry+https://github.com/rust-lang/crates.io-index`
- Original archive SHA-256:
  `56ff379b70af8e08307a8f65e7040c7301cb4a572538ade16b4984f0da77847f`

Original copyrights, authors, `.cargo_vcs_info.json` and license choices remain
retained, including the LLVM exception. This checksum identifies the original
archive, not the maintained directory or a Baozi archive.

## Changes and ownership

- The maintained package identity is `baozi-cap-fs-ext` 0.1.0 and its repository
  is Baozi. Its library name remains `cap_fs_ext`; the README identifies this fork.
- `cap-primitives` and optional `cap-std` aliases use the maintained
  `baozi-cap-primitives` and `baozi-cap-std` packages through relative development
  paths and exact `=0.1.0` distribution requirements, with default features
  disabled. Existing `std`, `fs_utf8` and `arf_strings` features remain available.
- The Windows `windows-sys` range is narrowed to `=0.61.2`, preserving the
  already reviewed and locked binding version used by the native closure.
- Production Rust source and build behavior are otherwise unchanged. The
  candidate selects only the `std` feature for repository capture.

`Cargo.toml` is the maintained manifest; `Cargo.toml.orig` and the bundled
upstream lockfile are retained historical package material. Cargo packages the
active manifest and normalizes its path-plus-version edges for distribution.
The package remains `publish = false`. Historical `Cargo.toml.orig` and
`.cargo_vcs_info.json` are excluded from archive input because Cargo reserves
those names and generates current package metadata itself.

Baozi owns dependency updates, license/provenance preservation and revalidation
of the complete native closure. The optional package does not enter the default
runtime closure or the portable facade. Its distribution does not qualify
repository capture or register any native target.
