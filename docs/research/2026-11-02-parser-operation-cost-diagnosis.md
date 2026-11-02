# Parser and Operation Cost Diagnosis

Date: 2026-11-02. Production control: `4455d03db190c6bef05ff5a1de024d2aac461465`.
The user requested causal diagnosis before further fearless refactoring, explicitly including
format registration, parser traits, ownership and core data structures. This continues the
active pre-release evolution goal; its final B1/U14 qualification is deferred until this
architecture decision and implementation settle.

## What Was Measured

The [Windows report](../reviews/2026-11-02-windows-native-performance.md) contains all 110 lanes,
three warmups and twenty retained pairs per lane. The source, protocol, corpus, precision,
ordinary Release binaries and allocator policies are recorded with the raw samples.

| Memory-input endpoint | Baozi median / IQR (ms) | Assimp median / IQR (ms) |
| --- | ---: | ---: |
| Repeated OBJ 16,384: import | 100.2815 / 0.7165 | 7.6323 / 0.3520 |
| Repeated OBJ 16,384: prepare | 190.1163 / 1.7790 | 1.0507 / 0.0440 |
| Repeated OBJ 16,384: extract | 4.7679 / 0.3002 | 0.4007 / 0.0376 |
| Grid OBJ 128: import | 379.1617 / 2.6508 | 23.5635 / 0.5584 |
| Grid OBJ 128: prepare | 0.1389 / 0.0055 | 0.0327 / 0.0066 |
| Grid OBJ 128: extract | 15.1121 / 0.3745 | 0.9930 / 0.0938 |

Separate endpoint medians are not an additive decomposition of the pipeline. Import includes
native scene construction and each library's required validation/preprocessing. The generated
normal case and preserved-normal case have different preparation work. Process working set,
commit and Linux RSS are whole-child measurements, not parser-only or stage memory.

### A Causal Accounting Change Already Has a Native Result

Candidate 2 changes only direct producer accounting: bind ordinary dimensions under one ledger
guard instead of repeating membership locks/searches. It leaves format parsing, observed-work
permits, cancellation and normal algorithms unchanged. Twenty retained Linux pairs per lane,
using immutable binaries on the same ext4 execution filesystem, give:

| Endpoint | Previous scanner candidate (ms) | Candidate 2 (ms) | Change |
| --- | ---: | ---: | ---: |
| Grid import | 520.1448 | 379.5184 | -27.0% |
| Repeated import | 147.7535 | 102.6568 | -30.5% |
| Repeated prepare | 190.1440 | 190.3519 | +0.1%, no demonstrated improvement |
| Grid pipeline | 536.3579 | 393.7790 | -26.6% |
| Repeated pipeline | 342.1191 | 295.3677 | -13.7% |

Import work evidence is identical for every corresponding observation: grid 13,950,268 units,
repeated 2,032,061. Assimp import controls changed about +0.2% and +1.5%, respectively. Raw
samples, dispersion and memory remain in `target/assimp-comparison/U13-ledger/report.json` and
`target/review/u13-ledger-disposition.json`. This is a six-lane incremental result, not full B1.
It is not a Windows-versus-Linux speedup claim.

### Windows Normal-Kernel Isolation

An explicit ignored manual diagnostic runs the unchanged smooth normal kernel on three
positions and 16,384 identical indexed triangles, with original-order accumulation and the
same cached face-cross representation. Buffers are allocated before its timer. Its observer
only increments a local counter; it does not provide operation accounting, admission,
cancellation, storage ownership, editing or publication.

| Position source | Kernel median / IQR (ms) | Control callback calls |
| --- | ---: | ---: |
| F32 slice | 6.5508 / 0.1429 | 933,788 |
| Public `AttributeRef` | 6.7044 / 0.0641 | 982,943 |

Both use three warmups/twenty observations and assert every output is literal `[0, 0, 1]`.
This is a diagnostic lower-scope task, not a feature-equivalent replacement for the 190 ms
operation or the Assimp comparison. The difference cannot all be assigned to bookkeeping:
the complete operation also initializes scratch, validates/edits the scene and publishes
evidence. It does establish that the unchanged geometric kernel and attribute reads can run
much faster, while producing nearly a million callback sites.

A second Windows diagnostic runs exactly 982,943 empty physical actions through the **real**
scene-edit `run_unit` interface. Its twenty retained observations have median **127.0353 ms**,
IQR **0.7660 ms**. Setup and final publication remain outside its timer. It asserts the exact
attempted/consumed increments, successful terminal evidence and zero terminal live storage and
reservations. Thus the observed-work protocol alone can consume substantial time at this
call count, even without geometric work. This is a separate controlled task; its median is
not subtracted from 190 ms or added to the kernel median to manufacture an exact breakdown.

Evidence: `target/review/parser-cost-diagnosis/integrated.json` binds both diagnostics to one
frozen source and retained optimized Windows binary, all samples and the actual execution log.
The earlier separate runs remain historical receipts and are not relabelled as this source.
The manual test
lives in `crates/baozi-postprocess/src/normals/diagnostics.rs`; both diagnostics are ignored by
normal test runs. The second diagnostic initially failed compilation because it used the wrong
evidence getter; the corrected run queries the existing header and retains the failed log.

## Source-Backed Causal Chain

1. OBJ records, token chunks and face/corner actions enter `ParseScope::with_work` and its
   group protocol. Each entry remaps semantic dimensions, reacquires the session outcome
   owner, and crosses the concrete producer into the generic ledger. Group completion sorts
   the same fixed group shape again. Candidate 2 reduced duplicate binding but retained this
   shape. Owners: `baozi-import/src/{scope.rs,store.rs}` and
   `baozi-operation/src/{concrete_store.rs,retained/operation.rs,ledger.rs}`.
2. Postprocess `operation/runtime.rs::run_unit` creates an observed-work permit around one
   small action. Normal initialization uses this for individual vector pushes, and
   `normals/input.rs::generate` routes the kernel callback through it. The common single-role
   successful route still performs three ledger acquisitions, two catalog lookups, and a
   permit Arc clone/drop. These are source-route counts, not measured contention or CPU shares.
3. `normals/kernel.rs` checks control during initialization, position reads, heap construction,
   sift-down, accumulation and output. This explains why a deliberately bounded algorithm
   repeatedly exercises the full operation protocol. The separate Windows diagnostic
   measures the number of these sites rather than inferring it from source size.
4. Historical whole-worker Callgrind attributes 38.8%/48.5% of exclusive instructions to ledger
   functions on grid/repeated. These are old Linux instruction fractions, before Candidate 2,
   not current wall-time shares. Inclusive costs are not added to their children. They identify
   routes; the native Candidate 2 comparison supplies the causal timing result. These totals
   sum every exclusive function row containing `baozi_operation::ledger::`, correcting the
   earlier narrower/understated summary. Raw counts and annotation hashes are retained in
   `target/review/parser-cost-diagnosis/historical-instruction-totals.json`.
5. Text input itself also has repeated work: `text_input.rs::plan_text_input` classifies and
   plans the complete source, then `build_text_input` classifies again, allocates decoded text
   and a line/replacement index. OBJ scans bounded chunks again and retains a parsed model
   before scene submission. These are real passes/representations; their remaining native
   cost has not yet been isolated. A full streaming framework is not justified by pass counts.

The operation interface makes stable authority expensive to reuse. This is an architectural
responsibility problem, not evidence that Rust trait dispatch is intrinsically slow.

## Explanations Ruled Out or Still Unproven

- **Uniform Assimp streaming:** false at the pinned source. OBJ uses a 16 MiB chunk cache and
  copied line scratch; PLY handles incremental records; STL copies the whole file; glTF uses
  a complete JSON DOM and owned binary buffers. OBJ retains an intermediate model, then
  builds expanded native arrays. See pinned Assimp paths `AssetLib/Obj/ObjFileImporter.cpp`,
  `include/assimp/IOStreamBuffer.h`, `AssetLib/STL/STLLoader.cpp` and
  `AssetLib/glTF2/glTF2Asset.inl` at
  [392a658f](https://github.com/assimp/assimp/tree/392a658f9c271be965271f45e7521a1b80ea4392).
- **A registry vcall per scalar:** not the current cross-format design. Erasure occurs at
  detection and whole parse invocation. PLY's separate `dyn RowReader` is a finer seam worth
  measuring if its profile warrants it. Neither justifies a second static built-in path.
- **No intermediate allocations in Assimp:** false. Its OBJ parser has per-face allocations,
  numeric token copies, retained attribute arrays and temporary-to-native expansion.
- **Identical general normal semantics:** false. Assimp's selected 175-degree branch groups
  nearby positions and sums unit face normals; Baozi smooths by source vertex identity and
  group with area weighting and bounded numerical decisions. The original planar fixture
  admits the same output, not arbitrary geometry equivalence. A redesign must preserve or
  deliberately specify these semantics rather than copy a fast branch blindly.
- **The 6.8 ms kernel explains all remaining time:** unproven. It deliberately omits work the
  complete operation promises. Phase attribution/controlled replacement must distinguish
  initialization, accounting, canonical edits and validation.
- **Every public trait must be rewritten for speed:** unproven. The user authorizes replacing
  them; that removes compatibility constraints, not the need to justify the resulting interface.

## Recommended Direction

Make work authority operation-local and bind dimensions/group order once. Independently
surviving storage, publication and physical owners keep real synchronization. Do not hold a
ledger lock or `RefMut` across user callbacks. Do not introduce a Local/Shared/Noop policy stack,
shadow ledger, deferred accounting replay or a compiler-like contract verifier.

Separate truthful per-action accounting from bounded control polling. The current exact poll
count is not a reason to preserve almost a million full protocol crossings. A revised explicit
polling bound is a breaking contract decision, to be reviewed and tested as such; resource
admission, truthful completed prefixes and failure precedence still have one authority.

The reviewed implementation continuation is recorded in the
[operation architecture plan](../plans/2026-11-02-1700-perf-import-operation-architecture-plan.md).
It also covers a typed format/plan/context and host-owned scene construction interface where
that removes real coupling. Payload adoption, borrowed text and geometry layout changes remain
measurement gates, not assumptions that every current intermediate must disappear.
