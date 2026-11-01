"""Build the standalone asset consumer and collect advisory per-process measurements."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
CONSUMER = ROOT / "tools/consumers/assets"
GNU_TIME = Path("/usr/bin/time")


def consumer_executable(build_log: Path) -> str:
    """Use Cargo's executable artifact instead of guessing a target layout."""
    executables = set()
    for line in build_log.read_text(encoding="utf-8").splitlines():
        message = json.loads(line)
        target = message.get("target", {})
        if (
            message.get("reason") == "compiler-artifact"
            and target.get("name") == "baozi-asset-consumer"
            and "bin" in target.get("kind", []) and message.get("executable")
        ):
            executables.add(message["executable"])
    if len(executables) != 1:
        raise RuntimeError(f"expected one asset consumer executable, found {len(executables)}")
    executable = executables.pop()
    if not Path(executable).is_file():
        raise RuntimeError(f"asset consumer executable is missing: {executable}")
    return executable


def read_peak_rss(path: Path) -> tuple[int | None, str | None]:
    """GNU time %M reports the individual child's peak RSS in KiB on Linux."""
    try:
        kib = int(path.read_text(encoding="utf-8").strip())
        if kib > 0:
            return kib * 1024, None
    except (OSError, ValueError):
        pass
    return None, "GNU time did not provide a positive peak RSS value"


def collect(arguments: argparse.Namespace, artifact_root: Path, report: dict) -> None:
    environment = os.environ.copy()
    environment.setdefault("CARGO_BUILD_JOBS", "1")
    environment["CARGO_TERM_COLOR"] = "never"
    report["cargo_build_jobs"] = environment["CARGO_BUILD_JOBS"]
    time_tool = str(GNU_TIME) if sys.platform == "linux" and GNU_TIME.is_file() else None
    report["rss_tool"] = time_tool

    def run(name: str, command: list[str], asset: str | None = None) -> dict:
        record = {
            "name": name, "stdout": f"{name}.stdout.log", "stderr": f"{name}.stderr.log",
            "returncode": None,
        }
        rss_path = artifact_root / f"{name}.rss-kib.txt"
        if asset is not None:
            record.update(
                asset=asset, consumer_argv=command, summary=None, peak_rss_bytes=None,
                rss_unavailable_reason=f"requires Linux with /usr/bin/time; platform={sys.platform}",
            )
            if time_tool:
                command = [time_tool, "-q", "-f", "%M", "-o", str(rss_path), "--", *command]
        record["argv"] = command
        report["commands"].append(record)
        print(f"{name}: logs in {artifact_root / record['stdout']}", flush=True)
        started = time.monotonic()
        try:
            with ((artifact_root / record["stdout"]).open("xb") as stdout,
                  (artifact_root / record["stderr"]).open("xb") as stderr):
                result = subprocess.run(command, cwd=ROOT, env=environment,
                                        stdout=stdout, stderr=stderr, check=False)
            record["returncode"] = result.returncode
        finally:
            record["elapsed_seconds"] = round(time.monotonic() - started, 6)
            if asset is not None and time_tool:
                record["peak_rss_bytes"], record["rss_unavailable_reason"] = read_peak_rss(rss_path)
        result.check_returncode()
        if asset is not None:
            record["summary"] = json.loads((artifact_root / record["stdout"]).read_text(encoding="utf-8"))
            if not isinstance(record["summary"], dict):
                raise ValueError(f"{name}: expected one JSON summary object")
        return record

    report["git_sha"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    report["git_dirty"] = bool(subprocess.check_output(
        ["git", "status", "--porcelain", "--untracked-files=all"], cwd=ROOT))
    toolchain = [f"+{arguments.toolchain}"] if arguments.toolchain else []
    rustc = run("rustc", ["rustc", *toolchain, "-vV"])
    report["rustc_vv"] = (artifact_root / rustc["stdout"]).read_text(encoding="utf-8").strip()
    assets = arguments.asset
    if assets is None:
        fixtures = json.loads((CONSUMER / "fixtures/expectations.json").read_text(encoding="utf-8"))
        assets = [fixture["path"] for fixture in fixtures["fixtures"]
                  if fixture["expected_outcome"] == "success"]
    if not assets:
        raise ValueError("no assets selected")
    command = ["cargo", *toolchain, "build", "--locked", "--release", "--manifest-path",
               str(CONSUMER / "Cargo.toml"), "--target-dir", str(ROOT / "target"),
               "--message-format=json", "--bin", "baozi-asset-consumer"]
    if arguments.allocation_metrics:
        command += ["--features", "allocation-metrics"]
    build = run("build", command)
    executable = consumer_executable(artifact_root / build["stdout"])
    forwarded = ["--assume-standard-space"] if arguments.assume_standard_space else []
    if arguments.max_work_units is not None:
        forwarded += ["--max-work-units", str(arguments.max_work_units)]
    for repetition in range(1, arguments.repetitions + 1):
        for index, asset in enumerate(assets, 1):
            record = run(
                f"asset-{repetition:02d}-{index:03d}",
                [executable, report["root"], asset, *forwarded], asset,
            )
            record["repetition"] = repetition


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    default_root = Path("target/asset-profiles") / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    parser.add_argument("--artifact-root", type=Path, default=default_root,
                        help="new output directory, repository-relative or absolute (default: %(default)s)")
    parser.add_argument("--asset", action="append",
                        help="relative asset path; repeatable (default: successful fixtures)")
    parser.add_argument("--root", type=Path, default=CONSUMER / "fixtures", help="asset scope root")
    parser.add_argument("--toolchain", help="Rust toolchain name, e.g. nightly")
    parser.add_argument("--allocation-metrics", action="store_true", help="enable allocator instrumentation")
    parser.add_argument("--assume-standard-space", action="store_true", help="forward explicit OBJ space assumption")
    parser.add_argument("--max-work-units", type=int, help="forward an explicit consumer work limit")
    parser.add_argument("--repetitions", type=int, default=1,
                        help="fresh consumer processes per selected asset (default: %(default)s)")
    arguments = parser.parse_args()
    if arguments.repetitions < 1:
        parser.error("--repetitions must be positive")
    artifact_root = (ROOT / arguments.artifact_root).resolve()
    try:
        artifact_root.mkdir(parents=True, exist_ok=False)
    except OSError as error:
        parser.error(f"cannot create a new artifact directory: {error}")
    report = {
        "status": "failed", "started_at": datetime.now(timezone.utc).isoformat(),
        "root": str((ROOT / arguments.root).resolve()), "cwd": str(ROOT),
        "allocation_metrics": arguments.allocation_metrics, "toolchain": arguments.toolchain,
        "repetitions": arguments.repetitions,
        "host": {"system": platform.system(), "release": platform.release(),
                 "version": platform.version(), "machine": platform.machine()},
        "python_version": platform.python_version(),
        "measurement_note": (
            "Timing and RSS are advisory with no thresholds. Elapsed time includes process startup; "
            "allocation instrumentation adds timing overhead when enabled."
        ),
        "rss_tool": None, "commands": [],
    }
    exit_code = 1
    try:
        collect(arguments, artifact_root, report)
        report["status"] = "completed"
        exit_code = 0
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
        if isinstance(error, subprocess.CalledProcessError):
            exit_code = error.returncode if error.returncode > 0 else 128 - error.returncode
        print(f"asset profile failed: {error}", file=sys.stderr)
    except KeyboardInterrupt:
        report["error"] = "interrupted"
        exit_code = 130
    finally:
        report["finished_at"] = datetime.now(timezone.utc).isoformat()
        report_path = artifact_root / "report.json"
        report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"Asset report: {report_path}", flush=True)
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
