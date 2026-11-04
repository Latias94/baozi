# Borrowed Parser Singleton Replenishment

The isolated [lexical invocation refactor](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md)
now exposes the existing singleton replenishment operation through its borrowed Operation
and Import parser roles. This is a prerequisite for migrating actual OBJ, PLY and glTF
comparison loops. Their remaining full-service/detection/format migration is still open.
Main Rust and ordinary performance are unchanged by this isolated checkpoint.

## Actual Change

`StoreInvocationParser<..., 1, CHILD>::replenish` checks the real invocation's running
state and forwards to its actual `ParserFrame<1>::replenish`. The existing kernel owns
attempted admission, remaining amounts, item limits and channel validation. Runtime
failure uses the existing first-fault recording and mapping. The Import singleton
`ParseParser` forwards through its normal terminal preflight and runtime projection.

No new work owner, counter, parser channel, policy or comparison algorithm is introduced.
Replenishment retains the original channel and does not reset control debt. An unfinished
suffix or any per-item limit rejects additional admission before changing counters. The
multi-lane role has no singleton replenishment method.

Three direct Operation host tests use a real retained operation, account and unique
invocation. They verify 257 actual comparisons across two replenishments, exact completed
counts and the 257th-prefix checkpoint; an unfinished suffix denies further admission
without a poll or new work; a completed item-limited channel still cannot be split.
Failure remains sticky, channels resolve, and physical retained bytes/allocations release
without poisoning the account.

The existing public custom-format Import fixture adds two real lookups requiring six
comparisons. Each comparison replenishes one unit and completes through the borrowed
parser. It checks the literal delta `(6, 6, 0)`, correct lookup positions and no Token work,
then closes the numeric region before real Scene submission. External assertions require
successful import, six comparator calls, one actual submission and the prescribed three
region polls. Public compiler examples pair a valid singleton lookup with forbidden
multi-lane replenishment through the same real entry.

## Source and Qualification

The complete source contains 1,415 files, set SHA-256
`bc699786f882a102d0f19cc318b14660c1574b7f64e0809c384f34845b22773e`.
Compared with semantic source5, exactly four files change: Operation's exclusive facade
and its private tests, Import's borrowed parser facade and its public work-callback tests.
The numeric kernel and existing format implementations remain byte-identical.

| Gate | Actual result |
| --- | --- |
| Operation and Import all-feature doctests | 140 pass, including the new positive/negative pair |
| Complete Operation and Import all-feature nextest | 1,450 pass, zero skips; run `238ab1cb-5d85-4ca5-883a-7cab9d9d0aeb` |
| Workspace all-target/all-feature Clippy | Pass with `-D warnings` |
| Root format check | Pass |

The manifest, five-command serial QA and terminal association are under
`target/review/convergence/`, named `borrowed-parser-replenishment-source-1.json`,
`borrowed-parser-replenishment-qa-1.json` and
`borrowed-parser-replenishment-terminal-association-1.json`. Both complete live/frozen
path sets and file hashes match; all five log hashes match. The preparatory formatter
changes only the new Operation test formatting, and its after map matches the frozen
source. All eight before/after maps for the four subsequent checks match that source.
Independent source/evidence review verifies both complete frozen sources, the four-file
delta, unchanged prior assertions, public compiler/importer probes and all terminal
command/log/map associations without an actionable finding. The report is
`.git/baozi-review/performance-convergence/borrowed-parser-replenishment-review-1.md`.
The new 257-action runtime test covers direct Operation cadence; the new public Import
test covers six comparisons and its existing region polls. Import's threshold-specific
cadence remains supported by the unchanged runner and inspected branch, rather than a
new 257-comparison Import execution. No broader denial/parent matrix is claimed.

This is affected runtime/compiler qualification, not a new complete-workspace runtime,
asset-consumer or ordinary benchmark campaign. The full Import replacement, global
`LocalWork`/`BoundWorkGroup` deletion, current-state attachment, shared promotion, final
main application and unchanged Windows/Linux 14-case/110-lane acceptance remain open.
