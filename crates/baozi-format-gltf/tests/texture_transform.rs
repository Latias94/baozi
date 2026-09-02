mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{ImportDimension, SceneSnapshot, SceneSnapshotLimits};
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, expected_error, import_assets,
    import_assets_result, sidecar_options, triangle_bin, triangle_gltf, triangle_png,
};

const KHR_TEXTURE_TRANSFORM: &str = "KHR_texture_transform";

fn replace_once(source: String, needle: &str, replacement: String) -> String {
    assert_eq!(
        source.matches(needle).count(),
        1,
        "fixture fragment must be unique"
    );
    source.replacen(needle, &replacement, 1)
}

fn texture_transform_fixture(payload: &str, declare_extension: bool) -> Vec<u8> {
    let mut document = String::from_utf8(triangle_gltf()).expect("fixture is valid utf-8");
    let asset = r#"  "asset": { "version": "2.0", "generator": "baozi-test" },"#;
    let asset_replacement = if declare_extension {
        format!(
            "{asset}\n  \"extensionsUsed\": [\"{KHR_TEXTURE_TRANSFORM}\"],\n  \"extensionsRequired\": [\"{KHR_TEXTURE_TRANSFORM}\"],"
        )
    } else {
        asset.to_owned()
    };
    document = replace_once(document, asset, asset_replacement);
    document = replace_once(
        document,
        r#""baseColorTexture": { "index": 0, "texCoord": 0 }"#,
        format!(
            r#""baseColorTexture": {{ "index": 0, "texCoord": 0, "extensions": {{ "{KHR_TEXTURE_TRANSFORM}": {payload} }} }},
      "metallicRoughnessTexture": {{ "index": 0, "texCoord": 0, "extensions": {{ "{KHR_TEXTURE_TRANSFORM}": {payload} }} }}"#
        ),
    );
    document = replace_once(
        document,
        r#"    "name": "Red","#,
        format!(
            r#"    "name": "Red",
    "normalTexture": {{ "index": 0, "texCoord": 0, "extensions": {{ "{KHR_TEXTURE_TRANSFORM}": {payload} }} }},
    "occlusionTexture": {{ "index": 0, "texCoord": 0, "extensions": {{ "{KHR_TEXTURE_TRANSFORM}": {payload} }} }},
    "emissiveTexture": {{ "index": 0, "texCoord": 0, "extensions": {{ "{KHR_TEXTURE_TRANSFORM}": {payload} }} }},"#
        ),
    );
    document = replace_once(
        document,
        r#""TEXCOORD_0": 2"#,
        r#""TEXCOORD_0": 2, "TEXCOORD_1": 2"#.to_owned(),
    );
    document.into_bytes()
}

fn import_texture_transform_snapshot(document: Vec<u8>) -> TestResult<SceneSnapshot> {
    let (report, diagnostics) = import_assets(
        "models/scene.gltf",
        [
            ("models/scene.gltf", document),
            ("models/triangle.bin", triangle_bin()),
            ("models/textures/base.png", triangle_png()),
        ],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty());
    Ok(report
        .scene()
        .capture_snapshot(SceneSnapshotLimits::try_new(64 * 1024, 128, 1_000_000)?)?)
}

#[test]
fn typed_texture_transforms_change_canonical_snapshot() -> TestResult<()> {
    let identity = texture_transform_fixture(r#"{ "texCoord": 1 }"#, true);
    let transformed = texture_transform_fixture(
        r#"{ "offset": [0.125, 0.25], "rotation": 0.375, "scale": [2.0, 0.5], "texCoord": 1 }"#,
        true,
    );
    let identity_snapshot = import_texture_transform_snapshot(identity)?;
    let transformed_snapshot = import_texture_transform_snapshot(transformed)?;

    assert_eq!(transformed_snapshot.material_count(), 1);
    assert_eq!(transformed_snapshot.material_binding_count(), 5);
    assert_eq!(
        transformed_snapshot
            .preview()
            .matches("coordinate_set=1")
            .count(),
        5
    );
    assert_ne!(identity_snapshot.digest(), transformed_snapshot.digest());
    Ok(())
}

#[test]
fn texture_transform_requires_global_declaration_before_dependency_io() -> TestResult<()> {
    let document = texture_transform_fixture(r#"{ "texCoord": 1 }"#, false);
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [("models/scene.gltf", document)],
        sidecar_options(),
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &failure,
        GltfFailureCode::InvalidDocument,
        Some("uses KHR_texture_transform"),
    )?;
    assert!(failure.source_scope_record().is_none());
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .expect("dependency references are configured")
            .attempted(),
        0
    );
    Ok(())
}

#[test]
fn texture_transform_rejects_out_of_range_coordinate_override_before_io() -> TestResult<()> {
    let document = texture_transform_fixture(r#"{ "texCoord": 65536 }"#, true);
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [("models/scene.gltf", document)],
        GltfTestOptions::memory(),
    )?;
    let failure = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &failure,
        GltfFailureCode::InvalidDocument,
        Some("texture coordinate set"),
    )?;
    assert!(failure.source_scope_record().is_none());
    assert_eq!(
        failure
            .ledger()
            .dimension(ImportDimension::DependencyReference)
            .expect("dependency references are configured")
            .attempted(),
        0
    );
    Ok(())
}
