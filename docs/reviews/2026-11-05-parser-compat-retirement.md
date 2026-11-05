# Retire the Aliasable Parser Compatibility Chain

The unused `ParserWorkScope` -> `ExecutionParserScope` -> `StoreParserWorkScope`
chain and its constructor methods/reexports are removed. Current Import and format
parsers already use the lexical `StoreInvocationParser` host; no production caller
is migrated to another numeric implementation by this deletion.

`ParserWorkError`, its concrete error conversion, the actual `ParserFrame`/
`WorkLayout` arithmetic, exclusive parser host and existing host tests remain.
Internal/protected/share work, the surviving execution facade and `LocalWork`
still have callers. They are not represented as retired by this checkpoint.

## Retained regression meaning

The former 31 adapter tests become 27 direct kernel tests. Ordered attempts,
item-limit precedence, duplicate lanes, overflow, atomic completion, pending
Internal work, parent/child exclusion, zero/empty ownership and replenishment
remain literal counter/flag oracles. Adapter-only unwind reuse and dynamic-borrow
cleanup fixtures are removed with their adapter; real host unwind/control tests
and remaining owner borrow-fault tests continue to cover their actual contracts.

Four integration tests now call the real exclusive host. They retain decoded
bytes, storage/progress during parsing, completed prefixes, rejected callbacks
and stopped continuation. Forgetting a public parser borrow cannot forget its
host-owned frame: exact zero/full callbacks resolve, and a subsequent child and
success remain valid. In contrast, deliberately unresolved raw kernel frames
still block quiescence and attachment qualification. The attachment test keeps
both positive explicit resolution and persistent forgotten-channel rejection.
This difference follows actual ownership rather than compatibility with a deleted
by-value adapter.

Independent source review checks the complete 13-file delta, test-oracle map and
byte identity of the kernel/real host. It finds no correctness defect or essential
loss of current-host coverage. Its minor raw-frame `forget_non_drop` risk is fixed
using local `ManuallyDrop` wrappers without resolving those frames. A follow-up
reviews exactly that change and formatting, finding no semantic drift.

## Qualification

Integration baseline is `9fd7a9145a29578fd53f39e73cabe75ae3ccdba8`.
Frozen source2 is under
`.git/baozi-review/performance-convergence/parser-compat-retirement-source-2/`;
manifest SHA-256 is
`72c9947d8ea1c18b87953a106ca745ad09121860bed752696d18d6fb960f3f1c`.
All 13 after hashes match the reviewed source; the only post-review deltas are
the raw-frame wrappers and formatting. No benchmark, work weights, numerical
algorithm, physical-storage policy or cancellation cadence changes.

The actual main-owned Operation nextest run
`0d03f7e3-19ea-42a8-9f65-b5c57afbae03` passes **1,047 tests, zero skips**, using
the root target, locked/offline dependencies, one build job and one test thread.
Its raw log is
`target/review/convergence/parser-compat-retirement-operation-runtime-1.log`.
The six subsequent affected gates all pass:

| Gate | Result |
| --- | --- |
| Operation doctests | 128 passed: 32 positive examples and 96 compile-fail cases |
| Workspace strict Clippy | All targets/all features, warnings denied; passed |
| Independent asset consumer | 138 passed; one existing skip; run `e036c56c-998f-4005-bad2-1e9fc5fc07a9` |
| Asset-consumer strict Clippy | All targets/all features, warnings denied; passed |
| Workspace and consumer formatting | Both passed |

Execution `59110` actually exits zero for Operation runtime; `47677` actually exits
zero for the six subsequent gates. The latter receipt is
`target/review/convergence/parser-compat-retirement-qa-1.json`, SHA-256
`45734fc2a9946531f48a2eda88c90705ca78798425fa4ac1cee6a759cba07529`.
Its 1,452 input hashes remain unchanged through those gates. The runtime log
SHA-256 is `60a634707501eb9c513e6cf83011b290f492ed8ce43afff3a8570f159b69dee5`.

Independent final evidence review verifies the actual source/map/log hashes,
terminal results and test counts, unchanged kernel/real host, and fresh root
dep-info/binary association without a concrete finding. The review is retained as
`parser-compat-retirement-final-evidence-review-1.md` under the review directory.
These observations are bounded source/evidence association, not a compiler
attestation or a separately executed test run. The preceding 3,566-test full
workspace result belongs to `9fd7a91`; it is not assigned to this later parser cut.

The independent implementation/source reports remain under
`.git/baozi-review/performance-convergence/` as
`parser-compat-retirement-implementation-1.md` and
`parser-compat-retirement-source-review-{1,2}.md`.

## Remaining scope

This prerequisite deletion has no new ordinary performance or heap measurement.
The [convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md)
remains active and release remains HOLD. Connected retained Internal/protected/share
borrowing, full-ledger alias removal, fixed/dynamic backing, shared promotion,
exact accounting and indexing remain required. Final Windows/Linux 14-case/110-lane
time/whole-child memory and separate heap acceptance remain open.
