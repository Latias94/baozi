"""Generate deterministic V1 asset inputs and a manifest, without running Baozi."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys
import zlib


def write_json(path: Path, value: dict) -> None:
    with path.open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def write_grid(root: Path, size: int) -> dict:
    relative = f"static-quads-{size}.obj"
    with (root / relative).open("x", encoding="ascii", newline="\n") as stream:
        stream.write("# V1 planar integer grid; source conventions are caller assumptions.\n")
        stream.write("o grid\ns 1\n")
        for y in range(size + 1):
            for x in range(size + 1):
                stream.write(f"v {x} {y} 0\n")
        for y in range(size):
            for x in range(size):
                first = y * (size + 1) + x + 1
                stream.write(
                    f"f {first} {first + 1} {first + size + 2} {first + size + 1}\n"
                )
    return {
        "id": f"static-quads-{size}",
        "family": "static-quads",
        "path": relative,
        "parameters": {"grid_size": size},
        "source_assumptions": {
            "handedness": "right",
            "up_axis": "+Y",
            "front_axis": "-Z",
            "unit_scale_to_meters": 1.0,
            "front_face_winding": "counterclockwise",
        },
        "source_counts": {
            "vertices": (size + 1) ** 2,
            "quad_faces": size**2,
            "corners": 4 * size**2,
            "meshes": 1,
            "primitives": 1,
            "authored_normals": 0,
            "external_resources": 0,
        },
        "expected": {
            "triangle_faces": 2 * size**2,
            "draw_vertices": (size + 1) ** 2,
            "draw_indices": 6 * size**2,
            "mesh_local_bounds": [[-100 * size, 0, 0], [0, 100 * size, 0]],
            "normal": [0, 0, 1],
            "opened_assets": 1,
            "changed_operations": 3,
        },
    }


def pixel_png() -> bytes:
    """An original opaque RGBA texel, with no third-party image content."""
    def chunk(kind: bytes, data: bytes) -> bytes:
        checksum = zlib.crc32(kind + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", checksum)

    # A fixed uncompressed DEFLATE block avoids compression-library version differences.
    scanline = b"\x00\x20\x80\xe0\xff"
    compressed = (
        b"\x78\x01\x01"
        + struct.pack("<HH", len(scanline), len(scanline) ^ 0xFFFF)
        + scanline
        + struct.pack(">I", zlib.adler32(scanline) & 0xFFFFFFFF)
    )
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", compressed)
        + chunk(b"IEND", b"")
    )


def write_shared_primitives(root: Path, count: int) -> dict:
    directory = root / f"shared-primitives-{count}"
    directory.mkdir()
    # All draws reference these same accessors. Geometry stays at four vertices / six
    # indices per primitive, while primitive count varies. Normals are deliberately absent.
    positions = [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0]
    uvs = [0, 0, 1, 0, 1, 1, 0, 1]
    geometry = struct.pack("<12f8f6H", *positions, *uvs, 0, 1, 2, 0, 2, 3)
    for name, data in [("geometry.bin", geometry), ("pixel.png", pixel_png())]:
        with (directory / name).open("xb") as stream:
            stream.write(data)
    document = {
        "asset": {"version": "2.0", "generator": "Baozi V1 workload generator"},
        "scene": 0,
        "scenes": [{"nodes": [0, 1]}],
        "nodes": [
            {"name": "first", "mesh": 0},
            {"name": "translated", "mesh": 0, "translation": [2, 0, 0]},
        ],
        "meshes": [{
            "name": "shared-quad",
            "primitives": [
                {"attributes": {"POSITION": 0, "TEXCOORD_0": 1},
                 "indices": 2, "mode": 4, "material": index % 2}
                for index in range(count)
            ],
        }],
        "materials": [
            {"name": name, "pbrMetallicRoughness": {
                "baseColorFactor": color,
                "baseColorTexture": {"index": 0, "texCoord": 0},
                "metallicFactor": 0,
                "roughnessFactor": 1,
            }}
            for name, color in [("white", [1, 1, 1, 1]), ("tinted", [0.5, 1, 0.5, 1])]
        ],
        "textures": [{"source": 0}],
        "images": [{"uri": "pixel.png"}],
        "buffers": [{"uri": "geometry.bin", "byteLength": len(geometry)}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 48, "target": 34962},
            {"buffer": 0, "byteOffset": 48, "byteLength": 32, "target": 34962},
            {"buffer": 0, "byteOffset": 80, "byteLength": 12, "target": 34963},
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3",
             "min": [0, 0, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC2"},
            {"bufferView": 2, "componentType": 5123, "count": 6, "type": "SCALAR"},
        ],
    }
    write_json(directory / "model.gltf", document)
    return {
        "id": directory.name,
        "family": "shared-primitives",
        "path": f"{directory.name}/model.gltf",
        "parameters": {"primitive_count": count},
        "source_assumptions": None,
        "source_conventions": "glTF 2.0: right-handed, +Y up, +Z front, meters, CCW",
        "source_counts": {
            "nodes": 2, "meshes": 1, "primitives": count, "mesh_instances": 2,
            "unique_position_rows": 4, "vertices_per_primitive": 4,
            "indices_per_primitive": 6, "authored_normals": 0,
            "materials": 2, "textures": 1, "images": 1, "external_resources": 2,
            "geometry_bytes": len(geometry),
        },
        "expected": {
            "draw_vertices": 4 * count,
            "draw_indices": 6 * count,
            "mesh_local_bounds": [[0, 0, 0], [100, 100, 0]],
            "normal": [0, 0, -1],
            "second_node_translation": [200, 0, 0],
            "opened_assets": 3,
            "changed_operations": count + 1,
        },
    }


def positive_int(value: str) -> int:
    result = int(value)
    if result < 1:
        raise argparse.ArgumentTypeError("sizes must be positive integers")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", required=True, type=Path,
                        help="new or empty output directory; existing files are never replaced")
    parser.add_argument("--grid-size", action="append", type=positive_int,
                        help="quad cells per side; repeatable (default: 128 and 256)")
    parser.add_argument("--primitive-count", action="append", type=positive_int,
                        help="fixed-geometry draws per mesh; repeatable (default: 8 and 32)")
    arguments = parser.parse_args()
    grids = arguments.grid_size or [128, 256]
    primitives = arguments.primitive_count or [8, 32]
    if len(set(grids)) != len(grids) or len(set(primitives)) != len(primitives):
        parser.error("duplicate sizes would produce duplicate output paths")
    root = arguments.output_dir.resolve()
    try:
        root.mkdir(parents=True, exist_ok=True)
        if any(root.iterdir()):
            parser.error(f"output directory must be empty: {root}")
        workloads = [write_grid(root, size) for size in grids]
        workloads.extend(write_shared_primitives(root, count) for count in primitives)
        files = []
        for path in sorted(root.rglob("*")):
            if path.is_file():
                data = path.read_bytes()
                files.append({"path": path.relative_to(root).as_posix(),
                              "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
        manifest = {
            "schema_version": 1,
            "workload_version": "V1",
            "generator": {"path": "scripts/generate_asset_workloads.py",
                          "sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()},
            "expectation_basis": "Construction formulas and authored source bytes, not Baozi output.",
            "target": "left-handed, +Y up, -Z front, centimeters, counterclockwise fronts",
            "notes": [
                "Only inputs are generated; no import, build, timing, allocation or RSS run occurs.",
                "Draw vertices are indexed representatives; draw indices count triangle corners.",
                "Mesh-local counts do not multiply by node instances; bounds exclude node transforms.",
                "Generated geometry and pixel data are original project test content.",
                "The third V1 family is unchanged vendored CesiumMan; it is not copied or modified.",
                "File checksums cover generated inputs; manifest.json does not checksum itself.",
            ],
            "workloads": workloads,
            "files": files,
        }
        write_json(root / "manifest.json", manifest)
    except OSError as error:
        parser.exit(1, f"cannot generate workloads: {error}\n")
    print(f"Generated {len(workloads)} workloads: {root / 'manifest.json'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
