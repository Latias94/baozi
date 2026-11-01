# Release and Publish Policy

Baozi is currently foundation-first. Breaking changes are allowed when they protect the long-term
IR, parser security model, or public API boundary.

H in the [delivery plan](../plans/2026-10-29-001-refactor-library-delivery-plan.md#h--update-dependencies-and-prove-first-publication)
qualifies packaged consumers and executed target/profile behavior. It does not require implementing
a registry publication workflow, registry-state validator, or signing/SBOM/attestation framework.
Local artifact qualification does not authorize a registry upload. Any actual upload must satisfy
[ADR 0006](../adr/0006-public-api-versioning-and-crate-stability-policy.md)'s publication and recovery
requirements and the protections below.

## Registry State

The crates.io registry already contains `baozi 0.0.1`, published on 2026-07-09 with placeholder
`hello world` metadata. It is not the canonical architecture release and its version must never be
overwritten or reused. Before the canonical train, release owners record that placeholder's
noncanonical status and current yank state. Publish and verify the complete canonical train,
including a fresh registry-only consumer that downloads, builds and runs the facade, before
yanking `baozi 0.0.1`. Verify the live yank afterward. A partial train leaves retirement pending;
a failed yank is retried as a separate retirement step and does not undo the verified release.
Yanking is not deletion or rollback. The current workspace train is `0.1.0`.

[The exact-version evidence record](../research/crates-io-baozi-0.0.1-registry-evidence.md) and its
[normalized capture manifest](../research/crates-io-baozi-0.0.1-registry-evidence.toml) fix one
time-point API observation and the immutable version/archive identity. The raw API response includes
mutable fields, so release automation must never compare its live byte length or digest with the
capture. It compares the live immutable tuple separately and checks yank status, current owners, and
authentication configuration as mutable release-time state.

Before a future registry publication, release owners must provide `docs/release/registry-state.toml`
and its fail-closed validator as required by ADR 0006. Registry uploads remain disabled until both
exist; they are not prerequisites for H's local artifact qualification. The manifest is the sole
expected-state authority and binds the release tag, full checkout SHA, registry, version,
complete release-unit package set, expected package presence,
registry-stable owner kind/IDs, credential route, maximum token TTL, and complete approved Trusted
Publisher configuration set. The workflow must exhaust pagination and reject a missing, extra,
duplicate, unknown, stale, or display-name-only identity; live registry state never supplies its own
expected value.

Before publishing the first crate, one fail-closed preflight queries every release-unit package name
and its current owners. Every name must be either absent as expected or present with the exact
approved owner set. An unexpected occupation, owner drift, registry timeout, or partial response
stops the train before any immutable package upload; name checks are not deferred until each crate's
turn.

That preflight is a state observation, not a namespace reservation. Immediately before each upload,
automation rechecks the exact package name and owner set. A late occupation or owner change stops
before that upload; any packages already published are an explicit partial train and follow the
recovery policy in ADR 0006. crates.io provides no transaction that can guarantee zero publications
after a successful initial preflight.

The first publication of each supporting crate name that is absent from crates.io uses an approved
API-token path. API tokens are short-lived, carry only the crates.io endpoint scope required by that
job (`publish-new`, `publish-update`, or `yank`), and are restricted to the exact crate names used by
the job. Wildcard crate scopes, unrelated endpoint scopes, and no-expiry tokens are forbidden. A
token has a manifest-declared maximum TTL no greater than 24 hours. An `always()` cleanup attempts
immediate revocation and verifies the result, but runner cancellation, process loss, or registry
outage is not reported as successful cleanup. Unverified revocation blocks every later publication,
alerts the release owner, and requires confirmed manual revocation or expiry before a new credential
is used. The existing `baozi` name may use crates.io Trusted Publishing/OIDC only after exhaustive
enumeration proves the complete configured publisher set exactly matches the manifest and an exchange
verifies the selected repository owner, repository, workflow, and environment. Crate owner and
repository owner are distinct identities. If full enumeration is unavailable, all publication is
blocked unless an independently protected, equally current complete snapshot can be exhaustively
compared; changing to an API token does not remove unknown OIDC authority. An exact-crate
`publish-update` fallback is allowed only after complete-set equality succeeds, a typed service-
availability failure occurs before token issuance, and the protected release environment records an
approval. A claim, binding, configuration, or authentication rejection always stops the train. No
token is stored in the repository or printed in logs. OIDC, provenance attestation, SBOM, signatures,
and reproducible-build claims are separate release evidence axes, not implied by a successful upload.

## Intended Release Unit and Current Manifest Eligibility

Crates intended for eventual publication:

- `baozi`
- `baozi-operation`
- `baozi-core`
- `baozi-io`
- `baozi-import`
- `baozi-postprocess`
- `baozi-format-stl`
- `baozi-format-obj`
- `baozi-format-ply`
- `baozi-format-gltf` and its maintained `baozi-gltf` / `baozi-gltf-json` dependencies

The [2026-11-01 first-release audit](../reviews/2026-11-01-first-release-usability-audit.md)
selects these twelve packages as the portable 0.1.0 candidate, with STL/OBJ defaults
and PLY/selected-glTF opt-ins. It records confirmed correctness and workflow gaps
and the [remaining implementation plan](../plans/2026-11-01-011-feat-first-release-usability-plan.md).
Candidate selection and local archive success do not enable publication or change
the eligibility flags below. Its public registry observation is documentary,
not the approved expected-state manifest required for a real train.

The following packages currently set `publish = false`:

- `baozi-test-support`: test-only helpers
- `baozi-xtask`: repository-only governance tooling
- `baozi-format-ply`: intended for the release unit, but publication remains disabled until its
  package and release evidence is complete
- `baozi-format-gltf`: the selected profile is source-admitted after the `2dc2fc8` Linux ASan and
  independent consumer gates; publication remains disabled pending package/release evidence
- `baozi-fuzz`: cargo-fuzz workspace
- `baozi-native-io-candidate`: standalone, unregistered native adapter and optional repository capture
- `baozi-cap-primitives`, `baozi-cap-std`, and `baozi-cap-fs-ext`: maintained native dependency
  packages, each version 0.1.0, based on the complete upstream 4.0.3 source and license choices

The four native packages remain outside the canonical facade release unit and publish order.
Baozi owns maintenance of their distinct identities and exact dependency edges; the Rust library
aliases remain `cap_primitives`, `cap_std`, and `cap_fs_ext`. The
[third-party notices](../../THIRD_PARTY_NOTICES.md#cap-std-project) and per-package `PATCHES.md`
records preserve upstream authorship, copyrights, licenses and archive provenance. Local native
archive checks do not add those packages to a release train or register a supported target.

Manifest eligibility, local artifact qualification, and actual registry publication are distinct.
Local archives and their consumer checks do not change these flags or authorize publication.

Shell format crates must stay `publish = false` until they import representative fixtures into
`Scene` with validation, snapshots, malformed tests, resource limits, and fuzz coverage.

## Publish Order

The facade crate `baozi` depends on workspace crates that are not yet present on crates.io for the
canonical train. The existing placeholder `baozi 0.0.1` does not satisfy any dependency or
compatibility requirement for that train. Registry-based package verification and publication of
`baozi` require every admitted supporting dependency to be available at the matching version.
Local archive creation and extracted-artifact consumer qualification can complete before that train.

Canonical release order:

1. `baozi-operation`
2. `baozi-io`
3. `baozi-core`
4. `baozi-import`
5. `baozi-format-stl`
6. `baozi-format-obj`
7. `baozi-postprocess` (after its versioned OBJ development dependency)
8. `baozi-format-ply` after its publication gate is complete
9. `baozi-gltf-json`, `baozi-gltf`, then `baozi-format-gltf` after their publication gates are complete
10. `baozi`

Package each crate from a clean checkout of the exact tagged release commit with
`cargo package --locked -p <crate>`. Registry release packaging must not use `--allow-dirty` or
`--no-verify`.
The `-p` form selects root workspace members only. The excluded maintained glTF
packages need their own `--manifest-path` selection and reviewed lock inputs;
settle those before the real train. The local rehearsal's temporary workspace
does not establish the real clean-tag publication procedure for those packages.
The corresponding real upload command is
`cargo publish --locked --registry crates-io -p <crate>`; a dry run adds `--dry-run` to that exact
locked command. Record the local packaged `.crate` SHA-256 before upload and require the post-publish
registry checksum to equal it. Then bounded-retry the exact-version archive download and require the
downloaded `.crate` SHA-256, registry checksum, and local archive SHA-256 all to match before the next
crate may start.
For actual registry publication, verify the facade package only after its dependency versions are
available from the registry. Every required or optional production dependency must be
registry-publishable and registry-resolvable, including format dependencies whose features are
disabled in a particular build. Every admitted facade dependency must therefore be published first
or removed from the release manifest by an explicit architecture decision. The preferred
first-release path is to complete PLY's publication evidence and publish it before `baozi`.

CI packages both release-unit leaves, `baozi-operation` and `baozi-io`, before the canonical registry
train. Registry-based verification of dependent crates follows publish order after their internal
dependencies exist on crates.io at the matching version. The existing
[`check_packages.py`](../../scripts/check_packages.py) additionally rehearses the complete local
production closure before registry publication. It creates actual `.crate` archives and retains
their normalized manifests unchanged. Independent consumers declare exact version-only roots and
resolve against a standard Cargo directory-source snapshot containing those archives' extracted
contents and the required registry dependencies. No consumer-root patches, rewritten artifact
dependency paths, or checkout sources satisfy this rehearsal. The supplied snapshot does not
establish that unpublished versions exist on crates.io or that their names are available.

Run `python scripts/check_packages.py --toolchain 1.95.0` for all maintained cases, and use
`--toolchain stable --release` for the current-stable release matrix. Local archive creation uses
`--locked --offline --allow-dirty --no-verify --exclude-lockfile`, omitting generated package
lockfiles. The scene consumer keeps its handwritten source and fixed feature cases. Its checked-in
lockfile is staging input; the initial offline resolution regenerates or transitions only the
staging consumer locks to registry identities. Subsequent checks, builds and runs use
`--locked --offline`, without an unlocked retry. Source manifests and lockfiles remain unchanged.

Add `--native --features repository-capture` to rehearse the unpublished native chain. In runtime
mode the native baseline executes file import; the repository feature is an additional compile
check, not an executed capture result. `--native` also runs a separate compile-only all-feature
helper consumer covering retained upstream helpers such as `arbitrary`, `fs_utf8`, and
`arf_strings`; candidate `--all-features` does not enable all dependency features. Neither helper
coverage nor repository compilation expands the admitted runtime feature set. CI configures the
scene matrix, native baseline, optional repository compilation and helper all-feature checks on
Linux x86_64, Windows x86_64, and Apple ARM64, with MSRV/debug and current-stable/release jobs on
each host.

For the copied native packages, the historical upstream `Cargo.toml.orig` and `.cargo_vcs_info.json`
remain in the checkout but are excluded from package inputs. Cargo generates a new archive
`Cargo.toml.orig` from the maintained active manifest and a normalized archive `Cargo.toml`.
Upstream checksums identify original archives; new maintained archive digests identify the actual
rehearsal artifacts. See [the package procedure](package-consumer-checks.md) for these boundaries.

Workflow configuration and archive creation are not execution receipts. Record the actual source
revision, cases, targets, toolchains and results once executed; an in-progress implementation has
no frozen qualification revision. The rehearsal does not replace exact-tag packaging, registry
ownership/authentication checks, or the real release commands above, and it does not authorize
publication or native target promotion.

## Release Checklist

This checklist applies before an actual registry publication. H's local artifact qualification
follows the delivery plan and does not require completing publication automation.

Before any crate publish:

- `cargo fmt --all -- --check`
- `cargo check --locked --workspace --all-targets`
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- `cargo nextest run --locked --workspace --exclude baozi-xtask`
- `cargo nextest run --locked --workspace --all-features --exclude baozi-xtask`
- `cargo test --locked --doc --workspace --all-features`
- `cargo doc --locked --workspace --all-features --no-deps` with `RUSTDOCFLAGS=-D warnings`
- `cargo deny --locked check`
- `cargo deny --locked --manifest-path fuzz/Cargo.toml check`
- fuzz smoke for public parser crates in Linux CI
- `CHANGELOG.md` updated
- support matrix and format docs updated
- workspace path dependencies include matching `version` fields
- security-impacting parser changes called out in release notes
- the crates.io registry-state check records the placeholder `baozi 0.0.1` and its live yank state;
  no manifest or publish workflow targets that version
- before the first upload, one complete registry preflight confirms every release-unit name is
  absent or owned exactly as expected; any unexpected occupation or owner drift visible then stops
  the whole train with zero publications
- immediately before every upload, the exact package name and owner set are rechecked; a late race
  stops before that upload and exercises the documented partial-train recovery path rather than
  claiming the initial preflight reserved the name
- each never-published supporting crate has a short-lived exact-crate `publish-new` token path;
  updates and yanks use separate exact-crate `publish-update` and `yank` credentials, while any
  Trusted Publishing/OIDC path requires complete publisher-set equality and a separate positive
  token exchange for an already-existing package name
- every API token has a manifest-declared TTL no greater than 24 hours; `always()` cleanup attempts
  and verifies immediate revocation, while an unverified cleanup blocks later jobs and requires
  confirmed manual revocation or expiry
- workflow tests prove the credential routing and negative cases; `cargo publish --dry-run` validates
  packaging only and is never treated as evidence that production authentication succeeds
- release, publish, signing, and attestation actions use full commit SHA pins; privileged jobs use a
  protected release environment and per-job least privilege, with `id-token: write` present only
  where the exact OIDC or attestation flow requires it
- after every real publish, bounded retries verify the expected owner and exact version, download the
  exact `.crate`, and require `download SHA-256 == registry checksum == local package SHA-256` before
  any later package is allowed to publish
- provenance, SBOM, signature, and reproducibility claims are backed by explicit release evidence
  or are documented as deferred

After the complete canonical train is published:

- verify every exact-version registry artifact, then download, build and run a fresh facade consumer
  using crates.io alone, without path dependencies, patches or source replacement
- yank `baozi 0.0.1` only after that consumer succeeds and verify the live yank result
- record a failed retirement as pending work; preserve the verified canonical release and retry
  the exact-version yank separately

## MSRV

The workspace currently uses Rust 1.95. Lowering MSRV can be considered later after format coverage
and public API shape stabilize. Until then, implementation velocity and language ergonomics take
priority.
