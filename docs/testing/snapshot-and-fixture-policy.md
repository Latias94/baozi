# Snapshot and Fixture Policy

Baozi snapshot fixtures are test artifacts. They are not a public interchange format, a cache
format, or a promise that `Scene` debug output is stable.

## Snapshot Schema

`baozi-test-support` owns the normalized scene snapshot schema used by parser tests. The schema is
handwritten text with a version line, stable object IDs, deterministic metadata key ordering,
bounded vertex samples, material summaries, and diagnostics sorted by severity, code, source,
location, and message.

Floats are printed with six decimal places by default. Negative zero is normalized to positive zero
so geometry snapshots do not churn on platform-specific formatting details.

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
