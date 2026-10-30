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
