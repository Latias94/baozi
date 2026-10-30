# Baozi

Rust-native model import facade with validated immutable Scenes, borrowed typed
readers, structured errors, and explicit postprocess operations. This is the
application entry point; parser authoring uses the experimental `baozi-import`
extension API, and backend/storage mechanisms remain internal.

Baozi is an experimental parser foundation requiring the Rust standard library.
`default-formats` enables STL and OBJ/MTL; `default-features = false` selects no
built-in formats. `all-formats` additionally enables PLY and the selected glTF/GLB
profile. Individual `format-stl`, `format-obj`, `format-ply`, and `format-gltf`
features select each backend. Format declarations are `Unverified`, with no
assigned Baozi maturity; enabling a feature does not imply full format coverage.

The [glTF/GLB restrictions](https://github.com/Latias94/baozi/blob/main/docs/formats/gltf.md) apply, including selected
scene closure, limited topology and extension support, and encoded images without
pixel decoding. Optional `postprocess` provides single-primitive triangulation,
flat/smooth normals, and whole-Scene static coordinate/unit conversion; spatial
conversion of deformation, animation, cameras, and lights remains unsupported.

Use memory or custom asset IO for portable imports. `native-fs-candidate` is opt-in
and limited to `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, and
`aarch64-apple-darwin`; no target has release-registered native filesystem support.
Byte-only imports deny dependencies; sidecars require an asset IO/resolver path.

Start with the [repository quickstart](https://github.com/Latias94/baozi/blob/main/README.md#quick-start) and
[Scene reading guide](https://github.com/Latias94/baozi/blob/main/docs/model/reading-scenes.md). See the
[support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md),
[native IO boundary](https://github.com/Latias94/baozi/blob/main/crates/baozi-io/README.md),
[API tiers](https://github.com/Latias94/baozi/blob/main/docs/adr/0006-public-api-versioning-and-crate-stability-policy.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi/LICENSE-APACHE), at your option.
