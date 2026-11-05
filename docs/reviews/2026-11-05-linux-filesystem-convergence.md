# Linux Filesystem Convergence Checkpoint

The original fourteen-case, 110-lane comparison now completes four campaigns
with both engines, unchanged corpus bytes and runtime artifacts on the Linux
filesystem. Actual wrapper exec96388 terminates with exit zero. Every campaign
retains three warmup pairs and twenty AB/BA pairs per lane, with 5,060 successful
native children and no missing lanes. This environmental correction does not
change the Rust source or qualify performance convergence.

Candidate N0/N1 each retain fourteen of twenty-eight pipelines above 3x Assimp.
Shared glTF 32 exceeds 10x in both file and memory pipelines in both repeats;
repeated STL 1024 also exceeds 10x in both memory repeats. All whole-child RSS
ratios remain below 3x. Windows tools overlap both candidate campaigns, so the
source-change contrasts are descriptive. Windows ordinary qualification and
release acceptance remain HOLD.

## Exact Workers and Placement

The source comparison remains `05b7d9b` versus `56eecd9`: startup/publication
borrowing plus direct numeric WorkState ownership together. The collector runs
from documentation-only successor `93f547f`; its HEAD is not the measured
worker revision. The immutable source/build/archive identities and independent
qualification are recorded in the
[preceding measurement report](2026-11-05-direct-work-state-performance.md).

No source is rebuilt for this comparison. Exact ordinary debug0 binaries are
copied to `/home/frankorz/baozi-direct-state-nativefs-56eecd9/`:

| Worker | Bytes | SHA-256 |
| --- | ---: | --- |
| Baozi control | 14,686,208 | `e6ba1da06832a87810607c209e668824b14d7f3cb24cf86d59c01b8079d76d87` |
| Baozi candidate | 14,591,232 | `db4f9d8e1b465d852fcb2ef09eb4104b4abd978c5c4505b7fbbc00a14df6d6da` |
| Assimp B0 F32 | 1,714,952 | `c08e25f8d0745d1e4981f3117d6d4f66eba2bcda9e165c1ff8bf6642ce1fdb50` |
| Assimp B0 F64 | 791,552 | `7454bc3f2974f6885f1c23e6c1376f52d8a62df63c31d4c2190a6ade157f35a7` |

Both Baozi workers retain Rust 1.95.0, generic opt3/debug0 GNU x86-64, System
allocation and no allocation metrics. Assimp retains the original B0 Clang 21
Release/debug0 normal/double identities, not the separate symbolized heap
workers. All fourteen input sizes/digests are checked before copying. `stat -f`
identifies the native workspace as `ext2/ext3`; this label does not identify an
exact ext-family subtype.

The collector script and endpoint clocks are unchanged. The Python collector
remains in the repository; worker executables, corpus, stdout/stderr/RSS files
and reports are on the Linux filesystem during collection. Completed artifacts
are copied back only after the collector exits. The 380–411 seconds spent
copying each campaign's many small files are excluded from native stage clocks,
parent child-elapsed metrics and campaign monitoring windows. Both workers pass
all 110 zero-warmup/one-pair functional preflights before ordinary collection;
preflight timings are not ordinary samples.

The previous `/mnt/f` matrix and the separate three-case exact-binary placement
experiment remain retained. Differences from those observations are not Rust
optimization gains. No clock, task, fields, normal policy, precision, work limit,
output lifetime or failed sample is removed to obtain this result.

## Actual Load and Sequence

The prospective decision fixes C0, N0, N1, C1. Before each ordinary campaign,
both Windows and Linux sampled named-tool sets must stay empty for twenty
seconds, within a 180-second start limit. The atomic Windows state must be at
most eight seconds old. Monitoring continues during collection; a start gate
cannot prevent later external tools from starting.

| Campaign | Collector seconds | Linux tool overlap / snapshots | Windows tool overlap / snapshots |
| --- | ---: | ---: | ---: |
| C0 | 109.46 | 0 / 55 | 26 / 54 |
| N0 | 106.96 | 0 / 54 | 33 / 52 |
| N1 | 109.37 | 0 / 55 | 38 / 54 |
| C1 | 110.97 | 0 / 56 | 0 / 55 |

These Windows counts select only the actual collector start/finish intervals,
not setup, preflight, inter-campaign waits or artifact-copy intervals. Overlap
includes Cargo, nextest and rustc. Named-tool absence is not a global host-quiet
proof. This is Linux under WSL with a shared Windows host; the three overlapping
campaigns are not clean controls or clean candidates. Preserve all observations
and the quieter fourth campaign, without outcome-selected replacement runs.

## All-Lane Results

Every ratio is the median of twenty individual Baozi/Assimp pair ratios. There
is no pooling, trimming, ratio of engine medians or inversion of an even-sample
median. Per-engine median/IQR/min/max and every retained sample remain in the
original reports and the complete 110-row summary.

| Metric | C0 | N0 | N1 | C1 |
| --- | ---: | ---: | ---: | ---: |
| Native endpoint/pipeline lanes above 3x | 70 | 69 | 70 | 70 |
| Native endpoint/pipeline lanes at least 10x | 34 | 34 | 38 | 37 |
| Complete pipelines above 3x / 28 | 14 | 14 | 14 | 14 |
| Maximum endpoint/pipeline ratio | 77.114x | 77.471x | 164.467x | 143.472x |
| Whole-child elapsed lanes above 3x | 16 | 16 | 16 | 16 |
| Maximum whole-child elapsed ratio | 5.158x | 5.157x | 5.155x | 5.158x |
| Whole-child RSS lanes above 3x | 0 | 0 | 0 | 0 |
| Maximum whole-child RSS ratio | 2.068x | 2.097x | 2.051x | 2.049x |

The largest endpoint ratio remains preparation of a tiny preserved primitive:
F64 PLY memory preparation is 38.790 microseconds versus 0.514 microseconds in
N0, and 82.810 versus 0.540 microseconds in N1. Moving the executables has removed
the preceding experiment's multi-millisecond scale for that endpoint, but a
substantial public preparation cost remains. The short Assimp denominator and
load limits prevent extrapolating this ratio to a universal parser difference.

Representative complete pipelines retain substantial nontrivial gaps:

| Case / mode | Baozi N0 / N1 ms | Baozi/Assimp N0 / N1 |
| --- | ---: | ---: |
| Repeated OBJ 1 / memory | 0.674 / 0.904 | 0.157x / 0.191x |
| Repeated OBJ 1024 / memory | 3.457 / 3.611 | 0.752x / 0.788x |
| Repeated OBJ 16384 / memory | 44.931 / 45.109 | 4.814x / 4.959x |
| Grid OBJ 32 / memory | 9.594 / 9.948 | 1.698x / 1.707x |
| Grid OBJ 128 / memory | 154.861 / 155.216 | 7.187x / 7.289x |
| Shared glTF 1 / memory | 0.968 / 1.196 | 7.597x / 6.981x |
| Shared glTF 8 / memory | 1.170 / 1.484 | 9.004x / 7.381x |
| Shared glTF 32 / file | 2.388 / 2.542 | 10.891x / 10.241x |
| Shared glTF 32 / memory | 2.333 / 2.534 | 14.168x / 12.345x |
| Repeated STL 1 / memory | 0.527 / 0.587 | 10.144x / 8.276x |
| Repeated STL 1024 / memory | 1.876 / 2.013 | 11.905x / 11.339x |
| F64 PLY neighbors / memory | 0.567 / 0.721 | 0.804x / 1.012x |
| PLY points 1024 / memory | 0.932 / 0.985 | 1.144x / 1.239x |

The same fourteen pipeline lanes exceed 3x in both candidates: both modes of
large repeated OBJ and grid OBJ, all shared glTF cases, and both STL cases. PLY
pipelines remain below 3x; this does not qualify their separate endpoints.

Source-change contrasts remain mixed. N0/C0 has increased Baozi native medians
in 35 lanes; N1/C1 has increases in 89. Thirty-three lanes, including eight
pipelines, increase in both contrasts. Repeated-16384/memory pipeline is
+1.03%/+1.41%, grid-128/memory is +0.11%/+0.36%, while STL-1/memory is
-5.60%/-1.49%. N0 RSS medians increase in all 110 lanes, by 0.31%–5.32%; N1/C1
increases in 61. These less favorable results are retained rather than hidden
behind the environmental timing improvement. Their load and residency limits
preclude assigning a clean source-change effect or performance keep.

## Evidence and Qualification

Original Linux artifacts remain in the native workspace's `C0`, `N0`, `N1` and
`C1` directories. Complete retained copies are
`target/assimp-comparison/convergence-linux-direct-work-state-nativefs-{C0,N0,N1,C1}/`.
The prospective decision is
`.git/baozi-review/performance-convergence/direct-work-state-linux-nativefs-decision-1.json`.
Preparation, command receipts and the completed Windows wrapper use prefix
`target/review/convergence/direct-work-state-linux-nativefs-`.

The main summary/table are `direct-work-state-linux-nativefs-four-campaign-main-summary-1.json`
and `direct-work-state-linux-nativefs-four-campaign-main-table-1.csv` in that
evidence directory. `direct-work-state-linux-nativefs-load-summary-1.json`
correlates actual campaign windows and retains its monitoring scope.

Independent final raw review completes with no unresolved material evidence
discrepancy. It reconstructs all 20,240 native stdout observations and their
stderr/RSS records, reconciles the 110-lane summaries and actual load windows,
and directly compares every original artifact to its retained copy. All 60,772
file pairs, covering 117,517,145 bytes, match with no missing or extra files.
The original copy-audit process terminates zero; it is not restarted on an
observation timeout. The final private receipt is
`.git/baozi-review/performance-convergence/direct-work-state-nativefs-raw-review-1.md`,
SHA-256 `33e398815f8ef7693a2075890abd901965416cbaa7763f9190defc6b8b81238f`.

The audit retains two resolved expectation differences: raw Assimp glTF import
labels say `assimp-native-one-minus-source-v`, while the collector describes the
same verified `1-v` values as `assimp-native-inverted-v`; command receipts also
include the Python interpreter prefix absent from `sys.argv`. Neither resolution
changes raw evidence. Parent monotonic elapsed readings have no independent
stdout reconstruction; raw native clock and GNU whole-child RSS checks have
different, explicit scopes. Aggregate geometry observations do not replace
qualified tuple/winding/lifetime tests. The completed review preserves the
descriptive/HOLD classification and every remaining release gate.

## Next Owner Work

The subsequent [parser/pipeline root-cause investigation](2026-11-05-parser-pipeline-root-causes.md)
completes two separate same-invocation diagnostics and pinned-source comparisons.
It refines the order below: common semantic preflight and OBJ record/lexical
execution precede a broad root replacement; glTF normalization precedes a whole
JSON parser replacement. Fixed setup and terminal ownership remain required.
The diagnostic intervals do not qualify an optimization or replace this matrix.

This checkpoint changes the cost diagnosis, not the release criteria. The
preceding six-case Callgrind and eighteen-case heap evidence still shows small
mixed instruction changes and unchanged allocations for this source interval.
That evidence does not become a native timing proof because placement changes.

The next coherent ownership work must address both finite setup and repeated
execution. The current direct runtime has six dimensions, three ledger phases,
and four possible progress phase/unit pairs with initial occupancy zero.
Concrete external fixed backing and transferred dynamic backing must reach the
actual host/coordinator/start/attach/finish types, replacing temporary Vec plans
and eager sharing. Progress metadata must borrow the ordinary owner's actual
storage authority briefly, rather than retaining a second mutable owner. Start
construction failure must retain its actual backing for cleanup without resetting
the start latch. Shared promotion must allocate/qualify before taking the complete
state from its explicit source slot. Clean finite facts cannot replace the
actual owner needed to contain dirty state.

Bounded independent design review identifies these required interface decisions;
selected dispositions are recorded privately but are not yet a compiled lifetime,
cleanup or allocation proof. Existing invariant invocation/layout branding and
the one numeric kernel remain. Owned finite terminal facts,
cleanup-before-finish and indexed single-classifier selection are still required
to remove the real empty/single unchanged allocations.

Large OBJ's remaining 45–155 ms pipelines also require analysis of the actual
per-element parser/numeric/execution protocol. Replacing initialization Vecs alone
is not a demonstrated solution to that scaling cost. Do not infer another normal
algorithm rewrite from a fixed setup endpoint, weaken accounting/control/failure
semantics, or move public work outside the measured clock. Continue under the
[convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md) and
[exclusive invocation plan](../plans/2026-11-03-1120-refactor-exclusive-invocation-plan.md).
All Windows and final integrated convergence gates remain open.
