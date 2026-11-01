---
status: completed
date: 2026-11-01
---

# Base-pose world queries: execution and review

This record covers follow-on candidate 3 selected by the
[execution plan](../plans/2026-11-01-003-feat-base-pose-world-queries-plan.md), starting from
`6c0b1ea`. The earlier tangent, asset-workflow and fuzz results remain historical evidence for
their own changes. No additional format or package-release scope is selected here.
The implemented and tested source is committed as
`947a92b4345f455bfa9ed9afd86d3dc76685efe4`. All nine recorded implementation/test file hashes in
`target/review/base-pose-tested-source.json` were checked against the staged blobs before commit;
none changed between integrated qualification and publication of that commit.

## Implemented contract

`ValidatedScene::base_pose_world_transforms` derives an owned `BasePoseWorldTransforms` table.
The existing centrally validated parent-before-child canonical order allows one forest traversal
without recursion, a queue, a parent index, or another graph. Authoring IDs need not be topological.
The internal synthetic root contributes identity. Node lookups check exact generation and range
in O(1), and the table remains usable after the Scene is dropped.

`instance_bounds` scans only the requested placement. `scene_bounds` explicitly scans and unions
all instances under one work limit. Both transform actual base Position vertices, including
unreferenced vertices and sparse defaults; uninstanced meshes do not contribute. This scope is
deliberately distinct from indexed draw extraction. Empty geometry returns `None`, while an
actual point returns a zero-size `Some(Aabb)`. Shared logical meshes have independent placements.

Node matrices use authored base poses, with no clip/time evaluation. Static bounds reject any
instance skin and any effective nonzero morph weight, even on empty geometry. Instance overrides
replace mesh defaults, and negative zero is zero. Unbound influence rows and zero-weight morph
deltas remain inert. A failing instance does not prevent targeted queries on other static instances;
it does reject a complete Scene union.

TRS arithmetic is shared with `Mat4::from_trs`, using F64 intermediates and one F32 rounding per
local component. World matrices round at each node; vertex components round after the affine
dot product. Terms accumulate in column order, with point translation last. Nonfinite results and
nonzero F64 values rounding to zero report typed errors identifying the responsible node or
instance/primitive/vertex. Bounds enclose those rounded vertices, without an exact-real interval
claim. Negative, nonuniform and zero scale, affine shear and finite cancellation are supported.

Explicit query limits bound work and transform-vector capacity bytes, with checked products,
fallible reservation, pre-allocation work admission and cooperative cancellation. Bounds and
lookups allocate no heap memory. These query-owned derived values do not participate in canonical
publication, import/postprocess storage accounts, provenance or snapshot hashing. Retained Scene
storage and generations remain unchanged. No dependency or snapshot schema change is required.

The standalone asset consumer exposes an opt-in `BasePoseSpatialSnapshot`; its ordinary animated
and deformed buffer extraction remains available. API, numeric-policy and Scene-ownership docs
describe the new contract rather than claiming bounds are retained Scene fields.

## Review and behavioral checks

Independent API/resource and numerical reviews preceded integration. Review confirmed canonical
order can remove traversal scratch, effective morph precedence must include instance overrides,
and legal TRS inputs do not guarantee representable derived matrices. The completed code review
reported no blocking issue after checking work admission, all traversal cancellation checkpoints,
deformation gates and shared-mesh placement behavior.

Core tests use handwritten expectations for reordered authoring IDs, multiple roots, two ancestors,
shared geometry, rotation, negative/nonuniform/zero scale, shear tighter than a transformed local
box, dense/sparse equivalence, sparse full coverage with an unusable default, zero explicit sparse
entries, empty geometry, unreferenced positions and present-empty indices. They check effective
morph precedence, skins, animated authored poses, foreign/range handles, Scene-drop lifetime,
exact byte/work boundaries, zero bounds allocations, pre-cancellation and unchanged digests and
retained-account snapshots on success and rejection. Numeric errors are checked at a nonzero
instance/primitive/vertex index as well as roots and descendants.

The first combined focused run passed 17/20. Three failures came from a newly authored test
fixture placing Normal before Position, violating the existing canonical attribute-key order.
Reordering that fixture and charging the one actually inspected Position descriptor resolved all
three; production code did not change. The corrected focused run passed 20/20, run ID
`1a3483a4-f46d-41a6-b62f-6c1649bcea02`.

The initial independent consumer run passed 4/4, run ID
`399063c0-4a9e-47dc-86ed-a30aa9df9479`. Its self-authored shared-placement glTF gives handwritten
Scene bounds `[-2, -16, 24] .. [14, 35, 35]`, including an unreferenced fourth vertex. Tests also
exercise unchanged upstream `RiggedSimple`, `MorphPrimitivesTest` and `AnimatedMorphCube` for
unsupported skin/nonzero morph and accepted animated zero-weight base-pose behavior.
The final consumer suite includes an actual coordinate-conversion publication: old tables reject
new-generation nodes and Scene queries, fresh tables reject old nodes, and the explicitly converted
world bounds equal `[-14, -16, -35] .. [2, 35, -24]`.

## Qualification scope

Commands use Rust `1.95.0` (`59807616e1fa2540724bfbac14d7976d7e4a3860`), Cargo nextest
`0.9.138`, and host `x86_64-pc-windows-msvc`. Direct Cargo checks use `--locked --offline`;
the existing `check_features.py` matrix uses `--locked`. Both use one Cargo build job and the
existing root target directory. Test execution uses two threads for workspace checks and one for
the standalone consumer. Logs are retained locally under `target/review/base-pose-*`.

Cancellation tests assert the pre-execution cancelled outcome. Source review verifies polling
through initialization, roots, child evaluation, leaf traversal, weights, attributes and vertices,
and before returning success; no deterministic mid-execution cancellation campaign is claimed.
No new Linux, Apple, browser-runtime, WASI-runtime, fuzz-duration, memory-growth or performance
qualification is inferred from these read-only-query tests. Portable compile checks, when executed,
remain compile-only evidence.

| Executed check | Result | Run identity |
| --- | --- | --- |
| Focused core `test(base_pose)` | 20/20 pass | `1a3483a4-f46d-41a6-b62f-6c1649bcea02` |
| Windows workspace, all features | 2,565/2,565 pass; 4 existing manual skips | `3707f025-ebb0-46dc-8a79-c39ca68f8daf` |
| Independent asset consumer, all features | 29/29 pass; 1 existing manual skip | `2c61ba5a-4294-4ecd-8ef4-731e27db7d9d` |
| Workspace and independent asset consumer strict Clippy | Both pass with all targets/features and `-D warnings` | Local logs |
| Workspace doctests, all features | 58/58 pass | Local log |
| Workspace rustdoc | Pass with `RUSTDOCFLAGS=-D warnings` | Local log |
| Workspace and independent asset consumer formatting | Both pass | `cargo +1.95.0 fmt ... -- --check` |
| Independent Scene consumer, all features, Windows execution | Pass for glTF/OBJ/PLY/STL | Local log |
| Independent Scene consumer, all features, `wasm32-unknown-unknown` | Compile pass | No browser execution claim |
| Independent Scene consumer, all features, `wasm32-wasip1` | Compile pass | No WASI execution claim |

The executed commands were (PowerShell environment assignment shown for rustdoc):

```text
cargo +1.95.0 nextest run --locked --offline --workspace --all-features --build-jobs 1 --test-threads 2 --no-fail-fast --status-level fail --final-status-level none
cargo +1.95.0 nextest run --locked --offline --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --all-features --build-jobs 1 --test-threads 1 --no-fail-fast --status-level fail --final-status-level none
cargo +1.95.0 clippy --locked --offline --workspace --all-targets --all-features --jobs 1 -- -D warnings
cargo +1.95.0 clippy --locked --offline --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --all-targets --all-features --jobs 1 -- -D warnings
cargo +1.95.0 test --locked --offline --workspace --all-features --doc --jobs 1
$env:RUSTDOCFLAGS = '-D warnings'
cargo +1.95.0 doc --locked --offline --workspace --all-features --no-deps --jobs 1
cargo +1.95.0 fmt --all -- --check
cargo +1.95.0 fmt --manifest-path tools/consumers/assets/Cargo.toml -- --check
python scripts/check_features.py --toolchain 1.95.0 --case all
python scripts/check_features.py --toolchain 1.95.0 --check --target wasm32-unknown-unknown --case all
python scripts/check_features.py --toolchain 1.95.0 --check --target wasm32-wasip1 --case all
```

The workspace's existing `ordinary_host_unwind_drops_converted_scene_and_detached_report` case deliberately
catches host panics; printed panic messages in its captured output do not indicate a test failure.
