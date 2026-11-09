# Preparation selection: retain each resolved primitive once

Source `6203ce29850e18e71834f5010803f12e9cda2d52` passes all seven integrated
source gates and independent production, test and QA/commit/archive reviews.
Its GNU symbol build and original seven-task instruction campaign complete.
The prospective instruction predicate and independent actual-evidence and
compiled-owner reviews pass. Native timing, source-speed retention, main
integration and release acceptance remain unestablished.

The [reviewed plan](../plans/2026-11-09-1535-prepare-selection-owner-plan.md)
replaces the old sorted-ID scratch owner, discarded-reader lookup pass,
full-Scene traversal and membership binary search with one private owner:

```text
SelectedPrimitives<'scene>
  Empty
  One([PrimitiveRef<'scene>; 1])
  Many(ScratchVec<PrimitiveRef<'scene>>)
```

Empty and One need no selection scratch allocation or promotion. Many admits
one real backing before allocation, resolves each requested handle once in
caller order, appends the actual reader, heapsorts those readers and checks
adjacent duplicates. The original metadata/effect loop and unchanged-prefix
backfill borrow the same ordered slice. Selection storage still drops before
the original final observer. No executor, accounting kernel, deferred counter,
Preserve bypass, dependency or alternate selection producer is introduced.

Core now documents that handles successfully resolved in the same current
`ValidatedScene` order by logical mesh, then local primitive. That guarantee
does not validate handles or describe file indices, physical storage, persistent
identity or order across generations. Postprocess uses the public order; it
does not decode private IDs or resolve readers again to recover source order.

## Breaking error contract and retained guarantees

Requested handles resolve in caller order. The first failed Core lookup is
reported before duplicate-target or topology checks. Multiple invalid targets
can therefore select a different first error than the retired ID-sorted lookup
pass. This is an intentional breaking change, documented on
`prepare_triangles` and in the source commit.

Successful membership, canonical processing/report order, geometry, remaps,
output capacity, final generation and source association retain their existing
contracts. Actual resource/control denial still precedes its action body, a
returned semantic lookup error completes its real numerical action, and no
later selection action runs after rejection. Storage envelopes and actual
capacities remain admitted and reconciled. Removed physical actions are not
replaced with padded work. The consumer's `import_work` snapshot precedes
Postprocess and must remain exactly equal in the new fixed-worker comparison.

## Actual integrated source qualification

The original seven gates run serially with Rust 1.95, one Cargo job and one
nextest test thread in the current repository and existing target.

| Gate | Actual result |
| --- | --- |
| Workspace nextest | 3,992 PASS, one slow, six existing manual skips |
| Workspace doctests | 218 PASS |
| Workspace strict all-target/all-feature Clippy | PASS |
| Independent asset-consumer nextest | 138 PASS, one existing manual scale skip |
| Independent asset-consumer strict Clippy | PASS |
| Workspace format | PASS |
| Independent asset-consumer format | PASS |

Source-5 final receipt SHA-256 is
`7dd8bbd0fcf4cac30052f3d31508f0c4420a0e8109f3da46c9ad631bbabad3f7`.
The passing ordinary-host unwind test retains its expected panic stderr and
replayed output tail; it is counted once. Manually skipped tests remain
unexecuted. These are Windows source gates, not a GNU or wasm test execution.

All 1,066 frozen, QA-before/after, persisted staged, committed, archived and
current input hashes agree. The source commit contains nine scoped Rust paths.
The 30,945,280-byte Git archive authenticates all 1,647 file members against the
actual committed tree. Its SHA-256 is
`2e4965894c3c5e77741b770fa548e5b080b04918851939745a6aca4a59f1d344`.
Five separately recorded current user-note identities remain unchanged. This
associates the actual precommit QA with committed bytes; it does not relabel
that run as a postcommit invocation or claim observation of the historical
live index by the independent reviewer.

Meaningful new oracles cover cross-mesh canonical ordering, edited/shared
identities, reversed foreign/stale first errors, duplicate/topology precedence,
Empty/One selection storage, real Many capacity denial, lookup/append/
comparison/duplicate numerical rejection prefixes, denied swaps, cancellation,
semantic-error completion without an extra cleanup poll, unchanged-prefix
backfill and Empty/One/Many final-observer rejection after selection cleanup.
Existing attached ownership and execution-kernel oracles remain.

The bounded source review is `prepare-selection-owner-source-review-1.md`
(SHA-256 `ef03e3d65289871103ec1d042024178c92f2007b78e007d40b584bb7a210054f`).
The test follow-up is `prepare-selection-owner-test-amendment-review-2.md`
(`079fbb91d69ac8773e93ae83325bc4ac118e3ffa46f5d2ccd8424f97b213583d`).
The independent actual integrated review is
`prepare-selection-owner-source5-integrated-review-1.md`
(`fa6e5af251671cccc513e9db3ed9c15fec4944488a6632a4ece52e3edefb07a7`).
These named artifacts remain in the local performance review directory.

## Preserved failed attempts and limits

Sources 1 and 2 fail before executing tests with test-setup/accessor E0599
diagnostics. Source 3 records 1,024 PASS / two FAIL; source 4 records 1,025
PASS / four FAIL. Later lint/docs gates do not start after those failures.
None of these attempts is relabeled PASS.

Source 5 corrects three test-file premises without changing source-4 production
or its real test-hook placement. Historical peak equality now includes both
the admitted `CapacityPlan` envelope and returned Vec capacity. Empty final
observation requires exactly zero semantic work while retaining the real
observer rejection. The tangent fixture funds the original protected terminal
requirement plus twelve ordinary actions, within its original 150-action
ceiling, and still requires rejection at the actual Tangent attempted-work
frontier before allocating the full output. Its typed cause, exact prefix,
source readability, snapshot and terminal-zero assertions remain.

The inherited second-valid-semantic-error order oracle and public
same-generation range-invalid fixture limits remain explicit. No ID-forging
API or source-verification framework is added to manufacture those fixtures.

## Frozen diagnostic and remaining acceptance

The fixed control is `b918256652e3bd15b08bd9a54de3d12caf125152`, explicitly
aliased from its unchanged historical candidate role. Its incomplete ordinary
campaigns remain incomplete; using it as a control does not accept its speed.
The new candidate is the exact qualified `6203ce2` source above.

Before candidate build or sampling, the experiment freezes its original seven
pipeline/memory instruction tasks and precise offline entry attribution. At
least one same GLB1, GLB32 or STL1 task must satisfy both:

```text
10 * candidate_prepare_Ir <= 9 * control_prepare_Ir
500 * candidate_whole_worker_Ir <= 499 * control_whole_worker_Ir
```

All seven whole-worker totals must satisfy `100*N <= 101*B`, and every complete
non-clock observation, including numeric lexemes/types, import work and output
capacity, must match. Only `stage_ns` is removed. No glTF work-delta normalizer
or historical population substitutes for these new observations.

The exact public `prepare_triangles -> prepare::run` incoming edge is read
from the same Callgrind profile, with one actual call and an unambiguous worker,
source and function identity. Summing overlapping inline annotation rows is
not permitted. Missing or ambiguous entry attribution remains failed or
unavailable; no replacement metric is fitted after sampling. All signs,
warnings, load records and failed attempts remain retained.

The GNU candidate builds from this repository into its existing target with
Rust1.95, opt3/debug1, generic CPU, System allocator, empty consumer features and
one Cargo job. Its SHA-256 is
`0f71217143f08109b8d88ade6a7ed676e49f660f42a0b23154f882f3607f4eaf`;
the build receipt is
`03b6715513fab43dacaa799d6b604d155ff7d15797780f98f95aad02ade92133`.
The source archive and all 1,066 inputs remain exact. The historical b918
symbol worker retains SHA-256
`82e0000033f2405b9072d302bf307433cee3af7b43d197ccfe64d3759d2ee477`.
Neither symbol build is eligible for ordinary timing.

All fourteen fresh serial workers and twenty-eight annotations complete with
zero failed commands. All seven complete literal non-clock pairs match,
including import work. The exact report SHA-256 is
`2832f49b2e9a4a43f177b21888656176f939b24311b0afc87eb959a3838804e1`.

| Original pipeline/memory task | Control Ir | Candidate Ir | Whole-worker change |
| --- | ---: | ---: | ---: |
| glTF1 | 5,623,353 | 5,611,022 | -0.21928198% |
| glTF32 | 24,740,255 | 24,646,594 | -0.37857734% |
| F64 PLY neighbors | 2,375,303 | 2,361,853 | -0.56624355% |
| Binary STL1024 | 12,176,675 | 12,164,370 | -0.10105386% |
| Repeated OBJ16384 | 709,670,195 | 709,644,780 | -0.00358124% |
| Grid OBJ128 | 1,988,093,668 | 1,988,080,614 | -0.00065661% |
| Binary STL1 | 2,025,552 | 2,012,002 | -0.66895345% |

| Exact preparation incoming edge | Control Ir | Candidate Ir | Entry change |
| --- | ---: | ---: | ---: |
| glTF1 | 41,779 | 28,902 | -30.82170468% |
| glTF32 | 425,543 | 332,269 | -21.91881901% |
| Binary STL1 | 41,271 | 28,368 | -31.26408374% |

All six preparation edges have one recorded incoming call from the public
wrapper. Every one of the three target cases satisfies both frozen reduction
predicates; all seven whole-worker increase ceilings pass. All signed values,
literal pairs and owner predicates remain in the
[seven-row export](2026-11-09-prepare-selection-owner-instructions.csv).
These are executed guest instructions, not stage CPU or native elapsed time.
The entry and whole-worker reductions are different scopes and are not added.

Both grid profiles retain Callgrind's `brk segment overflow` warnings; host load
and the WSL localhost-proxy/NAT warning remain recorded. No machine-isolation
claim follows. Raw observation, annotation and profile identities are retained
for independent actual-evidence review. That review authenticates all fourteen
profiles, 84 raw streams, six unique incoming edges and 112 export cells and
independently recomputes the passing predicate. Its report is
`prepare-selection-owner-actual-instruction-review-1.md` (SHA-256
`5f4bb45bbe1007a1f39078bd984b838903baee65f0289f70b5aedf0b4cc12d49`).

The separate source/archive/ELF owner review establishes positive Empty/One/
Many branches, one real Many reader backing, the returned Core reader stores,
reader sorting, adjacent duplicate checks, actual metadata/backfill consumers
and retained admission/cleanup calls. Its sealed GLB1 and GLB32 execution
witnesses include actual One lookup and Many lookup/append/sort/metadata/
destruction paths. Empty, prepared-prefix backfill, rejection and unwind have
static compiled witnesses; no new dynamic execution of those paths is claimed.
This is positive replacement evidence, rather than a missing-symbol argument.
The report is `prepare-selection-owner-generated-code-review-1.md`
(`7611b86a3b630900f868919635f067c69d241ec35ab5356b1c3933c79831fe67`).
It supplies a bounded owner witness without rewriting the original collector's
collection-time `unestablished` field or inferring native elapsed benefit.

Both new ordinary candidate builds now complete serially, Windows then GNU,
from the same repository and existing target. They use Rust1.95, opt3/debug0,
generic CPU, System allocator, empty features and one Cargo job. Per-platform
compiler, argv and environment vectors match their immutable b918 controls.
The complete actual build association is
`prepare-selection-owner-ordinary-build-association-1.json` (SHA-256
`8d0d5f96e43dff3893e201f33cce950adabd1bf94a4a965880d97d6c2d502384`).

| Ordinary worker | SHA-256 |
| --- | --- |
| Windows control, b918 | `1b208cfe32b0c7091fa0cdb5698d8047a9be838914f32e92e150348e0f6d074f` |
| Windows candidate, 6203 | `1d1a6f5dd49f2d2e25390a39755120dab60ae7c7216344343f0a6a45980c489d` |
| GNU control, b918 | `790f2d9de741b9a3912b864ad597946c8d30ecae17fbb103382643fbb4ff4f1c` |
| GNU candidate, 6203 | `765cf579d492203a57f0ae627cd9cf54cafd49034a11352e1af969f870fe5e00` |

No ordinary worker clock has run for this experiment at this checkpoint.
Static runner review identifies an interruption-finalization omission; its
amendment remains under separate review before native launch. The two normal
candidate builds pass through the separately accepted builder path. The
builder's interrupted-run receipt limitation remains explicit and is not
represented as a tested interruption contract.

This diagnostic still requires original Windows/GNU 14-case/110-lane
C0/N0/N1/C1 comparisons, three warmups and twenty retained AB/BA pairs,
comparable whole-child memory and separate representative heap attribution.
The original engineering convergence targets remain active. Source retention,
main integration and release remain **HOLD**.
