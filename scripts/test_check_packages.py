"""Check registry rehearsal boundaries without invoking Cargo or the network."""

import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from check_packages import (
    CRATES_IO,
    file_sha256,
    read_manifest,
    snapshot_registry,
    stage_consumer,
    stage_helper_consumer,
    stage_package_inputs,
    unpack,
    validate_registry_manifest,
    verify_artifact_sources,
)


class RegistryRehearsalTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def artifact(self, name="baozi"):
        directory = self.root / "source" / f"{name}-0.1.0"
        directory.mkdir(parents=True)
        (directory / "Cargo.toml").write_text(
            f'[package]\nname = "{name}"\nversion = "0.1.0"\n', encoding="utf-8",
        )
        return directory

    def registry_graph(self):
        source = self.root / "registry/src/local-index/example-1.0.0"
        source.mkdir(parents=True)
        files = {
            "Cargo.toml": b'[package]\nname = "example"\nversion = "1.0.0"\n',
            "src/lib.rs": b"// Original registry artifact\n",
        }
        archive = self.root / "registry/cache/local-index/example-1.0.0.crate"
        archive.parent.mkdir(parents=True)
        with tarfile.open(archive, "w:gz") as bundle:
            for filename, contents in files.items():
                entry = tarfile.TarInfo(f"example-1.0.0/{filename}")
                entry.size = len(contents)
                bundle.addfile(entry, io.BytesIO(contents))
        for filename, contents in files.items():
            path = source / filename
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(contents)
        workspace = self.root / "checkout"
        workspace.mkdir()
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        (workspace / "Cargo.lock").write_text(
            '[[package]]\nname = "example"\nversion = "1.0.0"\n'
            f'source = "{CRATES_IO}"\nchecksum = "{checksum}"\n', encoding="utf-8",
        )
        graph = {"workspace_root": str(workspace), "packages": [{
            "id": "example", "name": "example", "version": "1.0.0", "source": CRATES_IO,
            "manifest_path": str(source / "Cargo.toml"),
        }]}
        return graph, archive, source

    def test_archive_checksums_cover_normalized_files_and_real_archive(self):
        files = {
            "Cargo.toml": b'[package]\nname = "baozi"\nversion = "0.1.0"\nreadme = "README.md"\n',
            "Cargo.toml.orig": b'[dependencies]\nexample = { path = "../example", version = "1" }\n',
            "README.md": b"Package documentation\n",
            "LICENSE-MIT": b"MIT license\n",
            "LICENSE-APACHE": b"Apache license\n",
            "src/lib.rs": b"pub fn packaged() {}\n",
        }
        archive = self.root / "baozi-0.1.0.crate"
        with tarfile.open(archive, "w:gz") as bundle:
            for filename, contents in files.items():
                entry = tarfile.TarInfo(f"baozi-0.1.0/{filename}")
                entry.size = len(contents)
                bundle.addfile(entry, io.BytesIO(contents))
        source = self.root / "source"
        source.mkdir()
        directory = unpack(archive, source, {"name": "baozi", "version": "0.1.0"})
        checksums = json.loads((directory / ".cargo-checksum.json").read_text(encoding="utf-8"))
        self.assertEqual(checksums["package"], hashlib.sha256(archive.read_bytes()).hexdigest())
        self.assertEqual(checksums["files"], {
            filename: hashlib.sha256(contents).hexdigest() for filename, contents in files.items()
        })

    def test_native_helper_archives_require_retained_upstream_and_patch_documents(self):
        required = ("COPYRIGHT", "LICENSE-Apache-2.0_WITH_LLVM-exception", "PATCHES.md")
        for name in ("baozi-cap-primitives", "baozi-cap-std", "baozi-cap-fs-ext"):
            for omitted in (*required, None):
                with self.subTest(package=name, omitted=omitted):
                    files = {
                        "Cargo.toml": (f'[package]\nname = "{name}"\nversion = "0.1.0"\n'
                                       'readme = "README.md"\n').encode(),
                        **{document: b"Retained documentation\n" for document in (
                            "README.md", "LICENSE-MIT", "LICENSE-APACHE", *required,
                        ) if document != omitted},
                    }
                    destination = self.root / f"{name}-{omitted or 'complete'}"
                    destination.mkdir()
                    archive = destination / f"{name}-0.1.0.crate"
                    with tarfile.open(archive, "w:gz") as bundle:
                        for filename, contents in files.items():
                            entry = tarfile.TarInfo(f"{name}-0.1.0/{filename}")
                            entry.size = len(contents)
                            bundle.addfile(entry, io.BytesIO(contents))
                    package = {"name": name, "version": "0.1.0"}
                    if omitted is None:
                        unpack(archive, destination, package)
                    else:
                        with self.assertRaisesRegex(ValueError, f"missing {omitted}"):
                            unpack(archive, destination, package)

    def test_registry_snapshot_extracts_archive_instead_of_blessing_modified_source(self):
        graph, archive, cached = self.registry_graph()
        modified = b"// Modified mutable registry/src cache\n"
        (cached / "src/lib.rs").write_bytes(modified)
        (cached / ".cargo-checksum.json").write_bytes(b"untrusted cached checksum\n")
        snapshot = self.root / "source"
        snapshot.mkdir()
        snapshot_registry([graph, graph], snapshot, {"baozi"})
        self.assertEqual((snapshot / "example-1.0.0/src/lib.rs").read_bytes(),
                         b"// Original registry artifact\n")
        self.assertEqual((cached / "src/lib.rs").read_bytes(), modified)
        checksums = json.loads((snapshot / "example-1.0.0/.cargo-checksum.json").read_text())
        self.assertEqual(checksums["package"], hashlib.sha256(archive.read_bytes()).hexdigest())
        self.assertEqual(checksums["files"]["src/lib.rs"],
                         hashlib.sha256(b"// Original registry artifact\n").hexdigest())

    def test_registry_snapshot_rejects_archive_that_disagrees_with_source_lock(self):
        graph, archive, cached = self.registry_graph()
        archive.write_bytes(archive.read_bytes() + b"modified archive")
        snapshot = self.root / "source"
        snapshot.mkdir()
        with self.assertRaisesRegex(ValueError, "checksum does not match source lock"):
            snapshot_registry([graph], snapshot, {"baozi"})
        self.assertFalse((snapshot / "example-1.0.0").exists())
        self.assertFalse((cached / ".cargo-checksum.json").exists())

    def test_shared_cache_replacement_after_checksum_cannot_change_extracted_source(self):
        graph, archive, _ = self.registry_graph()
        replacement = io.BytesIO()
        with tarfile.open(fileobj=replacement, mode="w:gz") as bundle:
            for filename, contents in {
                "Cargo.toml": b'[package]\nname = "example"\nversion = "1.0.0"\n',
                "src/lib.rs": b"// Replaced shared registry archive\n",
            }.items():
                entry = tarfile.TarInfo(f"example-1.0.0/{filename}")
                entry.size = len(contents)
                bundle.addfile(entry, io.BytesIO(contents))

        def replace_shared_cache_after_digest(path):
            digest = file_sha256(path)
            if path.name == archive.name:
                archive.write_bytes(replacement.getvalue())
            return digest

        snapshot = self.root / "source"
        snapshot.mkdir()
        with patch("check_packages.file_sha256", side_effect=replace_shared_cache_after_digest):
            snapshot_registry([graph], snapshot, {"baozi"})
        self.assertEqual((snapshot / "example-1.0.0/src/lib.rs").read_bytes(),
                         b"// Original registry artifact\n")
        checksums = json.loads((snapshot / "example-1.0.0/.cargo-checksum.json").read_text())
        original_digest = read_manifest(Path(graph["workspace_root"]) / "Cargo.lock")["package"][0]["checksum"]
        self.assertEqual(checksums["package"], original_digest)
        self.assertNotEqual(file_sha256(archive), original_digest)

    def test_scene_manifest_uses_versions_and_preserves_source_lock(self):
        source = self.root / "scene"
        source.mkdir()
        manifest = '''[package]
name = "consumer"
version = "0.0.0"
edition = "2024"
rust-version = "1.95"
[dependencies]
baozi = { path = "../../crates/baozi", default-features = false }
baozi-import = { path = "../../crates/baozi-import", default-features = false }
[features]
default = ["baozi/default"]
'''
        (source / "Cargo.toml").write_text(manifest, encoding="utf-8")
        (source / "Cargo.lock").write_bytes(b"original lockfile\n")
        artifacts = {name: self.artifact(name) for name in ("baozi", "baozi-import")}
        destination = self.root / "consumer"
        stage_consumer(source, destination, artifacts)
        staged = read_manifest(destination / "Cargo.toml")
        validate_registry_manifest(staged)
        self.assertEqual(staged["dependencies"]["baozi"]["version"], "=0.1.0")
        self.assertNotIn("patch", staged)
        self.assertEqual(staged["features"], {"default": ["baozi/default"]})
        self.assertEqual((source / "Cargo.toml").read_text(encoding="utf-8"), manifest)
        self.assertEqual((source / "Cargo.lock").read_bytes(), b"original lockfile\n")

    def test_artifact_resolution_requires_registry_identity_and_exact_manifest(self):
        artifact = self.artifact()
        consumer = self.root / "consumer"
        package = {"name": "baozi", "version": "0.1.0", "source": CRATES_IO,
                   "manifest_path": str(artifact / "Cargo.toml")}
        artifacts = {"baozi": artifact}
        verify_artifact_sources({"packages": [package]}, consumer, artifacts, {"baozi"})
        invalid = [
            [],
            [{**package, "source": None}],
            [{**package, "version": "0.2.0"}],
            [{**package, "manifest_path": str(self.root / "checkout/Cargo.toml")}],
            [package, {**package, "manifest_path": str(self.root / "other/Cargo.toml")}],
            [package, {"name": "unexpected", "source": None,
                       "manifest_path": str(self.root / "unexpected/Cargo.toml")}],
            [package, {"name": "unexpected", "version": "1.0.0", "source": CRATES_IO,
                       "manifest_path": str(self.root / "checkout/registry/Cargo.toml")}],
        ]
        for packages in invalid:
            with self.subTest(packages=packages), self.assertRaises(ValueError):
                verify_artifact_sources({"packages": packages}, consumer, artifacts, {"baozi"})

    def test_helper_features_stay_in_separate_registry_consumer_without_artifact_edits(self):
        features = {
            "baozi-cap-primitives": ("arbitrary",),
            "baozi-cap-std": ("arf-strings", "arf_strings", "camino", "fs_utf8"),
            "baozi-cap-fs-ext": ("arf_strings", "fs_utf8", "std"),
        }
        artifacts = {name: self.artifact(name) for name in features}
        originals = {name: (directory / "Cargo.toml").read_bytes()
                     for name, directory in artifacts.items()}
        destination = self.root / "helper-consumer"
        stage_helper_consumer(destination, artifacts, features)
        manifest = read_manifest(destination / "Cargo.toml")
        validate_registry_manifest(manifest)
        self.assertEqual(set(manifest["dependencies"]), set(features))
        for name, enabled in features.items():
            self.assertEqual(manifest["dependencies"][name]["features"], list(enabled))
            self.assertEqual((artifacts[name] / "Cargo.toml").read_bytes(), originals[name])

    def test_detached_workspace_uses_metadata_paths_and_keeps_sibling_edges(self):
        packages = []
        for name in ("first", "second"):
            directory = self.root / "checkout/candidate/vendor" / name
            directory.mkdir(parents=True)
            (directory / "Cargo.toml").write_text(
                f'[package]\nname = "{name}"\nversion = "0.1.0"\n'
                + ('[dependencies]\nsecond = { path = "../second", version = "0.1.0" }\n'
                   if name == "first" else ""), encoding="utf-8",
            )
            (directory / "Cargo.lock").write_bytes(b"historical upstream lock\n")
            packages.append({"id": name, "name": name,
                             "manifest_path": str(directory / "Cargo.toml")})
        destination = self.root / "package-inputs"
        destination.mkdir()
        manifests = stage_package_inputs(packages, [{"workspace_members": []}],
                                         self.root / "checkout", destination)
        self.assertEqual(read_manifest(destination / "Cargo.toml")["workspace"]["members"],
                         ["candidate/vendor/first", "candidate/vendor/second"])
        dependency = read_manifest(manifests["first"])["dependencies"]["second"]["path"]
        self.assertEqual((manifests["first"].parent / dependency / "Cargo.toml").resolve(),
                         manifests["second"])
        for package in packages:
            original_lock = Path(package["manifest_path"]).parent / "Cargo.lock"
            self.assertEqual(original_lock.read_bytes(), b"historical upstream lock\n")
            self.assertFalse((manifests[package["name"]].parent / "Cargo.lock").exists())

    def test_normalized_manifests_reject_patches_and_dependency_paths(self):
        for manifest in [
            {"patch": {}}, {"replace": {}},
            {"dependencies": {"example": {"path": "../example", "version": "1"}}},
            {"target": {"cfg(windows)": {
                "build-dependencies": {"example": {"path": "../example", "version": "1"}},
            }}},
        ]:
            with self.subTest(manifest=manifest), self.assertRaises(ValueError):
                validate_registry_manifest(manifest)


if __name__ == "__main__":
    unittest.main()
