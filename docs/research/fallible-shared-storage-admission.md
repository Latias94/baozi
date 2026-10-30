# Fallible Shared Storage Admission

## Decision

Baozi conditionally admits `triomphe` 0.1.17 as a crate-private primitive for
fallibly allocating immutable shared storage. The dependency is not part of a
public contract and does not authorize copy-on-write APIs that hide allocation.

The admitted Cargo configuration is:

```toml
triomphe = { version = "=0.1.17", default-features = false }
```

The initial consumer is `baozi-core` provenance storage. ADR 0042's canonical cutover moves the
private shared-allocation backend into `baozi-operation`, where it becomes the sole concrete
retained-allocation/final-owner implementation. Baozi remains responsible for budget admission,
fallible payload construction, fresh branch segment identity, segment-qualified attachment
validation, physical-account release ordering, and transactional publication.

## Package Identity

| Field | Admitted value |
| --- | --- |
| Package | `triomphe 0.1.17` |
| Cargo source | `registry+https://github.com/rust-lang/crates.io-index` |
| Published | 2026-10-10 18:00:59 UTC |
| Yanked | No |
| Crate size | 41,935 bytes |
| SHA-256 / index checksum | `0d15f4f5898bcfc5d8a8e45ed30b2f108d710c1a2e5489dbf80e078f707181bc` |
| Reviewed VCS commit | `1c3cfa221b3f97729cbc989fc67976f45f874331` |
| Annotated tag | `v0.1.17` |
| License | `MIT OR Apache-2.0` |
| Declared MSRV | Rust 1.85 |

The locked checksum matches the locally downloaded `.crate` SHA-256, and its
`.cargo_vcs_info.json` identifies the reviewed upstream commit. The source archive is a public
registry artifact, not signed provenance.

## Allowed Surface

The Baozi wrapper may use only these ownership operations:

- `Arc::try_new` and `UniqueArc::try_new` only after Baozi has admitted all payload storage;
- `UniqueArc::shareable` only for the one owner-to-share transition;
- `Arc::try_unique` only when restoring the original shared handle after a non-final-owner result;
- `Arc::into_unique` only when consuming every private clone in the ADR 0042 final-owner path;
- `UniqueArc::into_inner` only for the exactly-one winner, to deallocate the shared block before the
  returned payload and bound lease are destroyed in that order;
- `Arc::get_mut` only in `#[cfg(test)]` code that deliberately corrupts private fixtures;
- `Arc::clone` for allocation-free reference-count increments;
- ordinary borrow, dereference, and drop operations.

The wrapper must not expose `triomphe` types or a production mutable/COW API. It must not call
`Arc::new`, `make_mut`, `make_unique`, infallible conversion or deserialization paths, or any API
that clones a payload behind an infallible interface. Once a physical lease is bound inside the
shared allocation, ordinary `Arc` drop is forbidden: every published clone is consumed by
`Arc::into_unique`, and the sole winner is consumed immediately by `UniqueArc::into_inner`. Ordinary
drop remains admitted for the current pre-cutover `SharedStorage` wrapper and for values that never
contain a bound physical lease; the U4 cutover deletes that legacy path.

## Failure Semantics

`Arc::try_new` checks the combined header and payload layout, calls the global
allocator directly, and returns `AllocError` for layout overflow or a null
allocation. On failure, its input is dropped normally. This makes the outer
shared allocation reportable, but it does not make construction or cloning of
the payload fallible.

Baozi must therefore complete these steps before publication:

1. admit the complete retained-capacity and work envelope;
2. build any replacement payload with Baozi-owned `try_reserve` paths;
3. mint fresh scene or branch-segment identity where the contract requires it;
4. preserve shared ancestor attachments while rejecting foreign sibling attachments, and rebind
   generation-qualified derived relations;
5. call `Arc::try_new` and publish only after every prior step succeeds.

Identity-preserving provenance remaps retain a bounded source-segment snapshot in shared storage and
remain qualified to one target branch. Cloning that remap increments only shared reference counts;
it neither allocates nor weakens source/target validation.

For an ADR 0042 bound allocation, the private wrapper must establish these stronger conditions:

1. the bound lease and payload are passed directly to `Arc::try_new` after both admissions succeed;
2. a successful raw `Arc` is wrapped immediately by an infallible expression before any callback,
   validation, fault injection, or other panic-capable action;
3. every wrapper destruction calls `Arc::into_unique` rather than ordinary drop;
4. the exactly-one winner calls `UniqueArc::into_inner`, then destroys the Baozi-owned inert payload
   and its nested backing blocks, then destroys the bound lease; and
5. the wrapper exposes no raw/weak handle, `try_unwrap`, `try_unique`, detach, rebrand, or manual
   release path.

`Arc::clone` does not allocate. Its reference-count overflow guard aborts after
`isize::MAX` references; Baozi accepts that unreachable process-liveness edge
and does not treat it as a recoverable allocation failure. `Arc::into_unique` performs one `AcqRel`
decrement and guarantees exactly one winner when every clone uses it. `UniqueArc::into_inner`
reconstructs and consumes the allocation before returning the inner value. The wrapper then drops
the payload before its bound lease, so charged headroom is not released before the shared control
block or exclusive nested backing blocks are deallocated. Shared payloads must be sealed Baozi-owned
inert types with bounded inline size and non-panic-prone destructors.

## Feature And Closure Review

The upstream default enables `serde`, `stable_deref_trait`, and `std`. Baozi
disables default features, producing a resolved closure containing only
`triomphe`. The crate uses `core` and `alloc` without native code, FFI, a build
script, or an operating-system API. Optional `serde`, `arc-swap`, and `unsize`
features are not admitted.

The source is compatible with `wasm32-unknown-unknown` and `wasm32-wasip1`,
but upstream CI does not exercise those targets. Baozi's integration gates must
therefore compile both targets with the actual locked dependency graph.

## Safety And Maintenance Assessment

The crate is an unsafe trusted-computing-base dependency. A source scan found
11 Rust files and approximately 3,315 lines, including 84 `unsafe` blocks, 24
unsafe function declarations, and 22 unsafe implementations. The core risks are raw allocation and
layout synthesis, DST metadata construction, reference counting, raw-pointer ownership conversion,
final-owner election, deallocation-before-return behavior, and `Send`/`Sync` invariants. The expanded
allowlist specifically reviews `Arc::into_unique` at `src/arc.rs:163-218` and
`UniqueArc::into_inner` at `src/unique_arc.rs:103-114` in the admitted source.

Upstream runs stable, MSRV, nightly, Miri, AddressSanitizer, and
ThreadSanitizer checks. The repository is active and not archived, but crates.io
publishing authority has one owner. `try_new` remains an explicit allocation failure path.
Version 0.1.17 fixes an `OffsetArc::make_mut` reentrant-drop soundness defect and changes the
`UniqueArc::shareable` unsafe body; review this exact ownership transfer in addition to the unchanged
`Arc::into_unique` and `UniqueArc::into_inner` paths. A clean advisory query is not treated as a
safety proof.

At review time, RustSec advisory-db commit
`6e3286f4efa8c142fb33e5ea4342c8db6693cf34` and GitHub Advisory Database
queries had no entry affecting `triomphe` 0.1.16. The 0.1.17 dependency was selected after a
separate 2026-10-29 version/source comparison. Baozi's license allowlist already accepts MIT and
Apache-2.0, and both license texts ship in the crate.

## Integration Gates

Admission becomes effective only after the locked integration passes:

```text
cargo +1.95.0 check --locked -p baozi-core --all-targets
cargo +1.95.0 check --locked -p baozi-operation --all-targets
cargo +1.95.0 clippy --locked -p baozi-core --all-targets --all-features -- -D warnings
cargo +1.95.0 clippy --locked -p baozi-operation --all-targets --all-features -- -D warnings
cargo +1.95.0 nextest run --locked -p baozi-core
cargo +1.95.0 nextest run --locked -p baozi-operation
cargo +1.95.0 check --locked -p baozi --target wasm32-unknown-unknown --no-default-features --features all-formats
cargo +1.95.0 check --locked -p baozi --target wasm32-wasip1 --no-default-features --features all-formats
cargo deny --locked check
cargo deny --locked --manifest-path fuzz/Cargo.toml check
```

The lockfile must show exactly `triomphe 0.1.17` with the admitted checksum and no transitive
dependency. Tests must also inject wrapper allocation failure, prove that provenance fork failure
leaves the input arena and attachments unchanged, race final-owner extraction, and use an
allocator-level barrier to prove the physical account remains charged through outer-control and
nested-backing deallocation. Source/API-shape checks reject an ordinary post-bind Arc drop or any
raw/weak/try-unwrapping escape.

## 2026-07-17 Final-Owner Surface Re-review

This section records the review of version 0.1.16 at that date. The 0.1.17 update is reviewed below.

The package identity, source tree, checksum, features, transitive closure, MSRV, licenses, advisory
queries, target claims, and upstream safety posture were unchanged from the initial review. The
allowlist expansion was limited to the two safe consuming methods above. Both rely on existing
unsafe reference-count/raw-allocation internals, so their exact source and deallocation order are
treated as security-sensitive implementation evidence rather than inferred from the safe signature.

The rejected alternative is a Baozi-owned custom allocator/refcount implementation. It could provide
the same post-deallocation hook but would add first-party unsafe layout, atomic, and deallocation
logic. A paired account-slot design remains a safe fallback if final extraction disappears, but it
duplicates the handle count and needs a generational ABA protocol. The pinned extraction path was
admitted because it was smaller, kept one refcount, and preserved the strict ADR 0042 release rule.

## 2026-10-29 Version 0.1.17 Re-review

The locked 0.1.17 checksum and local crate archive SHA-256 both equal
`0d15f4f5898bcfc5d8a8e45ed30b2f108d710c1a2e5489dbf80e078f707181bc`. Its registry manifest
declares Rust 1.85, below workspace MSRV 1.95. Upstream commit is
`1c3cfa221b3f97729cbc989fc67976f45f874331`.

`Arc::into_unique`, `Arc::try_unique`, and `UniqueArc::into_inner` retain the final-owner semantics
used by the deallocation-before-lease-release path. The crate adds an `OffsetArc::make_mut`
reentrant-drop soundness fix. Baozi does not use `OffsetArc`. Its actual `UniqueArc::shareable` call
now uses a `ManuallyDrop` and `transmute_copy` transfer: safety review requires the unique Arc to
move exactly once without running either source or destination destructor during the transfer.
Retain operation-level final-owner tests and review that exact upstream source on future updates.

## Re-review Triggers

Reopen the complete admission review on any version, source, checksum, VCS
commit, feature, transitive-closure, allocator, wrapper allowlist, MSRV, target,
license, advisory, ownership, or unsafe-surface change. Enabling an optional
feature is a new dependency decision, not a local configuration tweak.
