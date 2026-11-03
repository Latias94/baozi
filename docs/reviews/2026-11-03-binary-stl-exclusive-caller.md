# Binary STL Decode Through the Exclusive Owner

The [qualified import bridge](2026-11-03-exclusive-import-bridge.md) provides a real retained
owner and branded parser roles. This slice migrates one complete production caller: binary
STL decode, including facet/color processing and name/mesh packaging. It also moves ordinary
vector/string growth to one owner shared by the replacement and remaining legacy callers.
Qualified source is `48d6f17e2f0829bdd5d50c8d7c48cea72ce343a1`. No performance keep is
inferred from the source qualification.

## One Decoder and One Ordinary Storage Owner

Content-based exact binary dispatch remains before entry selection. Binary decode uses one
`with_parser_execution` region and returns the existing session-owned `ParsedStl`; ASCII
retains its text path. Both reach the same authoring/submission owner after the decode borrow
ends. The old binary `ParseExecution`/work-scope calls and execution-taking scalar/range helpers
are removed. There is no second decoder, selector flag or compatibility backend.

The existing checked arithmetic and exact `84 + 50 * F` byte length establish complete fixed
records while input is immutably borrowed. Each facet uses the same little-endian F32 decoding,
preserving authored normals, signed zero and nonfinite bits. Nonfinite input still reaches the
existing canonical validation error. No normal generation, vector normalization, welding,
precision change or early finite filter is introduced. Color channel order, fallback alpha,
separate expansion, name allocation and mesh ownership retain their actual behavior.

Ordinary storage loans contain the existing storage-only release view, a short borrow of the
actual poll owner, immutable vocabulary/domain and references to the existing failure/debt/state.
They hold no numeric WorkView, parser frame, owning permit, whole Ledger, progress Vec or new
account. The actual issuer remains in `StorageIdentity`; a temporary loan cannot escape or
overlap root/parser reentry, while a metered container can survive the region and grow later in
the same session. Mixed sibling owners and foreign sessions remain rejected.

Vector reserve, string reserve and bounded text copying have one implementation. Legacy
entrypoints forward to that owner while real ASCII/text/other-format callers remain. Capacity
fit still checks identity and polls. Growth retains original/replacement overlap, checked plans,
actual-capacity reconciliation, allocation/qualification failure, UTF-8-safe 4 KiB copies and
the final poll before replacement commit. Payload destruction precedes its capacity token.
Ordinary logical live tokens do not become retained physical leases or acquire Internal share
work. Retained reservation/share and failure-materializer roles remain distinct actual services.

Growing push and string append use the same host-owned prefix executor and frame guard.
Only disjoint storage/poll fields are lent during a physical callback. The caller retains
uninvoked callback/value slots until runtime failure is projected; returned payloads also stay
alive until completion/control failure is projected. Actual import cause latching therefore
precedes user-owned Drop. Verified prefixes, normal semantic error, caught unwind and prescribed
exit polling keep their different completion rules. No second runner or general error policy
framework supplies those rules.

## Literal Semantic, Control and Ownership Oracles

Inherited tests retain binary/ASCII scalar bits, authored normals, color composition, arbitrary
and `solid` headers, zero facets, canonical NaN errors, detached ownership and the existing
257-facet/color cancellation hooks. Additional tests use the actual owners and public importer:

- Two facets attempt `[184,2,6,12,2]` across DecodeWork/Faces/Vertices/AttributeCells/Records.
  Cancellation after one physical facet retains consumed `[134,1,3,6,1]`, exactly three
  positions/normals, no second facet or mesh construction, and the exact Signal cause.
- DecodeWork 184 succeeds; 183 rejects the attempted 184 envelope before payload work.
  Faces=0 retains its precise earlier DecodeWork/Faces attempted prefix, no later lane or
  consumption, and no canonical payload peak. No allowance increases replace these assertions.
- Real compound writes return containers across a region, followed by same-session legacy
  growth. Paired sessions test foreign capacity fit and growth, preserving source payload,
  capacity/accounting and sender authority while stopping the destination.
- Actual allocation/qualification faults preserve old payload/capacity/token and a verified
  prefix. A configured live-byte limit admits replacement alone but rejects old-plus-new
  overlap with the exact Ledger Live cause.
- New-text append cancellation retains an actual 4096-byte appended prefix and no completion
  of its failed piece. Separate forced-growth old-string cancellation preserves the entire
  original payload/pointer/capacity/token. It checks four real polls without claiming an
  observable prefix in discarded replacement scratch.
- Real registered-importer value/capture destructor panics follow denied admission with the
  original overrun cause and no later callback/submission. Separate allocation failure is
  observed inside the unused value's Drop before its deliberate panic.
- A real asymmetric Tokens policy `(attempted=8, consumed=1)` permits child admission `[2]`.
  The second physical piece returns a payload, then completion rejects Consumed/Tokens/2/1.
  Its destructor must see that exact actual import terminal cause before counting its Drop;
  the outer catch matches the intentional panic sentinel. Physical writes, `(2,1,0)`, one
  Drop, denied reuse and cleanup exclude an unrelated earlier panic as a passing result.
- The private actual guard probe checks Running immediately after rejected completion cleanup.
  Existing Ledger precedence alone would not detect a spuriously armed Drop selecting
  UnwindPending. This strengthens that probe without adding another fixture or shadow ledger.

Three new storage-loan compiler negatives fail for their intended escape lifetime or E0499
reentry reason, not missing APIs. The owned-container and sequential-loan positives compile.
Affected import 30 and operation 54 doctests and strict all-target Clippy pass in the isolated
source. These compiler results remain separate from actual main qualification.

## Retained Failures and Artifact Association

The first compiler checkpoint exposed a control argument whose inner lifetime was tied too
tightly to the storage lender; independent short reborrows fix it while preserving the actual
sibling identity check. Later lint corrections use a short-circuit let-chain and lexical loan
blocks; no error allocation, new destructor or lint suppression is added. One new test closure
needed an explicit `ParseFailure` error type. Original failed checks remain retained.

Shared-target qualification also exposed an actual artifact/source mismatch. `cargo check`
used the new source, but test compilation reused an older import `.rlib`; its raw dep-info
pointed to the prior bridge compiler workspace. Targeted fingerprint invalidation fixed that
library association. A later expected-three test selection ran only two: an old operation
test-lib binary omitted the new private probe. That partial run is not its qualification.
After the two confirmed old test fingerprints were separately invalidated, all three named
gap tests actually ran and passed, run `4893c6a8-892b-4c35-92a6-e7ce211dbdc0`.

The earlier isolated 1454-pass result thus does not qualify the omitted private probe. The
later exact-three result supplies that evidence, alongside the two real import oracles.
All failed/partial receipts, old fingerprint records and the independent forensic review remain.
This observed association failure is not a claim of a general Cargo defect. Present source
hashes alone do not substitute for checking the tested artifact's source association.

## Final Source and Actual Main Gate

Runtime base is `67b5ca609470268b124fd1915741ee81105c4a24`; the complete compiler archive is
documentation head `37e800541be6cae31585e40c01828aa64b5809ef`. The frozen final 23-path packet
has full-patch SHA-256 `f8c8a12407eb7b463c88ff8d8ae12db7489b4cf0337bd4da4bd85ba08769e99a`
and NUL-separated source-set SHA-256
`7f58f1b87bbd52f9a512ddd9912bf34aa529b2311e803206f84684734ec28d36`.
It changes 22 files; `metered_storage.rs` is an unchanged ownership witness. Independent
production, lifetime/lint/format, STL and focused-oracle reviews close with no remaining P1/P2.

Main applies exactly those reviewed changes and formatting changes no bytes. Before actual
qualification, confirmed library and test fingerprints associated with the isolated compiler
workspace are archived to require builds from main. Source, `.rlib` and test-binary files are
not deleted. Commands retain all 23 source identities before and after each check.

Actual main qualification passes on that exact committed source:

| Check | Actual result |
| --- | --- |
| Operation/core/import/postprocess nextest | 2355/2355, six existing skips; `452ddf4a-2c6d-4192-a681-ee23ee7adc82` |
| glTF/OBJ/PLY/STL nextest | 855/855, no skips; `3b5409c3-4a45-475e-80d3-4db6f05e77dc` |
| Facade control assurance | 9/9, no skips; `dddd1c6a-87fc-47e3-b5ca-048d35e46ea5` |
| Independent asset consumer, all features | 138/138, one existing manual skip; `85033cf1-9080-46b4-ba88-581631fc323e` |
| Import/operation doctests | 30 + 54 passed |
| Affected + STL and independent consumer strict Clippy | Both pass, all targets/features, `-D warnings` |
| Affected formatting | Passed, no byte changes |

All eight checks retain identical 23-path before/after identities and no outside-scope mutation.
Aggregate receipt `target/review/convergence/binary-stl-main-qualification-1.json` has SHA-256
`eb2905ebe03b7b88627dec4af05271374ea42b03c5ee3edbd9f478836cfbb4c3`.
`binary-stl-source-commit-1.json` ties all 23 committed Git blobs to those qualified bytes.

The final main nextest JSON inventory lists 1457 operation/import/STL tests. All 11 critical
new test names are present, unignored and selected; their actual binary paths, hashes and raw
dep-info records associate them with main, without the prior compiler-workspace paths.
`binary-stl-main-artifact-association-1.json` records that bounded check, SHA-256
`37f79a52782ba8e15ee2d23353ca39187b8ee00c1d7cfc70cece68470d81afb7`.
Inventory is not another full runtime campaign; the completed suites supply runtime evidence.
Individual commands/logs remain under `target/review/convergence/binary-stl-main-*`.
These are Windows correctness/compiler results; the new caller has no Linux runtime or native
performance qualification in this receipt yet.

## Performance and Remaining Cutover

The ordinary Windows control is retained from runtime `67b5ca6`/documentation head `37e8005`,
binary SHA-256 `a3ec752a0060f5db24d5ba5a524645051ab07effe2e0145bf7adbe64907c806c`.
No ordinary candidate or paired campaign is qualified by this document yet. The frozen task,
selected detached fields, allocation/cleanup boundaries and explicit Smooth workload remain.
Normal mathematics and source-format normal intent do not change to manufacture a speedup.

ASCII/shared text, PLY/OBJ/glTF parser callers, typed authoring and postprocess remain real
later migrations. Roots, administrative metrics, consuming attachment, indexed unchanged
selection and owned geometry payload v3 remain separate. Previous timing/memory costs and the
parent Windows/Linux 14-case/110-lane 3x gate remain open. This caller/source proof alone does
not establish release convergence or fix the separate glTF FLOAT direction-admission P2.
