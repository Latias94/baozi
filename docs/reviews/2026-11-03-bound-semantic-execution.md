# Session-bound Canonical Semantic Execution

This is the first implementation experiment of the
[Assimp convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md), following
the [current cost diagnosis](../research/2026-11-03-performance-convergence-diagnosis.md).
It replaces repeated semantic-host binding; it is not a complete convergence result.

## Replacement

Canonical semantic preflight borrows the existing work, storage and control capabilities once
from its operation and binds its work dimension once. Each real action still admits one unit
before execution, completes that unit immediately after normal return, explicitly resolves the
permit and only then advances the control cadence. A returned domain error is completed work;
an unwinding action is attempted-only. No numeric borrow spans an action or destructor.

The session retains scoped scratch reservations, single-use completion branding, nonempty
completion, first-cause precedence and contained callback/result destruction. It uses the
invocation's existing at-most-256-action cadence, with entry, scratch and final checkpoints.
The old semantic-session batch/cold producer-host path and its now-unused cadence method are
removed. This experiment does not change normal arithmetic, OBJ remaps, parser staging or
canonical payload layouts, and does not depend on the separate compact-capability draft.

Pre-cancelled control may now reject `begin_semantic_work` before constructing the session.
No action or work unit runs. Cancellation after entry is still observed by the bounded or final
checkpoint. The active session now borrows the local numeric authority directly; its capability
and scoped lineage restrictions are qualified separately by compiler contracts.

## Failure Corrections Found During Qualification

The first operation run passed 858/859 tests. The existing test for an already selected Ledger
failure exposed an entry regression: the new ordinary execution checkpoint returned an opaque
stopped state, which activation misclassified as an ownership contract failure. Independent
source review found the same problem. The corrected scoped outcome owner resolves an existing
terminal before lending ordinary execution or invoking the semantic transaction, retaining the
original terminal code and the established `SuccessOutcomeRejected`/NeverActivated wrapper.
It does not make arbitrary stopped/contract errors recoverable.

The first affected-caller run then passed 1,399/1,400 tests. The unchanged import test for late
cancellation after Ledger rejection showed that the early terminal exit also needed a control
observation before retaining its failure owner. The corrected cold branch uses the existing
terminal-control poll inside the containment catch boundary. A later cancellation is recorded
without replacing the already selected Ledger code. The source review's earlier conclusion that
downstream fallback preparation alone covered this boundary was withdrawn and corrected.

Tests retain the original healthy fallback expectations. New literal probes verify no semantic
transaction executes on prior failure, capture-drop panic still contains the owner, and forgotten
zero/full permits cannot use the healthy exit. One initial zero-envelope fixture incorrectly used
the legacy zero-rejecting host; it was corrected to the existing zero-capable execution API.
All original failed logs remain retained rather than relabelled as passing evidence.

## Qualification Checkpoint

All commands use Rust 1.95.0, locked/offline resolution, one build job and the existing target.

| Check | Result |
| --- | --- |
| Corrected operation/core/import/postprocess nextest | 2,262/2,262; six existing skips; `6ff4e406-5340-48a6-a55b-ccbf72a081f0` |
| Affected ownership/lifetime/publication doctests | Import 17; operation one positive plus 39 compile-fail; all passed |
| Affected all-target/all-feature strict Clippy | Passed with `-D warnings` |
| Real-format facade control assurance | 9/9; `ab88fff9-2a58-4cf0-828a-95fb79e1ed27` |
| Package formatting | Passed |
| Independent source review | Initial P1 and subsequent control-evidence correction closed; no open P1/P2 at final inspected hashes |
| Asset consumer, all features | 120/120; one existing skip; `e8d0127e-9c89-42a5-ad04-0f5ef6c41291` |
| Ordinary Windows pilot and fresh confirmation | Four 23-lane campaigns; 4,232 successful children retained; independent record review supports a bounded keep |
| Refreshed Linux whole-worker attribution | Six successful profiles; all exclusive sums reconciled |

The detailed local command/hash receipts are `target/review/convergence/r1-*.json` and matching
logs. The independent design/source reports and experiment ledger are under
`.git/baozi-review/performance-convergence/`. Runtime snapshots are identified by file hashes in
each command receipt; the source commit and immutable ordinary worker will identify the measured
candidate. The measurements below qualify this bounded improvement; the full convergence and
release gates remain open.

## Confirmed Ordinary Result

Implementation is `a54d39dcd0862b12d292e7541cab0bf5db92986c`. Ordinary workers use the same
release/System allocator policy with no profiling or allocation interception. Each campaign
retains twenty alternating AB/BA pairs after three warmup pairs per lane. The initial campaign
showed material control drift, so both the old and R1 binaries were measured again in separate
fresh campaigns. No original observation was discarded.

These are memory-input medians in milliseconds from the confirmation. Each row is its own
endpoint, not an additive pipeline component.

| Task / endpoint | Old Baozi | R1 Baozi | R1 Assimp | Baozi change |
| --- | ---: | ---: | ---: | ---: |
| Repeated OBJ 16,384 pipeline | 90.84085 | 75.92985 | 9.40610 | -16.41% |
| Grid OBJ 128 pipeline | 196.20450 | 169.84830 | 25.53880 | -13.43% |
| Shared glTF 32 pipeline | 5.31255 | 4.70510 | 0.33045 | -11.43% |
| Binary STL repeated 1,024 pipeline | 4.54035 | 3.07090 | 0.23670 | -32.36% |
| PLY points 1,024 pipeline | 1.86260 | 1.64570 | 0.74220 | -11.65% |
| F64 PLY neighbors pipeline | 1.32655 | 1.30555 | 0.56960 | descriptive |
| Repeated OBJ normal preparation | 48.78930 | 37.79115 | 1.04010 | -22.54% |

F64 PLY's pipeline change is smaller than its candidate IQR and is not credited as a material
gain. PLY points import still has a 22.37% Assimp control shift; that does not describe its
separate pipeline control, which changes 1.14%. Report controls per lane rather than describing
the whole host as stable.

This is not a universal no-regression result. F64 PLY extraction changes 12.60 to 14.35
microseconds (+1.75 microseconds), with old/new IQRs of 1.60/0.70 microseconds and overlapping
ranges; the first campaign also moves in this direction. glTF and PLY points extraction rise
4.71%/4.67%. Retain these short-endpoint observations and their unresolved causal attribution.
They receive an explicit intermediate disposition alongside the material import/pipeline gains,
not an exemption from the final per-endpoint convergence gate. The aggregate decision's empty
regression list is not a per-lane assertion. No timing or historical budget limit is widened.

Across confirmation lanes, whole-child working-set medians rise 0.030–1.139%; private-commit
deltas range from -16 to +36 KiB. Neither metric rises 5% in a compared lane. These are different
whole-child lifetimes from Linux RSS or heap peaks and do not establish a heap reduction.
The largest pipeline quotient remains glTF at about 14.24x. The roughly 80–110x short no-change
preparation quotients also remain open, with explicit dispersion limits on their small controls.
This candidate is kept as an intermediate improvement; it does not meet release acceptance.

## Refreshed Attribution

The separate Linux symbolized worker uses the same committed source and no allocator
interception. Callgrind records whole-worker instructions including setup, checks and teardown;
these figures are not native CPU-time fractions or isolated endpoint timings.

| Case | Previous Ir | R1 Ir | Change |
| --- | ---: | ---: | ---: |
| Repeated OBJ 16,384 | 1,492,581,301 | 1,276,397,733 | -14.48% |
| Grid OBJ 128 | 3,318,117,260 | 2,963,675,201 | -10.68% |
| Shared glTF 32 | 53,681,092 | 45,463,698 | -15.31% |
| Binary STL repeated 1,024 | 50,217,920 | 31,044,583 | -38.18% |
| PLY points 1,024 | 12,801,577 | 9,852,016 | -23.04% |
| F64 PLY neighbors | 2,864,983 | 2,589,461 | -9.62% |

Operation owners still account for about 49–66% of the repeated/grid/glTF/STL workers. The
next bounded experiment removes copied semantic keys from work capabilities and projects them
lazily through the immutable catalogue. Existing observed-permit lifecycle, glTF comparison
accounting, extraction descriptor lookup and read-only preparation startup remain separate owners.

## Retained Identities

- Ordinary R1 worker: `target/assimp-comparison/convergence-windows-r1-build/baozi-probe.exe`,
  SHA-256 `1605e7b2343f86e53ef7bac3fc01e734165d4cc5fba2729575a342e3f972b4ab`.
- Initial R1 report: `convergence-windows-r1/report.json`, SHA-256
  `c3ffe2488aa4c39c86b1db59c9fd4bdb64a429dab5350ac622b4b2ddd72e2f7f`.
- Fresh old control: `convergence-windows-r1-old-confirm/report.json`, SHA-256
  `0c01c780d6d5b653b34b818d4385c9576bbb48c5090dba32540edd29ba41c7b1`.
- Fresh R1 confirmation: `convergence-windows-r1-confirm/report.json`, SHA-256
  `928ee31ea040c8a41d9774c85d6a49d125eb3d5f71fea953bac8756f07af565f`.
- Linux symbolized worker: `convergence-linux-r1-symbols/baozi-probe`, SHA-256
  `e81bde327b0a572df2e11b9335e331ddd1421388d8122bc6ed9590d862bc2d44`.
- Both source archives: SHA-256
  `00f59c83e7f24d68536db94ef2bd06f7a7ce0c1069105e60e6b48274df8df730`.

Report/worker directories above are under `target/assimp-comparison/`. The six raw profiles,
annotations, exact commands and reconciled summary are under
`target/review/convergence/r1-attribution/`. Independent source and exhaustive measurement
reviews remain in the local evidence inventory. Final Windows/Linux 110-lane qualification,
representative heap evidence and the remaining release requirements are not replaced by this pilot.
