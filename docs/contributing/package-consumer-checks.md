# Packaged Scene Consumer Checks

Run `scripts/check_packages.py` with Python 3.12 or later from a checkout with the
required Rust toolchain installed. It reuses `tools/consumers/scene`, including its
handwritten semantic assertions and its checked-in `Cargo.lock`.

```text
python scripts/check_packages.py --toolchain 1.95.0 --case default
python scripts/check_packages.py --toolchain 1.95.0 --case empty --case obj --case ply --case all
python scripts/check_packages.py --toolchain 1.95.0 --release
python scripts/check_packages.py --toolchain 1.95.0 --case all --target x86_64-pc-windows-msvc --check
```

Without `--case`, the script runs the fixed matrix from `scripts/check_features.py`.
CI executes that complete matrix on Windows x86_64 and Apple ARM64, using MSRV/debug and
current-stable/release jobs on each actual host. Each job verifies the compiler host before
packaging and running the consumer. A configured job alone does not establish target qualification.
Repeat `--case` to select a subset of the twelve maintained cases. Fixed cases retain that matrix's handwritten
expected format sets; the driver does not infer expectations from feature wiring.
The maintained consumer forwards every public facade feature exactly once, so its
`--all-features` case is the facade's complete public feature union. Native IO development lives
in a separate unpublished candidate workspace and cannot be activated by the portable matrix.

The driver runs serially with `CARGO_BUILD_JOBS=1`. Archive creation reuses `target`;
artifact compilation uses a fresh `build` directory within that run's staging area.
This keeps cached checkout builds from substituting for compilation of the extracted archives.
It retains each run's archives, extracted packages, and consumer copy under
`target/package-check-*`. It does not delete previous runs or change source manifests.

The stages are distinct:

1. Call `cargo metadata --locked --format-version 1 --all-features` once for the
   workspace. Follow Cargo's resolved local normal/build dependency edges from
   `baozi` and `baozi-import`, excluding development dependencies and unrelated
   workspace members. The excluded maintained glTF vendor crates are packaged from
   a temporary two-member Cargo workspace because Cargo otherwise discovers the
   enclosing checkout workspace when invoked from a nested staging path. Run
   `cargo package --locked --allow-dirty --no-verify --exclude-lockfile` for each package
   in dependency order.
   Archive creation is not build or execution evidence.
2. Extract the resulting `.crate` files. Copy the maintained consumer without changing
   its Rust source or lockfile. Write a small staging manifest with its two explicit
   dependencies (`baozi`, `baozi-import`) pointing to extracted packages. Copy its
   feature forwarding lists unchanged, and add `[patch.crates-io]` entries pointing
   only to extracted transitive packages. Changes to the consumer's direct dependency
   list require updating this small manifest.
3. Resolve the staged consumer once with `cargo metadata --locked --all-features`.
   Require every local dependency's manifest to be the corresponding extracted artifact.
   This rejects inherited Cargo patch/path configuration that substitutes a checkout or
   registry dependency; the consumer's own manifest is the only exception.
4. Build the consumer and its packaged dependencies with `--locked`, then execute
   the maintained semantic workflow with the selected expected format set.
   `--check` performs only a Cargo check and makes no runtime claim.

Package-generated lockfiles are deliberately omitted: generating registry lockfiles
would require unpublished dependencies to exist in a registry. The workspace lockfile
remains untouched, and the maintained consumer lockfile controls the artifact build.
The script never retries without `--locked` or refreshes a lockfile. A stale consumer
lockfile must be updated separately by the owning maintainer.

This is a local package simulation, not a publication or admission gate. It does not
change `publish`, features, registration, or release policy. The fixed cases cover the
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

Every OBJ-enabled case also checks a partial-UV input: the default policy retains a typed loss
rejection, while explicit `SourceSemanticLoss` authority permits dropping the incomplete UV channel
and preserves the complete authored Corner normals. The consumer names `ImportEvidence` and the
loss/diagnostic types through `baozi`, checks the exact source, subject, code and disposition, and
reads the retained report after dropping its Scene. The glTF cases inspect the selected source
scene identity and selected/omitted node counts through the same facade. Ordinary report consumers
need no parser-authoring import to name these return types.

The glTF feature's complete local production closure includes
the maintained `baozi-gltf` and `baozi-gltf-json` packages as actual `.crate` artifacts.
No patch points back to workspace or vendor source directories.
The owning maintainer updates `check_features.py` and the supported-format list when
changing admission. A command-line flag cannot admit another format.

`--target` selects a Cargo target for the consumer build/check and run. Executing a
foreign target requires a working Cargo runner; a successful cross-target check is
only compile evidence. `--release` changes the consumer build profile, not archive
creation.
