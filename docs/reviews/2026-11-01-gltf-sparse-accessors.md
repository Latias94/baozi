---
status: completed
date: 2026-11-01
---

# Shared glTF sparse accessors: execution review

This record covers the [selected sparse accessor slice](../plans/2026-11-01-005-feat-gltf-sparse-accessors-plan.md)
after `c7bf38b`. Additional influence sets, explicit scene selection and new extensions remain
separate candidates. Earlier primitive-mode qualification belongs to its own implementation state.
Implementation is `562e6da0f950683c33c063d603e52c4dbf354ba2`. The tracked tree was clean before
integrated qualification; only completion/evidence documentation changes afterward. The local
source identity manifest and logs are retained in `target/review/gltf-sparse-evidence.zip`.

## Behavior and design

The private glTF reader already provides sequential dense/sparse iteration. The adapter now admits
sparse storage through shared validation, then uses the existing typed collectors for geometry,
indices, influences, morph deltas, animation and inverse bind matrices. Missing base components
are zero before replacement and normalized conversion. COLOR VEC3 still supplies opaque alpha
through its existing conversion. Canonical payloads use existing dense storage; there is no second
JSON/accessor model, overlay allocation, core sparse rewrite, new external/runtime dependency or
snapshot schema change. A direct dev dependency on existing `baozi-core` lets integration tests
match provenance kinds through typed public readers.

The new accessor module owns shared type/layout checks. Metadata preflight validates optional base
storage, positive sparse count bounded by logical count, U8/U16/U32 selectors, independent offset
alignment, forbidden sparse stride/target, declared buffer/view coverage and checked logical size.
Selected inverse-bind accessor validation now happens before dependency IO. The backend's U32 sparse
cursor has an explicit count limit. Admitted component/dimension semantics remain unchanged.

After selected buffer loading and before materialization, each selected sparse index stream is
scanned once. The borrowed scan admits K DecodeWork units, rejects unordered/duplicate/out-of-range
selectors, and polls between at-most-4096-index chunks. Shared accessors do not repeat this scan.
Sequential overlay costs O(N + K), with K <= N; the full existing logical cell budgets cover every
subsequent consumer. Inverse-bind matrix collection now additionally admits 16 WorkUnits per
matrix before output reservation, including default identity matrices.

Accessor provenance is a sorted unique inline set of up to three contributing buffer sources.
Primitive, attribute, morph delta and skin paths retain that complete set. Animation channels union
up to six sources from input/output accessors; clips union all retained and duration-only input
contributions. Existing bounded collectors admit the larger reference capacities and work before
decode scopes. Primary source zero, shared external buffers and absent base views are explicit.
Selected source-object associations and public provenance relations remain one retained graph.

Independent review also identified and closed two shared preflight defects: normalized integer
indices/JOINTS were accepted even though readers ignore their normalized flag, and an explicit
vertex stride such as five bytes could pass component alignment. Both dense and sparse regressions
exercise rejection before dependency IO, with valid normalized attributes and four-byte-strided
base RGB plus tightly packed sparse RGB as controls.
Base vertex relative offsets now enforce four-byte alignment while preserving component-aligned
buffer view offsets (including a valid U16 view starting at byte 74). Animation accessors use the
correct tightly packed contract instead of the former vertex-layout constructor; all four admitted
animation shapes reject byteStride before data loading.

Primary semantic evidence is the
[official Khronos specification source](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/Specification.adoc#sparse-accessors),
retrieved on 2026-11-01 at source commit `59ecbdba704206bb656965ab4e202881806066f1`
(2026-10-30). The registry endpoint returned HTTP 403 to the local fetch; the official repository
provided the pinned source instead. No Assimp implementation or fixture was copied.

## Review and verification

Three native Codex agents reviewed shared decoder assumptions, implemented disjoint provenance
storage/callers and authored integration regressions. The main thread integrated validation and
admission, added the independent facade consumer regression, and runs Cargo serially.

The first glTF package run passed 236/241 tests. Two failures were replaced contracts: the old
sparse-unsupported test and the earlier generic zero-count failure category. Three new tests needed
expectation corrections: normalized float arithmetic differed by one F32 rounding bit, format-local
preflight rejects before materialization ledger attempts, and a generic declared-range error need
not contain the word sparse. No production layout or ordering rule was relaxed. Sparse and dense
normalized integer inputs now compare through the same existing conversion path exactly; independent
rational expectations are checked separately within one F32 ULP. Normalization arithmetic was not
changed by this slice.

The next package run passed 252/253; its remaining new DecodeWork-boundary expectation counted only
one of the existing JSON syntax and backend scans. Correcting that baseline to twice the JSON byte
count made the complete 21-test sparse integration binary pass (`ab1d353c-9a87-4b72-a6cb-41c2f9397f61`).
The independent owned-buffer sparse consumer passed (`e7a5ab3b-5b7e-4265-b0bf-72b00d1aee7e`), and
workspace strict Clippy passed with all targets/features. The final workspace and consumer runs
below qualify the implementation commit; these intermediate runs do not substitute for them.

| Executed check | Result | Run identity |
| --- | --- | --- |
| Sparse accessor integration | 21/21 pass | `ab1d353c-9a87-4b72-a6cb-41c2f9397f61` |
| Windows workspace, all features | 2,608/2,608 pass; 4 existing manual skips | `952c7c51-bc1a-40e0-9b96-34bfb0b0f3ec` |
| Independent asset consumer, all features | 32/32 pass; 1 existing manual skip | `65bd9451-dba4-4e78-a55f-af9b50c64a50` |
| Workspace strict Clippy | Pass, all targets/features, `-D warnings` | Local log |
| Independent asset consumer strict Clippy | Pass, all targets/features, `-D warnings` | Local log |
| Workspace and independent consumer formatting | Both pass | `cargo fmt ... -- --check` |
| glTF library, wasm32-unknown-unknown | Compile pass | No browser execution claim |

The 21 integration tests cover typed dense/sparse equivalence, all selector widths, zero and
nonzero bases, normalized conversion and implicit RGB alpha, geometry and all three morph delta
semantics, animation and skin, complete public typed provenance, ordering/range/layout rejection,
logical budgets before dependency IO and a shared-accessor DecodeWork boundary. Selector 255 is
valid for 256 logical elements and is distinct from a primitive index value's restart sentinel.
An omitted scene's sparse accessor neither authorizes nor opens its two missing dependencies.
Additional shared-layout tests cover dense/sparse vertex offsets, packed U16 UVs with a valid
component-aligned view offset, and forbidden animation stride. The independent consumer copies
geometry and morph values into application-owned indexed buffers, then verifies them after dropping
the import report, Scene and IO.

Executed integrated commands:

```text
cargo +1.95.0 clippy --locked --offline --workspace --all-targets --all-features --jobs 1 -- -D warnings
cargo +1.95.0 nextest run --locked --offline --workspace --all-features --build-jobs 1 --test-threads 2 --no-fail-fast --status-level fail --final-status-level none
cargo +1.95.0 nextest run --locked --offline --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --all-features --build-jobs 1 --test-threads 1 --no-fail-fast --status-level fail --final-status-level none
cargo +1.95.0 clippy --locked --offline --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --all-targets --all-features --jobs 1 -- -D warnings
cargo +1.95.0 fmt --all -- --check
cargo +1.95.0 fmt --manifest-path tools/consumers/assets/Cargo.toml -- --check
cargo +1.95.0 check --locked --offline -p baozi-format-gltf --lib --target wasm32-unknown-unknown --jobs 1
```

## Qualification boundary

Validation uses Rust 1.95.0 on x86_64-pc-windows-msvc, locked/offline Cargo dependencies, one build
job and the existing root target directory. Fixtures are independently authored and bounded.
These results do not promote format maturity, establish full glTF conformance, qualify new runtime
targets, run a new sanitizer/fuzz campaign or establish performance thresholds.
