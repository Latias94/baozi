---
artifact_contract: "ce-handoff/v1"
created_at: "2026-11-10T01:29:32.682009+00:00"
title: "Device handoff: Baozi performance convergence paused"
summary: "Qualified preparation-selection source and partial terminal native attempts, preserved for continuation on another device."
keywords: ["baozi", "device-handoff", "assimp", "performance", "prepare-selection"]
cwd: "F:/SourceCodes/Rust/baozi"
resume_focus: "Orient from the qualified development branch and incomplete performance acceptance before resuming work."
repository: "Latias94/baozi"
repo_root_sha: "7cbc143e5b1bbce09b9cdb3a6f5ef408c70c4ee0"
branch: "refactor/prepare-selection-owner"
head: "14119b64cb141bc1c2abb67a584207dbee0c88ec"
---

# Device handoff

The user asks to finish the current session, commit and push to the Baozi
GitHub repository, then continue on another device. The long-running
performance goal is paused, incomplete, and release remains **HOLD**. This
handoff captures the pre-closeout HEAD above; the handoff and ordinary exports
are added in a later documentation-only commit on the same branch.

## Repository and source checkpoint

The repository is `https://github.com/Latias94/baozi`; the local remote is named
`origin`. The continuation branch is `refactor/prepare-selection-owner`, not
main. Its history contains the earlier connected refactors. No runtime merge
to main, PR, release, publication or yank is part of this handoff.

The current runtime is `6203ce29850e18e71834f5010803f12e9cda2d52`,
`refactor(postprocess)!: resolve selected primitives once`. The subsequent
`14119b6` records its source and instruction qualification. The fixed ordinary
control is `b918256652e3bd15b08bd9a54de3d12caf125152`. Its historical incomplete
campaigns are not accepted by using its qualified source as a new control.

The [selection review](2026-11-09-prepare-selection-owner.md) explains the actual
replacement: one Empty/One/Many owner retains each resolved primitive reader
once. Empty/One avoid selection scratch; Many has one admitted reader backing.
The retired sorted-ID owner, discarded-reader resolution, whole-Scene walk
and binary membership search are removed. Caller-order first lookup failure
is an intentional breaking contract; successful processing remains canonical.
The [selection plan](../plans/2026-11-09-1535-prepare-selection-owner-plan.md)
owns that contract and its frozen diagnostic criteria.

## Qualification and actual unfinished acceptance

The seven original source gates pass: 3,992 workspace tests, 218 doctests,
138 independent asset-consumer tests, both strict all-target/all-feature
Clippy gates and both format checks. Six workspace manual skips and one
consumer scale skip remain. Closeout rechecks all 1,066 qualified inputs
against the original before/after hashes; no Rust code changes or new Cargo
runs occur during closeout.

Independent source/QA/archive, actual instruction and compiled-owner reviews
were closed before ordinary sampling. The original seven-task instruction
predicate passes. GLB1/GLB32/STL1 preparation-entry instructions fall
30.82%/21.92%/31.26%; corresponding whole-worker reductions are
0.22%/0.38%/0.67%. Those scopes differ, and neither establishes native elapsed
benefit. The full signed data remains in the
[instruction export](2026-11-09-prepare-selection-owner-instructions.csv).

The [ordinary closeout](2026-11-09-prepare-selection-owner.md#attempt-1-ordinary-closeout-2026-11-10)
records terminal attempt 1 on both platforms:

| Platform | Functional control and candidate | Completed ordinary campaigns | Missing campaign |
| --- | --- | --- | --- |
| Windows | Both 110 lanes, zero failures | C0, N0, N1 | C1, quiet start deadline |
| GNU | Both 110 lanes, zero failures | C0, N0, C1 | N1, quiet start deadline |

Both envelopes are `partial`, with original owned children cleaned up and no
recorded cleanup failure. The GNU Windows monitor confirms GNU cleanup. Each
completed ordinary campaign has 110 lanes, 5,062 commands, zero failures,
three warmup and twenty retained AB/BA pairs. Complete literal non-clock
contracts pass. Full independent ordinary raw-pair review remains pending;
there is no live attempt to restart. Separate heap attribution for `6203ce2`
has not started.

The corrected [Windows](2026-11-10-prepare-selection-owner-native-windows-lanes.csv)
and [GNU](2026-11-10-prepare-selection-owner-native-linux-lanes.csv) exports each
contain 440 rows and 41 columns, including missing cells and all signed
changes. Every cell is checked against the original bound report. The
[portable snapshot](2026-11-10-device-handoff-evidence.json) binds the source
qualification, terminal records, cleanup and CSV digests. It also records the
original Windows export-path defect and corrected offline helper identity;
first exports remain retained on the original device.

The [convergence plan](../plans/2026-11-03-0900-perf-assimp-convergence-plan.md)
retains the release requirements and chronological experiment dispositions:
the original fourteen cases/110 lanes on Windows and GNU, every pipeline
median at most 3x same-campaign Assimp, removal of confirmed 10x endpoint gaps,
and comparable whole-child peak memory at most 3x. The latest complete
brackets are the earlier
[ASCII campaign](2026-11-09-ascii-emission-spans-native-heap.md), which fails
engineering acceptance. The newer partial selection results also show
remaining pipeline and endpoint gaps; none clears main or release acceptance.

## What transfers through Git

The development history, public plans/reviews, source/tests, corrected native
CSV exports and compact evidence snapshot transfer with this branch. The
source can be built and developed from a clone without the old target tree.

Machine-local state does not transfer through a normal push or clone:

- `F:/SourceCodes/Rust/baozi/.git/baozi-review/performance-convergence` contains
  the original review receipts, frozen inputs, experiment-specific helpers
  and ordinary raw archives. It is not part of the pushed Git tree.
- The ignored repository `target` contains builds and other evidence;
  `repo-ref` contains ignored reference source checkouts. GNU measurement
  copies are under the original device's WSL home. None is a portable build
  prerequisite for source development.
- Five pre-existing untracked engineering notes remain unchanged on the
  original device and are excluded from this scoped commit.
- The Codex conversation, saved goal and subagent state are separate from
  Git. This document carries the project context, not those application records.

The snapshot and exports preserve conclusions and provenance, but do not
replace original traces for independent reanalysis. Historical raw evidence
requires a separate transfer from the original device. A fresh device's
campaign needs new build/host identities and complete samples; it cannot fill
old missing cells or mix native clocks from different machines. No large raw
tree, binary archive, private frozen tree or ad hoc helper framework is added
to Git for this handoff.

## Development environment and entry points

The captured host is Windows 11 / PowerShell 7, Asia/Shanghai. The user asks
that builds stay in this checkout and its existing `target`. Recent GNU
builds also use this checkout through WSL with an explicit GNU triple; WSL
home copies are measurement-only. Rust is 1.95.0, Cargo jobs one and nextest
threads one. On a fresh device dependencies need fetching before offline
commands can work. No `cargo clean` or evidence deletion is part of handoff.

After cloning or fetching the branch, these source checks run from its root:

```text
cargo +1.95.0 nextest run --locked --workspace --all-features --build-jobs 1 --test-threads 1 --no-fail-fast
cargo +1.95.0 nextest run --locked --manifest-path tools/consumers/assets/Cargo.toml --target-dir target --all-features --build-jobs 1 --test-threads 1 --no-fail-fast
```

The [asset-consumer README](../../tools/consumers/assets/README.md) owns public
workflow examples. The [comparison protocol](../testing/assimp-comparison.md)
owns corpus generation, endpoint semantics and collection boundaries; tracked
`scripts/generate_asset_workloads.py` and `scripts/run_asset_profile.py` are
the reusable entry points. The [Assimp consumer](../../tools/consumers/assimp/README.md)
pins 6.0.5 at `392a658f9c271be965271f45e7521a1b80ea4392`, with distinct
F32 and PLY-only F64 builds. Old experiment-specific scripts depend on
machine-local manifests and paths and are not standalone portable launchers.

## Pending investigation and decisions

The user permits breaking architectural/ownership/interface changes and
deleting obsolete implementations, and requires measured Assimp convergence
before release. Current permission to push is for this device handoff. The
goal remains paused until the user resumes it; this snapshot does not grant
new release or main-integration acceptance.

The next performance decision depends on complete qualified ordinary evidence
and separate heap attribution, not another unmeasured parser rewrite. An
additional source-only study of the current glTF byte/block premise closed
`NO_GO / needs measurement`; it did not authorize an implementation. Baozi's
existing `JsonScanner` owns the borrowed JSON slice and cursor. The reference
Assimp `glTF2Asset.inl` reads the full JSON into a mutable buffer and calls
RapidJSON `ParseInsitu` to build a DOM; it is not streaming semantic extraction.
The buffer/DOM source model does not establish Baozi's dominant dynamic cost.
Any later bounded scanner proposal still needs actual attribution and an
explicit physical-work, accepted-cursor and cancellation contract.

The chronological convergence plan also records rejected allowance,
single-piece and plain parser-inline experiments. Their regressions and
retirement are evidence, not candidates to revive without a new premise.
Project artifacts and commits use English; user-facing conversation uses
Chinese. Other users' changes and the five original notes remain outside
this agent's scoped edits.
