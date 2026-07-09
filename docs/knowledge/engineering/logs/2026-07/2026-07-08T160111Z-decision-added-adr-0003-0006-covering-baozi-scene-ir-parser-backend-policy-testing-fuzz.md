---
type: "Memory Event"
title: "Decision: Added ADR 0003-0006 covering Baozi scene IR, parser backend policy, testing/fuzz"
description: "Added ADR 0003-0006 covering Baozi scene IR, parser backend policy, testing/fuzzing/oracle strategy, and public API stability tiers."
timestamp: 2026-07-08T16:01:11Z
event_kind: "Decision"
---
# Event

Added ADR 0003-0006 covering Baozi scene IR, parser backend policy, testing/fuzzing/oracle strategy, and public API stability tiers.

# Impact

Baozi now has proposed architecture decisions for the core scene IR, parser backend ownership,
verification strategy, and API stability policy. Future implementation should align workspace crate
creation and importer work with these ADRs unless a later ADR supersedes them.

# Citations

- [ADR 0003](../../../../adr/0003-core-scene-ir-and-material-model.md)
- [ADR 0004](../../../../adr/0004-parser-backend-and-format-coverage-policy.md)
- [ADR 0005](../../../../adr/0005-testing-fuzzing-and-differential-oracle-strategy.md)
- [ADR 0006](../../../../adr/0006-public-api-versioning-and-crate-stability-policy.md)
