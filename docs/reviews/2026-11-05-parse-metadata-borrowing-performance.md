# Parse Metadata Borrowing: Windows, Instruction and Heap Evidence

The [qualified metadata borrowing change](2026-11-05-parse-metadata-borrowing.md)
has a consistent descriptive large-OBJ benefit and fewer Linux copying instructions.
Ordinary performance keep and release remain HOLD: three of four Windows campaigns
record external build/test tools, nine complete pipelines still exceed 3x Assimp,
and smaller endpoints and some commit peaks retain positive costs.

This report concerns immutable source `308e9ab890b9e490c3f2bf3a444ca155d9244cd5`
against `67e9d1c215372744d55ddc2ecd72a6a9fe6324af`. Subsequent typed-table producer
work in the live tree is not measured by these workers. The full
[convergence goal](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md),
including current-source ordinary Linux and separate final qualification, remains open.

## Immutable Workers and Ordinary Scope

| Windows identity | Control | Candidate |
| --- | --- | --- |
| Runtime revision | `67e9d1c` | `308e9ab` |
| Executable SHA-256 | `0380fc17240dc48704e36da7674bfcb59a9ca236f8a4e2a50fb5e6a845ab4cc2` | `b3eb661bb9e99c4b18f53734d67b0d814648b571d3c2d726aa1369b9b0b6cc71` |
| Source archive SHA-256 | `89909baa56adba16c57661578d049d9e8f1241767c83562839b8aef9a7d27f80` | `e581f1278a2bad1a6b8d9fdd25f99c36f392ce5b1f1397b064b3346800614a46` |

Both use Rust 1.95.0, generic x86_64 MSVC, opt3/debug0, System allocation, dynamic CRT
and no allocation instrumentation. Assimp remains the original pinned 6.0.5 F32/F64
workers. The candidate build preserves 61 old Baozi release fingerprint directories,
freshly compiles the actual Import library, verifies main-root dep-info, and keeps
all build input hashes unchanged. Its archive contains the exact qualified two-file
candidate. Both workers retain a 1,048,576-byte PE stack reserve and 4,096-byte commit.
The candidate's separate zero-warmup/one-pair 110-lane functional preflight succeeds;
that preflight is not an ordinary timing result.

Four campaigns are prospectively fixed as C0, N0, N1, C1 before candidate build or
measurement. Each keeps the original fourteen cases, 110 lanes, three warmup pairs
and twenty retained AB/BA pairs per lane. No task, field, normal policy, scalar width,
allowance, allocator policy or timing boundary changes.

| Campaign | Whole sequence exit | Native children | Watched-tool snapshots |
| --- | ---: | ---: | ---: |
| C0 | 0 | 5,060 | 0 |
| N0 | 0 | 5,060 | 70 |
| N1 | 0 | 5,060 | 27 |
| C1 | 0 | 5,060 | 27 |

The main orchestration `31334` actually exits zero. There is no complete clean
confirmation pair. C0's absence of named-tool snapshots is not proof of global host
quiet; the other three campaigns remain descriptive. Every sample and observation
is retained. No lane is rehabilitated as clean, pooled across rounds or replaced by
an outcome-selected repeat. The finite decision's 20-second quiet start does not
establish quiet throughout a campaign.

Independent reconstruction reads all 20,240 native children and 40,480 stdout/stderr
files. Native times come from actual stdout, and elapsed/working-set/commit counters
come from original parent command records. Arguments, serial order, pairing, corpus,
geometry, output and reported import-work signatures agree. No actual raw or build
identity mismatch remains.

The first audit draft reports eighteen archive/build-input alerts. Each is retained
and individually resolved as a line-ending assumption error: original before/after/live
raw hashes agree exactly, and removing CR from CRLF alone yields the Git archive
content. Four manifest paths occur in both builds and ten documentation paths only
in the candidate's wider input map. No original file, hash or archive is rewritten
to manufacture agreement. Live Operation drift is separately disclosed as subsequent
work, not used as worker identity.

## Descriptive Benefits, Costs and Targets

Times are native medians in milliseconds. Contrasts compare separate campaign medians;
the children are not fictitiously paired across versions. Every Baozi/Assimp ratio is
the median of the twenty individual pair ratios, not the reciprocal of an even-count
median.

| Case / endpoint / memory IO | C0 | N0 | Initial change | Confirmation change, N1/C1 |
| --- | ---: | ---: | ---: | ---: |
| Repeated OBJ 16384 / import | 33.79185 | 31.62160 | -6.42% | -4.38% |
| Repeated OBJ 16384 / pipeline | 48.19950 | 45.47155 | -5.66% | -5.40% |
| Grid OBJ 128 / import | 163.61510 | 148.05590 | -9.51% | -9.92% |
| Grid OBJ 128 / pipeline | 167.26155 | 154.08030 | -7.88% | -9.90% |
| Shared glTF 1 / pipeline | 2.14365 | 2.15445 | +0.50% | +0.58% |
| Shared glTF 32 / pipeline | 3.73125 | 3.64585 | -2.29% | -2.27% |

Grid-128 file import falls 10.20%/10.47% and its file pipeline 9.48%/9.75%. Large
grid parent elapsed also falls descriptively. Those improvements do not clear the
observed-load qualification or the remaining Assimp gaps.

Small STL prepare/file rises 11.21%/8.27%, from 81.20 to 90.30 microseconds initially.
F64-neighbors prepare/memory rises 6.20%/11.25%, from 80.70 to 85.70 microseconds initially.
Grid-32 extract/memory rises 8.74%/6.74%. Positive native costs occur in 62/110 initial
and 45/110 confirmation lanes; 39 lanes are positive in both. Parent elapsed is
positive in 72/110 and 40/110, respectively. All of those costs and contradictory
signs remain in the complete table and independent JSON.

Working-set medians are positive in 47/110 and 43/110 lanes, while commit is positive
in 44/110 and 45/110. Shared-32 glTF pipeline/file commit rises 64 KiB in both
contrasts. Smaller working-set or executable size does not explain or erase that
separate whole-child commit cost.

| Campaign | Native >3x / >=10x, of 110 | Pipelines >3x, of 28 | Working set >3x | Commit >3x |
| --- | ---: | ---: | ---: | ---: |
| C0 | 58 / 37 | 9 | 0 | 0 |
| N0 | 57 / 36 | 9 | 0 | 0 |
| N1 | 59 / 38 | 9 | 0 | 0 |
| C1 | 58 / 37 | 9 | 0 | 0 |

These are same-campaign median ratios, not a broad convergence result. All Windows
working-set and peak-commit ratios remain within 3x; they cover the whole child and
are neither endpoint heap nor Linux RSS. Tiny Assimp endpoints, IQRs, tails and
absolute differences remain visible in the full raw statistics. Twenty samples
support descriptive high percentiles, not stable population-tail guarantees.

## Separate Linux Instruction Attribution

Six exact-source candidate profiles compare with the already retained `67e9d1c`
symbolized worker. Both use generic GNU x86-64 Rust 1.95.0 opt3/debug1/System, no
worker allocation metrics and external Callgrind 3.26.0. All source archive members,
build before/after maps, binaries, worker signatures, raw totals and annotation
identities reconcile independently.

| Case | Control whole-worker Ir | Candidate whole-worker Ir | Change | Memcpy exclusive change |
| --- | ---: | ---: | ---: | ---: |
| Shared glTF 1 | 6,244,333 | 6,071,746 | -2.76% | -88,854 |
| Shared glTF 32 | 31,833,318 | 30,277,355 | -4.89% | -575,678 |
| F64 PLY neighbors | 2,514,877 | 2,505,293 | -0.38% | -6,706 |
| Repeated STL 1024 | 27,440,619 | 27,370,260 | -0.26% | -768 |
| Repeated OBJ 16384 | 875,739,369 | 843,396,527 | -3.69% | -8,851,672 |
| Grid OBJ 128 | 3,235,431,365 | 2,998,843,880 | -7.31% | -114,806,282 |

Grid's `scan_token_ranges -> memcpy` direct edge falls from 3,529,324 calls /
105,879,720 Ir to zero under that symbol. Its net whole-worker reduction is
236,587,485 Ir: 114,806,282 fewer memcpy-exclusive Ir plus 121,781,203 fewer
exclusive Ir elsewhere. The direct `ParseRoot::child -> memcpy` edges also shrink
for glTF. Call counts are not copied bytes, and inclusive parents are not added
again to exclusive children.

Both grid runs retain Valgrind `brk segment overflow` warnings. Incoming annotation
trees are incomplete; attribution uses complete raw/outgoing graphs, with memcpy
incoming edges separately verified. Whole-worker Ir includes work outside the
endpoint clock and does not establish Windows glTF causation, native speed, heap
peaks or release acceptance.

## Separate Heap Observations

Eighteen fresh whole-worker heaptrack 1.5.0 observations complete successfully:
six cases each for `67e9d1c`, `308e9ab` and the retained symbolized Assimp control.
Independent review verifies the exact eighteen tasks, build/binary/trace identities,
54 original logs, actual semantics and reported work. Heaptrack's K/M peak values
are rounded decimal units.
Allocation counts include setup, preload, observation, serialization and teardown.
Instrumented elapsed time and heaptrack-overhead RSS are not ordinary measurements.

| Case | Control/candidate calls | Assimp calls | Control/candidate temporary A/S | Assimp temporary A/S | Control/candidate peak | Assimp peak |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| obj-repeated-16384 | 33,389 | 82,154 | 10/9 | 9/8 | 12.18M | 19.91M |
| obj-grid-128 | 99,425 | 360,697 | 33,262/33,261 | 10/9 | 28.82M | 28.69M |
| gltf-shared-32 | 1,116 | 845 | 14/13 | 16/15 | 416.52K | 174.36K |
| stl-repeated-1024 | 488 | 1,186 | 8/7 | 10/9 | 456.54K | 317.09K |
| ply-f64-neighbors | 518 | 187 | 10/9 | 17/14 | 135.96K | 2.18M |
| ply-points-1024 | 482 | 4,281 | 9/8 | 1,041/17 | 188.38K | 2.23M |

Control and candidate have identical reported allocation counts, temporary counts,
rounded peaks and exit-live values in every selected case. This experiment does not
show a heap reduction from metadata borrowing. A/S denotes the original analysis
and worker stderr temporary-allocation counts: they differ in all eighteen runs.
Assimp points reports 1,041 versus 17 and F64 neighbors 17 versus 14; the remaining
differences are one. Both originals are retained without an invented explanation
or selection of a preferred count.

Baozi reports 544B and Assimp 4.10K alive at exit, one allocation each. The tool's
label is not promoted into a leak-safety or production-lifetime conclusion.
Attribution stacks include input preload, primary binding, parser storage, canonical
retention and ledger/bootstrap. Historical interception calibration remains bounded
provenance rather than a new universal allocator-coverage claim. None of these
instrumented metrics substitutes for ordinary whole-child RSS or a stage heap peak.

## Artifacts, Disposition and Next Work

Windows originals are under
`target/assimp-comparison/convergence-windows-parse-metadata-borrow{,-control,-confirm,-control-confirm}/`.
The exact source/binary build is `convergence-windows-parse-metadata-borrow-build/`.
Linux raw profiles are `target/review/convergence/parse-metadata-borrow-attribution/`;
heap originals are `target/assimp-comparison/convergence-linux-parse-metadata-borrow-heap/`.

Independent reports and full statistics under `.git/baozi-review/performance-convergence/`:

- `parse-metadata-borrow-four-campaign-raw-review-1.md` and companion JSON/reader.
- `parse-metadata-borrow-linux-attribution-review-1.md` and companion JSON/reader.
- `parse-metadata-borrow-heap-review-1.md` and companion JSON/reader.

The full initial Windows table below is generated from the independent raw audit
and cross-checked with every numeric column of the main report-derived CSV. The
independent JSON retains all four rounds, actual samples, dispersion, signatures,
observation intersections, positive costs and provenance classification.

The candidate remains the source-qualified intermediate working implementation;
ordinary performance keep and release are unproven. Repair the named costs and
continue the measured owners rather than treating instruction gains or external
memory bounds as final acceptance. Actual typed-table producer work, remaining
Internal/protected/share adapters, fixed/dynamic backing, shared-shell promotion,
administrative accounting, obsolete-owner deletion and indexed selection remain
required, along with the complete ordinary Windows/Linux and separate final heap gates.

## Complete Initial Windows Table

Each row retains C0/N0 medians and both version signs. Ratio columns are N0 medians
of twenty same-campaign Baozi/Assimp pair ratios. Whole-child memory is separate
from endpoint timing.

| Lane | Case | Endpoint | IO | C0 ms | N0 ms | N0/C0 | N1/C1 | Native ratio | Working-set ratio | Commit ratio |
| ---: | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | obj-repeated-1 | import | file | 1.44420 | 1.46330 | +1.32% | -1.64% | 0.24023x | 0.44198x | 0.05974x |
| 2 | obj-repeated-1 | import | memory | 1.13850 | 1.12455 | -1.23% | -0.65% | 0.28842x | 0.43769x | 0.06016x |
| 3 | obj-repeated-1 | prepare | file | 0.50400 | 0.50650 | +0.50% | +2.63% | 20.78374x | 0.51117x | 0.05967x |
| 4 | obj-repeated-1 | prepare | memory | 0.49345 | 0.50410 | +2.16% | +1.01% | 20.81861x | 0.50749x | 0.06017x |
| 5 | obj-repeated-1 | extract | file | 0.01190 | 0.01205 | +1.26% | +6.90% | 9.62500x | 0.51190x | 0.05985x |
| 6 | obj-repeated-1 | extract | memory | 0.01235 | 0.01255 | +1.62% | +3.83% | 7.61667x | 0.50795x | 0.05997x |
| 7 | obj-repeated-1 | pipeline | file | 2.34220 | 2.40530 | +2.69% | -0.41% | 0.34548x | 0.51185x | 0.05975x |
| 8 | obj-repeated-1 | pipeline | memory | 1.82435 | 1.78235 | -2.30% | -1.65% | 0.44852x | 0.50799x | 0.06014x |
| 9 | obj-repeated-1024 | import | file | 3.57555 | 3.48075 | -2.65% | -3.94% | 0.56569x | 0.44156x | 0.10660x |
| 10 | obj-repeated-1024 | import | memory | 3.30880 | 3.14040 | -5.09% | -4.50% | 0.75091x | 0.43733x | 0.10740x |
| 11 | obj-repeated-1024 | prepare | file | 1.36240 | 1.32655 | -2.63% | +2.21% | 11.93425x | 0.50992x | 0.10655x |
| 12 | obj-repeated-1024 | prepare | memory | 1.33365 | 1.34590 | +0.92% | +0.11% | 12.40781x | 0.50623x | 0.10733x |
| 13 | obj-repeated-1024 | extract | file | 0.06835 | 0.06675 | -2.34% | +0.08% | 1.64015x | 0.51059x | 0.10661x |
| 14 | obj-repeated-1024 | extract | memory | 0.06785 | 0.07120 | +4.94% | -0.52% | 1.78588x | 0.50691x | 0.10737x |
| 15 | obj-repeated-1024 | pipeline | file | 5.32965 | 5.27950 | -0.94% | -1.39% | 0.74045x | 0.51068x | 0.10675x |
| 16 | obj-repeated-1024 | pipeline | memory | 4.92480 | 4.83445 | -1.83% | -3.50% | 1.05028x | 0.50695x | 0.10756x |
| 17 | obj-repeated-16384 | import | file | 33.99055 | 32.07585 | -5.63% | -6.43% | 3.25769x | 0.68350x | 0.64421x |
| 18 | obj-repeated-16384 | import | memory | 33.79185 | 31.62160 | -6.42% | -4.38% | 3.95403x | 0.68175x | 0.64738x |
| 19 | obj-repeated-16384 | prepare | file | 13.43610 | 13.55415 | +0.88% | +1.08% | 12.45603x | 0.68319x | 0.64367x |
| 20 | obj-repeated-16384 | prepare | memory | 13.49710 | 13.43405 | -0.47% | +0.47% | 12.36919x | 0.68218x | 0.64683x |
| 21 | obj-repeated-16384 | extract | file | 0.69665 | 0.68785 | -1.26% | -0.15% | 1.72139x | 0.68356x | 0.64451x |
| 22 | obj-repeated-16384 | extract | memory | 0.69630 | 0.69145 | -0.70% | -0.72% | 1.67241x | 0.68218x | 0.64684x |
| 23 | obj-repeated-16384 | pipeline | file | 49.23330 | 46.16630 | -6.23% | -3.81% | 3.62869x | 0.68334x | 0.64431x |
| 24 | obj-repeated-16384 | pipeline | memory | 48.19950 | 45.47155 | -5.66% | -5.40% | 4.64449x | 0.68209x | 0.64670x |
| 25 | obj-grid-2 | import | file | 1.56075 | 1.95180 | +25.06% | -2.74% | 0.20288x | 0.44590x | 0.06006x |
| 26 | obj-grid-2 | import | memory | 1.23010 | 1.27705 | +3.82% | -4.26% | 0.29484x | 0.44149x | 0.06038x |
| 27 | obj-grid-2 | prepare | file | 0.08210 | 0.08990 | +9.50% | +6.87% | 41.31263x | 0.45834x | 0.05987x |
| 28 | obj-grid-2 | prepare | memory | 0.08110 | 0.09225 | +13.75% | +4.33% | 41.60968x | 0.45396x | 0.06017x |
| 29 | obj-grid-2 | extract | file | 0.01325 | 0.01390 | +4.91% | +4.53% | 5.95673x | 0.45893x | 0.05992x |
| 30 | obj-grid-2 | extract | memory | 0.01355 | 0.01480 | +9.23% | +1.47% | 5.35072x | 0.45478x | 0.06038x |
| 31 | obj-grid-2 | pipeline | file | 2.02715 | 2.97390 | +46.70% | -4.23% | 0.22759x | 0.45885x | 0.05983x |
| 32 | obj-grid-2 | pipeline | memory | 1.49955 | 1.77080 | +18.09% | -1.01% | 0.35495x | 0.45482x | 0.06051x |
| 33 | obj-grid-32 | import | file | 11.34765 | 11.02655 | -2.83% | -10.41% | 1.25193x | 0.45226x | 0.16463x |
| 34 | obj-grid-32 | import | memory | 11.27515 | 10.24330 | -9.15% | -13.36% | 1.88065x | 0.44864x | 0.16966x |
| 35 | obj-grid-32 | prepare | file | 0.08750 | 0.09215 | +5.31% | +0.34% | 23.64331x | 0.45231x | 0.16461x |
| 36 | obj-grid-32 | prepare | memory | 0.08715 | 0.08995 | +3.21% | -0.45% | 23.09527x | 0.44839x | 0.16945x |
| 37 | obj-grid-32 | extract | file | 0.14990 | 0.15055 | +0.43% | +0.10% | 1.64275x | 0.45227x | 0.16448x |
| 38 | obj-grid-32 | extract | memory | 0.14365 | 0.15620 | +8.74% | +6.74% | 1.77904x | 0.44827x | 0.16951x |
| 39 | obj-grid-32 | pipeline | file | 12.43540 | 11.52020 | -7.36% | -6.96% | 1.27375x | 0.45205x | 0.16465x |
| 40 | obj-grid-32 | pipeline | memory | 11.62725 | 10.65220 | -8.39% | -9.02% | 1.90296x | 0.44810x | 0.16953x |
| 41 | obj-grid-128 | import | file | 165.83690 | 148.91660 | -10.20% | -10.47% | 5.68255x | 0.92756x | 0.95174x |
| 42 | obj-grid-128 | import | memory | 163.61510 | 148.05590 | -9.51% | -9.92% | 6.21543x | 0.92944x | 0.95764x |
| 43 | obj-grid-128 | prepare | file | 0.09125 | 0.09435 | +3.40% | -0.27% | 2.80092x | 0.92703x | 0.95362x |
| 44 | obj-grid-128 | prepare | memory | 0.08960 | 0.09480 | +5.80% | +4.41% | 2.61985x | 0.92916x | 0.95745x |
| 45 | obj-grid-128 | extract | file | 1.95210 | 1.94730 | -0.25% | -0.41% | 1.81684x | 0.92804x | 0.95409x |
| 46 | obj-grid-128 | extract | memory | 1.93390 | 1.88785 | -2.38% | -1.63% | 1.91368x | 0.92957x | 0.95801x |
| 47 | obj-grid-128 | pipeline | file | 168.73140 | 152.74395 | -9.48% | -9.75% | 5.32103x | 0.92769x | 0.95416x |
| 48 | obj-grid-128 | pipeline | memory | 167.26155 | 154.08030 | -7.88% | -9.90% | 5.66440x | 0.92901x | 0.95733x |
| 49 | gltf-shared-1 | import | file | 2.15100 | 2.19120 | +1.87% | -1.68% | 0.75180x | 1.75302x | 1.15938x |
| 50 | gltf-shared-1 | import | memory | 1.87415 | 1.87740 | +0.17% | -3.64% | 8.57804x | 1.76718x | 1.23273x |
| 51 | gltf-shared-1 | prepare | file | 0.09030 | 0.08780 | -2.77% | +5.90% | 54.37868x | 1.79172x | 1.15656x |
| 52 | gltf-shared-1 | prepare | memory | 0.08765 | 0.08940 | +2.00% | +4.17% | 62.57143x | 1.80995x | 1.23739x |
| 53 | gltf-shared-1 | extract | file | 0.01280 | 0.01240 | -3.12% | -6.06% | 7.75551x | 1.80099x | 1.16598x |
| 54 | gltf-shared-1 | extract | memory | 0.01300 | 0.01285 | -1.15% | -2.66% | 10.37500x | 1.81189x | 1.24580x |
| 55 | gltf-shared-1 | pipeline | file | 2.70920 | 2.65325 | -2.07% | -2.07% | 0.77435x | 1.79351x | 1.14035x |
| 56 | gltf-shared-1 | pipeline | memory | 2.14365 | 2.15445 | +0.50% | +0.58% | 7.92419x | 1.81305x | 1.25922x |
| 57 | gltf-shared-8 | import | file | 2.54030 | 2.52130 | -0.75% | -2.89% | 0.82456x | 1.74892x | 1.17864x |
| 58 | gltf-shared-8 | import | memory | 2.25985 | 2.23350 | -1.17% | -4.28% | 9.39664x | 1.76711x | 1.24618x |
| 59 | gltf-shared-8 | prepare | file | 0.09340 | 0.09505 | +1.77% | -0.54% | 59.00000x | 1.79517x | 1.19450x |
| 60 | gltf-shared-8 | prepare | memory | 0.09420 | 0.09735 | +3.34% | +5.39% | 65.67857x | 1.80404x | 1.26778x |
| 61 | gltf-shared-8 | extract | file | 0.01730 | 0.01695 | -2.02% | -6.76% | 2.92459x | 1.79339x | 1.13682x |
| 62 | gltf-shared-8 | extract | memory | 0.01685 | 0.01685 | +0.00% | -5.44% | 4.12440x | 1.80599x | 1.21717x |
| 63 | gltf-shared-8 | pipeline | file | 3.20080 | 3.07095 | -4.06% | -7.85% | 0.86117x | 1.79394x | 1.15447x |
| 64 | gltf-shared-8 | pipeline | memory | 2.46580 | 2.48910 | +0.94% | -4.39% | 8.76063x | 1.80713x | 1.23465x |
| 65 | gltf-shared-32 | import | file | 3.76255 | 3.69215 | -1.87% | -3.18% | 1.23953x | 1.73890x | 1.32904x |
| 66 | gltf-shared-32 | import | memory | 3.46260 | 3.39210 | -2.04% | -4.30% | 12.37088x | 1.75776x | 1.42193x |
| 67 | gltf-shared-32 | prepare | file | 0.10965 | 0.11440 | +4.33% | +12.03% | 57.28947x | 1.78257x | 1.31524x |
| 68 | gltf-shared-32 | prepare | memory | 0.11585 | 0.11915 | +2.85% | +4.90% | 66.27778x | 1.80566x | 1.45201x |
| 69 | gltf-shared-32 | extract | file | 0.03170 | 0.03080 | -2.84% | -5.72% | 2.05278x | 1.79409x | 1.34091x |
| 70 | gltf-shared-32 | extract | memory | 0.03215 | 0.03200 | -0.47% | -2.31% | 2.75162x | 1.79894x | 1.36800x |
| 71 | gltf-shared-32 | pipeline | file | 4.26895 | 4.18105 | -2.06% | -5.24% | 0.98251x | 1.79019x | 1.36782x |
| 72 | gltf-shared-32 | pipeline | memory | 3.73125 | 3.64585 | -2.29% | -2.27% | 10.60464x | 1.80232x | 1.38600x |
| 73 | stl-repeated-1 | import | file | 1.33400 | 1.33785 | +0.29% | +2.52% | 0.60525x | 1.50611x | 1.07661x |
| 74 | stl-repeated-1 | import | memory | 0.99740 | 1.05240 | +5.51% | +0.57% | 14.12889x | 1.51529x | 1.16269x |
| 75 | stl-repeated-1 | prepare | file | 0.08120 | 0.09030 | +11.21% | +8.27% | 53.79963x | 1.55095x | 1.08333x |
| 76 | stl-repeated-1 | prepare | memory | 0.08285 | 0.08675 | +4.71% | +6.70% | 66.53846x | 1.56053x | 1.16018x |
| 77 | stl-repeated-1 | extract | file | 0.01230 | 0.01210 | -1.63% | +3.97% | 10.65530x | 1.55174x | 1.08114x |
| 78 | stl-repeated-1 | extract | memory | 0.01195 | 0.01265 | +5.86% | +7.69% | 13.55556x | 1.56180x | 1.15905x |
| 79 | stl-repeated-1 | pipeline | file | 1.79370 | 1.83020 | +2.03% | -3.23% | 0.65299x | 1.55238x | 1.08502x |
| 80 | stl-repeated-1 | pipeline | memory | 1.23240 | 1.29290 | +4.91% | -2.19% | 11.14155x | 1.56162x | 1.15618x |
| 81 | stl-repeated-1024 | import | file | 2.75120 | 2.81150 | +2.19% | +0.20% | 1.33309x | 1.50451x | 1.20886x |
| 82 | stl-repeated-1024 | import | memory | 2.43515 | 2.45035 | +0.62% | -2.27% | 16.92097x | 1.51170x | 1.26613x |
| 83 | stl-repeated-1024 | prepare | file | 0.08735 | 0.08965 | +2.63% | +9.16% | 45.52632x | 1.54259x | 1.16853x |
| 84 | stl-repeated-1024 | prepare | memory | 0.08645 | 0.09055 | +4.74% | +2.27% | 50.33746x | 1.55482x | 1.27383x |
| 85 | stl-repeated-1024 | extract | file | 0.03655 | 0.03995 | +9.30% | -1.22% | 1.74675x | 1.54987x | 1.19259x |
| 86 | stl-repeated-1024 | extract | memory | 0.03820 | 0.04085 | +6.94% | -4.82% | 1.79259x | 1.56040x | 1.17383x |
| 87 | stl-repeated-1024 | pipeline | file | 3.24555 | 3.32545 | +2.46% | -2.71% | 1.15409x | 1.54815x | 1.18889x |
| 88 | stl-repeated-1024 | pipeline | memory | 2.74805 | 2.78460 | +1.33% | -3.46% | 11.74163x | 1.56157x | 1.17976x |
| 89 | ply-triangle-1 | import | file | 1.42800 | 1.49160 | +4.45% | +0.91% | 0.53843x | 1.25379x | 0.36301x |
| 90 | ply-triangle-1 | import | memory | 1.09840 | 1.19080 | +8.41% | +0.39% | 1.60065x | 1.25926x | 0.37291x |
| 91 | ply-triangle-1 | prepare | file | 0.08250 | 0.08915 | +8.06% | +5.19% | 58.23750x | 1.28982x | 0.36159x |
| 92 | ply-triangle-1 | prepare | memory | 0.08255 | 0.08925 | +8.12% | +6.16% | 59.50000x | 1.29611x | 0.37412x |
| 93 | ply-triangle-1 | extract | file | 0.01235 | 0.01275 | +3.24% | -2.01% | 9.06905x | 1.29180x | 0.36079x |
| 94 | ply-triangle-1 | extract | memory | 0.01195 | 0.01285 | +7.53% | -1.18% | 12.35000x | 1.29848x | 0.37273x |
| 95 | ply-triangle-1 | pipeline | file | 1.86120 | 1.89430 | +1.78% | +0.96% | 0.61479x | 1.29164x | 0.36357x |
| 96 | ply-triangle-1 | pipeline | memory | 1.38765 | 1.35610 | -2.27% | +0.24% | 2.28942x | 1.29728x | 0.37247x |
| 97 | ply-f64-neighbors | import | file | 1.43660 | 1.45400 | +1.21% | +0.08% | 0.49934x | 1.26355x | 0.36445x |
| 98 | ply-f64-neighbors | import | memory | 1.11960 | 1.12005 | +0.04% | +0.40% | 2.09509x | 1.26635x | 0.37377x |
| 99 | ply-f64-neighbors | prepare | file | 0.08600 | 0.09010 | +4.77% | +2.72% | 54.70221x | 1.29974x | 0.36475x |
| 100 | ply-f64-neighbors | prepare | memory | 0.08070 | 0.08570 | +6.20% | +11.25% | 56.56667x | 1.30357x | 0.37334x |
| 101 | ply-f64-neighbors | extract | file | 0.01215 | 0.01215 | +0.00% | -1.20% | 11.72143x | 1.30218x | 0.36234x |
| 102 | ply-f64-neighbors | extract | memory | 0.01195 | 0.01240 | +3.77% | +0.00% | 12.00000x | 1.30493x | 0.37273x |
| 103 | ply-f64-neighbors | pipeline | file | 1.89335 | 1.93225 | +2.05% | +0.58% | 0.61375x | 1.30158x | 0.36507x |
| 104 | ply-f64-neighbors | pipeline | memory | 1.37490 | 1.35550 | -1.41% | -0.31% | 2.37224x | 1.30439x | 0.37179x |
| 105 | ply-points-1024 | import | file | 1.82900 | 1.81190 | -0.93% | -3.79% | 0.77815x | 1.24206x | 0.35460x |
| 106 | ply-points-1024 | import | memory | 1.51665 | 1.48255 | -2.25% | -2.47% | 2.20538x | 1.24299x | 0.37106x |
| 107 | ply-points-1024 | extract | file | 0.02115 | 0.02220 | +4.96% | +4.26% | 4.48056x | 1.24499x | 0.35569x |
| 108 | ply-points-1024 | extract | memory | 0.02235 | 0.02420 | +8.28% | +1.91% | 5.36957x | 1.24576x | 0.37948x |
| 109 | ply-points-1024 | pipeline | file | 2.27185 | 2.19580 | -3.35% | -1.07% | 0.68828x | 1.24395x | 0.35551x |
| 110 | ply-points-1024 | pipeline | memory | 1.72230 | 1.70515 | -1.00% | -1.05% | 2.23932x | 1.24517x | 0.37858x |
