mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    ImportDimension, ImportTerminalCause, ImportTerminalFailureClass, SceneValidationError,
};
use common::{
    TestResult, assert_gltf_failure, expected_error, import_assets, import_assets_result,
    sidecar_options, skinned_triangle_bin, skinned_triangle_bin_with_joint_index,
    skinned_triangle_gltf, skinned_triangle_without_inverse_bind_matrices_gltf,
};

fn replace_fixture_fragment(source: String, from: &str, to: &str) -> String {
    assert!(source.contains(from), "fixture fragment not found: {from}");
    source.replacen(from, to, 1)
}

fn skinned_triangle_with_unselected_joint() -> String {
    replace_fixture_fragment(
        String::from_utf8(skinned_triangle_gltf()).expect("fixture is valid utf-8"),
        r#"{ "name": "Joint1" }"#,
        r#"{ "name": "Joint1" },
    { "name": "UnselectedJoint" }"#,
    )
}

#[test]
fn imports_skinned_triangle_with_node_level_binding() -> TestResult<()> {
    let (report, diagnostics) = import_assets(
        "models/skin.gltf",
        [
            ("models/skin.gltf", skinned_triangle_gltf()),
            ("models/skin.bin", skinned_triangle_bin()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    Ok(())
}

#[test]
fn imports_only_the_selected_scenes_referenced_skin() -> TestResult<()> {
    let gltf = replace_fixture_fragment(
        skinned_triangle_with_unselected_joint(),
        r#""scene": 0,
  "scenes": [{ "nodes": [0] }],"#,
        r#""scene": 1,
  "scenes": [{ "nodes": [4] }, { "nodes": [0] }],"#,
    );
    let gltf = replace_fixture_fragment(
        gltf,
        r#"{ "name": "MeshNode", "mesh": 0, "skin": 0 }"#,
        r#"{ "name": "MeshNode", "mesh": 0, "skin": 1 }"#,
    );
    let gltf = replace_fixture_fragment(
        gltf,
        r#""skins": [{ "name": "Skin", "joints": [1, 3], "skeleton": 1, "inverseBindMatrices": 4 }]"#,
        r#""skins": [
    { "name": "UnusedSkin", "joints": [4], "skeleton": 4 },
    { "name": "Skin", "joints": [1, 3], "skeleton": 1, "inverseBindMatrices": 4 }
  ]"#,
    )
    .into_bytes();
    let (report, diagnostics) = import_assets(
        "models/skin.gltf",
        [
            ("models/skin.gltf", gltf),
            ("models/skin.bin", skinned_triangle_bin()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    Ok(())
}

#[test]
fn selected_skin_rejects_a_joint_outside_the_imported_scene() -> TestResult<()> {
    let gltf = replace_fixture_fragment(
        skinned_triangle_with_unselected_joint(),
        r#""joints": [1, 3]"#,
        r#""joints": [1, 4]"#,
    )
    .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/skin.gltf",
        [("models/skin.gltf", gltf)],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("joint node 4 outside the imported scene"),
    )?;
    assert!(error.source_scope_record().is_none());
    assert_eq!(
        error
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .expect("dependency references are configured")
            .attempted(),
        0
    );
    Ok(())
}

#[test]
fn selected_skin_rejects_a_skeleton_outside_the_imported_scene() -> TestResult<()> {
    let gltf = replace_fixture_fragment(
        skinned_triangle_with_unselected_joint(),
        r#""skeleton": 1"#,
        r#""skeleton": 4"#,
    )
    .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/skin.gltf",
        [("models/skin.gltf", gltf)],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("skeleton root node 4 outside the imported scene"),
    )?;
    assert!(error.source_scope_record().is_none());
    assert_eq!(
        error
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .expect("dependency references are configured")
            .attempted(),
        0
    );
    Ok(())
}

#[test]
fn selected_skin_without_mesh_is_rejected_before_dependency_io() -> TestResult<()> {
    let gltf = String::from_utf8(skinned_triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "name": "MeshNode", "mesh": 0, "skin": 0 }"#,
            r#"{ "name": "MeshNode", "skin": 0 }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/skin.gltf",
        [("models/skin.gltf", gltf)],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("declares a skin without a mesh"),
    )?;
    assert!(error.source_scope_record().is_none());
    assert_eq!(
        error
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .expect("dependency references are configured")
            .attempted(),
        0
    );
    Ok(())
}

#[test]
fn multiple_influence_sets_are_rejected_before_dependency_io() -> TestResult<()> {
    let gltf = String::from_utf8(skinned_triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#""JOINTS_0": 2, "WEIGHTS_0": 3"#,
            r#""JOINTS_0": 2, "WEIGHTS_0": 3, "JOINTS_1": 2"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/skin.gltf",
        [("models/skin.gltf", gltf)],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::UnsupportedFeature,
        Some("multiple JOINTS/WEIGHTS sets"),
    )?;
    assert!(error.source_scope_record().is_none());
    assert_eq!(
        error
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .expect("dependency references are configured")
            .attempted(),
        0
    );
    Ok(())
}

#[test]
fn skin_without_inverse_bind_matrices_uses_identity_defaults() -> TestResult<()> {
    let (report, diagnostics) = import_assets(
        "models/skin.gltf",
        [
            (
                "models/skin.gltf",
                skinned_triangle_without_inverse_bind_matrices_gltf(),
            ),
            ("models/skin.bin", skinned_triangle_bin()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    Ok(())
}

#[test]
fn joint_index_outside_skin_palette_is_invalid_scene() -> TestResult<()> {
    let (result, diagnostics) = import_assets_result(
        "models/skin.gltf",
        [
            ("models/skin.gltf", skinned_triangle_gltf()),
            ("models/skin.bin", skinned_triangle_bin_with_joint_index(2)),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_eq!(
        error.scene_validation_error(),
        Some(&SceneValidationError::PaletteJointOutOfRange {
            instance: 0,
            logical_mesh: 0,
            skin: 0,
            joint_count: 2,
            primitive: 0,
            influence: 0,
            palette_joint: 2,
        })
    );
    let ImportTerminalCause::Failure(terminal) = error.ledger().terminal_cause() else {
        panic!("semantic rejection must retain failure evidence");
    };
    assert_eq!(terminal.class(), ImportTerminalFailureClass::Requested);
    assert_eq!(terminal.code(), 0x5343_1104);
    Ok(())
}

#[test]
fn inverse_bind_matrix_count_mismatch_is_parse_error_before_reader() -> TestResult<()> {
    let gltf = String::from_utf8(skinned_triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4" }"#,
            r#"{ "bufferView": 4, "componentType": 5126, "count": 1, "type": "MAT4" }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/skin.gltf",
        [
            ("models/skin.gltf", gltf),
            ("models/skin.bin", skinned_triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("inverse bind matrix count"),
    )?;
    Ok(())
}

#[test]
fn inverse_bind_matrix_type_mismatch_is_parse_error_before_reader() -> TestResult<()> {
    let gltf = String::from_utf8(skinned_triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4" }"#,
            r#"{ "bufferView": 4, "componentType": 5126, "count": 2, "type": "VEC4" }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/skin.gltf",
        [
            ("models/skin.gltf", gltf),
            ("models/skin.bin", skinned_triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("inverseBindMatrices accessor"),
    )?;
    Ok(())
}

#[test]
fn inverse_bind_matrices_reject_projective_rows_and_nonfinite_values() -> TestResult<()> {
    // The MAT4 accessor starts at byte 116; glTF stores its columns consecutively.
    for (column, row, value) in [
        (0, 3, 0.125_f32),
        (1, 3, -0.25),
        (2, 3, 0.5),
        (3, 3, 2.0),
        (1, 0, f32::NAN),
    ] {
        let mut bytes = skinned_triangle_bin();
        let offset = 116 + (column * 4 + row) * 4;
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        let (result, diagnostics) = import_assets_result(
            "models/skin.gltf",
            [
                ("models/skin.gltf", skinned_triangle_gltf()),
                ("models/skin.bin", bytes),
            ],
            sidecar_options(),
        )?;
        let error = expected_error(result)?;
        assert!(diagnostics.is_empty());
        assert_gltf_failure(
            &error,
            GltfFailureCode::InvalidDocument,
            Some("inverse bind matrix 0 must be finite and affine"),
        )?;
    }
    Ok(())
}

#[test]
fn inverse_bind_matrices_preserve_affine_shear_reflection_and_signed_zero() -> TestResult<()> {
    let mut bytes = skinned_triangle_bin();
    for (column, row, value) in [(0, 0, -2.0_f32), (0, 1, 0.25), (1, 3, -0.0)] {
        let offset = 116 + (column * 4 + row) * 4;
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let (report, diagnostics) = import_assets(
        "models/skin.gltf",
        [
            ("models/skin.gltf", skinned_triangle_gltf()),
            ("models/skin.bin", bytes),
        ],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty());
    let matrix = report
        .scene()
        .skins()
        .next()
        .expect("authored skin")
        .inverse_bind_matrices()
        .next()
        .expect("first inverse bind");
    assert_eq!(matrix.cols[0][0], -2.0);
    assert_eq!(matrix.cols[0][1], 0.25);
    assert_eq!(matrix.cols[1][3].to_bits(), (-0.0_f32).to_bits());
    Ok(())
}

#[test]
fn joint_and_weight_attributes_must_be_paired() -> TestResult<()> {
    let gltf = String::from_utf8(skinned_triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(r#", "WEIGHTS_0": 3"#, "")
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/skin.gltf",
        [
            ("models/skin.gltf", gltf),
            ("models/skin.bin", skinned_triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("JOINTS and WEIGHTS"),
    )?;
    Ok(())
}
