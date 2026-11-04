# Outer-Host Windows Comparison: Convergence Remains Open

The integrated outer-host implementation has a material large repeated-OBJ benefit,
but has not established a performance keep or release readiness. The initial candidate
still exceeds Assimp by 3x in 58 of 110 native timing lanes and nine of 28 complete
pipelines. All twelve glTF import/pipeline lanes become slower. Whole-child time and
memory also retain positive costs that endpoint improvements do not explain.

The [performance convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md)
and [lexical-host plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md)
remain active and HOLD. This receipt records six completed ordinary Windows campaigns;
it neither substitutes for the complete current-source Linux matrix nor completes the
remaining backing, promotion, selection and obsolete-owner work.

## Exact Scope and Independent Reconstruction

Control is `4c22eae585ee1e151e7521333d4d5a0a4d163638`; candidate is
`67e9d1c215372744d55ddc2ecd72a6a9fe6324af`. The candidate is the main integration of the
[qualified preparation-before-attachment implementation](2026-11-05-preparation-before-attachment.md).
Both versions contain the current glTF FLOAT direction repair. Historical `314d0`
workers are not the control for this comparison.

| Immutable input | Control SHA-256 | Candidate SHA-256 |
| --- | --- | --- |
| Ordinary `baozi-probe.exe` | `f3f7719623b1d26bff60e9d8cbb7a7e77570d1b5a35388643054b2ba2058d51b` | `0380fc17240dc48704e36da7674bfcb59a9ca236f8a4e2a50fb5e6a845ab4cc2` |
| Git source archive | `4a0add7cb535e1e86948b77a2347a92b740f231bb41a0661df9c7b5baa826602` | `89909baa56adba16c57661578d049d9e8f1241767c83562839b8aef9a7d27f80` |

Builds retain Rust 1.95.0, generic x86_64 MSVC, release opt3, System allocator, dynamic
CRT, debug 0 and no allocation instrumentation. Assimp remains pinned to 6.0.5 /
`392a658f9c271be965271f45e7521a1b80ea4392`, with the original separate F32/F64 workers.
The protocol, corpus, precision, fields, normal policy, work allowance and timing
boundaries are unchanged. Both engines still perform the original semantic task.

Each campaign covers the original fourteen cases and 110 lanes, with three warmup
pairs and twenty retained AB/BA pairs per lane. Independent raw reconstruction reads
all 30,360 native children and 60,720 stdout/stderr files, verifies serial execution,
arguments, pairing, corpus hashes, build/archive identities and every stored worker
object, and reconstructs all statistics without the main-derived summary as numeric
input. There are zero integrity errors and no reported geometry, field, policy, scalar
width or import-work signature changes. Every native child exits successfully.

Reported import work is captured before preparation; it is not a measurement of
postprocess counters. Aggregate geometry signatures do not establish arbitrary
tuple-by-tuple equality. The earlier small semantic tests and deterministic family
contract remain necessary.

## Collection Limits

Campaign roots below `target/assimp-comparison/` are:

| Label | Directory | Watched-tool observations |
| --- | --- | ---: |
| C0 | `convergence-windows-outer-host-control` | 0 |
| N0 | `convergence-windows-outer-host` | 0 |
| N1 | `convergence-windows-outer-host-confirm` | 4 |
| C1 | `convergence-windows-outer-host-control-confirm` | 90 |
| N2 | `convergence-windows-outer-host-clean-confirm` | 12 |
| C2 | `convergence-windows-outer-host-clean-control-confirm` | 7 |

The initial C0/N0 contrast has no watched-tool observations, which does not establish
global host quiet. The remaining four campaigns record Cargo/rustc/tool activity and
are descriptive contrasts, not clean confirmation. Directory names containing `clean`
do not override the observations. A 20-second quiet gate before N2/C2 does not establish
quiet during their children. There is no complete clean confirmation pair.

The decision to collect N2/C2 is prospective and linked to the earlier load receipt.
All six campaigns, samples, tails and positive costs remain retained. Nothing is
trimmed, pooled across campaigns or substituted. Repeating measurement without first
addressing the demonstrated costs is not the next development step.

Baozi/Assimp ratios below are medians after inverting each of the twenty raw
Assimp/Baozi pair ratios. They are not the reciprocal of the even-count median.
Version changes compare separate campaign medians; they are not contemporaneous C/N
pairs. Endpoint times are not summed to create a pipeline result.

## Benefits and Remaining Costs

All native times in this table are milliseconds. Later overlapped contrasts retain
the named signs but do not provide clean confirmation.

| Case / endpoint / IO | C0 | N0 | Initial change | N0 / Assimp |
| --- | ---: | ---: | ---: | ---: |
| Repeated OBJ 16384 / prepare / memory | 19.31980 | 13.33440 | -30.98% | 12.63x |
| Repeated OBJ 16384 / pipeline / memory | 55.22685 | 47.40995 | -14.15% | 4.89x |
| Grid OBJ 128 / import / memory | 165.54110 | 165.03005 | -0.31% | 6.98x |
| Grid OBJ 128 / pipeline / memory | 167.91000 | 166.82015 | -0.65% | 6.49x |
| Shared glTF 1 / import / memory | 1.57565 | 1.83820 | +16.66% | 8.69x |
| Shared glTF 1 / pipeline / memory | 1.87875 | 2.11720 | +12.69% | 7.76x |
| Shared glTF 32 / import / memory | 3.09050 | 3.38350 | +9.48% | 12.55x |
| Shared glTF 32 / pipeline / memory | 3.43430 | 3.60955 | +5.10% | 10.97x |
| Repeated STL 1024 / pipeline / memory | 3.05530 | 2.71190 | -11.24% | 11.56x |

Large repeated-OBJ preparation improves about 31% in both IO modes. Complete pipelines
fall 14.15%–14.53%, saving 7.82–8.13 ms initially. Twenty-five of 26 preparation medians
improve. Some preservation-only preparations fall about 35%–43%, but those gains are
generally tens of microseconds rather than multiple milliseconds. The large grid's
remaining import cost is about 165–166 ms; its small measured movements do not establish
a source-attributed improvement.

All twelve glTF import/pipeline lanes increase initially: import by 9.48%–18.85% and
pipeline by 5.10%–13.57%. Shared-8 file import grows from 2.17825 to 2.58895 ms, an
absolute 0.41070 ms. Both later descriptive contrasts retain every positive glTF sign.
These are unresolved costs; unchanged reported work is not an explanation for them.

Parent elapsed medians increase in 103/110 initial lanes, compared with 52/110 native
endpoint medians. F64 PLY prepare/file improves natively from 132.20 to 80.00 microseconds,
while whole-child elapsed grows from 41.62200 to 57.86050 ms. Shared-32 glTF pipeline/file
parent elapsed grows 24.79%, from 44.47765 to 55.50520 ms. Parent elapsed includes the
complete child lifetime; these observations do not identify a particular startup cost.

Initial whole-child peak working-set medians grow in 98/110 lanes, by 57,344–1,073,152
bytes. The two later contrasts also each have 98 positive lanes. Shared-1 glTF import/file
grows from 10,600,448 to 11,673,600 bytes, or 10.12%. Peak commit grows in 67 initial lanes,
with a maximum increase of 686,080 bytes. These version costs remain open even where
the external 3x memory target passes.

## Current Distance from the Targets

| Campaign | Native >3x / >=10x, of 110 | Pipeline >3x / >=10x, of 28 | Working set >3x | Commit >3x |
| --- | ---: | ---: | ---: | ---: |
| C0 | 60 / 37 | 9 / 3 | 0 | 0 |
| N0 | 58 / 37 | 9 / 3 | 0 | 0 |
| N1 | 58 / 37 | 9 / 3 | 0 | 0 |
| C1 | 60 / 36 | 9 / 3 | 0 | 0 |
| N2 | 59 / 38 | 9 / 3 | 0 | 0 |
| C2 | 60 / 37 | 9 / 3 | 0 | 0 |

N0 has nine import, 24 preparation, sixteen extraction and nine pipeline lanes above
3x. The nine failing pipelines are both IO modes of large repeated OBJ and grid OBJ,
all three shared-glTF memory modes, and both STL memory modes. Shared-32 glTF and the
two STL memory pipelines remain above 10x. Only 19/28 complete pipelines meet 3x in
the initial candidate, with clean confirmation still absent.

Every campaign meets the measured whole-child 3x memory bound. Maximum N0 working-set
and commit ratios are 1.82605x and 1.39344x; maxima across all six rounds are 1.82668x
and 1.39877x. Windows records `PeakWorkingSetSize` and `PeakPagefileUsage`; Linux RSS
is unavailable in these results. Neither counter is an endpoint-only or heap-only
peak. Separate heap attribution remains required.

Thirty-four N0 Assimp medians are below ten microseconds and twelve Assimp lanes have
IQR/median above 25%. For example, shared-8 prepare/memory is 89.90 versus 1.40 microseconds,
a recorded 64.63x pair ratio and an absolute 88.50-microsecond gap. Short denominators,
dispersion and the lack of clean confirmation remain explicit. With twenty samples,
high percentiles describe these observations rather than stable production tails.

## Evidence and Next Work

The independent report and complete six-campaign raw statistics are retained at:

- `.git/baozi-review/performance-convergence/outer-host-six-campaign-raw-review-1.md`
- `.git/baozi-review/performance-convergence/outer-host-six-campaign-raw-review-1.json`
- `.git/baozi-review/performance-convergence/outer-host-six-campaign-raw-audit-1.py`
- `target/review/convergence/outer-host-{C0,N0,N1,C1,N2,C2}-command-3.json`

The complete JSON includes all 660 lane/campaign combinations, warmup and retained
samples, signatures, source/build identities, observation intersections, dispersion,
tails, all version costs and every timing/memory ratio. The full initial candidate
table follows. The main-derived CSV at
`target/review/convergence/outer-host-six-campaign-main-table-1.csv` is separately
cross-checked against the independently reconstructed values.

Fresh matched Linux Callgrind collection for these exact source revisions has completed
twelve whole-worker profiles over six cases. Independent integrity/call-owner review is
separate from this Windows receipt. Instruction counts include setup, checking and
teardown; they are not endpoint time or Windows CPU percentages. In particular, Linux
instruction changes alone do not explain Windows glTF regressions.

Next work must attribute the positive costs and choose the demonstrated owner before
another implementation keep. Parse metadata and service construction are candidates
for investigation, not a proven universal explanation. Fixed/dynamic borrowed backing,
shared-shell-before-state promotion, exact administrative accounting, remaining
`LocalWork`/owning-path deletion and indexed selection remain open. Current-source
ordinary Linux fourteen-case/110-lane results, representative separate heap diagnostics,
regression repairs and final Windows/Linux acceptance are still required.

## Complete Initial Candidate Table

Native milliseconds and version changes are C0/N0. Ratio columns are the medians of
twenty Baozi/Assimp pair ratios. Working set and commit cover the whole child.

| Lane | Case | Endpoint | IO | C0 ms | N0 ms | Change | Native ratio | Working-set ratio | Commit ratio |
| ---: | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | obj-repeated-1 | import | file | 1.33575 | 1.40690 | +5.33% | 0.23783x | 0.44155x | 0.05976x |
| 2 | obj-repeated-1 | import | memory | 1.07365 | 1.11655 | +4.00% | 0.28495x | 0.43764x | 0.05996x |
| 3 | obj-repeated-1 | prepare | file | 0.47310 | 0.48800 | +3.15% | 21.36454x | 0.51081x | 0.05984x |
| 4 | obj-repeated-1 | prepare | memory | 0.47795 | 0.47345 | -0.94% | 21.13986x | 0.50731x | 0.06000x |
| 5 | obj-repeated-1 | extract | file | 0.01025 | 0.01185 | +15.61% | 8.73077x | 0.51162x | 0.05986x |
| 6 | obj-repeated-1 | extract | memory | 0.01005 | 0.01205 | +19.90% | 8.61538x | 0.50800x | 0.05993x |
| 7 | obj-repeated-1 | pipeline | file | 2.16090 | 2.23635 | +3.49% | 0.35400x | 0.51153x | 0.05989x |
| 8 | obj-repeated-1 | pipeline | memory | 1.70500 | 1.76395 | +3.46% | 0.43232x | 0.50795x | 0.06002x |
| 9 | obj-repeated-1024 | import | file | 3.59900 | 3.52400 | -2.08% | 0.56408x | 0.44093x | 0.10658x |
| 10 | obj-repeated-1024 | import | memory | 3.31415 | 3.23485 | -2.39% | 0.77162x | 0.43689x | 0.10723x |
| 11 | obj-repeated-1024 | prepare | file | 1.67385 | 1.31525 | -21.42% | 12.76306x | 0.50934x | 0.10655x |
| 12 | obj-repeated-1024 | prepare | memory | 1.72000 | 1.31670 | -23.45% | 12.61078x | 0.50587x | 0.10735x |
| 13 | obj-repeated-1024 | extract | file | 0.06780 | 0.06560 | -3.24% | 1.69151x | 0.51015x | 0.10642x |
| 14 | obj-repeated-1024 | extract | memory | 0.06700 | 0.06795 | +1.42% | 1.68586x | 0.50664x | 0.10732x |
| 15 | obj-repeated-1024 | pipeline | file | 5.73080 | 5.22890 | -8.76% | 0.75881x | 0.51015x | 0.10680x |
| 16 | obj-repeated-1024 | pipeline | memory | 5.23505 | 4.79515 | -8.40% | 1.09638x | 0.50650x | 0.10732x |
| 17 | obj-repeated-16384 | import | file | 35.35435 | 33.85315 | -4.25% | 3.47897x | 0.68393x | 0.64443x |
| 18 | obj-repeated-16384 | import | memory | 35.18755 | 33.25625 | -5.49% | 4.34120x | 0.68308x | 0.64740x |
| 19 | obj-repeated-16384 | prepare | file | 19.42260 | 13.44880 | -30.76% | 12.66288x | 0.68444x | 0.64402x |
| 20 | obj-repeated-16384 | prepare | memory | 19.31980 | 13.33440 | -30.98% | 12.62525x | 0.68320x | 0.64717x |
| 21 | obj-repeated-16384 | extract | file | 0.70865 | 0.69395 | -2.07% | 1.76502x | 0.68422x | 0.64453x |
| 22 | obj-repeated-16384 | extract | memory | 0.71640 | 0.69045 | -3.62% | 1.75782x | 0.68261x | 0.64793x |
| 23 | obj-repeated-16384 | pipeline | file | 55.97375 | 47.84290 | -14.53% | 3.94107x | 0.68444x | 0.64406x |
| 24 | obj-repeated-16384 | pipeline | memory | 55.22685 | 47.40995 | -14.15% | 4.88902x | 0.68283x | 0.64762x |
| 25 | obj-grid-2 | import | file | 1.40705 | 1.52920 | +8.68% | 0.25190x | 0.44535x | 0.05982x |
| 26 | obj-grid-2 | import | memory | 1.13860 | 1.22125 | +7.26% | 0.30902x | 0.44127x | 0.06010x |
| 27 | obj-grid-2 | prepare | file | 0.13585 | 0.07990 | -41.19% | 39.88947x | 0.45724x | 0.05987x |
| 28 | obj-grid-2 | prepare | memory | 0.13810 | 0.07890 | -42.87% | 43.83333x | 0.45340x | 0.06017x |
| 29 | obj-grid-2 | extract | file | 0.01325 | 0.01300 | -1.89% | 7.26754x | 0.45789x | 0.05986x |
| 30 | obj-grid-2 | extract | memory | 0.01315 | 0.01320 | +0.38% | 7.58824x | 0.45405x | 0.06038x |
| 31 | obj-grid-2 | pipeline | file | 1.90660 | 1.91320 | +0.35% | 0.29325x | 0.45784x | 0.06001x |
| 32 | obj-grid-2 | pipeline | memory | 1.42630 | 1.45990 | +2.36% | 0.36866x | 0.45396x | 0.06032x |
| 33 | obj-grid-32 | import | file | 11.46950 | 11.38220 | -0.76% | 1.60877x | 0.45307x | 0.16459x |
| 34 | obj-grid-32 | import | memory | 11.26390 | 10.93305 | -2.94% | 2.08647x | 0.44959x | 0.16953x |
| 35 | obj-grid-32 | prepare | file | 0.13925 | 0.08380 | -39.82% | 22.39069x | 0.45323x | 0.16445x |
| 36 | obj-grid-32 | prepare | memory | 0.13730 | 0.08375 | -39.00% | 23.39377x | 0.45032x | 0.16943x |
| 37 | obj-grid-32 | extract | file | 0.15865 | 0.15105 | -4.79% | 1.73971x | 0.45319x | 0.16435x |
| 38 | obj-grid-32 | extract | memory | 0.15815 | 0.14430 | -8.76% | 1.72824x | 0.45038x | 0.16932x |
| 39 | obj-grid-32 | pipeline | file | 12.38450 | 12.11995 | -2.14% | 1.50740x | 0.45310x | 0.16446x |
| 40 | obj-grid-32 | pipeline | memory | 11.79135 | 11.40500 | -3.28% | 2.12734x | 0.45038x | 0.16910x |
| 41 | obj-grid-128 | import | file | 166.75700 | 166.30200 | -0.27% | 6.44653x | 0.92904x | 0.95447x |
| 42 | obj-grid-128 | import | memory | 165.54110 | 165.03005 | -0.31% | 6.98446x | 0.93064x | 0.95722x |
| 43 | obj-grid-128 | prepare | file | 0.14975 | 0.08820 | -41.10% | 2.86316x | 0.92825x | 0.95305x |
| 44 | obj-grid-128 | prepare | memory | 0.15240 | 0.08800 | -42.26% | 2.75316x | 0.93033x | 0.95727x |
| 45 | obj-grid-128 | extract | file | 2.01165 | 1.91645 | -4.73% | 1.98378x | 0.92974x | 0.95428x |
| 46 | obj-grid-128 | extract | memory | 1.99825 | 1.92695 | -3.57% | 1.93744x | 0.93051x | 0.95728x |
| 47 | obj-grid-128 | pipeline | file | 168.76400 | 168.57390 | -0.11% | 5.99739x | 0.92912x | 0.95471x |
| 48 | obj-grid-128 | pipeline | memory | 167.91000 | 166.82015 | -0.65% | 6.49348x | 0.93021x | 0.95746x |
| 49 | gltf-shared-1 | import | file | 1.84755 | 2.13670 | +15.65% | 0.78912x | 1.76847x | 1.18330x |
| 50 | gltf-shared-1 | import | memory | 1.57565 | 1.83820 | +16.66% | 8.69155x | 1.78243x | 1.23531x |
| 51 | gltf-shared-1 | prepare | file | 0.13520 | 0.07975 | -41.01% | 53.61228x | 1.80253x | 1.15234x |
| 52 | gltf-shared-1 | prepare | memory | 0.13865 | 0.08060 | -41.87% | 61.80769x | 1.81833x | 1.22803x |
| 53 | gltf-shared-1 | extract | file | 0.01280 | 0.01295 | +1.17% | 9.96154x | 1.80600x | 1.15422x |
| 54 | gltf-shared-1 | extract | memory | 0.01250 | 0.01280 | +2.40% | 10.66667x | 1.82605x | 1.25703x |
| 55 | gltf-shared-1 | pipeline | file | 2.36545 | 2.59445 | +9.68% | 0.71486x | 1.80383x | 1.13840x |
| 56 | gltf-shared-1 | pipeline | memory | 1.87875 | 2.11720 | +12.69% | 7.75669x | 1.82344x | 1.22708x |
| 57 | gltf-shared-8 | import | file | 2.17825 | 2.58895 | +18.85% | 0.84615x | 1.76682x | 1.21818x |
| 58 | gltf-shared-8 | import | memory | 1.91455 | 2.20405 | +15.12% | 9.86940x | 1.78038x | 1.23060x |
| 59 | gltf-shared-8 | prepare | file | 0.14270 | 0.08690 | -39.10% | 51.23529x | 1.80490x | 1.18750x |
| 60 | gltf-shared-8 | prepare | memory | 0.14190 | 0.08990 | -36.65% | 64.63462x | 1.81896x | 1.28952x |
| 61 | gltf-shared-8 | extract | file | 0.01630 | 0.01675 | +2.76% | 4.04765x | 1.81054x | 1.14980x |
| 62 | gltf-shared-8 | extract | memory | 0.01640 | 0.01650 | +0.61% | 4.39189x | 1.82584x | 1.24243x |
| 63 | gltf-shared-8 | pipeline | file | 2.72680 | 3.09690 | +13.57% | 0.85328x | 1.80967x | 1.14892x |
| 64 | gltf-shared-8 | pipeline | memory | 2.22115 | 2.43125 | +9.46% | 8.86944x | 1.82123x | 1.24130x |
| 65 | gltf-shared-32 | import | file | 3.33255 | 3.72705 | +11.84% | 1.25629x | 1.75845x | 1.36635x |
| 66 | gltf-shared-32 | import | memory | 3.09050 | 3.38350 | +9.48% | 12.54871x | 1.77256x | 1.39308x |
| 67 | gltf-shared-32 | prepare | file | 0.17030 | 0.10770 | -36.76% | 49.93182x | 1.80043x | 1.34862x |
| 68 | gltf-shared-32 | prepare | memory | 0.17220 | 0.11135 | -35.34% | 62.12384x | 1.81595x | 1.39344x |
| 69 | gltf-shared-32 | extract | file | 0.03110 | 0.03255 | +4.66% | 2.75995x | 1.79382x | 1.28383x |
| 70 | gltf-shared-32 | extract | memory | 0.03030 | 0.03265 | +7.76% | 2.60589x | 1.81051x | 1.37476x |
| 71 | gltf-shared-32 | pipeline | file | 3.91080 | 4.19195 | +7.19% | 1.07890x | 1.80209x | 1.30418x |
| 72 | gltf-shared-32 | pipeline | memory | 3.43430 | 3.60955 | +5.10% | 10.97022x | 1.81107x | 1.37201x |
| 73 | stl-repeated-1 | import | file | 1.24665 | 1.31905 | +5.81% | 0.63023x | 1.50547x | 1.07460x |
| 74 | stl-repeated-1 | import | memory | 0.95520 | 0.99685 | +4.36% | 14.31858x | 1.51445x | 1.15054x |
| 75 | stl-repeated-1 | prepare | file | 0.13540 | 0.08120 | -40.03% | 54.40000x | 1.54627x | 1.07056x |
| 76 | stl-repeated-1 | prepare | memory | 0.13530 | 0.08380 | -38.06% | 60.53297x | 1.55819x | 1.14967x |
| 77 | stl-repeated-1 | extract | file | 0.01200 | 0.01220 | +1.67% | 11.81667x | 1.54998x | 1.07677x |
| 78 | stl-repeated-1 | extract | memory | 0.01210 | 0.01235 | +2.07% | 13.88333x | 1.56221x | 1.15871x |
| 79 | stl-repeated-1 | pipeline | file | 1.76570 | 1.73490 | -1.74% | 0.68409x | 1.54949x | 1.07229x |
| 80 | stl-repeated-1 | pipeline | memory | 1.26605 | 1.23625 | -2.35% | 11.30681x | 1.56034x | 1.15368x |
| 81 | stl-repeated-1024 | import | file | 3.06700 | 2.69615 | -12.09% | 1.27241x | 1.50549x | 1.18114x |
| 82 | stl-repeated-1024 | import | memory | 2.71255 | 2.41105 | -11.12% | 16.57164x | 1.51280x | 1.26104x |
| 83 | stl-repeated-1024 | prepare | file | 0.14060 | 0.08460 | -39.83% | 45.08889x | 1.54589x | 1.17714x |
| 84 | stl-repeated-1024 | prepare | memory | 0.13815 | 0.08425 | -39.02% | 45.84127x | 1.55059x | 1.26411x |
| 85 | stl-repeated-1024 | extract | file | 0.03590 | 0.03915 | +9.05% | 1.71429x | 1.54839x | 1.18450x |
| 86 | stl-repeated-1024 | extract | memory | 0.03505 | 0.04050 | +15.55% | 1.81439x | 1.55988x | 1.17010x |
| 87 | stl-repeated-1024 | pipeline | file | 3.62095 | 3.41860 | -5.59% | 1.14274x | 1.54274x | 1.18232x |
| 88 | stl-repeated-1024 | pipeline | memory | 3.05530 | 2.71190 | -11.24% | 11.55962x | 1.56007x | 1.17505x |
| 89 | ply-triangle-1 | import | file | 1.33855 | 1.50235 | +12.24% | 0.52384x | 1.25365x | 0.36038x |
| 90 | ply-triangle-1 | import | memory | 1.04250 | 1.10525 | +6.02% | 1.58100x | 1.26133x | 0.37412x |
| 91 | ply-triangle-1 | prepare | file | 0.13310 | 0.08175 | -38.58% | 54.10000x | 1.28866x | 0.36117x |
| 92 | ply-triangle-1 | prepare | memory | 0.13400 | 0.08440 | -37.01% | 56.26667x | 1.29638x | 0.37099x |
| 93 | ply-triangle-1 | extract | file | 0.01190 | 0.01185 | -0.42% | 10.68182x | 1.29127x | 0.36301x |
| 94 | ply-triangle-1 | extract | memory | 0.01150 | 0.01220 | +6.09% | 11.76818x | 1.29851x | 0.37263x |
| 95 | ply-triangle-1 | pipeline | file | 1.84360 | 1.86490 | +1.16% | 0.58785x | 1.28982x | 0.36104x |
| 96 | ply-triangle-1 | pipeline | memory | 1.33750 | 1.35745 | +1.49% | 1.78553x | 1.29771x | 0.37316x |
| 97 | ply-f64-neighbors | import | file | 1.33050 | 1.39815 | +5.08% | 0.51291x | 1.26355x | 0.36376x |
| 98 | ply-f64-neighbors | import | memory | 1.04775 | 1.11690 | +6.60% | 1.88552x | 1.26742x | 0.37421x |
| 99 | ply-f64-neighbors | prepare | file | 0.13220 | 0.08000 | -39.49% | 57.53571x | 1.29895x | 0.36370x |
| 100 | ply-f64-neighbors | prepare | memory | 0.13300 | 0.08015 | -39.74% | 60.15385x | 1.30198x | 0.37239x |
| 101 | ply-f64-neighbors | extract | file | 0.01195 | 0.01185 | -0.84% | 10.59091x | 1.30087x | 0.36320x |
| 102 | ply-f64-neighbors | extract | memory | 0.01185 | 0.01210 | +2.11% | 12.94444x | 1.30474x | 0.37333x |
| 103 | ply-f64-neighbors | pipeline | file | 1.85855 | 1.83565 | -1.23% | 0.57175x | 1.29984x | 0.36527x |
| 104 | ply-f64-neighbors | pipeline | memory | 1.33475 | 1.35290 | +1.36% | 1.79467x | 1.30439x | 0.37151x |
| 105 | ply-points-1024 | import | file | 1.70030 | 1.79620 | +5.64% | 0.65700x | 1.24206x | 0.35358x |
| 106 | ply-points-1024 | import | memory | 1.44070 | 1.50885 | +4.73% | 1.87619x | 1.24530x | 0.37646x |
| 107 | ply-points-1024 | extract | file | 0.02015 | 0.02155 | +6.95% | 4.52407x | 1.24447x | 0.35358x |
| 108 | ply-points-1024 | extract | memory | 0.02195 | 0.02320 | +5.69% | 5.53517x | 1.24808x | 0.37646x |
| 109 | ply-points-1024 | pipeline | file | 2.16775 | 2.18945 | +1.00% | 0.65292x | 1.24350x | 0.35488x |
| 110 | ply-points-1024 | pipeline | memory | 1.64545 | 1.68455 | +2.38% | 1.93141x | 1.24755x | 0.37603x |
