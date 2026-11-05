# Borrow Connected Retained Work

Canonical construction and rewriting, ordinary outcome-evidence construction,
canonical source mapping and inheritance, success validation, and protected
evidence construction/validation now use one unique numeric root beside the
existing shared storage authority. They no longer pass a full Ledger alias
alongside a bound work control.

The root splits actual state without inserting an earlier execution, quiescence,
mode or budget rejection before physical gates. Binding retains the original
key/mode/role precedence. Forgotten RefCell guards remain unavailable: split
records shared/mutable availability, and the corresponding actual numeric
operation checks it and preserves the existing fault. No numeric rows, policies
or counters are copied into a second backend.

## Admission and completion

Ordinary preparation retains its original layout or deferred error and poll
checkpoint, without admitting work. Protected binding remains at actual permit
entry. Account/table/action gates still precede numeric share admission. Joint
attribution follows that admission and checks current numeric integrity, mode
and the typed destination before the existing storage transaction. Protected
evidence additionally requires Closing storage. Ordinary and exclusive callers
share the same mode-to-storage-admission rule.

The handle share finishes before a builder receives its numeric loan. Builder
admission therefore sees the same row's latest attempted, consumed and pending
counts. Inheritance retains observed scan work before deciding whether another
physical share is required.

The real frame remains in an outer host. Callback controls and permissions borrow
its slots; forgetting a permission cannot forget that frame. Exact/partial/sticky
completion runs inside the existing physical builder adapter, before lease
commit, membership or attribution transfer. Ignored errors and incomplete work
cannot commit an accepted value. The same host resolves validation work before
proof acceptance, followed by the existing current-operation storage, outstanding,
partial and terminal-cause checks.

Ordinary work uses Ordinary/Internal, protected work Protected/Protected, and
protected sharing Protected/Internal. Terminal polling snapshots only whether a
control cause had already been accepted at the original split. A semantic cause
does not exempt a later cancellation. Poll reborrows preserve that snapshot.

The host settles numeric disposition during unwind while the existing physical
allocator retains its poison and unverified charge policy. A known completed
prefix remains consumed when later accounting fails; cleanup releases only the
unused suffix. Independent physical owners keep their release authority.

## Retired implementation

`RetainedWorkPermit`, `BorrowedRetainedWorkPermit` and
`RetainedFailureWorkPermit` are removed. Their replacements are short permissions
over the externally owned frame, using the existing WorkLayout/WorkFrame kernel.
Split methods return a deferred binding plus the unique numeric/storage root,
not a shared Ledger reference. The duplicate remaining-counter mirror and its
completion helper are also removed.

The old forgotten-validation test now expects proof rejection and clean fallback:
the actual frame remains resolvable in its host. Raw forgotten numeric frames
still fail quiescence; this change does not recover those owners or leaked guards.

## Verification

Final source qualification passes. Preliminary failures are retained:
the old forgotten-validation expectation and an undersized protected-test writer
fixture. Neither preliminary run is assigned as final qualification.

Base commit is `dcf163f649a4f46094bc0b936e955a8d1ef879ae`. The final source manifest is
`.git/baozi-review/performance-convergence/connected-retained-work-source-1.json`,
SHA-256 `7046438f4dcd4841cd8cc07548b41b71f06e702766f3f2d3dd5743d60f08906a`.
Its 1,165 input files remain byte-identical through all seven serial gates:

| Gate | Result |
| --- | --- |
| Workspace runtime | 3,589 passed; six existing skips; run `2638a8fb-44a7-4ff6-9674-ec162bdf30ad` |
| Workspace doctests | 188 passed |
| Workspace strict Clippy | All targets and features; warnings denied |
| Independent asset consumer | 138 passed; one existing skip; run `074bd363-8888-4885-adb0-0fb242c2f912` |
| Consumer strict Clippy | All targets and features; warnings denied |
| Workspace and consumer formatting | Both passed |

The actual orchestrator `40946` exits zero. Its terminal receipt is
`target/review/convergence/connected-retained-work-qa-1.json`, SHA-256
`a1077d92c807187ba2a96e05830b183308ba9e1a3feef48b63276e44114eaebf`.
All commands use Rust 1.95, locked/offline dependencies and the root target;
runtime checks use one build job and one test thread. The workspace runtime
includes an actual eleven-minute fresh compilation before its test run.

Eleven new ledger tests cover deferred fault observation, original catalog/row
identity, binding precedence, legitimate active Internal/Producer/parser frames,
forgotten dynamic guards, cleanup after a fault, and joint attribution against
current mode, destination and storage publication state.

Eight new connected-assembly tests cover sixteen scenarios: table/physical gates
before invalid deferred binding, fresh cancellation before deferred binding,
same-row post-share consumed headroom, owned/borrowed forgotten permissions with
partial/full prefixes, ignored incomplete/overrun errors, accepted versus fresh
protected cancellation, and owned/borrowed body/capture panic. The existing
forgotten-validation test now verifies rejection, the consumed prefix and clean
fallback. All nine focused tests pass on the final source.

Independent production and final test-delta reviews find no blocking defect.
The latter verifies all 1,165 frozen inputs and associates the production review:
13 original production files are byte-identical; two differ only in CRLF/LF.
The direct protected body/capture-panic combination remains a non-blocking
coverage limit; the current tests establish shared-host cleanup and protected
cancellation separately. These reviews do not supply performance evidence.

Independent final evidence review verifies the seven receipt/log hashes, terminal
counts and command parameters, all 1,165 source identities, the 22 staged Rust
files and the prior production-review association without a mismatch. The report
is `.git/baozi-review/performance-convergence/connected-retained-work-final-evidence-review-1.md`.
The five unrelated user notes remain untracked and byte-identical. Isolated next-step
source candidates are excluded from this qualification and have not been compiled.

## Remaining scope

The execution facade survives through forwarders, tests and a manual diagnostic;
retained backing still uses its storage helper. LocalWork, table/startup/publication
sharing and cold observation also have actual users. They require migration and
deletion with their last callers. Fixed/dynamic backing, shared promotion, exact
administrative accounting and indexed selection remain open.

No new ordinary performance, whole-child memory or heap result is assigned to
this source change. The original Windows/Linux matrices and release gates remain
open under the [convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md).
Release remains HOLD.
