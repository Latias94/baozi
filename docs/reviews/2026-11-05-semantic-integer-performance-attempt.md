# Semantic and Integer Validation: Finite Measurement Attempt

The candidate is correctness-qualified and builds as an ordinary worker on both
Windows and Linux. Its ordinary timing effect remains unmeasured: Windows stops
before the first candidate campaign, and Linux stops before its first ordinary
campaign because the fixed startup quiet condition is not met. There is no
candidate speedup, regression, memory improvement or release-acceptance result.

## Source and Ordinary Builds

The immutable candidate is `e5ca93b82f2041e82e6bbaa6913462be551bd69f`.
The control is the previously measured
`56eecd985693cba1826326169e6c673a7b36e3ff`. The candidate combines the
[Copy semantic/OBJ lexical changes](2026-11-05-semantic-copy-and-obj-lexical.md)
and [integer predicate removal](2026-11-05-integer-attribute-validation.md).
A later comparison would measure their cumulative effect, not isolate either
change's contribution.

The candidate passes the seven serial correctness gates recorded in the integer
review: 3,593 workspace runtime tests, 188 documentation tests, 138 external
consumer tests, both strict Clippy checks and both format checks, with existing
skips. All 908 recorded Rust/TOML inputs remain stable across qualification.

Windows builds the committed source with Rust 1.95, release opt3/debug0, generic
x86_64 MSVC, System allocation, dynamic CRT and no allocation instrumentation.
Its source archive is then extracted for the GNU build, which uses the existing
Linux target directory, one build job, release opt3/debug0, generic x86_64 GNU
and System allocation. Both build processes terminate successfully; each records
1,470 identical before/after source inputs.

| Worker | Retained build directory | Binary SHA-256 |
| --- | --- | --- |
| Windows candidate | `target/assimp-comparison/convergence-windows-semantic-integer-build` | `5ba73e83b353e92fc413f74c977c4a6eafaa9ab21d3bbe2be65867dc3e3c1274` |
| GNU candidate | `target/assimp-comparison/convergence-linux-semantic-integer-build` | `355668f0b824722ea56735d0a72c9cfd619afa31a62a2620c4cbd3e75cce4ca1` |

Each directory retains `build.json`, build logs and its immutable binary. The
Windows directory also owns the source archive. The prospective Linux decision's
free-text preparation sentence mistakenly says 1,464 source inputs; the actual
build manifests both contain 1,470. That wording error does not change their file
maps or identities. The original decision and its recorded hashes are retained.

## Original Functional Preflights

Both source versions pass every original lane on each platform: four separate
110-lane preflights, each with zero warmups and one retained pair. All 888 commands
succeed, including eight source metadata commands and 880 geometry observations.
These are functional results, not finite timing qualification.

Linux places both Baozi workers, both immutable pinned Assimp workers, the
byte-identical fourteen-case corpus and child artifacts on the Linux filesystem.
Completed artifacts are copied to retained Windows evidence after collection;
those copies are outside worker clocks. This follows the prior
[filesystem diagnosis](2026-11-05-linux-filesystem-convergence.md).

## Ordinary Sequence Outcomes

Both sequences are prospectively fixed as C0/N0/N1/C1, using the original fourteen
cases and 110 lanes, three warmup pairs and twenty retained AB/BA pairs per lane.
The startup rule requires twenty seconds without sampled named build/profiler
tools within a 180-second wait. Approximately two-second observations continue
during each campaign. Linux also requires a fresh Windows snapshot. Absence of
those named tools would not prove that all host load is absent.

| Sequence | Actual outcome | Evidence limit |
| --- | --- | --- |
| Windows C0, old source | Completes 110 lanes and 5,062 commands with no failed observations; 2,200 retained pairs plus 330 warmup pairs | 151 snapshots contain other Cargo, rustc, MSVC compiler or linker processes. Keep the result descriptive. |
| Windows N0, candidate | Does not start within the fixed wait limit | No candidate ordinary samples; N1/C1 are not run. |
| Linux C0 | Does not start within the fixed wait limit | No ordinary Linux campaign runs; the wrapper records child exit 2. |

The sequences stop at their prescribed failure frontier. No samples are trimmed,
no quiet condition is relaxed, and no outcome-selected repeat replaces the
retained attempt. Functional preflight durations and the loaded old-source C0
cannot establish the candidate's source-change effect.

## Retained Evidence

The decision, original process observations and review receipts live under
`.git/baozi-review/performance-convergence/` and `target/review/convergence/`.
Public source conclusions are recorded here so an unpublished local timing
attempt cannot be mistaken for a passing benchmark.

| Receipt under `target/review/convergence/` | SHA-256 |
| --- | --- |
| `semantic-integer-C0-command-1.json` | `79c5f019dce36586054452789032b4e4c5c3456fb7d28f74806f385e1be753ea` |
| `semantic-integer-N0-not-started-1.json` | `8a0ee4fdfdb8a9b58320ae46777a52d54b3c01e4002688fa4e8198629b7fc42c` |
| `semantic-integer-linux-nativefs-C0-not-started-1.json` | `734ee2900a8f0d7f676b71059e79d2e7615488998ff52696f020367afd7aa2c1` |
| `semantic-integer-linux-nativefs-wrapper-1.json` | `c982eadac8c689088248d69a01e777b093f7ed5c4ad52851ed5d46eae6dacf67` |

Independent final evidence review verifies the QA/build/archive identities, all
5,940 raw geometry observations across the four preflights and Windows C0, original
pair ordering, load records and both stopped frontiers. Its sole requested wording
correction identifies the eight metadata commands as Git revision/status queries;
that correction is applied here. The private receipt is
`semantic-integer-finite-attempt-evidence-review-1.md`. No unresolved material
evidence discrepancy remains; no candidate ordinary performance result follows.
The receipt's SHA-256 is
`e3bcf1b38f385d06f2490f3e313e8e86dd423136ef67578a9ca0b6e1f8f32bc9`.

## Next Measured Work

The original Windows/Linux release gates remain open. A future prospectively
defined collection needs an adequate host window and the same immutable source,
tasks, workers, corpus and artifact policy. Retain this attempt independently.
Fresh ordinary measurement and remaining same-source attribution are still
needed before treating a further floating-validation or parser representation
change as the next measured keep. The private finite-geometry construction
recommendation remains an unimplemented design direction.
