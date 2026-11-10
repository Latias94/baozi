# Exact success capacity within a parser read-row loan

Source `292a5630ce596e9630ba70670838f3d278b5088b` passes all seven
correctness gates and independent source/QA association review. Matched
instruction selection fails: neither OBJ improves, and both consume more
instructions. Independent evidence review closes; the production allowance is
removed and its behavioral tests retained. Retirement passes all seven gates
and independent final source/commit association review closes with GO.
Native convergence, source retention, main/runtime integration and release remain
**HOLD**. The [plan](../plans/2026-11-08-1440-parser-read-success-budget-plan.md)
retains the original full comparison and resource/control semantics.

## Actual owner and removed repeated arithmetic

The existing private `ParserReadRow` exclusively borrows the actual WorkEntry
and actual frame remainder. The candidate derives one disposable `u64` exact
success allowance after the original entry validations. For actual consumed C,
pending P, attempted A, consumed limit L and frame remainder R:

```text
b = min(R, min(A,L) - checked(C+P))
```

Either checked addition/subtraction failure gives zero without an entry error,
numeric mutation or callback. Within the exclusive loan, no competing safe
capability can mutate those numeric owners. A completed physical read with
amount <= b immediately updates actual C, actual R and b, with no intervening
fallible action. Positive amounts prove all original arithmetic and ceilings;
zero remains an identity operation even for invalid initial numeric state.

An amount exceeding b uses the original numeric checker on current actual
fields: overrun, zero, consumed overflow, effective overflow, attempted before
consumed limit, then commit. Rejected work does not alter actual accounting or
the allowance. No positive cold success is possible under the exact invariant.
The existing checker remains the source of failure semantics rather than a
second implementation or a new error/proof object.

The new value is derived loan metadata, not an authoritative counter, pending
reservation, precharged work, replay right or deferred commit. Loan drop settles
nothing. Original raw/debt polls end the loan and the next loan derives a new
allowance. Store stopped/debt checks, successful-zero debt, first cause, guarded
unwind, cancellation cadence, UTF-8/raw frontier and exact physical-prefix
commit before reserved writes remain. General multi-row completion is unchanged.
Production changes are confined to the kernel; the second changed Rust path is
its existing parser test harness. No public API, allocator, provider, dependency,
physical reader, format parser or benchmark interface changes.

## Independent test amendment and actual qualification

Source1 passes 1,882 affected Operation/Import tests, 198 doctests and format,
then all seven original gates: 3,962 workspace tests, 218 doctests, 138 consumer
tests, both strict Clippy and both formats. Independent review finds no production
defect but requests stronger tests to isolate frame and attempted ceilings:
the initial exact-frame fixture had A=R, and its attempted-invalid vector A=L.
Those equalities could conceal an omitted independent clamp.

After source1 qualification terminates, the test-only source2 amendment admits
and resolves an unused prefix before the tested frame, making A=6,R=3,L=20.
Another same-loan case starts C=2,P=1,A=4,L=10,R=2: first unit succeeds, second
returns exact ConsumedExceedsAttempted, zero succeeds, and handback retains the
actual successful prefix. The frame fixture rejects its first excess even with
spare numeric credit. Existing pending competition now checks reject-three,
success-two, reject-one in one loan. Initial overflow/invalid states, zero,
overrun precedence and successful prefixes near u64::MAX retain exact errors
and actual counters without an allowance accessor or mirrored checker.

The bounded amendment review closes both independent oracles with GO; production
kernel bytes are unchanged from source1. Source2 independently passes the full
original seven gates: **3,963 workspace tests, 218 doctests, 138 consumer tests**,
both strict all-target/all-feature Clippy and both formats. Two slow passes,
six existing workspace manual skips and one existing consumer manual skip remain.
Expected caught-panic stderr and the repeated post-summary PASS are counted once.
No earlier result is substituted for source2's qualification.

All 1,059 input hashes agree across frozen source2, before/after QA, current
checkout, qualified archive and committed blobs. The exact two-path source-only
commit immediately follows QA HEAD `c88a43719734df7cf0190934d7e0e9fe1c2e3131`.
QA runs before committing those exact bytes, not as a claimed postcommit rerun.
The five current user-note identities match their retained baseline. No tests,
stack bounds, consumer controls, failure ownership or publication checks are
removed or weakened. Independent actual QA/source/archive review closes with GO.

## Retained launcher failure and corrected preparation

The first GNU symbol launcher fails before Cargo or build.json creation because
its QA reference still selects source1/final1, while the candidate archive
contains source2's F1 test amendment. The original assertion rejects that stale
association. The failed symbols1 directory, source archive, original launcher,
error receipt and exact cause remain; this is not a Rust compilation failure.

Corrected builder2 selects actual final2 and fresh suffix2 artifact/manifests,
preserving failed symbols1. Profile2 changes only its candidate symbols2
manifest. A bounded independent review closes those literal changes with STATIC
GO; no assertion, compiler/profile override, source binding or predicate changes.
Native preparatory review also corrects the GNU monitor's predecessor filename
and stale direct final1 reference. Ordinary2 uses its actual self filename,
final2, ordinary2 workers and current protocol2/amendment receipt; heap2 uses
symbols2 and both new ordinary terminal decisions. All original scripts1 and
AMEND findings remain. These corrections create no performance observations.

## Fixed measurement

Use source-bound matched GNU opt3/debug1 System workers with qualified bdc control
on the six original pipeline/memory tasks. Preserve complete literal non-clock
equality, including integer/float spelling/types, work and output. One original
OBJ must satisfy `100*candidate <= 99*control`; every case must satisfy
`100*candidate <= 101*control`. No alternate task or post-result predicate.
Retain every increase, warning, missing row and error. Selection PASS would
qualify further measurement, without predicting a native saving.

## Actual matched instruction result

Both symbol workers use Rust 1.95, GNU opt3/debug1, generic CPU/System and
empty consumer features, uninstrumented and ineligible for ordinary timing.
Candidate GNU source/build stays in the current repository and existing target.
The exact candidate SHA-256 is
`02eec9e44c20e7c203471e62e97b11e185300537ff25185a40e54502aded9b6d`;
qualified bdc control is
`bdd276067e86505b6cf45ac20c2bef917649b925dd3aae098d6cae2b85ddfb1f`.
All original task settings and manifest remain unchanged. Twelve fresh workers
and twenty-four annotations complete, with all six full literal non-clock
outputs equal after removing only `stage_ns`.

| Case | Fresh control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| glTF1 | 5,633,588 | 5,633,609 | +0.00037276% |
| glTF32 | 25,293,353 | 25,292,954 | -0.00157749% |
| F64 PLY neighbors | 2,401,745 | 2,402,009 | +0.01099201% |
| Binary STL1024 | 12,119,290 | 12,119,582 | +0.00240938% |
| Repeated OBJ16384 | 711,721,299 | 712,541,648 | +0.11526267% |
| Grid OBJ128 | 2,107,478,702 | 2,118,606,671 | +0.52802284% |

The unchanged integer predicate fails because neither OBJ satisfies
`100*candidate <= 99*control`. Every case satisfies its independent ceiling
`100*candidate <= 101*control`; passing that ceiling does not repair the missing
required gain. The [complete twelve-row export](2026-11-08-parser-read-success-budget-instructions.csv)
retains both source/binary identities and every increase. Whole-worker Ir includes
setup, validation, observation, serialization and teardown; it is not stage time,
Assimp instruction comparison, RSS or native speed. Source-level arithmetic
hoisting has not earned this candidate's extra loan state and maintenance.

Independent actual raw/build/equality review closes with GO for fidelity and
FAIL for selection. Both builds' exact configuration, source association,
profiles, annotations, 72 streams, analytic facts and full twelve-row export
agree. All five increases and the one decrease remain. Both grid profiles
retain `brk segment overflow in thread #1: can't grow to 0x5419000`, manual
limitations and suppressed-further-instances notices. All 46 unresolved labels
per Self annotation remain included; matching warnings do not establish native
allocation or OOM equivalence. Outer supervisor exits remain coordinator
observations, distinct from independently read recorded commands/hashes.

The same 66 repeated-OBJ and 894 grid loan-entry calls are present in both raw
profiles. Entry Self/incoming Ir increases from 52 to 68 per call. Candidate
also has a separately named completion function, while control has no separately
named completion: inlining/attribution may differ, so absence is not zero work.
Neither outlined costs nor overlapping inclusive rows are summed as net added
instructions. Exact code-generation and native-time causes remain unknown.

No ordinary campaign or new heap worker runs for this failed candidate. The
production field, derivation and fast branch are removed by exact owned hunks;
the kernel matches the qualified bdc Git blob byte-for-byte. Source3 preserves
all four new behavior tests and the strengthened existing pending test. It
differs from source2 only at the production kernel. It independently passes all
seven gates: 3,963 workspace tests (two slow/six existing manual skips), 218
doctests, 138 consumer tests (one existing manual skip), both strict Clippy and
both formats. No source2 PASS is substituted. No reset, restore, stash or
user-note deletion occurs. Source-only retirement commit
`17f74d5a91f042a110b3ad5f7de466f3c6867887` immediately follows its QA HEAD
`292a5630ce596e9630ba70670838f3d278b5088b`; its archive equals all 1,059
frozen/QA/current input hashes. Independent retirement QA/archive/commit review
closes with GO, including the unchanged current user notes. No native benefit
or release follows.

Original fourteen cases/110 lanes, three warmup/twenty retained fresh serial
AB/BA pairs on Windows/GNU, exact source/binary/config identities, separate
representative calibrated heap and actual independent evidence review remain.
Every pipeline median <=3x same-campaign Assimp, no confirmed endpoint >=10x
and comparable whole-child memory <=3x remain the full-goal targets.
The prior [native and heap checkpoint](2026-11-08-import-policy-owner-native-heap.md)
still fails convergence; no earlier missing campaign or contention is repaired.
