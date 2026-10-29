"""Run a fixed public-consumer matrix without workspace feature unification."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess


CASES: dict[str, tuple[list[str], str]] = {
    "empty": (["--no-default-features"], ""),
    "default": ([], "obj,stl"),
    "default-formats": (["--no-default-features", "--features", "default-formats"], "obj,stl"),
    "obj": (["--no-default-features", "--features", "format-obj"], "obj"),
    "ply": (["--no-default-features", "--features", "format-ply"], "ply"),
    "stl": (["--no-default-features", "--features", "format-stl"], "stl"),
    "all-formats": (["--no-default-features", "--features", "all-formats"], "obj,ply,stl"),
    "tracing": (["--no-default-features", "--features", "tracing"], ""),
    "mint": (["--no-default-features", "--features", "mint"], ""),
    "all": (["--all-features"], "obj,ply,stl"),
}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", choices=CASES, action="append", dest="cases")
    parser.add_argument("--toolchain", help="Rust toolchain name, for example 1.95.0")
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--target", help="Cargo target triple")
    parser.add_argument("--check", action="store_true", help="Compile only; no runtime claim")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    environment = os.environ.copy()
    environment["CARGO_BUILD_JOBS"] = "1"

    for name in args.cases or list(CASES):
        features, expected = CASES[name]
        command = ["cargo"]
        if args.toolchain:
            command.append(f"+{args.toolchain}")
        command.extend([
            "check" if args.check else "run", "--locked",
            "--manifest-path", str(root / "tools/consumers/scene/Cargo.toml"),
            "--target-dir", str(root / "target"),
        ])
        command.extend(features)
        if args.release:
            command.append("--release")
        if args.target:
            command.extend(["--target", args.target])
        if not args.check:
            command.extend(["--", expected])
        print(f"Consumer case: {name} ({'compile' if args.check else 'runtime'})", flush=True)
        subprocess.run(command, cwd=root, env=environment, check=True)


if __name__ == "__main__":
    main()
