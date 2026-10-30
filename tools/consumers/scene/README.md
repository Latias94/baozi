# Independent Scene Consumer

This standalone package depends only on `baozi`, outside workspace feature unification. Its
handwritten fixtures assert actual geometry, hierarchy, F64 precision, and checked identities.
The glTF feature checks GLB/external equivalence, PBR sharing, skins, morphs, interpolation,
and malformed-document failures against handwritten expectations.
The `postprocess` feature with OBJ imports a textured polygon, triangulates it through the facade,
checks winding, area and authored UV/normal origins, preserves existing normals by default, and
explicitly replaces them through `generate_normals`. UVs survive both operations and each report's
retained evidence remains readable after Scene teardown.
`verify` takes the expected format set from its caller; it never infers expectations from registration.

Run from the repository root, reusing its target directory:

```text
cargo run --locked --manifest-path tools/consumers/scene/Cargo.toml --target-dir target -- obj,stl
cargo run --locked --manifest-path tools/consumers/scene/Cargo.toml --target-dir target --no-default-features --features format-ply -- ply
cargo run --locked --manifest-path tools/consumers/scene/Cargo.toml --target-dir target --no-default-features --features format-gltf -- gltf
cargo run --locked --manifest-path tools/consumers/scene/Cargo.toml --target-dir target --no-default-features --features format-obj,postprocess -- obj
```

For tests, set `BAOZI_EXPECT_FORMATS` to the comma-separated expected set (an empty value means
no built-in formats). The default expectation is `obj,stl`. Feature matrix and packaged-artifact
checks reuse this source. A native run is runtime evidence for its host; a target check is only
compile evidence.
