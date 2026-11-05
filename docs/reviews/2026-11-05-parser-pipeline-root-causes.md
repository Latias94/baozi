# Parser and Pipeline Root Causes

The current large gaps are primarily inside import. This is now supported by
same-invocation phase clocks, source-level causal chains, and the existing
instruction observations. Removing numeric `RefCell` ownership did not remove
the per-byte/per-token execution protocol or borrowed-authoring reconstruction.
Those are different costs at different owners.

The evidence does not support a universal streaming explanation, an inherent
Rust penalty, or another general ownership rewrite as the next performance
step. It supports measuring the few import-internal boundaries below and then
changing the responsible loop and ownership contract. Fixed setup, unchanged
preparation, and terminal ownership remain open work; they cannot explain the
observed large-model phase distributions by themselves.

## Source and Evidence Identity

Production Baozi source remains `56eecd985693cba1826326169e6c673a7b36e3ff`.
Documentation successors do not identify a different measured Rust worker.
The ordinary comparison and its load limits are retained in the
[Linux-filesystem checkpoint](2026-11-05-linux-filesystem-convergence.md).

The measured Assimp B0 declares revision
`392a658f9c271be965271f45e7521a1b80ea4392`, version 6.0.5. Its retained source
archive SHA-256 is
`8241204095ad8b6e6898fa6337b345c72b164ecca17a981a95bcaa81b7185948`.
The newer local `repo-ref/assimp` checkout is at
`a2a586d9370a493651256ec5d03524710ce37a5f` and still declares 6.0.5. That version
string does not associate its newer parser with the measured binary. The
independent investigations read pinned Git objects and retained archive members;
the relevant file digests match the retained source manifest. This verifies local
association to the archive-declared pin, not remote release authentication.

Assimp implementation statements below concern that pinned source. In particular,
the newer checkout's direct OBJ number parser and STL cursor changes cannot
explain the older B0 worker's measured advantage.

## Same-Invocation Pipeline Location

A separate opt3/debug0, System-allocator, generic Rust 1.95 diagnostic is built
from the immutable candidate archive. Only the consumer adds outer phase clocks
and JSON fields. The complete original pipeline clock, preload placement,
requested fields, normal policy, work limit, detached output, and object cleanup
remain. All 1,464 source paths are compared before/after patch and build; only
`tools/consumers/assets/src/comparison.rs` changes.

Nine original memory/pipeline cases each retain three warmups and twenty
observations. All 207 stdout/stderr pairs and geometry observations are checked;
main independently reconstructs the medians and same-call residuals. The report
is `target/review/convergence/same-pipeline-phases/report.json`, SHA-256
`b7fd26900295df97cb5d48692d8232fce14e44a69ae5d3aa38761902b3089a66`.
The diagnostic binary SHA-256 is
`7b2adaded43f92b62489fdd703ff3ef0c93f6a3eec624c281742b9fcf76314dd`.

| Original memory pipeline | Pipeline median ms | Import median ms | Median import fraction | Median preparation fraction |
| --- | ---: | ---: | ---: | ---: |
| Repeated OBJ 1 | 0.763 | 0.459 | 61.10% | 27.51% |
| Repeated OBJ 16384 | 44.644 | 31.478 | 70.51% | 27.96% |
| Grid OBJ 128 | 154.369 | 152.485 | 98.78% | 0.034% |
| Shared glTF 1 | 1.005 | 0.869 | 86.55% | 4.59% |
| Shared glTF 8 | 1.317 | 1.172 | 88.89% | 4.05% |
| Shared glTF 32 | 2.373 | 2.188 | 92.28% | 2.72% |
| Repeated STL 1 | 0.544 | 0.411 | 75.37% | 8.53% |
| Repeated STL 1024 | 1.926 | 1.746 | 91.26% | 2.45% |
| F64 PLY neighbors | 0.601 | 0.463 | 77.50% | 7.73% |

Fractions are medians of each observation's phase/total fraction. They are not
ratios of separate medians; the displayed medians are not additive. Setup medians
are 65–75 microseconds and extraction fractions are about 0.7–1.4% here. All
same-call unlabelled residuals are 414–1,655 ns.

These are locating observations, not acceptance workers or paired Assimp/source
effects. Added clocks perturb execution; this diagnostic has no full-window
Windows load watcher. It cannot establish an optimization's expected saving or
the origin of every instruction. It does establish that a fixed-root or public
extraction change alone cannot remove most of the observed grid/glTF/STL work.
The repeated-OBJ preparation interval remains a separate significant target.

## Import-Internal Location

The follow-up diagnostic also completes with exit zero. It extracts a fresh
candidate archive into a separate Linux workspace and adds only coarse spans at
fifteen source paths, including a diagnostic-only thread-local event recorder and
consumer output. It does not modify the production checkout or ordinary workers.
Each span captures its duration before appending one event; events are collected
after the original pipeline stops. There are no per-element clocks or trace writes.

All nine cases retain the original three warmups and twenty observations. Main
reconstructs every raw log and every span median/fraction, and checks that all 207
observations exactly match the previous diagnostic's import work, budget flag,
width, fields, normal policy, geometry and detached output sizes. The report is
`target/review/convergence/import-internal-phases/report.json`, SHA-256
`8f98e32381ff8a81ac50cbb6c7c2c75169a7930b54ee6d97740ac408d6db4ae1`.
The binary SHA-256 is
`9fbc4ac8d98f029cab98eda957375d26576b53044f9e520cf5d8f35fbd15d40b`.
The visible work counter is Import WorkUnits; other ledger dimensions are not
newly exposed by this worker. Unchanged business work calls and stable visible
counters are not a newly instrumented comparison of every ledger dimension.

Independent review verifies the archive, all modified snapshots, actual binary,
all raw outputs, statistics and known parent/child bounds with no unresolved
blocking discrepancy. Its receipt is
`.git/baozi-review/performance-convergence/import-internal-phase-review-1.md`,
SHA-256 `2366aa7b635f2f53636fe34fbd0f59b4e2e8bdced2b88b05aab267436f2f8a44`.

Representative macro intervals locate distinct owners:

| Original case | Import median ms | Measured interval | Interval median ms | Median fraction of import |
| --- | ---: | --- | ---: | ---: |
| Grid OBJ 128 | 152.859 | Record/token parsing | 68.275 | 44.78% |
| Grid OBJ 128 | 152.859 | Text planning | 18.919 | 12.36% |
| Grid OBJ 128 | 152.859 | Text construction | 19.366 | 12.62% |
| Grid OBJ 128 | 152.859 | OBJ normalization | 19.940 | 13.06% |
| Grid OBJ 128 | 152.859 | Core semantic preflight | 16.994 | 11.17% |
| Repeated OBJ 16384 | 31.704 | Record/token parsing | 19.403 | 61.40% |
| Shared glTF 32 | 2.206 | Format normalization | 0.746 | 33.56% |
| Shared glTF 32 | 2.206 | Syntax/index, including duplicates | 0.176 | 8.00% |
| Shared glTF 32 | 2.206 | Typed decoding | 0.183 | 8.27% |
| Shared glTF 32 | 2.206 | Core semantic preflight | 0.268 | 12.09% |
| Repeated STL 1024 | 1.735 | Binary facet decoding | 0.066 | 3.78% |
| Repeated STL 1024 | 1.735 | Core semantic preflight | 1.076 | 61.68% |
| Repeated STL 1024 | 1.735 | Core materialization | 0.167 | 9.66% |
| Repeated STL 1024 | 1.735 | Actual retained validation | 0.076 | 4.35% |

These numbers change implementation priority. Text prefix execution is a real
cost, but the two conformance intervals are not the largest grid interval:
record/token parsing is larger. STL's endian/facet decoder is a small interval;
common semantic preflight is the first shared owner to address. glTF's syntax
and typed decode are important but do not justify replacing the complete JSON
parser first. Its format normalization includes geometry, appearance, node and
source/profile work; this is not a measurement of binary accessor arithmetic
alone. Shared-1/8/32 normalization fractions grow from 12.93% through 20.29% to
33.56% while the BIN payload stays 102 bytes.

For tiny STL-1, binary decode is 3.33% and semantic preflight 19.56%; fixed import
startup/finalization remains material. Its larger sibling does not explain every
tiny-task gap. The content-hash spans are small in this diagnostic: about 0.46%
of grid import, 0.35% of shared-32, and 1.49% of STL-1024. Per-key hashes are
inside glTF syntax, not this resource-hash span. A hash rewrite is not the first
action justified by these observations.

`import.submit_total` encloses `core.activate_total`, which encloses preflight
and materialization. `core.validation_total` encloses actual retained validation.
`gltf.normalized_total` encloses syntax/index, typed decode and normalization;
syntax/index encloses duplicate checking. These parent and child spans must not
be summed. The displayed rows are independent median fractions, not a complete
additive partition. Clock/TLS allocation overhead, code-generation changes and
unwatched host load remain diagnostic limits. This is still not a paired Assimp
or source-optimization experiment.

## OBJ: Linear Scans with Expensive Unit Boundaries

The current reader uses the complete primary snapshot and
`with_conformed_text_input`, not the available incremental record cursor.
[Text conformance](../../crates/baozi-import/src/text_input.rs) first plans decoded
length and indexes, then scans again to construct owned decoded text and line
indexes. Both loops call `run_returned` around each `classify_text_unit`. Ordinary
ASCII units have length one. Copied valid runs also receive UTF-8 checking before
append; this is distinct from the two metered classification passes.

The retained grid has 1,830,715 bytes, 66,053 records, and 247,567 tokens. The two
ASCII conformance passes therefore perform about 3.66 million prefix completions.
The existing Callgrind call-context counts are 1,830,716 and 1,830,715, the first
including the zero-completion BOM probe. This is **not** a fresh full attempted
admission per byte. Each pass admits its source amount once; the repeated cost
is successful prefix completion and execution/terminal validation.

The live chain is `ParseParser::run_returned` ->
`StoreInvocationParser::run_guarded_prefix` -> `ParserActionHost::complete` ->
`ParserFrame::complete` -> `WorkLayout::consume_direct_atomic`. It checks running
state, owns unwind resolution, validates channel/remaining work, checks arithmetic
and policy limits, and updates the prefix. The actual text cancellation cursor
polls around 4 KiB boundaries; it does not poll for each byte. Other parser
operations introduce their own additional checkpoints.

[OBJ token scanning](../../crates/baozi-format-obj/src/parser.rs) stages decoded
characters in reusable fixed arrays, then calls `inspect(&mut exec.services()?,
...)` for each non-comment character. The OBJ inspector's NUL check is small,
but each services acquisition repeats Import preflight. The retained call count
of 1,830,715 agrees with non-newline characters plus per-line setup. Token and
DecodeWork layouts are also rebound for each line.

[OBJ records](../../crates/baozi-format-obj/src/obj.rs) allocate a fresh metered
token vector per line and enter a child frame for each token. Token contents are
borrowed slices, not separately allocated strings. Every grid line fits the
minimum four-token capacity, implying 66,053 initial token-buffer growth events.
Child close has a prescribed exit checkpoint; a capacity-fit vector push still
checks storage control before returning. Thus a text cursor's 4 KiB cadence does
not bound all token/storage checkpoint frequency.

The dense geometry path then builds source-remapping tables and sorts selected
UV/normal source IDs. It performs linear source/corner passes plus O(U log U)
source-ID sorting. The older occurrence-sorting fallback is not the grid's path.
These representation and normalization costs must be located separately from
text conformance. Most lexer work is linear with large constants; calling the
whole parser quadratic would misidentify the cause.

Pinned Assimp OBJ really uses `IOStreamBuffer<char>`, reusable line storage,
direct leading-byte dispatch, and model accumulation. Its default cache is
16 MiB; this grid fits one input block. It still scans component counts and
copies numeric words into a reusable number buffer before conversion. Neither
input streaming nor zero-copy numeric parsing alone explains the memory-task
difference. Its loops do not execute Baozi's exact-prefix ledger protocol for
each character/token.

## glTF: Structural Index, Repeated Key Work, and Per-Primitive Work

Baozi builds a structural index over borrowed JSON ranges, then decodes owned
typed rows. It is neither a serde value DOM nor an allocation-free SAX stream.
Unescaped text can remain borrowed; escaped text receives decoded storage.
Lexical numeric checks and later typed conversion are separate obligations.

The current [syntax scanner](../../crates/baozi-format-gltf/src/json_syntax.rs)
creates and finalizes one SHA-256 digest per object key, then heap-sorts keys by
owner/digest/range for duplicate checking. Decoded equality remains authoritative
on equal digests. This is separate from resource content hashing. Source-map
completion has additional sorts. The scanner advances small byte prefixes through
the guarded parser protocol even when cancellation polling is less frequent.

The [typed decoder](../../crates/baozi-format-gltf/src/document/decode.rs) walks
the object's sibling chain for each requested field. Each candidate comparison
binds a work layout, opens a child, and decodes key characters again. Q requests
over k members can perform O(Qk) comparisons within one object. This is not proof
of quadratic scaling in the number of primitives.

The shared-1/8/32 inputs all use four shared accessors and one 102-byte BIN
payload; their key counts are 51/86/206. The source buffer is loaded once, not
once per primitive. Selected resource marking already deduplicates references.
Per-primitive descriptor/layout/semantic checks and decoded output still repeat.
An immutable checked-layout cache could reuse valid facts by accessor and
semantic use; a position fact is not automatically a normal/tangent fact.

Pinned Assimp glTF reads mutable JSON storage and uses RapidJSON `ParseInsitu`
to build a DOM. Its lazy dictionary caches typed accessor/buffer objects. It
still allocates and fills output arrays per primitive. The indexed fixture takes
the remapped per-element copy path, not its unremapped whole-stream memcpy path.
Assimp does not demonstrate a universal decoded-geometry cache absent from Baozi.

## STL and Common Canonical Construction

Baozi's [binary STL decoder](../../crates/baozi-format-stl/src/binary.rs) checks
the exact `84 + 50F` shape, reserves arrays, and decodes fixed-width facets in
O(F). Each facet has a poll and one observed five-dimension action. Colorless
input still builds optional-color scratch, but skips final color expansion.
Sequential topology is subsequently authored with one action per index.

Pinned Assimp STL also buffers the complete source. It writes decoded vertices
and normals into final mesh arrays and creates face indexes in another linear
pass. It is not a streaming counterexample. Its older typed-pointer reads and
looser trailing-byte check are not correctness designs to copy.

Both Baozi glTF and STL submit **borrowed** authoring slices. The common
[activation boundary](../../crates/baozi-import/src/store/scene_activation.rs)
performs semantic preflight, allocates new canonical pools, reconstructs their
payloads, validates actual retained data, and installs the result. Geometry
preflight checks each floating scalar; typed storage copying also runs work
completion for each scalar. The actual stored values are checked again during
retained validation. Removing the numeric interior-mutability adapter did not
change this staging/copy/validation contract.

The scalar check is not just an abstract trait dispatch. The concrete path is
`inspect_f32_values_from` -> `GeometryValidationContext::run_unit` ->
`ScopedAuthoringControl::run_unit` -> `CanonicalSemanticWorkSession::run_unit`.
The last function performs attempted admission, a `catch_unwind` around the
semantic callback, completion, frame finish, controlled callback disposal and
control-debt updates for each unit. This is a different protocol from the
already simplified parser boundary, which does not catch each byte separately.
The small finite check is therefore wrapped in a general fallible callback
transaction. This is a concrete architectural target, not a requirement imposed
by Rust ownership or `is_finite`. The measured preflight interval also contains
other semantic work, so its 61.68% must not be reported as the callback protocol's
measured CPU share or a promised optimization gain.

For STL 1024, source-derived counts include 1,024 facet actions, 3,072 sequential
index writes, 18,432 position/normal scalar semantic checks, at least 18,432
canonical float writes, and retained checks of those values. Different work
dimensions and action costs are not interchangeable. Existing heap attribution
also finds fewer Baozi allocation calls than Assimp for this STL task, despite
slower import. Allocation count alone is therefore contradicted as a general
explanation.

Some checks protect different trust boundaries: source semantic validation and
actual retained-output validation are not interchangeable. The correct redesign
can amortize their execution protocol or transfer a unique admitted pool; it
cannot delete one by asserting that another proves the same stored bytes.
The current storage seam does not already support adopting the decoded pools.
That would require an explicit owner/capacity/admission/failure contract.

## Preparation, Normal Algorithm, and Extraction

Unchanged preparation still sorts and validates target handles, walks all scene
primitives to select targets, constructs output records, scans `any_changed`, and
finishes evidence. Selection currently costs about O(T log T + P log T), even
though a validated primitive handle can resolve through dense indexes. A
target-driven classifier can remove the full-scene selection walk while retaining
duplicate/error ordering. Its fixed runtime and terminal allocations remain
another independent owner problem.

Repeated OBJ requests missing smooth normals. Its observed preparation fraction
is about 28%; grid/glTF preserve authored normals. The current normal kernel
caches face crosses and uses stable vertex bucketing for eligible indexed,
uniform-group input, with sorting for other smoothing groups. It does not retain
the earlier general repeated-face-neighbor scan. Its remaining arithmetic,
per-action control, output construction and edit validation need their own
attribution before another normal algorithm rewrite. Area weighting, vertex/group
identity, finite/degenerate handling and F64 numerical behavior remain explicit
contracts, rather than a single interchangeable industry algorithm.

Public extraction already binds selected attribute streams once per draw. Base
pose world transforms take O(nodes) time/space. Its same-call fractions here are
small. A suggestion to reimplement those existing mechanisms would miss the
measured import cost.

## Next Boundary and Implementation Order

One separate import-internal diagnostic is prospectively fixed in
`.git/baozi-review/performance-convergence/import-internal-phase-decision-1.json`.
It is complete; the independent diagnostic/source review and evidence receipt
are retained with the local review artifacts. Its results choose the following
bounded work order, while preserving the existing finite-root design obligations:

1. Change the common semantic-validation execution owner. Bind actual immutable
   rows/policies once around an exclusive lexical run and keep the scalar loop
   small. Begin with geometry finite-value validation, where STL timings and the
   inspected scalar call chain identify repeated unit execution. A wrapper that still
   calls the complete old unit protocol per scalar does not implement this cut.
   Keep actual retained-output validation; pool transfer can follow only when the
   remaining copy interval warrants its separate ownership contract.
2. Change the OBJ record/lexical owner. Remove service-bearing pure character
   inspection and repeated line-layout binding; address per-token frame/container
   lifetime at the parser region. Keep Unicode, recovery, error order and live
   capacity requirements explicit. In the adjacent shared text-prefix seam,
   retain the one real frame and initially keep both physical conformance passes.
3. Address repeated glTF normalization and immutable accessor/semantic facts
   before a whole JSON parser replacement. Member classification/dispatch can
   subsequently remove demonstrated repeated schema lookups. Duplicate checking
   must retain decoded equality for escaped-equivalent keys and worst-case bounds.

Each lexical cut must retain exact completed-prefix ownership at error, unwind,
allocation, callback, progress and poll frontiers. Admission still precedes each
physical effect at the same limit frontier. A whole chunk reported only after
success loses earlier completed work on panic and is not an acceptable
replacement. Cached facts do not replace actual completed physical copies or
stored-output validation. The actual compiler, error/panic and resource proofs
are required before replacing the old production path; this report does not
claim those new interfaces are already implemented.

This is an order decision, not another open-ended framework. Retire superseded
paths in each production cutover, verify affected failure/cancellation/precision
contracts, then measure the actual source change. Keep the original Windows and
Linux 14-case/110-lane release gates. Neither the diagnostics, instruction shares,
heap counts, nor an improved environmental measurement clear those gates.
