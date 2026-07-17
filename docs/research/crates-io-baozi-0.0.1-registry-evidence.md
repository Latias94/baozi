# crates.io `baozi 0.0.1` Registry Evidence

**Capture date:** 2026-07-17

**Purpose:** Record the immutable package identity of the pre-canonical `baozi 0.0.1` placeholder
without treating mutable registry controls, such as yank status or current ownership, as historical
facts.

## Exact Registry Capture

The machine-readable normalization of this observation is committed as
[`crates-io-baozi-0.0.1-registry-evidence.toml`](crates-io-baozi-0.0.1-registry-evidence.toml). It
separates the documentary raw-response fingerprint, immutable version/archive identity, and mutable
release-time observations so a gate cannot accidentally compare a changing live JSON body with a
historical digest.

The official exact-version endpoint was retrieved after redirects as raw UTF-8 bytes:

- request URL: <https://crates.io/api/v1/crates/baozi/0.0.1>
- final URL: <https://crates.io/api/v1/crates/baozi/0.0.1>
- HTTP status: `200 OK`
- retrieval date: 2026-07-17
- response encoding: UTF-8
- response byte length: `1388`
- response SHA-256: `afd7d3d57fb9503fc745ac247d7d260f3cb0a06f665ce2fb51e563590e46bff2`
- package: `baozi 0.0.1`
- package description: `hello world`
- published at (`version.created_at`): `2026-07-09T05:03:00.933097Z`
- crates.io package checksum / `.crate` SHA-256:
  `e74b14d1e2410f8a062bb423f7bbc8bc9fdaa9cf4ac64b10efccc562d62bbd28`

The exact version and package checksum identify immutable published bytes. The API body is not
immutable: it includes mutable registry fields such as yank state and may change representation.
Its byte length and digest identify only this time-point response observation so later reviewers can
distinguish it from a different response. The raw response is not used as a live equality oracle.
U9 must parse the normalized manifest, query the live endpoint, compare only the immutable tuple,
and independently require live `yanked = true` before the canonical train.

## Mutable Registry State

The capture reported `yanked = false` on 2026-07-17. That value is an observation only: crates.io
owners can change yank status after publication. Release automation must therefore query the live
exact-version endpoint immediately before the canonical release train and fail closed unless
`baozi 0.0.1` is yanked.

Current owners, Trusted Publishing configuration, workflow bindings, and token availability are also
mutable operational state. They require live release-time checks and are not inferred from this
evidence record. U9's tag/commit-bound `docs/release/registry-state.toml` supplies the reviewed
expected owner and credential state; the live response is only the observed side of that comparison.

The pre-release registry check covers the complete release unit before any upload. It fails if an
expected-absent supporting name is occupied, if an existing name has the wrong owner set, or if the
registry cannot return a complete answer. This observation does not reserve names, so the exact name
and owner set are checked again immediately before each upload; a late conflict enters the partial-
train recovery path. API-token fallback uses crates.io's endpoint scopes `publish-new`,
`publish-update`, and `yank`, an exact crate-name scope, and a maximum 24-hour TTL. An `always()`
cleanup attempts and verifies immediate revocation; unverified cleanup blocks later publication and
falls back to confirmed manual revocation or expiry. It never uses an unrestricted or no-expiry
token.

## Official Publication Authentication Guidance

### Cargo Book: Initial Publication

The current official Cargo publishing page was captured after redirects as a living page:

- request URL: <https://doc.rust-lang.org/cargo/reference/publishing.html>
- final URL: <https://doc.rust-lang.org/cargo/reference/publishing.html>
- HTTP status: `200 OK`
- retrieval date: 2026-07-17
- response encoding: UTF-8
- response byte length: `38911`
- response SHA-256: `3c1aaefdfa64f025f1f287b02be4ec3e623105012eea142ac91c09ba7351053e`

Its "Before your first publish" section instructs publishers to acquire an API token and use it
through `cargo login`. This supports requiring an API-token bootstrap for a package name that has not
yet been published; a dry run remains packaging evidence and does not authenticate to the registry.

### crates.io: Trusted Publishing

The crates.io repository's official Trusted Publishing documentation source was captured at full
commit `e9c29a333bcb7c14e82adb59309e0220a771648a`:

- request URL:
  <https://raw.githubusercontent.com/rust-lang/crates.io/e9c29a333bcb7c14e82adb59309e0220a771648a/svelte/src/routes/docs/trusted-publishing/+page.svelte>
- final URL:
  <https://raw.githubusercontent.com/rust-lang/crates.io/e9c29a333bcb7c14e82adb59309e0220a771648a/svelte/src/routes/docs/trusted-publishing/+page.svelte>
- HTTP status: `200 OK`
- retrieval date: 2026-07-17
- response encoding: UTF-8
- response byte length: `8884`
- response SHA-256: `ceab97a72a8b47ffdc43acc7f9bd2437d75fa1af7348e83e584dabd5d50ebe40`

That fixed source states that a crate must already be published and that initial publication requires
an API token. It describes Trusted Publishing as an OIDC exchange for a short-lived token bound to a
configured repository, workflow, owner, and optional environment. It also states that API-token and
Trusted Publishing methods may coexist during migration.

Baozi therefore treats Trusted Publishing as a conditional path for an already-published package
name, not as proof that `baozi` currently has a valid binding. U9 must exhaustively enumerate the
complete live publisher configuration set, compare it with the reviewed manifest, keep crate-owner
and repository-owner identities distinct, and separately prove the selected binding with a token
exchange. If complete enumeration or an equally current protected snapshot is unavailable, all
publication is blocked; an approved exact-crate token is a fallback only after set equality succeeds
and a typed service-availability failure occurs before token issuance. Claim, binding,
configuration, and authentication rejection are not fallback conditions.

The endpoint/crate-scope and expiry model above is grounded in the crates.io source at commit
`e9c29a333bcb7c14e82adb59309e0220a771648a`: `EndpointScope` declares `publish-new`,
`publish-update`, and `yank`, `CrateScope` validates exact or prefix patterns, and the token settings
UI accepts endpoint scopes, crate scopes, and expiration. Baozi deliberately chooses exact names
rather than prefix patterns for release credentials.

## Release Interpretation

The placeholder is not the canonical Baozi API or a member of the lockstep `0.1.0` release train.
Yanking it does not delete or replace its bytes, does not free the version for reuse, and does not
make the canonical train a first publication of the `baozi` package name. Supporting package names
that do not yet exist on crates.io still require their own first-publication bootstrap.
