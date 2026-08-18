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
  U4d and U4f gate-retirement slices.
- **Retired checks:** obsolete exact occurrence checks removed in `fae8050`, plus the remaining
  final-owner extraction spelling, terminal declaration placement, curated-export spelling, and
  selected private-method scans formerly in the retained architecture tests.
- **Risk previously protected:** generic retained authority could escape the deep module; terminal
  ownership states could be reordered or bypassed; an obsolete publication rail could regain
  production authority.
- **Replacement proof for the retired subset:** Rust visibility and curated crate-root exports;
  compile-time typestate and ownership boundaries; operation activation, publication, accounting,
  poison, panic, and destructor fault-injection tests; and the focused conformance inventory's
  durable behavior gates.
- **Deletion stage:** completed across U4d and U4f. No source-name or AST-shape replacement checker
  is planned.
- **Status:** retired. Reintroducing a source-string scan requires a new concrete risk that cannot
  be covered by a type boundary, compiler check, behavior test, or fault injection.

## `u4-retained-tests-source-assembly-and-invariant-scans`

- **Owner:** `u4-retained-physical-ownership`
- **Path:** `crates/baozi-operation/src/retained/tests.rs`
- **Retired checks:** test-only `include_str!` source assembly and module-boundary checks;
  allocation/table dependency-name scans; private brand and range-shape scans; rollback ordering
  and exactness substring scans; and legacy terminal-rail name scans.
- **Risk previously protected:** the migration tests guarded private ownership placement,
  table/range separation, storage-before-attribution rollback, incremental-proof usage, and
  removal of the obsolete terminal evidence rail.
- **Durable replacement proof:** crate-private visibility and compile-fail coverage continue to
  protect authority escape. Retained publication rollback, accounting, poison, panic, destructor
  fault injection, terminal evidence round trips, and
  `inherited_blocks_deduplicate_source_slots_without_new_attribution_or_physical_charge` continue
  to protect observable ownership and accounting behavior through the
  `retained-store-behavior-tests` gate.
- **Intentionally retired implementation shape:** allocation/table dependency spellings,
  range/table private call structure, the absence of internal structural scans, exact private
  mutation-site counts, and legacy private type names are no longer repository contracts. The
  retirement does not claim executable proof that these source shapes can never recur. A future
  complexity or authority concern must be expressed as a bounded behavioral test, compiler-enforced
  visibility boundary, or public API absence check rather than a production-source substring scan.
- **Deletion stage:** U4f.
- **Status:** retired. The retained tests no longer assemble or parse production Rust sources.

## `u4-retained-startup-source-scans`

- **Owner:** `u4-retained-physical-ownership`
- **Path:** `crates/baozi-operation/src/retained/tests/startup_operation.rs`
- **Retired checks:** the `single_value_bootstrap_and_primary_prepare_exclude_duplicate_tail_work`
  test assembles production sources and scans for removed setter/single-part spellings and exact
  occurrences of the primary-control and ledger-binding calls.
- **Risk previously protected:** startup admission could regain duplicate tail work or an obsolete
  setter/allocator path without changing the runtime assertions that currently cover the same
  scenarios.
- **Durable replacement proof:** startup admission and publication behavior tests; exact allocator
  and ledger counters; structural-failure, builder-error, panic, and poison precedence tests; and
  the compile-time private startup API boundary. These proofs exercise effects and ownership
  rather than freezing private call counts.
- **Intentionally retired implementation shape:** removed setter/allocator names and the exact
  number or placement of private helper calls are not durable contracts.
- **Deletion stage:** U4f.
- **Status:** retired. Startup tests no longer inspect production source spellings or call counts.
