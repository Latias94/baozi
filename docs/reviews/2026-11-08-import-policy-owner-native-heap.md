# Import policy-owner native and tracked-heap checkpoint

Windows completes the original control/candidate/candidate/control bracket;
GNU completes only the first control and candidate. Native convergence fails:
both Windows candidates have nine of 28 pipeline medians above 3x Assimp and
thirty common endpoints at least 10x. GNU has fourteen of 28 pipeline medians
above 3x in its sole candidate. All available comparable whole-child memory
median ratios remain below 3x. Source retention, main/runtime integration and
release remain **HOLD**.

The [source and instruction report](2026-11-08-import-policy-owner.md) qualifies
the owner change and seven correctness gates separately. Its earlier pending
native/heap checkpoint is superseded here, without repairing missing campaigns
in earlier experiments. Both Windows and GNU actual evidence reviews close
with GO. Independent eighteen-worker heap stack/interception review also closes
with bounded GO; neither verdict establishes native performance acceptance.

## Fixed source, workers and scope

Control is `7b8c5f3d2559085967dcdfd4098130c77bd877ce`; candidate is
`bdc92b3fa5ca523a99920ff980b909ecfe6d61e9`. Assimp remains version 6.0.5,
archive-declared source `392a658f9c271be965271f45e7521a1b80ea4392`, runtime
revision 0. Collector-checkout HEAD `5bf5683` adds documentation and is not
either worker's source identity.

| Platform / role | Ordinary worker SHA-256 |
| --- | --- |
| Windows control | `bdb86a1dfc3463705dbb9693d122eb914b8f98f6f55e04444fc5147927fa8372` |
| Windows candidate | `611edc905f13b893efafe2c0099122e9a0a61baa548f349d7ef089ea64452110` |
| GNU control | `e4185358a45ccd04fd2cd293cb869f9b5f12b3ee895b51b5b60df9f73f370d46` |
| GNU candidate | `3f38bef6b7929f22379d24c625c7b7d2cc03b211cb1051bacbb9c3e479b622d9` |

The closed build/prerequisite reviews bind original seven-gate QA, source
archives, compiler/argv/environment and all four ordinary Baozi workers plus
precision-matched Assimp workers. Ordinary Baozi uses Rust 1.95, opt3/debug0,
generic CPU, System allocator and no allocation instrumentation. Symbol workers
used for instruction/heap analysis are distinct opt3/debug1 binaries and do not
enter ordinary timing. All source/builds stay in the current repository and
existing target, including GNU `/mnt/f`; GNU home paths are measurement copies.

Manifest SHA-256 is
`db52cc017949e3345a07ca51411a1ab42e47b05ee2c79aef63964e0796e73f49`;
collector SHA-256 is
`9af6fbf91bb2fcdf8d36547ccef047e40b17ff1fad291b4664b22e2186dcea87`.
All original fourteen cases, 110 lanes, source/retained scalar widths, F64
observations/copied output, fields, normals, IO policies, work limits and stage
clocks remain. Both functional matrices on each host pass all 110 lanes with
zero warmups/one fresh pair, 222 commands and no failures per matrix. Functional
results do not substitute for native samples.

Ordinary campaigns use three warmup pairs and twenty retained fresh serial
AB/BA pairs per lane, a 60-second child timeout and the original statistics.
Each completed campaign has 110 lanes, 5,062 commands and zero failures. Warmup
records remain retained outside the prescribed statistics. No new confidence
interval, significance rule, cross-lane average or outlier filter is introduced.

## Complete Windows bracket

| Campaign | Pipelines >3x / 28 | Endpoints >3x / 110 | Endpoints >=10x | Maximum WS median ratio | Maximum commit median ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| C0 | 9 | 58 | 33 | 1.836024 | 1.352000 |
| N0 | 9 | 57 | 30 | 1.835651 | 1.369168 |
| N1 | 9 | 57 | 31 | 1.839139 | 1.381148 |
| C1 | 8 | 54 | 31 | 1.837769 | 1.371197 |

Ratios are Baozi median divided by same-campaign Assimp median. The N0/N1
intersection of endpoints at least 10x is exactly thirty. It includes 24
prepare endpoints and six import/extract/pipeline endpoints. The extra N1
endpoint is glTF32 pipeline/memory: N0 is 9.911710x, N1 is 10.655111x.
Worst pipeline is tiny STL1/memory, 11.000000x and 11.302366x in N0/N1;
its prepare/memory endpoint is 73.266667x and 76.071429x. These repeated
descriptive gaps already prevent convergence; no statistical significance
claim is needed to report the failed pipeline screen.

Windows lifetime working-set and commit peaks are separate whole-child metrics,
not stage heap or output capacity. All 440 medians for each stay below 3x.
Their global maxima are N1 glTF8 extract/memory WS 1.8391386953768207x and
N1 glTF32 extract/memory commit 1.3811475409836065x. Linux RSS is unavailable
on Windows, with its original explicit reason rather than substituted zero.

All signs and changes against both controls are in the
[440-row Windows export](2026-11-08-import-policy-owner-native-windows-lanes.csv).
Positive change means regression. For stage clocks, Assimp-normalized
regression/improvement counts are N0:C0 62/48, N0:C1 72/38, N1:C0 45/65,
N1:C1 66/44. These descriptive counts do not establish a uniform source win.
The change directly removes import/bootstrap work; prepare/extract stage clocks
start after import and cannot directly time the deleted constructors.

Runtime named-tool busy samples are C0 0/130, N0 0/131, N1 0/132 and
C1 102/144. C1 names cargo, cargo-nextest, rustc and link; its actual noisy
observations remain the closing bracket. Functional groups also retain busy
samples. Quiet admission is twenty seconds within a prospective 180-second
deadline, sampled every two seconds; it does not ensure continued quiet.
Zero busy samples cover named snapshots only, not complete host isolation.
Assimp normalization cannot guarantee equal contention response. No causal
native-speed acceptance follows from the complete but affected bracket.

Independent Windows review verifies all 20,680 comparison workers, 41,384 raw
streams, complete non-clock signatures, original pairing and all summaries.
Every one of the 440 x 41 CSV cells agrees, including unavailable and control
change cells. Review-time stream hashes corroborate current raw/report equality;
the original collector did not retain historical per-stream digests. Historical
outer session exit is a coordinator observation, distinct from recorded child
exits and independently read raw evidence.

## Terminal partial GNU attempt

| Campaign | Status | Pipelines >3x / 28 | Endpoints >3x / 110 | Single-campaign endpoints >=10x | Maximum RSS median ratio |
| --- | --- | ---: | ---: | ---: | ---: |
| C0 | completed | 14 | 70 | 31 | 1.943044 |
| N0 | completed | 14 | 70 | 29 | 1.955813 |
| N1 | not_started | unavailable | unavailable | unavailable | unavailable |
| C1 | not_started | unavailable | unavailable | unavailable | unavailable |

N1 exhausts its prospective quiet-start opportunity after 180.482563081 seconds;
C1 after 180.481173354 seconds. Each records maximum quiet time zero. They are
terminal not-started decisions, not pending campaigns, failures with observations,
zeros, or replacement opportunities. Missing repeat and closing bracket prevent
GNU source-speed qualification. All 440 rows, including 220 missing campaign
rows, remain in the [GNU export](2026-11-08-import-policy-owner-native-linux-lanes.csv).

GNU C0/N0 retain zero Linux named-tool busy samples out of 49/50; their joint
Windows monitor reports 22/49 and 43/50 busy, with zero stale samples. The
continuous monitor and two logged snapshot replacement PermissionErrors remain.
The bounded helper retries the same observation at most six times, five 50 ms
backoffs, without refreshing its timestamp or weakening freshness/quiet gates.
No worker is restarted and no historical gap is repaired. GNU time's recorded
pre-version remains UNKNOWN; its before/after executable hash is stable.

The sole candidate has normalized stage regressions/improvements 57/53 against
C0; both signs stay visible. Maximum candidate RSS median ratio is 1.955813x
for glTF32 pipeline/file. RSS is GNU whole-child maximum resident memory, not
Windows commit, heaptrack heap or final output storage. Missing campaigns leave
their memory observations missing despite the available median screen passing.

Independent GNU review streams all four retained raw archives without extraction,
verifies 10,560 worker stdout observations and RSS logs, 21,136 stdout/stderr
streams and every one of the 18,040 CSV cells. All 412 joint observations bind
an exact retained Windows snapshot, with zero stale samples and maximum age
2.814309597015381 seconds. The Windows monitor is terminal partial with child
returncode 2, following the partial ordinary envelope; its 415 snapshots include
320 busy, maximum adjacent gap 2.8618853092193604 seconds. Sampling does not
prove uninterrupted observation or isolation. The coordinator's outer session
terminal display 1 is distinct from that persisted child exit and does not
establish a Rust worker failure. Original Git status stderr warnings remain
context metadata, with all actual worker stderr streams empty.

## Separate eighteen-worker tracked heap

Collection completes six original pipeline/memory tasks for control, candidate
and precision-matched Assimp: eighteen fresh heaptrack workers and eighteen
explicit unmerged analyses, 36 successful commands and 72 retained streams.
All six Baozi complete literal non-clock output pairs match. The closed bounded
calibration is reused; independent actual worker stack/interception review closes.
No new injected worker or native-time sample is introduced by offline analysis.

| Case | Control allocation calls | Candidate calls | Assimp calls | Control/candidate displayed peak | Assimp displayed peak |
| --- | ---: | ---: | ---: | --- | --- |
| glTF1 | 645 | 589 | 355 | 174.50K | 155.19K |
| glTF32 | 1,100 | 1,044 | 825 | 389.93K | 174.36K |
| F64 PLY neighbors | 492 | 436 | 175 | 136.00K | 2.18M |
| Binary STL1024 | 462 | 406 | 1,174 | 456.58K | 317.09K |
| Repeated OBJ16384 | 16,970 | 16,914 | 82,142 | 12.18M | 19.91M |
| Grid OBJ128 | 33,347 | 33,291 | 360,685 | 28.82M | 28.69M |

Every candidate has 56 fewer observed allocation calls than control. Their
temporary-allocation counts remain 14, 14, 10, 8, 7 and 7; Assimp records
16, 16, 17, 10, 9 and 10. Candidate calls remain above Assimp on both glTFs
and PLY, and displayed peaks remain above Assimp on glTF, STL and grid OBJ.
Equal rounded control/candidate peak strings do not prove exact peak equality.
These are whole-worker tracked allocations, not application-only allocations,
stage peaks, RSS, native speed or universal dependency/allocator interception.

The [complete eighteen-row heap export](2026-11-08-import-policy-owner-heap.csv)
retains source/binary identities, original counts and collection-time pending
stack-qualification notes. The closed actual review supersedes that note;
raw collection history is not rewritten. The symbol-worker hashes and bounded
calibration limits remain in the retained receipt.

The actual review rehashes all eighteen traces and 72 streams and replays every
unmerged analysis byte-for-byte. For every worker, raw allocation events, full
collapsed allocation-stack weight, worker footer, analyzer footer and receipt
count agree; stack references are valid and tracked live counts never become
negative. Every trace ends with one tracked allocation: 544 bytes for Baozi,
4,096 bytes for Assimp. This is captured accounting consistency, not a proof
of unrecorded-allocation absence or an application leak diagnosis.

All six pairs locate aggregate allocation-stack weight through
`import_ledger_limits` decreasing 84 to 28; format/consumer/remainder buckets
are unchanged while the other-import bucket decreases 179 to 123. That locates
the observed 56-call reduction in the policy-construction paths. Inlined stack
presentation does not separately establish 28 allocations per removed constructor.
Actual parser/storage paths, Rust allocation/System backing and each selected
Assimp importer/primary-memory IO are visible. Unknown frames remain in totals:
one Baozi allocation and seventeen Assimp allocations per trace retain unresolved
loader or filesystem details. Tool executable hashes observed during review do
not retroactively prove collection-time tool hashes; the receipt binds versions.

Worker/analyzer temporary counts disagree and their reason remains unresolved:
Baozi records 13/14, 13/14, 9/10, 7/8, 6/7 and 6/7 for both source roles;
Assimp records 15/16, 15/16, 14/17, 9/10, 8/9 and 9/10. Neither temporary
classification substitutes for exact captured allocation-call counts. Failed
calibration1 and its native not-started outcome remain retained; the closed
same-trace diagnosis and calibration2's seven finite controls are reused without
rerunning them. Known System/malloc/realloc/alignment/thread/C++/shared-library
paths establish bounded interception only, without universal runtime/dependency
coverage. Actual sanity checks and complete captured accounting do not remove
those limits or make one diagnostic worker per role a repeated memory experiment.

## Disposition

Both native evidence/export reviews close with GO for faithful data, without
a performance win. Actual heap qualification closes with bounded GO, preserving
all calibration, temporary-classification and interception limits above.
No missing observation, busy sample, unavailable metric, regression, failed
threshold or historical limitation is removed. The unchanged targets remain
every complete pipeline median <=3x same-campaign Assimp, no confirmed endpoint
>=10x, and comparable whole-child memory <=3x on both required platforms.
The fixed instruction predicate's PASS does not satisfy these targets.

Further work should remove demonstrated owner/algorithm overhead. The reviewed
parser-row success allowance is a bounded next candidate: derive disposable
capacity during an exclusive actual-row loan, while committing each physical
read immediately and preserving the original cold failure checker. Current
source/evidence revalidation is required before implementation. Preparation's
direct memcpy arc is another locating lead; the exact copied value is still
unproven, so no whole-host rewrite or byte-saving claim follows from that arc.
