# Owned Dynamic Invocation: First Integration Cut

Status: source 12 passes all seven integrated gates: 3,664 workspace tests, 191
doctests and 138 assets-consumer tests, both strict all-target/all-feature Clippy
checks and both format checks. The six workspace manual skips and one consumer
manual skip are unchanged. All 974 named inputs remain stable. Bounded source
reviews close their findings, and independent final-receipt review is GO for
these seven gates. Performance and finite-owner acceptance remain separate.
The finite owner is the
second cut of the [owned invocation plan](../plans/2026-11-06-1240-owned-finite-invocation-plan.md).
Release remains **HOLD**. No native performance keep or local-main runtime merge is
established by this report.

## Ownership problem and resulting structure

The previous coordinator coupled invocation-local work to a shared storage owner.
Progress retained a separate storage binding, and terminal preparation consumed a
whole ledger at three transfers. That representation forced the state through the
owning shared path even when a caller needed only short numerical operations.

The dynamic invocation now owns the actual coordinator, work rows, storage, progress
and control in its root. Startup, body and terminal stages receive shorter permissions.
Storage arithmetic uses views into the actual header and rows. Progress contains only
progress fields and receives a temporary storage authority when reporting. This cut
does not add a second ledger, numeric `RefCell`, unsafe storage backend or finite
variant to Import's dynamic owner and error types.

The root arms terminal ownership before lending permission. All three former ledger
transfers, the pre-freeze classifier and the primary/fallback guards carry that loan
until actual detach. Detached publication remains owned. Losing a permission leaves
the root able to retain the original physical graph and its real accounting state.

Ordinary abandonment disposes physical owners and control extensions before examining
the remaining state. An outstanding terminal permission preserves its complete
quarantine. Dirty unattached output retains the original domain with an explicit
`ManuallyDrop` disposition; a clean semantic failure retires its domain normally.

## Inline storage and the first independent token

The movable dynamic owner selects Inline storage until an independently releasable
token requires Shared storage. A tokenless empty attribution batch keeps Inline
storage and its exact administration. A checked stable ledger identity still rejects
a foreign empty batch. A genuine zero-byte reservation is a live capability and
therefore crosses the Shared boundary.

Promotion qualifies the source, known payload budget and vacant destination while the
source remains installed. Taking and installing the source performs no allocation,
callback or fallible conversion. Later reservations retain the same Shared identity.
Publication qualifies aggregate allocation, key binding and current limits before
promotion. Empty reservation transitions use the same numerical kernel without an
unrelated Shared-owner error. Poisoned detach preserves the original owner, or retires
only the known shell credit if that shell has actually been dismantled.

The retired Shared-only progress growth and candidate implementations are removed.
The selected candidate uses field-order RAII: its real backing dies before its owned
admission refunds. Existing allocation, envelope, concurrent-counter and destructor
oracles exercise the remaining implementation. Selected Inline and Shared tests cover
replacement, ZST and ordinary spare capacity, including an armed allocation failpoint.

## Administrative observations and disposal tests

The observation is named administrative bytes. It counts actual owned vector
capacities, progress, known Shared payload and candidate overlap exactly once. Opaque
Arc headers, allocator overhead and runtime metadata are outside this accounting;
the value is not RSS or a whole-worker memory measurement.

Terminal capture preserves the actual value before teardown. Qualification checks
structural coverage, including actual progress capacity agreeing with its retained
credit. Positive metadata alone does not establish cleanliness. Live storage,
reservations, unfinished work, pending publication and physical ownership retain
their independent rejection gates. Post-destruction detached scalars still require
zero. The unpublished terminal evidence schema remains revision 1.

Same-owner interim snapshots supply exact expectations where the public fixture can
capture one. The tight-budget fixture cannot afford an interim allocation: its exact
capacity and first-token denial/success oracle therefore lives at the private owner
boundary. Public header tests check their documented high-water relation; the actual
typed header/cell test retains exact values. These inequalities are not presented as
capacity proofs.

Import storage observations are inert, invariantly branded positions that read through
the current context. The temporary wrapper-lifetime flag was removed. Element and
owned-iterator destructors inspect the existing physical account's actual retained
credit, with logical current-context checks after disposal. An intentional early-refund
mutation is rejected by the oracle. Physical credit does not independently prove a
dimension's logical attribution during Drop; the logical release kernel's tests and
the adapter's composition cover that separate obligation.

## Qualification history

The local evidence root is `.git/baozi-review/performance-convergence/`. Each frozen
subject composes 974 named Rust/Cargo inputs, including the two new ownership modules.
Every attempted command retains its source maps, output and terminal status.

| Attempt | Actual outcome |
| --- | --- |
| Focused 1-5 | Compilation failed; no runtime tests executed. Type, visibility, observation and test-fixture migrations are retained in their receipts. |
| All-target check 1 | Collected the remaining internal and integration test caller migrations. |
| All-target check 2, source 8 | Passed without warnings; all 974 inputs stable. |
| Focused 6, source 8 | 2,596 passed, nine failed, six existing skips. |
| Focused 7, source 9 | All 2,606 tests passed, six existing skips; strict Clippy rejected 30 Operation library diagnostics. |
| Lint 1, source 10 | Production diagnostics cleared; 11 test-target diagnostics remained. |
| Lint 2, source 11 | Strict all-target/all-feature owner Clippy passed; all 974 inputs stable. |
| Final 1, source 11 | Workspace nextest: 3,606 passed, 58 failed, six existing skips, two slow. The later six gates did not start. |
| Final 2, source 12 | All seven gates passed: 3,664 workspace tests, 191 doctests, 138 consumer tests, both strict Clippy checks and both fmt checks. Six workspace and one consumer existing manual skips; two workspace slow tests. All 974 inputs stable. |

The nine runtime failures exposed both fixture changes and two implementation defects:
catalog shape lost precedence to a live-reservation error, and ordinary abandonment
retained control extensions with dirty state. Those production paths were corrected;
the original tests passed on source 9. The other repairs account for first Shared
payload admission, truthful capture metadata, revision 1 and the deferred finish API.

Independent reviews separately cover terminal/root ownership, storage and promotion,
and Import destructor oracles. Their source verdicts do not substitute for the final
integrated gates. Source and test changes made after a passing subject require their
own binding and verification.

The 58 full-workspace failures all reached the 14 public helper sites still requiring
zero captured administration. Source 12 replaces their 16 zero assertions with the
public high-water relation. It changes no production implementation, resource/work
limits, control causes, zero live/active checks, physical-prefix or geometry oracles.
The exact private capacity/header qualification checks remain the ownership proof;
the public inequalities do not replace them. Final 1 remains a failed attempt.

## Windows inline layout observation

The standalone standard-Rust probe linked the three existing current-target rlibs
after Final 1 was terminal. Its source-11 map and dependency SHA-256 hashes stayed
unchanged across compilation and execution. The compiler was Rust 1.95.0, Windows
x86-64 MSVC, with the existing unoptimized debug libraries. The probe adds no product
API, test, feature or dependency.

| Observed type | Size, bytes | Alignment, bytes | Scope |
| --- | ---: | ---: | --- |
| Canonical invocation host | 7,296 | 16 | Import-shaped public instantiation |
| Store invocation root | 3,384 | 8 | Import-shaped public instantiation |
| Store invocation context | 360 | 8 | Import-shaped public instantiation |
| Canonical body operation | 48 | 8 | Import-shaped public instantiation |
| ParseContext | 856 | 8 | Exact public type |
| ParseRoot | 200 | 8 | Exact public type |
| ParseServices | 24 | 8 | Exact public type |
| ImportLedgerSnapshot | 248 | 8 | Exact public type |
| ParseFailure | 32 | 8 | Exact public type |
| StoreLedgerEvidence | 192 | 8 | Exact public type |
| Store runtime/prepare/start errors | 128 each | 16 | Exact public types |

The Import-shaped rows use actual Import progress and Core scene output/proof/error
types and tags, substituting a public test Owner for the private Import owner marker.
Changing that marker from zero bytes to 1,024 bytes preserved all four reported sizes
and alignments. This is a concrete observation of the public instantiations, not the
exact private alias, field offsets, ABI equivalence, stack-frame size, heap use, RSS,
native timing, or evidence that a later finite owning variant would leave them equal.
No previous-runtime layout comparison was executed. The private activation-error
layout remains outside the probe's scope.

## Remaining work

Preserve the qualified source and its scoped local commit, along with the layout
probe's private-owner limitations.
The next integration adds the real finite scene-edit owner, fixed plan, finite facts
and dirty error carrier through every Core/postprocess exit, and removes the replaced
Vec-backed finite paths. Allocation and native results remain unmeasured for this cut.
The original Windows/GNU 14-case, 110-lane, three-warmup/twenty-retained-pair protocol
and release targets continue unchanged.
