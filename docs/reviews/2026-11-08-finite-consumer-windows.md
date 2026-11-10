# Finite consumer integration: Windows measurement checkpoint

Release and native convergence remain **HOLD**. Both candidate campaigns
complete every original lane, but the same nine of 28 pipeline medians remain
above 3x same-campaign Assimp and 33 endpoint medians are at least 10x in both
candidate campaigns. Whole-child Windows memory medians remain below 3x.

This is a fresh, independently reviewed Windows checkpoint for the
[owned finite invocation plan](../plans/2026-11-06-1240-owned-finite-invocation-plan.md)
under the unchanged [comparison protocol](../testing/assimp-comparison.md).
GNU and separate heap work remain pending at this checkpoint. No local-main
runtime integration, source-change speed acceptance or release follows.

## Qualified source and immutable workers

| Role | Source | Ordinary x64 Windows worker SHA-256 |
| --- | --- | --- |
| Control | `00b54f46ec22795377dcbc9df0f931cd93368f25` | `d4fe1c812bb31832798561432a000423b383f7da0ee80f42ac5cde26f234e249` |
| Candidate | `b76fbc0bfdb83739fdfef8425702c7b5c5d4472a` | `89243b7e45b4e51e1c3adeec41a00ddb65a0a1b3e84f8949ba2ad7a1e418c219` |
| Assimp F32 | 6.0.5, `392a658f9c271be965271f45e7521a1b80ea4392` | `07a597a733bcdf52083b9880b2c03dad51906f9f735af417b61033dd28504746` |
| Assimp F64 | same pin, PLY only | `2ba78fcdfd53a35167184612b05bc30838cb6e7624d4af0f9f98945d6e94cbb2` |

The control already contains finite Operation freeze/publication ownership.
The candidate integrates actual finite Core/Postprocess scene-edit ownership,
removes eleven resource-counter mirrors, and retains original failure/domain
custody with smaller private preparation status transport. This comparison
does not measure the total effect of every preceding refactor.

Final27 qualifies control with all seven gates; Final29/source172 qualifies
candidate with 3,956 workspace tests, 218 doctests, 138 consumer tests, both
strict Clippy gates and both formats. Existing six workspace manual skips and
one consumer scale skip remain. All 1,056 control and 1,058 candidate inputs
match committed archives and actual build inputs. Five user notes are unchanged.

All builds use this repository and existing `target`, serial jobs1. Rust is
1.95.0, opt3/debug0, generic CPU/System allocator, empty consumer features,
without allocation instrumentation or CPU/LTO flag overrides. Recorded actual
Cargo profiles and fingerprints substantiate these claims. Assimp is static,
MSVC 19.51.36256.0 `/O2 /Ob2 /DNDEBUG`, dynamic CRT, F32 OBJ/STL/PLY/glTF
and separate F64 PLY. Retained verbose commands, CMake caches and metadata
bind effective flags. UCRT is 10.0.26100.9711, vcruntime/msvcp 14.51.36247.0;
MSVC STL version145/update202604L. Ordinary workers are distinct retained files.

The release archive's embedded Git commit and every extracted source hash are
verified; the reference checkout is unchanged. The worker still correctly
reports `archive-declared-pin` rather than runtime Git verification. All seven
comparator consumer files match the actual candidate commit. F32 passes ten
CTest scenarios and F64 seven. Forty actual CLI checks pass: sixteen successes
and twenty-four expected rejection/unsupported cases, with stdout, exit code
and failure stage retained. Both full functional matrices pass 110 lanes.

## Sampling and limits

Corpus: original fourteen cases/110 lanes, manifest SHA-256
`db52cc017949e3345a07ca51411a1ab42e47b05ee2c79aef63964e0796e73f49`.
Unchanged collector SHA-256:
`9af6fbf91bb2fcdf8d36547ccef047e40b17ff1fad291b4664b22e2186dcea87`.
Both committed archives contain those same collector bytes.

Each completed campaign uses three warmup pairs and twenty retained fresh
AB/BA pairs per lane. C0 does not start: its 91 quiet-gate samples include
84 busy records; maximum quiet duration is 12.0774518s. Actual wait is
181.2150271s against the prospective 180s deadline. N0, N1 and C1 subsequently
start after their required quiet intervals and finish all lanes. Every sample,
outlier and missing cell remains. One trailing C1 is not an opening/closing
control bracket. Original raw B0 files removed during disk cleanup are
unavailable; this control does not reconstruct them.

All ratios here are **Baozi median / same-campaign Assimp median**. Collector
paired-ratio statistics remain separately expressed as Assimp/Baozi per pair.

| Campaign | Completed lanes | Pipeline >3x | Endpoints >3x | Endpoints >=10x | Working set >3x | Commit >3x | Busy/load samples |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| C0, control | not started | unavailable | unavailable | unavailable | unavailable | unavailable | 84/91 waiting |
| N0, candidate | 110 | 9/28 | 59/110 | 33/110 | 0/110 | 0/110 | 0/107 |
| N1, candidate | 110 | 9/28 | 59/110 | 33/110 | 0/110 | 0/110 | 0/107 |
| C1, control | 110 | 9/28 | 58/110 | 34/110 | 0/110 | 0/110 | 0/107 |

The repeated candidate pipeline breaches are repeated OBJ16384 and grid OBJ128
in both IO modes; shared GLB1/8/32 and binary STL1/1024 in memory mode. STL1
pipeline/memory ratios are 11.419469x in N0, 11.386143x in N1, and 11.112061x
in C1. C1's same nine pipeline breaches do not establish candidate causation.

Maximum median memory ratios across all three campaigns are 1.842939x working
set and 1.454918x commit. These are separate whole-child lifetime metrics,
including setup/checking/serialization. GNU RSS and stage heap are different
scopes; no historical memory-regression acceptance follows. The two functional
matrices have 14/14 busy load snapshots, distinct from the 321/321 clear ordinary
snapshots. Approximately two-second named-tool sampling is not machine isolation.

The [complete 110-lane CSV](2026-11-08-finite-consumer-windows-lanes.csv) exports
all five metrics and both candidates' raw and Assimp-normalized changes against
the single trailing control, including every adverse sign and unavailable RSS
cell. The tracked CSV uses repository-required LF line endings; parsed cells
are identical to the original private CRLF export. Positive change percentages are regressions, negative values reductions.
These changes remain descriptive because C0 is missing. Do not sum stage clocks
to obtain pipeline time.

## Source facts for further attribution

Assimp is not uniformly a streaming parser. At the pinned release, glTF2 reads
complete JSON/BIN regions and uses an in-place RapidJSON DOM before native mesh
conversion; binary STL reads the complete file before direct native decoding.
OBJ supports block input, but every generated OBJ fits its initial block: the
largest is 1,830,715 bytes, below its default 16MiB cache, which is clamped to the
smaller file size. Multi-block input streaming is not exercised by this corpus.

Baozi snapshots complete input, then performs format-specific parsing and
normalization before retained canonical construction. Its primary snapshot is
released before successful public import returns. Source anchors include
`crates/baozi-import/src/session.rs:526`, `:1121`, `:1367`;
`crates/baozi-format-gltf/src/parser.rs:193`, `:833`, `:1192`;
`crates/baozi-format-stl/src/binary.rs:80`, `:204`; and canonical pool allocation
and copying in `crates/baozi-core/src/canonical_scene/store/typed_storage/build.rs:408`,
`:546`. In the pinned Assimp archive the corresponding anchors are
`include/assimp/IOStreamBuffer.h:66`, `:160`,
`code/AssetLib/glTF2/glTF2Asset.inl:1955`, `:1990`,
`code/AssetLib/STL/STLLoader.cpp:165`, `:448`, and
`code/Common/BaseImporter.cpp:463`.

These facts identify work and ownership boundaries, not dominant costs. Next
attribution must separate acquisition, text/JSON planning, numeric decoding,
per-primitive repetition, canonical/provenance construction, operation accounting,
preparation, selected extraction and teardown. Assimp's required ScenePreprocessor
also executes inside import despite zero optional flags. Keep equivalent clocks,
physical work/resource bounds, cancellation and failure ownership while selecting
the next refactor from actual profiles and independently reviewed design.

## Evidence and independent review

Private retained evidence lives under `.git/baozi-review/performance-convergence`.
The ordinary envelope is `finite-consumer-ordinary-windows-campaigns-2`;
`decision.json` is partial solely because C0 is unavailable. All three collectors
return0. The launcher's intended partial exit2 and enclosing PowerShell exit1
are not failed comparison workers. Earlier MSVC environment-quoting and collector
QA-map startup failures remain separately recorded before compiler/worker launch
as applicable; corrected attempts never overwrite them.

Independent protocol review closes all amendments. Independent actual evidence
review reparses 15,180 worker stdout objects and checks 30,372 stream files;
recomputes 2,640 populated engine summaries and 1,320 paired-ratio series with
26,400 ratios; and preserves 660 null engine RSS summaries/330 empty RSS series.
All 110 export rows and 1,760 numeric change values/signs match. Identity,
precision, task/output availability, analytic geometry and F64 neighbor facts
are verified. The initial verifier's 414 UV-label vocabulary differences are
resolved against the unchanged worker convention, without rejecting samples
or altering production. A separate pinned-source inspection qualifies the
parser-strategy statements above; it makes no performance claim.

| Retained evidence | SHA-256 |
| --- | --- |
| Campaign decision | `b3b49ce7e2c7dded0324b78452e31deaa470549e308b8a65e6eca5e523836336` |
| N0 raw report | `e996752d3ace73e16f6024ff3580b4cd3e949a64ba9c5d5ec03952c96cfb747b` |
| N1 raw report | `32b7cd6d6326e3141af6bb0053dce31baed389b7dbedbbac8407d68790853b2f` |
| C1 raw report | `6071f3a67e771dacb3f9ff3d487870b43db714fdb9b2d2826f20a110e14ef528` |
| Complete matrix JSON | `06e35a6ccef15f749a61b7b5101f575cec8700dce2a312a75eab39ce622142cf` |
| Tracked complete CSV (LF) | `45fea6dbb8b1e70bee70c0240ebd6310ef23900673242755610a4469aacb75ad` |
| Original private CSV (CRLF) | `21f6e348c7c12f7e9b9c8b7e64a779ed0e6fbdcd648553d9d58d2c5d2e0ff5a4` |

The original convergence requirements remain active. GNU ordinary evidence and
fresh calibrated heap attribution must be qualified separately before final
source/main disposition.
