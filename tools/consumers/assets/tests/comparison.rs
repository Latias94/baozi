use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

struct Corpus(PathBuf);

impl Corpus {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "baozi-comparison-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let corpus = Self(root);
        let python = std::env::var_os("BAOZI_WORKLOAD_PYTHON")
            .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into());
        let status = Command::new(python)
            .arg(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../scripts/generate_asset_workloads.py"),
            )
            .arg("--comparison")
            .arg("--output-dir")
            .arg(&corpus.0)
            .status()
            .expect("Python corpus generator starts");
        assert!(status.success(), "corpus generation succeeds");
        corpus
    }
}

impl Drop for Corpus {
    fn drop(&mut self) {
        // This exact directory was exclusively created by this test.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(root: &Path, stage: &str, io: &str, asset: &str, fields: &str, normals: &str) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-asset-consumer"))
        .arg(root)
        .args([
            asset,
            "--comparison",
            stage,
            "--io-mode",
            io,
            "--fields",
            fields,
            "--normals",
            normals,
        ])
        .output()
        .expect("comparison worker starts");
    assert!(
        output.status.success(),
        "{stage}/{io}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("one JSON observation")
}

#[test]
fn every_endpoint_has_its_own_output_lifetime_and_no_hidden_conversion() {
    let corpus = Corpus::new();
    for io in ["memory", "file"] {
        for stage in ["import", "prepare", "extract", "pipeline"] {
            let result = run(
                &corpus.0,
                stage,
                io,
                "obj-grid-2.obj",
                "position-normal-uv0",
                "preserve",
            );
            assert_eq!(result["schema"], "baozi-assimp-observation/v1");
            assert_eq!(result["stage"], stage);
            assert_eq!(result["io_mode"], io);
            assert_eq!(result["status"], "success");
            assert_eq!(
                result["observation"]["bounds"],
                serde_json::json!([[0.0, 0.0, 0.0], [2.0, 2.0, 0.0]])
            );
            assert_eq!(result["observation"]["tuples"], 24);
            assert_eq!(result["observation"]["area"], 4.0);
            assert_eq!(
                result["output"].is_null(),
                matches!(stage, "import" | "prepare")
            );
            assert!(result["stage_ns"].as_u64().unwrap() > 0);
        }
    }
}

#[test]
fn import_does_not_generate_unrequested_missing_normals() {
    let corpus = Corpus::new();
    let imported = run(
        &corpus.0,
        "import",
        "memory",
        "obj-repeated-1.obj",
        "position-normal",
        "missing",
    );
    assert!(imported["observation"]["normal_bounds"].is_null());
    let prepared = run(
        &corpus.0,
        "prepare",
        "memory",
        "obj-repeated-1.obj",
        "position-normal",
        "missing",
    );
    assert_eq!(
        prepared["observation"]["normal_bounds"],
        serde_json::json!([[0.0, 0.0, 1.0], [0.0, 0.0, 1.0]])
    );
}

#[test]
fn shared_draw_definitions_and_placements_are_not_flattened() {
    let corpus = Corpus::new();
    let result = run(
        &corpus.0,
        "pipeline",
        "memory",
        "gltf-shared-8.glb",
        "position-normal-uv0",
        "preserve",
    );
    assert_eq!(result["observation"]["draws"], 8);
    assert_eq!(result["observation"]["placements"], 16);
    assert_eq!(result["observation"]["tuples"], 24);
    assert_eq!(result["observation"]["translation_x_sum"], 16.0);
    assert_eq!(
        result["observation"]["uv0_bounds"],
        serde_json::json!([[0.0, 0.0], [1.0, 1.0]])
    );
}

#[test]
fn wide_geometry_is_not_relabelled_from_its_output_width() {
    let corpus = Corpus::new();
    let result = run(
        &corpus.0,
        "pipeline",
        "memory",
        "ply-f64-neighbors.ply",
        "position",
        "preserve",
    );
    assert_eq!(result["scalar_bytes"], 8);
    assert_eq!(
        result["observation"]["bounds"][1][0].as_f64().unwrap(),
        1.0 + 2_f64.powi(-30)
    );
    assert_eq!(
        result["observation"]["area"].as_f64().unwrap(),
        2_f64.powi(-31)
    );
}

#[test]
fn owned_grid_output_matches_every_analytic_tuple_after_native_teardown() {
    use baozi_asset_consumer::comparison::{Fields, Options, Stage, measure_file};
    let corpus = Corpus::new();
    let result = measure_file(
        &corpus.0,
        "obj-grid-2.obj",
        Options {
            stage: Stage::Pipeline,
            fields: Fields::PositionNormalUv0,
            ..Options::default()
        },
    )
    .unwrap();
    let output = result.output.unwrap();
    assert_eq!(output.draws.len(), 1);
    let draw = &output.draws[0];
    let expected: Vec<[f64; 3]> = (0..2)
        .flat_map(|y| {
            (0..2).flat_map(move |x| {
                let (x, y) = (f64::from(x), f64::from(y));
                [
                    [x, y, 0.0],
                    [x + 1.0, y, 0.0],
                    [x + 1.0, y + 1.0, 0.0],
                    [x, y, 0.0],
                    [x + 1.0, y + 1.0, 0.0],
                    [x, y + 1.0, 0.0],
                ]
            })
        })
        .collect();
    assert_eq!(draw.positions, expected);
    for (index, position) in expected.iter().enumerate() {
        assert_eq!(draw.normals.as_ref().unwrap()[index], [0.0, 0.0, 1.0]);
        assert_eq!(
            draw.uv0.as_ref().unwrap()[index],
            [position[0] / 2.0, position[1] / 2.0]
        );
    }
    assert_eq!(output.placements.len(), 1);
    assert_eq!(output.placements[0].draw, 0);
    assert_eq!(
        output.placements[0].world,
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0]
        ]
    );
}

#[test]
fn wide_detached_tuple_bits_include_neighbors_and_negative_zero() {
    use baozi_asset_consumer::comparison::{Options, Stage, measure_file};
    let corpus = Corpus::new();
    let result = measure_file(
        &corpus.0,
        "ply-f64-neighbors.ply",
        Options {
            stage: Stage::Pipeline,
            ..Options::default()
        },
    )
    .unwrap();
    let positions = &result.output.as_ref().unwrap().draws[0].positions;
    assert_eq!(
        positions[0].map(f64::to_bits),
        [1.0, -0.0, 0.0].map(f64::to_bits)
    );
    assert_eq!(positions[1][0].to_bits(), (1.0 + 2_f64.powi(-30)).to_bits());
    assert_eq!(positions[2], [1.0, 1.0, 0.0]);
}

#[test]
fn rejection_keeps_the_actual_failing_phase_instead_of_requested_endpoint() {
    let corpus = Corpus::new();
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-asset-consumer"))
        .arg(&corpus.0)
        .args([
            "obj-repeated-1.obj",
            "--comparison",
            "pipeline",
            "--max-work-units",
            "0",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["stage"], "pipeline");
    assert_eq!(result["failure_stage"], "import");
    assert_eq!(result["status"], "policy_rejected");
    assert!(result["output"].is_null());
}

#[test]
fn shared_output_keeps_every_uv_tuple_matrix_and_draw_reference() {
    use baozi_asset_consumer::comparison::{Fields, Options, Stage, measure_file};
    let corpus = Corpus::new();
    let result = measure_file(
        &corpus.0,
        "gltf-shared-8.glb",
        Options {
            stage: Stage::Pipeline,
            fields: Fields::PositionNormalUv0,
            ..Options::default()
        },
    )
    .unwrap();
    let output = result.output.unwrap();
    for draw in &output.draws {
        assert_eq!(
            draw.positions,
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        );
        assert_eq!(
            draw.uv0.as_ref().unwrap(),
            &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]
        );
        assert_eq!(draw.normals.as_ref().unwrap(), &[[0.0, 0.0, 1.0]; 3]);
    }
    assert_eq!(output.placements.len(), 16);
    for (index, placement) in output.placements.iter().enumerate() {
        assert_eq!(placement.draw, index % 8);
        let x = if index < 8 { 0.0 } else { 2.0 };
        assert_eq!(
            placement.world,
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [x, 0.0, 0.0, 1.0]
            ]
        );
    }
}

#[test]
fn invalid_cli_policy_still_returns_one_failure_object() {
    let output = Command::new(env!("CARGO_BIN_EXE_baozi-asset-consumer"))
        .args([
            ".",
            "unused.obj",
            "--comparison",
            "extract",
            "--io-mode",
            "invalid",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema"], "baozi-assimp-observation/v1");
    assert_eq!(value["stage"], "extract");
    assert_eq!(value["failure_stage"], "setup");
    assert!(value["observation"].is_null());
}

#[test]
fn stl_ply_and_points_have_full_analytic_detached_payloads() {
    use baozi_asset_consumer::comparison::{Options, Stage, measure_file};
    let corpus = Corpus::new();
    for name in ["stl-repeated-1.stl", "ply-triangle-1.ply"] {
        let result = measure_file(
            &corpus.0,
            name,
            Options {
                stage: Stage::Pipeline,
                ..Options::default()
            },
        )
        .unwrap();
        let output = result.output.unwrap();
        assert_eq!(output.draws.len(), 1);
        assert_eq!(
            output.draws[0].positions,
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        );
        assert!(output.draws[0].normals.is_none());
        assert!(output.draws[0].uv0.is_none());
        assert_eq!(output.placements.len(), 1);
        assert_eq!(output.placements[0].draw, 0);
    }
    let result = measure_file(
        &corpus.0,
        "ply-points-1024.ply",
        Options {
            stage: Stage::Pipeline,
            ..Options::default()
        },
    )
    .unwrap();
    let output = result.output.unwrap();
    assert_eq!(output.draws.len(), 1);
    assert_eq!(output.draws[0].topology, baozi::TopologyKind::Points);
    let expected: Vec<_> = (0..1024).map(|x| [f64::from(x), 0.0, 0.0]).collect();
    assert_eq!(output.draws[0].positions, expected);
}
