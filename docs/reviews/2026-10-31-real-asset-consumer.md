# Real-Asset Consumer Integration — 2026-10-31

The later [source-object metadata increment](2026-10-31-source-object-metadata.md) admits selected
glTF extras and makes `MorphPrimitivesTest` successful. Counts and measurements below describe the
earlier source revisions named here; they are not updated with the later increment's observations.

## Scope

PR #3 was merged as `04b4371b18d484d23641fdb8bda222741ddf2521`. The completed A–H library delivery
remains closed. This increment exercises the delivered API through an excluded, unpublished
[offline application](../../tools/consumers/assets/README.md) with actual native capability IO,
complete Scene edits, application-owned buffers, and detached original import evidence.

The source implementation is recorded by:

- `ded6781`: independent tangent construction and handedness declarations;
- `f9cd5d2`: offline consumer, unchanged upstream corpus, profiling, and CI wiring;
- `98f1f3b`: correct positive-Z glTF asset-front declaration and both consumers' regressions.

The corpus comes from the official Khronos sample repository at
`8f492390ddc7a2ae221a1369ef560589cf8c70fb`. All 52 upstream files retain their exact bytes, original
per-model notices, metadata, and applicable full license texts. The fixture directory's 55 SHA256
entries cover every other file, including independently transcribed source facts. See its
[provenance and expectations](../../tools/consumers/assets/fixtures/README.md).

## Concrete Findings and Changes

The first unchanged morph-animation asset exposed incorrect tangent declarations. Partial
`SceneSpace` validation coupled the construction algorithm to the handedness encoding, preventing
the importer from declaring glTF's known `SignInW` without inventing MikkTSpace provenance. The two
facts are now independent. A claimed MikkTSpace basis still requires its defined encoding, complete
validation still requires every fact, and reflection still rejects unknown present tangent
encodings. The real asset checks all tangent components, including the reflected W sign.

Source-coordinate verification additionally found that glTF inherited Baozi's negative-Z-front
target as a source declaration. The
[glTF coordinate contract](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#coordinate-system-and-units)
defines a positive-Z asset front; camera-local negative-Z viewing is separate. All eight source
facts are now declared explicitly. Raw data is unchanged. To the application's default target,
`Q(x,y,z)=(x,y,-z)` and `s=100`; to the existing independent consumer's LH/Z-up/+Y-front/half-meter
target, `Q(x,y,z)=(x,z,y)` and `s=2`. Handwritten bind, morph, animation, instance, winding and
tangent expectations check these maps independently of the conversion implementation.

Independent review found a consumer ordering defect: missing normals were generated before source
winding was resolved, although `generate_normals` uses a numeric cross product rather than inferring
front faces. The consumer now triangulates, converts to the target axes/units with CCW fronts,
generates only missing normals, and applies a final winding-only conversion if a library caller
selects CW output. A regression covers all four CW/CCW source/target combinations. Handles are
reacquired after each Scene publication, including edits across two polygon primitives.

`SimpleSkin` revealed an admission gap rather than a reason to silently repair source data. Its
three-decimal rotation values exceed Baozi's existing `abs(norm2 - 1) <= 1e-4` policy. The original
model remains a typed `gltf.invalid_document` rejection fixture; no source quaternion or tolerance
was rewritten. `RiggedSimple` provides successful real skeletal animation coverage: mixed influence
rows, joint bindings, inverse binds, and three 50-key TRS channels with their nonzero source start
time. `MorphPrimitivesTest` remains a separate explicit rejection for selected `/asset/extras`.

## Executed Verification

On Windows x86_64 with Rust 1.95.0:

- **2,375/2,375** all-feature workspace nextest cases pass (`--exclude baozi-xtask`); four existing manual storage pilots
  remain excluded from ordinary broad runs. Log:
  `target/review/real-asset-front-axis-workspace-nextest-msrv.log`.
- The real-asset package passes **9/9** integration tests with all features, strict Clippy across all
  targets/features, and formatting. It covers five successful real assets and both exact policy
  rejections, plus current-generation edits and both source/target winding conventions.
- The existing independent source consumer passes glTF-only, OBJ/postprocess, and all-feature cases;
  the all-feature case was rerun after the positive-Z correction.
- Strict workspace Clippy, **58 doctests**, warning-free rustdoc, actionlint, Python compilation/CLI,
  and locked standalone dependency policy pass. Cargo-deny 0.20.2 reports advisories, bans, licenses,
  and sources all OK; existing duplicate-version/path-dependency warnings remain warnings under
  the unchanged policy. Log: `target/review/real-asset-dependency-policy.log`.
- Fixture SHA256 verification passes for every listed file. Independent review found the normal
  ordering issue above; its fix and the source-coordinate correction were separately re-reviewed.

The existing three native-candidate CI jobs now lint and test this standalone consumer. Linux also
retains normal and allocator-instrumented release reports. These workflow changes are local wiring;
they do not claim a completed hosted run, Apple execution, release registration, or publication.

On Ubuntu 26.04 / WSL2 x86_64, Rust 1.95.0 executes the same **9/9** application tests in release
mode with all features. Log: `target/review/real-asset-linux-release-nextest.log`. This is actual
Linux runtime evidence in WSL, separate from both Windows execution and future hosted jobs.

## Measurement Contract

The thin Python runner records the exact compiler/host, Git revision and dirty state, command argv,
stage summary, return code, and elapsed process time. Each asset runs in a new process. Linux GNU
time supplies per-child peak RSS; Windows reports RSS as unavailable. Allocator instrumentation is
an optional second executable variant with timing overhead. Library reservation peaks, actual
allocator requests, retained application/evidence bytes, and process RSS have distinct scopes.

The fixtures are small functional integration examples. One observation per asset is not a
statistical throughput benchmark or a production-memory bound. Measurements have no thresholds.
The preliminary `f9cd5d2` Linux profile predates the positive-Z correction and is not the final
source measurement.

### Final Linux Observations

Both variants execute all five successful fixtures at full source revision
`98f1f3b9007528a6d3a5382a93fdf86237be1986`, on an Intel Core i9-13900KF host, Ubuntu 26.04 / WSL2
kernel `6.18.33.2-microsoft-standard-WSL2`, Rust 1.95.0. Repository, assets, and target are on the
Windows filesystem mounted at `/mnt/f`; these timings include that environment's costs. The
reports record `git_dirty: true` with untracked engineering notes present; the
tracked implementation matches the recorded commit.

Normal release observations, one fresh process per row:

| Asset | Import (ms) | Postprocess (ms) | Buffer extraction (ms) | Peak RSS (bytes) |
| --- | ---: | ---: | ---: | ---: |
| BoxTextured | 40.683 | 9.559 | 0.174 | 9,125,888 |
| BoxInterleaved | 36.412 | 9.810 | 0.144 | 9,453,568 |
| SimpleMeshes | 35.565 | 10.529 | 0.126 | 9,474,048 |
| RiggedSimple | 42.301 | 12.634 | 0.263 | 9,302,016 |
| AnimatedMorphCube | 41.525 | 10.706 | 0.253 | 9,551,872 |

Separate allocator-instrumented release observations:

| Asset | Allocation requests | Cumulative requested bytes | Requested peak bytes | Retained output/evidence bytes |
| --- | ---: | ---: | ---: | ---: |
| BoxTextured | 737 | 380,630 | 131,880 | 26,018 |
| BoxInterleaved | 636 | 309,278 | 101,017 | 19,875 |
| SimpleMeshes | 610 | 293,350 | 94,099 | 15,539 |
| RiggedSimple | 932 | 682,022 | 221,299 | 131,812 |
| AnimatedMorphCube | 851 | 454,678 | 168,421 | 26,136 |

The normal run's import ledger peaks are 204,181–893,673 bytes; the maximum individual postprocess
ledger peaks are 15,288–46,800 bytes. Their conservative admission scopes differ from the allocator
observations above. RSS includes the whole process. Retained output bytes intentionally include the
application layout and detached original import evidence after Scene destruction.

Full JSON summaries, actual argv, host/compiler identity and stdout/stderr are retained in:

- `target/asset-profiles/98f1f3b-linux-normal/report.json`;
- `target/asset-profiles/98f1f3b-linux-allocations/report.json`.

The instrumented run's stage times and RSS remain in its separate report; they are not substituted
for normal-build measurements. The five small assets show a substantial environment/setup floor;
larger caller-provided workloads are needed before attributing scaling costs or setting thresholds.

### Final Windows Observations and Failure Recording

The same five assets complete both release variants on Windows 11, build `10.0.26300`, x86_64,
with Rust 1.95.0 and the same implementation revision. Normal observations are:

| Asset | Import (ms) | Postprocess (ms) | Buffer extraction (ms) |
| --- | ---: | ---: | ---: |
| BoxTextured | 7.797 | 0.743 | 0.035 |
| BoxInterleaved | 3.865 | 0.431 | 0.021 |
| SimpleMeshes | 3.898 | 0.422 | 0.015 |
| RiggedSimple | 6.531 | 0.866 | 0.118 |
| AnimatedMorphCube | 5.655 | 0.648 | 0.032 |

Instrumented requested peaks and retained output/evidence bytes exactly match the corresponding
Linux rows. Windows allocation request counts are 807, 683, 657, 979 and 898 respectively. All
Windows RSS fields are null with an explicit platform-unavailable reason; no Windows process-memory
claim is inferred from allocator counters. The WSL-mounted and native Windows timings have
different filesystem/runtime conditions and are not an OS comparison benchmark.

Reports and logs:

- `target/asset-profiles/98f1f3b-windows-normal/report.json`;
- `target/asset-profiles/98f1f3b-windows-allocations/report.json`.

A bounded manual negative check selects `--allocation-metrics --max-work-units 1000000`. The first
model fails before opening the primary asset: attempted `WorkUnits` would reach **29,953,674**.
The driver propagates exit **1**, stops after that asset, retains its stderr, and saves a partial
`status: failed` report with `summary: null`. Evidence is
`target/asset-profiles/98f1f3b-windows-open-work-rejection/report.json`. An earlier explicit limit
of **1** rejects during retained-store initialization and likewise preserves the partial report
under `98f1f3b-windows-work-rejection`. No automatic retry or budget increase occurs in the driver;
these are two separately requested negative test invocations.

## Next Product Decision

Keep this concrete consumer as the place to reproduce missing profiles and measure actual costs.
The next useful design work is explicit selected-metadata preservation and separately authorized
numeric recovery for common exporter output. A quaternion normalization path needs source values,
policy authority, provenance and loss evidence; a broader silent tolerance is not equivalent.
Native promotion, a renderer, exporters, and a generic compiler/cache remain separate work.
