---
type: "Memory Event"
title: "Progress: Completed U1/U2 import facade and WASM-aware IO feature graph in commit 0633fd4."
description: "Completed U1/U2 import facade and WASM-aware IO feature graph in commit 0633fd4. Verified importer_api, baozi-import registry tests, baozi-i"
timestamp: 2026-07-09T03:35:46Z
event_kind: "Progress"
---
# Event

Completed U1/U2 import facade and WASM-aware IO feature graph in commit 0633fd4. Verified importer_api, baozi-import registry tests, baozi-io tests with and without fs, workspace check, no-default, wasm32-unknown-unknown, and wasm32-wasip1 native-fs checks.

# Impact

U1 and U2 are complete. The next implementation unit should start from scene validation: add the
shared validator, change `SceneBuilder::finish` to return `Result<Scene>`, and wire
`PostProcessStep::ValidateScene` to the same validator.

# Citations

- `0633fd4 feat(import): add bytes-first importer facade`
- [Plan](../../../plans/2026-07-09-002-feat-stl-importer-vertical-slice-plan.md)
