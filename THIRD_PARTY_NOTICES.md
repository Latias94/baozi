# Third-Party Notices

Baozi-owned clean-room code is licensed as `MIT OR Apache-2.0`. The complete terms are in
`LICENSE-MIT` and `LICENSE-APACHE`.

Add notices here when Baozi includes:

- translated or copied source from another project
- third-party fixtures
- vendored assets
- code with attribution requirements
- crates or native libraries whose license requires bundled notices

## Assimp

Baozi currently uses `repo-ref/assimp` as local reference material only. Do not add Assimp-derived source or copied Assimp test assets without preserving the applicable notice and license text here.

## gltf-rs

`vendor/gltf` and `vendor/gltf-json` contain the gltf-rs `gltf` and `gltf-json`
1.4.1 source, release revision `0f1e94f5e2307de7336be6d3a65c38c4d34aea08`,
originally authored by David Harvey-Macaulay and the gltf-rs contributors.
Upstream: <https://github.com/gltf-rs/gltf>.

The source is licensed `MIT OR Apache-2.0`. The original
[MIT](vendor/gltf/LICENSE-MIT) and [Apache-2.0](vendor/gltf/LICENSE-APACHE)
license texts are retained in each package. Baozi maintains distinct package
identities, narrow JSON validation corrections, and explicit unsupported morph-payload retention.
The complete changes are recorded
in [reader maintenance](vendor/gltf/BAOZI_PATCHES.md) and
[JSON maintenance](vendor/gltf-json/BAOZI_PATCHES.md).

## cap-primitives

`candidates/baozi-native-io/vendor/cap-primitives` retains the complete upstream
4.0.3 package from the cap-std project, revision
`b7acf8e8807fe3fab991884d2208b7e03d35a409`, authored by Dan Gohman,
Jakub Konka and the cap-std contributors.
Upstream: <https://github.com/bytecodealliance/cap-std>.

The source is licensed `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`.
The original [copyright](candidates/baozi-native-io/vendor/cap-primitives/COPYRIGHT),
[MIT](candidates/baozi-native-io/vendor/cap-primitives/LICENSE-MIT),
[Apache-2.0](candidates/baozi-native-io/vendor/cap-primitives/LICENSE-APACHE), and
[Apache-2.0 with LLVM exception](candidates/baozi-native-io/vendor/cap-primitives/LICENSE-Apache-2.0_WITH_LLVM-exception)
texts are retained. The unpublished native candidate applies a bounded-link
patch and test-only packaging repairs, documented in
[PATCHES.md](candidates/baozi-native-io/vendor/cap-primitives/PATCHES.md).
Its original archive checksum records upstream provenance, not the modified
directory's identity. Portable Baozi production packages do not depend on it.
