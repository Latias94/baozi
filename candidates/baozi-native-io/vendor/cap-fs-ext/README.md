# Baozi-maintained package

This is `baozi-cap-fs-ext` 0.1.0, maintained from upstream `cap-fs-ext` 4.0.3.
It uses the exact Baozi-maintained standard-library and primitives packages
without a consumer-root patch. See [PATCHES.md](PATCHES.md) for provenance and
the maintained dependency edges. The library name remains `cap_fs_ext`.
Upstream project documentation follows.

<div align="center">
  <h1><code>cap-fs-ext</code></h1>

  <p>
    <strong>Extension traits for `Dir`, `File`, etc.</strong>
  </p>

  <p>
    <a href="https://github.com/bytecodealliance/cap-std/actions?query=workflow%3ACI"><img src="https://github.com/bytecodealliance/cap-std/workflows/CI/badge.svg" alt="Github Actions CI Status" /></a>
    <a href="https://crates.io/crates/cap-fs-ext"><img src="https://img.shields.io/crates/v/cap-fs-ext.svg" alt="crates.io page" /></a>
    <a href="https://docs.rs/cap-fs-ext"><img src="https://docs.rs/cap-fs-ext/badge.svg" alt="docs.rs docs" /></a>
  </p>
</div>

The `cap-fs-ext` crate provides extension traits adding extra features
to directory types such as [`Dir`], [`File`] and related types.

[`Dir`]: https://docs.rs/cap-std/latest/cap_std/fs/struct.Dir.html
[`File`]: https://docs.rs/cap-std/latest/cap_std/fs/struct.File.html
