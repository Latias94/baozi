# Explicit Normal Intent in the Asset Consumer

The [normal standards research](../research/2026-11-03-normal-generation-standards.md)
identifies a concrete real-consumer gap: its unconditional Smooth request can smooth a crease
on a glTF surface whose absent NORMAL requires flat shading. This change replaces that
decision at the application boundary. The bare importer, generic normal estimator, evaluator,
buffer sampler and frozen Assimp comparison task are unchanged.

## Intent and Recovery

`Options::normal_intent` defaults to `NormalIntent::SourceFormat`. The resolver uses the
successful import's registered `FormatId`, before consuming its original `ImportReport`.
It does not guess a format from a filename, metadata locator or field absence.

| Intent | Selected field behavior |
| --- | --- |
| `SourceFormat` | Static glTF/GLB missing normals select Flat with authored normals preserved. Other formats preserve authored fields but require an explicit choice for a nonempty surface without normals. |
| `Preserve` | Retain authored shading fields and absence; request no normal generation. |
| `GeometryEdit { normals, tangents }` | Pass the caller's exact Flat/Smooth, Preserve/Replace and tangent policy to the existing public preparation owner. |

Only where SourceFormat would generate an implicit Flat field, preflight rejects authored
tangents, then declared morph targets, then actual bound-skin mesh use. An unbound influence
row or unused skin does not imply binding. A zero morph weight does not erase the declared
unsupported geometry. An empty surface is identified by `face_count() == 0`. Existing owner
and extraction checks still govern unsupported topology, field ambiguity and numeric values.

These rejections are typed `NormalPolicyCause` values. `NormalPolicyFailure` owns the complete
original import publication and exposes the report, cause and consuming recovery. Fallible
metadata reservation retains its actual `TryReserveError`. Planning does not clone/reimport
the Scene or fabricate a postprocess result. Static base normals are not claimed to supply
the geometric normals of morphed or skinned positions.

Accepted input uses one `prepare_triangles` call with one unchanged preparation budget,
followed by one whole-Scene conversion. There are no per-primitive budget resets, retries,
partial success, automatic tangent removal or second material-driven pipeline. Planning is
included in the real consumer's postprocess timer. Its application metadata, like detached
buffers, remains outside the library operation's storage ledger. No application-wide control
or aggregate resource promise is added.

The CLI adds `--normal-intent source-format|preserve|flat|smooth`. Flat and Smooth preserve
authored normals/tangents; Rust callers can additionally authorize replacement or removal.
The README's missing-normal OBJ example now requests Smooth explicitly. The old unconditional
Smooth path and its universal-default claim are removed.

## Standard and Algorithm Boundaries

Generic Smooth remains area weighted over source identities and Face smoothing groups.
Existing NORMAL fields retain their authored values. Unlit data and all lighting fallback
resources remain available; Preserve is suitable for a field-only unlit or position task.
The full-field SourceFormat choice may also generate static Flat normals on unlit triangles.

Normal-map UV selection and tangent declarations remain actual output fields. The public
tangent generator remains **Triangle UV Derivative v1**; this workflow does not implement or
claim MikkTSpace. Imported Unknown remains Unknown. Explicit base-normal editing does not
manufacture missing morph-normal coverage: all normal-consuming evaluator modes still reject
active Position morphs without the required Normal deltas. PositionOnly remains available.

## Qualification

Independent design review removes an unnecessary coverage/report framework before code is
written. Independent source review reconstructs the exact nine-file candidate and finds no
actionable P1/P2. Fourteen new tests preserve existing assertions and add literal folded-face,
authored-normal, smooth-bit, source-CSR, material/UV/tangent, unlit, morph, skin and failure
ownership oracles. Low action/storage allowances still fail through the original producer
without retry or partial publication.

Actual Windows qualification uses Rust 1.95.0, locked/offline dependencies, one Cargo job
and the shared target:

| Check | Result |
| --- | --- |
| All-feature asset-consumer nextest | 138/138; one existing skip; `c1492c0a-aa00-48c7-b1f3-cbf4e3e87dbb` |
| Strict all-target/all-feature consumer Clippy | Passed with `-D warnings` |
| Consumer formatting | Passed |
| Actual debug CLI smoke | OBJ SourceFormat rejection; Preserve/Flat/Smooth success with 4/6/4 detached vertices; retired work flag rejected |

The first strict Clippy run rejects a nested test `if let`; the corrected let-chain retains
the same predicates and assertions. The failed log is kept. Exact integrated source/check
receipts are `target/review/convergence/normal-policy-*.json` and matching logs; CLI observations
are under `normal-policy-cli/`. Compiler/test/lint evidence belongs to this consumer source,
not to the earlier isolated candidate. No new fuzz campaign or renderer-conformance result
is claimed.

The R5 [normal-action performance evidence](2026-11-03-bounded-normal-actions.md) remains
pinned to its own source and immutable binaries. This policy change does not replace that
comparison's explicit Smooth task or establish a speedup. Full release convergence and the
named prior performance repairs remain open.
