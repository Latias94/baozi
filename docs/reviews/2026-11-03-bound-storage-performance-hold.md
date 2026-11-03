# Bound Storage Services: Ordinary Performance Hold

Date: 2026-11-03. Independent offline audit of the four completed C,N,N,C campaigns. Control runtime `48d6f17e2f0829bdd5d50c8d7c48cea72ce343a1`; candidate `314d0fe3855cdd53159d2ffc0d6e10206d299b9f`.


Tracked export of the independent raw review, SHA-256
`7ab0c9494042d33880e6cd0548c46c982e266444eff8f09f0dd07e56c2ecd3bd`.
The [source qualification](2026-11-03-bound-storage-services.md) remains valid;
performance acceptance is separately HOLD. This supports the
[convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md)
and [bound services implementation](../plans/2026-11-03-1800-refactor-bound-storage-services-plan.md).
Local reviewer filenames below refer to
`.git/baozi-review/performance-convergence/`; original campaign directories remain
under `target/assimp-comparison/`. They identify retained local artifacts, not files
packaged with this Markdown export.

## Findings and disposition

**HOLD performance keep. The measured owner replacement does not establish an overall optimization acceptance or closure of the preceding binary-STL HOLD.** The source-qualified ownership representation can remain a structural prerequisite for continued work, with the named costs below OPEN. This is a structural continuation disposition, not a measured performance keep, universal no-regression finding or release clearance. No Windows code-generation or instruction-owner cause is established by elapsed times.

1. **P2 — retain repeated/grid extraction costs and the unresolved repeated OBJ import repair.** Repeated OBJ extract increases **686.8 -> 712.0 us (+3.67%)** and **699.0 -> 723.0 us (+3.43%)**, absolute **+25.2/+24.0 us**. Grid extract increases **1.91980 -> 1.95990 ms (+2.09%)** and **1.92805 -> 1.97985 ms (+2.69%)**, **+40.10/+51.80 us**. Both rounds' half-campaign and engine-order subgroups are positive for both extraction lanes. Repeated import's pilot **36.69245 -> 35.23810 ms (-3.96%)** does not reproduce in confirmation: **36.76525 -> 36.78875 ms (+0.06%)**. Repeated pipeline's **-1.90/-0.16%** is also much smaller on confirmation. This does not close the earlier explicit repeated-OBJ import/pipeline repair. Locations: all four `report.json#/series/{0,2,3,6}` and original retained `comparison-{001,003,004,007}-*-baozi` stdout.
2. **P2 — retain confirmation glTF pipeline/tail costs rather than approving from small median gains elsewhere.** glTF1 pipeline flips from **-1.51%** to **1.94790 -> 2.10495 ms (+8.06%, +157.05 us)**. Confirmation's early/late halves are **+6.36/+11.46%**, and both engine-order groups are positive. This is an unresolved confirmation cost, not a two-round confirmed regression; Assimp's same comparison is only **+0.88%**, so broad comparator drift does not explain it away. glTF32 pipeline's confirmation median is **-0.40%**, but p95 grows **3.856530 -> 7.033135 ms (+82.37%)**, maximum **3.8609 -> 9.7736 ms**. Keep those raw tail samples. Locations: series 26 and 11, original retained `comparison-{027,012}-*-baozi` logs plus parent command records.
3. **P2 — carry positive whole-child memory costs explicitly.** OBJ working-set medians rise in all eight OBJ lanes, including repeated import **+98/+98 KiB**. STL commit rises at import **+2/+8 KiB**, prepare **+6/+6 KiB**, extract **+50/+28 KiB**, and pipeline **+46/+44 KiB**. Repeated import/prepare commit median shifts are much larger in one round (**+24/+576 KiB**, **+546/+6 KiB**) but sit in broad, visibly bimodal distributions; they must neither be silently discarded nor treated as a deterministic new allocation. These are parent-recorded whole-child peaks, not preparation/extraction allocations. Locations: per-child `windows_peak_working_set_bytes` and `windows_peak_commit_bytes`; every lane is listed below.

Preserve the narrower positives: grid import **-1.09/-1.14%**, grid pipeline **-4.92/-2.99%**, several no-op preparation medians, and STL pipeline **-2.11/-2.44%**. These do not pay off the named costs by aggregation. The measured table supports neither a claimed large repeated-import reduction nor a claim that structural descriptor removal is sufficient for the all-endpoint target. Main-owned next work should preserve the source-qualified owner, investigate the new/existing endpoint costs with the exact retained artifacts, and qualify any next correction with unchanged ordinary tasks. No particular implementation or extra measurement is executed by this review.

## Frozen protocol and identities

Aliases: C0 = first fresh control, N0 = pilot candidate, N1 = candidate confirmation, C1 = reverse-order fresh control. Source campaign order is C0,N0,N1,C1. Each campaign launches fresh serial children for the same ordered eight cases and 31 memory lanes, three warmup pairs plus twenty retained AB/BA pairs per lane. The retained sequence is balanced ten runs in each engine order; the global alternating pair index, including warmups, was reconstructed. Version order is reversed between rounds, not randomly interleaved source versions. The engine pairing cannot remove all drift between separate source campaigns.

Timing input is the original worker stdout `stage_ns`. Parent elapsed and Windows peaks have no independent worker stdout field: they are read from the original captured per-child command records. Report aggregate statistics and embedded `worker_json` are comparison targets only, not numeric inputs. No main-derived summary, scalar decision or Linux profile is a numeric input.

| Identity | Exact value |
| --- | --- |
| Control source | `48d6f17e2f0829bdd5d50c8d7c48cea72ce343a1` |
| Candidate source | `314d0fe3855cdd53159d2ffc0d6e10206d299b9f` |
| Control worker SHA-256 | `3d7a6ac3fac4002eaee5b95f814afbac2ab3134e8af8b2f0cbff6aeeaed9a089` |
| Candidate worker SHA-256 | `0d90bae93c0bf3177a6ca16ed3385b74a20bb737ee8460d1fca8d70c05331482` |
| Control source archive SHA-256 | `169d3fda4c70ab084264c0dda986e7e20bdc78fb251b69ebe4fecb36e3aedca4` |
| Candidate source archive SHA-256 | `aabed1d2cd73f673ad01b667b31ca80543973455a2f5ae068278a6b18e376977` |
| Control build manifest SHA-256 | `1d46953f8ffbcc2ca43800fbca8c6b3e5140242bf6a30a12c6cfaf74cf1482bd` |
| Candidate build manifest SHA-256 | `bed18875fd42a1ff9c80d35e6c27675debcc21ceb15269ad51752534179e258e` |
| Assimp F32 worker SHA-256 | `d3f5753ef588395a0cf7ec111d5af293a767bbcd58c894291a1b5ea836d18a83` |
| Assimp F64 worker SHA-256 | `5db8fde3a7b23d9e804021409a470fd912e6ffcd5e6d0aeae569a0d9ec1db9a7` |
| Assimp F32 build manifest SHA-256 | `60c4c7358ebf05cf69b45c6b1f6c7b011bf8749c7827eff67ce8b73d153200f4` |
| Assimp F64 build manifest SHA-256 | `7fa97888b730bccbea7ba48ab66b3e568d92224e9827ce1da3b9a5c613ec799c` |
| Captured collector SHA-256 | `9af6fbf91bb2fcdf8d36547ccef047e40b17ff1fad291b4664b22e2186dcea87` |
| Comparison protocol SHA-256 | `82e9735649e12024bd5052eee2861453118fdf68a29c33cab18dec47b7d6c565` |
| Fourteen-case corpus manifest SHA-256 | `db52cc017949e3345a07ca51411a1ab42e47b05ee2c79aef63964e0796e73f49` |

Both Baozi builds use the same recorded Rust 1.95.0 MSVC release opt3/System allocator/dynamic CRT policy, debug 0 and no allocation instrumentation. Control worker size is 10,940,928 bytes; candidate is 10,874,880 bytes, a 66,048-byte reduction. This does not identify code-layout or memory causes. Build completion precedes the campaigns; captured manifest, current retained binary and archive hashes agree. Assimp F32/F64 binaries and manifests are unchanged across all campaigns. The independently accepted candidate association is `bound-owner-ordinary-candidate-association-review.md`, SHA-256 `3202e3e614372e3199d3033350955811a7b9586e6328d4326676794cb814e985`; this audit independently hashes the actual retained artifacts rather than relabeling control from campaign HEAD.

The captured Git commands report HEAD `314d0fe...` and a dirty checkout in every campaign; all reported dirty paths are Markdown. That is a collection-context fact, not control source identity. Control/candidate source identity comes from their retained build manifests and archives. All 29 selected consumer Rust/Cargo, root manifest/lockfile, collector and protocol archive members match between the two source archives, including `tools/consumers/assets/src/comparison.rs` SHA-256 `134933892e61ebfdcc581b60693ce4362c1e8ac6617a640e4b5ae23623ca12eb`. The child arguments match the pinned preceding eight-case recipe except worker/manifest/artifact destinations: `memory`, requested fields, precision, `missing`/`preserve` normal policy and 200,000,000 work ceiling are unchanged. The explicit generic Smooth task behind `missing` is not replaced with a cheaper format-policy task. Source archives use Git bytes; captured live manifests have their separately recorded byte identities.

The selected cases are OBJ repeated16384, OBJ grid128, glTF shared32, STL repeated1024, PLY points1024, PLY F64 neighbors, glTF shared1 and glTF shared8. This remains the same eight-case selection from the fourteen-case corpus. It must not be ranked against a previous six-case aggregate or expanded to unmeasured file-mode/other-case claims.

Campaign receipts and reports (all paths under `target/assimp-comparison/convergence-windows-bound-owner{suffix}/report.json`; command receipts under `target/review/convergence/bound-owner{suffix}-command.json`):

| Campaign/suffix | UTC command interval | Report SHA-256 | Command receipt SHA-256 |
| --- | --- | --- | --- |
| C0 / -control | 2026-11-03T11:02:13.038195+00:00 to 2026-11-03T11:03:55.446287+00:00 | `ad4a0544d1f5bb37855095ca5d901f88c89521b18f16d4d6f296d8f5a646fc7d` | `e9bbb1b4ba2edbc2dfd2da76bbe1c85c6628ccca0473cc8cb828733736815ec8` |
| N0 / empty | 2026-11-03T11:03:55.454732+00:00 to 2026-11-03T11:05:35.813423+00:00 | `003a2b55915cb15ab69f0b3b8783e0e4aa751b17099bfe47f492d79e77ce10a9` | `2b3564d9368a32a154ff045b31ac9ba52094d06ed88af4844795417b50843f6d` |
| N1 / -confirm | 2026-11-03T11:05:35.823093+00:00 to 2026-11-03T11:07:16.557887+00:00 | `dabc7c24d0c59cd0a17020ce42e68a6ded9534437dbb56fdb3b2d926f3155015` | `dd2aac6b5f79e06288931e2d77a814daab432fa47224c6e5896961fbaaa4580c` |
| C1 / -control-confirm | 2026-11-03T11:07:16.567351+00:00 to 2026-11-03T11:08:56.329608+00:00 | `829241a10815110263f352478ac9b47c8243ce03795933b9bdfae5677c99aa1b` | `642aabddec12974adc181c5e69e70deecded3b9a0f05ae4dcd3db5fc12f9372a` |

## Complete raw reconstruction

- Opened all **11,408 comparison-child stdout/stderr logs** and all **16 Git stdout/stderr logs**. Each campaign has 1,426 comparison children and two Git commands. Across four campaigns: **5,704 successful children = 4,960 retained + 744 warmups**, with return code 0, launched/success status, finite valid JSON and empty comparison stderr. No timeout, missing/extra child log or failed geometry observation was found.
- Reconstructed all 124 lane sequences and 2,852 engine pairs (2,480 retained and 372 warmup), verified each exact child argv, source/observation/output scalar widths and every sample's series membership. Child/tool timestamps are serial within and between campaigns. This verifies the captured sequence; exclusion of all unrelated host load still relies on main's serialized measurement procedure and cannot be proved by timestamps alone.
- Validated every child's analytic tuple/draw/placement counts, area, positions, normals, UV and base-pose translation with the fixed F32/F64 tolerances. The documented Assimp glTF import V inversion and repeated OBJ import's absent normals are handled at exactly those protocol boundaries. Prepared/extracted normal bounds are unchanged; the F64 neighboring positions remain distinct.
- Worker `stage_ns` from stdout agrees with the command's captured stage value. Every non-timing worker signature is stable across warmups/retained samples, rounds and both source versions, including reported `import_work`, geometry, field/normal policy, budget diagnostics and actual copied output lengths/capacities. Only reported work is proven by those fields; no unreported postprocess counter is invented.
- Import/preparation emit no copied output; extract/pipeline keep their original selected output. Per-engine placement/index overhead and capacity differ as documented by the existing protocol; those values are unchanged across versions, not forced equal between engines. The same case's output and normal policy are never substituted to obtain a gain.
- Recomputed **3,596 report summary/null-membership slots**, checking count, median, inclusive-linear IQR, minimum and maximum, plus every original paired ratio sample and matched-engine summary. Warmups are excluded from final statistics. Added raw-derived inclusive p90/p95, Tukey markers, half/order medians and all retained vectors to the companion JSON. No trimming, dropped outlier, winsorization or aggregate winner. There are **zero audit validation errors**.

Stable Baozi reported import work and selected extract/pipeline output:

| Case | Reported import work | Tuples | Placements | Logical bytes | Capacity bytes |
| --- | --- | --- | --- | --- | --- |
| OBJ repeated | 393811 | 49152 | 1 | 2359432 | 2359520 |
| OBJ grid | 4607139 | 98304 | 1 | 6291592 | 6291680 |
| glTF32 | 86468 | 96 | 64 | 14848 | 17664 |
| STL | 116328 | 3072 | 1 | 73864 | 73952 |
| PLY points | 22345 | 1024 | 1 | 24712 | 24800 |
| PLY F64 | 977 | 3 | 1 | 208 | 296 |
| glTF1 | 12243 | 3 | 2 | 464 | 552 |
| glTF8 | 27960 | 24 | 16 | 3712 | 4416 |

## Every stage lane

Medians are **milliseconds**, with positive delta denoting increased Baozi stage time. P = N0/C0 - 1; C = N1/C1 - 1. Assimp deltas are its independently reconstructed same campaign contrasts. Endpoint ratios are the **median of twenty matched per-pair Baozi/Assimp ratios**, not ratios of median times. Raw distributions remain in the JSON; signs alone are not a significance test or a causal attribution.

| Lane | C0 -> N0 ms | P delta | C1 -> N1 ms | C delta | Assimp P/C delta | Candidate P/C ratio |
| --- | --- | --- | --- | --- | --- | --- |
| 01 OBJ repeated / import | 36.692450 -> 35.238100 | -3.96% | 36.765250 -> 36.788750 | +0.06% | -1.46/+24.85% | 4.56/3.76x |
| 02 OBJ repeated / prepare | 19.220850 -> 19.251650 | +0.16% | 19.637150 -> 19.079250 | -2.84% | +4.15/+4.60% | 17.16/17.46x |
| 03 OBJ repeated / extract | 0.686800 -> 0.712000 | +3.67% | 0.699000 -> 0.723000 | +3.43% | +0.54/+3.47% | 1.75/1.79x |
| 04 OBJ repeated / pipeline | 56.949700 -> 55.866400 | -1.90% | 57.248850 -> 57.155750 | -0.16% | +0.09/+27.16% | 5.69/4.71x |
| 05 OBJ grid / import | 171.117800 -> 169.251850 | -1.09% | 171.756550 -> 169.807050 | -1.14% | +10.04/+12.21% | 6.35/6.34x |
| 06 OBJ grid / prepare | 0.156100 -> 0.146050 | -6.44% | 0.155150 -> 0.148650 | -4.19% | +9.64/+21.82% | 4.05/4.01x |
| 07 OBJ grid / extract | 1.919800 -> 1.959900 | +2.09% | 1.928050 -> 1.979850 | +2.69% | -4.34/+8.68% | 1.94/1.88x |
| 08 OBJ grid / pipeline | 176.020100 -> 167.357600 | -4.92% | 173.818950 -> 168.617900 | -2.99% | -5.27/+0.99% | 6.65/6.67x |
| 09 glTF32 / import | 3.243650 -> 3.146350 | -3.00% | 3.261200 -> 3.186900 | -2.28% | -4.14/-2.24% | 11.76/11.55x |
| 10 glTF32 / prepare | 0.180850 -> 0.180700 | -0.08% | 0.188500 -> 0.177800 | -5.68% | +5.56/-2.70% | 97.70/101.00x |
| 11 glTF32 / extract | 0.030650 -> 0.030300 | -1.14% | 0.030900 -> 0.030150 | -2.43% | -6.06/-8.71% | 2.36/2.54x |
| 12 glTF32 / pipeline | 3.573000 -> 3.537300 | -1.00% | 3.630400 -> 3.615750 | -0.40% | +6.15/+4.94% | 10.06/10.56x |
| 13 STL / import | 2.878050 -> 2.805100 | -2.53% | 2.887750 -> 2.873250 | -0.50% | -2.09/-8.26% | 19.06/20.51x |
| 14 STL / prepare | 0.154100 -> 0.142650 | -7.43% | 0.158750 -> 0.146200 | -7.91% | -5.26/-10.00% | 80.97/81.20x |
| 15 STL / extract | 0.037150 -> 0.038400 | +3.36% | 0.038800 -> 0.036050 | -7.09% | +14.11/-11.01% | 1.62/1.82x |
| 16 STL / pipeline | 3.272650 -> 3.203600 | -2.11% | 3.272000 -> 3.192200 | -2.44% | +0.24/-4.71% | 12.43/13.53x |
| 17 PLY points / import | 1.516850 -> 1.491400 | -1.68% | 1.502450 -> 1.498600 | -0.26% | +10.73/+0.37% | 1.62/2.17x |
| 18 PLY points / extract | 0.022850 -> 0.022500 | -1.53% | 0.023000 -> 0.021800 | -5.22% | +2.13/+13.04% | 4.73/4.36x |
| 19 PLY points / pipeline | 1.682600 -> 1.709350 | +1.59% | 1.704450 -> 1.671450 | -1.94% | +25.88/-4.08% | 1.77/2.25x |
| 20 PLY F64 / import | 1.073550 -> 1.057650 | -1.48% | 1.070050 -> 1.104150 | +3.19% | -1.15/+0.13% | 1.89/2.01x |
| 21 PLY F64 / prepare | 0.153150 -> 0.140550 | -8.23% | 0.153850 -> 0.144200 | -6.27% | +3.23/+12.50% | 88.71/83.08x |
| 22 PLY F64 / extract | 0.011500 -> 0.011400 | -0.87% | 0.011500 -> 0.011900 | +3.48% | +0.00/+0.00% | 10.41/11.18x |
| 23 PLY F64 / pipeline | 1.361900 -> 1.365050 | +0.23% | 1.387050 -> 1.391750 | +0.34% | +2.07/-2.76% | 2.32/2.38x |
| 24 glTF1 / import | 1.652300 -> 1.627800 | -1.48% | 1.632300 -> 1.635700 | +0.21% | +1.45/-2.76% | 7.09/7.60x |
| 25 glTF1 / prepare | 0.155200 -> 0.136350 | -12.15% | 0.148000 -> 0.142900 | -3.45% | +0.00/+0.00% | 97.82/97.66x |
| 26 glTF1 / extract | 0.012550 -> 0.012550 | +0.00% | 0.012500 -> 0.012650 | +1.20% | +4.00/+0.00% | 9.72/10.08x |
| 27 glTF1 / pipeline | 1.950900 -> 1.921350 | -1.51% | 1.947900 -> 2.104950 | +8.06% | -0.59/+0.88% | 7.19/7.46x |
| 28 glTF8 / import | 1.976850 -> 1.957400 | -0.98% | 2.006350 -> 2.078800 | +3.61% | -0.13/+2.49% | 8.41/8.82x |
| 29 glTF8 / prepare | 0.156900 -> 0.148600 | -5.29% | 0.158900 -> 0.154200 | -2.96% | +6.67/+3.12% | 93.62/93.56x |
| 30 glTF8 / extract | 0.015900 -> 0.016400 | +3.14% | 0.016400 -> 0.016550 | +0.91% | +5.26/-3.66% | 4.29/4.23x |
| 31 glTF8 / pipeline | 2.309000 -> 2.301350 | -0.33% | 2.336500 -> 2.389150 | +2.25% | +4.36/-1.34% | 7.73/8.24x |

Narrow dispositions for the full table:

- **Repeated OBJ:** import repair is unconfirmed; pipeline has a small confirmation decrease, not the earlier pilot-sized reduction. Real normal preparation is **+0.16/-2.84%**; its unchanged kernel cannot be credited with a proven speedup. Extraction is a repeated cost with higher p95 in both rounds.
- **Grid:** import and independently timed pipeline decrease twice, and their half/order subgroups are negative. Import saves **1.86595/1.94950 ms**; pipeline saves **8.66250/5.20105 ms**. These are bounded positives; independently timed stages do not add up to the pipeline measurement. Extraction and its p95 worsen twice. No-op preparation medians improve, but confirmation p95 worsens.
- **glTF32:** import **-3.00/-2.28%** is narrower than a causal improvement claim: its early-half contrasts are **+0.31/+0.04%**, while late halves are negative, and Assimp changes **-4.14/-2.24%**. Tiny extract and pipeline medians fall twice; pipeline's confirmation tail is a new open cost. Preparation is essentially flat in pilot and lower in confirmation, with a pilot maximum of 321.3 us.
- **STL:** import **-2.53/-0.50%** and pipeline **-2.11/-2.44%** are modest bounded improvements. Its parser algorithm has not changed in this slice. Extract flips **+3.36/-7.09%**; no repeatable extract gain is established. Preparation median falls twice but its pilot p95 is positive. Commit costs remain despite working-set reductions.
- **PLY points:** import/extract medians decrease twice, but extraction p95 and parent elapsed increase twice. Pipeline flips **+1.59/-1.94%**; preserve the pilot cost and both parent-wall increases.
- **PLY F64:** prepare **-8.23/-6.27%** is a bounded positive. Import flips **-1.48/+3.19%**, extract **-0.87/+3.48%**; both are open confirmation costs, with F64 extract p95 higher twice. Pipeline's repeated **+0.23/+0.34%** is only **+3.15/+4.70 us** against overlapping distributions and mixed subgroups: retain the measurements without upgrading them to a demonstrated material causal regression.
- **glTF1:** import flips **-1.48/+0.21%**. Preparation medians fall **-12.15/-3.45%**, but confirmation p95 rises. Extract is **unchanged/+1.20%**, including a confirmation order split **+8.10/-0.79%** and 16.80% candidate IQR/median; this does not repair earlier small-endpoint costs. Pipeline's confirmation **+8.06%** is open, as detailed above.
- **glTF8:** import flips **-0.98/+3.61%**, pipeline **-0.33/+2.25%**. Extract increases **+3.14/+0.91%**, only **+0.50/+0.15 us**; the second difference spans just 1.5 of the observed 100-ns increments. Retain this cost and uncertainty. Preparation medians decrease twice; this does not clear the earlier source contrast or all pipeline/tail repairs.

## Controls, order and drift

Balanced engine order and reverse source order are necessary checks, not proof of host stationarity. Main recorded only selected environment controls; Windows load averages are unavailable. The parent clock reports QueryPerformanceCounter resolution 100 ns; every retained worker-stage sample in these campaigns is a multiple of 100 ns. That observed quantization is not a calibrated worker timer accuracy claim, especially for 1.4–1.8 us Assimp preparation and sub-microsecond median differences.

Assimp is unchanged yet shifts **+24.85%** for repeated import and **+27.16%** for repeated pipeline in confirmation's candidate/control campaign contrast. Its candidate-campaign N1/N0 medians shift **+27.24/+22.37%**, while Baozi's corresponding N1/N0 shifts are **+4.40/+2.31%**. Repeated import's Assimp medians are **7.86785, 7.75335, 9.86525, 7.90185 ms** in C0,N0,N1,C1. Dividing by the anomalous comparator contrast would manufacture a larger apparent version gain; it does not demonstrate a repaired public import endpoint.

Grid import's Assimp contrast is **+10.04/+12.21%**, so its paired Baozi/Assimp ratio reduction cannot all be credited to Baozi. Grid pipeline Assimp shifts **-5.27/+0.99%**: the pilot pipeline benefit nearly tracks that comparator movement, while the confirmation supports a narrower Baozi decrease. PLY points import's Assimp changes **+10.73/+0.37%**, pipeline **+25.88/-4.08%**. Such lane-specific drift precludes a universal control correction or a normalized scalar winner. All comparator raw values are retained.

The following contrasts are candidate-versus-control within the same half/order subgroup; each subgroup has ten retained values per engine. A range crossing zero is not hidden by the overall median:

| Lane | Pilot early/late | Confirm early/late | Pilot first/second engine order | Confirm first/second engine order |
| --- | --- | --- | --- | --- |
| 01 OBJ repeated / import | -3.59/-4.33% | -0.45/+0.57% | -3.65/-4.48% | +0.31/+1.34% |
| 03 OBJ repeated / extract | +5.00/+3.19% | +1.73/+7.36% | +2.39/+5.09% | +3.63/+2.90% |
| 04 OBJ repeated / pipeline | -1.43/-2.83% | -0.58/+0.18% | -2.05/-2.43% | +0.60/-0.82% |
| 05 OBJ grid / import | -1.91/-0.61% | -1.03/-1.24% | -1.19/-0.64% | -1.09/-1.52% |
| 07 OBJ grid / extract | +1.57/+2.75% | +1.18/+5.53% | +2.90/+2.18% | +4.41/+1.87% |
| 08 OBJ grid / pipeline | -4.61/-4.81% | -1.78/-3.39% | -4.95/-4.55% | -2.70/-2.98% |
| 09 glTF32 / import | +0.31/-4.15% | +0.04/-3.99% | -3.49/-1.09% | -1.57/-3.70% |
| 20 PLY F64 / import | -0.23/-2.22% | +3.63/+2.56% | -2.11/-2.14% | +3.87/+2.31% |
| 22 PLY F64 / extract | -1.74/+0.43% | +3.83/+2.63% | +0.00/-1.73% | +4.31/+2.17% |
| 26 glTF1 / extract | +0.39/+1.23% | +2.38/+2.02% | +2.42/-2.73% | +8.10/-0.79% |
| 27 glTF1 / pipeline | -3.77/-0.87% | +6.36/+11.46% | -0.56/-1.94% | +10.92/+4.71% |
| 28 glTF8 / import | -1.02/-0.62% | +8.37/+1.93% | -2.25/+1.06% | +5.12/+1.73% |
| 30 glTF8 / extract | +3.10/+3.80% | +3.05/+0.00% | +2.50/+6.37% | +0.00/+2.73% |
| 31 glTF8 / pipeline | -1.93/+1.56% | +2.81/+0.42% | -1.48/+1.99% | +0.37/+3.18% |

Candidate IQR/median is **2.03/2.79%** for repeated import, **4.25/8.27%** for repeated extract, **0.98/1.45%** for grid import, **1.85/3.81%** for grid extract and **0.61/1.64%** for grid pipeline. glTF1 pipeline's confirmation is **8.50%** versus control **3.54%**, with candidate N1/N0 median drift **+9.56%** versus control C1/C0 **-0.15%**. Its confirmation cost is present in both halves and orders despite the non-reproducing pilot. Work/geometry/output signatures are unchanged, so these differences are timing costs rather than detectable task substitutions; their source/code-generation cause remains unproven.

## Tails, outliers and parent wall

The table reports Baozi **p95 and maximum in milliseconds**, preserving all samples. P95 is inclusive linear interpolation on twenty observations and is descriptive, not a population tail guarantee. Companion JSON records p90, all outlier values, min/max and parent distributions for both engines. Tukey-marked stage samples retained (Baozi/Assimp) are C0 **30/34**, N0 **33/41**, N1 **36/42**, C1 **28/35**; none is removed.

| Lane | Pilot p95 ms | Confirm p95 ms | Pilot max ms | Confirm max ms | Parent median P/C delta |
| --- | --- | --- | --- | --- | --- |
| 01 OBJ repeated / import | 38.008560 -> 36.564270 | 37.610185 -> 37.861680 | 38.397300 -> 37.061500 | 38.003200 -> 38.153900 | -1.80/+3.83% |
| 02 OBJ repeated / prepare | 19.677805 -> 20.257920 | 22.008615 -> 19.645575 | 19.826100 -> 20.511000 | 22.062100 -> 19.987100 | +3.09/-2.69% |
| 03 OBJ repeated / extract | 0.720510 -> 0.816670 | 0.826555 -> 0.893935 | 0.743500 -> 0.863600 | 0.989100 -> 1.316400 | +4.60/-4.16% |
| 04 OBJ repeated / pipeline | 58.456180 -> 57.666145 | 58.241845 -> 57.857020 | 58.832000 -> 58.294000 | 58.275000 -> 57.859300 | +2.23/-2.72% |
| 05 OBJ grid / import | 180.279435 -> 171.171055 | 173.300795 -> 172.845810 | 181.195900 -> 173.569900 | 175.749800 -> 177.483900 | +2.06/-1.84% |
| 06 OBJ grid / prepare | 0.166035 -> 0.156885 | 0.181605 -> 0.200980 | 0.168600 -> 0.164200 | 0.193100 -> 0.210100 | -2.62/+2.52% |
| 07 OBJ grid / extract | 1.970180 -> 2.048370 | 2.094510 -> 2.155020 | 2.072400 -> 2.131400 | 2.108000 -> 2.199100 | -2.89/+1.43% |
| 08 OBJ grid / pipeline | 180.391835 -> 168.922220 | 176.395635 -> 171.700480 | 182.932800 -> 169.532500 | 177.838400 -> 172.028800 | -5.24/-0.37% |
| 09 glTF32 / import | 3.453190 -> 3.429090 | 3.423380 -> 3.590115 | 3.494800 -> 3.493500 | 3.428700 -> 3.605600 | -7.98/-0.29% |
| 10 glTF32 / prepare | 0.201690 -> 0.209295 | 0.207815 -> 0.202270 | 0.207200 -> 0.321300 | 0.217600 -> 0.220700 | -11.38/-2.90% |
| 11 glTF32 / extract | 0.033160 -> 0.033720 | 0.033920 -> 0.033625 | 0.034300 -> 0.034100 | 0.034300 -> 0.037900 | -6.00/+1.07% |
| 12 glTF32 / pipeline | 3.715825 -> 3.778090 | 3.856530 -> 7.033135 | 3.860700 -> 3.874800 | 3.860900 -> 9.773600 | -2.07/+2.49% |
| 13 STL / import | 3.003560 -> 2.928105 | 3.083170 -> 3.099945 | 3.010400 -> 3.023200 | 3.107300 -> 3.104600 | -1.79/+2.25% |
| 14 STL / prepare | 0.164555 -> 0.167180 | 0.190840 -> 0.158350 | 0.169400 -> 0.170600 | 0.212500 -> 0.176400 | -5.98/-3.62% |
| 15 STL / extract | 0.041635 -> 0.040545 | 0.044675 -> 0.040670 | 0.053700 -> 0.041400 | 0.059400 -> 0.051500 | -8.16/+2.89% |
| 16 STL / pipeline | 3.421085 -> 3.371245 | 3.491455 -> 3.396970 | 3.439800 -> 3.383500 | 4.130900 -> 3.603500 | -5.69/+4.28% |
| 17 PLY points / import | 1.603950 -> 1.565085 | 1.702640 -> 1.616575 | 1.618200 -> 1.580000 | 1.771800 -> 1.621800 | +5.46/+5.22% |
| 18 PLY points / extract | 0.025275 -> 0.025715 | 0.024765 -> 0.025975 | 0.026700 -> 0.026000 | 0.026000 -> 0.027400 | +8.14/+4.48% |
| 19 PLY points / pipeline | 1.761100 -> 1.760210 | 1.801840 -> 1.741750 | 1.782000 -> 1.796500 | 1.823500 -> 1.752200 | +10.23/+2.98% |
| 20 PLY F64 / import | 1.139290 -> 1.131895 | 1.117830 -> 1.293890 | 1.220800 -> 1.205900 | 1.118400 -> 1.702200 | -0.38/+4.19% |
| 21 PLY F64 / prepare | 0.185335 -> 0.154015 | 0.166200 -> 0.157040 | 0.195500 -> 0.154300 | 0.170000 -> 0.157800 | -8.53/-3.55% |
| 22 PLY F64 / extract | 0.012780 -> 0.013425 | 0.012750 -> 0.016290 | 0.020000 -> 0.015800 | 0.013700 -> 0.019900 | -6.90/-2.37% |
| 23 PLY F64 / pipeline | 1.468465 -> 1.426160 | 1.451830 -> 1.470160 | 1.522900 -> 1.446300 | 1.463800 -> 1.515000 | +4.10/+1.74% |
| 24 glTF1 / import | 1.733040 -> 1.688775 | 1.780130 -> 1.762575 | 1.771800 -> 1.711100 | 1.822500 -> 1.952100 | +1.62/-1.49% |
| 25 glTF1 / prepare | 0.181135 -> 0.149370 | 0.161755 -> 0.173490 | 0.181800 -> 0.154500 | 0.170400 -> 0.180900 | +1.12/-2.70% |
| 26 glTF1 / extract | 0.015820 -> 0.014180 | 0.015165 -> 0.015955 | 0.016200 -> 0.015700 | 0.016400 -> 0.020800 | +0.53/-3.18% |
| 27 glTF1 / pipeline | 2.111240 -> 2.063005 | 2.060720 -> 2.384880 | 2.353300 -> 2.074500 | 2.061100 -> 2.449100 | +0.54/-1.85% |
| 28 glTF8 / import | 2.055625 -> 2.068250 | 2.064065 -> 2.324585 | 2.156800 -> 2.145200 | 2.071000 -> 2.337600 | +0.55/+0.11% |
| 29 glTF8 / prepare | 0.174135 -> 0.167405 | 0.187305 -> 0.165420 | 0.176700 -> 0.178900 | 0.196900 -> 0.169600 | -2.49/-2.76% |
| 30 glTF8 / extract | 0.017710 -> 0.020330 | 0.020705 -> 0.018740 | 0.017900 -> 0.020900 | 0.020800 -> 0.023300 | +9.22/-1.57% |
| 31 glTF8 / pipeline | 2.452230 -> 2.399530 | 2.421555 -> 2.500525 | 2.462300 -> 2.468500 | 2.434000 -> 2.559900 | +5.64/-2.57% |

In addition to the headline extraction and glTF1 pipeline costs, glTF32 pipeline's N1 outliers are **9.7736, 5.5727 and 6.8889 ms**. Its whole-child elapsed p95 rises **82.41574 -> 127.55925 ms (+54.78%)**, maximum **84.2348 -> 127.5849 ms**. Grid extraction confirmation parent p95 rises **249.98956 -> 299.39714 ms (+19.76%)**, even though most other parent medians fluctuate only a few percent. PLY points pipeline parent median is **+10.23/+2.98%**, parent p95 **+29.99/+12.14%**; its extraction parent median is **+8.14/+4.48%**, despite lower worker-stage medians. glTF8 extraction pilot parent p95 increases **73.085255 -> 95.878345 ms (+31.19%)**. These costs include setup, import/preparation outside the selected clock, output validation/serialization, process startup and scheduling. They cannot be equated to stage CPU, explained by Linux Ir, or erased because a selected stage median falls.

## Every lane's Windows whole-child memory

The two Windows fields are **lifetime PeakWorkingSetSize and PeakPagefileUsage**, queried by the collector after child wait using a retained query handle. They include all child setup, stages, checking and serialization; they are not stage peaks, allocator live bytes, Linux RSS or owned-resource counters. `peak_rss_bytes` is null with the explicit Windows-unavailable reason throughout. No heap allocation instrumentation is enabled. Do not claim an RSS or heap improvement from these fields.

All values below are **KiB**. Each C0->N0 or C1->N1 pair is a median over twenty children. No zero or small positive is silently classified as a reduction:

| Lane | Pilot working set KiB | Confirm working set KiB | Pilot commit KiB | Confirm commit KiB |
| --- | --- | --- | --- | --- |
| 01 OBJ repeated / import | 17350 -> 17448 (+98) | 17358 -> 17456 (+98) | 13194 -> 13218 (+24) | 13198 -> 13774 (+576) |
| 02 OBJ repeated / prepare | 17342 -> 17444 (+102) | 17364 -> 17444 (+80) | 13204 -> 13750 (+546) | 13200 -> 13206 (+6) |
| 03 OBJ repeated / extract | 17354 -> 17450 (+96) | 17354 -> 17448 (+94) | 13482 -> 13194 (-288) | 13744 -> 13204 (-540) |
| 04 OBJ repeated / pipeline | 17358 -> 17452 (+94) | 17358 -> 17454 (+96) | 13204 -> 13194 (-10) | 13196 -> 13778 (+582) |
| 05 OBJ grid / import | 32964 -> 33080 (+116) | 32978 -> 33068 (+90) | 30238 -> 30240 (+2) | 30238 -> 30240 (+2) |
| 06 OBJ grid / prepare | 32998 -> 33058 (+60) | 32978 -> 33054 (+76) | 30240 -> 30234 (-6) | 30238 -> 30236 (-2) |
| 07 OBJ grid / extract | 32980 -> 33062 (+82) | 32988 -> 33062 (+74) | 30236 -> 30236 (+0) | 30236 -> 30238 (+2) |
| 08 OBJ grid / pipeline | 32990 -> 33080 (+90) | 32996 -> 33054 (+58) | 30240 -> 30236 (-4) | 30236 -> 30236 (+0) |
| 09 glTF32 / import | 10324 -> 10276 (-48) | 10312 -> 10278 (-34) | 1304 -> 1304 (+0) | 1320 -> 1294 (-26) |
| 10 glTF32 / prepare | 10772 -> 10692 (-80) | 10772 -> 10692 (-80) | 1308 -> 1298 (-10) | 1308 -> 1308 (+0) |
| 11 glTF32 / extract | 10780 -> 10700 (-80) | 10780 -> 10700 (-80) | 1308 -> 1298 (-10) | 1308 -> 1304 (-4) |
| 12 glTF32 / pipeline | 10780 -> 10700 (-80) | 10780 -> 10700 (-80) | 1308 -> 1298 (-10) | 1310 -> 1300 (-10) |
| 13 STL / import | 9110 -> 9088 (-22) | 9108 -> 9084 (-24) | 1326 -> 1328 (+2) | 1324 -> 1332 (+8) |
| 14 STL / prepare | 9572 -> 9512 (-60) | 9572 -> 9512 (-60) | 1326 -> 1332 (+6) | 1326 -> 1332 (+6) |
| 15 STL / extract | 9580 -> 9520 (-60) | 9578 -> 9524 (-54) | 1332 -> 1382 (+50) | 1350 -> 1378 (+28) |
| 16 STL / pipeline | 9580 -> 9520 (-60) | 9580 -> 9520 (-60) | 1332 -> 1378 (+46) | 1332 -> 1376 (+44) |
| 17 PLY points / import | 9194 -> 9208 (+14) | 9194 -> 9216 (+22) | 1138 -> 1138 (+0) | 1146 -> 1128 (-18) |
| 18 PLY points / extract | 9202 -> 9226 (+24) | 9200 -> 9224 (+24) | 1150 -> 1120 (-30) | 1156 -> 1130 (-26) |
| 19 PLY points / pipeline | 9200 -> 9224 (+24) | 9200 -> 9224 (+24) | 1156 -> 1136 (-20) | 1152 -> 1140 (-12) |
| 20 PLY F64 / import | 9200 -> 9222 (+22) | 9198 -> 9220 (+22) | 1080 -> 1078 (-2) | 1078 -> 1080 (+2) |
| 21 PLY F64 / prepare | 9660 -> 9644 (-16) | 9656 -> 9644 (-12) | 1092 -> 1084 (-8) | 1084 -> 1080 (-4) |
| 22 PLY F64 / extract | 9664 -> 9652 (-12) | 9666 -> 9656 (-10) | 1084 -> 1080 (-4) | 1084 -> 1084 (+0) |
| 23 PLY F64 / pipeline | 9664 -> 9652 (-12) | 9664 -> 9656 (-8) | 1080 -> 1082 (+2) | 1084 -> 1084 (+0) |
| 24 glTF1 / import | 10258 -> 10220 (-38) | 10264 -> 10218 (-46) | 1112 -> 1120 (+8) | 1120 -> 1116 (-4) |
| 25 glTF1 / prepare | 10708 -> 10630 (-78) | 10708 -> 10636 (-72) | 1114 -> 1096 (-18) | 1118 -> 1102 (-16) |
| 26 glTF1 / extract | 10716 -> 10640 (-76) | 10716 -> 10640 (-76) | 1116 -> 1100 (-16) | 1116 -> 1100 (-16) |
| 27 glTF1 / pipeline | 10716 -> 10640 (-76) | 10716 -> 10636 (-80) | 1112 -> 1100 (-12) | 1120 -> 1100 (-20) |
| 28 glTF8 / import | 10256 -> 10214 (-42) | 10252 -> 10216 (-36) | 1164 -> 1160 (-4) | 1156 -> 1164 (+8) |
| 29 glTF8 / prepare | 10700 -> 10636 (-64) | 10704 -> 10636 (-68) | 1164 -> 1126 (-38) | 1162 -> 1132 (-30) |
| 30 glTF8 / extract | 10710 -> 10648 (-62) | 10712 -> 10644 (-68) | 1164 -> 1166 (+2) | 1160 -> 1158 (-2) |
| 31 glTF8 / pipeline | 10708 -> 10644 (-64) | 10708 -> 10644 (-64) | 1164 -> 1164 (+0) | 1160 -> 1164 (+4) |

Twelve lanes have positive working-set median deltas twice: the eight OBJ lanes, three PLY points lanes and F64 import. glTF and STL working sets generally fall **22–80 KiB**, while their commit results must be assessed separately. Seven lanes have positive commit deltas twice: repeated import/prepare, grid import and all four STL lanes. Grid import's **+2/+2 KiB** and STL prepare's **+6/+6 KiB** are preserved with their granularity and dispersion, not promoted to a material-allocation diagnosis. STL extract's **+50/+28 KiB (+3.75/+2.07%)** and pipeline **+46/+44 KiB (+3.45/+3.30%)** remain visible costs even with lower working sets.

Repeated OBJ commit distributions have two occupied bands near **13,150–13,225 KiB** and **13,740–13,840 KiB** in both versions. For example repeated import control/candidate maxima are **13,820/13,824 KiB** in pilot and **13,812/13,832 KiB** in confirmation, while the corresponding median contrast changes **+24 to +576 KiB**. Candidate import IQR is **571/615 KiB**; preparation's **+546/+6 KiB** switches its large shift to the other round. The medians and maxima are retained; this supports uncertainty from the observed distribution, not an invented 576-KiB allocation or automatic dismissal of its positive median.

Candidate maximum median paired memory ratios are working set **1.68578x / 1.68421x** and commit **1.32143x / 1.31755x**. No selected memory lane exceeds 3x under either Windows method. This narrow pass does not qualify other cases, file mode, heap bounds, Linux memory, or all resource contracts. glTF8's earlier **+58/+62 KiB** extract and **+58/+60 KiB** pipeline source-contrast costs remain historical facts; current near-zero commit changes are not retroactive closure.

## Release gates and prior-cost continuity

| Campaign | Stage lanes >3x | Stage lanes >10x | Pipelines >3x | Maximum median paired stage ratio |
| --- | --- | --- | --- | --- |
| C0 | 23 | 12 | 6/8 | 107.78333x |
| N0 | 23 | 11 | 6/8 | 97.82143x |
| N1 | 23 | 12 | 6/8 | 101.00000x |
| C1 | 23 | 11 | 6/8 | 100.44669x |

Candidate still has **23/31 lanes and 6/8 pipelines above 3x** in both rounds, with maxima **97.82143x / 101.0x**. F64 extraction is **10.41/11.18x**, glTF1 extraction **9.72/10.08x**, glTF32 pipeline **10.06/10.56x**, and STL pipeline **12.43/13.53x**. A threshold crossed in only one campaign is not a stable repair. glTF32 import and the large preparation gaps remain above 10x. Most no-op preparation candidate medians are **136.35–180.70 us** in pilot and **142.90–177.80 us** in confirmation, versus roughly **1.4–1.8 us** for the small Assimp paths. Grid no-op preparation is an exception at about **36–37 us** in Assimp. Real repeated-OBJ normal preparation is **19.25165/19.07925 ms** versus **1.11235/1.09035 ms**. Descriptor binding does not close these architectural/algorithmic gaps. A roughly 101x current endpoint would still need about a further 33.7-fold reduction to reach 3x at the same comparator time.

The continuity register stays OPEN without recomputing or combining old campaigns into this experiment:

- **R4:** original F64 extraction **+34.80/+28.97%** remains in `bound-reader-measurement-review.md`. This pair's **-0.87/+3.48%** and >10x endpoint do not establish repair.
- **R5:** grid extraction confirmation **+105.10 us / +5.36%**, with its original controls/drift qualifiers, remains in `normal-actions-measurement-review.md`. Grid extraction is positive again here; no repair is established.
- **R7:** glTF1/8 extract **+10.85/+9.63%**, **+6.85/+5.74%**, glTF32 confirmation **+5.12%**, F64 **+.70/+.85 us**, and the small STL/glTF1 commit costs remain in `gltf-profile-measurement-review.md`. Current glTF32 decreases and other mixed changes preserve a later contrast, not deletion of those facts or proof of their cause.
- **R8 cardinality:** its six extraction costs, glTF8 preparation, glTF1 commit and pipeline/parent tails, with exact bounded intermediate dispositions, remain in `profile-cardinality-measurement-review.md`. All reported work signatures in the present source pair are equal; the earlier vocabulary change is not replayed or relabeled. Current preparation decreases do not silently close R8's register.
- **Binary STL held costs:** `binary-stl-ordinary-measurement-review.md` retains repeated/grid import **+7.66/+7.59%**, **+5.13/+5.08%**, and their pipeline/commit/tail costs. Current grid improvements are recognized as bounded progress against the current control, not proof that the earlier loss has been fully repaired. Repeated import does not improve on confirmation. All old extraction positives and negatives remain; no optimized-STL-parser, aggregate-winner or no-regression claim follows.

Recommendation: keep performance acceptance **HOLD**, preserve this exact pair as the next measurement baseline, and carry all above costs while main proceeds with required ownership/lifecycle work. Any later Linux profile belongs to its own exact-source lane and may locate executed Linux work only. It cannot explain Windows timing causes, convert whole-worker Ir into stage CPU or clear these native regressions. The full Windows/Linux fourteen-case/110-lane all-endpoint release gate remains open.

## Audit artifact closure

Only new reviewer artifacts were written. No original report/log/collector, budget, task, production source or historical review was changed. No Cargo, rustc, formatter, test, comparison worker, profiler, new measurement, WSL or outside agent was run. Python only read retained offline artifacts and wrote these reviewer outputs. The program is a bounded adaptation of the prior raw reconstructor, not a new benchmark or source-analysis framework.

Companion summary SHA-256: `ccfee28a11fdfca2d581250581b265594422977a20b1e1d3dcec15826c9f27e7`.

Offline audit program SHA-256: `0d6fe2eb34f5ec251f754488a2892d8ddf26ae39aca29d47b06cd92d4af92f6e` (also embedded in the summary).

The JSON includes all retained metric vectors, warmup stage vectors, rebuilt statistics/ratios, half/order/drift comparisons, original Git outputs and every raw log hash. The identity-set hashes below encode sorted UTF-8 `log filename + NUL + SHA-256 + LF`; these are NUL-encoding hashes, not JSON hashes. Each campaign's set includes 2,852 child and four Git logs. Separate collector log hashes are also pinned.

| Campaign | All 2,856 raw stdout/stderr identity-set SHA-256 | Collector log SHA-256 |
| --- | --- | --- |
| C0 | `1b5be9d3b12b842d46e4cf85d0afa76ae6d98450234947c39ecf0bdcca1bd569` | `572c47250f0475be4d156870f0c9ab0d776985fcacd83fb0e35ffd981a987d4b` |
| N0 | `c1b317283971e5beafc2d998a8784246f06ff0e2be09c39fd0a233fcd28f5914` | `f12cf69c7765cbf7280fce044d9dfc1c1a645e10641029cef02e85ff4837cb24` |
| N1 | `85e9d29c3df76dac8b89445e4e9582c6f44a56549e37057c2cc491a48cbbc48c` | `7487501c59771ca1fabeb124bac0e1964e99af17a3d54c065717a989c0d1f93c` |
| C1 | `37156810a59d71b9193b9028e758eddb1ff87c88c19b166052e4d411aa6bcc90` | `c7ff1c51b95d72ebf4f62ac8f1a564f2e138170855addc175b8b68e42d22c3d0` |

All four final reports were completed with command receipt return code 0 before this audit. Build and source identity checks are independent from the measurement verdict. No Linux attribution result was read as a numerical input, and no historical performance disposition is automatically cleared.

## Finite Retained Evidence

`target/review/convergence/bound-owner-performance-evidence-1.zip` retains 11,538
named evidence files: all four campaigns, the frozen independent raw audit, six exact
Linux raw/annotated profiles and their build receipts, and affected Linux qualification.
Every member was rehashed after creation. ZIP SHA-256 is
`b33425005cd124d1cce0aa1db9524bcb0108b0cde0dae6529447df1f245d2469`;
the embedded manifest SHA-256 is
`47bacbb17a2ea8a56689fa938192aeb06897d402f3b15994428767ecfa2d0961`.
The original ordinary executables/source archives remain identified by immutable build
receipts rather than duplicated in this bundle. The later independent Linux call-owner
review and subsequent root design were not final at this boundary and are excluded.

## Subsequent Matched Linux Instructions

The [independent instruction audit](../research/2026-11-03-bound-storage-instruction-costs.md)
reconciles all six raw profiles, every exclusive flat row and both directions of
every annotated caller edge. Control is the exact `48d6f17` symbol worker, candidate
the exact `314d0fe` symbol worker. Whole-worker Ir is:

| Pipeline | Control Ir | Candidate Ir | Change |
| --- | ---: | ---: | ---: |
| Repeated OBJ | 1,011,204,654 | 997,737,279 | -1.3318% |
| Grid OBJ | 3,232,140,919 | 3,190,335,809 | -1.2934% |
| STL | 34,224,811 | 34,226,152 | +0.0039% |

Independent storage-construction self cost falls from 90 to 23 Ir per captured
entry. Real poll, reservation/resize kernel and the inspected numeric-work counts
remain unchanged. Old Control-to-Services symbols are matched as renamed owners,
not counted as eliminated work. Current canonical action and grid parser-completion
owners remain substantial and separately identified in the linked research.
These are bounded Linux executed-instruction facts including setup/checking/teardown;
they are neither endpoint CPU shares, predicted savings nor Windows timing causes.
The Windows performance HOLD and every named repair remain unchanged.
