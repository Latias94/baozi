# baozi-gltf-json

Baozi's maintained JSON model and validator based on `gltf-json` 1.4.1. The package
name is `baozi-gltf-json`; the Rust library name remains `gltf_json`. It supports
the `baozi-gltf` backend and is not the application-facing Baozi format API.

The maintained corrections reject invalid accessor references and missing camera
projection payloads without panicking during validation. The model also retains
additional morph semantics so the Baozi adapter can reject unsupported behavior
explicitly. [BAOZI_PATCHES.md](BAOZI_PATCHES.md) records the exact upstream revision
and source changes. The `gltf-derive` dependency remains the upstream 1.4.1 release.

Applications should use the `baozi` facade with `format-gltf` and consult its
[selected glTF/GLB profile](https://github.com/Latias94/baozi/blob/main/docs/formats/gltf.md).
Backend features do not establish support for the corresponding behavior in Baozi.
See the [repository](https://github.com/Latias94/baozi) for current source and
verification scope.

Upstream source attribution is retained. The package is available under either the
MIT license in [LICENSE-MIT](LICENSE-MIT) or the Apache License 2.0 in
[LICENSE-APACHE](LICENSE-APACHE).
