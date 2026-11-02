# Final Pre-Release Evolution Qualification

Status: integrated execution and independent final evidence review complete. This is a
local qualification record, not registry publication.
The active authority is the
[pre-release evolution plan](../plans/2026-11-02-0923-refactor-pre-release-evolution-plan.md)
and its [operation architecture continuation](../plans/2026-11-02-1700-perf-import-operation-architecture-plan.md).
All historical milestones retain their own tested source identities.

## Source and Scope

The final qualification source is `088d7176d5982d1ae9e5045c0250dc6f608f66c1`. Its production
runtime is `52446ec8511b7dfba2e5352ce6941558d85e0ae2`; the later collector change only binds
build manifests to binary bytes. `target/review/u14-final-source.json` records the source
tree IDs. Each phase checks a committed, stable runtime/tool/consumer/fuzz/script closure
and unchanged lockfiles. Documentation drafts and the two target-cache ignore entries below
are outside that executed source closure.

The changes include typed per-format options and borrowed execution contexts, private scene
publication, local work/shared storage ownership, actual physical completion prefixes, bounded
parser control, OBJ source remaps and stable normal ordering. Earlier material/light/quantization,
deformation/renderer and canonical-v1 product units are part of the same integrated closure.
This qualification neither promotes native targets nor authorizes registry operations.

## Observed Final Checks

| Scope | Actual result |
| --- | --- |
| Windows Rust 1.95 workspace all features | 3,278/3,278; six existing manual skips; nextest `150893c9-a4bd-4e7c-9412-1fcaa7786eb9` |
| Windows workspace all-target/all-feature strict Clippy | Passed with `-D warnings` |
| Windows workspace formatting | Passed |
| Windows workspace doctests | 63/63 passed |
| Windows workspace public docs | Passed with `RUSTDOCFLAGS=-D warnings` |
| Independent assets | 120/120; one existing manual skip; `1c83be5e-d0c5-4b1d-a271-b57cd51b66e0` |
| Direct glTF consumer | 8/8; one existing manual skip; `01061968-1fbd-41e1-8ae1-13c64b46490c` |
| Renderer CPU | 37/37; 25 separate manual GPU tests not executed in this row; `4c7996ff-2e08-49f3-b49a-eab482b67add` |
| Independent consumer strict Clippy and formatting | Assets, direct glTF and renderer all passed |
| Deterministic fuzz seeds | 16/16; `3546b8da-749b-482c-90a9-84df21c5f51d`; strict Clippy/fmt passed |
| Feature/dependency graph and source consumers | Twelve feature cases executed; crate graph, runtime admission, eleven native dependency profiles and portable normal/build tree inspection passed |
| Windows unchanged package consumers | Rust 1.95.0/debug: all twelve facade cases, direct glTF 8/8 with one manual skip, native baseline executed; optional capture/helper compilation passed; fresh outer phase passed |
| Linux unchanged package consumers | Rust 1.97.1/release: all twelve facade cases, direct glTF 8/8 with one manual skip, native baseline executed; optional capture/helper compilation passed |
| Linux Rust 1.95 workspace | 3,278/3,278; six existing manual skips; `e518f9e9-23c9-4974-9ba0-86b79acf03d6` |
| Linux independent assets | 120/120; one existing manual skip; `ab83e88d-1eca-4b1a-8903-b4160d66c56e` |
| Linux direct glTF consumer | 8/8; one existing manual skip; `8d1c80f6-4354-4f24-a691-1f1da80357b9` |
| Linux renderer CPU | 37/37; 25 separate manual GPU tests not executed in this row; `f6a6fb88-176f-4e4a-b067-96cb44ce02ba` |
| Linux deterministic fuzz seeds | 16/16; `abf8ce52-a9eb-4dc0-83e1-4c21750c0ca3` |
| Linux workspace doctests | 63/63 passed |
| Browser compile | Empty/all source consumer compiled for wasm32-unknown-unknown; no browser runtime claim |
| WASI | Empty/all consumer executed under Wasmtime 50.0.0; 58 snapshot and three glTF issue tests passed |
| Named Vulkan readback | 25/25; RTX 4090, vendor 4318/device 9860, NVIDIA 621.01; `b93339af-0b71-4c30-93d4-2b08e0816932` |
| Linux scoped AddressSanitizer tests | 242/242; two existing manual diagnostics ignored; instrumented symbols checked; capture disposition below |
| Bounded fuzz smoke/campaign/resume | Both targets pass; six stages, zero failing artifacts; exact observations below |

The command receipts and exact run IDs are written under `target/review/u14/`. Prepared or
pending commands are not passing evidence. Manual test exclusions, target compilation and
runtime execution retain distinct claims.

Linux host execution was resumed only for unfinished rows after offline metadata preflight
failures. Sixty-five missing `.crate` archives were copied from the existing Windows cache
with exact lockfile checksum checks. Five missing index paths were filled; a subsequent
full-version preflight verified all 239 locked registry versions and refreshed eleven stale
entries from supersets, preserving prior bytes. No source, lockfile or dependency version was
changed. Failed metadata attempts remain retained; successful workspace/assets/direct glTF
rows are reused only with the identical HEAD, locks and stable source closure.

The initial Windows archive command succeeded on all twelve facade cases, direct glTF and
native baseline, with the optional compile-only checks. Its outer stability guard failed:
Cargo created `tools/consumers/gltf-conformance/target/.rustc_info.json`, the sole unexpected
untracked path. Creation time/content and the exact status output are retained. The repository's
existing explicit target-ignore list omitted the two newer consumer workspaces; `.gitignore`
adds those two directories. No production source or lockfile changes. The successful command
inside that failed qualification receipt is not relabelled as a passing phase. A fresh complete
Windows archive qualification then passed after the Linux phase; both Windows staging
directories remain retained.

Python checks are separately source-bound: the package verifier's ten tests pass on its
unchanged closure; the final collector has nineteen passing tests and one Windows platform
skip. The new collector cases prevent stale or identity-free supplied build manifests from
launching a worker. They do not replace archive, native or renderer execution.

## Sanitizer Harness Disposition

The first scoped normal test execution under ASan passes 55 cases, skips two existing manual
diagnostics and fails two allocation-balance assertions after caught panics: `count_current`
is zero but `bytes_current` is 212/217. The complete failed log remains retained. This is
investigated before any rerun or qualification claim.

A task-local control imports only `allocation-counter 0.8.1` and standard-library
`catch_unwind`. Its captured run fails with the same zero-count/nonzero-byte pattern under
both Rust 1.95 and the ASan nightly. Both uncaptured runs pass; ordinary Vec grow/drop balances
in every run. These controls isolate the capture-dependent counter imbalance from Baozi code
and show that it also occurs without ASan, consistent with growth of libtest's retained
output-capture buffer. A counter window can observe a replacement allocation and
the release of a buffer created before the window, leaving a byte delta with no count delta.

The complete original normal selection then passes **57/57**, with the same two manual
diagnostics excluded, after adding only `--nocapture` to its argv. Source, compiler flags,
ASan options, filters and every owner/allocation assertion are unchanged. No panic case is
skipped and no allocation is opted out. This matches the uncaptured libtest execution used by
nextest. `target/review/u14/panic-capture-diagnostic/` retains the four controls;
`asan/` retains the failed attempt and `asan-resume/` retains the corrected execution with
byte-checked reuse of the successful preceding rows. Full sanitizer/fuzz completion is still
reported separately in the table above; this disposition addresses those two assertions only.
Independent read-only review confirms identical test sets, unchanged assertions, the sole argv
addition, matching source/locks/log bytes and the four-control result. Its causal-wording
suggestion is applied here; no P1/P2 finding remains in this bounded disposition.

The eight sanitizer selections contain 126 glTF malformed/feature tests, nineteen real parser
controls, six OBJ remap tests, eight comparison tests, five text controls, nine public facade
controls, 57 normal tests and twelve precision tests. The installed nightly is Rust
`1.101.0-nightly`, compiler `92ffebeceaa0a071066f233873c2438e73d70fe7` (2026-10-28), LLVM 23.1.4.
`-Zsanitizer=address` and `ASAN_OPTIONS=detect_leaks=1:halt_on_error=1` are explicit. Retained
`nm` output verifies `__asan_init` and `__asan_report_load1` in the representative malformed
glTF binary and the executed control/normal binaries. This finite selection is not exhaustive
memory-safety proof or instrumentation of every platform/standard-library path.

Both libFuzzer targets use cargo-fuzz 0.13.2 with AddressSanitizer, one build job and a
separate copy of the unchanged seed corpus. Each stage uses a recorded seed, ten-second
per-input timeout and 2,048 MiB RSS limit. Maximum inputs are 65,536 bytes for `gltf_import`
and 8,196 bytes for `edit_lifecycle`.

| Target | Smoke | Exploration | Saved-corpus resume | Failing artifacts |
| --- | ---: | ---: | ---: | ---: |
| `gltf_import` | 256 runs | 29,184 runs / 123 seconds | 5,700 runs / 32 seconds | 0 |
| `edit_lifecycle` | 256 runs | 30,000 runs / 122 seconds | 6,494 runs / 31 seconds | 0 |

Exploration/resume arguments are `-max_total_time=120` and `-max_total_time=30`; the table
retains actual libFuzzer completion observations, including overrun. Original and saved corpus
inventories, target hashes, sanitizer symbols, stderr and empty failing-artifact inventories
are retained. This is bounded exploration and resume evidence, not sustained fuzz coverage.

## Unchanged Archive Consumers

Each package phase creates fourteen maintained archives: the ten-package portable release unit,
three maintained cap helpers and the unregistered native candidate. This extra closure check
does not add the latter four to the portable release unit. Cargo creates normalized archive
manifests without generated package lockfiles; the verifier preserves those archive manifests.
Independent roots use exact version dependencies, a fresh Cargo home/build tree and standard
directory-source replacement. Consumer-root patches, replacement tables and checkout resolution
are rejected. Only copied consumer locks transition to registry identities; builds/runs then
use `--locked --offline`.

The facade matrix is `empty`, `default`, `default-formats`, `obj`, `ply`, `stl`, `gltf`,
`postprocess`, `all-formats`, `tracing`, `mint` and `all`. Direct single-format execution adds
eight glTF tests with one existing manual exclusion. Native baseline executes scoped file reads
and STL import. Repository-capture and the separate helper all-features consumer compile only;
no repository traversal, native promotion or every-helper-feature runtime claim follows.

Linux completes on installed stable Rust 1.97.1, compiler
`8bab26f4f68e0e26f0bb7960be334d5b520ea452`, LLVM 22.1.6, with release optimization. Its
retained staging is `target/package-check-it63z8cr/`. Windows completes on Rust 1.95.0/debug in
`target/package-check-oobenpac/`, including the fresh outer stability check. Each retains
fourteen maintained and 132 third-party registry archives. All fourteen maintained archive sets
have identical file-member names and bytes across the hosts, including normalized manifests;
container identities and filesystem modes differ and retain their separate digests. No claim
of byte-identical compressed archives follows. This is a local offline resolution/execution
rehearsal, not a live registry test.

## Final Evidence

`target/review/u14-final-evidence.zip` contains 1,763 source/log/artifact/review members, each
read back and checked against its manifest. Its size is 23,971,311 bytes and SHA-256 is
`95acd86abc07579528474dfc03b09dc5ec53bf42d7eb87525266e000e1819ade`.
`target/review/u14-final-evidence-verification.json` records the complete member check.
The separate Windows/Linux performance archives retain their earlier verified identities.
The read-only final artifact review is retained separately in
`target/review/u14-final-evidence-review.md`, after the ZIP was sealed.

The ZIP includes the immutable source archive/inventory, successful and failed phase receipts,
sanitizer symbols, copied seed/saved corpora, parser-capture diagnostic, exact maintained package
archives, version-only consumer sources/locks and third-party archive identities. Complete
directory-source and build staging remain at the paths above; the ZIP does not duplicate every
third-party cache/build file. `target/review/u14/final-execution.json` binds completed/reused
rows, sanitizer counts and both package sets to the executed source. Initial failed attempts
remain failed. The final packaging guard success is from a fresh full execution.

## Performance and Review Boundaries

The [complete Windows result](2026-11-03-windows-final-comparison.md) and corrected
[Linux B1/heap result](2026-11-03-linux-b1-and-heap-comparison.md) each retain 110 ordinary
lanes, 2,200 retained pairs and 330 warmup pairs. Linux separately retains twelve heap
observations, six final Ir profiles and bounded exit-map diagnostics. Invalid initial Linux
control identities remain explicitly disqualified and preserved.

All Linux stage-time medians fit the frozen B0 limits. Six original parent-elapsed breaches
do not recur in same-lane repeat medians. Fifty-seven RSS breaches remain above their original
limits and receive explicit reviewed cost acceptance, with limited resident-image attribution.
Neither the original budget JSON nor raw observations are rewritten as all passing.
No general Assimp parity, speed/memory superiority or isolated-stage heap claim follows.

Independent milestone source reviews close the typed-context/control/remap/normal changes.
The final P5/P6 integration review checks their cross-module ownership, admission, error,
precision, provenance and old-scene/plan-reuse seams with no P1/P2 finding. Measurement and
budget reviewers independently verify the retained identities/data and close their wording
corrections. Final integrated execution now closes the scheduled runtime obligations. The
final independent review also checks both completed package phases, version-only roots,
unchanged source/locks/logs, archive member identities and the complete 1,763-member ZIP. It
finds no P1/P2 issue or remaining execution/archive obligation. The recorded review is read-only;
the local documentation/ignore commit is separate from the tested
runtime revision. No registry action or target promotion follows.
