# Local Operation Work Ownership

Status: verified operation-engine foundation at implementation commit
`19eb1e014c059b4fa9bb62edf3da102954ca6b3b`. This is the retained/snapshot/terminal foundation
of P2 in the [architecture continuation](../plans/2026-11-02-1700-perf-import-operation-architecture-plan.md),
within the active [pre-release evolution plan](../plans/2026-11-02-0923-refactor-pre-release-evolution-plan.md).
It does not complete that plan, qualify B1, or establish a performance improvement.

## Implemented Boundary

The previous shared ledger carried work and storage in the same synchronized state. Work now has
one invocation-local owner, while independently surviving reservations and physical payloads retain
their synchronized storage owner. Mixed dimensions are split by field and policy; combined policy
tightening and observations join both authorities in one short interval.

`LocalWork` provides the sole attempted/consumed/pending numeric implementation. Frozen, metered
catalogues bind dense groups before numeric guards. Bound actions reuse their duplicate layout and
completion permutation without owner-key comparisons, sorting, a storage mutex or work-owner Arc
clones. The existing raw adapters currently bind per call; migrating repeated parser and preparation
loops to once-bound roles remains P3 work.

Linear envelopes borrow their real local owner. Physical completion preserves representable
completed prefixes, and full completion still requires explicit finish. Forgotten full, partial,
empty and zero-valued envelopes leave persistent ownership that prevents successful finalization.
Trusted retained wrappers explicitly resolve verified partial cleanup on rejection or unwind;
their failure remains sticky. Raw abandonment and conflicting numeric borrows retain integrity
faults rather than manufacturing a clean terminal.

Progress checkpoints borrow control and progress fields independently of the ledger. Retained
producer permits can therefore report progress or poll without a mutable borrow of the whole
coordinator. No work guard is retained across those calls.

## Snapshot and Cleanup

Interim snapshot backing is admitted and allocated before the joint work/storage copy. Replacement
progress backing and generic commitment aggregation also allocate outside accounting guards.
Their RAII admissions own only their exact admitted bytes, so failure cleanup subtracts those bytes
from current counters and preserves unrelated intervening releases or admissions. Outgoing snapshot
backing is excluded from current ledger evidence while construction overlap remains in the peak.

Final composition uses preadmitted staging and retains local catalogue backing through observation.
Generic pending-publication backing and qualified canonical/evidence discard backing are destroyed
outside the shared storage mutex. Their covering reservations release only after backing
destruction. Physical payload destruction still precedes attribution release; terminal detach still
requires unique storage ownership through `Arc::try_unwrap`.

## Deliberate Contract Changes

- Ordinary work and allocation are irreversibly sealed at publication preparation. Rolling back a
  pending publication releases its verified storage and commitments, and does not reopen work or
  permit another ordinary transaction. Retry remains available before successful preparation.
- A configured, finite failure role can be activated after ordinary sealing and before close. It
  admits protected producer work plus nested internal handle shares through the same numeric
  engine. An operation without that role cannot construct new failure evidence after sealing.
- The final control gate is single-use. Both extension owners are disposed outside guards, even if
  final delivery or one destructor fails. Disposal failures are returned to the existing terminal
  cause selector before success or immutable evidence is accepted.
- Accepted final control leaves work sealed until the later generation/terminal decision. A rejected
  candidate can still enter its finite protected rail. Closed work is never reopened.
- Generic `OperationCoordinator::finish` returns `OperationFinishError` carrying `PublicationCause`
  and the exact final snapshot. A remembered control failure remains a failed finalization after
  clean storage release. Minimal fallback invokes no new cancellation/progress callback.

These contracts amend [ADR 0033](../adr/0033-operation-control-and-observability.md) and
[ADR 0047](../adr/0047-operation-ledger-evidence-and-resource-accounting.md). No compatibility alias,
shadow work ledger or alternate runtime backend was introduced.

## Verification

Final qualification used Rust 1.95.0 on Windows `x86_64-pc-windows-msvc`, the shared repository
`target`, locked offline dependencies and one build job. These are development-profile correctness
checks, not release-profile timing measurements.

| Check | Result | Receipt |
| --- | --- | --- |
| Workspace all-feature nextest | 3,128/3,128 passed; 6 explicit manual diagnostics/pilots skipped | `285ced6d-1a5e-4d68-8bf8-57508649b769` |
| Independent assets consumer all-feature nextest | 120/120 passed; 1 explicit manual profile skipped | `0c8480f6-a7eb-4e83-a2d6-1adf4d22d911` |
| Workspace and assets all-target/all-feature strict Clippy | Passed with `-D warnings` | `p2-workspace-clippy.log`, `p2-assets-clippy.log` |
| Workspace and assets formatting | Passed | `p2-workspace-fmt.log`, `p2-assets-fmt.log` |
| Operation doctests | 37 passed: 36 compile-fail contracts and 1 compile example | `p2-operation-doctests.log` |
| Operation rustdoc | Passed with `-D warnings` | `p2-operation-doc.log` |

The earlier standalone operation run passed 805/805 (`160733dd-7d38-49b7-9309-9b1c31b3b1c8`)
before final lint-only integration. The final workspace run refreshes that package with the complete
integrated source. Earlier failed/intermediate `p2-*` logs are preserved as diagnosis history and
are not final-source passing receipts.

Local evidence is `target/review/p2-operation-verification.json` and
`target/review/p2-operation-evidence.zip` (SHA-256
`dad1649182bbb91c83c14945b2ed59666ca173cde379f1aefb57ed0386fe2746`). It contains logs, bounded review
artifacts, verification metadata and the committed source archive. The source archive alone has
SHA-256 `baf1af45b8361ea66a3309a43d1924eafb07c6c479ef8730619c2b7a421cb043`.
No Linux, browser/WASI, sanitizer, unchanged-package or fresh GPU execution is claimed by this slice;
those final-source qualifications remain in P6.

A fresh native Codex reviewer, with no implementation role, inspected the changed numeric,
snapshot, storage-admission and terminal paths and reported no actionable correctness finding
within that scope. It did not run tests or audit the entire historical retained engine. An earlier
implementation-owner review identified catalogue lifetime and locked discard deallocation gaps;
both were corrected before this receipt. Source-string implementation-shape assertions were removed
where arithmetic, comparator, allocation-failure, cleanup-order or ownership tests exercise the
actual behavior.

## Remaining Continuation

P3 must connect parser parent/child channels to the local owner, lend concrete disjoint execution
views, bind repeated layouts once, implement the reviewed 256-action geometry cadence and
4,096-byte text frontier, and replace deferred text completion with physical-unit completion.
Normal-kernel callbacks must surround the real leaf action rather than complete an empty action
before subsequent arithmetic. Consuming Scene submission must occur after execution borrows end.

The typed format/plan/context replacement remains P4. Remaining text/construction/reader work is
subject to P5 measurement. Complete Windows/Linux B1, separate heap observations and original U14
package/portable/sanitizer/GPU qualification remain P6. Native target admission and publication
authority are unchanged.
