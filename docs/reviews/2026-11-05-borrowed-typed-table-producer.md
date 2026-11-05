# Borrow Typed-Table Producer Work from the Existing Invocation

The actual immutable typed-table builder now borrows the invocation's unique work
view and uses the existing `WorkLayout`/`WorkFrame` arithmetic. Its former owning
producer permit, public forwarding facade and dedicated admission shim are removed.
This is one connected production-consumer migration in the
[lexical-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md),
not complete numeric backing replacement or a performance keep.

## Preserved construction contract

`CanonicalLane::retain_storage` still admits physical backing before preparation,
then admits the entire nonzero producer envelope before the physical builder.
Each normally returned unit records one actual completed unit immediately. Entry
and first-action polls, the existing 4,096-unit cadence, semantic-string completion,
sticky errors and exact/partial cleanup retain their existing meaning. Zero work
creates no producer frame. No policy, normal expression, benchmark task or timing
boundary changes.

The private `RetainedStorageWorkHost` owns the frame outside the builder callback;
the callback borrows only its build control. Forgetting that control borrow cannot
move or forget the host. Partial failure resolves the unused suffix before physical
cleanup. Incomplete success and excess units reject rather than publish a table.
The kernel owns the remaining count; the duplicate wrapper counter is gone.
Physical storage leases still independently survive successful construction.

Removed paths include `RetainedProducerWorkPermit`, `RetainedStoreObservedWork`,
their entry/export/finish types, `Ledger::try_admit_producer_work_batch`, and now-unused
coordinator checkpoint splitting. Surviving direct producer accounting, progress,
policy and state interfaces remain. Existing Internal/protected/share work, storage
owners and the common arithmetic kernel are still live and are not removed by this cut.

## Meaningful regression evidence

The focused run `325c7cce-15cc-4dc5-bc7f-e98c15203607` passes 16 tests with 318
filtered cases: eight actual typed-table host tests and eight surviving direct
producer tests. One host test executes two policy branches. Actual host oracles cover:

- Whole-envelope denial before the builder, both attempted-limit and consumed-limit
  failures, exact nonrefundable attempted evidence and physical preparation cleanup.
- Real Internal sharing during prepare, successful nonzero Producer construction,
  child lifetime after the original owner drops, and the next table's clean admission.
- Cancellation raised during prepare, before producer admission, with no builder call,
  no producer attempt charge, completed preparation Drop and original Control failure.
- Exact, partial and zero work after a forgotten control borrow; excess and incomplete
  units/strings; panic containment; and cancellation before physical unit 4,097.

Seven old owning-permit API tests are replaced at the actual host and existing kernel
boundaries; deletion is not reported as a test-count increase. Two new host cases and
the additional consumed-policy iteration close the source2 review's G1/G2/G3 requests.
The earlier test compiler-type mismatch and invalid asymmetric-start fixture remain
in focused logs 2/3. The corrected test starts with a valid symmetric policy and
tightens it through the real current-state API before the operation under test.

## Frozen source and validation

Source3 is frozen under
`.git/baozi-review/performance-convergence/typed-storage-borrowed-envelope-source-3/`.
Its 13-path manifest SHA-256 is
`b964aab4913bce8c088fc9eb1b6dda7240f6964d74ee708877dd288544ab0f83`.
The baseline revision is `308e9ab890b9e490c3f2bf3a444ca155d9244cd5`; main HEAD
`2503673` adds only the preceding measurement report. All seven changed production
files are byte-identical to independently reviewed source2; source3 adds only the
two integration-test deltas.

Independent source reviews 1/2 find no confirmed P1/P2 defect. Review 2 checks all
13 after hashes, 12 baseline mirrors, the actual sharing/cancellation/admission
edges and the focused raw log, and closes G1/G2/G3 without a production fix request.
The reports remain under `.git/baozi-review/performance-convergence/` with names
`typed-storage-borrowed-envelope-source-review-{1,2}.md`.

The seven gates use Rust 1.95.0, locked/offline dependencies, the main root target,
one build job and one nextest thread. All 1,451 input hashes, including the new
untracked test file, remain unchanged through the recorded gates.

| Gate | Actual result |
| --- | --- |
| Workspace runtime | 3,566 passed; six existing skips; run `1cbc9000-fe27-48da-a1ca-2b13031d59af` |
| Workspace doctests | 188 passed |
| Workspace strict Clippy | All targets/all features; warnings denied; passed |
| Independent asset consumer | 138 passed; one existing skip; run `979c278c-0621-4fdf-b5c0-7f3e627010e7` |
| Asset-consumer strict Clippy | All targets/all features; warnings denied; passed |
| Workspace and consumer formatting | Both passed |

The original execution is interrupted during consumer compilation. Its handle
is subsequently absent and no matching OS process remains. Original QA1 preserves
the first three completed gates and the incomplete consumer log. QA2 verifies the
same source map and original log hashes, retains those three results unchanged,
then completes only the remaining four gates. Resume execution `55933` actually
exits zero; the original orchestration is not called terminal-successful.

The completed receipt is
`target/review/convergence/typed-storage-borrowed-envelope-qa-2.json`, SHA-256
`a8a61bb27e1383d382104463868b9bd4a0308ee8325dee9b799a1f08039a6195`.
Main-root dep-info and binary hashes are retained separately in
`typed-storage-borrowed-envelope-artifact-association-1.json`. The root integration
binary built by the focused run is reused for unchanged source; the full workspace
compiles the current Operation unit-test binary, and the asset consumer compiles
current Operation/Core/Import/format libraries against the same root. Those are
observational build associations, not a compiler attestation or an independent rebuild.
Independent final evidence review verifies the 13 frozen paths, 12 baseline mirrors,
complete input maps, original/resumed receipts, all seven log hashes/counts and the
main-root compilation association without an actionable mismatch. Its report is
`.git/baozi-review/performance-convergence/typed-storage-borrowed-envelope-final-evidence-review-1.md`.

## Remaining release work

No ordinary Windows/Linux or heap measurement has yet used this source. The preceding
[metadata performance report](2026-11-05-parse-metadata-borrowing-performance.md)
cannot be attributed to this table-builder change. Performance/release remain HOLD.

Connected Internal/protected/share migration, obsolete parser/numeric adapter deletion,
fixed/dynamic borrowed backing, shared-shell-before-state promotion, exact administrative
accounting and indexed selection remain open. Complete current-source ordinary
Windows/Linux 14-case/110-lane time and whole-child memory acceptance, representative
separate heap attribution and final correctness/consumer gates remain required.
