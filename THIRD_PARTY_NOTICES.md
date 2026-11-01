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

The former gltf-rs `gltf` and `gltf-json` 1.4.1 decoder used release revision
`0f1e94f5e2307de7336be6d3a65c38c4d34aea08`, originally authored by
David Harvey-Macaulay and the gltf-rs contributors.
Upstream: <https://github.com/gltf-rs/gltf>.

That source is licensed `MIT OR Apache-2.0`; its original
[MIT license](https://github.com/gltf-rs/gltf/blob/0f1e94f5e2307de7336be6d3a65c38c4d34aea08/LICENSE-MIT) and
[Apache-2.0 license](https://github.com/gltf-rs/gltf/blob/0f1e94f5e2307de7336be6d3a65c38c4d34aea08/LICENSE-APACHE)
remain available at the exact upstream source revision. The complete vendor trees and both
maintained package identities were removed when Baozi's owned selected-profile document replaced
them. The current portable dependency graph contains no gltf-rs decoder package. Historical source
and its original notices remain in repository history; the archived qualification for that older
decoder is not qualification of its replacement.

## cap-std project

Baozi maintains three packages from the complete upstream 4.0.3 archives of the
cap-std project, revision `b7acf8e8807fe3fab991884d2208b7e03d35a409`, authored by
Dan Gohman, Jakub Konka and the cap-std contributors.
Upstream: <https://github.com/bytecodealliance/cap-std>.

| Maintained package | Upstream package | Retained source and change record |
| --- | --- | --- |
| `baozi-cap-primitives` 0.1.0 | `cap-primitives` 4.0.3 | [Source](candidates/baozi-native-io/vendor/cap-primitives), [PATCHES.md](candidates/baozi-native-io/vendor/cap-primitives/PATCHES.md) |
| `baozi-cap-std` 0.1.0 | `cap-std` 4.0.3 | [Source](candidates/baozi-native-io/vendor/cap-std), [PATCHES.md](candidates/baozi-native-io/vendor/cap-std/PATCHES.md) |
| `baozi-cap-fs-ext` 0.1.0 | `cap-fs-ext` 4.0.3 | [Source](candidates/baozi-native-io/vendor/cap-fs-ext), [PATCHES.md](candidates/baozi-native-io/vendor/cap-fs-ext/PATCHES.md) |

Each package retains the original authorship, source, build scripts, `COPYRIGHT`,
`LICENSE-MIT`, `LICENSE-APACHE`, and `LICENSE-Apache-2.0_WITH_LLVM-exception`.
The license remains `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`.
The Rust library names remain `cap_primitives`, `cap_std`, and `cap_fs_ext`.
Baozi owns maintenance of the distinct package identities and their relative
path-plus-exact-version dependency edges. The primitives package carries the
bounded-link patch and test-only packaging repairs; the upper two packages
retain their upstream production Rust source. Each change record identifies
manifest changes and the original archive checksum separately from the
maintained directory or a newly generated Baozi archive.

The copied upstream `Cargo.toml.orig`, `.cargo_vcs_info.json`, and bundled
lockfile remain historical material in the source directories. The two reserved
filenames are excluded from package inputs. Cargo generates a new archive
`Cargo.toml.orig` from the maintained active manifest and normalizes the archive
`Cargo.toml`; that generated `.orig` is not the historical upstream manifest.

The three maintained packages and `baozi-native-io-candidate` all set
`publish = false`. Portable Baozi production packages do not depend on this
native chain. Preparing or checking its archives does not publish packages or
register a native target.

## Khronos glTF Sample Assets

`tools/consumers/assets/fixtures/upstream` contains unchanged selected models and notices from
[glTF-Sample-Assets](https://github.com/KhronosGroup/glTF-Sample-Assets), commit
`8f492390ddc7a2ae221a1369ef560589cf8c70fb`. Model content uses CC-BY-4.0 or CC0-1.0 as specified in
each original per-model `LICENSE.md`; metadocumentation and the Cesium logo have their separately
retained notices. The [fixture inventory](tools/consumers/assets/fixtures/README.md) identifies the
authors, exact models, scope, and checksums. Complete applicable license texts and upstream notices
are retained under that fixture directory. These assets are independent of Baozi's source license.
