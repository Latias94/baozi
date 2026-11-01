---
status: completed
date: 2026-11-01
---

# Maintained native dependency closure: execution review

This record covers the [dependency closure plan](../plans/2026-11-01-009-refactor-native-dependency-closure-plan.md)
after `625d339972f68bea81293469ec3d1aa6a63ff883`. It implements a bounded part of the
separate release track in the [asset workflow plan](../plans/2026-11-01-001-feat-asset-workflow-expansion-plan.md).
Implementation is `e7e9b00247eb3000e9db54ef5315fabd7ec9479b`. The tracked source tree
was clean after that commit; subsequent documentation changes bind the already
executed source qualification to this revision and complete the plan.
Final Windows source fingerprints, logs and archive inventories are retained locally
under `.git/baozi-review/native-dependency-closure/`. Each package rehearsal creates
its archive/source/build staging under `target/package-check-*`; that build cache
is disposable and is not the durable execution record.

## Dependency ownership and distribution behavior

The old asset consumer needed its own root patch because the registry releases
of `cap-std` and `cap-fs-ext` still selected the upstream `cap-primitives` identity.
A patch inside the native dependency's workspace cannot control a downstream
workspace. The fix moves that responsibility into the packages that own the
edges: `baozi-cap-primitives`, `baozi-cap-std` and `baozi-cap-fs-ext`, all 0.1.0,
with unchanged library aliases and relative path-plus-exact-version dependencies.
Both root patch tables are removed. The source native candidate and independent
asset consumer resolve and build normally without a patch.

The upper two packages retain their complete upstream 4.0.3 production Rust
source and build scripts. Their original 35-file and 20-file archives were
checksum-verified and copied byte-for-byte before manifest/README maintenance.
The existing bounded manual-link primitives implementation is preserved. Each
package retains copyrights, all license choices and an explicit `PATCHES.md`;
upstream digests identify original archives rather than modified Baozi sources.
Historical upstream `.orig`, VCS metadata and bundled locks remain in the
checkout. Reserved metadata names are excluded from archive input; Cargo creates
the archive's `.orig` from the active maintained manifest.

All four native packages keep `publish = false`. The native runtime candidate
set remains Linux x86_64 GNU, Windows x86_64 MSVC and Apple ARM64; FreeBSD remains
dependency-audit-only. Release registration remains empty. Repository capture
retains its independent Unix operational and Windows fail-closed boundaries.

## Verification design

The package checker now creates actual `.crate` archives and leaves their
normalized dependency manifests unchanged. Independent consumers use exact
registry versions with no path, patch or replace tables. Standard Cargo directory
sources supply the unpublished artifacts for a local registry rehearsal, with a
fresh staging `CARGO_HOME`. External registry packages come from original cached
archives whose SHA-256 matches the source lock, including when mutable registry
source directories have been modified. A run-owned archive copy is verified and
extracted so replacement of the shared cache between reads cannot change the
accepted source. Each maintained package must resolve to
this run's exact extracted directory with a registry identity; checkout or mixed
source substitution fails.

The staging scene lock transitions once from local to registry identities;
subsequent checks, builds and runs stay `--locked --offline`. Source locks are
unchanged by the checker. Sixteen maintained packages comprise the native-enabled
archive set. The fixed twelve-case scene matrix, a separate scoped native STL
import, optional repository compilation, and separate helper-feature compilation
exercise different contracts. Helper features (`arbitrary`, `fs_utf8`,
`arf_strings`) are resolved in copied workspaces; they do not widen native
admission. Archive creation uses `--no-verify --exclude-lockfile`, followed by
real independent compilation and execution rather than treating packaging as a
build result. Both final outputs and intermediate build files use explicit fresh
staging directories, and CI passes the exact matrix target.

The dependency checker audits all three maintained identities, exact paths,
versions, aliases and child edges, with distinct upstream provenance. Runtime
baseline schema is 6 and repository delta schema is 5. It continues to reject
upstream substitution, duplicates, incorrect features/targets and both upstream
and maintained native identities leaking into portable or xtask graphs.

This local rehearsal proves source/package closure with a supplied registry
snapshot. It does not prove name ownership or availability on crates.io, replace
the exact-tag release procedure, publish anything or promote a native target.
Cargo's [multiple-location dependency contract](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#multiple-locations)
and [directory-source format](https://doc.rust-lang.org/cargo/reference/source-replacement.html#directory-sources)
are the primary tooling references. No custom registry server or Rust source
interpreter was added.

## Executed results

Rust checks use 1.95.0, one build job and serialized Cargo invocations. Windows
uses the shared repository target for source checks; each package rehearsal
uses a fresh artifact build directory. Native Codex agents own the disjoint audit,
package checker and ownership-documentation changes plus an independent integration
review. The main thread owns Cargo and source qualification.

| Check | Executed result | Identity |
| --- | --- | --- |
| Windows native default | 20/20 pass, no skips | `162fee2f-d6b0-457a-bfe9-2f1a46fcf733` |
| Windows native repository feature | 30/30 pass, no skips | `9ec30f5b-e47b-45dd-82ef-ac2689e8201c` |
| Windows bounded primitives | 21/21 pass, no skips | `37beefd9-c28a-44bf-b9bf-5ec9caea607b` |
| Linux native default | 21/21 pass, no skips | `340c6d06-aded-4e20-aa70-f7cf2dd14d1d` |
| Linux native repository feature | 39/39 pass, no skips | `f985ce28-790d-4380-a28b-9fff9f25fee7` |
| Linux bounded primitives | 18/18 pass, no skips | `428366b6-497b-46b2-8d1e-0ef3abcd91c2` |
| Windows independent asset consumer, all features | 43/43 pass, one existing manual skip | `34f16bd8-3286-4ca6-9cf6-8425b41a63f1` |
| Dependency audit regressions, final source recheck | 40/40 pass, no skips | `8c75a177-9107-46fe-a2cb-8ce3f2bfc465` |
| Actual dependency admission, final source recheck | All 11 target-filtered profiles pass | `native-admission.log` |
| Linux archived consumers, final checker | 16 archives; 12/12 scene cases and native STL execute; repository/helper features compile | `package-check-e61ki1gi`, 1,697.858 s |
| Windows archived consumers, final checker | 16 archives; 12/12 scene cases and native STL execute; repository/helper features compile | `package-check-xnvozuhw`, 374.438 s |
| Strict Clippy | Candidate, independent assets and xtask pass, all targets/features; xtask rechecked after interruption | `audit-clippy.log` and earlier main-thread results |
| Formatting, final source recheck | Root, candidate, assets and all three maintained helpers pass | `source-final-results.json` |
| Python archive-boundary regressions | 10/10 pass | `scripts/test_check_packages.py` |
| Workflow syntax | Pass | actionlint 1.7.12 |

The final locked metadata recheck also retained the complete resolved graphs for
the scene consumer (50 packages), native consumer (65 packages), and helper
consumer (49 packages). All non-root packages have registry identities and resolve
to this run's directory source; maintained packages resolve to their exact
extracted archive directories. Source fingerprints and the staged tree identity
were checked before and after execution. The final Windows compiler is Rust
1.95.0 (`59807616e`, LLVM 22.1.2) on `x86_64-pc-windows-msvc`, with Python 3.13.11.

The independent integration review found two package-checker boundary defects:
shared-cache replacement between digest/extraction, and inherited intermediate
build-directory/target configuration. The final implementation verifies and
extracts a private archive copy, fixes both build directories and passes CI's
exact target explicitly. A regression replaces the shared archive immediately
after hashing; extracted source still matches the locked original. The reviewer
rechecked and closed both findings with no further P1/P2 in this bounded review.
An independent native-audit review found no confirmed P1/P2 in the migrated
contract and checked both frozen graphs against their preceding versions:
apart from maintained identities and removal of the old repository
`cap-std/default` increment, the target graphs retain their prior structure.
These reviews are static checks, separate from the executed Cargo results.

Supplementary local native/assets policy checks pass with installed cargo-deny
0.19.9 and its cached advisory database (`--disable-fetch`). The configured CI
version remains 0.20.2; the older local result is not a claim of that CI gate.

The Linux run used the unprivileged Ubuntu-26.04 WSL environment on
`x86_64-unknown-linux-gnu`. WSL's mirrored-network setup failed, so the run used
offline Cargo. Five missing original registry archives were copied from the
Windows cache after verification against the source locks, together with three
missing index records. The 28-minute rehearsal includes extracting the complete
cross-platform registry snapshot on the Windows-mounted filesystem; it is no
performance observation about the importer. Linux was executed locally under
WSL, not on a hosted Linux runner.

The initial Windows rehearsal also passed all twelve scene executions, native
STL execution and repository/helper compilation (`package-check-ygjzj5pl`). The
first attempt to repeat Windows with the review fixes was interrupted; its
completion could not be established after the build cache was cleared. That cache
cleanup also removed the earlier raw local logs and archive directories. The
completed earlier runs above remain identified by their observed nextest IDs and
session-recorded results; those raw files are no longer claimed to exist locally.
The final Windows rerun passed with the same reviewed implementation, an explicit
MSVC target and deliberately conflicting inherited target/build-directory settings.
Its driver exited zero and compiled the actual MSVC executables in fresh staging
directories despite inherited `CARGO_BUILD_TARGET=wasm32-unknown-unknown`. The
additional wrapper initially required the inherited build directory to be entirely
absent; Cargo packaging legitimately put temporary `.crate` files there. Corrected
inspection confirmed that it contains only those archives and `CACHEDIR.TAG`, with
no compiler artifacts. The wrapper's overly broad existence assertion is not a
failed package/consumer gate. The final qualification record preserves both this
diagnostic correction and the successful driver result outside the build cache.

## Release boundary and next work

Linux debug/release package-consumer CI is now configured alongside Windows and
Apple ARM64, reusing the same script and semantic consumer. Workflow wiring is
not hosted execution evidence. Current-stable/release and Apple execution for this
revision remain distinct from local MSRV/debug results.

The next release unit still needs an explicit format/package-set decision,
current target/profile qualification, registry name/ownership checks and the real
dependency-order publication procedure. Optional facade dependencies remain a
release-manifest obligation even when a consumer disables their features. Native
publication and target registration remain independent of portable publication.
