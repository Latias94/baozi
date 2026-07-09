# Security Policy

Baozi parses untrusted asset files. Treat every importer as security-sensitive.

## Supported Versions

Security fixes apply to the current `main` branch until the project starts publishing versioned releases.

## Reporting a Vulnerability

Please report suspected vulnerabilities privately to Mingzhen Zhuang at `superfrankie621@gmail.com`.

Include:

- affected Baozi revision or crate version
- input file or minimized reproducer, if shareable
- observed behavior and expected behavior
- platform, Rust version, and enabled Cargo features

## Security Baseline

- Parser code must not use `unsafe`.
- Parser failures must return `Result::Err` or diagnostics, not panic.
- Resource limits must bound primary bytes, sidecar bytes, strings, lines, vertices, faces, meshes, diagnostics, and future archive/data URI expansion.
- Sidecar and texture references must go through `AssetIo` and `ImportContext`.
- Fuzz targets and malformed fixtures are required before a format moves beyond shell status.
