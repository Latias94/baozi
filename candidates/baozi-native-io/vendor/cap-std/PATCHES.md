# Baozi-maintained capability standard library

## Provenance

The full crates.io `cap-std` 4.0.3 archive was copied on 2026-11-01. All 35
original package files were verified byte-for-byte against the cached archive
before maintenance changes. No original source, build script, license or
bundled lockfile was pruned. Registry cache markers are not package contents.

- Upstream: https://github.com/bytecodealliance/cap-std
- Upstream commit: `b7acf8e8807fe3fab991884d2208b7e03d35a409`
- Upstream package path: `cap-std`
- Upstream version: `4.0.3`
- Upstream source: `registry+https://github.com/rust-lang/crates.io-index`
- Original archive SHA-256:
  `c1ec78e242cfa2cfe276807ac2ecc00315a6c97786977414bcd1c3963b6c91b8`

Original copyrights, authors, `.cargo_vcs_info.json` and license choices remain
retained, including the LLVM exception. This checksum identifies the original
archive, not the maintained directory or a Baozi archive.

## Changes and ownership

- The maintained package identity is `baozi-cap-std` 0.1.0 and its repository is
  Baozi. Its library name remains `cap_std`; the README identifies this fork.
- Its `cap-primitives` dependency alias points to `baozi-cap-primitives` with a
  relative development path and exact `=0.1.0` distribution requirement, with
  default features disabled. There is no upstream same-name fallback or patch.
- Production Rust source and build behavior are unchanged. Existing optional
  `fs_utf8` and `arf_strings` features retain their upstream helper dependencies.

`Cargo.toml` is the maintained manifest; `Cargo.toml.orig` and the bundled
upstream lockfile are retained historical package material. Cargo packages the
active manifest and normalizes its path-plus-version edge for distribution.
The package remains `publish = false`. Historical `Cargo.toml.orig` and
`.cargo_vcs_info.json` are excluded from archive input because Cargo reserves
those names and generates current package metadata itself.

Baozi owns dependency updates, license/provenance preservation and revalidation
of the complete native closure. Maintaining this package does not register a
native target or qualify APIs beyond the candidate's reviewed usage.
