# PLY Format Support

## Summary

- Format: PLY
- Crate: `baozi-format-ply`
- Maturity: Experimental shell
- Default feature: not enabled by `default-formats`
- Parser backend: not implemented yet
- Supported extensions: `.ply`
- Supported media types: `model/ply`
- Encoding: text or binary planned
- Sidecar policy: none

## Current Status

`baozi-format-ply` is a descriptor-only crate. It is present so the workspace can define the crate boundary, feature flag, support-matrix row, and parser policy before implementation starts. It is marked `publish = false` until it imports fixtures into `Scene` with validation, snapshots, malformed tests, and fuzz coverage.

## Planned Scope

- ASCII PLY and binary little-endian PLY geometry.
- Point clouds and triangle/polygon meshes.
- Common vertex properties as typed SoA streams.
- Unknown vertex properties through `VertexAttribute`.
- Resource limits, diagnostics, and corpus fuzzing before promotion beyond shell status.

## Known Non-Support

Calling the current importer returns `UnsupportedFormat`. Users should not enable `format-ply` expecting model loading yet.
