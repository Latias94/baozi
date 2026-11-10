# Reuse one layout within a glTF schema-field query

Status: source implemented and qualified; fixed instruction predicate passes. Native
convergence, local-main runtime integration and release remain HOLD under
the [convergence plan](2026-11-03-0900-perf-assimp-convergence-plan.md).

## Problem and evidence

`Decoder::field` checks Object kind, then visits immediate members in source
order. Its sole `key_eq` helper currently binds the same singleton WorkUnits
layout for every candidate, admits a child, compares the original escaped
scalar prefix and takes the child's prescribed healthy exit checkpoint.
Repeated queries traverse the object again; no persistent key-name index is
retained after JSON admission.

In the qualified3da glTF32 diagnostic, disjoint physical Self-row labels for
WorkLayout binding and invocation binding total2,026,113 and326,878 Ir out of
25,590,778 whole-worker Ir. These are all-caller labels including inline work,
not measured field-only cost or a savings forecast. The design review
independently reproduces these sums. Whether field-local reuse reaches the
fixed improvement threshold remains an experimental question.

## Selected design and alternatives

Keep one local `Option<ParseWorkLayout<'op,1>>` in each `field` call and lend
it to `key_eq`. After reading that candidate's original immutable raw span
and amount, the helper initializes the layout on its first call, then reuses
the same operation-branded mapping for the remaining candidates. Empty
objects invoke no helper and add no binding boundary. Wrong-kind validation
stays first. The existing layout is Copy mapping metadata, not an owner,
quota allowance or copy of mutable row counters.

| Alternative | Consequence | Decision |
| --- | --- | --- |
| One lazy layout per nonempty query | Removes repeated stable catalog/domain setup while keeping every child and real comparison | Selected bounded experiment |
| One replenished child per query | Changes channel lifetime, action-debt ownership and entry/close/control boundaries | Separate deeper design |
| Retain authoritative name evidence on actual index rows | Changes index/storage admission and collision-safe lookup authority | Future structural alternative |
| Memoize ValueId or owned error | Skips later comparison admission, physical work and control | Invalid under the current contract |

Production changes belong in `document/decode.rs`. A private test module may
hold the one missing behavioral quota fixture. Add no public Import/Operation
API, provider, persistent Document state, decoded key strings, second field
table, replacement numerical owner, new allocation or general executor.
Remove the per-candidate binding path rather than retaining a parallel adapter.

## Stable mapping and live checks

Within this legal exclusive root, domain/catalog identity, WorkUnits membership,
ordinary grant and vocabulary remain fixed. WorkUnits has no per-item limit.
The field comparison cannot seal ordinary work, activate protected work,
replace rows or mutate the catalog; poll callbacks do not receive that root.
Later successful rebinding therefore cannot discover a uniquely new stable
mapping rejection. This does not justify arbitrary cross-operation caching.

Keep the original child/piece checks of current callback reentry, terminal
cause and invocation running state. Every candidate still admits and runs
its quoted raw key width plus one. Attempted quota checks, immediate consumed
completion, pending/remaining integrity, channel close, action costs and the
healthy exit checkpoint remain unchanged. Mutable row policy/counters and
control state are never hoisted into the layout.

Keep source order, first match, escaped Unicode aliases, early scalar mismatch,
extra-scalar rejection and nested error structure. A physical comparison's
inner format error still completes its work, closes/polls the child, then
materializes the original owned failure. Propagate control/ledger/admission
errors immediately. No fake comparison, digest/length shortcut or result-cache
hit may replace a real visit. Each later query starts its own local layout.

## Behavioral and integrated qualification

Reuse existing escaped-name/presence, unknown-values/extras, null/default,
resource, cancellation, work-callback, ownership and consumer tests. Add one
real admitted escaped-name repeated-query quota regression. On
`{"na\u006de":0}`, a successful comparison charges12 WorkUnits. With only
one comparison's attempted headroom after actual JSON admission, the first
query succeeds; the second rejects attempted admission, preserving consumed
work and the recorded over-limit attempted prefix. Use actual original
index/output/account owners and existing registry/control facilities. Do not
add bind-count hooks, malformed admitted-key generators, fake counters or
a new fixture framework. Add a cancellation variant only if existing tests
cannot exercise the same healthy comparison-exit boundary.

Freeze the actual source and run affected glTF nextest/docs, strict all-target/
all-feature Clippy and fmt. Then run the original seven workspace/independent
consumer gates and independently review the source, error/control order and
actual qualification evidence. Preserve failures as distinct numbered attempts.
All builds remain in the current repository and existing target, Rust1.95,
Cargo jobs1 and nextest threads1. User notes and retained evidence are untouched.

## Fixed measured predicate and disposition

Use the already qualified3da text-emission source and immutable workers as the
control for this new experiment. Match generic CPU/System allocation,
opt3/debug1 and the same six original pipeline/memory tasks. Require complete
literal non-clock JSON equality, preserving numeric spelling/types, work and
output. At least one of glTF1/glTF32 must reduce whole-worker Ir by>=1%; no
one of six may increase>1%. Decide with exact integer cross products,
`100*candidate <= 99*control` and `100*candidate <= 101*control`. Preserve every
adverse sign, warning and failure. A failed predicate rejects this candidate
without silently widening it to a longer child or persistent index.

Diagnostic selection is separate from native/source retention. The original
14-case/110-lane Windows/GNU protocol, three warmup pairs plus20 retained
fresh AB/BA pairs and whole-child memory targets remain required. The3da
[native attempt1](../reviews/2026-11-08-closed-text-emission-native.md) has no
candidate ordinary interval; retain its partial records and immutable workers.
Functional/Callgrind results do not replace missing native evidence. Mapping
reuse intends no allocation change; attribute any observed change before
acceptance. Commit qualified source locally with Conventional Commits; main
integration and release remain subject to complete convergence.

The original draft and bounded design GO are retained privately as
`gltf-field-layout-plan-draft-1.md` and `gltf-field-layout-plan-review-1.md`.
The latter includes the stable-state proof, minimal test requirement and
verified attribution limits; it approves this candidate design only.

## Qualified implementation checkpoint

Candidate `7b8c5f3d2559085967dcdfd4098130c77bd877ce` implements the selected
query-local lazy mapping and the real repeated-query quota regression in
exactly two Rust paths. Source3 passes 537 affected glTF tests and the original
seven integrated gates: 3,958 workspace tests, 218 doctests, 138 independent
consumer tests, both strict Clippy gates and both formats. Existing manual
skips remain. All 1,059 frozen, QA, current and committed archive inputs match.
Independent source/amendment and actual QA association reviews close with GO.
The [implementation and instruction report](../reviews/2026-11-08-gltf-field-layout.md)
retains both failed fixture attempts and their narrower corrections.

Twelve fresh matched GNU opt3/debug1 profiles complete the six original tasks
with full literal non-clock equality. glTF1 decreases 2.32029303% and glTF32
decreases 0.77512688%; other changes remain near zero, including STL +7 Ir and
repeated OBJ +227 Ir. The original exact-integer predicate passes without
widening scope. Independent raw evidence review closes with GO and retains
both grid brk warnings. Ordinary builds and original Windows/GNU campaigns
remain separate qualification; no native or source-retention decision follows
from these diagnostic results.

The [ordinary native attempt](../reviews/2026-11-08-gltf-field-layout-native.md)
is now terminal partial on both platforms. Windows C0/N0/C1 each complete all
110 lanes, but N1 does not start. All three measured campaigns retain nine
pipeline breaches above 3x Assimp; the single candidate has 31 endpoint
medians at least 10x. Whole-child memory medians stay below 3x. GNU functional
matrices pass, but all ordinary opportunities remain unstarted. Its failed
Windows monitor, recovered observation gap and original worker continuation
remain explicit. Independent actual Windows/GNU and recovery reviews close.
The missing candidate repetition and GNU intervals prevent native acceptance.

An [additional tiny-STL locating profile](../reviews/2026-11-08-small-stl-fixed-cost-attribution.md)
identifies three actual import-policy constructions on one successful read.
Independent raw call-arc and source traces confirm entry validation discards
the first owner and requirements/bootstrap reconstruct it. The next bounded
design is under review: retain that original immutable owner, borrow it for
requirements, then consume it in bootstrap while preserving the genuinely
different Store-keyed policy owner. No saving, allocation or native result
is inferred before implementation and measurement.
