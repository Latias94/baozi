"""Run the maintained scene consumer against unpacked Cargo package artifacts."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib

from check_features import CASES


FORMATS = {"gltf", "obj", "ply", "stl"}
CONSUMER_DEPENDENCIES = ("baozi", "baozi-import")


def read_manifest(path: Path) -> dict:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def package_order(metadata: dict) -> list[dict]:
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

    for name in CONSUMER_DEPENDENCIES:
        matches = [key for key, package in packages.items() if package["name"] == name]
        if len(matches) != 1:
            raise ValueError(f"Expected one local {name} package in Cargo metadata")
        visit(matches[0])
    return ordered


def select_cases(names: list[str]) -> list[tuple[str, list[str], str]]:
    cases = []
    for name in dict.fromkeys(names):
        if name in CASES:
            flags, expected = CASES[name]
            flags = list(flags)
        else:
            formats = name.split(",")
            if not all(formats) or len(set(formats)) != len(formats):
                raise ValueError(f"Invalid format combination: {name}")
            flags = ["--no-default-features", "--features",
                     ",".join(f"format-{format_name}" for format_name in formats)]
            expected = ",".join(sorted(formats))
        unadmitted = set(filter(None, expected.split(","))) - FORMATS
        if unadmitted:
            raise ValueError(
                f"Case {name} contains unsupported formats: {','.join(sorted(unadmitted))}"
            )
        cases.append((name, flags, expected))
    return cases


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
        lines.append(
            f'{name} = {{ path = {json.dumps(artifacts[name].as_posix(), ensure_ascii=False)}, '
            "default-features = false }"
        )
    lines.extend(["", "[features]"])
    # Copy the maintained forwarding lists verbatim; Cargo interprets the features.
    lines.extend(f"{json.dumps(name, ensure_ascii=False)} = {json.dumps(features, ensure_ascii=False)}"
                 for name, features in consumer["features"].items())
    lines.extend(["", "[patch.crates-io]"])
    # The facade is the direct root, so patching it would create an unused lock entry.
    lines.extend(
        f"{json.dumps(name)} = {{ path = {json.dumps(directory.as_posix(), ensure_ascii=False)} }}"
        for name, directory in sorted(artifacts.items()) if name != "baozi"
    )
    (destination / "Cargo.toml").write_text("\n".join(lines) + "\n", encoding="utf-8")


def verify_artifact_sources(metadata: dict, consumer: Path, artifacts: dict[str, Path]) -> None:
    """Reject Cargo configuration that substitutes checkout paths for archive contents."""
    consumer_manifest = (consumer / "Cargo.toml").resolve()
    for package in metadata["packages"]:
        manifest = Path(package["manifest_path"]).resolve()
        if manifest == consumer_manifest:
            continue
        artifact = artifacts.get(package["name"])
        if package["source"] is None:
            if artifact is None or manifest != (artifact / "Cargo.toml").resolve():
                raise ValueError(f"Resolved local dependency is not an extracted artifact: {manifest}")
        elif artifact is not None:
            raise ValueError(f"Resolved {package['name']} from a registry instead of its artifact")


def unpack(archive: Path, destination: Path, package: dict) -> Path:
    directory = destination / f"{package['name']}-{package['version']}"
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


def run(command: list[str], label: str, root: Path, environment: dict[str, str],
        capture: bool = False) -> subprocess.CompletedProcess:
    print(f"\n{label}\n{subprocess.list2cmdline(command)}", flush=True)
    return subprocess.run(command, cwd=root, env=environment, check=True,
                          text=True, stdout=subprocess.PIPE if capture else None,
                          encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--case", action="append", dest="cases",
        help=f"Repeat a fixed case ({', '.join(CASES)}) or a format combination, e.g. obj,ply",
    )
    parser.add_argument("--toolchain", help="Rust toolchain name, for example 1.95.0")
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--target", help="Cargo target triple")
    parser.add_argument("--check", action="store_true", help="Compile only; no runtime claim")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    source = root / "tools/consumers/scene"
    try:
        cases = select_cases(args.cases or list(CASES))
    except ValueError as error:
        parser.error(str(error))

    cargo = ["cargo", *([f"+{args.toolchain}"] if args.toolchain else [])]
    environment = os.environ.copy()
    environment["CARGO_BUILD_JOBS"] = "1"
    result = run(cargo + [
        "metadata", "--locked", "--format-version", "1", "--all-features",
        "--manifest-path", str(root / "Cargo.toml"),
    ], "Resolve package closure (one Cargo metadata call; no build)", root, environment,
        capture=True)
    packages = package_order(json.loads(result.stdout))
    target = (root / "target").resolve()
    if not target.is_relative_to(root):
        raise ValueError(f"Staging target is outside the repository: {target}")
    target.mkdir(exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix="package-check-", dir=target))
    (staging / "archives").mkdir()
    (staging / "unpacked").mkdir()
    package_workspace = staging / "package-inputs"
    package_workspace.mkdir()
    vendored = {"baozi-gltf": "gltf", "baozi-gltf-json": "gltf-json"}
    for package_dir in vendored.values():
        shutil.copytree(root / "vendor" / package_dir, package_workspace / package_dir)
    (package_workspace / "Cargo.toml").write_text(
        '[workspace]\nresolver = "3"\nmembers = ["gltf", "gltf-json"]\n',
        encoding="utf-8",
    )
    print(f"Staging (retained): {staging}", flush=True)
    print("Production package closure: " + " -> ".join(package["name"] for package in packages))
    print("Archive creation skips verification and generated package lockfiles.")
    print("Consumer builds use the unchanged maintained Cargo.lock and --locked.", flush=True)
    artifacts = {}
    for package in packages:
        manifest = Path(package["manifest_path"]).resolve()
        if not manifest.is_relative_to(root):
            raise ValueError(f"Local package is outside the repository: {manifest}")
        package_root = root
        if package["name"] in vendored:
            manifest = package_workspace / vendored[package["name"]] / "Cargo.toml"
            package_root = package_workspace
        run(cargo + [
            "package", "--locked", "--allow-dirty", "--no-verify", "--exclude-lockfile",
            "--manifest-path", str(manifest), "--target-dir", str(target),
        ], f"Archive: {package['name']} {package['version']} (no build)", package_root, environment)
        filename = f"{package['name']}-{package['version']}.crate"
        archive = staging / "archives" / filename
        shutil.copy2(target / "package" / filename, archive)
        artifacts[package["name"]] = unpack(archive, staging / "unpacked", package)

    consumer = staging / "consumer"
    stage_consumer(source, consumer, artifacts)
    resolved = run(cargo + [
        "metadata", "--locked", "--format-version", "1", "--all-features",
        "--manifest-path", str(consumer / "Cargo.toml"),
    ], "Verify Cargo resolves local dependencies only from extracted artifacts", root, environment,
        capture=True)
    verify_artifact_sources(json.loads(resolved.stdout), consumer, artifacts)
    # Compile archive contents from a fresh build directory. Source checkouts with the same
    # package identity must not satisfy this publication check through cached build artifacts.
    artifact_target = staging / "build"
    for name, features, expected in cases:
        options = [
            "--locked", "--manifest-path", str(consumer / "Cargo.toml"),
            "--target-dir", str(artifact_target), *features,
        ]
        if args.release:
            options.append("--release")
        if args.target:
            options.extend(["--target", args.target])
        run(cargo + ["check" if args.check else "build", *options],
            f"Packaged dependencies / consumer {'check' if args.check else 'build'}: {name}",
            root, environment)
        if not args.check:
            run(cargo + ["run", *options, "--", expected],
                f"Consumer execute: {name} (expected formats: {expected or 'none'})",
                root, environment)
    print(
        "\nAll selected packaged-consumer cases passed "
        f"({'compile only; no runtime claim' if args.check else 'build and execute'}).",
        flush=True,
    )


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, tarfile.TarError, subprocess.CalledProcessError) as error:
        print(f"Package consumer check failed: {error}", file=sys.stderr)
        sys.exit(1)
