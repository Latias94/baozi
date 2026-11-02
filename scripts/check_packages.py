"""Run packaged consumers in an offline registry rehearsal, not a publication proof."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib

from check_features import CASES, validate_consumer_features


CONSUMER_DEPENDENCIES = ("baozi", "baozi-import")
GLTF_CONSUMER_DEPENDENCIES = ("baozi-format-gltf", "baozi-import", "baozi-io")
NATIVE_DEPENDENCY = "baozi-native-io-candidate"
NATIVE_HELPERS = ("baozi-cap-primitives", "baozi-cap-std", "baozi-cap-fs-ext")
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"

NATIVE_STL = """solid packaged
facet normal 0 0 1
outer loop
vertex 0 0 0
vertex 1 0 0
vertex 0 1 0
endloop
endfacet
endsolid packaged
"""

NATIVE_CONSUMER = """use baozi::{AssetIo, AssetPath, Importer, TopologyKind};
use baozi_native_io_candidate::{
    FileSystemAssetIo, NativeDirectoryCapability, NativeFsSupportStatus,
    NATIVE_FS_REGISTERED_TARGETS, NATIVE_FS_SUPPORT_STATUS,
};

fn main() {
    assert_eq!(NATIVE_FS_SUPPORT_STATUS, NativeFsSupportStatus::CandidateUnregistered);
    assert!(NATIVE_FS_REGISTERED_TARGETS.is_empty());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let capability = NativeDirectoryCapability::open_ambient(root).expect("native root scope");
    let io = FileSystemAssetIo::from_capability(capability);
    let source = AssetPath::new("triangle.stl").expect("relative STL path");
    let mut asset = io.open(&source).expect("open scoped native STL");
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 32];
    loop {
        let count = asset.read_chunk(&mut chunk).expect("read scoped native STL");
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    assert_eq!(bytes, include_bytes!("../fixtures/triangle.stl"));
    let report = Importer::new().expect("STL registry")
        .read_asset(&io, source.clone()).expect("native STL import");
    assert_eq!(report.source(), &source);
    assert_eq!(report.format().id().as_str(), "org.baozi.stl");
    assert!(report.diagnostics().is_empty());
    assert_eq!(report.scene().mesh_count(), 1);
    assert_eq!(report.scene().primitive_count(), 1);
    let mesh = report.scene().meshes().next().expect("one mesh");
    let primitive = mesh.primitives().next().expect("one primitive");
    assert_eq!(primitive.topology(), TopologyKind::TriangleList);
    assert_eq!(primitive.vertex_count(), 3);
    assert_eq!(primitive.face_count(), 1);
    println!("Unregistered native candidate offline registry rehearsal passed");
}
"""


def read_manifest(path: Path) -> dict:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def package_order(metadata: dict, roots: tuple[str, ...] = CONSUMER_DEPENDENCIES) -> list[dict]:
    """Topologically order local production edges already resolved by Cargo."""
    packages = {package["id"]: package for package in metadata["packages"]
                if package["source"] is None}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    ordered = []
    visited: set[str] = set()

    def visit(package_id: str) -> None:
        if package_id in visited:
            return
        visited.add(package_id)
        dependencies = {
            dependency["pkg"] for dependency in nodes[package_id]["deps"]
            if dependency["pkg"] in packages
            and any(kind["kind"] in (None, "build") for kind in dependency["dep_kinds"])
        }
        for dependency in sorted(dependencies, key=lambda key: packages[key]["name"]):
            visit(dependency)
        ordered.append(packages[package_id])

    for name in roots:
        matches = [key for key, package in packages.items() if package["name"] == name]
        if len(matches) != 1:
            raise ValueError(f"Expected one local {name} package in Cargo metadata")
        visit(matches[0])
    return ordered


def select_cases(names: list[str]) -> list[tuple[str, list[str], str]]:
    cases = []
    for name in dict.fromkeys(names):
        if name not in CASES:
            raise ValueError(f"Unknown fixed consumer case: {name}")
        flags, expected = CASES[name]
        cases.append((name, list(flags), expected))
    return cases


def registry_dependency(name: str, artifacts: dict[str, Path], features: tuple[str, ...] = ()) -> str:
    version = read_manifest(artifacts[name] / "Cargo.toml")["package"]["version"]
    return (f'{json.dumps(name)} = {{ version = "={version}", default-features = false, '
            f'features = {json.dumps(features)} }}')


def stage_consumer(source: Path, destination: Path, artifacts: dict[str, Path]) -> None:
    consumer = read_manifest(source / "Cargo.toml")
    if set(consumer["dependencies"]) != set(CONSUMER_DEPENDENCIES):
        raise ValueError("Update the small staging manifest for changed consumer dependencies")
    shutil.copytree(source, destination, ignore=shutil.ignore_patterns("target", "__pycache__"))
    if not (destination / "Cargo.lock").is_file():
        raise ValueError("The maintained scene consumer needs a checked-in Cargo.lock")
    package = consumer["package"]
    lines = [
        "[package]",
        *(f"{key} = {json.dumps(package[key], ensure_ascii=False)}"
          for key in ("name", "version", "edition", "rust-version")),
        "publish = false", "", "[workspace]", 'resolver = "3"', "", "[dependencies]",
    ]
    for name in CONSUMER_DEPENDENCIES:
        lines.append(registry_dependency(name, artifacts))
    lines.extend(["", "[features]"])
    # Copy the maintained forwarding lists verbatim; Cargo interprets the features.
    lines.extend(f"{json.dumps(name, ensure_ascii=False)} = {json.dumps(features, ensure_ascii=False)}"
                 for name, features in consumer["features"].items())
    (destination / "Cargo.toml").write_text("\n".join(lines) + "\n", encoding="utf-8")


def stage_native_consumer(destination: Path, artifacts: dict[str, Path]) -> None:
    (destination / "src").mkdir(parents=True)
    (destination / "fixtures").mkdir()
    (destination / "src/main.rs").write_text(NATIVE_CONSUMER, encoding="utf-8")
    (destination / "fixtures/triangle.stl").write_text(NATIVE_STL, encoding="utf-8")
    (destination / "Cargo.toml").write_text("\n".join([
        "[package]", 'name = "baozi-packaged-native-consumer"', 'version = "0.0.0"',
        'edition = "2024"', 'rust-version = "1.95"', "publish = false", "", "[workspace]",
        'resolver = "3"', "", "[dependencies]",
        registry_dependency("baozi", artifacts, ("format-stl",)),
        registry_dependency(NATIVE_DEPENDENCY, artifacts), "", "[features]",
        'repository-capture = ["baozi-native-io-candidate/repository-capture"]', "",
    ]), encoding="utf-8")


def stage_gltf_consumer(source: Path, destination: Path, artifacts: dict[str, Path]) -> None:
    """Rehearse the one maintained direct-format consumer with version-only roots."""
    consumer = read_manifest(source / "Cargo.toml")
    if set(consumer["dependencies"]) != set(GLTF_CONSUMER_DEPENDENCIES):
        raise ValueError("Update the fixed glTF staging roots for changed dependencies")
    development = consumer["dev-dependencies"]
    if set(development) != {"baozi-core", "gltf"}:
        raise ValueError("Update the fixed glTF staging development dependencies")
    reference = development["gltf"]
    if reference["version"] != "=1.4.1" or reference.get("default-features", True):
        raise ValueError("The glTF reference must remain exact 1.4.1 without import helpers")
    shutil.copytree(source, destination, ignore=shutil.ignore_patterns("target", "__pycache__"))
    if not (destination / "Cargo.lock").is_file():
        raise ValueError("The direct glTF consumer needs a checked-in Cargo.lock")
    package = consumer["package"]
    lines = [
        "[package]",
        *(f"{key} = {json.dumps(package[key], ensure_ascii=False)}"
          for key in ("name", "version", "edition", "rust-version")),
        "publish = false", "", "[workspace]", 'resolver = "3"', "", "[dependencies]",
        *(registry_dependency(name, artifacts) for name in GLTF_CONSUMER_DEPENDENCIES),
        "", "[dev-dependencies]", registry_dependency("baozi-core", artifacts),
        'gltf = { version = "=1.4.1", default-features = false, features = '
        + json.dumps(reference.get("features", [])) + " }", "",
    ]
    (destination / "Cargo.toml").write_text("\n".join(lines), encoding="utf-8")


def stage_helper_consumer(destination: Path, artifacts: dict[str, Path],
                          features: dict[str, tuple[str, ...]]) -> None:
    """Exercise every helper feature reported by Cargo, separately from native profiles."""
    (destination / "src").mkdir(parents=True)
    (destination / "src/main.rs").write_text("fn main() {}\n", encoding="utf-8")
    (destination / "Cargo.toml").write_text("\n".join([
        "[package]", 'name = "baozi-packaged-helper-consumer"', 'version = "0.0.0"',
        'edition = "2024"', 'rust-version = "1.95"', "publish = false", "", "[workspace]",
        'resolver = "3"', "", "[dependencies]",
        *(registry_dependency(name, artifacts, features[name]) for name in NATIVE_HELPERS), "",
    ]), encoding="utf-8")


def verify_artifact_sources(metadata: dict, consumer: Path, artifacts: dict[str, Path],
                            required: set[str]) -> None:
    """Require registry identities backed by the exact archive directories in this run."""
    consumer_manifest = (consumer / "Cargo.toml").resolve()
    seen = set()
    for package in metadata["packages"]:
        manifest = Path(package["manifest_path"]).resolve()
        if manifest == consumer_manifest:
            continue
        artifact = artifacts.get(package["name"])
        if package["source"] != CRATES_IO:
            raise ValueError(f"Dependency does not have a crates.io registry identity: {manifest}")
        expected = artifact or consumer.parent / "source" / f"{package['name']}-{package['version']}"
        if manifest != (expected / "Cargo.toml").resolve():
            raise ValueError(f"Dependency resolved outside its staged registry source: {manifest}")
        if artifact is not None:
            version = read_manifest(artifact / "Cargo.toml")["package"]["version"]
            if package["name"] in seen or package["version"] != version:
                raise ValueError(f"Mixed or substituted artifact source for {package['name']}: {manifest}")
            seen.add(package["name"])
    if missing := required - seen:
        raise ValueError(f"Missing maintained artifacts in registry resolution: {sorted(missing)}")


def validate_registry_manifest(manifest: dict) -> None:
    if "patch" in manifest or "replace" in manifest:
        raise ValueError("Rehearsal manifests must not contain patch or replace tables")
    for table in [manifest, *manifest.get("target", {}).values()]:
        for kind in ("dependencies", "build-dependencies", "dev-dependencies"):
            for name, dependency in table.get(kind, {}).items():
                if isinstance(dependency, dict) and (
                    "version" not in dependency
                    or any(key in dependency for key in ("path", "git", "registry", "registry-index"))
                ):
                    raise ValueError(f"Dependency {name} is not a version-only registry dependency")


def extract_archive(archive: Path, destination: Path, package: dict) -> Path:
    directory = destination / f"{package['name']}-{package['version']}"
    if directory.exists():
        raise ValueError(f"Duplicate package source directory: {directory}")
    with tarfile.open(archive, "r:gz") as bundle:
        # Cargo archives contain regular files and directories. Refuse links and
        # traversal instead of allowing extraction to leave this run's staging area.
        members = bundle.getmembers()
        for member in members:
            path = (destination / member.name).resolve()
            if not (member.isfile() or member.isdir()) or not path.is_relative_to(directory):
                raise ValueError(f"Unexpected Cargo archive member: {member.name}")
        bundle.extractall(destination, members=members, filter="data")
    manifest = read_manifest(directory / "Cargo.toml")
    if (manifest["package"]["name"], manifest["package"]["version"]) != (
        package["name"], package["version"]
    ):
        raise ValueError(f"Unexpected package identity in {archive}")
    return directory


def unpack(archive: Path, destination: Path, package: dict) -> Path:
    directory = extract_archive(archive, destination, package)
    manifest = read_manifest(directory / "Cargo.toml")
    validate_registry_manifest(manifest)
    if manifest["package"].get("readme") != "README.md":
        raise ValueError(f"Package {package['name']} must declare its own README.md")
    documents = ["README.md", "LICENSE-MIT", "LICENSE-APACHE"]
    if package["name"] in NATIVE_HELPERS:
        documents.extend(["COPYRIGHT", "LICENSE-Apache-2.0_WITH_LLVM-exception", "PATCHES.md"])
    for filename in documents:
        document = directory / filename
        if not document.is_file() or document.stat().st_size == 0:
            raise ValueError(f"Package {package['name']} is missing {filename}")
    write_checksum(directory, file_sha256(archive))
    return directory


def file_sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_checksum(directory: Path, package_checksum: str) -> None:
    files = {
        file.relative_to(directory).as_posix(): file_sha256(file)
        for file in sorted(directory.rglob("*"))
        if file.is_file() and file != directory / ".cargo-checksum.json"
    }
    (directory / ".cargo-checksum.json").write_text(json.dumps({
        "files": files, "package": package_checksum,
    }, sort_keys=True) + "\n", encoding="utf-8")


def snapshot_registry(metadata: list[dict], destination: Path, maintained: set[str]) -> None:
    """Extract Cargo-selected registry archives only after checking their locked digests."""
    for graph in metadata:
        lock = read_manifest(Path(graph["workspace_root"]) / "Cargo.lock")
        lock_checksums = {
            (item["name"], item["version"], item.get("source")): item["checksum"]
            for item in lock["package"] if "checksum" in item
        }
        for package in graph["packages"]:
            if package["source"] is None:
                continue
            if package["source"] != CRATES_IO or package["name"] in maintained:
                raise ValueError(f"Unexpected or mixed source in input graph: {package['id']}")
            checksum = lock_checksums.get((package["name"], package["version"], CRATES_IO))
            if checksum is None:
                raise ValueError(f"Registry package has no locked checksum: {package['id']}")
            directory = destination / f"{package['name']}-{package['version']}"
            if directory.exists():
                staged_checksum = json.loads((directory / ".cargo-checksum.json").read_text(
                    encoding="utf-8"))["package"]
                if staged_checksum != checksum:
                    raise ValueError(f"Conflicting registry archive checksums: {package['id']}")
                continue
            source = Path(package["manifest_path"]).resolve().parent
            if source.parent.parent.name != "src":
                raise ValueError(f"Expected a Cargo registry/src cache path: {source}")
            archive = (source.parent.parent.parent / "cache" / source.parent.name
                       / f"{package['name']}-{package['version']}.crate")
            # The shared cache may change between reads. Verify and extract one
            # run-owned copy so its file checksums bind to the locked archive.
            private_archive = destination.parent / "registry-archives" / archive.name
            private_archive.parent.mkdir(exist_ok=True)
            shutil.copyfile(archive, private_archive)
            archive = private_archive
            if file_sha256(archive) != checksum:
                raise ValueError(f"Registry archive checksum does not match source lock: {archive}")
            write_checksum(extract_archive(archive, destination, package), checksum)


def stage_package_inputs(packages: list[dict], metadata: list[dict], root: Path,
                         destination: Path) -> dict[str, Path]:
    """Detach excluded packages while preserving actual sibling dependency paths."""
    members = {member for graph in metadata for member in graph["workspace_members"]}
    manifests = {}
    detached = []
    for package in packages:
        manifest = Path(package["manifest_path"]).resolve()
        if not manifest.is_relative_to(root):
            raise ValueError(f"Local package is outside the repository: {manifest}")
        if package["id"] not in members:
            relative = manifest.parent.relative_to(root)
            shutil.copytree(manifest.parent, destination / relative,
                            ignore=shutil.ignore_patterns("target", ".git", "__pycache__", "Cargo.lock"))
            manifest = destination / relative / "Cargo.toml"
            detached.append(relative.as_posix())
        manifests[package["name"]] = manifest
    if detached:
        (destination / "Cargo.toml").write_text(
            '[workspace]\nresolver = "3"\nmembers = ' + json.dumps(detached) + "\n",
            encoding="utf-8",
        )
    return manifests


def run(command: list[str], label: str, root: Path, environment: dict[str, str],
        capture: bool = False) -> subprocess.CompletedProcess:
    print(f"\n{label}\n{subprocess.list2cmdline(command)}", flush=True)
    return subprocess.run(command, cwd=root, env=environment, check=True,
                          text=True, stdout=subprocess.PIPE if capture else None,
                          encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--case", choices=CASES, action="append", dest="cases",
        help="Repeat to select cases from the maintained consumer matrix",
    )
    parser.add_argument("--toolchain", help="Rust toolchain name, for example 1.95.0")
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--target", help="Cargo target triple")
    parser.add_argument("--check", action="store_true", help="Compile only; no runtime claim")
    parser.add_argument("--offline", action="store_true",
                        help="Gather input metadata offline too; all archives and builds are offline")
    parser.add_argument("--native", action="store_true",
                        help="Also rehearse the unregistered native candidate and all helper features")
    parser.add_argument("--gltf-conformance", action="store_true",
                        help="Also test direct glTF/import/IO archive roots and the pinned reference")
    parser.add_argument("--features", choices=["repository-capture"],
                        help="Also compile this native feature; runtime exercises only the baseline")
    args = parser.parse_args()
    if args.features and not args.native:
        parser.error("--features requires --native")
    root = Path(__file__).resolve().parents[1]
    source = root / "tools/consumers/scene"
    validate_consumer_features(root)
    try:
        cases = select_cases(args.cases or list(CASES))
    except ValueError as error:
        parser.error(str(error))

    cargo = ["cargo", *([f"+{args.toolchain}"] if args.toolchain else [])]
    environment = os.environ.copy()
    environment["CARGO_BUILD_JOBS"] = "1"
    metadata = []
    packages = []
    native_packages = []
    inputs = [(root / "Cargo.toml", CONSUMER_DEPENDENCIES)]
    if args.native:
        inputs.append((root / "candidates/baozi-native-io/Cargo.toml", (NATIVE_DEPENDENCY,)))
    if args.gltf_conformance:
        inputs.append((root / "tools/consumers/gltf-conformance/Cargo.toml",
                       GLTF_CONSUMER_DEPENDENCIES))
    for manifest, roots in inputs:
        result = run(cargo + [
            "metadata", "--locked", "--format-version", "1", "--all-features",
            "--manifest-path", str(manifest), *(["--offline"] if args.offline else []),
        ], "Gather package closure (source manifests and lockfiles stay unchanged)",
            root, environment, capture=True)
        graph = json.loads(result.stdout)
        metadata.append(graph)
        closure = package_order(graph, roots)
        if roots == (NATIVE_DEPENDENCY,):
            native_packages = closure
        for package in closure:
            previous = next((item for item in packages if item["name"] == package["name"]), None)
            if previous is not None and previous["id"] != package["id"]:
                raise ValueError(f"Mixed maintained package identities: {package['name']}")
            if previous is None:
                packages.append(package)
    target = (root / "target").resolve()
    if not target.is_relative_to(root):
        raise ValueError(f"Staging target is outside the repository: {target}")
    target.mkdir(exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix="package-check-", dir=target))
    print(f"Staging (retained): {staging}", flush=True)
    (staging / "archives").mkdir()
    directory_source = staging / "source"
    directory_source.mkdir()
    package_workspace = staging / "package-inputs"
    package_workspace.mkdir()
    manifests = stage_package_inputs(packages, metadata, root, package_workspace)
    snapshot_metadata = list(metadata)
    helper_features = {}
    if args.native:
        helper_workspace = staging / "helper-inputs"
        helper_workspace.mkdir()
        helpers = [package for package in native_packages if package["name"] in NATIVE_HELPERS]
        if len(helpers) != len(NATIVE_HELPERS):
            raise ValueError("Native metadata must include all three maintained cap helpers")
        stage_package_inputs(helpers, metadata, root, helper_workspace)
        if not args.offline:
            run(cargo + ["fetch", "--manifest-path", str(helper_workspace / "Cargo.toml")],
                "Gather optional helper registry archives (staged workspace only)",
                helper_workspace, environment)
        # All three copied helpers are members of this virtual workspace, so each gets all features.
        result = run(cargo + [
            "metadata", "--offline", "--format-version", "1", "--all-features",
            "--manifest-path", str(helper_workspace / "Cargo.toml"),
        ], "Resolve all three copied helpers with all features; historical checkout locks stay untouched",
            helper_workspace, environment, capture=True)
        helper_graph = json.loads(result.stdout)
        # These local IDs belong to copies, so use this graph only for registry inputs and features.
        snapshot_metadata.append(helper_graph)
        helper_features = {package["name"]: tuple(sorted(package["features"]))
                           for package in helper_graph["packages"] if package["name"] in NATIVE_HELPERS}
    print("Production package closure: " + " -> ".join(package["name"] for package in packages))
    print("Archive creation skips verification and generated package lockfiles.")
    print("Offline registry rehearsal only; no network registry or publication proof.")
    print("Staged locks transition to registry identities once; builds use --locked --offline.",
          flush=True)
    snapshot_registry(snapshot_metadata, directory_source, {package["name"] for package in packages})
    artifacts = {}
    for package in packages:
        manifest = manifests[package["name"]]
        run(cargo + [
            "package", "--locked", "--offline", "--allow-dirty", "--no-verify", "--exclude-lockfile",
            "--manifest-path", str(manifest), "--target-dir", str(target),
        ], f"Archive: {package['name']} {package['version']} (no build)",
            manifest.parent, environment)
        filename = f"{package['name']}-{package['version']}.crate"
        archive = staging / "archives" / filename
        shutil.copy2(target / "package" / filename, archive)
        artifacts[package["name"]] = unpack(archive, directory_source, package)

    consumer = staging / "consumer"
    stage_consumer(source, consumer, artifacts)
    consumers = [(consumer, set(package["name"] for package in package_order(metadata[0])))]
    if args.gltf_conformance:
        gltf_consumer = staging / "gltf-consumer"
        stage_gltf_consumer(root / "tools/consumers/gltf-conformance", gltf_consumer, artifacts)
        consumers.append((gltf_consumer,
                          set(GLTF_CONSUMER_DEPENDENCIES) | {"baozi-core", "baozi-operation"}))
    if args.native:
        native_consumer = staging / "native-consumer"
        stage_native_consumer(native_consumer, artifacts)
        consumers.append((native_consumer, {package["name"] for package in native_packages}
                          | {"baozi", "baozi-import", "baozi-format-stl"}))
        helper_consumer = staging / "helper-consumer"
        stage_helper_consumer(helper_consumer, artifacts, helper_features)
        consumers.append((helper_consumer, set(NATIVE_HELPERS)))
    artifact_target = staging / "build"
    artifact_build = staging / "build-intermediates"
    config = staging / "rehearsal.toml"
    config.write_text(
        '[build]\nbuild-dir = ' + json.dumps(artifact_build.as_posix()) + '\n\n'
        '[source.crates-io]\nreplace-with = "baozi-offline-rehearsal"\n\n'
        '[source.baozi-offline-rehearsal]\ndirectory = '
        + json.dumps(directory_source.as_posix()) + "\n", encoding="utf-8",
    )
    rehearsal_environment = environment.copy()
    rehearsal_environment["CARGO_HOME"] = str(staging / "cargo-home")
    rehearsal_environment["CARGO_BUILD_BUILD_DIR"] = str(artifact_build)
    # Command-line configuration takes precedence over inherited source replacements.
    rehearsal_cargo = cargo + ["--config", str(config)]
    for staged, required in consumers:
        validate_registry_manifest(read_manifest(staged / "Cargo.toml"))
        resolved = run(rehearsal_cargo + [
            "metadata", "--offline", "--format-version", "1", "--all-features",
            "--manifest-path", str(staged / "Cargo.toml"),
        ], "Transition staged lock to registry identities and verify exact archive sources",
            staged, rehearsal_environment, capture=True)
        verify_artifact_sources(json.loads(resolved.stdout), staged, artifacts, required)
    # Compile archive contents in a fresh directory, independent of checkout build artifacts.
    operation = "check" if args.check else "build"
    builds = [(consumer, name, features, [expected], operation) for name, features, expected in cases]
    if args.gltf_conformance:
        builds.append((gltf_consumer, "direct glTF conformance consumer", ["--all-targets"],
                       None, "check" if args.check else "test"))
    if args.native:
        builds.append((native_consumer, "native candidate baseline", [], [], operation))
        if args.features:
            builds.append((native_consumer, "native candidate repository-capture (compile only)",
                           ["--features", args.features], None, operation))
        builds.append((helper_consumer, "all helper features (compile only; no native profile claim)",
                       ["--all-features"], None, "check"))
    for staged, name, features, runtime_args, operation in builds:
        options = [
            "--locked", "--offline", "--manifest-path", str(staged / "Cargo.toml"),
            "--target-dir", str(artifact_target), *features,
        ]
        if args.release:
            options.append("--release")
        if args.target:
            options.extend(["--target", args.target])
        run(rehearsal_cargo + [operation, *options],
            f"Offline registry rehearsal / {operation}: {name}",
            staged, rehearsal_environment)
        if not args.check and runtime_args is not None:
            run(rehearsal_cargo + ["run", *options, "--", *runtime_args],
                f"Offline registry rehearsal / execute: {name}", staged, rehearsal_environment)
    mode = "compile only; no runtime claim" if args.check else "build and baseline execution"
    print(f"\nAll selected offline registry rehearsal cases passed ({mode}).", flush=True)
    if args.features:
        print("Native repository-capture was compiled only; capture was not invoked.", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, tarfile.TarError, subprocess.CalledProcessError) as error:
        print(f"Package consumer check failed: {error}", file=sys.stderr)
        sys.exit(1)
