# Baozi glTF JSON maintenance

This package contains `gltf-json` 1.4.1 from gltf-rs, release source revision
`0f1e94f5e2307de7336be6d3a65c38c4d34aea08`, under its original MIT OR Apache-2.0
licenses. Both license files and upstream authorship are retained.

The package name is `baozi-gltf-json`; the library name remains `gltf_json`.
`gltf-derive` remains the unchanged registry release 1.4.1. The two source changes
are:

1. `src/mesh.rs`: use a checked accessor lookup in `primitive_validate_hook`.
   Derived index validation reports the invalid reference before the hook runs;
   the hook must not panic while inspecting that same reference. This follows
   [upstream correction 36c8a5e](https://github.com/gltf-rs/gltf/commit/36c8a5e564862014df19b8421e4dfeaac7e2aa52).
2. `src/camera.rs`: require the projection payload selected by the validated
   camera discriminator. Having only the other payload must not yield a validated
   document whose projection getter later panics.

Baozi's `backend_malformed` integration target exercises these paths directly,
without the adapter's panic containment, and through JSON and GLB import. Upstream
source and test files are otherwise retained. No extra reference-validation pass
or backend IO path was added. Changes to this inventory must accompany future
source changes.

The package identity and path-plus-version dependency in `baozi-gltf` ensure the
same corrections reach the workspace, separate fuzz workspace, independent
consumers, and packaged dependency manifests. The ordinary test suite does not
establish Linux sanitizer admission; the format remains quarantined until that
gate and the selected-profile consumer are executed.
