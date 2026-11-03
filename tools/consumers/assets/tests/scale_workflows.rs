use std::{collections::BTreeSet, path::PathBuf, process::Command, time::SystemTime};

use baozi::{
    AnimationInterpolation, TargetSpace, TextureRole,
    postprocess::{NormalOptions, TangentPolicy},
};
use baozi_asset_consumer::{
    ApplicationAsset, ChannelTarget, NormalIntent, Options, Output, Result, process_file,
};
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn expectations() -> Value {
    serde_json::from_str(include_str!("../fixtures/workload-expectations.json"))
        .expect("valid independently derived workload expectations")
}

fn family<'a>(expected: &'a Value, id: &str) -> &'a Value {
    expected["families"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == id)
        .expect("recorded workload family")
}

fn count(value: &Value) -> usize {
    usize::try_from(value.as_u64().expect("nonnegative source count")).unwrap()
}

fn integers(value: &Value) -> Vec<usize> {
    value.as_array().unwrap().iter().map(count).collect()
}

fn floats(value: &Value) -> Vec<f32> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap() as f32)
        .collect()
}

struct Generated(PathBuf);

impl Generated {
    fn new(grids: &[usize], primitives: &[usize]) -> Result<Self> {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "baozi-asset-workloads-{}-{nonce}",
            std::process::id()
        ));
        // Successful exclusive creation establishes ownership before any cleanup is possible.
        std::fs::create_dir(&root)?;
        let generated = Self(root);
        let python = std::env::var_os("BAOZI_WORKLOAD_PYTHON")
            .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into());
        let mut command = Command::new(python);
        command
            .arg(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../scripts/generate_asset_workloads.py"),
            )
            .arg("--output-dir")
            .arg(&generated.0);
        for size in grids {
            command.arg("--grid-size").arg(size.to_string());
        }
        for size in primitives {
            command.arg("--primitive-count").arg(size.to_string());
        }
        let output = command.output()?;
        if !output.status.success() {
            return Err(format!(
                "workload generator failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        Ok(generated)
    }
}

impl Drop for Generated {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn assert_released(output: &Output, opened: u64, changed: usize, reconstructions: u32) {
    assert!(output.import_evidence.loss_evidence().is_empty());
    assert_eq!(output.import_evidence.ledger().current_live_bytes(), 0);
    assert_eq!(output.import_evidence.ledger().active_reservations(), 0);
    assert_eq!(output.metrics.opened_assets, opened);
    assert_eq!(output.metrics.changed_operations, changed);
    assert_eq!(
        output.metrics.selected_geometry_reconstructions,
        reconstructions
    );
    assert_eq!(
        output.metrics.traced_primitive_owners,
        output.asset.draws.len()
    );
    assert!(output.metrics.traced_source_objects > 0);
    assert!(output.metrics.traced_provenance_nodes >= output.metrics.traced_source_objects);
}

fn assert_grid(asset: &ApplicationAsset, size: usize) {
    assert_eq!(asset.mesh_count, 1);
    assert_eq!(asset.draws.len(), 1);
    assert_eq!(asset.vertex_count(), (size + 1).pow(2));
    let draw = &asset.draws[0];
    assert_eq!(draw.indices.len(), 6 * size * size);
    assert!(draw.influences.is_none());
    assert!(draw.morphs.is_empty());
    assert_eq!(
        asset.bounds(),
        Some([
            [-100.0 * size as f64, 0.0, 0.0],
            [0.0, 100.0 * size as f64, 0.0]
        ])
    );
    let mut positions = BTreeSet::new();
    for vertex in &draw.vertices {
        let [x, y, z] = vertex.position;
        assert_eq!(z, 0.0);
        let (column, row) = (-x / 100.0, y / 100.0);
        assert!(column >= 0.0 && column <= size as f64 && column.fract() == 0.0);
        assert!(row >= 0.0 && row <= size as f64 && row.fract() == 0.0);
        assert!(positions.insert((column as usize, row as usize)));
        assert_eq!(vertex.normal, Some([0.0, 0.0, 1.0]));
        assert!(vertex.tangent.is_none());
    }
    assert!(draw.uv_streams.is_empty());
    for triangle in draw.indices.chunks_exact(3) {
        let [a, b, c] = std::array::from_fn::<_, 3, _>(|i| {
            draw.vertices[usize::try_from(triangle[i]).unwrap()].position
        });
        // Every source unit quad contributes two CCW triangles of area 5,000 cm^2.
        let cross_z = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        assert_eq!(cross_z, 10_000.0);
    }
}

fn assert_shared_primitives(asset: &ApplicationAsset, primitives: usize, image: &[u8]) {
    assert_eq!(asset.mesh_count, 1);
    assert_eq!(asset.nodes.len(), 2);
    assert_eq!(asset.roots, [0, 1]);
    assert_eq!(asset.instances.len(), 2);
    assert_eq!(asset.instances[0].mesh, 0);
    assert_eq!(asset.instances[1].mesh, 0);
    assert_eq!(asset.instances[0].node, 0);
    assert_eq!(asset.instances[1].node, 1);
    assert!(
        asset
            .instances
            .iter()
            .all(|instance| instance.skin.is_none())
    );
    assert!(asset.skins.is_empty());
    assert!(asset.animations.is_empty());
    assert_eq!(asset.draws.len(), primitives);
    assert_eq!(asset.vertex_count(), 4 * primitives);
    assert_eq!(asset.images.len(), 1);
    assert_eq!(asset.images[0].encoded_bytes, image);
    assert_eq!(asset.materials.len(), 2);
    for (material, name) in asset.materials.iter().zip(["white", "tinted"]) {
        assert_eq!(material.name.as_deref(), Some(name));
        assert_eq!(material.bindings.len(), 1);
        assert_eq!(material.bindings[0].role, TextureRole::BaseColor);
        assert_eq!(asset.textures[material.bindings[0].texture].image, 0);
        assert_eq!(material.bindings[0].uv_set, 0);
    }
    let translation = asset.nodes[1]
        .transform
        .authored_trs()
        .unwrap()
        .translation();
    assert_eq!(
        [translation.x, translation.y, translation.z],
        [200.0, 0.0, 0.0]
    );
    assert_eq!(asset.bounds(), Some([[0.0, 0.0, 0.0], [100.0, 100.0, 0.0]]));
    let positions = [
        [0.0, 0.0, 0.0],
        [100.0, 0.0, 0.0],
        [100.0, 100.0, 0.0],
        [0.0, 100.0, 0.0],
    ];
    let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    for (primitive, draw) in asset.draws.iter().enumerate() {
        assert_eq!(
            (draw.mesh, draw.primitive, draw.material),
            (
                0,
                primitive,
                baozi::MaterialAssignment::Authored(primitive % 2)
            )
        );
        assert_eq!((draw.vertices.len(), draw.indices.len()), (4, 6));
        assert!(draw.influences.is_none());
        assert!(draw.morphs.is_empty());
        // glTF's +Z front reflects to -Z: reverse each source triangle, preserving UV tuples.
        for (&index, source) in draw.indices.iter().zip([0, 2, 1, 0, 3, 2]) {
            let vertex = &draw.vertices[usize::try_from(index).unwrap()];
            assert_eq!(vertex.position, positions[source]);
            assert_eq!(
                draw.uv_stream(0).unwrap().values[index as usize],
                uvs[source]
            );
            assert_eq!(vertex.normal, Some([0.0, 0.0, -1.0]));
            assert!(vertex.tangent.is_none());
        }
    }
}

fn generated_workflows(grids: &[usize], primitives: &[usize]) -> Result<()> {
    let generated = Generated::new(grids, primitives)?;
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(generated.0.join("manifest.json"))?)?;
    assert_eq!(manifest["workload_version"], "V1");
    let cases = manifest["workloads"].as_array().unwrap();
    assert_eq!(cases.len(), grids.len() + primitives.len());
    let mut outputs = Vec::new();
    for case in cases {
        let options = Options {
            normal_intent: if case["family"] == "static-quads" {
                NormalIntent::GeometryEdit {
                    normals: NormalOptions::default(),
                    tangents: TangentPolicy::Preserve,
                }
            } else {
                NormalIntent::SourceFormat
            },
            source_assumptions: if case["family"] == "static-quads" {
                Some(TargetSpace::STANDARD)
            } else {
                None
            },
            ..Options::default()
        };
        outputs.push(process_file(
            &generated.0,
            case["path"].as_str().unwrap(),
            options,
        )?);
    }
    let image = std::fs::read(
        generated
            .0
            .join(format!("shared-primitives-{}/pixel.png", primitives[0])),
    )?;
    // No Scene or FileSystemAssetIo survives process_file. Remove the owned input directory
    // as well, then read all application buffers and bindings from the retained outputs.
    let removed_root = generated.0.clone();
    drop(generated);
    assert!(
        !removed_root.exists(),
        "owned generated inputs must be removable after import"
    );
    for (case, output) in cases.iter().zip(outputs) {
        match case["family"].as_str().unwrap() {
            "static-quads" => {
                let size = count(&case["parameters"]["grid_size"]);
                assert_eq!(count(&case["source_counts"]["quad_faces"]), size * size);
                assert_eq!(count(&case["expected"]["draw_vertices"]), (size + 1).pow(2));
                assert_eq!(count(&case["expected"]["draw_indices"]), 6 * size * size);
                assert_released(&output, 1, 2, 1);
                let asset = output.asset;
                drop(output.import_evidence);
                assert_grid(&asset, size);
            }
            "shared-primitives" => {
                let size = count(&case["parameters"]["primitive_count"]);
                assert_eq!(count(&case["source_counts"]["unique_position_rows"]), 4);
                assert_eq!(count(&case["expected"]["draw_vertices"]), 4 * size);
                assert_eq!(count(&case["expected"]["draw_indices"]), 6 * size);
                assert_released(&output, 3, 2, 1);
                let asset = output.asset;
                drop(output.import_evidence);
                assert_shared_primitives(&asset, size, &image);
            }
            family => panic!("unexpected generated family: {family}"),
        }
    }
    Ok(())
}

#[test]
fn small_generated_workflows_keep_indexed_geometry_and_shared_resources() -> Result<()> {
    let expected = expectations();
    generated_workflows(
        &integers(&family(&expected, "static-quads")["ci_grid_sizes"]),
        &integers(&family(&expected, "shared-primitives")["ci_primitive_counts"]),
    )
}

#[test]
#[ignore = "manual V1 scale regression: 98,304 and 393,216 triangulated corners; no timing gate"]
fn generated_v1_scale_workflows() -> Result<()> {
    let expected = expectations();
    generated_workflows(
        &integers(&family(&expected, "static-quads")["default_grid_sizes"]),
        &integers(&family(&expected, "shared-primitives")["default_primitive_counts"]),
    )
}

// This deliberately bounded decoder reads the exact dense U16/F32 fixture layout. It uses
// source byte offsets and strides, independently of Baozi's importer or postprocess readers.
struct SourceGltf {
    document: Value,
    binary: Vec<u8>,
}

impl SourceGltf {
    fn rows(&self, index: usize, component: u64, width: usize) -> Vec<&[u8]> {
        let accessor = &self.document["accessors"][index];
        assert_eq!(accessor["componentType"].as_u64(), Some(component));
        assert!(accessor.get("sparse").is_none());
        assert!(!accessor["normalized"].as_bool().unwrap_or(false));
        assert_eq!(
            match accessor["type"].as_str().unwrap() {
                "SCALAR" => 1,
                "VEC2" => 2,
                "VEC3" => 3,
                "VEC4" => 4,
                "MAT4" => 16,
                other => panic!("unsupported fixture accessor type: {other}"),
            },
            width
        );
        let view = &self.document["bufferViews"][count(&accessor["bufferView"])];
        assert_eq!(view["buffer"], 0);
        let start = view.get("byteOffset").map_or(0, count);
        let data = &self.binary[start..start + count(&view["byteLength"])];
        let offset = accessor.get("byteOffset").map_or(0, count);
        let row_bytes = width
            * match component {
                5123 => 2,
                5126 => 4,
                _ => unreachable!(),
            };
        let stride = view.get("byteStride").map_or(row_bytes, count);
        assert!(stride >= row_bytes);
        (0..count(&accessor["count"]))
            .map(|row| &data[offset + row * stride..offset + row * stride + row_bytes])
            .collect()
    }

    fn f32_rows(&self, index: usize, width: usize) -> Vec<Vec<f32>> {
        self.rows(index, 5126, width)
            .into_iter()
            .map(|row| {
                row.chunks_exact(4)
                    .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
                    .collect()
            })
            .collect()
    }

    fn u16_rows(&self, index: usize, width: usize) -> Vec<Vec<u16>> {
        self.rows(index, 5123, width)
            .into_iter()
            .map(|row| {
                row.chunks_exact(2)
                    .map(|bytes| u16::from_le_bytes(bytes.try_into().unwrap()))
                    .collect()
            })
            .collect()
    }
}

#[test]
fn cesium_man_binds_keys_influences_and_image_survive_scene_drop() -> Result<()> {
    let expected = expectations();
    let expected = family(&expected, "cesium-man");
    let relative = expected["path"].as_str().unwrap();
    let primary = fixtures().join(relative);
    let directory = primary.parent().unwrap();
    let source = SourceGltf {
        document: serde_json::from_slice(&std::fs::read(&primary)?)?,
        binary: std::fs::read(directory.join("CesiumMan_data.bin"))?,
    };
    let positions = source.f32_rows(3, 3);
    let normals = source.f32_rows(2, 3);
    let uvs = source.f32_rows(4, 2);
    let joints = source.u16_rows(1, 4);
    let weights = source.f32_rows(5, 4);
    let indices: Vec<_> = source
        .u16_rows(0, 1)
        .into_iter()
        .map(|row| usize::from(row[0]))
        .collect();
    assert_eq!(
        positions.len(),
        count(&expected["primitive"]["vertex_count"])
    );
    assert_eq!(indices.len(), count(&expected["primitive"]["index_count"]));
    assert_eq!(
        indices.iter().copied().collect::<BTreeSet<_>>().len(),
        positions.len()
    );
    assert_eq!(
        &indices[..18],
        integers(&expected["primitive"]["first_source_indices"])
    );
    for lane in 0..3 {
        assert_eq!(
            positions
                .iter()
                .map(|row| row[lane])
                .fold(f32::INFINITY, f32::min),
            expected["primitive"]["position_min"][lane]
                .as_f64()
                .unwrap() as f32
        );
        assert_eq!(
            positions
                .iter()
                .map(|row| row[lane])
                .fold(f32::NEG_INFINITY, f32::max),
            expected["primitive"]["position_max"][lane]
                .as_f64()
                .unwrap() as f32
        );
    }
    let mut histogram = [0; 5];
    for (joint, weight) in joints.iter().zip(&weights) {
        assert!(
            joint
                .iter()
                .all(|&j| usize::from(j) < count(&expected["counts"]["joints"]))
        );
        assert!((weight.iter().map(|&w| f64::from(w)).sum::<f64>() - 1.0).abs() < 1e-6);
        histogram[weight.iter().filter(|&&w| w > 0.0).count()] += 1;
    }
    for (active, &vertices) in histogram.iter().enumerate().skip(1) {
        assert_eq!(
            vertices,
            count(&expected["influences"]["active_count_histogram"][active.to_string()])
        );
    }
    for sample in expected["influences"]["samples"].as_array().unwrap() {
        let vertex = count(&sample["vertex"]);
        assert_eq!(
            joints[vertex]
                .iter()
                .map(|&j| usize::from(j))
                .collect::<Vec<_>>(),
            integers(&sample["joints"])
        );
        assert_eq!(weights[vertex], floats(&sample["weights"]));
    }
    let output = process_file(&fixtures(), relative, Options::default())?;
    assert_released(&output, 3, 1, 0);
    let asset = output.asset;
    drop(output.import_evidence);
    // Every comparison below uses owned application data, after both Scene and evidence drop.
    for (key, actual) in [
        ("nodes", asset.nodes.len()),
        ("meshes", asset.mesh_count),
        ("primitives", asset.draws.len()),
        ("mesh_instances", asset.instances.len()),
        ("skins", asset.skins.len()),
        ("animations", asset.animations.len()),
        ("materials", asset.materials.len()),
        ("images", asset.images.len()),
    ] {
        assert_eq!(actual, count(&expected["counts"][key]), "{key}");
    }
    // Canonical nodes preserve forest order in breadth-first layout; source array indices
    // are source identity, not application node indices. Derive the fixture's mapping from
    // its independent roots/child lists instead of assuming source order equals storage order.
    let roots = integers(&expected["scene_roots"]);
    let mut source_order = roots.clone();
    let mut next = 0;
    while next < source_order.len() {
        let index = source_order[next];
        next += 1;
        if let Some(children) = source.document["nodes"][index].get("children") {
            source_order.extend(integers(children));
        }
        assert!(source_order.len() <= asset.nodes.len());
    }
    assert_eq!(source_order.len(), asset.nodes.len());
    let mut node_indices = vec![usize::MAX; asset.nodes.len()];
    for (canonical, &authored) in source_order.iter().enumerate() {
        assert_eq!(node_indices[authored], usize::MAX);
        node_indices[authored] = canonical;
    }
    assert_eq!(
        asset.roots,
        roots
            .iter()
            .map(|&node| node_indices[node])
            .collect::<Vec<_>>()
    );
    for (node, &index) in asset.nodes.iter().zip(&source_order) {
        assert_eq!(node.name.as_deref(), expected["node_names"][index].as_str());
        let children = source.document["nodes"][index]
            .get("children")
            .map(integers)
            .unwrap_or_default();
        assert_eq!(
            node.children,
            children
                .iter()
                .map(|&child| node_indices[child])
                .collect::<Vec<_>>()
        );
    }
    let instance = &expected["mesh_instances"][0];
    assert_eq!(
        asset.instances[0].node,
        node_indices[count(&instance["node"])]
    );
    assert_eq!(asset.instances[0].mesh, count(&instance["mesh"]));
    assert_eq!(asset.instances[0].skin, Some(count(&instance["skin"])));
    let draw = &asset.draws[0];
    assert_eq!(
        (draw.mesh, draw.primitive, draw.material),
        (0, 0, baozi::MaterialAssignment::Authored(0))
    );
    assert_eq!(
        (draw.vertices.len(), draw.indices.len()),
        (positions.len(), indices.len())
    );
    assert!(draw.morphs.is_empty());
    let influences = draw
        .influences
        .as_ref()
        .ok_or("missing CesiumMan influences")?;
    assert_eq!(influences.offsets.len(), draw.vertices.len() + 1);
    assert_eq!(influences.offsets[0], 0);
    assert_eq!(influences.joints.len(), 4 * draw.vertices.len());
    assert_eq!(influences.weights.len(), influences.joints.len());
    assert_eq!(*influences.offsets.last().unwrap(), influences.joints.len());
    let mut representatives = vec![None; draw.vertices.len()];
    for (actual, source_triangle) in draw.indices.chunks_exact(3).zip(indices.chunks_exact(3)) {
        for (corner, source_lane) in [0, 2, 1].into_iter().enumerate() {
            let source_index = source_triangle[source_lane];
            let index = usize::try_from(actual[corner])?;
            if let Some(previous) = representatives[index] {
                assert_eq!(previous, source_index);
            } else {
                representatives[index] = Some(source_index);
            }
            let vertex = &draw.vertices[index];
            let position = std::array::from_fn::<_, 3, _>(|lane| {
                let scale = if lane == 2 { -100.0 } else { 100.0 };
                f64::from((f64::from(positions[source_index][lane]) * scale) as f32)
            });
            assert_eq!(vertex.position, position);
            assert_eq!(
                vertex.normal,
                Some([
                    f64::from(normals[source_index][0]),
                    f64::from(normals[source_index][1]),
                    -f64::from(normals[source_index][2])
                ])
            );
            assert_eq!(
                draw.uv_stream(0).unwrap().values[index],
                [
                    f64::from(uvs[source_index][0]),
                    f64::from(uvs[source_index][1])
                ]
            );
            assert!(vertex.tangent.is_none());
            let range = influences.offsets[index]..influences.offsets[index + 1];
            assert_eq!(range.len(), 4);
            for (lane, offset) in range.enumerate() {
                assert_eq!(
                    influences.joints[offset],
                    u32::from(joints[source_index][lane])
                );
                assert_eq!(
                    influences.weights[offset].to_bits(),
                    weights[source_index][lane].to_bits()
                );
            }
        }
    }
    assert!(representatives.iter().all(Option::is_some));
    assert_eq!(
        representatives
            .into_iter()
            .flatten()
            .collect::<BTreeSet<_>>()
            .len(),
        positions.len()
    );
    let skin = &asset.skins[0];
    assert_eq!(
        skin.joints,
        integers(&expected["skin"]["joints"])
            .iter()
            .map(|&joint| node_indices[joint])
            .collect::<Vec<_>>()
    );
    let binds = source.f32_rows(count(&expected["skin"]["inverse_bind_accessor"]), 16);
    assert_eq!(skin.inverse_binds.len(), skin.joints.len());
    assert_eq!(binds.len(), skin.joints.len());
    for sample in expected["skin"]["inverse_bind_samples"].as_array().unwrap() {
        assert_eq!(
            binds[count(&sample["joint_index"])],
            floats(&sample["values_column_major"])
        );
    }
    // Independent C * M * C^-1, with C = diag(100, 100, -100, 1).
    let diagonal = [100.0, 100.0, -100.0, 1.0];
    for (actual, source) in skin.inverse_binds.iter().zip(&binds) {
        for column in 0..4 {
            for row in 0..4 {
                let expected =
                    (f64::from(source[column * 4 + row]) * diagonal[row] / diagonal[column]) as f32;
                assert_eq!(actual.cols[column][row], expected);
            }
        }
    }
    let animation = &source.document["animations"][0];
    let clip = &asset.animations[0];
    assert_eq!(
        clip.channels.len(),
        count(&expected["animation"]["channel_count"])
    );
    assert_eq!(clip.name.as_deref(), expected["animation"]["name"].as_str());
    assert_eq!(
        clip.duration,
        expected["animation"]["time_max"].as_f64().unwrap()
    );
    let expected_times: Vec<_> = floats(&expected["animation"]["input_times"])
        .into_iter()
        .map(f64::from)
        .collect();
    for (index, channel) in clip.channels.iter().enumerate() {
        let authored = &animation["channels"][index];
        let sampler = &animation["samplers"][count(&authored["sampler"])];
        let node = node_indices[count(&authored["target"]["node"])];
        let path = authored["target"]["path"].as_str().unwrap();
        let target = match path {
            "translation" => ChannelTarget::Translation(node),
            "rotation" => ChannelTarget::Rotation(node),
            "scale" => ChannelTarget::Scale(node),
            _ => panic!("unexpected CesiumMan channel path: {path}"),
        };
        assert_eq!(channel.target, target);
        assert_eq!(channel.interpolation, AnimationInterpolation::Linear);
        assert_eq!(sampler["interpolation"], "LINEAR");
        let width = if path == "rotation" { 4 } else { 3 };
        assert_eq!(channel.width, width);
        let times: Vec<_> = source
            .f32_rows(count(&sampler["input"]), 1)
            .into_iter()
            .map(|row| f64::from(row[0]))
            .collect();
        assert_eq!(times, expected_times);
        assert_eq!(channel.times, times);
        let values = source.f32_rows(count(&sampler["output"]), width);
        assert_eq!(values.len(), times.len());
        assert_eq!(channel.values.len(), values.len() * width);
        assert!(channel.in_tangents.is_none() && channel.out_tangents.is_none());
        for (key, value) in values.iter().enumerate() {
            for (lane, &value) in value.iter().enumerate() {
                let expected = match (path, lane) {
                    ("translation", 2) => (f64::from(value) * -100.0) as f32,
                    ("translation", _) => (f64::from(value) * 100.0) as f32,
                    ("rotation", 0 | 1) => -value,
                    _ => value,
                };
                assert_eq!(
                    channel.values[key * width + lane],
                    expected,
                    "channel {index}, key {key}, lane {lane}"
                );
            }
        }
    }
    for sample in expected["animation"]["channel_samples"].as_array().unwrap() {
        let authored = &animation["channels"][count(&sample["channel"])];
        assert_eq!(authored["target"]["node"], sample["target_node"]);
        assert_eq!(authored["target"]["path"], sample["target_path"]);
        let width = if sample["target_path"] == "rotation" {
            4
        } else {
            3
        };
        let values = source.f32_rows(count(&sample["output_accessor"]), width);
        for key in sample["samples"].as_array().unwrap() {
            assert_eq!(values[count(&key["key"])], floats(&key["value"]));
            assert_eq!(
                expected_times[count(&key["key"])],
                f64::from(key["time"].as_f64().unwrap() as f32)
            );
        }
    }
    assert_eq!(
        clip.channels.iter().map(|c| c.values.len()).sum::<usize>(),
        count(&expected["animation"]["output_scalar_count"])
    );
    assert_eq!(
        asset.images[0].encoded_bytes,
        std::fs::read(directory.join("CesiumMan_img0.jpg"))?
    );
    assert_eq!(asset.materials[0].bindings.len(), 1);
    let binding = &asset.materials[0].bindings[0];
    assert_eq!(binding.role, TextureRole::BaseColor);
    assert_eq!(
        (asset.textures[binding.texture].image, binding.uv_set),
        (0, 0)
    );
    Ok(())
}
