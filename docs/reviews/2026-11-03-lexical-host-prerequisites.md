# Lexical Host Prerequisite Qualification

Date: 2026-11-03. This records an **isolated implementation**, based on main source
`0484dcb581869afef106f43721ffbb1589e1b631`, for the
[lexical invocation host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
The Rust changes below are not applied to main. The complete lexical-root/permission-host,
current-state attachment and promotion design remains in progress. No ordinary performance
measurement or optimization keep follows from this checkpoint.

## Connected Prerequisites

The actual `RetainedOperation` coordinator, first cause, control debt, execution state and
canonical lineage are grouped in `OperationInvocation`. Real ordinary/exclusive execution,
publication/terminal consumers and tests use that common field; no counters are replayed or
coordinator reconstructed. This is a field-owner extraction: `RetainedOperation` still owns
common by value. Its final outer-host lifetime and stage separation are not implemented.

Actual postprocess preparation and space conversion now split scene-edit validation from
consuming finish. `validate_changed` returns an owned pending central/rejected/never-activated
alternative after correspondence loans end and local `PreparedEdit` is destroyed. The selected
caller then destroys replacements and prepared/generated scratch before `finish_changed`.
Real central `PendingMinimalFallback`, failure ownership, source generation and branch-specific
control behavior remain. This by-value pending owner is an implementation prerequisite, not
the final forget-resistant permission API.

The unused activation `Fallback` alternative and two consequently unreachable
`PreparedFallback` wrappers are removed along with their dead matches. Real deferred central
fallback and its tests remain. Independent review distinguishes the direct field-drop ordering
change and confirms actual value-before-attribution cleanup in the inspected prerequisite.
It does not certify the later root-bearing containment design.

## Actual Cleanup Oracles

Six new private unit tests run the production `prepare_triangles` Selection path, with the
existing imported square and nonempty generated normals, corner origins, mappings, replacements
and prepared storage. Test-only scalar observations follow actual storage release/local owner
destruction and precede real consuming terminal calls. Events also identify the real handed-off
ledger. No public hook, dependency test feature, synthetic pending owner, budget search, added
production poll or numerical/accounting change selects the alternatives.

| Actual reached alternative | Selector and decisive checks |
| --- | --- |
| Candidate | Normal execution; real scratch destruction precedes publication preparation, final Publication progress and success ledger. Output bits, generation-bound remaps/report and detached thread transfer remain valid. |
| Activation Rejected | Existing cancellation signal becomes active inside the actual materializer callback; no evidence/validation stage follows. |
| NeverActivated | Signal becomes active after the real selected plan and semantic completion, before activation sealing; materialization is never entered. |
| CentralRejected | Existing Validation progress observer requests cancellation before the validator callback. Public cause is exactly the existing `FinalControl` projection; terminal ledger remains Control/code 1. |
| Interrupted | Signal becomes active at actual validation-closure entry; real validation scratch admission returns verified Control. Public cancellation retains Signal origin. |
| PendingMinimalFallback | Signal becomes active at actual evidence-builder entry; real evidence admission fails through `defer_assembly_minimal` before Validation progress. |

Every failure retains original input generation, positions and indices, no generated corner
normal field in the original input, the actual independent fallback owner, Control/code 1,
two-record terminal evidence, zero committed canonical/outcome bytes and zero final live
reservations. Explicit Signal cancellation is required in the four signal cases; it is not
replaced by a broad Control-or-FinalControl assertion. End balances supplement the observed
destruction order, rather than proving it alone. These tests compare generated output with
captured real scratch; they do not introduce a new independent normal/triangulation oracle.

## Serialized Windows Qualification

Rust 1.95.0, locked/offline nextest/Clippy, one build job and the repository target. Main alone
executes Cargo. Compiler/test source identities are retained before and after the named commands.

| Isolated source checkpoint | Actual result |
| --- | --- |
| Formatted checkpoint 2, complete 1,402-file set `9acb18c7…` | Workspace/all-target/all-feature check passed with no warnings; operation + postprocess nextest **1,201 passed**, two existing skips, `bf48d325-b01c-48af-80d0-42927f6a0b91`; strict four-package affected Clippy and fmt passed. |
| Formatted checkpoint 5, complete 1,403-file set `5999da70…` | Focused six cleanup tests **6 passed**, `2252715c-1d62-405a-9982-32609e2f5cae`; 161 unselected filter skips are not a full-suite result. Complete postprocess nextest **295 passed**, two existing skips, `12be4754-eed4-4d95-9dd6-6966a4dbcedc`; strict postprocess all-target/all-feature Clippy and workspace fmt passed. |

The operation/core/import changes are identical between these checkpoints; the incremental
cleanup-test/probe changes are postprocess-only. Do not add these test counts into a single
same-source campaign, or treat the earlier workspace check as a new checkpoint-5 check.
The three source-qualified glTF direction files remain byte-identical to main `0484dcb`.
There is no new full core/import/format runtime matrix, Linux run, doctest qualification,
ordinary binary, heap attribution or release claim in this record.

Preserved failures: initial workspace check rejected 14 missed test field projections and
identified two dead wrappers; checkpoint 3 incorrectly expected Interrupted for observer
cancellation (five selected tests: four passed, one failed);
checkpoint 4 added genuine Interrupted but still incorrectly expected explicit Observer cause
from CentralRejected (five passed, one failed). Source tracing established the pre-existing
`FinalControl` projection; checkpoint 5 corrects only that oracle. Production control policy
was not changed to satisfy tests. All failed raw logs and source snapshots remain retained.

## Frozen Evidence and Remaining Work

Local source/review snapshots are under `.git/baozi-review/performance-convergence/` and
execution receipts under `target/review/convergence/`:

- `lexical-root-checkpoint-2-qualified-source-1.json`, SHA-256
  `98a9d9aa71ab64d4cd23b761dd9e9113ddff080d7a475f673a0a4644f22e8559`,
  pins 21 formatted source changes; qualification aggregate `bbe8e62e…`.
- `lexical-root-checkpoint-5-qualified-source-1.json`, SHA-256
  `652632288a91280227505855054be2e44fb8fb656cc3024e3c3e59313031251f`,
  pins 24 formatted source changes, including one new test module; qualification aggregate
  `eaa3fcb4e4e93ff846eb23ba0256c2fb2a125b8f22d99d6907a261ef9439115e`.

The 24-file packet plus its manifest has 25 artifacts; it does not contain 25 changed source
files. Earlier main progress wording is corrected. Complete source identities use sorted UTF-8
`path + NUL + lowercase file SHA-256 + LF`. Fixed copies preserve every reviewed/qualified
checkpoint while the isolated implementation continues.

Next: move actual common and retained/assembly/candidate state into the outer host, introduce
the real short permission flow and lexical fixed/dynamic roots, implement current-state
attachment and shell-before-state-move promotion, preserve independently released storage,
then delete the remaining `LocalWork`/owning permit/parser/startup/whole-owner containment
paths when their actual consumers migrate. Full ownership/panic/forget/compiler/accounting
gates, independent integrated review and unchanged Windows/Linux 110-lane performance targets
remain required before applying or retaining the complete cut.
