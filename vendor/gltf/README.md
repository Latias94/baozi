# baozi-gltf

Baozi's maintained reader backend based on `gltf-rs` 1.4.1. The package name is
`baozi-gltf`; the Rust library name remains `gltf`. It depends on the maintained
`baozi-gltf-json` package so the corrected validator is also used outside the Baozi
workspace and in packaged consumers.

Applications should use the `baozi` facade and its `format-gltf` feature. The
facade imports the explicitly selected glTF/GLB profile through Baozi's scoped IO,
resource limits, central validation, and owned Scene API. Backend feature names
do not expand that admitted format profile. See the
[selected profile](https://github.com/Latias94/baozi/blob/main/docs/formats/gltf.md)
and [repository](https://github.com/Latias94/baozi) for integration guidance.

Baozi disables this backend's default features and supplies resource bytes through
its own IO scope. The upstream optional `import` feature and examples remain backend
utilities; they do not implement Baozi's import boundary.

[BAOZI_PATCHES.md](BAOZI_PATCHES.md) records the upstream revision, maintained
dependency boundary, and source change policy. Upstream source attribution is
retained. The package is available under either the MIT license in
[LICENSE-MIT](LICENSE-MIT) or the Apache License 2.0 in
[LICENSE-APACHE](LICENSE-APACHE).
