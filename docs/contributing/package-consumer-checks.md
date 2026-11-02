# Packaged Consumer Checks

Run `scripts/check_packages.py` with Python 3.12 or later from a checkout with the
required Rust toolchain installed. It reuses the handwritten semantic assertions
in `tools/consumers/scene` and copies its checked-in `Cargo.lock` as staging input.
The staged lock transitions to registry identities before locked artifact builds;
the checkout's source manifests and lockfiles remain unchanged.

The unpublished `baozi-test-support` package is a path-only workspace development
dependency without a version. Cargo omits it from normalized published manifests,
so the facade and format archives do not require an unpublished registry helper
when resolved as root packages. Repository fixture tests still use the checkout;
this does not claim those tests run from the stripped published development graph.
Keep ordinary versioned development dependencies in dependency-order publication
planning as well as the required and optional production closure.

```text
python scripts/check_packages.py --toolchain 1.95.0 --case default
python scripts/check_packages.py --toolchain 1.95.0 --case empty --case obj --case ply --case all
python scripts/check_packages.py --toolchain 1.95.0 --release
python scripts/check_packages.py --toolchain 1.95.0 --case all --target x86_64-pc-windows-msvc --check
python scripts/check_packages.py --toolchain 1.95.0 --native --features repository-capture
python scripts/check_packages.py --toolchain 1.95.0 --native --features repository-capture --offline
python scripts/check_packages.py --toolchain 1.95.0 --case gltf --gltf-conformance --offline
```

Without `--case`, the script runs the fixed matrix from `scripts/check_features.py`.
CI configures that complete matrix on Linux x86_64 (`ubuntu-24.04`), Windows x86_64
(`windows-2025`), and Apple ARM64 (`macos-15`), with MSRV/debug and current-stable/release
jobs on each actual host. Each job verifies the compiler host before packaging and
running the consumers, and includes the native baseline, optional repository
compilation, and separate helper all-feature checks. This describes workflow
configuration, not completed execution or target qualification; results must
identify the source revision actually run.
Repeat `--case` to select a subset of the twelve maintained cases. Fixed cases retain that matrix's handwritten
expected format sets; the driver does not infer expectations from feature wiring.
The maintained consumer forwards every public facade feature exactly once, so its
`--all-features` case is the facade's complete public feature union. Native IO development lives
in a separate unpublished candidate workspace and cannot be activated by the portable matrix.

The driver runs serially with `CARGO_BUILD_JOBS=1`. Archive creation reuses `target`;
artifact compilation uses fresh `build` output and `build-intermediates` directories
within that run's staging area. Both Cargo's build-dir configuration and its
environment override are set explicitly; CI also passes its exact matrix target.
This keeps cached checkout builds from substituting for compilation of the extracted archives.
It retains each run's archives, directory-source snapshot, and staged consumers under
`target/package-check-*`. It does not delete previous runs or change source manifests.

The stages are distinct:

1. Gather locked Cargo metadata for the root workspace and follow the resolved local
   normal/build edges from `baozi` and `baozi-import`, excluding development edges and
   unrelated workspace members from the archive set. `--native` also gathers the
   standalone candidate's feature closure and separately resolves each maintained
   dependency package with all its features enabled. The candidate's
   `--all-features` alone does not enable those upstream helpers. `--offline` also
   makes this input-gathering phase offline; otherwise Cargo may fetch missing inputs.
2. Package the local production closure in dependency order using
   `cargo package --locked --offline --allow-dirty --no-verify --exclude-lockfile`.
   Excluded maintained packages use a temporary workspace that preserves their real
   sibling dependency paths. Archive creation is not build or execution evidence.
3. Extract the actual `.crate` files without rewriting their normalized manifests.
   Check package identities, version-only dependency declarations, README and license
   files. Copy selected registry archives into this run's private staging area,
   verify their locked digests and extract those same private copies. Assemble a
   standard Cargo directory-source snapshot from the maintained artifacts and
   these registry dependencies. Its `.cargo-checksum.json`
   records use each new local archive's own digest; upstream archive provenance is
   a separate identity. Include the registry closure for any helper features being
   checked as well as the selected native profiles.
4. Stage the scene consumer with its unchanged Rust source and feature forwarding
   lists, but exact version-only `baozi` and `baozi-import` root dependencies.
   `--native` stages a separate small native consumer and STL fixture, plus a helper
   consumer that explicitly requests all maintained dependency features. Configure
   Cargo's standard source replacement to use the directory snapshot, with a fresh
   staging `CARGO_HOME`. Consumer roots contain no patches or checkout paths, and
   artifact manifests are not rewritten to point at source directories.
5. Resolve each staged consumer offline and regenerate or transition only its
   staging `Cargo.lock` to the registry package identities. This initial resolution
   deliberately omits `--locked`; subsequent artifact checks, builds and runs use
   `--locked --offline`. Verify the package identities and manifest paths against
   this run's exact extracted artifacts and registry snapshot. There is no retry
   that relaxes locking after a build failure.
6. Build the selected consumer cases in the fresh artifact build directory and
   execute their semantic assertions. `--check` performs only a Cargo check and
   makes no runtime claim. The optional repository case compiles the feature;
   it does not execute repository capture. The helper consumer always uses a
   compile-only all-feature Cargo check.

Package-generated lockfiles are deliberately omitted with `--exclude-lockfile`.
The source workspace, candidate, and maintained consumer lockfiles remain untouched;
only staging locks are regenerated for the supplied snapshot. The directory source
supplies the unpublished versions for this local registry rehearsal. It does not
establish their availability or ownership on crates.io, authorize publication,
admit a dependency, or promote a target. The fixed portable cases cover the
source-admitted OBJ/PLY/STL and selected glTF profiles:

```text
python scripts/check_packages.py --toolchain 1.95.0 --case gltf
python scripts/check_packages.py --toolchain 1.95.0 --case postprocess
```

The postprocess case imports an OBJ quad with Corner UVs and authored normals, triangulates it,
checks preservation before explicitly replacing normals, and converts the full static Scene to
left-handed centimeters. Handwritten assertions check positions, normals, index/UV lane pairing,
assumed source units, and evidence lifetime after Scene teardown. It builds application-owned
vertex/index buffers from complete borrowed draw tuples and checks them after dropping the Scene.
Global vertex deduplication is a separate caller policy.

The same fixed consumer also checks first-release maturation semantics. PLY cases retain adjacent
F64 Position values, signed zero and F64 world bounds. glTF cases distinguish authored `{}` from
one generated implicit material and use an owned parent/placement index after Scene teardown.
Postprocess cases prepare two polygon meshes with normals in one bounded reconstruction and
publication, rejecting the old handles. These are ordinary public calls in the unchanged consumer
source, executed against extracted archives along with the older cases.

Every OBJ-enabled case also checks a partial-UV input: the default policy retains a typed loss
rejection, while explicit `SourceSemanticLoss` authority permits dropping the incomplete UV channel
and preserves the complete authored Corner normals. The consumer names `ImportEvidence` and the
loss/diagnostic types through `baozi`, checks the exact source, subject, code and disposition, and
reads the retained report after dropping its Scene. The glTF cases inspect the selected source
scene identity and selected/omitted node counts through the same facade. Ordinary report consumers
need no parser-authoring import to name these return types.

The owned selected-profile glTF document adds no maintained decoder package to the portable
ten-package production closure. Every local production package is checked as its unchanged
`.crate` artifact; no consumer-root patch or checkout dependency substitutes for those artifacts.
The owning maintainer updates `check_features.py` and the supported-format list when
changing admission. A command-line flag cannot admit another format.

`--gltf-conformance` additionally stages the fixed standalone direct-format consumer. Its
production roots are exact version-only `baozi-format-gltf`, `baozi-import` and `baozi-io`,
without the facade. The copied tests name `baozi-core` as a development dependency and compare
with exact `gltf = 1.4.1`, with default import/pixel helpers disabled. Registry snapshots include
that locked development closure separately; reference libraries do not enter Baozi's production
archives or portable feature union. The driver tests the extracted consumer with all targets,
or checks all targets without execution under `--check`. It preserves the existing twelve
facade cases and exact-archive source verification. Separate official Validator results remain
a distinct evidence requirement; a successful package rehearsal establishes neither full glTF
conformance nor live registry availability.

Native package and feature checks have separate scopes:

| Scope | Dependency and evidence boundary |
| --- | --- |
| Portable scene matrix | The twelve facade feature cases keep the complete public feature union portable; they do not activate native IO. |
| Native default/runtime | `--native` builds and, unless `--check` is selected, executes scoped file reads and public STL import through `baozi-native-io-candidate`. The runtime chain includes `baozi-cap-std` and `baozi-cap-primitives`, with dependency default features disabled. |
| Optional repository capture | `--native --features repository-capture` additionally compiles `baozi-cap-fs-ext` with its explicit `std` feature. This is optional dependency and compilation coverage, not an executed capture result. The default native closure must remain free of this package. |
| Upstream helper features | `--native` also runs a separate compile-only all-feature helper consumer. It covers primitives' `arbitrary`/derive and the upper packages' `fs_utf8`/`arf_strings` helpers, including `camino`, `arf-strings`, and their registry closure. These checks do not enlarge the admitted runtime or repository feature sets. |

The three maintained `baozi-cap-*` packages are version 0.1.0, based on upstream
4.0.3, and retain the `cap_primitives`, `cap_std`, and `cap_fs_ext` library aliases.
Every maintained edge has a relative development path and exact distribution
version. The original source, build scripts, authorship, copyright and full license
choices remain documented in [third-party notices](../../THIRD_PARTY_NOTICES.md#cap-std-project)
and each package's `PATCHES.md`. All three maintained packages and the native
candidate set `publish = false`; the native registered-target set remains empty.

Copied upstream `Cargo.toml.orig` and `.cargo_vcs_info.json` files remain historical
source material and are excluded from package inputs because Cargo reserves those
names. Cargo generates the archive's `Cargo.toml.orig` from the maintained active
manifest and produces the normalized archive `Cargo.toml`. The generated `.orig`
therefore describes the maintained manifest, not the preserved upstream original.
Historical upstream lockfiles do not replace the staging consumer's registry lock.

`--target` selects a Cargo target for the consumer build/check and run. Executing a
foreign target requires a working Cargo runner; a successful cross-target check is
only compile evidence. `--release` changes the consumer build profile, not archive
creation.
