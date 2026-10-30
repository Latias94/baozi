# baozi-core

Baozi's canonical scene model and domain vocabulary: validated immutable Scenes,
borrowed typed readers, entity IDs, geometry and attributes, appearance,
deformation and animation data, math, coordinate conventions, and validation.
Bounded snapshots support test and debug evidence.

This is the canonical-model layer. Applications normally use its types through
the [baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md). Scene construction, validation proofs, and
storage machinery are internal; the `_scene-integration` feature serves workspace
adapters. Model vocabulary does not imply that every importer supports that data.
The crate requires the Rust standard library; optional `mint` support currently
provides `Vec3` conversions.

See [reading Scenes](https://github.com/Latias94/baozi/blob/main/docs/model/reading-scenes.md), the
[API tiers](https://github.com/Latias94/baozi/blob/main/docs/adr/0006-public-api-versioning-and-crate-stability-policy.md),
[format support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi-core/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi-core/LICENSE-APACHE), at your option.
