# Assimp Principles and Rust Ownership at the Current Hotspots

Date: 2026-11-03. Baozi source: `4010bd205c7f712278bcda7b9378cd5b8019375d`
(R2). Assimp: **6.0.5**, source revision
`392a658f9c271be965271f45e7521a1b80ea4392`, as recorded in the retained comparison
build/source receipts. This investigation supports the
[convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md). It does
not establish release readiness, algorithm equivalence or a speedup for an unapplied design.

## Finding

The useful common principle is to resolve representation and ownership at a coarse boundary,
then traverse already resolved data. Both libraries acquire input, decode format-specific
representations, construct an owned native scene and copy selected detached output. Baozi
repeatedly crosses accounting, descriptor and publication boundaries inside some of those
traversals. Rust does not require that repetition.

Assimp is not uniformly streaming or allocation-free. Its pinned OBJ loader uses
`IOStreamBuffer`, constructs an OBJ model and converts it into native arrays. STL loads a
complete input buffer. glTF loads complete JSON, parses it in situ into a RapidJSON document,
loads typed resources and copies geometry into `aiScene`. These are verified loader-specific
properties, not an explanation based on the language or a universal pass count. See the pinned
[OBJ importer](https://github.com/assimp/assimp/blob/392a658f9c271be965271f45e7521a1b80ea4392/code/AssetLib/Obj/ObjFileImporter.cpp#L105),
[STL loader](https://github.com/assimp/assimp/blob/392a658f9c271be965271f45e7521a1b80ea4392/code/AssetLib/STL/STLLoader.cpp#L165)
and [glTF asset loader](https://github.com/assimp/assimp/blob/392a658f9c271be965271f45e7521a1b80ea4392/code/AssetLib/glTF2/glTF2Asset.inl#L1990).

## Costs and Necessary Semantics

| Hot owner | Pinned Assimp / actual probe | Baozi R2 | Refactor implication |
| --- | --- | --- | --- |
| Normal generation | Resolved contiguous arrays; normal arithmetic in loops; spatial sorting and proximity grouping | Source-identity buckets and robust arithmetic, but 573,463 normal actions each cross Begin/Complete | Group fixed, bounded face/candidate/output operations; retain numerical order and explicit cancellation bounds |
| Selected extraction | Resolve mesh field pointers once, then follow indices and copy tuples | Each requested tuple reacquires attributes through `ListVertex::attribute`; resolved `AttributeRef` already borrows payload | Expose and use one borrowed field reader per draw; preserve sparse access and all semantic domains |
| Unchanged preparation | Check meshes/faces/field presence; no new owning scene | Start generic retained operation, allocate selection/output staging and construct retained evidence even without a write | Inline direct invocation facts, indexed selection and deferred consuming attachment to the same operation |
| Canonical construction | Importer allocates native arrays; common preprocessor also performs real work | Source admission, authoring validation, materialization and output graph validation; per-scalar execution adapters | Preserve independent output validation; investigate prepared bounded spans and compatible buffer ownership at the owner |
| Input lifetime | Memory probe lends an already preloaded primary file; loaders can still copy it | Import admits an owned stable input snapshot and source identity | Borrowing caller bytes or consuming a supplied buffer needs an explicit lifetime/admission contract; deleting the snapshot is not automatically equivalent |
| Shared resource release | Importer owns native scene and raw borrowed views | Storage leases can escape the invocation and release independently | Shared storage release authority is justified; invocation-local arithmetic need not inherit every shared-owner cost |

The frozen probes retain the same selected detached F64 output through native cleanup. The
complete-pipeline clocks include construction and cleanup on both sides. Independent endpoint
medians are not additive pipeline components. The Assimp probe calls `ReadFile(path, 0)` and
requests `aiProcess_GenSmoothNormals` only when the selected preparation policy requires it.
Baozi's provenance, canonical validation, resource limits and rejection ownership are additional
contracts; their necessary cost does not justify repeating their discovery for every getter.

The R2 whole-worker normal call-path profile is
`target/review/convergence/r2-attribution/obj-repeated-16384.callgrind`. Begin/Complete
edges from normal callers account for 372,840,470 inclusive instructions. They overlap their
children and the enclosing normal route. They are neither additive exclusive CPU components
nor a prediction of the saving from any replacement. The whole worker totals 1,254,867,325
instructions and includes import, extraction, checking and teardown.

Normal algorithms also differ. Assimp's measured 175-degree path groups nearby positions using
a bounding-box-relative epsilon and sums unit face normals. Baozi groups exact source Vertex
identities and smoothing groups, preserves original contribution order, uses area contributions
and retains its F32/F64 numerical rejection rules. Assimp builds a spatial sort; Baozi already
has an eligible O(V+C) stable-bucket route. Copying Assimp's normal algorithm would change
semantics and is not justified by this investigation. The pinned
[normal process](https://github.com/assimp/assimp/blob/392a658f9c271be965271f45e7521a1b80ea4392/code/PostProcessing/GenVertexNormalsProcess.cpp#L123)
and [spatial lookup](https://github.com/assimp/assimp/blob/392a658f9c271be965271f45e7521a1b80ea4392/code/Common/SpatialSort.cpp#L141)
were inspected directly.

## Rust Ownership Assessment

The current design already has useful borrowing: authoring descriptors borrow parser-owned
payloads, immutable `AttributeRef` stores borrowed slices, and `list_vertices` allocates no
hidden cache. Preserve those properties. A resolved field reader can keep the borrow across
the entire loop without a second scene representation, unsafe pointer bypass or heap cache.

Work arithmetic currently uses invocation-local `RefCell`; it does **not** lock the shared
storage mutex for every numeric action. Storage has `Arc<Mutex<LedgerState>>` because release
tokens and retained allocations can outlive or move independently of an execution view.
Replacing those genuine independent owners with arbitrary borrows or `Rc` would change real
lifetime/thread contracts. The opportunity is to narrow sharing to those owners.

An exclusive, host-owned execution context is a serious alternative to continuing the current
freely split work capabilities. A scoped `&mut Context` could give one numeric kernel direct
mutable row access, restrict policy changes to action boundaries and prevent context reentry
through the borrow checker. A host-owned context also cannot be abandoned by merely forgetting
the borrowed handle. Before adoption, check actual parser nesting, storage sharing, control
callbacks and public compiler contracts. Do not retain dynamic role/reentry checks solely to
support a permissive interface that no real consumer needs, but do not remove an actual
consumer guarantee based only on an anticipated speedup.

This follows Rust's exclusive-borrow rules rather than a language-level performance promise.
The installed Rust 1.95.0 documentation was inspected: the
[borrowing rules](https://doc.rust-lang.org/book/ch04-02-references-and-borrowing.html),
[`RefCell::get_mut`](https://doc.rust-lang.org/std/cell/struct.RefCell.html#method.get_mut)
and [`Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html) explain the relevant ownership
mechanisms. They do not prove that a specific Baozi architecture is optimal.

RAII remains essential for physical release and honest failure evidence. Scratch must free
storage before releasing its lease, completed actions must be recorded immediately, and an
unwinding or forgotten pending action must not become fabricated completion. No numeric guard
may span an action, control callback or allocator. Allocation, reference-count and error traffic
should be removed by changing unnecessary ownership transitions, not by hiding required work.

Likewise, generic adapters are not automatically virtual calls. Monomorphization cannot erase
the observable admission, policy and failure operations implemented by an adapter. There is
no evidence that the coarse format-registry dispatch is the main problem. Blanket `inline`,
boxing every error, replacing `?`, introducing unchecked access or removing the format trait
are unsupported responses to the measured costs.

## Design Decisions Still Open

Independent review accepts the direction of one invocation, deferred write attachment and
indexed classification, but blocks implementation of the initial promotion proposal on two
specific issues: it must allocate an unissued shared shell **before** moving the sole state,
and it must define an honest administrative allocation metric. Standard `Arc`'s private
header is not an all-future-toolchains layout contract. A toolchain-qualified requested-layout
bound and an explicitly logical payload metric have different public meanings; neither may be
silently presented as complete heap footprint. An exclusive scoped-context alternative is
being assessed before selecting that representation.

The consuming attachment and cleanup-before-terminal seam must precede selection integration.
There must remain one accounting interpretation and one publication protocol, with obsolete
startup, full-scene selection, no-change retained-terminal and scratch-as-output paths deleted
when their replacements qualify. No second compatibility backend is planned.

The normal vocabulary proposal groups bounded physical operations and at most 32 concrete
Copy records per scan action. Its separate conservative control debt would poll before the
next action exceeds the existing 256-leaf scheduling envelope. This is a versioned budget
contract change, not a claim that fewer reported actions mean less arithmetic. It remains
design-only and needs independent review, denial/prefix/panic tests and native measurements.

## Remaining Performance Gate

Current source has not converged. In the R2 Windows confirmation, repeated normal preparation
is 37.93210 ms against 1.09160 ms and shared-glTF unchanged preparation is 194.35 us against
1.80 us. Ordinary Linux checks also retain large normal/extraction gaps. Both necessary
semantic work and removable architecture costs remain; the analysis does not exempt any lane
from the plan's complete Windows/Linux timing, memory and correctness gates.

Read-only investigations, exact inspected file hashes and the independent design review are
retained under `.git/baozi-review/performance-convergence/`. Ordinary R2 disposition belongs
in the [experiment receipt](../reviews/2026-11-03-compact-numeric-work.md), not in a claim of
speed inferred from source or instruction counts.
