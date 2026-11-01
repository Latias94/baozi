# Asset Fidelity and Consumer Workflow Expansion — 2026-11-01

The user adopted the initial tranche of the
[execution plan](../plans/2026-11-01-001-feat-asset-workflow-expansion-plan.md) after the
[Assimp comparison](../research/2026-11-01-baozi-assimp-gap-analysis.md). The completed A–H sequence
stays closed. This record distinguishes implemented units from planned or currently running work.

| Unit | Status | Evidence |
| --- | --- | --- |
| A1: PLY color fidelity | Implemented and locally verified | Public typed RGBA and loss-policy regressions below. |
| A2: MTL semantics and bounded omissions | In progress | No completion claim. |
| B1: indexed application buffers | In progress | No completion claim. |
| B2: retained provenance readers | In progress | No completion claim. |
| V1: larger workflow evidence | In progress | No workload qualification yet. |
| V2: edit-lifecycle fuzz | In progress | No campaign qualification yet. |

## A1 — PLY Color Profile

Unsigned U8/U16/U32 colors and alpha now use full-range normalization with F64 division and one
final F32 rounding. Floating color range checks precede narrowing; signed channels and nonfinite
or out-of-range values fail. Exact floating values and signed zero survive. Inexact F64 color
narrowing requires `SourceSemanticLoss` and records the vertex/property plus original/output
bits; `FormatRecovery` alone does not authorize it. Custom scalar precision is unchanged.

The named profile is documented in [PLY support](../formats/ply.md#accepted-color-profile).
Independent review found no conversion defect and requested stronger denominator-order and loss
detail tests; those additions now distinguish premature F32 conversion and a nonzero record index
with alpha alias `a`.

Executed on Windows x86_64 with Rust 1.95.0, serial Cargo builds and nextest:

- `cargo +1.95.0 nextest run --locked -p baozi --all-features --test ply_colors -j 1`:
  **4/4 passed**, nextest run `2d299493-f784-49b8-a2f8-6c5a44f142ba`. The cases iterate all three
  encodings and both channel-name spellings and assert actual typed values/diagnostics. Log:
  `target/review/asset-expansion-a1-colors-nextest.log`.
- `cargo +1.95.0 nextest run --locked -p baozi-format-ply -j 1`: **68/68 passed**, run
  `3699fdb6-a94b-4715-8c8c-cfdeea875a8e`, including existing custom F64, malformed, resource-limit
  and text-profile cases. This run preceded A2's partial-channel policy changes.
- `cargo +1.95.0 clippy --locked -p baozi --all-features --test ply_colors -- -D warnings` passes.
  Log: `target/review/asset-expansion-a1-clippy.log`. Rustfmt was run on the two changed Rust files.

These are local A1 results, not a new hosted/platform matrix, format maturity or release claim.
