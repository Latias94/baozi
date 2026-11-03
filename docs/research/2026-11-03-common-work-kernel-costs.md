# Execution Cost After the Common Work Kernel Extraction

Research date: 2026-11-03. This supports the
[convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md) and the
[cardinality evidence](../reviews/2026-11-03-profile-cardinality-actions.md). The first exclusive
invocation proof establishes one numeric implementation and a compiled ownership boundary.
It does not establish that transitional callers execute efficiently. Three pinned Linux
workers separate that structural change from the subsequent count-only algorithm change.

## Matched Sources and Scope

| Worker | Source | Symbol binary SHA-256 |
| --- | --- | --- |
| R7 | `ae11d58a6702a4f2a54910b277043d9aa9dbfbc2` | `eb15424d6013c22c0353a01ddf0d84bb98fef66814cd47be1142e1c567ce44f1` |
| Common kernel first proof | `45a09d19d948600d9b2a4261eb2e3394fe594ef1` | `7cb1dbd4f5c89bfd0a3db01f6734e61a8dae2883ca7da0565c4c39d0e40f267b` |
| Cardinality actions | `0001bd8ab5ccf0c6c1b16754a1c2016ad8a76684` | `770e57de18310100577b53b03d13db4c39ae85aad4d3f949d7b25bb2d83874f1` |

All run the same eight memory-IO pipeline tasks with Rust 1.95.0, release opt3, debuginfo 1,
generic x86_64, System allocator and no allocation instrumentation. Every profile's raw total
equals its exclusive flat sum. These are whole-worker instructions, including setup, checks
and teardown; profiled elapsed times are not ordinary benchmark observations. Source/build
paths and generated-code boundaries differ. Do not infer Windows code generation, stage CPU
percentages or heap behavior from these profiles.

The exact 45a-to-cardinality contrast removes actual actions: glTF1/8/32 counting changes
1,646/2,381/4,925 -> 33/47/95, while all other semantic-runner calls remain 438/1,369/4,561.
Whole-worker Ir falls 17.735%/14.067%/10.908%. Earlier R7-to-45a whole-worker Ir instead rises
1.448%--8.934% across the eight cases. Version totals establish those contrasts; the following
actual call edges identify a bounded contributing route.

## Unchanged Actions, More Authority/Control Instructions

The repeated-OBJ worker calls the selected-edit typed-storage runner exactly 262,189 times
in both R7 and 45a. Its function-exclusive cost remains 30,462,585 Ir. The normal kernel,
numeric source, selected output and actual scalar-action count are unchanged. A disjoint
shallow cut of that runner records:

| Runner component | R7 Ir | 45a Ir | Difference |
| --- | ---: | ---: | ---: |
| Self, preserved-fault child and fixed reads | 31,773,725 | 31,773,725 | 0 |
| Before-action control | 9,479,764 | 14,984,709 | +5,504,945 |
| Admission | 57,943,769 | 66,596,006 | +8,652,237 |
| Completion | 40,377,106 | 51,389,044 | +11,011,938 |
| Permit finish | 36,968,649 | 38,017,405 | +1,048,756 |
| Completed-action control | 3,932,835 | 8,652,237 | +4,719,402 |
| **Runner inclusive** | **180,475,848** | **211,413,126** | **+30,937,278** |

The work/control child increments are executed at unchanged actual action counts. They are
not additional normal mathematics. Do not add this nested runner delta to whole-worker or
normal-preparation deltas as independent savings. Existing normal generation retains its
114,696 emitted action calls and 12,059,375 exclusive Ir; its authority descendants grow.

R7 and 45a also perform the same 14,228 glTF32 profile comparisons and 221 cadence-poll
children. Replenishment grows by 24 Ir/comparison and completion by 18, a total 597,576 Ir
increase on these two edges. The later cardinality revision changes neither these calls nor
their cost. Binding frequencies likewise remain unchanged; moving symbols into a kernel
does not by itself remove actual binding work.

## Concrete Composition Boundaries

The source/line/call investigation identifies these current boundaries, using 45a source:

- `ledger/work.rs:94` constructs short catalog/rows/header views for numeric transactions.
  The successful path has actual view-assembly instructions, though attributed line cost is
  not a causal saving estimate and no separate constructor call is emitted.
- Admission returns a frame and packs it with a copied bound layout into an owning permit.
  Parser admission similarly repacks its frame. The aliasable adapter checks active/faulted/
  amount state before borrowing, and the shared kernel checks its own authoritative state.
  Resolution aggregates remaining work at both composition boundaries.
- The same before/completed action counts now cross a shared `checked_action_debt` function
  boundary. The debt rule existed before; a newly emitted call is not new cancellation work.
- The import `with_work` adapter has observed extra memcpy calls: repeated OBJ goes from
  49,157 to 131,094, grid from 98,309 to 379,165. Ir alone identifies neither the copied objects
  nor byte counts or an optimizer mechanism. These libc costs already belong to the flat
  non-operation partition and are not added again.

Lazy registered-key projection remains lazy: this is not evidence that successful work now
materializes diagnostic keys or repeats key comparisons. Normal contribution order, area
weights, robust arithmetic and F32/F64 predicates are not replacement candidates on this evidence.

## Owner-Level Direction

Migrate a complete real caller region to the already proven exclusive owner. Lend one durable
mutable numeric view with disjoint storage/control services; retain host-owned frames instead
of returning aliasable work permits. Bind at the actual region frontier and reuse that layout.
Delete obsolete forwarding, repeated adaptation and overlapping checks only where exclusive
ownership establishes the replacement invariant. Keep one numeric and debt implementation.

This is a concrete removal candidate, not a measured benefit of the future exclusive path:
these profiles execute transitional callers. The exclusive action still creates a frame and
checks debt, so adoption cannot be credited with removing all control/frame costs. Preserve
current policy/role/error order, denied-body exclusion, immediate actual prefixes, panic and
forgotten-owner handling, real physical leases, and each caller's entry/due/final checkpoints.
The real import bridge's original cause and registry/session exits must qualify before wider
parser migration. No blanket inline hint, second numeric backend or weaker normal task follows.

The full read-only investigation is retained as
`.git/baozi-review/performance-convergence/profile-cardinality-attribution-investigation.md`,
SHA-256 `c51806a28512cb7d83b03279ed16cd53faaeb5367b3a758b82a71697de78acb7`. Raw/annotated
profiles remain under the three attribution directories named in the cardinality receipt.
Native endpoint costs and the all-endpoint 3x release gate remain open.

Independent bounded raw review confirms the repeated runner's unchanged 262,189 calls,
exclusive cost, 30,937,278-Ir shallow-cut increase and the glTF32 4,925-to-95 actions. It parses
the original compressed records for seven selected profiles and verifies their raw self sums,
rather than deriving edges from this report's table. It finds no material count/partition issue
and retains the causal limitations above. The frozen receipt is
`.git/baozi-review/performance-convergence/kernel-cost-attribution-review.md`, SHA-256
`6b543f2acda3abb4bcee9015b17b6d77aaf605d92848c9f6b7babc3c331484b8`.
