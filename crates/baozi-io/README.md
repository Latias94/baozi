# baozi-io

Runtime-neutral asset IO contracts, normalized logical paths, capability-bound
reads, and mutable and immutable memory-backed implementations. Applications
normally use the [baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md); custom `AssetIo` implementations
use the experimental extension contract.

Every `AssetIo` implementation must declare `open_work_bound` explicitly. It returns
a nonzero conservative billed weight calculated only from the path and stable
configuration, without IO, authority queries, or reader preparation. Its units
cover path bytes copied, byte pairs compared, comparison/control actions, and
native-resolution actions. A returned `Ok` or `Err` from `open` completes the full
weight; an unwind completes zero. This contract does not promise OS wall time,
RSS, or an exact syscall count. Return `AssetOpenWorkError::Unbounded` when a
finite bound cannot be promised, or `Overflow` when its calculation overflows.

The weight must survive namespace changes before the next open. `MemoryAssetIo`
therefore covers every possible table size without inspecting the mutable index;
`ImmutableMemoryAssetIo` uses its frozen entry count for a tighter bound. Both
include path cloning and conservative binary-search byte comparisons. Wrappers
must include their own work in addition to the delegated weight.

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
