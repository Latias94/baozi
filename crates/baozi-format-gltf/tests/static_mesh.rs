mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{ImportCause, ImportDimension, ScopeAbortCause};
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, assert_resource_limit, data_uri_gltf,
    expected_error, import_assets, import_assets_result, interleaved_triangle_bin,
    interleaved_triangle_gltf, sidecar_options, triangle_bin, triangle_glb, triangle_gltf,
    triangle_gltf_with_buffer_uri, triangle_png,
};

#[test]
fn imports_static_mesh_material_texture_uri_and_hierarchy() -> TestResult<()> {
    let (report, diagnostics) = import_assets(
        "models/scene.gltf",
        [
            ("models/scene.gltf", triangle_gltf()),
            ("models/triangle.bin", triangle_bin()),
            ("models/textures/base.png", triangle_png()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    Ok(())
}

#[test]
fn repeated_external_texture_uri_uses_one_canonical_path() -> TestResult<()> {
    let document = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#""name": "Red","#,
            r#""name": "Red",
    "emissiveTexture": { "index": 0 },"#,
        )
        .into_bytes();
    let (report, diagnostics) = import_assets(
        "models/deep/scene.gltf",
        [
            ("models/deep/scene.gltf", document),
            ("models/deep/triangle.bin", triangle_bin()),
            ("models/deep/textures/base.png", triangle_png()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    Ok(())
}

#[test]
fn repeated_resolved_texture_uri_uses_canonical_budget() -> TestResult<()> {
    let document = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#""name": "Red","#,
            r#""name": "Red",
    "emissiveTexture": { "index": 0 },"#,
        )
        .into_bytes();
    let mut options = sidecar_options();
    options.limits.max_canonical_string_bytes = 100;
    let (result, diagnostics) = import_assets_result(
        "models/deep/scene.gltf",
        [
            ("models/deep/scene.gltf", document),
            ("models/deep/triangle.bin", triangle_bin()),
        ],
        options,
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_canonical_string_bytes");
    Ok(())
}

#[test]
fn imports_glb_static_mesh_from_bin_chunk() -> TestResult<()> {
    let (report, diagnostics) = import_assets(
        "models/scene.glb",
        [("models/scene.glb", triangle_glb())],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    Ok(())
}

#[test]
fn imports_interleaved_byte_stride_mesh() -> TestResult<()> {
    let (report, diagnostics) = import_assets(
        "models/interleaved.gltf",
        [
            ("models/interleaved.gltf", interleaved_triangle_gltf()),
            ("models/interleaved.bin", interleaved_triangle_bin()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    Ok(())
}

#[test]
fn gltf_scene_snapshot_covers_imported_ir() -> TestResult<()> {
    let (report, diagnostics) = import_assets(
        "models/scene.gltf",
        [
            ("models/scene.gltf", triangle_gltf()),
            ("models/triangle.bin", triangle_bin()),
            ("models/textures/base.png", triangle_png()),
        ],
        sidecar_options(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    Ok(())
}

#[test]
fn external_buffer_denied_is_fatal() -> TestResult<()> {
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [("models/scene.gltf", triangle_gltf())],
        GltfTestOptions::memory(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error.cause(),
        ImportCause::Scope(abort)
            if matches!(abort.cause(), ScopeAbortCause::DependencyPolicy(_))
    ));
    Ok(())
}

#[test]
fn overlong_external_buffer_uri_hits_the_string_limit_before_policy_errors() -> TestResult<()> {
    let uri = "x".repeat(4_096);
    let document = triangle_gltf_with_buffer_uri(&uri, 104);
    let mut options = GltfTestOptions::memory();
    options.limits.max_string_bytes = 32;

    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [("models/scene.gltf", document)],
        options,
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_string_bytes");
    Ok(())
}

#[test]
fn mesh_name_limit_is_rejected_before_sidecar_io() -> TestResult<()> {
    const PRIMITIVE_COUNT: usize = 8;
    const NAME_BYTES: usize = 4_096;

    let primitive = r#"{
      "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
      "indices": 3,
      "material": 0,
      "mode": 4
    }"#;
    let primitives = (0..PRIMITIVE_COUNT)
        .map(|_| primitive)
        .collect::<Vec<_>>()
        .join(",");
    let long_name = "n".repeat(NAME_BYTES);
    let document = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#""name": "Triangle",
    "primitives""#,
            &format!(
                r#""name": "{long_name}",
    "primitives""#
            ),
        )
        .replace(primitive, &primitives)
        .into_bytes();
    let mut options = GltfTestOptions::memory();
    // The canonical complete-batch model stores the logical-mesh name once, regardless of
    // primitive count. Freeze the one-name budget and keep the rejection ahead of sidecar I/O.
    options.limits.max_canonical_string_bytes = NAME_BYTES - 1;

    let (result, diagnostics) = import_assets_result(
        "models/repeated-name.gltf",
        [("models/repeated-name.gltf", document)],
        options,
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_canonical_string_bytes");
    Ok(())
}

#[test]
fn generated_primitive_name_suffix_is_limited_before_sidecar_io() -> TestResult<()> {
    const PRIMITIVE_COUNT: usize = 11;

    let primitive = r#"{
      "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
      "indices": 3,
      "material": 0,
      "mode": 4
    }"#;
    let primitives = (0..PRIMITIVE_COUNT)
        .map(|_| primitive)
        .collect::<Vec<_>>()
        .join(",");
    let document = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#""name": "Triangle",
    "primitives""#,
            r#""name": "n",
    "primitives""#,
        )
        .replace(primitive, &primitives)
        .into_bytes();
    let mut options = GltfTestOptions::memory();
    options.limits.max_string_bytes = 3;

    let (result, diagnostics) = import_assets_result(
        "models/generated-name.gltf",
        [("models/generated-name.gltf", document)],
        options,
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_string_bytes");
    Ok(())
}

#[test]
fn ignored_overlong_texture_uri_does_not_consume_canonical_string_budget() -> TestResult<()> {
    let long_uri = "x".repeat(128);
    let document = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace("textures/base.png", &long_uri)
        .into_bytes();
    let mut options = sidecar_options();
    options.limits.max_string_bytes = 32;
    options.limits.max_canonical_string_bytes = 100;

    let (result, diagnostics) = import_assets_result(
        "models/ignored-texture.gltf",
        [
            ("models/ignored-texture.gltf", document),
            ("models/triangle.bin", triangle_bin()),
        ],
        options,
    )?;
    let report = result?;

    assert!(report.scene().canonical_block_count() > 0);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code().as_str(), "gltf.texture_uri_limit");
    Ok(())
}

#[test]
fn missing_external_buffer_is_fatal() -> TestResult<()> {
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [("models/scene.gltf", triangle_gltf())],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert!(matches!(
        error.cause(),
        ImportCause::Scope(abort) if matches!(abort.cause(), ScopeAbortCause::AssetOpen(_))
    ));
    Ok(())
}

#[test]
fn short_external_buffer_is_fatal() -> TestResult<()> {
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", triangle_gltf()),
            ("models/triangle.bin", vec![0; 10]),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("declares 104 bytes"),
    )?;
    Ok(())
}

#[test]
fn required_external_buffer_resource_limits_are_fatal() -> TestResult<()> {
    let gltf = triangle_gltf();
    let bin = triangle_bin();

    let mut sidecar_limit = sidecar_options();
    sidecar_limit.limits.max_dependency_asset_bytes = bin.len() as u64 - 1;

    let mut total_limit = sidecar_options();
    total_limit.limits.max_total_asset_bytes = (gltf.len() + bin.len() - 1) as u64;

    let mut open_limit = sidecar_options();
    open_limit.limits.max_open_assets = 1;

    for (options, expected_limit) in [
        (sidecar_limit, "max_dependency_asset_bytes"),
        (total_limit, "max_total_asset_bytes"),
        (open_limit, "max_open_assets"),
    ] {
        let (result, diagnostics) = import_assets_result(
            "models/scene.gltf",
            [
                ("models/scene.gltf", gltf.clone()),
                ("models/triangle.bin", bin.clone()),
            ],
            options,
        )?;
        let error = expected_error(result)?;

        assert!(diagnostics.is_empty());
        assert_resource_limit(&error, expected_limit);
    }
    Ok(())
}

#[test]
fn indexed_point_topology_cells_are_bounded_before_reader_allocation() -> TestResult<()> {
    let document = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace("\"mode\": 4", "\"mode\": 0")
        .into_bytes();
    let buffer = triangle_bin();

    let mut exact = sidecar_options();
    exact.limits.max_total_topology_cells = 3;
    let (result, diagnostics) = import_assets_result(
        "models/points.gltf",
        [
            ("models/points.gltf", document.clone()),
            ("models/triangle.bin", buffer.clone()),
            ("models/textures/base.png", triangle_png()),
        ],
        exact,
    )?;
    let report = result?;
    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);

    let mut excess = sidecar_options();
    excess.limits.max_total_topology_cells = 2;
    let (result, diagnostics) = import_assets_result(
        "models/points.gltf",
        [
            ("models/points.gltf", document),
            ("models/triangle.bin", buffer),
        ],
        excess,
    )?;
    let error = expected_error(result)?;
    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_total_topology_cells");
    Ok(())
}

#[test]
fn unsupported_primitive_mode_is_fatal() -> TestResult<()> {
    let gltf = String::from_utf8(triangle_gltf())?
        .replace("\"mode\": 4", "\"mode\": 5")
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::UnsupportedFeature,
        Some("TriangleStrip"),
    )?;
    Ok(())
}

#[test]
fn malformed_accessor_root_is_caught_at_backend_boundary() -> TestResult<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(r#""accessors""#, r#""acceOsors""#)
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(&error, GltfFailureCode::ParserPanicked, None)?;
    Ok(())
}

#[test]
fn invalid_index_accessor_type_is_parse_error_not_panic() -> TestResult<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" }"#,
            r#"{ "bufferView": 3, "componentType": 5126, "count": 3, "type": "SCALAR" }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("indices accessor type"),
    )?;
    Ok(())
}

#[test]
fn rejected_primitive_does_not_consume_its_preflighted_name() -> TestResult<()> {
    let document = br#"{
  "asset": { "version": "2.0" },
  "buffers": [{ "uri": "data:application/octet-stream;base64,AAAAAAAAAAAAAAAA", "byteLength": 12 }],
  "bufferViews": [{ "buffer": 0, "byteOffset": 0, "byteLength": 12 }],
  "accessors": [{ "bufferView": 0, "componentType": 5123, "count": 1, "type": "VEC3" }],
  "meshes": [{
    "name": "NeverBuilt",
    "primitives": [{ "attributes": { "POSITION": 0 } }]
  }]
}"#
    .to_vec();
    let (result, diagnostics) = import_assets_result(
        "rejected-name.gltf",
        [("rejected-name.gltf", document)],
        GltfTestOptions::memory(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(&error, GltfFailureCode::InvalidDocument, None)?;
    let strings = error
        .ledger()
        .dimension(ImportDimension::CanonicalStringBytes)
        .expect("canonical string dimension is configured");
    assert_eq!(strings.attempted(), 0);
    assert_eq!(strings.consumed(), 0);
    Ok(())
}

#[test]
fn mismatched_attribute_count_is_parse_error_before_reader() -> TestResult<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" }"#,
            r#"{ "bufferView": 1, "componentType": 5126, "count": 2, "type": "VEC3" }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("NORMAL accessor count"),
    )?;
    Ok(())
}

#[test]
fn integer_texcoord_must_be_normalized_before_reader() -> TestResult<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2" }"#,
            r#"{ "bufferView": 2, "componentType": 5123, "count": 3, "type": "VEC2" }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("integer TEXCOORD accessor"),
    )?;
    Ok(())
}

#[test]
fn short_buffer_view_is_parse_error_before_reader() -> TestResult<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962 }"#,
            r#"{ "buffer": 0, "byteOffset": 0, "byteLength": 24, "target": 34962 }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("byteLength does not cover"),
    )?;
    Ok(())
}

#[test]
fn index_accessor_byte_stride_is_parse_error_before_reader() -> TestResult<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 6, "target": 34963 }"#,
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 6, "byteStride": 4, "target": 34963 }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(&error, GltfFailureCode::InvalidDocument, Some("byteStride"))?;
    Ok(())
}

#[test]
fn sparse_accessor_is_explicitly_unsupported_before_reader() -> TestResult<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(r#""byteLength": 104"#, r#""byteLength": 116"#)
        .replace(
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 6, "target": 34963 }"#,
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 6, "target": 34963 },
    { "buffer": 0, "byteOffset": 102, "byteLength": 2 },
    { "buffer": 0, "byteOffset": 104, "byteLength": 12 }"#,
        )
        .replace(
            r#"{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] }"#,
            r#"{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0], "sparse": { "count": 1, "indices": { "bufferView": 4, "componentType": 5123 }, "values": { "bufferView": 5 } } }"#,
        )
        .into_bytes();
    let mut bin = triangle_bin();
    bin.resize(116, 0);
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [("models/scene.gltf", gltf), ("models/triangle.bin", bin)],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::UnsupportedFeature,
        Some("sparse accessor"),
    )?;
    Ok(())
}

#[test]
fn float_accessor_must_not_be_normalized_before_reader() -> TestResult<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(
            r#"{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] }"#,
            r#"{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "normalized": true, "min": [0, 0, 0], "max": [1, 1, 0] }"#,
        )
        .into_bytes();
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", triangle_bin()),
        ],
        sidecar_options(),
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("float POSITION accessor"),
    )?;
    Ok(())
}

#[test]
fn imports_base64_buffer_data_uri() -> TestResult<()> {
    let (report, diagnostics) = import_assets(
        "scene.gltf",
        [("scene.gltf", data_uri_gltf())],
        GltfTestOptions::memory(),
    )?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    Ok(())
}

#[test]
fn opaque_backend_allocation_respects_live_memory_limit() -> TestResult<()> {
    let mut options = sidecar_options();
    options.limits.max_live_bytes = 128 * 1024;
    let (result, diagnostics) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", triangle_gltf()),
            ("models/triangle.bin", triangle_bin()),
        ],
        options,
    )?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_live_bytes");
    Ok(())
}

#[test]
fn data_uri_decode_work_is_budgeted() -> TestResult<()> {
    let document = data_uri_gltf();
    let mut options = GltfTestOptions::memory();
    options.limits.max_decode_work = (document.len() * 2) as u64;
    let (result, diagnostics) =
        import_assets_result("scene.gltf", [("scene.gltf", document)], options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_decode_work");
    Ok(())
}

#[test]
fn data_uri_byte_limit_is_enforced_before_decode() -> TestResult<()> {
    let mut options = sidecar_options();
    options.limits.max_data_uri_bytes = 103;
    let (result, diagnostics) =
        import_assets_result("scene.gltf", [("scene.gltf", data_uri_gltf())], options)?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_resource_limit(&error, "max_data_uri_bytes");
    Ok(())
}

#[test]
fn data_uri_does_not_require_a_dependency_asset() -> TestResult<()> {
    let gltf = data_uri_gltf();
    let mut options = GltfTestOptions::memory();
    options.limits.max_total_asset_bytes = gltf.len() as u64;
    let (report, diagnostics) = import_assets("scene.gltf", [("scene.gltf", gltf)], options)?;

    assert!(diagnostics.is_empty());
    assert!(report.scene().canonical_block_count() > 0);
    Ok(())
}

#[test]
fn malformed_data_uri_is_parse_error() -> TestResult<()> {
    let gltf = triangle_gltf_with_buffer_uri("data:application/octet-stream;base64AAAA", 104);
    let (result, diagnostics) =
        import_assets_result("scene.gltf", [("scene.gltf", gltf)], sidecar_options())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(&error, GltfFailureCode::InvalidDocument, Some("comma"))?;
    Ok(())
}

#[test]
fn non_base64_data_uri_is_unsupported() -> TestResult<()> {
    let gltf = triangle_gltf_with_buffer_uri("data:application/octet-stream,AAAA", 104);
    let (result, diagnostics) =
        import_assets_result("scene.gltf", [("scene.gltf", gltf)], sidecar_options())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::UnsupportedFeature,
        Some("non-base64"),
    )?;
    Ok(())
}

#[test]
fn invalid_base64_data_uri_is_parse_error() -> TestResult<()> {
    let gltf = triangle_gltf_with_buffer_uri("data:application/octet-stream;base64,@@@=", 104);
    let (result, diagnostics) =
        import_assets_result("scene.gltf", [("scene.gltf", gltf)], sidecar_options())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidReference,
        Some("invalid base64"),
    )?;
    Ok(())
}

#[test]
fn short_data_uri_buffer_is_parse_error() -> TestResult<()> {
    let gltf = triangle_gltf_with_buffer_uri("data:application/octet-stream;base64,AAAA", 104);
    let (result, diagnostics) =
        import_assets_result("scene.gltf", [("scene.gltf", gltf)], sidecar_options())?;
    let error = expected_error(result)?;

    assert!(diagnostics.is_empty());
    assert_gltf_failure(
        &error,
        GltfFailureCode::InvalidDocument,
        Some("declares 104 bytes"),
    )?;
    Ok(())
}

#[test]
fn vertex_limit_is_enforced_from_accessor_count() -> TestResult<()> {
    let mut options = sidecar_options();
    options.limits.max_vertices = 2;
    let (result, _) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", triangle_gltf()),
            ("models/triangle.bin", triangle_bin()),
        ],
        options,
    )?;
    let error = expected_error(result)?;

    assert_resource_limit(&error, "max_vertices");
    Ok(())
}

#[test]
fn face_limit_is_enforced_from_index_accessor_count() -> TestResult<()> {
    let gltf = String::from_utf8(triangle_gltf())
        .expect("fixture is valid utf-8")
        .replace(r#""byteLength": 104"#, r#""byteLength": 700"#)
        .replace(
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 6, "target": 34963 }"#,
            r#"{ "buffer": 0, "byteOffset": 96, "byteLength": 600, "target": 34963 }"#,
        )
        .replace(
            r#"{ "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" }"#,
            r#"{ "bufferView": 3, "componentType": 5123, "count": 300, "type": "SCALAR" }"#,
        )
        .into_bytes();
    let mut options = sidecar_options();
    options.limits.max_faces = 1;
    let (result, _) = import_assets_result(
        "models/scene.gltf",
        [
            ("models/scene.gltf", gltf),
            ("models/triangle.bin", vec![0; 700]),
        ],
        options,
    )?;
    let error = expected_error(result)?;

    assert_resource_limit(&error, "max_faces");
    Ok(())
}
