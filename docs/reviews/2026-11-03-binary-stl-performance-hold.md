# Binary STL Caller: Ordinary Performance Hold

2026-11-03. Independent reconstruction of the four completed eight-case, 31-lane memory-input campaigns, comparing the retained bridge runtime `67b5ca609470268b124fd1915741ee81105c4a24` with candidate `48d6f17e2f0829bdd5d50c8d7c48cea72ce343a1`.


This is the tracked export of the independent raw audit, SHA-256
`9c3e0efac9558f84521d0266dbc5cd13441bd9545d5aa5d9f837d3d8fbd0c7f5`.
The [source qualification](2026-11-03-binary-stl-exclusive-caller.md) remains valid;
performance acceptance is a separate HOLD. The
[convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md)
remains active. Local audit artifacts named below live under
`.git/baozi-review/performance-convergence/`; original campaign artifacts remain
under `target/assimp-comparison/`. Their names and hashes identify local retained
evidence, not artifacts distributed with this Markdown file.

## Findings and disposition

**HOLD performance keep. Reject a claim that this experiment establishes an overall performance improvement or an optimized STL import endpoint.** The source-qualified exclusive caller can remain a structural prerequisite for continued owner work, with the costs below OPEN. That is a structural continuation decision, not a measured optimization acceptance, universal no-regression finding, or release clearance. The current comparison does not identify the cause of the observed costs or gains.

1. **P2 — repair the repeated OBJ and grid import/pipeline regressions before granting performance keep.** Repeated OBJ import increases **34.27825 -> 36.90300 ms (+7.66%)** in the pilot and **34.38215 -> 36.99280 ms (+7.59%)** in reverse confirmation: **+2.62475 / +2.61065 ms**. Grid import increases **164.62195 -> 173.06785 ms (+5.13%)** and **164.65980 -> 173.01735 ms (+5.08%)**: **+8.44590 / +8.35755 ms**. Independently timed pipelines also worsen twice: repeated OBJ **+1.65380 / +2.47745 ms (+3.00% / +4.53%)**, grid **+8.21800 / +5.93040 ms (+4.93% / +3.51%)**. These are real costs in both source orders, not a scalar-score artifact. Locations: each campaign's `report.json#/series/{0,3,4,7}` and original retained `comparison-{001,004,005,008}-*-baozi` logs.
2. **P2 — retain the other preparation and extraction costs individually.** STL prepare is **+6.55 / +8.05 us (+4.42% / +5.39%)**, with p95 **+20.47% / +8.28%**; F64 prepare is **+8.35 / +3.35 us (+5.76% / +2.29%)**, p95 **+4.92% / +24.40%**. Both retain positive half/order comparisons, although F64 confirmation's last-half increase is only 0.03%. Grid extract is **+11.85 / +38.25 us (+0.63% / +2.04%)**, with worse maxima in both rounds; its pilot first-half result is nearly flat. Repeated OBJ prepare, grid prepare and glTF1 prepare also have small positive medians twice, documented below without claiming uniform split evidence. Locations: series 13/20/6, raw prefixes 014/021/007, and the full lane table.
3. **P2 — carry the repeated whole-child commit increases, especially glTF8.** glTF8 extract grows **58 / 62 KiB (+5.24% / +5.61%)**, pipeline **58 / 60 KiB (+5.24% / +5.43%)**. Points import/extract/pipeline grow **24–28 KiB**, glTF32 pipeline **18 / 20 KiB**, STL import **16 / 20 KiB** and preparation **20 / 20 KiB**. These remain explicit costs even where stage medians improve or working set falls. Locations: original per-child `windows_peak_commit_bytes` at series 29/30, 16–18, 11, 12/13. No selected Windows memory median ratio exceeds 3x; that does not make these increases zero.
4. **Preserve the positive extraction evidence, without assigning it to an unchanged algorithm.** F64 **-14.76% / -14.18%**, glTF1 **-12.54% / -13.01%**, glTF8 **-9.64% / -11.78%**, STL **-8.09% / -10.97%**, glTF32 **-11.06% / -4.74%**, and points **-4.96% / -4.46%** repeat in both rounds and all half/order groups. The comparison and extraction implementation are unchanged. These are version-associated native observations; neither source hashing nor these measurements proves a code-generation, allocator, arithmetic, or decoder mechanism.
5. **No measurement-integrity P1/P2 found.** All 5,704 child stdout/stderr pairs, task/geometry facts, statuses, retained membership, pair order, source/binary bindings and statistical summaries reconcile. The performance hold concerns the measured result, not a disqualified campaign. Main's provisional derived table was not used as numerical input to this audit.

The actionable next step is main-owned attribution of these repeated OBJ public-import and pipeline costs against the exact retained binaries/source states, followed by a source-qualified correction and fresh same-task ordinary evidence. Keep the existing task, budgets, explicit Smooth request, controls, failures and historical repairs. A later Linux instruction result may locate Linux executed work; it cannot clear these Windows costs or establish Windows code generation. No extra reader-hint experiment or widened benchmark task is justified by this audit.

## Frozen identities and scope

Chronological labels are **C0 -> N0 -> N1 -> C1**. Pilot compares N0/C0; confirmation compares N1/C1. Versions are serial campaigns, not source-interleaved children. Within each lane the engine pairs retain AB/BA alternation, with parity continuing through three warmups and across lanes. Every retained lane has ten Baozi-first and ten Assimp-first pairs.

| Label / directory | UTC start -> finish | Report SHA-256 |
| --- | --- | --- |
| C0 / `convergence-windows-binary-stl-cutover-control` | 09:25:06.163832+00:00 -> 09:26:31.965083+00:00 | `58b6d9508ec3985cb1fb9af47f4282b9c1eea42f8fc8c96ca89863474cb6beb6` |
| N0 / `convergence-windows-binary-stl-cutover` | 09:26:31.971311+00:00 -> 09:27:57.130223+00:00 | `75b6c3cc705e05d8b8b1a7eb8f2aaba210aca34a93d92f6422c3b3aadba7d716` |
| N1 / `convergence-windows-binary-stl-cutover-confirm` | 09:27:57.243677+00:00 -> 09:29:21.429881+00:00 | `23fb27aae6d899cc6fbcddb15c4bb21256d46ed1f389531bf8299fc92c19e84b` |
| C1 / `convergence-windows-binary-stl-cutover-control-confirm` | 09:29:21.436025+00:00 -> 09:30:47.408349+00:00 | `e6c18a195d5f7c3af680e546122ff8658b899c1a23d8dfc692a564adbdbd5690` |

All directories in that table are under `target/assimp-comparison/`. Completed command receipts are `target/review/convergence/binary-stl-cutover{,-control,-confirm,-control-confirm}-command.json`; the companion JSON pins their exact hashes and timestamps. Each receipt reports exit 0 and its original report hash matches independently. All child/tool intervals and the four version intervals are serial. Main reports no Cargo/profiler overlap; the retained candidate build ends at **09:24:46.789414Z**, before C0 begins **09:25:06.163832Z**. These records establish their own chronology, not the absence of unrelated machine activity.

| Identity | Control | Candidate |
| --- | --- | --- |
| Declared source revision | `37e800541be6cae31585e40c01828aa64b5809ef` | `48d6f17e2f0829bdd5d50c8d7c48cea72ce343a1` |
| Runtime identity | `67b5ca609470268b124fd1915741ee81105c4a24` | Same as declared source |
| Binary SHA-256 | `a3ec752a0060f5db24d5ba5a524645051ab07effe2e0145bf7adbe64907c806c` | `3d7a6ac3fac4002eaee5b95f814afbac2ab3134e8af8b2f0cbff6aeeaed9a089` |
| Binary bytes | 10,799,104 | 10,940,928 |
| Source archive SHA-256 | `7e29fab78351316ae775a0c489ca86f43fb40717b445d5bbfee6db59c00a937a` | `169d3fda4c70ab084264c0dda986e7e20bdc78fb251b69ebe4fecb36e3aedca4` |
| Build-manifest SHA-256 | `1b2693479412ab5381896e5ca0ac10853312ac9a2b0a9ac23438219192b4f3fb` | `1d46953f8ffbcc2ca43800fbca8c6b3e5140242bf6a30a12c6cfaf74cf1482bd` |

Rehashed all selected executables and both archives, and checked every report's captured build manifest. Both Rust workers have identical argv/toolchain/manifests/build policy: Rust 1.95.0, LLVM 22.1.2, generic x86_64 MSVC release opt3, debug 0, dynamic CRT/System allocator and no allocation metrics. Source linkage and the concrete stale-artifact concern were independently closed in `binary-stl-ordinary-candidate-association-review.md` (SHA-256 `e7491f4a033bb301d849721bb158874d138c8525e995df038d70e72e672248e4`): all 23 candidate archive files match qualified/current source and the commit, all 829 runtime/tools/scripts members match Git, exactly 22 differ from control, and fresh release compilation/dependency paths name the actual main workspace. The qualification receipt hash is `eb2905ebe03b7b88627dec4af05271374ea42b03c5ee3edbd9f478836cfbb4c3`. This audit checks that source chain, not a duplicate reconstruction of the separately reviewed qualification test inventory.

All four collection headers say `git_sha=48d6f17...`, `git_dirty=true`. Captured source-status logs contain only plans/review/knowledge notes; these are collection-checkout metadata. The immutable control worker is correctly bound to its own older source/archive, not relabeled 48d6. No later working source is substituted for either measured archive.

Assimp remains 6.0.5 at `392a658f9c271be965271f45e7521a1b80ea4392`. Rehashed F32 binary: `d3f5753ef588395a0cf7ec111d5af293a767bbcd58c894291a1b5ea836d18a83`; F64: `5db8fde3a7b23d9e804021409a470fd912e6ffcd5e6d0aeae569a0d9ec1db9a7`. Captured build-manifest hashes are `60c4c7358ebf05cf69b45c6b1f6c7b011bf8749c7827eff67ce8b73d153200f4` / `7fa97888b730bccbea7ba48ab66b3e568d92224e9827ce1da3b9a5c613ec799c`. Both reference workers and their F32/F64 routing match the accepted preflight.

The fixed script remains `203932e5cbc96fb0e8893bdc60b458de2e35d11cf88a267e38c4ba851cd60d10`. Every campaign uses the same collector (`9af6fbf91bb2fcdf8d36547ccef047e40b17ff1fad291b4664b22e2186dcea87`), protocol (`82e9735649e12024bd5052eee2861453118fdf68a29c33cab18dec47b7d6c565`) and finite corpus (`db52cc017949e3345a07ca51411a1ab42e47b05ee2c79aef63964e0796e73f49`). Comparison implementation hash remains `134933892e61ebfdcc581b60693ce4362c1e8ac6617a640e4b5ae23623ca12eb`; the CLI comparison branch and normal options also match both archives. The repeated OBJ still explicitly requests missing Smooth normals with authored normals preserved. The ordinary consumer's source-format policy is not substituted. Fields, transforms, precision, IO, budgets, setup, clocks, native cleanup and detached output remain unchanged. No eight-case aggregate is compared to an older six-case aggregate.

## Complete raw reconstruction

The reviewer-owned offline program `binary-stl-ordinary-raw-audit.py` reads every original report, command receipt, child stdout/stderr, captured task/build metadata and selected input. It imports no collector implementation and launches no worker. The companion `binary-stl-ordinary-raw-summary.json` retains all raw retained metric arrays, warmup stage values, full non-timing signatures, median/quartiles/IQR/min/p90/p95/max, descriptive Tukey outliers, half/order comparisons, paired ratios, memory and version deltas. SHA-256: **`9e7016662e50ee5016088575760b2e4b46d64947c98463f5fc216473ad4c67b0`**. Its validation-error list is empty.

- **5,704 children = 4,960 retained + 744 warmups**: 1,426 children/campaign, 1,240 retained and 186 warmups. All launched, returned 0, emitted success JSON and had empty stderr. No warmup, lane, failure or outlier is omitted. All 2,480 retained engine pairs match; every lane has exactly 20 pairs.
- Read all **11,408 child stdout/stderr files**, plus 16 captured Git-tool logs. Each directory's raw stdout/stderr file set exactly equals its recorded command file set: no missing, duplicate or extra sample log. Per-campaign identity-set hashes cover 2,856 files using sorted UTF-8 `relative artifact path + NUL + lowercase file SHA-256 + LF`.
- Parsed every original worker object and compared it with captured `worker_json`. Independently checked exact argv, engine/stage/IO/fields/normal policy/budget, selected worker, all input size/hashes, integer counts, finite analytic bounds/area/normals/UV/translation facts and source/output widths. OBJ import's missing normals and Assimp glTF import's native V convention retain their documented exceptions.
- Retained/source precision is 4/8 as declared; observation and copied output are 8. Baozi's native computation width remains null, Assimp's is its matching library width. Budget enforcement remains true for Baozi and honestly false for Assimp.
- Import/prepare copied output remains null. Extract/pipeline logical payload independently equals selected F64 tuple components plus placement transform/index records; capacity covers logical bytes. Baozi's eight-byte placement index and Assimp's four-byte index explain their established cross-engine byte difference. All **62 lane/engine non-timing signatures are exactly identical within and across the four campaigns**, including `import_work`, output logical/capacity bytes, widths and observations. Unlike the earlier cardinality experiment, this source contrast has no work-diagnostic exception.
- Reconstructed **3,596 summary slots**: 1,736 engine metric/output slots and 1,860 paired/matched-engine slots, including null metrics. Original retained membership, statuses, count, median, inclusive IQR, min/max, pair arrays and matched-engine statistics reconcile. Floating-point comparisons allow final-bit interpolation differences within `1e-12`; no discrepant integer timing/memory statistic was found. Percentiles use inclusive linear interpolation. Outlier flags are descriptive only; all values remain.

This proves finite observation/task/statistical consistency, not arbitrary-mesh equivalence from aggregate bounds. Full tuple bits, authored-normal behavior, failure prefixes and ownership semantics rely on the separate qualified source/tests. No tests were executed here.

## Every stage lane

Times are **milliseconds**, `median [IQR]`. Positive deltas mean slower. Pilot is N0/C0; confirmation is N1/C1. Assimp movement is shown independently and never subtracted from Baozi's native cost. Final-column ratios are medians of retained per-pair **Baozi/Assimp**, not the inverse of a ratio median or a sum of independently timed endpoints.

| Case / stage | Pilot C0 -> N0 ms [IQR] | Confirmation C1 -> N1 ms [IQR] | Baozi delta % P / C | Assimp delta % P / C | Candidate paired B/A P / C |
| --- | --- | --- | ---: | ---: | ---: |
| Repeated OBJ import | 34.27825 [0.47918] -> 36.90300 [0.66750] | 34.38215 [0.52950] -> 36.99280 [0.72062] | +7.66 / +7.59 | +0.38 / -0.01 | 4.765 / 4.714 |
| Repeated OBJ prepare | 19.29015 [0.25393] -> 19.44615 [0.49928] | 19.32620 [0.27225] -> 19.48285 [0.48150] | +0.81 / +0.81 | -0.09 / -1.15 | 18.007 / 17.778 |
| Repeated OBJ extract | 0.67935 [0.01460] -> 0.69560 [0.02545] | 0.68970 [0.02560] -> 0.68890 [0.02325] | +2.39 / -0.12 | -1.14 / -1.52 | 1.752 / 1.720 |
| Repeated OBJ pipeline | 55.17965 [1.27513] -> 56.83345 [0.45322] | 54.68450 [0.91017] -> 57.16195 [0.78995] | +3.00 / +4.53 | -1.01 / +0.54 | 5.847 / 5.924 |
| Grid OBJ import | 164.62195 [1.89760] -> 173.06785 [2.13883] | 164.65980 [0.97347] -> 173.01735 [1.78588] | +5.13 / +5.08 | +0.82 / -0.37 | 7.192 / 7.227 |
| Grid OBJ prepare | 0.15615 [0.01150] -> 0.15910 [0.01345] | 0.15585 [0.00783] -> 0.15785 [0.00695] | +1.89 / +1.28 | -0.89 / +0.15 | 4.877 / 4.935 |
| Grid OBJ extract | 1.87895 [0.05752] -> 1.89080 [0.01773] | 1.87435 [0.08987] -> 1.91260 [0.07115] | +0.63 / +2.04 | -2.51 / -1.95 | 1.881 / 1.916 |
| Grid OBJ pipeline | 166.68095 [1.44350] -> 174.89895 [2.09843] | 168.84335 [2.00398] -> 174.77375 [1.53503] | +4.93 / +3.51 | +0.48 / -0.77 | 6.833 / 6.765 |
| glTF32 import | 3.15185 [0.09233] -> 3.12475 [0.12015] | 3.20815 [0.11105] -> 3.16180 [0.12960] | -0.86 / -1.44 | -2.20 / -2.42 | 11.720 / 11.721 |
| glTF32 prepare | 0.18115 [0.00553] -> 0.17725 [0.00755] | 0.18575 [0.00940] -> 0.18205 [0.01192] | -2.15 / -1.99 | +2.78 / -2.63 | 98.417 / 104.132 |
| glTF32 extract | 0.03255 [0.00155] -> 0.02895 [0.00145] | 0.03270 [0.00137] -> 0.03115 [0.00225] | -11.06 / -4.74 | -3.56 / -2.29 | 2.361 / 2.568 |
| glTF32 pipeline | 3.49745 [0.09183] -> 3.55270 [0.10998] | 3.60635 [0.14988] -> 3.51260 [0.15323] | +1.58 / -2.60 | +0.26 / -1.96 | 10.357 / 10.342 |
| STL import | 2.81195 [0.10272] -> 2.82870 [0.11263] | 2.87660 [0.07940] -> 2.79320 [0.09515] | +0.60 / -2.90 | -0.94 / -4.85 | 19.764 / 19.631 |
| STL prepare | 0.14825 [0.00573] -> 0.15480 [0.00900] | 0.14940 [0.00810] -> 0.15745 [0.01180] | +4.42 / +5.39 | -2.63 / +5.56 | 81.789 / 84.608 |
| STL extract | 0.03895 [0.00272] -> 0.03580 [0.00315] | 0.04055 [0.00528] -> 0.03610 [0.00345] | -8.09 / -10.97 | -1.45 / -11.84 | 1.780 / 1.804 |
| STL pipeline | 3.14075 [0.11357] -> 3.12475 [0.11367] | 3.24685 [0.10780] -> 3.17735 [0.13805] | -0.51 / -2.14 | -0.76 / -3.30 | 13.283 / 13.205 |
| Points import | 1.48200 [0.05290] -> 1.45000 [0.06490] | 1.49145 [0.05590] -> 1.43860 [0.08555] | -2.16 / -3.54 | -0.21 / -0.02 | 2.091 / 2.079 |
| Points extract | 0.02320 [0.00120] -> 0.02205 [0.00085] | 0.02355 [0.00280] -> 0.02250 [0.00217] | -4.96 / -4.46 | +6.67 / +2.25 | 4.523 / 4.746 |
| Points pipeline | 1.64490 [0.05183] -> 1.60710 [0.07262] | 1.69570 [0.07902] -> 1.66710 [0.06625] | -2.30 / -1.69 | -3.95 / +0.42 | 2.194 / 2.218 |
| F64 import | 1.05200 [0.08805] -> 1.02620 [0.05758] | 1.09390 [0.05620] -> 1.05290 [0.05300] | -2.45 / -3.75 | -3.24 / +4.34 | 1.915 / 1.868 |
| F64 prepare | 0.14505 [0.00995] -> 0.15340 [0.01030] | 0.14660 [0.00825] -> 0.14995 [0.01667] | +5.76 / +2.29 | -15.15 / +6.67 | 110.308 / 96.298 |
| F64 extract | 0.01355 [0.00075] -> 0.01155 [0.00135] | 0.01340 [0.00112] -> 0.01150 [0.00090] | -14.76 / -14.18 | +0.00 / +10.00 | 10.918 / 10.595 |
| F64 pipeline | 1.34040 [0.06588] -> 1.36910 [0.08858] | 1.39900 [0.05445] -> 1.37160 [0.08927] | +2.14 / -1.96 | +1.78 / +1.39 | 2.274 / 2.315 |
| glTF1 import | 1.65760 [0.11640] -> 1.60170 [0.04705] | 1.65910 [0.07485] -> 1.62930 [0.08370] | -3.37 / -1.80 | -2.91 / -3.31 | 7.209 / 7.409 |
| glTF1 prepare | 0.14630 [0.00540] -> 0.14730 [0.01282] | 0.14785 [0.01115] -> 0.14965 [0.01310] | +0.68 / +1.22 | +7.14 / +3.45 | 97.000 / 99.631 |
| glTF1 extract | 0.01435 [0.00095] -> 0.01255 [0.00060] | 0.01460 [0.00103] -> 0.01270 [0.00145] | -12.54 / -13.01 | +8.33 / +8.33 | 9.777 / 9.514 |
| glTF1 pipeline | 1.89920 [0.07755] -> 1.91920 [0.04827] | 1.92385 [0.05930] -> 1.90565 [0.06995] | +1.05 / -0.95 | -2.16 / -5.14 | 7.203 / 7.294 |
| glTF8 import | 1.95115 [0.07773] -> 1.91670 [0.08565] | 1.96745 [0.08060] -> 1.94860 [0.18585] | -1.77 / -0.96 | -0.78 / -1.15 | 8.361 / 8.649 |
| glTF8 prepare | 0.15330 [0.00500] -> 0.16190 [0.01153] | 0.15935 [0.01720] -> 0.15430 [0.00485] | +5.61 / -3.17 | +9.09 / -3.12 | 95.751 / 103.783 |
| glTF8 extract | 0.01815 [0.00198] -> 0.01640 [0.00115] | 0.01825 [0.00183] -> 0.01610 [0.00108] | -9.64 / -11.78 | +1.27 / -5.00 | 4.139 / 4.285 |
| glTF8 pipeline | 2.27375 [0.10440] -> 2.30525 [0.12473] | 2.29300 [0.03623] -> 2.30735 [0.10453] | +1.39 / +0.63 | -5.07 / +0.48 | 8.047 / 7.980 |

## Controls, order, dispersion and actual benefit limits

The two OBJ import costs persist in both halves and both engine positions. Repeated OBJ half changes are **+6.99/+8.51%** pilot and **+6.96/+8.35%** confirmation; order groups **+7.98/+7.36%** and **+7.61/+7.43%**. Its complete old/new ranges are disjoint in both rounds. Grid import half changes are **+5.19/+5.05%** and **+5.44/+4.48%**; order groups **+4.45/+5.16%** and **+4.41/+5.31%**. Grid's pilot has an old high outlier overlapping the new range; confirmation's full ranges are disjoint. Thus do not claim all four distributions are completely separated.

Both OBJ pipeline increases also persist in every half/order comparison. Same-build import drift is tiny: repeated control/candidate **+0.30/+0.24%**, grid **+0.02/-0.03%**. Matched Assimp import movements are repeated **+0.38/-0.01%**, grid **+0.82/-0.37%**. Neither whole-campaign reversal, halves, engine position, a single outlier, nor a similar comparator shift explains away the repeated version cost. This is strong native evidence for these binaries, not a proof that one particular operation adapter, branch, or generated call is responsible.

STL import itself is **+0.60% / -2.90%**; pilot half/order results are mixed. STL pipeline is **-0.51% / -2.14%**, with mixed pilot half/order evidence. Its control drifts **+2.30%** import and **+3.38%** pipeline between rounds, while Assimp confirmation improves **4.85% / 3.30%** in the corresponding source comparison. The evidence does not establish a repeated material native STL-import gain or a 5% pipeline win. Keep its modest literal pipeline reductions, but do not call the decoder optimization proven from source-level removal of an adapter.

The six extraction improvements listed first retain negative half/order deltas throughout. Absolute changes are F64 **-2.00/-1.90 us**, glTF1 **-1.80/-1.90 us**, glTF8 **-1.75/-2.15 us**, STL **-3.15/-4.45 us**, glTF32 **-3.60/-1.55 us**, points **-1.15/-1.05 us**. glTF32's candidate itself drifts **+7.60%** across rounds, so its varying -11.06/-4.74% magnitude is not a single portable gain. Assimp movements also matter: STL extract **-1.45/-11.84%**, glTF1 **+8.33/+8.33%**, F64 **0.00/+10.00%**, points **+6.67/+2.25%**. These do not cancel Baozi's improvements but limit mechanism and ratio claims. No unchanged extraction mathematics is credited with an algorithmic improvement.

Points import **-2.16/-3.54%** and pipeline **-2.30/-1.69%** also retain negative half/order results. F64 import improves **-2.45/-3.75%**, but its pipeline changes sign (**+2.14/-1.96%**). glTF32 and glTF1 pipeline changes reverse sign; glTF8 pipeline has small positive medians twice (**+31.50/+14.35 us, +1.39/+0.63%**) but mixed halves/orders in both rounds. These remain bounded observations, not an overall pipeline benefit.

Assimp's largest stage shifts are tiny preparation/extraction clocks: F64 prepare **-15.15/+6.67%**, F64 extract **0.00/+10.00%**, glTF8 prepare **+9.09/-3.12%** and STL extract above. Absolute times and raw arrays are retained to avoid inflating quantized percentages. Host load averages are unavailable on Windows; the parent records QueryPerformanceCounter resolution 100 ns, while worker resolution is not independently established. Fresh serial children with warm caches do not imply CPU isolation or cold caches.

## Tails and whole-child elapsed

| Endpoint | Pilot C0 -> N0 median / p95 / max us | Confirmation C1 -> N1 median / p95 / max us |
| --- | --- | --- |
| Repeated OBJ extract | 679.350 / 750.065 / 964.100 -> 695.600 / 771.480 / 782.500 | 689.700 / 749.150 / 966.700 -> 688.900 / 785.255 / 790.100 |
| Grid OBJ extract | 1878.950 / 2049.845 / 2113.400 -> 1890.800 / 2081.795 / 3846.800 | 1874.350 / 2092.985 / 2292.200 -> 1912.600 / 2167.110 / 3786.100 |
| glTF32 extract | 32.550 / 40.350 / 43.200 -> 28.950 / 31.575 / 38.700 | 32.700 / 36.590 / 38.300 -> 31.150 / 34.790 / 36.500 |
| STL prepare | 148.250 / 158.100 / 213.200 -> 154.800 / 190.460 / 214.400 | 149.400 / 165.080 / 170.400 -> 157.450 / 178.755 / 179.800 |
| STL extract | 38.950 / 43.095 / 48.700 -> 35.800 / 46.910 / 56.600 | 40.550 / 51.035 / 57.400 -> 36.100 / 40.595 / 48.100 |
| Points extract | 23.200 / 27.800 / 31.600 -> 22.050 / 24.930 / 25.500 | 23.550 / 28.975 / 30.400 -> 22.500 / 27.605 / 27.700 |
| F64 prepare | 145.050 / 170.800 / 170.800 -> 153.400 / 179.200 / 183.000 | 146.600 / 161.760 / 284.500 -> 149.950 / 201.225 / 268.200 |
| F64 extract | 13.550 / 14.610 / 14.800 -> 11.550 / 14.770 / 16.100 | 13.400 / 18.520 / 18.900 -> 11.500 / 15.845 / 16.700 |
| glTF1 extract | 14.350 / 16.715 / 17.000 -> 12.550 / 15.235 / 19.700 | 14.600 / 18.690 / 37.500 -> 12.700 / 15.970 / 17.300 |
| glTF8 extract | 18.150 / 23.175 / 30.300 -> 16.400 / 17.700 / 17.700 | 18.250 / 22.805 / 28.600 -> 16.100 / 17.050 / 18.000 |

All tail entries retain their raw outliers. Repeated/grid import p95 rises **8.14/8.75%** and **5.31/5.44%**; their pipeline p95 rises **2.39/4.94%** and **5.23/2.99%**. Grid extraction maxima grow to **3.8468/3.7861 ms** against controls **2.1134/2.2922 ms**, despite its smaller median differences. F64 extraction's pilot p95 rises slightly despite the lower median, and STL extraction's pilot p95 also rises. glTF32 prepare has lower medians twice but p95 **185.655 -> 191.810 us** and **197.000 -> 199.760 us**; its confirmation maximum is **267.4 us**. A median benefit does not establish a tail repair.

glTF8 prepare is mixed rather than cleared: pilot **+5.61%** with p95 **163.040 -> 214.470 us**, confirmation **-3.17%** with p95 **241.305 -> 167.040 us**. glTF8 pipeline p95 is **-2.75/+2.90%**, with confirmation maximum **2.4131 -> 2.7742 ms**. The original preparation/tail concern therefore remains separately visible.

Parent elapsed includes startup, prior work, validation and serialization. It must not replace or correct stage timers. Repeated OBJ import parent median is **+3.45/+3.77%**, pipeline **+0.90/+3.56%**; grid import **+2.24/+3.66%**, pipeline **+5.53/+1.37%**. Other parent results are strongly mixed: STL pipeline **+11.87/-13.55%**, F64 prepare **-2.81/+35.52%**, glTF1 prepare **-12.70/+4.40%**. All parent arrays/p90/p95/max are in the companion JSON; no universal end-user or startup win is claimed.

Large parent outliers remain, including N0 grid-prepare retained-13 Baozi **962.229 ms** with stage **152.8 us**, C1 repeated-prepare retained-2 Baozi **952.154 ms** with stage **19.2433 ms**, N1 points-import retained-17 Baozi **915.080 ms** with stage **1.5148 ms**, and N0 STL-prepare retained-1 Baozi **883.913 ms** with stage **157.7 us**. Assimp C0 glTF32-prepare retained-16 is **873.348 ms** with stage **1.7 us**. These observations preclude a claim of uniform whole-process stability; no elapsed/stage subtraction or trimming is used.

## Every lane's Windows memory

These are whole-child lifetime **PeakWorkingSetSize** and **PeakPagefileUsage** captured with a retained query handle, including setup/import/preparation/final checking. All children have both positive measurements. `peak_rss_bytes` is null with the platform reason; heap metrics are unavailable. No stage-specific RSS, allocation count, allocator-request peak or live-scene footprint is inferred from these high-water marks.

Table entries are candidate-minus-control median **KiB**, pilot / confirmation. All memory min/p90/p95/max and original values remain in the companion JSON.

| Case / stage | Working-set delta KiB P / C | Commit delta KiB P / C |
| --- | ---: | ---: |
| Repeated OBJ import | -72.0 / -28.0 | -624.0 / -614.0 |
| Repeated OBJ prepare | -56.0 / -58.0 | -586.0 / -612.0 |
| Repeated OBJ extract | -66.0 / -60.0 | -592.0 / -592.0 |
| Repeated OBJ pipeline | -58.0 / -62.0 | -596.0 / -610.0 |
| Grid OBJ import | -58.0 / -62.0 | +0.0 / +0.0 |
| Grid OBJ prepare | -58.0 / -60.0 | +0.0 / +0.0 |
| Grid OBJ extract | -82.0 / -52.0 | +0.0 / +4.0 |
| Grid OBJ pipeline | -64.0 / -30.0 | -4.0 / +0.0 |
| glTF32 import | +30.0 / +24.0 | -2.0 / +6.0 |
| glTF32 prepare | +44.0 / +44.0 | +2.0 / +8.0 |
| glTF32 extract | +40.0 / +38.0 | +28.0 / +6.0 |
| glTF32 pipeline | +40.0 / +40.0 | +18.0 / +20.0 |
| STL import | +32.0 / +32.0 | +16.0 / +20.0 |
| STL prepare | +64.0 / +64.0 | +20.0 / +20.0 |
| STL extract | +60.0 / +60.0 | -54.0 / -56.0 |
| STL pipeline | +60.0 / +60.0 | -52.0 / -60.0 |
| Points import | -36.0 / -32.0 | +28.0 / +24.0 |
| Points extract | -40.0 / -40.0 | +26.0 / +28.0 |
| Points pipeline | -36.0 / -40.0 | +26.0 / +28.0 |
| F64 import | -16.0 / -16.0 | +2.0 / +0.0 |
| F64 prepare | +8.0 / +8.0 | +2.0 / +2.0 |
| F64 extract | +4.0 / +0.0 | -2.0 / +0.0 |
| F64 pipeline | +4.0 / +4.0 | -4.0 / +2.0 |
| glTF1 import | +28.0 / +26.0 | -8.0 / -4.0 |
| glTF1 prepare | +42.0 / +44.0 | -4.0 / -4.0 |
| glTF1 extract | +40.0 / +36.0 | +0.0 / +4.0 |
| glTF1 pipeline | +40.0 / +40.0 | +0.0 / +4.0 |
| glTF8 import | +16.0 / +16.0 | +6.0 / -4.0 |
| glTF8 prepare | +30.0 / +32.0 | +2.0 / +0.0 |
| glTF8 extract | +28.0 / +28.0 | +58.0 / +62.0 |
| glTF8 pipeline | +28.0 / +28.0 | +58.0 / +60.0 |

Repeated OBJ commit medians fall **586–624 KiB (about 4.25–4.52%)** across its four stage-worker lanes, while selected output capacity/logical bytes remain exactly unchanged. STL extraction/pipeline commit also falls **52–60 KiB**; its import/preparation rises as noted above. Do not infer which allocation or retained object caused these whole-process differences.

Working-set medians fall in repeated/grid/points lanes, while glTF and STL lanes generally rise; small F64 changes are mixed. The candidate executable grows **138.5 KiB**, but this alone does not explain the memory distribution or prove a code-layout cause. glTF8's repeated **58–62 KiB** commit cost is the largest positive median increase, about **5.24–5.61%**. glTF1 extract/pipeline median commit is unchanged in the pilot and **+4 KiB** in confirmation; that does not erase its earlier R7/R8 commit costs on different version comparisons. GlTF32 extract's **+28/+6 KiB** is narrower than its repeated pipeline **+18/+20 KiB** increase; keep both magnitudes rather than relabeling all glTF memory stable.

Maximum candidate paired memory medians are working set **1.69457x / 1.69421x** and commit **1.31819x / 1.31621x**. None of the 31 selected memory lanes exceeds 3x in either campaign, under either Windows method. This does not qualify unmeasured file-mode, other-corpus, Linux or heap limits.

## Release gates and prior-cost continuity

All four campaigns still have **23/31 stage lanes and 6/8 pipelines above 3x**. Candidate maximum paired stage ratios are **110.31x / 104.13x**, compared with control **100.93x / 105.93x**. Candidate still has **11/31 lanes above 10x** in both rounds; the fresh control has 12/31. glTF1 extraction drops below 10x in this pair, while F64 extraction remains **10.92x / 10.60x**, glTF32 pipeline **10.36x / 10.34x** and STL pipeline **13.28x / 13.20x**. This is not removal of all confirmed tenfold gaps or satisfaction of the all-endpoint 3x target.

No-op preparation remains the large lifecycle deficit. Candidate glTF1 is **147.30/149.65 us** against Assimp **1.50/1.50 us**; glTF32 **177.25/182.05 us** against **1.85/1.85 us**; STL **154.80/157.45 us** against **1.85/1.90 us**; F64 **153.40/149.95 us** against **1.40/1.60 us**. Those are ratios of medians for illustration; the preceding paired-ratio maxima are calculated per pair. Real repeated-OBJ Smooth preparation is **19.44615/19.48285 ms** versus Assimp **1.07455/1.10905 ms**, still about 18x. Faster selected extraction does not exempt any preparation, import or pipeline endpoint.

The following accepted evidence remains intact and is not automatically closed by this different matched version pair:

- **R4:** F64 extraction's **+34.80/+28.97%** original cost, with its exact historical controls, remains in `bound-reader-measurement-review.md`. The current F64 decrease is preserved, but its still-greater-than-10x ratio is not a repair of the release gap.
- **R5:** grid extraction confirmation **+105.10 us / +5.36%**, plus the associated control/drift qualification, remains in `normal-actions-measurement-review.md`. Current grid extraction is itself positive twice, with larger maxima; no grid repair is established.
- **R7:** glTF1/8 extraction **+10.85/+9.63%** and **+6.85/+5.74%**, glTF32 confirmation **+5.12%**, F64 **+.70/+.85 us**, and small STL/glTF1 commit increases remain in `gltf-profile-measurement-review.md`. Current decreases are not retroactive erasure of those source contrasts or proof of their cause.
- **R8 cardinality:** six extraction costs (F64, glTF1/8/32, STL, points), glTF8 prepare, glTF1 commit, pipeline/parent tails and their exact accepted intermediate dispositions remain in `profile-cardinality-measurement-review.md`. Its original counter-vocabulary change is not replayed or relabeled here: current work signatures are unchanged. Improvements in this later source pair do not silently grant closure to those repairs.
- Rejected reader-hint and node experiments remain rejected, with original positive and negative evidence preserved. This campaign does not revive them or qualify later owner work.

Preserve this pair as the measured baseline for the next source-qualified correction. Continue structural ownership work only with explicit costs carried forward; keep performance acceptance on HOLD until the repeated OBJ import/pipeline regressions are attributed and repaired or receive a documented bounded disposition supported by the parent goal. No release, cross-platform safety, universal no-regression, optimized-STL-parser or aggregate-winner claim is supported now.

## Audit artifact closure

Only new reviewer artifacts were written. No completed report, log, collector, budget, task, production source or historical review was modified; no Cargo, compiler, formatter, test, worker, profiler, benchmark or outside agent was launched. The offline program's hash is embedded in the companion JSON. Raw-log identity-set hashes below cover all captured child and tool stdout/stderr; they are NUL-encoding hashes, not JSON hashes.

| Campaign | Captured stdout/stderr files | NUL-encoded raw-log identity-set SHA-256 |
| --- | ---: | --- |
| C0 | 2856 | `73fdb34fa341db952a6c5f7d5766b0e937901581a90c9e7334b3ee4de4ec3f94` |
| N0 | 2856 | `0a1f14264443746c630c0529878f88df6b948702fed7ac66405a0dfb1136013e` |
| N1 | 2856 | `0ddfd70ea1d9708d6018591d26840327bc58c0cac77ec7376738812064d42188` |
| C1 | 2856 | `fea899040e55667080da51d0dc616f0369312a1cd452034e4ab771fd78deb852` |


## Matched Linux Instruction Checkpoint

A separate serialized build/profile sequence completed after all ordinary campaigns.
It uses those exact source archives and input/task identities, fresh separate targets
because of the previously observed artifact/source association failures, Rust 1.95.0
release with debug symbols level 1, no allocation metrics, and Callgrind 3.26.0.
These are whole-worker instructions including setup, checking and teardown. They are
neither native endpoint timings nor Windows code-generation explanations.

| Whole-worker pipeline | Control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| Repeated OBJ | 976,698,068 | 1,011,204,654 | +3.53% |
| Grid OBJ | 3,119,221,307 | 3,232,140,919 | +3.62% |
| Binary STL | 34,515,977 | 34,224,811 | -0.84% |

All six raw `summary: Ir` values equal the sum of every exclusive flat row.
Main's machine-readable reconstruction is
`target/review/convergence/binary-stl-linux-attribution-main-summary-1.json`,
SHA-256 `94479dfb8949c1b647bf4f0d1cdffd8505ca8a2d3fe0d5e50c1419eab15dfcc3`.
The completed [independent call-owner review](../research/2026-11-03-ordinary-storage-binding-costs.md)
distinguishes actual repeated loan construction from common checkpoint work.
Legacy storage construction executes 147,509 / 444,732 times for repeated/grid OBJ;
two constructor self costs total 13,275,810 / 40,025,880 Ir, with no poll in those
constructors. Scope checkpoint routes grow 93 to 104 Ir per entry while all real
poll counts and the poll primitive's exclusive cost remain unchanged. These are
bounded Linux owner observations; they do not establish a future removable saving.
The six-owner reservation replacement net is only +297,733 / +892,485 Ir, so gross
new lookup-owner totals are not additional work by themselves. Extra memcpy cost
is concentrated in `with_work` and `try_push_reserved` caller contexts; the existing
vector-growth copy route is unchanged. Copied object/bytes and Windows causes
remain unestablished. Neither this attribution nor the STL instruction reduction
clears the ordinary Windows regressions.

| Symbol worker | Control | Candidate |
| --- | --- | --- |
| Binary SHA-256 | `538abbbd81cb10c404b76c46c6fd5e191b82a8595ca9e1eb4b0883e44c6cb645` | `ad4c62f0f26fea5f995f7b3e12ec07e867e4c785ea15e307ae3df52692bd16bb` |
| Binary bytes | 63,277,936 | 63,764,200 |
| Source archive | Same control archive qualified above | Same candidate archive qualified above |

Build receipts are `target/assimp-comparison/convergence-linux-binary-stl-{control,candidate}-symbols/build.json`.
The six exact profiles, full flat/call annotations, worker observations, stdout/stderr
and per-profile identities are under `target/review/convergence/binary-stl-attribution/`.
The independent call-owner review is complete; the next owner correction remains pending.
These profiling invocations supply no Linux ordinary campaign or
caller correctness gate. A separate later exact-archive nextest/docs/Clippy run passes
Linux operation/import/STL 1457/1457, docs 30 + 54 and strict all-target/all-feature lint;
its receipt is recorded in the [source qualification](2026-11-03-binary-stl-exclusive-caller.md).

The finite evidence bundle `target/review/convergence/binary-stl-performance-evidence-1.zip`
contains 11,531 source-named evidence files: all four campaigns' raw stdout/stderr and
reports, the ordinary audit, six raw/annotated profiles and Linux correctness receipts.
Every member was rehashed after creation. Bundle SHA-256 is
`f134464d7ea7640840373bc6d928f6236c17a3a56fccf212e119e7f2d0b7b4b5`;
its embedded manifest SHA-256 is
`a4a40e2572a3fedc46e0e1766aff918041a86b2ff19b4f8b4ad2d6537f291e35`.
The bundle predates the final independent Linux review and does not contain that
later review or its summary; their exact hashes are recorded in the linked research.
