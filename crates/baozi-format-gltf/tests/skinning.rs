mod common;

use baozi_core::{BaoziError, MeshBinding, NodeId, Result, SkinId};
use common::{
    expected_error, import_assets, import_assets_result, sidecar_options, skinned_triangle_bin,
    skinned_triangle_bin_with_joint_index, skinned_triangle_gltf,
    skinned_triangle_without_inverse_bind_matrices_gltf,
};

#[test]
fn imports_skinned_triangle_with_node_level_binding() -> Result<()> {
    let (scene, diagnostics) = import_assets(
        "models/skin.gltf",
        [
            ("models/skin.gltf", skinned_triangle_gltf()),
            ("models/skin.bin", skinned_triangle_bin()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    assert_eq!(scene.skins.len(), 1);
    assert_eq!(scene.skins[0].name.as_deref(), Some("Skin"));
    assert_eq!(scene.skins[0].joints, vec![NodeId::new(2), NodeId::new(3)]);
    assert_eq!(scene.skins[0].skeleton_root, Some(NodeId::new(2)));
    assert_eq!(scene.skins[0].inverse_bind_matrices.len(), 2);
    assert_eq!(
        scene.nodes[4].mesh_bindings,
        vec![MeshBinding::skinned(
            baozi_core::MeshId::new(0),
            SkinId::new(0)
        )]
    );
    assert_eq!(scene.meshes[0].joint_indices.len(), 3);
    assert_eq!(scene.meshes[0].joint_weights.len(), 3);
    assert_eq!(scene.meshes[0].joint_indices[1], [1, 0, 0, 0]);
    assert_eq!(scene.meshes[0].joint_weights[2], [0.5, 0.5, 0.0, 0.0]);
    Ok(())
}

#[test]
fn skin_without_inverse_bind_matrices_keeps_empty_list() -> Result<()> {
    let (scene, diagnostics) = import_assets(
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
    assert_eq!(scene.skins.len(), 1);
    assert!(scene.skins[0].inverse_bind_matrices.is_empty());
    Ok(())
}

#[test]
fn joint_index_outside_skin_palette_is_invalid_scene() -> Result<()> {
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
    assert!(matches!(error, BaoziError::InvalidScene { .. }));
    assert!(error.to_string().contains("joint index"));
    Ok(())
}

#[test]
fn inverse_bind_matrix_count_mismatch_is_parse_error_before_reader() -> Result<()> {
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
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("inverse bind matrix count"));
    Ok(())
}

#[test]
fn inverse_bind_matrix_type_mismatch_is_parse_error_before_reader() -> Result<()> {
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
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("inverseBindMatrices accessor"));
    Ok(())
}

#[test]
fn joint_and_weight_attributes_must_be_paired() -> Result<()> {
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
    assert!(matches!(error, BaoziError::Parse { .. }));
    assert!(error.to_string().contains("JOINTS and WEIGHTS"));
    Ok(())
}
