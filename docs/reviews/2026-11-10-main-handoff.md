---
artifact_contract: "ce-handoff/v1"
created_at: "2026-11-10T03:29:50.458493+00:00"
title: "Continue Baozi from main after device-transfer integration"
summary: "Main contains the qualified development checkpoint; other branch names are retired while performance convergence remains paused."
keywords: ["baozi", "handoff", "main", "branch-cleanup", "assimp"]
cwd: "F:/SourceCodes/Rust/baozi"
resume_focus: "Continue from main and the documented incomplete performance gates."
repository: "Latias94/baozi"
branch: "main"
head: "892c8823faafde17e03256fdb19d768628693bfe"
---

# Continue from main

The user's latest instruction is to merge the current development checkpoint
into remote `main` and delete the other branches. This supersedes the branch
selection in the [earlier device snapshot](2026-11-10-device-handoff.md).
Its source, test, measurement and evidence-transfer details remain historical
context; use `main` for the receiving checkout.

Merge `892c8823faafde17e03256fdb19d768628693bfe` retains both the previous local
main `0db5aa7` and development checkpoint `83426d1` as parents. Its complete
tree matches `83426d1` exactly. The integration follow-up only updates these
documents; runtime remains the qualified `6203ce2` source. No rejected
experimental runtime is merged merely to remove its branch name.

## Qualified state and open work

The [selection review](2026-11-09-prepare-selection-owner.md) owns the actual
Empty/One/Many reader replacement and deliberate caller-order error contract.
Its seven source gates passed: 3,992 workspace tests, 218 doctests, 138
asset-consumer tests, both strict Clippy gates and both format checks, with
the previously recorded manual skips. Original 1,066 input hashes remain
the source qualification boundary. There is no new Cargo execution for this
tree-preserving integration and documentation update.

Both Windows and GNU ordinary attempt 1 are terminal partial, with owned
children cleaned up. Windows C1 and GNU N1 remain not started; the other
three campaigns and both functional matrices on each host complete. Separate
heap attribution for `6203ce2` and independent full ordinary raw-pair review
remain pending. The [portable evidence snapshot](2026-11-10-device-handoff-evidence.json)
and its linked CSV exports retain those exact outcomes. Its development-branch
identity describes the historical collector checkout, not today's branch.

The [convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md)
remains paused. The latest integration permission does not clear measured
performance or release acceptance. Assimp convergence is still incomplete;
release remains **HOLD**. No publication, yank or PR is included.

## Branch cleanup and preserved local work

The cleanup removes the 31 non-main local branch names and the two non-main
remote branches after the checkpoint is retained on main. Some old branches
contain rejected experiments or history previously squash-merged into main;
they are not all ancestors of the current source. Before deletion, all 32
local branch tips and the observed origin refs are saved in a verified local
Git bundle:

```text
F:/SourceCodes/Rust/baozi/.git/baozi-review/branch-cleanup-20261110T032855Z/branches.bundle
```

The bundle has 12,765,884 bytes and SHA-256
`7c8491f363a08d417907e828341d0aeb338b4defcadf2971bc9378eb55b7123e`.
Its adjacent `manifest.json` records original refs and dirty-file hashes;
two patch files additionally preserve the modified tracked files in the old
worktrees. This archive is machine-local and does not transfer through a
normal clone. Branch removal does not imply their committed history was
accepted for performance.

Two old worktrees retain uncommitted changes. Their branch bindings are
detached at their existing HEADs to permit deleting the branch names; no
worktree directory or source file is removed, reverted or cleaned. The
original five untracked user notes also remain on this device unchanged.

## Receiving device

For a new checkout:

```text
git clone https://github.com/Latias94/baozi.git
cd baozi
```

For an existing clean checkout, fetch origin, switch to main and fast-forward
from origin/main. Local work on that device needs preserving before switching.
Rust remains 1.95.0, Cargo jobs one, nextest threads one and builds in the
checkout's existing `target`. The original snapshot contains the exact
test and corpus entry points.

Source/history, tracked reports, result tables and both handoff documents
transfer with main. Original raw profiling/measurement artifacts under
`.git/baozi-review` and `target`, ignored reference sources, the five untracked
notes, this recovery bundle and the Codex conversation/goal state do not.
Historical raw reanalysis needs a separate transfer. A new machine's native
campaign must have new host/build identities; its samples cannot fill old
missing cells. The next session can orient from this document and the
convergence plan before the user resumes performance work.
