---
type: "Verification Evidence"
title: "Typed storage and retained import integration acceptance"
description: "BDE integration passes 2071 workspace tests, strict lint/docs, measured typed-storage workloads, and normal-stack consumers."
timestamp: 2026-10-29T12:42:19Z
record_id: "6c68a7cc74cb45efae979f68dfea46d9"
producer_id: "codex-baozi-main"
run_id: "session-01a185d8-b43d-7903-8eea-cbfea81861d2"
source_session: "01a185d8-b43d-7903-8eea-cbfea81861d2"
related_plan: "docs/plans/2026-10-29-001-refactor-library-delivery-plan.md"
git_branch: "refactor/baozi-canonical-architecture"
---

# Verification

Integration of retained import ownership, typed node/geometry storage, and the normalized glTF
profile on top of `04d6874`. The implementation uses one central physical admission/release
mechanism and one Scene representation. Unregistered F modules and H tooling are excluded from
this acceptance unit.

# Result

- The all-feature workspace nextest run passed 2,071/2,071 tests in 79.832 seconds. Three manual
  scale fixtures were skipped; the separate serial scale/control run passed 4/4.
- Workspace/all-target/all-feature Clippy passed with `-D warnings` on Rust 1.95.0 after the
  hierarchy/provenance depth split. Workspace doctests passed, including 36 operation tests.
- Workspace public rustdoc passed with `RUSTDOCFLAGS=-D warnings`; one private method link in a
  public storage description was replaced with a description of the actual admission guarantee.
- `cargo +1.95.0 fmt --all -- --check` and `git diff --check` passed.
- The independent complex glTF/GLB typed consumer passed on ordinary Windows debug and release
  main stacks. It covers external resources, PBR ownership, interleaved/multiple primitives,
  Matrix/TRS hierarchy, skins, morphs, and Step/Linear/CubicSpline animation. It manually registers
  the public plugin; the facade remains quarantined pending updated Linux sanitizer evidence.
- Intact dormant sessions release their healthy data during ordinary drop and unrelated host
  unwind. Explicit transient abandonment retains fail-closed containment. Real startup-baseline
  tests require zero final physical bytes/allocations without poisoning.
- Retained manifest, diagnostic, loss, dependency-context, and format-error payload tests cover
  the storage's actual lifetime after Scene/session teardown.
- Exact hierarchy limits do not accidentally limit generated/default provenance. Core separately
  rejects excess provenance entities and depth; the empty glTF depth-one regression now passes.

# Evidence

Representative commands, run serially with the root target directory:

```text
cargo +1.95.0 nextest run --locked --build-jobs 1 --test-threads 4 --no-fail-fast --workspace --all-features --failure-output immediate-final
cargo +1.95.0 clippy --locked --jobs 1 --workspace --all-targets --all-features -- -D warnings
cargo +1.95.0 test --locked --jobs 1 --workspace --all-features --doc
cargo +1.95.0 doc --locked --jobs 1 --workspace --all-features --no-deps
cargo +1.95.0 run --locked --jobs 1 --manifest-path target/review/gltf-consumer/Cargo.toml --target-dir target
cargo +1.95.0 run --release --locked --jobs 1 --manifest-path target/review/gltf-consumer/Cargo.toml --target-dir target
```

The documentation command used `RUSTDOCFLAGS=-D warnings`. Local execution logs are
`target/review/bde-workspace-final.log`, `bde-workspace-clippy-final.log`,
`bde-workspace-doctests-final.log`, `bde-workspace-rustdoc-final.log`,
`gltf-independent-consumer-final.log`, and `gltf-independent-consumer-release-final.log`.
The pre-fix exact-depth reproduction is `gltf-empty-projection-depth-before.log`.

The typed-storage scale run measured 40,937 / 160,745 / 639,977 consumed work for 256 / 1,024 /
4,096 independent indexed primitive buffers. Peak allocator-request bytes were 546,042 /
2,075,898 / 8,195,322. The 10,000-node transform edit with shared 8 MiB geometry peaked at
10,387,326 requested bytes, down from 19,604,867. The 100,000-vertex, four-influence and 8 MiB
metadata edit peaked at 33,870 requested bytes, down from 12,033,084. These observations are
allocator requests, not exact RSS. The log is `target/review/typed-storage-scale.log`.

# Follow-up

Qualify a fresh committed archive with Linux ASan before glTF facade registration. Finish the
remaining D2 typed domain cutovers, F's complete geometry workflow, and G/H behavioral target and
packaged-consumer acceptance. This integration checkpoint does not complete the A-H goal.

# Citations

- [Active plan](../../../../plans/2026-10-29-001-refactor-library-delivery-plan.md)
- [Delivery progress and measurements](../../../../reviews/2026-10-29-delivery-progress.md)
- [Retained physical storage boundary](../../../../adr/0044-retained-physical-storage-ownership-and-account-lineage.md)
- [Scene ownership and generation](../../../../adr/0045-canonical-scene-ownership-construction-and-generation.md)
