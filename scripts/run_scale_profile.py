"""Run the Rust OBJ scale contract and collect advisory performance measurements."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[1]
CARGO_FLAGS = [
    "--locked", "-p", "baozi", "--no-default-features", "--features", "format-obj",
]
GNU_TIME = Path("/usr/bin/time")


def probe_executable(build_log: Path) -> str:
    """Read Cargo's executable path instead of guessing a target-directory layout."""
    executables = set()
    with build_log.open(encoding="utf-8") as stream:
        for line in stream:
            message = json.loads(line)
            target = message.get("target", {})
            if (
                message.get("reason") == "compiler-artifact"
                and target.get("name") == "import_scale_probe"
                and "bench" in target.get("kind", [])
                and message.get("executable")
            ):
                executables.add(message["executable"])
    if len(executables) != 1:
        raise RuntimeError(f"expected one scale probe executable, found {len(executables)}")
    executable = executables.pop()
    if not Path(executable).is_file():
        raise RuntimeError(f"scale probe executable is missing: {executable}")
    return executable


def read_peak_rss(path: Path) -> tuple[int | None, str | None]:
    """GNU time %M is the individual probe's peak resident set size in KiB on Linux."""
    try:
        kib = int(path.read_text(encoding="utf-8").strip())
        if kib > 0:
            return kib * 1024, None
    except (OSError, ValueError):
        pass
    return None, "GNU time did not provide a positive peak RSS value"


def collect(artifact_root: Path, report: dict) -> None:
    environment = os.environ.copy()
    environment.setdefault("CARGO_BUILD_JOBS", "1")
    environment["CARGO_TERM_COLOR"] = "never"
    environment["CRITERION_HOME"] = str(artifact_root / "criterion")
    time_tool = str(GNU_TIME) if sys.platform == "linux" and GNU_TIME.is_file() else None
    report["rss_tool"] = time_tool

    def run(name: str, command: list[str], timeout: int) -> dict:
        record = {
            "name": name,
            "argv": command,
            "timeout_seconds": timeout,
            "stdout": f"{name}.stdout.log",
            "stderr": f"{name}.stderr.log",
            "returncode": None,
        }
        report["commands"].append(record)
        print(f"{name}: logs in {artifact_root / record['stdout']}", flush=True)
        started = time.monotonic()
        try:
            with (
                (artifact_root / record["stdout"]).open("xb") as stdout,
                (artifact_root / record["stderr"]).open("xb") as stderr,
            ):
                result = subprocess.run(
                    command, cwd=ROOT, env=environment, stdout=stdout, stderr=stderr,
                    timeout=timeout, check=True,
                )
                record["returncode"] = result.returncode
        except subprocess.CalledProcessError as error:
            record["returncode"] = error.returncode
            raise
        except (OSError, subprocess.TimeoutExpired) as error:
            record["error"] = str(error)
            raise
        finally:
            record["elapsed_seconds"] = round(time.monotonic() - started, 3)
        return record

    run("contract", [
        "cargo", "nextest", "run", *CARGO_FLAGS, "--test", "import_scale_contract",
        "--test", "unique_vertex_growth_contract",
    ], 900)
    run("criterion", [
        "cargo", "bench", *CARGO_FLAGS, "--bench", "import_baseline", "--",
        "obj_face_import_scale", "--noplot", "--warm-up-time", "1",
        "--measurement-time", "3", "--sample-size", "20", "--nresamples", "2000",
    ], 900)
    build = run("probe-build", [
        "cargo", "bench", *CARGO_FLAGS, "--bench", "import_scale_probe",
        "--no-run", "--message-format=json",
    ], 600)
    executable = probe_executable(artifact_root / build["stdout"])

    for scale in ("idle", "1", "10", "100", "1000"):
        name = f"probe-{scale}"
        rss_path = artifact_root / f"{name}.rss-kib.txt"
        command = [executable, scale]
        if time_tool:
            command = [time_tool, "-f", "%M", "-o", str(rss_path), "--", *command]
        record = run(name, command, 180)
        record["peak_rss_bytes"] = None
        record["rss_unavailable_reason"] = "requires Linux with /usr/bin/time"
        if time_tool:
            record["peak_rss_bytes"], record["rss_unavailable_reason"] = read_peak_rss(rss_path)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    default_root = Path("target/scale-artifacts") / datetime.now(timezone.utc).strftime(
        "%Y%m%dT%H%M%S%fZ"
    )
    parser.add_argument(
        "--artifact-root", type=Path, default=default_root,
        help="new output directory, relative to the repository or absolute (default: %(default)s)",
    )
    arguments = parser.parse_args()
    artifact_root = (ROOT / arguments.artifact_root).resolve()
    try:
        artifact_root.mkdir(parents=True, exist_ok=False)
    except OSError as error:
        parser.error(f"cannot create a new artifact directory: {error}")

    report = {
        "status": "failed",
        "started_at": datetime.now(timezone.utc).isoformat(),
        "measurement_note": (
            "Timing and RSS are advisory. Completion means the commands succeeded; "
            "it does not mean a timing or RSS threshold passed. Command elapsed time "
            "includes build and harness overhead; Criterion data is in criterion/."
        ),
        "rss_tool": None,
        "commands": [],
    }
    exit_code = 1
    try:
        collect(artifact_root, report)
        report["status"] = "completed"
        exit_code = 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
        print(f"scale profile failed: {error}", file=sys.stderr)
    except KeyboardInterrupt:
        report["error"] = "interrupted"
        exit_code = 130
    finally:
        report["finished_at"] = datetime.now(timezone.utc).isoformat()
        report_path = artifact_root / "report.json"
        report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"Scale report: {report_path}", flush=True)
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
