# Actual Outer Host and Postprocess Checkpoint

This is an isolated implementation checkpoint of the
[lexical invocation-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md),
following [fixed import publication](2026-11-04-closed-import-publication.md). Both real
postprocess entry paths now request a disposition inside a retained body and finish after
body/capture cleanup. This is not whole-driver qualification, a main Rust application or a
performance keep. Import's outer migration and the remaining ownership cut are incomplete.

## Actual State and Removed Postprocess Path

The existing raw root holds finite pending Operation, Assembly, Failure, Fallback and Success
stages. Their actual closure proofs, causes and containment flags remain beside the same
coordinator, baseline, tables, attribution and protected-work capability. Deferral installs
the reference-free stage before disarming only its short borrowed wrapper. Restoration checks
the exact variant and physical fields before consuming it; a wrong restorer or missing field
does not consume a valid pending stage.

The concrete host retains each exact key domain, typed root, failure budget and destination,
alongside its actual output/proof and distinct typed cause slots. Its seven dispositions keep
started unchanged/fallback, candidate, never activated, activation rejection, central rejection,
interruption and central fallback separate. No pending wrapper borrowing its own host, new
ledger, replacement account, cloned domain, erased error or additional payload lifetime bound
is introduced.

`run_body` retains `Option<FnMut>`, calls it once with short permissions and accepts unit.
The body requests a concrete disposition and returns. The driver separately destroys captures
while actual state remains armed, then lends final-only restoration authority. Missing requests,
second admission and body/capture panic fail or contain. Host values are destroyed before
physical containment; the original body panic precedes capture and payload cleanup panics.
Forgetting a permission or finish leaves actual values and physical owners in the root.

Both `prepare::run` and `convert_scene_space` use this path. They keep the original Scene and
typed local error outside the callback. Mapping/transaction loans end first, followed by
replacement descriptors, generated scratch and callback captures, before final control, freeze
or generation. Early local errors request fallback; unchanged work requests evidence-only
success. `PendingEdit`, its borrowed return path, the old `finish_changed` chain and eager
body rejection/finalization are removed from these production entries. Existing bounded
`with_execution` action results remain ordinary inner work.

## Preserved Failure Construction After Sealing

A real new regression initially failed: a never-activated owner restored after body cleanup
could no longer open its rich-failure assembly because ordinary work was already sealed.
The historical run `68fa82b3-c981-4636-b39a-6778329ab4b9` records nine passes and this failure.
It remains retained as `target/review/convergence/outer-host-stages-runtime-3.log`.

The repair leaves an actual `TerminalOperation` marker in the root after restoring its
unopened operation. Only a failure-only assembly can use this marker. Existing key/table/account
admission and the original protected-work capability still govern that opening. Ordinary
work and success-capable activation remain denied; a second body deferral is rejected.
The original test still requires actual rooted construction, validation and rich publication
with requested code 11, followed by clean release. Minimal fallback is explicitly rejected
as a substitute. The later ten-test run and complete Operation package run pass this oracle.

## Frozen Sources and Actual Checks

The independently reviewed source1 has 1,409 files. Source2 is reconstructed from that immutable
tree plus the exact final Operation QA inputs; moving live Import is excluded. Its only
source1 delta is an Operation driver doctest and additional host tests. Production behavior
is unchanged, so the earlier P runtime covers the same inspected executable implementation.
This association does not turn the separate package runs into a whole-source run.

| Artifact | SHA-256 |
| --- | --- |
| Source1 complete set, sorted UTF-8 path + NUL + hash + LF | `02a2003b1d1cafcb4bf39c4afef40ecec7a665f10d9d25daabc58bfd10755199` |
| Source2 complete reconstructed set | `60c4a117317eae1939710e76e4d12dd7bab74d136084ceaad36feb7ff417d704` |
| Source2 manifest | `6ae9d8f7076d79210d13ced10326c8739edf9d21291fdcf60ba6e4693416ec08` |
| Final Operation QA receipt | `6faabf8d6752425bf4c3ee3bbf1a602fa42bde24dca98b78b68d9b3f9284cc69` |
| Earlier exact O/P lint and P runtime receipt | `633a84eac1c8c0a56045051ae8436db6276cc3f6bf622c1e916a752beb0aaace` |
| Independent source/evidence review | `84df0f5226a696293c1a685d0908585ca6ab6709db1f205fd5e176fcd1633500` |

Local manifests and QA receipts are under `target/review/convergence/`:
`outer-host-postprocess-development-source-2.json`,
`outer-host-stages-operation-development-qa-5.json` and
`outer-host-postprocess-development-qa-2.json`. Immutable sources and the report
`outer-host-postprocess-source-review-1.md` are under
`.git/baozi-review/performance-convergence/`. They identify retained local evidence,
not files bundled with this export.

| Actual check | Result and source scope |
| --- | --- |
| Complete Operation nextest | **951 passed**, no skips; run `80b4bd33-4c78-4a69-a0f6-fa6531e63b80`, exact final 116-input scope. |
| Operation doctests | **59 passed**: 7 positive and 52 compile-fail, including body-permission escape rejection. |
| Operation strict Clippy and format check | PASS, all targets/features, `-D warnings`; exact final scope. |
| Complete Postprocess nextest | **295 passed**, 2 unchanged manual skips; source1 run `57ca7e84-4499-4189-8ab2-24c2c39de02f`. |
| Earlier O/P strict Clippy | PASS, all targets/features, `-D warnings`; complete source1 maps unchanged. |
| Postprocess formatting | Successful format command; no separate explicit P format-check receipt at this checkpoint. |

Final O runtime/docs/lint/format-check maps are stable and match source2's corresponding
inputs. Initial formatting changes are recorded rather than described as stable checks.
Independent review verifies all frozen files, manifest/receipt links and log hashes, and
finds no actionable introduced defect in the raw/concrete/P delta.

All six real P cleanup branches pass through the new production request/resume path: candidate,
activation rejection, never activated, central rejection, interruption and central fallback.
Their existing assertions retain nonempty mappings and generated/descriptive scratch,
exact cleanup order, one freeze, original Scene contents, first-control facts, failure owners
and resource evidence. Existing numerical, declaration-only space conversion, cancellation
and one-MiB stack regressions also pass.

New Operation probes carry borrowed non-Copy output/proof through a real request, forgotten
finish and direct host Drop. Every payload-panic mask preserves exactly-once proof/output
cleanup while storage remains live. Another probe combines body, capture, proof and output
panic, preserves the original body panic and checks actual cleanup order. These verify the
new host path rather than relying only on earlier unit-payload tests.

## Open Gates

The old Operation `split` and client-callable prepare/freeze/joint APIs remain available.
Migrating P alone therefore does not establish exclusive outer terminal authority. Import's
outer session/cell and owned staging are a separate moving successor. Its precommit terminal
projection and late joint-token cleanup are also unresolved. Borrowed numeric backing,
current-state attachment, one start admission, shared-shell-before-state promotion and deletion
of superseded ownership paths remain necessary.

No new whole-workspace or independent consumer run, final integrated doctest/lint gate, or
ordinary timing/memory campaign is associated with this checkpoint. The unchanged Windows/Linux
14-case/110-lane comparisons and release targets remain HOLD. Main Rust remains unapplied.
