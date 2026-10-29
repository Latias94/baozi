# Independent Scene Consumer

This standalone package depends only on `baozi`, outside workspace feature unification. Its
handwritten fixtures assert actual geometry, hierarchy, F64 precision, and checked identities.
`verify` takes the expected format set from its caller; it never infers expectations from registration.

Run from the repository root, reusing its target directory:

```text
cargo run --locked --manifest-path tools/consumers/scene/Cargo.toml --target-dir target -- obj,stl
cargo run --locked --manifest-path tools/consumers/scene/Cargo.toml --target-dir target --no-default-features --features format-ply -- ply
```

For tests, set `BAOZI_EXPECT_FORMATS` to the comma-separated expected set (an empty value means
no built-in formats). The default expectation is `obj,stl`. Feature matrix and packaged-artifact
checks reuse this source. A native run is runtime evidence for its host; a target check is only
compile evidence.
