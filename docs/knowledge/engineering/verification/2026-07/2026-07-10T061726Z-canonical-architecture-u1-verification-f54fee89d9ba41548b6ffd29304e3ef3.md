---
type: "Verification Evidence"
title: "Canonical architecture U1 verification"
description: "Workspace, feature-isolation, fuzz compile, parser corpus, and independent review gates passed for U1."
timestamp: 2026-07-10T06:17:26Z
record_id: "f54fee89d9ba41548b6ffd29304e3ef3"
status: "passed"
producer_id: "codex-root"
run_id: "019f4919-ef38-7b32-a0b7-476f42d35dd1"
source_session: "019f4919-ef38-7b32-a0b7-476f42d35dd1"
related_plan: "docs/plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md"
git_branch: "refactor/baozi-canonical-architecture"
verified_by: "codex-root"
---

# Verification

U1 was tested at parser, facade, workspace, feature-isolation, documentation, and fuzz-compilation boundaries after the final resource-budget fixes.

# Result

Passed. Five independent final reviews found no unresolved P0/P1 issue after follow-up fixes.

# Evidence

- `cargo check --workspace --all-targets`: passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `cargo nextest run --workspace`: 314/314 passed.
- `cargo nextest run --workspace --all-features`: 326/326 passed from a clean build after removing generated `target` artifacts.
- Four format packages together: 188/188 passed.
- Facade isolation: no formats 26/26; STL 31/31; OBJ 35/35; PLY 28/28; glTF 29/29.
- `cargo +nightly-2026-05-27 fuzz check`: passed for every harness.
- `cargo test --doc --workspace --all-features`, `cargo fmt --all -- --check`, and `git diff --check`: passed.
- Exact corpus regressions cover glTF cycles/shared children, resolved texture URI reuse and amplification, generated primitive-name suffixes, PLY ASCII/binary zero-width records, duplicate topology aliases, O(1) custom attribute slots, and polygon topology exact/excess boundaries.
- A short Windows fuzz execution still cannot link the MSVC sanitizer coverage symbols (`__start/__stop___sancov_*`). The fuzz binaries did not run; Linux sanitizer CI remains the runtime authority.

# Follow-up

U2 must update ADR examples and authority. U3 must replace the temporary resource ledger with reservation/live/committed phase accounting. U9 must retain Linux sanitizer execution and must not treat Windows harness compilation as runtime evidence.

# Citations

- [Canonical architecture plan](../../../../plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md)
- [U1 progress](../../progress/2026-07/2026-07-10T061700Z-canonical-architecture-u1-safety-hardening-complete-b9cfb19f32f045dbaa60f898687ed024.md)
