# Bound Execution and Physical Prefix Foundation

Implementation: `46eb6c27b4f272d956eb5ae4889a0470eec3a26c`.
This is the execution, normal-kernel and text-completion foundation of P3 in the
[architecture continuation](../plans/2026-11-02-1700-perf-import-operation-architecture-plan.md).
It does not complete P3's import migration, P4-P6 or the original pre-release goal.

## Implemented behavior

The existing invocation-local work owner now lends separate concrete work, storage and control
views. Repeated postprocess actions use one bound numeric group without re-resolving keys,
sorting groups, cloning work ownership or taking the shared storage mutex. The cursor retains
only control cadence debt: geometry checks at most 256 bounded completed actions apart, with
explicit entry, allocation, semantic and final checkpoints. Admission remains per action and
actual completion is immediate. Removed the obsolete key argument from the action helper and
the forwarding arguments that existed only for it.

Parser parent/child channels use those same work rows and numeric kernel. They preserve ordered
attempted/item-limit checks, atomic direct completion and attempted-only admission without
inventing consumed envelopes. Zero-sized and fully completed channels remain outstanding until
closed; forgetting them prevents successful finalization. Ignored admission, completion and
incomplete-finish errors latch the existing failure selector. Storage rejects a foreign
invocation's control before either polling it or admitting bytes.

Normal generation now admits and completes actual bounded tuple, sparse-comparison, arithmetic,
sorting and output leaves. There is no outer producer held across nested reads. Semantic-error
returns retain completed work; unwind cannot fabricate completion. The numerical algorithms,
precision and deterministic ordering remain covered by existing and new output tests. Tangent
reads likewise account their actual sparse comparisons and tuple access instead of completing
an estimated number of empty actions. The obsolete synthetic-overflow tangent error was removed.

Text planning, building and record discovery record each classified raw prefix immediately.
`WorkCursor` retains only the 4,096-byte frontier; processed/completed offsets and flush replay
are removed. BOM recognition, malformed/replacement prefixes and CRLF/lookahead preserve their
declared raw units. Copy/index/length failures observe the work already performed without credit
for an unclassified suffix. The new `a LF b` forced-copy failure retains attempted 6 / consumed 5
across planning and building.

The current Scene activation transition releases its state-slot borrow before activation,
validation, callbacks and capture destruction. Installation uses a short borrow; conflicting
or foreign state preserves the incoming complete owner in physical containment. This prepares
the remaining exclusive-context cutover without moving work into shared ownership.

## Verification and source qualification

Windows 11, Rust 1.95.0, serialized Cargo with one build job and shared `target`:

| Check | Result / retained run |
| --- | --- |
| Operation and import, all features | 1,290/1,290; `05766185-b6a5-4f35-8bf9-0c728baaa1b2` |
| Added unclassified-suffix fixture | 1/1; `1949afe8-170d-4a71-a927-3b9938a81a50` |
| Four format crates and postprocess, all features | 1,056/1,056; two explicit manual diagnostics ignored; `4590e751-4e07-4b80-ba61-d27e9a86465a` |
| Postprocess after unused-argument cleanup | 249/249; the same two manual diagnostics ignored; `3ae37602-3c8b-4ae3-a8a5-2be61a0049f2` |
| Independent comparison consumer, ordinary features | 10/10; `07f81d6c-2bb7-4aba-8b5b-cd7e13b39f93` |
| Seven affected packages, strict Clippy, all targets/features | Passed |
| Operation/import/postprocess doctests | 52/52: 37 operation, 15 import |
| Those three packages' docs, warnings denied | Passed |
| Workspace formatting and diff whitespace | Passed |

The 1,290-test run preceded the additional text fixture; its fresh targeted run checks that
test-only addition. The 1,056-test run preceded removal of unused private forwarding parameters;
the fresh 249-test postprocess run and comparison consumer qualify that cleanup. The four format
crates' production source did not change between these runs. Strict Clippy, docs, doctests and
format checks follow the final code edits. Counts are separate runs, not a sum of unique tests.

New integration coverage includes control boundaries 0/1/255/256/257, between-poll resource
rejection, final short regions, ignored parser failures, forgotten zero/full channels, and real
storage/progress inside active work. Four real normal-adapter tests execute 68 bounded operations:
literal dense/sparse/sort/output prefixes, final 24-action cancellation, physical cleanup and
unchanged input generation. A caller-owned output test checks its written prefix and untouched
sentinel suffix. The adapter's provisional buffer is destroyed on failure and is not claimed
to have been inspected after destruction.

Independent bounded execution and text source reviews reported no actionable correctness finding.
Their reported coverage gaps were addressed by ignored-completion/finish tests, real normal
integration and the unclassified text suffix. Reviews did not run builds or tests; the main thread
ran the checks above. The final private-argument cleanup was main-reviewed and regression-tested.

## Remaining architecture work

The selected import still uses the old whole-owner host and ParseScope projection; its raw
parser channels are not yet the production parser interface. The
[accepted typed-context design](../research/2026-11-02-typed-format-context-design.md) combines
that migration with P4: exclusive context, explicit execution callbacks and independently
session-branded containers. Independent feasibility review found no blocker; actual context,
container and complex borrowed-input compilation is the next entry gate.

Canonical semantic helpers still need their once-bound caller audit, and the polygon kernel's
legacy empty control hook still needs actual action boundaries. The foundation does not claim
that every preparation path has completed P3. Complete these obligations before the integrated
P3 performance acceptance. P5 attribution, full Windows/Linux B1 and separate heap observations,
final U14, control-assurance comparison and final independent review remain open. Native IO stays
CandidateUnregistered. No publishing, registry yank, push, PR or target promotion follows.

Local verification manifest: `target/review/p3-foundation-verification.json`.
Retained evidence archive: `target/review/p3-foundation-evidence.zip`, SHA-256
`9110e838e02fb74b170184c4c003f259e15b0b3095749ce51092a5d3df0b3a7c`.
