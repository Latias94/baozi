# glTF schema-field layout reuse

The fixed instruction-selection predicate passes: shared glTF1 decreases
2.32029303% and shared glTF32 decreases 0.77512688% in whole-worker Ir.
All six complete non-clock output pairs match. Native convergence,
source-retention, local-main runtime integration and release remain **HOLD**.
This report provides no candidate native-time or memory result.

The experiment follows the [bounded field-query plan](../plans/2026-11-08-1010-gltf-field-layout-plan.md).
Its control is the qualified [actual-row text-emission source](2026-11-08-closed-text-emission.md).
That source's [partial native attempt](2026-11-08-closed-text-emission-native.md)
remains separate evidence; its missing candidate intervals are not filled by
this instruction experiment.

## Change and ownership

Candidate `7b8c5f3d2559085967dcdfd4098130c77bd877ce` changes exactly
`crates/baozi-format-gltf/src/document/decode.rs` and its new private
`decode/tests.rs`. The immediate parent is precommit QA base
`8364cbb2d291a76bd2a87260d3018d7bb8a4a035`. Control is retained
`3da88939a91326371a5634b2421fd22e0153aafe`; intervening commits add only
documentation. Production `decode.rs` is the only changed original input.

Each `Decoder::field` call keeps one local optional WorkUnits layout.
`key_eq` initializes it lazily after reading the first candidate's original
raw key span and amount, then reuses it within that query. Wrong-kind
validation remains first, and empty objects add no binding. The layout is
operation-branded Copy mapping metadata. There is no new mutable counter,
owner, persistent document index, decoded key allocation or public interface.

Every candidate still performs its real escaped-scalar comparison in source
order, with the original child admission, run, immediate completion and exit
checkpoint. Attempted limits, consumed/pending checks, callback reentry,
terminal-state checks, first match and owned failure materialization retain
their order. Each later query starts a fresh local mapping. Successful layout
reuse does not grant quota or cache a field result.

The independent source review and source3 calibration amendment close with
GO. They establish stable mapping within this exclusive operation and retain
all mutable checks at the original child calls. They do not authorize arbitrary
cross-operation caching or a longer-lived child.

## Actual correctness qualification

Frozen source3 has 1,059 input hashes. Affected qualification passes 537 glTF
tests, with no skips, strict all-target/all-feature Clippy and format. The
glTF doctest command succeeds with zero actual tests; it adds no doctest
coverage. Original final seven gates then pass:

- 3,958 workspace tests, including two slow passes and six existing manual skips.
- 218 workspace doctests.
- 138 independent consumer tests, with one existing manual scale skip.
- Both strict Clippy gates and both format checks.

The new fixture uses the actual registry, forced format, admission, index,
root and output/account owners on `{"na\u006de":0}`. After observing admission
baseline B, each real lookup charges 12 WorkUnits. Calibration requires two
successful queries with exact attempted/consumed/pending counters. With only
B+12 attempted capacity, the second query rejects at attempted B+24 while
consumed stays B+12 and pending stays zero. It verifies the typed Attempted /
WorkUnits limit, Ledger terminal class, and zero live storage/reservations.

Two failed attempts remain retained. Source1 fails compilation with two
E0639 errors from external non-exhaustive ResourceLimits literals. Source2
fixes those constructors, then passes 536 tests and fails the new calibration:
its unnecessary u64::MAX work limit overflows the aggregate bootstrap budget
before parsing. Source3 uses normal default calibration capacity; the actual
B+12 limited run and every assertion remain. Production is byte-identical
across all three sources. No failure is relabeled, skipped or removed.

Independent actual QA review closes with GO. All 1,059 before/after/current,
archive and committed input hashes match. Command order, flags, jobs1 and
threads1 equal the original seven-gate reference. All seven existing manual
skips and six explicit 1 MiB stack tests are retained. Expected caught-panic
stderr and repeated success-output tails are counted once. Five current user
note identities remain unchanged.

QA executes before the exact two-file source commit, whose parent is the
recorded QA HEAD. The archive binds those tested bytes to the commit; this
does not claim a postcommit rerun, general GNU runtime or wasm qualification.

## Matched instruction experiment

| Role | Source | GNU symbol-worker SHA-256 |
| --- | --- | --- |
| Control | `3da88939a91326371a5634b2421fd22e0153aafe` | `ccb81771bc7b4fcca62f1d12fce9b86c54ec8c2ca45afb57fa51c2bbf686e333` |
| Candidate | `7b8c5f3d2559085967dcdfd4098130c77bd877ce` | `14c41c745814ee2116b55691d1b649e86f1120f2dfef5b22733bfa0048a14ef4` |

Both are matched Rust 1.95 opt3/debug1, generic CPU/System workers, with empty
consumer features and no allocation instrumentation or CPU/LTO override.
Recorded compiler, build argv, environment overrides and actual consumer
profiles are equal. Source/builds remain in the current repository and its
existing target, including GNU `/mnt/f/SourceCodes/Rust/baozi`. These debug1
workers are ineligible for ordinary native timing.

Twelve fresh Callgrind workers run the six original pipeline/memory tasks,
with 24 retained Self/inclusive annotations. Original corpus, fields, normal
policy, work bounds and collector helper are unchanged. Complete paired JSON
matches after removing only `stage_ns`, preserving numeric spelling/types,
work, output and metadata.

| Case | Fresh control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| Shared glTF1 | 5,868,957 | 5,732,780 | -2.32029303% |
| Shared glTF32 | 25,590,778 | 25,392,417 | -0.77512688% |
| F64 PLY neighbors | 2,500,439 | 2,500,424 | -0.00059989% |
| Repeated binary STL1024 | 12,218,418 | 12,218,425 | +0.00005729% |
| Repeated OBJ16384 | 711,820,358 | 711,820,585 | +0.00003189% |
| Grid OBJ128 | 2,107,577,838 | 2,107,577,793 | -0.00000214% |

Changes use these fresh same-campaign controls, not earlier counts of the
same binary. Selection uses exact integer cross products: at least one of
glTF1/glTF32 has `100*candidate <= 99*control`; all six have
`100*candidate <= 101*control`. glTF1 meets the first test; all six meet the
second. Full completion/equality is required separately. Floating-point
percentages are descriptive and do not decide the 1% boundary.

The [complete 12-row CSV](2026-11-08-gltf-field-layout-instructions.csv) retains
both roles, exact source/binary identities, every count and adverse sign,
literal equality and integer-predicate flags. Both grid profiles retain
`brk segment overflow in thread #1` at `0x5419000`, manual-limitations notices
and suppressed-further-instances notices. Whole-worker Ir includes setup,
validation, observation, serialization and teardown. It is not stage time,
Assimp Ir, native speed, RSS or heap evidence.

## Next disposition

Independent actual instruction evidence review closes with GO. All twelve raw
summary/footer/stderr/Self totals, twenty-four annotations, seventy-two command
streams and binary/source/configuration associations reproduce. The six literal
non-clock pairs and exact integer selection tests reproduce, including both
adverse changes and both grid warning addresses. The review verifies exit0 for
all thirty-six child commands. Its supervisor terminal0 observation comes from
the coordinator's tool result rather than a separate retained supervisor log;
the report preserves that evidence limit.

The original fourteen-case/110-lane Windows/GNU comparisons still require
three warmup pairs and twenty retained fresh AB/BA pairs, complete controls,
repeats, load records and comparable whole-child memory. Ordinary workers and
the native launcher are being qualified separately. The private run receipts
remain under `.git/baozi-review/performance-convergence`; raw profiles remain
under `target/assimp-comparison/gltf-field-layout-instructions-1`.

The bounded mapping change intends no allocation change; no new candidate
heap or memory result is inferred from earlier finite-consumer measurements.
Collection and diagnostic selection grant no source-retention, native,
local-main integration or release acceptance.
