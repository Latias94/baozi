mod common;

use baozi_format_gltf::GltfFailureCode;
use baozi_import::{
    FormatFailureDetailValue, FormatFailureEnvelope, ImportCause, ImportDimension,
    SceneSnapshotLimits, ScopeAbortCause,
};
use base64::Engine as _;
use common::{
    GltfTestOptions, TestResult, assert_gltf_failure, expected_error, glb_with_json, gltf_failure,
    import_assets, import_assets_result, sidecar_options, triangle_bin, triangle_gltf,
    triangle_png,
};
use serde_json::{Value, json};

fn string_detail<'a>(failure: &'a FormatFailureEnvelope, key: &str) -> Option<&'a str> {
    failure.details().iter().find_map(|detail| {
        if detail.key() != key {
            return None;
        }
        match detail.value() {
            FormatFailureDetailValue::String(value) => Some(value.as_str()),
            _ => None,
        }
    })
}

fn assert_uri_context(failure: &FormatFailureEnvelope, image: bool, index: u64) {
    assert_eq!(
        string_detail(failure, "subject"),
        Some(if image { "texture-uri" } else { "buffer-uri" })
    );
    assert!(failure.details().iter().any(|detail| {
        detail.key() == "index"
            && matches!(detail.value(), FormatFailureDetailValue::Unsigned(value) if value == index)
    }));
}

fn document_with_uri(uri: &str, image: bool) -> TestResult<Vec<u8>> {
    let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
    if image {
        document["images"][0]["uri"] = json!(uri);
        let encoded = base64::engine::general_purpose::STANDARD.encode(triangle_bin());
        document["buffers"][0]["uri"] =
            json!(format!("data:application/octet-stream;base64,{encoded}"));
    } else {
        document["buffers"][0]["uri"] = json!(uri);
    }
    Ok(serde_json::to_vec(&document)?)
}

#[test]
fn version_metadata_requires_objects_and_non_null_strings() -> TestResult<()> {
    for document in [
        json!([]),
        json!(null),
        json!({}),
        json!({"asset": null}),
        json!({"asset": []}),
        json!({"asset": [null, null, null, null, null, "2.0"]}),
        json!({"asset": {}}),
        json!({"asset": {"version": null}}),
        json!({"asset": {"version": 2.0}}),
        json!({"asset": {"version": "2.0", "minVersion": null}}),
        json!({"asset": {"version": "2.0", "minVersion": 2.0}}),
    ] {
        let bytes = serde_json::to_vec(&document)?;
        for (source, bytes) in [
            ("shape.gltf", bytes.clone()),
            ("shape.glb", glb_with_json(&bytes)),
        ] {
            let (result, diagnostics) =
                import_assets_result(source, [(source, bytes)], GltfTestOptions::memory())?;
            let failure = expected_error(result)?;
            assert!(diagnostics.is_empty());
            assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, None)?;
        }
    }
    Ok(())
}

#[test]
fn both_version_fields_follow_the_decimal_component_grammar() -> TestResult<()> {
    for field in ["version", "minVersion"] {
        for value in [
            "",
            "2",
            "2.0.1",
            "+2.0",
            "-2.0",
            "02.0",
            "2.00",
            "2.-1",
            "2.+0",
            "2. 0",
            "2.0\n",
            "2.０",
            "1000000000.0",
            "2.1000000000",
        ] {
            let mut document = json!({"asset": {"version": "2.0"}});
            document["asset"][field] = json!(value);
            let (result, diagnostics) = import_assets_result(
                "version.gltf",
                [("version.gltf", serde_json::to_vec(&document)?)],
                GltfTestOptions::memory(),
            )?;
            let failure = expected_error(result)?;
            assert!(diagnostics.is_empty());
            assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, Some(field))?;
        }
    }
    Ok(())
}

#[test]
fn version_metadata_handles_json_escaped_keys_and_reports_the_actual_field() -> TestResult<()> {
    let (_, diagnostics) = import_assets(
        "version.gltf",
        [(
            "version.gltf",
            br#"{"ass\u0065t":{"v\u0065rsion":"2.0","min\u0056ersion":"2.0"}}"#.to_vec(),
        )],
        GltfTestOptions::memory(),
    )?;
    assert!(diagnostics.is_empty());
    for (document, field) in [
        (json!({"asset": {"version": null}}), "asset.version"),
        (
            json!({"asset": {"version": "2.0", "minVersion": null}}),
            "asset.minVersion",
        ),
    ] {
        let (result, diagnostics) = import_assets_result(
            "version.gltf",
            [("version.gltf", serde_json::to_vec(&document)?)],
            GltfTestOptions::memory(),
        )?;
        let failure = expected_error(result)?;
        assert!(diagnostics.is_empty());
        assert_gltf_failure(&failure, GltfFailureCode::InvalidDocument, Some(field))?;
    }
    Ok(())
}

#[test]
fn compatible_minor_versions_are_accepted_and_minimums_are_negotiated_numerically() -> TestResult<()>
{
    for (version, minimum) in [
        ("2.0", None),
        ("2.1", None),
        ("2.999999999", None),
        ("2.0", Some("2.0")),
        ("2.999999999", Some("2.0")),
    ] {
        let mut document = json!({"asset": {"version": version}});
        if let Some(minimum) = minimum {
            document["asset"]["minVersion"] = json!(minimum);
        }
        let (_, diagnostics) = import_assets(
            "version.gltf",
            [("version.gltf", serde_json::to_vec(&document)?)],
            GltfTestOptions::memory(),
        )?;
        assert!(diagnostics.is_empty());
    }

    for (version, minimum, code, field) in [
        ("1.0", None, GltfFailureCode::UnsupportedFeature, "version"),
        (
            "3.0",
            Some("2.0"),
            GltfFailureCode::UnsupportedFeature,
            "version",
        ),
        (
            "999999999.999999999",
            None,
            GltfFailureCode::UnsupportedFeature,
            "version",
        ),
        (
            "2.1",
            Some("2.1"),
            GltfFailureCode::UnsupportedFeature,
            "minVersion",
        ),
        (
            "2.0",
            Some("1.0"),
            GltfFailureCode::UnsupportedFeature,
            "minVersion",
        ),
        (
            "2.0",
            Some("2.1"),
            GltfFailureCode::InvalidDocument,
            "minVersion",
        ),
        (
            "2.9",
            Some("2.10"),
            GltfFailureCode::InvalidDocument,
            "minVersion",
        ),
        (
            "2.10",
            Some("2.9"),
            GltfFailureCode::UnsupportedFeature,
            "minVersion",
        ),
    ] {
        let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
        document["asset"]["version"] = json!(version);
        if let Some(minimum) = minimum {
            document["asset"]["minVersion"] = json!(minimum);
        }
        let bytes = serde_json::to_vec(&document)?;
        for (source, bytes) in [
            ("models/version.gltf", bytes.clone()),
            ("models/version.glb", glb_with_json(&bytes)),
        ] {
            let (result, diagnostics) =
                import_assets_result(source, [(source, bytes)], sidecar_options())?;
            let failure = expected_error(result)?;
            assert!(diagnostics.is_empty());
            assert_gltf_failure(&failure, code, Some(field))?;
            assert_eq!(
                failure
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0
            );
        }
    }
    Ok(())
}

#[test]
fn utf8_paths_and_percent_encoded_paths_have_identical_canonical_resources() -> TestResult<()> {
    let mut digest = None;
    for (buffer, image) in [
        ("géométrie.bin", "textures/grande_sphère.png"),
        ("g%C3%A9om%C3%A9trie.bin", "textures/grande_sph%c3%a8re.png"),
    ] {
        let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
        document["buffers"][0]["uri"] = json!(buffer);
        document["images"][0]["uri"] = json!(image);
        let (report, diagnostics) = import_assets(
            "models/uri.gltf",
            [
                ("models/uri.gltf", serde_json::to_vec(&document)?),
                ("models/géométrie.bin", triangle_bin()),
                ("models/textures/grande_sphère.png", triangle_png()),
            ],
            sidecar_options(),
        )?;
        assert!(diagnostics.is_empty());
        let identities = report
            .successful_input_manifest()
            .node_classes()
            .iter()
            .map(|class| class.identity().value().as_str())
            .collect::<Vec<_>>();
        assert!(identities.contains(&"models/géométrie.bin"));
        assert!(identities.contains(&"models/textures/grande_sphère.png"));
        let snapshot = report
            .scene()
            .capture_snapshot(SceneSnapshotLimits::try_new(32 * 1024, 256, 1_000_000)?)?;
        if let Some(previous) = digest {
            assert_eq!(snapshot.digest(), previous);
        }
        digest = Some(snapshot.digest());
    }
    Ok(())
}

#[test]
fn uri_decoding_is_single_pass_and_preserves_literal_plus_and_escaped_fragment_characters()
-> TestResult<()> {
    let mut document: Value = serde_json::from_slice(&triangle_gltf())?;
    document["buffers"][0]["uri"] = json!("tri+%252F.bin");
    document["images"][0]["uri"] = json!("textures/base%23.png");
    let (report, diagnostics) = import_assets(
        "models/uri.gltf",
        [
            ("models/uri.gltf", serde_json::to_vec(&document)?),
            ("models/tri+%2F.bin", triangle_bin()),
            ("models/textures/base#.png", triangle_png()),
        ],
        sidecar_options(),
    )?;
    assert!(diagnostics.is_empty());
    let identities = report
        .successful_input_manifest()
        .node_classes()
        .iter()
        .map(|class| class.identity().value().as_str())
        .collect::<Vec<_>>();
    assert!(identities.contains(&"models/tri+%2F.bin"));
    assert!(identities.contains(&"models/textures/base#.png"));
    let snapshot = report
        .scene()
        .capture_snapshot(SceneSnapshotLimits::try_new(32 * 1024, 256, 1_000_000)?)?;
    assert_eq!(snapshot.profile_resource_count(), 3);
    assert_eq!(snapshot.profile_source_count(), 3);
    Ok(())
}

#[test]
fn unsupported_uri_forms_fail_with_resource_context_before_dependency_io() -> TestResult<()> {
    for image in [false, true] {
        for uri in [
            "https://example.invalid/file.bin",
            "file:///file.bin",
            "urn:uuid:example",
            "//host/file.bin",
            "/file.bin",
            "C:/file.bin",
            "file.bin?download=1",
            "file.bin#part",
            r"folder\file.bin",
            "folder%2ffile.bin",
            "folder%5Cfile.bin",
            "%3Afile.bin",
            "%2e%2E/file.bin",
            "folder/.%2e/file.bin",
            "folder/%2e/file.bin",
        ] {
            let (result, diagnostics) = import_assets_result(
                "models/uri.gltf",
                [("models/uri.gltf", document_with_uri(uri, image)?)],
                sidecar_options(),
            )?;
            let failure = expected_error(result)?;
            assert!(diagnostics.is_empty());
            assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
            assert_uri_context(gltf_failure(&failure)?, image, 0);
            assert_eq!(
                failure
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0
            );
        }
    }
    Ok(())
}

#[test]
fn malformed_uri_escapes_and_control_characters_fail_with_resource_context() -> TestResult<()> {
    for image in [false, true] {
        for uri in [
            "",
            "file%",
            "file%2",
            "file%GG",
            "%FF.bin",
            "%C0%AF.bin",
            "%00.bin",
            "file\n.bin",
            "file\u{0085}.bin",
        ] {
            let (result, diagnostics) = import_assets_result(
                "models/uri.gltf",
                [("models/uri.gltf", document_with_uri(uri, image)?)],
                sidecar_options(),
            )?;
            let failure = expected_error(result)?;
            assert!(diagnostics.is_empty());
            assert_gltf_failure(&failure, GltfFailureCode::InvalidReference, None)?;
            assert_uri_context(gltf_failure(&failure)?, image, 0);
            assert_eq!(
                failure
                    .ledger()
                    .dimension(ImportDimension::DependencyReference)
                    .unwrap()
                    .attempted(),
                0
            );
        }
    }
    Ok(())
}

#[test]
fn uri_failures_retain_source_indexes_after_omitted_resources() -> TestResult<()> {
    for image in [false, true] {
        let mut document: Value = serde_json::from_slice(&document_with_uri(
            "https://example.invalid/resource",
            image,
        )?)?;
        if image {
            document["images"]
                .as_array_mut()
                .unwrap()
                .insert(0, json!({"uri": "file:///omitted.png"}));
            document["textures"][0]["source"] = json!(1);
        } else {
            document["buffers"]
                .as_array_mut()
                .unwrap()
                .insert(0, json!({"uri": "file:///omitted.bin", "byteLength": 1}));
            for view in document["bufferViews"].as_array_mut().unwrap() {
                view["buffer"] = json!(1);
            }
        }
        let (result, diagnostics) = import_assets_result(
            "models/uri.gltf",
            [("models/uri.gltf", serde_json::to_vec(&document)?)],
            sidecar_options(),
        )?;
        let failure = expected_error(result)?;
        assert!(diagnostics.is_empty());
        assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
        assert_uri_context(gltf_failure(&failure)?, image, 1);
        assert_eq!(
            failure
                .ledger()
                .dimension(ImportDimension::DependencyReference)
                .unwrap()
                .attempted(),
            0
        );
    }
    Ok(())
}

#[test]
fn decoded_paths_still_require_dependency_authorization() -> TestResult<()> {
    for image in [false, true] {
        let (result, diagnostics) = import_assets_result(
            "models/uri.gltf",
            [(
                "models/uri.gltf",
                document_with_uri("../shared/r%C3%A9source.bin", image)?,
            )],
            GltfTestOptions::memory(),
        )?;
        let failure = expected_error(result)?;
        assert!(diagnostics.is_empty());
        assert!(
            matches!(failure.cause(), ImportCause::Scope(abort) if matches!(abort.cause(), ScopeAbortCause::DependencyPolicy(_)))
        );
    }
    Ok(())
}

#[test]
fn unsupported_data_url_spellings_are_not_misinterpreted_as_base64_payloads() -> TestResult<()> {
    for image in [false, true] {
        for uri in [
            "data:application/octet-stream;base64,AA%3D%3D",
            "data:base64,AAAA",
            "data:application/octet-stream;base64;charset=UTF-8,AAAA",
        ] {
            let (result, diagnostics) = import_assets_result(
                "models/uri.gltf",
                [("models/uri.gltf", document_with_uri(uri, image)?)],
                GltfTestOptions::memory(),
            )?;
            let failure = expected_error(result)?;
            assert!(diagnostics.is_empty());
            assert_gltf_failure(&failure, GltfFailureCode::UnsupportedFeature, None)?;
            assert_uri_context(gltf_failure(&failure)?, image, 0);
        }
    }
    Ok(())
}
