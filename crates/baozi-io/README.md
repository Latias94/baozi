# baozi-io

Runtime-neutral asset IO contracts, normalized logical paths, capability-bound
reads, and mutable and immutable memory-backed implementations. Applications
normally use the [baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md); custom `AssetIo` implementations
use the experimental extension contract.

Native filesystem IO is opt-in through `fs-candidate` (facade
`native-fs-candidate`). Runtime candidates are limited to
`x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, and `aarch64-apple-darwin`.
The status is `CandidateUnregistered`: no target has release-registered native
filesystem support. FreeBSD dependency auditing does not enable a runtime adapter.
`repository-fs-candidate` separately exposes experimental repository capture; it
does not promote filesystem support. Use memory or custom IO on other targets.

See the [native IO admission boundary](https://github.com/Latias94/baozi/blob/main/docs/research/native-filesystem-bound-handle-admission.md),
[asset IO policy](https://github.com/Latias94/baozi/blob/main/docs/adr/0010-asset-io-virtual-filesystem-uri-archive-and-path-security.md),
[format support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi-io/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi-io/LICENSE-APACHE), at your option.
