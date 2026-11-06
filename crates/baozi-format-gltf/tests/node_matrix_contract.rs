mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::authoring::{Mat4, Quat, Vec3};
use baozi_import::{
    ImportDimension, ImportLedgerSnapshot, ImportLossKind, ImportTerminalCause, LossPolicy,
};
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, expected_error, glb_with_json, import_assets,
    import_assets_result, sidecar_options,
};

const IDENTITY: &str = "[1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1]";

fn encodings(json: &[u8]) -> [(&'static str, Vec<u8>); 2] {
    [
        ("node-matrix.gltf", json.to_vec()),
        ("node-matrix.glb", glb_with_json(json)),
    ]
}

fn node_document(properties: &str, selected: bool, needs_buffer: bool) -> Vec<u8> {
    let roots = if selected { "0,1" } else { "0" };
    let mesh = if needs_buffer { r#", "mesh": 0"# } else { "" };
    let resources = if needs_buffer {
        r#",
  "buffers": [{"uri": "missing.bin", "byteLength": 12}],
  "bufferViews": [{"buffer": 0, "byteLength": 12}],
  "accessors": [{"bufferView": 0, "componentType": 5126, "count": 1,
                 "type": "VEC3", "min": [0,0,0], "max": [0,0,0]}],
  "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "mode": 0}]}]"#
    } else {
        ""
    };
    format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scene": 0,
  "scenes": [{{"nodes": [{roots}]}}],
  "nodes": [{{"name": "SelectedNode"{mesh}}}, {{"name": "TransformNode", {properties}}}]
  {resources}
}}"#
    )
    .into_bytes()
}

fn assert_released_without_dependencies(ledger: &ImportLedgerSnapshot) {
    assert_eq!(ledger.current_live_bytes(), 0);
    assert!(ledger.current_administrative_bytes() <= ledger.peak_administrative_bytes());
    assert_eq!(ledger.active_reservations(), 0);
    for dimension in [
        ImportDimension::DependencyReference,
        ImportDimension::DependencyBytes,
    ] {
        let counts = ledger.dimension(dimension).unwrap();
        assert_eq!(counts.attempted(), 0);
        assert_eq!(counts.consumed(), 0);
    }
    assert_eq!(
        ledger
            .dimension(ImportDimension::AssetOpen)
            .unwrap()
            .consumed(),
        1
    );
}

fn assert_invalid_transform(properties: &str, reason: Option<&str>) -> TestResult<()> {
    for selected in [true, false] {
        for (source, bytes) in encodings(&node_document(properties, selected, true)) {
            let (result, diagnostics) =
                import_assets_result(source, [(source, bytes)], sidecar_options())?;
            let failure = expected_error(result)?;
            assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, reason)?;
            assert!(diagnostics.is_empty());
            assert!(failure.source_scope_record().is_none());
            assert!(failure.scene_validation_error().is_none());
            assert_eq!(failure.ledger().committed_canonical_members(), 0);
            assert!(matches!(
                failure.ledger().terminal_cause(),
                ImportTerminalCause::Failure(_)
            ));
            assert_released_without_dependencies(failure.ledger());
        }
    }
    Ok(())
}

fn assert_matrix_bits(actual: Mat4, expected: [[f32; 4]; 4]) {
    for (column, expected) in actual.cols.iter().zip(expected) {
        for (actual, expected) in column.iter().zip(expected) {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
    }
}

fn assert_accepted_matrix(
    literal: &str,
    expected: [[f32; 4]; 4],
    expected_point: Option<[f64; 3]>,
) -> TestResult<()> {
    let properties = format!(r#""matrix": {literal}"#);
    for selected in [true, false] {
        for (source, bytes) in encodings(&node_document(&properties, selected, false)) {
            let (report, diagnostics) =
                import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
            assert!(diagnostics.is_empty());
            assert!(report.loss_evidence().is_empty());
            assert_released_without_dependencies(report.ledger());
            let scene = report.scene();
            let node = scene
                .nodes()
                .find(|node| node.name() == Some("TransformNode"));
            if selected {
                let transform = node.expect("selected transform node").authored_transform();
                assert!(transform.authored_trs().is_none());
                let matrix = transform.authored_matrix().expect("preserved matrix form");
                assert_matrix_bits(matrix, expected);
                assert_matrix_bits(transform.local_matrix(), expected);
                if let Some(expected_point) = expected_point {
                    // Analytic witnesses use p = (1, 2, 3), independently of admission.
                    let point: [f64; 3] = std::array::from_fn(|row| {
                        f64::from(matrix.cols[0][row])
                            + 2.0 * f64::from(matrix.cols[1][row])
                            + 3.0 * f64::from(matrix.cols[2][row])
                            + f64::from(matrix.cols[3][row])
                    });
                    assert_eq!(point, expected_point);
                }
            } else {
                assert!(node.is_none());
            }
        }
    }
    Ok(())
}

fn diagonal_literal(x: &str, y: &str, z: &str) -> String {
    format!("[{x},0,0,0, 0,{y},0,0, 0,0,{z},0, 5,-7,11,1]")
}

#[test]
fn zero_spatial_columns_fail_globally_before_dependency_io() -> TestResult<()> {
    for (x, y, z) in [
        ("0", "2", "3"),
        ("1", "0", "3"),
        ("1", "2", "0"),
        ("0", "0", "3"),
        ("0", "2", "0"),
        ("1", "0", "0"),
        ("0", "0", "0"),
        ("-0.0", "2", "3"),
        ("1", "-0.0", "3"),
        ("1", "2", "-0.0"),
        ("-0.0", "-0.0", "3"),
        ("-0.0", "-0.0", "-0.0"),
    ] {
        assert_invalid_transform(
            &format!(r#""matrix": {}"#, diagonal_literal(x, y, z)),
            Some("node matrix"),
        )?;
    }
    Ok(())
}

#[test]
fn shear_and_dependent_nonzero_columns_fail_globally_before_io() -> TestResult<()> {
    for literal in [
        "[1,0,0,0, 0.25,1,0,0, 0,0,1,0, 5,-7,11,1]",
        "[1,0,0,0, 2,0,0,0, 0,0,1,0, 5,-7,11,1]",
        "[1,0,0,0, 0,1,0,0, -2,0,0,0, 5,-7,11,1]",
        "[1,0,0,0, 0,1,0,0, 0,2,0,0, 5,-7,11,1]",
        "[1e-30,0,0,0, 2e-30,0,0,0, 0,0,1e-30,0, 5,-7,11,1]",
        "[1.401298464324817e-45,0,0,0, 1.401298464324817e-45,0,0,0, \
         0,0,1.401298464324817e-45,0, 5,-7,11,1]",
    ] {
        assert_invalid_transform(&format!(r#""matrix": {literal}"#), Some("node matrix"))?;
    }
    Ok(())
}

#[test]
fn affine_bottom_row_is_exact_even_at_the_smallest_subnormal() -> TestResult<()> {
    for (column, literal) in [
        (0, "1.401298464324817e-45"),
        (1, "-1.401298464324817e-45"),
        (2, "1.401298464324817e-45"),
        (3, "1.00000011920928955078125"),
        (3, "0.999999940395355224609375"),
    ] {
        let mut cells = ["0"; 16];
        for index in [0, 5, 10, 15] {
            cells[index] = "1";
        }
        cells[column * 4 + 3] = literal;
        assert_invalid_transform(
            &format!(r#""matrix": [{}]"#, cells.join(",")),
            Some("node matrix"),
        )?;
    }
    Ok(())
}

#[test]
fn matrix_components_must_remain_finite_after_f32_conversion() -> TestResult<()> {
    for index in 0..16 {
        for literal in ["3.4028236e38", "-3.4028236e38"] {
            let mut cells = ["0"; 16];
            for diagonal in [0, 5, 10, 15] {
                cells[diagonal] = "1";
            }
            cells[index] = literal;
            assert_invalid_transform(&format!(r#""matrix": [{}]"#, cells.join(",")), None)?;
        }
    }
    Ok(())
}

#[test]
fn malformed_authored_matrices_never_default_or_reach_dependency_io() -> TestResult<()> {
    for literal in [
        "null",
        "false",
        "{}",
        r#""identity""#,
        "[]",
        "[1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,1]",
        "[1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1,0]",
        "[1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,null,1]",
    ] {
        assert_invalid_transform(&format!(r#""matrix": {literal}"#), None)?;
    }
    Ok(())
}

#[test]
fn authored_identity_trs_alongside_a_matrix_fails_even_on_unused_nodes() -> TestResult<()> {
    for trs in [
        r#""translation": [0,0,0]"#,
        r#""rotation": [0,0,0,1]"#,
        r#""scale": [1,1,1]"#,
        r#""translation": [-0.0,0,0], "rotation": [0,0,0,1], "scale": [1,1,1]"#,
    ] {
        assert_invalid_transform(&format!(r#""matrix": {IDENTITY}, {trs}"#), None)?;
    }
    Ok(())
}

#[test]
fn matrix_reflections_and_rotation_preserve_bits_and_analytic_points() -> TestResult<()> {
    assert_accepted_matrix(IDENTITY, Mat4::IDENTITY.cols, Some([1.0, 2.0, 3.0]))?;
    for (literal, columns, point) in [
        (
            "[-2,-0.0,0,-0.0, 0,3,-0.0,0, -0.0,0,4,-0.0, 5,-7,11,1]",
            [
                [-2.0, -0.0, 0.0, -0.0],
                [0.0, 3.0, -0.0, 0.0],
                [-0.0, 0.0, 4.0, -0.0],
                [5.0, -7.0, 11.0, 1.0],
            ],
            [3.0, -1.0, 23.0],
        ),
        (
            "[-2,0,0,0, 0,-3,0,0, 0,0,4,0, 5,-7,11,1]",
            [
                [-2.0, 0.0, 0.0, 0.0],
                [0.0, -3.0, 0.0, 0.0],
                [0.0, 0.0, 4.0, 0.0],
                [5.0, -7.0, 11.0, 1.0],
            ],
            [3.0, -13.0, 23.0],
        ),
        (
            "[-0.0,2,0,-0.0, -3,0,-0.0,0, 0,-0.0,4,-0.0, 5,-7,11,1]",
            [
                [-0.0, 2.0, 0.0, -0.0],
                [-3.0, 0.0, -0.0, 0.0],
                [0.0, -0.0, 4.0, -0.0],
                [5.0, -7.0, 11.0, 1.0],
            ],
            [-1.0, -5.0, 23.0],
        ),
    ] {
        assert_accepted_matrix(literal, columns, Some(point))?;
    }
    Ok(())
}

#[test]
fn nonzero_subnormal_and_extreme_finite_columns_are_not_zero_scales() -> TestResult<()> {
    for (x, y, z, expected) in [
        (
            "1.401298464324817e-45",
            "2",
            "3",
            [f32::from_bits(1), 2.0, 3.0],
        ),
        (
            "-1.401298464324817e-45",
            "1.401298464324817e-45",
            "1.401298464324817e-45",
            [
                f32::from_bits(0x8000_0001),
                f32::from_bits(1),
                f32::from_bits(1),
            ],
        ),
        (
            "3.4028234663852886e38",
            "-3.4028234663852886e38",
            "3.4028234663852886e38",
            [f32::MAX, -f32::MAX, f32::MAX],
        ),
        (
            "3.4028234663852886e38",
            "1.401298464324817e-45",
            "-2",
            [f32::MAX, f32::from_bits(1), -2.0],
        ),
    ] {
        assert_accepted_matrix(
            &diagonal_literal(x, y, z),
            [
                [expected[0], 0.0, 0.0, 0.0],
                [0.0, expected[1], 0.0, 0.0],
                [0.0, 0.0, expected[2], 0.0],
                [5.0, -7.0, 11.0, 1.0],
            ],
            None,
        )?;
    }
    Ok(())
}

#[test]
fn orthogonality_tau_and_adjacent_f32_vectors_use_independent_expectations() -> TestResult<()> {
    // For ci = ei and cj = s*ei + ej, compare s^2 with 2^-34 * (1 + s^2).
    for (literal, bits, accepted) in [
        ("0.000003814697265625", 0x3680_0000, true),
        (
            "0.00000762939407650264911353588104248046875",
            0x36ff_ffff,
            true,
        ),
        ("0.00000762939453125", 0x3700_0000, true),
        (
            "0.0000076293954407447017729282379150390625",
            0x3700_0001,
            false,
        ),
        ("0.0000152587890625", 0x3780_0000, false),
    ] {
        for (left, right) in [(0, 1), (0, 2), (1, 2)] {
            for negative in [false, true] {
                let value = if negative {
                    format!("-{literal}")
                } else {
                    literal.to_owned()
                };
                let mut cells = ["0"; 16];
                for diagonal in [0, 5, 10, 15] {
                    cells[diagonal] = "1";
                }
                cells[right * 4 + left] = &value;
                let matrix = format!("[{}]", cells.join(","));
                if accepted {
                    let mut expected = Mat4::IDENTITY.cols;
                    expected[right][left] =
                        f32::from_bits(if negative { bits | 0x8000_0000 } else { bits });
                    assert_accepted_matrix(&matrix, expected, None)?;
                } else {
                    assert_invalid_transform(
                        &format!(r#""matrix": {matrix}"#),
                        Some("node matrix"),
                    )?;
                }
            }
        }
    }
    Ok(())
}

#[test]
fn explicit_zero_trs_scales_keep_their_authored_form_and_component_bits() -> TestResult<()> {
    for (literal, scale) in [
        ("[0,2,3]", [0.0_f32, 2.0, 3.0]),
        ("[-0.0,0,3]", [-0.0, 0.0, 3.0]),
        ("[-0.0,0,-0.0]", [-0.0, 0.0, -0.0]),
    ] {
        let properties = format!(r#""scale": {literal}"#);
        for selected in [true, false] {
            for (source, bytes) in encodings(&node_document(&properties, selected, false)) {
                let (report, diagnostics) =
                    import_assets(source, [(source, bytes)], GltfTestOptions::memory())?;
                assert!(diagnostics.is_empty());
                assert!(report.loss_evidence().is_empty());
                assert_released_without_dependencies(report.ledger());
                let scene = report.scene();
                let node = scene
                    .nodes()
                    .find(|node| node.name() == Some("TransformNode"));
                if selected {
                    let transform = node.expect("selected TRS node").authored_transform();
                    assert!(transform.authored_matrix().is_none());
                    let trs = transform.authored_trs().expect("preserved TRS form");
                    assert_eq!(trs.translation(), Vec3::ZERO);
                    assert_eq!(trs.rotation(), Quat::IDENTITY);
                    let actual = trs.scale();
                    assert_eq!(
                        [actual.x, actual.y, actual.z].map(f32::to_bits),
                        scale.map(f32::to_bits)
                    );
                    let derived = transform.local_matrix();
                    for (axis, expected) in scale.into_iter().enumerate() {
                        assert_eq!(derived.cols[axis][axis].to_bits(), expected.to_bits());
                    }
                    assert_eq!(derived.cols[3], [0.0, 0.0, 0.0, 1.0]);
                } else {
                    assert!(node.is_none());
                }
            }
        }
    }
    Ok(())
}

#[test]
fn matrix_authored_trs_targets_fail_globally_in_omitted_and_mixed_clips() -> TestResult<()> {
    for path in ["translation", "rotation", "scale"] {
        for mixed in [false, true] {
            for allow_loss in [false, true] {
                let mut document: serde_json::Value = serde_json::from_slice(&node_document(
                    &format!(r#""matrix": {IDENTITY}"#),
                    false,
                    true,
                ))?;
                // These buffer-free animation accessors are valid and need no runtime IO.
                document["accessors"].as_array_mut().unwrap().extend([
                    serde_json::json!({"componentType":5126,"count":1,"type":"SCALAR",
                                       "min":[0],"max":[0]}),
                    serde_json::json!({"componentType":5126,"count":1,
                                       "type":if path == "rotation" {"VEC4"} else {"VEC3"}}),
                ]);
                let mut channels = vec![serde_json::json!({
                    "sampler":0,"target":{"node":1,"path":path}
                })];
                if mixed {
                    channels.push(serde_json::json!({
                        "sampler":0,"target":{"node":0,"path":path}
                    }));
                }
                document["animations"] = serde_json::json!([{
                    "samplers":[{"input":1,"output":2}],"channels":channels
                }]);
                for (source, bytes) in encodings(&serde_json::to_vec(&document)?) {
                    let mut options = sidecar_options();
                    if allow_loss {
                        options.loss_policy = LossPolicy::preserve_or_fail()
                            .allow(ImportLossKind::SourceSemanticLoss);
                    }
                    let (result, diagnostics) =
                        import_assets_result(source, [(source, bytes)], options)?;
                    let failure = expected_error(result)?;
                    assert!(diagnostics.is_empty());
                    assert_gltf_failure(
                        &failure,
                        GltfFailureCode::InvalidDocument,
                        Some("TRS animation targets a matrix-authored node"),
                    )?;
                    assert!(failure.source_scope_record().is_none());
                    assert_released_without_dependencies(failure.ledger());
                    assert_eq!(failure.ledger().committed_canonical_members(), 0);
                    assert_eq!(
                        failure
                            .ledger()
                            .dimension(ImportDimension::AnimationCells)
                            .unwrap()
                            .attempted(),
                        0
                    );
                }
            }
        }
    }
    Ok(())
}
