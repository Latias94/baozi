# Trusted Parser Scan Close

This isolated Operation change supplies a required mechanism for the connected
[Import borrowing refactor](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
Actual Import text, PLY and full-service migration remain open. The preceding measured
runtime and its [Windows performance HOLD](2026-11-04-lexical-host-performance-checkpoint.md)
are separate source identities; this mechanism is not a newly measured optimization.

## Real Motivation and Shared Owner

The existing text record scanner returns Boundary, PendingCr and conformance handbacks
through sentinel errors to release unused admission without declaring parsing failure.
Its old host preserves the physical prefix and continues. The replacement borrowed
parser normally classifies callback errors as KnownError and ordinary incomplete success
as IncompleteSuccess. Migrating the old sentinel return literally would stop valid short
records before the following row.

`StoreScanReturn::{Exhausted, Boundary}` expresses that distinction at the trusted host.
Root `parser_scan_child` and parent `scan_child` use the same actual `ParserHost`, frame,
admission, prefix kernel and close implementation as ordinary parser work. Exhausted
remains exact. Boundary resolves only the genuine unused suffix without consuming it,
marking KnownError or adding successful Import-channel debt. It still checks the real
running state after close; an earlier error or caught physical panic cannot be healed.
Ordinary parent/child callbacks wrap their results in Exhausted and retain exactness.

The child role cannot admit another scan. Public compiler examples pair the permitted
parent operation with the forbidden child operation using the same exported types and
real method. No counter kernel, provider backend, owning client permit or public
partial-finish method on a child is introduced. The eventual Import scan adapter must
stay private to trusted text and preserve its own prescribed exit poll.

## Runtime Oracles

The first four direct tests use an actual retained operation, unique invocation and
physical account. A parent admits six bytes, classifies the actual `a\nrest` boundary
after two, admits the following body child, completes the parent and then runs another
ordinary child. Literal final work is `(10, 6, 0)`, with correct intermediate prefixes,
quiescence, three polls and healthy final retained-storage release.

An active Import boundary then runs 257 normally returned newline boundaries. Even a
fully consumed Boundary does not add exhausted-channel debt. A following ordinary
child adds exactly one. Another test keeps both typed Exhausted and ordinary Ok strict
when a suffix remains; neither permits subsequent work. A caught direct physical panic
also cannot return healthy Boundary: its completed prefix and abandonment cause survive.

Independent source review finds no demonstrated defect and identifies two direct branch
coverage gaps. The test-only successor appends two regressions without changing any
production, compiler-contract or format source or any existing test bytes:

- After 256 actual ordinary Import channels, an actual `ab` scan reaches its due entry
  poll, consumes both classifications and returns Exhausted. It adds exactly one channel
  debt unit, permits the next child and retains `(259, 259, 0)` and three total polls.
- An active Import boundary defers a caught physical panic as UnwindPending with no
  premature abandonment record. Returning Boundary settles that fault, retains `(2, 1, 0)`
  and quiescence, and denies the next physical callback without new attempted work.

These are real host branches, not assignments to the failure slot or mock counters.
The future actual text integration still needs PendingCr/BOM/violation handback and its
exit-poll/cancellation evidence. Passing unchanged PLY tests does not prove that PLY
already uses the new scan entry.

## Exact Sources and Qualification

| Source | Actual evidence |
| --- | --- |
| Source1, 1,415 files; set `0bb15a645f4494f24b7195d62c966cde1a1da9e007a3b59133670a0c5b4829c6` | Four focused tests pass; 142 Operation/Import doctests pass; all 2,317 Operation/Import/glTF/OBJ/PLY/STL runtime tests pass with zero skips; strict workspace all-target/all-feature Clippy and root format check pass. |
| Source2, 1,415 files; set `446b220510f6c3fff4ed11b83521f5dc9048a4bf73ee7f23c894e4daff5bffb5` | Only the appended private test file differs. Six focused scan tests pass; Operation all-target/all-feature strict Clippy and root format check pass. This is affected successor evidence, not a second complete 2,317-test execution. |

Source1 focused run is `86958961-8968-4efc-b32d-5b8950758ee3`; its 588 other library
tests are filtered. Its complete selected-package run is
`7b01828d-7214-4510-8947-406712bdce15`. Source2 focused run is
`19ffcba2-fa47-45ea-b5f2-b7b5bfd77fc7`, also with 588 filtered tests. No new manual
skips or predecessor PASS substitution is used.

Source manifests, serial QA and terminal associations are under
`target/review/convergence/`, using the prefix `trusted-parser-scan-close-` and suffixes
`source-1.json`, `qa-1.json`, `terminal-association-1.json`, and their source2 equivalents.
Frozen sources and independent reviews remain under
`.git/baozi-review/performance-convergence/`. Main re-enumerates both exact 1,415-file
live/frozen sets at each terminal checkpoint and hashes every log. Source1's ten
qualification maps and source2's six maps all match their own frozen sources. In both
receipts the preparatory formatter changes its input; its after map matches the frozen
candidate. The pre-format input is not mislabeled as that candidate.

The original independent report is `trusted-parser-scan-close-review-1.md`; it preserves
the two original coverage gaps and its source1 terminal evidence. The subsequent
`trusted-parser-scan-close-final-evidence-review-1.md` verifies both actual branch tests,
closes both gaps without a finding and independently checks both full frozen sources,
the test-only delta, all ten logs and exact before/after map associations. It preserves
the two qualification scopes and both preparatory formatter exceptions. No complete
workspace runtime, external asset-consumer, actual full Import migration, main Rust
application or performance qualification is claimed by this bounded mechanism.
