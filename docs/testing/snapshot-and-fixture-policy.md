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

## Review Rules

Snapshot diffs should be readable in code review. If a snapshot becomes too large to inspect, split
the fixture or lower the snapshot sample budget rather than depending on `Debug` output or an
opaque serialized scene.

Snapshot changes must be reviewed against the parser behavior they prove. Regenerating snapshots is
not a substitute for explaining why the importer output changed.
