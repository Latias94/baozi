# baozi-operation

Runtime-neutral operation control and accounting used by Baozi's core, import, and
postprocess layers. This crate provides cancellation and progress mechanisms,
resource ledgers, capacity planning, and retained-storage accounting and evidence.

This is an internal/backend support layer. Cross-crate visibility does not give
its mechanisms a stable direct-consumer contract. Applications should start with
the public [baozi facade](https://github.com/Latias94/baozi/blob/main/crates/baozi/README.md); storage activation and proof
construction remain internal integration surfaces.

See the [API tiers](https://github.com/Latias94/baozi/blob/main/docs/adr/0006-public-api-versioning-and-crate-stability-policy.md),
[operation accounting contract](https://github.com/Latias94/baozi/blob/main/docs/adr/0047-operation-ledger-evidence-and-resource-accounting.md),
[format support matrix](https://github.com/Latias94/baozi/blob/main/docs/formats/support-matrix.md), and
[repository](https://github.com/Latias94/baozi).

Licensed under [MIT](https://github.com/Latias94/baozi/blob/main/crates/baozi-operation/LICENSE-MIT) or [Apache-2.0](https://github.com/Latias94/baozi/blob/main/crates/baozi-operation/LICENSE-APACHE), at your option.
