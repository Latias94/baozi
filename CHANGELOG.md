# Changelog

All notable Baozi changes should be recorded here.

Baozi currently publishes no stable compatibility promise. The project may make breaking API and IR changes while the importer foundation is being built.

## Unreleased

- Replaced `PostProcessSuccess` with the requested-run `PostProcessRunOutcome` and
  `PostProcessReport` contract. Callers now inspect `disposition()`, read ledger evidence through
  `report().ledger()`, and consume unchanged or changed results with
  `into_unchanged_report()` or `into_changed()`. Empty and already-satisfied pipelines preserve the
  input generation, skip validation and edit staging, and report `Unchanged`.
- Hardened importer registration, format descriptors, import reports, diagnostics, and postprocess lifecycle boundaries.
- Added OBJ and STL experimental importers with support-matrix checks, fuzz smoke targets, and facade coverage.
- Added core IR extension fields for skins, morph targets, custom vertex attributes, cameras, lights, animation channels, material properties, texture samplers, and texture transforms.
