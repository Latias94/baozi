# Preserve one import policy owner through entry bootstrap

Status: implemented and qualified; fixed instruction predicate passes. Native
convergence, source retention, main/runtime integration and release remain HOLD.
The fixed 7b8 Windows/GNU ordinary attempts are terminal partial; all missing
campaigns, loads, monitor failure/gap and existing native target breaches remain.
No native convergence, main integration or release acceptance follows.

## Demonstrated repeated work and owner

The additional original 134-byte STL1 pipeline/memory profile records exactly
three incoming calls to `import_ledger_limits`: entry 50,958 Ir, requirements
49,480 Ir and bootstrap 49,280 Ir. Their total 149,718 is included once in the
2,140,762 whole-worker instructions, not added across inline contexts. The
independent raw profile/call-arc review closes with GO. Two later constructor
arcs total 98,760 Ir, an addressed-cost upper bound of 4.61% of this worker,
not a measured saving or native-time prediction. Borrow/move changes and
allocation/teardown interactions must be measured.

The independent source trace `import-ledger-limits-repeated-construction-source-1.md`
confirms each successful read entry constructs a complete 58-dimension
`LedgerLimits<ImportDimension>`. Preparation validates then discards it;
requirements and bootstrap reconstruct the same policies. Each owns four Vec
buffers; actual allocator calls are not inferred from that buffer count.
Bootstrap genuinely maps import policies into a distinct 62-dimension
`LedgerLimits<StoreDimensionKey>` for the real operation. Preserve that owner.

## Selected seam and rejected alternatives

Keep the entry's original Result and initial error-injection seam. In
`prepare_import_entry`, bind the successful value as `import_limits`, then
create the existing physical account. Requirements borrows that immutable
owner and returns its original checked scalar requirements. Bootstrap then
consumes the same original owner; no borrow escapes to the operation or parser.

Change private interfaces to:

```rust
ImportStoreBootstrapRequirements::for_node_scene(
    import_limits: &LedgerLimits<ImportDimension>,
) -> Result<Self, ImportStoreBootstrapError>

try_import_store_bootstrap(
    import_limits: LedgerLimits<ImportDimension>,
    requirements: ImportStoreBootstrapRequirements,
) -> Result<ImportStoreBootstrap, ImportStoreBootstrapError>
```

Use the original owner's `operation_live_byte_limit()` for the startup outcome
policy instead of raw `ResourceLimits.max_live_bytes`; original construction
makes them equal. Remove both internal import-policy reconstructions. Both
private functions otherwise use the existing checked algorithms and return
types. Keep one genuine Store-keyed policy construction and its real allocation,
evidence, canonical/protected work budgets and final operation owner.

| Alternative | Decision |
| --- | --- |
| Original invocation-local owner, borrow then consume | Selected; removes demonstrated duplicate construction without state replication |
| Borrow in bootstrap and drop at caller | Valid but unnecessarily extends the temporary past the existing bootstrap disposal point |
| Request-global cached owner or Arc | Adds lifetime/sharing/allocation and effective-limit invalidation concerns; unnecessary |
| One Copy counter/table cache or proof provider | Adds a second authority or framework; unnecessary |
| Move import-typed buffers into Store-keyed policy | Different domains and ownership/evidence; rejected |

## Semantics and failure ordering

Preserve initial policy validation, physical account creation, requirements
preflight, bootstrap plan, and actual bound operation start in that exact order.
Keep typed initial LedgerLimits errors and their source chain before IO,
progress, tracing or parser callbacks. Keep surviving initialization codes for
account unavailable, bootstrap rejected, plan rejected and start rejected.
No initialization failure acquires an operation-failure owner. Runtime failures
retain the original operation, domain and physical evidence custody.

Requirements still checks complete policies, aggregate attempted/consumed
arithmetic, canonical structural capacity and protected failure allowance.
Bootstrap still independently validates the checked ordinary/protected budget
partition and completeness before mapper `expect`. Do not remove those checks
because requirements read the same original owner: private tests intentionally
provide independent inconsistent requirements.

Preserve all individual dimension fields, operation/evidence/live ceilings,
shared-account quotas and selected-format runtime tightening. No constructor
unwrap, clone, unsafe, added callback/catch boundary or dummy numerical owner.
The original import owner lives longer across account creation and ends inside
bootstrap; the removed reconstructions lose their independent allocation-failure
points. Exact OOM occurrence and transient process memory are not equivalent
by definition. Preserve surviving error classification and deterministic
precedence; qualify actual memory/allocation signs separately.

## Scope and smallest meaningful tests

Production write scope: `crates/baozi-import/src/session.rs` and `src/store.rs`.
The complete eleven-path Rust allowlist includes these two plus nine test-only
caller/import migrations under `crates/baozi-import/src/`: `dependency.rs`,
`detection.rs`, `metered_storage.rs`, `registry.rs`, `scope.rs`,
`scope/ownership_tests.rs`, `resource_graph/mod.rs`,
`session/retained_output_tests.rs`, and `store/scene_activation/transition_tests.rs`.
Update their private test callers to build one actual import owner, borrow for
normal requirements, then move into bootstrap. Keep deliberately inconsistent
requirements independent. Do not add test-only constructor counters or a new
fixture layer. No ImportPlan/public parser, dynamic/finite root, resource graph,
digest, reader, quota schema, benchmark consumer or public API changes.

Retire private `ImportStoreBootstrapError::ImportLedgerLimits` and its Display /
source arms after removing its two constructors. Move the remaining
`import_ledger_limits` store import to test-only scope. Keep `LedgerLimitsError`
for the genuine Store-keyed failure and keep production ResourceLimits use in
scene validation. Public initialization errors and the initial Result seam remain.

Reuse existing typed initialization/no-side-effects, aggregate-overflow,
exact attempted/consumed budget mismatch, startup evidence denial, zero canonical
capacity, complete-vocabulary mapping, sole-owner startup, shared-account release,
root-entry quota/control and published/failure storage/unwind tests. Extend
existing assertions only when the owner seam exposes a genuine missing behavior.
Document source-order coverage for requirements overflow before Store mapping;
do not invent allocation or account failure injection merely to count calls.

Extend the existing aggregate-overflow test to assert requirements rejects the
same actual policy owner with `StorePlan(SizeOverflow)` before moving it into
bootstrap with its independent requirements. Add one incomplete-owner regression,
omitting late `DependencyAcquisitionWork`, to assert exact typed missing-policy
rejection in both requirements and direct bootstrap. Use independently valid
complete-policy requirements for that bootstrap; retain completeness before
mapping `expect`. Successful `LedgerLimits::new` alone proves neither complete
import vocabulary nor nonoverflowing aggregate policy.

Freeze actual inputs and run affected Import tests/docs/strict Clippy/fmt serially,
then original seven workspace/independent consumer gates. Independent source and
actual QA association reviews are required. Builds remain current repository /
existing target, Rust 1.95, Cargo jobs1, nextest threads1. Preserve user notes.

## Fixed experiment and acceptance

Control is qualified 7b8, candidate is this isolated policy-owner change. Keep
original six pipeline/memory GNU opt3/debug1 tasks, exact source/binary/config
bindings and full literal non-clock equality preserving numeric spelling/types,
work and output. At least one glTF1/glTF32 must decrease whole-worker Ir by >=1%
(`100*candidate <= 99*control`), and no one of six may increase >1%
(`100*candidate <= 101*control`). This fixed setup targets per-import startup;
do not choose a different winner predicate after seeing results.

Also repeat the separate original tiny STL locating profile to confirm the
constructor calls reduce from three to one, with no predicted native ratio.
That is attribution, not a replacement selection case. Retain all adverse signs,
warnings, errors and missing data. Failure rejects or separately redesigns the
candidate; do not widen to request-global caching, mapped-policy elision or
pipeline admission changes within the same measured experiment.

Original 14-case/110-lane Windows/GNU ordinary comparisons, 3 warmup pairs /
20 retained fresh AB/BA pairs and whole-child memory targets remain mandatory.
Separate representative calibrated heap attribution must verify allocation and
rounded-peak changes without claiming exact RSS/application heap or universal
interception. Prior finite heap evidence is not attributed to this new source.
No source-retention, main integration or release follows from diagnostic PASS.

The original draft SHA and three required scope/guard/retirement corrections are
retained in `import-policy-owner-plan-review-1.md` (AMEND). This revision adopts
all of them without a new gate or architectural expansion. Source and actual QA
reviews will verify implementation against these obligations.

## Qualified implementation checkpoint

Source `bdc92b3fa5ca523a99920ff980b909ecfe6d61e9` implements the selected
borrow-then-consume owner seam, retires both duplicate constructions and their
private error branch, and migrates all nine test-only caller files. Source1
passes 488 affected tests and 49 doctests, strict Clippy/fmt and the original
seven gates: 3,959 workspace tests, 218 doctests, 138 consumer tests, both
strict checks and both formats. Existing skips remain. All 1,059 frozen, QA,
current, archive and committed input hashes match. Independent source and
actual QA/source-commit reviews close with GO.

The [complete diagnostic report](../reviews/2026-11-08-import-policy-owner.md)
retains the twelve-row export and extra tiny-STL attribution. Six fresh
whole-worker instruction pairs all decrease; glTF1 -1.72953523% meets the
fixed selection threshold, while F64 PLY decreases 3.97271493%. All literal
non-clock outputs match and all six pass the integer regression ceiling.
Independent actual evidence review closes with GO and preserves both grid
warnings. The tiny task confirms three constructor calls become one; its
separate earlier-control comparison is descriptive, not native timing.
Ordinary Windows/GNU comparison and calibrated heap qualification remain
separate requirements; no candidate native or memory result is inferred.
