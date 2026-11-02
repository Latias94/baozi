# Snapshot and Fixture Policy

Baozi snapshot fixtures are test artifacts. They are not a public interchange format, a cache
format, or a promise that `Scene` debug output is stable.

## Snapshot Schema

`baozi-core` owns the bounded canonical Scene snapshot traversed by parser and facade tests. The
snapshot combines a complete, versioned semantic digest with a bounded review preview and typed
totals. Tests consume that API through `ValidatedScene`; test support does not duplicate the Scene
or define a second snapshot schema.

Snapshot traversal and normalization rules are defined beside the retained canonical schema. Tests
must use explicit preview-byte, collection-item, and work limits.

## Fixture Rules

Fixtures committed to Baozi must be hand-authored, generated inside Baozi tests, or copied only
from sources with an explicit license audit recorded near the fixture. Do not copy Assimp test
assets into this repository.

Parser tests should prefer small in-memory fixtures first. File fixtures are appropriate when the
format behavior depends on filenames, sidecar lookup, archive layout, or binary corpus reuse.

## Independent Behavior Evidence

Expected meaning comes from authored literal values, independently decoded source bytes, a stated
mathematical invariant, or a pinned normative rule. A production decoder, remap, serializer mapping
or snapshot encoder cannot provide its own expected answer. Round trips, cross-loader comparisons
and copy-correspondence checks remain useful when their narrower purpose is explicit; anchor the
claimed interpretation with an independent value or identity assertion.

Keep a negative input valid except for the condition under test and include a nearby valid control.
Check its typed failure and distinguishing subject or diagnostic where the public contract provides
them. An unrelated rejection does not prove the intended guard. Preserve authored absence, source
precision and signed bits separately from calculated values with justified tolerances.

Detached-consumer tests read actual fields or evaluate output after the original Scene, reports,
importer and IO are destroyed. Nonempty output, a summary count or a digest alone does not prove
that the selected data remains useful. Failure and resource-release checks retain their independent
purpose alongside these observations.

External comparisons record the specification, loader, tool and asset revisions and their finite
common profile. Validator success is evidence only for the checks that validator implements;
another loader's acceptance is not a normative answer. Track deliberate policy differences rather
than repairing fixtures or weakening expectations to obtain agreement. When fixing an identified
assertion-sensitivity defect, retain a localized fault-injection result where practical. Test counts
and aggregate mutation scores are not maturity gates.

## Review Rules

Snapshot diffs should be readable in code review. If a snapshot becomes too large to inspect, split
the fixture or lower the snapshot sample budget rather than depending on `Debug` output or an
opaque serialized scene.

Snapshot changes must be reviewed against the parser behavior they prove. Regenerating snapshots is
not a substitute for explaining why the importer output changed.
