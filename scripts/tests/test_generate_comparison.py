"""Independent checks of the finite comparison corpus, without either importer."""

import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "generate_asset_workloads.py"
SPEC = importlib.util.spec_from_file_location("generate_asset_workloads", SCRIPT)
generator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(generator)


class ComparisonInputs(unittest.TestCase):
    def test_generation_is_repeatable_and_files_are_bound(self):
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            left = generator.write_comparison(Path(first))
            right = generator.write_comparison(Path(second))
            self.assertEqual(left, right)
            self.assertGreaterEqual(len(left["cases"]), 12)
            for case in left["cases"]:
                self.assertIn(case["precision"], ["f32", "f64"])
                self.assertGreater(case["expected"]["tuples"], 0)
                for entry in case["files"]:
                    data = (Path(first) / entry["path"]).read_bytes()
                    self.assertEqual(len(data), entry["bytes"])
                    self.assertEqual(hashlib.sha256(data).hexdigest(), entry["sha256"])
                    self.assertEqual(data, (Path(second) / entry["path"]).read_bytes())

    def test_grid_source_is_analytic_triangles_with_independent_uvs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cases = generator.write_comparison(root)["cases"]
            case = next(case for case in cases if case["id"] == "obj-grid-2")
            lines = (root / case["path"]).read_text().splitlines()
            self.assertEqual(sum(line.startswith("v ") for line in lines), 9)
            self.assertEqual(sum(line.startswith("f ") for line in lines), 8)
            self.assertEqual(case["expected"]["tuples"], 24)
            self.assertEqual(case["expected"]["area"], 4)
            self.assertEqual(case["expected"]["bounds"], [[0, 0, 0], [2, 2, 0]])
            self.assertIn("f 1/1/1 2/2/1 5/5/1", lines)
            self.assertIn("f 1/1/1 5/5/1 4/4/1", lines)

    def test_glb_has_no_external_or_image_work_and_preserves_placements(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            case = next(case for case in generator.write_comparison(root)["cases"]
                        if case["id"] == "gltf-shared-8")
            data = (root / case["path"]).read_bytes()
            magic, version, length, json_length, kind = struct.unpack_from("<5I", data)
            self.assertEqual((magic, version, length, kind), (0x46546C67, 2, len(data), 0x4E4F534A))
            doc = json.loads(data[20:20 + json_length])
            self.assertNotIn("uri", doc["buffers"][0])
            self.assertNotIn("images", doc)
            self.assertEqual(len(doc["meshes"][0]["primitives"]), 8)
            self.assertEqual(doc["nodes"][1]["translation"], [2, 0, 0])
            self.assertEqual(case["expected"]["placements"], 16)

    def test_double_source_contains_distinct_f32_colliding_values(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            case = next(case for case in generator.write_comparison(root)["cases"]
                        if case["id"] == "ply-f64-neighbors")
            data = (root / case["path"]).read_bytes()
            payload = data.split(b"end_header\n", 1)[1]
            points = struct.unpack_from("<9d", payload)
            self.assertEqual(points[0], 1.0)
            self.assertEqual(points[3], 1.0 + 2.0 ** -30)
            self.assertNotEqual(points[0], points[3])
            self.assertEqual(struct.pack("<f", points[0]), struct.pack("<f", points[3]))


if __name__ == "__main__":
    unittest.main()
