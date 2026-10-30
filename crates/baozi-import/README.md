# baozi-import

Baozi's experimental importer extension layer: format traits and descriptors,
registration and detection, typed requests and options, import sessions, dependency
resolution, Scene authoring, and bounded diagnostic, loss, and resource evidence.

Applications should use the [baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md) for normal imports.
Format implementers use this crate's parser scopes and authoring contracts;
registry, storage, and proof mechanisms retain their internal/backend status.
Dependencies require origin-qualified resolver authority. Registering a format
does not establish Baozi maturity or authorize filesystem or network access.

See [format onboarding](https://github.com/Latias94/baozi/blob/main/docs/contributing/format-onboarding.md), the
[API tiers](https://github.com/Latias94/baozi/blob/main/docs/adr/0006-public-api-versioning-and-crate-stability-policy.md),
[format support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi-import/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi-import/LICENSE-APACHE), at your option.
