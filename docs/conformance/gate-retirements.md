# Conformance Gate Retirement Records

This file records migration-only source-shape checks that were removed or are still pending
replacement by stronger proofs. It is a review record, not an executable Rust parser
specification. The repository checker must not infer private names, field order, call counts, or AST
shape from this file.

## `u4c-metrics` and `_u4c-collector`

- **Owners:** `u4-canonical-scene-lifecycle`, `u4-private-producer-consumer-proof`, and
  `u4-retained-physical-ownership`.
- **Retired checks:** the operation-level measurement hooks, core JSON production probes, frozen
  source and build-output digests, the `u4c-metrics` xtask command, and the U4c/U4d aggregation jobs.
- **Reason:** these checks qualified a migration implementation and exact source closure. ADR 0044
  superseded those U4-specific commands, hashes, fixtures, and thresholds as architectural
  authority, and the receipts had become stale under ordinary implementation refactoring.
- **Replacement proof:** retained publication, activation, accounting, rollback, poison, panic, and
  destructor fault-injection tests; canonical Scene construction, semantic, snapshot, edit, format,
  and facade behavior tests; Rust visibility and ownership checks; workspace CI; and sanitizer fuzz
  smoke. The focused conformance inventory points directly to these durable tests.
- **Conformance effect:** affected contracts remain `partial`. Removing the migration collector does
  not promote a contract or claim release readiness.
- **Status:** retired. A future performance or allocation regression gate must measure a stable
  public behavior and bounded workload, not private source shape or a recursively reconstructed
  Cargo/Rust frontend.

## `u4-retained-private-source-shape`

- **Owner:** `u4-retained-physical-ownership`
- **Retired in:** `fae8050` (`fix(operation): harden canonical activation finalization`) and the
  current U4d gate-retirement slice.
- **Retired checks:** obsolete exact occurrence checks removed in `fae8050`, plus the remaining
  final-owner extraction spelling, terminal declaration placement, curated-export spelling, and
  selected private-method scans formerly in
  `crates/baozi-operation/src/retained/tests/architecture.rs`.
- **Remaining checks outside this record:** `crates/baozi-operation/src/retained/tests.rs` and
  `crates/baozi-operation/src/retained/tests/startup_operation.rs` still contain other
  migration-only source scans. They require separate risk and replacement-proof records before
  deletion; this entry does not declare the entire repository source-shape family retired.
- **Risk previously protected:** generic retained authority could escape the deep module; terminal
  ownership states could be reordered or bypassed; an obsolete publication rail could regain
  production authority.
- **Replacement proof for the retired subset:** Rust visibility and curated crate-root exports;
  compile-time typestate and ownership boundaries; operation activation, publication, accounting,
  poison, panic, and destructor fault-injection tests; the U4c production closure and behavioral
  report.
- **Deletion stage:** U4f removes the remaining migration-only activation feature and legacy Scene
  rail, then deletes the remaining assertions in this file's scope. No source-name or AST-shape
  replacement checker is planned.
- **Status:** partial. Reintroducing a source-string scan requires a new concrete risk that cannot
  be covered by a type boundary, compiler check, behavior test, or fault injection.

## `u4-retained-tests-source-assembly-and-invariant-scans`

- **Owner:** `u4-retained-physical-ownership`
- **Path:** `crates/baozi-operation/src/retained/tests.rs`
- **Pending checks:** test-only `include_str!` source assembly and module-boundary checks;
  allocation/table dependency-name scans; private brand and range-shape scans; rollback ordering
  and exactness substring scans; and legacy terminal-rail name scans.
- **Risk:** the migration tests currently guard private ownership placement, table/range
  separation, storage-before-attribution rollback, incremental-proof usage, and removal of the
  obsolete terminal evidence rail. These checks depend on source spellings and are not yet
  replaced by independently executable interface or behavior evidence.
- **Replacement proof:** crate-private visibility and curated exports; compile-fail coverage for
  brand/ownership escape; runtime publication, rollback, accounting, poison, panic, and
  destructor fault-injection tests; and the U4c/U4d behavioral receipts once the production
  consumer owns these invariants.
- **Deletion stage:** U4f, after the legacy rail and migration-only retained test scaffolding are
  deleted and the replacement proofs are mapped in the conformance inventory.
- **Status:** pending. These scans still execute; this record does not claim them retired.

## `u4-retained-startup-source-scans`

- **Owner:** `u4-retained-physical-ownership`
- **Path:** `crates/baozi-operation/src/retained/tests/startup_operation.rs`
- **Pending checks:** the `single_value_bootstrap_and_primary_prepare_exclude_duplicate_tail_work`
  test assembles production sources and scans for removed setter/single-part spellings and exact
  occurrences of the primary-control and ledger-binding calls.
- **Risk:** startup admission could regain duplicate tail work or an obsolete setter/allocator
  path without changing the runtime assertions that currently cover the same scenarios.
- **Replacement proof:** startup admission and publication behavior tests, allocator/ledger
  counters, fail-closed poison tests, and a compile-time private startup API boundary. The exact
  call-count assertion still needs to be replaced by those proofs before deletion.
- **Deletion stage:** U4f, together with the remaining migration-only startup assertions after a
  real canonical consumer is wired and the replacement evidence is recorded.
- **Status:** pending. The scan remains executable and must not be treated as a retired gate.
