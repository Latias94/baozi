# Joint Payload Cleanup Before Failure Preparation

The isolated successor to the [connected Import checkpoint](2026-11-04-connected-import-outer-host.md)
keeps actual canonical output and proof inside their joint owner. A caller receives these
values only with prepared success. Discarded failure payloads are destroyed before failure
qualification, physical discard, evidence freeze and terminal commitment. This is a bounded
ownership repair; the [lexical-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md)
and performance release gates remain in progress.

## Actual Transition and Deletions

The raw terminal entry now classifies the real success, rich-failure or fallback rail without
qualifying its ledger or freezing evidence. The classification retains its original guard,
attribution, tables, account, roots, set identity and control cause. Existing ordinary finish
methods compose classification and the same finishing rails. Frozen-success integrity
rejections retain their existing containment policy; this repair does not invent a fallback
for a broken exact graph or add a cancellation poll.

`CanonicalJointCommit` owns the original host-backed output/proof loans alongside that physical
contribution. Its higher-ranked binder passes only the joint and matching seal. Fixed internal
preparation retains both payloads through success commitment. For a selected failure, it
destroys output then proof, with separate unwind boundaries, while the selected graph remains
owned. It checks the graph's actual account after clean destructors; a recorded storage-release
failure or destructor fault contains that same graph before failure qualification. No new
ledger, account, public finalizer callback, proof copy or unmetered allocation is introduced.

The three Core consumers construct their Scene token or edit success fields only after the
fixed success result returns its actual output/proof and store. The old post-preparation
`drop(token)` branches, separately owned `ScopedNodeSceneEditJointCommit`, concrete late-intent
wrapper and unused frozen contribution wrapper are deleted. The direct edit consumer retains
the actual optional control cause on rich/minimal results instead of replacing it with a
generic publication cause.

Evidence freeze still precedes the last account eligibility check in the existing joint
commit. A later account poison remains unpublished containment. This repair does not claim
that cleanup after such a containment can precede an already completed success evidence
freeze. Generation reservation can burn one ID on a later failed joint; it is not a terminal
Scene commitment, and the reserved ID is neither rolled back nor reused.

## Actual Source and Checks

Source1 changes 14 files from the reviewed Import source2. Source2 changes only two newly
added test assertions and explanatory comments. Its production bytes are unchanged from
source1. Both complete frozen trees contain 1,409 files.

| Artifact | SHA-256 |
| --- | --- |
| Source2 complete set, sorted UTF-8 path + NUL + lowercase hash + LF | `d888e34389c92f16374526a2b2eb106d2207ce43eae3e67c01e40ddc4be71248` |
| `precommit-joint-development-source-2.json` | `79b02864451e479561a140173dd312b2896f183a737456cf048e060cea8b8ab4` |
| Affected source2 QA, `precommit-joint-development-qa-4.json` | `7bec87042bef20cf15a36f3f68e1a3a5f096a8de66fb464ca6138454b77d2346` |
| Independent source1/source2 review | `926a6673e5ff95e93e5e1592aa1dc87c63811a273e86f6c7a1a8846a53e19752` |
| Workspace/consumer QA5, including retained format-command failure | `9f31184538ba67d55ed2dde308a0fe75ea321dce96d07f611d1e775b7892292e` |
| Exact consumer format-check QA6 | `5f2dfba916ce0114fba03f81d8761aaf370aa703ee17d0cc683328a42adfa5b3` |

Local manifests, logs and QA are under `target/review/convergence/`; frozen trees and review
artifacts are under `.git/baozi-review/performance-convergence/`. These retained local
identities are not bundled release artifacts.

| Actual check | Result and scope |
| --- | --- |
| Affected strict Clippy on source1 | Operation/Core/Postprocess, all targets/features, `-D warnings`, PASS. |
| Complete Operation nextest on source2 | **958 passed**, no skips; run `bccfd2e8-6276-4368-80c5-f893656c1a6a`. |
| Core/Import/Postprocess runtime on source2 | **1,461 passed**, 6 existing skips; run `967182df-2e52-43fa-9356-c4d987068ed0`, 42 binaries. |
| Affected doctests on source2 | **105 passed**, including positive and compile-fail contracts. |
| Affected strict Clippy and workspace format check on source2 | PASS; all six QA commands preserve the complete frozen source maps. |
| Independent actual source review | No actionable introduced defect in frozen source1/source2; two additional composition-test gaps remain below. |
| Workspace strict Clippy on source2 | All targets/features, `-D warnings`, PASS. |
| Independent asset consumer on source2 | **138 passed**, 1 existing skip; run `807f756b-0777-4770-9b8b-d9a45130b583`, strict Clippy and exact consumer format check PASS. |

The source review is `precommit-joint-source-review-1.md`. It separately verifies the test-only
source2 repair and exact 958-test runtime association. It does not qualify the broader
caller/docs/consumer checks from implementation intent alone. QA5's consumer format command
used `--all`, which traversed an unrelated candidate vendor dependency and failed Cargo metadata
workspace discovery. No source changed. QA6 checks the exact consumer manifest without `--all`
and passes; the complete root workspace format check already passed in QA4. The failed command
and its log are retained, and no candidate manifest or native-promotion setting was changed.

The first Operation run on source1 records **956 passes and two failures**, run
`4d293619-655c-4812-8023-38f25531ef05`, in `precommit-joint-operation-runtime-1.log`.
Both failures were new expectations that containment records only its first fault. Existing
containment also records six actual orphan edges: table and member for each one-member table,
plus fallback and ledger-evidence allocations. Source2 asserts those exact additional events
and retains the original first poison, unqualified ledger, unfrozen evidence and physical
charge checks. No production code or pre-existing oracle is changed for this repair.

New real host tests cover borrowed non-Copy output/proof through successful joint preparation,
forgotten joint cleanup at host Drop, and direct joint Drop under every output/proof panic
mask. The raw tests cover exact frozen success, control-selected rich/minimal branches,
explicit minimal request and cleanup poison while the original graph is still unqualified.
Independent review requests direct composed coverage of preparation failure plus pair-destructor
panic/unwind, and real shared-retained storage refunds/release failure on selected rich/minimal
rails. Those combinations are the immediate successor; the current checks do not claim them.

## Remaining Qualification

This checkpoint does not remove Import's post-handoff ledger projection or old split/client
terminal authority. Numeric backing, current-state consuming attachment, one startup admission,
shared-shell-before-state promotion and deletion of remaining superseded paths are still open.
No whole-workspace runtime is attributed to this successor yet. The earlier 3,457-test integrated
receipt belongs to Import source2; the independent consumer receipt above belongs to joint
source2. The composed-regression successor remains a separate moving source. Main Rust remains unapplied, and no
new ordinary timing or memory campaign exists for this successor. Release remains HOLD.

## Composed Successor, Source6

The fixed classified finishing body is now a single private function called by actual joint
preparation and its real general-classifier composition fixtures. The production body is
extracted without changing its transitions. The actual attributed `SharedRetainedStorage`
tests check clean refunds against the original weak ledger before failure qualification,
and a destructor that returns normally after its physical payload fails to release. The
latter preserves first poison, physical charges and ledger attribution, and prevents freeze.
Both rich and minimal decisions are exercised; the minimal fixture stages actual evidence
headroom before central validation rather than relying on its name or limit alone.

The actual prepare-error test covers three distinct failure points, two unwind states and
all four output/proof panic masks. A test-only extension of the existing primary-bind hook
selects true canonical binding for genuine classifier failure; its original evidence-only
false-binding behavior remains. Evidence-qualification failure correctly observes a qualified
ledger. Account poison is a separate final-eligibility failure after detach/freeze, preserving
the original poison and unpublished containment. Both payloads are attempted in order and the
original outer panic survives. No test callback is added to a production transition.

Earlier composed snapshots remain RED: source3's wrong typed reader and source4's test loans
fail compilation; source5's phase/branch oracles fail runtime; an intervening run still
mislabels account poison as early classification. The immutable logs and independent report2
retain these findings. Source6 fixes the actual test triggers and observations; it does not
change production semantics to match an incorrect oracle. Independent report3 closes all
three findings and both composition gaps, and verifies the final source/QA associations.

| Source6 artifact | SHA-256 |
| --- | --- |
| Complete 1,409-file set | `9fc2c52ebb059e90c0f32e6bf4305fa79d6146be9011be5e6390d772fadbcff2` |
| `precommit-joint-development-source-6.json` | `c6b1c527721d821df860bf5e80ad28d4e116d34038466325d2e0b3adfc8fabf5` |
| Operation QA11 | `9c43818b721e3b43566cb9c066845181c269805d2b18f5f0b5aaf05cead69542` |
| Connected caller/docs/workspace/consumer QA12 | `c76eb9fa894aa9f875a4b0855ea092d50a41822749c18d9bf26d6b1ec7d0e072` |
| Independent final `precommit-joint-source-review-3.md` | `770ca13c60fcabd5a803b596fceea692534ec60df0f396eed53d6039986f650b` |

Actual source6 passes **961 Operation tests** (run `2841f816-3dfb-4642-ba10-fd85a2baac46`),
**1,461 Core/Import/Postprocess tests** with 6 existing skips (run
`d4969a83-bd65-4053-b8aa-6696faca119c`), **105 doctests**, and **138 independent consumer
tests** with 1 existing skip (run `43714b44-9fa5-4067-bafe-26f1809b2272`). Workspace and consumer
all-target/all-feature strict Clippy, root format check and exact consumer format check pass.
All stable check maps match source6. QA11's initial formatter is recorded as a mutation with
its after-map; it is not mislabeled as a stable format check.

This closes the bounded joint composition checkpoint. It does not establish a whole-workspace
runtime result or performance keep. The actual canonical frozen contribution still returns
success or containment; a later simplification can remove its unreachable rich/minimal API
arms while preserving the real general terminal rails. Import schema admission/projection,
exclusive terminal authority, numeric backing, attachment/promotion and all unchanged ordinary
comparison targets remain open. Main Rust is still unapplied.
