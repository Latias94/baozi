# Baozi glTF reader maintenance

This package contains `gltf` 1.4.1 from gltf-rs, release source revision
`0f1e94f5e2307de7336be6d3a65c38c4d34aea08`, under its original MIT OR Apache-2.0
licenses. Both license files and upstream authorship are retained.

The Rust reader source is unchanged. The package is named `baozi-gltf` and its
`gltf-json` dependency selects the maintained sibling `baozi-gltf-json`. The library
name stays `gltf` for the existing private adapter. Package source inclusion also
retains upstream tests, fixtures, examples, and these maintenance notes.
The manifest allows the newer lifetime-spelling lint for the unchanged upstream
signatures; it does not alter their inferred lifetimes.

The distinct package identities carry the corrected dependency through independent
workspaces and Cargo packaging. A workspace-only registry patch would disappear
from published dependency manifests. This package is maintained together with the
JSON package; neither is an independent Baozi format API. Baozi disables the
upstream default features and supplies all resource bytes through its own IO scope.

See [the sibling change inventory](../gltf-json/BAOZI_PATCHES.md) and
[the backend investigation](../../docs/research/gltf-rs-backend-notes.md).
