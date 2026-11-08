# Borrow the actual accounting row during text emission

Status: design reviewed; fixed finite-consumer heap qualification completed;
implementation pending. Release and local-main runtime integration remain HOLD under
the [convergence plan](2026-11-03-0900-perf-assimp-convergence-plan.md).

## Problem and selected change

The [finite-consumer diagnostic checkpoint](../reviews/2026-11-08-finite-consumer-attribution.md)
records no instruction reduction from the Core/Postprocess ownership cut.
In grid OBJ128, the text emission caller invokes `TextEmission::read` and
generic `ParserFrame::complete`1,830,715 times each. This equals that ASCII
fixture's input length. Those edges identify repeated setup to investigate;
neither their inclusive costs nor namespace/source Self shares predict savings.

Planning already uses a private actual singleton-row loan. Reuse that loan
for the emission pass between control frontiers. Keep the original numeric
row, frame remainder, child admission, failure cells, output owners and host
unwind guard. Check stable channel/layout identity at entry and commit each
literal returned read immediately. Preserve numerical consumed/pending/
attempted/limit checks and the original two-pass physical work.

This changes the private Import emission caller. It needs no public API,
provider/backing hierarchy, replacement owner, copied counters, deferred
counter reconciliation, generalized multi-row executor or new digest.

| Alternative | Effect | Decision |
| --- | --- | --- |
| Inline only the generic prefix runner | Keeps general callback/storage protocol and repeated stable checks | Do not select without contrary measurements |
| Merge planning and emission | Changes reservation and denial ordering and physical work | Outside this experiment |
| Generalize glTF accessor rows | Requires separate four-lane atomicity and API design | Later independent experiment |
| Reuse the existing actual singleton row | Retains original guard and per-unit numerical semantics | Selected |

## Control and ownership ordering

Keep output reservation, raw-range construction, child admission, BOM
revalidation/completion and initial line-index insertion in their original
positions. The raw frontier starts at4096, including BOM bytes in offsets.

Within each loan:

1. Perform the same bounded width lookahead before classification. If the
   next UTF-8/newline/replacement unit crosses the raw frontier, return to the
   original host without classifying it.
2. Check row readiness before reading a unit. If readiness requires a debt
   checkpoint, return to the host. Loan entry never clears action debt.
3. Call the unchanged `TextEmission::read`, then immediately complete its
   literal raw width on the actual row before arithmetic or output writes.
   Attempted admission does not prove a pending-consumption exemption.
4. Execute the original valid/newline/replacement branches and reserved
   output operations in the same order. Preserve range, identity, UTF-8,
   capacity, length, copy-ceiling and first-cause checks. On error, stop
   before another classification.
5. Update the original offsets and Building test hook after the unit's
   actual writes. Reaching a raw frontier returns to the host even at EOF.

After a raw exit, poll the original host and advance the raw frontier only
after poll success. A debt-only poll must leave that frontier unchanged.
End the row loan before every host/control/services/error-materialization
boundary. Keep the existing armed guard through the whole region. Do not
flush output merely because a row loan ends. Flush the final valid suffix
in its original position, then retain exact finish and index-count checks.

Reserved string/vector writes do not poll, invoke user callbacks or grow
storage. Keep their actual-owner checks. Preserve output owner drop order;
any small cursor groups only offsets and the existing WorkCursor. Remove
the retired per-unit emission runner path and unused WorkCursor polling
wrappers after verifying remaining callers. Retain generic parser services
still used elsewhere. Do not introduce a general region framework.

## Reachable modes and required tests

Actual Import root/parser entry always supplies `Some(importer_boundary)`:
`scope/parser_execution.rs` delegates through `ImportStoreSessionOwner`,
whose dormant outcome owner requires that boundary. Direct ParseContext
calls have the same mode. The emission caller cannot currently enter
Operation's `None` mode, where direct per-unit action debt is enabled.

Test the changed caller with its actual original owner and reservations.
Keep unchanged real planning/building frontier tests for ASCII4096, a
four-byte unit starting4095, later malformed bytes, literal accounting and
cleanup. Preserve existing BOM/CR/LF/CRLF, Unicode/replacement ranges, empty
input, terminal newline, limits and allocation/denial tests. The existing
reserved-copy failure test already checks exact successful prefixes and
the original `decoded-text-length-exhausted` cause; reuse it.

Add one private-probe panic oracle inside actual emission after a committed
unit and actual output writes, with an unread suffix. For `a\nb\n` stopped
at Building offset2, DecodeWork is attempted8/consumed6 before settlement:
four planning bytes and two completed emission bytes. Verify the original
first cause after caught unwind, no accepted later physical work/success,
and actual output/account cleanup. A panic after emission returns does not
exercise this guard. Reuse the existing private probe and production path.

Keep readiness and distinct debt/raw exits despite today's reachability.
The existing actual Operation direct-row handback,255-to256 cancellation,
numeric failure and unwind tests remain unchanged. Do not add a public/test
feature or substitute execution owner solely to manufacture a direct Import
mode. If a future change exposes `None` for this caller, that change must add
real combined debt/raw-frontier emission tests, including continued work
after a successful debt-only checkpoint. No such coverage is claimed here.

## Measured acceptance and execution

Finish independent raw/stack/sanity qualification of all18 fixed
finite-consumer heap workers before production edits. Preserve ordinary
partial campaigns, every instruction regression, failed calibration1 and
its correction. No prior failure is overwritten or reclassified.

Freeze the actual implementation. Run affected Import/text tests, retained
Operation row tests as appropriate, docs, strict all-target/all-feature
Clippy and fmt. Then complete the original seven workspace/independent
consumer gates using Rust1.95, Cargo jobs1 and nextest threads1. Builds stay
in this repository and existing target. Retain failed numbered attempts.
Independently review source/control/error changes and the new panic oracle.

The diagnostic selection predicate is fixed before implementation: matched
generic CPU/System allocator opt3/debug1 workers on the same six original
pipeline/memory cases must preserve complete non-clock JSON, numeric
spelling/types, work and output. At least one OBJ whole-worker instruction
count must decrease >=1%; no one of six may increase >1%. This selects a
diagnostic experiment and supplies no native speed or release acceptance.

Use the unchanged14-case/110-lane Windows/GNU ordinary protocol, three
warmup pairs plus twenty retained fresh AB/BA pairs, for native/source
disposition. Preserve missing campaigns, load records and every adverse
sign. This loop intends no allocation change; compare separately qualified
heap data if the implementation affects allocation behavior. Commit the
qualified source with a scoped Conventional Commit. Main/release disposition
continues to require the active convergence evidence.

The bounded design review and its reachability amendment are closed in
`closed-text-emission-plan-review-1.md` and
`closed-text-emission-reachability-review-1.md` under the private review
directory. Their approval covers this seam and reachable test contract,
not implementation or performance results.
