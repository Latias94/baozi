# Changelog

All notable Baozi changes should be recorded here.

Baozi currently publishes no stable compatibility promise. The project may make breaking API and IR changes while the importer foundation is being built.

## Unreleased

- Replaced the legacy post-process step pipeline with an owning canonical
  `PostProcessRequests -> CompiledPostProcessRun -> PostProcessRunOutcome` contract. The first
  truthful slice admits only explicit empty requests, preserves the input generation, opens no
  edit transaction, and reports `Unchanged`; transformation requests return only when their
  canonical compiler, edit program, report, and fuzz evidence land together.
- Hardened importer registration, format descriptors, import reports, diagnostics, and postprocess lifecycle boundaries.
- Added OBJ and STL experimental importers with support-matrix checks, fuzz smoke targets, and facade coverage.
- Added core IR extension fields for skins, morph targets, custom vertex attributes, cameras, lights, animation channels, material properties, texture samplers, and texture transforms.
