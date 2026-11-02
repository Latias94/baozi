"""Exercise the collector CLI using only short-lived synthetic probe processes."""

from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "run_asset_profile.py"
FACTS = {
    "draws": 1, "placements": 1, "tuples": 24,
    "bounds": [[0, 0, 0], [2, 2, 0]], "area": 4,
    "normal_bounds": [[0, 0, 1], [0, 0, 1]],
    "uv0_bounds": [[0, 0], [1, 1]], "translation_x_sum": 0,
}

# Python can execute a directory containing __main__.py. This exercises the exact
# PROBE ROOT RELATIVE_ASSET interface on Windows as well as POSIX, without builds.
WORKER = r'''
import json
from pathlib import Path
import sys
import time

root = Path(__file__).parent
config = json.loads((root / "worker.json").read_text())
state_path = root / "state.json"
state = json.loads(state_path.read_text()) if state_path.exists() else {"calls": 0, "baozi": 0, "assimp": 0}
engine = config.get("engine") or ("baozi" if state["calls"] % 4 in (0, 3) else "assimp")
index = state[engine]
state["calls"] += 1
state[engine] += 1
state_path.write_text(json.dumps(state))
case_id = Path(sys.argv[1]).stem
case = config["cases"][case_id]
buffer = bytearray(config.get("touch_bytes", 0))
for offset in range(0, len(buffer), 4096):
    buffer[offset] = 1
fault = case.get("fault", {}).get(engine)
if fault == "timeout":
    print("started before timeout", flush=True)
    print("timeout diagnostic", file=sys.stderr, flush=True)
    time.sleep(30)
if fault == "malformed":
    print("{broken-json")
    sys.exit(0)
if fault == "crash":
    print("crash diagnostic", file=sys.stderr)
    sys.exit(7)
if fault == "overflow-json":
    print('{"schema":"baozi-assimp-observation/v1","diagnostic":1e999}')
    sys.exit(0)
args = sys.argv[2:]
if "--comparison" not in args:
    print(json.dumps({"workflow": "full-asset", "asset": sys.argv[1], "args": args}))
    sys.exit(0)
stage = args[args.index("--comparison") + 1]
observation = case.get("stage_observations", {}).get(stage, {}).get(
    engine, case.get("observations", {}).get(engine, case["expected"]))
response = {
    "schema": "baozi-assimp-observation/v1", "engine": engine,
    "case_id": case_id, "status": "success", "stage": stage,
    "io_mode": args[args.index("--io-mode") + 1],
    "fields": args[args.index("--fields") + 1],
    "normal_policy": args[args.index("--normals") + 1],
    "scalar_bytes": 8 if case["precision"] == "f64" else 4,
    "computation_scalar_bytes": 8, "output_scalar_bytes": 8,
    "stage_ns": config.get("times", {}).get(engine, [1000] * 100)[index],
    "observation": observation,
    "output": None if stage in ("import", "prepare") else {"logical_bytes": 144, "capacity_bytes": 192},
}
response.update(case.get("overrides", {}).get(engine, {}))
response.update(case.get("indexed_overrides", {}).get(engine, {}).get(str(index), {}))
if config.get("checkpoint_report"):
    checkpoint = json.loads(Path(config["checkpoint_report"]).read_text())
    response["checkpoint_comparison_count"] = sum(
        entry["kind"] == "comparison" for entry in checkpoint["commands"])
    response["checkpoint_cases"] = [entry["case_id"] for entry in checkpoint.get("series", [])]
for field in case.get("omit_fields", []):
    response.pop(field, None)
print(json.dumps(response))
print("synthetic diagnostic", file=sys.stderr)
sys.exit(0 if response["status"] == "success" else 2)
'''


class AssetProfileCLI(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "__main__.py").write_text(WORKER, encoding="utf-8")

    def case(self, case_id="obj-grid-2", **updates):
        case = {
            "id": case_id, "path": case_id + ".obj", "family": "obj-grid", "size": 2,
            "fields": "position-normal-uv0", "normal_policy": "preserve",
            "precision": "f32", "topology": "triangles", "expected": deepcopy(FACTS),
            "stages": ["import", "prepare", "extract", "pipeline"],
            "io_modes": ["file", "memory"], "max_work_units": 200000000,
        }
        case.update(updates)
        payload = b"synthetic input; the worker does not import production code\n"
        (self.root / case["path"]).write_bytes(payload)
        case["files"] = [{"path": case["path"], "bytes": len(payload),
                          "sha256": hashlib.sha256(payload).hexdigest()}]
        return case

    def collect(self, cases, *, repetitions=1, warmups=0, timeout=5, extra=(), config=None,
                stages=("pipeline",), io_modes=("memory",), python310_hashlib=False):
        manifest = {
            "schema": "baozi-assimp-cases/v1", "protocol": "baozi-assimp-observation/v1",
            "cases": cases,
        }
        manifest_path = self.root / "comparison-cases.json"
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        worker = {"cases": {case["id"]: case for case in cases}}
        worker.update(config or {})
        (self.root / "worker.json").write_text(json.dumps(worker), encoding="utf-8")
        artifact_root = self.root / "artifacts"
        command = [sys.executable, str(SCRIPT), "--comparison-cases", str(manifest_path),
                   "--baozi-probe", sys.executable, "--assimp-probe", sys.executable,
                   "--artifact-root", str(artifact_root), "--timeout", str(timeout)]
        if repetitions is not None:
            command += ["--repetitions", str(repetitions)]
        if warmups is not None:
            command += ["--warmups", str(warmups)]
        for stage in stages:
            command += ["--stage", stage]
        for io_mode in io_modes:
            command += ["--io-mode", io_mode]
        command += extra
        if python310_hashlib:
            bootstrap = (
                "import hashlib, runpy, sys\n"
                "if hasattr(hashlib, 'file_digest'):\n"
                "    del hashlib.file_digest\n"
                "sys.argv = sys.argv[1:]\n"
                "runpy.run_path(sys.argv[0], run_name='__main__')\n"
            )
            command = [sys.executable, "-c", bootstrap, *command[1:]]
        result = subprocess.run(command, capture_output=True, text=True, timeout=45)
        report_path = artifact_root / "report.json"
        self.assertTrue(report_path.is_file(), result.stderr)
        return result, json.loads(report_path.read_text(encoding="utf-8")), artifact_root

    def test_balanced_fresh_processes_keep_warmups_and_matched_statistics(self):
        build_manifest = self.root / "external-build.json"
        build_manifest.write_text(json.dumps({
            "compiler": "synthetic", "precision": "f32",
            "binary": {"sha256": hashlib.sha256(Path(sys.executable).read_bytes()).hexdigest()},
        }), encoding="utf-8")
        result, report, artifacts = self.collect(
            [self.case()], repetitions=4, warmups=1,
            config={"times": {"baozi": [5, 10, 20, 30, 40],
                              "assimp": [10, 20, 40, 60, 80]}},
            extra=["--assimp-build-manifest", str(build_manifest)],
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        samples = [entry for entry in report["commands"] if entry["kind"] == "comparison"]
        self.assertEqual([entry["engine"] for entry in samples],
                         ["baozi", "assimp", "assimp", "baozi", "baozi", "assimp",
                          "assimp", "baozi", "baozi", "assimp"])
        self.assertEqual([entry["phase"] for entry in samples[:2]], ["warmup", "warmup"])
        self.assertEqual(len(samples), 10)
        for sample in samples:
            self.assertEqual(sample["status"], "success")
            self.assertEqual(sample["case_id"], "obj-grid-2")
            self.assertEqual(sample["timeout_seconds"], 5)
            self.assertIn("--max-work-units", sample["probe_argv"])
            self.assertEqual(sample["computation_scalar_bytes"], 8)
            self.assertEqual(sample["output_scalar_bytes"], 8)
            self.assertIn("synthetic diagnostic", (artifacts / sample["stderr"]).read_text())
        series = report["series"][0]
        self.assertEqual(series["engines"]["baozi"]["stage_ns"],
                         {"count": 4, "median": 25, "iqr": 15, "min": 10, "max": 40})
        self.assertEqual(series["ratios"]["stage_ns"]["samples"], [2, 2, 2, 2])
        self.assertEqual(series["ratios"]["stage_ns"]["statistics"]["median"], 2)
        self.assertEqual(series["matched_pairs"], 4)
        self.assertEqual(report["corpus"]["sha256"], hashlib.sha256(
            (self.root / "comparison-cases.json").read_bytes()).hexdigest())
        self.assertTrue(report["binaries"]["baozi"]["sha256"])
        retained_manifest = report["binaries"]["assimp"]["build_manifest"]
        self.assertEqual((artifacts / retained_manifest["artifact"]).read_bytes(), build_manifest.read_bytes())
        self.assertTrue(report["binaries"]["assimp"]["build_manifest_binary_verified"])
        self.assertIn("git_dirty", report)
        self.assertTrue(report["source_manifests"])
        if sys.platform != "linux":
            self.assertIsNone(samples[0]["peak_rss_bytes"])
            self.assertTrue(samples[0]["rss_unavailable_reason"])
        self.assertIsNone(report["heap"]["metrics"])

    def test_stale_build_manifest_rejects_before_any_worker_launch(self):
        manifest = self.root / "stale-build.json"
        manifest.write_text(json.dumps({"binary": {"sha256": "0" * 64}}), encoding="utf-8")
        result, report, _ = self.collect(
            [self.case()], extra=["--assimp-build-manifest", str(manifest)],
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(report["status"], "failed")
        self.assertIn("executable SHA-256 does not match", report["error"])
        self.assertFalse(any(row["kind"] == "comparison" for row in report["commands"]))
        self.assertFalse((self.root / "state.json").exists())

    def test_build_metadata_without_binary_identity_cannot_attest_the_worker(self):
        manifest = self.root / "unbound-build.json"
        manifest.write_text('{"compiler":"synthetic"}', encoding="utf-8")
        result, report, _ = self.collect(
            [self.case()], extra=["--assimp-build-manifest", str(manifest)],
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(report["status"], "failed")
        self.assertIn("build manifest must identify binary.sha256", report["error"])
        self.assertFalse((self.root / "state.json").exists())

    def test_python310_large_input_hash_includes_the_final_partial_chunk(self):
        payload = bytes(range(256)) * (3 * 4096) + b"tail-check"
        expected_hash = hashlib.sha256(payload).hexdigest()
        cases = [self.case("large-valid"), self.case("large-tampered")]
        for case in cases:
            (self.root / case["path"]).write_bytes(payload)
            case["files"] = [{"path": case["path"], "bytes": len(payload), "sha256": expected_hash}]
        (self.root / cases[1]["path"]).write_bytes(payload[:-1] + b"!")
        result, report, artifacts = self.collect(cases, python310_hashlib=True)
        self.assertEqual(report["status"], "partial", result.stderr)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["series"][0]["matched_pairs"], 1)
        self.assertIsNone(report["series"][0]["input_validation_error"])
        self.assertIn("SHA-256", report["series"][1]["input_validation_error"])
        self.assertEqual(report["series"][1]["matched_pairs"], 0)
        self.assertEqual(report["binaries"]["baozi"]["sha256"],
                         hashlib.sha256(Path(sys.executable).read_bytes()).hexdigest())
        for sample in report["commands"]:
            if sample.get("case_id") != "large-tampered":
                continue
            self.assertEqual(sample["status"], "semantic_mismatch")
            self.assertFalse(sample["launched"])
            self.assertTrue((artifacts / sample["stdout"]).is_file())
            self.assertTrue((artifacts / sample["stderr"]).is_file())

    def test_comparison_lane_checkpoints_exclude_in_progress_children(self):
        result, report, _ = self.collect([self.case("first"), self.case("second")],
            repetitions=2, warmups=1,
            config={"checkpoint_report": str(self.root / "artifacts/report.json")})
        self.assertEqual(result.returncode, 0, result.stderr)
        samples = [entry for entry in report["commands"] if entry["kind"] == "comparison"]
        self.assertEqual(len(samples), 12)
        for sample in samples:
            checkpoint = sample["worker_json"]
            if sample["case_id"] == "first":
                self.assertEqual(checkpoint["checkpoint_comparison_count"], 0)
                self.assertEqual(checkpoint["checkpoint_cases"], [])
            else:
                self.assertEqual(checkpoint["checkpoint_comparison_count"], 6)
                self.assertEqual(checkpoint["checkpoint_cases"], ["first"])
        self.assertEqual([series["matched_pairs"] for series in report["series"]], [2, 2])

    def test_failures_preserve_logs_and_continue_to_later_successful_cases(self):
        bad_facts = {**FACTS, "tuples": 25}
        cases = [
            self.case("before"),
            self.case("malformed", fault={"assimp": "malformed"}),
            self.case("timeout", fault={"baozi": "timeout"}),
            self.case("crash", fault={"assimp": "crash"}),
            self.case("semantic", observations={"baozi": bad_facts}),
            self.case("rejected", overrides={"assimp": {"status": "policy_rejected",
                "diagnostic": "work limit", "failure_stage": "import",
                "stage_ns": None, "observation": None, "output": None}}),
            self.case("after"),
        ]
        result, report, artifacts = self.collect(cases, timeout=0.6)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(report["status"], "partial")
        self.assertEqual([series["matched_pairs"] for series in report["series"]], [1, 0, 0, 0, 0, 0, 1])
        samples = [entry for entry in report["commands"] if entry["kind"] == "comparison"]
        by_case_engine = {(entry["case_id"], entry["engine"]): entry for entry in samples}
        for case_id, engine, status in (("malformed", "assimp", "semantic_mismatch"),
                                        ("timeout", "baozi", "timeout"),
                                        ("crash", "assimp", "crash"),
                                        ("semantic", "baozi", "semantic_mismatch"),
                                        ("rejected", "assimp", "policy_rejected")):
            self.assertEqual(by_case_engine[case_id, engine]["status"], status)
        self.assertEqual(by_case_engine["crash", "assimp"]["returncode"], 7)
        self.assertEqual(by_case_engine["rejected", "assimp"]["stage"], "pipeline")
        self.assertEqual(by_case_engine["rejected", "assimp"]["failure_stage"], "import")
        self.assertEqual(by_case_engine["rejected", "assimp"]["failure_reason"], "work limit")
        self.assertIn("started before timeout", (artifacts / by_case_engine["timeout", "baozi"]["stdout"]).read_text())
        self.assertIn("timeout diagnostic", (artifacts / by_case_engine["timeout", "baozi"]["stderr"]).read_text())
        self.assertIn("{broken-json", (artifacts / by_case_engine["malformed", "assimp"]["stdout"]).read_text())
        self.assertEqual(by_case_engine["semantic", "baozi"]["worker_json"]["observation"]["tuples"], 25)
        self.assertEqual(len(samples), 14)

    def test_missing_executable_keeps_baozi_results_for_every_case(self):
        missing = str(self.root / "missing-assimp.exe")
        result, report, _ = self.collect([self.case("first"), self.case("second")],
                                        config={"engine": "baozi"}, extra=["--assimp-probe", missing])
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["status"], "partial")
        self.assertIsNone(report["binaries"]["assimp"]["sha256"])
        for series in report["series"]:
            self.assertEqual(series["engines"]["baozi"]["stage_ns"]["count"], 1)
            self.assertEqual(series["engines"]["assimp"]["statuses"], {"crash": 1})
            self.assertIsNone(series["ratios"]["stage_ns"]["statistics"])

    def test_nonfinite_json_does_not_break_partial_report_serialization(self):
        result, report, artifacts = self.collect([
            self.case("overflow", fault={"assimp": "overflow-json"}), self.case("after"),
        ])
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["status"], "partial")
        self.assertEqual(report["series"][1]["matched_pairs"], 1)
        bad = next(entry for entry in report["commands"]
                   if entry.get("case_id") == "overflow" and entry.get("engine") == "assimp")
        self.assertEqual(bad["status"], "semantic_mismatch")
        self.assertIn("1e999", (artifacts / bad["stdout"]).read_text())

    def test_requested_lane_and_known_metric_types_are_enforced(self):
        changes = [
            {"case_id": "different-case"}, {"stage": "extract"}, {"io_mode": "file"},
            {"fields": "position"}, {"normal_policy": "missing"}, {"scalar_bytes": 8},
            {"stage_ns": True}, {"output": None}, {"status": "invented"},
            {"observation": {**FACTS, "draws": True}}, {"output_scalar_bytes": 4},
        ]
        cases = [self.case(f"invalid-{index}", overrides={"assimp": change})
                 for index, change in enumerate(changes)]
        result, report, _ = self.collect(cases)
        self.assertEqual(result.returncode, 1)
        for index, series in enumerate(report["series"]):
            with self.subTest(change=changes[index]):
                self.assertEqual(series["engines"]["assimp"]["statuses"], {"semantic_mismatch": 1})
                self.assertEqual(series["matched_pairs"], 0)
                self.assertIsNone(series["ratios"]["stage_ns"]["statistics"])

    def test_partial_repetitions_use_only_matched_pairs_for_ratio_statistics(self):
        case = self.case(indexed_overrides={"assimp": {"1": {"status": "semantic_mismatch"}}})
        result, report, _ = self.collect([case], repetitions=4,
            config={"times": {"baozi": [10, 20, 30, 40], "assimp": [20, 40, 60, 80]}})
        self.assertEqual(result.returncode, 1)
        series = report["series"][0]
        self.assertEqual(series["engines"]["baozi"]["stage_ns"]["median"], 25)
        self.assertEqual(series["matched_pairs"], 3)
        ratio = series["ratios"]["stage_ns"]
        self.assertEqual(ratio["samples"], [2, 2, 2])
        self.assertEqual(ratio["repetitions"], [1, 3, 4])
        self.assertEqual(ratio["matched_engine_statistics"]["baozi"]["median"], 30)
        self.assertEqual(ratio["matched_engine_statistics"]["assimp"]["median"], 60)

    def test_import_keeps_missing_obj_normals_without_preparing(self):
        expected = {**FACTS, "uv0_bounds": None}
        native = {**expected, "normal_bounds": None}
        case = self.case("obj-repeated-1", family="obj-repeated", fields="position-normal",
                         normal_policy="missing", expected=expected,
                         stage_observations={"import": {"baozi": native, "assimp": native}})
        result, report, _ = self.collect([case], stages=("import",))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(report["series"][0]["matched_pairs"], 1)
        samples = [entry for entry in report["commands"] if entry["kind"] == "comparison"]
        for sample in samples:
            self.assertIsNone(sample["worker_json"]["observation"]["normal_bounds"])
            self.assertIsNone(sample["output"])
            self.assertEqual(sample["probe_argv"][4], "import")

    def test_gltf_native_uv_is_checked_in_its_import_convention_only(self):
        expected = {**FACTS, "uv0_bounds": [[0.25, 0.125], [0.75, 0.625]]}
        native = {**expected, "uv0_bounds": [[0.25, 0.375], [0.75, 0.875]]}
        case = self.case("gltf-shared-1", family="gltf-shared", path="gltf-shared-1.glb",
                         expected=expected, stage_observations={"import": {"assimp": native}},
                         omit_fields=["case_id"])
        result, report, _ = self.collect([case], stages=("import", "prepare", "extract", "pipeline"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([series["matched_pairs"] for series in report["series"]], [1, 1, 1, 1])
        samples = [entry for entry in report["commands"] if entry["kind"] == "comparison"]
        self.assertEqual(samples[1]["uv0_convention"], "assimp-native-inverted-v")
        self.assertTrue(all(entry["uv0_convention"] == "source" for entry in samples[2:]))
        self.assertNotIn("case_id", samples[1]["worker_json"])

    def double_case(self, case_id="ply-f64-neighbors", **updates):
        values = dict(family="ply-neighbors", path=case_id + ".ply", precision="f64", fields="position",
                      expected={**FACTS, "tuples": 3, "normal_bounds": None, "uv0_bounds": None,
                                "bounds": [[1, 0, 0], [1 + 2 ** -30, 1, 0]], "area": 2 ** -31})
        values.update(updates)
        return self.case(case_id, **values)

    def test_missing_double_probe_is_explicitly_unavailable_without_f32_fallback(self):
        result, report, _ = self.collect([self.double_case()], config={"engine": "baozi"})
        self.assertEqual(result.returncode, 1)
        sample = next(entry for entry in report["commands"] if entry.get("engine") == "assimp")
        self.assertEqual(sample["status"], "unsupported")
        self.assertEqual(sample["binary"], "assimp_double")
        self.assertFalse(sample["launched"])
        self.assertIn("--assimp-double-probe", sample["failure_reason"])
        self.assertEqual(report["series"][0]["matched_pairs"], 0)

    def test_f64_lane_requires_double_probe_source_width_and_width_sensitive_facts(self):
        rounded = {**self.double_case()["expected"], "bounds": [[1, 0, 0], [1, 1, 0]], "area": 0}
        cases = [self.double_case(),
                 self.double_case("rounded", observations={"assimp": rounded}),
                 self.double_case("wrong-width", overrides={"assimp": {"scalar_bytes": 4}}),
                 self.case("f32")]
        result, report, _ = self.collect(cases, extra=["--assimp-double-probe", sys.executable])
        self.assertEqual(result.returncode, 1)
        self.assertEqual([series["matched_pairs"] for series in report["series"]], [1, 0, 0, 1])
        samples = [entry for entry in report["commands"] if entry.get("engine") == "assimp"]
        self.assertEqual([entry["binary"] for entry in samples],
                         ["assimp_double", "assimp_double", "assimp_double", "assimp"])
        self.assertEqual([entry["status"] for entry in samples],
                         ["success", "semantic_mismatch", "semantic_mismatch", "success"])

    def test_exact_case_stage_and_io_selection_respects_point_endpoints(self):
        point = self.case("ply-points-1024", family="ply-points", fields="position", topology="points",
                          stages=["import", "extract", "pipeline"],
                          expected={**FACTS, "area": 0, "normal_bounds": None, "uv0_bounds": None})
        result, report, _ = self.collect([self.case("grid"), point, self.case("ignored")],
            stages=("prepare", "import"), io_modes=("file",),
            extra=["--case", "ply-points-1024", "--case", "grid"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(report["case_order"], ["ply-points-1024", "grid"])
        self.assertEqual([(series["case_id"], series["stage"], series["io_mode"]) for series in report["series"]],
                         [("ply-points-1024", "import", "file"), ("grid", "prepare", "file"), ("grid", "import", "file")])

    def test_comparison_defaults_are_twenty_retained_and_three_warmup_processes(self):
        result, report, _ = self.collect([self.case()], repetitions=None, warmups=None)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((report["repetitions"], report["warmups"]), (20, 3))
        samples = [entry for entry in report["commands"] if entry["kind"] == "comparison"]
        self.assertEqual(len(samples), 46)
        self.assertEqual(sum(entry["phase"] == "warmup" for entry in samples), 6)
        self.assertEqual(report["series"][0]["matched_pairs"], 20)

    def test_full_asset_workflow_keeps_its_own_arguments_and_summary(self):
        case = self.case("full-asset")
        (self.root / "worker.json").write_text(json.dumps({"engine": "baozi", "cases": {case["id"]: case}}))
        artifacts = self.root / "asset-artifacts"
        result = subprocess.run([sys.executable, str(SCRIPT), "--asset", case["path"],
            "--root", str(self.root), "--baozi-probe", sys.executable, "--timeout", "5",
            "--artifact-root", str(artifacts), "--assume-standard-space", "--allocation-metrics",
            "--max-work-units", "1200", "--repetitions", "2"], capture_output=True, text=True, timeout=30,
            env={**os.environ, "BAOZI_TEST_SECRET": "synthetic-secret-must-not-be-recorded"})
        self.assertEqual(result.returncode, 0, result.stderr)
        text = (artifacts / "report.json").read_text()
        self.assertNotIn("synthetic-secret-must-not-be-recorded", text)
        report = json.loads(text)
        self.assertEqual(report["mode"], "asset")
        self.assertNotIn("series", report)
        samples = [entry for entry in report["commands"] if entry["kind"] == "asset"]
        self.assertEqual(len(samples), 2)
        for sample in samples:
            self.assertEqual(sample["summary"]["workflow"], "full-asset")
            self.assertEqual(sample["summary"]["args"], ["--assume-standard-space", "--max-work-units", "1200"])
            self.assertNotIn("--comparison", sample["probe_argv"])

    def test_timeout_is_required_finite_and_positive_before_any_run(self):
        for value in (None, "0", "-1", "nan", "inf"):
            with self.subTest(timeout=value):
                command = [sys.executable, str(SCRIPT)]
                if value is not None:
                    command += ["--timeout", value]
                result = subprocess.run(command, capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 2)
                self.assertIn("--timeout", result.stderr)

    @unittest.skipUnless(sys.platform == "linux" and Path("/usr/bin/time").is_file(), "Linux GNU time required")
    def test_linux_rss_measures_a_touched_buffer_over_the_whole_child(self):
        result, report, artifacts = self.collect([self.case()], config={"touch_bytes": 16 * 1024 * 1024})
        self.assertEqual(result.returncode, 0, result.stderr)
        for sample in report["commands"]:
            if sample["kind"] != "comparison":
                continue
            self.assertEqual(sample["rss_scope"], "whole_child_lifetime")
            self.assertGreater(sample["peak_rss_bytes"], 16 * 1024 * 1024)
            self.assertIsNone(sample["rss_unavailable_reason"])
            self.assertEqual(sample["peak_rss_bytes"], int((artifacts / sample["rss_log"]).read_text()) * 1024)

    @unittest.skipUnless(sys.platform == "win32", "Windows process API required")
    def test_windows_lifetime_peaks_survive_child_exit_without_becoming_linux_rss(self):
        touched = 32 * 1024 * 1024
        result, report, _ = self.collect([self.case()], repetitions=2, config={"touch_bytes": touched})
        self.assertEqual(result.returncode, 0, result.stderr)
        for sample in report["commands"]:
            if sample["kind"] != "comparison":
                continue
            self.assertEqual(sample["windows_memory_scope"], "whole_child_lifetime")
            self.assertGreater(sample["windows_peak_working_set_bytes"], touched)
            self.assertGreater(sample["windows_peak_commit_bytes"], touched)
            self.assertIsNone(sample["windows_memory_unavailable_reason"])
            self.assertIsNone(sample["peak_rss_bytes"])
            self.assertTrue(sample["rss_unavailable_reason"])
        for metric in ("windows_peak_working_set_bytes", "windows_peak_commit_bytes"):
            self.assertEqual(report["series"][0]["ratios"][metric]["statistics"]["count"], 2)
            for engine in ("baozi", "assimp"):
                self.assertEqual(report["series"][0]["engines"][engine][metric]["count"], 2)


if __name__ == "__main__":
    unittest.main()
