# Canonical Output and Proof in the Lexical Host

This isolated implementation moves actual canonical output and proof into their caller's host.
It continues the [lexical invocation plan](../plans/2026-11-03-1955-refactor-lexical-invocation-host-plan.md),
without completing its recovery, terminal-driver, numeric-backing or attachment work. The Rust
changes remain in the isolated compiler workspace; no new performance result or release
readiness follows from this checkpoint.

`CanonicalInvocationHost` owns actual `Option<T>` and `Option<Proof>` beside the physical store
root. Activated, candidate and freeze permissions borrow those slots. Forgetting a permission
leaves its actual values in the host. Ordinary permission cleanup and host cleanup both retain
the existing candidate proof-before-output and frozen output-before-proof order. A destructor
failure stops ordinary work immediately; all remaining values are disposed before physical
containment. Cleanup during body unwind preserves the original panic.

Both import entries construct the host outside `ImportSession`. The existing accounted session
cell holds short slot permissions. Both postprocess entries and the core construction/edit
fixtures forward matching slots. Parsing, normal mathematics, work weights, limits and benchmark
tasks/clocks are unchanged. The three qualified glTF FLOAT-direction files retain their exact
checkpoint-6 hashes.

## Root Extraction Found and Repaired

Independent review found that the initial `split()` returned `&mut StoreInvocationRoot`. After
forgetting an activated permission, a caller could split again and take/drop that root before
the actual output. A real Windows regression probe reproduced the defect: nextest run
`bfae12e6-d1d9-4344-86d4-d6fcbac413fd` observed no live physical allocation during output Drop.
That failing history remains retained.

The repaired split lends an opaque `StoreInvocationStart`. It exposes no root reference,
extractor, `DerefMut` or `Default`. Replacing a token moves only a loan. Startup still uses the
same checked operation path; the core binder preserves its original source/account choice.
Actual extracted compiler probes establish valid startup and exactly `E0277`/`E0308` for root
take/replacement attempts. A runtime test confirms that dropping a reentry token leaves the
real storage live until output cleanup. Independent repair review closes the original P1.

## Source and Actual Checks

The final formatted snapshot contains **1,405 files**, with **50 changed/new paths** relative
to checkpoint 6 source3. Its only new source file is `activation/host.rs`.

| Identity | SHA-256 |
| --- | --- |
| Source set | `95e988404318ba0e50dff994f287b412adc34021544254dc96778aba1adb0877` |
| `canonical-payload-1-formatted-source-2.json` manifest | `2b4246465bb54bd9f816e34305d1a53fbea1b3fe43c3aa952f8f47c7b79d2ed1` |

Source-set encoding is sorted UTF-8 relative slash path, NUL, lowercase file SHA-256 and LF.
The complete immutable snapshot is retained at
`.git/baozi-review/performance-convergence/canonical-payload-1-formatted-2-frozen`.
Checks run on Windows with Rust 1.95.0, locked/offline dependencies, one build job and one
nextest thread, reusing the root target directory.

| Actual check | Result |
| --- | --- |
| Workspace, all targets/features, compiler | PASS, zero warnings. |
| Complete workspace nextest | **3,426 passed**, 6 existing skips; run `b73ef5f0-0773-482b-8c35-0001565d70d5`. |
| Workspace strict Clippy, all targets/features | PASS with `-D warnings`. |
| Workspace rustdoc, all features | **102 passed**: core 6, import 34, operation 58, postprocess 4. |
| Standalone asset consumer nextest | **138 passed**, 1 existing manual skip; run `5964161d-d1a7-4d99-b523-d31b26c785a7`. |
| Standalone asset consumer strict Clippy, all targets/features | PASS with `-D warnings`. |
| Workspace and consumer `cargo fmt --check` | Both PASS. |

`canonical-payload-1-final-qa-1.json` records unchanged hashes for every one of the 1,405 files
before and after the complete runtime/lint checks. `canonical-payload-1-final-qa-2.json` does the
same for rustdoc, consumer and format checks, with every command passing. Earlier 548/553-test private runs and
1,451-test caller runs are intermediate evidence, not substitutes for the complete final run.

The thirteen new host tests cover actual forgotten activated/candidate/freeze permissions,
both disposal orders, individual and combined destructor panics, original body-unwind panic,
cross-root output/proof substitution and reentry-token cleanup. Independent caller review
checks all 763 affected public-fixture assertions and 185 literal test names without changed
limits, ignored tests or newly forgotten permissions used to bypass cleanup. Exact review
scope and hashes are retained in `canonical-payload-1-operation-review-2.md`,
`canonical-payload-1-final-caller-review-2.md` and `canonical-payload-1-postprocess-review-1.md`.

## Continuing Work

Actual rejection/error and recovery values still need outer ownership. The next code change
connects the real central pending-fallback cause, followed by distinct activation, validation
and generation causes. The existing client-callable final preparation/joint-detach interfaces
must then move behind the concrete outer driver, after callback/capture and scratch disposal.
Borrowed numeric backing, current-state attachment, storage promotion and retired-path deletion
remain required. The unchanged 14-case/110-lane Windows/Linux performance gates remain open.
