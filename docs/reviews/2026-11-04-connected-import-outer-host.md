# Connected Import Outer Host

Both byte import and asset/resolver import now use the retained outer body from the
[lexical invocation-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
The body requests an outcome, releases its session cell and temporary loans, and returns unit.
Final progress, generation and publication run after callback capture disposal. This isolated
successor is not applied to main Rust and does not establish performance convergence.

## Actual Ownership and Removed Paths

One `run_import_session` serves both physical entry paths. `ImportSessionData` is owned outside
the retained callback and contains the source, snapshots, dependency graph, diagnostics, report
staging, successful manifest, original failure cause and its storage reservation. The short
`ImportSession` borrows these actual data, plan, backend and resolver. No new `Clone`, `Copy`,
`Send`, `Sync` or `'static` requirement is imposed on parser results or evidence producers.

The same accounted session cell is popped and its backing/reservation released before the
actual pending owner requests a disposition. `ImportStoreFinishRequest` retains owned recipe,
parser outcome, vocabulary and completed-boundary facts. It does not retain a permission
borrowing its own host. Dormant/no-submission import requests the actual never-activated
failure route, preserving rich failure construction after body cleanup.

The old session `finalize_success`/`finalize_failure`, session `prepare_scene`/`finish_scene`
chain and unused cell generation/fallback slots are removed. Private final dispatch restores
the same host-owned candidate, activation rejection, never-activated, central rejection,
interruption or fallback stage. Parser Accepted/Rejected still controls candidate publication;
candidate presence alone does not imply successful import.

Importer/parser exit polling, first-cause precedence, required-dependency context and its
reservation, borrowed format-failure evidence producers, and original-import versus terminal
failure authority remain separate from the final checkpoint. A new actual capture-release
test checks scratch disposal before candidate final progress. Other captured-scratch/stage
combinations do not each have a new dedicated runtime assertion.

## Frozen Source and Verification

The complete source contains 1,409 files. Its only difference from the independently reviewed
Operation/Postprocess source2 is six Import files: `session.rs`, `store.rs`,
`store/scene_activation.rs`, and their three affected test files. Failed compiler/lint attempts
are retained; the final repairs remove obsolete fields, retain the same externally owned
physical-account fixture, and correct test loans without changing the assertions or limits.

| Artifact | SHA-256 |
| --- | --- |
| Complete source set, sorted UTF-8 path + NUL + lowercase hash + LF | `472d5abb9b5ef5a63d6103faecf2c060fae15e5bf2d26d2134275c8caff4c2a4` |
| `outer-host-import-development-source-2.json` | `bc3c4cf9af76f7816d59199a4d3a8b4d4843ef206249318cd1faeee02cdffcfb` |
| Package QA, `outer-host-import-development-qa-4.json` | `9e34429b1abc5a1610d920ce4afc1f3a39019680cf81bc79f435e9125cabea28` |
| Independent six-file source/evidence review | `2250723e9845cc373329a41a005c01dc456a07cd97a2183f035272c26febc5b8` |
| Seven-command integrated QA, `outer-host-import-integrated-qa-1.json` | `ae7a0b0df4192200631e26b244fcf7cedfd1aef3877d7ada395519aae0810d6b` |
| Independent integrated evidence review | `b04c456259acee985da7753379aa7beec8ffd130ccb9e8dfbb0e2522e1a5bf08` |

Manifests, logs and QA receipts are retained under `target/review/convergence/`. The immutable
full tree `outer-host-import-source-2-frozen` and report `outer-host-import-source-review-1.md`
are retained under `.git/baozi-review/performance-convergence/`. These identify local evidence,
not packaged release artifacts.

| Check | Actual result |
| --- | --- |
| Complete Import nextest | **471 passed**, no skips; `outer-host-import-runtime-1.log`. |
| Core/Import/Postprocess strict Clippy | All targets/features, `-D warnings`, PASS; `outer-host-import-clippy-4.log`. |
| Import formatting | Exit 0 with unchanged source maps; `outer-host-import-fmt-4.log`. |
| Complete workspace nextest on source2 | **3,457 passed**, 6 existing skips; run `349b5760-b5c4-49be-9740-76307e473977`, 151 binaries. Complete source maps match before/after. |
| Workspace strict Clippy | All targets/features, `-D warnings`, PASS. |
| Workspace doctests | **104 passed**, including the positive and compile-fail contracts. |
| Independent asset consumer nextest | **138 passed**, 1 existing skip; run `8584ed1c-07f1-46d5-9826-4424d7b9193a`, 19 binaries. |
| Independent asset consumer strict Clippy | All targets/features, `-D warnings`, PASS. |
| Workspace and independent consumer format checks | Both PASS. |

All seven integrated commands exit zero. Each command's complete 1,409-file before/after
maps match the same frozen source2; all retained log hashes match their QA records. The
separate independent integrated audit, `outer-host-import-integrated-review-1.md`, verifies
all 14 maps, seven logs, totals and claim boundaries without finding a source/evidence mismatch.

Independent review recomputes both frozen source sets, verifies all six deltas and QA/log
hashes, and finds no actionable introduced correctness defect. Its package review does not
qualify the separate integrated job, downstream consumers or performance.

## Open Connected Work

Import's fallible `ImportStoreTerminalEvidence::try_from_ledger` remains after handoff.
Its catalog, released-state and owner/cause requirements need an owner-correct precommit
guarantee. Late joint-failure Scene token/table cleanup also remains. Neither seam is closed
by moving the import body or by inspecting already committed prepared owners.

Old `split` and caller-accessible terminal APIs still prevent a final-only authority claim.
Borrowed numeric backing, current-state consuming attachment with one start admission,
shared-shell-before-state promotion, exact administrative accounting and deletion of
superseded ownership paths remain required. The unchanged Windows/Linux 14-case/110-lane
ordinary timing and whole-child memory requirements are open. No new performance campaign
belongs to this source; performance acceptance and release remain HOLD.
