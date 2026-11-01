# Attribute-owned tangent conventions and explicit generation review

Review date: 2026-11-01. This record covers the adopted
[tangent execution plan](../plans/2026-11-01-002-feat-attribute-owned-tangents-plan.md), beginning
at implementation baseline `4ecc5d4`. It is separate from the completed
[initial asset-workflow tranche](2026-11-01-asset-workflow-expansion.md). The earlier fuzz
campaign and target qualification do not qualify this implementation.

## Implemented boundaries

Tangent construction and bitangent reconstruction are retained per attribute, including independent
Vertex and Corner declarations on one primitive. Non-Tangent fields cannot carry declarations.
Unknown construction plus known `SignInW` is valid; MikkTSpace requires its defined encoding.
Namespaced identity/version and geometry-owned text participate in authoring admission, central
validation, borrowed reads, snapshot digest/preview and edits. The replaced global fields, schema
authority and spatial borrow lifetimes are removed. Snapshot schema revision is 6, increased from
the baseline's revision 5 because declarations moved to per-attribute transcript records.

glTF attaches unknown construction plus `SignInW` only to each authored base Tangent. Morph Tangent
XYZ deltas remain separate deformation data. Reflection checks each present base stream's encoding
before retained replacement; a supported stream cannot authorize an unrelated unsupported stream.
Normal-driven deletion removes the Tangent descriptor and owned schema names, reindexes surviving
names and metadata, and retains independent source metadata.

`generate_tangents` is a concrete single-primitive operation using the same SceneEdit transaction,
central correspondence and publication as triangulation, normal generation and space conversion.
Normal/UV domains and UV set are explicit. The initial profile accepts dense/sparse F32x3 Normal
and F32x2 UV in independently selected Vertex/Corner domains and emits dense Corner F32x4.
Authored Tangents are preserved by default; explicit replacement requires exactly one stream.

The private fixed-stack **Triangle UV Derivative v1** kernel normalizes and orthogonalizes each
corner frame without welding or smoothing across faces. Its exact-product determinant expansions
retain extreme F32 cancellation residuals. Generated XYZ rounds once to F32, generated zeros are
positive zero, and W is exactly +1 or -1. The
[algorithm research](../research/2026-11-01-tangent-generation-algorithm.md) records numeric
thresholds, independent fixtures, primary sources and the verified MikkTSpace backend decision.
This implementation does not claim MikkTSpace compatibility.

Independent numeric review identified an overly strong rounded-bitangent direction claim and
an overly permissive `64*eps64` frame threshold. The implemented contract keeps analytic UV-frame
handedness and explicitly distinguishes final F32 approximation: an ill-conditioned UV map may
produce a rounded bitangent whose near-zero dot with the raw V derivative changes sign. W is
not flipped to repair that rounded dot. The frame-conditioning threshold is `64*eps32`; a
non-axis-aligned near-zero projection counterexample rejects, while admitted near-threshold
frames are checked against an independent exact-direction reference. Neither unit-length nor
normal-orthogonality checks alone establish directional accuracy.

Any authored morph delta on the generated target, including sparse and explicit-zero payloads,
rejects the base-only profile. Empty logical morph targets remain allowed. Degenerate geometry,
UVs, normals/frames and mixed triangle handedness reject atomically. Output scratch and retained
overlap are admitted through existing fallible storage; reads and face work use existing control
checkpoints. Generated ancestry names the actual selected Position/Normal/UV and replaced Tangent
owners. No scheduler, extra Scene model, independent schema registry or new backend dependency is
introduced; only the existing SHA-256 workspace dependency is added to postprocess.

The initial preallocation envelope is deliberately conservative: `110*C + A + M + 16` work units
for Corner count `C`, attribute count `A` and morph-target count `M`. It covers worst-case sparse reads
even for dense input and must fit the existing 100,000,000-unit policy before generated scratch
is allocated. Core materialization, correspondence and publication still consume the same actual
operation ledger afterward. This is a bounded admission policy, not a measured runtime cost or a
claim that every mathematically valid large primitive is admitted.

Geometry correspondence compares the actual retained candidate with the exact borrowed source
and generated tuples. Provenance dependency selection is enforced by the private source-bound
builder and exact-parent-set tests; the central profile validator establishes reference identity,
ordering, cardinality and depth. It does not independently recompute the two new parent sets.
The sealed construction path exposes neither replaceable source inputs nor mutable candidate
tables to callers. A separate profile correspondence hardening would be a distinct change.

## Verification status

Five scoped workers implemented and cross-reviewed ownership/admission/readers, the numeric kernel,
format/consumer migration and public operations, spatial/snapshot tests, and the core transaction.
The numeric contract findings above, F32x2 selection mismatch and missing snapshot revision bump
were corrected before final qualification. The source-bound profile correspondence limitation
above is explicitly retained; it is not a claim of independent semantic parent recomputation.

Initial test runs exposed fixture attribute ordering, an out-of-range authored morph identity and
the stale frozen OBJ transcript digest. These test definitions were corrected and the transcript
was independently encoded, rather than accepting an implementation-produced hash. Five complete
handwritten transcripts retain exact bytes and SHA-256 assertions; the OBJ test retains a bounded,
test-only bridge to the frozen revision-4 oracle. No compatibility encoder exists in production.

The final Windows workspace command was `cargo +1.95.0 nextest run --locked --offline --workspace
--all-features --build-jobs 1 --test-threads 2 --no-fail-fast`: **2,545/2,545 passed**, with four
existing manual pilots skipped. Run ID: `1af46f52-a955-4e13-86f4-77aa353e11db`; log:
`target/review/tangents-workspace-nextest.log`. It includes schema revision 6, complete handwritten
transcripts, the one/last-Tangent deletion cases, final morph representations and all runtime
checkpoints. Build-time implementation input hashes are retained in
`target/review/tangents-tested-source.json`; committing the source did not change those bytes.

Standalone consumer checks and Linux numeric verification are distinct from Windows workspace
results.

The separate asset consumer ran its all-feature nextest suite with the shared `target`, one build
job and one test thread: **24/24 passed**, with one existing manual scale pilot skipped. Run ID:
`4fc276ce-95ca-461a-8dab-020039725a60`; log: `target/review/tangents-assets-nextest.log`.
The new public-facade workflow imports the unchanged upstream `BoxTextured`, generates Tangents,
reflects into the application's target, builds indexed buffers and drops the Scene. It retains
24 application vertices / 36 indices, verifies every transported tangent component and W sign
through the winding relation, checks unit/orthogonal frames and reads detached generation evidence.
Existing animated, skeletal, morph, source-metadata and indexed-buffer workflows also pass.

Linux Rust 1.95.0 ran the postprocess library's Tangent tests and the public `tangents` integration
binary using `--locked --offline --all-features --build-jobs 1 --test-threads 2 --no-fail-fast`:
**48/48 passed**, 60 unrelated tests filtered. Run ID:
`97938efb-9ddc-409d-b854-67424218018c`; log: `target/review/tangents-linux-nextest.log`.
This covers the final 24 numeric fixtures, all operation cancellation polls, callback-panic
allocation cleanup, 1 MiB caller stack, selected sparse UV reads and public generation/reflection.
It includes the final dense-zero and empty-sparse morph representation assertions. This is a
focused Linux test result, not a new sanitizer campaign or whole-workspace Linux qualification.

The final Windows all-target/all-feature workspace and asset-consumer Clippy runs both pass
with `-D warnings`: `target/review/tangents-workspace-clippy.log` and
`target/review/tangents-assets-clippy.log`. Workspace, both standalone consumers and fuzz
`cargo fmt --check` pass. Relative Markdown link review finds no missing paths.

Workspace doctests pass **58/58** under Rust 1.95.0 with `--locked --offline --workspace
--all-features --doc --jobs 1`. `cargo doc --locked --offline --workspace --all-features --no-deps
--jobs 1` also passes with `RUSTDOCFLAGS=-D warnings`. Logs:
`target/review/tangents-workspace-doctest.log` and `target/review/tangents-workspace-doc.log`.

The standalone Scene consumer's `all` feature case compiles on both `wasm32-unknown-unknown`
and `wasm32-wasip1`, using the existing `scripts/check_features.py` with `--toolchain 1.95.0
--check --target <target> --case all`. This includes the migrated authoring/read code and
postprocess. Logs: `target/review/tangents-wasm32-check.log` and
`target/review/tangents-wasip1-check.log`. These are compile checks, not browser or WASI execution.

The same Scene consumer's `all` case also executes successfully on Windows through
`python scripts/check_features.py --toolchain 1.95.0 --case all`, including its migrated custom
authoring and public-read assertions. Log: `target/review/tangents-scene-consumer.log`.

| Commit | Reviewable boundary |
| --- | --- |
| `905df7c` | Attribute-owned conventions, retained geometry/read/validation/snapshot, format migration and core Tangent edit transaction. |
| `60f12bd` | Public generator, private numeric kernel and contract, control/operation tests, independent consumer and updated model/ADRs. |

Final logs and the implementation-input hash manifest are bundled locally in
`target/review/tangents-evidence.zip`. The prior tranche's long ASan campaign remains historical
evidence only. This slice makes no new fuzz-campaign, hosted-CI, native-promotion or release claim.

## Remaining scope

Many-primitive batch editing, MikkTSpace adaptation, cross-face smoothing, deformation-aware tangent
generation, UV conversion, world transforms/bounds and additional format profiles remain separate
future slices. No registry publication, hosted CI result or native promotion is claimed here.
