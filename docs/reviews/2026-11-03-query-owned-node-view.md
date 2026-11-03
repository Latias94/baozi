# Query-Owned Checked Node View

Repeated node getters in `base_pose_world_transforms` bind a full semantic view and create
disposable cross-domain cursors. This slice binds one complete checked view at the query's
existing first semantic access and reuses a small borrowed node projection. It belongs to the
[Assimp convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md); it does not
implement the separate exclusive-caller or owned-payload cutovers.

## Owner and Preserved Behavior

The view is created after the original work/byte preflight, allocation, actual-capacity admission
and identity initialization. It performs the unchanged complete semantic constructor's ordered
node, spatial, appearance, geometry, deformation, animation and profile checks, including for
an empty forest. Its Scene borrow keeps storage/proof/generation immutable throughout the query.

One mechanical decoder serves the projected random reads, standalone node getters and genuine
full traversal. It preserves transform, name/UTF-8 and child-range checks; the full cursor keeps
its phase/count/finish protocol. Random access no longer constructs a disposable full cursor.
Reference reads remain before the following work step, transform reads remain after admission,
and final cancellation remains before returning the owned F64 transform table. A valid foreign
Scene handle is rejected by the private query interface. No narrower transform-only decoder,
cache, new work authority, numeric shortcut or public API is added.

For a successful forest of N source nodes, source inspection predicts full bindings `2N+1 -> 1`,
with the same `2N` full row projections, `2N` charged work units and admitted output capacity.
This is an implementation property to corroborate with separate attribution, not a native-time
gain. Normal mathematics, format policy, detached-buffer fields and comparison clocks are unchanged.

## Source Qualification

Application base is exclusive-proof commit `45a09d19d948600d9b2a4261eb2e3394fe594ef1`. Its six
relevant baseline files are byte-identical to candidate base
`509b80de4fff2494d5315f7a17e086cb7afe2f26`. The frozen four-file patch has SHA-256
`c738809984d3bb2e36140d51fa4231088e8dbfddd112679a139daae0374f83d8`. Only the three production
owners and one existing test file change. Actual formatting touches new test/helper code only;
independent candidate and application-delta reviews find no actionable P1/P2. Later glTF profile,
consumer normal intent and exclusive numeric-owner source remain intact.

Windows checks use Rust 1.95.0, locked/offline dependencies, one Cargo job and the shared target.

| Check | Result |
| --- | --- |
| Focused base-pose/decoder tests | 37/37, 647 other unit tests filtered; `3c921f90-6ed4-4a22-822e-8e44d5a55a8e` |
| All-feature core/postprocess nextest | 972/972, six existing skips; `3e51903e-af9a-47fd-a798-9c9a64467026` |
| Core/postprocess doctests | 6/6 and 4/4 |
| Independent asset consumer, all features | 138/138, one existing manual skip; `782fa6db-f788-4608-87d2-653004596765` |
| Strict affected and consumer Clippy | Passed, all targets/features with `-D warnings` |
| Core formatting | Passed |

Seven new tests observe actual published/corrupted Scenes and the shared decoder. They preserve
work/byte/pre-cancel admission before physical failure, lazy reference/transform reads,
reference-failure priority over the following cancelled step, final-poll rejection for empty
and nonempty tables, foreign-owner rejection, owned output after Scene destruction, and an
earlier numeric error before a later physical fault. Existing F64, capacity, allocation and
full-snapshot oracles are retained. The deterministic cancellation hooks are private test code;
they add no production callback or poll.

Exact application and check receipts are `target/review/convergence/node-view-*.json` with
matching logs. Source reviews and the frozen packet remain under
`.git/baozi-review/performance-convergence/node-view-*`. A filtered test is not counted as a
manual skip; the focused run is a subset of the complete reader qualification, not extra tests
to add to its total.

The final four-path identity-set SHA-256 is
`14f203e431c5d75382ed030e8d824749e498a4d53a1673f0cde6158cf4ba6871`, using sorted UTF-8
`path + NUL + lowercase raw SHA-256 + LF` with forward-slash paths. All seven completed check
receipts match those actual source bytes. Their aggregate is `node-view-qualification.json`,
SHA-256 `cbab955e8220345c462aa089dc2d5ea59b9e66605a997022817d681fa2aed9d1`.

## Measurement Boundary

Source qualification alone supplies no measured keep or repair of the prior F64/glTF costs.
Use a fresh ordinary control from `45a09d1`, so the preceding work-kernel extraction is common
to both sides; the earlier R7 ordinary worker is not that control. Retain exact source archives,
compiler/build settings and binary identities. Keep all eight selected cases and 31 lanes,
three warmups plus 20 retained AB/BA pairs, with the campaign order reversed for confirmation.
No build or profiler may overlap ordinary collection.

Control source is extracted from its immutable Git archive under the ignored artifact directory
without reverting the working tree; it uses the same shared target, Cargo release profile,
compiler, allocator, feature selection and one build job. Source/build paths are recorded.
Separate instruction profiles may corroborate owner changes but do not qualify native latency
or stage memory. All positive lanes, control drift, tails and whole-child peak memory must be
reviewed, including changes outside the targeted extraction endpoint. The parent plan's full
Windows/Linux 14-case/110-lane all-endpoint/pipeline/memory 3x gates remain open.
