# First-release maturation execution record

## Reviewed design

The [master plan](../plans/2026-11-02-001-refactor-first-release-maturation-plan.md)
replaces the implementation choices of the earlier first-release usability plan.
Problem evidence remains the [usability audit](2026-11-01-first-release-usability-audit.md)
at `881d105df8899fe2ce9f062262d6607a03f26e58`.

Five independent native Codex review lanes examined feasibility, coherence, scope,
adversarial failure scenarios and product usefulness on 2026-11-02. Feasibility and
adversarial review found no actionable plan defects. Scope and product review
identified the same missing portable file/sidecar onboarding acceptance. Coherence
review identified two incorrect references to existing precision owners.

The plan now assigns a bounded application-owned file-to-`MemoryAssetIo` example
to U5 and references the actual typed storage and base-pose modules. Final
coherence review found no unresolved findings. The reviewed design also makes the
material assignment tag, central morph/base-field invariant and animation
endpoint semantics explicit. These reviews approve an implementable design;
they are not execution, numerical correctness or allocation-bound evidence.

Raw design dossiers and review JSON are retained locally under
`.git/baozi-review/maturation-plan/`. No external programming model, registry
operation or native promotion was used.

## Execution status

All eight implementation units and their applicable local verification are complete. U1/U2
component revisions remain recorded below; the final integrated implementation is
`6fb9112cb75e1dc942f6d7e605aa9e7b23ec1559`. Independent milestone and final qualification
review have no unresolved findings. The portable release unit has ten Baozi packages: the owning
glTF decoder and both maintained forks have been removed. This graph observation is not an archive build or
registry availability result. U8 Windows/Linux unchanged-archive, current-source sanitizer and
separate WASI canonical-vector checks pass on the committed source as recorded below.

Actual publication, retirement of the existing `0.0.1`, target promotion and
unexecuted hosted checks remain separate work. Completed units retain their source identity,
commands, observed regressions and practical evidence limits here; local completion does not
substitute for those external release gates.

## U1: ownership and dependent-data integrity

MTL opacity now has one authority: the final validated opacity selects the alpha
mode. The sticky parse cache and projection parameter are deleted. Public typed
material reads cover repeated/interleaved `d` and `Tr`, plus complete replacement
within and across material libraries.

A private opened-asset owner contains reader destruction from immediately after
`AssetIo::open` returns, before work completion, identity checks and later
admission. Successful EOF followed by a destructor panic fails at the AssetIo
boundary; an earlier read, cancellation, limit or identity failure retains
precedence. Explicit finish and protected fallback destruction release each
reader exactly once. Optional dependency omissions retain their existing cache
semantics, and an IO fault leaves a healthy format registration usable.

The shared geometry transaction rejects Normal replacement or Tangent removal
when their authored morph deltas would lose meaning, including empty sparse
descriptors. Public errors identify the affected semantic. Preserve-existing
no-ops and Position-only morphs remain supported. Central deformation validation
also requires the corresponding base attributes, independently of the edit
entry point. Existing core public-deformation and application-tuple fixtures
declared Normal/Tangent deltas with only Position geometry; their bases and
profile owners were corrected rather than weakening validation.

glTF TRS and morph channels now declare `Constant` before/after extrapolation,
preserving the source's endpoint-clamping rule. Public tag regressions cover
Step, Linear and CubicSpline, with endpoints distinct from base/default values.
Actual curve evaluation remains U7 work.

PLY metadata uses one reserved append followed by existing interruptible sorting.
Structurally disjoint generated key prefixes establish uniqueness without a
repeated scan. Key order, values and owner remain observable through public
metadata reads. Windows/MSRV ledger measurements are:

| Short metadata input | 1,000 rows | 5,000 rows | 20,000 rows |
| --- | ---: | ---: | ---: |
| Comments | 100,850 | 537,045 | 2,292,166 |
| `obj_info` | 105,850 | 562,173 | 2,392,295 |
| One comment and one `obj_info` per row | 208,699 | 1,120,800 | 4,758,331 |

These are charged WorkUnits for complete imports, not CPU comparisons, elapsed
time, process memory or an allocator claim. The 20,000-comment input retains the
audit's 200,106-byte size and now imports within the default work limit. Exact
budget and comparison-chunk cancellation regressions also pass.

All five defect lanes have observed red-before-fix logs in the local review
directory. The initial focused public-interface run passed 53/53; the corrected
complete core library passed 601/601 with four existing manual skips; all seven
PLY limits tests passed. The final workspace run passed 2,708/2,708 with four
existing manual skips; strict all-target/all-feature Clippy and 58 doctests
passed. The independent asset consumer also passed 43/43, with one existing
manual skip. Three independent code-review lanes found no actionable defects in
the morph, IO/material/animation and PLY metadata changes; they inspected source
and the main-owned logs rather than independently rerunning Cargo.

### Revision and verification

The tested production/test source is `55b31c1ed1ee3e8b80d4d631e583e7974338c866`.
The four implementation commits are `eadcc09` (opacity), `bf691eb` (reader
ownership), `2b50d3b` (deformation/animation) and `55b31c1` (metadata work).
They commit the already-tested source without subsequent source edits. The
following documentation-only update does not change that source qualification.

All commands ran locally on `x86_64-pc-windows-msvc`, Rust `+1.95.0`.
Dependency-resolving commands used `--locked --offline`, one build job, the root
target directory, and no parallel Cargo processes. Ordinary nextest runs used
two test threads; the independent consumer used one.

| Gate | Command scope | Result |
| --- | --- | --- |
| Workspace | `nextest run --workspace --all-features` | 2,708 passed, 4 existing manual skips; run `ab3082b5-559e-4166-8ef4-767542e248a3` |
| Independent consumer | `nextest run --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --all-features` | 43 passed, 1 existing manual skip; run `41a1b14d-a8cf-4339-8ab8-426a4f338e23` |
| Lints | `clippy --workspace --all-targets --all-features -- -D warnings` | Passed |
| Documentation examples | `test --workspace --all-features --doc` | 58 passed |
| Formatting | `fmt --all -- --check` | Passed |

The raw logs, review JSON and revision-qualified evidence bundle are retained
locally in `.git/baozi-review/maturation-plan/`. The previous failed full run is
retained as well: all five failures came from the old tuple fixture's missing
Normal base; its corrected 14-test binary and the final full matrix both pass.
The 55,295-byte `u1-evidence.zip` has SHA-256
`7c2e6ca31014306f5e1d0a89c4f2191b498110efdac7862472cd8d6a1361fb7f`.

U1 does not qualify a new target, complete F64 support, replace the glTF decoder,
or establish release readiness. Fresh cross-target/sanitizer and final package
qualification remain U8 work. No registry, push, PR or native registration
operation was performed.

## U2: precision-preserving geometry

The component implementation is committed at
`c9182a6d3a2c3608f45707f12b759edc2a4ea569`. PLY chooses each built-in Position,
Normal and Texcoord stream's width from its declarations: Float32 and up-to-16-bit
integers use F32; Float64 and I32/U32 use F64. Mixed components join before
allocation. Dense and sparse F64 attributes use the existing canonical pools,
finite/shape validation, borrowed reads and bit-preserving snapshot path.
Color and unrelated authored transform/material/deformation types keep their
existing contracts.

Spatial queries now compose and retain F64 column-major matrices and return F64
Aabb min/max arrays. They read either Position width, including sparse defaults,
without an intermediate F32 result. Geometry-space conversion, triangulation
copy/remap and generated Normal/Tangent payloads retain their selected widths.
The common edit boundary rejects a candidate narrower than its selected
dependencies or replaced field, independently of generator behavior. Ordinary
F64 generation succeeds; unsupported numerical conditioning produces a typed
stage/location issue and returns the immutable original Scene.

The F32 determinant specialization remains selected by its source operands.
F64 computation uses common exact binary scaling, fixed FMA/TwoSum error bounds,
compensated area sums and certified direction/frame gates. An independent review
found three real defects in the initial tangent integration: promoted F64 output
bypassed projection uncertainty, an aggregate width switch displaced the F32
specialization, and face-local scales bypassed primitive admission. Those paths
were corrected, with concrete public/kernel regressions. All F64 outputs now use
the same certified frame path; each determinant selects its own operand width.
Sparse explicit values plus a used default cover the primitive magnitude scan
without expanding a large logical domain.

New sparse-F64 triangulation success cases also exposed an existing validation
count error: correspondence supplied a worst-case binary-search allowance to an
exact-completion session. Planning now reuses the measured comparison counts,
and the work regression requires equality. The numerical polygon kernel itself
already accepted the retained wide/tiny inputs.

### Component verification and practical limits

All commands ran on `x86_64-pc-windows-msvc`, Rust `+1.95.0`, with locked/offline
dependency resolution, one Cargo process and one build job. Nextest used two
test threads. The final component run covered `baozi-core`, `baozi-format-ply`
and `baozi-postprocess`, all features: **957 passed**, four existing manual skips,
run `ba5891bb-ff0a-4b0a-9b48-dde3f1b6ad5c`. Seven passing tests belong to the
concurrently implemented U5 relations query; the remaining 950 tests cover the
committed component scope. The tested U2 file bytes match the named commit.
The run was a working-tree component check with that additional U5 query, not
an isolated checkout or complete-workspace qualification.

Strict all-target/all-feature Clippy for the three component crates, six core
compile-fail doctests, and their formatting checks passed. Earlier focused runs
passed 776/776 and then 805/805. The initial red precision, derived-width and
triangulation logs remain available locally. Independent storage/remap,
query/PLY, space, generated-width/normal and numerical reviews have no remaining
actionable findings. Reviewers inspected source and main-owned evidence; they
did not independently run Cargo. The numerical reviewer also used directed
independent arithmetic probes, which are neither exhaustive proof nor repository
test execution.

Logs, reviewed-file hashes and exact staged U2 bytes are retained in
`.git/baozi-review/maturation-plan/`, including `u2-staged-source.json`,
`u2-complete-package-tests.log`, `u2-clippy-fixed.log`, `u2-doctests.log` and
the `review-u2-*.json` receipts. The independent consumer's new F64 copy-out and
world-bounds regression awaits U3's decoder integration, alongside its concurrent
animation/appearance changes. Full workspace, target, sanitizer and unchanged
archive-consumer qualification remain U8 work. This component result does not
qualify a new target or establish release readiness.

## U3 through U7: owned decoding and complete application workflows

These units are committed together at `6fb9112cb75e1dc942f6d7e605aa9e7b23ec1559` because
the default-material contract and selected transaction change canonical authoring, public reads,
snapshot framing and their downstream callers. The final combined checks and immutable source
identity are recorded below; earlier component runs retain their narrower scope.

### Owned glTF document

The private document reuses the iterative admitted JSON index and source map. Typed tables,
nested collections and escaped strings acquire their metered owners before growth. JSON/BIN
surfaces and unescaped text borrow the immutable primary snapshot. Checked dense/sparse accessor
reads feed the existing selected closure, normalized data and canonical submission. Document and
index owners are released before submission. The owning generic schema decoder, decoder-specific
panic shim and expansion multiplier are removed, together with both maintained glTF forks and
their manifest/lockfile edges. Full all-feature Cargo metadata resolves ten portable production
packages and no retired/upstream glTF decoder package.

Independent ownership and semantic review found no remaining decoder findings. Three migration
errors were corrected with selected/omitted regressions: generic textureInfo ignores normal-only
`scale`; an explicit null sparse byte offset rejects rather than defaulting; known optional morph
references treat null as absent while unknown properties still validate their required index.
The complete importer package passed 332/332 before final integration.

### Selected geometry and plan ownership

One selected plan counts, reconstructs and compares the geometry table, retaining source scalar
bits, sparse encoding, Vertex identity and exact strip/fan assembly. Primitive-local projected
inputs let lowering, Normal and Tangent stages share one final transaction. Single-target functions
delegate to the same production engine; their former full-table reconstruction implementations
are gone. Small test-only headers retain existing numeric/teardown coverage through that engine.

Independent review found a real plan-isolation defect: plan-specific mappings were cached in
shared replacement `Cell`s, so a second overlapping plan could overwrite a live plan's origins.
The replacement descriptors now contain no mutable plan cache. An independently metered mapping
owner lends an exclusive initialization capability, then immutable sealed rows; the caller cannot
reuse that owner's rows while any plan/postflight borrow remains live. The application transaction
owns and drops that scratch before terminal preparation. A real core regression plans A, completes
B, rejects a later B and then centrally validates/publishes A with its original strip/fan origins.

The common core seam now enforces Normal-to-Tangent invalidation independently of the public
adapter, including Tangent morph dependencies. Mixed selections retain an evidence header and
final-generation ID for every selected target. Actual no-ops have Unchanged disposition and keep
their original values/provenance; an entirely unchanged request keeps the original Scene. These
correct the other two U4 review findings. Focused independent re-review has no remaining findings.

Terminal work starts with the exact minimal failure guarantee. Once actual selection cardinalities
are known, operation transfers only unused ordinary capacity to protected validation under one
ledger lock. Both configured caps and every historical attempted/consumed counter are preserved.
Selected evidence includes its nonempty zero-origin count prefix, actual target headers and origin
ranges. Space conversion uses the known four-slot table capacity with its exact two-member field
work. The implementation neither reserves a storage-derived multiplier nor weakens member checks.
Attempted evidence retains a denied envelope; only admitted work may consume the hard allowance.

### Material semantics, navigation and public onboarding

`MaterialAssignment` replaces the ambiguous optional binding across authoring, canonical storage
and public reads. Authored and effective accessors derive from one Unspecified/Authored/ImplicitDefault
tag. Selected glTF primitives needing a fallback share one generated PBR default, parented by the
document source; authored material counts and absence remain distinct. Image, Texture and Sampler
retain independent names. Snapshot revision 7 records the assignment tag. Frozen transcript tests
retain independent prior-framing checks rather than accepting implementation-generated digests
without an oracle.

`SceneRelations` owns bounded parent and node-to-instance CSR data. Construction admits exact
work/capacity and polls cancellation; generation-checked reads allocate nothing and can outlive the
source Scene. Public tests cover forests, placements, stale handles and limits. The facade exports
common OBJ options and the host-bundle example snapshots only an explicitly approved bounded list
into memory IO. No filesystem authority or native qualification is inferred from that example.

### Complete owned application output and evaluation

The independent asset consumer retains every UV set, material factor/binding field, texture/image/
sampler identity, owner name, declared media type and encoded payload. All UV lane bits participate
in tuple reuse, including signed zero; missing/ambiguous referenced streams produce named failures.
Scene UV origin and attribute Tangent declarations remain explicit. Public copy/drop tests retain
these values after Scene, report, importer and IO teardown. Static-output review has no findings.

Its application-owned CPU reference evaluates declared F64 time/rates, Step/Linear/CubicSpline
TRS/morph curves, hierarchy, morph-before-skin and complete unnormalized influence rows. Known
rates permit seconds conversion; unknown-rate ticks remain usable in their declared domain.
Constant and SourceDefault extrapolation stay distinct. Quaternion linear interpolation uses
shortest-path slerp; cubic uses component Hermite with raw derivatives, then normalization.

Inverse review exposed a rounded nonzero pivot on an exactly singular matrix, a false translation
rejection on a well-conditioned affine matrix, and a rounded-zero residual that admitted an
inaccurate inverse. Scaled rank/condition checks, conservative original-operand residual bounds
and complete-vector translation scaling correct those cases. Both new numeric counterexamples
were observed red in Rust before correction. The final focused evaluator run passed 21/21,
`c655024c-e364-4395-a562-cc2ac70c6fb1`; independent directed arithmetic/source re-review has no
remaining inverse finding. The complete independent consumer passed 74/74 with one existing manual
skip, `c3ef0604-6a8d-4591-a5e5-83ccbc0ff46a`, before final format/lint cleanup.

The processing recipe now prepares the selection once before whole-Scene conversion, allowing
polygon reflection without a temporary coordinate space. Generated normals use source winding
or its explicit assumption. Source material fixture counts exclude the generated default and are
checked separately. Fixed scale tests require one selected reconstruction and at most two changed
pipeline operations rather than the former per-primitive loop.

### Final integration qualification

The Windows/MSRV full workspace passes 2,855/2,855 with four existing manual skips,
run `1ec0b0e6-8421-4f36-9fb1-aaad24e83f90`; all 58 doctests pass. Fuzz seeds pass 12/12
(`28d6ab69-31dd-4a73-b173-3a77d9504705`), all twelve independent feature cases execute
successfully, and the final asset consumer passes 74/74 with one existing manual skip
(`b472764e-8784-4e4a-93cb-6647cb6fd4cb`). Strict all-target/all-feature Clippy passes for
the workspace and asset consumer, and library Clippy passes for the fuzz harness. Formatting
checks pass for the root, both consumers and fuzz workspaces. U8 qualifies that committed
source below. Raw failing and corrected logs and independent receipts remain in
`.git/baozi-review/maturation-plan/`; no failed run is relabeled as a final pass. The finite
[portable candidate record](../release/portable-0.1.0-candidate.md) keeps publication eligibility
and actual distribution qualification separate.

## U8: immutable-source portable qualification

### Source and execution environment

The corrected source is `6fb9112cb75e1dc942f6d7e605aa9e7b23ec1559`. Its Git archive,
`maturation-6fb9112.tar`, has SHA-256
`bbf62babab0bbd4a61f4966b7095bcff0a2470a007e8ef721de671e1fb200943`.
The archive contains 1,478 members, including 1,228 regular files. Linux tests use a fresh
extraction at `/home/frankorz/baozi-maturation-6fb9112`; comparison of every regular file
with the archive before and after all Linux checks found no changes. Windows runs use the same
committed source in the working checkout. Later documentation updates do not change this
implementation qualification.

All Cargo processes are serialized with one build job. Ordinary nextest runs use two test
threads; independent asset/fuzz-seed runs use one. Dependency-resolving checks use
`--locked --offline`, except the archive verifier's documented one-time staged-lock transition
and cargo-fuzz, whose separate lock-preservation checks are recorded with its observations.
Linux is unprivileged Ubuntu 26.04 under WSL2 on `x86_64-unknown-linux-gnu`; these are local
execution receipts rather than hosted CI results.

### Executed source and target checks

| Target and toolchain | Scope | Observed result |
| --- | --- | --- |
| Windows x86_64 MSVC, Rust 1.95.0 | All-feature workspace, independent assets, doctests and fixed fuzz seeds | 2,855 + 74 + 58 + 12 passed; four workspace and one consumer existing manual skips; final integration run IDs above |
| Linux x86_64 GNU, Rust 1.95.0 | All-feature workspace | 2,855 passed, four existing manual skips; nextest `e6d30da7-7766-4b81-9d6b-93b662eb8706` |
| Linux x86_64 GNU, Rust 1.95.0 | Independent all-feature asset consumer | 74 passed, one existing manual skip; nextest `06c61576-de67-479e-a189-56746583f220` |
| Linux x86_64 GNU, Rust 1.95.0 | Fixed fuzz-library seeds, including selected preparation | 12 passed; nextest `0667baf6-b96a-4f31-bd53-fddc44c895c4` |
| Browser `wasm32-unknown-unknown`, Rust 1.95.0 | Empty and complete facade feature union | Both compile; no browser runtime execution |
| WASI `wasm32-wasip1`, Rust 1.95.0/release | Empty and complete facade feature union | Both build and execute on Wasmtime 50.0.0, runtime commit `ebe5e8df1` |
| WASI `wasm32-wasip1`, Rust 1.95.0/release | Core `snapshot::tests` module, all features | 57/57 execute and pass on the same runtime; 602 unrelated core tests filtered |

The WASI all-feature module has SHA-256
`f8b1e6df0390aee6bc335354ed181eed11e20337cc858c52eeb685e629e2210e`.
Its fixed public consumer executes F64 bounds, implicit material/relations and selected-geometry
cases wherever their features are enabled. Canonical known-vector execution is a separate gate.
Final review identified that the consumer alone did not cover it, so the complete core snapshot
test module was subsequently built and executed through the same Wasmtime runner:

```text
cargo +1.95.0 test --locked --offline -p baozi-core --lib --all-features --target wasm32-wasip1 --release --jobs 1 -- snapshot::tests --test-threads=1
```

All 57 tests pass, including the frozen TRS/matrix/point/spatial/metadata digest vectors, F64
eight-byte framing/signed-zero distinctions, material-assignment snapshot presence and public
dense/sparse/deformation reads. The 11,097,159-byte executed module has SHA-256
`13cdae574cf96c7bc268421d2fe56bd1364295cee26fc33d0d63706796d563a7`.
`u8-wasi-snapshot-vectors.log` and `u8-wasi-snapshot-evidence.json` retain the actual test list,
runner, times, exit status and module identity. CI now invokes the same snapshot-module gate
after its existing pinned Wasmtime setup. Actionlint passes; no new hosted CI execution is claimed.
Browser compilation does not establish browser execution. No current Apple ARM64 execution,
other-host runtime, native filesystem admission or new target registration is inferred.

### Unchanged package rehearsal

Both archive matrices pass all twelve maintained feature cases: empty, default,
default-formats, OBJ, PLY, STL, glTF, postprocess, all-formats, tracing, mint and all.
The two independent commands are:

```text
# Windows x86_64 MSVC, Rust 1.95.0/debug
python scripts/check_packages.py --toolchain 1.95.0 --offline
# Linux x86_64 GNU, installed stable Rust 1.97.1/release
python3 scripts/check_packages.py --toolchain stable --release --offline
```

The Linux `stable` alias resolves to `rustc 1.97.1 (8bab26f4f 2026-07-14)`, LLVM 22.1.6;
this records the installed compiler rather than claiming it is the newest public stable.
Each matrix creates the ten real `.crate` artifacts and compiles their extracted contents in
a fresh output directory and Cargo home. Consumer roots use version dependencies. Standard
Cargo directory-source replacement supplies the offline registry rehearsal; there is no root
`patch`, path/git dependency substitution or rewritten artifact manifest. A separate byte
comparison confirms every unpacked Cargo manifest equals its archived manifest. Staged locks
transition to registry identities once; builds and executions then use locked/offline resolution.
Public registry availability and registry-only installation remain future release gates.

Windows staging is `target/package-check-5ktgayx6`; Linux staging is
`/home/frankorz/baozi-maturation-6fb9112/target/package-check-_7cbgt21`.
The two sets of actual artifacts, their digests, root manifests/locks and rehearsal configuration
are also retained outside disposable build outputs in `.git/baozi-review/maturation-plan/`
under `u8-artifacts-windows-msrv-debug` and `u8-artifacts-linux-installed-stable-release`.
Raw results are `u8-windows-archives.log` and `u8-linux-stable-release-archives.log`.

The fixed archive consumer checks adjacent F64 Position coordinates and signed zero with exact
F64 bounds, authored versus generated default material semantics, detached reverse relations,
and two-mesh geometry preparation with one publication. These complement ordinary import,
source semantics and sparse/influence/selection/unlit checks in that consumer; they do not
substitute for core canonical known-vector tests.

### Current-source sanitizer qualification

Six `backend_malformed` tests execute and pass under AddressSanitizer with
`RUSTFLAGS=-Zsanitizer=address` and `ASAN_OPTIONS=detect_leaks=1:halt_on_error=1`.
They exercise invalid and valid JSON/GLB reference, camera and animation metadata paths.
The actual test binary and both executed fuzz binaries contain `__asan_init` and
`__asan_report_load1`; `u8-linux-asan-evidence.json` retains their sizes and SHA-256 values.

The installed nightly is `rustc 1.101.0-nightly (92ffebece 2026-10-28)`, full compiler commit
`92ffebeceaa0a071066f233873c2438e73d70fe7`, LLVM 23.1.4. Cargo-fuzz is 0.13.2.
The target is Linux x86_64 GNU with address sanitizer and default fuzz debug assertions,
one build job and one fuzzer process. The original six glTF and fourteen edit-lifecycle seeds
are copied into a separate campaign directory; checked-in seeds remain unchanged.

| Target and run | Bound | Actual executions | libFuzzer-reported seconds | Exit |
| --- | --- | ---: | ---: | ---: |
| glTF smoke | `-runs=256` | 256 | 1 | 0 |
| glTF campaign | `-max_total_time=120` | 124,068 | 121 | 0 |
| glTF saved-corpus resume | `-max_total_time=30` | 36,954 | 31 | 0 |
| Edit-lifecycle smoke | `-runs=256` | 256 | 1 | 0 |
| Edit-lifecycle campaign | `-max_total_time=120` | 118,807 | 121 | 0 |
| Edit-lifecycle saved-corpus resume | `-max_total_time=30` | 20,944 | 31 | 0 |

The random seeds are 20261102, 20261103 and 20261104 for smoke, campaign and resume.
Input backstops are 65,536 bytes for glTF and 8,196 bytes for edits, ten seconds per input
and 2,048 MiB libFuzzer RSS. GlTF resume loads 844 saved files and ends with 1,134 files /
363,671 bytes; edit resume loads 911 and ends with 1,062 files / 110,295 bytes. Saved files,
active coverage entries and successful import/edit outcomes are different measurements.
The deterministic seed tests separately assert changed/unchanged/cancelled behavior.

No sanitizer failure or failing artifact is observed. Root, fuzz and both consumer lockfiles
remain byte-identical after every invocation. Cargo-fuzz has no locked-build option; offline
resolution, a preceding locked metadata check and post-command byte comparisons retain that
scope explicitly. Final corpus inventories, actual arguments, timestamps and binary identities
are retained in `u8-linux-asan-evidence.json`; logs use `u8-linux-asan-*`. The saved corpora
are also retained outside build caches under `u8-sanitizer-corpus/` in the local review directory.
These short campaigns are bounded exploration, not sustained-fuzz or exhaustive-safety evidence.
Historical longer campaigns keep their own earlier source identities.

### Requirements and review closure

| Requirements | Implemented ownership or application seam | Evidence owners |
| --- | --- | --- |
| R1, R5 | Metered selected-profile document, independent source names, bounded metadata and final opacity authority | U1/U3 reviews, admission/material/metadata regressions, malformed-input ASan |
| R2 | Declared F32/F64 streams, canonical pools, queries, numerical kernels and explicit application conversion | U2 reviews and precision regressions; fixed F64 archive case |
| R3, R6-R8 | Central deformation dependency guards, exclusively owned selected plan, one publication and public policies/limits | U1/U4 reviews, core overlapping-plan regression, public batch/failure tests and fixed fuzz seeds |
| R4 | Opened-reader teardown owned and contained by IO with first-cause precedence | U1 IO review and public callback/destructor fault tests |
| R9-R10 | Complete owned material/UV output, typed default assignment, detached relations, facade options and bounded host recipe | U5/U6 reviews, 74-test consumer, fixed archive cases and executed host-bundle example |
| R11 | Application-owned CPU curve/hierarchy/morph/skin reference with certified inverse admission | U7 and inverse follow-up reviews, 21 independent numeric evaluator tests within the full consumer |
| R12-R14 | Ten-package closure, retired decoder/duplicate-path deletion and scoped source/target qualification | Final composition/qualification reviews, unchanged archive matrices and explicit limits above |

The host-bundle example actually ran on the approved nested OBJ/MTL/image fixture, reporting
`meshes=1 primitives=1 images=1 diagnostics=0`; `u5-host-bundle-example.log` retains the result.
Independent final qualification review found no additional implementation or archive-identity
finding. It identified one missing gate, `U8-WASI-VECTORS-001`: WASI public-consumer execution
does not execute core canonical known vectors. The subsequent 57-test runner result above
addresses that gate without changing vectors or weakening assertions. Independent follow-up
marks it resolved and the final receipt has `findings: []`. Reviewer receipts distinguish source
inspection and independent arithmetic probes from main-thread Rust test execution.

### Completion boundary and retained evidence

U1-U8 are locally complete for the named source and scopes. All independent review findings
are resolved, the final reviewer is closed, and the master plan/candidate/roadmap now describe
the same delivered interfaces and qualification. The final CI/docs update changes no Rust
implementation, test, manifest or lockfile bytes from the qualified source.

The local `u8-evidence.zip` contains 2,284 members / 5,789,813 bytes, SHA-256
`e91f18bbe039228ae510834d59ebded97ca3b998ce695af29ce738a93d95834b`.
It retains scoped review receipts, source/target/command records, raw U8 logs, both sets of ten
actual package archives, version-only consumer manifests/locks, rehearsal configuration and
the final sanitizer corpora. `u8-evidence-inventory.json` records payload member sizes and digests;
the adjacent original source archive retains its separate identity above. Executable bodies
remain at their recorded build paths; their identities are preserved in the retained records.
This is local review evidence, not a published registry bundle or hosted CI artifact.

Future publication still requires the explicit release-policy registry gates and a fresh
registry-only consumer before separately authorized retirement of the `0.0.1` placeholder.
Current unexecuted hosts, browser runtime, sustained fuzzing and target/format promotion retain
their stated limits. No upload, yank, push, PR or native registration occurred in this work.
