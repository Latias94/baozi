# Expose the existing parser completion to its actual caller

Status: independent plan-readiness review closes GO. The preceding ASCII
actual heap and document reviews are closed at `b02d7c3`; this experiment starts
from that completed documentation HEAD. The hypothesis remains unmeasured.
The original convergence goal and all release/main gates remain active.

## Located boundary and hypothesis

Qualified source `d81bbc5d51234e5360ba4db61b432a4ca8850851` is the control.
Its retained GNU opt3/debug1/System image has SHA-256
`372b6789b8acfafa31c0e670b568ef3a2d1d16f7e3d148bdb3de06d73e7b9198`.
`d81-parser-kernel-inlining-premise-1.md` binds current source, profiles and
symbol inventory. Grid/repeated OBJ whole-worker Ir is 1,988,095,712 /
709,672,211. The shared outlined `ParserFrame::complete` receives
1,727,858 / 311,336 incoming calls; its all-files Self is 149,598,851 /
36,392,356 Ir. Those are locating observations across all consumers, not
removable overhead, native CPU share or the expected gain.

The physical caller already projects a returned `[u64; N]` into the existing
completion. Exposing this body to that caller may let the compiler retain
amounts/results in scalars or simplify temporary transport. The hypothesis
concerns that existing caller/callee boundary only. `N` is already a compile-time
constant; runtime catalog positions/alias order are not claimed constant.
Generic source is already available for monomorphization. A plain hint changes
the compiler's placement decision, not generic-body accessibility.

## Alternatives and exact source scope

| Alternative | Assessment |
| --- | --- |
| Keep qualified d81 unchanged | Valid comparison control and fallback. |
| One plain `#[inline]` on `ParserFrame::complete` | Selected bounded experiment; no body, API, state or ownership change. |
| Also hint direct arithmetic, WorkFrame completion and admission | Do not bundle: hot direct arithmetic is already inlined, other owners need separate evidence. |
| Force inline(always), global LTO/CPU flags, cold splits or a specialized token API | Outside this hypothesis; do not escalate a failed plain hint into these changes. |
| Reintroduce cached allowance or single-piece host | Rejected experiments 292/2bd remain retired; no revival. |

Initial and entire Rust write scope is
`crates/baozi-operation/src/ledger/work/kernel.rs`: one plain `#[inline]`
immediately above `ParserFrame::complete`. Do not change any executable body,
visibility, type, dependency, configuration, algorithm, assertion or test.
No new fixture is justified for an annotation-only compiler experiment.
Inspect the actual diff before freezing and committing.

The source path remains
`run_guarded_prefix -> ParserActionHost::complete -> ParserFrame::complete ->
consume_direct_atomic`. Keep channel validation before zero identity, original
lane overrun, checked alias aggregation in catalog order, current pending
competition and consumed/attempted ceilings, atomic commit, then remainder
reduction. Keep admission, callback extraction, numerical completion, completion
control and error materialization in their existing order. Every unwind guard
and captured value retains its current owner; no new loan spans a user action.

## Qualification and actual-code condition

Once the preceding fixed heap phase and evidence closure end, branch from its
actual completed documentation HEAD while retaining exact d81 build inputs as
control. Preserve the five user notes and all unrelated work. All source/builds
stay in the current repository/existing target; Rust1.95, jobs1 and nextest
threads1 remain. No own build/profiler overlaps ordinary clocks.

Run affected Operation/Import/OBJ nextest, strict all-target/all-feature Clippy
and existing borrowing doctests, then original seven workspace/consumer gates.
Use exact frozen/current/staged/committed/archive identities, retaining every
failed attempt and original manual skip. Independent review may inherit the
unchanged numerical/source proofs but must authenticate the literal annotation
delta and actual final QA/commit association. Do not use the control's passing
QA as execution evidence for the hinted build.

Use matched GNU opt3/debug1/System/generic-CPU/empty-feature diagnostic images.
The immutable d81 control may be reused after source/QA/archive/ELF/configuration
revalidation; run fresh control/candidate processes for every profile.
Reinspect actual hot returned-piece callers in both images. Selection requires
evidence of the changed actual completion boundary and reduced materialized
amount/result transport at those callers, with original numerical checks and
error/control/unwind ownership still present. A missing symbol, moved Self row,
smaller local frame or source annotation alone is insufficient. If the compiler
ignores the hint or actual transport is already scalar, this premise is not
established. Do not invent a new lower-level authority to force success.

## Prospective selection and retirement

Run all seven original pipeline/memory tasks: glTF1, glTF32, F64 PLY neighbors,
binary STL1024, repeated OBJ16384, grid OBJ128 and binary STL1. Require seven
complete literal non-clock pairs equal after removing only `stage_ns`, including
numeric spelling/type, output and work. Literal successful `2N+Q` remains.

The inclusive integer screen is unchanged: at least one original OBJ must
satisfy `100*N <= 99*B`; every original task must satisfy
`100*N <= 101*B`. Here `B` is the fixed qualified d81 control of this distinct
experiment, not an older result chosen after observing the candidate. Keep
the earlier 0452-to-d81 contrast and every adverse result as historical evidence;
do not reclassify its source-speed HOLD as an accepted base improvement.
Whole-worker Ir is not native time or Assimp instructions.

Both the actual-code condition and the integer/output screen must pass to earn
further ordinary measurement. If either fails, remove the annotation by its
exact inverse after checking unrelated work, retain its actual failed evidence,
and reuse baseline qualification only through exact byte identity. Do not keep
a new compiler hint solely because its source delta is small. Do not broaden
the hint set, force always or change selection after inspecting the result.

A selected diagnostic candidate still requires the unchanged Windows/GNU
fourteen-case/110-lane full ordinary protocol: C0/N0/N1/C1, three warmup pairs,
twenty retained fresh serial AB/BA pairs, original precision, fields, normal
policy, input/work/resource/control/failure semantics and child clocks.
Retain all load, timeout, warning, missing/noisy campaign and signed regression
records; a quiet start is not isolation. Comparable whole-child memory and
separate representative fresh tracked heap remain. Reuse existing calibrated
tools and immutable comparator builds only after exact identity/configuration
checks; no needless reconstruction or repeat qualification is implied.

Source-speed keep still requires material same-task improvement beyond observed
dispersion with no unexplained affected regression. Original engineering targets
remain all complete-pipeline medians <=3x same-campaign Assimp, no confirmed
endpoint >=10x, and comparable whole-child peak memory <=3x. No diagnostic
screen, annotation-only review, partial pass or benchmark-boundary change can
substitute. No premature main integration, native promotion, push, PR,
publication/yank or cargo clean is authorized by this experiment.

The private readiness review is `parser-completion-inline-plan-review-1.md`,
SHA-256 `8f7cba5a3bdfa6800b36d12070a20c5f54a72cba54ff8c3c2fba57655c5a9948`.
It qualifies draft SHA-256
`4105142a8510dbf1112728bf0879be91ad21d6922eac33515f4e91bfeb29eeb7`.
This published copy changes only the status above, three spacing corrections
and this provenance paragraph; the source scope, code witness, integer screen,
retirement rule and original ordinary/heap requirements remain identical.
