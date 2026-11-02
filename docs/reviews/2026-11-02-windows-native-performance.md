# Windows Native Baozi/Assimp Performance

Completed 2026-11-02T08:48:19.232807+00:00: **110/110 lanes**, 2,200 retained successful pairs (4,400 children), plus 660 warmup children. No failed comparison or discarded outlier.

Baozi source: `4455d03db190c6bef05ff5a1de024d2aac461465`. Assimp 6.0.5: `392a658f9c271be965271f45e7521a1b80ea4392`. Rust 1.95.0, MSVC 19.51.36256, CMake Release; optimized ordinary x64 binaries without allocator interception. Three warmups and twenty retained fresh-process pairs per lane, serial AB/BA ordering. This is a Windows series, separate from Linux B0/B1.

## Complete Pipeline, Memory Input

| Case | Baozi median (ms) | Assimp median (ms) | Baozi / Assimp time | Baozi / Assimp working set (MiB) | Baozi / Assimp peak commit (MiB) |
| --- | ---: | ---: | ---: | ---: | ---: |
| obj-repeated-1 | 1.6179 | 4.1950 | 0.39 | 10.11 / 21.38 | 1.05 / 16.83 |
| obj-repeated-1024 | 18.2992 | 4.4149 | 4.14 | 10.21 / 21.63 | 1.77 / 17.08 |
| obj-repeated-16384 | 296.9121 | 9.6916 | 30.64 | 17.54 / 25.26 | 13.46 / 20.86 |
| obj-grid-2 | 1.4060 | 4.0331 | 0.35 | 9.20 / 21.38 | 1.06 / 16.84 |
| obj-grid-32 | 25.5725 | 5.5465 | 4.61 | 9.41 / 22.30 | 3.40 / 17.78 |
| obj-grid-128 | 393.7846 | 25.1408 | 15.66 | 37.88 / 35.04 | 39.33 / 30.99 |
| gltf-shared-1 | 2.1971 | 0.2527 | 8.69 | 9.88 / 6.17 | 1.08 / 0.93 |
| gltf-shared-8 | 3.3275 | 0.2675 | 12.44 | 9.88 / 6.18 | 1.09 / 0.92 |
| gltf-shared-32 | 7.9782 | 0.3258 | 24.48 | 9.94 / 6.22 | 1.27 / 0.99 |
| stl-repeated-1 | 1.2611 | 0.1106 | 11.40 | 8.99 / 5.94 | 1.08 / 0.91 |
| stl-repeated-1024 | 6.6153 | 0.2238 | 29.56 | 8.98 / 5.92 | 1.33 / 1.05 |
| ply-triangle-1 | 1.3166 | 0.6070 | 2.17 | 9.15 / 7.33 | 1.09 / 2.80 |
| ply-f64-neighbors | 1.3034 | 0.5666 | 2.30 | 9.14 / 7.29 | 1.08 / 2.80 |
| ply-points-1024 | 3.2035 | 0.7299 | 4.39 | 8.69 / 7.38 | 1.12 / 2.86 |

## Large OBJ Endpoints

| Case | Endpoint | Baozi median / IQR (ms) | Assimp median / IQR (ms) | Baozi / Assimp time |
| --- | --- | ---: | ---: | ---: |
| obj-repeated-16384 | import | 100.2815 / 0.7165 | 7.6322 / 0.3520 | 13.14 |
| obj-repeated-16384 | prepare | 190.1163 / 1.7790 | 1.0507 / 0.0440 | 180.95 |
| obj-repeated-16384 | extract | 4.7679 / 0.3002 | 0.4007 / 0.0376 | 11.90 |
| obj-repeated-16384 | pipeline | 296.9121 / 1.7565 | 9.6916 / 0.2550 | 30.64 |
| obj-grid-128 | import | 379.1617 / 2.6507 | 23.5635 / 0.5584 | 16.09 |
| obj-grid-128 | prepare | 0.1389 / 0.0055 | 0.0327 / 0.0066 | 4.25 |
| obj-grid-128 | extract | 15.1120 / 0.3745 | 0.9930 / 0.0938 | 15.22 |
| obj-grid-128 | pipeline | 393.7846 / 2.1324 | 25.1408 / 0.5102 | 15.66 |

## Interpretation and Limits

Large OBJ throughput remains far behind Assimp. The endpoints separate import, normal preparation and extraction; the observed gap is not established as a file-streaming issue. Investigation must attribute text preparation, format parsing, accounting, validation/canonical storage and preparation separately before choosing a breaking redesign. Small timings have substantial fixed initialization contributions and do not establish a general winner.

Pipeline includes IO/importer/request setup, import, explicit preparation, selected detached extraction and native-state destruction. Validation and JSON serialization occur outside its stage clock. These are actual native-scene contracts, not parser-only measurements: Baozi also retains its source/evidence/validation and resource-control obligations. Those costs must be located, not silently disabled to match a smaller task.

Working set and peak commit are whole-child lifetime metrics, including setup/checking/serialization. They are not stage peaks or memory attributable only to the parser. Windows `peak_rss_bytes` remains null. Linux RSS and Windows metrics are separate series. Lower memory on a repeated-input case does not establish universal memory superiority.

The collector checks generated analytic expectations for every sample. Separate precision/ownership/tuple tests passed 10/10 for F32 and 7/7 for F64 before collection. No model, optional postprocess preset, requested field, engine order or repetition count changed between engines.

## Reproducible Artifacts

- Complete raw samples, timestamps, parent wall time, stage time, median/IQR/min/max, per-pair ratios, source/corpus/build/binary digests: `target/assimp-comparison/Windows-native/report.json`.
- Report SHA-256: `9fea43624c49c8d97450327866c52cc17f641b9201bc23c88da41a14ab9f6ca6`.
- Exact worker commands and protocol snapshot: same report directory. The report records Windows 11, CPU and Python details. Additional CPU description: `target/assimp-comparison/Windows-builds/cpu.json`.
- Baozi optimized binary/build/source archive: `target/assimp-comparison/Windows-baozi-build/`.
- Assimp F32/F64 immutable binaries, MSVC configuration, CMake cache, link dependencies and source receipt: `target/assimp-comparison/Windows-builds/`.
- Touched-buffer memory API tests and independent review: `target/review/windows-memory-experiment.json`, `windows-collector-tests.log`, `.git/baozi-review/pre-release-evolution-execution/windows-memory-review.md`.
- Local evidence archive: `target/review/parser-cost-diagnosis-evidence.zip`, SHA-256 `c19ce41b37f7258e9ca1b43460013b56ba76cab89e9230d6864a742b72de4888`. Its manifest retains all Windows samples/build identities and the separately scoped follow-up diagnostics.

This records the measured Windows source, not final release qualification or publication. The initial provisional checkpoint is retained separately; this complete table supersedes its partial conclusions.
