"""Collect full-asset profiles or finite, semantically matched Baozi/Assimp observations."""

from __future__ import annotations

import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import signal
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
CONSUMER = ROOT / "tools/consumers/assets"
GNU_TIME = Path("/usr/bin/time")
PROTOCOL = "baozi-assimp-observation/v1"
STAGES = ("import", "prepare", "extract", "pipeline")
IO_MODES = ("file", "memory")
FIELDS = ("position", "position-normal", "position-normal-uv0")
PROCESS_METRICS = ("stage_ns", "elapsed_ns", "peak_rss_bytes",
                   "windows_peak_working_set_bytes", "windows_peak_commit_bytes")
STATUSES = ("success", "policy_rejected", "unsupported", "semantic_mismatch", "timeout", "crash")
OBSERVATION_FIELDS = (
    "draws", "placements", "tuples", "bounds", "area", "normal_bounds", "uv0_bounds",
    "translation_x_sum",
)
# Aggregate sanity tolerances cannot substitute for the probes' tuple tests.
TOLERANCES = {"f32": {"relative": 1e-5, "absolute": 1e-6},
              "f64": {"relative": 1e-12, "absolute": 1e-15}}


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def save_report(artifact_root: Path, report: dict) -> None:
    temporary = artifact_root / "report.json.tmp"
    temporary.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n", encoding="utf-8")
    temporary.replace(artifact_root / "report.json")


def digest(path: Path) -> str:
    checksum = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            checksum.update(chunk)
    return checksum.hexdigest()


def snapshot(path: Path, artifact_root: Path, name: str) -> dict:
    data = path.read_bytes()
    with (artifact_root / name).open("xb") as destination:
        destination.write(data)
    return {"path": str(path), "artifact": name, "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest()}


def executable_path(value: str) -> str:
    found = shutil.which(value)
    return str(Path(found if found else ROOT / value).resolve())


def binary_identity(path: str | None) -> dict:
    if path is None:
        return {"path": None, "sha256": None, "unavailable_reason": "no executable supplied"}
    try:
        return {"path": path, "sha256": digest(Path(path)), "bytes": Path(path).stat().st_size}
    except OSError as error:
        return {"path": path, "sha256": None, "unavailable_reason": str(error)}


def host_load() -> dict:
    try:
        return {"load_average_1_5_15": list(os.getloadavg()), "unavailable_reason": None}
    except (AttributeError, OSError):
        return {"load_average_1_5_15": None, "unavailable_reason": "os.getloadavg is unavailable"}


def read_peak_rss(path: Path) -> tuple[int | None, str | None]:
    """GNU time %M is the individual child's whole-lifetime peak RSS in KiB."""
    try:
        kib = int(path.read_text(encoding="utf-8").strip())
        if kib > 0:
            return kib * 1024, None
    except (OSError, ValueError):
        pass
    return None, "GNU time did not provide a positive whole-child peak RSS value"


class WindowsMemory:
    """Keep a query handle through termination; read lifetime peaks before closing it."""

    def __init__(self, pid: int):
        import ctypes
        from ctypes import wintypes

        class Counters(ctypes.Structure):
            _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + [
                (name, ctypes.c_size_t) for name in (
                    "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
                    "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage",
                    "QuotaNonPagedPoolUsage", "PagefileUsage", "PeakPagefileUsage", "PrivateUsage")]

        self.ctypes = ctypes
        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        self.api.OpenProcess.argtypes = (wintypes.DWORD, wintypes.BOOL, wintypes.DWORD)
        self.api.OpenProcess.restype = wintypes.HANDLE
        self.api.K32GetProcessMemoryInfo.argtypes = (
            wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD)
        self.api.K32GetProcessMemoryInfo.restype = wintypes.BOOL
        self.api.CloseHandle.argtypes = (wintypes.HANDLE,)
        self.api.CloseHandle.restype = wintypes.BOOL
        self.counters = Counters()
        self.counters.cb = ctypes.sizeof(self.counters)
        self.handle = self.api.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
        self.error = None if self.handle else f"OpenProcess: {ctypes.WinError(ctypes.get_last_error())}"

    def finish(self) -> dict:
        result = {"windows_peak_working_set_bytes": None, "windows_peak_commit_bytes": None,
                  "windows_memory_unavailable_reason": self.error}
        if not self.handle:
            return result
        try:
            if not self.api.K32GetProcessMemoryInfo(
                    self.handle, self.ctypes.byref(self.counters), self.counters.cb):
                result["windows_memory_unavailable_reason"] = (
                    f"GetProcessMemoryInfo: {self.ctypes.WinError(self.ctypes.get_last_error())}")
            elif self.counters.PeakWorkingSetSize <= 0 or self.counters.PeakPagefileUsage <= 0:
                result["windows_memory_unavailable_reason"] = "process API did not supply positive lifetime peaks"
            else:
                result.update(windows_peak_working_set_bytes=self.counters.PeakWorkingSetSize,
                              windows_peak_commit_bytes=self.counters.PeakPagefileUsage)
        finally:
            self.api.CloseHandle(self.handle)
            self.handle = None
        return result


def stop_child(process: subprocess.Popen, timeout: float) -> None:
    # GNU time and its worker share this fresh POSIX process group.
    if os.name == "posix":
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    else:
        process.kill()
    process.wait(timeout=timeout)


def run_command(arguments: argparse.Namespace, artifact_root: Path, report: dict,
                name: str, command: list[str], *, kind: str = "tool", details: dict | None = None,
                measure_rss: bool = False, skip: tuple[str, str] | None = None) -> dict:
    record = {
        "name": name, "kind": kind, "argv": command, "returncode": None,
        "stdout": f"{name}.stdout.log", "stderr": f"{name}.stderr.log",
        "timeout_seconds": arguments.timeout, "started_at": now(), "launched": False,
        "status": "crash", "elapsed_ns": None,
    }
    record.update(details or {})
    rss_path = artifact_root / f"{name}.rss-kib.txt"
    if measure_rss:
        record.update(probe_argv=command, peak_rss_bytes=None, rss_scope="whole_child_lifetime",
                      rss_unavailable_reason=f"requires Linux /usr/bin/time; platform={sys.platform}",
                      windows_peak_working_set_bytes=None, windows_peak_commit_bytes=None,
                      windows_memory_scope="whole_child_lifetime",
                      windows_memory_unavailable_reason="child not launched" if sys.platform == "win32"
                      else f"requires Windows process API; platform={sys.platform}")
        if report["rss_tool"] and skip is None:
            record["argv"] = [report["rss_tool"], "-q", "-f", "%M", "-o", str(rss_path),
                              "--", *command]
            record["rss_log"] = rss_path.name
    report["commands"].append(record)
    if kind != "comparison":
        save_report(artifact_root, report)
    started = time.monotonic_ns()
    process = None
    windows_memory = None
    try:
        with ((artifact_root / record["stdout"]).open("xb") as stdout,
              (artifact_root / record["stderr"]).open("xb") as stderr):
            if skip is not None:
                record["status"], record["failure_reason"] = skip
            else:
                process = subprocess.Popen(record["argv"], cwd=ROOT, env=arguments.environment,
                                           stdout=stdout, stderr=stderr,
                                           start_new_session=os.name == "posix")
                record["launched"] = True
                if measure_rss and sys.platform == "win32":
                    try:
                        windows_memory = WindowsMemory(process.pid)
                    except OSError as error:
                        record["windows_memory_unavailable_reason"] = f"process API: {error}"
                try:
                    record["returncode"] = process.wait(timeout=arguments.timeout)
                    record["status"] = "success" if record["returncode"] == 0 else "crash"
                    if record["returncode"] != 0:
                        record["failure_reason"] = "child exited unsuccessfully"
                except subprocess.TimeoutExpired:
                    record.update(status="timeout", failure_reason="fixed child timeout exceeded")
                    stop_child(process, arguments.timeout)
                    record["returncode"] = process.returncode
    except OSError as error:
        record.update(status="crash", failure_reason=f"{type(error).__name__}: {error}")
    except KeyboardInterrupt:
        record.update(status="crash", failure_reason="collector interrupted")
        if process is not None and process.poll() is None:
            stop_child(process, arguments.timeout)
            record["returncode"] = process.returncode
        raise
    finally:
        record["elapsed_ns"] = time.monotonic_ns() - started
        record["finished_at"] = now()
        record["host_load"] = host_load()
        if measure_rss and report["rss_tool"] and record["launched"]:
            record["peak_rss_bytes"], record["rss_unavailable_reason"] = read_peak_rss(rss_path)
        if windows_memory is not None:
            record.update(windows_memory.finish())
        if kind != "comparison":
            save_report(artifact_root, report)
    return record


def command_text(record: dict, artifact_root: Path) -> str | None:
    if record["status"] == "success":
        return (artifact_root / record["stdout"]).read_text(encoding="utf-8").strip()
    return None


def capture_context(arguments: argparse.Namespace, artifact_root: Path, report: dict) -> None:
    for name, command in (("source-head", ["git", "--no-optional-locks", "rev-parse", "HEAD"]),
                          ("source-status", ["git", "--no-optional-locks", "status", "--porcelain", "--untracked-files=all"])):
        record = run_command(arguments, artifact_root, report, name, command)
        text = command_text(record, artifact_root)
        if name == "source-head":
            report["git_sha"] = text
        else:
            report["git_dirty"] = bool(text) if text is not None else None
    report["source_manifests"] = [
        snapshot(path, artifact_root, f"baozi-source-{path.name}")
        for path in (CONSUMER / "Cargo.toml", CONSUMER / "Cargo.lock") if path.is_file()
    ]
    report["collector"] = snapshot(Path(__file__), artifact_root, "collector.py")
    report["protocol_document"] = snapshot(ROOT / "docs/testing/assimp-comparison.md",
                                           artifact_root, "assimp-comparison.md")


def consumer_executable(build_log: Path) -> str:
    """Use Cargo's executable artifact instead of guessing a target layout."""
    executables = set()
    for line in build_log.read_text(encoding="utf-8").splitlines():
        message = json.loads(line)
        target = message.get("target", {})
        if (message.get("reason") == "compiler-artifact"
                and target.get("name") == "baozi-asset-consumer"
                and "bin" in target.get("kind", []) and message.get("executable")):
            executables.add(message["executable"])
    if len(executables) != 1:
        raise RuntimeError(f"expected one asset consumer executable, found {len(executables)}")
    executable = executables.pop()
    if not Path(executable).is_file():
        raise RuntimeError(f"asset consumer executable is missing: {executable}")
    return executable


def baozi_executable(arguments: argparse.Namespace, artifact_root: Path, report: dict) -> str | None:
    if arguments.baozi_probe:
        return executable_path(arguments.baozi_probe)
    toolchain = [f"+{arguments.toolchain}"] if arguments.toolchain else []
    run_command(arguments, artifact_root, report, "rustc", ["rustc", *toolchain, "-vV"])
    command = ["cargo", *toolchain, "build", "--locked", "--release", "--manifest-path",
               str(CONSUMER / "Cargo.toml"), "--target-dir", str(ROOT / "target"),
               "--jobs", "1", "--message-format=json", "--bin", "baozi-asset-consumer"]
    if arguments.allocation_metrics:
        command += ["--features", "allocation-metrics"]
    build = run_command(arguments, artifact_root, report, "build", command, kind="build")
    if build["status"] != "success":
        report["baozi_build_error"] = build.get("failure_reason", build["status"])
        return None
    try:
        return consumer_executable(artifact_root / build["stdout"])
    except (OSError, ValueError, AttributeError, RuntimeError) as error:
        report["baozi_build_error"] = str(error)
        return None


def record_binaries(arguments: argparse.Namespace, artifact_root: Path, report: dict,
                    baozi: str | None) -> dict[str, str | None]:
    paths = {"baozi": baozi}
    if arguments.comparison_cases:
        paths.update(assimp=executable_path(arguments.assimp_probe),
                     assimp_double=executable_path(arguments.assimp_double_probe)
                     if arguments.assimp_double_probe else None)
    report["binaries"] = {name: binary_identity(path) for name, path in paths.items()}
    for name, identity in report["binaries"].items():
        supplied_manifest = getattr(arguments, f"{name}_build_manifest")
        identity["build_manifest"] = (
            snapshot((ROOT / supplied_manifest).resolve(), artifact_root, f"{name}-build-manifest.json")
            if supplied_manifest else None
        )
        identity["build_manifest_unavailable_reason"] = (
            None if supplied_manifest else
            "external build metadata not supplied; source manifests do not attest this binary's flags"
        )
    return paths


def finite_number(value: object) -> bool:
    return type(value) in (int, float) and math.isfinite(value)


def matches_fact(actual: object, expected: object, precision: str) -> bool:
    if expected is None:
        return actual is None
    if isinstance(expected, list):
        return (isinstance(actual, list) and len(actual) == len(expected)
                and all(matches_fact(left, right, precision) for left, right in zip(actual, expected)))
    if not finite_number(actual) or not finite_number(expected):
        return False
    tolerance = TOLERANCES[precision]
    return math.isclose(actual, expected, rel_tol=tolerance["relative"], abs_tol=tolerance["absolute"])


def read_cases(arguments: argparse.Namespace, artifact_root: Path, report: dict) -> tuple[Path, list[dict]]:
    manifest_path = (ROOT / arguments.comparison_cases).resolve()
    report["corpus"] = snapshot(manifest_path, artifact_root, "comparison-cases.json")
    manifest = read_json(artifact_root / "comparison-cases.json")
    if (not isinstance(manifest, dict) or manifest.get("schema") != "baozi-assimp-cases/v1"
            or manifest.get("protocol") != PROTOCOL or not isinstance(manifest.get("cases"), list)):
        raise ValueError("expected the finite baozi-assimp-cases/v1 manifest and observation/v1 protocol")
    root = (ROOT / arguments.root).resolve() if arguments.root else manifest_path.parent
    by_id = {}
    for case in manifest["cases"]:
        if not isinstance(case, dict) or not isinstance(case.get("id"), str) or not case["id"]:
            raise ValueError("each comparison case needs an exact nonempty ID")
        if case["id"] in by_id:
            raise ValueError(f"duplicate case ID: {case['id']}")
        if (case.get("precision") not in TOLERANCES or case.get("fields") not in FIELDS
                or case.get("normal_policy") not in ("preserve", "missing")
                or not isinstance(case.get("expected"), dict)
                or set(case["expected"]) != set(OBSERVATION_FIELDS)
                or not isinstance(case.get("stages"), list) or not case["stages"]
                or any(stage not in STAGES for stage in case["stages"])
                or not isinstance(case.get("io_modes"), list) or not case["io_modes"]
                or any(mode not in IO_MODES for mode in case["io_modes"])
                or type(case.get("max_work_units")) is not int or case["max_work_units"] < 1):
            raise ValueError(f"invalid finite case contract: {case['id']}")
        by_id[case["id"]] = case
    selected = list(dict.fromkeys(arguments.case)) if arguments.case else list(by_id)
    unknown = [case_id for case_id in selected if case_id not in by_id]
    if unknown:
        raise ValueError(f"unknown case IDs: {', '.join(unknown)}")
    if not selected:
        raise ValueError("no comparison cases selected")
    report["case_order"] = selected
    report["root"] = str(root)
    return root, [by_id[case_id] for case_id in selected]


def check_input(root: Path, case: dict) -> str | None:
    try:
        files = case["files"]
        if len(files) != 1 or files[0]["path"] != case["path"]:
            return "comparison requires exactly the declared single-file input"
        path = (root / case["path"]).resolve()
        if not path.is_relative_to(root) or Path(case["path"]).is_absolute():
            return "case input escapes its root"
        if path.stat().st_size != files[0]["bytes"] or digest(path) != files[0]["sha256"]:
            return "case input size/SHA-256 differs from the generated manifest"
    except (OSError, KeyError, TypeError) as error:
        return f"cannot verify case input: {error}"
    return None


def expected_observation(case: dict, stage: str, engine: str) -> dict:
    expected = dict(case["expected"])
    if stage == "import" and case["family"] == "obj-repeated":
        expected["normal_bounds"] = None
    if stage == "import" and engine == "assimp" and case["family"] == "gltf-shared":
        low, high = expected["uv0_bounds"]
        expected["uv0_bounds"] = [[low[0], 1 - high[1]], [high[0], 1 - low[1]]]
    return expected


def reject_constants(value: str) -> None:
    raise ValueError(f"nonfinite JSON number: {value}")


def json_float(value: str) -> float:
    number = float(value)
    if not math.isfinite(number):
        raise ValueError(f"nonfinite JSON number: {value}")
    return number


def read_json(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8"), parse_constant=reject_constants,
                      parse_float=json_float)


def validate_observation(record: dict, case: dict, artifact_root: Path) -> None:
    if not record["launched"]:
        return
    try:
        response = read_json(artifact_root / record["stdout"])
        record["worker_json"] = response
        if record["status"] == "timeout":
            return
        if not isinstance(response, dict) or response.get("schema") != PROTOCOL:
            raise ValueError("expected one observation/v1 JSON object")
        if response.get("status") not in STATUSES:
            raise ValueError("unknown worker status")
        for key in ("engine", "stage", "io_mode", "fields", "normal_policy"):
            if response.get(key) != record[key]:
                raise ValueError(f"worker {key} does not match the requested lane")
        if "case_id" in response and response["case_id"] != case["id"]:
            raise ValueError("worker case_id does not match the exact manifest ID")
        if "path" in response and response["path"] != case["path"]:
            raise ValueError("worker path does not match the exact manifest path")
        status = response["status"]
        if status != "success":
            if record["returncode"] == 0:
                raise ValueError("failure observation returned exit code zero")
            record.update(status=status, failure_stage=response.get("failure_stage"),
                          failure_reason=response.get("diagnostic", response.get("error", status)))
            return
        if record["returncode"] != 0:
            record.update(status="crash", failure_reason="success JSON accompanied an unsuccessful exit")
            return
        if type(response.get("scalar_bytes")) is not int or response["scalar_bytes"] != (
                8 if case["precision"] == "f64" else 4):
            raise ValueError("retained/source scalar_bytes does not match the precision lane")
        if type(response.get("stage_ns")) is not int or not 0 <= response["stage_ns"] < 2 ** 64:
            raise ValueError("stage_ns must be a nonnegative u64 integer")
        if (response.get("computation_scalar_bytes") is not None
                and (type(response["computation_scalar_bytes"]) is not int
                     or response["computation_scalar_bytes"] not in (4, 8))):
            raise ValueError("invalid computation scalar width diagnostic")
        if record["stage"] in ("extract", "pipeline") and "output_scalar_bytes" in response:
            if type(response["output_scalar_bytes"]) is not int or response["output_scalar_bytes"] != 8:
                raise ValueError("copied comparison output must use F64")
        facts = response.get("observation")
        if not isinstance(facts, dict):
            raise ValueError("missing observation facts")
        for key, expected in expected_observation(case, record["stage"], record["engine"]).items():
            if key not in facts:
                raise ValueError(f"missing observation.{key}")
            if key in ("draws", "placements", "tuples"):
                valid = type(facts[key]) is int and facts[key] == expected
            else:
                valid = matches_fact(facts[key], expected, case["precision"])
            if not valid:
                raise ValueError(f"observation.{key} differs from analytic source facts")
        if "output" not in response:
            raise ValueError("missing output field")
        output = response["output"]
        if record["stage"] in ("import", "prepare"):
            if output is not None:
                raise ValueError("import/prepare must not construct copied comparison output")
        elif (not isinstance(output, dict)
              or any(type(output.get(key)) is not int or not 0 <= output[key] < 2 ** 64
                     for key in ("logical_bytes", "capacity_bytes"))
              or output["capacity_bytes"] < output["logical_bytes"]):
            raise ValueError("invalid logical/capacity output byte counts")
        record.update(status="success", stage_ns=response["stage_ns"], output=output,
                      scalar_bytes=response["scalar_bytes"],
                      computation_scalar_bytes=response.get("computation_scalar_bytes"),
                      output_scalar_bytes=response.get("output_scalar_bytes"),
                      width_note="source width validated; absent computation/output width diagnostics are unknown")
        record.pop("failure_reason", None)
    except (OSError, ValueError, TypeError, OverflowError) as error:
        if record["status"] != "timeout" and (record["returncode"] == 0
                or isinstance(record.get("worker_json"), dict)):
            record["status"] = "semantic_mismatch"
        record["validation_error"] = str(error)
        if record["status"] == "semantic_mismatch":
            record["failure_reason"] = str(error)


def summarize(values: list[int | float]) -> dict | None:
    if not values:
        return None
    if len(values) == 1:
        iqr = 0
    else:
        quartiles = statistics.quantiles(values, n=4, method="inclusive")
        iqr = quartiles[2] - quartiles[0]
    return {"count": len(values), "median": statistics.median(values), "iqr": iqr,
            "min": min(values), "max": max(values)}


def summarize_series(series: dict, records: list[dict]) -> None:
    retained = [record for record in records if record["phase"] == "retained"]
    series["engines"] = {}
    for engine in ("baozi", "assimp"):
        attempts = [record for record in retained if record["engine"] == engine]
        successes = [record for record in attempts if record["status"] == "success"]
        summary = {"statuses": dict(Counter(record["status"] for record in attempts)),
                   "samples": [record["name"] for record in attempts]}
        for metric in PROCESS_METRICS:
            summary[metric] = summarize([record[metric] for record in successes
                                         if record.get(metric) is not None])
        for metric in ("logical_bytes", "capacity_bytes"):
            summary[metric] = summarize([record["output"][metric] for record in successes
                                         if record.get("output") is not None])
        summary["rss_unavailable_reasons"] = sorted({record["rss_unavailable_reason"]
                                                    for record in attempts
                                                    if record.get("rss_unavailable_reason")})
        summary["windows_memory_unavailable_reasons"] = sorted({
            record["windows_memory_unavailable_reason"] for record in attempts
            if record.get("windows_memory_unavailable_reason")})
        series["engines"][engine] = summary
    pairs = {}
    for record in retained:
        pairs.setdefault(record["repetition"], {})[record["engine"]] = record
    matched = [pair for pair in pairs.values()
               if all(pair.get(engine, {}).get("status") == "success" for engine in ("baozi", "assimp"))]
    series["matched_pairs"] = len(matched)
    series["ratios"] = {}
    for metric in PROCESS_METRICS:
        usable = [pair for pair in matched if all(pair[engine].get(metric) is not None
                  and pair[engine][metric] > 0 for engine in ("baozi", "assimp"))]
        ratios = [pair["assimp"][metric] / pair["baozi"][metric] for pair in usable]
        series["ratios"][metric] = {
            "direction": "assimp / baozi", "samples": ratios, "statistics": summarize(ratios),
            "repetitions": [pair["baozi"]["repetition"] for pair in usable],
            "matched_engine_statistics": {engine: summarize([pair[engine][metric] for pair in usable])
                                          for engine in ("baozi", "assimp")},
            "unavailable_reason": None if ratios else "no matched successful pair with positive metric values",
        }


def collect_comparison(arguments: argparse.Namespace, artifact_root: Path, report: dict) -> None:
    root, cases = read_cases(arguments, artifact_root, report)
    baozi = baozi_executable(arguments, artifact_root, report)
    paths = record_binaries(arguments, artifact_root, report, baozi)
    report["series"] = []
    pair_index = 0
    for case in cases:
        input_error = check_input(root, case)
        for stage in dict.fromkeys(arguments.stage or case["stages"]):
            if stage not in case["stages"]:
                continue
            for io_mode in dict.fromkeys(arguments.io_mode or case["io_modes"]):
                if io_mode not in case["io_modes"]:
                    continue
                lane = {"case_id": case["id"], "path": case["path"], "stage": stage,
                        "io_mode": io_mode, "precision": case["precision"], "fields": case["fields"],
                        "normal_policy": case["normal_policy"],
                        "max_work_units": arguments.max_work_units or case["max_work_units"]}
                series = {**lane, "samples": [], "input_validation_error": input_error}
                report["series"].append(series)
                records = []
                print(f"{case['id']} {stage} {io_mode}: collecting serial observations", flush=True)
                try:
                    for phase, count in (("warmup", arguments.warmups), ("retained", arguments.repetitions)):
                        for repetition in range(1, count + 1):
                            order = ("baozi", "assimp") if pair_index % 2 == 0 else ("assimp", "baozi")
                            pair_index += 1
                            for engine in order:
                                binary_key = "assimp_double" if engine == "assimp" and case["precision"] == "f64" else engine
                                executable = paths[binary_key]
                                skip = None
                                if input_error:
                                    skip = ("semantic_mismatch", input_error)
                                elif executable is None:
                                    skip = (("unsupported", "Assimp F64 lane unavailable: --assimp-double-probe not supplied")
                                            if binary_key == "assimp_double" else
                                            ("crash", report.get("baozi_build_error", "executable unavailable")))
                                command = ([executable] if executable else []) + [str(root), case["path"],
                                    "--comparison", stage, "--io-mode", io_mode, "--fields", case["fields"],
                                    "--normals", case["normal_policy"], "--max-work-units", str(lane["max_work_units"])]
                                name = f"comparison-{len(report['series']):03d}-{phase}-{repetition:03d}-{engine}"
                                native_uv = stage == "import" and engine == "assimp" and case["family"] == "gltf-shared"
                                record = run_command(arguments, artifact_root, report, name, command,
                                    kind="comparison", details={**lane, "engine": engine, "binary": binary_key,
                                    "phase": phase, "repetition": repetition, "pair_order": list(order),
                                    "uv0_convention": "assimp-native-inverted-v" if native_uv else "source"},
                                    measure_rss=True, skip=skip)
                                records.append(record)
                                series["samples"].append(name)
                                validate_observation(record, case, artifact_root)
                finally:
                    summarize_series(series, records)
                    save_report(artifact_root, report)
    if not report["series"]:
        raise ValueError("no case admits the selected stages and IO modes")


def collect_assets(arguments: argparse.Namespace, artifact_root: Path, report: dict) -> None:
    report["root"] = str((ROOT / (arguments.root or CONSUMER / "fixtures")).resolve())
    assets = arguments.asset
    if assets is None:
        fixtures = json.loads((CONSUMER / "fixtures/expectations.json").read_text(encoding="utf-8"))
        assets = [fixture["path"] for fixture in fixtures["fixtures"] if fixture["expected_outcome"] == "success"]
    if not assets:
        raise ValueError("no assets selected")
    executable = baozi_executable(arguments, artifact_root, report)
    record_binaries(arguments, artifact_root, report, executable)
    forwarded = ["--assume-standard-space"] if arguments.assume_standard_space else []
    if arguments.max_work_units is not None:
        forwarded += ["--max-work-units", str(arguments.max_work_units)]
    for repetition in range(1, arguments.repetitions + 1):
        for index, asset in enumerate(assets, 1):
            command = ([executable] if executable else []) + [report["root"], asset, *forwarded]
            record = run_command(arguments, artifact_root, report, f"asset-{repetition:03d}-{index:03d}",
                                 command, kind="asset", details={"asset": asset, "repetition": repetition},
                                 measure_rss=True, skip=("crash", report["baozi_build_error"])
                                 if executable is None else None)
            if record["launched"]:
                try:
                    record["summary"] = read_json(artifact_root / record["stdout"])
                    if not isinstance(record["summary"], dict):
                        raise ValueError("expected one full-asset JSON summary object")
                except (OSError, ValueError) as error:
                    if record["status"] == "success":
                        record.update(status="semantic_mismatch", failure_reason=str(error))
            save_report(artifact_root, report)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact-root", type=Path, help="new output directory; existing reports are never overwritten")
    parser.add_argument("--asset", action="append", help="full-asset workflow only; repeatable relative asset path")
    parser.add_argument("--root", type=Path, help="input scope root (comparison default: manifest directory)")
    parser.add_argument("--toolchain", help="Rust toolchain for the optional standard release build")
    parser.add_argument("--allocation-metrics", action="store_true", help="full-asset allocator instrumentation only")
    parser.add_argument("--assume-standard-space", action="store_true", help="full-asset explicit OBJ space assumption")
    parser.add_argument("--max-work-units", type=int, help="explicit positive consumer work limit")
    parser.add_argument("--repetitions", type=int, help="retained fresh processes (comparison: 20; asset: 1)")
    parser.add_argument("--timeout", type=float, required=True, help="fixed finite positive timeout in seconds for every child")
    parser.add_argument("--comparison-cases", type=Path, help="generated comparison-cases.json; selects comparison workflow")
    parser.add_argument("--baozi-probe", help="prebuilt consumer executable; otherwise build the standard release binary")
    parser.add_argument("--assimp-probe", help="prebuilt F32 Assimp comparison executable")
    parser.add_argument("--assimp-double-probe", help="separate F64 Assimp comparison executable")
    parser.add_argument("--warmups", type=int, default=3, help="fresh comparison warmups per engine/lane (default: 3)")
    parser.add_argument("--case", action="append", help="exact comparison case ID; repeatable (default: all)")
    parser.add_argument("--stage", choices=STAGES, action="append", help="comparison stage; repeatable (default: all admitted)")
    parser.add_argument("--io-mode", choices=IO_MODES, action="append", help="comparison IO mode; repeatable (default: both)")
    for engine in ("baozi", "assimp", "assimp-double"):
        parser.add_argument(f"--{engine}-build-manifest", type=Path,
                            help="retain supplied compiler/flags/link/allocator build metadata verbatim")
    arguments = parser.parse_args()
    if not math.isfinite(arguments.timeout) or arguments.timeout <= 0:
        parser.error("--timeout must be finite and positive")
    if arguments.repetitions is None:
        arguments.repetitions = 20 if arguments.comparison_cases else 1
    if arguments.repetitions < 1 or arguments.warmups < 0:
        parser.error("--repetitions must be positive and --warmups nonnegative")
    if arguments.max_work_units is not None and arguments.max_work_units < 1:
        parser.error("--max-work-units must be positive")
    if arguments.comparison_cases:
        if not arguments.assimp_probe:
            parser.error("--comparison-cases requires --assimp-probe")
        if arguments.asset or arguments.allocation_metrics or arguments.assume_standard_space:
            parser.error("full-asset options cannot change the uninstrumented source-space comparison task")
    elif arguments.case or arguments.stage or arguments.io_mode or arguments.assimp_probe or arguments.assimp_double_probe:
        parser.error("comparison selections require --comparison-cases")
    default_root = Path("target/assimp-comparison" if arguments.comparison_cases else "target/asset-profiles")
    default_root /= datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    artifact_root = (ROOT / (arguments.artifact_root or default_root)).resolve()
    try:
        artifact_root.mkdir(parents=True, exist_ok=False)
    except OSError as error:
        parser.error(f"cannot create a new artifact directory: {error}")
    arguments.environment = os.environ.copy()
    arguments.environment.setdefault("CARGO_BUILD_JOBS", "1")
    arguments.environment["CARGO_TERM_COLOR"] = "never"
    clock = time.get_clock_info("monotonic")
    report = {
        "schema": "baozi-assimp-report/v1" if arguments.comparison_cases else "baozi-asset-profile/v1",
        "protocol": PROTOCOL if arguments.comparison_cases else None,
        "mode": "comparison" if arguments.comparison_cases else "asset", "status": "running",
        "started_at": now(), "cwd": str(ROOT), "collector_argv": sys.argv,
        "allocation_metrics": arguments.allocation_metrics, "toolchain": arguments.toolchain,
        "repetitions": arguments.repetitions, "warmups": arguments.warmups if arguments.comparison_cases else 0,
        "timeout_seconds": arguments.timeout, "commands": [],
        "host": {"system": platform.system(), "release": platform.release(), "version": platform.version(),
                 "machine": platform.machine(), "processor": platform.processor(), "cpu_count": os.cpu_count()},
        "python": {"version": platform.python_version(), "executable": sys.executable},
        "initial_host_load": host_load(),
        "environment": {key: arguments.environment[key] for key in (
            "CARGO_BUILD_JOBS", "CARGO_TERM_COLOR", "OMP_NUM_THREADS", "MKL_NUM_THREADS",
            "OPENBLAS_NUM_THREADS", "RAYON_NUM_THREADS", "MALLOC_ARENA_MAX") if key in arguments.environment},
        "environment_scope": "selected build/thread/allocator controls only; other inherited values are not recorded",
        "rss_tool": str(GNU_TIME) if sys.platform == "linux" and GNU_TIME.is_file() else None,
        "rss_scope": "whole-child peak RSS, including setup, validation and serialization; never a stage peak",
        "windows_memory_tool": "kernel32!K32GetProcessMemoryInfo" if sys.platform == "win32" else None,
        "windows_memory_scope": "whole-child lifetime PeakWorkingSetSize and PeakPagefileUsage, queried after wait with a retained query handle; never stage peaks or Linux RSS",
        "parent_clock": {"implementation": clock.implementation, "resolution_ns": clock.resolution * 1e9},
        "statistics_policy": "median; inclusive linear quartiles; IQR=Q3-Q1; warmups excluded; no aggregate winner",
        "tolerances": TOLERANCES,
        "measurement_note": "Warm caches, fresh serial children. Parent wall includes startup and checks. Tiny times are descriptive; worker timer resolution may be unknown.",
        "heap": {"metrics": None, "available_tool": shutil.which("heaptrack"),
                 "unavailable_reason": "no separate verified heaptrack interception campaign; profiled timings are never speed results"},
    }
    exit_code = 1
    try:
        save_report(artifact_root, report)
        capture_context(arguments, artifact_root, report)
        if arguments.comparison_cases:
            collect_comparison(arguments, artifact_root, report)
        else:
            collect_assets(arguments, artifact_root, report)
        samples = [record for record in report["commands"] if record["kind"] in ("comparison", "asset")]
        report["status"] = "completed" if all(record["status"] == "success" for record in samples) else "partial"
        exit_code = 0 if report["status"] == "completed" else 1
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError) as error:
        report.update(status="failed", error=f"{type(error).__name__}: {error}")
        print(f"profile collection failed: {error}", file=sys.stderr)
    except KeyboardInterrupt:
        report.update(status="partial", error="interrupted")
        exit_code = 130
    finally:
        report["finished_at"] = now()
        report["final_host_load"] = host_load()
        save_report(artifact_root, report)
        print(f"Profile report: {artifact_root / 'report.json'}", flush=True)
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
