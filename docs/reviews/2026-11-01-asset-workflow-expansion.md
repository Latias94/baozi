# Asset Fidelity and Consumer Workflow Expansion — 2026-11-01

The user adopted the initial tranche of the
[execution plan](../plans/2026-11-01-001-feat-asset-workflow-expansion-plan.md) after the
[Assimp comparison](../research/2026-11-01-baozi-assimp-gap-analysis.md). The completed A–H sequence
stays closed. All six initial-tranche units are implemented and locally verified; ordered follow-on
candidates remain outside this completed tranche.

| Unit | Status | Evidence |
| --- | --- | --- |
| A1: PLY color fidelity | Implemented and locally verified | Public typed RGBA and loss-policy regressions below. |
| A2: MTL semantics and bounded omissions | Implemented and locally verified | Typed UV/factor fields, Decal coverage and bounded loss branches below. |
| B1: indexed application buffers | Implemented and locally verified | Bit-exact tuple reconstruction and deformation/seam cases below. |
| B2: retained provenance readers | Implemented and locally verified | Generation checks, retained ancestry and exact metadata below. |
| V1: larger workflow evidence | Implemented and locally verified | Full release scale regressions, 15 normal processes and five separate allocation observations below. |
| V2: edit-lifecycle fuzz | Implemented and locally verified | Linux ASan smoke, 30-minute campaign and saved-corpus resume below. |

## A1 — PLY Color Profile

Unsigned U8/U16/U32 colors and alpha now use full-range normalization with F64 division and one
final F32 rounding. Floating color range checks precede narrowing; signed channels and nonfinite
or out-of-range values fail. Exact floating values and signed zero survive. Inexact F64 color
narrowing requires `SourceSemanticLoss` and records the vertex/property plus original/output
bits; `FormatRecovery` alone does not authorize it. Custom scalar precision is unchanged.

The named profile is documented in [PLY support](../formats/ply.md#accepted-color-profile).
Independent review found no conversion defect and requested stronger denominator-order and loss
detail tests; those additions now distinguish premature F32 conversion and a nonzero record index
with alpha alias `a`.

Executed on Windows x86_64 with Rust 1.95.0, serial Cargo builds and nextest:

- `cargo +1.95.0 nextest run --locked -p baozi --all-features --test ply_colors -j 1`:
  **4/4 passed**, nextest run `2d299493-f784-49b8-a2f8-6c5a44f142ba`. The cases iterate all three
  encodings and both channel-name spellings and assert actual typed values/diagnostics. Log:
  `target/review/asset-expansion-a1-colors-nextest.log`.
- `cargo +1.95.0 nextest run --locked -p baozi-format-ply -j 1`: **68/68 passed**, run
  `3699fdb6-a94b-4715-8c8c-cfdeea875a8e`, including existing custom F64, malformed, resource-limit
  and text-profile cases. This run preceded A2's partial-channel policy changes.
- `cargo +1.95.0 clippy --locked -p baozi --all-features --test ply_colors -- -D warnings` passes.
  Log: `target/review/asset-expansion-a1-clippy.log`. Rustfmt was run on the two changed Rust files.

These are local A1 results, not a new hosted/platform matrix, format maturity or release claim.

## A2 — Accepted MTL Semantics and Bounded Omissions

`map_Kd` has a typed option record. The named **Baozi OBJ/MTL UV Sampling Profile** retains
authored U/V offset and scale and defines `q = offset + scale*p`; finite zero, negative and
subnormal components are admitted without an invented reciprocal. The original MTL manual gives
option arities/defaults but does not uniquely fix the sampling equation. The
[OBJ profile](../formats/obj.md#diffuse-texture-options) states this limit and links primary
ecosystem corroboration. This clarified acceptance replaces the plan's earlier assumption that a
single normative equation could be derived from the manual's illustrative prose.

`-clamp on` maps to the new canonical `TextureAddressMode::Decal`: outside a closed unit interval
on either Decal axis, skip the entire binding and retain its underlying material; covered
coordinates sample as ClampToEdge. This requires no unspecified border color. The typed reader,
snapshot digest/preview, edit preservation test and
[appearance ADR](../adr/0012-material-texture-image-and-color-space-policy.md) agree. Baozi does
not implement a renderer, so this verifies retained semantics rather than pixel output.

Representable `Ka/Ks` become typed legacy factors alongside exact source text. Duplicate material
names still replace the complete definition. Known omitted `-t/-mm/-bm/-boost/-type/-texres`,
effective nonneutral W, negative legacy factors, OBJ point/line records and the three PLY partial
Normal/UV/RGB groups require explicit `SourceSemanticLoss`. Strict conformance can authorize that
loss; FormatRecovery alone cannot. Malformed operands fail before consulting omission authority.
PLY partial groups record one schema loss; discarded F64 color components do not consume per-row
precision-loss entries. Numeric validation still runs on those source values.

The completed branch list and intentionally deferred unknown syntax/property cases are explicit in
the format documents. MTL evidence currently binds its public `source` field to the primary OBJ;
the diagnostic dependency path, line, column and option identify the actual sidecar snapshot.

Independent review caught negative extra operands being dispatched as unknown options. The parser
now rejects numeric tokens before option dispatch; regressions cover both `-1` and `-inf` even
when semantic loss and format recovery are authorized. Review also prompted the partial-double
color quota regression and stronger snapshot differentiation for Decal.

## B1 — Deterministic Indexed Application Buffers

Each draw keys representatives by source PrimitiveVertex plus every selected F64 bit pattern,
including Tangent W and signed zero. First Corner encounter fixes output order. Distinct source
vertices and draw boundaries remain distinct. Fallible map/buffer reservation and checked index
conversion report application errors. No library tuple-reader allocation or layout policy changes.

Every representative copies its complete exported joint/weight sequence and all exported morph
streams once. The application's selected layout still omits the core's source-order ordinals.
Tests reconstruct all original Corners via output indices, including UV/normal/tangent seams,
same-source signed zero, coincident distinct vertices, five-influence and empty rows, and real
flat-normal seams combined with morph/skin fields after Scene destruction. The independent
application's public `ApplicationAsset::from_scene` now also accepts an already preprocessed
Scene for this concrete extraction use.

BoxTextured and BoxInterleaved each produce **24 vertices / 36 indices**. RiggedSimple is 160/564,
AnimatedMorphCube 24/36 and MorphPrimitivesTest's two draws 21/72 and 9/24. These source-derived
counts do not multiply by node instances.

Windows asset-consumer nextest run `d8b0fbf6-3875-4bb1-9f35-60e143881d0a` passed **23/23**, with
the manual large-grid test still ignored at this stage. Log:
`target/review/asset-expansion-assets-nextest.log`.

## B2 — Borrowed Retained Provenance

Canonical owner tokens now select retained direct attachments. Generation-bound `ProvenanceId`
and borrowed Source/Operation views expose immediate parents, operation kind, exact options digest
and durable evidence. Source views provide a binary-searched metadata range, borrowing exact
payloads. No graph copy, recursive walk or inferred name/index join is introduced. The
[reader documentation](../model/reading-scenes.md#retained-provenance-relationships) states costs,
empty relations, traversal responsibility and limits on retained lineage.

Core tests cover direct relation order and zero allocations. Facade tests use reordered source
indices, repeated names, shared source objects and glTF node reorder; they trace exact metadata
after triangulation, conversion and tangent deletion. Stale-generation and cross-Scene tokens are
rejected. Tests require source bytes and joins to survive pool compaction, without claiming payload
addresses remain fixed across generations. Independent review found no additional defect.

The focused format/facade nextest run passed **272/272** and core/format library tests passed
**578/578** (four existing manual pilots ignored). Logs:
`target/review/asset-expansion-formats-facade-nextest.log` and
`target/review/asset-expansion-core-lib-nextest.log`. These preceded the last negative-operand and
Decal snapshot/fixture additions; the final workspace check is recorded separately below.

## V1 — Larger Complete Workflows

V1 definitions contain exactly three families: generated 128/256 quad grids (98,304/393,216
triangulated Corners), generated 8/32 shared-resource primitives with fixed geometry, and unchanged
CesiumMan. The 63-file fixture inventory includes the seven new upstream files; every CesiumMan
file matches the pinned tree by byte size, Git blob identity and SHA-256. All 63 checksums also
match the committed Git blobs, including line-ending treatment.

Linux/WSL release nextest run `a075e89d-fe33-4dec-86b9-1aa134599d11` explicitly included the
ignored large-grid case and passed **3/3**, with no skipped tests. Command:

```text
cargo +1.95.0 nextest run --locked --release --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --test scale_workflows --build-jobs 1 --test-threads 1 --run-ignored all
```

The cases verify indexed geometry/winding/normals, shared definitions/instances/resources,
provenance traversal, released import reservations and complete owned values after Scene and
evidence destruction. CesiumMan compares every influence row, all 19 binds and every key of its
57 channels against the original binary accessors. Log:
`target/review/asset-expansion-v1-linux/scale-nextest.log`.

All measurements below use implementation **`45aafad797bfae06d1dee23246e146a51bd5eb2d`**, Rust
**1.95.0**, x86_64 Linux/WSL2 kernel **6.18.33.2-microsoft-standard-WSL2**, and the mounted
`/mnt/f/SourceCodes/Rust/baozi` workspace. Git is dirty from this execution documentation and the
pre-existing untracked engineering notes; implementation files are committed. The V2 identity
record retains the tracked diff and status at the same implementation revision. The fuzzer runs
concurrently as one bounded process, while Cargo builds use one job. These are advisory shared-host
observations, not isolated throughput benchmarks or an OS comparison.

Import work was explicitly fixed at **200,000,000** before running either grid or either profile
variant. Other import limits retain their existing defaults, including the 1,536 MiB live-byte
ceiling. Postprocessing retains its existing per-operation 100,000,000-work/1 GiB policy, including
protected terminal accounting. No global limit was raised or automatic retry used.

The same thin driver executes each of five inputs three times in fresh normal-release processes,
then once in a separately built `allocation-metrics` release variant. All **15 normal** and **five
instrumented** invocations exit zero and record zero import loss entries. Normal summaries agree
exactly on geometry counts, work, ledger peaks, source opens, changed-operation counts and reached
provenance counts across all three repetitions.

| Case | Indexed vertices / indices | Source opens | Changed operations | Reached primitive owners / DAG nodes / source objects |
| --- | ---: | ---: | ---: | ---: |
| Grid 128 | 16,641 / 98,304 | 1 | 3 | 1 / 4 / 1 |
| Grid 256 | 66,049 / 393,216 | 1 | 3 | 1 / 4 / 1 |
| Shared 8 | 32 / 48 | 3 | 9 | 8 / 34 / 18 |
| Shared 32 | 128 / 192 | 3 | 33 | 32 / 106 / 42 |
| CesiumMan | 3,273 / 14,016 | 3 | 1 | 1 / 16 / 15 |

Index counts in this table are directly asserted in the semantic regressions; the process JSON
also records each indexed vertex count. Draw definitions are not multiplied by node instances.
Reached source objects include the ancestry actually retained by edits, not just the selected
primitive's original source object, and do not claim per-Face lineage.

Normal-release stage time in milliseconds is **median [minimum, maximum]** across three processes.
RSS is an independent per-process GNU `/usr/bin/time` observation, shown as a MiB range.

| Case | Import ms | Postprocess ms | Provenance ms | Buffers ms | Peak RSS MiB [min, max] |
| --- | ---: | ---: | ---: | ---: | ---: |
| Grid 128 | 311.001 [310.783, 324.345] | 772.336 [769.065, 781.013] | 0.004 [0.004, 0.004] | 18.581 [18.322, 18.736] | [23.324, 23.422] |
| Grid 256 | 1155.505 [1139.864, 1182.941] | 3184.337 [3176.381, 3185.870] | 0.012 [0.004, 0.014] | 78.614 [75.384, 85.096] | [68.535, 68.762] |
| Shared 8 | 42.455 [41.634, 58.858] | 15.658 [14.690, 17.154] | 0.013 [0.012, 0.013] | 0.497 [0.463, 0.513] | [9.039, 9.195] |
| Shared 32 | 49.394 [45.348, 56.488] | 20.966 [18.422, 23.626] | 0.039 [0.038, 0.195] | 0.649 [0.647, 0.734] | [8.938, 9.215] |
| CesiumMan | 108.520 [104.609, 117.226] | 21.904 [20.242, 22.191] | 0.011 [0.010, 0.016] | 4.947 [4.589, 4.951] | [11.098, 11.180] |

Library work and admitted reservation peaks have different scopes from allocator requests and RSS.
Postprocess work is cumulative across the workflow; its peak column is the maximum individual
operation peak, not the sum of peaks. All byte columns below are exact byte counts.

| Case | Import work | Postprocess work | Import ledger peak bytes | Max postprocess ledger peak bytes |
| --- | ---: | ---: | ---: | ---: |
| Grid 128 | 11,341,976 | 8,067,742 | 13,717,772 | 3,814,820 |
| Grid 256 | 22,490,781 | 33,039,546 | 54,808,844 | 15,218,084 |
| Shared 8 | 24,133,230 | 41,599 | 392,795 | 28,032 |
| Shared 32 | 24,181,687 | 382,732 | 804,711 | 69,696 |
| CesiumMan | 25,264,327 | 297,912 | 4,847,020 | 571,904 |

The separately instrumented allocator observations cover the calling thread's root open, import,
edits, provenance traversal and extraction. Retained bytes include owned application output and
detached original import evidence after Scene destruction. They are intentional retained output,
not a leak assertion. Instrumented timings are not used in the preceding table.

| Case | Allocation requests | Total requested bytes | Requested peak bytes | Retained output/evidence bytes |
| --- | ---: | ---: | ---: | ---: |
| Grid 128 | 66,465 | 61,937,701 | 13,742,658 | 4,338,354 |
| Grid 256 | 263,345 | 247,072,665 | 54,833,730 | 17,314,482 |
| Shared 8 | 1,414 | 657,081 | 157,882 | 22,833 |
| Shared 32 | 3,566 | 2,346,464 | 317,966 | 40,056 |
| CesiumMan | 2,429 | 6,465,089 | 2,244,878 | 963,976 |

Four times as many grid quads produces **4.095x** postprocess work and about four times the
reservation/allocation peaks in this observation. Shared resources stay at three opens; with four
times as many primitive definitions, the existing separate normal edits produce **9.201x** work
but only a small absolute latency change at these sizes. This identifies edit-work amplification
to examine for a real many-primitive consumer. It does not establish an asymptotic proof, a latency
threshold, or a requirement for a generic scheduler or a new Scene model.

Exact argv, compiler/host identity, repetition numbers, return codes, individual summaries and
stdout/stderr/RSS files are retained in these six directories, each with `report.json`:

- `target/asset-profiles/45aafad-v1-linux-normal-static/`
- `target/asset-profiles/45aafad-v1-linux-normal-shared/`
- `target/asset-profiles/45aafad-v1-linux-normal-cesium/`
- `target/asset-profiles/45aafad-v1-linux-allocations-static/`
- `target/asset-profiles/45aafad-v1-linux-allocations-shared/`
- `target/asset-profiles/45aafad-v1-linux-allocations-cesium/`

The execution sequence is retained in `target/review/asset-expansion-v1-linux/report.json`, and
the aggregate arithmetic in `target/review/asset-expansion-v1-summary.json`. Generated input bytes,
construction formulas and checksums remain under `target/asset-workloads-v1/manifest.json`.
An independent read-only evidence review checked the six original reports, aggregate arithmetic
and integrated verification logs against this record and found no factual inconsistency.
The V1 inputs, reports and execution artifacts are also saved in
`target/review/asset-expansion-v1-evidence.zip` (**109 files**, **806,381 bytes**), SHA-256
`d3b13907cf01d0fab2f0540f61370a6af508d67c9d1acb4ae640f414b15ecf01`; archive integrity was checked.
These planar grids, repeated quads and one LINEAR skeletal model do not qualify arbitrary polygons,
production cache behavior, sampled playback, rendering or another platform/profile.

## V2 — Edit-Lifecycle Fuzz Execution

The final lifecycle seed suite passes **10/10** on Linux/WSL with Rust 1.95.0, nextest run
`6f386866-6a12-4fa0-8ac8-dce8299e4d0f`. Log:
`target/review/asset-expansion-v2-seeds-linux.log`. Seeds assert actual Changed outcomes, unchanged
generation/digest/readability on rejection or pre-execution cancellation, and reacquisition after
publication. These deterministic outcome assertions are separate from the sanitizer observations
below.

The source is committed at `45aafad`. Linux ASan smoke, the required 30-minute campaign and a
subsequent bounded resume all complete with exit zero. The original 13 checked-in seeds are copied
into a separate retained campaign directory; fuzzing does not modify the handwritten seed files.

| Run | Random seed | Corpus files loaded | Actual executions | libFuzzer-reported time | Exit |
| --- | ---: | ---: | ---: | ---: | ---: |
| Smoke (`-runs=256`) | 20261101 | 13 | 256 | 4 seconds | 0 |
| Campaign (`-max_total_time=1800`) | 20261102 | 78 | 375,593 | 1,803 seconds | 0 |
| Resume (`-max_total_time=120`) | 20261103 | 1,378 | 16,388 | 126 seconds | 0 |

The resume log explicitly loads the campaign's **1,378 saved files**. The final directory contains
**1,443 files / 169,790 bytes**, each inventoried by name, size and SHA-256. At campaign end,
libFuzzer reports 31,239 covered counters, 71,783 features and 1,177 active corpus entries; the
resume ends with 31,239 counters, 72,299 features and 947 active entries after reinitialization.
Active coverage entries differ from saved on-disk files; neither is a count of successful edits.
The separately asserted seeds establish successful Changed and cancellation coverage. No failing
input or sanitizer report was produced; the retained artifact directory is empty, so no minimized
failure regression was added. This bounded run is exploration evidence rather than proof that all
edits or inputs are safe.

The local installed `nightly` resolves to **rustc 1.101.0-nightly (2026-10-28)**, exact compiler
commit `92ffebeceaa0a071066f233873c2438e73d70fe7`, LLVM **23.1.4**, with **cargo-fuzz 0.13.2**.
The target is `x86_64-unknown-linux-gnu` with address sanitizer, debug assertions and one build job.
Runs use maximum input length 8,196, a 10-second per-input timeout and a 1,024 MiB libFuzzer RSS
backstop. The fixed random seeds are 20261101 (smoke), 20261102 (campaign) and 20261103 (resume).
Import limits and at most three edits bound the target separately from those runtime backstops.
`target/review/asset-expansion-v2-campaign/report.json` retains exact argv, start/end timestamps,
exit status, source SHA and corpus inventory; adjacent logs retain compiler/host and run output.
The ASan binary is **92,027,880 bytes**, SHA-256
`14c9d6ccd8d264103393ff27d1b8f81dd6ba43a36bcf361b9838db99b22110c4`. Its identity is rechecked
after resume. A final Git diff confirms all implementation paths remain unchanged through the
observations. The dirty tracked differences are documentation; pre-existing untracked engineering
notes are preserved separately.

All campaign corpus hashes and archive integrity were verified. Logs, source/status diffs, binary
identity, final corpus inventory, seed/lint results and the corpus are retained in
`target/review/asset-expansion-v2-evidence.zip` (**1,459 files**, **642,236 bytes**), SHA-256
`73893837baf7f5addca1219f07dd2e0b6a217eb49f83a119c22a285377e09a32`. The binary itself remains
under `target/x86_64-unknown-linux-gnu/release/edit_lifecycle`; the archive records its hash rather
than copying it. Raw run logs are `smoke.log`, `campaign-1800s.log` and `resume-120s.log` under
`target/review/asset-expansion-v2-campaign/`.

The hosted workflow adds only a manual `resume_run_id` path: download the requested target's
retained corpus, merge by content hash without overwriting handwritten seeds, and keep corpus/log
upload on success or failure. `actionlint` and focused review pass, but no new hosted dispatch was
performed. The local saved-corpus resume above is the executed claim. WSL reported a mirrored
network setup failure and fell back to no networking; cached offline builds and all recorded
commands still completed. No WSL/network settings were changed.

## Integrated Verification and Commit Boundaries

The final Windows nextest run with `--locked --workspace --all-features --build-jobs 1
--test-threads 2` passed **2,482/2,482**, with four existing manual pilots skipped. Run ID:
`d85c6af5-018c-4a77-8db7-6b8b781c0e34`; log:
`target/review/asset-expansion-workspace-nextest.log`. It includes the final negative-extra-operand,
Decal snapshot/edit preservation and partial-double quota regressions. The source built during
this run is the implementation retained through `45aafad`; only documentation is dirty afterward.

Windows workspace Clippy with `--locked --workspace --all-targets --all-features -- -D warnings`
passes (`target/review/asset-expansion-workspace-clippy.log`). Workspace, asset-consumer and fuzz
`cargo fmt --check` all pass. `actionlint` accepts the updated
fuzz workflow. Hosted CI was queried before local supplementation: the latest completed run,
`41207025651`, passed at `04b4371b18d484d23641fdb8bda222741ddf2521`. It does not qualify this
new implementation revision. No hosted, native promotion or new release claim is inferred.

The standalone consumer's all-target/all-feature Clippy passes with `-D warnings`
(`target/review/asset-expansion-assets-clippy.log`). Core's no-default-feature `public_api` tests
pass **3/3**, run `ca4dc28e-f530-41de-9e84-84664b5380cb`
(`target/review/asset-expansion-public-api-nextest.log`). Workspace rustdoc with `--all-features
--no-deps` and `RUSTDOCFLAGS=-D warnings` passes
(`target/review/asset-expansion-workspace-doc.log`).
Workspace documentation examples also pass **58/58**, including the compile-fail ownership/API
boundaries, with `cargo +1.95.0 test --locked --workspace --all-features --doc --jobs 1`
(`target/review/asset-expansion-workspace-doctest.log`).
The new fuzz library and its seed-test code also pass Linux stable Clippy with `--locked --lib
--tests -- -D warnings`, using the fuzz manifest and shared target directory
(`target/review/asset-expansion-v2-clippy-linux.log`).

| Commit | Independently reviewable implementation |
| --- | --- |
| `4329c32` | A1 PLY admitted colors and precision evidence. |
| `f89ee3f` | A2 typed MTL profile, canonical Decal and bounded recognized omissions. |
| `b0a5f84` | B2 borrowed retained provenance and exact source-metadata ranges. |
| `d860054` | B1 indexed application buffers with seam/deformation reconstruction. |
| `81958a0` | V1 generated workloads, original CesiumMan bytes, source assertions and profiling. |
| `45aafad` | V2 bounded lifecycle target, seeds, regressions and CI corpus resume. |
