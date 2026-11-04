# Borrowed Import Detection and OBJ Construction Checkpoint

Actual detection, Session root transactions and OBJ/MTL now use short loans of the same
numeric invocation. Import's former owning execution, bound work and observed-permit
adapters have been deleted. This frozen construction checkpoint passes the selected
runtime, compiler, lint and formatting gates below. Import's remaining private test
callers and glTF are unfinished in this snapshot; isolated Rust is still unapplied to
main. Performance acceptance remains HOLD.

This continues the [lexical-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md).
The [poll-baseline correction](2026-11-05-import-root-entry-baseline-correction.md) applies:
ordinary root acquisition preserves the original one entry checkpoint. The earlier
zero-entry-poll premise is withdrawn.

## Actual Ownership and Error Changes

`DetectionScope` holds a short mutable root loan and an invariant cached work layout.
It has no `RefCell` numeric owner or independently owning permit. A bounded work callback
uses the existing observed-action host: success consumes its physical chunk, an outer
callback error leaves that attempted chunk unconsumed, and panic resolves the real frame
before reaching registry containment. Existing explicit detector polls and quantum
limits remain in place; observed calls add no parser-action cadence debt.

Same-scope nested work and polling during a callback are now rejected by Rust borrowing.
Paired valid and compile-fail public documentation exercises these restrictions. Runtime
tests retain cancellation, callback-error transport, abort replay, poisoned registration,
panic, quantum and physical-prefix checks. The callback-error fixture now transports a
real abort from an earlier scope; its attempted/consumed prefix remains `1/0`, and the
current Session rejects the missing current-scope cause. It does not recreate a forbidden
same-scope mutable reentry merely to produce a runtime error.

Session transactions propagate their original owned errors as outer callback errors.
The ordinary root's successful return checks quiescence and settles caught unwinds;
it does not introduce a new successful-close running frontier. Actual entry still rejects
failed execution. Session's next original entry/checkpoint observes a detector failure
before publishing evidence. Parser and exclusive geometry regions retain their own
successful-close state and checkpoint policies.

OBJ/MTL record, token, comparison, dense-remap and authoring callers use actual
root/parent/child roles and short services/storage loans. Their physical algorithms,
completion positions and original facade entry polls are retained. Existing vector
resize and formatted-string append implementations move to ordinary `ParseStorage`;
services delegate to that implementation instead of maintaining another growth path.

## Immutable Source and Selected Verification

`target/review/convergence/import-detection-obj-borrow-source-1.json` identifies **1,335
files**, set SHA-256 `daf3c50a7690d10c3763aea1fe0a80a54f4dece68909011e2804d6e31a225a63`.
The entire worker-owned `crates/baozi-format-gltf/` prefix is explicitly excluded. Frozen
inputs are under `.git/baozi-review/performance-convergence/` with the matching
`import-detection-obj-borrow-source-1-frozen` name. This is not a whole-workspace source
qualification.

All checks below ran serially on Windows with Rust 1.95.0, locked/offline dependencies,
one build job and one runtime test thread where applicable. Each receipt retains stable
before/after source maps and original log hashes.

| Exact snapshot check | Result |
| --- | --- |
| Operation runtime | 994 passed, 0 skipped; run `e2a91c0e-9cbb-4ff8-ba3b-cf6dfa598639` |
| Operation documentation | 107 passed, in groups of 24 and 83 |
| OBJ/PLY/STL runtime | 338 passed, 0 skipped; run `a8500685-d69f-4f8c-ab70-c0c8ab607e92` |
| Corrected public root-entry regression binary | 3 passed, 0 skipped; run `d41d2ae9-4e35-455a-bf04-9890de047853` |
| Public detection and cancellation-stage binaries | 17 passed, 0 skipped; run `20b82986-20de-46aa-9170-2b3f8e7f888a` |
| Detection compiler documentation | 6 passed, 33 unrelated examples filtered out |
| Strict Clippy | Operation and all three format packages: all targets/all features; Import: the three named regression targets; all passed with warnings denied |
| Formatting | Operation, Import, OBJ, PLY and STL passed |

Receipts are `import-detection-obj-borrow-qa-1.json` and
`import-borrowed-detection-selected-qa-1.json`, under `target/review/convergence/`.
They do not establish all Import targets, all Import documentation, consumer, Linux or
complete-workspace qualification.

## Independent Review and Remaining Connection

The bounded independent report is
`.git/baozi-review/performance-convergence/import-borrowed-detection-source-review-1.md`.
It finds no new actionable production defect in the inspected root/detection/registry
ownership, prefix, error, close and publication paths, independently of test success.
It verifies the first receipt's terminal checks and source/log association. The second
receipt completed after reviewer handback; its terminal identity is checked separately
by the main executor. The review does not cover OBJ algorithm fidelity or glTF.

Subsequent private test migration and shared storage-transition support are new source
changes and require their own qualification. Remaining work includes those original
Import callers and public compiler examples, glTF and all-format connection, independent
integrated review, current-state attachment, shared-shell promotion, obsolete Operation
owner deletion and main application. The unchanged Windows/Linux 14-case/110-lane
performance comparison and separate heap attribution remain required. No performance
measurement or optimization keep is claimed for this construction snapshot.
