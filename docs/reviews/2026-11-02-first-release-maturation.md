# First-release maturation execution record

## Reviewed design

The [master plan](../plans/2026-11-02-001-refactor-first-release-maturation-plan.md)
replaces the implementation choices of the earlier first-release usability plan.
Problem evidence remains the [usability audit](2026-11-01-first-release-usability-audit.md)
at `881d105df8899fe2ce9f062262d6607a03f26e58`.

Five independent native Codex review lanes examined feasibility, coherence, scope,
adversarial failure scenarios and product usefulness on 2026-11-02. Feasibility and
adversarial review found no actionable plan defects. Scope and product review
identified the same missing portable file/sidecar onboarding acceptance. Coherence
review identified two incorrect references to existing precision owners.

The plan now assigns a bounded application-owned file-to-`MemoryAssetIo` example
to U5 and references the actual typed storage and base-pose modules. Final
coherence review found no unresolved findings. The reviewed design also makes the
material assignment tag, central morph/base-field invariant and animation
endpoint semantics explicit. These reviews approve an implementable design;
they are not execution, numerical correctness or allocation-bound evidence.

Raw design dossiers and review JSON are retained locally under
`.git/baozi-review/maturation-plan/`. No external programming model, registry
operation or native promotion was used.

## Execution status

U1–U8 are pending implementation and unit-specific verification. The current
portable graph still has twelve packages. Its reduction to ten depends on the
complete U3 decoder cutover and deletion of both maintained glTF forks.

Actual publication, retirement of the existing `0.0.1`, target promotion and
unexecuted hosted checks remain separate work. A completed implementation unit
will record its source identity, commands, observed regressions and practical
evidence limits here before the master plan can be considered complete.
