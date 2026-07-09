# glTF Format Support

## Summary

- Format: glTF 2.0 / GLB
- Crate: `baozi-format-gltf`
- Maturity: Experimental shell
- Default feature: not enabled by `default-formats`
- Parser backend: not implemented yet
- Supported extensions: `.gltf`, `.glb`
- Supported media types: `model/gltf+json`, `model/gltf-binary`
- Encoding: JSON plus binary buffers, or GLB container
- Sidecar policy: external buffers planned

## Current Status

`baozi-format-gltf` is a descriptor-only crate. It is present so Baozi can reserve the format crate, descriptor metadata, feature flag, and support matrix while the core IR grows the required material, texture, skinning, morph target, camera, light, and animation shapes.

The crate is marked `publish = false` until it imports representative glTF fixtures into `Scene` with validation, snapshots, malformed fixtures, resource-ledger accounting, and fuzz coverage.

## Planned Scope

- `.gltf` JSON with external buffers and data URIs subject to `ResourceLimits`.
- `.glb` binary container.
- Static geometry, materials, textures as URI/buffer references, skins, morph targets, cameras, lights, and raw animation channels.
- Strict path/data URI accounting through `ImportContext`.

## Known Non-Support

Calling the current importer returns `UnsupportedFormat`. Users should not enable `format-gltf` expecting model loading yet.
