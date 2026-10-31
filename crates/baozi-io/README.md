# baozi-io

Runtime-neutral asset IO contracts, normalized logical paths, capability-bound
reads, and mutable and immutable memory-backed implementations. Applications
normally use the [baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md); custom `AssetIo` implementations
use the experimental extension contract.

Native filesystem experiments live in the separate, unpublished
[`baozi-native-io-candidate` workspace](https://github.com/Latias94/baozi/tree/main/candidates/baozi-native-io).
It implements these public IO contracts without adding native dependencies or
features to this crate. Native filesystem support remains unregistered for
release use. Use memory or custom IO on other targets.

See the [native IO admission boundary](https://github.com/Latias94/baozi/blob/main/docs/research/native-filesystem-bound-handle-admission.md),
[asset IO policy](https://github.com/Latias94/baozi/blob/main/docs/adr/0010-asset-io-virtual-filesystem-uri-archive-and-path-security.md),
[format support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi-io/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi-io/LICENSE-APACHE), at your option.
