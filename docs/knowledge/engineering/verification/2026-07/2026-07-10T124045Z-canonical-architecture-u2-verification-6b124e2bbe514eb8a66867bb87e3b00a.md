---
type: "Verification Evidence"
title: "Canonical architecture U2 verification"
description: "ADR governance, workspace, feature, documentation, fuzz-check, and independent review gates passed for U2."
timestamp: 2026-07-10T12:40:45Z
record_id: "6b124e2bbe514eb8a66867bb87e3b00a"
status: "passed"
producer_id: "codex-root"
run_id: "019f4919-ef38-7b32-a0b7-476f42d35dd1"
source_session: "019f4919-ef38-7b32-a0b7-476f42d35dd1"
related_plan: "docs/plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md"
git_branch: "refactor/baozi-canonical-architecture"
verified_by: "codex-root"
---

# Verification

U2 was checked at ADR governance, workspace, feature-isolation, documentation, fuzz-compilation, dependency-policy, and independent-review boundaries after the final contract and checker fixes.

# Result

Passed. Final independent architecture and documentation reviews were clean, and the final adversarial implementation review found no reproducible P0/P1 issue.

# Evidence

- `cargo run -p baozi-xtask -- adr-check`: 38 ADRs passed.
- `cargo nextest run -p baozi-xtask`: 44/44 passed, including fenced/HTML pseudo-index, symlink/reparse, path casing, `.MD`, malformed link/fence, fragment, and normalized-duplicate regressions.
- `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and strict production-target xtask Clippy: passed.
- `cargo nextest run --workspace`: 357/357 passed; `cargo nextest run --workspace --all-features`: 365/365 passed.
- `cargo check -p baozi --no-default-features`, `cargo check -p baozi --all-targets --all-features`, and `cargo nextest run -p baozi --no-default-features --features all-formats`: passed, with the all-formats run at 44/44.
- `cargo test --doc --workspace --all-features` and `RUSTDOCFLAGS=-D warnings cargo doc --workspace --all-features --no-deps`: passed.
- `cargo +nightly-2026-05-27 fuzz check gltf_import` and changed-target `rustfmt --edition 2024 --check`: passed. This is compile evidence, not sanitizer runtime evidence.
- `cargo deny check`, Actionlint 1.7.12, `cargo fmt --all -- --check`, trailing-whitespace scanning, and `git diff --check`: passed.

# Follow-up

U3 must implement the selected operation, IO, registry, option, and session contracts rather than preserving the current APIs. U4 must make the canonical Scene IR executable. U6 remains the only path for glTF admission, and U9 must retain Linux sanitizer runtime evidence.

# Citations

- [Canonical architecture plan](../../../../plans/2026-07-10-001-refactor-baozi-canonical-architecture-plan.md)
- [U2 progress](../../progress/2026-07/2026-07-10T124041Z-canonical-architecture-u2-authority-contracts-complete-2cc663e9dac544c4ac0100eacdea427a.md)
