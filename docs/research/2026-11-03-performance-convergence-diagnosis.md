# Performance Convergence: Current Owners and Refactor Order

Date: 2026-11-03. Baseline `e0a7b387371b71c28bc40f6e2cfd4c1ff5f91726`, runtime
`52446ec8511b7dfba2e5352ce6941558d85e0ae2`. This starts the new
[convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md); it does not reopen
or claim another gain from the completed operation/remap/bucket changes.

## What the Fresh Baseline Establishes

An unchanged ordinary Windows worker completed 23 representative memory-input lanes with
twenty retained alternating pairs after three warmup pairs. All 1,058 child observations
succeeded. The collector checked the supplied build-manifest executable hashes and retained
the same analytic task, fields, precision, normal policy, setup/cleanup boundaries and corpus.

| Endpoint | Baozi median ms | Assimp median ms | Baozi / Assimp |
| --- | ---: | ---: | ---: |
| Repeated OBJ 16,384 import | 37.72885 | 8.03875 | 4.69 |
| Repeated OBJ normal preparation | 49.25445 | 1.07400 | 45.86 |
| Grid OBJ 128 extraction | 15.14110 | 1.05670 | 14.33 |
| Shared glTF 32 pipeline | 5.39230 | 0.33850 | 15.93 |
| Binary STL repeated 1,024 pipeline | 4.73065 | 0.23820 | 19.86 |
| Shared glTF 32 no-change preparation | 0.20105 | 0.00190 | 105.82 |

The large ratio of a short no-change endpoint is not a larger absolute optimization opportunity
than 49 ms of normal preparation. Both remain visible and unresolved. These are independently
timed endpoints; their medians cannot be added or subtracted as an exact pipeline decomposition.
This pilot is not a complete Windows/Linux release comparison.

Report: `target/assimp-comparison/convergence-windows-baseline/report.json`, SHA-256
`b0a155d1531068c06e0adc0a0a4dad87297076aee0d52d7c48531b481b4ff962`. Commands and a read-only
summary are in `target/review/convergence/`. Historical full campaigns remain separately
qualified by the [Windows receipt](../reviews/2026-11-03-windows-final-comparison.md) and
[Linux receipt](../reviews/2026-11-03-linux-b1-and-heap-comparison.md).

## Current Attribution

Three independent source/profile investigations reconciled the current raw Callgrind profiles
and concrete source routes. The whole-worker exclusive totals are 3,318,117,260 Ir for grid OBJ,
1,492,581,301 for repeated OBJ, 53,681,092 for shared glTF and 50,217,920 for STL. Function-owner
grouping assigns approximately 50.21%, 66.84%, 58.87% and 76.42% respectively to the operation
crate. This grouping includes accounting, admission, validation and control. It is neither a
stage decomposition nor a native CPU-time percentage.

The evidence supports these bounded decisions:

1. **Bind canonical semantic execution first.** Every semantic unit currently constructs a
   producer host, checks dimension ownership/membership, binds again, opens a permit, completes
   it and resolves it. Grid has 411,126 such admissions. Three disjoint admission/completion/
   finish child edges cover 552.964M Ir, or 16.66% of this worker. STL's binding plus dimension
   validation alone cover 31.86% of exclusive whole-worker Ir; glTF's cover 26.19%. The replacement
   still needs actual numeric admission/completion and failure/control handling, so none of
   these figures is a predicted saving.
2. **Keep numeric capabilities small and owner-bound.** Existing bound groups carry full
   semantic keys as well as positions and order maps. Measure moving repeated key traffic to
   cold identity/error projection through the immutable owner catalogue. Retain current mode,
   policy, duplicate aggregation, protected-role, pending-headroom and forgotten-permit checks.
3. **Amortize the normal leaf protocol before changing its math.** The current stable-bucket
   kernel performs 573,463 real leaves on repeated OBJ. Begin/Complete runtime paths account for
   388.910M inclusive Ir across normal and nineteen surrounding actions. This overlaps the
   generation route and cannot be added to its parents. Position enum/read wrapping itself is
   only 4.178M exclusive Ir; repeated public attribute lookup is mainly extraction work.
4. **Stop using cold accounting inside glTF relation sorting.** Source-profile finishing
   sorts and deduplicates 810 input link records in shared-32. Its subtree is 18.548M Ir, with
   13,419 sort comparisons calling the cold `charge_work` adapter. Bind the comparison work once
   and complete the actual comparison immediately; only then reassess the sorting algorithm.
5. **Reassess representation changes after those keeps.** OBJ token staging has measured
   repeated binding/allocation opportunities. Borrowed text copying alone has an enclosing
   route of only 0.43% of grid Ir and does not remove both conformance scans. Attribute payload
   materialization is about 3.10%; adoption requires a compatible canonical layout and ownership
   transfer. A broad parser/storage rewrite is not yet justified by those narrower observations.

Outer `Importer::new` plus `Importer::plan` occupy only about 59,500 inclusive instructions,
0.11–0.12% of the profiled shared-glTF/STL workers. Reusing them cannot explain away the current
large gaps. Internal operation startup matters more to very small models and remains a later
separate lifecycle measurement. The frozen complete pipeline continues to include setup.

## Contract and Review Conditions

The first replacement lends one execution region with separate work, storage and control
capabilities. No numeric borrow may span an action or callback. Each action is attempted before
execution and consumed only after normal return, including a returned semantic error. Preserve
callback-capture destruction handling, locally caught panic containment, sticky first-cause
precedence, scoped scratch ownership, current numerical policy, mandatory final control and
single-use completion branding. Amortizing binding or polling never permits speculative work
completion or delayed replay of physical prefixes.

Independent plan review found no P1 blocker. It required two corrections, both adopted: the
final at-most-3x engineering class applies to endpoints as well as pipelines, and existing
affected-package compile-fail doctests run before retaining borrowing/capability replacements.
The 10x endpoint threshold is only an intermediate checkpoint. The user supplied the release
condition; these numerical development gates are explicitly engineering choices.

At this checkpoint no production optimization from the new plan has been retained. Exact
implementation, ordinary before/after results, source review and remaining failures belong to
the subsequent experiment receipts. Local investigations and the plan review remain under
`.git/baozi-review/performance-convergence/` until exported with their evidence inventory.
